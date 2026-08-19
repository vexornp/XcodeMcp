use serde_json::{json, Value};
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use xcode_mcp_core::{
    diagnostic::load_diagnostics,
    scheme::list_schemes,
    store::BuildStore,
    xcode::{run_build, BuildParams},
};

const PROTOCOL_VERSION: &str = "2024-11-05";

struct XcodeMcpServer {
    root: PathBuf,
    result_dir: PathBuf,
    log_dir: PathBuf,
    store: Arc<BuildStore>,
}

pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let root_str = env::var("XCODE_MCP_ROOT").map_err(|_| {
        "XCODE_MCP_ROOT not set. Set it to the directory containing your Xcode projects."
    })?;
    let root = PathBuf::from(&root_str);
    if !root.exists() {
        return Err(format!("XCODE_MCP_ROOT does not exist: {root_str}").into());
    }
    let root = root.canonicalize()?;

    let result_dir = PathBuf::from(env::var("XCODE_MCP_RESULT_DIR").unwrap_or_else(|_| {
        root.join(".xcode-mcp-results")
            .to_string_lossy()
            .into_owned()
    }));
    let log_dir = PathBuf::from(
        env::var("XCODE_MCP_LOG_DIR")
            .unwrap_or_else(|_| root.join(".xcode-mcp-logs").to_string_lossy().into_owned()),
    );
    std::fs::create_dir_all(&result_dir)?;
    std::fs::create_dir_all(&log_dir)?;

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("server.log"))?;
    let _ = tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    tracing::info!(
        "xcode-mcp server starting: root={}, result_dir={}, log_dir={}",
        root.display(),
        result_dir.display(),
        log_dir.display()
    );

    let store = Arc::new(BuildStore::new(
        env::var("XCODE_MCP_STORE_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(32),
    ));
    let server = XcodeMcpServer {
        root,
        result_dir,
        log_dir,
        store,
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("stdin read error: {e}");
                break;
            }
        };
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("invalid JSON line: {e}");
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        if id.is_none() {
            if method == "notifications/initialized" {
                tracing::debug!("client initialized");
            }
            continue;
        }

        let result: Option<Result<Value, Value>> = match method {
            "initialize" => Some(Ok(handle_initialize())),
            "ping" => Some(Ok(json!({}))),
            "tools/list" => Some(Ok(handle_list_tools())),
            "tools/call" => Some(server.handle_call_tool(&params).await),
            _ => Some(Err(jsonrpc_error(
                -32601,
                &format!("method not found: {method}"),
            ))),
        };

        if let Some(res) = result {
            let response = match res {
                Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
                Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": err }),
            };
            let serialized = match serde_json::to_string(&response) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("response serialization error: {e}");
                    break;
                }
            };
            if let Err(e) = writeln!(stdout, "{serialized}") {
                tracing::error!("stdout write error: {e}");
                break;
            }
            if let Err(e) = stdout.flush() {
                tracing::error!("stdout flush error: {e}");
                break;
            }
        }
    }

    tracing::info!("xcode-mcp server shutting down (stdin closed)");
    Ok(())
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": "xcode-mcp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_list_tools() -> Value {
    json!({
        "tools": [
            {
                "name": "xcode_list_schemes",
                "description": "List schemes, targets, and configurations for an Xcode project or workspace",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_or_workspace": { "type": "string" }
                    },
                    "required": ["project_or_workspace"]
                }
            },
            {
                "name": "xcode_build",
                "description": "Run an xcodebuild and return build_id + status",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_or_workspace": { "type": "string" },
                        "scheme": { "type": "string" },
                        "action": { "type": "string", "enum": ["build", "clean", "clean+build"] },
                        "configuration": { "type": "string", "enum": ["Debug", "Release"] },
                        "destination": { "type": "string" },
                        "timeout_secs": { "type": "integer" }
                    },
                    "required": ["project_or_workspace", "scheme"]
                }
            },
            {
                "name": "xcode_get_build_errors",
                "description": "Get structured build diagnostics for a build",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "build_id": { "type": "string" }
                    }
                }
            }
        ]
    })
}

impl XcodeMcpServer {
    /// Returns `Ok(CallToolResult)` for valid tool calls (with `isError: true`
    /// for tool execution failures), or `Err(jsonrpc_error)` for protocol-level
    /// errors (missing params, unknown tool).
    async fn handle_call_tool(&self, params: &Value) -> Result<Value, Value> {
        let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

        let result: Result<Value, Value> = match name {
            "xcode_list_schemes" => {
                let project = arguments
                    .get("project_or_workspace")
                    .and_then(|p| p.as_str());
                let Some(project) = project else {
                    return Err(jsonrpc_error(-32602, "project_or_workspace required"));
                };
                list_schemes(project, &self.root)
                    .await
                    .map(|info| serde_json::to_value(&info).unwrap_or(Value::Null))
                    .map_err(|e| jsonrpc_error(-32603, &format!("{e}")))
            }
            "xcode_build" => {
                let project = arguments
                    .get("project_or_workspace")
                    .and_then(|p| p.as_str());
                let Some(project) = project else {
                    return Err(jsonrpc_error(-32602, "project_or_workspace required"));
                };
                let scheme = arguments.get("scheme").and_then(|s| s.as_str());
                let Some(scheme) = scheme else {
                    return Err(jsonrpc_error(-32602, "scheme required"));
                };
                let build_params = BuildParams {
                    project_or_workspace: project.to_string(),
                    scheme: scheme.to_string(),
                    action: arguments
                        .get("action")
                        .and_then(|a| a.as_str())
                        .map(String::from),
                    configuration: arguments
                        .get("configuration")
                        .and_then(|c| c.as_str())
                        .map(String::from),
                    destination: arguments
                        .get("destination")
                        .and_then(|d| d.as_str())
                        .map(String::from),
                    timeout_secs: arguments
                        .get("timeout_secs")
                        .and_then(|t| t.as_u64())
                        .map(|n| n as u32),
                };
                run_build(
                    build_params,
                    &self.root,
                    &self.result_dir,
                    &self.log_dir,
                    &self.store,
                )
                .await
                .map(|output| serde_json::to_value(&output).unwrap_or(Value::Null))
                .map_err(|e| jsonrpc_error(-32603, &format!("{e}")))
            }
            "xcode_get_build_errors" => {
                let build_id = arguments.get("build_id").and_then(|b| b.as_str());
                load_diagnostics(build_id, &self.store, &self.result_dir, &self.log_dir)
                    .await
                    .map(|output| serde_json::to_value(&output).unwrap_or(Value::Null))
                    .map_err(|e| jsonrpc_error(-32603, &format!("{e}")))
            }
            _ => return Err(jsonrpc_error(-32602, &format!("unknown tool: {name}"))),
        };

        match result {
            Ok(value) => {
                let text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                }))
            }
            Err(err) => {
                let text = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                Ok(json!({
                    "content": [{ "type": "text", "text": format!("Error: {text}") }],
                    "isError": true
                }))
            }
        }
    }
}

fn jsonrpc_error(code: i32, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

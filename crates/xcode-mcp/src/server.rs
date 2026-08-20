use rmcp::{
    self,
    model::{
        CallToolRequestParams, CallToolResponse, ContentBlock, ErrorData, Implementation,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
        Tool,
    },
    service::{MaybeSendFuture, RequestContext},
    RoleServer, ServerHandler, ServiceExt,
};
use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use xcode_mcp_core::{
    diagnostic::load_diagnostics,
    scheme::list_schemes,
    store::BuildStore,
    xcode::{run_build, BuildParams},
};

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

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = server.serve((stdin, stdout)).await?;
    running.waiting().await?;

    tracing::info!("xcode-mcp server shutting down (stdin closed)");
    Ok(())
}

fn make_tool_list() -> Vec<Tool> {
    let list_schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
            "type": "object",
            "properties": {
                "project_or_workspace": { "type": "string" }
            },
            "required": ["project_or_workspace"]
        }"#,
    )
    .unwrap();
    let build_schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
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
        }"#,
    )
    .unwrap();
    let errors_schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
            "type": "object",
            "properties": {
                "build_id": { "type": "string" }
            }
        }"#,
    )
    .unwrap();
    vec![
        Tool::new(
            "xcode_list_schemes",
            "List schemes, targets, and configurations for an Xcode project or workspace",
            list_schema,
        ),
        Tool::new(
            "xcode_build",
            "Run an xcodebuild and return build_id + status",
            build_schema,
        ),
        Tool::new(
            "xcode_get_build_errors",
            "Get structured build diagnostics for a build",
            errors_schema,
        ),
    ]
}

impl ServerHandler for XcodeMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new("xcode-mcp", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(make_tool_list())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let name = request.name.clone();
        let arguments = request.arguments.clone();
        async move { self.dispatch_tool(name.as_ref(), arguments.as_ref()).await }
    }
}

impl XcodeMcpServer {
    async fn dispatch_tool(
        &self,
        name: &str,
        arguments: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = arguments.cloned().unwrap_or_default();
        let result: Result<serde_json::Value, String> = match name {
            "xcode_list_schemes" => {
                let project = get_string_arg(&args, "project_or_workspace")?;
                list_schemes(&project, &self.root)
                    .await
                    .map(|info| serde_json::to_value(&info).unwrap_or(serde_json::Value::Null))
                    .map_err(|e| e.to_string())
            }
            "xcode_build" => {
                let project = get_string_arg(&args, "project_or_workspace")?;
                let scheme = get_string_arg(&args, "scheme")?;
                let build_params = BuildParams {
                    project_or_workspace: project,
                    scheme,
                    action: get_optional_string_arg(&args, "action"),
                    configuration: get_optional_string_arg(&args, "configuration"),
                    destination: get_optional_string_arg(&args, "destination"),
                    timeout_secs: get_optional_u64_arg(&args, "timeout_secs").map(|n| n as u32),
                };
                run_build(
                    build_params,
                    &self.root,
                    &self.result_dir,
                    &self.log_dir,
                    &self.store,
                )
                .await
                .map(|output| serde_json::to_value(&output).unwrap_or(serde_json::Value::Null))
                .map_err(|e| e.to_string())
            }
            "xcode_get_build_errors" => {
                let build_id = get_optional_string_arg(&args, "build_id");
                load_diagnostics(
                    build_id.as_deref(),
                    &self.store,
                    &self.result_dir,
                    &self.log_dir,
                )
                .await
                .map(|output| serde_json::to_value(&output).unwrap_or(serde_json::Value::Null))
                .map_err(|e| e.to_string())
            }
            _ => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool: {name}"),
                    None,
                ));
            }
        };

        match result {
            Ok(value) => {
                let text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                Ok(CallToolResponse::Complete(
                    rmcp::model::CallToolResult::success(vec![ContentBlock::text(text)]),
                ))
            }
            Err(msg) => Ok(CallToolResponse::Complete(
                rmcp::model::CallToolResult::error(vec![ContentBlock::text(format!(
                    "Error: {msg}"
                ))]),
            )),
        }
    }
}

fn get_string_arg(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, ErrorData> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ErrorData::invalid_params(format!("{key} required"), None))
}

fn get_optional_string_arg(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_optional_u64_arg(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

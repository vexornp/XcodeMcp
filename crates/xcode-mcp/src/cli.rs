use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use xcode_mcp_core::{
    diagnostic::load_diagnostics,
    scheme::list_schemes,
    store::BuildStore,
    xcode::{run_build, BuildParams},
};

#[derive(Parser)]
#[command(
    name = "xcode-mcp",
    version,
    about = "MCP server that drives xcodebuild and parses build diagnostics"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run as MCP server (stdio transport)
    Serve,
    /// Debug subcommands for non-MCP testing
    Debug {
        #[command(subcommand)]
        subcommand: DebugCommand,
    },
}

#[derive(Subcommand)]
pub enum DebugCommand {
    /// List schemes for a project or workspace
    ListSchemes {
        project_or_workspace: String,
        #[arg(long)]
        root: Option<String>,
    },
    /// Run a build
    Build {
        #[arg(long)]
        project: String,
        #[arg(long)]
        scheme: String,
        #[arg(long, default_value = "build")]
        action: String,
        #[arg(long)]
        configuration: Option<String>,
        #[arg(long)]
        destination: Option<String>,
        #[arg(long)]
        timeout_secs: Option<u32>,
        #[arg(long)]
        root: Option<String>,
        #[arg(long)]
        result_dir: Option<PathBuf>,
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
    /// Get build errors for a build
    BuildErrors {
        build_id: Option<String>,
        #[arg(long)]
        result_dir: Option<PathBuf>,
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
    /// Print MCP Inspector instructions
    InspectorHelp,
}

pub async fn run_debug(subcommand: DebugCommand) -> Result<(), Box<dyn std::error::Error>> {
    match subcommand {
        DebugCommand::ListSchemes {
            project_or_workspace,
            root,
        } => {
            let root = resolve_root(&root, &project_or_workspace)?;
            let info = list_schemes(&project_or_workspace, &root).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        DebugCommand::Build {
            project,
            scheme,
            action,
            configuration,
            destination,
            timeout_secs,
            root,
            result_dir,
            log_dir,
        } => {
            let root = resolve_root(&root, &project)?;
            let result_dir = resolve_dir(result_dir, &root, ".xcode-mcp-results")?;
            let log_dir = resolve_dir(log_dir, &root, ".xcode-mcp-logs")?;
            std::fs::create_dir_all(&result_dir)?;
            std::fs::create_dir_all(&log_dir)?;
            let store = BuildStore::new(32);
            let params = BuildParams {
                project_or_workspace: project,
                scheme,
                action: Some(action),
                configuration,
                destination,
                timeout_secs,
                pod_action: None,
                pod_timeout_secs: None,
            };
            let output = run_build(params, &root, &result_dir, &log_dir, &store).await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        DebugCommand::BuildErrors {
            build_id,
            result_dir,
            log_dir,
        } => {
            let root = std::env::current_dir()?;
            let result_dir = resolve_dir(result_dir, &root, ".xcode-mcp-results")?;
            let log_dir = resolve_dir(log_dir, &root, ".xcode-mcp-logs")?;
            let store = BuildStore::new(32);
            let output =
                load_diagnostics(build_id.as_deref(), &store, &result_dir, &log_dir).await?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        DebugCommand::InspectorHelp => {
            print_inspector_help();
        }
    }
    Ok(())
}

fn resolve_root(
    root: &Option<String>,
    project: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(r) = root {
        let p = PathBuf::from(r);
        if !p.is_absolute() {
            return Err("--root must be absolute".into());
        }
        Ok(p.canonicalize()?)
    } else {
        let p = PathBuf::from(project);
        Ok(p.parent().ok_or("cannot determine root")?.canonicalize()?)
    }
}

fn resolve_dir(
    dir: Option<PathBuf>,
    root: &Path,
    default_subdir: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match dir {
        Some(p) => {
            if !p.is_absolute() {
                return Err("--result-dir/--log-dir must be absolute".into());
            }
            Ok(p)
        }
        None => Ok(root.join(default_subdir)),
    }
}

fn print_inspector_help() {
    println!("MCP Inspector Instructions");
    println!("===========================");
    println!();
    println!("1. Build the server:");
    println!("   cargo build --release");
    println!();
    println!("2. Launch the MCP Inspector:");
    println!("   npx @modelcontextprotocol/inspector ./target/release/xcode-mcp serve");
    println!();
    println!("3. In the Inspector UI:");
    println!("   - Transport: STDIO");
    println!("   - Command:   ./target/release/xcode-mcp");
    println!("   - Args:      serve");
    println!("   - Env:       XCODE_MCP_ROOT=/path/to/your/projects");
    println!("   - Click Connect");
    println!();
    println!("4. Set XCODE_MCP_ROOT in the Environment panel BEFORE connecting.");
    println!();
    println!("5. Verify each tool:");
    println!("   - xcode_list_schemes: pass a .xcodeproj path");
    println!("   - xcode_build:        pass path + scheme");
    println!("   - xcode_get_build_errors: pass the build_id");
}

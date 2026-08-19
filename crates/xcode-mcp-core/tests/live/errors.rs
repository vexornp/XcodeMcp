#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{
    diagnostic::{load_diagnostics, DiagnosticSourceLabel},
    store::BuildStore,
    xcode::{run_build, BuildParams},
};

fn skip_if_not_enabled() -> bool {
    env::var("XCODE_MCP_LIVE_TESTS").is_err()
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/MiniApp")
        .canonicalize()
        .unwrap()
}

#[tokio::test]
async fn get_build_errors_returns_xcresult_diagnostics() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: root.join("MiniApp.xcodeproj").to_string_lossy().into(),
        scheme: "MiniAppBroken".into(),
        action: Some("build".into()),
        configuration: Some("Debug".into()),
        destination: Some("platform=macOS".into()),
        timeout_secs: Some(300),
    };
    let build_output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .unwrap();
    assert_eq!(build_output.status, "Failed");
    let diag_output = load_diagnostics(Some(&build_output.build_id), &store, &result_dir, &log_dir)
        .await
        .unwrap();
    assert_eq!(diag_output.build_id, build_output.build_id);
    assert!(matches!(
        diag_output.source,
        DiagnosticSourceLabel::Xcresult
    ));
    assert!(!diag_output.merged.errors.is_empty());
    let first_error = &diag_output.merged.errors[0];
    assert!(first_error.file.is_some());
    assert!(first_error.message.contains("nonexistentVariable") || first_error.line.is_some());
}

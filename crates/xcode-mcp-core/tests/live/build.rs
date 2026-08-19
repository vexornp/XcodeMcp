#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{
    scheme::list_schemes,
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
async fn list_schemes_finds_miniapp() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let info = list_schemes(proj.to_str().unwrap(), &root).await.unwrap();
    assert!(info.schemes.contains(&"MiniApp".to_string()));
    assert!(info.schemes.contains(&"MiniAppBroken".to_string()));
}

#[tokio::test]
async fn build_succeeds_for_valid_app() {
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
        scheme: "MiniApp".into(),
        action: Some("build".into()),
        configuration: Some("Debug".into()),
        destination: Some("platform=macOS".into()),
        timeout_secs: Some(300),
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .unwrap();
    assert_eq!(output.status, "Succeeded");
    assert!(output.result_bundle_written);
}

#[tokio::test]
async fn build_fails_for_broken_app() {
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
    let output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .unwrap();
    assert_eq!(output.status, "Failed");
    assert!(output.error_count > 0);
}

#[tokio::test]
async fn build_times_out() {
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
        scheme: "MiniApp".into(),
        action: Some("clean+build".into()),
        configuration: Some("Debug".into()),
        destination: Some("platform=macOS".into()),
        timeout_secs: Some(1),
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .unwrap();
    assert_eq!(output.status, "TimedOut");
    assert!(output.exit_code.is_none());
}

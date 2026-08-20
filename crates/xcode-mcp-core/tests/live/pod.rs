#![cfg(feature = "live-xcode")]

use std::env;
use std::path::PathBuf;
use xcode_mcp_core::{
    pod::{run_pod, PodParams},
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
async fn pod_returns_podfile_not_found_for_clean_project() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let params = PodParams {
        project_or_workspace: proj.to_string_lossy().into_owned(),
        action: Some("install".into()),
        timeout_secs: None,
    };
    let result = run_pod(params, &root, &log_dir).await;
    assert!(result.is_err(), "MiniApp has no Podfile — expected error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("no Podfile found"),
        "expected PodfileNotFound, got: {msg}"
    );
}

#[tokio::test]
async fn build_with_pod_action_aborts_when_no_podfile() {
    if skip_if_not_enabled() {
        eprintln!("skipping: set XCODE_MCP_LIVE_TESTS=1");
        return;
    }
    let root = fixture_root();
    let proj = root.join("MiniApp.xcodeproj");
    let result_dir = root.join(".xcode-mcp-results");
    let log_dir = root.join(".xcode-mcp-logs");
    std::fs::create_dir_all(&result_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let params = BuildParams {
        project_or_workspace: proj.to_string_lossy().into_owned(),
        scheme: "MiniApp".into(),
        action: Some("build".into()),
        configuration: None,
        destination: None,
        timeout_secs: None,
        pod_action: Some("install".into()),
        pod_timeout_secs: None,
    };
    let output = run_build(params, &root, &result_dir, &log_dir, &store)
        .await
        .expect("run_build should not hard-error on PodFailed");
    assert_eq!(output.status, "PodFailed");
    assert!(!output.result_bundle_written);
    assert!(output.pod.is_some(), "pod field must be present");
    let pod = output.pod.unwrap();
    assert_eq!(pod.status, "Failed");
    assert!(pod.stderr_excerpt.is_some());
    assert!(output.error_count == 0);
}

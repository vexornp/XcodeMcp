use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use xcode_mcp_core::diagnostic::*;
use xcode_mcp_core::store::*;

fn make_record(id: &str, xcresult: &Path, log: &Path, status: BuildStatus) -> BuildRecord {
    BuildRecord {
        build_id: id.into(),
        status,
        exit_code: Some(1),
        duration_secs: 1.0,
        project_or_workspace: PathBuf::from("/tmp/App.xcodeproj"),
        scheme: "App".into(),
        xcresult_path: xcresult.to_path_buf(),
        log_path: log.to_path_buf(),
        result_bundle_written: true,
        error_count: 0,
        warning_count: 0,
        stderr_excerpt: None,
        created_at: std::time::SystemTime::now(),
    }
}

#[tokio::test]
async fn returns_none_source_for_succeeded_build() {
    let dir = tempdir().unwrap();
    let result_dir = dir.path().join("results");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&result_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    let xcresult = result_dir.join("b1.xcresult");
    let log = log_dir.join("b1.log");
    fs::write(&log, "").unwrap();
    let store = BuildStore::new(32);
    store.push(make_record("b1", &xcresult, &log, BuildStatus::Succeeded));
    let output = load_diagnostics(Some("b1"), &store, &result_dir, &log_dir)
        .await
        .unwrap();
    assert_eq!(output.build_id, "b1");
    assert!(matches!(output.source, DiagnosticSourceLabel::None));
    assert!(output.merged.errors.is_empty());
}

#[tokio::test]
async fn errors_when_build_not_found() {
    let dir = tempdir().unwrap();
    let result_dir = dir.path().join("results");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&result_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    assert!(
        load_diagnostics(Some("nonexistent"), &store, &result_dir, &log_dir)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_path_traversal_in_build_id() {
    let dir = tempdir().unwrap();
    let result_dir = dir.path().join("results");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&result_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    let store = BuildStore::new(32);
    let result = load_diagnostics(Some("../../etc/passwd"), &store, &result_dir, &log_dir).await;
    assert!(result.is_err());
}

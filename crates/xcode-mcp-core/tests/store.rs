use std::path::PathBuf;
use xcode_mcp_core::store::*;

fn make_record(id: &str) -> BuildRecord {
    BuildRecord {
        build_id: id.into(),
        status: BuildStatus::Failed,
        exit_code: Some(1),
        duration_secs: 1.0,
        project_or_workspace: PathBuf::from("/tmp/App.xcodeproj"),
        scheme: "App".into(),
        xcresult_path: PathBuf::from(format!("/tmp/{id}.xcresult")),
        log_path: PathBuf::from(format!("/tmp/{id}.log")),
        result_bundle_written: true,
        error_count: 1,
        warning_count: 0,
        stderr_excerpt: None,
        created_at: std::time::SystemTime::now(),
    }
}

#[test]
fn push_and_get() {
    let store = BuildStore::new(32);
    store.push(make_record("build-1"));
    assert_eq!(store.get("build-1").unwrap().build_id, "build-1");
}

#[test]
fn most_recent_returns_last_pushed() {
    let store = BuildStore::new(32);
    store.push(make_record("build-1"));
    store.push(make_record("build-2"));
    assert_eq!(store.most_recent().unwrap().build_id, "build-2");
}

#[test]
fn returns_none_when_empty() {
    let store = BuildStore::new(32);
    assert!(store.most_recent().is_none());
    assert!(store.get("nope").is_none());
}

#[test]
fn evicts_oldest_when_full() {
    let store = BuildStore::new(2);
    store.push(make_record("build-1"));
    store.push(make_record("build-2"));
    store.push(make_record("build-3"));
    assert!(store.get("build-1").is_none());
    assert!(store.get("build-2").is_some());
    assert!(store.get("build-3").is_some());
    assert_eq!(store.most_recent().unwrap().build_id, "build-3");
}

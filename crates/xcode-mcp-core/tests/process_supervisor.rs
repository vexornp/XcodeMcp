use std::time::Duration;
use tokio::process::Command;
use xcode_mcp_core::xcode::*;

#[tokio::test]
async fn runs_command_to_completion() {
    let mut cmd = Command::new("echo");
    cmd.arg("hello");
    let result = run_supervised(cmd, 10, None).await.unwrap();
    assert!(!result.timed_out);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
}

#[tokio::test]
async fn captures_stderr() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "echo err >&2; exit 3"]);
    let result = run_supervised(cmd, 10, None).await.unwrap();
    assert_eq!(result.exit_code, Some(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("err"));
}

#[tokio::test]
async fn times_out_and_kills() {
    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let start = tokio::time::Instant::now();
    let result = run_supervised(cmd, 1, None).await.unwrap();
    assert!(result.timed_out);
    assert_eq!(result.exit_code, None);
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn writes_to_log_file_when_provided() {
    let log_path = tempfile::NamedTempFile::new()
        .unwrap()
        .into_temp_path()
        .keep()
        .unwrap();
    let mut cmd = Command::new("echo");
    cmd.arg("logged");
    let result = run_supervised(cmd, 10, Some(&log_path)).await.unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(std::fs::read_to_string(&log_path)
        .unwrap()
        .contains("logged"));
}

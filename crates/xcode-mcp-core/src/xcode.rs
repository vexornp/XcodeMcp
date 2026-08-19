use crate::error::{Error, Result};
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;

pub fn build_list_schemes_command(project_or_workspace: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-list");
    if project_or_workspace
        .to_string_lossy()
        .ends_with(".xcworkspace")
    {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    cmd
}

#[allow(clippy::too_many_arguments)]
pub fn build_xcodebuild_command(
    project_or_workspace: &Path,
    scheme: &str,
    action: &str,
    configuration: Option<&str>,
    destination: Option<&str>,
    xcresult_path: &Path,
    derived_data_path: &Path,
) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcodebuild").arg("-scheme").arg(scheme);
    if project_or_workspace
        .to_string_lossy()
        .ends_with(".xcworkspace")
    {
        cmd.arg("-workspace").arg(project_or_workspace);
    } else {
        cmd.arg("-project").arg(project_or_workspace);
    }
    if let Some(cfg) = configuration {
        cmd.arg("-configuration").arg(cfg);
    }
    if let Some(dest) = destination {
        cmd.arg("-destination").arg(dest);
    }
    cmd.arg("-resultBundlePath")
        .arg(xcresult_path)
        .arg("-derivedDataPath")
        .arg(derived_data_path)
        .arg("-quiet");
    match action {
        "clean+build" => {
            cmd.arg("clean").arg("build");
        }
        "clean" => {
            cmd.arg("clean");
        }
        _ => {
            cmd.arg("build");
        }
    }
    cmd
}

pub fn build_xcresulttool_command(xcresult_path: &Path) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.arg("xcresulttool")
        .arg("get")
        .arg("build-results")
        .arg("--format")
        .arg("json")
        .arg("--path")
        .arg(xcresult_path);
    cmd
}

pub struct SupervisedResult {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

pub async fn run_supervised(
    mut cmd: Command,
    timeout_secs: u32,
    log_file: Option<&Path>,
) -> Result<SupervisedResult> {
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::XcodeSpawnFailed(e.to_string()))?;
    let pid = child.id().expect("child pid");
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let log_writer = if let Some(path) = log_file {
        Some(Arc::new(Mutex::new(std::fs::File::create(path)?)))
    } else {
        None
    };

    let stdout_task = {
        let buf = stdout_buf.clone();
        let log = log_writer.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = &chunk[..n];
                        buf.lock().await.extend_from_slice(data);
                        if let Some(ref log) = log {
                            use std::io::Write;
                            let _ = log.lock().await.write_all(data);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    };
    let stderr_task = {
        let buf = stderr_buf.clone();
        let log = log_writer.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = &chunk[..n];
                        buf.lock().await.extend_from_slice(data);
                        if let Some(ref log) = log {
                            use std::io::Write;
                            let _ = log.lock().await.write_all(data);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    };

    let wait_result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64),
        child.wait(),
    )
    .await;

    match wait_result {
        Ok(Ok(status)) => {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            Ok(SupervisedResult {
                exit_code: status.code(),
                timed_out: false,
                stdout: stdout_buf.lock().await.clone(),
                stderr: stderr_buf.lock().await.clone(),
            })
        }
        Ok(Err(e)) => Err(Error::Internal(format!("wait failed: {e}"))),
        Err(_) => {
            kill_process_group(pid, libc::SIGTERM);
            if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                kill_process_group(pid, libc::SIGKILL);
                let _ = child.wait().await;
            }
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            Ok(SupervisedResult {
                exit_code: None,
                timed_out: true,
                stdout: stdout_buf.lock().await.clone(),
                stderr: stderr_buf.lock().await.clone(),
            })
        }
    }
}

fn kill_process_group(pid: u32, sig: i32) {
    unsafe {
        libc::kill(-(pid as i32), sig);
    }
}

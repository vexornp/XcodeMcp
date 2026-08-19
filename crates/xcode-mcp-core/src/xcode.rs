use crate::diagnostic::load_diagnostics;
use crate::error::{Error, Result};
use crate::security::{
    validate_action, validate_configuration, validate_destination, validate_project_or_workspace,
    validate_scheme, validate_timeout,
};
use crate::store::{BuildRecord, BuildStatus, BuildStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;

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
                let _ = child.start_kill();
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

static BUILD_PERMIT: Semaphore = Semaphore::const_new(1);

#[derive(Debug, Clone, Deserialize)]
pub struct BuildParams {
    pub project_or_workspace: String,
    pub scheme: String,
    pub action: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildOutput {
    pub build_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub xcresult_path: String,
    pub log_path: String,
    pub result_bundle_written: bool,
    pub error_count: u32,
    pub warning_count: u32,
    pub truncated_stderr_excerpt: Option<String>,
}

pub async fn run_build(
    params: BuildParams,
    root: &Path,
    result_dir: &Path,
    log_dir: &Path,
    store: &BuildStore,
) -> Result<BuildOutput> {
    // 1. Validate inputs (before acquiring permit)
    let validated_path: PathBuf =
        validate_project_or_workspace(&params.project_or_workspace, root)?;
    let scheme = validate_scheme(&params.scheme)?;
    let action = validate_action(params.action.as_deref().unwrap_or("build"))?;
    let configuration = match params.configuration.as_deref() {
        Some(c) => Some(validate_configuration(c)?),
        None => None,
    };
    let destination = match params.destination.as_deref() {
        Some(d) => Some(validate_destination(d)?),
        None => None,
    };
    let timeout_secs = validate_timeout(params.timeout_secs)?;

    // 2. Reserve build_id + paths
    let build_id = uuid::Uuid::new_v4().to_string();
    let xcresult_path = result_dir.join(format!("{build_id}.xcresult"));
    let derived_data_path = result_dir.join("DerivedData").join(&build_id);
    let log_path = log_dir.join(format!("{build_id}.log"));
    std::fs::File::create(&log_path)?;

    // 3. Acquire global build permit (serialized execution)
    let _permit = BUILD_PERMIT
        .acquire()
        .await
        .map_err(|e| crate::error::Error::Internal(format!("semaphore closed: {e}")))?;

    // 4. Build command
    let cmd = build_xcodebuild_command(
        &validated_path,
        &scheme,
        &action,
        configuration.as_deref(),
        destination.as_deref(),
        &xcresult_path,
        &derived_data_path,
    );

    // 5. Run supervised
    let start = std::time::Instant::now();
    let result = run_supervised(cmd, timeout_secs, Some(&log_path)).await?;
    let duration = start.elapsed().as_secs_f64();

    // 6. Check result bundle
    let result_bundle_written = xcresult_path.exists();

    // 7. Determine status
    let (status, exit_code) = if result.timed_out {
        ("TimedOut".to_string(), None)
    } else if result.exit_code == Some(0) {
        ("Succeeded".to_string(), result.exit_code)
    } else {
        ("Failed".to_string(), result.exit_code)
    };

    // 8. Truncated stderr excerpt (last 2KB)
    let truncated_stderr_excerpt =
        if (status == "Failed" || status == "TimedOut") && !result_bundle_written {
            let s = String::from_utf8_lossy(&result.stderr);
            let chars: Vec<char> = s.chars().rev().take(2048).collect();
            Some(chars.into_iter().rev().collect())
        } else {
            None
        };

    // 9. Clean up derived data
    if derived_data_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(&derived_data_path) {
            tracing::warn!("failed to clean derived data: {e}");
        }
    }

    // 10. Determine build status
    let build_status = match status.as_str() {
        "Succeeded" => BuildStatus::Succeeded,
        "TimedOut" => BuildStatus::TimedOut,
        _ => BuildStatus::Failed,
    };

    // 11. Best-effort: compute error/warning counts (before storing so
    //     load_diagnostics uses the filesystem fallback, not a stale record)
    let (error_count, warning_count) = if result_bundle_written {
        match load_diagnostics(Some(&build_id), store, result_dir, log_dir).await {
            Ok(o) => (o.merged.errors.len() as u32, o.merged.warnings.len() as u32),
            Err(e) => {
                tracing::warn!("failed to compute diagnostic counts: {e}");
                (0, 0)
            }
        }
    } else {
        (0, 0)
    };

    // 12. Register in store with computed counts
    store.push(BuildRecord {
        build_id: build_id.clone(),
        status: build_status.clone(),
        exit_code,
        duration_secs: duration,
        project_or_workspace: validated_path.clone(),
        scheme: scheme.clone(),
        xcresult_path: xcresult_path.clone(),
        log_path: log_path.clone(),
        result_bundle_written,
        error_count,
        warning_count,
        stderr_excerpt: truncated_stderr_excerpt.clone(),
        created_at: std::time::SystemTime::now(),
    });

    Ok(BuildOutput {
        build_id,
        status,
        exit_code,
        duration_secs: duration,
        xcresult_path: xcresult_path.to_string_lossy().into_owned(),
        log_path: log_path.to_string_lossy().into_owned(),
        result_bundle_written,
        error_count,
        warning_count,
        truncated_stderr_excerpt,
    })
}

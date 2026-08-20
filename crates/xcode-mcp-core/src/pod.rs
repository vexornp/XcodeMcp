use crate::error::{Error, Result};
use crate::security::{
    validate_pod_action, validate_pod_timeout, validate_project_or_workspace,
};
use crate::xcode::run_supervised;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub fn build_pod_command(working_dir: &Path, action: &str) -> Command {
    let mut cmd = Command::new("pod");
    cmd.arg(action).arg("--no-ansi");
    cmd.current_dir(working_dir);
    cmd
}

#[derive(Debug, Clone, Deserialize)]
pub struct PodParams {
    pub project_or_workspace: String,
    pub action: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PodOutput {
    pub run_id: String,
    pub action: String,
    pub working_dir: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub log_path: String,
    pub stderr_excerpt: Option<String>,
}

pub async fn run_pod(params: PodParams, root: &Path, log_dir: &Path) -> Result<PodOutput> {
    let validated_path = validate_project_or_workspace(&params.project_or_workspace, root)?;
    let working_dir: PathBuf = validated_path
        .parent()
        .ok_or_else(|| Error::PathRejected(format!(
            "cannot resolve parent of {}", validated_path.display()
        )))?
        .to_path_buf();

    let action = validate_pod_action(params.action.as_deref().unwrap_or("install"))?;
    let timeout_secs = validate_pod_timeout(params.timeout_secs)?;

    if !working_dir.join("Podfile").exists() {
        return Err(Error::PodfileNotFound { working_dir });
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let log_path = log_dir.join(format!("{run_id}.pod.log"));
    std::fs::File::create(&log_path)?;

    let cmd = build_pod_command(&working_dir, &action);
    let start = std::time::Instant::now();
    let result = run_supervised(cmd, timeout_secs, Some(&log_path)).await?;
    let duration = start.elapsed().as_secs_f64();

    let (status, exit_code) = if result.timed_out {
        ("TimedOut".to_string(), None)
    } else if result.exit_code == Some(0) {
        ("Succeeded".to_string(), result.exit_code)
    } else {
        ("Failed".to_string(), result.exit_code)
    };

    let stderr_excerpt = if status != "Succeeded" {
        let s = String::from_utf8_lossy(&result.stderr);
        let bytes = s.as_bytes();
        let start = bytes.len().saturating_sub(2048);
        let start = s.ceil_char_boundary(start);
        Some(s[start..].to_string())
    } else {
        None
    };

    Ok(PodOutput {
        run_id,
        action,
        working_dir: working_dir.to_string_lossy().into_owned(),
        status,
        exit_code,
        duration_secs: duration,
        log_path: log_path.to_string_lossy().into_owned(),
        stderr_excerpt,
    })
}

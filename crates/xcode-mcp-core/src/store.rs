use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BuildStatus {
    Succeeded,
    Failed,
    TimedOut,
    Canceled,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct BuildRecord {
    pub build_id: String,
    pub status: BuildStatus,
    pub exit_code: Option<i32>,
    pub duration_secs: f64,
    pub project_or_workspace: PathBuf,
    pub scheme: String,
    pub xcresult_path: PathBuf,
    pub log_path: PathBuf,
    pub result_bundle_written: bool,
    pub error_count: u32,
    pub warning_count: u32,
    pub stderr_excerpt: Option<String>,
    pub created_at: SystemTime,
}

pub struct BuildStore {
    records: Mutex<VecDeque<BuildRecord>>,
    cap: usize,
}

impl BuildStore {
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            records: Mutex::new(VecDeque::with_capacity(cap)),
            cap,
        }
    }

    pub fn push(&self, record: BuildRecord) {
        let mut records = self.records.lock().unwrap();
        if records.len() >= self.cap {
            if let Some(evicted) = records.pop_front() {
                cleanup_build_artifacts(&evicted);
            }
        }
        records.push_back(record);
    }

    pub fn most_recent(&self) -> Option<BuildRecord> {
        self.records.lock().unwrap().back().cloned()
    }

    pub fn get(&self, build_id: &str) -> Option<BuildRecord> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.build_id == build_id)
            .cloned()
    }
}

/// Best-effort cleanup of on-disk artifacts for an evicted build record.
/// Removes the `.xcresult` bundle and `.log` file. Errors are logged via
/// `tracing` and never propagated — the store eviction must not fail.
fn cleanup_build_artifacts(record: &BuildRecord) {
    if record.xcresult_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(&record.xcresult_path) {
            tracing::warn!(
                "failed to remove evicted xcresult {}: {e}",
                record.xcresult_path.display()
            );
        }
    }
    if record.log_path.exists() {
        if let Err(e) = std::fs::remove_file(&record.log_path) {
            tracing::warn!(
                "failed to remove evicted log {}: {e}",
                record.log_path.display()
            );
        }
    }
}

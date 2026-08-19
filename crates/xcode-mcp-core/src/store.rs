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
            records.pop_front();
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

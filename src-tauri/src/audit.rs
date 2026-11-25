use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;

use crate::models::AuditEntry;

const MAX_RECENT: usize = 500;

/// Appends one JSON object per line to a daily file under `<app_data>/audit`
/// and keeps a bounded in-memory copy for the GUI.
pub struct AuditLogger {
    dir: PathBuf,
    recent: Mutex<VecDeque<AuditEntry>>,
}

impl AuditLogger {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("audit");
        let _ = fs::create_dir_all(&dir);
        Self {
            dir,
            recent: Mutex::new(VecDeque::new()),
        }
    }

    pub fn log(&self, mut entry: AuditEntry) -> Result<AuditEntry, String> {
        if entry.id.is_empty() {
            entry.id = uuid::Uuid::new_v4().to_string();
        }
        if entry.timestamp.is_empty() {
            entry.timestamp = Utc::now().to_rfc3339();
        }

        let line =
            serde_json::to_string(&entry).map_err(|error| format!("序列化审计记录失败: {error}"))?;
        let path = self
            .dir
            .join(format!("audit-{}.jsonl", Utc::now().format("%Y-%m-%d")));
        if let Err(error) = fs::create_dir_all(&self.dir) {
            return Err(format!("无法创建审计目录: {error}"));
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("无法打开审计日志: {error}"))?;
        writeln!(file, "{line}").map_err(|error| format!("写入审计日志失败: {error}"))?;

        if let Ok(mut recent) = self.recent.lock() {
            recent.push_front(entry.clone());
            while recent.len() > MAX_RECENT {
                recent.pop_back();
            }
        }

        Ok(entry)
    }

    pub fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        let limit = limit.clamp(1, MAX_RECENT);
        self.recent
            .lock()
            .map(|guard| guard.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }
}

//! A small persisted record of files `enforce_storage_limit` has auto-deleted —
//! surfaced in Settings -> Storage so the storage-limit/auto-delete behavior
//! (otherwise silent besides a log line) is actually verifiable from the UI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeleteReason {
    StorageLimit,
    FolderAge { folder: String, days: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletionLogEntry {
    pub name: String,
    pub size: u64,
    pub reason: DeleteReason,
    pub deleted_at: i64,
}

/// Newest-first; capped so a machine that's been auto-cleaning for years
/// doesn't grow this file forever.
const MAX_ENTRIES: usize = 200;

pub struct DeletionLogStore {
    path: PathBuf,
    entries: Mutex<Vec<DeletionLogEntry>>,
}

impl DeletionLogStore {
    pub fn load(config_dir: PathBuf) -> Self {
        let path = config_dir.join("deletion_log.json");
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, entries: Mutex::new(entries) }
    }

    pub fn get_all(&self) -> Vec<DeletionLogEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn record(&self, name: String, size: u64, reason: DeleteReason) {
        let json = {
            let mut entries = self.entries.lock().unwrap();
            entries.insert(0, DeletionLogEntry { name, size, reason, deleted_at: chrono::Utc::now().timestamp() });
            entries.truncate(MAX_ENTRIES);
            serde_json::to_string_pretty(&*entries).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }

    pub fn clear(&self) {
        *self.entries.lock().unwrap() = Vec::new();
        let _ = std::fs::write(&self.path, "[]");
    }
}

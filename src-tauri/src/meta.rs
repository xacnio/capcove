use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Metadata collected at the moment a recording is started.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Capture time (Unix seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    /// "clip" for replay-buffer saves and editor exports; absent = a full
    /// recording ("video"). Drives the gallery's Videos/Clips filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Human-readable capture stats for YouTube live entries
    /// (e.g. "1080p · 60 FPS · 12 Mbps"), shown on the gallery card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_info: Option<String>,
    /// Session length for YouTube live entries, stamped at stop — the card
    /// shows this without needing a YouTube API round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    /// Set after a successful "Upload to YouTube" — the gallery card links
    /// straight to the uploaded video instead of just offering to upload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_video_id: Option<String>,
    /// Tag IDs assigned to this recording
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// User-starred — never touched by auto-cleanup when
    /// `Settings::keep_favorites` is on (see `video_thumb::enforce_storage_limit`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl VideoMeta {
    /// True for entries with no real local file — a YouTube live session, or
    /// a recording whose local copy was deleted after upload (link-only card).
    pub fn is_virtual(&self) -> bool {
        matches!(self.kind.as_deref(), Some("youtube_live") | Some("youtube_only"))
    }
}

/// Filename -> metadata map; persisted in `metadata.json`.
pub struct MetaStore {
    path: PathBuf,
    map: Mutex<HashMap<String, VideoMeta>>,
}

impl MetaStore {
    pub fn load(config_dir: PathBuf) -> Self {
        let path = config_dir.join("metadata.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            map: Mutex::new(map),
        }
    }

    pub fn get(&self, name: &str) -> Option<VideoMeta> {
        self.map.lock().unwrap().get(name).cloned()
    }

    pub fn set(&self, name: String, meta: VideoMeta) {
        let json = {
            let mut map = self.map.lock().unwrap();
            map.insert(name, meta);
            serde_json::to_string_pretty(&*map).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }

    pub fn set_batch(&self, updates: Vec<(String, VideoMeta)>) {
        let json = {
            let mut map = self.map.lock().unwrap();
            for (name, meta) in updates {
                map.insert(name, meta);
            }
            serde_json::to_string_pretty(&*map).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }

    pub fn remove(&self, name: &str) {
        let json = {
            let mut map = self.map.lock().unwrap();
            map.remove(name);
            serde_json::to_string_pretty(&*map).unwrap_or_default()
        };
        let _ = std::fs::write(&self.path, json);
    }

    pub fn get_all(&self) -> HashMap<String, VideoMeta> {
        self.map.lock().unwrap().clone()
    }

    /// Moves every entry keyed `<old_prefix>/...` to the same suffix under
    /// `<new_prefix>`, keeping metadata in sync when a recording folder is renamed.
    pub fn rekey_prefix(&self, old_prefix: &str, new_prefix: &str) {
        let old_lead = format!("{old_prefix}/");
        let json = {
            let mut map = self.map.lock().unwrap();
            let affected: Vec<String> = map.keys().filter(|k| k.starts_with(&old_lead)).cloned().collect();
            for old_key in affected {
                if let Some(v) = map.remove(&old_key) {
                    map.insert(format!("{new_prefix}/{}", &old_key[old_lead.len()..]), v);
                }
            }
            serde_json::to_string_pretty(&*map).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }

    pub fn overwrite(&self, new_map: HashMap<String, VideoMeta>) {
        let json = {
            let mut map = self.map.lock().unwrap();
            *map = new_map;
            serde_json::to_string_pretty(&*map).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
}

pub struct TagStore {
    path: PathBuf,
    tags: Mutex<Vec<Tag>>,
}

impl TagStore {
    pub fn load(config_dir: PathBuf) -> Self {
        let path = config_dir.join("tags.json");
        let tags = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, tags: Mutex::new(tags) }
    }

    pub fn get_all(&self) -> Vec<Tag> {
        self.tags.lock().unwrap().clone()
    }

    pub fn save(&self, tags: Vec<Tag>) {
        let json = {
            let mut store = self.tags.lock().unwrap();
            *store = tags;
            serde_json::to_string_pretty(&*store).unwrap_or_default()
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }
}

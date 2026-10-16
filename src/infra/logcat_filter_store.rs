//! Logcat's filter history and favorites kept across sessions, as Android
//! Studio keeps them: `logcat-filters.json` in the settings folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::logcat_filter::History;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    favorites: Vec<String>,
    #[serde(default)]
    history: Vec<String>,
}

fn store_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("logcat-filters.json"))
}

pub fn load() -> History {
    store_path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(history: &History) {
    if let Some(path) = store_path() {
        save_to(&path, history);
    }
}

fn load_from(path: &Path) -> History {
    let saved: Saved = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    History { favorites: saved.favorites, recent: saved.history }
}

fn save_to(path: &Path, history: &History) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let saved = Saved { favorites: history.favorites.clone(), history: history.recent.clone() };
    if let Ok(json) = serde_json::to_vec_pretty(&saved) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("junction-logcat-filters-{}/logcat-filters.json", std::process::id()));
        assert_eq!(load_from(&path), History::default());
        let history = History { favorites: vec!["level:error".into()], recent: vec!["package:mine".into(), "tag:OkHttp".into()] };
        save_to(&path, &history);
        assert_eq!(load_from(&path), history);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

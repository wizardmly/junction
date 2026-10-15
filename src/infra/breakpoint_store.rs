//! Line breakpoints kept across sessions, as IntelliJ keeps them in the
//! workspace: `breakpoints.json` in the settings folder, each a file's
//! absolute path and a 0-based line.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedBreakpoint {
    pub file: String,
    /// 0-based.
    pub line: u32,
}

fn store_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("breakpoints.json"))
}

pub fn load() -> Vec<SavedBreakpoint> {
    store_path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(list: &[SavedBreakpoint]) {
    if let Some(path) = store_path() {
        save_to(&path, list);
    }
}

fn load_from(path: &Path) -> Vec<SavedBreakpoint> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_to(path: &Path, list: &[SavedBreakpoint]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("junction-breakpoints-{}/breakpoints.json", std::process::id()));
        assert!(load_from(&path).is_empty());
        let list = vec![SavedBreakpoint { file: "/p/A.kt".into(), line: 4 }];
        save_to(&path, &list);
        assert_eq!(load_from(&path), list);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

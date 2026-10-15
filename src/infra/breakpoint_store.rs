//! Line breakpoints and watches kept across sessions, as IntelliJ keeps
//! them in the workspace: `breakpoints.json` in the settings folder, each a
//! file's absolute path, a 0-based line, whether it's enabled and its
//! condition; `watches.json`, the Watches' expressions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedBreakpoint {
    pub file: String,
    /// 0-based.
    pub line: u32,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

impl SavedBreakpoint {
    pub fn new(file: &str, line: u32) -> Self {
        Self { file: file.to_owned(), line, enabled: true, condition: None }
    }
}

fn enabled() -> bool {
    true
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

fn watches_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("watches.json"))
}

pub fn load_watches() -> Vec<String> {
    watches_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_watches(list: &[String]) {
    let Some(path) = watches_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(path, json);
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
        let list = vec![SavedBreakpoint::new("/p/A.kt", 4), SavedBreakpoint { condition: Some("x > 1".into()), enabled: false, ..SavedBreakpoint::new("/p/B.kt", 2) }];
        save_to(&path, &list);
        assert_eq!(load_from(&path), list);
        // Saved before conditions existed: enabled, unconditional.
        std::fs::write(&path, r#"[{"file":"/p/C.kt","line":1}]"#).unwrap();
        assert_eq!(load_from(&path), vec![SavedBreakpoint::new("/p/C.kt", 1)]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

//! A project's editor session, as IntelliJ keeps it in the workspace: the
//! open tabs with their carets, the active one, and Recent Locations'
//! places; in `editor-sessions.json` in the settings folder, by project
//! root. Read once, kept in memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTab {
    pub path: String,
    pub line: u32,
    pub col: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPlace {
    pub path: String,
    pub line: u32,
    pub col: u32,
    #[serde(default)]
    pub changed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorSession {
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub places: Vec<SessionPlace>,
}

type Store = BTreeMap<String, EditorSession>;

static CACHE: LazyLock<Mutex<Option<Store>>> = LazyLock::new(Mutex::default);

fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("editor-sessions.json"))
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut cache = CACHE.lock().unwrap();
    let store = cache.get_or_insert_with(|| path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    f(store)
}

pub fn load(root: &Path) -> EditorSession {
    with_store(|s| s.get(&root.to_string_lossy().into_owned()).cloned().unwrap_or_default())
}

pub fn save(root: &Path, session: &EditorSession) {
    let json = with_store(|s| {
        let key = root.to_string_lossy().into_owned();
        if s.get(&key) == Some(session) {
            return None;
        }
        s.insert(key, session.clone());
        serde_json::to_vec_pretty(s).ok()
    });
    let (Some(path), Some(json)) = (path(), json) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_old_and_partial_sessions() {
        let read: EditorSession = serde_json::from_str(r#"{"tabs":[{"path":"a.kt","line":3,"col":1}]}"#).unwrap();
        assert_eq!(read.tabs[0], SessionTab { path: "a.kt".into(), line: 3, col: 1, pinned: false });
        assert!(read.active.is_none() && read.places.is_empty());
        let session = EditorSession { active: Some("a.kt".into()), places: vec![SessionPlace { path: "b.kt".into(), line: 1, col: 0, changed: true }], ..read };
        let back: EditorSession = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(back, session);
    }
}

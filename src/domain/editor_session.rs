//! A project's editor session, as IntelliJ keeps it in the workspace: the
//! open tabs with their carets, the active one, and Recent Locations'
//! places, restored when the project opens again.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::navigation_history::Place;

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
pub struct EditorSession {
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub places: Vec<Place>,
    /// Logcat's tabs and panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logcat: Option<super::logcat_layout::LogcatLayout>,
    /// The open tool windows, by key (none saved: the defaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_windows: Option<Vec<String>>,
}

impl EditorSession {
    /// The tabs to reopen: the ones whose files still exist, and the active
    /// one among them.
    pub fn reopenable(&self, exists: impl Fn(&str) -> bool) -> (Vec<SessionTab>, Option<SessionTab>) {
        let tabs: Vec<SessionTab> = self.tabs.iter().filter(|t| exists(&t.path)).cloned().collect();
        let active = self.active.as_ref().and_then(|a| tabs.iter().find(|t| t.path == *a).cloned());
        (tabs, active)
    }
}

/// Where sessions are kept, by project root.
pub trait SessionStore {
    fn load(&self, root: &Path) -> EditorSession;
    fn save(&self, root: &Path, session: &EditorSession);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_old_and_partial_sessions() {
        let read: EditorSession = serde_json::from_str(r#"{"tabs":[{"path":"a.kt","line":3,"col":1}]}"#).unwrap();
        assert_eq!(read.tabs[0], SessionTab { path: "a.kt".into(), line: 3, col: 1, pinned: false });
        assert!(read.active.is_none() && read.places.is_empty());
        let session = EditorSession { active: Some("a.kt".into()), places: vec![Place { path: "b.kt".into(), line: 1, col: 0, changed: true }], ..read };
        let back: EditorSession = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(back, session);
    }

    #[test]
    fn reopens_existing_files() {
        let tab = |p: &str| SessionTab { path: p.into(), ..SessionTab::default() };
        let session = EditorSession { tabs: vec![tab("a.kt"), tab("gone.kt"), tab("b.kt")], active: Some("gone.kt".into()), places: Vec::new(), ..Default::default() };
        let (tabs, active) = session.reopenable(|p| p != "gone.kt");
        assert_eq!(tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["a.kt", "b.kt"]);
        assert!(active.is_none());
    }
}

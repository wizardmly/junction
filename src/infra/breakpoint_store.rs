//! Line breakpoints and watches kept across sessions, as IntelliJ keeps
//! them in the workspace: `breakpoints.json` in the settings folder, each a
//! file's absolute path, a 0-based line, whether it's enabled and its
//! condition and what a hit does; `watches.json`, the Watches' expressions.

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
    /// Suspend execution; off, a hit only logs.
    #[serde(default = "enabled", skip_serializing_if = "is_true")]
    pub suspend: bool,
    /// "Breakpoint hit" message.
    #[serde(default, skip_serializing_if = "is_false")]
    pub log_message: bool,
    /// Stack trace.
    #[serde(default, skip_serializing_if = "is_false")]
    pub log_stack: bool,
    /// Evaluate and log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_expression: Option<String>,
    /// A method breakpoint: the method declared on the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

impl SavedBreakpoint {
    pub fn new(file: &str, line: u32) -> Self {
        Self { file: file.to_owned(), line, enabled: true, condition: None, suspend: true, log_message: false, log_stack: false, log_expression: None, method: None }
    }
}

fn enabled() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
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
        let logging = SavedBreakpoint { suspend: false, log_message: true, log_expression: Some("x".into()), ..SavedBreakpoint::new("/p/D.kt", 3) };
        let list = vec![SavedBreakpoint::new("/p/A.kt", 4), SavedBreakpoint { condition: Some("x > 1".into()), enabled: false, ..SavedBreakpoint::new("/p/B.kt", 2) }, logging];
        save_to(&path, &list);
        assert_eq!(load_from(&path), list);
        // Saved before conditions existed: enabled, unconditional.
        std::fs::write(&path, r#"[{"file":"/p/C.kt","line":1}]"#).unwrap();
        assert_eq!(load_from(&path), vec![SavedBreakpoint::new("/p/C.kt", 1)]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn any_exception_comes_first() {
        let mut list = vec![SavedExceptionBreakpoint::new("java.io.IOException")];
        with_any(&mut list);
        assert_eq!(list[0], SavedExceptionBreakpoint::any());
        list.swap(0, 1);
        with_any(&mut list);
        assert_eq!((list[0].class.as_str(), list[1].class.as_str()), ("", "java.io.IOException"));
        let read: SavedExceptionBreakpoint = serde_json::from_str(r#"{"class":"a.B"}"#).unwrap();
        assert!(!read.enabled && read.caught && read.uncaught);
    }
}

/// A Java Exception Breakpoint, as IntelliJ keeps it: the class (empty
/// for "Any exception"), whether it's on, and which throws it stops at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedExceptionBreakpoint {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "enabled")]
    pub caught: bool,
    #[serde(default = "enabled")]
    pub uncaught: bool,
}

impl SavedExceptionBreakpoint {
    /// IntelliJ's "Any exception": there, off, caught and uncaught.
    pub fn any() -> Self {
        Self { class: String::new(), enabled: false, caught: true, uncaught: true }
    }

    pub fn new(class: &str) -> Self {
        Self { class: class.to_owned(), enabled: true, caught: true, uncaught: true }
    }
}

fn exceptions_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("exception-breakpoints.json"))
}

/// The exception breakpoints, "Any exception" first.
pub fn load_exceptions() -> Vec<SavedExceptionBreakpoint> {
    let mut list: Vec<SavedExceptionBreakpoint> = exceptions_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    with_any(&mut list);
    list
}

fn with_any(list: &mut Vec<SavedExceptionBreakpoint>) {
    match list.iter().position(|e| e.class.is_empty()) {
        Some(0) => {}
        Some(ix) => {
            let any = list.remove(ix);
            list.insert(0, any);
        }
        None => list.insert(0, SavedExceptionBreakpoint::any()),
    }
}

pub fn save_exceptions(list: &[SavedExceptionBreakpoint]) {
    let Some(path) = exceptions_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(path, json);
    }
}

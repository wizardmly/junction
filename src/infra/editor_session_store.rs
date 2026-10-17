//! Editor sessions in `editor-sessions.json` in the settings folder, by
//! project root. Read once, kept in memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use crate::domain::editor_session::{EditorSession, SessionStore};

type Store = BTreeMap<String, EditorSession>;

static CACHE: LazyLock<Mutex<Option<Store>>> = LazyLock::new(Mutex::default);

/// The settings folder's sessions file.
pub struct JsonSessions;

fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("editor-sessions.json"))
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut cache = CACHE.lock().unwrap();
    let store = cache.get_or_insert_with(|| path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    f(store)
}

impl SessionStore for JsonSessions {
    fn load(&self, root: &Path) -> EditorSession {
        with_store(|s| s.get(&root.to_string_lossy().into_owned()).cloned().unwrap_or_default())
    }

    fn save(&self, root: &Path, session: &EditorSession) {
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
}

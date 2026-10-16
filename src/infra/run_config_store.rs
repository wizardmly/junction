//! The Run / Debug Configurations the user edited, by build root, in
//! `run-configurations.json` (Android Studio keeps them in the project's
//! workspace; here they stay out of it). Read once, kept in memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use crate::domain::run::SavedRunConfiguration;

type Store = BTreeMap<String, Vec<SavedRunConfiguration>>;

static CACHE: LazyLock<Mutex<Option<Store>>> = LazyLock::new(Mutex::default);

fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("run-configurations.json"))
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut cache = CACHE.lock().unwrap();
    let store = cache.get_or_insert_with(|| path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    f(store)
}

/// A build's saved configurations; empty when it uses the defaults.
pub fn load(build_root: &Path) -> Vec<SavedRunConfiguration> {
    with_store(|s| s.get(&build_root.to_string_lossy().into_owned()).cloned().unwrap_or_default())
}

pub fn save(build_root: &Path, list: &[SavedRunConfiguration]) {
    let json = with_store(|s| {
        s.insert(build_root.to_string_lossy().into_owned(), list.to_vec());
        serde_json::to_vec_pretty(s).ok()
    });
    let (Some(path), Some(json)) = (path(), json) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}

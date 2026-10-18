//! Generate Signed Bundle / APK's keystore paths and key aliases in
//! `signing.json` in the settings folder, by module folder. Passwords are
//! never written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::domain::signing::{RememberedKey, SigningStore};

/// The settings folder's signing file.
pub struct JsonSigning;

fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("signing.json"))
}

fn read() -> BTreeMap<String, RememberedKey> {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

impl SigningStore for JsonSigning {
    fn load(&self, module_dir: &Path) -> Option<RememberedKey> {
        read().remove(&module_dir.to_string_lossy().into_owned())
    }

    fn save(&self, module_dir: &Path, key: &RememberedKey) {
        let Some(path) = path() else { return };
        let mut all = read();
        all.insert(module_dir.to_string_lossy().into_owned(), key.clone());
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&all) {
            let _ = std::fs::write(path, json);
        }
    }
}

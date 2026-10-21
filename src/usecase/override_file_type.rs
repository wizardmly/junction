//! Override File Type / Revert File Type Override.

use crate::domain::file_type_overrides::FileTypeStore;

/// Overrides `file` (an absolute path) to `language`, or reverts it;
/// whether anything changed.
pub fn set_file_type(store: &dyn FileTypeStore, file: &str, language: Option<&str>) -> Result<bool, String> {
    let mut overrides = store.load();
    let changed = overrides.set(file, language);
    if changed {
        store.save(&overrides)?;
    }
    Ok(changed)
}

/// A file or folder moved (absolute paths): its overrides follow it.
pub fn file_moved(store: &dyn FileTypeStore, from: &str, to: &str) -> Result<(), String> {
    let mut overrides = store.load();
    if overrides.moved(from, to) {
        store.save(&overrides)?;
    }
    Ok(())
}

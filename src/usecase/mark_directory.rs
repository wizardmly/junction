//! Mark Directory As › Excluded / Cancel Exclusion.

use crate::domain::excluded_folders::ExcludedStore;

/// Marks `dir` Excluded, or cancels its exclusion; whether anything changed.
pub fn set_excluded(store: &dyn ExcludedStore, dir: &str, excluded: bool) -> Result<bool, String> {
    let mut folders = store.load();
    let changed = if excluded { folders.exclude(dir) } else { folders.cancel(dir) };
    if changed {
        store.save(&folders)?;
    }
    Ok(changed)
}

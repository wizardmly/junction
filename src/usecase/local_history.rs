//! Local History: records each saved text of a file, labels, and gives a
//! file's history back with its texts.

use crate::domain::local_history::{self as lh, HistoryStore, Kind, Revision};

fn now() -> i64 {
    chrono::Local::now().timestamp_millis()
}

/// Records `text` as `path`'s newest version, unless it is the same as the last one.
pub fn record(store: &dyn HistoryStore, path: &str, text: &str, kind: Kind, label: &str) -> Result<bool, String> {
    if text.len() > lh::MAX_FILE_BYTES || text.contains('\0') {
        return Ok(false);
    }
    let hash = store.put_content(text)?;
    let last = store.revisions().into_iter().filter(|r| r.path == path).max_by_key(|r| r.time);
    if last.is_some_and(|r| r.content == hash) {
        return Ok(false);
    }
    store.append(&Revision { time: now(), kind, path: path.to_owned(), content: hash, label: label.to_owned() })?;
    Ok(true)
}

/// Whether the file has any recorded version.
pub fn has_history(store: &dyn HistoryStore, path: &str) -> bool {
    store.revisions().iter().any(|r| r.path == path)
}

/// Put Label: a name on the whole project at this moment.
pub fn put_label(store: &dyn HistoryStore, label: &str) -> Result<(), String> {
    store.append(&Revision { time: now(), kind: Kind::Label, path: String::new(), content: String::new(), label: label.to_owned() })
}

/// Drops what is older than the last working days kept.
pub fn prune(store: &dyn HistoryStore) -> Result<(), String> {
    let all = store.revisions();
    let day = |t: i64| chrono::DateTime::from_timestamp_millis(t).map(|d| d.with_timezone(&chrono::Local).date_naive().to_epoch_days() as i64).unwrap_or(0);
    let kept = lh::keep_working_days(&all, lh::WORKING_DAYS_TO_KEEP, day);
    if kept.len() != all.len() { store.rewrite(&kept) } else { Ok(()) }
}

pub fn history(store: &dyn HistoryStore, path: &str) -> Vec<Revision> {
    lh::file_history(&store.revisions(), path)
}

/// A folder's history (`dir` empty: the project's), newest first.
pub fn folder_history(store: &dyn HistoryStore, dir: &str) -> Vec<Revision> {
    lh::folder_history(&store.revisions(), dir)
}

/// The folder's files changed since `time`, with their text then.
pub fn changed_since(store: &dyn HistoryStore, dir: &str, time: i64) -> Vec<(String, String)> {
    lh::changed_since(&store.revisions(), dir, time).into_iter().filter_map(|(path, hash)| Some((path, store.content(&hash)?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::local_history_store::DiskHistory;

    #[test]
    fn records_changes_only() {
        let dir = std::env::temp_dir().join(format!("junction-lh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = DiskHistory::at(dir.clone());
        assert!(record(&store, "a.kt", "one", Kind::Saved, "").unwrap());
        assert!(!record(&store, "a.kt", "one", Kind::Saved, "").unwrap());
        std::thread::sleep(std::time::Duration::from_millis(5));
        put_label(&store, "before refactor").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(record(&store, "a.kt", "two", Kind::Saved, "").unwrap());
        let h = history(&store, "a.kt");
        assert_eq!(h.len(), 3);
        assert_eq!(store.content(&h[0].content).as_deref(), Some("two"));
        assert_eq!(h[1].label, "before refactor");
        prune(&store).unwrap();
        assert_eq!(history(&store, "a.kt").len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

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

/// After something outside the editor changed files (a Git operation):
/// each file with a history whose text on disk (`read`) is not its last
/// version gets an "External change" version, as IntelliJ records VFS
/// changes. The files recorded.
pub fn record_external(store: &dyn HistoryStore, read: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut last: std::collections::HashMap<String, &Revision> = std::collections::HashMap::new();
    let all = store.revisions();
    for r in all.iter().filter(|r| r.kind != Kind::Label) {
        if last.get(&r.path).is_none_or(|l| r.time >= l.time) {
            last.insert(r.path.clone(), r);
        }
    }
    let mut paths: Vec<&String> = last.keys().collect();
    paths.sort();
    paths
        .into_iter()
        .filter(|path| read(path).is_some_and(|text| record(store, path, &text, Kind::External, "External change").unwrap_or(false)))
        .cloned()
        .collect()
}

/// Show History for Selection: the versions in which the lines
/// `start..end` of `current` (the editor's text) were different from the
/// next newer version, newest first, each with those lines as they were
/// then; and the lines now.
pub fn selection_history(store: &dyn HistoryStore, path: &str, current: &str, start: usize, end: usize) -> (Vec<(Revision, String)>, String) {
    let now = lh::block_text(current, start, end);
    let mut out = Vec::new();
    let (mut newer_text, mut range, mut newer_block) = (current.to_owned(), (start, end), now.clone());
    for r in history(store, path).into_iter().filter(|r| r.kind != Kind::Label) {
        let Some(text) = store.content(&r.content) else { break };
        range = lh::previous_block(&newer_text, range.0, range.1, &text);
        let block = lh::block_text(&text, range.0, range.1);
        if block != newer_block {
            out.push((r, block.clone()));
        }
        newer_text = text;
        newer_block = block;
    }
    (out, now)
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

        // Changed by git: an external change; unchanged files get none.
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(record(&store, "b.kt", "same", Kind::Saved, "").unwrap());
        let read = |p: &str| Some(if p == "a.kt" { "three".to_owned() } else { "same".to_owned() });
        assert_eq!(record_external(&store, &read), vec!["a.kt".to_owned()]);
        assert!(record_external(&store, &read).is_empty());
        assert_eq!(history(&store, "a.kt")[0].kind, Kind::External);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selection_history_skips_versions_that_kept_the_lines() {
        let dir = std::env::temp_dir().join(format!("junction-lh-sel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = DiskHistory::at(dir.clone());
        let step = |text: &str| {
            std::thread::sleep(std::time::Duration::from_millis(3));
            record(&store, "a.kt", text, Kind::Saved, "").unwrap();
        };
        step("fun a() {\n  one()\n}\nfun b() {}\n");
        step("fun a() {\n  two()\n}\nfun b() {}\n");
        // Only b changed: no new row for a's lines.
        step("fun a() {\n  two()\n}\nfun b() { x() }\n");
        let current = "// header\nfun a() {\n  three()\n}\nfun b() { x() }\n";
        let (rows, now) = selection_history(&store, "a.kt", current, 1, 4);
        assert_eq!(now, "fun a() {\n  three()\n}\n");
        let blocks: Vec<&str> = rows.iter().map(|(_, b)| b.as_str()).collect();
        assert_eq!(blocks, ["fun a() {\n  two()\n}\n", "fun a() {\n  one()\n}\n"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

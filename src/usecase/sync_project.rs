//! Sync Project with Gradle Files, lightweight: when to sync a build, and
//! what a sync leaves. Android Studio syncs at every project open; Junction
//! offers it then (only when the build files changed since the last sync)
//! and runs it by itself only when they change while the project is open,
//! as after a branch switch or a pull.

use std::path::Path;

use crate::domain::sync::{ProjectSync, SyncRecord, SyncReport};

/// Why a build is looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// First seen this session: the project was opened.
    Opened,
    /// Its build files were seen before, with this stamp.
    Seen(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Nothing,
    /// "Gradle files have changed since last project sync": Sync Now.
    Offer,
    Sync,
}

pub fn decide(record: Option<&SyncRecord>, stamp: u64, trigger: Trigger) -> Next {
    let synced = record.is_some_and(|r| r.stamp == stamp);
    match trigger {
        Trigger::Opened if synced => Next::Nothing,
        Trigger::Opened => Next::Offer,
        // Unchanged since last seen: a failed sync isn't retried by itself.
        Trigger::Seen(seen) if seen == stamp || synced => Next::Nothing,
        Trigger::Seen(_) => Next::Sync,
    }
}

/// Syncs a build; a successful sync becomes its record (the libraries are
/// then re-indexed from it), a failed one keeps the previous record.
pub fn sync(port: &dyn ProjectSync, root: &Path, build_root: &Path, files: &[String], variants: &[(String, String)], line: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> SyncReport {
    let stamp = port.stamp(root, build_root, files);
    let report = port.sync(build_root, variants, line, cancelled);
    if report.succeeded {
        port.save(build_root, &SyncRecord { stamp, dependencies: report.dependencies.clone(), modules: report.modules.clone() });
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    #[test]
    fn when_to_sync() {
        let record = SyncRecord { stamp: 7, ..Default::default() };
        assert_eq!(decide(Some(&record), 7, Trigger::Opened), Next::Nothing);
        assert_eq!(decide(Some(&record), 8, Trigger::Opened), Next::Offer);
        assert_eq!(decide(None, 8, Trigger::Opened), Next::Offer);
        assert_eq!(decide(Some(&record), 8, Trigger::Seen(7)), Next::Sync);
        assert_eq!(decide(Some(&record), 8, Trigger::Seen(8)), Next::Nothing);
        // Switched back to the synced state.
        assert_eq!(decide(Some(&record), 7, Trigger::Seen(8)), Next::Nothing);
    }

    struct Fake {
        ok: bool,
        saved: RefCell<Option<SyncRecord>>,
    }
    impl ProjectSync for Fake {
        fn build_roots(&self, root: &Path, _: &[String]) -> Vec<PathBuf> {
            vec![root.to_path_buf()]
        }
        fn stamp(&self, _: &Path, _: &Path, _: &[String]) -> u64 {
            3
        }
        fn sync(&self, _: &Path, _: &[(String, String)], _: &mut dyn FnMut(&str), _: &dyn Fn() -> bool) -> SyncReport {
            let dependencies = vec![crate::domain::sync::Coordinate::parse("a.b:c:1").unwrap()];
            SyncReport { succeeded: self.ok, dependencies, ..Default::default() }
        }
        fn load(&self, _: &Path) -> Option<SyncRecord> {
            self.saved.borrow().clone()
        }
        fn save(&self, _: &Path, record: &SyncRecord) {
            *self.saved.borrow_mut() = Some(record.clone());
        }
    }

    #[test]
    fn only_a_successful_sync_is_kept() {
        let root = Path::new("/p");
        let failed = Fake { ok: false, saved: RefCell::new(None) };
        sync(&failed, root, root, &[], &[], &mut |_| {}, &|| false);
        assert!(failed.load(root).is_none());
        let ok = Fake { ok: true, saved: RefCell::new(None) };
        sync(&ok, root, root, &[], &[], &mut |_| {}, &|| false);
        assert_eq!(ok.load(root).unwrap().stamp, 3);
        assert_eq!(ok.load(root).unwrap().dependencies.len(), 1);
    }
}

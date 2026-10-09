//! Moving between the files of one change set, as IntelliJ's diff window
//! does: Compare Previous / Next File (Alt+Left / Alt+Right), the file
//! list, and F7 past the last change asking once, then going on.

use gpui_kit::{AppContext as _, Context};

use super::{DiffSource, DiffView};
use crate::git::{Repository, log};

/// Where F7 / Shift+F7 stopped at the end of a file: pressing it again
/// moves to the next / previous file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Next,
    Previous,
}

/// The change set a source belongs to (its files share it).
fn set_key(source: &DiffSource) -> String {
    match source {
        DiffSource::Commit { hash, .. } => format!("commit {hash}"),
        DiffSource::WorkingTree { .. } => "local".into(),
        DiffSource::Staged { .. } => "staged".into(),
        DiffSource::Unstaged { .. } => "unstaged".into(),
        DiffSource::Between { old, new, .. } => format!("between {old} {new:?}"),
        DiffSource::Files { path, other } => format!("files {path} {}", other.display()),
        DiffSource::Texts { path, old_title, new_title, .. } => format!("texts {path} {old_title} {new_title}"),
        DiffSource::Clipboard { path, .. } => format!("clipboard {path}"),
    }
}

/// The files of a source's change set, as sources of the same kind.
/// `keep` adds the source when git doesn't list it (an unversioned file).
fn list_files(repository: &Repository, source: &DiffSource, keep: bool) -> Vec<DiffSource> {
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let name_status = |args: &[&str]| repository.run(args).map(|out| log::parse_name_status(&out)).unwrap_or_default();
    let changes = match source {
        DiffSource::Commit { hash, .. } => {
            let parent = format!("{hash}^");
            let old = if repository.run(["rev-parse", "--verify", "-q", &parent]).is_ok() { parent } else { EMPTY_TREE.to_owned() };
            name_status(&["diff", "--name-status", "-z", "-M", &old, hash, "--"])
        }
        DiffSource::WorkingTree { .. } => name_status(&["diff", "--name-status", "-z", "-M", "HEAD", "--"]),
        DiffSource::Staged { .. } => name_status(&["diff", "--cached", "--name-status", "-z", "-M", "--"]),
        DiffSource::Unstaged { .. } => name_status(&["diff", "--name-status", "-z", "--"]),
        DiffSource::Between { old, new, .. } => match new {
            Some(new) => name_status(&["diff", "--name-status", "-z", "-M", old, new, "--"]),
            None => name_status(&["diff", "--name-status", "-z", "-M", old, "--"]),
        },
        DiffSource::Files { .. } | DiffSource::Texts { .. } | DiffSource::Clipboard { .. } => return vec![source.clone()],
    };
    let mut files: Vec<DiffSource> = changes
        .into_iter()
        .map(|c| match source {
            DiffSource::Commit { hash, .. } => DiffSource::Commit { hash: hash.clone(), path: c.path, old_path: c.old_path },
            DiffSource::WorkingTree { .. } => DiffSource::WorkingTree { path: c.path, unversioned: false },
            DiffSource::Staged { .. } => DiffSource::Staged { path: c.path },
            DiffSource::Unstaged { .. } => DiffSource::Unstaged { path: c.path },
            DiffSource::Between { old, new, .. } => DiffSource::Between { old: old.clone(), new: new.clone(), path: c.path, old_path: c.old_path },
            DiffSource::Files { .. } | DiffSource::Texts { .. } | DiffSource::Clipboard { .. } => source.clone(),
        })
        .collect();
    // An unversioned file opened from the Commit tool window is in the set too.
    if keep && !files.iter().any(|f| f.path() == source.path()) {
        files.push(source.clone());
        files.sort_by(|a, b| a.path().cmp(b.path()));
    }
    files
}

impl DiffView {
    /// Loads the file list when the shown source is from a new change set.
    pub(super) fn load_files(&mut self, repository: &Repository, source: &DiffSource, cx: &mut Context<Self>) {
        let key = set_key(source);
        if self.files_key == key && self.files.iter().any(|f| f.path() == source.path()) {
            return;
        }
        self.files_key = key;
        self.files = vec![source.clone()];
        // After a refresh, a file that is no longer changed leaves the set
        // (unless it's unversioned) and the view moves on to its neighbor,
        // so "N of M" counts what is left.
        let refresh_index = self.refresh_index.take();
        let keep = refresh_index.is_none() || matches!(source, DiffSource::WorkingTree { unversioned: true, .. });
        let (repository, source) = (repository.clone(), source.clone());
        self._files_task = Some(cx.spawn(async move |this, cx| {
            let files = cx.background_spawn(async move { list_files(&repository, &source, keep) }).await;
            this.update(cx, |this, cx| {
                this.files = files;
                this.apply_file_order();
                if let Some(ix) = refresh_index
                    && this.file_index().is_none()
                    && !this.files.is_empty()
                {
                    let next = this.files[ix.min(this.files.len() - 1)].clone();
                    this.open_file(next, false, cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The Commit tool window's tree order for local changes, so "N of M" and
    /// Compare Next File follow the tree rather than git's path order.
    pub fn set_file_order(&mut self, order: Vec<String>) {
        self.file_order = order;
    }

    fn apply_file_order(&mut self) {
        let local = self.files.first().is_some_and(|f| {
            matches!(f, DiffSource::WorkingTree { .. } | DiffSource::Staged { .. } | DiffSource::Unstaged { .. })
        });
        if !local || self.file_order.is_empty() {
            return;
        }
        let rank = |path: &str| self.file_order.iter().position(|p| p == path).unwrap_or(usize::MAX);
        let mut files = std::mem::take(&mut self.files);
        files.sort_by_key(|f| rank(f.path()));
        self.files = files;
    }

    /// Reloads a working-tree / staged / unstaged diff after its change moved
    /// on (a commit, rollback or stage); other diffs stay as they are.
    pub fn refresh_local(&mut self, cx: &mut Context<Self>) {
        let (Some(repository), Some(source)) = (self.repository.clone(), self.source.clone()) else { return };
        if !matches!(source, DiffSource::WorkingTree { .. } | DiffSource::Staged { .. } | DiffSource::Unstaged { .. }) {
            return;
        }
        self.refresh_index = self.file_index();
        self.source = None;
        self.files_key.clear();
        self.show(repository, source, cx);
    }

    fn file_index(&self) -> Option<usize> {
        let source = self.source.as_ref()?;
        self.files.iter().position(|f| f.path() == source.path())
    }

    pub(super) fn file_position(&self) -> Option<(usize, usize)> {
        Some((self.file_index()?, self.files.len()))
    }

    pub(super) fn neighbor(&self, forward: bool) -> Option<DiffSource> {
        let ix = self.file_index()?;
        let target = if forward { ix + 1 } else { ix.checked_sub(1)? };
        self.files.get(target).cloned()
    }

    /// Shows another file of the set; `at_end` lands on its last change.
    pub(super) fn open_file(&mut self, source: DiffSource, at_end: bool, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else { return };
        self.edge = None;
        self.arrive_at_end = at_end;
        self.show(repository, source, cx);
    }

    pub fn compare_next_file(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.neighbor(true) {
            self.open_file(next, false, cx);
        }
    }

    pub fn compare_previous_file(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.neighbor(false) {
            self.open_file(previous, false, cx);
        }
    }

    /// F7 with no change after the caret: the first press says so, the
    /// second goes to the next file.
    pub(super) fn past_end(&mut self, edge: Edge, cx: &mut Context<Self>) {
        let forward = edge == Edge::Next;
        let Some(target) = self.neighbor(forward) else { return };
        if self.edge == Some(edge) {
            self.open_file(target, !forward, cx);
        } else {
            self.edge = Some(edge);
            cx.notify();
        }
    }
}

//! Patches and the Shelf. IntelliJ writes plain unified diffs (not mail
//! patches), and shelves changes as patch files it can re-apply later.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, bail};

use super::Repository;
use super::log::{FileChange, parse_name_status};

/// The tree git uses for "no parent".
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A throwaway index, so building trees never touches the user's staging area.
pub(crate) struct TempIndex {
    path: PathBuf,
}

impl TempIndex {
    pub(crate) fn new(repository: &Repository) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self { path: repository.git_dir().join(format!("gitglass-index-{}-{n}", std::process::id())) }
    }

    pub(crate) fn env(&self) -> [(&str, &str); 1] {
        [("GIT_INDEX_FILE", self.path.to_str().unwrap_or_default())]
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

fn head(repository: &Repository) -> Option<String> {
    repository.run(["rev-parse", "--verify", "-q", "HEAD"]).ok().map(|s| s.trim().to_owned())
}

/// A tree holding HEAD plus the working-tree state of `paths` (new files
/// included, deleted files removed).
pub fn working_tree(repository: &Repository, paths: &[String]) -> Result<String> {
    let index = TempIndex::new(repository);
    let env = index.env();
    match head(repository) {
        Some(head) => repository.run_with_env(["read-tree", head.as_str()], &env)?,
        None => repository.run_with_env(["read-tree", "--empty"], &env)?,
    };
    let mut args = vec!["add".to_owned(), "-A".to_owned(), "--".to_owned()];
    args.extend(paths.iter().cloned());
    repository.run_with_env(&args, &env)?;
    Ok(repository.run_with_env(["write-tree"], &env)?.trim().to_owned())
}

/// Create Patch… for commits: the combined change from `old` to `new`.
pub fn between(repository: &Repository, old: &str, new: &str, reverse: bool) -> Result<String> {
    let mut args = vec!["diff", "--binary", "--full-index", "--no-color", "--no-ext-diff"];
    if reverse {
        args.push("-R");
    }
    args.extend([old, new]);
    repository.run(args)
}

/// The tree's parent for "Create Patch" over the oldest of several commits.
pub fn parent_of(repository: &Repository, commit: &str) -> String {
    repository
        .run(["rev-parse", "--verify", "-q", &format!("{commit}^")])
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| EMPTY_TREE.to_owned())
}

/// Create Patch… / Copy as Patch for local changes, unversioned files included.
pub fn local_changes(repository: &Repository, paths: &[String], reverse: bool) -> Result<String> {
    let tree = working_tree(repository, paths)?;
    let base = head(repository).unwrap_or_else(|| EMPTY_TREE.to_owned());
    between(repository, &base, &tree, reverse)
}

/// Files a patch touches, for the Apply Patch preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

pub fn files(repository: &Repository, patch: &str) -> Result<Vec<PatchFile>> {
    let output = repository.run_with_input(["apply", "--numstat", "-z", "-"], Some(patch))?;
    Ok(parse_numstat(&output))
}

fn parse_numstat(output: &str) -> Vec<PatchFile> {
    let mut files = Vec::new();
    let mut parts = output.split('\0');
    while let Some(record) = parts.next() {
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
        // Renames leave the path empty and follow with old and new paths.
        let path = if path.is_empty() {
            parts.next();
            parts.next().unwrap_or_default().to_owned()
        } else {
            path.to_owned()
        };
        files.push(PatchFile { path, added: added.parse().ok(), removed: removed.parse().ok() });
    }
    files
}

/// Applies a patch to the working tree. A patch that no longer applies
/// cleanly falls back to a three-way merge, leaving conflicts to resolve.
pub fn apply(repository: &Repository, patch: &str, to_index: bool) -> Result<ApplyOutcome> {
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if to_index {
        args.push("--index");
    }
    args.push("-");
    if repository.run_with_input(&args, Some(patch)).is_ok() {
        return Ok(ApplyOutcome::Clean);
    }
    match repository.run_with_input(["apply", "--3way", "--whitespace=nowarn", "-"], Some(patch)) {
        Ok(_) => Ok(ApplyOutcome::Merged),
        Err(error) if error.to_string().contains("with conflicts") => Ok(ApplyOutcome::Conflicts),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    Clean,
    Merged,
    Conflicts,
}

/// Puts `paths` back to HEAD, deleting files HEAD doesn't have.
pub fn rollback(repository: &Repository, paths: &[String]) -> Result<()> {
    rollback_with(repository, paths, true)
}

/// The Rollback Changes dialog: files HEAD doesn't have are unstaged, and
/// deleted only with "Delete local copies of added files".
pub fn rollback_with(repository: &Repository, paths: &[String], delete_added: bool) -> Result<()> {
    let head = head(repository);
    let mut tracked = Vec::new();
    for path in paths {
        let in_head = head
            .as_ref()
            .is_some_and(|h| repository.run(["cat-file", "-e", &format!("{h}:{path}")]).is_ok());
        if in_head {
            tracked.push(path.clone());
        } else {
            repository.run(["rm", "--cached", "-q", "--ignore-unmatch", "--", path])?;
            let file = repository.root().join(path);
            if delete_added && file.is_file() {
                std::fs::remove_file(&file).with_context(|| format!("failed to delete {path}"))?;
            }
        }
    }
    if !tracked.is_empty() {
        let mut args = vec!["restore".to_owned(), "--staged".into(), "--worktree".into(), "--source=HEAD".into(), "--".into()];
        args.extend(tracked);
        repository.run(&args)?;
    }
    Ok(())
}

// ---- Shelf ----

/// One shelved changelist. Kept as `<git dir>/gitglass/shelf/<id>/` with
/// `shelved.patch` (the IntelliJ-compatible patch) and `info`, plus
/// `refs/gitglass/shelf/<id>` so the base and shelved trees survive gc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shelf {
    pub id: String,
    pub name: String,
    pub time: i64,
    pub base: String,
    pub commit: String,
    pub deleted: bool,
}

impl Shelf {
    pub fn patch_path(&self, repository: &Repository) -> PathBuf {
        shelf_dir(repository).join(&self.id).join("shelved.patch")
    }
}

fn shelf_dir(repository: &Repository) -> PathBuf {
    repository.git_dir().join("gitglass").join("shelf")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn write_info(dir: &Path, shelf: &Shelf) -> Result<()> {
    let info = format!(
        "name={}\ntime={}\nbase={}\ncommit={}\ndeleted={}\n",
        shelf.name.replace('\n', " "),
        shelf.time,
        shelf.base,
        shelf.commit,
        shelf.deleted
    );
    std::fs::write(dir.join("info"), info)?;
    Ok(())
}

fn read_info(id: &str, text: &str) -> Shelf {
    let mut shelf = Shelf {
        id: id.to_owned(),
        name: String::new(),
        time: 0,
        base: String::new(),
        commit: String::new(),
        deleted: false,
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "name" => shelf.name = value.to_owned(),
            "time" => shelf.time = value.parse().unwrap_or_default(),
            "base" => shelf.base = value.to_owned(),
            "commit" => shelf.commit = value.to_owned(),
            "deleted" => shelf.deleted = value == "true",
            _ => {}
        }
    }
    shelf
}

/// Shelved changelists, newest first.
pub fn shelves(repository: &Repository) -> Vec<Shelf> {
    let Ok(entries) = std::fs::read_dir(shelf_dir(repository)) else { return Vec::new() };
    let mut shelves: Vec<Shelf> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path().join("info")).ok()?;
            Some(read_info(&id, &text))
        })
        .collect();
    shelves.sort_by(|a, b| b.time.cmp(&a.time).then(b.id.cmp(&a.id)));
    shelves
}

/// Shelve Changes: saves `paths` to a new shelf and, unless `keep`, rolls them back.
pub fn shelve(repository: &Repository, paths: &[String], name: &str, keep: bool) -> Result<Shelf> {
    if paths.is_empty() {
        bail!("No changes to shelve");
    }
    let base = head(repository).context("Cannot shelve before the first commit")?;
    let tree = working_tree(repository, paths)?;
    let commit = repository
        .run_with_input(["commit-tree", tree.as_str(), "-p", base.as_str()], Some(name))?
        .trim()
        .to_owned();
    let patch = between(repository, &base, &commit, false)?;
    let time = now();
    let mut id = format!("{time}");
    let root = shelf_dir(repository);
    let mut n = 1;
    while root.join(&id).exists() {
        id = format!("{time}-{n}");
        n += 1;
    }
    let dir = root.join(&id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("shelved.patch"), &patch)?;
    let shelf = Shelf { id: id.clone(), name: name.to_owned(), time, base, commit, deleted: false };
    write_info(&dir, &shelf)?;
    repository.run(["update-ref", &format!("refs/gitglass/shelf/{id}"), &shelf.commit])?;
    if !keep {
        rollback(repository, paths)?;
    }
    Ok(shelf)
}

/// Files in a shelf, as the Shelf tab's change tree shows them.
pub fn shelf_files(repository: &Repository, shelf: &Shelf) -> Result<Vec<FileChange>> {
    if !shelf.commit.is_empty() && repository.run(["cat-file", "-e", &shelf.commit]).is_ok() {
        let output = repository.run(["diff", "--name-status", "-z", "-M", &shelf.base, &shelf.commit])?;
        return Ok(parse_name_status(&output));
    }
    // A shelf imported from a bare patch: list what the patch touches.
    let patch = std::fs::read_to_string(shelf.patch_path(repository))?;
    Ok(files(repository, &patch)?
        .into_iter()
        .map(|f| FileChange { kind: super::FileChangeKind::Modified, path: f.path, old_path: None })
        .collect())
}

/// Unshelve: applies the shelf (optionally only `paths`) and, unless
/// `keep`, moves it to Recently Deleted.
pub fn unshelve(repository: &Repository, shelf: &Shelf, paths: Option<&[String]>, keep: bool) -> Result<ApplyOutcome> {
    let patch = match paths {
        Some(paths) if !shelf.commit.is_empty() => {
            let mut args = vec!["diff", "--binary", "--full-index", shelf.base.as_str(), shelf.commit.as_str(), "--"];
            args.extend(paths.iter().map(String::as_str));
            repository.run(args)?
        }
        _ => std::fs::read_to_string(shelf.patch_path(repository))?,
    };
    let outcome = apply(repository, &patch, false)?;
    if !keep && paths.is_none() {
        set_deleted(repository, shelf, true)?;
    }
    Ok(outcome)
}

/// Delete moves a shelf to Recently Deleted; deleting it there removes it.
pub fn delete_shelf(repository: &Repository, shelf: &Shelf) -> Result<()> {
    if !shelf.deleted {
        return set_deleted(repository, shelf, true);
    }
    std::fs::remove_dir_all(shelf_dir(repository).join(&shelf.id))?;
    repository.run(["update-ref", "-d", &format!("refs/gitglass/shelf/{}", shelf.id)]).ok();
    Ok(())
}

pub fn set_deleted(repository: &Repository, shelf: &Shelf, deleted: bool) -> Result<()> {
    let shelf = Shelf { deleted, ..shelf.clone() };
    write_info(&shelf_dir(repository).join(&shelf.id), &shelf)
}

pub fn rename_shelf(repository: &Repository, shelf: &Shelf, name: &str) -> Result<()> {
    let shelf = Shelf { name: name.to_owned(), ..shelf.clone() };
    write_info(&shelf_dir(repository).join(&shelf.id), &shelf)
}

/// Import Patches…: copies a patch file into the shelf.
pub fn import_patch(repository: &Repository, name: &str, patch: &str) -> Result<Shelf> {
    files(repository, patch)?;
    let time = now();
    let id = format!("{time}-import");
    let dir = shelf_dir(repository).join(&id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("shelved.patch"), patch)?;
    let shelf = Shelf {
        id,
        name: name.to_owned(),
        time,
        base: String::new(),
        commit: String::new(),
        deleted: false,
    };
    write_info(&dir, &shelf)?;
    Ok(shelf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitConsole;

    fn repo() -> (tempdir::Dir, Repository) {
        let dir = tempdir::Dir::new();
        let git = |args: &[&str]| {
            std::process::Command::new("git").current_dir(&dir.0).args(args).output().unwrap();
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(dir.0.join("a.txt"), "one\ntwo\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        let repository = Repository::discover(&dir.0, GitConsole::default()).unwrap();
        (dir, repository)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir()
                    .join(format!("gitglass-patch-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
    }

    #[test]
    fn shelve_and_unshelve_round_trip() {
        let (dir, repository) = repo();
        std::fs::write(dir.0.join("a.txt"), "one\nTWO\n").unwrap();
        std::fs::write(dir.0.join("new.txt"), "fresh\n").unwrap();
        let paths = vec!["a.txt".to_owned(), "new.txt".to_owned()];

        let patch = local_changes(&repository, &paths, false).unwrap();
        assert!(patch.contains("+TWO") && patch.contains("+fresh"));
        // The user's index is untouched.
        assert_eq!(repository.run(["diff", "--cached", "--name-only"]).unwrap(), "");

        let shelf = shelve(&repository, &paths, "work", false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.0.join("a.txt")).unwrap(), "one\ntwo\n");
        assert!(!dir.0.join("new.txt").exists());
        let names: Vec<_> = shelf_files(&repository, &shelf).unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(names, ["a.txt", "new.txt"]);

        assert_eq!(unshelve(&repository, &shelf, None, false).unwrap(), ApplyOutcome::Clean);
        assert_eq!(std::fs::read_to_string(dir.0.join("new.txt")).unwrap(), "fresh\n");
        assert!(shelves(&repository)[0].deleted);
    }

    #[test]
    fn lists_patch_files() {
        let (dir, repository) = repo();
        std::fs::write(dir.0.join("a.txt"), "one\n").unwrap();
        let patch = local_changes(&repository, &["a.txt".to_owned()], false).unwrap();
        let files = files(&repository, &patch).unwrap();
        assert_eq!(files, [PatchFile { path: "a.txt".into(), added: Some(0), removed: Some(1) }]);
    }
}

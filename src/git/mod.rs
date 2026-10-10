//! Git backend. Like IntelliJ's git4idea, every operation shells out to the
//! `git` executable and parses its machine-readable output.

mod command;
pub mod blame;
pub mod blob;
pub mod changelists;
pub mod diff;
pub mod gpg;
pub mod graph;
pub mod hosting;
pub mod log;
pub mod merge;
pub mod ops;
pub mod patch;
pub mod rebase;
pub mod refs;
pub mod remotes;
pub mod roots;
pub mod worktree;
pub mod status;
pub mod submodule;
#[cfg(test)]
pub mod test_support;

pub use command::{GitConsole, Repository, OpenError, detected_executable, executable, executable_version, git_process, redetect_executable, trust_directory, run_in, set_executable, set_use_credential_helper};
pub use graph::GraphLayout;
pub use log::{Commit, CommitDetails, FileChangeKind, LogFilter};
pub use refs::{RefKind, RefName, RepositoryRefs};
pub use status::{StatusKind, WorkingTreeStatus};

/// A long-running operation the repository is in the middle of, shown in the
/// branch widget the way IntelliJ shows "Rebasing master".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryState {
    Normal,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

/// The app was called GitGlass before: its per-repository data (changelists,
/// shelf, favorites, roots, index cache) lived in `<git dir>/gitglass` and
/// shelves under `refs/gitglass/`. Moves both to the Junction names once.
pub fn migrate_legacy_data(repository: &Repository) {
    let (legacy, current) = (repository.git_dir().join("gitglass"), repository.git_dir().join("junction"));
    if legacy.is_dir() && !current.exists() {
        let _ = std::fs::rename(&legacy, &current);
    }
    let Ok(refs) = repository.run(["for-each-ref", "--format=%(refname) %(objectname)", "refs/gitglass/"]) else { return };
    for line in refs.lines() {
        let Some((name, target)) = line.split_once(' ') else { continue };
        let renamed = name.replacen("refs/gitglass/", "refs/junction/", 1);
        if repository.run(["update-ref", renamed.as_str(), target]).is_ok() {
            let _ = repository.run(["update-ref", "-d", name]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_gitglass_data_and_refs() {
        let dir = std::env::temp_dir().join(format!("junction-test-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success());
        git(&["init", "-q", "-b", "main"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first"]);
        git(&["update-ref", "refs/gitglass/shelf/1", "HEAD"]);
        std::fs::create_dir_all(dir.join(".git/gitglass")).unwrap();
        std::fs::write(dir.join(".git/gitglass/changelists"), "x").unwrap();
        let repo = Repository::discover(&dir, GitConsole::default()).unwrap();
        migrate_legacy_data(&repo);
        assert!(dir.join(".git/junction/changelists").exists());
        assert!(!dir.join(".git/gitglass").exists());
        assert!(repo.run(["rev-parse", "--verify", "-q", "refs/junction/shelf/1"]).is_ok());
        assert!(repo.run(["rev-parse", "--verify", "-q", "refs/gitglass/shelf/1"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

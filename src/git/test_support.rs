//! Throwaway repositories for the git backend's tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{GitConsole, Repository};

/// A repository in the temp folder with one commit on `main`, removed on drop.
pub struct TestRepo {
    pub dir: PathBuf,
    pub repo: Repository,
}

impl TestRepo {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("junction-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "T"]);
        git(&dir, &["config", "user.email", "t@x"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        let repo = Repository::discover(&dir, GitConsole::default()).unwrap();
        let this = Self { dir, repo };
        this.commit("a.txt", "1\n", "first");
        this
    }

    /// Runs git in the repository, panicking on failure; returns stdout.
    pub fn git(&self, args: &[&str]) -> String {
        git(&self.dir, args)
    }

    /// Writes `file` and commits it; returns the new commit's hash.
    pub fn commit(&self, file: &str, content: &str, message: &str) -> String {
        let path = self.dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
        self.git(&["add", "--", file]);
        self.git(&["commit", "-qm", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// A bare repository next to this one, added as remote `name` and fetched.
    pub fn bare_remote(&self, name: &str) -> PathBuf {
        let bare = self.dir.with_file_name(format!("{}-{name}.git", self.dir.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&bare);
        git(&self.dir, &["init", "-q", "--bare", bare.to_str().unwrap()]);
        bare
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let prefix = format!("{}-", self.dir.file_name().unwrap().to_string_lossy());
        if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

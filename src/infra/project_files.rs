//! The project's files on disk, for refactorings: a renamed file moves
//! with `git mv` when tracked, so Git sees a rename.

use std::path::PathBuf;

use crate::git::Repository;
use crate::usecase::rename_symbol::ProjectFiles;

pub struct DiskFiles {
    pub root: PathBuf,
    pub repository: Option<Repository>,
}

impl ProjectFiles for DiskFiles {
    fn read(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(self.root.join(path)).map_err(|e| format!("{path}: {e}"))
    }

    fn write(&self, path: &str, text: &str) -> Result<(), String> {
        std::fs::write(self.root.join(path), text).map_err(|e| format!("{path}: {e}"))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (source, dest) = (self.root.join(from), self.root.join(to));
        if dest.exists() {
            return Err(format!("{to} already exists"));
        }
        crate::git::ops::rename_path(self.repository.as_ref(), from, &source, &dest);
        if dest.exists() { Ok(()) } else { Err(format!("Cannot rename {from}")) }
    }
}

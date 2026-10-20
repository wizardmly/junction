//! Edits to the project's other Kotlin and Java files for a refactoring
//! that changes a declaration's uses (Inline Property / Field): the files
//! mentioning a name, what each holds, then each rewritten in one write.

use crate::domain::introduce_parameter::{Edit, apply};
use crate::domain::project_files::ProjectFiles;

/// A file and what the refactoring found in it, with its text when read.
#[derive(Clone, Debug)]
pub struct FileChange<T> {
    pub path: String,
    pub text: String,
    pub found: T,
}

/// The project's other Kotlin and Java files containing `word`, with what
/// `found` sees in each (files where it sees nothing are left out).
pub fn scan<T>(files: &dyn ProjectFiles, except: &str, word: &str, found: &dyn Fn(&str, &str) -> Option<T>) -> Vec<FileChange<T>> {
    files
        .list()
        .into_iter()
        .filter(|f| f != except && (f.ends_with(".kt") || f.ends_with(".java")))
        .filter_map(|path| {
            let text = files.read(&path).ok().filter(|t| t.contains(word))?;
            let found = found(&path, &text)?;
            Some(FileChange { path, text, found })
        })
        .collect()
}

/// Rewrites each file with its edits; the files written.
pub fn write<T>(files: &dyn ProjectFiles, changes: &[FileChange<T>], edits: &dyn Fn(&FileChange<T>) -> Vec<Edit>) -> Vec<String> {
    changes
        .iter()
        .filter_map(|change| {
            let list = edits(change);
            if list.is_empty() {
                return None;
            }
            files.write(&change.path, &apply(&change.text, list)).ok().map(|_| change.path.clone())
        })
        .collect()
}

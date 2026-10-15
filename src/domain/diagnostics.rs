//! Problems shown in the editor and the Problems tool window: syntax
//! errors found while typing, and the compiler's errors and warnings from
//! the last build of the module.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// Who reported a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    Syntax,
    Compiler,
}

/// A 0-based line and column; the column counts characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

impl Pos {
    pub fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }

    /// The position of a byte offset of `text`.
    pub fn of(text: &str, offset: usize) -> Self {
        let offset = offset.min(text.len());
        let before = &text[..offset];
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        Self { line: before.matches('\n').count() as u32, col: before[line_start..].chars().count() as u32 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub start: Pos,
    pub end: Pos,
    pub severity: Severity,
    pub message: String,
    pub origin: Origin,
}

/// A problem in a file of the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProblem {
    pub file: PathBuf,
    pub problem: Problem,
}

/// Finds syntax errors in a file's text. Implemented per language.
pub trait SyntaxChecker {
    fn check(&self, text: &str) -> Vec<Problem>;
}

/// The compiler's problems across the project, by file, from each
/// module's last build. A file key is normalized with [`file_key`].
#[derive(Clone, Debug, Default)]
pub struct ProblemStore {
    /// By file key: the file as reported, and its problems.
    by_file: std::collections::HashMap<String, (PathBuf, Vec<Problem>)>,
}

impl ProblemStore {
    /// Replaces what a module's last build said: every file under `dir`.
    pub fn replace_under(&mut self, dir: &std::path::Path, problems: Vec<FileProblem>) {
        let prefix = format!("{}/", file_key(dir));
        self.by_file.retain(|file, _| !file.starts_with(&prefix));
        for p in problems {
            self.by_file.entry(file_key(&p.file)).or_insert_with(|| (p.file.clone(), Vec::new())).1.push(p.problem);
        }
    }

    pub fn problems(&self, file: &std::path::Path) -> Vec<Problem> {
        self.by_file.get(&file_key(file)).map(|(_, p)| p.clone()).unwrap_or_default()
    }

    /// Every file with problems, by path, each file's problems in order.
    pub fn all(&self) -> Vec<(PathBuf, Vec<Problem>)> {
        let mut out: Vec<(PathBuf, Vec<Problem>)> = self.by_file.values().cloned().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, problems) in &mut out {
            problems.sort_by_key(|p| (p.start, p.end));
        }
        out
    }
}

/// A path as a comparable key: `/` separators, no trailing slash, and on
/// Windows no case (compilers and the editor spell drives differently).
pub fn file_key(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_owned();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_replaces_a_module() {
        let p = |file: &str| FileProblem { file: file.into(), problem: Problem { start: Pos::default(), end: Pos::new(0, 1), severity: Severity::Error, message: "x".into(), origin: Origin::Compiler } };
        let mut store = ProblemStore::default();
        store.replace_under(std::path::Path::new("/p/app"), vec![p("/p/app/A.kt"), p("/p/app/A.kt")]);
        store.replace_under(std::path::Path::new("/p/lib"), vec![p("/p/lib/B.kt")]);
        assert_eq!(store.problems(std::path::Path::new("/p/app/A.kt")).len(), 2);
        store.replace_under(std::path::Path::new("/p/app"), vec![]);
        assert!(store.problems(std::path::Path::new("/p/app/A.kt")).is_empty());
        assert_eq!(store.problems(std::path::Path::new("/p/lib/B.kt")).len(), 1);
    }

    #[test]
    fn positions() {
        assert_eq!(Pos::of("ab\ncd", 4), Pos::new(1, 1));
    }
}

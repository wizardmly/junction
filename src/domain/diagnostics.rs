//! Problems shown in the editor and the Problems tool window: syntax
//! errors found while typing, the compiler's errors and warnings from
//! the last build of the module, and a language server's.

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
    /// An inspection of the file (a newer dependency version).
    Inspection,
    /// The file's language server (`textDocument/publishDiagnostics`).
    Server,
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

/// A language server's problem in the editor's terms: its UTF-16
/// columns as characters of `text`'s lines. Errors and warnings only:
/// IntelliJ doesn't highlight a server's information and hints.
pub fn from_server(text: &str, start: (u32, u32), end: (u32, u32), severity: u8, message: &str) -> Option<Problem> {
    let severity = match severity {
        1 => Severity::Error,
        2 => Severity::Warning,
        _ => return None,
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let pos = |(line, col): (u32, u32)| {
        let text = lines.get(line as usize).copied().unwrap_or("");
        let mut units = 0;
        let chars = text.chars().take_while(|c| {
            units += c.len_utf16() as u32;
            units <= col
        });
        Pos::new(line, chars.count() as u32)
    };
    Some(Problem { start: pos(start), end: pos(end), severity, message: message.to_owned(), origin: Origin::Server, fix: None })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub start: Pos,
    pub end: Pos,
    pub severity: Severity,
    pub message: String,
    pub origin: Origin,
    /// Alt+Enter's fix: replaces the problem's text.
    pub fix: Option<Fix>,
}

/// A quick fix: "Change to 1.13.1".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    pub replacement: String,
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

/// Two reports of problems by file in one (a build's and the language
/// servers'), sorted by file; a problem the first already has on the same
/// line with the same message isn't repeated.
pub fn merge_files(mut first: Vec<(PathBuf, Vec<Problem>)>, second: &[(PathBuf, Vec<Problem>)]) -> Vec<(PathBuf, Vec<Problem>)> {
    for (path, problems) in second {
        let key = file_key(path);
        match first.iter_mut().find(|(p, _)| file_key(p) == key) {
            Some((_, existing)) => {
                let new: Vec<Problem> = problems.iter().filter(|p| !existing.iter().any(|e| e.start.line == p.start.line && e.message == p.message)).cloned().collect();
                existing.extend(new);
                existing.sort_by_key(|p| (p.start, p.end));
            }
            None => first.push((path.clone(), problems.clone())),
        }
    }
    first.sort_by(|a, b| a.0.cmp(&b.0));
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_reports() {
        let p = |line, message: &str, origin| Problem { start: Pos::new(line, 0), end: Pos::new(line, 1), severity: Severity::Error, message: message.into(), origin, fix: None };
        let build = vec![(PathBuf::from("/p/b.kt"), vec![p(3, "Unresolved reference: x", Origin::Compiler)])];
        let server = vec![
            (PathBuf::from("/p/b.kt"), vec![p(3, "Unresolved reference: x", Origin::Server), p(1, "Type mismatch", Origin::Server)]),
            (PathBuf::from("/p/a.kt"), vec![p(0, "Expecting '}'", Origin::Server)]),
        ];
        let merged = merge_files(build, &server);
        assert_eq!(merged.iter().map(|(f, ps)| (f.to_str().unwrap(), ps.len())).collect::<Vec<_>>(), [("/p/a.kt", 1), ("/p/b.kt", 2)]);
        assert_eq!(merged[1].1[0].message, "Type mismatch");
        assert_eq!(merged[1].1[1].origin, Origin::Compiler);
    }

    #[test]
    fn reads_server_problems() {
        // "😀" is two UTF-16 units, one character.
        let text = "fun main() {\n    val s = \"😀\" + x\n}";
        let p = from_server(text, (1, 19), (1, 20), 1, "Unresolved reference 'x'.").unwrap();
        assert_eq!((p.start, p.end, p.severity, p.origin), (Pos::new(1, 18), Pos::new(1, 19), Severity::Error, Origin::Server));
        assert_eq!(from_server(text, (1, 8), (1, 9), 2, "unused").map(|p| p.severity), Some(Severity::Warning));
        assert_eq!(from_server(text, (0, 0), (0, 3), 4, "hint"), None);
    }

    #[test]
    fn store_replaces_a_module() {
        let p = |file: &str| FileProblem { file: file.into(), problem: Problem { start: Pos::default(), end: Pos::new(0, 1), severity: Severity::Error, message: "x".into(), origin: Origin::Compiler, fix: None } };
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

//! Analyze › Inspect Code…: the inspections Junction has, run over a scope
//! of the project, as IntelliJ's Inspection Results: syntax errors from
//! the parser, and unused imports in Java and Kotlin.

use crate::domain::diagnostics::{Origin, Pos, Problem, Severity, SyntaxChecker};
use crate::domain::imports::{self, ImportStyle};
use crate::domain::project_files::ProjectFiles;

/// One inspection's finding in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// IntelliJ's inspection name, the group in the results.
    pub inspection: &'static str,
    pub problem: Problem,
}

pub const SYNTAX: &str = "Syntax error";
pub const UNUSED_KOTLIN: &str = "Unused import directive";
pub const UNUSED_JAVA: &str = "Unused import";

/// The files of the selection (project-relative paths and whether each is
/// a folder; the empty path is the project) `inspectable` accepts, in order.
pub fn targets(files: &dyn ProjectFiles, items: &[(String, bool)], inspectable: &dyn Fn(&str) -> bool) -> Vec<String> {
    let listed = items.iter().any(|(_, dir)| *dir).then(|| files.list()).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (path, dir) in items {
        if *dir {
            let prefix = if path.is_empty() { String::new() } else { format!("{}/", path.trim_end_matches('/')) };
            out.extend(listed.iter().filter(|p| p.starts_with(&prefix) && inspectable(p)).cloned());
        } else if inspectable(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A file's findings, in order; `syntax` checks its language's parse.
pub fn inspect(path: &str, text: &str, syntax: Option<&dyn SyntaxChecker>) -> Vec<Finding> {
    let mut out: Vec<Finding> = syntax.map(|s| s.check(text)).unwrap_or_default().into_iter().map(|problem| Finding { inspection: SYNTAX, problem }).collect();
    let style = if path.ends_with(".kt") || path.ends_with(".kts") {
        Some((ImportStyle::Kotlin, UNUSED_KOTLIN))
    } else if path.ends_with(".java") {
        Some((ImportStyle::Java, UNUSED_JAVA))
    } else {
        None
    };
    if let Some((style, inspection)) = style {
        for (line, path) in imports::unused_imports(style, text) {
            let col = text.lines().nth(line as usize).map_or(0, |l| (l.len() - l.trim_start().len()) as u32);
            let message = match style {
                ImportStyle::Kotlin => format!("Unused import directive '{path}'"),
                ImportStyle::Java => format!("Unused import '{path}'"),
            };
            let problem = Problem { start: Pos::new(line, col), end: Pos::new(line, col), severity: Severity::Warning, message, origin: Origin::Inspection, fix: None };
            out.push(Finding { inspection, problem });
        }
    }
    out.sort_by_key(|f| (f.problem.start, f.problem.end));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoErrors;
    impl SyntaxChecker for NoErrors {
        fn check(&self, _: &str) -> Vec<Problem> {
            Vec::new()
        }
    }

    #[test]
    fn finds_unused_imports() {
        let text = "package p\n\nimport a.B\n\nfun f() = 1\n";
        let found = inspect("app/F.kt", text, Some(&NoErrors));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].inspection, UNUSED_KOTLIN);
        assert_eq!(found[0].problem.start, Pos::new(2, 0));
    }
}

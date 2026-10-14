//! Syntax highlighting of errors while typing (IntelliJ's red squiggles
//! from the parser), sorted for the editor.

use crate::domain::diagnostics::{Problem, SyntaxChecker};

pub fn check(checker: &dyn SyntaxChecker, text: &str) -> Vec<Problem> {
    let mut problems = checker.check(text);
    problems.sort_by_key(|p| (p.start, p.end));
    problems.dedup_by_key(|p| p.start);
    problems
}

//! Syntax errors from the tree-sitter parse of the file being edited: its
//! ERROR and MISSING nodes, as the parser recovers from them.

use tree_sitter::Node;

use crate::domain::diagnostics::{Origin, Pos, Problem, Severity, SyntaxChecker};
use crate::index::lang::Lang;
use crate::index::symbols;

pub struct TreeSitterSyntax {
    pub lang: Lang,
}

/// At most this many errors: past a few, recovery reports noise.
const LIMIT: usize = 20;

impl SyntaxChecker for TreeSitterSyntax {
    fn check(&self, text: &str) -> Vec<Problem> {
        let Some(tree) = symbols::parse(self.lang, text) else { return Vec::new() };
        let root = tree.root_node();
        if !root.has_error() {
            return Vec::new();
        }
        let mut out = Vec::new();
        collect(root, text, &mut out);
        out
    }
}

fn collect(node: Node, text: &str, out: &mut Vec<Problem>) {
    if out.len() >= LIMIT {
        return;
    }
    // A zero-width node holding the error itself: a MISSING leaf the
    // grammar hides (Kotlin's `simple_identifier` over `_alpha_identifier`).
    let hidden_missing = node.has_error() && node.start_byte() == node.end_byte() && node.child_count() == 0;
    if node.is_missing() || hidden_missing {
        // IntelliJ: "'}' expected", at the place it should be.
        let at = Pos::of(text, node.start_byte());
        let what = if hidden_missing { node.kind().trim_start_matches("simple_") } else { node.kind() };
        let message = if what.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("{} expected", what.replace('_', " ")) } else { format!("'{what}' expected") };
        out.push(Problem { start: at, end: Pos::new(at.line, at.col + 1), severity: Severity::Error, message, origin: Origin::Syntax, fix: None });
        return;
    }
    if node.is_error() {
        // The unexpected text, on its first line only.
        let start = node.start_byte();
        let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        let end = node.end_byte().min(line_end).max(start);
        let snippet: String = text[start..end].trim().chars().take(30).collect();
        let message = if snippet.is_empty() { "Unexpected tokens".to_owned() } else { format!("Unexpected tokens: {snippet}") };
        let (s, mut e) = (Pos::of(text, start), Pos::of(text, end));
        if e == s {
            e.col += 1;
        }
        out.push(Problem { start: s, end: e, severity: Severity::Error, message, origin: Origin::Syntax, fix: None });
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, text, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_kotlin_and_java_errors() {
        let ok = TreeSitterSyntax { lang: Lang::Kotlin };
        assert!(ok.check("fun f() {\n    val x = 1\n}\n").is_empty());
        let broken = ok.check("fun f() {\n    val x = \n}\nclass A {\n");
        assert!(!broken.is_empty());
        assert!(broken.iter().all(|p| p.severity == Severity::Error && p.origin == Origin::Syntax));
        let p = ok.check("class A {\n    fun f() {\n        val x = (1 + )\n    }\n}\n");
        assert!(!p.is_empty(), "{p:?}");
        let java = TreeSitterSyntax { lang: Lang::Java };
        let p = java.check("class A {\n    void f() {\n        int x = 1\n    }\n}\n");
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].start.line, 2);
        assert!(p[0].message.contains("';'"), "{p:?}");
    }
}

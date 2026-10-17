//! Code › Unwrap/Remove (Ctrl+Shift+Delete), as IntelliJ's: the
//! statements around the caret it can take apart, innermost first —
//! "Unwrap 'if...'" keeps the branch's body in place of the `if`, "Remove
//! 'else...'" drops the else branch, and loops and `try` give up their
//! bodies the same way (a `try` keeps its `finally` code after).

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::children;
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

/// A body's statements: inside its braces, blank edge lines and their
/// common indent removed.
fn body_lines(text: &str, body: Node) -> Vec<String> {
    let mut inner = &text[body.byte_range()];
    if inner.starts_with('{') && inner.ends_with('}') && inner.len() >= 2 {
        inner = &inner[1..inner.len() - 1];
    }
    let lines: Vec<&str> = inner.lines().collect();
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else { return Vec::new() };
    let lines = &lines[first..=last];
    // A first line sharing the brace's line (`{ a()`) has no indent of its own.
    let indent = lines.iter().skip(usize::from(first == 0)).filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| if i == 0 && first == 0 { l.trim().to_owned() } else { l.get(indent..).unwrap_or(l.trim_start()).trim_end().to_owned() })
        .collect()
}

/// `construct` replaced by `lines`, indented as it was; the caret at the
/// first one.
fn replace_with(text: &str, construct: Node, lines: &[String], title: String) -> Option<Intention> {
    let line_start = text[..construct.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let indent = &text[line_start..construct.start_byte()];
    if !indent.trim().is_empty() {
        return None;
    }
    let end = construct.end_byte();
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let end = if text[end..line_end].trim().is_empty() { line_end } else { end };
    let replacement = if lines.is_empty() {
        String::new()
    } else {
        lines.iter().map(|l| if l.is_empty() { String::new() } else { format!("{indent}{l}") }).collect::<Vec<_>>().join("\n")
    };
    // Nothing left: the construct's whole line goes.
    let (range, replacement) = if lines.is_empty() { (line_start..(end + 1).min(text.len()), replacement) } else { (line_start..end, replacement) };
    let caret = range.start + replacement.len() - replacement.trim_start().len();
    Some(Intention { title, range, replacement, caret })
}

fn contains(node: Node, at: usize) -> bool {
    node.start_byte() <= at && at <= node.end_byte()
}

/// The Unwrap/Remove choices at `offset`, innermost first.
pub fn unwrap_options(lang: Lang, text: &str, offset: usize) -> Vec<Intention> {
    let mut out = Vec::new();
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return out;
    }
    let Some(tree) = symbols::parse(lang, text) else { return out };
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
    while let Some(n) = node {
        node = n.parent();
        match n.kind() {
            "if_expression" | "if_statement" => {
                // Kotlin's `val x = if …` is a value, not a statement.
                if lang == Lang::Kotlin && !n.parent().is_some_and(|p| matches!(p.kind(), "statements" | "control_structure_body")) {
                    continue;
                }
                let (Some(then), alt) = (n.child_by_field_name("consequence"), n.child_by_field_name("alternative")) else { continue };
                match alt {
                    Some(alt) if contains(alt, offset) && offset >= alt.start_byte() => {
                        out.extend(replace_with(text, n, &body_lines(text, alt), "Unwrap 'else...'".into()));
                        // Up to the end of the then branch.
                        let kept = &text[n.start_byte()..then.end_byte()];
                        let end = n.end_byte();
                        out.push(Intention { title: "Remove 'else...'".into(), range: n.start_byte()..end, replacement: kept.to_owned(), caret: n.start_byte() });
                    }
                    _ => out.extend(replace_with(text, n, &body_lines(text, then), "Unwrap 'if...'".into())),
                }
            }
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_while_statement" | "do_statement" => {
                let body = n.child_by_field_name("body").or_else(|| children(n).into_iter().rev().find(|c| c.kind() == "control_structure_body"));
                let Some(body) = body else { continue };
                let keyword = if n.kind().starts_with("do") { "do" } else if n.kind().contains("for") { "for" } else { "while" };
                out.extend(replace_with(text, n, &body_lines(text, body), format!("Unwrap '{keyword}...'")));
            }
            "try_expression" | "try_statement" => {
                let parts = children(n);
                let body = n.child_by_field_name("body").or_else(|| parts.iter().find(|c| c.kind() == "statements").copied());
                let Some(body) = body else { continue };
                let mut lines = body_lines(text, body);
                if let Some(finally) = parts.iter().find(|c| matches!(c.kind(), "finally_block" | "finally_clause")) {
                    let block = children(*finally).into_iter().find(|c| matches!(c.kind(), "statements" | "block"));
                    lines.extend(block.map(|b| body_lines(text, b)).unwrap_or_default());
                }
                out.extend(replace_with(text, n, &lines, "Unwrap 'try...'".into()));
            }
            "function_declaration" | "method_declaration" | "class_body" | "lambda_literal" | "lambda_expression" => break,
            _ => {}
        }
    }
    out
}

/// The text after applying a choice (tests).
#[cfg(test)]
fn apply(text: &str, i: &Intention) -> String {
    let mut out = text.to_owned();
    out.replace_range(i.range.clone(), &i.replacement);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_kotlin() {
        let text = "fun f() {\n    if (a) {\n        x()\n        y()\n    } else {\n        z()\n    }\n    done()\n}\n";
        let at = text.find("y()").unwrap();
        let options = unwrap_options(Lang::Kotlin, text, at);
        assert_eq!(options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Unwrap 'if...'"]);
        assert_eq!(apply(text, &options[0]), "fun f() {\n    x()\n    y()\n    done()\n}\n");
        let at = text.find("z()").unwrap();
        let options = unwrap_options(Lang::Kotlin, text, at);
        assert_eq!(options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Unwrap 'else...'", "Remove 'else...'"]);
        assert_eq!(apply(text, &options[0]), "fun f() {\n    z()\n    done()\n}\n");
        assert_eq!(apply(text, &options[1]), "fun f() {\n    if (a) {\n        x()\n        y()\n    }\n    done()\n}\n");
        // Nested: innermost first; try keeps finally's code.
        let text = "fun g() {\n    for (i in l) {\n        try { q(i) } finally { s() }\n    }\n}\n";
        let options = unwrap_options(Lang::Kotlin, text, text.find("q(").unwrap());
        assert_eq!(options.iter().map(|o| o.title.as_str()).collect::<Vec<_>>(), ["Unwrap 'try...'", "Unwrap 'for...'"]);
        assert_eq!(apply(text, &options[0]), "fun g() {\n    for (i in l) {\n        q(i)\n        s()\n    }\n}\n");
        assert_eq!(apply(text, &options[1]), "fun g() {\n    try { q(i) } finally { s() }\n}\n");
    }

    #[test]
    fn unwraps_java() {
        let text = "class A {\n  void f() {\n    while (c)\n      w();\n    if (a) { x(); }\n  }\n}\n";
        let options = unwrap_options(Lang::Java, text, text.find("w()").unwrap());
        assert_eq!(apply(text, &options[0]), "class A {\n  void f() {\n    w();\n    if (a) { x(); }\n  }\n}\n");
        let options = unwrap_options(Lang::Java, text, text.find("x()").unwrap());
        assert_eq!(apply(text, &options[0]), "class A {\n  void f() {\n    while (c)\n      w();\n    x();\n  }\n}\n");
    }
}

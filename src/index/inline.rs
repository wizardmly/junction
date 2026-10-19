//! Inline Variable (Ctrl+Alt+N), as IntelliJ's for a local: each use of
//! the variable becomes its initializer (parenthesized where an operator
//! around it would bind tighter), and the declaration goes. A variable
//! written after its declaration, without an initializer or unused is
//! refused with IntelliJ's message.

use tree_sitter::Node;

use super::lang::Lang;
use super::{locals, symbols};
use crate::domain::line_edit::Edited;

/// Expressions that stay one operand wherever they go.
pub(super) fn is_atom(kind: &str) -> bool {
    matches!(
        kind,
        "simple_identifier" | "identifier" | "call_expression" | "navigation_expression" | "parenthesized_expression" | "string_literal" | "line_string_literal"
            | "multi_line_string_literal" | "integer_literal" | "long_literal" | "real_literal" | "boolean_literal" | "character_literal" | "null_literal" | "null"
            | "true" | "false" | "decimal_integer_literal" | "hex_integer_literal" | "decimal_floating_point_literal" | "method_invocation" | "field_access"
            | "object_creation_expression" | "array_access" | "indexing_expression" | "this_expression" | "this" | "lambda_literal" | "collection_literal"
            | "array_creation_expression" | "class_literal" | "callable_reference"
    )
}

/// Places where any expression fits without parentheses.
pub(super) fn takes_any(kind: &str) -> bool {
    matches!(
        kind,
        "value_argument" | "value_arguments" | "argument_list" | "parenthesized_expression" | "jump_expression" | "return_statement" | "statements"
            | "expression_statement" | "variable_declarator" | "property_declaration" | "assignment" | "assignment_expression" | "block" | "function_body"
            | "control_structure_body" | "interpolated_expression" | "indexing_suffix" | "array_access" | "collection_literal" | "array_initializer"
    )
}

/// The declaration holding the declared name, and its initializer.
fn declaration<'t>(lang: Lang, name: Node<'t>) -> Option<(Node<'t>, Option<Node<'t>>)> {
    let mut at = name.parent();
    while let Some(n) = at {
        match (lang, n.kind()) {
            // A local: a member is Inline Property's (its references may be
            // in other files).
            (Lang::Kotlin, "property_declaration") => {
                if !n.parent().is_some_and(|p| p.kind() == "statements") {
                    return None;
                }
                let mut cursor = n.walk();
                let children: Vec<Node> = n.children(&mut cursor).collect();
                let eq = children.iter().position(|c| c.kind() == "=");
                return Some((n, eq.and_then(|i| children.get(i + 1).copied())));
            }
            (Lang::Java, "variable_declarator") => {
                let decl = n.parent().filter(|p| p.kind() == "local_variable_declaration")?;
                return Some((decl, n.child_by_field_name("value")));
            }
            (_, "function_declaration" | "method_declaration" | "class_body" | "block" | "statements" | "parameter" | "formal_parameter" | "lambda_literal") => return None,
            _ => at = n.parent(),
        }
    }
    None
}

/// Whether a use is written to: `x = …`, `x += …`, `x++`.
pub(super) fn is_written(lang: Lang, use_node: Node) -> bool {
    let Some(parent) = use_node.parent() else { return false };
    match lang {
        Lang::Kotlin => {
            let target = if parent.kind() == "directly_assignable_expression" { parent.parent() } else { Some(parent) };
            let assigned = target.is_some_and(|t| t.kind() == "assignment" && t.child(0).is_some_and(|c| c.byte_range().contains(&use_node.start_byte())));
            let mut cursor = parent.walk();
            let stepped = matches!(parent.kind(), "postfix_expression" | "prefix_expression") && parent.children(&mut cursor).any(|c| matches!(c.kind(), "++" | "--"));
            assigned || stepped
        }
        _ => {
            (parent.kind() == "assignment_expression" && parent.child_by_field_name("left") == Some(use_node)) || parent.kind() == "update_expression"
        }
    }
}

/// Inline the local variable named at `offset` (its declaration or a use).
pub fn inline_variable(lang: Lang, text: &str, offset: usize) -> Result<Edited, String> {
    const NOT_HERE: &str = "Cannot perform refactoring. Caret should be positioned at the name of element to be inlined.";
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err(NOT_HERE.into());
    }
    let local = locals::resolve(lang, text, offset).ok_or(NOT_HERE)?;
    let tree = symbols::parse(lang, text).ok_or(NOT_HERE)?;
    let root = tree.root_node();
    let name_node = root.descendant_for_byte_range(local.decl, local.decl).ok_or(NOT_HERE)?;
    let name = name_node.utf8_text(text.as_bytes()).unwrap_or_default().to_owned();
    let (decl, init) = declaration(lang, name_node).ok_or(NOT_HERE)?;
    let init = init.ok_or_else(|| format!("Cannot perform refactoring. Variable {name} has no initializer."))?;
    if local.uses.is_empty() {
        return Err(format!("Variable {name} is never used."));
    }
    let uses: Vec<Node> = local.uses.iter().filter_map(|&u| root.descendant_for_byte_range(u, u + name.len())).collect();
    if uses.iter().any(|u| is_written(lang, *u)) {
        return Err(format!("Cannot perform refactoring. Variable {name} is accessed for writing."));
    }
    let expr = &text[init.byte_range()];
    let atom = is_atom(init.kind()) || !expr.contains(|c: char| c.is_whitespace() || "+-*/%<>=!&|?:".contains(c));
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for u in &uses {
        let r = u.byte_range();
        // `"$x"` in a Kotlin string template: `"${expr}"`.
        if lang == Lang::Kotlin && r.start > 0 && text.as_bytes()[r.start - 1] == b'$' {
            let plain = init.kind() == "simple_identifier";
            edits.push((r.start - 1..r.end, if plain { format!("${expr}") } else { format!("${{{expr}}}") }));
            continue;
        }
        let fits = u.parent().is_some_and(|p| takes_any(p.kind()) || p.kind() == "value_argument");
        edits.push((r, if atom || fits { expr.to_owned() } else { format!("({expr})") }));
    }
    // The declaration: its lines when it has them to itself.
    let line_start = text[..decl.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let mut end = decl.end_byte();
    if lang == Lang::Java && !text[..end].ends_with(';') && text[end..].starts_with(';') {
        end += 1;
    }
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let removal = if text[line_start..decl.start_byte()].trim().is_empty() && text[end..line_end].trim().is_empty() {
        line_start..(line_end + 1).min(text.len())
    } else {
        decl.start_byte()..end
    };
    edits.push((removal.clone(), String::new()));
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = text.to_owned();
    for (r, with) in &edits {
        out.replace_range(r.clone(), with);
    }
    // The caret at the first inlined use.
    let first = uses.iter().map(|u| u.start_byte()).min().unwrap_or(0);
    let caret = if first > removal.start { first - removal.len() } else { first };
    Ok(Edited { text: out, selection: caret..caret })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_kotlin_locals() {
        let text = "fun f(a: Int) {\n    val sum = a + 1\n    println(sum)\n    val twice = sum * 2\n    log(\"got $sum\")\n}\n";
        let at = text.find("sum").unwrap();
        let out = inline_variable(Lang::Kotlin, text, at).unwrap();
        assert_eq!(out.text, "fun f(a: Int) {\n    println(a + 1)\n    val twice = (a + 1) * 2\n    log(\"got ${a + 1}\")\n}\n");
        assert_eq!(&out.text[out.selection.start..out.selection.start + 5], "a + 1");
        // From a use, a plain value.
        let text = "fun g() {\n    val n = size\n    use(n, n)\n}\n";
        let out = inline_variable(Lang::Kotlin, text, text.rfind("n)").unwrap()).unwrap();
        assert_eq!(out.text, "fun g() {\n    use(size, size)\n}\n");
        // A member isn't a local.
        let text = "object O {\n    const val MAX = 1\n    fun f() = MAX\n}\n";
        assert!(inline_variable(Lang::Kotlin, text, text.find("MAX").unwrap()).unwrap_err().contains("Caret should be positioned"));
        // Written later: refused.
        let text = "fun h() {\n    var i = 0\n    i = 2\n    use(i)\n}\n";
        assert!(inline_variable(Lang::Kotlin, text, text.find("i =").unwrap()).unwrap_err().contains("accessed for writing"));
    }

    #[test]
    fn inlines_java_locals() {
        let text = "class A {\n  int f(int a) {\n    int x = a - 1;\n    return x * x;\n  }\n}\n";
        let out = inline_variable(Lang::Java, text, text.find("x =").unwrap()).unwrap();
        assert_eq!(out.text, "class A {\n  int f(int a) {\n    return (a - 1) * (a - 1);\n  }\n}\n");
        let text = "class A {\n  void g() {\n    int y;\n    y = 1;\n    use(y);\n  }\n}\n";
        assert!(inline_variable(Lang::Java, text, text.find("y;").unwrap()).is_err());
    }
}

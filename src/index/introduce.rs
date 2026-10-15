//! Introduce Variable's view of the syntax tree (Kotlin and Java): the
//! expressions at the caret IntelliJ's "Expressions" popup lists, and for
//! one of them its occurrences in the enclosing block, the statement each
//! is in, what to name it after and, in Java, its type when the
//! expression shows it.

use std::ops::Range;

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;
use crate::domain::introduce_variable::{Extraction, Literal, NameSource, Occurrence};

const NO_EXPRESSION: &str = "Cannot find an expression to introduce";
const NOT_AN_EXPRESSION: &str = "Selected block should represent an expression";
const NO_CONTAINER: &str = "Cannot refactor in this place";

fn literal(lang: Lang, node: Node, text: &str) -> Option<Literal> {
    let t = &text[node.byte_range()];
    Some(match (lang, node.kind()) {
        (_, "string_literal") => Literal::String,
        (_, "character_literal") => Literal::Char,
        (Lang::Kotlin, "boolean_literal") | (Lang::Java, "true" | "false") => Literal::Boolean,
        (Lang::Kotlin, "long_literal") => Literal::Long,
        (Lang::Kotlin, "integer_literal" | "hex_literal" | "bin_literal" | "unsigned_literal") => Literal::Int,
        (Lang::Java, "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal") => {
            if t.ends_with(['l', 'L']) { Literal::Long } else { Literal::Int }
        }
        (Lang::Kotlin, "real_literal") | (Lang::Java, "decimal_floating_point_literal" | "hex_floating_point_literal") => {
            if t.ends_with(['f', 'F']) { Literal::Float } else { Literal::Double }
        }
        _ => return None,
    })
}

/// A value expression the variable can stand for (not a callee, a
/// declared name, an assignment's target or a type).
fn is_expression(lang: Lang, node: Node, text: &str) -> bool {
    let Some(parent) = node.parent() else { return false };
    let kind = node.kind();
    if literal(lang, node, text).is_some() {
        return true;
    }
    match lang {
        Lang::Kotlin => {
            // The callee of a call is not a value: `foo` of `foo()`.
            if parent.kind() == "call_expression" && parent.named_child(0).is_some_and(|c| c.id() == node.id()) {
                return false;
            }
            match kind {
                "simple_identifier" => {
                    let named_argument = parent.kind() == "value_argument" && node.next_sibling().is_some_and(|s| s.kind() == "=");
                    !named_argument
                        && (parent.kind().ends_with("_expression")
                            || matches!(parent.kind(), "value_argument" | "statements" | "control_structure_body" | "property_declaration"))
                }
                "call_expression" | "navigation_expression" | "additive_expression" | "multiplicative_expression" | "comparison_expression"
                | "equality_expression" | "conjunction_expression" | "disjunction_expression" | "elvis_expression" | "infix_expression"
                | "range_expression" | "prefix_expression" | "postfix_expression" | "as_expression" | "check_expression"
                | "parenthesized_expression" | "indexing_expression" | "lambda_literal" | "object_literal" | "this_expression" => true,
                _ => false,
            }
        }
        Lang::Java => match kind {
            "identifier" => {
                let field = |name: &str| parent.child_by_field_name(name).is_some_and(|c| c.id() == node.id());
                match parent.kind() {
                    "binary_expression" | "unary_expression" | "argument_list" | "parenthesized_expression" | "ternary_expression" | "array_access"
                    | "return_statement" | "array_initializer" => true,
                    "field_access" | "method_invocation" => field("object"),
                    "variable_declarator" => field("value"),
                    "assignment_expression" => field("right"),
                    "cast_expression" => field("value"),
                    _ => false,
                }
            }
            "parenthesized_expression" => !matches!(parent.kind(), "if_statement" | "while_statement" | "do_statement" | "switch_expression" | "synchronized_statement"),
            "method_invocation" | "field_access" | "binary_expression" | "unary_expression" | "object_creation_expression" | "array_access"
            | "cast_expression" | "ternary_expression" | "lambda_expression" | "array_creation_expression" | "instanceof_expression"
            | "method_reference" | "this" | "null_literal" => true,
            _ => false,
        },
        _ => false,
    }
}

/// The node spanning exactly `range` that is an expression.
fn expression_node<'t>(lang: Lang, root: Node<'t>, text: &str, range: Range<usize>) -> Option<Node<'t>> {
    let mut node = root.descendant_for_byte_range(range.start, range.end)?;
    loop {
        if node.byte_range() == range && is_expression(lang, node, text) {
            return Some(node);
        }
        if node.start_byte() < range.start || node.end_byte() > range.end {
            return None;
        }
        node = node.parent()?;
    }
}

fn supported(lang: Lang) -> bool {
    matches!(lang, Lang::Kotlin | Lang::Java)
}

/// The expressions Introduce Variable can take: the selected one, or with
/// no selection the ones around the caret, innermost first (IntelliJ's
/// "Expressions" popup).
pub fn expressions_at(lang: Lang, text: &str, selection: Range<usize>) -> Result<Vec<Range<usize>>, String> {
    if !supported(lang) {
        return Err(NO_EXPRESSION.into());
    }
    let tree = symbols::parse(lang, text).ok_or(NO_EXPRESSION)?;
    let root = tree.root_node();
    if !selection.is_empty() {
        let s = &text[selection.clone()];
        let start = selection.start + (s.len() - s.trim_start().len());
        let end = selection.end - (s.len() - s.trim_end().len());
        return expression_node(lang, root, text, start..end).map(|n| vec![n.byte_range()]).ok_or_else(|| NOT_AN_EXPRESSION.into());
    }
    let mut out: Vec<Range<usize>> = Vec::new();
    // The caret right after a name still means it.
    for at in [selection.start, selection.start.saturating_sub(1)] {
        let Some(mut node) = root.descendant_for_byte_range(at, at) else { continue };
        loop {
            if is_expression(lang, node, text) && !out.contains(&node.byte_range()) {
                out.push(node.byte_range());
            }
            match node.parent() {
                Some(p) if !matches!(p.kind(), "statements" | "block" | "class_body" | "source_file" | "program") => node = p,
                _ => break,
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    if out.is_empty() { Err(NO_EXPRESSION.into()) } else { Ok(out) }
}

fn is_block(lang: Lang, node: Node) -> bool {
    match lang {
        Lang::Kotlin => node.kind() == "statements",
        _ => node.kind() == "block",
    }
}

/// The statement of `node`: its ancestor directly in a block.
fn statement_of<'t>(lang: Lang, node: Node<'t>) -> Option<Node<'t>> {
    let mut n = node;
    loop {
        let parent = n.parent()?;
        if is_block(lang, parent) {
            return Some(n);
        }
        n = parent;
    }
}

/// The expression is its statement (`foo()` on its own).
fn is_whole(lang: Lang, node: Node, statement: Node) -> bool {
    match lang {
        Lang::Kotlin => node.id() == statement.id(),
        _ => statement.kind() == "expression_statement" && statement.named_child(0).is_some_and(|c| c.id() == node.id()),
    }
}

fn normalized(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn name_source(lang: Lang, node: Node, text: &str) -> NameSource {
    let t = |n: Node| text[n.byte_range()].to_owned();
    if let Some(l) = literal(lang, node, text) {
        return NameSource::Literal(l);
    }
    // The last name in a dotted chain: `a.b.getC` → `getC`.
    let last_name = |n: Node| {
        let s = t(n);
        s.rsplit('.').next().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
    };
    let source = match (lang, node.kind()) {
        (Lang::Kotlin, "call_expression") => node.named_child(0).and_then(last_name),
        (Lang::Kotlin, "navigation_expression") | (_, "simple_identifier" | "identifier") => last_name(node),
        (Lang::Kotlin | Lang::Java, "parenthesized_expression") => {
            return node.named_child(0).map_or(NameSource::Unknown, |inner| name_source(lang, inner, text));
        }
        (Lang::Java, "method_invocation") => node.child_by_field_name("name").map(t),
        (Lang::Java, "field_access") => node.child_by_field_name("field").map(t),
        (Lang::Java, "object_creation_expression" | "cast_expression") => {
            node.child_by_field_name("type").map(t).map(|ty| ty.split('<').next().unwrap_or(&ty).rsplit('.').next().unwrap_or_default().to_owned())
        }
        _ => None,
    };
    if let Some(name) = source {
        return NameSource::Name(name);
    }
    match boolean_or_string(lang, node, text) {
        Some(l) => NameSource::Literal(l),
        None => NameSource::Unknown,
    }
}

/// Comparisons and logic are booleans; `+` with a string is a string.
fn boolean_or_string(lang: Lang, node: Node, text: &str) -> Option<Literal> {
    match node.kind() {
        "comparison_expression" | "equality_expression" | "conjunction_expression" | "disjunction_expression" | "check_expression"
        | "instanceof_expression" => Some(Literal::Boolean),
        "binary_expression" => {
            let op = node.child_by_field_name("operator").map(|o| &text[o.byte_range()])?;
            match op {
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" => Some(Literal::Boolean),
                "+" if has_string(lang, node, text) => Some(Literal::String),
                _ => None,
            }
        }
        "additive_expression" if has_string(lang, node, text) => Some(Literal::String),
        _ => None,
    }
}

fn has_string(lang: Lang, node: Node, text: &str) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter().any(|c| literal(lang, c, text) == Some(Literal::String) || boolean_or_string(lang, c, text) == Some(Literal::String))
}

fn java_type(node: Node, text: &str) -> Option<String> {
    if let Some(l) = literal(Lang::Java, node, text) {
        return Some(l.java_type().to_owned());
    }
    match node.kind() {
        "object_creation_expression" | "cast_expression" => node.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned()),
        "parenthesized_expression" => node.named_child(0).and_then(|n| java_type(n, text)),
        _ => boolean_or_string(Lang::Java, node, text).map(|l| l.java_type().to_owned()),
    }
}

/// Names declared in the function around `node` (parameters and locals):
/// the new variable must not clash with them.
fn declared_names(lang: Lang, node: Node, text: &str) -> Vec<String> {
    let mut scope = node;
    while let Some(p) = scope.parent() {
        scope = p;
        if matches!(scope.kind(), "function_declaration" | "method_declaration" | "constructor_declaration" | "anonymous_initializer" | "secondary_constructor") {
            break;
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![scope];
    while let Some(n) = stack.pop() {
        let declares = match (lang, n.kind()) {
            (Lang::Kotlin, "simple_identifier") => n.parent().is_some_and(|p| matches!(p.kind(), "variable_declaration" | "parameter" | "lambda_parameter" | "class_parameter")),
            (Lang::Java, "identifier") => n.parent().is_some_and(|p| {
                matches!(p.kind(), "variable_declarator" | "formal_parameter" | "catch_formal_parameter" | "enhanced_for_statement" | "inferred_parameters" | "lambda_expression")
                    && (p.child_by_field_name("name").is_some_and(|c| c.id() == n.id()) || matches!(p.kind(), "inferred_parameters" | "lambda_expression"))
            }),
            _ => false,
        };
        if declares {
            out.push(text[n.byte_range()].to_owned());
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    out
}

/// Introduce Variable for the expression at `range`: its occurrences in
/// the block around it, what to name it after, its Java type, and the
/// names already declared there.
pub fn extraction(lang: Lang, text: &str, range: Range<usize>) -> Result<(Extraction, Vec<String>), String> {
    let tree = symbols::parse(lang, text).ok_or(NO_EXPRESSION)?;
    let root = tree.root_node();
    let node = expression_node(lang, root, text, range).ok_or(NOT_AN_EXPRESSION)?;
    let statement = statement_of(lang, node).ok_or(NO_CONTAINER)?;
    let block = statement.parent().ok_or(NO_CONTAINER)?;
    let wanted = (node.kind(), normalized(&text[node.byte_range()]));
    let mut occurrences = Vec::new();
    let mut stack = vec![block];
    let mut found: Vec<Node> = Vec::new();
    while let Some(n) = stack.pop() {
        if n.kind() == wanted.0 && is_expression(lang, n, text) && normalized(&text[n.byte_range()]) == wanted.1 {
            found.push(n);
            continue;
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    found.sort_by_key(|n| n.start_byte());
    let mut selected = 0;
    for n in found {
        // Its statement in this block.
        let mut s = n;
        while let Some(p) = s.parent() {
            if p.id() == block.id() {
                break;
            }
            s = p;
        }
        if n.id() == node.id() {
            selected = occurrences.len();
        }
        let whole = is_whole(lang, n, s);
        occurrences.push(Occurrence { range: n.byte_range(), statement: s.byte_range(), whole });
    }
    let extraction = Extraction {
        occurrences,
        selected,
        source: name_source(lang, node, text),
        java_type: (lang == Lang::Java).then(|| java_type(node, text)).flatten(),
    };
    Ok((extraction, declared_names(lang, node, text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, needle: &str) -> Range<usize> {
        let i = text.find(needle).unwrap();
        i..i + needle.len()
    }

    #[test]
    fn lists_expressions_at_the_caret() {
        let kt = "fun f(p: Point) {\n    println(p.getX() + 1)\n}\n";
        let caret = kt.find("getX").unwrap() + 1;
        let found: Vec<&str> = expressions_at(Lang::Kotlin, kt, caret..caret).unwrap().into_iter().map(|r| &kt[r]).collect();
        assert_eq!(found, ["p.getX()", "p.getX() + 1", "println(p.getX() + 1)"]);
        let java = "class A { void f(Point p) { foo(new Bar(1), p.x); } }\n";
        let caret = java.find("Bar").unwrap();
        let found: Vec<&str> = expressions_at(Lang::Java, java, caret..caret).unwrap().into_iter().map(|r| &java[r]).collect();
        assert_eq!(found, ["new Bar(1)", "foo(new Bar(1), p.x)"]);
        // A selection must be an expression.
        assert_eq!(expressions_at(Lang::Kotlin, kt, at(kt, "p.getX() ")).unwrap(), vec![at(kt, "p.getX()")]);
        assert_eq!(expressions_at(Lang::Kotlin, kt, at(kt, "getX() +")), Err(NOT_AN_EXPRESSION.into()));
    }

    #[test]
    fn finds_occurrences_and_names() {
        let kt = "fun f(p: Point, x: Int) {\n    println(p.x + 1)\n    if (ok) show(p.x)\n    val q = 2\n}\n";
        let first = at(kt, "p.x");
        let (ex, taken) = extraction(Lang::Kotlin, kt, first.clone()).unwrap();
        assert_eq!(ex.occurrences.len(), 2);
        assert_eq!(&kt[ex.occurrences[1].statement.clone()], "if (ok) show(p.x)");
        assert_eq!(ex.source, NameSource::Name("x".into()));
        assert!(taken.contains(&"x".to_owned()) && taken.contains(&"q".to_owned()) && taken.contains(&"p".to_owned()));
        // A call on its own line is its whole statement.
        let (ex, _) = extraction(Lang::Kotlin, kt, at(kt, "println(p.x + 1)")).unwrap();
        assert!(ex.occurrences[0].whole);
        // Java types from what the expression shows.
        let java = "class A { void f() { foo(\"a\" + b, new Bar<String>(1), c > 2, d.get()); } }\n";
        let ty = |e: &str| extraction(Lang::Java, java, at(java, e)).unwrap().0.java_type;
        assert_eq!(ty("\"a\" + b").as_deref(), Some("String"));
        assert_eq!(ty("new Bar<String>(1)").as_deref(), Some("Bar<String>"));
        assert_eq!(ty("c > 2").as_deref(), Some("boolean"));
        assert_eq!(ty("d.get()"), None);
        assert_eq!(extraction(Lang::Java, java, at(java, "new Bar<String>(1)")).unwrap().0.source, NameSource::Name("Bar".into()));
        // Outside a block: nowhere to declare it.
        let field = "class A { val x = 1 + 2 }\n";
        assert_eq!(extraction(Lang::Kotlin, field, at(field, "1 + 2")).unwrap_err(), NO_CONTAINER);
    }
}

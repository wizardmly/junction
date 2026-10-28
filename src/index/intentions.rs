//! Alt+Enter intentions that read the syntax tree (Kotlin), as IntelliJ's
//! Kotlin plugin names them: Convert concatenation to template / Convert
//! template to concatenated string, Specify type explicitly / Specify
//! return type explicitly (when the expression or the declarations of
//! its calls show its type), and Add
//! names to call arguments (given the callee's parameters).

use tree_sitter::Node;

use super::extract_method::children;
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

fn innermost<'t>(root: Node<'t>, at: usize) -> Option<Node<'t>> {
    root.descendant_for_byte_range(at, at)
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(Some(node), |n| n.parent())
}

/// A non-raw string literal.
fn plain_string(node: Node, text: &str) -> bool {
    node.kind() == "string_literal" && !text[node.byte_range()].starts_with("\"\"\"")
}

/// The intentions at `caret` in a Kotlin file.
pub fn intentions(text: &str, caret: usize) -> Vec<Intention> {
    let Some(tree) = symbols::parse(Lang::Kotlin, text) else { return Vec::new() };
    let root = tree.root_node();
    let mut out = Vec::new();
    let shown = |n: Node| super::introduce::kotlin_type(n, text).filter(|t| !t.is_empty()).map(|t| (t, Vec::new()));
    out.extend(specify_type(root, text, caret, &shown).map(|(i, _)| i));
    out.extend(to_template(root, text, caret));
    out.extend(to_concatenation(root, text, caret));
    out
}

/// Specify type explicitly / Specify return type explicitly where only
/// the declarations of the calls show the type (`val a = user.getAddress()`
/// → `val a: Address = …`), with the classes to import; None where the
/// expression shows it itself (those are among `intentions`).
pub fn specify_inferred_type(text: &str, caret: usize, classes: super::catches::Classes) -> Option<(Intention, Vec<String>)> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let declared = |n: Node| {
        if super::introduce::kotlin_type(n, text).is_some_and(|t| !t.is_empty()) {
            return None;
        }
        super::types::type_of(Lang::Kotlin, text, n.byte_range(), classes).map(|t| (t.ty, t.imports))
    };
    specify_type(tree.root_node(), text, caret, &declared)
}

type Typing<'a> = dyn Fn(Node) -> Option<(String, Vec<String>)> + 'a;

/// `val x = 1` → `val x: Int = 1`; `fun f() = 1` → `fun f(): Int = 1`;
/// the type (and its imports) from `typing`.
fn specify_type(root: Node, text: &str, caret: usize, typing: &Typing) -> Option<(Intention, Vec<String>)> {
    let at = innermost(root, caret)?;
    for n in ancestors(at) {
        match n.kind() {
            "property_declaration" => {
                let parts = children(n);
                let variable = parts.iter().find(|c| c.kind() == "variable_declaration")?;
                // Typed already, or a destructuring declaration.
                if children(*variable).len() != 1 {
                    return None;
                }
                let eq = {
                    let mut cursor = n.walk();
                    n.children(&mut cursor).find(|c| c.kind() == "=")?
                };
                // On the declaration, not in its initializer.
                if caret > eq.start_byte() {
                    return None;
                }
                let init = eq.next_named_sibling()?;
                let (ty, imports) = typing(init)?;
                let end = variable.end_byte();
                return Some((Intention { title: "Specify type explicitly".into(), range: end..end, replacement: format!(": {ty}"), caret: end + 2 + ty.len() }, imports));
            }
            "function_declaration" => {
                let parts = children(n);
                let params = parts.iter().find(|c| c.kind() == "function_value_parameters")?;
                // A declared return type follows the parameters.
                if parts.iter().any(|c| c.start_byte() >= params.end_byte() && matches!(c.kind(), "user_type" | "nullable_type" | "function_type")) {
                    return None;
                }
                let body = parts.iter().find(|c| c.kind() == "function_body")?;
                if caret >= body.start_byte() || !text[body.byte_range()].starts_with('=') {
                    return None;
                }
                let expression = body.named_child(0)?;
                let (ty, imports) = typing(expression)?;
                let end = params.end_byte();
                return Some((Intention { title: "Specify return type explicitly".into(), range: end..end, replacement: format!(": {ty}"), caret: end + 2 + ty.len() }, imports));
            }
            "statements" | "class_body" | "lambda_literal" => return None,
            _ => {}
        }
    }
    None
}

/// The operands of a `+` chain (none when it has another operator).
fn plus_operands<'t>(chain: Node<'t>, text: &str, out: &mut Vec<Node<'t>>) -> bool {
    if chain.kind() != "additive_expression" {
        out.push(chain);
        return true;
    }
    let mut cursor = chain.walk();
    let all: Vec<Node> = chain.children(&mut cursor).collect();
    let [left, op, right] = all.as_slice() else { return false };
    &text[op.byte_range()] == "+" && plus_operands(*left, text, out) && plus_operands(*right, text, out)
}

/// `"a" + x + "b"` → `"a${x}b"`: a `+` chain starting with a string, so
/// every `+` in it joins strings.
fn to_template(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let at = innermost(root, caret)?;
    // The outermost `+` chain around the caret.
    let chain = ancestors(at).take_while(|n| n.kind() == "additive_expression" || n == &at || n.parent().is_some_and(|p| p.kind() == "additive_expression")).filter(|n| n.kind() == "additive_expression").last()?;
    let mut operands = Vec::new();
    if !plus_operands(chain, text, &mut operands) || !operands.first().is_some_and(|o| plain_string(*o, text)) {
        return None;
    }
    if operands.iter().any(|o| o.kind() == "string_literal" && !plain_string(*o, text)) {
        return None;
    }
    let mut body = String::new();
    // `$name` waits for what follows: a letter or digit makes it `${name}`.
    let mut pending: Option<String> = None;
    let push = |body: &mut String, pending: &mut Option<String>, piece: &str| {
        if let Some(name) = pending.take() {
            if piece.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                body.push_str(&format!("${{{name}}}"));
            } else {
                body.push('$');
                body.push_str(&name);
            }
        }
        body.push_str(piece);
    };
    for o in &operands {
        let t = &text[o.byte_range()];
        match o.kind() {
            "string_literal" => push(&mut body, &mut pending, &t[1..t.len() - 1]),
            "simple_identifier" => {
                push(&mut body, &mut pending, "");
                pending = Some(t.to_owned());
            }
            "integer_literal" | "boolean_literal" if !t.contains(['_', 'x', 'X', 'b', 'B']) => push(&mut body, &mut pending, t),
            "null" => push(&mut body, &mut pending, "null"),
            "character_literal" => {
                let inner = &t[1..t.len() - 1];
                let piece = match inner {
                    "\"" => "\\\"",
                    "$" => "\\$",
                    "\\'" => "'",
                    other => other,
                };
                push(&mut body, &mut pending, piece);
            }
            _ => push(&mut body, &mut pending, &format!("${{{t}}}")),
        }
    }
    push(&mut body, &mut pending, "");
    let replacement = format!("\"{body}\"");
    Some(Intention { title: "Convert concatenation to template".into(), range: chain.byte_range(), caret: chain.start_byte() + replacement.len(), replacement })
}

/// Whether an expression needs parentheses as an operand of `+`.
fn needs_parens(node: Node) -> bool {
    !matches!(
        node.kind(),
        "simple_identifier" | "call_expression" | "navigation_expression" | "parenthesized_expression" | "string_literal" | "integer_literal" | "real_literal" | "boolean_literal"
            | "character_literal" | "indexing_expression" | "this_expression" | "postfix_expression" | "multiplicative_expression"
    )
}

/// `"a${x}b"` → `"a" + x + "b"`.
fn to_concatenation(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let literal = ancestors(innermost(root, caret)?).find(|n| n.kind() == "string_literal")?;
    if !plain_string(literal, text) {
        return None;
    }
    let mut cursor = literal.walk();
    let entries: Vec<Node> = literal.children(&mut cursor).collect();
    if !entries.iter().any(|e| matches!(e.kind(), "interpolated_identifier" | "interpolated_expression")) {
        return None;
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut literal_run = String::new();
    let mut literal_first = None;
    for e in &entries {
        match e.kind() {
            "interpolated_identifier" | "interpolated_expression" => {
                if !literal_run.is_empty() {
                    pieces.push(format!("\"{literal_run}\""));
                    literal_run.clear();
                }
                literal_first.get_or_insert(false);
                let t = &text[e.byte_range()];
                let inner = if e.kind() == "interpolated_expression" { e.named_child(0) } else { None };
                pieces.push(if inner.is_some_and(needs_parens) { format!("({t})") } else { t.to_owned() });
            }
            "interpolation_identifier_start" | "interpolation_expression_start" | "interpolation_expression_end" | "\"" => {}
            _ => {
                literal_first.get_or_insert(true);
                literal_run.push_str(&text[e.byte_range()]);
            }
        }
    }
    if !literal_run.is_empty() {
        pieces.push(format!("\"{literal_run}\""));
    }
    // Starting with a value: `"" +` keeps the `+` a string's.
    if literal_first == Some(false) && pieces.len() < 2 || literal_first == Some(false) && !pieces[1].starts_with('"') {
        pieces.insert(0, "\"\"".into());
    }
    let replacement = pieces.join(" + ");
    Some(Intention { title: "Convert template to concatenated string".into(), range: literal.byte_range(), caret: literal.start_byte(), replacement })
}

/// A parameter's name, from its source ("x: Int = 1", "vararg xs: Int");
/// none for a vararg or a Java parameter ("int x").
fn parameter_name(param: &str) -> Option<String> {
    let (head, _) = param.split_once(':')?;
    let words: Vec<&str> = head.split_whitespace().filter(|w| !w.starts_with('@')).collect();
    if words.contains(&"vararg") {
        return None;
    }
    words.last().map(|w| w.trim_matches('`').to_owned())
}

/// Add names to call arguments: the call whose `(` is at `open`, its
/// positional arguments named after `params` (the chosen declaration's).
pub fn add_argument_names(text: &str, open: usize, params: &[String]) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let args = innermost(tree.root_node(), open).and_then(|n| ancestors(n).find(|a| a.kind() == "value_arguments" && a.start_byte() == open))?;
    let arguments: Vec<Node> = children(args).into_iter().filter(|a| a.kind() == "value_argument").collect();
    let mut edits: Vec<(usize, String)> = Vec::new();
    let mut seen_named = false;
    for (i, a) in arguments.iter().enumerate() {
        let named = a.named_child(0).is_some_and(|c| c.kind() == "simple_identifier" && c.next_sibling().is_some_and(|s| s.kind() == "="));
        if named {
            seen_named = true;
            continue;
        }
        // A positional argument after a named one, or a spread: leave it.
        if seen_named || text[a.byte_range()].starts_with('*') {
            return None;
        }
        let name = parameter_name(params.get(i)?)?;
        edits.push((a.start_byte(), format!("{name} = ")));
    }
    if edits.is_empty() {
        return None;
    }
    let range = args.byte_range();
    let mut replacement = text[range.clone()].to_owned();
    for (at, insert) in edits.iter().rev() {
        replacement.insert_str(at - range.start, insert);
    }
    Some(Intention { title: "Add names to call arguments".into(), caret: range.start, range, replacement })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        let mut out = text.to_owned();
        out.replace_range(i.range.clone(), &i.replacement);
        out
    }

    fn find(text: &str, at: &str, title: &str) -> Option<String> {
        let caret = text.find(at).unwrap();
        intentions(text, caret).into_iter().find(|i| i.title == title).map(|i| apply(text, &i))
    }

    #[test]
    fn converts_concatenation_to_a_template() {
        let text = "fun f(x: Int, name: String) {\n    val a = \"a\" + x + \"b\\n\" + name.length + 'c' + 1 + name + \"!\"\n}\n";
        assert_eq!(
            find(text, "+ x", "Convert concatenation to template").unwrap(),
            "fun f(x: Int, name: String) {\n    val a = \"a${x}b\\n${name.length}c1$name!\"\n}\n"
        );
        // A value first: maybe a number's `+`.
        let text = "fun f(x: Int) = x + \"a\"\n";
        assert_eq!(find(text, "+", "Convert concatenation to template"), None);
        let text = "fun f(x: Int) = \"$\" + '$' + x + '\"'\n";
        assert_eq!(find(text, "+ x", "Convert concatenation to template").unwrap(), "fun f(x: Int) = \"$\\$$x\\\"\"\n");
    }

    #[test]
    fn converts_a_template_to_concatenation() {
        let text = "fun f(x: Int, n: String) = \"x=$x, n=${n.length} \\$ ${x + 1}\"\n";
        assert_eq!(
            find(text, "x=", "Convert template to concatenated string").unwrap(),
            "fun f(x: Int, n: String) = \"x=\" + x + \", n=\" + n.length + \" \\$ \" + (x + 1)\n"
        );
        let text = "fun f(x: Int) = \"$x\"\n";
        assert_eq!(find(text, "$x", "Convert template to concatenated string").unwrap(), "fun f(x: Int) = \"\" + x\n");
        let text = "val s = \"plain\"\n";
        assert_eq!(find(text, "plain", "Convert template to concatenated string"), None);
    }

    #[test]
    fn specifies_types() {
        let text = "fun f() {\n    val n = 1\n    val s = \"a\" + n\n    val p = Point(1, 2)\n    val q: Int = 2\n    val u = g()\n}\n";
        assert_eq!(find(text, "n = 1", "Specify type explicitly").unwrap(), text.replace("val n = 1", "val n: Int = 1"));
        assert_eq!(find(text, "s =", "Specify type explicitly").unwrap(), text.replace("val s =", "val s: String ="));
        assert_eq!(find(text, "p =", "Specify type explicitly").unwrap(), text.replace("val p =", "val p: Point ="));
        assert_eq!(find(text, "q:", "Specify type explicitly"), None);
        assert_eq!(find(text, "u =", "Specify type explicitly"), None);
        let text = "fun twice(x: Int) = x * 2\n";
        assert_eq!(find(text, "twice", "Specify return type explicitly").unwrap(), "fun twice(x: Int): Int = x * 2\n");
        let text = "fun twice(x: Int): Int = x * 2\n";
        assert_eq!(find(text, "twice", "Specify return type explicitly"), None);        // From the declarations of the calls, with their imports.
        let user = "package com.model\n\nclass User {\n    fun address(): Address = Address()\n}\n\nclass Address\n";
        let classes = |name: &str| -> Vec<String> { if name == "User" || name == "Address" { vec![user.to_owned()] } else { Vec::new() } };
        let text = "package app\n\nimport com.model.User\n\nfun f(user: User) {\n    val a = user.address()\n    val n = 1\n}\n\nfun g(user: User) = user.address()\n";
        let at = |s: &str| text.find(s).unwrap();
        let (i, imports) = specify_inferred_type(text, at("a ="), &classes).unwrap();
        assert_eq!((i.title.as_str(), i.replacement.as_str(), imports), ("Specify type explicitly", ": Address", vec!["com.model.Address".to_owned()]));
        let (i, _) = specify_inferred_type(text, at("g(user"), &classes).unwrap();
        assert_eq!((i.title.as_str(), i.range.start), ("Specify return type explicitly", at(") = user") + 1));
        // What the expression shows is offered already.
        assert_eq!(specify_inferred_type(text, at("n = 1"), &classes), None);
    }

    #[test]
    fn adds_names_to_call_arguments() {
        let text = "fun f() {\n    g(1, \"a\", flag = true)\n}\n";
        let open = text.find('(').unwrap();
        let open = text[open + 1..].find('(').unwrap() + open + 1;
        let params = vec!["count: Int".to_owned(), "label: String = \"\"".into(), "flag: Boolean".into()];
        let i = add_argument_names(text, open, &params).unwrap();
        assert_eq!(apply(text, &i), "fun f() {\n    g(count = 1, label = \"a\", flag = true)\n}\n");
        // Java parameters and varargs have no names to use.
        assert_eq!(add_argument_names(text, open, &["int count".to_owned(), "String label".into()]), None);
        assert_eq!(add_argument_names(text, open, &["vararg xs: Int".to_owned()]), None);
    }
}

//! IntelliJ's SimplifiableCallInspection fixes (Kotlin): a lambda that only
//! does what a stdlib function does, `xs.filter { it is T }` →
//! `xs.filterIsInstance<T>()`, `xs.mapNotNull { it as? T }` → the same,
//! `xs.filter { it != null }` → `xs.filterNotNull()` and
//! `xs.flatMap { it }` → `xs.flatten()`. Titled like its fix, "Convert
//! 'filter' call to 'filterIsInstance<T>()'".

use tree_sitter::Node;

use super::catches::Classes;
use super::extract_method::children as named;
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

/// The call to simplify around the caret, on its name: `filter`,
/// `mapNotNull` or `flatMap` with only a lambda.
pub fn simplify_call(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let mut node = tree.root_node().descendant_for_byte_range(caret, caret)?;
    let (call, name, receiver) = loop {
        if node.kind() == "call_expression"
            && let [callee, _] = named(node)[..]
        {
            let (name, receiver) = match callee.kind() {
                "navigation_expression" => (callee.named_child(1).and_then(|s| named(s).into_iter().find(|c| c.kind() == "simple_identifier")), callee.named_child(0)),
                // `filter { … }` on an implicit receiver.
                "simple_identifier" => (Some(callee), None),
                _ => (None, None),
            };
            if let Some(name) = name
                && name.start_byte() <= caret
                && caret <= name.end_byte()
            {
                break (node, name, receiver);
            }
        }
        node = node.parent()?;
    };
    let function = &text[name.byte_range()];
    if !matches!(function, "filter" | "mapNotNull" | "flatMap") {
        return None;
    }
    // Not on a map (its `filter` takes entries), and `flatten` needs
    // collections of collections.
    let receiver_type = receiver.and_then(|r| super::types::type_of(Lang::Kotlin, text, r.byte_range(), classes)).map(|t| t.ty);
    let head = |ty: &str| ty.trim_end_matches('?').split('<').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_owned();
    if receiver_type.as_deref().is_some_and(|t| head(t).ends_with("Map")) {
        return None;
    }
    let [_, suffix] = named(call)[..] else { return None };
    let [annotated] = named(suffix)[..] else { return None };
    let lambda = named(annotated).into_iter().find(|c| c.kind() == "lambda_literal")?;
    let parts = named(lambda);
    let param = match parts.iter().find(|c| c.kind() == "lambda_parameters") {
        Some(p) => {
            let [declaration] = named(*p)[..] else { return None };
            let pieces = named(declaration);
            if declaration.kind() != "variable_declaration" || pieces.len() != 1 {
                return None;
            }
            text[pieces[0].byte_range()].to_owned()
        }
        None => "it".to_owned(),
    };
    let statements = parts.iter().find(|c| c.kind() == "statements")?;
    let [statement] = named(*statements)[..] else { return None };
    let is_param = |n: Node| n.kind() == "simple_identifier" && text[n.byte_range()] == *param;
    let replacement = match (function, statement.kind()) {
        // `it is T`, not `it !is T`.
        ("filter", "check_expression") => {
            let [left, ty] = named(statement)[..] else { return None };
            let operator = text[left.end_byte()..ty.start_byte()].trim();
            (is_param(left) && operator == "is").then(|| format!("filterIsInstance<{}>()", &text[ty.byte_range()]))?
        }
        // `it != null`, `null !== it`.
        ("filter", "equality_expression") => {
            let [left, right] = named(statement)[..] else { return None };
            let operator = text[left.end_byte()..right.start_byte()].trim();
            let null_check = (is_param(left) && right.kind() == "null_literal") || (left.kind() == "null_literal" && is_param(right));
            (null_check && matches!(operator, "!=" | "!==")).then(|| "filterNotNull()".to_owned())?
        }
        // `it as? T`.
        ("mapNotNull", "as_expression") => {
            let [left, ty] = named(statement)[..] else { return None };
            let operator = text[left.end_byte()..ty.start_byte()].trim();
            (is_param(left) && operator == "as?").then(|| format!("filterIsInstance<{}>()", &text[ty.byte_range()]))?
        }
        // `flatMap { it }` over collections (or arrays) of collections.
        ("flatMap", "simple_identifier") if is_param(statement) => {
            let element = receiver_type.as_deref().and_then(super::lambdas::collection_element)?;
            super::lambdas::collection_element(&element)?;
            if head(receiver_type.as_deref()?) == "Array" && head(&element) != "Array" || head(receiver_type.as_deref()?) == "Sequence" {
                return None;
            }
            "flatten()".to_owned()
        }
        _ => return None,
    };
    Some(Intention {
        title: format!("Convert '{function}' call to '{replacement}'"),
        range: name.start_byte()..call.end_byte(),
        replacement,
        caret: name.start_byte(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(text: &str, at: &str) -> Option<(String, String)> {
        let classes = |_: &str| Vec::new();
        simplify_call(text, text.find(at).unwrap() + 1, &classes).map(|i| (i.title.clone(), format!("{}{}{}", &text[..i.range.start], i.replacement, &text[i.range.end..])))
    }

    #[test]
    fn simplifies_calls() {
        assert_eq!(
            convert("fun f(xs: List<Any>) = xs.filter { it is String }", "filter"),
            Some(("Convert 'filter' call to 'filterIsInstance<String>()'".into(), "fun f(xs: List<Any>) = xs.filterIsInstance<String>()".into()))
        );
        assert_eq!(convert("fun List<Any>.f() = filter { x -> x is Int }", "filter").unwrap().1, "fun List<Any>.f() = filterIsInstance<Int>()");
        assert_eq!(convert("fun f(xs: List<String?>) = xs.filter { null != it }", "filter").unwrap().1, "fun f(xs: List<String?>) = xs.filterNotNull()");
        assert_eq!(convert("fun f(xs: List<Any>) = xs.mapNotNull { it as? String }", "mapNotNull").unwrap().1, "fun f(xs: List<Any>) = xs.filterIsInstance<String>()");
        assert_eq!(convert("fun f(xs: List<List<Int>>) = xs.flatMap { it }", "flatMap").unwrap().1, "fun f(xs: List<List<Int>>) = xs.flatten()");
        // `!is`, a map, more than the check, an unknown or flat receiver: none.
        assert_eq!(convert("fun f(xs: List<Any>) = xs.filter { it !is String }", "filter"), None);
        assert_eq!(convert("fun f(m: Map<String?, String?>) = m.filter { it != null }", "filter"), None);
        assert_eq!(convert("fun f(xs: List<Any>) = xs.filter { it is String && ok }", "filter"), None);
        assert_eq!(convert("fun f(xs: List<Int>) = xs.flatMap { it }", "flatMap"), None);
    }
}

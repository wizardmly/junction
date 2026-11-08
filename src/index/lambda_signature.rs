//! IntelliJ's Specify explicit lambda signature and Remove explicit lambda
//! parameter types (Kotlin): `xs.map { it.name }` ↔
//! `xs.map { it: User -> it.name }`. The parameter types come from the
//! call the lambda is passed to, for the standard library's collection and
//! scope functions whose receiver type the index can tell.

use tree_sitter::Node;

use super::catches::Classes;
use super::extract_method::children as named;
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

/// The lambda whose braces or parameter list hold the caret (not its body).
fn lambda_at<'t>(root: Node<'t>, caret: usize) -> Option<Node<'t>> {
    let mut node = root.descendant_for_byte_range(caret, caret)?;
    loop {
        if node.kind() == "lambda_literal" {
            let body = named(node).into_iter().find(|c| c.kind() == "statements");
            let on_head = body.is_none_or(|b| caret <= b.start_byte());
            let on_close = caret + 1 >= node.end_byte();
            return (on_head || on_close).then_some(node);
        }
        node = node.parent()?;
    }
}

/// The types of the lambda's parameters, from the call it is passed to.
fn expected_parameters(lambda: Node, text: &str, classes: Classes) -> Option<Vec<String>> {
    let mut up = lambda.parent()?;
    if up.kind() == "annotated_lambda" {
        up = up.parent()?;
    } else if up.kind() == "value_argument" {
        up = up.parent().filter(|p| p.kind() == "value_arguments")?.parent()?;
    }
    let call = up.parent().filter(|c| up.kind() == "call_suffix" && c.kind() == "call_expression")?;
    let callee = call.named_child(0)?;
    if callee.kind() != "navigation_expression" {
        return None;
    }
    let name = named(callee.named_child(1)?).into_iter().find(|c| c.kind() == "simple_identifier")?;
    let receiver = callee.named_child(0)?;
    let ty = super::types::type_of(Lang::Kotlin, text, receiver.byte_range(), classes)?.ty;
    let element = || super::lambdas::collection_element(&ty);
    Some(match &text[name.byte_range()] {
        "forEach" | "map" | "mapNotNull" | "filter" | "filterNot" | "any" | "all" | "none" | "find" | "findLast" | "first" | "firstOrNull" | "last" | "lastOrNull" | "single" | "singleOrNull" | "onEach" | "flatMap" | "sortedBy" | "sortedByDescending" | "groupBy" | "associateBy"
        | "associateWith" | "associate" | "count" | "sumOf" | "takeWhile" | "dropWhile" | "takeLastWhile" | "dropLastWhile" | "partition" | "maxBy" | "minBy" | "maxByOrNull" | "minByOrNull" | "maxOf" | "minOf" | "distinctBy" | "indexOfFirst" | "indexOfLast" => vec![element()?],
        "forEachIndexed" | "mapIndexed" | "mapIndexedNotNull" | "filterIndexed" | "onEachIndexed" => vec!["Int".into(), element()?],
        // `x?.let { }` gets the non-null type.
        "let" | "also" | "takeIf" | "takeUnless" => vec![ty.trim_end_matches('?').to_owned()],
        _ => return None,
    })
}

/// Specify explicit lambda signature: `{ it.name }` → `{ it: User -> it.name }`,
/// `{ a, b -> … }` → `{ a: Int, b: User -> … }`, `{ (x, y) -> … }` →
/// `{ (x, y): Point -> … }`; the caret on the braces or the parameters.
pub fn specify_signature(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let lambda = lambda_at(tree.root_node(), caret)?;
    let parts = named(lambda);
    let parameters = parts.iter().find(|c| c.kind() == "lambda_parameters");
    let types = expected_parameters(lambda, text, classes)?;
    let title = "Specify explicit lambda signature".to_owned();
    match parameters {
        None => {
            let [ty] = &types[..] else { return None };
            // `{ it: T -> ` after the brace, keeping a line break there.
            let after = lambda.start_byte() + 1;
            let rest = &text[after..];
            let end = after + rest.len() - rest.trim_start_matches([' ', '\t']).len();
            let newline = text[end..].starts_with('\n') || text[end..].starts_with("\r\n");
            Some(Intention { title, range: lambda.start_byte()..end, replacement: format!("{{ it: {ty} ->{}", if newline { "" } else { " " }), caret: lambda.start_byte() })
        }
        Some(p) => {
            // A destructuring's type follows it in the list itself.
            let all = named(*p);
            if all.iter().any(|c| c.kind().ends_with("type")) {
                return None;
            }
            let declarations: Vec<Node> = all.into_iter().filter(|c| c.kind().ends_with("declaration")).collect();
            if declarations.len() != types.len() {
                return None;
            }
            // Already typed: nothing to add.
            let typed = |d: &Node| named(*d).iter().any(|c| c.kind().ends_with("type"));
            if declarations.iter().any(typed) || declarations.iter().any(|d| d.kind() == "multi_variable_declaration" && named(*d).iter().any(typed)) {
                return None;
            }
            let mut out = String::new();
            let mut at = p.start_byte();
            for (d, ty) in declarations.iter().zip(&types) {
                out.push_str(&text[at..d.end_byte()]);
                out.push_str(&format!(": {ty}"));
                at = d.end_byte();
            }
            Some(Intention { title, range: p.start_byte()..at, replacement: out, caret: p.start_byte() })
        }
    }
}

/// Remove explicit lambda parameter types (may break code):
/// `{ a: Int, b: String -> … }` → `{ a, b -> … }`.
pub fn remove_parameter_types(text: &str, caret: usize) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let lambda = lambda_at(tree.root_node(), caret)?;
    let parameters = named(lambda).into_iter().find(|c| c.kind() == "lambda_parameters")?;
    let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
    // `(a, b): Pair<…>`: the type after a destructuring.
    let list = named(parameters);
    for (i, c) in list.iter().enumerate().skip(1) {
        if c.kind().ends_with("type") {
            cuts.push(list[i - 1].end_byte()..c.end_byte());
        }
    }
    let mut stack: Vec<Node> = list.into_iter().filter(|c| c.kind().ends_with("declaration")).collect();
    while let Some(d) = stack.pop() {
        let pieces = named(d);
        if let Some(ty) = pieces.iter().find(|c| c.kind().ends_with("type")) {
            // From the end of what precedes the type (the name, or the
            // destructuring's `)`) to the type's end.
            let before = pieces.iter().take_while(|c| c.id() != ty.id()).last().map_or(d.start_byte(), |c| c.end_byte());
            cuts.push(before..ty.end_byte());
        }
        if d.kind() == "multi_variable_declaration" {
            stack.extend(pieces.into_iter().filter(|c| c.kind() == "variable_declaration"));
        }
    }
    if cuts.is_empty() {
        return None;
    }
    cuts.sort_by_key(|r| r.start);
    let mut out = String::new();
    let mut at = parameters.start_byte();
    for cut in &cuts {
        out.push_str(&text[at..cut.start]);
        at = cut.end;
    }
    out.push_str(&text[at..parameters.end_byte()]);
    Some(Intention { title: "Remove explicit lambda parameter types (may break code)".into(), range: parameters.byte_range(), replacement: out, caret: parameters.start_byte() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        format!("{}{}{}", &text[..i.range.start], i.replacement, &text[i.range.end..])
    }

    #[test]
    fn specifies_signature() {
        let classes = |_: &str| Vec::new();
        let specify = |text: &str, at: usize| specify_signature(text, at, &classes).map(|i| apply(text, &i));
        let text = "fun f(xs: List<User>) = xs.map {it.name}";
        assert_eq!(specify(text, text.find('{').unwrap()).unwrap(), "fun f(xs: List<User>) = xs.map { it: User -> it.name}");
        let text = "fun f(xs: List<User>) = xs.forEachIndexed { i, u -> show(i, u) }";
        assert_eq!(specify(text, text.find("i,").unwrap()).unwrap(), "fun f(xs: List<User>) = xs.forEachIndexed { i: Int, u: User -> show(i, u) }");
        let text = "fun f(xs: List<Pair<Int, String>>) = xs.forEach { (a, b) -> show(a) }";
        assert_eq!(specify(text, text.find("(a").unwrap()).unwrap(), "fun f(xs: List<Pair<Int, String>>) = xs.forEach { (a, b): Pair<Int, String> -> show(a) }");
        let text = "fun f(u: User) = u.let {\n    it.name\n}";
        assert_eq!(specify(text, text.find('{').unwrap()).unwrap(), "fun f(u: User) = u.let { it: User ->\n    it.name\n}");
        // The caret in the body, an unknown call or receiver, already typed: none.
        let text = "fun f(xs: List<User>) = xs.map { it.name }";
        assert_eq!(specify(text, text.find("name").unwrap()), None);
        let text = "fun f(xs: List<User>) = xs.whatever { it.name }";
        assert_eq!(specify(text, text.find('{').unwrap()), None);
        let text = "fun f(xs: List<User>) = xs.map { u: User -> u.name }";
        assert_eq!(specify(text, text.find('{').unwrap()), None);
    }

    #[test]
    fn removes_parameter_types() {
        let remove = |text: &str| remove_parameter_types(text, text.find('{').unwrap()).map(|i| (i.title.clone(), apply(text, &i)));
        let text = "val f = { a: Int, b: String -> a }";
        assert_eq!(remove(text).unwrap(), ("Remove explicit lambda parameter types (may break code)".to_owned(), "val f = { a, b -> a }".to_owned()));
        assert_eq!(remove("val f = xs.forEach { (a, b): Pair<Int, Int> -> a }").unwrap().1, "val f = xs.forEach { (a, b) -> a }");
        assert_eq!(remove("val f = xs.map { a -> a }"), None);
    }
}

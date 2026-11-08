//! Alt+Enter intentions between Kotlin's scope functions, as IntelliJ's
//! ConvertScopeFunctionToReceiver and ConvertScopeFunctionToParameter:
//! `x.let { it.foo() }` ↔ `x.run { foo() }` and `x.also { it.a = 1 }` ↔
//! `x.apply { a = 1 }` (each pair returns the same value). Turning the
//! receiver into a parameter needs the receiver type's members, from the
//! declarations the index finds.

use tree_sitter::Node;

use super::catches::Classes;
use super::extract_method::{children as named, descendants};
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

/// The scope function call around the caret, on its name: the call, the
/// function's name node, the receiver expression and the lambda.
fn scope_call<'t>(root: Node<'t>, text: &str, caret: usize, names: &[&str]) -> Option<(Node<'t>, Node<'t>, Node<'t>, Node<'t>)> {
    let mut node = root.descendant_for_byte_range(caret, caret)?;
    loop {
        if node.kind() == "call_expression" {
            let parts = named(node);
            if let [callee, suffix] = parts[..]
                && callee.kind() == "navigation_expression"
            {
                let name = callee.named_child(1).and_then(|s| named(s).into_iter().find(|c| c.kind() == "simple_identifier"));
                if let Some(name) = name
                    && names.contains(&&text[name.byte_range()])
                    // On the function's name.
                    && name.start_byte() <= caret
                    && caret <= name.end_byte()
                {
                    let suffix_parts = named(suffix);
                    let [annotated] = suffix_parts[..] else { return None };
                    let lambda = named(annotated).into_iter().find(|c| c.kind() == "lambda_literal")?;
                    return Some((node, name, callee.named_child(0)?, lambda));
                }
            }
        }
        node = node.parent()?;
    }
}

/// Every node under `node` outside nested lambdas, and whether any nested
/// lambda refers to `name` (or `this`).
fn outside_nested<'t>(node: Node<'t>, text: &str, name: &str) -> (Vec<Node<'t>>, bool) {
    let mut out = Vec::new();
    let mut nested_uses = false;
    let mut stack: Vec<Node> = named(node);
    while let Some(n) = stack.pop() {
        if n.kind() == "lambda_literal" {
            nested_uses |= descendants(n).iter().any(|d| d.kind() == "this_expression" || d.kind() == "simple_identifier" && &text[d.byte_range()] == name);
            continue;
        }
        out.push(n);
        stack.extend(named(n));
    }
    (out, nested_uses)
}

/// The names declared in the function around `node` (its parameters and
/// locals) and the class's: an implicit receiver's member of the same
/// name loses to a local, and `name` alone would mean the local.
fn names_around(node: Node, text: &str) -> Vec<String> {
    let scope = std::iter::successors(Some(node), |n| n.parent()).find(|n| matches!(n.kind(), "function_declaration" | "class_body" | "source_file"));
    let Some(scope) = scope else { return Vec::new() };
    descendants(scope)
        .into_iter()
        .filter(|d| matches!(d.kind(), "variable_declaration" | "parameter" | "class_parameter"))
        .filter_map(|d| named(d).into_iter().find(|c| c.kind() == "simple_identifier").map(|n| text[n.byte_range()].to_owned()))
        .collect()
}

/// Convert to 'run' / 'apply' (IntelliJ's ConvertScopeFunctionToReceiver):
/// `x.let { it.foo(it.a) }` → `x.run { foo(a) }`, `x.also { it.a = 1 }` →
/// `x.apply { a = 1 }`; `it` alone becomes `this`, and a member whose name
/// a local around also has keeps `this.`. Not when the lambda already uses
/// `this` (it would change meaning) or a nested lambda uses the parameter.
pub fn to_receiver(text: &str, caret: usize) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (_, name, _, lambda) = scope_call(tree.root_node(), text, caret, &["let", "also"])?;
    let function = &text[name.byte_range()];
    let target = if function == "let" { "run" } else { "apply" };
    let parts = named(lambda);
    let parameters = parts.iter().find(|c| c.kind() == "lambda_parameters");
    let param = match parameters {
        Some(p) => {
            let [declaration] = named(*p)[..] else { return None };
            // `{ x: Foo -> … }` or a destructuring: left as it is.
            if declaration.kind() != "variable_declaration" || named(declaration).len() != 1 {
                return None;
            }
            text[declaration.byte_range()].trim().to_owned()
        }
        None => "it".to_owned(),
    };
    let statements = parts.iter().find(|c| c.kind() == "statements");
    let (nodes, nested) = statements.map(|s| outside_nested(*s, text, &param)).unwrap_or_default();
    if nested || nodes.iter().any(|n| n.kind() == "this_expression") {
        return None;
    }
    let around = names_around(lambda, text);
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for n in nodes.iter().filter(|n| matches!(n.kind(), "simple_identifier" | "interpolated_identifier") && text[n.byte_range()] == *param) {
        // `"$it"` → `"$this"`.
        if n.kind() == "interpolated_identifier" {
            edits.push((n.byte_range(), "this".into()));
            continue;
        }
        let parent = n.parent()?;
        // A label, a named argument or a member called the same: not the parameter.
        if parent.kind() == "navigation_suffix" || parent.kind() == "value_argument" && named(parent).len() == 2 && named(parent)[0].id() == n.id() {
            continue;
        }
        // `it.foo` and, assigned to, `it.foo = …`.
        let receiver_of = matches!(parent.kind(), "navigation_expression" | "directly_assignable_expression") && parent.named_child(0).is_some_and(|c| c.id() == n.id());
        if receiver_of && parent.named_child(1).is_some_and(|s| s.kind() == "navigation_suffix") {
            let suffix = parent.named_child(1)?;
            let suffix_text = &text[suffix.byte_range()];
            if suffix_text.trim_start().starts_with("?.") {
                return None;
            }
            let member = named(suffix).into_iter().find(|c| c.kind() == "simple_identifier").map(|m| &text[m.byte_range()]);
            let keep_this = member.is_some_and(|m| around.iter().any(|a| a == m));
            // `it.foo` → `foo`: the receiver and the dot go.
            let dot_end = suffix.start_byte() + suffix_text.find('.')? + 1;
            edits.push((n.start_byte()..dot_end, if keep_this { "this.".into() } else { String::new() }));
        } else {
            edits.push((n.byte_range(), "this".into()));
        }
    }
    // The parameter list and its arrow go.
    if let Some(p) = parameters {
        let arrow = text[p.end_byte()..].find("->")? + p.end_byte() + 2;
        let end = arrow + text[arrow..].len() - text[arrow..].trim_start_matches([' ', '\t']).len();
        edits.push((p.start_byte()..end, String::new()));
    }
    edits.push((name.byte_range(), target.to_owned()));
    Some(combined(text, edits, &format!("Convert to '{target}'"), name.start_byte()))
}

/// The members a Kotlin class declares (properties, functions, the
/// primary constructor's `val`s / `var`s), from `source`.
fn class_members(source: &str, class_name: &str) -> Option<Vec<String>> {
    let tree = symbols::parse(Lang::Kotlin, source)?;
    let class = descendants(tree.root_node())
        .into_iter()
        .find(|n| matches!(n.kind(), "class_declaration" | "object_declaration") && named(*n).iter().any(|c| c.kind() == "type_identifier" && &source[c.byte_range()] == class_name))?;
    let mut out = Vec::new();
    for part in named(class) {
        match part.kind() {
            "primary_constructor" => {
                for p in descendants(part).into_iter().filter(|p| p.kind() == "class_parameter" && named(*p).iter().any(|c| c.kind() == "binding_pattern_kind")) {
                    out.extend(named(p).into_iter().find(|c| c.kind() == "simple_identifier").map(|n| source[n.byte_range()].to_owned()));
                }
            }
            "class_body" | "enum_class_body" => {
                for m in named(part) {
                    let declared = match m.kind() {
                        "function_declaration" => named(m).into_iter().find(|c| c.kind() == "simple_identifier"),
                        "property_declaration" => named(m).into_iter().find(|c| c.kind() == "variable_declaration").and_then(|v| named(v).into_iter().find(|c| c.kind() == "simple_identifier")),
                        _ => None,
                    };
                    out.extend(declared.map(|n| source[n.byte_range()].to_owned()));
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// Convert to 'let' / 'also' (IntelliJ's ConvertScopeFunctionToParameter):
/// `x.run { foo(a) }` → `x.let { it.foo(it.a) }`, `x.apply { a = 1 }` →
/// `x.also { it.a = 1 }`; `this` becomes `it`. The receiver's members come
/// from its type's declaration (the index's); not offered when the type
/// isn't known, the lambda already uses `it`, or a nested lambda uses the
/// receiver.
pub fn to_parameter(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (_, name, receiver, lambda) = scope_call(tree.root_node(), text, caret, &["run", "apply"])?;
    let function = &text[name.byte_range()];
    let target = if function == "run" { "let" } else { "also" };
    let parts = named(lambda);
    if parts.iter().any(|c| c.kind() == "lambda_parameters") {
        return None;
    }
    let ty = super::types::type_of(Lang::Kotlin, text, receiver.byte_range(), classes).map(|t| t.ty)?;
    let simple = ty.trim_end_matches('?').split('<').next()?.rsplit('.').next()?.trim().to_owned();
    let members = std::iter::once(text.to_owned()).chain(classes(&simple)).find_map(|s| class_members(&s, &simple))?;
    let statements = parts.iter().find(|c| c.kind() == "statements");
    let Some(statements) = statements else { return None };
    let (nodes, _) = outside_nested(*statements, text, "it");
    if descendants(*statements).iter().any(|d| matches!(d.kind(), "simple_identifier" | "interpolated_identifier") && &text[d.byte_range()] == "it") {
        return None;
    }
    // Names declared around or inside the lambda win over the receiver's members.
    let mut locals = names_around(lambda, text);
    locals.extend(
        descendants(*statements)
            .into_iter()
            .filter(|d| matches!(d.kind(), "variable_declaration" | "parameter"))
            .filter_map(|d| named(d).into_iter().find(|c| c.kind() == "simple_identifier").map(|n| text[n.byte_range()].to_owned())),
    );
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // Nested lambdas aren't looked into: one that uses the receiver stops it.
    let nested_use = descendants(*statements).iter().filter(|d| d.kind() == "lambda_literal").any(|l| {
        descendants(*l).iter().any(|d| d.kind() == "this_expression" || d.kind() == "simple_identifier" && is_implicit_member(*d, text, &members, &locals))
    });
    if nested_use {
        return None;
    }
    for n in &nodes {
        match n.kind() {
            "this_expression" => {
                if text[n.byte_range()].contains('@') {
                    return None;
                }
                edits.push((n.byte_range(), "it".into()));
            }
            "simple_identifier" if is_implicit_member(*n, text, &members, &locals) => edits.push((n.start_byte()..n.start_byte(), "it.".into())),
            // `"$name"` → `"${it.name}"`.
            "interpolated_identifier" if is_implicit_member(*n, text, &members, &locals) => {
                let dollar = n.prev_sibling().filter(|p| p.kind() == "interpolation_identifier_start")?;
                edits.push((dollar.start_byte()..n.end_byte(), format!("${{it.{}}}", &text[n.byte_range()])));
            }
            _ => {}
        }
    }
    edits.push((name.byte_range(), target.to_owned()));
    Some(combined(text, edits, &format!("Convert to '{target}'"), name.start_byte()))
}

/// A bare `name` or `name(…)` meaning the implicit receiver's member.
fn is_implicit_member(n: Node, text: &str, members: &[String], locals: &[String]) -> bool {
    let name = &text[n.byte_range()];
    if !members.iter().any(|m| m == name) || locals.iter().any(|l| l == name) {
        return false;
    }
    let Some(parent) = n.parent() else { return false };
    match parent.kind() {
        // `a.name`, a declaration, a type, a label.
        "navigation_suffix" | "variable_declaration" | "parameter" | "function_declaration" | "class_parameter" | "user_type" | "label" => false,
        // `f(name = 1)`: the argument's name.
        "value_argument" => !(named(parent).len() == 2 && named(parent)[0].id() == n.id()),
        // `name.x` / `name(…)` / `name = 1`: the receiver's.
        _ => true,
    }
}

/// One edit from several over `text` (each range its own), from the
/// first to the last.
fn combined(text: &str, mut edits: Vec<(std::ops::Range<usize>, String)>, title: &str, caret: usize) -> Intention {
    edits.sort_by_key(|(r, _)| r.start);
    let start = edits.first().map_or(caret, |(r, _)| r.start);
    let end = edits.iter().map(|(r, _)| r.end).max().unwrap_or(caret);
    let mut out = String::new();
    let mut at = start;
    for (range, new) in &edits {
        out.push_str(&text[at..range.start]);
        out.push_str(new);
        at = range.end;
    }
    out.push_str(&text[at..end]);
    Intention { title: title.into(), range: start..end, replacement: out, caret }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        format!("{}{}{}", &text[..i.range.start], i.replacement, &text[i.range.end..])
    }

    #[test]
    fn let_and_also_to_receiver() {
        let convert = |text: &str, at: &str| to_receiver(text, text.find(at).unwrap() + 1).map(|i| (i.title.clone(), apply(text, &i)));
        let text = "fun f(u: User) = u.let { it.greet(it.name) }";
        assert_eq!(convert(text, "let").unwrap(), ("Convert to 'run'".to_owned(), "fun f(u: User) = u.run { greet(name) }".to_owned()));
        let text = "fun f(u: User) = u.also { it.age = 3; log(it) }";
        assert_eq!(convert(text, "also").unwrap().1, "fun f(u: User) = u.apply { age = 3; log(this) }");
        let text = "fun f(u: User) = u.also { user -> user.age = 3 }";
        assert_eq!(convert(text, "also").unwrap().1, "fun f(u: User) = u.apply { age = 3 }");
        // A local with the member's name keeps `this.`.
        let text = "fun f(u: User, name: String) = u.let { it.name == name }";
        assert_eq!(convert(text, "let").unwrap().1, "fun f(u: User, name: String) = u.run { this.name == name }");
        // `this` inside, a nested lambda using `it`, the caret off the name: none.
        assert_eq!(convert("class A { fun f(u: User) = u.let { it.x(this) } }", "let"), None);
        assert_eq!(convert("fun f(u: User) = u.let { xs.map { it.y } }", "let"), None);
        assert_eq!(convert("fun f(u: User) = u.let { it.x() }", "{ it"), None);
        let text = "fun f(u: User) = u.let { log(\"$it\") }";
        assert_eq!(convert(text, "let").unwrap().1, "fun f(u: User) = u.run { log(\"$this\") }");
    }

    #[test]
    fn run_and_apply_to_parameter() {
        let classes = |n: &str| if n == "User" { vec!["class User(val name: String) {\n    var age = 0\n    fun greet(s: String) {}\n}\n".to_owned()] } else { Vec::new() };
        let convert = |text: &str, at: &str| to_parameter(text, text.find(at).unwrap() + 1, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let text = "fun f(u: User) = u.run { greet(name) }";
        assert_eq!(convert(text, "run").unwrap(), ("Convert to 'let'".to_owned(), "fun f(u: User) = u.let { it.greet(it.name) }".to_owned()));
        let text = "fun f(u: User) = u.apply { age = 3; log(this) }";
        assert_eq!(convert(text, "apply").unwrap().1, "fun f(u: User) = u.also { it.age = 3; log(it) }");
        // A local or parameter named like a member is the local.
        let text = "fun f(u: User, name: String) = u.run { greet(name) }";
        assert_eq!(convert(text, "run").unwrap().1, "fun f(u: User, name: String) = u.let { it.greet(name) }");
        let text = "fun f(u: User) = u.run { log(\"$name ${age}\") }";
        assert_eq!(convert(text, "run").unwrap().1, "fun f(u: User) = u.let { log(\"${it.name} ${it.age}\") }");
        // Unknown type, `it` already used, a nested lambda using a member: none.
        assert_eq!(convert("fun f(u: Other) = u.run { greet(name) }", "run"), None);
        assert_eq!(convert("fun f(u: User, xs: List<Int>) = xs.forEach { u.run { greet(\"$it\") } }", "run"), None);
        assert_eq!(convert("fun f(u: User, xs: List<Int>) = u.run { xs.forEach { greet(\"a\") } }", "run"), None);
    }
}

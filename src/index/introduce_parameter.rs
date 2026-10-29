//! Introduce Parameter's view of the syntax tree (Kotlin and Java): the
//! expression and the function it's in, the function's parameters and the
//! members it uses (a local variable can't go along), its occurrences in
//! the function, and the function's calls in a file.

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::{children, descendants, inside, is_function, is_reference, java_decls, kotlin_decls, kotlin_type};
use super::lang::Lang;
use super::symbols;
use crate::domain::introduce_parameter::{CallSite, ParameterPlan};
use crate::domain::introduce_variable::NameSource;

const NOT_AN_EXPRESSION: &str = "Selected block should represent an expression";
const NO_FUNCTION: &str = "Cannot introduce parameter: the expression is not in a function";

fn params_node<'t>(lang: Lang, function: Node<'t>) -> Option<Node<'t>> {
    match lang {
        Lang::Kotlin => children(function).into_iter().find(|c| c.kind() == "function_value_parameters"),
        _ => function.child_by_field_name("parameters"),
    }
}

fn function_name(lang: Lang, function: Node, text: &str) -> String {
    match lang {
        Lang::Kotlin => children(function).into_iter().find(|c| c.kind() == "simple_identifier").map(|c| text[c.byte_range()].to_owned()).unwrap_or_default(),
        _ => function.child_by_field_name("name").map(|c| text[c.byte_range()].to_owned()).unwrap_or_default(),
    }
}

fn normalized(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Introduce Parameter for the selection; the plan and where the name
/// comes from.
pub fn plan(lang: Lang, text: &str, selection: Range<usize>) -> Result<(ParameterPlan, NameSource), String> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err("Introduce Parameter works in Kotlin and Java files".into());
    }
    let tree = symbols::parse(lang, text).ok_or(NOT_AN_EXPRESSION)?;
    let root = tree.root_node();
    // The selected expression, or the innermost one at the caret.
    let selected = &text[selection.clone()];
    let range = if selected.trim().is_empty() {
        super::introduce::expressions_at(lang, text, selection.clone())?.into_iter().next().ok_or(NOT_AN_EXPRESSION)?
    } else {
        let start = selection.start + (selected.len() - selected.trim_start().len());
        start..selection.start + selected.trim_end().len()
    };
    let node = super::introduce::expression_node(lang, root, text, range).ok_or(NOT_AN_EXPRESSION)?;
    let mut function = node;
    while !is_function(lang, function) {
        function = function.parent().ok_or(NO_FUNCTION)?;
    }
    let params_node = params_node(lang, function).ok_or(NO_FUNCTION)?;
    if inside(&node.byte_range(), &params_node.byte_range()) {
        return Err(NO_FUNCTION.into());
    }
    let decls = match lang {
        Lang::Kotlin => kotlin_decls(function, text),
        _ => java_decls(function, text),
    };
    // The function's own parameters, in order.
    let is_param = |d: &super::extract_method::Decl| d.scope == function.byte_range() && inside(&(d.at..d.at), &params_node.byte_range());
    let params: Vec<&super::extract_method::Decl> = decls.iter().filter(|d| is_param(d)).collect();
    let resolve = |name: &str, at: usize| decls.iter().filter(|d| d.name == name && d.at <= at && inside(&(at..at), &d.scope)).max_by_key(|d| d.at);
    let mut uses = Vec::new();
    let mut members = Vec::new();
    let base = node.start_byte();
    for n in descendants(node).into_iter().filter(|n| is_reference(lang, *n, text)) {
        let name = text[n.byte_range()].trim_start_matches('$');
        let rel = n.start_byte() - base..n.end_byte() - base;
        match resolve(name, n.start_byte()) {
            Some(d) if is_param(d) => uses.push((rel, params.iter().position(|p| p.name == d.name).unwrap_or(0))),
            Some(d) if !inside(&(d.at..d.at), &node.byte_range()) => return Err(format!("Cannot introduce parameter: the expression uses local variable '{}'", d.name)),
            Some(_) => {}
            // A member (a field, a method of the class); classes and constants are left alone.
            None if name.starts_with(|c: char| c.is_lowercase()) => members.push(rel),
            None => {}
        }
    }
    // Its occurrences in the function body.
    let wanted = (node.kind(), normalized(&text[node.byte_range()]));
    let mut occurrences: Vec<Range<usize>> = descendants(function)
        .into_iter()
        .filter(|n| !inside(&n.byte_range(), &params_node.byte_range()) && n.kind() == wanted.0 && normalized(&text[n.byte_range()]) == wanted.1)
        .map(|n| n.byte_range())
        .collect();
    occurrences.sort_by_key(|r| r.start);
    occurrences.dedup_by(|a, b| inside(a, b));
    let selected_ix = occurrences.iter().position(|r| *r == node.byte_range()).unwrap_or(0);
    let ty = match lang {
        Lang::Kotlin => kotlin_type(node, text, &|name| params.iter().find(|p| p.name == name).map(|p| p.ty.clone()).unwrap_or_default()),
        _ => super::introduce::java_type(node, text).or_else(|| (uses.len() == 1 && node.kind() == "identifier").then(|| params[uses[0].1].ty.clone())).unwrap_or_default(),
    };
    let plan = ParameterPlan {
        function: function_name(lang, function, text),
        expression: text[node.byte_range()].to_owned(),
        occurrences,
        selected: selected_ix,
        params_close: params_node.end_byte() - 1,
        params: params.iter().map(|p| p.name.clone()).collect(),
        uses,
        members,
        ty,
        taken: decls.iter().map(|d| d.name.clone()).collect(),
    };
    Ok((plan, super::introduce::name_source(lang, node, text)))
}

/// Keeps only the calls of the function whose name is on `line` of the
/// file `declaration`, not of another function of its name (another
/// class's method), as far as the index tells: what Change Signature,
/// Inline Function and Introduce Parameter update.
pub struct OurCalls {
    pub index: Option<std::sync::Arc<std::sync::RwLock<super::store::ProjectIndex>>>,
    pub declaration: String,
    pub line: u32,
    pub name: String,
}

impl OurCalls {
    /// The declarations overriding it, or that it overrides (path, line, column).
    pub fn overrides(&self) -> Vec<(String, u32, u32)> {
        let Some(index) = self.index.as_ref().and_then(|i| i.read().ok()) else { return Vec::new() };
        super::nav::SameSymbol::new(&index, &self.declaration, self.line, &self.name).map(|mut same| same.overrides()).unwrap_or_default()
    }

    pub fn keep(&self, path: &str, text: &str, calls: Vec<CallSite>) -> Vec<CallSite> {
        let Some(index) = self.index.as_ref().and_then(|i| i.read().ok()) else { return calls };
        let Some(mut same) = super::nav::SameSymbol::new(&index, &self.declaration, self.line, &self.name) else { return calls };
        calls.into_iter().filter(|c| same.other_at(path, text, c.name_range.start).is_none()).collect()
    }
}

/// A Java type's simple name: `A` in `A`, `A<B>`, `p.A`.
fn type_name(ty: Node) -> Option<Node> {
    match ty.kind() {
        "type_identifier" => Some(ty),
        "generic_type" => children(ty).into_iter().next().and_then(type_name),
        "scoped_type_identifier" => children(ty).into_iter().filter(|c| c.kind() == "type_identifier").last(),
        _ => None,
    }
}

/// A Kotlin call's `(…)` arguments: the name when named, and the value.
fn kotlin_arguments(args: Node, text: &str) -> Vec<(Option<String>, String)> {
    children(args)
        .into_iter()
        .filter(|a| a.kind() == "value_argument")
        .map(|a| {
            let parts = children(a);
            let named = parts.first().filter(|p| p.kind() == "simple_identifier" && p.next_sibling().is_some_and(|s| s.kind() == "=")).map(|p| text[p.byte_range()].to_owned());
            (named, parts.last().map(|p| text[p.byte_range()].to_owned()).unwrap_or_default())
        })
        .collect()
}

/// The calls of `function` in a file, with `params` arguments (Kotlin's
/// may leave defaults out).
pub fn call_sites(lang: Lang, text: &str, function: &str, params: usize) -> Vec<CallSite> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let mut out = Vec::new();
    for n in descendants(tree.root_node()) {
        let site = match lang {
            Lang::Kotlin if n.kind() == "call_expression" => {
                let named = children(n);
                let (Some(callee), Some(suffix)) = (named.first(), named.iter().find(|c| c.kind() == "call_suffix")) else { continue };
                let (name_node, receiver) = match callee.kind() {
                    "simple_identifier" => (*callee, None),
                    "navigation_expression" => {
                        let parts = children(*callee);
                        let (Some(target), Some(last)) = (parts.first(), parts.last()) else { continue };
                        let Some(id) = children(*last).into_iter().find(|c| c.kind() == "simple_identifier") else { continue };
                        let receiver = text[target.byte_range()].to_owned();
                        (id, (receiver != "this").then_some(receiver))
                    }
                    _ => continue,
                };
                let name = &text[name_node.byte_range()];
                let parts = children(*suffix);
                let args = parts.iter().find(|c| c.kind() == "value_arguments");
                let lambda = parts.iter().find(|c| c.kind() == "annotated_lambda").map(|l| l.byte_range());
                // `run { … }`: no parentheses, the lambda alone.
                let (args_open, args_close) = match (args, &lambda) {
                    (Some(a), _) => (a.start_byte(), a.end_byte() - 1),
                    (None, Some(_)) => {
                        let end = parts.iter().find(|c| c.kind() == "type_arguments").map_or(callee.end_byte(), |t| t.end_byte());
                        (end, end)
                    }
                    (None, None) => continue,
                };
                let values = args.map(|a| kotlin_arguments(*a, text)).unwrap_or_default();
                let passed = values.len() + usize::from(lambda.is_some());
                let lambda_text = lambda.clone().map(|l| text[l].to_owned()).unwrap_or_default();
                (name == function && passed <= params && (lambda.is_none() || params > 0)).then(|| CallSite { name_range: name_node.byte_range(), args_open, args_close, args: values, receiver, lambda, lambda_text, reference: false })
            }
            // A Kotlin constructor's calls besides `A(…)`: a supertype's
            // `: A(…)`, and `: this(…)` / `: super(…)` from a secondary constructor.
            Lang::Kotlin if matches!(n.kind(), "constructor_invocation" | "constructor_delegation_call") => {
                let Some(args) = children(n).into_iter().find(|c| c.kind() == "value_arguments") else { continue };
                let name = if n.kind() == "constructor_invocation" {
                    children(n).into_iter().find(|c| c.kind() == "user_type").and_then(|t| children(t).into_iter().filter(|c| c.kind() == "type_identifier").last())
                } else {
                    let mut cursor = n.walk();
                    let keyword = n.children(&mut cursor).find(|c| matches!(c.kind(), "this" | "super"));
                    let class = std::iter::successors(n.parent(), |p| p.parent()).find(|p| p.kind() == "class_declaration");
                    let named = class.and_then(|c| match keyword.map(|k| k.kind()) {
                        Some("this") => children(c).into_iter().find(|t| t.kind() == "type_identifier"),
                        _ => children(c)
                            .into_iter()
                            .filter(|d| d.kind() == "delegation_specifier")
                            .flat_map(children)
                            .find(|i| i.kind() == "constructor_invocation")
                            .and_then(|i| children(i).into_iter().find(|t| t.kind() == "user_type"))
                            .and_then(|t| children(t).into_iter().filter(|c| c.kind() == "type_identifier").last()),
                    });
                    named.filter(|c| &text[c.byte_range()] == function).and(keyword)
                };
                let Some(name) = name.filter(|c| matches!(c.kind(), "this" | "super") || &text[c.byte_range()] == function) else { continue };
                let values = kotlin_arguments(args, text);
                (values.len() <= params).then(|| CallSite { name_range: name.byte_range(), args_open: args.start_byte(), args_close: args.end_byte() - 1, args: values, ..Default::default() })
            }
            // References: Kotlin's `::name` / `Type::name`, Java's `Type::name`.
            Lang::Kotlin if n.kind() == "callable_reference" => {
                let Some(id) = children(n).into_iter().filter(|c| c.kind() == "simple_identifier").last() else { continue };
                (&text[id.byte_range()] == function).then(|| CallSite { name_range: id.byte_range(), reference: true, ..Default::default() })
            }
            Lang::Java if n.kind() == "method_reference" => {
                let Some(id) = children(n).into_iter().filter(|c| c.kind() == "identifier").last().filter(|id| id.start_byte() > n.start_byte()) else { continue };
                (&text[id.byte_range()] == function).then(|| CallSite { name_range: id.byte_range(), reference: true, ..Default::default() })
            }
            Lang::Java if n.kind() == "method_invocation" => {
                let (Some(name), Some(args)) = (n.child_by_field_name("name"), n.child_by_field_name("arguments")) else { continue };
                let receiver = n.child_by_field_name("object").map(|o| text[o.byte_range()].to_owned()).filter(|r| r != "this");
                let values: Vec<(Option<String>, String)> = children(args).into_iter().filter(|a| !a.kind().ends_with("comment")).map(|a| (None, text[a.byte_range()].to_owned())).collect();
                (&text[name.byte_range()] == function && values.len() == params).then(|| CallSite { name_range: name.byte_range(), args_open: args.start_byte(), args_close: args.end_byte() - 1, args: values, receiver, ..Default::default() })
            }
            // A constructor's calls: `new A(…)` (`new A<>(…)`, `new p.A(…)`),
            // and `this(…)` / `super(…)` from the class or a subclass.
            Lang::Java if matches!(n.kind(), "object_creation_expression" | "explicit_constructor_invocation") => {
                let Some(args) = n.child_by_field_name("arguments") else { continue };
                let name = if n.kind() == "object_creation_expression" {
                    n.child_by_field_name("type").and_then(type_name).filter(|t| &text[t.byte_range()] == function)
                } else {
                    let Some(keyword) = n.child_by_field_name("constructor") else { continue };
                    let class = std::iter::successors(n.parent(), |p| p.parent()).find(|p| matches!(p.kind(), "class_declaration" | "enum_declaration" | "record_declaration"));
                    let named = class.and_then(|c| match keyword.kind() {
                        "this" => c.child_by_field_name("name"),
                        _ => c.child_by_field_name("superclass").and_then(|s| children(s).into_iter().next()).and_then(type_name),
                    });
                    named.is_some_and(|c| &text[c.byte_range()] == function).then_some(keyword)
                };
                let Some(name) = name else { continue };
                let values: Vec<(Option<String>, String)> = children(args).into_iter().filter(|a| !a.kind().ends_with("comment")).map(|a| (None, text[a.byte_range()].to_owned())).collect();
                (values.len() == params).then(|| CallSite { name_range: name.byte_range(), args_open: args.start_byte(), args_close: args.end_byte() - 1, args: values, receiver: None, ..Default::default() })
            }
            _ => None,
        };
        out.extend(site);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_kotlin() {
        let text = "class Cart(val rate: Double) {\n    fun total(price: Double, count: Int): Double {\n        val tax = 2\n        return price * rate + price * rate\n    }\n    fun use() = total(1.0, 2) + Cart(1.0).total(count = 3, price = 4.0)\n}\n";
        let at = text.find("price * rate").unwrap();
        let (plan, _) = plan(Lang::Kotlin, text, at..at + 12).unwrap();
        assert_eq!((plan.function.as_str(), plan.params.clone(), plan.occurrences.len(), plan.ty.as_str()), ("total", vec!["price".to_owned(), "count".into()], 2, ""));
        assert_eq!((plan.uses.clone(), plan.members.clone()), (vec![(0..5, 0)], vec![8..12]));
        assert_eq!(&text[plan.params_close..plan.params_close + 1], ")");
        // A local can't go along.
        let tax = text.find("tax").unwrap();
        let line = text.find("price * rate + ").unwrap();
        let with_local = text.replacen("price * rate + price", "price * tax + price", 1);
        let at = with_local.find("price * tax").unwrap();
        assert!(super::plan(Lang::Kotlin, &with_local, at..at + 11).unwrap_err().contains("local variable 'tax'"));
        let _ = (tax, line);
        // The calls: positional, and named on another object.
        let calls = call_sites(Lang::Kotlin, text, "total", 2);
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].args.len(), calls[0].receiver.clone()), (2, None));
        assert_eq!((calls[1].args[1].0.as_deref(), calls[1].receiver.as_deref()), (Some("price"), Some("Cart(1.0)")));
    }

    #[test]
    fn plans_java() {
        let text = "class A {\n  int rate;\n  int total(int price) { return price * 2; }\n  int use() { return total(3) + other.total(4); }\n}\n";
        let at = text.find("price * 2").unwrap();
        let (plan, _) = plan(Lang::Java, text, at..at + 9).unwrap();
        assert_eq!((plan.params.clone(), plan.uses.clone()), (vec!["price".to_owned()], vec![(0..5, 0)]));
        let calls = call_sites(Lang::Java, text, "total", 1);
        assert_eq!(calls.iter().map(|c| c.receiver.clone()).collect::<Vec<_>>(), [None, Some("other".to_owned())]);
    }

    #[test]
    fn finds_java_constructor_calls() {
        let text = "class Box<T> extends Base<T> {\n  Box(int a) { super(a); }\n  Box() { this(1); }\n  Object make() { return List.of(new Box<>(2), new p.Box(3), new Box(4, 5), new Other(6)); }\n}\nclass Sub extends Box<String> { Sub() { super(7); } }\n";
        let args = |function: &str| call_sites(Lang::Java, text, function, 1).iter().map(|c| c.args[0].1.clone()).collect::<Vec<_>>();
        assert_eq!(args("Box"), ["1", "2", "3", "7"]);
        assert_eq!(args("Base"), ["a"]);
        let this = call_sites(Lang::Java, text, "Box", 1).remove(0);
        assert_eq!(&text[this.name_range], "this");
    }
}

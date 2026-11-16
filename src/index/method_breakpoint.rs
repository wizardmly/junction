//! Which lines take a method breakpoint, as IntelliJ's gutter decides: a
//! click on a Kotlin / Java method's declaration line, with no code of the
//! method on it, sets a method breakpoint instead of a line breakpoint.

use tree_sitter::Node;

use super::extract_method::{children, descendants, is_function};
use super::lang::Lang;
use super::symbols;
use crate::domain::debugger::MethodHeader;

/// The method declared on a 0-based line: its JVM name (`<init>` for a
/// constructor) and its 1-based first and last lines. An interface's or
/// abstract method (no body) is one too: its breakpoint stops in the
/// implementations.
pub fn method_header(lang: Lang, text: &str, line: u32) -> Option<MethodHeader> {
    if let Some((name, _)) = abstract_method(lang, text, line) {
        return Some(MethodHeader { name, line: line + 1, end: line + 1 });
    }
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    descendants(tree.root_node()).into_iter().filter(|n| is_function(lang, *n)).find_map(|function| {
        let (name, body) = match lang {
            Lang::Kotlin => (children(function).into_iter().find(|c| c.kind() == "simple_identifier")?, children(function).into_iter().find(|c| c.kind() == "function_body")?),
            _ => (function.child_by_field_name("name")?, function.child_by_field_name("body")?),
        };
        if name.start_position().row as u32 != line {
            return None;
        }
        // A block body whose code starts on a later line; an expression
        // body (`fun f() = …`) is code on the line.
        if !text[body.byte_range()].starts_with('{') {
            return None;
        }
        let first = descendants(body).into_iter().skip(1).find(|n| !n.kind().ends_with("comment"));
        if first.is_some_and(|n| n.start_position().row as u32 == line) {
            return None;
        }
        let name = if function.kind() == "constructor_declaration" { "<init>".to_owned() } else { text[name.byte_range()].to_owned() };
        Some(MethodHeader { name, line: line + 1, end: function.end_position().row as u32 + 1 })
    })
}

/// A method without a body (an interface's, or abstract) whose name is on
/// a 0-based line: its name and the name's offset.
pub fn abstract_method(lang: Lang, text: &str, line: u32) -> Option<(String, usize)> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    descendants(tree.root_node()).into_iter().filter(|n| is_function(lang, *n) && n.kind() != "constructor_declaration").find_map(|function| {
        let (name, body) = match lang {
            Lang::Kotlin => (children(function).into_iter().find(|c| c.kind() == "simple_identifier")?, children(function).into_iter().find(|c| c.kind() == "function_body")),
            _ => (function.child_by_field_name("name")?, function.child_by_field_name("body")),
        };
        // Kotlin's `external fun` has no body either, and no implementations.
        let external = lang == Lang::Kotlin && text[function.byte_range()].split(" fun ").next().is_some_and(|m| m.split_whitespace().any(|w| w == "external"));
        let native = lang == Lang::Java && text[function.byte_range()].split('(').next().is_some_and(|m| m.split_whitespace().any(|w| w == "native"));
        (body.is_none() && !external && !native && name.start_position().row as u32 == line).then(|| (text[name.byte_range()].to_owned(), name.start_byte()))
    })
}

/// The field declared on a 0-based line: a Java field, or a Kotlin
/// property of a class or file (its backing field has its name).
pub fn field_at(lang: Lang, text: &str, line: u32) -> Option<String> {
    fields(lang, text).into_iter().find(|(_, l)| *l == line).map(|(name, _)| name)
}

/// The 0-based line declaring a field (as [`field_at`] finds them) by name.
pub fn field_line(lang: Lang, text: &str, name: &str) -> Option<u32> {
    fields(lang, text).into_iter().find(|(n, _)| n == name).map(|(_, line)| line)
}

/// The fields a file declares and their 0-based lines.
fn fields(lang: Lang, text: &str) -> Vec<(String, u32)> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Vec::new();
    }
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    descendants(tree.root_node()).into_iter().filter_map(|n| {
        let name = match (lang, n.kind()) {
            (Lang::Java, "field_declaration") => n.child_by_field_name("declarator")?.child_by_field_name("name")?,
            (Lang::Kotlin, "property_declaration") => {
                // Not a local variable: in a class body or at the top level.
                let mut up = n.parent()?;
                while up.kind() == "statements" {
                    up = up.parent()?;
                }
                if !matches!(up.kind(), "class_body" | "source_file") {
                    return None;
                }
                let decl = children(n).into_iter().find(|c| c.kind() == "variable_declaration")?;
                children(decl).into_iter().find(|c| c.kind() == "simple_identifier")?
            }
            _ => return None,
        };
        Some((text[name.byte_range()].to_owned(), name.start_position().row as u32))
    })
    .collect()
}

/// A type's simple name: `Adder` in `Adder`, `a.Adder`, `Adder<T>`, `Adder?`.
fn simple_type(ty: &str) -> &str {
    let ty = ty.split('<').next().unwrap_or(ty).trim().trim_end_matches('?');
    ty.rsplit('.').next().unwrap_or(ty)
}

/// The functions (and Java constructors) of a file with a parameter of
/// type `interface`: their names and the parameters' positions, where a
/// lambda passed implements the interface.
pub fn taking(lang: Lang, text: &str, interface: &str) -> Vec<(String, usize)> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let mut out = Vec::new();
    for function in descendants(tree.root_node()).into_iter().filter(|n| is_function(lang, *n)) {
        let (name, types): (Option<Node>, Vec<String>) = match lang {
            Lang::Kotlin => (
                children(function).into_iter().find(|c| c.kind() == "simple_identifier"),
                children(function)
                    .into_iter()
                    .find(|c| c.kind() == "function_value_parameters")
                    .map(|ps| children(ps).into_iter().filter(|p| p.kind() == "parameter").map(|p| children(p).last().map(|t| text[t.byte_range()].to_owned()).unwrap_or_default()).collect())
                    .unwrap_or_default(),
            ),
            _ => (
                function.child_by_field_name("name"),
                function
                    .child_by_field_name("parameters")
                    .map(|ps| children(ps).into_iter().filter(|p| p.kind() == "formal_parameter").map(|p| p.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned()).unwrap_or_default()).collect())
                    .unwrap_or_default(),
            ),
        };
        let Some(name) = name else { continue };
        for (i, ty) in types.iter().enumerate() {
            if simple_type(ty) == interface {
                out.push((text[name.byte_range()].to_owned(), i));
            }
        }
    }
    out
}

/// The lambdas of a file implementing `interface`, as far as the syntax
/// tells (IntelliJ's functional expression search): the value of a
/// variable, field or cast of that type, returned from a function
/// returning it, a Kotlin SAM constructor `Adder { … }`, or passed where
/// `taking` (from [`taking`] over the project) says a parameter has the
/// type. Their 1-based first and last lines.
pub fn lambdas(lang: Lang, text: &str, interface: &str, taking: &[(String, usize)]) -> Vec<(u32, u32)> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let named = |n: Node| text[n.byte_range()].to_owned();
    let passed = |name: &str, i: usize| taking.iter().any(|(n, at)| n == name && *at == i);
    let is_type = |ty: Option<Node>| ty.is_some_and(|t| simple_type(&text[t.byte_range()]) == interface);
    let lines = |n: Node| (n.start_position().row as u32 + 1, n.end_position().row as u32 + 1);
    let mut out = Vec::new();
    for lambda in descendants(tree.root_node()) {
        let implements = match (lang, lambda.kind()) {
            (Lang::Java, "lambda_expression") => {
                let mut at = lambda;
                while let Some(p) = at.parent().filter(|p| p.kind() == "parenthesized_expression") {
                    at = p;
                }
                let Some(parent) = at.parent() else { continue };
                match parent.kind() {
                    "variable_declarator" => is_type(parent.parent().and_then(|d| d.child_by_field_name("type"))),
                    "cast_expression" => is_type(parent.child_by_field_name("type")),
                    "return_statement" => {
                        let mut p = parent.parent();
                        while let Some(x) = p.filter(|x| !matches!(x.kind(), "method_declaration" | "lambda_expression")) {
                            p = x.parent();
                        }
                        is_type(p.filter(|x| x.kind() == "method_declaration").and_then(|m| m.child_by_field_name("type")))
                    }
                    "argument_list" => {
                        let i = children(parent).into_iter().filter(|c| c.is_named() && !c.kind().ends_with("comment")).position(|c| c.id() == at.id()).unwrap_or(usize::MAX);
                        parent.parent().is_some_and(|call| match call.kind() {
                            "method_invocation" => call.child_by_field_name("name").is_some_and(|n| passed(&named(n), i)),
                            "object_creation_expression" => call.child_by_field_name("type").is_some_and(|t| passed(simple_type(&text[t.byte_range()]), i)),
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            (Lang::Kotlin, "lambda_literal") => {
                let Some(parent) = lambda.parent() else { continue };
                // `f(…) { … }` or `Adder { … }`: the call and the lambda's position.
                let (suffix, i) = match parent.kind() {
                    "annotated_lambda" => {
                        let Some(suffix) = parent.parent().filter(|s| s.kind() == "call_suffix") else { continue };
                        let before = children(suffix).into_iter().find(|c| c.kind() == "value_arguments").map_or(0, |a| children(a).into_iter().filter(|c| c.kind() == "value_argument").count());
                        (suffix, before)
                    }
                    "value_argument" => {
                        let Some(arguments) = parent.parent() else { continue };
                        let Some(suffix) = arguments.parent().filter(|s| s.kind() == "call_suffix") else { continue };
                        (suffix, children(arguments).into_iter().filter(|c| c.kind() == "value_argument").position(|c| c.id() == parent.id()).unwrap_or(usize::MAX))
                    }
                    _ => continue,
                };
                let Some(callee) = suffix.parent().filter(|c| c.kind() == "call_expression").and_then(|c| c.child(0)) else { continue };
                // `x.f { … }`: the name after the last dot.
                let name = match callee.kind() {
                    "navigation_expression" => descendants(callee).into_iter().filter(|n| n.kind() == "simple_identifier").last().map(named).unwrap_or_default(),
                    _ => named(callee),
                };
                (name == interface && parent.kind() == "annotated_lambda") || passed(&name, i)
            }
            _ => false,
        };
        if implements {
            out.push(lines(lambda));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lambdas_implementing_an_interface() {
        let java = "class Main {\n  static int run(int x, Adder op) { return op.add(x, 1); }\n  Adder field = (a, b) -> a + b;\n  static Adder make() {\n    return (a, b) -> {\n      return a * b;\n    };\n  }\n  void m() {\n    Adder local = ((a, b) -> a - b);\n    Runnable r = () -> {};\n    run(2, (a, b) -> a);\n    run((a, b) -> b, 3);\n    Object o = (Adder) (a, b) -> 0;\n  }\n}\n";
        let taking = taking(Lang::Java, java, "Adder");
        assert_eq!(taking, [("run".to_owned(), 1)]);
        assert_eq!(lambdas(Lang::Java, java, "Adder", &taking), [(3, 3), (5, 7), (10, 10), (12, 12), (14, 14)]);
        let kt = "fun interface Adder { fun add(a: Int, b: Int): Int }\nfun twice(x: Int, op: Adder?) = op?.add(x, x)\nfun main() {\n    val a = Adder { x, y -> x + y }\n    twice(1) { x, y ->\n        x * y\n    }\n    calc.twice(1, { x, _ -> x })\n    listOf(1).map { it }\n    twice({ x, _ -> x }, 1)\n}\n";
        let taking = super::taking(Lang::Kotlin, kt, "Adder");
        assert_eq!(taking, [("twice".to_owned(), 1)]);
        assert_eq!(lambdas(Lang::Kotlin, kt, "Adder", &taking), [(4, 4), (5, 7), (8, 8)]);
    }

    #[test]
    fn finds_declaration_lines() {
        let kt = "class Cart {\n    fun total(price: Double): Double {\n        return price\n    }\n    fun one() = 1\n    fun inline() { println() }\n}\n";
        assert_eq!(method_header(Lang::Kotlin, kt, 1), Some(MethodHeader { name: "total".into(), line: 2, end: 4 }));
        assert_eq!(method_header(Lang::Kotlin, kt, 2), None);
        assert_eq!(method_header(Lang::Kotlin, kt, 4), None);
        assert_eq!(method_header(Lang::Kotlin, kt, 5), None);
        let java = "class A {\n  A() {\n    super();\n  }\n  int total(int p)\n  {\n    return p;\n  }\n}\n";
        assert_eq!(method_header(Lang::Java, java, 1).map(|h| h.name), Some("<init>".to_owned()));
        assert_eq!(method_header(Lang::Java, java, 4), Some(MethodHeader { name: "total".into(), line: 5, end: 8 }));
        // Methods without a body.
        let iface = "interface Shape {\n    double area(double scale);\n    default int sides() {\n        return 0;\n    }\n    native void n();\n}\n";
        assert_eq!(abstract_method(Lang::Java, iface, 1), Some(("area".to_owned(), iface.find("area").unwrap())));
        assert_eq!(method_header(Lang::Java, iface, 1), Some(MethodHeader { name: "area".into(), line: 2, end: 2 }));
        assert_eq!(abstract_method(Lang::Java, iface, 2), None);
        assert_eq!(abstract_method(Lang::Java, iface, 5), None);
        let kt = "interface Shape {\n    fun area(scale: Double): Double\n    fun sides(): Int = 0\n}\nabstract class Base {\n    abstract fun name(): String\n    external fun n()\n}\n";
        assert_eq!(abstract_method(Lang::Kotlin, kt, 1).map(|(n, _)| n).as_deref(), Some("area"));
        assert_eq!(abstract_method(Lang::Kotlin, kt, 2), None);
        assert_eq!(abstract_method(Lang::Kotlin, kt, 5).map(|(n, _)| n).as_deref(), Some("name"));
        assert_eq!(abstract_method(Lang::Kotlin, kt, 6), None);
    }

    #[test]
    fn finds_fields() {
        let kt = "val top = 1\nclass Cart(val rate: Double) {\n    var total = 0.0\n    fun f() {\n        val local = 2\n    }\n}\n";
        assert_eq!(field_at(Lang::Kotlin, kt, 0).as_deref(), Some("top"));
        assert_eq!(field_at(Lang::Kotlin, kt, 2).as_deref(), Some("total"));
        assert_eq!(field_at(Lang::Kotlin, kt, 4), None);
        let java = "class A {\n  private int count = 0;\n  void f() { int x = 1; }\n}\n";
        assert_eq!(field_at(Lang::Java, java, 1).as_deref(), Some("count"));
        assert_eq!(field_at(Lang::Java, java, 2), None);
        assert_eq!((field_line(Lang::Java, java, "count"), field_line(Lang::Java, java, "x")), (Some(1), None));
        assert_eq!(field_line(Lang::Kotlin, kt, "total"), Some(2));
    }
}

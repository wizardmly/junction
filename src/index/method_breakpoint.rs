//! Which lines take a method breakpoint, as IntelliJ's gutter decides: a
//! click on a Kotlin / Java method's declaration line, with no code of the
//! method on it, sets a method breakpoint instead of a line breakpoint.

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
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    descendants(tree.root_node()).into_iter().find_map(|n| {
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
        (name.start_position().row as u32 == line).then(|| text[name.byte_range()].to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}

//! Which lines take a method breakpoint, as IntelliJ's gutter decides: a
//! click on a Kotlin / Java method's declaration line, with no code of the
//! method on it, sets a method breakpoint instead of a line breakpoint.

use super::extract_method::{children, descendants, is_function};
use super::lang::Lang;
use super::symbols;
use crate::domain::debugger::MethodHeader;

/// The method declared on a 0-based line: its JVM name (`<init>` for a
/// constructor) and its 1-based first and last lines.
pub fn method_header(lang: Lang, text: &str, line: u32) -> Option<MethodHeader> {
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
    }
}

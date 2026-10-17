//! Safe Delete's (Alt+Delete) view of the syntax tree: the declaration
//! whose name is at the caret (a class, function, property or field, or a
//! local), what kind it is, and the lines deleting it removes (with a doc
//! comment right above it).

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::children;
use super::lang::Lang;
use super::symbols;
use crate::domain::line_edit::declaration_lines;

/// A declaration to delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    /// "Function", "Class", "Property", … as IntelliJ's messages name it.
    pub kind: &'static str,
    pub name_offset: usize,
    /// Its lines (and its doc comment's), as deleting it removes them.
    pub lines: Range<usize>,
}

fn named<'t>(node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    children(node).into_iter().find(|c| kinds.contains(&c.kind()))
}

/// The declaration node a declaration kind names, and its name node.
fn declaration<'t>(lang: Lang, n: Node<'t>) -> Option<(&'static str, Node<'t>)> {
    match (lang, n.kind()) {
        (Lang::Kotlin, "function_declaration") => Some(("Function", named(n, &["simple_identifier"])?)),
        (Lang::Kotlin, "class_declaration") => Some(("Class", named(n, &["type_identifier"])?)),
        (Lang::Kotlin, "object_declaration") => Some(("Object", named(n, &["type_identifier"])?)),
        (Lang::Kotlin, "type_alias") => Some(("Type alias", named(n, &["type_identifier"])?)),
        (Lang::Kotlin, "property_declaration") => Some(("Property", named(named(n, &["variable_declaration"])?, &["simple_identifier"])?)),
        (Lang::Java, "method_declaration") => Some(("Method", n.child_by_field_name("name")?)),
        (Lang::Java, "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration") => Some(("Class", n.child_by_field_name("name")?)),
        // One declarator per declaration only: `int a, b;` would lose both.
        (Lang::Java, "field_declaration" | "local_variable_declaration") => {
            let declarators: Vec<Node> = children(n).into_iter().filter(|c| c.kind() == "variable_declarator").collect();
            let kind = if n.kind() == "field_declaration" { "Field" } else { "Variable" };
            (declarators.len() == 1).then(|| declarators[0].child_by_field_name("name")).flatten().map(|name| (kind, name))
        }
        _ => None,
    }
}

/// The declaration whose name is at `offset`.
pub fn declaration_at(lang: Lang, text: &str, offset: usize) -> Option<Declaration> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
    while let Some(n) = node {
        if let Some((kind, name)) = declaration(lang, n) {
            if !(name.start_byte() <= offset && offset <= name.end_byte()) {
                return None;
            }
            // A doc comment right above goes with it.
            let start = match n.prev_sibling() {
                Some(c) if c.kind().contains("comment") && text[c.end_byte()..n.start_byte()].matches('\n').count() <= 1 => c.start_byte(),
                _ => n.start_byte(),
            };
            let kind = if kind == "Property" && n.parent().is_some_and(|p| p.kind() == "statements") { "Variable" } else { kind };
            return Some(Declaration { name: text[name.byte_range()].to_owned(), kind, name_offset: name.start_byte(), lines: declaration_lines(text, start..n.end_byte()) });
        }
        node = n.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_declarations_to_delete() {
        let text = "class Cart {\n    /** The total. */\n    fun total() = 1\n\n    val rate = 2\n}\n";
        let d = declaration_at(Lang::Kotlin, text, text.find("total()").unwrap()).unwrap();
        assert_eq!((d.name.as_str(), d.kind), ("total", "Function"));
        assert_eq!(&text[d.lines.clone()], "    /** The total. */\n    fun total() = 1\n\n");
        let d = declaration_at(Lang::Kotlin, text, text.find("rate").unwrap()).unwrap();
        assert_eq!((d.kind, &text[d.lines]), ("Property", "\n    val rate = 2\n"));
        // Not on a name: nothing.
        assert!(declaration_at(Lang::Kotlin, text, text.find("= 1").unwrap()).is_none());
        let java = "class A {\n  int x;\n  void f() { }\n}\n";
        assert_eq!(declaration_at(Lang::Java, java, java.find("x;").unwrap()).unwrap().kind, "Field");
        assert_eq!(declaration_at(Lang::Java, java, java.find("A").unwrap()).unwrap().kind, "Class");
    }
}

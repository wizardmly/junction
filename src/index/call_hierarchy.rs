//! Call Hierarchy's (Ctrl+Alt+H) view of a file: the function a call is
//! in, IntelliJ's node for that caller (`Cart.use()`), with where its name
//! is so its own callers can be looked up.

use super::extract_method::{children, descendants, is_function};
use super::lang::Lang;
use super::symbols;

/// A function: its display name (`Cart.total`), its name's offset and
/// 0-based line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub name: String,
    pub offset: usize,
    pub line: u32,
    pub col: u32,
}

fn function_name<'t>(lang: Lang, function: tree_sitter::Node<'t>) -> Option<tree_sitter::Node<'t>> {
    match lang {
        Lang::Kotlin => children(function).into_iter().find(|c| c.kind() == "simple_identifier"),
        _ => function.child_by_field_name("name"),
    }
}

/// The class around a node: its name.
fn class_of(lang: Lang, node: tree_sitter::Node, text: &str) -> Option<String> {
    let mut at = node.parent();
    while let Some(n) = at {
        let class = match (lang, n.kind()) {
            (Lang::Kotlin, "class_declaration" | "object_declaration") => children(n).into_iter().find(|c| c.kind() == "type_identifier"),
            (Lang::Java, "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration") => n.child_by_field_name("name"),
            _ => None,
        };
        if let Some(c) = class {
            return Some(text[c.byte_range()].to_owned());
        }
        at = n.parent();
    }
    None
}

/// The innermost function holding `offset`; with none, the class (a field
/// initializer) or the file.
pub fn caller_at(lang: Lang, text: &str, offset: usize, file_name: &str) -> Option<Caller> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    let root = tree.root_node();
    let function = descendants(root)
        .into_iter()
        .filter(|n| is_function(lang, *n) && n.start_byte() <= offset && offset < n.end_byte())
        .min_by_key(|n| n.end_byte() - n.start_byte());
    let Some(function) = function else {
        let leaf = root.descendant_for_byte_range(offset, offset)?;
        let name = class_of(lang, leaf, text).unwrap_or_else(|| file_name.to_owned());
        return Some(Caller { name, offset, line: leaf.start_position().row as u32, col: leaf.start_position().column as u32 });
    };
    let name = function_name(lang, function)?;
    let plain = text[name.byte_range()].to_owned();
    let display = match class_of(lang, function, text) {
        Some(class) => format!("{class}.{plain}"),
        None => plain,
    };
    Some(Caller { name: display, offset: name.start_byte(), line: name.start_position().row as u32, col: name.start_position().column as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_calling_function() {
        let text = "class Cart {\n    fun total() = 1\n    fun use() {\n        println(total())\n    }\n}\nfun top() = Cart().total()\n";
        let at = text.find("total())").unwrap();
        let c = caller_at(Lang::Kotlin, text, at, "Cart.kt").unwrap();
        assert_eq!((c.name.as_str(), c.line, &text[c.offset..c.offset + 3]), ("Cart.use", 2, "use"));
        let at = text.rfind("total").unwrap();
        assert_eq!(caller_at(Lang::Kotlin, text, at, "Cart.kt").unwrap().name, "top");
        let java = "class A {\n  int x = f();\n  void g() { f(); }\n}\n";
        assert_eq!(caller_at(Lang::Java, java, java.find("f();").unwrap(), "A.java").unwrap().name, "A");
        assert_eq!(caller_at(Lang::Java, java, java.rfind("f()").unwrap(), "A.java").unwrap().name, "A.g");
    }
}

/// The byte offset of a 0-based line and UTF-16 column.
pub fn offset_of(text: &str, line: u32, col: u32) -> usize {
    let start = if line == 0 { 0 } else { text.match_indices('\n').nth(line as usize - 1).map_or(text.len(), |(i, _)| i + 1) };
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units >= col || c == '\n' {
            return start + i;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

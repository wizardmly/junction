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

/// A function in a Call Hierarchy: its name, file and name's position,
/// its calls of the function above it (callers) or the parent's calls of
/// it (callees) as 0-based (line, UTF-16 column), and whether it's an
/// ancestor already (recursion, not expanded again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallNode {
    pub name: String,
    pub path: String,
    pub offset: usize,
    pub line: u32,
    pub col: u32,
    pub calls: Vec<(u32, u32)>,
    pub recursive: bool,
}

/// The 0-based line and UTF-16 column of a byte offset.
pub fn position_of(text: &str, offset: usize) -> (u32, u32) {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    (text[..offset].matches('\n').count() as u32, text[line_start..offset].encode_utf16().count() as u32)
}

/// The functions some places are in, one node each with its calls: each
/// place is a file's (path, line, col) and the call to list for it. `read`
/// gives a file's text; `ancestors` are the names above, for recursion.
pub fn group_by_function(places: Vec<(String, u32, u32, (u32, u32))>, ancestors: &[String], read: &dyn Fn(&str) -> Option<String>) -> Vec<CallNode> {
    let mut texts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    let mut out: Vec<CallNode> = Vec::new();
    for (path, line, col, call) in places {
        let Some(lang) = Lang::from_path(&path) else { continue };
        let Some(text) = texts.entry(path.clone()).or_insert_with(|| read(&path)).as_deref() else { continue };
        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        let Some(function) = caller_at(lang, text, offset_of(text, line, col), &file_name) else { continue };
        match out.iter_mut().find(|n| n.path == path && n.offset == function.offset) {
            Some(n) => n.calls.push(call),
            None => out.push(CallNode {
                recursive: ancestors.contains(&function.name),
                name: function.name,
                path,
                offset: function.offset,
                line: function.line,
                col: function.col,
                calls: vec![call],
            }),
        }
    }
    out
}

/// Callee Hierarchy's view of a function: the calls in the body of the
/// function whose name is at `name_offset`, each as its callee name's
/// offset, in order.
pub fn calls_in(lang: Lang, text: &str, name_offset: usize) -> Vec<usize> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let root = tree.root_node();
    let Some(function) = descendants(root)
        .into_iter()
        .filter(|n| is_function(lang, *n) && function_name(lang, *n).is_some_and(|name| name.start_byte() == name_offset))
        .min_by_key(|n| n.end_byte() - n.start_byte())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for n in descendants(function) {
        let callee = match (lang, n.kind()) {
            (Lang::Kotlin, "call_expression") => n.child(0).and_then(|c| match c.kind() {
                "simple_identifier" => Some(c),
                // `a.b()`: b.
                "navigation_expression" => descendants(c).into_iter().filter(|d| d.kind() == "simple_identifier").last(),
                _ => None,
            }),
            (Lang::Kotlin, "constructor_invocation") => descendants(n).into_iter().find(|d| d.kind() == "type_identifier"),
            (Lang::Java, "method_invocation") => n.child_by_field_name("name"),
            (Lang::Java, "object_creation_expression") => n.child_by_field_name("type").map(|t| {
                // `B<C>`: B; `a.B`: B.
                let t = if t.kind() == "generic_type" { t.child(0).unwrap_or(t) } else { t };
                children(t).into_iter().filter(|d| d.kind() == "type_identifier").last().unwrap_or(t)
            }),
            _ => None,
        };
        if let Some(c) = callee {
            out.push(c.start_byte());
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The type a Type Hierarchy (Ctrl+H) is for: the word at the caret when
/// `is_type` says it names one, else the class around the caret.
pub fn type_at(lang: Lang, text: &str, offset: usize, is_type: impl Fn(&str) -> bool) -> Option<String> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let offset = offset.min(text.len());
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(i, _)| i);
    let end = text[offset..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| offset + i);
    let word = &text[start..end];
    if !word.is_empty() && is_type(word) {
        return Some(word.to_owned());
    }
    let tree = symbols::parse(lang, text)?;
    let leaf = tree.root_node().descendant_for_byte_range(offset, offset)?;
    // The caret on a class's own name: that class.
    let own = leaf.parent().filter(|p| matches!(p.kind(), "class_declaration" | "object_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration"));
    match own {
        Some(_) if !word.is_empty() => Some(word.to_owned()),
        _ => class_of(lang, leaf, text),
    }
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

    #[test]
    fn finds_the_calls() {
        let text = "class Cart {\n    fun use() {\n        println(total(a.b(1)))\n        Item()\n    }\n    fun total(x: Int) = 1\n}\n";
        let at = text.find("use").unwrap();
        let names: Vec<&str> = calls_in(Lang::Kotlin, text, at).into_iter().map(|o| &text[o..o + text[o..].find(|c: char| !c.is_alphanumeric()).unwrap()]).collect();
        assert_eq!(names, ["println", "total", "b", "Item"]);
        let java = "class A {\n  void g() { f(); new B<C>(); x.h(); }\n}\n";
        let at = java.find("g()").unwrap();
        let names: Vec<&str> = calls_in(Lang::Java, java, at).into_iter().map(|o| &java[o..o + 1]).collect();
        assert_eq!(names, ["f", "B", "h"]);
    }

    #[test]
    fn groups_calls_by_function() {
        let cart = "class Cart {\n    fun total() = 1\n    fun use() {\n        total()\n        total()\n    }\n}\n";
        let main = "fun main() {\n    Cart().total()\n}\n";
        let read = |p: &str| Some(if p == "Cart.kt" { cart } else { main }.to_owned());
        let places = vec![("Cart.kt".to_owned(), 3, 8, (3, 8)), ("Main.kt".to_owned(), 1, 11, (1, 11)), ("Cart.kt".to_owned(), 4, 8, (4, 8))];
        let nodes = group_by_function(places, &["main".to_owned()], &read);
        let got: Vec<(&str, usize, bool)> = nodes.iter().map(|n| (n.name.as_str(), n.calls.len(), n.recursive)).collect();
        assert_eq!(got, [("Cart.use", 2, false), ("main", 1, true)]);
        assert_eq!(position_of("ab\ncé!", 6), (1, 2));
    }

    #[test]
    fn finds_the_type() {
        let text = "class Cart : Base() {\n    fun f(x: Item) = 1\n}\n";
        let is_type = |w: &str| w == "Base" || w == "Item";
        assert_eq!(type_at(Lang::Kotlin, text, text.find("Item").unwrap() + 1, is_type).as_deref(), Some("Item"));
        assert_eq!(type_at(Lang::Kotlin, text, text.find("fun").unwrap(), is_type).as_deref(), Some("Cart"));
        assert_eq!(type_at(Lang::Kotlin, text, text.find("Cart").unwrap(), |_| false).as_deref(), Some("Cart"));
        assert_eq!(type_at(Lang::Kotlin, text, text.len(), is_type), None);
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

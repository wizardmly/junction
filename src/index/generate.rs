//! Generate's view of the syntax tree (Kotlin and Java): the class around
//! the caret, its properties or fields with their written types, the
//! methods it already has, and where new members go (after the member
//! the caret is in, else first in the body; a body-less Kotlin class
//! gets one).

use std::ops::Range;

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;
use crate::domain::generate::Field;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    pub name: String,
    /// The declaration up to its body: supertypes for Override Methods.
    pub header: String,
    pub type_params: usize,
    /// Kotlin `object`: toString() only.
    pub is_object: bool,
    /// Kotlin: it has a primary constructor (else Secondary Constructor).
    pub primary_constructor: bool,
    /// Kotlin: body properties that must be initialized (no initializer,
    /// getter or delegate), for Secondary Constructor with the `var`s.
    pub uninitialized: Vec<String>,
    pub fields: Vec<Field>,
    /// Names of the methods it declares.
    pub methods: Vec<String>,
    /// Their ranges (annotations and modifiers included), by name.
    pub method_ranges: Vec<(String, Range<usize>)>,
    /// Where the members go: after this offset each one follows a blank
    /// line; `body` false: the class has no body and gets ` { … }`.
    pub insert_at: usize,
    pub after_member: bool,
    /// Another member follows right after (no blank line): one is added.
    pub tight: bool,
    pub body: bool,
    /// The members' indentation, and the class line's.
    pub indent: String,
    pub class_indent: String,
    /// The class body's `{`…`}` when it has no members (to fill).
    pub empty_body: Option<Range<usize>>,
}

const NOT_A_CLASS: &str = "Generate works inside a class";

fn text_of<'a>(n: Node, text: &'a str) -> &'a str {
    &text[n.byte_range()]
}

fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = n.walk();
    let found = n.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// The written type after `:` in a Kotlin declaration's text, or a
/// literal initializer's type.
fn kotlin_type(decl: &str) -> String {
    let head = decl.split('=').next().unwrap_or("");
    if let Some((_, ty)) = head.split_once(':') {
        return ty.trim().to_owned();
    }
    let init = decl.split_once('=').map_or("", |(_, v)| v.trim());
    if init.starts_with('"') {
        "String".into()
    } else if init == "true" || init == "false" {
        "Boolean".into()
    } else if init.ends_with('L') && init[..init.len() - 1].chars().all(|c| c.is_ascii_digit()) {
        "Long".into()
    } else if !init.is_empty() && init.chars().all(|c| c.is_ascii_digit() || c == '_') {
        "Int".into()
    } else if let Some(kind) = ["Int", "Long", "Short", "Byte", "Char", "Boolean", "Float", "Double"]
        .iter()
        .find(|k| init.starts_with(&format!("{k}Array(")) || init.starts_with(&format!("{}ArrayOf(", k.to_lowercase())))
    {
        // `IntArray(3)`, `intArrayOf(1, 2)`: arrays compare by content.
        format!("{kind}Array")
    } else if init.starts_with("arrayOf(") || init.starts_with("Array(") || init.starts_with("arrayOfNulls") {
        "Array<Any>".into()
    } else {
        String::new()
    }
}

fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The class around `offset`, for Generate.
pub fn class_at(lang: Lang, text: &str, offset: usize, unit: &str) -> Result<ClassInfo, String> {
    let tree = symbols::parse(lang, text).ok_or(NOT_A_CLASS)?;
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset).ok_or(NOT_A_CLASS)?;
    let class = loop {
        let kind = node.kind();
        let is_class = match lang {
            Lang::Kotlin => kind == "class_declaration" || kind == "object_declaration",
            _ => kind == "class_declaration",
        };
        if is_class {
            break node;
        }
        node = node.parent().ok_or(NOT_A_CLASS)?;
    };
    let header = text_of(class, text).split('{').next().unwrap_or("");
    let words: Vec<&str> = header.split_whitespace().collect();
    if lang == Lang::Kotlin && (words.contains(&"interface") || words.contains(&"enum") || words.contains(&"annotation")) {
        return Err(NOT_A_CLASS.into());
    }
    let name = child_of_kind(class, if lang == Lang::Kotlin { "type_identifier" } else { "identifier" }).map(|n| text_of(n, text).to_owned()).ok_or(NOT_A_CLASS)?;
    let type_params = child_of_kind(class, "type_parameters").map_or(0, |p| {
        let mut cursor = p.walk();
        let count = p.named_children(&mut cursor).filter(|c| c.kind() == "type_parameter").count();
        count
    });
    let body = child_of_kind(class, if lang == Lang::Kotlin { "class_body" } else { "class_body" });
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut method_ranges = Vec::new();
    let mut uninitialized = Vec::new();
    let primary_constructor = lang == Lang::Kotlin && child_of_kind(class, "primary_constructor").is_some();
    match lang {
        Lang::Kotlin => {
            if let Some(ctor) = child_of_kind(class, "primary_constructor") {
                let params = child_of_kind(ctor, "class_parameters").unwrap_or(ctor);
                let mut cursor = params.walk();
                for p in params.named_children(&mut cursor).filter(|c| c.kind() == "class_parameter") {
                    let t = text_of(p, text);
                    let is_property = p.child(0).is_some_and(|c| matches!(text_of(c, text), "val" | "var")) || t.contains("val ") || t.contains("var ");
                    if !is_property {
                        continue;
                    }
                    if let Some(id) = child_of_kind(p, "simple_identifier") {
                        fields.push(Field { name: text_of(id, text).to_owned(), ty: kotlin_type(t), is_static: false, is_final: t.contains("val ") });
                    }
                }
            }
            if let Some(body) = body {
                let mut cursor = body.walk();
                for m in body.named_children(&mut cursor) {
                    match m.kind() {
                        "property_declaration" => {
                            let t = text_of(m, text);
                            let modifiers = child_of_kind(m, "modifiers").map_or("", |n| text_of(n, text));
                            // No backing field: a getter and no initializer.
                            let getter_only = child_of_kind(m, "getter").is_some() && !t.split("get").next().unwrap_or("").contains('=');
                            if modifiers.split_whitespace().any(|w| w == "const") || getter_only {
                                continue;
                            }
                            if let Some(id) = child_of_kind(m, "variable_declaration").and_then(|v| child_of_kind(v, "simple_identifier")) {
                                let decl = &t[id.start_byte() - m.start_byte()..];
                                let decl = decl.lines().next().unwrap_or(decl);
                                let words: Vec<&str> = modifiers.split_whitespace().collect();
                                if !decl.contains('=') && !decl.contains(" by ") && child_of_kind(m, "getter").is_none() && !words.contains(&"abstract") && !words.contains(&"lateinit") {
                                    uninitialized.push(text_of(id, text).to_owned());
                                }
                                fields.push(Field { name: text_of(id, text).to_owned(), ty: kotlin_type(decl), is_static: false, is_final: t.trim_start_matches(modifiers).trim_start().starts_with("val") });
                            }
                        }
                        "function_declaration" => {
                            if let Some(n) = child_of_kind(m, "simple_identifier") {
                                methods.push(text_of(n, text).to_owned());
                                method_ranges.push((text_of(n, text).to_owned(), m.byte_range()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {
            if let Some(body) = body {
                let mut cursor = body.walk();
                for m in body.named_children(&mut cursor) {
                    match m.kind() {
                        "field_declaration" => {
                            let modifiers = child_of_kind(m, "modifiers").map_or("", |n| text_of(n, text));
                            let words: Vec<&str> = modifiers.split_whitespace().collect();
                            let ty = m.child_by_field_name("type").map(|t| text_of(t, text).to_owned()).unwrap_or_default();
                            let mut c = m.walk();
                            for d in m.named_children(&mut c).filter(|d| d.kind() == "variable_declarator") {
                                if let Some(n) = d.child_by_field_name("name") {
                                    fields.push(Field { name: text_of(n, text).to_owned(), ty: ty.clone(), is_static: words.contains(&"static"), is_final: words.contains(&"final") });
                                }
                            }
                        }
                        "method_declaration" => {
                            if let Some(n) = m.child_by_field_name("name") {
                                methods.push(text_of(n, text).to_owned());
                                method_ranges.push((text_of(n, text).to_owned(), m.byte_range()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let class_indent = line_indent(text, class.start_byte());
    let indent = format!("{class_indent}{unit}");
    let Some(body) = body else {
        return Ok(ClassInfo {
            header: header.to_owned(),
            name,
            type_params,
            is_object: class.kind() == "object_declaration",
            primary_constructor,
            uninitialized,
            fields,
            methods,
            method_ranges,
            insert_at: class.end_byte(),
            after_member: false,
            tight: false,
            body: false,
            indent,
            class_indent,
            empty_body: None,
        });
    };
    let members: Vec<Node> = {
        let mut cursor = body.walk();
        body.named_children(&mut cursor).filter(|m| !m.kind().contains("comment")).collect()
    };
    // After the member the caret is in or after; else first in the body.
    let anchor = members.iter().filter(|m| m.start_byte() <= offset).last();
    let indent = members.first().map_or(indent, |m| line_indent(text, m.start_byte()));
    let tight = anchor.and_then(|a| members.iter().find(|m| m.start_byte() >= a.end_byte()).map(|m| text[a.end_byte()..m.start_byte()].matches('\n').count() == 1)).unwrap_or(false);
    let (insert_at, after_member, empty_body) = match anchor {
        Some(m) => (m.end_byte(), true, None),
        None if members.is_empty() => (body.start_byte() + 1, false, Some(body.byte_range())),
        None => (body.start_byte() + 1, false, None),
    };
    Ok(ClassInfo { header: header.to_owned(), name, type_params, is_object: class.kind() == "object_declaration", primary_constructor, uninitialized, fields, methods, method_ranges, insert_at, after_member, tight, body: true, indent, class_indent, empty_body })
}

/// The text to insert for generated members at `info`'s place.
pub fn insertion(info: &ClassInfo, members: &[String]) -> (Range<usize>, String) {
    let ind = |m: &String| m.lines().map(|l| if l.is_empty() { String::new() } else { format!("{}{l}", info.indent) }).collect::<Vec<_>>().join("\n");
    let all: Vec<String> = members.iter().map(ind).collect();
    if !info.body {
        return (info.insert_at..info.insert_at, format!(" {{\n{}\n{}}}", all.join("\n\n"), info.class_indent));
    }
    if let Some(body) = &info.empty_body {
        return (body.clone(), format!("{{\n{}\n{}}}", all.join("\n\n"), info.class_indent));
    }
    if info.after_member {
        let mut insert: String = all.iter().map(|m| format!("\n\n{m}")).collect();
        if info.tight {
            insert.push('\n');
        }
        (info.insert_at..info.insert_at, insert)
    } else {
        (info.insert_at..info.insert_at, all.iter().map(|m| format!("\n{m}\n")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kotlin_classes() {
        let kt = "class Person(val name: String, var age: Int, other: Long) {\n    val tags = IntArray(3)\n    var nick: String? = null\n    val upper get() = name.uppercase()\n    const val X = 1\n\n    fun greet() = 1\n}\n";
        let info = class_at(Lang::Kotlin, kt, kt.find("greet").unwrap(), "    ").unwrap();
        assert_eq!(info.name, "Person");
        let got: Vec<(&str, &str)> = info.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str())).collect();
        assert_eq!(got, [("name", "String"), ("age", "Int"), ("tags", "IntArray"), ("nick", "String?")]);
        assert_eq!(info.methods, ["greet"]);
        let (range, insert) = insertion(&info, &["fun a() {\n}".into()]);
        let mut out = kt.to_owned();
        out.replace_range(range, &insert);
        assert!(out.contains("    fun greet() = 1\n\n    fun a() {\n    }\n}"));
        // No body: one is added.
        let kt = "data class P(val x: Int)\n";
        let info = class_at(Lang::Kotlin, kt, 3, "    ").unwrap();
        let (range, insert) = insertion(&info, &["fun a() = 1".into()]);
        let mut out = kt.to_owned();
        out.replace_range(range, &insert);
        assert_eq!(out, "data class P(val x: Int) {\n    fun a() = 1\n}\n");
        assert!(class_at(Lang::Kotlin, "interface I {\n}\n", 12, "    ").is_err());
        assert!(info.primary_constructor);
        let kt = "class Q : Base {\n    val a: String\n    var b: Int = 0\n    lateinit var c: String\n    val d by lazy { 1 }\n}\n";
        let info = class_at(Lang::Kotlin, kt, kt.find("a:").unwrap(), "    ").unwrap();
        assert!(!info.primary_constructor);
        assert_eq!(info.uninitialized, ["a"]);
        let (range, insert) = insertion(&info, &["fun f() = 1".into()]);
        let mut out = kt.to_owned();
        out.replace_range(range, &insert);
        assert!(out.contains("    val a: String\n\n    fun f() = 1\n\n    var b"));
    }

    #[test]
    fn reads_java_classes() {
        let java = "class Box<T> {\n  private static int count;\n  private final String name, label;\n  int[] data;\n\n  void run() {}\n}\n";
        let info = class_at(Lang::Java, java, java.find("run").unwrap(), "  ").unwrap();
        assert_eq!(info.type_params, 1);
        let got: Vec<(&str, &str, bool, bool)> = info.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str(), f.is_static, f.is_final)).collect();
        assert_eq!(got, [("count", "int", true, false), ("name", "String", false, true), ("label", "String", false, true), ("data", "int[]", false, false)]);
        assert_eq!(info.indent, "  ");
        let empty = "class E {\n}\n";
        let info = class_at(Lang::Java, empty, 9, "    ").unwrap();
        let (range, insert) = insertion(&info, &["void a() {}".into(), "void b() {}".into()]);
        let mut out = empty.to_owned();
        out.replace_range(range, &insert);
        assert_eq!(out, "class E {\n    void a() {}\n\n    void b() {}\n}\n");
    }
}

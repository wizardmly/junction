//! Surround With's declarations moved out (IntelliJ's Kotlin
//! MoveDeclarationsOutHelper and Java SurroundWithUtil.moveDeclarationsOut):
//! a declaration among the wrapped lines that the code after them uses
//! goes in front of the new construct, its initializer left inside as an
//! assignment (Kotlin's last line as the construct's value, `val b = if
//! (…) { … }`).

use std::ops::Range;

use tree_sitter::Node;

use super::introduce::{java_type, kotlin_type};
use super::lang::Lang;
use super::symbols;
use crate::domain::surround::DeclarationsOut;

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// The whole lines of a node, through its line break.
fn lines(text: &str, node: Node) -> Range<usize> {
    let end = text[node.end_byte()..].find('\n').map_or(text.len(), |i| node.end_byte() + i + 1);
    line_start(text, node.start_byte())..end
}

fn indent_at(text: &str, at: usize) -> String {
    text[line_start(text, at)..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Whether `name` is used in `range` of the text, as a name of its own
/// (not a member, `x.name`).
fn used(lang: Lang, root: Node, text: &str, name: &str, range: Range<usize>) -> bool {
    let kind = if lang == Lang::Kotlin { "simple_identifier" } else { "identifier" };
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.end_byte() <= range.start || n.start_byte() >= range.end {
            continue;
        }
        if n.kind() == kind && n.start_byte() >= range.start && &text[n.byte_range()] == name && !text[..n.start_byte()].trim_end().ends_with('.') {
            return true;
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    false
}

/// A Kotlin `var`'s default value by its type, as the Analysis API's
/// `defaultInitializer`; none for other types.
fn kotlin_default(ty: &str) -> Option<&'static str> {
    if ty.ends_with('?') {
        return Some("null");
    }
    Some(match ty {
        "Int" | "Long" | "Short" | "Byte" => "0",
        "Double" => "0.0",
        "Float" => "0.0f",
        "Char" => "'\\u0000'",
        "Boolean" => "false",
        _ => return None,
    })
}

/// PsiTypesUtil.getDefaultValueOfType.
fn java_default(ty: &str) -> &'static str {
    match ty {
        "boolean" => "false",
        "int" | "long" | "short" | "byte" | "char" | "float" | "double" => "0",
        _ => "null",
    }
}

/// The declarations to move out of the whole lines `range` for a
/// surrounder (`defaults`: a `var` keeps a default value); None when
/// none is used after them, or nothing would be left to wrap.
pub fn declarations_out(lang: Lang, text: &str, range: Range<usize>, defaults: bool) -> Option<DeclarationsOut> {
    let tree = symbols::parse(lang, text)?;
    let root = tree.root_node();
    let first = range.start + text[range.clone()].len() - text[range.clone()].trim_start().len();
    let mut node = root.descendant_for_byte_range(first, first)?;
    let container_kind = if lang == Lang::Kotlin { "statements" } else { "block" };
    let container = loop {
        let parent = node.parent()?;
        if parent.kind() == container_kind {
            break parent;
        }
        node = parent;
    };
    // What the code after the lines can see: to the end of the block.
    let scope_end = if lang == Lang::Kotlin { container.parent().map_or(container.end_byte(), |p| p.end_byte()) } else { container.end_byte() };
    let after = range.end..scope_end;
    let mut cursor = container.walk();
    let statements: Vec<Node> = container
        .named_children(&mut cursor)
        .filter(|s| s.start_byte() >= range.start && s.end_byte() <= range.end && !s.kind().contains("comment"))
        .collect();
    let last = statements.last()?.id();
    let mut out = DeclarationsOut::default();
    // (lines replaced, their new text), in order.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for statement in &statements {
        let indent = indent_at(text, statement.start_byte());
        match (lang, statement.kind()) {
            (Lang::Kotlin, "property_declaration") => {
                let Some(variable) = statement.named_children(&mut statement.walk()).find(|c| c.kind() == "variable_declaration") else { continue };
                let Some(name) = variable.named_child(0).filter(|n| n.kind() == "simple_identifier").map(|n| &text[n.byte_range()]) else { continue };
                if !used(lang, root, text, name, after.clone()) {
                    continue;
                }
                let keyword = if text[statement.byte_range()].trim_start().split_whitespace().any(|w| w == "var") { "var" } else { "val" };
                let declared = text[variable.byte_range()].split_once(':').map(|(_, t)| t.trim().to_owned());
                let mut c = statement.walk();
                let children: Vec<Node> = statement.children(&mut c).collect();
                let eq = children.iter().position(|c| c.kind() == "=");
                let Some(initializer) = eq.and_then(|i| children[i + 1..].iter().find(|c| c.is_named())) else {
                    // No initializer: the declaration itself goes out.
                    if text[statement.byte_range()].contains(" by ") {
                        continue;
                    }
                    out.declarations.push(text[statement.byte_range()].to_owned());
                    edits.push((lines(text, *statement), String::new()));
                    continue;
                };
                let value = &text[initializer.byte_range()];
                let typed = declared.as_ref().map(|t| format!(": {t}")).unwrap_or_default();
                if statement.id() == last {
                    out.prefix = format!("{keyword} {name}{typed} = ");
                    edits.push((lines(text, *statement), format!("{indent}{value}\n")));
                    continue;
                }
                let Some(ty) = declared.clone().or_else(|| kotlin_type(*initializer, text)) else { continue };
                let default = if defaults && keyword == "var" { kotlin_default(&ty).map(|d| format!(" = {d}")) } else { None };
                out.declarations.push(format!("{keyword} {name}: {ty}{}", default.unwrap_or_default()));
                edits.push((lines(text, *statement), format!("{indent}{name} = {value}\n")));
            }
            (Lang::Kotlin, "function_declaration" | "class_declaration" | "object_declaration") => {
                let name = statement.named_children(&mut statement.walk()).find(|c| matches!(c.kind(), "simple_identifier" | "type_identifier")).map(|n| text[n.byte_range()].to_owned());
                if name.is_some_and(|n| used(lang, root, text, &n, after.clone())) {
                    out.declarations.push(text[statement.byte_range()].to_owned());
                    edits.push((lines(text, *statement), String::new()));
                }
            }
            (Lang::Java, "local_variable_declaration") => {
                let mut c = statement.walk();
                let declarators: Vec<Node> = statement.children_by_field_name("declarator", &mut c).collect();
                let names: Vec<&str> = declarators.iter().filter_map(|d| d.child_by_field_name("name")).map(|n| &text[n.byte_range()]).collect();
                if !names.iter().any(|n| used(lang, root, text, n, after.clone())) {
                    continue;
                }
                let Some(type_node) = statement.child_by_field_name("type") else { continue };
                let mut ty = text[type_node.byte_range()].to_owned();
                if ty == "var" {
                    // The type written out, when the initializer shows it.
                    match declarators.first().and_then(|d| d.child_by_field_name("value")).and_then(|v| java_type(v, text)) {
                        Some(t) if declarators.len() == 1 => ty = t,
                        _ => continue,
                    }
                }
                let modifiers = statement.named_child(0).filter(|m| m.kind() == "modifiers").map(|m| format!("{} ", &text[m.byte_range()])).unwrap_or_default();
                let is_final = modifiers.split_whitespace().any(|w| w == "final");
                let mut assignments = String::new();
                let mut parts = Vec::new();
                for d in &declarators {
                    let Some(name) = d.child_by_field_name("name").map(|n| &text[n.byte_range()]) else { continue };
                    match d.child_by_field_name("value") {
                        Some(value) => {
                            let value_text = &text[value.byte_range()];
                            // An array initializer needs its type in an assignment.
                            let value_text = if value.kind() == "array_initializer" { format!("new {ty}{value_text}") } else { value_text.to_owned() };
                            assignments.push_str(&format!("{indent}{name} = {value_text};\n"));
                            parts.push(if defaults && !is_final { format!("{name} = {}", java_default(&ty)) } else { name.to_owned() });
                        }
                        None => parts.push(name.to_owned()),
                    }
                }
                out.declarations.push(format!("{modifiers}{ty} {};", parts.join(", ")));
                edits.push((lines(text, *statement), assignments));
            }
            _ => {}
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut block = String::new();
    let mut at = range.start;
    for (lines, with) in &edits {
        block.push_str(&text[at..lines.start]);
        block.push_str(with);
        at = lines.end;
    }
    block.push_str(&text[at..range.end]);
    if block.trim().is_empty() {
        return None;
    }
    // The block's last line keeps its break as the lines had one.
    if !range.end.eq(&text.len()) && !block.ends_with('\n') && text[..range.end].ends_with('\n') {
        block.push('\n');
    }
    Some(out_with(out, block))
}

fn out_with(mut out: DeclarationsOut, block: String) -> DeclarationsOut {
    out.block = block;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::imports::ImportStyle;
    use crate::domain::surround::{surround_lines_moving, Surrounder};

    fn surround(lang: Lang, surrounder: Surrounder, text: &str, from: &str, to: &str) -> String {
        let style = if lang == Lang::Kotlin { ImportStyle::Kotlin } else { ImportStyle::Java };
        let start = text.find(from).unwrap();
        let start = line_start(text, start);
        let end = text.find(to).unwrap() + to.len() + 1;
        let defaults = crate::domain::surround::moves_declarations(style, surrounder).unwrap();
        let out = declarations_out(lang, text, start..end, defaults).expect("declarations to move");
        let done = surround_lines_moving(style, surrounder, text, start..end, "    ", &out);
        let mut result = text.to_owned();
        result.replace_range(done.range.clone(), &done.text);
        result
    }

    #[test]
    fn moves_kotlin_declarations_out() {
        // IntelliJ's surroundWith/tryCatch/moveDeclarationsOut.kt.
        let text = "fun foo() {\n    val a = \"aaa\"\n    val b = \"aaa\"\n\n    a.charAt(1)\n    b.charAt(1)\n}\n";
        assert_eq!(
            surround(Lang::Kotlin, Surrounder::TryCatch, text, "val a", "val b = \"aaa\""),
            "fun foo() {\n    val a: String\n    val b = try {\n        a = \"aaa\"\n        \"aaa\"\n    } catch (e: Exception) {\n        throw e\n    }\n\n    a.charAt(1)\n    b.charAt(1)\n}\n"
        );
        // if/moveDeclarationsOut/var/defaultValue: defaults for `var`s.
        let text = "fun foo() {\n    var a: Int = 1\n    var c: Boolean = true\n    var g: Float = 1\n    a = 2\n    c = false\n    g = 2\n}\n";
        assert_eq!(
            surround(Lang::Kotlin, Surrounder::If, text, "var a", "var g: Float = 1"),
            "fun foo() {\n    var a: Int = 0\n    var c: Boolean = false\n    var g: Float = if () {\n        a = 1\n        c = true\n        1\n    }\n    a = 2\n    c = false\n    g = 2\n}\n"
        );
        // functionLiteral/moveDeclarationsOutWithNonDeclarationLastStatement.kt,
        // and a local function used after (if/moveDeclarationsOut/function).
        let text = "fun foo() {\n    val a = \"aaa\"\n    fun test() {}\n    boo(\"a\")\n\n    a.get(1)\n    test()\n}\n";
        assert_eq!(
            surround(Lang::Kotlin, Surrounder::Run, text, "val a", "boo(\"a\")"),
            "fun foo() {\n    val a: String\n    fun test() {}\n    run {\n        a = \"aaa\"\n        boo(\"a\")\n    }\n\n    a.get(1)\n    test()\n}\n"
        );
        // Nothing used after: nothing moves.
        let text = "fun foo() {\n    val a = 1\n    println(a)\n}\n";
        assert_eq!(declarations_out(Lang::Kotlin, text, text.find("    val").unwrap()..text.find("}\n").unwrap(), true), None);
    }

    #[test]
    fn moves_java_declarations_out() {
        let text = "class A {\n    void f() {\n        int a = 1, b;\n        final String s = \"x\";\n        int[] xs = {1, 2};\n        g();\n        use(a, s, xs);\n    }\n}\n";
        assert_eq!(
            surround(Lang::Java, Surrounder::If, text, "int a", "g();"),
            "class A {\n    void f() {\n        int a = 0, b;\n        final String s;\n        int[] xs = null;\n        if () {\n            a = 1;\n            s = \"x\";\n            xs = new int[]{1, 2};\n            g();\n        }\n        use(a, s, xs);\n    }\n}\n"
        );
        // No default values for these surrounders.
        let text = "class A {\n    void f() {\n        var n = 5;\n        g();\n        use(n);\n    }\n}\n";
        assert_eq!(
            surround(Lang::Java, Surrounder::TryFinally, text, "var n", "g();"),
            "class A {\n    void f() {\n        int n;\n        try {\n            n = 5;\n            g();\n        } finally {\n            \n        }\n        use(n);\n    }\n}\n"
        );
    }
}

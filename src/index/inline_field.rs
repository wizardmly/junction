//! Inline Property / Inline Field (Ctrl+Alt+N on a member), as IntelliJ
//! inlines a class's property, an object's or companion's constant, a
//! top-level property or a Java field: its references become its
//! initializer (parenthesized where an operator would bind tighter), and
//! the declaration goes. Without type resolution a reference is only
//! followed when it can't mean anything else: the bare name or `this.x` in
//! its own file, `Owner.x` anywhere, the bare name in a file importing it
//! (the import goes too). `other.x` makes the refactoring stop rather than
//! guess, and an initializer naming anything is only inlined in its own
//! file, where those names mean the same.

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::{children, descendants};
use super::inline::{is_atom, is_written, takes_any};
use super::lang::Lang;
use super::{locals, symbols};
use crate::domain::line_edit::declaration_lines;

const NOT_HERE: &str = "Cannot perform refactoring. Caret should be positioned at the name of element to be inlined.";

/// A property or field to inline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// "Property" (Kotlin) or "Field" (Java).
    pub kind: &'static str,
    /// The class or object it's in, as references qualify it; none at the
    /// top level.
    pub owner: Option<String>,
    /// Its file's package.
    pub package: String,
    pub initializer: String,
    /// The initializer stays one operand wherever it goes.
    pub atom: bool,
    /// The initializer names nothing, so it means the same in any file.
    pub portable: bool,
    pub name_range: Range<usize>,
    /// The lines deleting it removes.
    pub declaration: Range<usize>,
}

/// A reference to replace: the name, or `Owner.name` / `this.name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub range: Range<usize>,
    /// Any expression fits there unparenthesized.
    pub fits: bool,
    /// Kotlin's `"$name"`: the `$` is part of the range.
    pub template: bool,
}

/// What a file holds of the field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    pub refs: Vec<Reference>,
    /// Lines importing it (with their line break).
    pub imports: Vec<Range<usize>>,
    /// References that may be to it but can't be told (`other.name`,
    /// `::name`): their offsets.
    pub unresolved: Vec<usize>,
}

fn name_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Kotlin => &["simple_identifier", "interpolated_identifier"],
        _ => &["identifier"],
    }
}

/// The member declaration a name node names, and its initializer.
fn declaration_of<'t>(lang: Lang, name: Node<'t>) -> Option<(Node<'t>, Option<Node<'t>>)> {
    match lang {
        Lang::Kotlin => {
            let decl = name.parent().filter(|p| p.kind() == "variable_declaration")?.parent().filter(|p| p.kind() == "property_declaration")?;
            // A local is Inline Variable's.
            if decl.parent().is_some_and(|p| p.kind() == "statements") {
                return None;
            }
            let mut cursor = decl.walk();
            let all: Vec<Node> = decl.children(&mut cursor).collect();
            let init = all.iter().position(|c| c.kind() == "=").and_then(|i| all.get(i + 1).copied());
            Some((decl, init))
        }
        _ => {
            let declarator = name.parent().filter(|p| p.kind() == "variable_declarator" && p.child_by_field_name("name") == Some(name))?;
            let decl = declarator.parent().filter(|p| p.kind() == "field_declaration")?;
            Some((decl, declarator.child_by_field_name("value")))
        }
    }
}

/// The class or object a member is in, as a reference qualifies it (a
/// companion's members by the class).
fn owner_of(lang: Lang, decl: Node, text: &str) -> Option<String> {
    let mut at = decl.parent();
    while let Some(n) = at {
        let name = match (lang, n.kind()) {
            (Lang::Kotlin, "class_declaration" | "object_declaration") => children(n).into_iter().find(|c| c.kind() == "type_identifier"),
            (Lang::Java, "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration") => n.child_by_field_name("name"),
            _ => None,
        };
        if let Some(name) = name {
            return Some(text[name.byte_range()].to_owned());
        }
        at = n.parent();
    }
    None
}

fn package_of(lang: Lang, root: Node, text: &str) -> String {
    let kind = if lang == Lang::Kotlin { "package_header" } else { "package_declaration" };
    children(root)
        .into_iter()
        .find(|c| c.kind() == kind)
        .and_then(|p| children(p).into_iter().next())
        .map(|id| text[id.byte_range()].chars().filter(|c| !c.is_whitespace()).collect())
        .unwrap_or_default()
}

/// The property or field at `offset` (its name, or a reference in its
/// file): it, and the reference when on one.
pub fn field_at(lang: Lang, text: &str, offset: usize) -> Result<(Field, Option<Reference>), String> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err(NOT_HERE.into());
    }
    let tree = symbols::parse(lang, text).ok_or(NOT_HERE)?;
    let root = tree.root_node();
    let at = root.descendant_for_byte_range(offset, offset).ok_or(NOT_HERE)?;
    let at = [Some(at), at.prev_sibling(), root.descendant_for_byte_range(offset.saturating_sub(1), offset.saturating_sub(1))]
        .into_iter()
        .flatten()
        .find(|n| name_kinds(lang).contains(&n.kind()))
        .ok_or(NOT_HERE)?;
    let word = text[at.byte_range()].to_owned();
    // On the declaration, or a reference to one in this file.
    let name_node = match declaration_of(lang, at) {
        Some(_) => at,
        None => descendants(root)
            .into_iter()
            .find(|n| name_kinds(lang).contains(&n.kind()) && text[n.byte_range()] == word && declaration_of(lang, *n).is_some())
            .ok_or(NOT_HERE)?,
    };
    let (decl, init) = declaration_of(lang, name_node).ok_or(NOT_HERE)?;
    let kind = if lang == Lang::Kotlin { "Property" } else { "Field" };
    let init = init.ok_or_else(|| format!("Cannot perform refactoring. {kind} {word} has no initializer."))?;
    let declarators = children(decl).into_iter().filter(|c| c.kind() == "variable_declarator").count();
    if lang == Lang::Java && declarators > 1 {
        return Err(format!("Cannot perform refactoring. Field {word} is declared with others."));
    }
    let initializer = text[init.byte_range()].to_owned();
    let names = descendants(init).into_iter().any(|n| matches!(n.kind(), "simple_identifier" | "interpolated_identifier" | "identifier" | "this_expression" | "this" | "type_identifier"));
    let atom = is_atom(init.kind()) || !initializer.contains(|c: char| c.is_whitespace() || "+-*/%<>=!&|?:".contains(c));
    let field = Field {
        name: word.clone(),
        kind,
        owner: owner_of(lang, decl, text),
        package: package_of(lang, root, text),
        initializer,
        atom,
        portable: !names,
        name_range: name_node.byte_range(),
        declaration: declaration_lines(text, decl.start_byte()..decl.end_byte()),
    };
    let own = references(lang, text, &field, true);
    // Written somewhere: not a constant.
    let written = descendants(root)
        .into_iter()
        .filter(|n| name_kinds(lang).contains(&n.kind()) && n.byte_range() != field.name_range && text[n.byte_range()] == word)
        .any(|n| own.refs.iter().any(|r| r.range.contains(&n.start_byte())) && is_written(lang, written_node(n)));
    if written {
        return Err(format!("Cannot perform refactoring. {kind} {word} is accessed for writing."));
    }
    let on = own.refs.iter().find(|r| r.range.start <= offset && offset <= r.range.end).cloned();
    Ok((field, on))
}

/// The node an assignment would have on its left for a reference: the
/// qualified expression around a member access.
fn written_node(name: Node) -> Node {
    match name.parent() {
        Some(p) if p.kind() == "navigation_suffix" => p.parent().unwrap_or(name),
        Some(p) if p.kind() == "field_access" => p,
        _ => name,
    }
}

/// The field's references in a file: `own` is its declaring file.
pub fn references(lang: Lang, text: &str, field: &Field, own: bool) -> References {
    let mut out = References::default();
    let Some(tree) = symbols::parse(lang, text) else { return out };
    let root = tree.root_node();
    let other_package = !own && package_of(lang, root, text) != field.package;
    let mut imported = false;
    for n in descendants(root) {
        if !name_kinds(lang).contains(&n.kind()) || text[n.byte_range()] != field.name || (own && n.byte_range() == field.name_range) {
            continue;
        }
        // An import of it: `a.b.Owner.NAME`, `a.b.NAME` (top level).
        if let Some(import) = n.ancestors_until(root, &["import_header", "import_declaration"]) {
            let path: String = text[import.byte_range()].chars().filter(|c| !c.is_whitespace()).collect();
            let path = path.trim_end_matches(';');
            let of_it = match &field.owner {
                Some(owner) => path.ends_with(&format!("{owner}.{}", field.name)) || path.ends_with(&format!("{owner}.Companion.{}", field.name)),
                None => path.ends_with(&format!("{}.{}", field.package, field.name)),
            };
            if of_it {
                imported = true;
                let start = text[..import.start_byte()].rfind('\n').map_or(0, |i| i + 1);
                let end = text[import.end_byte()..].find('\n').map_or(text.len(), |i| import.end_byte() + i + 1);
                out.imports.push(start..end);
            }
            continue;
        }
        let parent = n.parent();
        // Something else's declaration or a named argument.
        if parent.is_some_and(|p| {
            matches!(p.kind(), "variable_declaration" | "parameter" | "class_parameter" | "function_declaration" | "formal_parameter" | "method_declaration" | "class_declaration" | "enum_constant")
                || (p.kind() == "variable_declarator" && p.child_by_field_name("name") == Some(n))
                || (p.kind() == "value_argument" && n.next_sibling().is_some_and(|s| s.kind() == "="))
        }) {
            continue;
        }
        match parent.map(|p| p.kind()) {
            // `::name`: can't say whose.
            Some("callable_reference") | Some("method_reference") => out.unresolved.push(n.start_byte()),
            // `x.name`
            Some("navigation_suffix") | Some("field_access") => {
                let access = if parent.unwrap().kind() == "navigation_suffix" { parent.unwrap().parent() } else { parent };
                let Some(access) = access else { continue };
                let receiver = match lang {
                    Lang::Kotlin => children(access).into_iter().next(),
                    _ => access.child_by_field_name("object"),
                };
                // A method of that name: not a field.
                if lang == Lang::Java && access.kind() != "field_access" {
                    continue;
                }
                let Some(receiver) = receiver.filter(|r| r.byte_range() != n.byte_range()) else { continue };
                let receiver_text: String = text[receiver.byte_range()].chars().filter(|c| !c.is_whitespace()).collect();
                let by_owner = field.owner.as_ref().is_some_and(|o| {
                    receiver_text == *o || receiver_text.ends_with(&format!(".{o}")) || receiver_text == "Companion" || receiver_text == format!("{o}.Companion")
                });
                let by_this = own && matches!(receiver.kind(), "this_expression" | "this");
                if by_owner || by_this {
                    out.refs.push(Reference { range: access.byte_range(), fits: access.parent().is_some_and(|p| takes_any(p.kind())), template: false });
                } else {
                    out.unresolved.push(n.start_byte());
                }
            }
            _ => {
                // A local of that name shadows it.
                let field_decl = own.then_some(field.name_range.start);
                if locals::resolve(lang, text, n.start_byte()).is_some_and(|l| Some(l.decl) != field_decl) {
                    continue;
                }
                let template = n.kind() == "interpolated_identifier" || (lang == Lang::Kotlin && n.start_byte() > 0 && text.as_bytes()[n.start_byte() - 1] == b'$');
                let range = if template { n.start_byte() - 1..n.end_byte() } else { n.byte_range() };
                out.refs.push(Reference { range, fits: parent.is_some_and(|p| takes_any(p.kind())), template });
            }
        }
    }
    // Elsewhere a bare name is it only when imported, or a top-level one
    // of the same package.
    if !own && !imported && (field.owner.is_some() || other_package || lang == Lang::Java) {
        let qualified: Vec<Reference> = out.refs.iter().filter(|r| text[r.range.clone()].contains('.')).cloned().collect();
        out.refs = qualified;
    }
    out
}

trait Ancestors<'t> {
    fn ancestors_until(&self, root: Node<'t>, kinds: &[&str]) -> Option<Node<'t>>;
}

impl<'t> Ancestors<'t> for Node<'t> {
    fn ancestors_until(&self, root: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
        let mut at = self.parent();
        while let Some(n) = at {
            if kinds.contains(&n.kind()) {
                return Some(n);
            }
            if n == root {
                return None;
            }
            at = n.parent();
        }
        None
    }
}

/// The edits replacing references with the initializer (and dropping the
/// imports).
pub fn edits(field: &Field, found: &References) -> Vec<(Range<usize>, String)> {
    let mut out: Vec<(Range<usize>, String)> = found
        .refs
        .iter()
        .map(|r| {
            let value = if r.template {
                format!("${{{}}}", field.initializer)
            } else if field.atom || r.fits {
                field.initializer.clone()
            } else {
                format!("({})", field.initializer)
            };
            (r.range.clone(), value)
        })
        .collect();
    out.extend(found.imports.iter().map(|r| (r.clone(), String::new())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::introduce_parameter::apply;

    #[test]
    fn inlines_a_kotlin_constant_in_its_file() {
        let text = "package a\n\nobject Limits {\n    const val MAX = 10\n    fun twice() = MAX * 2\n    fun label() = \"max $MAX\"\n}\n\nfun f(MAX: Int) = MAX\n";
        let (field, on) = field_at(Lang::Kotlin, text, text.find("MAX =").unwrap()).unwrap();
        assert_eq!((field.owner.as_deref(), field.package.as_str(), field.initializer.as_str(), field.portable, on), (Some("Limits"), "a", "10", true, None));
        let found = references(Lang::Kotlin, text, &field, true);
        // The parameter of the same name isn't it.
        assert_eq!(found.refs.len(), 2);
        let mut changes = edits(&field, &found);
        changes.push((field.declaration.clone(), String::new()));
        assert_eq!(apply(text, changes), "package a\n\nobject Limits {\n    fun twice() = 10 * 2\n    fun label() = \"max ${10}\"\n}\n\nfun f(MAX: Int) = MAX\n");
    }

    #[test]
    fn follows_qualified_and_imported_references_elsewhere() {
        let decl = "package a\n\nclass Api {\n    companion object {\n        val BASE = \"https://x\"\n    }\n}\n";
        let (field, _) = field_at(Lang::Kotlin, decl, decl.find("BASE").unwrap()).unwrap();
        assert_eq!(field.owner.as_deref(), Some("Api"));
        let other = "package b\n\nimport a.Api.Companion.BASE\nimport a.Api\n\nfun g() = BASE + Api.BASE + other.BASE\n";
        let found = references(Lang::Kotlin, other, &field, false);
        assert_eq!((found.refs.len(), found.imports.len(), found.unresolved.len()), (2, 1, 1));
        let not_imported = "package b\n\nfun h(BASE: String) = BASE\nval z = Api.BASE\n";
        assert_eq!(references(Lang::Kotlin, not_imported, &field, false).refs.len(), 1);
        let changes = edits(&field, &References { unresolved: Vec::new(), ..found });
        assert_eq!(apply(other, changes), "package b\n\nimport a.Api\n\nfun g() = \"https://x\" + \"https://x\" + other.BASE\n");
    }

    #[test]
    fn inlines_a_java_field_and_parenthesizes() {
        let text = "class A {\n  static final int SIZE = 4 + 1;\n  int f() { return SIZE * this.SIZE + A.SIZE; }\n}\n";
        let (field, on) = field_at(Lang::Java, text, text.rfind("SIZE;").unwrap()).unwrap();
        assert_eq!(on.map(|r| r.range), Some(text.rfind("A.SIZE").unwrap()..text.rfind("A.SIZE").unwrap() + 6));
        let found = references(Lang::Java, text, &field, true);
        let mut changes = edits(&field, &found);
        changes.push((field.declaration.clone(), String::new()));
        assert_eq!(apply(text, changes), "class A {\n  int f() { return (4 + 1) * (4 + 1) + (4 + 1); }\n}\n");
        // Written: refused.
        let text = "class B {\n  int n = 0;\n  void inc() { n = n + 1; }\n}\n";
        assert!(field_at(Lang::Java, text, text.find("n =").unwrap()).unwrap_err().contains("accessed for writing"));
        let text = "class C { var count = 1\n fun up() { count += 1 } }\n";
        assert!(field_at(Lang::Kotlin, text, text.find("count").unwrap()).unwrap_err().contains("accessed for writing"));
    }

    #[test]
    fn knows_what_names_things() {
        let text = "class K {\n    val scale = 2\n    val size = scale * 3\n    fun f() = size\n}\n";
        let (field, _) = field_at(Lang::Kotlin, text, text.rfind("size").unwrap()).unwrap();
        assert_eq!((field.name.as_str(), field.portable, field.atom), ("size", false, false));
        // A local isn't a member.
        let text = "fun f() {\n    val x = 1\n    println(x)\n}\n";
        assert!(field_at(Lang::Kotlin, text, text.find("x =").unwrap()).is_err());
    }
}

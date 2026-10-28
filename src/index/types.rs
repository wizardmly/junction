//! The declared types of Java / Kotlin method calls, field and property
//! reads, followed through their receivers (`user.getAddress().getCity()`):
//! a call's return type as its method declares it (in this file, the
//! project or the libraries' sources, its class's superclasses too), the
//! class's type parameters replaced by the receiver's type arguments
//! (`List<String>.get(0)` is `String`). What Introduce Variable writes for
//! a call, and what the other refactorings resolve receivers with.

use std::ops::Range;

use tree_sitter::Node;

use super::catches::{Classes, Finder, argument_count, bare_type, enclosing_class, find_class, kotlin_functions, lang_of};
use super::lang::Lang;
use super::symbols;

/// A type and the classes its text names that the file must import
/// (qualified).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed {
    pub ty: String,
    pub imports: Vec<String>,
}

/// The type of the expression at `range` of `text`, as `text` would
/// write it, when its declarations show it.
pub fn type_of(lang: Lang, text: &str, range: Range<usize>, classes: Classes) -> Option<Typed> {
    let tree = symbols::parse(lang, text)?;
    let node = super::introduce::expression_node(lang, tree.root_node(), text, range)?;
    let finder = Finder { classes, own: text };
    let (ty, context) = resolve(&finder, lang, node, text, 0)?;
    // Each class it names, qualified where it was declared.
    let mut imports = Vec::new();
    let h = crate::domain::imports::header(text);
    let imported: Vec<String> = h.imports.iter().map(|(_, q)| q.clone()).chain(h.wildcards.iter().map(|w| format!("{w}.*"))).collect();
    let style = if lang == Lang::Kotlin { crate::domain::imports::ImportStyle::Kotlin } else { crate::domain::imports::ImportStyle::Java };
    let implicit = crate::domain::imports::implicit_packages(style);
    for word in ty.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).filter(|w| w.starts_with(|c: char| c.is_uppercase())) {
        if lang == Lang::Kotlin && KOTLIN_BUILTINS.contains(&word) {
            continue;
        }
        let qualified = finder.qualify(word, &context);
        if !word.contains('.') && !crate::domain::imports::is_visible(&qualified, Some(&h.package), &imported, &implicit) && !imports.contains(&qualified) {
            imports.push(qualified);
        }
    }
    Some(Typed { ty, imports })
}

/// Kotlin's own types, and the Java collections Kotlin maps to them.
const KOTLIN_BUILTINS: &[&str] = &[
    "Int", "Long", "Short", "Byte", "Double", "Float", "Char", "Boolean", "String", "Unit", "Any", "Nothing", "List", "MutableList", "Map", "MutableMap", "Set",
    "MutableSet", "Collection", "Iterable", "Array", "IntArray", "Pair", "Triple", "Sequence",
];

/// A Java type as Kotlin writes it; None for `void` and arrays.
fn kotlin_of_java(ty: &str) -> Option<String> {
    if ty == "void" || ty.ends_with("[]") {
        return None;
    }
    Some(
        match ty {
            "int" | "Integer" => "Int",
            "long" => "Long",
            "short" => "Short",
            "byte" => "Byte",
            "double" => "Double",
            "float" => "Float",
            "char" | "Character" => "Char",
            "boolean" => "Boolean",
            "Object" => "Any",
            _ => return Some(ty.to_owned()),
        }
        .to_owned(),
    )
}

/// An expression's type, as declared, for the sibling lookups.
pub(super) fn declared(finder: &Finder, lang: Lang, node: Node, text: &str) -> Option<String> {
    resolve(finder, lang, node, text, 0).map(|(t, _)| t)
}

/// The expression's type and the text of the file it is written in.
fn resolve(finder: &Finder, lang: Lang, node: Node, text: &str, depth: usize) -> Option<(String, String)> {
    if depth > 8 {
        return None;
    }
    if let Some(ty) = super::surround::expression_type(lang, node, text) {
        return Some((ty, text.to_owned()));
    }
    match (lang, node.kind()) {
        (_, "parenthesized_expression") => resolve(finder, lang, node.named_child(0)?, text, depth + 1),
        (Lang::Java, "method_invocation") => {
            let name = &text[node.child_by_field_name("name")?.byte_range()];
            let arity = argument_count(node.child_by_field_name("arguments"));
            let receiver = match node.child_by_field_name("object") {
                None => enclosing_class(node, text)?,
                Some(o) if o.kind() == "this" => enclosing_class(node, text)?,
                Some(o) if o.kind() == "super" => finder.superclass_of(&enclosing_class(node, text)?, text)?,
                Some(o) => receiver(finder, lang, o, node, text, depth)?,
            };
            member_type(finder, &receiver, text, Member::Method(name, arity), lang)
        }
        (Lang::Java, "field_access") => {
            let name = &text[node.child_by_field_name("field")?.byte_range()];
            let receiver = receiver(finder, lang, node.child_by_field_name("object")?, node, text, depth)?;
            member_type(finder, &receiver, text, Member::Field(name), lang)
        }
        (Lang::Kotlin, "call_expression") => {
            let callee = node.named_child(0)?;
            let arguments = node.named_child(1).and_then(|s| {
                let mut c = s.walk();
                s.named_children(&mut c).find(|a| a.kind() == "value_arguments")
            });
            let arity = argument_count(arguments);
            match callee.kind() {
                "simple_identifier" => {
                    // A function of this file.
                    let name = &text[callee.byte_range()];
                    let tree = symbols::parse(Lang::Kotlin, text)?;
                    let function = kotlin_functions(tree.root_node(), text, name, arity).into_iter().next()?;
                    Some((kotlin_return(function, text)?, text.to_owned()))
                }
                "navigation_expression" => {
                    let name = &text[callee.named_child(1)?.named_child(0)?.byte_range()];
                    let receiver = receiver(finder, lang, callee.named_child(0)?, node, text, depth)?;
                    member_type(finder, &receiver, text, Member::Method(name, arity), lang)
                }
                _ => None,
            }
        }
        (Lang::Kotlin, "navigation_expression") => {
            let name = &text[node.named_child(1)?.named_child(0)?.byte_range()];
            let receiver = receiver(finder, lang, node.named_child(0)?, node, text, depth)?;
            member_type(finder, &receiver, text, Member::Field(name), lang)
        }
        _ => None,
    }
}

/// The class (with type arguments) a receiver expression is.
fn receiver(finder: &Finder, lang: Lang, receiver: Node, call: Node, text: &str, depth: usize) -> Option<String> {
    if matches!(receiver.kind(), "identifier" | "simple_identifier") && super::locals::resolve(lang, text, receiver.start_byte()).is_none() {
        // A class name, or a field of the enclosing class.
        return finder.receiver_type(lang, receiver, call);
    }
    resolve(finder, lang, receiver, text, depth + 1).map(|(t, _)| t.trim_end_matches('?').to_owned())
}

enum Member<'a> {
    Method(&'a str, usize),
    /// A field, or Kotlin's property (a Java getter from Kotlin).
    Field(&'a str),
}

/// A member's declared type in `class` (`List<String>`: its type
/// arguments put in), found in its superclasses too, written for `lang`.
fn member_type(finder: &Finder, class: &str, context: &str, member: Member, lang: Lang) -> Option<(String, String)> {
    let arguments = type_arguments(class);
    let mut class = bare_type(class).to_owned();
    let mut context = context.to_owned();
    for _ in 0..8 {
        let file = finder.class_text(&class, &context)?;
        let simple = class.rsplit('.').next().unwrap_or(&class).to_owned();
        let file_lang = lang_of(&file);
        let tree = symbols::parse(file_lang, &file)?;
        let node = find_class(tree.root_node(), &file, &simple)?;
        let parameters = type_parameters(node, &file);
        let found = match file_lang {
            Lang::Kotlin => kotlin_member(node, &file, &member),
            _ => java_member(node, &file, &member),
        };
        if let Some(ty) = found {
            // The class's type parameters replaced by the receiver's arguments.
            let mut ty = ty;
            for (p, a) in parameters.iter().zip(&arguments) {
                ty = replace_word(&ty, p, a);
            }
            if parameters.iter().skip(arguments.len()).any(|p| has_word(&ty, p)) {
                return None;
            }
            let ty = match (file_lang, lang) {
                (Lang::Java, Lang::Kotlin) => kotlin_of_java(&ty)?,
                _ => ty,
            };
            return Some((ty, file));
        }
        let superclass = node.child_by_field_name("superclass").and_then(|s| s.named_child(0))?;
        class = finder.qualify(bare_type(&file[superclass.byte_range()]), &file);
        context = file;
    }
    None
}

/// `Map<String, List<Int>>` → ["String", "List<Int>"].
fn type_arguments(ty: &str) -> Vec<String> {
    let Some(open) = ty.find('<') else { return Vec::new() };
    let inner = &ty[open + 1..ty.rfind('>').unwrap_or(ty.len())];
    let (mut out, mut depth, mut start) = (Vec::new(), 0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim().to_owned());
    out
}

fn type_parameters(class: Node, text: &str) -> Vec<String> {
    let parameters = class.child_by_field_name("type_parameters").or_else(|| {
        let mut c = class.walk();
        class.named_children(&mut c).find(|n| n.kind() == "type_parameters")
    });
    let Some(parameters) = parameters else { return Vec::new() };
    let mut c = parameters.walk();
    parameters
        .named_children(&mut c)
        .filter_map(|p| {
            let mut d = p.walk();
            let name = p.named_children(&mut d).find(|n| matches!(n.kind(), "type_identifier" | "identifier"));
            name.map(|n| text[n.byte_range()].to_owned())
        })
        .collect()
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == word)
}

fn replace_word(text: &str, word: &str, with: &str) -> String {
    let mut out = String::new();
    let mut current = String::new();
    for c in text.chars().chain(std::iter::once('\0')) {
        if c.is_alphanumeric() || c == '_' {
            current.push(c);
            continue;
        }
        out.push_str(if current == word { with } else { &current });
        current.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

/// A Java method's return type (not a method type parameter's), or a
/// field's type; from Kotlin, a property `name` is its getter.
fn java_member(class: Node, text: &str, member: &Member) -> Option<String> {
    let body = class.child_by_field_name("body")?;
    let mut c = body.walk();
    let members: Vec<Node> = body.named_children(&mut c).collect();
    let method = |name: &str, arity: Option<usize>| {
        members.iter().find(|m| {
            m.kind() == "method_declaration"
                && m.child_by_field_name("name").is_some_and(|n| &text[n.byte_range()] == name)
                && arity.is_none_or(|a| {
                    m.child_by_field_name("parameters").is_some_and(|p| {
                        let mut c = p.walk();
                        p.named_children(&mut c).filter(|n| matches!(n.kind(), "formal_parameter" | "spread_parameter")).count() == a
                    })
                })
        })
    };
    let returned = |m: &Node| {
        let ty = text[m.child_by_field_name("type")?.byte_range()].to_owned();
        let own: Vec<String> = m.child_by_field_name("type_parameters").map(|p| type_parameters_of(p, text)).unwrap_or_default();
        (!own.iter().any(|p| has_word(&ty, p))).then_some(ty)
    };
    match member {
        Member::Method(name, arity) => method(name, Some(*arity)).and_then(returned),
        Member::Field(name) => {
            let field = members.iter().find(|m| {
                m.kind() == "field_declaration" && {
                    let mut d = m.walk();
                    m.children_by_field_name("declarator", &mut d).any(|v| v.child_by_field_name("name").is_some_and(|n| &text[n.byte_range()] == *name))
                }
            });
            if let Some(f) = field {
                return f.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned());
            }
            // Kotlin's `user.name`: `getName()` / `isName()`.
            let capital: String = name.chars().take(1).flat_map(char::to_uppercase).chain(name.chars().skip(1)).collect();
            method(&format!("get{capital}"), Some(0)).or_else(|| method(&format!("is{capital}"), Some(0))).and_then(returned)
        }
    }
}

fn type_parameters_of(parameters: Node, text: &str) -> Vec<String> {
    let mut c = parameters.walk();
    parameters
        .named_children(&mut c)
        .filter_map(|p| {
            let mut d = p.walk();
            let name = p.named_children(&mut d).find(|n| matches!(n.kind(), "type_identifier" | "identifier"));
            name.map(|n| text[n.byte_range()].to_owned())
        })
        .collect()
}

/// A Kotlin function's or property's declared type in a class.
fn kotlin_member(class: Node, text: &str, member: &Member) -> Option<String> {
    match member {
        Member::Method(name, arity) => kotlin_functions(class, text, name, *arity).into_iter().next().and_then(|f| kotlin_return(f, text)),
        Member::Field(name) => {
            let mut stack = vec![class];
            while let Some(n) = stack.pop() {
                match n.kind() {
                    "property_declaration" => {
                        let mut c = n.walk();
                        let variable = n.named_children(&mut c).find(|v| v.kind() == "variable_declaration")?;
                        if variable.named_child(0).is_some_and(|v| &text[v.byte_range()] == *name) {
                            if let Some((_, t)) = text[variable.byte_range()].split_once(':') {
                                return Some(t.trim().to_owned());
                            }
                            // `val toy = Toy()`: its initializer's type.
                            let initializer = n.named_children(&mut n.walk()).filter(|v| v.start_byte() > variable.end_byte()).last()?;
                            return super::introduce::kotlin_type(initializer, text);
                        }
                        continue;
                    }
                    "class_parameter" => {
                        let mut c = n.walk();
                        let named = n.named_children(&mut c).any(|v| v.kind() == "simple_identifier" && &text[v.byte_range()] == *name);
                        if named {
                            return text[n.byte_range()].split_once(':').map(|(_, t)| t.split('=').next().unwrap_or(t).trim().to_owned());
                        }
                        continue;
                    }
                    "function_declaration" | "lambda_literal" => continue,
                    _ => {}
                }
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
            }
            None
        }
    }
}

/// A Kotlin function's return type: declared, or its expression body's
/// when that shows it; None for Unit or a type parameter of its own.
fn kotlin_return(function: Node, text: &str) -> Option<String> {
    let mut c = function.walk();
    let children: Vec<Node> = function.named_children(&mut c).collect();
    let own: Vec<String> = children.iter().find(|n| n.kind() == "type_parameters").map(|p| type_parameters_of(*p, text)).unwrap_or_default();
    let after = children.iter().position(|n| n.kind() == "function_value_parameters")?;
    let ty = match children.get(after + 1) {
        Some(t) if matches!(t.kind(), "user_type" | "nullable_type" | "function_type" | "parenthesized_type") => text[t.byte_range()].to_owned(),
        Some(b) if b.kind() == "function_body" && text[b.byte_range()].starts_with('=') => super::introduce::kotlin_type(b.named_child(0)?, text)?,
        _ => return None,
    };
    (ty != "Unit" && !own.iter().any(|p| has_word(&ty, p))).then_some(ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(lang: Lang, text: &str, what: &str, files: &[&str]) -> Option<(String, Vec<String>)> {
        let at = text.find(what).unwrap();
        let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        let classes = |name: &str| -> Vec<String> { files.iter().filter(|t| t.contains(&format!("class {name}")) || t.contains(&format!("interface {name}"))).cloned().collect() };
        type_of(lang, text, at..at + what.len(), &classes).map(|t| (t.ty, t.imports))
    }

    const USER: &str = "package com.model;\n\nimport java.util.List;\n\npublic class User extends Person {\n    private Address address;\n    public Address getAddress() { return address; }\n    public List<String> getTags() { return null; }\n    public boolean isAdmin() { return false; }\n    public <T> T as(Class<T> c) { return null; }\n}\n";
    const PERSON: &str = "package com.model;\n\npublic class Person {\n    public String getName() { return \"\"; }\n}\n";
    const ADDRESS: &str = "package com.model;\n\npublic class Address {\n    public String city;\n}\n";
    const BOX: &str = "package com.model;\n\npublic class Box<T> {\n    public T get() { return null; }\n}\n";

    #[test]
    fn types_java_calls() {
        let files = [USER, PERSON, ADDRESS, BOX];
        let text = "package app;\n\nimport com.model.User;\nimport com.model.Box;\n\nclass A {\n    void f(User user, Box<User> box) {\n        g(user.getAddress().city, user.getName(), user.getTags(), user.as(String.class), box.get().getAddress(), count());\n    }\n    int count() { return 0; }\n}\n";
        assert_eq!(typed(Lang::Java, text, "user.getAddress()", &files), Some(("Address".into(), vec!["com.model.Address".into()])));
        assert_eq!(typed(Lang::Java, text, "user.getAddress().city", &files), Some(("String".into(), vec![])));
        // From the superclass.
        assert_eq!(typed(Lang::Java, text, "user.getName()", &files), Some(("String".into(), vec![])));
        assert_eq!(typed(Lang::Java, text, "user.getTags()", &files), Some(("List<String>".into(), vec!["java.util.List".into()])));
        // The method's own type parameter: unknown.
        assert_eq!(typed(Lang::Java, text, "user.as(String.class)", &files), None);
        // The class's type parameter, from the receiver's argument.
        assert_eq!(typed(Lang::Java, text, "box.get()", &files), Some(("User".into(), vec![])));
        assert_eq!(typed(Lang::Java, text, "box.get().getAddress()", &files), Some(("Address".into(), vec!["com.model.Address".into()])));
        assert_eq!(typed(Lang::Java, text, "count()", &files), Some(("int".into(), vec![])));
    }

    #[test]
    fn types_kotlin_calls() {
        let files = [USER, PERSON, ADDRESS];
        let repo = "package app.data\n\nclass Repo {\n    val size: Int = 0\n    val cache = Cache()\n    fun find(id: Long): User? = null\n    fun all() = listOf<User>()\n    fun clear() {}\n}\n";
        let files = [files[0], files[1], files[2], repo];
        let text = "package app\n\nimport app.data.Repo\nimport com.model.User\n\nfun f(repo: Repo, user: User) {\n    g(repo.find(1), repo.size, user.name, user.isAdmin(), repo.clear(), user.address, total())\n}\n\nfun total(): Long = 0\n";
        assert_eq!(typed(Lang::Kotlin, text, "repo.find(1)", &files), Some(("User?".into(), vec![])));
        assert_eq!(typed(Lang::Kotlin, text, "repo.size", &files), Some(("Int".into(), vec![])));
        // Java getters as properties, Java types as Kotlin's.
        assert_eq!(typed(Lang::Kotlin, text, "user.name", &files), Some(("String".into(), vec![])));
        assert_eq!(typed(Lang::Kotlin, text, "user.isAdmin()", &files), Some(("Boolean".into(), vec![])));
        assert_eq!(typed(Lang::Kotlin, text, "user.address", &files), Some(("Address".into(), vec!["com.model.Address".into()])));
        assert_eq!(typed(Lang::Kotlin, text, "repo.clear()", &files), None);
        assert_eq!(typed(Lang::Kotlin, text, "total()", &files), Some(("Long".into(), vec![])));
        // A property typed by its initializer, read through a local.
        let text = "package app\n\nimport app.data.Repo\n\nfun f() {\n    val repo = Repo()\n    g(repo.cache)\n}\n";
        assert_eq!(typed(Lang::Kotlin, text, "repo.cache", &files), Some(("Cache".into(), vec!["app.data.Cache".into()])));
    }
}

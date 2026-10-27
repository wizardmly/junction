//! The members a class can override or must implement, for Override /
//! Implement Methods: its supertypes' members through the hierarchy
//! (nearest first, so a class's own override of an interface's member
//! wins), read from the project's and libraries' sources, type
//! parameters replaced by the class's arguments; then `Any` / `Object`.
//! Of several types of a name, the one in the referring file wins, then
//! one in its directory (its package), then any.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use super::lang::Lang;
use super::store::ProjectIndex;
use super::symbols::SymbolKind;
use crate::domain::imports;
use crate::domain::overrides::{self as ov, Member, SuperType};

/// The directory part of a project path.
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// (owner's simple name, member) for each member, nearest owner first;
/// `from` is the class's file.
pub fn super_members(index: &ProjectIndex, root: &Path, lang: Lang, from: &str, class_header: &str) -> Vec<(String, Member)> {
    let kotlin = lang == Lang::Kotlin;
    let family = lang.family();
    let mut sources: HashMap<String, Option<String>> = HashMap::new();
    // (supertype, its outer type arguments, the file naming it, the direct
    // supertype it's reached through when that is a Java interface).
    let mut queue: VecDeque<(SuperType, Vec<(String, String)>, String, Option<String>)> = ov::supertypes(class_header, kotlin).into_iter().map(|s| (s, Vec::new(), from.to_owned(), None)).collect();
    let direct: Vec<String> = queue.iter().map(|(s, ..)| s.name.clone()).collect();
    let mut visited = HashSet::new();
    let mut out: Vec<(String, Member)> = Vec::new();
    while let Some((sup, outer, referrer, through)) = queue.pop_front() {
        if !visited.insert(sup.name.clone()) || visited.len() > 64 {
            continue;
        }
        let args: Vec<String> = sup.args.iter().map(|a| ov::substitute(a, &outer)).collect();
        let rank = |path: &str| if path == referrer { 0 } else if dir_of(path) == dir_of(&referrer) { 1 } else { 2 };
        let Some((path, entry, symbol)) = index.symbols_named(&sup.name).filter(|(_, e, s)| s.kind.is_type() && family.contains(&e.lang)).min_by_key(|(p, _, _)| rank(p)) else { continue };
        let source = sources.entry(path.to_owned()).or_insert_with(|| std::fs::read_to_string(root.join(path)).ok());
        let Some(source) = source.clone() else { continue };
        let file_header = imports::header(&source);
        let lines: Vec<&str> = source.split('\n').collect();
        let header_end = (symbol.end_line as usize).min(symbol.line as usize + 8).min(lines.len().saturating_sub(1));
        let header = lines[symbol.line as usize..=header_end].join("\n");
        let header = header.split('{').next().unwrap_or(&header).to_owned();
        let map: Vec<(String, String)> = ov::type_params(&header, &sup.name).into_iter().zip(args).collect();
        let in_interface = symbol.kind == SymbolKind::Interface || header.split_whitespace().any(|w| w == "interface");
        // A Java default method reached through a direct superinterface.
        let through = through.or_else(|| (!kotlin && in_interface && direct.contains(&sup.name)).then(|| sup.name.clone()));
        for m in &entry.symbols {
            let owner = m.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next());
            if owner != Some(sup.name.as_str()) || !matches!(m.kind, SymbolKind::Method | SymbolKind::Function | SymbolKind::Property) {
                continue;
            }
            let Some((member_header, has_body)) = member_header(&lines, m.line as usize, kotlin) else { continue };
            let Some(mut member) = ov::member(&ov::substitute(&member_header, &map), kotlin, in_interface, has_body) else { continue };
            member.doc = ov::doc_comment(&lines, m.line as usize);
            member.through = through.clone();
            // The classes its signature names as the supertype's file sees
            // them; type arguments come from the class itself.
            if let Some(declared) = ov::member(&member_header, kotlin, in_interface, has_body) {
                member.imports = ov::type_names(&declared, kotlin)
                    .into_iter()
                    .filter(|name| !map.iter().any(|(p, _)| p == name))
                    .filter_map(|name| qualified_name(index, root, &mut sources, &file_header, &name, family))
                    .collect();
            }
            out.push((sup.name.clone(), member));
        }
        for next in ov::supertypes(&header, kotlin) {
            queue.push_back((next, map.clone(), path.to_owned(), through.clone()));
        }
    }
    out.extend(ov::root_members(kotlin));
    // The nearest declaration of each member.
    let mut seen = HashSet::new();
    out.retain(|(_, m)| seen.insert(ov::label(m, kotlin)));
    out
}

/// The superclass (not an interface, `Any` or `Object`) of the class
/// with `class_header` and its constructors, as their parameter lists are
/// written, for Generate's constructors to call; none when it has only a
/// no-argument one (or isn't found).
pub fn super_constructors(index: &ProjectIndex, root: &Path, lang: Lang, from: &str, class_header: &str) -> Option<(String, Vec<Vec<String>>)> {
    let kotlin = lang == Lang::Kotlin;
    let family = lang.family();
    let header = class_header.split('{').next().unwrap_or(class_header);
    // Java names it after `extends`; Kotlin's is the supertype that's a class.
    let names: Vec<String> = if kotlin {
        ov::supertypes(header, true).into_iter().map(|s| s.name).collect()
    } else if header.split_whitespace().any(|w| w == "interface") {
        Vec::new()
    } else {
        header.split_once(" extends ").and_then(|(_, rest)| ov::supertypes(&format!("class X extends {}", rest.split(" implements ").next().unwrap_or(rest)), false).into_iter().next()).map(|s| s.name).into_iter().collect()
    };
    let rank = |path: &str| if path == from { 0 } else if dir_of(path) == dir_of(from) { 1 } else { 2 };
    for name in names {
        let Some((path, _, symbol)) = index.symbols_named(&name).filter(|(_, e, s)| s.kind.is_type() && s.container.is_none() && family.contains(&e.lang)).min_by_key(|(p, _, _)| rank(p)) else { continue };
        if symbol.kind == SymbolKind::Interface {
            continue;
        }
        let full = if ProjectIndex::is_external(path) { std::path::PathBuf::from(path) } else { root.join(path) };
        let Ok(source) = std::fs::read_to_string(full) else { continue };
        let line_start: usize = source.split_inclusive('\n').take(symbol.line as usize).map(str::len).sum();
        let Some(at) = source[line_start..].find(name.as_str()).map(|i| line_start + i) else { continue };
        let declaration = &source[line_start..at];
        if declaration.split_whitespace().any(|w| w == "interface") {
            continue;
        }
        let constructors = crate::domain::parameter_info::constructors(&source, at).unwrap_or_default();
        if constructors.iter().all(|c| c.is_empty()) {
            return None;
        }
        // Kotlin defaults aren't part of the type.
        let constructors = constructors.into_iter().map(|c| c.into_iter().map(|p| p.split(" = ").next().unwrap_or(&p).trim().to_owned()).collect()).collect();
        return Some((name, constructors));
    }
    None
}

/// The qualified name of a class a file of `header` names by its simple
/// name: imported by name, or a top-level class of the file's package.
fn qualified_name(index: &ProjectIndex, root: &Path, sources: &mut HashMap<String, Option<String>>, header: &imports::Header, name: &str, family: &[Lang]) -> Option<String> {
    if let Some((_, q)) = header.imports.iter().find(|(n, _)| n == name) {
        return Some(q.clone());
    }
    if header.package.is_empty() {
        return None;
    }
    let declared = index.symbols_named(name).filter(|(_, e, s)| s.kind.is_type() && s.container.is_none() && family.contains(&e.lang)).any(|(path, _, _)| {
        let source = sources.entry(path.to_owned()).or_insert_with(|| std::fs::read_to_string(root.join(path)).ok());
        source.as_deref().is_some_and(|s| imports::header(s).package == header.package)
    });
    declared.then(|| format!("{}.{name}", header.package))
}

/// A member's declaration from its line to where its body or initializer
/// starts, and whether it has one.
fn member_header(lines: &[&str], line: usize, kotlin: bool) -> Option<(String, bool)> {
    let mut text = String::new();
    let mut depth = 0i32;
    for l in lines.iter().skip(line).take(12) {
        for c in l.chars() {
            match c {
                '(' | '<' | '[' => depth += 1,
                ')' | '>' | ']' if depth > 0 => depth -= 1,
                '{' if depth == 0 => return Some((text, true)),
                '=' if depth == 0 && kotlin => return Some((text, true)),
                ';' if depth == 0 => return Some((text, false)),
                _ => {}
            }
            text.push(c);
        }
        if depth == 0 && kotlin {
            return Some((text, false));
        }
        text.push(' ');
    }
    (!text.is_empty()).then_some((text, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_docs_and_imports() {
        let dir = std::env::temp_dir().join(format!("junction-overrides-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let files = [
            ("lib/Shape.kt", "package com.example.lib\n\nimport com.example.model.Item\n\ninterface Shape {\n    /** The area. */\n    fun area(item: Item, unit: Unit2): Double\n}\n"),
            ("lib/Unit2.kt", "package com.example.lib\n\nclass Unit2\n"),
            ("model/Item.kt", "package com.example.model\n\nclass Item\n"),
        ];
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        let mut index = ProjectIndex::load(&dir);
        index.update(&|_, _| {});
        let members = super_members(&index, &dir, Lang::Kotlin, "app/Circle.kt", "class Circle : Shape {");
        let (owner, area) = members.iter().find(|(_, m)| m.name == "area").unwrap();
        assert_eq!(owner, "Shape");
        assert_eq!(area.doc.as_deref(), Some("/** The area. */"));
        assert_eq!(area.imports, ["com.example.model.Item", "com.example.lib.Unit2"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn java_default_methods_go_through_their_interface() {
        let dir = std::env::temp_dir().join(format!("junction-overrides-java-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let files = [
            ("a/Shape.java", "package a;\n\npublic interface Shape {\n    default String name() { return \"shape\"; }\n    double area();\n}\n"),
            ("a/Round.java", "package a;\n\npublic interface Round extends Shape {\n    default double radius() { return 1; }\n}\n"),
            ("a/Base.java", "package a;\n\npublic class Base implements Shape {\n    public double area() { return 0; }\n}\n"),
        ];
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        let mut index = ProjectIndex::load(&dir);
        index.update(&|_, _| {});
        let through = |header: &str, name: &str| {
            let members = super_members(&index, &dir, Lang::Java, "a/C.java", header);
            members.into_iter().find(|(_, m)| m.name == name).map(|(_, m)| m.through)
        };
        // Directly, and inherited through another interface.
        assert_eq!(through("public class C implements Shape {", "name"), Some(Some("Shape".into())));
        assert_eq!(through("public class C implements Round {", "name"), Some(Some("Round".into())));
        assert_eq!(through("public class C implements Round {", "radius"), Some(Some("Round".into())));
        // Through a superclass: plain `super`.
        assert_eq!(through("public class C extends Base {", "name"), Some(None));
        assert_eq!(through("public class C extends Base {", "toString"), Some(None));
        let m = super_members(&index, &dir, Lang::Java, "a/C.java", "public class C implements Shape {").into_iter().find(|(_, m)| m.name == "name").unwrap().1;
        assert_eq!(ov::generate(&m, false, "    "), "@Override\npublic String name() {\n    return Shape.super.name();\n}");
        // Generate's constructors: the superclass's, not an interface's.
        let supers = |lang, header: &str| super_constructors(&index, &dir, lang, "a/C.java", header);
        assert_eq!(supers(Lang::Java, "public class C extends Base implements Shape {"), None);
        std::fs::write(dir.join("a/Animal.java"), "package a;\n\npublic class Animal {\n    public Animal(String name) {}\n    protected Animal(String name, int age) {}\n}\n").unwrap();
        std::fs::write(dir.join("a/Pet.kt"), "package a\n\nopen class Pet(val name: String, age: Int = 0) {\n    constructor(n: Int) : this(\"x\")\n}\n").unwrap();
        index.update(&|_, _| {});
        let supers = |lang, header: &str| super_constructors(&index, &dir, lang, "a/C.java", header);
        assert_eq!(supers(Lang::Java, "public class C extends Animal implements Shape {"), Some(("Animal".into(), vec![vec!["String name".into()], vec!["String name".into(), "int age".into()]])));
        assert_eq!(supers(Lang::Kotlin, "class Dog : Round, Pet {"), Some(("Pet".into(), vec![vec!["name: String".into(), "age: Int".into()], vec!["n: Int".into()]])));
        std::fs::remove_dir_all(dir).ok();
    }
}

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
    let mut queue: VecDeque<(SuperType, Vec<(String, String)>, String)> = ov::supertypes(class_header, kotlin).into_iter().map(|s| (s, Vec::new(), from.to_owned())).collect();
    let mut visited = HashSet::new();
    let mut out: Vec<(String, Member)> = Vec::new();
    while let Some((sup, outer, referrer)) = queue.pop_front() {
        if !visited.insert(sup.name.clone()) || visited.len() > 64 {
            continue;
        }
        let args: Vec<String> = sup.args.iter().map(|a| ov::substitute(a, &outer)).collect();
        let rank = |path: &str| if path == referrer { 0 } else if dir_of(path) == dir_of(&referrer) { 1 } else { 2 };
        let Some((path, entry, symbol)) = index.symbols_named(&sup.name).filter(|(_, e, s)| s.kind.is_type() && family.contains(&e.lang)).min_by_key(|(p, _, _)| rank(p)) else { continue };
        let source = sources.entry(path.to_owned()).or_insert_with(|| std::fs::read_to_string(root.join(path)).ok());
        let Some(source) = source.as_deref() else { continue };
        let lines: Vec<&str> = source.split('\n').collect();
        let header_end = (symbol.end_line as usize).min(symbol.line as usize + 8).min(lines.len().saturating_sub(1));
        let header = lines[symbol.line as usize..=header_end].join("\n");
        let header = header.split('{').next().unwrap_or(&header).to_owned();
        let map: Vec<(String, String)> = ov::type_params(&header, &sup.name).into_iter().zip(args).collect();
        let in_interface = symbol.kind == SymbolKind::Interface || header.split_whitespace().any(|w| w == "interface");
        for m in &entry.symbols {
            let owner = m.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next());
            if owner != Some(sup.name.as_str()) || !matches!(m.kind, SymbolKind::Method | SymbolKind::Function | SymbolKind::Property) {
                continue;
            }
            let Some((member_header, has_body)) = member_header(&lines, m.line as usize, kotlin) else { continue };
            let member_header = ov::substitute(&member_header, &map);
            if let Some(member) = ov::member(&member_header, kotlin, in_interface, has_body) {
                out.push((sup.name.clone(), member));
            }
        }
        for next in ov::supertypes(&header, kotlin) {
            queue.push_back((next, map.clone(), path.to_owned()));
        }
    }
    out.extend(ov::root_members(kotlin));
    // The nearest declaration of each member.
    let mut seen = HashSet::new();
    out.retain(|(_, m)| seen.insert(ov::label(m, kotlin)));
    out
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

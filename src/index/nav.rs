//! Navigation over the built-in index: Go to Declaration and Find Usages.
//! Language servers answer first when available (see `lsp`); this is the
//! fallback and the cross-language part no single server knows.

use std::collections::HashSet;
use std::path::Path;

use super::bridge::Role;
use super::lang::Lang;
use super::store::ProjectIndex;
use super::symbols::SymbolKind;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    /// Relative to the project root.
    pub path: String,
    /// 0-based line and UTF-16 column.
    pub line: u32,
    pub col: u32,
    pub name: String,
    /// "method · Kotlin", "JNI", …
    pub label: String,
    pub container: Option<String>,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The identifier around a byte offset, and its byte range.
pub fn word_at(text: &str, offset: usize) -> Option<(String, std::ops::Range<usize>)> {
    let offset = offset.min(text.len());
    let mut start = offset;
    while start > 0 {
        let prev = text[..start].chars().next_back()?;
        if !is_word_char(prev) {
            break;
        }
        start -= prev.len_utf8();
    }
    let mut end = offset;
    while end < text.len() {
        let next = text[end..].chars().next()?;
        if !is_word_char(next) {
            break;
        }
        end += next.len_utf8();
    }
    (start < end && !text[start..end].chars().next()?.is_ascii_digit()).then(|| (text[start..end].to_owned(), start..end))
}

/// (0-based line, UTF-16 column) of a byte offset.
pub fn position(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].encode_utf16().count() as u32)
}

fn symbol_target(path: &str, lang: Lang, s: &super::symbols::Symbol) -> Target {
    Target {
        path: path.to_owned(),
        line: s.line,
        col: s.col,
        name: s.name.clone(),
        label: format!("{} · {}", s.kind.label(), lang.name()),
        container: s.container.clone(),
    }
}

/// The other side of a bridge site at `offset` (JNI, C ABI, FFI, Swift /
/// Objective-C), or nothing when the identifier there isn't one.
pub fn bridge_definitions(index: &ProjectIndex, path: &str, text: &str, offset: usize) -> Vec<Target> {
    let Some((word, range)) = word_at(text, offset) else { return Vec::new() };
    let (line, col) = position(text, range.start);
    let entry = index.files.get(path);
    if let Some(entry) = entry {
        let sites: Vec<_> = entry.bridges.iter().filter(|b| b.line == line && b.col == col && b.name == word).collect();
        let mut out = Vec::new();
        for site in &sites {
            let want = match site.role {
                Role::Import => Role::Export,
                Role::Export => Role::Import,
            };
            for (other_path, other_entry, other) in index.bridges_keyed(&site.key) {
                if other.role != want || (other_path == path && other.line == line) {
                    continue;
                }
                // A RegisterNatives entry stands for its C function.
                if let Some(via) = &other.via {
                    if let Some(s) = other_entry.symbols.iter().find(|s| &s.name == via && !s.decl) {
                        out.push(symbol_target(other_path, other_entry.lang, s));
                        continue;
                    }
                }
                out.push(Target {
                    path: other_path.to_owned(),
                    line: other.line,
                    col: other.col,
                    name: other.name.clone(),
                    label: format!("{} · {}", other.label(), other_entry.lang.name()),
                    container: None,
                });
            }
        }
        return dedup(out);
    }
    Vec::new()

}

/// Go to Declaration for the identifier at `offset` in `path`.
pub fn definitions(index: &ProjectIndex, path: &str, lang: Lang, text: &str, offset: usize) -> Vec<Target> {
    let Some((word, range)) = word_at(text, offset) else { return Vec::new() };
    let (line, col) = position(text, range.start);

    // 1. A bridge site: jump to the other side.
    let bridged = bridge_definitions(index, path, text, offset);
    if !bridged.is_empty() {
        return bridged;
    }

    // 2. Symbols with this name in the languages this one can see.
    let family = lang.family();
    let mut candidates: Vec<(i32, Target)> = Vec::new();
    let mut on_definition = false;
    let mut hints = import_hints(lang, text);
    if matches!(lang, Lang::C | Lang::Cpp | Lang::ObjC) {
        hints = included_hints(index, hints);
    }
    // `x.name`: the type of `x` picks among same-named members.
    let receiver = receiver_type(index, lang, text, range.start, 0);
    let as_type = used_as_type(text, range.end);
    for (p, e, s) in index.symbols_named(&word) {
        if !family.contains(&e.lang) {
            continue;
        }
        if p == path && s.line == line && s.col == col {
            on_definition = true;
            continue;
        }
        let mut score = 0;
        if p == path {
            score += 100;
        } else if !ProjectIndex::is_external(p) {
            score += PROJECT;
        } else {
            // A library symbol: the file's imports and the types it
            // mentions decide, since the index knows no types.
            if hinted(p, &hints) {
                score += IMPORTED;
            }
            if s.container.as_deref().and_then(|c| c.rsplit('.').next()).is_some_and(|c| mentions(text, c)) {
                score += 25;
            }
        }
        if let Some(receiver) = &receiver {
            if s.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next()) == Some(receiver.as_str()) {
                score += RECEIVER;
            }
        }
        if as_type && s.kind.is_type() {
            score += 15;
        }
        if e.lang == lang {
            score += 40;
        }
        if !s.decl {
            score += 20;
        }
        score += shared_prefix(p, path) as i32;
        candidates.push((score, symbol_target(p, e.lang, s)));
    }
    // On a definition, IntelliJ shows its other declarations (header ↔
    // source); a C definition with a counterpart elsewhere goes there.
    if candidates.is_empty() && on_definition {
        return Vec::new();
    }
    // Only prototypes here: the implementation may live across the C ABI
    // (Rust #[no_mangle], Swift @_cdecl, Go //export…).
    if !candidates.is_empty() {
        let all_decls = index.symbols_named(&word).filter(|(_, e, _)| family.contains(&e.lang)).all(|(_, _, s)| s.decl);
        if all_decls {
            let exports: Vec<Target> = index
                .bridges_keyed(&format!("c:{word}"))
                .filter(|(_, _, b)| b.role == Role::Export)
                .map(|(p, e, b)| Target { path: p.to_owned(), line: b.line, col: b.col, name: b.name.clone(), label: format!("{} · {}", b.label(), e.lang.name()), container: None })
                .collect();
            if !exports.is_empty() {
                return dedup(exports);
            }
        }
    }
    // Members of the receiver's type win over every other same-named member.
    if candidates.iter().any(|(s, _)| *s >= RECEIVER) && receiver.is_some() {
        candidates.retain(|(s, _)| *s >= RECEIVER);
    }
    // Unrelated library symbols only count when nothing better exists.
    let strong = candidates.iter().any(|(s, t)| !ProjectIndex::is_external(&t.path) || *s >= IMPORTED);
    if strong {
        candidates.retain(|(s, t)| !ProjectIndex::is_external(&t.path) || *s >= IMPORTED);
    }
    if !candidates.is_empty() {
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)).then(a.1.line.cmp(&b.1.line)));
        candidates.truncate(MAX_CHOICES);
        // Same-file hits win outright: locals and members shadow the rest.
        let best = candidates[0].0;
        let cut = if best >= 100 { 100 } else { i32::MIN };
        return dedup(candidates.into_iter().filter(|(s, _)| *s >= cut).map(|(_, t)| t).collect());
    }

    // 3. Names other languages expose to this one.
    let mut out = Vec::new();
    let keys: &[&str] = match lang {
        Lang::Swift => &["objc", "c"],
        Lang::ObjC => &["objc", "c"],
        Lang::C | Lang::Cpp => &["c"],
        _ => &[],
    };
    for key in keys {
        for (p, e, b) in index.bridges_keyed(&format!("{key}:{word}")) {
            if b.role == Role::Export {
                out.push(Target { path: p.to_owned(), line: b.line, col: b.col, name: b.name.clone(), label: format!("{} · {}", b.label(), e.lang.name()), container: None });
            }
        }
    }
    // Objective-C sees Swift classes through the generated -Swift.h header.
    if out.is_empty() && lang == Lang::ObjC {
        for (p, e, s) in index.symbols_named(&word) {
            if e.lang == Lang::Swift && s.kind.is_type() {
                out.push(symbol_target(p, e.lang, s));
            }
        }
    }
    dedup(out)
}

/// Rank of a project symbol over a library's.
const PROJECT: i32 = 50;
/// Rank of a library symbol the file imports.
const IMPORTED: i32 = 60;
/// Rank of a member of the receiver's type.
const RECEIVER: i32 = 1000;
/// Most targets offered in the chooser.
const MAX_CHOICES: usize = 40;

fn mentions(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

/// `List<String> x`, `Foo bar =`: the word ending at `end` names a type.
fn used_as_type(text: &str, end: usize) -> bool {
    let rest = &text[end..];
    let trimmed = rest.trim_start_matches([' ', '\t']);
    if trimmed.starts_with('<') || trimmed.starts_with("[]") {
        return true;
    }
    if trimmed.len() == rest.len() {
        return false;
    }
    let ident = trimmed.find(|c: char| !is_word_char(c)).unwrap_or(trimmed.len());
    ident > 0 && trimmed[ident..].trim_start().starts_with(['=', ';', ',', ')', ':'])
}

/// The identifier before `.` / `->` / `::` / `?.` ending right before `start`.
fn qualifier(text: &str, start: usize) -> Option<(String, usize)> {
    let before = text[..start].trim_end();
    let before = before.strip_suffix("?.").or_else(|| before.strip_suffix("!!.")).or_else(|| before.strip_suffix('.')).or_else(|| before.strip_suffix("->")).or_else(|| before.strip_suffix("::"))?;
    let before = before.trim_end().trim_end_matches(['?', '!', ')']);
    // `foo().bar`: the call's name stands in for its result.
    let before = before.trim_end_matches(|c: char| c == '(');
    let end = before.len();
    let begin = before.rfind(|c: char| !is_word_char(c)).map_or(0, |i| i + before[i..].chars().next().unwrap().len_utf8());
    let word = &before[begin..end];
    (!word.is_empty() && !word.starts_with(|c: char| c.is_ascii_digit())).then(|| (word.to_owned(), begin))
}

/// The declared type of the receiver of the member at `start`, from the
/// file's own declarations or a member's declaration line. A best effort:
/// the index has no types.
fn receiver_type(index: &ProjectIndex, lang: Lang, text: &str, start: usize, depth: usize) -> Option<String> {
    let (name, at) = qualifier(text, start)?;
    if matches!(name.as_str(), "this" | "self" | "super" | "Self") {
        return None;
    }
    // A type or namespace: `System.out`, `Build.VERSION`, `std::vector`.
    if name.starts_with(|c: char| c.is_uppercase()) || matches!(lang, Lang::Cpp | Lang::Rust) && text[at + name.len()..].trim_start().starts_with("::") {
        return Some(name);
    }
    if let Some(ty) = declared_type(text, &name) {
        return Some(ty);
    }
    // A member of another receiver: `System.out` → the field `out` of `System`.
    if depth < 2 {
        let owner = receiver_type(index, lang, text, at, depth + 1)?;
        for (p, _, s) in index.symbols_named(&name) {
            if s.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next()) != Some(owner.as_str()) {
                continue;
            }
            let path = if ProjectIndex::is_external(p) { std::path::PathBuf::from(p) } else { index.root.join(p) };
            let line = std::fs::read_to_string(&path).ok().and_then(|t| t.lines().nth(s.line as usize).map(str::to_owned))?;
            if let Some(ty) = declared_type(&line, &name).or_else(|| return_type(&line, &name)) {
                return Some(ty);
            }
        }
    }
    None
}

/// `Type name`, `Type<…> name`, `name: Type`, `name = Type(` / `new Type(`.
fn declared_type(text: &str, name: &str) -> Option<String> {
    let name = regex::escape(name);
    let patterns = [
        format!(r"\b([A-Z][\w]*)\s*(?:<[^;=(){{}}]*>)?(?:\[\])?\s*[*&]?\s*\b{name}\s*[;=,)]"),
        format!(r"\b(?:val|var|let|const)?\s*\b{name}\s*:\s*(?:\[)?([A-Z]\w*)"),
        format!(r"\b{name}\s*(?::=|=)\s*(?:new\s+|&)?([A-Z]\w*)\s*[(<{{.]"),
    ];
    for pattern in patterns {
        let Ok(re) = regex::Regex::new(&pattern) else { continue };
        if let Some(c) = re.captures(text) {
            return Some(c[1].to_owned());
        }
    }
    None
}

/// A method's return type from its declaration line: `Foo name(`, `fun name(): Foo`, `-> Foo`.
fn return_type(line: &str, name: &str) -> Option<String> {
    let name = regex::escape(name);
    let re = regex::Regex::new(&format!(r"(?:\b([A-Z]\w*)(?:<[^()]*>)?\s+{name}\s*\(|\b{name}\s*\([^)]*\)\s*(?::|->)\s*([A-Z]\w*))")).ok()?;
    let c = re.captures(line)?;
    c.get(1).or(c.get(2)).map(|m| m.as_str().to_owned())
}

/// C includes plus what those library headers include in turn, so
/// `<vector>` reaches `bits/stl_vector.h`.
fn included_hints(index: &ProjectIndex, hints: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = hints.clone();
    let mut seen: HashSet<String> = hints.iter().cloned().collect();
    let mut frontier = hints;
    for _ in 0..2 {
        let mut next = Vec::new();
        for hint in &frontier {
            let suffix = format!("/{hint}");
            let Some(file) = index.external.libraries.iter().flat_map(|l| l.files.iter()).find(|f| f.replace('\\', "/").ends_with(&suffix)) else { continue };
            let text = std::fs::read_to_string(file).unwrap_or_default();
            for include in import_hints(Lang::C, &text) {
                if seen.insert(include.clone()) {
                    next.push(include);
                }
            }
        }
        out.extend(next.iter().cloned());
        frontier = next;
        if out.len() > 300 {
            break;
        }
    }
    out
}

/// Path fragments a file's imports point at: "java/util/List",
/// "kotlinx/coroutines/", "fmt/", "serde/de/", "stdio.h"…
pub fn import_hints(lang: Lang, text: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static JVM: OnceLock<regex::Regex> = OnceLock::new();
    static GO: OnceLock<regex::Regex> = OnceLock::new();
    static RUST: OnceLock<regex::Regex> = OnceLock::new();
    static DART: OnceLock<regex::Regex> = OnceLock::new();
    static C: OnceLock<regex::Regex> = OnceLock::new();
    static PY: OnceLock<regex::Regex> = OnceLock::new();
    static JS: OnceLock<regex::Regex> = OnceLock::new();
    static SWIFT: OnceLock<regex::Regex> = OnceLock::new();
    let re = |slot: &'static OnceLock<regex::Regex>, pattern: &str| slot.get_or_init(|| regex::Regex::new(pattern).unwrap());
    let captures = |re: &regex::Regex| re.captures_iter(text).filter_map(|c| c.get(1).map(|m| m.as_str().to_owned())).collect::<Vec<_>>();
    let mut out = Vec::new();
    match lang {
        Lang::Java | Lang::Kotlin => {
            for import in captures(re(&JVM, r"(?m)^\s*import\s+(?:static\s+)?([\w.]+)")) {
                let path = import.replace('.', "/");
                if let Some((parent, _)) = path.rsplit_once('/') {
                    out.push(format!("{parent}/"));
                }
                out.push(path);
            }
            // Same-package and java.lang / kotlin defaults.
            out.extend(["java/lang/".to_owned(), "kotlin/".to_owned(), "kotlin/collections/".to_owned(), "kotlin/text/".to_owned(), "kotlin/io/".to_owned()]);
        }
        Lang::Go => out.extend(captures(re(&GO, r#"(?m)^\s*(?:import\s+)?(?:[\w.]+\s+)?"([\w./\-]+)"\s*$"#)).into_iter().map(|p| format!("{p}/"))),
        Lang::Rust => {
            for import in captures(re(&RUST, r"(?m)^\s*(?:pub\s+)?use\s+([\w:]+)")) {
                let parts: Vec<&str> = import.split("::").filter(|p| !p.is_empty()).collect();
                if parts.len() > 1 {
                    out.push(format!("{}/", parts[..parts.len() - 1].join("/")));
                }
                out.push(parts.join("/"));
            }
            out.extend(["core/".to_owned(), "alloc/".to_owned(), "std/prelude/".to_owned()]);
        }
        Lang::Dart => {
            for import in captures(re(&DART, r#"(?m)^\s*(?:import|export)\s+['"]([^'"]+)['"]"#)) {
                if let Some(rest) = import.strip_prefix("package:") {
                    out.push(rest.to_owned());
                    out.push(format!("{}/", rest.split('/').next().unwrap_or(rest)));
                } else if let Some(rest) = import.strip_prefix("dart:") {
                    out.push(format!("{rest}/"));
                }
            }
            out.push("core/".to_owned());
        }
        Lang::C | Lang::Cpp | Lang::ObjC => out.extend(captures(re(&C, r#"(?m)^\s*#\s*(?:include|import)\s*[<"]([^>"]+)[>"]"#))),
        Lang::Python => out.extend(captures(re(&PY, r"(?m)^\s*(?:from|import)\s+([\w.]+)")).into_iter().map(|p| p.replace('.', "/"))),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            out.extend(captures(re(&JS, r#"(?:from\s+|require\(\s*|import\s+)['"]([^'"./][^'"]*)['"]"#)).into_iter().map(|p| format!("node_modules/{p}")))
        }
        Lang::Swift => out.extend(captures(re(&SWIFT, r"(?m)^\s*(?:@\w+\s+)*import\s+(?:\w+\s+)?(\w+)")).into_iter().map(|m| format!("{m}.")).chain(["Swift.".to_owned()])),
        Lang::V => out.extend(captures(re(&JVM, r"(?m)^\s*import\s+(?:static\s+)?([\w.]+)")).into_iter().map(|p| format!("{}/", p.replace('.', "/"))).chain(["builtin/".to_owned()])),
    }
    out
}

/// Whether a library file is one of the hinted ones: its path, without
/// versions and `src` / `lib` levels, contains a hint at a segment start.
fn hinted(path: &str, hints: &[String]) -> bool {
    if hints.is_empty() {
        return false;
    }
    let mut normal = String::with_capacity(path.len());
    for segment in path.replace('\\', "/").split('/') {
        if matches!(segment, "src" | "lib" | "library" | "Headers" | "Modules") {
            continue;
        }
        // serde-1.0.219 → serde; module@v1.2.0 → module; Foo.framework → Foo.
        let segment = segment.split('@').next().unwrap_or(segment);
        let segment = match segment.rfind('-') {
            Some(i) if segment[i + 1..].starts_with(|c: char| c.is_ascii_digit()) => &segment[..i],
            _ => segment,
        };
        let segment = segment.strip_suffix(".framework").or_else(|| segment.strip_suffix(".swiftmodule")).unwrap_or(segment);
        normal.push('/');
        normal.push_str(segment);
    }
    hints.iter().any(|h| normal.contains(&format!("/{h}")))
}

fn shared_prefix(a: &str, b: &str) -> usize {
    a.split('/').zip(b.split('/')).take_while(|(x, y)| x == y).count()
}

fn dedup(targets: Vec<Target>) -> Vec<Target> {
    let mut seen = HashSet::new();
    targets.into_iter().filter(|t| seen.insert((t.path.clone(), t.line, t.col))).collect()
}

/// One Find Usages hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub text: String,
    /// "Rust", "JNI", "declaration", …
    pub group: String,
}

/// Find Usages: whole-word occurrences in indexed source files (via
/// `git grep`), plus the other side of any bridge the symbol crosses.
pub fn usages(index: &ProjectIndex, root: &Path, word: &str, lang: Option<Lang>) -> Vec<Usage> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    // The symbol's own language family lists first.
    let family: Vec<Lang> = lang.map(|l| l.family().to_vec()).unwrap_or_default();

    // Bridges: every site whose key mentions this name on either side.
    let mut keys: HashSet<String> = HashSet::new();
    for entry in index.files.values() {
        for b in entry.bridges.iter().filter(|b| b.name == word || b.via.as_deref() == Some(word)) {
            keys.insert(b.key.clone());
        }
    }
    for key in &keys {
        for (p, e, b) in index.bridges_keyed(key) {
            if seen.insert((p.to_owned(), b.line)) {
                let text = line_of(root, p, b.line);
                out.push(Usage { path: p.to_owned(), line: b.line, col: b.col, text, group: format!("{} ({})", b.label(), e.lang.name()) });
            }
        }
    }

    let output = crate::git::git_process()
        .args(["grep", "-n", "--column", "-w", "-I", "-F", "--untracked", "--exclude-standard", "-e", word])
        .current_dir(root)
        .output();
    if let Ok(output) = output {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut parts = line.splitn(4, ':');
            let (Some(path), Some(l), Some(c), Some(text)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
            let Some(entry) = index.files.get(path) else { continue };
            let (Ok(l), Ok(c)) = (l.parse::<u32>(), c.parse::<u32>()) else { continue };
            let line0 = l.saturating_sub(1);
            if !seen.insert((path.to_owned(), line0)) {
                continue;
            }
            // git reports a byte column; editors want UTF-16.
            let byte_col = (c.saturating_sub(1) as usize).min(text.len());
            let col = text.get(..byte_col).map_or(0, |s| s.encode_utf16().count() as u32);
            let is_decl = entry.symbols.iter().any(|s| s.line == line0 && s.name == word);
            let group = if is_decl { "Declarations".to_owned() } else { entry.lang.name().to_owned() };
            out.push(Usage { path: path.to_owned(), line: line0, col, text: text.trim().to_owned(), group });
        }
    }
    let rank = |u: &Usage| {
        let lang = index.files.get(&u.path).map(|e| e.lang);
        (u.group != "Declarations", !lang.is_some_and(|l| family.contains(&l)))
    };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.group.cmp(&b.group)).then(a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    out
}

fn line_of(root: &Path, path: &str, line: u32) -> String {
    std::fs::read_to_string(root.join(path))
        .ok()
        .and_then(|t| t.lines().nth(line as usize).map(|l| l.trim().to_owned()))
        .unwrap_or_default()
}

/// One Go to Symbol / Go to Class match.
#[derive(Clone, Debug)]
pub struct SymbolMatch {
    pub target: Target,
    pub kind: SymbolKind,
    pub score: i32,
}

/// IntelliJ-style matching: the query's characters in order, each one
/// either right after the previous match or at a word start (camel hump,
/// after `_`, `.`, `/`, `-`, a digit run). A plain substring also matches,
/// ranked lower. `None` when the query doesn't match.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace() && *c != '*').flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let starts: Vec<bool> = (0..chars.len())
        .map(|i| {
            i == 0
                || (chars[i].is_uppercase() && !chars[i - 1].is_uppercase())
                || (chars[i].is_alphanumeric() && !chars[i - 1].is_alphanumeric())
                || (chars[i].is_ascii_digit() != chars[i - 1].is_ascii_digit() && chars[i].is_alphanumeric())
        })
        .collect();
    // Depth-first over match positions; queries are short.
    fn walk(q: &[char], qi: usize, lower: &[char], starts: &[bool], from: usize, prev: Option<usize>, budget: &mut u32) -> Option<i32> {
        if qi == q.len() {
            return Some(0);
        }
        let mut best: Option<i32> = None;
        for i in from..lower.len() {
            if *budget == 0 {
                break;
            }
            if lower[i] != q[qi] {
                continue;
            }
            let contiguous = prev == Some(i.wrapping_sub(1));
            if !contiguous && !starts[i] {
                continue;
            }
            *budget -= 1;
            let here = if contiguous { 6 } else { 10 } + if i == qi { 3 } else { 0 };
            if let Some(rest) = walk(q, qi + 1, lower, starts, i + 1, Some(i), budget) {
                let total = here + rest;
                if best.is_none_or(|b| total > b) {
                    best = Some(total);
                }
            }
        }
        best
    }
    let mut budget = 2000;
    let hump = walk(&q, 0, &lower, &starts, 0, None, &mut budget);
    let needle: String = q.iter().collect();
    let hay: String = lower.iter().collect();
    let substring = hay.find(&needle).map(|_| q.len() as i32 * 4);
    let score = match (hump, substring) {
        (Some(h), Some(s)) => h.max(s),
        (Some(h), None) => h,
        (None, Some(s)) => s,
        (None, None) => return None,
    };
    let prefix = if hay.starts_with(&needle) { 15 } else { 0 };
    Some(score + prefix - (chars.len() as i32 - q.len() as i32).max(0) / 4)
}

#[cfg(test)]
#[test]
fn intellij_matching() {
    assert!(fuzzy_score("search", "NvAPI_SYS_GetDriverAndBranchVersion_t").is_none());
    assert!(fuzzy_score("dap", "DiffApplier").is_some());
    assert!(fuzzy_score("fiv", "FileIndexView").is_some());
    assert!(fuzzy_score("fsv", "find_view.rs").is_none() || fuzzy_score("fv", "find_view.rs").is_some());
    assert!(fuzzy_score("findv", "find_view.rs").is_some());
    assert!(fuzzy_score("earch", "research").is_some());
    assert!(fuzzy_score("Main", "MainActivity").unwrap() > fuzzy_score("Main", "DomainModel").unwrap_or(-100));
}

/// Go to Symbol (or Go to Class with `types_only`), best first.
pub fn search_symbols(index: &ProjectIndex, query: &str, types_only: bool, limit: usize) -> Vec<SymbolMatch> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<SymbolMatch> = Vec::new();
    for name in index.names().chain(index.external_names()) {
        let Some(score) = fuzzy_score(query, name) else { continue };
        for (p, e, s) in index.symbols_named(name) {
            if (types_only && !s.kind.is_type()) || s.decl && types_only {
                continue;
            }
            let exact = if name.eq_ignore_ascii_case(query) { 50 } else { 0 };
            // Library items follow the project's, as with IntelliJ's
            // "Include non-project items".
            let external = if ProjectIndex::is_external(p) { 40 } else { 0 };
            out.push(SymbolMatch { target: symbol_target(p, e.lang, s), kind: s.kind, score: score + exact - s.decl as i32 * 5 - external });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.target.name.len().cmp(&b.target.name.len())).then(a.target.path.cmp(&b.target.path)));
    out.truncate(limit);
    out
}

/// File Structure (Ctrl+F12): the file's symbols in source order.
pub fn file_symbols(index: &ProjectIndex, path: &str) -> Vec<SymbolMatch> {
    let Some(entry) = index.file(path) else { return Vec::new() };
    let mut out: Vec<SymbolMatch> = entry
        .symbols
        .iter()
        .map(|s| SymbolMatch { target: symbol_target(path, entry.lang, s), kind: s.kind, score: 0 })
        .collect();
    out.sort_by_key(|m| (m.target.line, m.target.col));
    out
}

/// Go to File, best first.
pub fn search_files(index: &ProjectIndex, query: &str, limit: usize) -> Vec<(String, i32)> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    // A query with a slash matches against the whole path, otherwise the name.
    let mut out: Vec<(String, i32)> = index
        .all_files
        .iter()
        .filter_map(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            let score = if query.contains('/') { fuzzy_score(query, path)? } else { fuzzy_score(query, name).map(|s| s + 20).or_else(|| fuzzy_score(query, path))? };
            Some((path.clone(), score))
        })
        .collect();
    // Library files, after the project's.
    for library in &index.external.libraries {
        for path in &library.files {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
            if let Some(score) = fuzzy_score(query, name) {
                out.push((path.clone(), score + 20 - 40));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.len().cmp(&b.0.len())));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> (std::path::PathBuf, ProjectIndex) {
        let dir = std::env::temp_dir().join(format!("junction-nav-{}-{}", std::process::id(), files.len()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        let mut index = ProjectIndex::load(&dir);
        index.update(&|_, _| {});
        (dir, index)
    }

    fn at(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap() + 1
    }

    #[test]
    fn jumps_across_jni_and_ffi() {
        let java = "package com.ex;\npublic class Native {\n  public static native int add(int a, int b);\n  int use() { return add(1, 2); }\n}\n";
        let c = "#include <jni.h>\nJNIEXPORT jint JNICALL Java_com_ex_Native_add(JNIEnv *env, jclass c, jint a, jint b) { return rust_add(a, b); }\nint rust_add(int a, int b);\n";
        let rust = "#[no_mangle]\npub extern \"C\" fn rust_add(a: i32, b: i32) -> i32 { a + b }\n";
        let dart = "final add = lib.lookupFunction<A, B>('rust_add');\n";
        let (dir, index) = project(&[("app/src/main/java/com/ex/Native.java", java), ("app/src/main/cpp/native.c", c), ("rust/src/lib.rs", rust), ("lib/ffi.dart", dart)]);

        // Java native method → C implementation.
        let t = definitions(&index, "app/src/main/java/com/ex/Native.java", Lang::Java, java, at(java, "add(int a"));
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].path.as_str(), t[0].line), ("app/src/main/cpp/native.c", 1));
        // A call to the native method resolves to the Java declaration.
        let t = definitions(&index, "app/src/main/java/com/ex/Native.java", Lang::Java, java, at(java, "add(1"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("app/src/main/java/com/ex/Native.java", 2));
        // C implementation → Java native declaration.
        let t = definitions(&index, "app/src/main/cpp/native.c", Lang::C, c, at(c, "Java_com"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("app/src/main/java/com/ex/Native.java", 2));
        // A C call whose only C match is a prototype → the Rust implementation.
        let t = definitions(&index, "app/src/main/cpp/native.c", Lang::C, c, at(c, "rust_add(a"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("rust/src/lib.rs", 1));
        // C prototype → Rust #[no_mangle] function.
        let t = definitions(&index, "app/src/main/cpp/native.c", Lang::C, c, at(c, "rust_add(int"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("rust/src/lib.rs", 1));
        // Dart FFI lookup → Rust.
        let t = definitions(&index, "lib/ffi.dart", Lang::Dart, dart, at(dart, "rust_add"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("rust/src/lib.rs", 1));
        // Find Usages of the Rust export reaches C and Dart.
        let u = usages(&index, &dir, "rust_add", Some(Lang::Rust));
        let paths: HashSet<&str> = u.iter().map(|u| u.path.as_str()).collect();
        assert!(paths.contains("lib/ffi.dart") && paths.contains("app/src/main/cpp/native.c") && paths.contains("rust/src/lib.rs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn swift_reaches_objc() {
        let objc = "@interface GGLoader : NSObject\n- (instancetype)initWithURL:(NSURL *)url;\n- (void)start;\n@end\n";
        let swift = "let l = GGLoader.init(url: u)\nl.start()\n";
        let (dir, index) = project(&[("Sources/GGLoader.h", objc), ("App/Main.swift", swift)]);
        let t = definitions(&index, "App/Main.swift", Lang::Swift, swift, at(swift, "GGLoader"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("Sources/GGLoader.h", 0));
        let t = definitions(&index, "App/Main.swift", Lang::Swift, swift, at(swift, "start"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("Sources/GGLoader.h", 2));
        let t = definitions(&index, "App/Main.swift", Lang::Swift, swift, at(swift, "init"));
        assert_eq!((t[0].path.as_str(), t[0].line), ("Sources/GGLoader.h", 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fuzzy_matching() {
        assert!(fuzzy_score("rc", "RepoCache").unwrap() > fuzzy_score("rc", "parcel").unwrap_or(-100));
        assert!(fuzzy_score("xyz", "RepoCache").is_none());
        assert!(fuzzy_score("repo", "repo").unwrap() > fuzzy_score("repo", "repository_view").unwrap());
    }
}

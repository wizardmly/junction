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
    if !candidates.is_empty() {
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)).then(a.1.line.cmp(&b.1.line)));
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
    for name in index.names() {
        let Some(score) = fuzzy_score(query, name) else { continue };
        for (p, e, s) in index.symbols_named(name) {
            if (types_only && !s.kind.is_type()) || s.decl && types_only {
                continue;
            }
            let exact = if name.eq_ignore_ascii_case(query) { 50 } else { 0 };
            out.push(SymbolMatch { target: symbol_target(p, e.lang, s), kind: s.kind, score: score + exact - s.decl as i32 * 5 });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.target.name.len().cmp(&b.target.name.len())).then(a.target.path.cmp(&b.target.path)));
    out.truncate(limit);
    out
}

/// File Structure (Ctrl+F12): the file's symbols in source order.
pub fn file_symbols(index: &ProjectIndex, path: &str) -> Vec<SymbolMatch> {
    let Some(entry) = index.files.get(path) else { return Vec::new() };
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

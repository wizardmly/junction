//! Groovy build scripts (`build.gradle`) are not indexed, but Android
//! Studio still resolves the `buildSrc` constants they use
//! (`Deps.coreKtx`, `Versions.compileSdk`): a name qualified by a project
//! class resolves to that class's Kotlin or Java member, and the class
//! name itself to the class.

use super::lang::Lang;
use super::nav::{Target, symbol_target, word_at};
use super::store::{FileEntry, ProjectIndex};
use super::symbols::{Symbol, SymbolKind};

pub fn is_groovy_script(path: &str) -> bool {
    path.ends_with(".gradle")
}

fn is_type(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum | SymbolKind::Struct)
}

/// The dotted names before `start` (`Deps.Android` in `Deps.Android.minSdk`),
/// when the chain starts with a type name.
fn qualifier(text: &str, start: usize) -> Option<&str> {
    let before = text[..start].strip_suffix('.')?;
    let from = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).map_or(0, |i| i + 1);
    let chain = &before[from..];
    chain.chars().next().filter(|c| c.is_uppercase())?;
    (!chain.is_empty() && !chain.contains("..")).then_some(chain)
}

/// The project's Kotlin and Java types the qualifier names, with their full
/// container path (`Deps.Android`).
fn types<'a>(index: &'a ProjectIndex, chain: &str) -> Vec<(&'a str, &'a FileEntry, String)> {
    let (outer, last) = chain.rsplit_once('.').map_or((None, chain), |(o, l)| (Some(o), l));
    index
        .project_symbols_named(last)
        .filter(|(_, e, s)| matches!(e.lang, Lang::Kotlin | Lang::Java) && is_type(s.kind))
        .filter(|(_, _, s)| match (outer, s.container.as_deref()) {
            (None, _) => true,
            (Some(outer), Some(c)) => c == outer || c.ends_with(&format!(".{outer}")),
            (Some(_), None) => false,
        })
        .map(|(p, e, s)| (p, e, s.container.as_ref().map_or_else(|| s.name.clone(), |c| format!("{c}.{}", s.name))))
        .collect()
}

/// The members a type declares directly.
fn members<'a>(entry: &'a FileEntry, container: &'a str) -> impl Iterator<Item = &'a Symbol> + 'a {
    entry.symbols.iter().filter(move |s| s.container.as_deref() == Some(container) && !matches!(s.kind, SymbolKind::Constructor))
}

/// Go to Declaration in a Groovy build script.
pub fn targets(index: &ProjectIndex, text: &str, offset: usize) -> Vec<Target> {
    let Some((word, range)) = word_at(text, offset) else { return Vec::new() };
    match qualifier(text, range.start) {
        Some(chain) => types(index, chain)
            .into_iter()
            .flat_map(|(path, entry, container)| {
                members(entry, &container).filter(|s| s.name == word).map(|s| symbol_target(path, entry.lang, s)).collect::<Vec<_>>()
            })
            .collect(),
        None if word.starts_with(char::is_uppercase) => types(index, &word)
            .into_iter()
            .filter_map(|(path, entry, container)| {
                let s = entry.symbols.iter().find(|s| is_type(s.kind) && s.container.as_ref().map_or_else(|| s.name.clone(), |c| format!("{c}.{}", s.name)) == container)?;
                Some(symbol_target(path, entry.lang, s))
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Find Usages from a `buildSrc` declaration: the Groovy build scripts'
/// uses of `word` that resolve to one of `declarations` (path, line).
pub fn usages(index: &ProjectIndex, root: &std::path::Path, word: &str, declarations: &[(String, u32)]) -> Vec<super::nav::Usage> {
    let mut out = Vec::new();
    for path in index.all_files.iter().filter(|p| is_groovy_script(p)) {
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        for (at, _) in text.match_indices(word) {
            let whole = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !whole(text[..at].chars().next_back()) || !whole(text[at + word.len()..].chars().next()) {
                continue;
            }
            if !targets(index, &text, at).iter().any(|t| declarations.iter().any(|(p, l)| *p == t.path && *l == t.line)) {
                continue;
            }
            let (line, col) = super::nav::position(&text, at);
            let line_text = text.lines().nth(line as usize).unwrap_or_default().trim().to_owned();
            out.push(super::nav::Usage { path: path.clone(), line, col, text: line_text, group: "Groovy build scripts".into() });
        }
    }
    out
}

/// Completion after `Deps.` in a Groovy build script: the byte range the
/// typed name takes and the type's members, with their kinds and types.
pub fn completions(index: &ProjectIndex, text: &str, offset: usize) -> Option<(std::ops::Range<usize>, Vec<(String, SymbolKind, String)>)> {
    let before = &text[..offset];
    let start = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    let chain = qualifier(text, start)?;
    let mut items: Vec<(String, SymbolKind, String)> = Vec::new();
    for (_, entry, container) in types(index, chain) {
        for s in members(entry, &container) {
            if !items.iter().any(|(n, _, _)| *n == s.name) {
                items.push((s.name.clone(), s.kind, container.clone()));
            }
        }
    }
    Some((start..offset, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> ProjectIndex {
        let mut index = ProjectIndex::default();
        let text = "object Deps {\n    const val coreKtx = \"a:b:1\"\n    object Android {\n        const val minSdk = 24\n    }\n}\n";
        let entry = FileEntry { lang: Lang::Kotlin, mtime: 0, size: 0, symbols: super::super::symbols::extract(Lang::Kotlin, text), bridges: Vec::new() };
        index.insert_for_test("buildSrc/src/main/kotlin/Deps.kt", entry);
        index
    }

    #[test]
    fn resolves_qualified_constants() {
        let index = index();
        let script = "dependencies {\n    implementation Deps.coreKtx\n    minSdk Deps.Android.minSdk\n}";
        let at = script.find("coreKtx").unwrap() + 2;
        let t = targets(&index, script, at);
        assert_eq!(t.iter().map(|t| (t.path.as_str(), t.line)).collect::<Vec<_>>(), vec![("buildSrc/src/main/kotlin/Deps.kt", 1)]);
        let at = script.find("minSdk\n").unwrap() + 1;
        assert_eq!(targets(&index, script, at).iter().map(|t| t.line).collect::<Vec<_>>(), vec![3]);
        let at = script.find("Deps.").unwrap() + 1;
        assert_eq!(targets(&index, script, at).iter().map(|t| t.line).collect::<Vec<_>>(), vec![0]);
        // `libs.x` and plain words are not buildSrc names.
        assert!(targets(&index, "implementation libs.coreKtx", 22).is_empty());
    }

    #[test]
    fn finds_script_usages() {
        let mut index = index();
        let dir = std::env::temp_dir().join(format!("junction-groovy-usages-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app")).unwrap();
        std::fs::write(dir.join("app/build.gradle"), "dependencies {\n    implementation Deps.coreKtx\n    implementation libs.coreKtx\n}\n").unwrap();
        index.all_files = vec!["app/build.gradle".into(), "buildSrc/src/main/kotlin/Deps.kt".into()];
        let found = usages(&index, &dir, "coreKtx", &[("buildSrc/src/main/kotlin/Deps.kt".into(), 1)]);
        assert_eq!(found.iter().map(|u| (u.path.as_str(), u.line, u.col)).collect::<Vec<_>>(), vec![("app/build.gradle", 1, 24)]);
        assert!(usages(&index, &dir, "coreKtx", &[("other.kt".into(), 1)]).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn completes_members() {
        let index = index();
        let script = "implementation Deps.co";
        let (range, items) = completions(&index, script, script.len()).unwrap();
        assert_eq!(&script[range], "co");
        let mut names: Vec<_> = items.iter().map(|(n, _, _)| n.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["Android", "coreKtx"]);
        let script = "minSdk Deps.Android.";
        let (_, items) = completions(&index, script, script.len()).unwrap();
        assert_eq!(items.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>(), vec!["minSdk"]);
        assert!(completions(&index, "implementation libs.", 20).is_none());
    }
}

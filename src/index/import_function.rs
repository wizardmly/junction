//! Alt+Enter › Import function / Import extension function (Kotlin): the
//! top-level functions of the called name in the project and libraries
//! that the file doesn't see, as IntelliJ's ImportFix finds them. A plain
//! call wants a plain function (or, inside a class, an extension on it);
//! `x.name()` wants an extension whose receiver fits `x`'s type. Nothing
//! is offered when the file already sees one, or the name is a member of
//! the receiver (or of the classes around the call), a local or one of
//! the file's own declarations.

use std::collections::HashMap;
use std::path::PathBuf;

use super::lang::Lang;
use super::store::ProjectIndex;
use super::symbols::{self, SymbolKind};
use crate::domain::import_function as imf;
use crate::domain::imports::{self, ImportStyle};

/// The Alt+Enter item and the functions to choose from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionImport {
    /// `Import function 'greet'`, `Import extension function 'String.shout'`.
    pub title: String,
    /// Their qualified names, sorted.
    pub qualified: Vec<String>,
}

/// What to import for the call at `offset` of the Kotlin file at `path`.
pub fn candidates(index: &ProjectIndex, path: &str, text: &str, offset: usize) -> Option<FunctionImport> {
    let lang = Lang::Kotlin;
    let call = imf::call_at(text, offset)?;
    let name = call.name.as_str();
    if super::nav::comments_and_strings(text, lang).iter().any(|r| r.start < call.start && call.start < r.end) {
        return None;
    }
    // The file's own declarations, locals and parameters.
    let own = symbols::extract(lang, text);
    if own.iter().any(|s| s.name == name) || super::locals::visible(lang, text, call.start).iter().any(|(n, _)| n == name) {
        return None;
    }
    let header = imports::header(text);
    if header.imports.iter().any(|(n, _)| n == name) {
        return None;
    }
    let imported: Vec<String> = header.imports.iter().map(|(_, q)| q.clone()).chain(header.wildcards.iter().map(|w| format!("{w}.*"))).collect();
    let implicit = imports::implicit_packages(ImportStyle::Kotlin);
    let package = (!header.package.is_empty()).then_some(header.package.as_str());
    // The types the call is a member of: the receiver's, or `this`'s (the
    // classes around the call), with their supertypes.
    let types: Vec<String> = if call.member {
        super::nav::member_receivers(index, lang, text, call.start).unwrap_or_default()
    } else {
        let line = text[..call.start].matches('\n').count() as u32;
        let around: Vec<&symbols::Symbol> = own.iter().filter(|s| s.kind.is_type() && s.line <= line && line <= s.end_line).collect();
        let supers: Vec<String> = around.iter().flat_map(|s| s.supers.clone()).collect();
        around.iter().map(|s| s.name.clone()).chain(super::special::super_chain(index, &supers)).collect()
    };
    let owner = |s: &symbols::Symbol| s.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next()).map(str::to_owned);
    let is_member = |s: &symbols::Symbol| matches!(s.kind, SymbolKind::Function | SymbolKind::Method | SymbolKind::Property | SymbolKind::Field);
    // A member call: the member wins over any extension. When the
    // receiver's type is unknown, a member of that name anywhere may be it.
    if index.symbols_named(name).any(|(_, _, s)| is_member(s) && owner(s).is_some_and(|o| types.contains(&o) || call.member && types.is_empty())) {
        return None;
    }
    let mut sources: HashMap<&str, Option<String>> = HashMap::new();
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for (file, entry, s) in index.symbols_named(name) {
        if entry.lang != Lang::Kotlin || s.kind != SymbolKind::Function || s.container.is_some() || file == path {
            continue;
        }
        let source = sources.entry(file).or_insert_with(|| {
            let full = if ProjectIndex::is_external(file) { PathBuf::from(file) } else { index.root.join(file) };
            std::fs::read_to_string(full).ok()
        });
        let Some(source) = source.as_deref() else { continue };
        let decl = source.lines().skip(s.line as usize).take(4).collect::<Vec<_>>().join(" ");
        let receiver = imf::receiver(&decl, name);
        let fits = match &receiver {
            None => !call.member,
            // Inside a class, `shout()` may call an extension on it.
            Some(r) => (call.member || !types.is_empty()) && imf::receiver_fits(r, &imf::type_params(&decl), &types),
        };
        let pkg = imports::header(source).package;
        if !fits || pkg.is_empty() {
            continue;
        }
        let qualified = format!("{pkg}.{name}");
        // The file sees it already: the call resolves.
        if imports::is_visible(&qualified, package, &imported, &implicit) {
            return None;
        }
        if !found.iter().any(|(q, _)| *q == qualified) {
            found.push((qualified, receiver));
        }
    }
    let title = imf::title(name, found.first()?.1.as_deref());
    let mut qualified: Vec<String> = found.into_iter().map(|(q, _)| q).collect();
    qualified.sort();
    Some(FunctionImport { title, qualified })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> (std::path::PathBuf, ProjectIndex) {
        let dir = std::env::temp_dir().join(format!("junction-import-fn-{}", std::process::id()));
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

    #[test]
    fn finds_functions_to_import() {
        let (dir, index) = project(&[
            ("util/Text.kt", "package com.example.util\n\nfun greet(name: String) = \"hi $name\"\n\nfun String.shout(): String = uppercase()\n\nfun Cart.clear() {}\n"),
            ("other/Text.kt", "package com.example.other\n\nfun greet(n: Int) = n\n"),
            ("app/Cart.kt", "package com.example.app\n\nclass Cart {\n    fun total() = 1\n}\n"),
        ]);
        let main = "package com.example.app\n\nfun main() {\n    greet(\"a\")\n    val s: String = \"x\"\n    s.shout()\n    val cart = Cart()\n    cart.total()\n    cart.clear()\n}\n";
        let at = |s: &str| main.find(s).unwrap() + 1;
        let got = candidates(&index, "app/Main.kt", main, at("greet")).unwrap();
        assert_eq!(got.title, "Import function 'greet'");
        assert_eq!(got.qualified, ["com.example.other.greet", "com.example.util.greet"]);
        let got = candidates(&index, "app/Main.kt", main, at("shout")).unwrap();
        assert_eq!((got.title.as_str(), got.qualified.as_slice()), ("Import extension function 'String.shout'", &["com.example.util.shout".to_owned()][..]));
        assert_eq!(candidates(&index, "app/Main.kt", main, at("clear")).map(|f| f.qualified), Some(vec!["com.example.util.clear".to_owned()]));
        // A member, and a function the file imports.
        assert_eq!(candidates(&index, "app/Main.kt", main, at("total")), None);
        let imported = main.replace("package com.example.app\n", "package com.example.app\n\nimport com.example.util.*\n");
        assert_eq!(candidates(&index, "app/Main.kt", &imported, imported.find("greet").unwrap()), None);
        // An extension on another type.
        let int = main.replace("val s: String = \"x\"", "val s: Int = 1");
        assert_eq!(candidates(&index, "app/Main.kt", &int, int.find("shout").unwrap()), None);
        std::fs::remove_dir_all(dir).ok();
    }
}

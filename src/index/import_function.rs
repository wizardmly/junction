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

/// Alt+Enter › Import static method (Java), as IntelliJ's
/// StaticImportMethodFix offers it on an unqualified call the file can't
/// resolve: the static methods of that name in the project's and
/// libraries' classes it can reach (public, or package-private in its own
/// package), one per class, those whose parameters take the call's
/// arguments first (all when none do). The qualified names come as
/// `static a.B.name` for `import static`.
pub fn static_methods(index: &ProjectIndex, path: &str, text: &str, offset: usize) -> Option<FunctionImport> {
    use crate::domain::parameter_info as pi;
    let lang = Lang::Java;
    let call = imf::call_at(text, offset)?;
    let name = call.name.as_str();
    if call.member || super::nav::comments_and_strings(text, lang).iter().any(|r| r.start < call.start && call.start < r.end) {
        return None;
    }
    // A method of the file (or its classes' supertypes), a local.
    let own = symbols::extract(lang, text);
    if own.iter().any(|s| s.name == name) || super::locals::visible(lang, text, call.start).iter().any(|(n, _)| n == name) {
        return None;
    }
    let supers: Vec<String> = own.iter().filter(|s| s.kind.is_type()).flat_map(|s| s.supers.clone()).collect();
    let inherited = super::special::super_chain(index, &supers);
    let owner = |s: &symbols::Symbol| s.container.as_deref().and_then(|c| c.rsplit(['.', ':']).next()).map(str::to_owned);
    if index.symbols_named(name).any(|(_, _, s)| matches!(s.kind, SymbolKind::Method | SymbolKind::Function) && owner(s).is_some_and(|o| inherited.contains(&o))) {
        return None;
    }
    // `import static a.B.name;` or `import static a.B.*;` already.
    let statics: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix("import static ")).map(|r| r.trim_end_matches(';').trim()).collect();
    if statics.iter().any(|q| q.rsplit('.').next() == Some(name)) {
        return None;
    }
    let package = imports::header(text).package;
    let args = pi::typed_arguments(text, call.start + text[call.start..].find('(')?, false);
    let mut sources: HashMap<&str, Option<String>> = HashMap::new();
    // (qualified, takes the arguments).
    let mut found: Vec<(String, bool)> = Vec::new();
    for (file, entry, s) in index.symbols_named(name) {
        if entry.lang != Lang::Java || s.kind != SymbolKind::Method || file == path {
            continue;
        }
        let Some(class) = s.container.clone() else { continue };
        let source = sources.entry(file).or_insert_with(|| {
            let full = if ProjectIndex::is_external(file) { PathBuf::from(file) } else { index.root.join(file) };
            std::fs::read_to_string(full).ok()
        });
        let Some(source) = source.as_deref() else { continue };
        let Some(line) = source.lines().nth(s.line as usize) else { continue };
        let Some(head) = line.find(&format!("{name}(")).map(|at| &line[..at]) else { continue };
        let words: Vec<&str> = head.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
        let pkg = imports::header(source).package;
        let reachable = words.contains(&"public") || !words.contains(&"private") && !words.contains(&"protected") && pkg == package;
        if !words.contains(&"static") || !reachable || pkg.is_empty() {
            continue;
        }
        let qualified = format!("{pkg}.{class}");
        // The file wildcard-imports the class's statics: the call resolves.
        if statics.iter().any(|q| q.strip_suffix(".*") == Some(qualified.as_str())) {
            return None;
        }
        let at = source.lines().take(s.line as usize).map(|l| l.len() + 1).sum::<usize>() + head.len();
        let takes = pi::parameters(source, at).is_some_and(|params| pi::takes_arguments(&params, &args));
        match found.iter_mut().find(|(q, _)| *q == qualified) {
            Some(entry) => entry.1 |= takes,
            None => found.push((qualified, takes)),
        }
    }
    let applicable = found.iter().any(|(_, takes)| *takes);
    let mut qualified: Vec<String> = found.into_iter().filter(|(_, takes)| *takes || !applicable).map(|(q, _)| format!("static {q}.{name}")).collect();
    qualified.sort();
    // IntelliJ names the one method, else leaves the choice to the list.
    let title = match qualified.as_slice() {
        [] => return None,
        [one] => format!("Import static method '{}'", &one["static ".len()..]),
        _ => "Import static method...".to_owned(),
    };
    Some(FunctionImport { title, qualified })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> (std::path::PathBuf, ProjectIndex) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("junction-import-fn-{}-{n}", std::process::id()));
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

    #[test]
    fn finds_static_methods_to_import() {
        let (dir, index) = project(&[
            ("util/Text.java", "package com.example.util;\n\npublic final class Text {\n    public static String greet(String name) { return name; }\n    public static int twice(int n) { return n * 2; }\n    private static int hidden(int n) { return n; }\n    public String shout(String s) { return s; }\n}\n"),
            ("other/Nums.java", "package com.example.other;\n\npublic class Nums {\n    public static int twice(int a, int b) { return a; }\n    public static int greet(int n) { return n; }\n}\n"),
            ("app/Base.java", "package com.example.app;\n\npublic class Base {\n    protected static void inherited() {}\n}\n"),
        ]);
        let main = "package com.example.app;\n\npublic class Main extends Base {\n    void run() {\n        greet(\"a\");\n        twice(3);\n        hidden(1);\n        shout(\"x\");\n        inherited();\n        local();\n    }\n    void local() {}\n}\n";
        let at = |s: &str| main.find(s).unwrap() + 1;
        let path = "app/Main.java";
        // Only the overload the arguments fit; the other class's when none do.
        let got = static_methods(&index, path, main, at("greet")).unwrap();
        assert_eq!((got.title.as_str(), got.qualified.as_slice()), ("Import static method 'com.example.util.Text.greet'", &["static com.example.util.Text.greet".to_owned()][..]));
        let got = static_methods(&index, path, main, at("twice")).unwrap();
        assert_eq!(got.qualified, ["static com.example.util.Text.twice"]);
        let both = main.replace("twice(3)", "twice()");
        let got = static_methods(&index, path, &both, both.find("twice").unwrap()).unwrap();
        assert_eq!((got.title.as_str(), got.qualified.len()), ("Import static method...", 2));
        // Private, not static, inherited, the file's own.
        for name in ["hidden", "shout", "inherited", "local("] {
            assert_eq!(static_methods(&index, path, main, at(name)), None, "{name}");
        }
        // Imported already, one by one or with `.*`.
        for import in ["import static com.example.util.Text.greet;", "import static com.example.util.Text.*;"] {
            let imported = main.replace("package com.example.app;\n", &format!("package com.example.app;\n\n{import}\n"));
            assert_eq!(static_methods(&index, path, &imported, imported.find("greet").unwrap()), None, "{import}");
        }
        std::fs::remove_dir_all(dir).ok();
    }
}

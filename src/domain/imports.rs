//! JVM imports: whether a file already sees a qualified name, the edit
//! adding its import where IntelliJ puts it (with the other imports, kept
//! in order, or after the package line), and Optimize Imports.

use std::collections::HashSet;
use std::ops::Range;

/// The import syntax of the file's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStyle {
    Java,
    Kotlin,
}

/// The packages every file of the language sees without an import:
/// Kotlin's default imports, Java's `java.lang`.
pub fn implicit_packages(style: ImportStyle) -> Vec<String> {
    let packages: &[&str] = match style {
        ImportStyle::Java => &["java.lang"],
        ImportStyle::Kotlin => &["kotlin", "kotlin.annotation", "kotlin.collections", "kotlin.comparisons", "kotlin.io", "kotlin.ranges", "kotlin.sequences", "kotlin.text", "kotlin.jvm", "java.lang"],
    };
    packages.iter().map(|p| p.to_string()).collect()
}

/// Whether `qualified` (`a.b.C`) is visible in a file of `package` with
/// these imports and implicitly imported packages.
pub fn is_visible(qualified: &str, package: Option<&str>, imports: &[String], implicit: &[String]) -> bool {
    let Some((pkg, name)) = qualified.rsplit_once('.') else { return true };
    if package == Some(pkg) || implicit.iter().any(|p| p == pkg) {
        return true;
    }
    imports.iter().any(|i| {
        i == qualified
            || i.strip_suffix(".*") == Some(pkg)
            // Another class of the same simple name is imported: picking
            // this one must not add a clashing import either.
            || i.rsplit('.').next() == Some(name)
    })
}

/// The edit adding `import qualified` to `text`: (byte offset, text to
/// insert). Imports stay sorted; without any, the import goes after the
/// package line, separated by a blank line.
pub fn import_edit(style: ImportStyle, text: &str, qualified: &str) -> (usize, String) {
    let line = match style {
        ImportStyle::Java => format!("import {qualified};"),
        ImportStyle::Kotlin => format!("import {qualified}"),
    };
    // (start, end incl. newline, imported name) of each top-level import.
    let mut imports: Vec<(usize, usize, &str)> = Vec::new();
    let mut package_end: Option<usize> = None;
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let name = rest.trim_start_matches("static ").trim_end_matches(';').trim();
            imports.push((offset, offset + raw.len(), name));
        } else if trimmed.starts_with("package ") {
            package_end = Some(offset + raw.len());
        } else if !(trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('@')) {
            // The first declaration ends the header.
            break;
        }
        offset += raw.len();
    }
    if let Some(&(_, last_end, _)) = imports.last() {
        // Before the first import sorting after it, else after the last.
        let at = imports.iter().find(|(_, _, name)| *name > qualified).map_or(last_end, |(start, _, _)| *start);
        let newline = if at == text.len() && !text.ends_with('\n') { "\n" } else { "" };
        return (at, format!("{newline}{line}\n"));
    }
    match package_end {
        Some(end) => {
            let newline = if text[..end].ends_with('\n') { "" } else { "\n" };
            (end, format!("{newline}\n{line}\n"))
        }
        None => (0, format!("{line}\n\n")),
    }
}

/// Optimize Imports (Ctrl+Alt+O): the import block rewritten, and how many
/// imports it dropped.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportsEdit {
    /// Byte range of the old block.
    pub range: Range<usize>,
    pub text: String,
    pub removed: usize,
}

/// Kotlin operators and delegates are called without their names
/// (`by remember { }` needs `getValue`): their imports always stay.
const KOTLIN_IMPLICIT: &[&str] = &[
    "getValue", "setValue", "provideDelegate", "plus", "minus", "times", "div", "rem", "mod", "rangeTo", "rangeUntil", "unaryPlus", "unaryMinus", "not", "inc", "dec",
    "contains", "get", "set", "invoke", "iterator", "next", "hasNext", "compareTo", "equals", "plusAssign", "minusAssign", "timesAssign", "divAssign", "remAssign",
];

struct Import<'a> {
    is_static: bool,
    /// `a.b.C`, `a.b.*`.
    path: &'a str,
    alias: Option<&'a str>,
    /// A comment after it on its line (`// for tests`), kept with it.
    comment: Option<&'a str>,
}

impl Import<'_> {
    /// The line as written back: the import and its comment.
    fn written(&self, style: ImportStyle) -> String {
        match self.comment {
            Some(c) => format!("{} {c}", self.line(style)),
            None => self.line(style),
        }
    }

    fn line(&self, style: ImportStyle) -> String {
        let alias = self.alias.map(|a| format!(" as {a}")).unwrap_or_default();
        match style {
            ImportStyle::Java => format!("import {}{};", if self.is_static { "static " } else { "" }, self.path),
            ImportStyle::Kotlin => format!("import {}{alias}", self.path),
        }
    }

    /// Where IntelliJ's default import layout puts it: Kotlin's `*`,
    /// `java.*`, `javax.*`, `kotlin.*`, aliases; Java's others, then
    /// `javax.*` and `java.*` after a blank line, then static imports.
    fn group(&self, style: ImportStyle) -> u8 {
        let under = |p: &str| self.path.starts_with(p);
        match style {
            ImportStyle::Kotlin if self.alias.is_some() => 4,
            ImportStyle::Kotlin if under("java.") => 1,
            ImportStyle::Kotlin if under("javax.") => 2,
            ImportStyle::Kotlin if under("kotlin.") => 3,
            ImportStyle::Kotlin => 0,
            ImportStyle::Java if self.is_static => 3,
            ImportStyle::Java if under("javax.") => 1,
            ImportStyle::Java if under("java.") => 2,
            ImportStyle::Java => 0,
        }
    }
}

fn parse_import(line: &str) -> Option<Import<'_>> {
    let rest = line.trim().strip_prefix("import ")?.trim();
    let (rest, comment) = match rest.find("//").into_iter().chain(rest.find("/*")).min() {
        Some(at) => (rest[..at].trim(), Some(rest[at..].trim())),
        None => (rest, None),
    };
    let rest = rest.strip_suffix(';').unwrap_or(rest).trim();
    let (is_static, rest) = match rest.strip_prefix("static ") {
        Some(r) => (true, r.trim()),
        None => (false, rest),
    };
    let (path, alias) = match rest.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim())),
        None => (rest, None),
    };
    (!path.is_empty() && !path.contains(char::is_whitespace)).then_some(Import { is_static, path, alias, comment })
}

/// Optimize Imports, as IntelliJ's with its default code style: imports
/// nothing in the file names are removed (star imports and Kotlin operator
/// imports stay), so are duplicates, ones of the file's own package and
/// Java's `java.lang`; the rest sorted into the default layout. `None`
/// when the block is already optimized. As IntelliJ's, a comment after an
/// import stays on its line; comment lines among the imports move after
/// them.
pub fn optimize(style: ImportStyle, text: &str) -> Option<ImportsEdit> {
    let mut package = None;
    let mut block: Option<Range<usize>> = None;
    let mut imports = Vec::new();
    let mut comments: Vec<&str> = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let trimmed = raw.trim();
        if trimmed.starts_with("import ") {
            imports.push(parse_import(trimmed)?);
            let start = block.as_ref().map_or(offset, |b| b.start);
            block = Some(start..offset + raw.len());
        } else if let Some(p) = trimmed.strip_prefix("package ") {
            package = Some(p.trim_end_matches(';').trim());
        } else if trimmed.is_empty() {
        } else if block.is_none() {
            // Before the imports: comments and file annotations; the first
            // declaration ends the header.
            if !(trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('@')) {
                return None;
            }
        } else {
            // A comment among the imports (not after the last one).
            let comment = trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*');
            if comment && text[offset..].lines().any(|l| l.trim().starts_with("import ")) {
                comments.push(raw.trim_end());
                offset += raw.len();
                continue;
            }
            break;
        }
        offset += raw.len();
    }
    let block = block?;
    let words: HashSet<&str> = text[..block.start]
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .chain(text[block.end..].split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
        .filter(|w| !w.is_empty())
        .collect();
    let mut seen = HashSet::new();
    let mut kept: Vec<&Import> = Vec::new();
    for import in &imports {
        let (owner, name) = import.path.rsplit_once('.').unwrap_or(("", import.path));
        let used = name == "*"
            || words.contains(import.alias.unwrap_or(name))
            || style == ImportStyle::Kotlin && KOTLIN_IMPLICIT.contains(&name);
        let redundant = !import.is_static
            && import.alias.is_none()
            && name != "*"
            && (Some(owner) == package || style == ImportStyle::Java && owner == "java.lang");
        if used && !redundant && seen.insert(import.line(style)) {
            kept.push(import);
        }
    }
    // Within a group by their text, backticks ignored (IntelliJ's KotlinImportPathComparator).
    kept.sort_by_cached_key(|i| (i.group(style), format!("{}{}", i.path, i.alias.map(|a| format!(" as {a}")).unwrap_or_default()).replace('`', "")));
    let mut out = String::new();
    let mut last_group = None;
    for import in &kept {
        let group = import.group(style);
        // Java's layout separates its three blocks with a blank line.
        let block_of = |g: u8| if g == 0 { 0 } else if g < 3 { 1 } else { 2 };
        if style == ImportStyle::Java && last_group.is_some_and(|g| block_of(g) != block_of(group)) {
            out.push('\n');
        }
        last_group = Some(group);
        out.push_str(&import.written(style));
        out.push('\n');
    }
    for comment in &comments {
        out.push_str(comment);
        out.push('\n');
    }
    let mut range = block;
    if kept.is_empty() && comments.is_empty() {
        // No imports left: their blank lines go too.
        while text[range.end..].starts_with('\n') {
            range.end += 1;
        }
    } else if !text[..range.end].ends_with('\n') {
        out.pop();
    }
    if text[range.clone()] == out {
        return None;
    }
    Some(ImportsEdit { range, text: out, removed: imports.len() - kept.len() })
}

/// Inspect Code's unused imports: the imports Optimize Imports drops
/// (unused, duplicated, of the file's own package or `java.lang`), each
/// with its 0-based line and path.
pub fn unused_imports(style: ImportStyle, text: &str) -> Vec<(u32, String)> {
    let mut package = None;
    let mut imports = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.starts_with("import ") {
            if let Some(import) = parse_import(trimmed) {
                imports.push((line as u32, import));
            }
        } else if let Some(p) = trimmed.strip_prefix("package ") {
            package = Some(p.trim_end_matches(';').trim());
        } else if !(trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('@')) {
            break;
        }
    }
    if imports.is_empty() {
        return Vec::new();
    }
    let words: HashSet<&str> = text
        .lines()
        .filter(|l| !(l.trim_start().starts_with("import ") || l.trim_start().starts_with("package ")))
        .flat_map(|l| l.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
        .filter(|w| !w.is_empty())
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (line, import) in &imports {
        let (owner, name) = import.path.rsplit_once('.').unwrap_or(("", import.path));
        let used = name == "*" || words.contains(import.alias.unwrap_or(name)) || style == ImportStyle::Kotlin && KOTLIN_IMPLICIT.contains(&name);
        let redundant = !import.is_static
            && import.alias.is_none()
            && name != "*"
            && (Some(owner) == package || style == ImportStyle::Java && owner == "java.lang");
        if !used || redundant || !seen.insert(import.line(style)) {
            out.push((*line, import.path.to_owned()));
        }
    }
    out
}

/// A JVM file's package and imports, read from its header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub package: String,
    /// (the name the file uses, the qualified name): `import a.B` gives
    /// ("B", "a.B"), Kotlin's `import a.B as C` ("C", "a.B").
    pub imports: Vec<(String, String)>,
    /// Packages (or classes) imported with `.*`.
    pub wildcards: Vec<String>,
}

/// The package and imports at the top of a Java or Kotlin file.
pub fn header(text: &str) -> Header {
    let mut out = Header::default();
    let clean = |s: &str| s.trim().trim_end_matches(';').trim().replace('`', "");
    for raw in text.lines() {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("package ") {
            out.package = clean(rest);
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            let rest = clean(rest);
            if rest.starts_with("static ") {
                continue;
            }
            if let Some(pkg) = rest.strip_suffix(".*") {
                out.wildcards.push(pkg.to_owned());
                continue;
            }
            let (qualified, name) = match rest.split_once(" as ") {
                Some((q, alias)) => (q.trim().to_owned(), alias.trim().to_owned()),
                None => (rest.clone(), rest.rsplit('.').next().unwrap_or(&rest).to_owned()),
            };
            out.imports.push((name, qualified));
        } else if !(trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('@')) {
            // The first declaration ends the header.
            break;
        }
    }
    out
}

/// Which of the classes called `name` a file with this header means,
/// given their qualified names, as the compiler picks: the one imported
/// by name, else the file's own package's, else one under a `.*` import.
/// Their positions in `qualified`; `None` when the header doesn't say
/// (an implicitly imported or nested class).
pub fn resolve(header: &Header, name: &str, qualified: &[String]) -> Option<Vec<usize>> {
    let matching = |want: &dyn Fn(&str) -> bool| (0..qualified.len()).filter(|i| want(&qualified[*i])).collect::<Vec<_>>();
    if let Some((_, imported)) = header.imports.iter().find(|(n, _)| n == name) {
        return Some(matching(&|q| q == imported));
    }
    let local = format!("{}.{name}", header.package);
    let found = matching(&|q| q == local || header.package.is_empty() && q == name);
    if !found.is_empty() {
        return Some(found);
    }
    let outer = |q: &str| q.strip_suffix(name).and_then(|p| p.strip_suffix('.')).map(str::to_owned);
    let found = matching(&|q| outer(q).is_some_and(|o| header.wildcards.contains(&o)));
    if !found.is_empty() {
        return Some(found);
    }
    // `Outer.Name`, with the index keeping only the last name: a class
    // nested in an imported class, or in one of the file's package.
    let found = matching(&|q| outer(q).is_some_and(|o| header.imports.iter().any(|(_, i)| *i == o)));
    if !found.is_empty() {
        return Some(found);
    }
    let found = matching(&|q| outer(q).is_some_and(|o| o.rsplit_once('.').map_or(header.package.is_empty(), |(p, _)| p == header.package)));
    (!found.is_empty()).then_some(found)
}

/// The qualifiers written before `name` in the declaration starting on
/// `line` (0-based): ["Job"] for `class Factory : Job.Factory<T>`.
pub fn qualifiers(text: &str, line: u32, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(start) = (if line == 0 { Some(0) } else { text.match_indices('\n').nth(line as usize - 1).map(|(i, _)| i + 1) }) else { return out };
    // The declaration's head: up to its body, over the lines that go on
    // with it (at most a few).
    let mut end = start;
    for (i, raw) in text[start..].split_inclusive('\n').take(8).enumerate() {
        let line = raw.trim();
        let goes_on = |l: &str| l.starts_with([':', ',', '.', '<']) || l.starts_with("extends") || l.starts_with("implements") || l.starts_with("where");
        if i > 0 && !(text[start..end].trim_end().ends_with([',', ':', '(', '<']) || goes_on(line)) {
            break;
        }
        end += raw.len();
        if line.contains('{') {
            break;
        }
    }
    let head = &text[start..end];
    let head = &head[..head.find('{').unwrap_or(head.len())];
    let word = |c: char| c.is_alphanumeric() || c == '_';
    for (at, _) in head.match_indices(name) {
        if !head[..at].ends_with('.') || head[at + name.len()..].starts_with(word) {
            continue;
        }
        let before = &head[..at - 1];
        let from = before.rfind(|c: char| !(word(c) || c == '.')).map_or(0, |i| i + 1);
        let qualifier = &before[from..];
        if !qualifier.is_empty() && !qualifier.starts_with('.') && !out.iter().any(|q| q == qualifier) {
            out.push(qualifier.to_owned());
        }
    }
    out
}

/// Which of the classes called `name` a file with this header means by
/// `qualifier.name`, given their qualified names.
pub fn resolve_qualified(header: &Header, qualifier: &str, name: &str, qualified: &[String]) -> Vec<usize> {
    let (first, rest) = qualifier.split_once('.').map_or((qualifier, ""), |(f, r)| (f, r));
    let tail = |base: &str| [base, rest, name].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(".");
    let mut wanted: Vec<String> = Vec::new();
    if let Some((_, imported)) = header.imports.iter().find(|(n, _)| n == first) {
        wanted.push(tail(imported));
    } else {
        // The file's package, `.*` imports, or a qualified name as is.
        let local = if header.package.is_empty() { first.to_owned() } else { format!("{}.{first}", header.package) };
        wanted.push(tail(&local));
        wanted.extend(header.wildcards.iter().map(|w| tail(&format!("{w}.{first}"))));
        wanted.push(format!("{qualifier}.{name}"));
    }
    wanted.iter().map(|w| (0..qualified.len()).filter(|i| qualified[*i] == *w).collect::<Vec<_>>()).find(|v| !v.is_empty()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility() {
        let imports = vec!["android.view.View".to_owned(), "kotlinx.coroutines.*".to_owned()];
        let implicit = vec!["kotlin".to_owned()];
        assert!(is_visible("android.view.View", None, &imports, &implicit));
        assert!(is_visible("kotlinx.coroutines.Job", None, &imports, &implicit));
        assert!(is_visible("com.app.Foo", Some("com.app"), &imports, &implicit));
        assert!(is_visible("kotlin.Pair", None, &imports, &implicit));
        assert!(!is_visible("android.widget.TextView", None, &imports, &implicit));
        // A same-named class is already imported.
        assert!(is_visible("other.View", None, &imports, &implicit));
    }

    fn optimized(style: ImportStyle, text: &str) -> String {
        let edit = optimize(style, text).expect("an edit");
        let mut out = text.to_owned();
        out.replace_range(edit.range, &edit.text);
        out
    }

    #[test]
    fn finds_unused_imports() {
        let kt = "package a.b\n\nimport x.Used\nimport x.Unused\nimport a.b.Same\nimport x.Used\nimport y.*\n\nclass C : Used, Same\n";
        assert_eq!(unused_imports(ImportStyle::Kotlin, kt), vec![(3, "x.Unused".to_owned()), (4, "a.b.Same".to_owned()), (5, "x.Used".to_owned())]);
        let java = "package p;\nimport java.lang.String;\nimport java.util.List;\nclass C { List<String> l; }\n";
        assert_eq!(unused_imports(ImportStyle::Java, java), vec![(1, "java.lang.String".to_owned())]);
    }

    #[test]
    fn optimizes_kotlin_imports() {
        let text = "package a.b\n\nimport kotlin.math.max\nimport java.io.File\nimport androidx.compose.runtime.getValue\nimport a.b.Same\nimport com.z.Unused\nimport com.y.*\nimport com.x.Used as Alias\nimport android.view.View\nimport android.view.View\n\nclass A(val v: View) {\n    val f = File(\"x\")\n    val m = max(1, 2)\n    val q = Alias()\n}\n";
        assert_eq!(
            optimized(ImportStyle::Kotlin, text),
            "package a.b\n\nimport android.view.View\nimport androidx.compose.runtime.getValue\nimport com.y.*\nimport java.io.File\nimport kotlin.math.max\nimport com.x.Used as Alias\n\nclass A(val v: View) {\n    val f = File(\"x\")\n    val m = max(1, 2)\n    val q = Alias()\n}\n"
        );
        assert_eq!(optimize(ImportStyle::Kotlin, "package a\n\nimport b.C\n\nval c = C()\n"), None);
    }

    #[test]
    fn optimizes_java_imports() {
        let text = "package a;\n\nimport static org.junit.Assert.assertEquals;\nimport java.util.List;\nimport java.lang.String;\nimport org.junit.Test;\nimport javax.inject.Inject;\nimport java.util.Map;\n\nclass A {\n  @Inject @Test void t() { List<String> l; assertEquals(1, 1); }\n}\n";
        assert_eq!(
            optimized(ImportStyle::Java, text),
            "package a;\n\nimport org.junit.Test;\n\nimport javax.inject.Inject;\nimport java.util.List;\n\nimport static org.junit.Assert.assertEquals;\n\nclass A {\n  @Inject @Test void t() { List<String> l; assertEquals(1, 1); }\n}\n"
        );
    }

    #[test]
    fn removes_every_import() {
        let edit = optimize(ImportStyle::Kotlin, "package a\n\nimport b.C\n\nclass A\n").unwrap();
        assert_eq!(edit.removed, 1);
        let mut out = "package a\n\nimport b.C\n\nclass A\n".to_owned();
        out.replace_range(edit.range, &edit.text);
        assert_eq!(out, "package a\n\nclass A\n");
        // A comment line among the imports moves after them (IntelliJ's
        // ImportHelper); one after an import stays with it.
        let text = "import d.E\n// keep\nimport b.C // for C\nimport x.Unused // gone\n\nval x: C = E()\n";
        let edit = optimize(ImportStyle::Kotlin, text).unwrap();
        let mut out = text.to_owned();
        out.replace_range(edit.range, &edit.text);
        assert_eq!(out, "import b.C // for C\nimport d.E\n// keep\n\nval x: C = E()\n");
        assert_eq!(optimize(ImportStyle::Kotlin, &out), None);
        let java = "import java.util.List; /* lists */\n/* block\n * comment */\nimport a.B;\n\nclass A { B b; List l; }\n";
        let edit = optimize(ImportStyle::Java, java).unwrap();
        assert_eq!(edit.text, "import a.B;\n\nimport java.util.List; /* lists */\n/* block\n * comment */\n");
        // No imports: nothing in the body is taken for one.
        assert_eq!(optimize(ImportStyle::Kotlin, "package a\n\nval s = \"\"\"\nimport b.C\n\"\"\"\n"), None);
    }

    #[test]
    fn sorted_among_imports() {
        let text = "package a\n\nimport android.view.View\nimport java.io.File\n\nclass A\n";
        let (at, add) = import_edit(ImportStyle::Kotlin, text, "android.widget.TextView");
        let mut out = text.to_owned();
        out.insert_str(at, &add);
        assert_eq!(out, "package a\n\nimport android.view.View\nimport android.widget.TextView\nimport java.io.File\n\nclass A\n");
    }

    #[test]
    fn after_package_without_imports() {
        let text = "package a.b;\n\npublic class A {}\n";
        let (at, add) = import_edit(ImportStyle::Java, text, "java.util.List");
        let mut out = text.to_owned();
        out.insert_str(at, &add);
        assert_eq!(out, "package a.b;\n\nimport java.util.List;\n\npublic class A {}\n");
    }

    #[test]
    fn header_and_resolve() {
        let h = header("// c\npackage a.b;\n\nimport x.Base;\nimport y.*;\nimport static z.Q.f;\nimport k.Long as `L`\n@Ann\nclass A extends Base {}\nimport no.Thing;\n");
        assert_eq!(h.package, "a.b");
        assert_eq!(h.imports, [("Base".to_owned(), "x.Base".to_owned()), ("L".to_owned(), "k.Long".to_owned())]);
        assert_eq!(h.wildcards, ["y"]);
        let q = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve(&h, "Base", &q(&["a.b.Base", "x.Base"])), Some(vec![1]));
        // Imported, but not one of these.
        assert_eq!(resolve(&h, "Base", &q(&["a.b.Base"])), Some(vec![]));
        assert_eq!(resolve(&h, "L", &q(&["k.Long", "k.L"])), Some(vec![0]));
        assert_eq!(resolve(&h, "C", &q(&["y.C", "a.b.C", "z.C"])), Some(vec![1]));
        assert_eq!(resolve(&h, "D", &q(&["y.D", "z.D", "y.E.D"])), Some(vec![0]));
        assert_eq!(resolve(&h, "E", &q(&["z.E", "w.E"])), None);
        // `Base.Factory`, `Local.Factory`.
        assert_eq!(resolve(&h, "Factory", &q(&["x.Base.Factory", "w.Other.Factory"])), Some(vec![0]));
        assert_eq!(resolve(&h, "Factory", &q(&["a.b.Local.Factory", "w.Other.Factory"])), Some(vec![0]));
        assert_eq!(resolve(&h, "Factory", &q(&["a.b.Local.Factory", "x.Base.Factory"])), Some(vec![1]));

        let text = "class A {\n  class Factory : Base.Factory<T>, Other\n  public static class F implements x.y.Factory, B.Factory {\n";
        assert_eq!(qualifiers(text, 1, "Factory"), ["Base"]);
        assert!(qualifiers(text, 1, "Other").is_empty());
        assert_eq!(qualifiers(text, 2, "Factory"), ["x.y", "B"]);
        assert_eq!(qualifiers("class X :\n  A.Callback,\n  B.Callback {\n  C.Callback\n", 0, "Callback"), ["A", "B"]);
        assert_eq!(qualifiers("class X\n    extends P.Callback\n\nclass Y : Q.Callback\n", 0, "Callback"), ["P"]);
        let fs = q(&["x.Base.Factory", "a.b.Base.Factory", "x.y.Factory", "y.Base.Factory"]);
        assert_eq!(resolve_qualified(&h, "Base", "Factory", &fs), [0]);
        assert_eq!(resolve_qualified(&h, "x.y", "Factory", &fs), [2]);
        assert_eq!(resolve_qualified(&header("package z\nimport y.*\n"), "Base", "Factory", &fs), [3]);
    }
}

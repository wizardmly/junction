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
}

impl Import<'_> {
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
    let rest = rest.strip_suffix(';').unwrap_or(rest).trim();
    let (is_static, rest) = match rest.strip_prefix("static ") {
        Some(r) => (true, r.trim()),
        None => (false, rest),
    };
    let (path, alias) = match rest.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim())),
        None => (rest, None),
    };
    (!path.is_empty() && !path.contains(char::is_whitespace)).then_some(Import { is_static, path, alias })
}

/// Optimize Imports, as IntelliJ's with its default code style: imports
/// nothing in the file names are removed (star imports and Kotlin operator
/// imports stay), so are duplicates, ones of the file's own package and
/// Java's `java.lang`; the rest sorted into the default layout. `None`
/// when the block is already optimized or can't be rewritten safely (a
/// comment inside it).
pub fn optimize(style: ImportStyle, text: &str) -> Option<ImportsEdit> {
    let mut package = None;
    let mut block: Option<Range<usize>> = None;
    let mut imports = Vec::new();
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
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                // A comment among the imports: leave the block alone,
                // unless the imports are over.
                if text[offset..].lines().any(|l| l.trim().starts_with("import ")) {
                    return None;
                }
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
        out.push_str(&import.line(style));
        out.push('\n');
    }
    let mut range = block;
    if kept.is_empty() {
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
        // A comment among the imports: left alone.
        assert_eq!(optimize(ImportStyle::Kotlin, "import b.C\n// keep\nimport d.E\n\nval x = 1\n"), None);
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
}

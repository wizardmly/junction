//! JVM imports: whether a file already sees a qualified name, and the edit
//! adding its import where IntelliJ puts it (with the other imports, kept
//! in order, or after the package line).

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

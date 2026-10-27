//! Reformat Code and Optimize Imports on the Project view's selection, as
//! IntelliJ runs them on files and directories: every Java and Kotlin file
//! (and for Reformat Code, XML and Gradle Groovy file) under the selected items, each given
//! its new text. Optimize Imports also takes the files a language server
//! organizes (Go, Rust, TypeScript…).

use crate::domain::imports::{self, ImportStyle};
use crate::domain::project_files::ProjectFiles;
use crate::domain::reformat;
use crate::domain::xml_reformat::{self, XmlKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanUp {
    Reformat,
    OptimizeImports,
    /// Analyze › Code Cleanup…: the fixes of the clean-up inspections
    /// Junction has (unused imports removed).
    CodeCleanup,
}

impl CleanUp {
    pub fn title(self) -> &'static str {
        match self {
            CleanUp::Reformat => "Reformat Code",
            CleanUp::OptimizeImports => "Optimize Imports",
            CleanUp::CodeCleanup => "Code Cleanup",
        }
    }
}

fn style_of(path: &str) -> Option<ImportStyle> {
    if path.ends_with(".java") {
        Some(ImportStyle::Java)
    } else if path.ends_with(".kt") {
        Some(ImportStyle::Kotlin)
    } else {
        None
    }
}

/// The Java and Kotlin files of the selection (project-relative paths and
/// whether each is a folder; the empty path is the project), in order; for
/// Optimize Imports, also those `by_server` takes.
pub fn targets(files: &dyn ProjectFiles, items: &[(String, bool)], clean_up: CleanUp, by_server: &dyn Fn(&str) -> bool) -> Vec<String> {
    let applies = |p: &str| style_of(p).is_some() || (clean_up == CleanUp::Reformat && (p.ends_with(".xml") || p.ends_with(".gradle"))) || (clean_up == CleanUp::OptimizeImports && by_server(p));
    let mut out: Vec<String> = Vec::new();
    let listed = items.iter().any(|(_, dir)| *dir).then(|| files.list()).unwrap_or_default();
    for (path, dir) in items {
        if *dir {
            let prefix = if path.is_empty() { String::new() } else { format!("{}/", path.trim_end_matches('/')) };
            out.extend(listed.iter().filter(|p| p.starts_with(&prefix) && applies(p)).cloned());
        } else if applies(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether Junction cleans the file itself (else its language server does).
pub fn built_in(path: &str) -> bool {
    style_of(path).is_some() || path.ends_with(".xml") || path.ends_with(".gradle")
}

/// A file's text after the clean-up; `None` when nothing changes.
/// `indent`: the indent unit `.editorconfig` gives the file.
pub fn cleaned(path: &str, text: &str, clean_up: CleanUp, indent: Option<&str>) -> Option<String> {
    if path.ends_with(".xml") {
        let new = (clean_up == CleanUp::Reformat).then(|| xml_reformat::reformat_xml_indented(XmlKind::of(path), text, indent))?;
        return (new != text).then_some(new);
    }
    if path.ends_with(".gradle") {
        let new = (clean_up == CleanUp::Reformat).then(|| reformat::reformat_groovy(text, None, indent))?;
        return (new != text).then_some(new);
    }
    let style = style_of(path)?;
    let new = match clean_up {
        CleanUp::Reformat => reformat::reformat_indented(style, text, None, indent),
        CleanUp::OptimizeImports | CleanUp::CodeCleanup => {
            let edit = imports::optimize(style, text)?;
            format!("{}{}{}", &text[..edit.range.start], edit.text, &text[edit.range.end..])
        }
    };
    (new != text).then_some(new)
}

/// IntelliJ's notification: how many files changed.
pub fn summary(clean_up: CleanUp, changed: usize) -> String {
    match (clean_up, changed) {
        (CleanUp::Reformat, 0) => "No files changed: content is already properly formatted".into(),
        (CleanUp::OptimizeImports, 0) => "Unused imports not found".into(),
        (CleanUp::CodeCleanup, 0) => "No code to clean up found".into(),
        (_, 1) => "1 file changed".into(),
        (_, n) => format!("{n} files changed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Files(Vec<&'static str>);
    impl ProjectFiles for Files {
        fn read(&self, _: &str) -> Result<String, String> {
            Err("no".into())
        }
        fn write(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn rename(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn list(&self) -> Vec<String> {
            self.0.iter().map(|s| s.to_string()).collect()
        }
    }

    #[test]
    fn picks_jvm_files_and_cleans_them() {
        let files = Files(vec!["app/src/A.java", "app/src/B.kt", "app/build.gradle", "app/src/main/res/layout/a.xml", "lib/C.java", "appx/D.java"]);
        assert_eq!(targets(&files, &[("app".into(), true), ("lib/C.java".into(), false)], CleanUp::OptimizeImports, &|_| false), ["app/src/A.java", "app/src/B.kt", "lib/C.java"]);
        assert_eq!(targets(&files, &[("".into(), true)], CleanUp::OptimizeImports, &|_| false).len(), 4);
        assert_eq!(targets(&files, &[("".into(), true)], CleanUp::Reformat, &|_| false).len(), 6);
        assert_eq!(cleaned("app/src/main/res/layout/a.xml", "<a><b/></a>", CleanUp::Reformat, None).as_deref(), Some("<a>\n\n    <b />\n</a>"));
        assert_eq!(cleaned("app/src/main/res/layout/a.xml", "<a><b/></a>", CleanUp::OptimizeImports, None), None);
        let text = "package a;\n\nimport java.util.List;\nimport java.util.Map;\n\nclass A { Map m; }\n";
        assert_eq!(cleaned("A.java", text, CleanUp::OptimizeImports, None).unwrap(), "package a;\n\nimport java.util.Map;\n\nclass A { Map m; }\n");
        assert_eq!(cleaned("build.gradle", "android {\nminSdk 24\n}", CleanUp::Reformat, None).as_deref(), Some("android {\n    minSdk 24\n}"));
        assert!(cleaned("build.gradle", "android {\n    minSdk 24\n}", CleanUp::OptimizeImports, None).is_none());
        assert_eq!(cleaned("A.java", "class A {\nint x;\n}\n", CleanUp::Reformat, None).unwrap(), "class A {\n    int x;\n}\n");
        assert_eq!(summary(CleanUp::Reformat, 2), "2 files changed");
    }

    #[test]
    fn optimize_imports_takes_server_files() {
        let files = Files(vec!["a/A.java", "a/main.go", "a/lib.rs", "a/notes.txt"]);
        let go = |p: &str| p.ends_with(".go");
        assert_eq!(targets(&files, &[("a".into(), true)], CleanUp::OptimizeImports, &go), ["a/A.java", "a/main.go"]);
        assert_eq!(targets(&files, &[("a".into(), true)], CleanUp::Reformat, &go), ["a/A.java"]);
        assert!(built_in("a/A.java") && !built_in("a/main.go"));
    }
}

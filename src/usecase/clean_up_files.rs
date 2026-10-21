//! Reformat Code and Optimize Imports on the Project view's selection, as
//! IntelliJ runs them on files and directories: every Java and Kotlin file
//! under the selected items, each given its new text.

use crate::domain::imports::{self, ImportStyle};
use crate::domain::project_files::ProjectFiles;
use crate::domain::reformat;

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
/// whether each is a folder; the empty path is the project), in order.
pub fn targets(files: &dyn ProjectFiles, items: &[(String, bool)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let listed = items.iter().any(|(_, dir)| *dir).then(|| files.list()).unwrap_or_default();
    for (path, dir) in items {
        if *dir {
            let prefix = if path.is_empty() { String::new() } else { format!("{}/", path.trim_end_matches('/')) };
            out.extend(listed.iter().filter(|p| p.starts_with(&prefix) && style_of(p).is_some()).cloned());
        } else if style_of(path).is_some() {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A file's text after the clean-up; `None` when nothing changes.
pub fn cleaned(path: &str, text: &str, clean_up: CleanUp) -> Option<String> {
    let style = style_of(path)?;
    let new = match clean_up {
        CleanUp::Reformat => reformat::reformat(style, text, None),
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
        let files = Files(vec!["app/src/A.java", "app/src/B.kt", "app/build.gradle", "lib/C.java", "appx/D.java"]);
        assert_eq!(targets(&files, &[("app".into(), true), ("lib/C.java".into(), false)]), ["app/src/A.java", "app/src/B.kt", "lib/C.java"]);
        assert_eq!(targets(&files, &[("".into(), true)]).len(), 4);
        let text = "package a;\n\nimport java.util.List;\nimport java.util.Map;\n\nclass A { Map m; }\n";
        assert_eq!(cleaned("A.java", text, CleanUp::OptimizeImports).unwrap(), "package a;\n\nimport java.util.Map;\n\nclass A { Map m; }\n");
        assert!(cleaned("build.gradle", text, CleanUp::Reformat).is_none());
        assert_eq!(cleaned("A.java", "class A {\nint x;\n}\n", CleanUp::Reformat).unwrap(), "class A {\n    int x;\n}\n");
        assert_eq!(summary(CleanUp::Reformat, 2), "2 files changed");
    }
}

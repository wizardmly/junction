//! The calls of a function in the project's other files, for refactorings
//! that change them with the declaration (Introduce Parameter, Change
//! Signature): found through the files port, then each file's calls
//! rewritten in one write.

use crate::domain::introduce_parameter::{apply, CallSite, Edit};
use crate::domain::project_files::ProjectFiles;

/// A file calling the function: its text when read and its calls.
#[derive(Clone, Debug)]
pub struct CallingFile {
    pub path: String,
    pub text: String,
    pub calls: Vec<CallSite>,
}

/// The project's other Kotlin and Java files calling `function`; `sites`
/// finds the calls in a file's text.
pub fn find(files: &dyn ProjectFiles, except: &str, function: &str, sites: &dyn Fn(&str, &str) -> Vec<CallSite>) -> Vec<CallingFile> {
    let needle = format!("{function}(");
    files
        .list()
        .into_iter()
        .filter(|f| f != except && (f.ends_with(".kt") || f.ends_with(".java")))
        .filter_map(|path| {
            let text = files.read(&path).ok().filter(|t| t.contains(&needle))?;
            let calls = sites(&path, &text);
            (!calls.is_empty()).then_some(CallingFile { path, text, calls })
        })
        .collect()
}

/// Rewrites each file's calls with `edits`; the files written.
pub fn rewrite(files: &dyn ProjectFiles, calling: &[CallingFile], edits: &dyn Fn(&CallingFile) -> Vec<Edit>) -> Vec<String> {
    calling
        .iter()
        .filter_map(|file| {
            let changes = edits(file);
            if changes.is_empty() {
                return None;
            }
            files.write(&file.path, &apply(&file.text, changes)).ok().map(|_| file.path.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Files(RefCell<BTreeMap<String, String>>);

    impl ProjectFiles for Files {
        fn read(&self, path: &str) -> Result<String, String> {
            self.0.borrow().get(path).cloned().ok_or_else(|| format!("{path} not found"))
        }
        fn write(&self, path: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(path.into(), text.into());
            Ok(())
        }
        fn rename(&self, _: &str, _: &str) -> Result<(), String> {
            Err("no".into())
        }
        fn list(&self) -> Vec<String> {
            self.0.borrow().keys().cloned().collect()
        }
    }

    #[test]
    fn finds_and_rewrites_calls_elsewhere() {
        let files = Files::default();
        for (p, t) in [("A.kt", "fun f() = g(1)"), ("B.kt", "val x = g(2)"), ("C.kt", "val y = 3"), ("notes.txt", "g(4)")] {
            files.write(p, t).unwrap();
        }
        // A call site: just the closing parenthesis.
        let sites = |_: &str, text: &str| text.match_indices("g(").map(|(i, _)| CallSite { args_close: i + text[i..].find(')').unwrap(), ..Default::default() }).collect();
        let calling = find(&files, "A.kt", "g", &sites);
        assert_eq!(calling.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["B.kt"]);
        let written = rewrite(&files, &calling, &|f| f.calls.iter().map(|c| (c.args_close..c.args_close, ", 0".to_owned())).collect());
        assert_eq!(written, ["B.kt"]);
        assert_eq!(files.read("B.kt").unwrap(), "val x = g(2, 0)");
    }
}

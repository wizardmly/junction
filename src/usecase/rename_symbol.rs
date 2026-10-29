//! Rename a symbol, as Android Studio's Rename refactoring: the checked
//! occurrences of the old name, file by file, become the new name, and a
//! class's file follows its class. Files go through a port.

use std::collections::BTreeMap;

use crate::domain::rename::{apply, check_name, renamed_file, Occurrence};

pub use crate::domain::project_files::ProjectFiles;

/// What a rename changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Renamed {
    /// Files whose text changed (under their old paths).
    pub changed: Vec<String>,
    pub replaced: usize,
    /// A class's file renamed along with it: (from, to).
    pub file: Option<(String, String)>,
    /// Each changed file's (path, text before, text after), for Undo.
    pub edits: Vec<(String, String, String)>,
}

/// Renames `old` to `new` at the given occurrences. `declaration` is the
/// file declaring the symbol: when `rename_file` and it's named after
/// the old name, it's renamed too.
pub fn rename(files: &dyn ProjectFiles, occurrences: &[Occurrence], old: &str, new: &str, declaration: &str, rename_file: bool) -> Result<Renamed, String> {
    let extension = declaration.rsplit('.').next().unwrap_or_default();
    check_name(extension, new)?;
    if old == new {
        return Ok(Renamed::default());
    }
    let mut by_file: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
    for o in occurrences {
        by_file.entry(o.path.as_str()).or_default().push((o.line, o.col));
    }
    let mut result = Renamed::default();
    for (path, positions) in by_file {
        let text = files.read(path)?;
        let (new_text, count) = apply(&text, &positions, old, new);
        if count > 0 {
            files.write(path, &new_text)?;
            result.changed.push(path.to_owned());
            result.replaced += count;
            result.edits.push((path.to_owned(), text, new_text));
        }
    }
    if rename_file {
        if let Some(to) = renamed_file(declaration, old, new) {
            files.rename(declaration, &to)?;
            result.file = Some((declaration.to_owned(), to));
        }
    }
    Ok(result)
}

/// Undoes a rename, as Ctrl+Z after Android Studio's refactoring: the
/// file goes back to its old name and every changed file to its old
/// text. Refuses, touching nothing, when a file changed since.
pub fn undo(files: &dyn ProjectFiles, renamed: &Renamed) -> Result<(), String> {
    let moved = |path: &str| match &renamed.file {
        Some((from, to)) if from == path => to.clone(),
        _ => path.to_owned(),
    };
    for (path, _, after) in &renamed.edits {
        let current = moved(path);
        if files.read(&current)? != *after {
            return Err(format!("Cannot undo: {current} has changed since the rename"));
        }
    }
    if let Some((from, to)) = &renamed.file {
        files.rename(to, from)?;
    }
    for (path, before, _) in &renamed.edits {
        files.write(path, before)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Files(RefCell<HashMap<String, String>>);

    impl ProjectFiles for Files {
        fn read(&self, path: &str) -> Result<String, String> {
            self.0.borrow().get(path).cloned().ok_or_else(|| format!("{path} not found"))
        }
        fn write(&self, path: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(path.into(), text.into());
            Ok(())
        }
        fn rename(&self, from: &str, to: &str) -> Result<(), String> {
            let mut files = self.0.borrow_mut();
            if files.contains_key(to) {
                return Err(format!("{to} exists"));
            }
            let text = files.remove(from).ok_or("missing")?;
            files.insert(to.into(), text);
            Ok(())
        }
        fn list(&self) -> Vec<String> {
            self.0.borrow().keys().cloned().collect()
        }
    }

    fn at(path: &str, line: u32, col: u32) -> Occurrence {
        Occurrence { path: path.into(), line, col, text: String::new(), code: true, other: None }
    }

    #[test]
    fn renames_a_class_and_its_file() {
        let files = Files::default();
        files.write("a/Foo.kt", "class Foo {\n  fun make() = Foo()\n}\n").unwrap();
        files.write("a/Use.kt", "val f: Foo = Foo()\n").unwrap();
        let occurrences = [at("a/Foo.kt", 0, 6), at("a/Foo.kt", 1, 15), at("a/Use.kt", 0, 7), at("a/Use.kt", 0, 13)];
        let result = rename(&files, &occurrences, "Foo", "Bar", "a/Foo.kt", true).unwrap();
        assert_eq!(result.replaced, 4);
        assert_eq!(result.changed, ["a/Foo.kt", "a/Use.kt"]);
        assert_eq!(result.file, Some(("a/Foo.kt".into(), "a/Bar.kt".into())));
        assert_eq!(files.read("a/Bar.kt").unwrap(), "class Bar {\n  fun make() = Bar()\n}\n");
        assert_eq!(files.read("a/Use.kt").unwrap(), "val f: Bar = Bar()\n");
    }

    #[test]
    fn undoes_a_rename() {
        let files = Files::default();
        files.write("a/Foo.kt", "class Foo\n").unwrap();
        files.write("a/Use.kt", "val f = Foo()\n").unwrap();
        let result = rename(&files, &[at("a/Foo.kt", 0, 6), at("a/Use.kt", 0, 8)], "Foo", "Bar", "a/Foo.kt", true).unwrap();
        undo(&files, &result).unwrap();
        let mut list = files.list();
        list.sort();
        assert_eq!(list, ["a/Foo.kt", "a/Use.kt"]);
        assert_eq!(files.read("a/Foo.kt").unwrap(), "class Foo\n");
        assert_eq!(files.read("a/Use.kt").unwrap(), "val f = Foo()\n");
    }

    #[test]
    fn undo_refuses_after_later_edits() {
        let files = Files::default();
        files.write("A.kt", "val x = 1\nval y = x\n").unwrap();
        let result = rename(&files, &[at("A.kt", 0, 4), at("A.kt", 1, 8)], "x", "z", "A.kt", false).unwrap();
        files.write("A.kt", "val z = 2\nval y = z\n").unwrap();
        assert_eq!(undo(&files, &result), Err("Cannot undo: A.kt has changed since the rename".into()));
        assert_eq!(files.read("A.kt").unwrap(), "val z = 2\nval y = z\n");
    }

    #[test]
    fn refuses_bad_names() {
        let files = Files::default();
        files.write("A.kt", "val x = 1").unwrap();
        assert_eq!(rename(&files, &[at("A.kt", 0, 4)], "x", "val", "A.kt", false), Err("'val' is not a valid identifier".into()));
        assert_eq!(files.read("A.kt").unwrap(), "val x = 1");
        // Unchanged name: nothing to do.
        assert_eq!(rename(&files, &[at("A.kt", 0, 4)], "x", "x", "A.kt", false), Ok(Renamed::default()));
    }
}

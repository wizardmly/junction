//! IntelliJ changelists: named groups of local changes, kept per repository
//! in `<git dir>/junction/changelists`. Git itself knows nothing about them.

use std::collections::HashMap;
use std::path::PathBuf;

use super::Repository;

pub const DEFAULT_NAME: &str = "Changes";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changelist {
    pub name: String,
    /// The changelist's description, used as the commit message when it is committed.
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changelists {
    pub lists: Vec<Changelist>,
    pub active: String,
    /// Which list each changed file belongs to; files not listed go to the active list.
    pub files: HashMap<String, String>,
}

impl Default for Changelists {
    fn default() -> Self {
        Self {
            lists: vec![Changelist { name: DEFAULT_NAME.into(), comment: String::new() }],
            active: DEFAULT_NAME.into(),
            files: HashMap::new(),
        }
    }
}

fn path(repository: &Repository) -> PathBuf {
    repository.git_dir().join("junction").join("changelists")
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Changelists {
    pub fn load(repository: &Repository) -> Self {
        std::fs::read_to_string(path(repository)).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    pub fn save(&self, repository: &Repository) {
        let file = path(repository);
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, self.serialize());
    }

    fn parse(text: &str) -> Self {
        let mut this = Self { lists: Vec::new(), active: DEFAULT_NAME.into(), files: HashMap::new() };
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["list", name, active, comment] => {
                    let name = unescape(name);
                    if *active == "1" {
                        this.active = name.clone();
                    }
                    this.lists.push(Changelist { name, comment: unescape(comment) });
                }
                ["file", name, path] => {
                    this.files.insert(unescape(path), unescape(name));
                }
                _ => {}
            }
        }
        this.normalize();
        this
    }

    fn serialize(&self) -> String {
        let mut out = String::new();
        for list in &self.lists {
            let active = if list.name == self.active { "1" } else { "0" };
            out.push_str(&format!("list\t{}\t{active}\t{}\n", escape(&list.name), escape(&list.comment)));
        }
        let mut files: Vec<_> = self.files.iter().collect();
        files.sort();
        for (path, name) in files {
            out.push_str(&format!("file\t{}\t{}\n", escape(name), escape(path)));
        }
        out
    }

    /// The default list always exists, and the active list and file assignments point at real lists.
    fn normalize(&mut self) {
        if !self.lists.iter().any(|l| l.name == DEFAULT_NAME) {
            self.lists.insert(0, Changelist { name: DEFAULT_NAME.into(), comment: String::new() });
        }
        if !self.lists.iter().any(|l| l.name == self.active) {
            self.active = DEFAULT_NAME.into();
        }
        let names: Vec<String> = self.lists.iter().map(|l| l.name.clone()).collect();
        self.files.retain(|_, name| names.contains(name));
    }

    /// Assigns new changes to the active list and forgets files that are no longer changed.
    /// Returns whether anything changed.
    pub fn sync(&mut self, changed: &[String]) -> bool {
        let before = self.files.clone();
        self.files.retain(|path, _| changed.contains(path));
        for path in changed {
            self.files.entry(path.clone()).or_insert_with(|| self.active.clone());
        }
        before != self.files
    }

    pub fn list_of(&self, path: &str) -> &str {
        self.files.get(path).map(String::as_str).unwrap_or(&self.active)
    }

    pub fn add(&mut self, name: &str, comment: &str, make_active: bool) -> bool {
        let name = name.trim();
        if name.is_empty() || self.lists.iter().any(|l| l.name == name) {
            return false;
        }
        self.lists.push(Changelist { name: name.into(), comment: comment.into() });
        if make_active {
            self.active = name.into();
        }
        true
    }

    pub fn rename(&mut self, old: &str, name: &str, comment: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || (name != old && self.lists.iter().any(|l| l.name == name)) {
            return false;
        }
        // The default list keeps its name, as in IntelliJ ("Changes" can only get a comment).
        let name = if old == DEFAULT_NAME { DEFAULT_NAME } else { name };
        let Some(list) = self.lists.iter_mut().find(|l| l.name == old) else { return false };
        list.name = name.into();
        list.comment = comment.into();
        for value in self.files.values_mut() {
            if value == old {
                *value = name.into();
            }
        }
        if self.active == old {
            self.active = name.into();
        }
        true
    }

    /// Deletes a list; its files move to the default list.
    pub fn remove(&mut self, name: &str) {
        if name == DEFAULT_NAME {
            return;
        }
        self.lists.retain(|l| l.name != name);
        for value in self.files.values_mut() {
            if value == name {
                *value = DEFAULT_NAME.into();
            }
        }
        if self.active == name {
            self.active = DEFAULT_NAME.into();
        }
    }

    pub fn move_files(&mut self, paths: &[String], to: &str) {
        for path in paths {
            self.files.insert(path.clone(), to.into());
        }
    }

    pub fn comment(&self, name: &str) -> &str {
        self.lists.iter().find(|l| l.name == name).map_or("", |l| l.comment.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_moves_files() {
        let mut lists = Changelists::default();
        assert!(lists.add("Feature\tX", "line one\nline two", true));
        lists.sync(&["a.rs".into(), "b.rs".into()]);
        assert_eq!(lists.list_of("a.rs"), "Feature\tX");
        lists.move_files(&["b.rs".into()], DEFAULT_NAME);
        let parsed = Changelists::parse(&lists.serialize());
        assert_eq!(parsed, lists);

        lists.remove("Feature\tX");
        assert_eq!(lists.active, DEFAULT_NAME);
        assert_eq!(lists.list_of("a.rs"), DEFAULT_NAME);
        // Committed files are forgotten.
        lists.sync(&["b.rs".into()]);
        assert!(!lists.files.contains_key("a.rs"));
    }
}

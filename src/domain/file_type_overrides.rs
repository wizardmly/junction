//! Override File Type: files opened as another type than their name says,
//! as IntelliJ's OverrideFileTypeManager (kept per file, for every project).

use std::collections::BTreeMap;

/// The types a file can be overridden to: IntelliJ's names, and the
/// editor language each one highlights as.
pub const FILE_TYPES: &[(&str, &str)] = &[
    ("Plain text", "plaintext"),
    ("C", "c"),
    ("C++", "cpp"),
    ("C#", "csharp"),
    ("CMake", "cmake"),
    ("CSS", "css"),
    ("Dart", "dart"),
    ("Diff", "diff"),
    ("Go", "go"),
    ("Groovy", "groovy"),
    ("HTML", "html"),
    ("Java", "java"),
    ("JavaScript", "javascript"),
    ("JSON", "json"),
    ("Kotlin", "kotlin"),
    ("Lua", "lua"),
    ("Makefile", "make"),
    ("Markdown", "markdown"),
    ("Objective-C", "objc"),
    ("PHP", "php"),
    ("Properties", "properties"),
    ("Protocol Buffer", "proto"),
    ("Python", "python"),
    ("Ruby", "ruby"),
    ("Rust", "rust"),
    ("Scala", "scala"),
    ("Shell Script", "bash"),
    ("SQL", "sql"),
    ("Swift", "swift"),
    ("TOML", "toml"),
    ("TypeScript", "typescript"),
    ("TypeScript JSX", "tsx"),
    ("V", "v"),
    ("XML", "xml"),
    ("YAML", "yaml"),
];

/// IntelliJ's name for an editor language.
pub fn type_name(language: &str) -> Option<&'static str> {
    FILE_TYPES.iter().find(|(_, l)| *l == language).map(|(name, _)| *name)
}

/// The overridden files (absolute paths) and their editor languages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTypeOverrides {
    files: BTreeMap<String, String>,
}

impl FileTypeOverrides {
    /// One `path<TAB>language` per line; unknown languages are dropped.
    pub fn parse(text: &str) -> Self {
        let files = text
            .lines()
            .filter_map(|l| l.rsplit_once('\t'))
            .filter(|(path, language)| !path.is_empty() && type_name(language).is_some())
            .map(|(path, language)| (path.to_owned(), language.to_owned()))
            .collect();
        FileTypeOverrides { files }
    }

    pub fn text(&self) -> String {
        self.files.iter().map(|(path, language)| format!("{path}\t{language}\n")).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// Overrides `path` to `language`, or reverts it (`None`); whether
    /// anything changed.
    pub fn set(&mut self, path: &str, language: Option<&str>) -> bool {
        match language {
            Some(language) if type_name(language).is_some() => self.files.insert(path.to_owned(), language.to_owned()).as_deref() != Some(language),
            Some(_) => false,
            None => self.files.remove(path).is_some(),
        }
    }

    /// A file moved: its override follows it, and those of files inside a
    /// moved folder.
    pub fn moved(&mut self, from: &str, to: &str) -> bool {
        let moved: Vec<_> = self.files.keys().filter(|p| *p == from || p.strip_prefix(from).is_some_and(|r| r.starts_with('/'))).cloned().collect();
        for path in &moved {
            let language = self.files.remove(path).unwrap_or_default();
            self.files.insert(format!("{to}{}", &path[from.len()..]), language);
        }
        !moved.is_empty()
    }
}

/// Where the overrides are kept.
pub trait FileTypeStore {
    fn load(&self) -> FileTypeOverrides;
    fn save(&self, overrides: &FileTypeOverrides) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_and_reverts() {
        let mut o = FileTypeOverrides::parse("/p/a.txt\tkotlin\n/p/b.txt\tnope\n");
        assert_eq!(o.get("/p/a.txt"), Some("kotlin"));
        assert_eq!(o.get("/p/b.txt"), None);
        assert!(!o.set("/p/a.txt", Some("kotlin")));
        assert!(o.set("/p/c", Some("bash")));
        assert!(!o.set("/p/c", Some("klingon")));
        assert!(o.moved("/p", "/q"));
        assert_eq!(o.get("/q/c"), Some("bash"));
        assert!(o.set("/q/c", None));
        assert_eq!(FileTypeOverrides::parse(&o.text()), o);
    }
}

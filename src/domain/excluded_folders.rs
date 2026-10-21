//! Mark Directory As › Excluded: folders the project leaves out of its
//! index, searches and file lists, as IntelliJ's excluded roots.

/// The excluded folders, relative to the project with `/` separators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExcludedFolders {
    folders: Vec<String>,
}

impl ExcludedFolders {
    /// One folder per line.
    pub fn parse(text: &str) -> Self {
        let mut folders: Vec<String> = text.lines().map(|l| l.trim().trim_matches('/').to_owned()).filter(|l| !l.is_empty()).collect();
        folders.sort();
        folders.dedup();
        ExcludedFolders { folders }
    }

    pub fn text(&self) -> String {
        self.folders.iter().map(|f| format!("{f}\n")).collect()
    }

    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    /// Whether `path` is an excluded folder or inside one.
    pub fn is_excluded(&self, path: &str) -> bool {
        self.folders.iter().any(|f| path == f || path.strip_prefix(f.as_str()).is_some_and(|rest| rest.starts_with('/')))
    }

    /// Whether `dir` itself is marked Excluded (Cancel Exclusion applies).
    pub fn is_marked(&self, dir: &str) -> bool {
        self.folders.iter().any(|f| f == dir)
    }

    /// Marks `dir` Excluded; folders marked inside it are covered by it.
    pub fn exclude(&mut self, dir: &str) -> bool {
        let dir = dir.trim_matches('/');
        if dir.is_empty() || self.is_excluded(dir) {
            return false;
        }
        self.folders.retain(|f| !f.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/')));
        self.folders.push(dir.to_owned());
        self.folders.sort();
        true
    }

    pub fn cancel(&mut self, dir: &str) -> bool {
        let before = self.folders.len();
        self.folders.retain(|f| f != dir.trim_matches('/'));
        self.folders.len() != before
    }
}

/// Where a project keeps its excluded folders.
pub trait ExcludedStore {
    fn load(&self) -> ExcludedFolders;
    fn save(&self, folders: &ExcludedFolders) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_matches() {
        let mut e = ExcludedFolders::parse("build/\n\nout\n");
        assert_eq!(e.folders(), ["build", "out"]);
        assert!(e.is_excluded("build") && e.is_excluded("build/a.kt") && !e.is_excluded("buildSrc/a.kt"));
        assert!(e.exclude("docs/api"));
        assert!(e.exclude("docs"));
        assert_eq!(e.folders(), ["build", "docs", "out"]);
        assert!(!e.exclude("docs/api"));
        assert!(e.is_marked("docs") && !e.is_marked("docs/api"));
        assert!(e.cancel("docs") && !e.cancel("docs"));
        assert_eq!(ExcludedFolders::parse(&e.text()), e);
    }
}

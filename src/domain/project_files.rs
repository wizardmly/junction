//! The port through which refactorings reach the project's files.

/// The project's files, by project-relative path.
pub trait ProjectFiles {
    fn read(&self, path: &str) -> Result<String, String>;
    fn write(&self, path: &str, text: &str) -> Result<(), String>;
    /// Renames a file; fails when the new one exists.
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
    /// The project's files (not ignored ones).
    fn list(&self) -> Vec<String>;
}

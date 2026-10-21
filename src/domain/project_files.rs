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

/// Where a source file declares the class (interface, object, enum…)
/// named `name`: the name's offset, for Find Usages on the file.
pub fn class_declaration(text: &str, name: &str) -> Option<usize> {
    const KEYWORDS: [&str; 5] = ["class", "interface", "object", "enum", "record"];
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut from = 0;
    while let Some(found) = text[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = !text[..at].ends_with(is_ident);
        let after_ok = !text[from..].starts_with(is_ident);
        let keyword = text[..at].trim_end().rsplit(|c: char| !is_ident(c)).next().unwrap_or("");
        // Not in a comment.
        let line = &text[text[..at].rfind('\n').map_or(0, |i| i + 1)..at];
        let commented = line.contains("//") || line.trim_start().starts_with(['*', '/']);
        if before_ok && after_ok && !commented && text[..at].ends_with(char::is_whitespace) && KEYWORDS.contains(&keyword) {
            return Some(at);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_class_of_a_file() {
        let java = "package a;\n// class Calc is here\npublic final class Calc {\n}\n";
        assert_eq!(class_declaration(java, "Calc"), Some(java.rfind("Calc {").unwrap()));
        let kt = "@file:JvmName(\"X\")\npackage a\n\ndata class CalcItem(val x: Int)\nobject Calc\n";
        assert_eq!(class_declaration(kt, "Calc"), Some(kt.rfind("Calc").unwrap()));
        assert_eq!(class_declaration("fun calc() = 1\n", "Calc"), None);
    }
}

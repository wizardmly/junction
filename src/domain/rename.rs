//! Rename (Shift+F6), as Android Studio's: whether a new name is valid in
//! the language, the occurrences a rename changes (code, and optionally
//! comments and strings), applying them to a file's text, and the file a
//! class rename renames along with it.

/// An occurrence of the old name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    /// Project-relative.
    pub path: String,
    /// 0-based line, and UTF-16 column (as the index reports positions).
    pub line: u32,
    pub col: u32,
    /// The line's text, for the preview.
    pub text: String,
    /// In code; not in a comment or string literal.
    pub code: bool,
    /// What the name refers to here when it's another symbol of the same
    /// name (`Other.add`, a parameter): listed, but unchecked.
    pub other: Option<String>,
}

const JAVA_KEYWORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
    "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native", "new", "package", "private", "protected", "public",
    "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void", "volatile", "while", "true", "false",
    "null",
];

/// Kotlin's hard keywords: they can't name anything without backticks.
const KOTLIN_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null", "object", "package", "return", "super", "this", "throw",
    "true", "try", "typealias", "typeof", "val", "var", "when", "while",
];

/// Whether `name` can replace an identifier in a file of this extension;
/// Android Studio's message when it can't.
pub fn check_name(extension: &str, name: &str) -> Result<(), String> {
    let invalid = || Err(format!("'{name}' is not a valid identifier"));
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return invalid() };
    if !(first.is_alphabetic() || first == '_' || first == '$') || !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
        return invalid();
    }
    let keywords = match extension {
        "java" => JAVA_KEYWORDS,
        "kt" | "kts" => KOTLIN_KEYWORDS,
        _ => &[],
    };
    if keywords.contains(&name) {
        return invalid();
    }
    Ok(())
}

/// The byte offset of a 0-based line and UTF-16 column.
pub fn offset_of(text: &str, line: u32, col: u32) -> Option<usize> {
    let start = if line == 0 { 0 } else { text.match_indices('\n').nth(line as usize - 1)?.0 + 1 };
    let rest = &text[start..];
    let mut units = 0u32;
    for (i, c) in rest.char_indices() {
        if units >= col || c == '\n' {
            return (units == col).then_some(start + i);
        }
        units += c.len_utf16() as u32;
    }
    (units == col).then_some(start + rest.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Replaces the old name at each position (line, UTF-16 column) of a
/// file's text; a position where the old name, as a whole word, no
/// longer is gets skipped. The new text, and how many it replaced.
pub fn apply(text: &str, positions: &[(u32, u32)], old: &str, new: &str) -> (String, usize) {
    let mut starts: Vec<usize> = positions
        .iter()
        .filter_map(|(line, col)| offset_of(text, *line, *col))
        .filter(|&at| {
            text[at..].starts_with(old)
                // A `$` before it: a Kotlin string template's reference.
                && !text[..at].chars().next_back().is_some_and(|c| c != '$' && is_word_char(c))
                && !text[at + old.len()..].chars().next().is_some_and(is_word_char)
        })
        .collect();
    starts.sort_unstable();
    starts.dedup();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &at in &starts {
        out.push_str(&text[last..at]);
        out.push_str(new);
        last = at + old.len();
    }
    out.push_str(&text[last..]);
    (out, starts.len())
}

/// The file a class rename renames too: `Foo.kt` / `Foo.java` declaring
/// `Foo` becomes `Bar.kt`.
pub fn renamed_file(path: &str, old: &str, new: &str) -> Option<String> {
    let (dir, file) = match path.rsplit_once('/') {
        Some((d, f)) => (Some(d), f),
        None => (None, path),
    };
    let (stem, ext) = file.rsplit_once('.')?;
    if stem != old || !matches!(ext, "kt" | "java") {
        return None;
    }
    Some(match dir {
        Some(d) => format!("{d}/{new}.{ext}"),
        None => format!("{new}.{ext}"),
    })
}

/// "3 occurrences in 2 files", as the dialog counts them.
pub fn summary(occurrences: &[&Occurrence]) -> String {
    let mut files: Vec<&str> = occurrences.iter().map(|o| o.path.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    let n = occurrences.len();
    let f = files.len();
    format!("{n} occurrence{} in {f} file{}", if n == 1 { "" } else { "s" }, if f == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_names() {
        assert_eq!(check_name("kt", "newName"), Ok(()));
        assert_eq!(check_name("java", "_x$1"), Ok(()));
        assert_eq!(check_name("kt", "fun"), Err("'fun' is not a valid identifier".into()));
        assert_eq!(check_name("java", "class"), Err("'class' is not a valid identifier".into()));
        // A Kotlin soft keyword is a fine name.
        assert_eq!(check_name("kt", "data"), Ok(()));
        assert!(check_name("kt", "1abc").is_err());
        assert!(check_name("kt", "a-b").is_err());
        assert!(check_name("kt", "").is_err());
    }

    #[test]
    fn applies_occurrences() {
        let text = "val total = 1\nprint(total + totals)\n// é total\n";
        // Line 1 col 6 is `total`; col 14 is `totals` (not whole word); line 2 after an é.
        let (out, n) = apply(text, &[(0, 4), (1, 6), (1, 14), (2, 5)], "total", "sum");
        assert_eq!(n, 3);
        assert_eq!(out, "val sum = 1\nprint(sum + totals)\n// é sum\n");
        // A stale position (the text moved) is skipped.
        assert_eq!(apply("x total", &[(0, 0)], "total", "sum"), ("x total".to_string(), 0));
        assert_eq!(apply("a", &[(5, 0)], "a", "b").1, 0);
        assert_eq!(apply("\"$total\"", &[(0, 2)], "total", "sum").0, "\"$sum\"");
    }

    #[test]
    fn renames_class_files() {
        assert_eq!(renamed_file("app/src/main/java/a/Foo.kt", "Foo", "Bar").as_deref(), Some("app/src/main/java/a/Bar.kt"));
        assert_eq!(renamed_file("Foo.java", "Foo", "Bar").as_deref(), Some("Bar.java"));
        assert_eq!(renamed_file("a/Utils.kt", "Foo", "Bar"), None);
        assert_eq!(renamed_file("a/Foo.xml", "Foo", "Bar"), None);
    }

    #[test]
    fn counts() {
        let o = |path: &str| Occurrence { path: path.into(), line: 0, col: 0, text: String::new(), code: true, other: None };
        let (a, b, c) = (o("a"), o("a"), o("b"));
        assert_eq!(summary(&[&a, &b, &c]), "3 occurrences in 2 files");
        assert_eq!(summary(&[&a]), "1 occurrence in 1 file");
    }
}

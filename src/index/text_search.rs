//! Find in Files / Replace in Files: plain text, whole words or regex over
//! the project's files, with a file mask and IntelliJ's context filter
//! (anywhere, in comments, in string literals, or except either).

use std::ops::Range;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use regex::{Regex, RegexBuilder};

/// Where in the code a match may be, as the Find in Files filter menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchContext {
    #[default]
    Anywhere,
    InComments,
    InStringLiterals,
    ExceptComments,
    ExceptStringLiterals,
    ExceptCommentsAndStringLiterals,
}

impl SearchContext {
    pub const ALL: [SearchContext; 6] = [
        SearchContext::Anywhere,
        SearchContext::InComments,
        SearchContext::InStringLiterals,
        SearchContext::ExceptComments,
        SearchContext::ExceptStringLiterals,
        SearchContext::ExceptCommentsAndStringLiterals,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SearchContext::Anywhere => "Anywhere",
            SearchContext::InComments => "In Comments",
            SearchContext::InStringLiterals => "In String Literals",
            SearchContext::ExceptComments => "Except Comments",
            SearchContext::ExceptStringLiterals => "Except String Literals",
            SearchContext::ExceptCommentsAndStringLiterals => "Except Comments and String Literals",
        }
    }

    fn accepts(self, region: Option<Region>) -> bool {
        match self {
            SearchContext::Anywhere => true,
            SearchContext::InComments => region == Some(Region::Comment),
            SearchContext::InStringLiterals => region == Some(Region::String),
            SearchContext::ExceptComments => region != Some(Region::Comment),
            SearchContext::ExceptStringLiterals => region != Some(Region::String),
            SearchContext::ExceptCommentsAndStringLiterals => region.is_none(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_words: bool,
    pub regex: bool,
    /// "*.kt, *.java, !*Test.kt"; `None` when the File mask box is off.
    pub mask: Option<String>,
    pub context: SearchContext,
}

/// One line with at least one occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMatch {
    /// Relative to the project root.
    pub path: String,
    /// 0-based.
    pub line: u32,
    /// UTF-16 column of the first occurrence, for the editor.
    pub col: u32,
    /// The line as shown: leading whitespace trimmed, long lines cut around
    /// the first occurrence.
    pub text: String,
    /// Byte ranges of the occurrences within `text`.
    pub ranges: Vec<Range<usize>>,
    /// Byte range of the first occurrence in the file.
    pub offset: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    pub matches: Vec<TextMatch>,
    pub occurrences: usize,
    pub files: usize,
    /// More were found than the limit allowed.
    pub truncated: bool,
}

/// Which files a search covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileScope {
    /// Tracked and untracked files git does not ignore.
    Project,
    /// Files under a project folder ("" is the root).
    Under { dir: String, recursive: bool },
    /// Project files outside test folders.
    Production,
    /// Project files in test folders or named as tests.
    Tests,
    /// Exactly these files.
    Files(Vec<String>),
}

impl FileScope {
    pub fn select(&self, all: impl FnOnce() -> Vec<String>) -> Vec<String> {
        match self {
            FileScope::Files(files) => files.clone(),
            FileScope::Project => all(),
            FileScope::Production => all().into_iter().filter(|f| !is_test_path(f)).collect(),
            FileScope::Tests => all().into_iter().filter(|f| is_test_path(f)).collect(),
            FileScope::Under { dir, recursive } => {
                let dir = dir.trim_matches('/');
                all()
                    .into_iter()
                    .filter(|f| {
                        let rest = if dir.is_empty() { Some(f.as_str()) } else { f.strip_prefix(dir).and_then(|r| r.strip_prefix('/')) };
                        rest.is_some_and(|r| *recursive || !r.contains('/'))
                    })
                    .collect()
            }
        }
    }
}

/// Test sources: a test folder (src/test, androidTest, tests, __tests__, …)
/// or a test file name (FooTest.kt, foo_test.go, test_foo.py, foo.spec.ts).
pub fn is_test_path(path: &str) -> bool {
    let mut parts = path.split('/');
    let name = parts.next_back().unwrap_or(path);
    let folder = parts.any(|p| {
        let p = p.to_ascii_lowercase();
        p == "test" || p == "tests" || p == "androidtest" || p == "testfixtures" || p == "__tests__" || p == "spec" || p == "testing" || p.ends_with("test") && p.len() > 4 && p.starts_with("unit")
    });
    let stem = name.split('.').next().unwrap_or(name);
    folder
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || stem.ends_with("Spec")
        || stem.ends_with("_test")
        || stem.starts_with("test_")
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// The regex a query searches with, or the error to show under the field.
pub fn compile(query: &TextQuery) -> Result<Regex, String> {
    if query.text.is_empty() {
        return Err(String::new());
    }
    let mut pattern = if query.regex { query.text.clone() } else { regex::escape(&query.text) };
    if query.whole_words {
        if query.regex {
            pattern = format!(r"\b(?:{pattern})\b");
        } else {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if word(query.text.chars().next()) {
                pattern = format!(r"\b{pattern}");
            }
            if word(query.text.chars().last()) {
                pattern = format!(r"{pattern}\b");
            }
        }
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|e| match e {
            regex::Error::Syntax(s) => s.lines().last().unwrap_or("Bad regular expression").trim().to_owned(),
            other => other.to_string(),
        })
}

/// A compiled file mask: comma-separated globs, `!` excludes.
pub struct FileMask {
    include: Vec<Regex>,
    exclude: Vec<Regex>,
}

impl FileMask {
    pub fn new(mask: &str) -> Self {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for part in mask.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
            let (negated, glob) = match part.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, part),
            };
            let mut re = String::from("(?i)^");
            if !glob.contains('/') {
                re.push_str("(?:.*/)?");
            }
            for c in glob.chars() {
                match c {
                    '*' => re.push_str("[^/]*"),
                    '?' => re.push_str("[^/]"),
                    c => re.push_str(&regex::escape(&c.to_string())),
                }
            }
            re.push('$');
            if let Ok(re) = Regex::new(&re) {
                if negated { exclude.push(re) } else { include.push(re) }
            }
        }
        Self { include, exclude }
    }

    pub fn matches(&self, path: &str) -> bool {
        (self.include.is_empty() || self.include.iter().any(|r| r.is_match(path))) && !self.exclude.iter().any(|r| r.is_match(path))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Comment,
    String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Syntax {
    /// `//`, `/* */`, "…", '…'.
    CLike { char_quote: bool, backtick: bool, triple: bool, hash: bool },
    /// `#`, "…", '…'.
    Hash { triple: bool },
    /// `<!-- -->`, attribute values.
    Markup,
    /// `--`, '…'.
    Sql,
    None,
}

fn syntax_of(path: &str) -> Syntax {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let c = |char_quote, backtick, triple| Syntax::CLike { char_quote, backtick, triple, hash: false };
    match ext.as_str() {
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "m" | "mm" | "java" | "cs" | "scala" | "proto" | "css" | "scss" | "less" | "aidl" => c(true, false, false),
        "kt" | "kts" | "swift" | "groovy" | "gradle" => c(true, false, true),
        "dart" => c(true, false, true),
        "rs" => c(false, false, false),
        "go" | "v" => c(true, true, false),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "json5" => c(true, true, false),
        "php" => Syntax::CLike { char_quote: true, backtick: false, triple: false, hash: true },
        "py" | "pyi" => Syntax::Hash { triple: true },
        "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "cmake" | "rb" | "pl" | "mk" | "properties" | "conf" | "cfg" | "ini" | "pro" | "r" => Syntax::Hash { triple: false },
        "xml" | "html" | "htm" | "svg" | "xhtml" | "vue" | "plist" | "xsd" => Syntax::Markup,
        "sql" | "lua" => Syntax::Sql,
        _ if name == "Makefile" || name == "CMakeLists.txt" || name == "Dockerfile" || name.starts_with(".git") => Syntax::Hash { triple: false },
        _ => Syntax::None,
    }
}

/// Comment and string-literal ranges of a file, in order, by a small lexer
/// per language family.
pub fn regions(path: &str, text: &str) -> Vec<(Range<usize>, Region)> {
    let syntax = syntax_of(path);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let n = bytes.len();
    let until = |from: usize, end: &[u8]| -> usize {
        text[from..].as_bytes().windows(end.len()).position(|w| w == end).map(|p| from + p + end.len()).unwrap_or(n)
    };
    // A quoted string from `i` (at the quote) to after the closing quote.
    let quoted = |i: usize, quote: u8, multiline: bool| -> usize {
        let mut j = i + 1;
        while j < n {
            match bytes[j] {
                b'\\' => j += 2,
                b if b == quote => return j + 1,
                b'\n' if !multiline => return j,
                _ => j += 1,
            }
        }
        n
    };
    while i < n {
        let rest = &bytes[i..];
        match syntax {
            Syntax::CLike { char_quote, backtick, triple, hash } => {
                if rest.starts_with(b"//") || (hash && rest[0] == b'#') {
                    let end = until(i, b"\n").min(n);
                    let end = if end > i && bytes[end - 1] == b'\n' { end - 1 } else { end };
                    out.push((i..end, Region::Comment));
                    i = end.max(i + 1);
                } else if rest.starts_with(b"/*") {
                    let end = until(i + 2, b"*/");
                    out.push((i..end, Region::Comment));
                    i = end;
                } else if triple && rest.starts_with(b"\"\"\"") {
                    let end = until(i + 3, b"\"\"\"");
                    out.push((i..end, Region::String));
                    i = end;
                } else if rest[0] == b'"' {
                    let end = quoted(i, b'"', false);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else if backtick && rest[0] == b'`' {
                    let end = quoted(i, b'`', true);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else if rest[0] == b'\'' && (char_quote || rust_char(rest)) {
                    let end = quoted(i, b'\'', false);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
            Syntax::Hash { triple } => {
                if rest[0] == b'#' {
                    let end = until(i, b"\n");
                    let end = if end > i && bytes[end - 1] == b'\n' { end - 1 } else { end };
                    out.push((i..end, Region::Comment));
                    i = end.max(i + 1);
                } else if triple && (rest.starts_with(b"\"\"\"") || rest.starts_with(b"'''")) {
                    let end = until(i + 3, &rest[..3]);
                    out.push((i..end, Region::String));
                    i = end;
                } else if rest[0] == b'"' || rest[0] == b'\'' {
                    let end = quoted(i, rest[0], false);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
            Syntax::Markup => {
                if rest.starts_with(b"<!--") {
                    let end = until(i + 4, b"-->");
                    out.push((i..end, Region::Comment));
                    i = end;
                } else if rest[0] == b'"' {
                    let end = quoted(i, b'"', true);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
            Syntax::Sql => {
                if rest.starts_with(b"--") {
                    let end = until(i, b"\n");
                    let end = if end > i && bytes[end - 1] == b'\n' { end - 1 } else { end };
                    out.push((i..end, Region::Comment));
                    i = end.max(i + 1);
                } else if rest[0] == b'\'' || rest[0] == b'"' {
                    let end = quoted(i, rest[0], true);
                    out.push((i..end, Region::String));
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
            Syntax::None => break,
        }
    }
    out
}

/// Rust: `'a'`, `'\n'` are chars; `'a` alone is a lifetime.
fn rust_char(rest: &[u8]) -> bool {
    match rest.get(1) {
        Some(b'\\') => true,
        Some(_) => {
            let s = std::str::from_utf8(&rest[1..rest.len().min(6)]).unwrap_or("");
            s.chars().next().is_some_and(|c| s[c.len_utf8()..].starts_with('\''))
        }
        None => false,
    }
}

fn region_at(regions: &[(Range<usize>, Region)], offset: usize) -> Option<Region> {
    let ix = regions.partition_point(|(r, _)| r.end <= offset);
    regions.get(ix).filter(|(r, _)| r.start <= offset).map(|(_, region)| *region)
}

/// Occurrences of `re` in `text` that pass the query's context filter.
pub fn occurrences(path: &str, text: &str, re: &Regex, context: SearchContext) -> Vec<Range<usize>> {
    let regions = if context == SearchContext::Anywhere { Vec::new() } else { regions(path, text) };
    re.find_iter(text)
        .filter(|m| !m.is_empty())
        .filter(|m| context == SearchContext::Anywhere || context.accepts(region_at(&regions, m.start())))
        .map(|m| m.range())
        .collect()
}

const MAX_SHOWN: usize = 240;

/// Groups occurrences into one row per line.
pub fn line_matches(path: &str, text: &str, found: &[Range<usize>]) -> Vec<TextMatch> {
    let mut out: Vec<TextMatch> = Vec::new();
    let mut line = 0u32;
    let mut line_start = 0usize;
    let mut scanned = 0usize;
    let mut i = 0;
    while i < found.len() {
        let first = found[i].clone();
        for (k, b) in text.as_bytes()[scanned..first.start].iter().enumerate() {
            if *b == b'\n' {
                line += 1;
                line_start = scanned + k + 1;
            }
        }
        scanned = first.start;
        let mut line_end = text[line_start..].find('\n').map(|p| line_start + p).unwrap_or(text.len());
        if line_end > line_start && text.as_bytes()[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        let full = &text[line_start..line_end];
        let at = first.start - line_start;
        let (from, to) = shown_window(full, at);
        let mut ranges = Vec::new();
        // Every occurrence starting on this line.
        while i < found.len() && found[i].start <= line_end {
            let s = found[i].start - line_start;
            let e = found[i].end.min(line_end).max(found[i].start) - line_start;
            if s >= from && s < to.max(from + 1) {
                ranges.push(s - from..e.min(to) - from);
            }
            i += 1;
        }
        let col = full[..at].encode_utf16().count() as u32;
        out.push(TextMatch { path: path.to_owned(), line, col, text: full[from..to].to_owned(), ranges, offset: first });
    }
    out
}

/// The part of a line to show: leading whitespace trimmed and at most
/// MAX_SHOWN bytes around `at`, on char boundaries.
fn shown_window(line: &str, at: usize) -> (usize, usize) {
    let mut from = line.len() - line.trim_start().len();
    if from > at {
        from = at;
    }
    if at > from + MAX_SHOWN / 2 {
        from = at - MAX_SHOWN / 3;
    }
    while !line.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (from + MAX_SHOWN).min(line.len());
    while !line.is_char_boundary(to) {
        to -= 1;
    }
    let to = from + line[from..to].trim_end().len().max(at.saturating_sub(from).min(to - from));
    (from, to.max(from))
}

/// Reads a text file; `None` for binary, huge, or unreadable files.
pub fn read_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 20 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Searches `files` (relative to `root`) in order, in parallel. Stops after
/// `limit` lines or when `cancel` is set.
pub fn search(root: &Path, files: &[String], query: &TextQuery, re: &Regex, limit: usize, cancel: &AtomicBool) -> SearchResult {
    let mask = query.mask.as_deref().filter(|m| !m.trim().is_empty()).map(FileMask::new);
    let files: Vec<&String> = files.iter().filter(|f| mask.as_ref().is_none_or(|m| m.matches(f))).collect();
    let next = AtomicUsize::new(0);
    let found_lines = AtomicUsize::new(0);
    let per_file: Mutex<Vec<(usize, Vec<TextMatch>, usize)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) || found_lines.load(Ordering::Relaxed) > limit {
                        return;
                    }
                    let ix = next.fetch_add(1, Ordering::Relaxed);
                    let Some(rel) = files.get(ix) else { return };
                    let Some(text) = read_text(&root.join(rel)) else { continue };
                    let found = occurrences(rel, &text, re, query.context);
                    if found.is_empty() {
                        continue;
                    }
                    let rows = line_matches(rel, &text, &found);
                    found_lines.fetch_add(rows.len(), Ordering::Relaxed);
                    per_file.lock().unwrap().push((ix, rows, found.len()));
                }
            });
        }
    });
    let mut per_file = per_file.into_inner().unwrap();
    per_file.sort_by_key(|(ix, _, _)| *ix);
    let mut result = SearchResult::default();
    for (_, rows, count) in per_file {
        if result.matches.len() >= limit {
            result.truncated = true;
            break;
        }
        result.files += 1;
        result.occurrences += count;
        let room = limit - result.matches.len();
        if rows.len() > room {
            result.truncated = true;
        }
        result.matches.extend(rows.into_iter().take(room));
    }
    result
}

/// Replaces the query's occurrences in `text`. `only` limits it to these
/// byte offsets of occurrence starts (Replace Selected). Returns the new
/// text and how many were replaced.
pub fn replace(path: &str, text: &str, query: &TextQuery, re: &Regex, replacement: &str, only: Option<&[usize]>) -> (String, usize) {
    let found = occurrences(path, text, re, query.context);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut count = 0;
    for range in found {
        if only.is_some_and(|o| !o.contains(&range.start)) {
            continue;
        }
        out.push_str(&text[last..range.start]);
        if query.regex {
            let caps = re.captures_at(text, range.start);
            match caps.filter(|c| c.get(0).map(|m| m.range()) == Some(range.clone())) {
                Some(caps) => caps.expand(&java_replacement(replacement), &mut out),
                None => out.push_str(replacement),
            }
        } else {
            out.push_str(replacement);
        }
        last = range.end;
        count += 1;
    }
    out.push_str(&text[last..]);
    (out, count)
}

/// IntelliJ writes groups as `$1` and escapes with `\`; the regex crate
/// needs `${1}` so `$1abc` is not read as group "1abc".
fn java_replacement(replacement: &str) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('$') => out.push_str("$$"),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            '$' if chars.peek().is_some_and(|d| d.is_ascii_digit()) => {
                let mut digits = String::new();
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    digits.push(*d);
                    chars.next();
                }
                out.push_str(&format!("${{{digits}}}"));
            }
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> TextQuery {
        TextQuery { text: text.into(), ..Default::default() }
    }

    #[test]
    fn plain_case_and_words() {
        let text = "Foo foo food\nfoo_bar FOO";
        let re = compile(&q("foo")).unwrap();
        assert_eq!(occurrences("a.txt", text, &re, SearchContext::Anywhere).len(), 5);
        let re = compile(&TextQuery { case_sensitive: true, ..q("foo") }).unwrap();
        assert_eq!(occurrences("a.txt", text, &re, SearchContext::Anywhere).len(), 3);
        let re = compile(&TextQuery { whole_words: true, ..q("foo") }).unwrap();
        assert_eq!(occurrences("a.txt", text, &re, SearchContext::Anywhere).len(), 3);
        let re = compile(&q("a.b")).unwrap();
        assert!(re.is_match("a.b") && !re.is_match("axb"));
        assert!(compile(&TextQuery { regex: true, ..q("(") }).is_err());
    }

    #[test]
    fn rows_per_line() {
        let text = "  let a = foo(foo);\nbar\nfoo";
        let re = compile(&q("foo")).unwrap();
        let found = occurrences("a.rs", text, &re, SearchContext::Anywhere);
        let rows = line_matches("a.rs", text, &found);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "let a = foo(foo);");
        assert_eq!(rows[0].ranges, vec![8..11, 12..15]);
        assert_eq!((rows[0].line, rows[0].col), (0, 10));
        assert_eq!((rows[1].line, rows[1].col), (2, 0));
    }

    #[test]
    fn comments_and_strings() {
        let text = "val x = \"foo\" // foo\n/* foo */ foo('f')";
        let re = compile(&q("foo")).unwrap();
        let count = |c| occurrences("a.kt", text, &re, c).len();
        assert_eq!(count(SearchContext::Anywhere), 4);
        assert_eq!(count(SearchContext::InComments), 2);
        assert_eq!(count(SearchContext::InStringLiterals), 1);
        assert_eq!(count(SearchContext::ExceptCommentsAndStringLiterals), 1);
        assert_eq!(count(SearchContext::ExceptComments), 2);
        // Rust lifetimes are not char literals.
        let rs = "fn f<'a>(x: &'a str) { foo }";
        assert_eq!(occurrences("a.rs", rs, &re, SearchContext::ExceptStringLiterals).len(), 1);
        let py = "# foo\nx = 'foo'\nfoo";
        assert_eq!(occurrences("a.py", py, &re, SearchContext::ExceptCommentsAndStringLiterals).len(), 1);
    }

    #[test]
    fn scopes() {
        let all = || vec!["a.rs".to_owned(), "app/src/main/A.kt".to_owned(), "app/src/test/ATest.kt".to_owned(), "lib/x.go".to_owned(), "lib/x_test.go".to_owned()];
        assert_eq!(FileScope::Tests.select(all), vec!["app/src/test/ATest.kt", "lib/x_test.go"]);
        assert_eq!(FileScope::Production.select(all).len(), 3);
        assert_eq!(FileScope::Under { dir: "app".into(), recursive: true }.select(all).len(), 2);
        assert_eq!(FileScope::Under { dir: "app".into(), recursive: false }.select(all).len(), 0);
        assert_eq!(FileScope::Under { dir: "".into(), recursive: false }.select(all), vec!["a.rs"]);
    }

    #[test]
    fn masks() {
        let m = FileMask::new("*.kt, *.java, !*Test.kt");
        assert!(m.matches("app/src/Main.kt"));
        assert!(m.matches("A.JAVA"));
        assert!(!m.matches("app/src/MainTest.kt"));
        assert!(!m.matches("build.gradle"));
        assert!(FileMask::new("!*.md").matches("a.rs"));
    }

    #[test]
    fn replaces_with_groups() {
        let query = TextQuery { regex: true, ..q(r"(\w+)=(\d+)") };
        let re = compile(&query).unwrap();
        let (out, n) = replace("a.txt", "a=1 b=2", &query, &re, "$2$1x", None);
        assert_eq!((out.as_str(), n), ("1ax 2bx", 2));
        let plain = q("a.b");
        let re = compile(&plain).unwrap();
        let (out, n) = replace("a.txt", "a.b axb a.b", &plain, &re, "$0", Some(&[8]));
        assert_eq!((out.as_str(), n), ("a.b axb $0", 1));
    }

    #[test]
    fn searches_files_in_order() {
        let dir = std::env::temp_dir().join(format!("junction-find-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "foo\nfoo\n").unwrap();
        std::fs::write(dir.join("src/b.kt"), "x foo").unwrap();
        std::fs::write(dir.join("bin"), b"foo\0\0").unwrap();
        let files = vec!["bin".to_owned(), "src/a.rs".to_owned(), "src/b.kt".to_owned()];
        let query = q("foo");
        let re = compile(&query).unwrap();
        let cancel = AtomicBool::new(false);
        let r = search(&dir, &files, &query, &re, 100, &cancel);
        assert_eq!((r.files, r.occurrences, r.matches.len(), r.truncated), (2, 3, 3, false));
        assert_eq!(r.matches[2].path, "src/b.kt");
        let masked = TextQuery { mask: Some("*.kt".into()), ..query.clone() };
        assert_eq!(search(&dir, &files, &masked, &re, 100, &cancel).matches.len(), 1);
        let r = search(&dir, &files, &query, &re, 2, &cancel);
        assert!(r.truncated && r.matches.len() == 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}

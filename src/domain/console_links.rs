//! The hyperlinks IntelliJ's consoles put on output lines (its
//! `UrlFilter` and file-path filters): web addresses, and source paths
//! with a line (`/src/Foo.kt:12:5`, `file:///C:/src/Foo.kt:12:5`,
//! `Foo.kt: (12, 5)`, javac's `Foo.java:12: error`).

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleLink {
    Url(String),
    /// An absolute path; 1-based line and column.
    File { path: String, line: Option<u32>, col: Option<u32> },
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"https?://[^\s'"<>\]\)]+"#).unwrap())
}

fn file_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:file://)?(/?[A-Za-z]:[\\/][^\s:'"<>|*?]+|/[^\s:'"<>|*?]+)\.(kt|kts|java|xml|gradle|toml|json|properties|pro|html|txt|log)(?::(\d+)(?::(\d+))?|: \((\d+), (\d+)\))?"#).unwrap()
    })
}

/// The links in a line, in order.
pub fn links(line: &str) -> Vec<(Range<usize>, ConsoleLink)> {
    let mut out: Vec<(Range<usize>, ConsoleLink)> = Vec::new();
    for m in url_re().find_iter(line) {
        // A sentence's end isn't part of the address.
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':']);
        if url.starts_with("file:") {
            continue;
        }
        out.push((m.start()..m.start() + url.len(), ConsoleLink::Url(url.to_owned())));
    }
    for c in file_re().captures_iter(line) {
        let whole = c.get(0).unwrap();
        if out.iter().any(|(r, _)| r.start < whole.end() && whole.start() < r.end) {
            continue;
        }
        let mut path = format!("{}.{}", &c[1], &c[2]);
        // `file:///C:/…` on Windows.
        if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
            path.remove(0);
        }
        let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse().ok());
        let (line, col) = (num(3).or(num(5)), num(4).or(num(6)));
        out.push((whole.range(), ConsoleLink::File { path, line, col }));
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_urls_and_paths() {
        let l = links("For more details see https://docs.gradle.org/8.7/release-notes.html.");
        assert_eq!(l, vec![(21..67, ConsoleLink::Url("https://docs.gradle.org/8.7/release-notes.html".into()))]);
        let l = links("e: file:///C:/work/app/src/Main.kt:12:5 Unresolved reference: foo");
        assert_eq!(l[0].1, ConsoleLink::File { path: "C:/work/app/src/Main.kt".into(), line: Some(12), col: Some(5) });
        assert_eq!(l[0].0.start, 3);
        let l = links("/home/u/app/src/A.java:7: error: ';' expected");
        assert_eq!(l[0].1, ConsoleLink::File { path: "/home/u/app/src/A.java".into(), line: Some(7), col: None });
        let l = links("e: /p/B.kt: (3, 9): Type mismatch");
        assert_eq!(l[0].1, ConsoleLink::File { path: "/p/B.kt".into(), line: Some(3), col: Some(9) });
        assert!(links("BUILD FAILED in 42s").is_empty());
    }
}

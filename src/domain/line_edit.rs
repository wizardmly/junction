//! IntelliJ's basic editing actions on lines, as pure edits of a text and
//! its selection (byte offsets): Duplicate Line or Selection (Ctrl+D),
//! Comment with Line Comment (Ctrl+/) and Block Comment (Ctrl+Shift+/),
//! Move Line Up / Down (Alt+Shift+↑/↓), Join Lines (Ctrl+Shift+J), Start
//! New Line (Shift+Enter) and Toggle Case (Ctrl+Shift+U).

use std::ops::Range;

/// The result: the whole new text and the selection in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub selection: Range<usize>,
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The whole lines a selection touches (a selection ending at a line's
/// start doesn't take that line), without the last line break.
fn lines_of(text: &str, sel: &Range<usize>) -> Range<usize> {
    let end = if sel.end > sel.start && line_start(text, sel.end) == sel.end { sel.end - 1 } else { sel.end };
    line_start(text, sel.start)..line_end(text, end.max(sel.start))
}

/// Ctrl+D: the caret's line copied below it (the caret goes along), or
/// the selection copied after itself and selected.
pub fn duplicate(text: &str, sel: Range<usize>) -> Edited {
    if sel.is_empty() {
        let lines = lines_of(text, &sel);
        let copy = format!("\n{}", &text[lines.clone()]);
        let mut out = text.to_owned();
        out.insert_str(lines.end, &copy);
        let caret = sel.start + copy.len();
        return Edited { text: out, selection: caret..caret };
    }
    let copy = text[sel.clone()].to_owned();
    let mut out = text.to_owned();
    out.insert_str(sel.end, &copy);
    Edited { text: out, selection: sel.end..sel.end + copy.len() }
}

/// The language's line comment and block comment.
pub fn comment_tokens(extension: &str) -> (Option<&'static str>, Option<(&'static str, &'static str)>) {
    match extension {
        "kt" | "kts" | "java" | "gradle" | "groovy" | "c" | "h" | "cc" | "cpp" | "hpp" | "m" | "mm" | "rs" | "go" | "js" | "ts" | "tsx" | "jsx" | "swift" | "dart" | "scala" | "cs" | "aidl" | "proto" | "json5" => {
            (Some("//"), Some(("/*", "*/")))
        }
        "py" | "sh" | "bash" | "zsh" | "yml" | "yaml" | "toml" | "properties" | "pro" | "cmake" | "rb" | "r" | "gitignore" | "dockerfile" => (Some("#"), None),
        "xml" | "html" | "htm" | "svg" => (None, Some(("<!--", "-->"))),
        "sql" | "lua" | "hs" => (Some("--"), None),
        "css" | "scss" | "less" => (None, Some(("/*", "*/"))),
        _ => (Some("//"), Some(("/*", "*/"))),
    }
}

/// Ctrl+/: the selected lines (or the caret's) commented at the first
/// column, as IntelliJ's default for Java and Kotlin; uncommented when all
/// of them are. With no selection the caret moves to the next line. A
/// language without line comments (XML) wraps each line in its block
/// comment.
pub fn toggle_line_comment(text: &str, sel: Range<usize>, line: Option<&str>, block: Option<(&str, &str)>) -> Edited {
    let lines = lines_of(text, &sel);
    let body = &text[lines.clone()];
    let rows: Vec<&str> = body.split('\n').collect();
    let content = |row: &str| -> bool { !row.trim().is_empty() };
    let new_rows: Vec<String> = match (line, block) {
        (Some(prefix), _) => {
            let commented = rows.iter().filter(|r| content(r)).all(|r| r.trim_start().starts_with(prefix));
            rows.iter()
                .map(|r| {
                    if !content(r) {
                        r.to_string()
                    } else if commented {
                        // `// x` loses the space IntelliJ added only when it added one: keep it simple, drop the prefix.
                        let i = r.find(prefix).unwrap_or(0);
                        format!("{}{}", &r[..i], &r[i + prefix.len()..])
                    } else {
                        format!("{prefix}{r}")
                    }
                })
                .collect()
        }
        (None, Some((open, close))) => {
            let commented = rows.iter().filter(|r| content(r)).all(|r| r.trim().starts_with(open) && r.trim().ends_with(close));
            rows.iter()
                .map(|r| {
                    if !content(r) {
                        r.to_string()
                    } else if commented {
                        let indent = &r[..r.len() - r.trim_start().len()];
                        let inner = r.trim().strip_prefix(open).and_then(|s| s.strip_suffix(close)).unwrap_or(r.trim());
                        format!("{indent}{}", inner.trim())
                    } else {
                        let indent = &r[..r.len() - r.trim_start().len()];
                        format!("{indent}{open} {} {close}", r.trim())
                    }
                })
                .collect()
        }
        (None, None) => return Edited { text: text.to_owned(), selection: sel },
    };
    let replaced = new_rows.join("\n");
    let mut out = text.to_owned();
    out.replace_range(lines.clone(), &replaced);
    let delta = replaced.len() as isize - body.len() as isize;
    if sel.is_empty() {
        // The caret to the same column of the next line.
        let new_end = (lines.end as isize + delta) as usize;
        let col = sel.start - lines.start;
        let next = (new_end + 1).min(out.len());
        let caret = if new_end < out.len() { (next + col).min(line_end(&out, next)) } else { (sel.start as isize + delta.max(0)) as usize };
        return Edited { text: out, selection: caret..caret };
    }
    // The whole commented lines selected.
    Edited { selection: lines.start..(lines.end as isize + delta) as usize, text: out }
}

/// Ctrl+Shift+/: the selection wrapped in the block comment, or unwrapped
/// when it is one; with no selection, an empty comment at the caret.
pub fn toggle_block_comment(text: &str, sel: Range<usize>, block: (&str, &str)) -> Edited {
    let (open, close) = block;
    let selected = &text[sel.clone()];
    let mut out = text.to_owned();
    if let Some(inner) = selected.trim().strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
        let inner = inner.to_owned();
        out.replace_range(sel.clone(), &inner);
        return Edited { text: out, selection: sel.start..sel.start + inner.len() };
    }
    let wrapped = format!("{open}{selected}{close}");
    out.replace_range(sel.clone(), &wrapped);
    if sel.is_empty() {
        let caret = sel.start + open.len();
        return Edited { text: out, selection: caret..caret };
    }
    Edited { text: out, selection: sel.start..sel.start + wrapped.len() }
}

/// Alt+Shift+↑/↓: the selected lines (or the caret's) swapped with the
/// line above or below, the selection moving with them.
pub fn move_lines(text: &str, sel: Range<usize>, up: bool) -> Option<Edited> {
    let lines = lines_of(text, &sel);
    let block = &text[lines.clone()];
    if up {
        if lines.start == 0 {
            return None;
        }
        let above = line_start(text, lines.start - 1)..lines.start - 1;
        let mut out = text.to_owned();
        out.replace_range(above.start..lines.end, &format!("{block}\n{}", &text[above.clone()]));
        let shift = lines.start - above.start;
        return Some(Edited { text: out, selection: sel.start - shift..sel.end - shift });
    }
    if lines.end >= text.len() {
        return None;
    }
    let below = lines.end + 1..line_end(text, lines.end + 1);
    let mut out = text.to_owned();
    out.replace_range(lines.start..below.end, &format!("{}\n{block}", &text[below.clone()]));
    let shift = below.len() + 1;
    Some(Edited { text: out, selection: sel.start + shift..sel.end + shift })
}

/// Ctrl+Shift+J: the next line (or the selected lines) joined to the
/// caret's with a single space, its indent removed; the caret at the join.
pub fn join_lines(text: &str, sel: Range<usize>) -> Option<Edited> {
    let lines = lines_of(text, &sel);
    let mut out = text.to_owned();
    let mut end = if sel.is_empty() || !text[lines.clone()].contains('\n') { lines.end } else { line_start(text, lines.end).saturating_sub(1) };
    if end >= out.len() {
        return None;
    }
    let mut caret = end;
    // Join each break from the first through `end`.
    let mut at = lines.start;
    loop {
        let Some(nl) = out[at..].find('\n').map(|i| at + i) else { break };
        if nl > end {
            break;
        }
        let next_start = nl + 1;
        let indent = out[next_start..].len() - out[next_start..].trim_start_matches([' ', '\t']).len();
        let left = out[..nl].trim_end_matches([' ', '\t']).len();
        let next_is_empty = out[next_start + indent..].starts_with('\n') || next_start + indent == out.len();
        let joiner = if next_is_empty || out[..left].ends_with(['(', '[']) || out[next_start + indent..].starts_with([')', ']', '.', ',']) { "" } else { " " };
        out.replace_range(left..next_start + indent, joiner);
        caret = left;
        let removed = next_start + indent - left - joiner.len();
        end = end.saturating_sub(removed);
        at = left + joiner.len();
        if sel.is_empty() {
            break;
        }
    }
    Some(Edited { text: out, selection: caret..caret })
}

/// Shift+Enter: a new line after the caret's, indented like it.
pub fn start_new_line(text: &str, sel: Range<usize>) -> Edited {
    let start = line_start(text, sel.start);
    let end = line_end(text, sel.end);
    let line = &text[start..end];
    let mut indent = line[..line.len() - line.trim_start().len()].to_owned();
    if line.trim_end().ends_with(['{', '(', '[']) {
        indent.push_str("    ");
    }
    let mut out = text.to_owned();
    out.insert_str(end, &format!("\n{indent}"));
    let caret = end + 1 + indent.len();
    Edited { text: out, selection: caret..caret }
}

/// Ctrl+Shift+U: the selection (or the word at the caret) in upper case,
/// or lower case when it has no lower-case letter.
pub fn toggle_case(text: &str, sel: Range<usize>) -> Edited {
    let range = if sel.is_empty() {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = text[..sel.start].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(sel.start, |(i, _)| i);
        let end = text[sel.start..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(i, _)| sel.start + i);
        start..end
    } else {
        sel.clone()
    };
    let part = &text[range.clone()];
    let changed = if part.chars().any(char::is_lowercase) { part.to_uppercase() } else { part.to_lowercase() };
    let mut out = text.to_owned();
    out.replace_range(range.clone(), &changed);
    let selection = if sel.is_empty() { sel } else { range.start..range.start + changed.len() };
    Edited { text: out, selection }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates() {
        assert_eq!(duplicate("a\nbc\nd", 3..3), Edited { text: "a\nbc\nbc\nd".into(), selection: 6..6 });
        assert_eq!(duplicate("abc", 1..2), Edited { text: "abbc".into(), selection: 2..3 });
        assert_eq!(duplicate("x", 1..1).text, "x\nx");
    }

    #[test]
    fn line_comments() {
        let text = "fun a() {\n    b()\n}\n";
        let one = toggle_line_comment(text, 14..14, Some("//"), None);
        assert_eq!(one.text, "fun a() {\n//    b()\n}\n");
        // The caret on the next line (its column clamped to the line).
        assert!(one.text[line_start(&one.text, one.selection.start)..].starts_with('}'));
        let back = toggle_line_comment(&one.text, 10..10, Some("//"), None);
        assert_eq!(back.text, text);
        let all = toggle_line_comment(text, 0..text.len() - 1, Some("//"), None);
        assert_eq!(all.text, "//fun a() {\n//    b()\n//}\n");
        let xml = toggle_line_comment("  <a/>\n", 3..3, None, Some(("<!--", "-->")));
        assert_eq!(xml.text, "  <!-- <a/> -->\n");
        assert_eq!(toggle_line_comment(&xml.text, 3..3, None, Some(("<!--", "-->"))).text, "  <a/>\n");
    }

    #[test]
    fn block_comments() {
        let e = toggle_block_comment("a b c", 2..3, ("/*", "*/"));
        assert_eq!(e.text, "a /*b*/ c");
        assert_eq!(toggle_block_comment(&e.text, e.selection.clone(), ("/*", "*/")).text, "a b c");
    }

    #[test]
    fn moves_lines() {
        let text = "one\ntwo\nthree";
        let down = move_lines(text, 1..1, false).unwrap();
        assert_eq!(down, Edited { text: "two\none\nthree".into(), selection: 5..5 });
        let up = move_lines(text, 9..9, true).unwrap();
        assert_eq!(up.text, "one\nthree\ntwo");
        assert_eq!(up.selection, 5..5);
        assert!(move_lines(text, 0..0, true).is_none());
        assert!(move_lines(text, 9..9, false).is_none());
        assert_eq!(move_lines(text, 0..7, false).unwrap().text, "three\none\ntwo");
    }

    #[test]
    fn joins_and_starts_lines() {
        assert_eq!(join_lines("val a =\n    1 + 2\nx", 2..2).unwrap(), Edited { text: "val a = 1 + 2\nx".into(), selection: 7..7 });
        assert_eq!(join_lines("foo(\n  a\n)", 0..0).unwrap().text, "foo(a\n)");
        assert!(join_lines("last", 1..1).is_none());
        assert_eq!(start_new_line("    if (x) {\n}", 5..5), Edited { text: "    if (x) {\n        \n}".into(), selection: 21..21 });
    }

    #[test]
    fn toggles_case() {
        assert_eq!(toggle_case("val name = 1", 5..5).text, "val NAME = 1");
        assert_eq!(toggle_case("ABC", 0..3), Edited { text: "abc".into(), selection: 0..3 });
    }
}

//! Code › Rearrange Code for Java, with IntelliJ's default arrangement
//! (`JavaRearranger`'s default settings): in each class, fields first
//! (static final, then static, by visibility public, protected, package,
//! private), static initializers, final then other fields, initializers,
//! constructors, static methods, methods, then nested enums, interfaces,
//! static classes and classes. Entries matching the same rule keep their
//! order; a setter follows its getter (the GETTERS_AND_SETTERS grouping);
//! a field stays after the fields its initializer reads. Comments and
//! annotations move with their member. IntelliJ has no rearranger for
//! Kotlin.

use std::ops::Range;

use crate::domain::reformat::java_code_mask;

/// A member's kind, with what the rules read.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Field { name: String, reads: Vec<String> },
    Init,
    StaticInit,
    Constructor,
    Method { name: String, params: usize },
    Enum,
    Interface,
    Class,
}

/// One member of a class body: its text (comments and annotations above
/// it included, blank lines before it counted apart) and its rule.
#[derive(Debug)]
struct Entry {
    text: String,
    /// Blank lines before it: kept between fields only when it followed
    /// a field already.
    blank_before: usize,
    kind: Kind,
    rank: usize,
}

/// The file with every class's members in IntelliJ's default order.
pub fn rearrange_java(text: &str) -> String {
    let mask = java_code_mask(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (open, close, kind) in type_bodies(text, &mask, 0..text.len()) {
        out.push_str(&text[at..=open]);
        out.push_str(&arrange_body(text, &mask, open, close, &kind));
        at = close;
    }
    out.push_str(&text[at..]);
    out
}

/// The type declarations directly in `range` (not in a nested body): each
/// body's `{` and `}`, and its kind and name.
fn type_bodies(text: &str, mask: &[bool], range: Range<usize>) -> Vec<(usize, usize, (Kind, String))> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut header_start = range.start;
    let bytes = text.as_bytes();
    let mut i = range.start;
    while i < range.end {
        if !mask[i] {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'{' if depth == 0 => {
                let header = code_of(text, mask, header_start..i);
                let close = matching(text, mask, i);
                if let (Some(kind), Some(close)) = (type_kind(&header), close) {
                    found.push((i, close, kind));
                }
                i = close.unwrap_or(range.end);
                header_start = i + 1;
            }
            b';' | b'}' if depth == 0 => header_start = i + 1,
            _ => {}
        }
        i += 1;
    }
    found
}

/// The `}` closing the bracket at `open`.
fn matching(text: &str, mask: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, b) in text.bytes().enumerate().skip(open) {
        if !mask[i] {
            continue;
        }
        match b {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The code in `range`, comments and strings blanked to spaces and `_`.
fn code_of(text: &str, mask: &[bool], range: Range<usize>) -> String {
    text[range.clone()].char_indices().map(|(i, c)| if mask[range.start + i] { c } else if c.is_whitespace() { ' ' } else { '_' }).collect()
}

/// A type header's kind and name: `public static class A<T> extends B`.
fn type_kind(header: &str) -> Option<(Kind, String)> {
    let header = strip_annotations(header);
    let words: Vec<&str> = header.split(|c: char| c.is_whitespace() || c == '<' || c == '(').filter(|w| !w.is_empty()).collect();
    let at = words.iter().position(|w| matches!(*w, "class" | "interface" | "enum" | "record" | "@interface"))?;
    let kind = match words[at] {
        "enum" => Kind::Enum,
        "interface" | "@interface" => Kind::Interface,
        _ => Kind::Class,
    };
    Some((kind, words.get(at + 1)?.to_string()))
}

/// The text without its annotations (`@Override`, `@SuppressWarnings({"x"})`).
fn strip_annotations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut k = 0;
    while k < chars.len() {
        if chars[k] == '@' && chars.get(k + 1).is_some_and(|c| c.is_alphabetic()) && !text[text.char_indices().nth(k).map_or(0, |(b, _)| b)..].starts_with("@interface") {
            k += 1;
            while k < chars.len() && (chars[k].is_alphanumeric() || chars[k] == '_' || chars[k] == '.') {
                k += 1;
            }
            if chars.get(k) == Some(&'(') {
                let mut depth = 0;
                while k < chars.len() {
                    match chars[k] {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                k += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
            }
            continue;
        }
        out.push(chars[k]);
        k += 1;
    }
    out
}

/// A class body's new text, between its `{` at `open` and `}` at `close`.
fn arrange_body(text: &str, mask: &[bool], open: usize, close: usize, (kind, name): &(Kind, String)) -> String {
    let body = &text[open + 1..close];
    // The members start on the line after `{`; a one-line body stays.
    let Some(first_newline) = body.find('\n') else { return body.to_owned() };
    let mut start = open + 1 + first_newline + 1;
    // An enum's constants stay first.
    if *kind == Kind::Enum {
        let constants_end = (start..close).find(|&i| mask[i] && text.as_bytes()[i] == b';' && depth_at(text, mask, start, i) == 0);
        let Some(end) = constants_end else { return body.to_owned() };
        start = text[end..close].find('\n').map_or(close, |n| end + n + 1);
    }
    let interface = *kind == Kind::Interface;
    let mut entries = Vec::new();
    let mut entry_start = start;
    let mut depth = 0usize;
    let mut assigned = false;
    let bytes = text.as_bytes();
    let mut i = start;
    while i < close {
        if !mask[i] {
            i += 1;
            continue;
        }
        let ends = match bytes[i] {
            b'(' | b'[' | b'{' => {
                depth += 1;
                false
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                false
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                depth == 0 && !assigned
            }
            b'=' if depth == 0 => {
                assigned = true;
                false
            }
            b';' => depth == 0,
            _ => false,
        };
        if ends {
            // The member takes the rest of its line (a trailing comment).
            let end = text[i..close].find('\n').map_or(close, |n| i + n + 1);
            if end == close && !text[i + 1..close].trim().is_empty() {
                // Something else on the closing line: leave the body.
                return body.to_owned();
            }
            let mut e = entry(text, mask, entry_start..end, name, interface);
            if !entries.last().is_some_and(|p: &Entry| matches!(p.kind, Kind::Field { .. })) && matches!(e.kind, Kind::Field { .. }) {
                e.blank_before = 0;
            }
            entries.push(e);
            entry_start = end;
            assigned = false;
            i = end;
            continue;
        }
        i += 1;
    }
    if entries.len() < 2 && entries.iter().all(|e| !is_type(&e.kind)) {
        return body.to_owned();
    }
    let tail = &text[entry_start..close];
    let mut ordered = order(entries);
    // Nested classes are arranged too.
    for e in &mut ordered {
        if is_type(&e.kind) {
            let inner_mask = java_code_mask(&e.text);
            e.text = rearrange_java_nested(&e.text, &inner_mask);
        }
    }
    let mut out = text[open + 1..start].to_owned();
    for (n, e) in ordered.iter().enumerate() {
        let blanks = if n == 0 {
            0
        } else {
            let fields = matches!(e.kind, Kind::Field { .. }) && matches!(ordered[n - 1].kind, Kind::Field { .. });
            e.blank_before.min(2).max(usize::from(!fields))
        };
        for _ in 0..blanks {
            out.push('\n');
        }
        out.push_str(&e.text);
    }
    out.push_str(tail);
    out
}

/// A nested type's text with its body arranged.
fn rearrange_java_nested(text: &str, mask: &[bool]) -> String {
    match type_bodies(text, mask, 0..text.len()).into_iter().next() {
        Some((open, close, kind)) => format!("{}{}{}", &text[..=open], arrange_body(text, mask, open, close, &kind), &text[close..]),
        None => text.to_owned(),
    }
}

/// The bracket depth at `at`, counted from `from`.
fn depth_at(text: &str, mask: &[bool], from: usize, at: usize) -> usize {
    let mut depth = 0usize;
    for (i, b) in text[from..at].bytes().enumerate() {
        if mask[from + i] {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    depth
}

fn is_type(kind: &Kind) -> bool {
    matches!(kind, Kind::Enum | Kind::Interface | Kind::Class)
}

/// One member's entry: its text without the blank lines before it, and
/// its kind and rule.
fn entry(text: &str, mask: &[bool], range: Range<usize>, class: &str, interface: bool) -> Entry {
    let raw = &text[range.clone()];
    let mut blank_before = 0;
    let mut skip = 0;
    for line in raw.split_inclusive('\n') {
        if line.trim().is_empty() && skip + line.len() < raw.len() {
            blank_before += 1;
            skip += line.len();
        } else {
            break;
        }
    }
    let code = code_of(text, mask, range.start + skip..range.end);
    let (kind, modifiers) = classify(&code, class);
    let is_static = modifiers.contains(&"static") || interface && matches!(kind, Kind::Field { .. }) || matches!(kind, Kind::Enum | Kind::Interface);
    let is_final = modifiers.contains(&"final") || interface && matches!(kind, Kind::Field { .. });
    let visibility = if modifiers.contains(&"public") || interface {
        0
    } else if modifiers.contains(&"protected") {
        1
    } else if modifiers.contains(&"private") {
        3
    } else {
        2
    };
    let rank = match &kind {
        Kind::Field { .. } if is_static && is_final => visibility,
        Kind::Field { .. } if is_static => 4 + visibility,
        Kind::StaticInit => 8,
        Kind::Field { .. } if is_final => 9 + visibility,
        Kind::Field { .. } => 13 + visibility,
        Kind::Init => 18,
        Kind::Constructor => 19,
        Kind::Method { .. } if is_static => 20,
        Kind::Method { .. } => 21,
        Kind::Enum => 22,
        Kind::Interface => 23,
        Kind::Class if is_static => 24,
        Kind::Class => 25,
    };
    Entry { text: raw[skip..].to_owned(), blank_before, kind, rank }
}

/// A member's kind from its code, and its modifiers.
fn classify<'a>(code: &'a str, class: &str) -> (Kind, Vec<&'a str>) {
    // The header: up to the body, the initializer or the `;`.
    let end = code.find(['{', '=', ';']).unwrap_or(code.len());
    let header = &code[..end];
    let stripped = strip_annotations(header);
    let modifiers: Vec<&str> = ["public", "protected", "private", "static", "final", "abstract", "default"]
        .into_iter()
        .filter(|m| stripped.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == *m))
        .collect();
    if let Some((kind, _)) = type_kind(header) {
        return (kind, modifiers);
    }
    let words: Vec<&str> = stripped.split_whitespace().collect();
    if code[end..].starts_with('{') && words.iter().all(|w| *w == "static") {
        return (if words.is_empty() { Kind::Init } else { Kind::StaticInit }, modifiers);
    }
    if let Some(paren) = stripped.find('(') {
        let name: String = stripped[..paren].trim_end().chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
        let inside = &stripped[paren + 1..stripped.rfind(')').unwrap_or(stripped.len())];
        let params = if inside.trim().is_empty() { 0 } else { top_level_commas(inside) + 1 };
        return (if name == class { Kind::Constructor } else { Kind::Method { name, params } }, modifiers);
    }
    let name = stripped.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).last().unwrap_or("").to_owned();
    let reads = if code[end..].starts_with('=') { identifiers(&code[end + 1..]) } else { Vec::new() };
    (Kind::Field { name, reads }, modifiers)
}

fn top_level_commas(text: &str) -> usize {
    let mut depth = 0i32;
    text.chars()
        .filter(|c| {
            match c {
                '(' | '<' | '[' | '{' => depth += 1,
                ')' | '>' | ']' | '}' => depth -= 1,
                _ => {}
            }
            *c == ',' && depth == 0
        })
        .count()
}

/// The names an initializer uses (not after a `.`).
fn identifiers(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut after_dot = false;
    for c in code.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            if !after_dot && !word.starts_with(|d: char| d.is_ascii_digit()) {
                out.push(std::mem::take(&mut word));
            }
            word.clear();
        }
        if !c.is_whitespace() {
            after_dot = c == '.';
        }
    }
    out
}

/// The entries by rule, stable within a rule; setters after their getters
/// and fields after the fields they read.
fn order(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by_key(|e| e.rank);
    // GETTERS_AND_SETTERS: `setX(v)` right after `getX()` / `isX()`.
    let getter_of = |e: &Entry| match &e.kind {
        Kind::Method { name, params: 0 } => name.strip_prefix("get").or_else(|| name.strip_prefix("is")).filter(|p| p.starts_with(char::is_uppercase)).map(str::to_owned),
        _ => None,
    };
    let mut k = 0;
    while k < entries.len() {
        if let Some(property) = getter_of(&entries[k]) {
            let setter = entries.iter().position(|e| matches!(&e.kind, Kind::Method { name, params: 1 } if *name == format!("set{property}")));
            if let Some(s) = setter.filter(|s| *s != k + 1) {
                let moved = entries.remove(s);
                let at = if s < k { k } else { k + 1 };
                entries.insert(at, moved);
                if s < k {
                    k -= 1;
                }
            }
        }
        k += 1;
    }
    // A field's initializer reading a field placed after it: it moves
    // after that field, as Java needs.
    for _ in 0..entries.len() {
        let names: Vec<Option<String>> = entries.iter().map(|e| if let Kind::Field { name, .. } = &e.kind { Some(name.clone()) } else { None }).collect();
        let misplaced = entries.iter().enumerate().find_map(|(k, e)| {
            let Kind::Field { reads, .. } = &e.kind else { return None };
            let last = names.iter().enumerate().filter(|(j, n)| *j > k && n.as_ref().is_some_and(|n| reads.contains(n))).map(|(j, _)| j).max()?;
            Some((k, last))
        });
        let Some((k, last)) = misplaced else { break };
        let moved = entries.remove(k);
        entries.insert(last, moved);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_members_by_the_default_rules() {
        let text = "\
package a;

public class A {
    public void run() {
        if (x) { y(); }
    }

    private int count = LIMIT;
    // The limit.
    public static final int LIMIT = 3;

    static {
        init();
    }

    public A() {
    }

    class Inner {
        void b() {}
        int f;
    }

    private static String name() { return \"}\"; }
    protected final int[] values = {1, 2};
    enum Mode { ON, OFF }
}
";
        assert_eq!(
            rearrange_java(text),
            "\
package a;

public class A {
    // The limit.
    public static final int LIMIT = 3;

    static {
        init();
    }

    protected final int[] values = {1, 2};
    private int count = LIMIT;

    public A() {
    }

    private static String name() { return \"}\"; }

    public void run() {
        if (x) { y(); }
    }

    enum Mode { ON, OFF }

    class Inner {
        int f;

        void b() {}
    }
}
"
        );
    }

    #[test]
    fn keeps_setters_by_getters_and_field_dependencies() {
        let text = "class B {\n    void setName(String n) { name = n; }\n    int other() { return 1; }\n    String getName() { return name; }\n    private int b = a + 1;\n    private int a = 2;\n}\n";
        assert_eq!(
            rearrange_java(text),
            "class B {\n    private int a = 2;\n    private int b = a + 1;\n\n    int other() { return 1; }\n\n    String getName() { return name; }\n\n    void setName(String n) { name = n; }\n}\n"
        );
    }

    #[test]
    fn interfaces_and_enum_constants() {
        let text = "interface I {\n    void f();\n    int X = 1;\n}\nenum E {\n    A, B;\n    void f() {}\n    private final int v = 0;\n}\n";
        assert_eq!(rearrange_java(text), "interface I {\n    int X = 1;\n\n    void f();\n}\nenum E {\n    A, B;\n    private final int v = 0;\n\n    void f() {}\n}\n");
    }
}

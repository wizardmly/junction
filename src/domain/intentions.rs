//! Alt+Enter intentions that need no types, as IntelliJ's Kotlin and Java
//! intentions name them: Convert to block body / expression body, Add
//! braces to / Remove braces from 'if' (and 'else', 'for', 'while')
//! statements, Put arguments (parameters) on separate lines / on one line.

use std::ops::Range;

use crate::domain::imports::ImportStyle;
use crate::domain::parameter_info::call_at;

/// An intention: its title and the edit it makes; the caret after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intention {
    pub title: String,
    pub range: Range<usize>,
    pub replacement: String,
    pub caret: usize,
}

/// The intentions available at `caret`, in IntelliJ's order.
pub fn intentions(style: ImportStyle, text: &str, caret: usize) -> Vec<Intention> {
    let kotlin = style == ImportStyle::Kotlin;
    let unit = crate::domain::surround::indent_unit(text);
    let mut out = Vec::new();
    if kotlin {
        out.extend(block_body(text, caret, &unit));
        out.extend(expression_body(text, caret));
    }
    out.extend(add_braces(text, caret, &unit));
    out.extend(remove_braces(text, caret, kotlin));
    if kotlin {
        out.extend(argument_lines(text, caret, &unit));
    }
    out
}

/// The intentions Settings › Editor › Intentions lists, by language:
/// IntelliJ's family name (what the settings turn off) and description.
pub const FAMILIES: &[(&str, &str, &str)] = &[
    ("Kotlin", "Convert to block body", "Converts a function's expression body (= …) to a block body with return"),
    ("Kotlin", "Convert to expression body", "Converts a block body with a single return to an expression body"),
    ("Kotlin", "Add braces to statement", "Adds braces to an if, else, for or while statement without them"),
    ("Kotlin", "Remove braces from statement", "Removes the braces around a single-statement body"),
    ("Kotlin", "Put arguments on separate lines", "Puts each argument of a call on its own line"),
    ("Kotlin", "Put arguments on one line", "Joins a call's arguments onto one line"),
    ("Kotlin", "Put parameters on separate lines", "Puts each parameter of a declaration on its own line"),
    ("Kotlin", "Put parameters on one line", "Joins a declaration's parameters onto one line"),
    ("Kotlin", "Convert concatenation to template", "Converts a string concatenation to a string template"),
    ("Kotlin", "Convert template to concatenated string", "Converts a string template to a concatenation"),
    ("Kotlin", "Specify type explicitly", "Writes the inferred type of a property or variable"),
    ("Kotlin", "Specify return type explicitly", "Writes the inferred return type of a function"),
    ("Kotlin", "Add names to call arguments", "Names a call's positional arguments after the parameters"),
    ("Kotlin", "Add indices to 'for' loop", "Iterates with withIndex(), adding an index variable to a for loop"),
    ("Kotlin", "Remove indices in 'for' loop", "Drops an unused index variable and withIndex() from a for loop"),
    ("Kotlin", "Iterate over collection", "Wraps an expression of a collection, array, map or string in a for loop over it"),
    ("Java", "Add braces to statement", "Adds braces to an if, else, for or while statement without them"),
    ("Java", "Remove braces from statement", "Removes the braces around a single-statement body"),
    ("Java", "Replace 'for each' loop with indexed 'for' loop", "Loops over a list or an array by index instead of with for each"),
];

/// The family an intention's title belongs to (`Add braces to 'if'
/// statement` → `Add braces to statement`).
pub fn family(title: &str) -> &str {
    if title.starts_with("Add braces to ") {
        "Add braces to statement"
    } else if title.starts_with("Remove braces from ") {
        "Remove braces from statement"
    } else if title.starts_with("Iterate over '") {
        "Iterate over collection"
    } else {
        title
    }
}

/// A family's key in the settings' list of those turned off.
pub fn family_key(kotlin: bool, family: &str) -> String {
    format!("{}/{family}", if kotlin { "Kotlin" } else { "Java" })
}

/// The line holding `at`: its range (without the newline).
fn line_at(text: &str, at: usize) -> Range<usize> {
    let start = text[..at.min(text.len())].rfind('\n').map_or(0, |i| i + 1);
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    start..end
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// The bracket matching the opener at `open`, past strings and comments.
pub fn matching(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut i = open;
    while i < bytes.len() {
        let rest = &text[i..];
        match bytes[i] {
            _ if rest.starts_with("//") => {
                i += rest.find('\n').unwrap_or(rest.len());
                continue;
            }
            _ if rest.starts_with("/*") => {
                i += rest.find("*/").map_or(rest.len(), |e| e + 2);
                continue;
            }
            _ if rest.starts_with("\"\"\"") => {
                i += 3 + rest[3..].find("\"\"\"").map_or(rest.len() - 3, |e| e + 3);
                continue;
            }
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != q && bytes[j] != b'\n' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A Kotlin function on the caret's line: the header's `(`, its `)`, and
/// what follows.
fn function_at(text: &str, caret: usize) -> Option<(Range<usize>, usize, usize)> {
    let line = line_at(text, caret);
    let fun = text[line.clone()].find("fun ").map(|i| line.start + i)?;
    let open = text[fun..line.end].find('(').map(|i| fun + i)?;
    let close = matching(text, open)?;
    Some((line, open, close))
}

/// `fun f(): Int = 1` → `fun f(): Int {\n    return 1\n}`; offered when
/// the return type is written (without it, the block's type is Unit).
fn block_body(text: &str, caret: usize, unit: &str) -> Option<Intention> {
    let (line, _, close) = function_at(text, caret)?;
    let after = &text[close + 1..];
    let eq = after.find('=')? + close + 1;
    let ty = text[close + 1..eq].trim();
    let ty = ty.strip_prefix(':')?.trim();
    if ty.is_empty() || text[close + 1..eq].contains(['{', '\n']) {
        return None;
    }
    // The expression: to the end of its line once its brackets close.
    let mut end = eq + 1;
    let mut depth = 0i32;
    for (i, c) in text[eq + 1..].char_indices() {
        let at = eq + 1 + i;
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '\n' if depth <= 0 => {
                end = at;
                break;
            }
            _ => {}
        }
        end = at + c.len_utf8();
    }
    let expr = text[eq + 1..end].trim();
    if expr.is_empty() {
        return None;
    }
    let indent = indent_of(&text[line.clone()]).to_owned();
    let header = text[line.start..eq].trim_end();
    let statement = if ty == "Unit" { expr.to_owned() } else { format!("return {expr}") };
    let body_indent = format!("{indent}{unit}");
    let statement = statement.replace('\n', &format!("\n{unit}"));
    let replacement = format!("{} {{\n{body_indent}{statement}\n{indent}}}", header.trim_start());
    let range = line.start + indent.len()..end;
    let caret = range.start + replacement.find('{').unwrap_or(0) + 2 + body_indent.len();
    Some(Intention { title: "Convert to block body".into(), range, replacement, caret })
}

/// `fun f(): Int {\n    return 1\n}` → `fun f(): Int = 1`: a body of one
/// `return` statement.
fn expression_body(text: &str, caret: usize) -> Option<Intention> {
    let (line, _, close) = function_at(text, caret)?;
    let brace = text[close..line.end].rfind('{').map(|i| close + i)?;
    if !text[brace + 1..line.end].trim().is_empty() {
        return None;
    }
    let end = matching(text, brace)?;
    let body = text[brace + 1..end].trim();
    let expr = body.strip_prefix("return ")?.trim();
    // One statement: no other line at the body's level.
    if expr.lines().count() > 1 && matching_flat(expr) || expr.contains(';') {
        return None;
    }
    let header = text[line.start..brace].trim_end();
    let replacement = format!("{header} = {expr}");
    let range = line.start..end + 1;
    Some(Intention { title: "Convert to expression body".into(), caret: line.start + header.len() + 3, range, replacement })
}

/// Whether a multi-line text has a line break at bracket depth 0 (two
/// statements, not one expression across lines).
fn matching_flat(text: &str) -> bool {
    let mut depth = 0i32;
    let mut previous = ' ';
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '\n' if depth == 0 && !matches!(previous, '+' | '-' | '*' | '/' | '&' | '|' | '=' | '.' | ',' | '(') => return true,
            _ => {}
        }
        if !c.is_whitespace() {
            previous = c;
        }
    }
    false
}

/// The statement keyword starting a line (`if`, `else`, `for`, `while`)
/// and where its condition's `(` is (none for `else`).
fn statement_head(line: &str) -> Option<(&'static str, Option<usize>)> {
    let trimmed = line.trim_start();
    let lead = line.len() - trimmed.len();
    let trimmed_after_brace = trimmed.strip_prefix('}').map(str::trim_start).unwrap_or(trimmed);
    let lead = lead + (trimmed.len() - trimmed_after_brace.len());
    for kw in ["if", "for", "while"] {
        for prefix in ["else if", kw] {
            if prefix == "else if" && kw != "if" {
                continue;
            }
            if let Some(rest) = trimmed_after_brace.strip_prefix(prefix) {
                if rest.trim_start().starts_with('(') {
                    let paren = lead + prefix.len() + (rest.len() - rest.trim_start().len());
                    return Some((kw, Some(paren)));
                }
            }
        }
    }
    if trimmed_after_brace.starts_with("else") && !trimmed_after_brace.starts_with("else if") && trimmed_after_brace[4..].starts_with([' ', '{', '\t']) || trimmed_after_brace == "else" {
        return Some(("else", None));
    }
    None
}

/// `if (a) b()` → `if (a) {\n    b()\n}`; the body on the line or the next.
fn add_braces(text: &str, caret: usize, unit: &str) -> Option<Intention> {
    let line = line_at(text, caret);
    let line_text = &text[line.clone()];
    let (kw, paren) = statement_head(line_text)?;
    let body_start = match paren {
        Some(p) => matching(text, line.start + p)? + 1,
        None => line.start + line_text.find("else")? + 4,
    };
    if body_start > line.end {
        return None;
    }
    let rest = text[body_start..line.end].trim();
    if rest.starts_with('{') {
        return None;
    }
    let indent = indent_of(line_text).to_owned();
    // The body: the rest of the line, or the next line.
    let (body, body_end) = if rest.is_empty() {
        let next = line_at(text, (line.end + 1).min(text.len()));
        if next.start <= line.end || text[next.clone()].trim().is_empty() {
            return None;
        }
        (text[next.clone()].trim().to_owned(), next.end)
    } else {
        // Up to an `else` on the same line: that's its own statement.
        let rest_full = &text[body_start..line.end];
        let else_at = rest_full.find(" else ").or_else(|| rest_full.strip_suffix(" else").map(|s| s.len()));
        match else_at {
            Some(e) => (rest_full[..e].trim().to_owned(), body_start + e),
            None => (rest.to_owned(), line.end),
        }
    };
    // A comment alone isn't a body.
    if body.is_empty() || body.starts_with("//") {
        return None;
    }
    let tail = if body_end < line.end { format!(" {}", text[body_end..line.end].trim_start()) } else { String::new() };
    let replacement = format!(" {{\n{indent}{unit}{body}\n{indent}}}{tail}");
    let range = body_start..if body_end < line.end { line.end } else { body_end };
    let caret = body_start + 3 + indent.len() + unit.len();
    Some(Intention { title: format!("Add braces to '{kw}' statement"), range, replacement, caret })
}

/// `if (a) {\n    b()\n}` → `if (a) b()`: one statement, not a declaration.
fn remove_braces(text: &str, caret: usize, kotlin: bool) -> Option<Intention> {
    let line = line_at(text, caret);
    let line_text = &text[line.clone()];
    let (kw, paren) = statement_head(line_text)?;
    let after = match paren {
        Some(p) => matching(text, line.start + p)? + 1,
        None => line.start + line_text.find("else")? + 4,
    };
    let brace = text[after..line.end].find('{').map(|i| after + i)?;
    if !text[after..brace].trim().is_empty() {
        return None;
    }
    let end = matching(text, brace)?;
    let body = text[brace + 1..end].trim();
    let declares = ["val ", "var ", "fun ", "class ", "int ", "String ", "final "].iter().any(|d| body.starts_with(d));
    if body.is_empty() || body.contains('\n') || declares || (!kotlin && body.matches(';').count() > 1) {
        return None;
    }
    let replacement = format!(" {body}");
    let range = after..end + 1;
    Some(Intention { title: format!("Remove braces from '{kw}' statement"), caret: after + 1, range, replacement })
}

/// The top-level comma-separated items between `open` and `close`, each
/// trimmed, as written.
fn items(text: &str, open: usize, close: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = open + 1;
    let mut i = open + 1;
    while i < close {
        match text.as_bytes()[i] {
            b'(' | b'[' | b'{' | b'"' | b'\'' => {
                i = match text.as_bytes()[i] {
                    b'"' | b'\'' => {
                        let q = text.as_bytes()[i];
                        let mut j = i + 1;
                        while j < close && text.as_bytes()[j] != q {
                            j += if text.as_bytes()[j] == b'\\' { 2 } else { 1 };
                        }
                        j + 1
                    }
                    _ => matching(text, i).map_or(close, |m| m + 1),
                };
                continue;
            }
            b',' => {
                out.push(text[start..i].trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = text[start..close].trim();
    if !last.is_empty() {
        out.push(last.to_owned());
    }
    out
}

/// Put arguments (parameters) on separate lines, or on one line.
fn argument_lines(text: &str, caret: usize, unit: &str) -> Vec<Intention> {
    let Some(call) = call_at(text, caret, true).or_else(|| {
        // On a declaration's line before its `(`.
        let line = line_at(text, caret);
        let open = text[line.clone()].find('(').map(|i| line.start + i)?;
        call_at(text, open + 1, true)
    }) else {
        return Vec::new();
    };
    let Some(close) = matching(text, call.open) else { return Vec::new() };
    let args = items(text, call.open, close);
    if args.len() < 2 {
        return Vec::new();
    }
    let line = line_at(text, call.open);
    let declaration = text[line.start..call.name_at].split_whitespace().any(|w| matches!(w, "fun" | "class" | "constructor")) || text[line.start..call.open].trim_end().ends_with("constructor");
    let what = if declaration { "parameters" } else { "arguments" };
    let indent = indent_of(&text[line.clone()]).to_owned();
    let mut out = Vec::new();
    let multiline = text[call.open..close].contains('\n');
    if !multiline {
        let inner: Vec<String> = args.iter().map(|a| format!("{indent}{unit}{a}")).collect();
        let replacement = format!("(\n{}\n{indent})", inner.join(",\n"));
        out.push(Intention { title: format!("Put {what} on separate lines"), range: call.open..close + 1, caret: call.open + 2 + indent.len() + unit.len(), replacement });
    } else {
        let replacement = format!("({})", args.join(", "));
        out.push(Intention { title: format!("Put {what} on one line"), range: call.open..close + 1, caret: call.open + 1, replacement });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        let mut out = text.to_owned();
        out.replace_range(i.range.clone(), &i.replacement);
        out
    }

    #[test]
    fn families_name_every_title() {
        assert_eq!(family("Add braces to 'else' statement"), "Add braces to statement");
        assert_eq!(family("Remove braces from 'for' statement"), "Remove braces from statement");
        assert_eq!(family("Put parameters on one line"), "Put parameters on one line");
        for title in ["Convert to block body", "Put arguments on separate lines", "Specify type explicitly", "Add names to call arguments"] {
            assert!(FAMILIES.iter().any(|(l, f, _)| *l == "Kotlin" && *f == family(title)), "{title}");
        }
        assert_eq!(family_key(false, "Add braces to statement"), "Java/Add braces to statement");
    }

    fn find<'a>(all: &'a [Intention], title: &str) -> Option<&'a Intention> {
        all.iter().find(|i| i.title == title)
    }

    #[test]
    fn converts_function_bodies() {
        let text = "class A {\n    fun f(x: Int): Int = x * 2\n}";
        let all = intentions(ImportStyle::Kotlin, text, 18);
        let block = find(&all, "Convert to block body").unwrap();
        let converted = apply(text, block);
        assert_eq!(converted, "class A {\n    fun f(x: Int): Int {\n        return x * 2\n    }\n}");
        assert_eq!(&converted[block.caret..block.caret + 6], "return");
        let back = find(&intentions(ImportStyle::Kotlin, &converted, 18), "Convert to expression body").cloned().unwrap();
        assert_eq!(apply(&converted, &back), text);
        // No declared type: the block's would be Unit.
        assert!(find(&intentions(ImportStyle::Kotlin, "fun g() = 1", 4), "Convert to block body").is_none());
        let unit = apply("fun h(): Unit = println(1)", find(&intentions(ImportStyle::Kotlin, "fun h(): Unit = println(1)", 4), "Convert to block body").unwrap());
        assert_eq!(unit, "fun h(): Unit {\n    println(1)\n}");
        // Two statements stay a block.
        assert!(find(&intentions(ImportStyle::Kotlin, "fun k(): Int {\n    log()\n    return 1\n}", 4), "Convert to expression body").is_none());
    }

    #[test]
    fn adds_and_removes_braces() {
        let text = "fun f() {\n    if (a > 1) go()\n    for (x in xs)\n        use(x)\n}";
        let add = intentions(ImportStyle::Kotlin, text, 16);
        let braced = apply(text, find(&add, "Add braces to 'if' statement").unwrap());
        assert_eq!(braced, "fun f() {\n    if (a > 1) {\n        go()\n    }\n    for (x in xs)\n        use(x)\n}");
        let back = find(&intentions(ImportStyle::Kotlin, &braced, 16), "Remove braces from 'if' statement").cloned().unwrap();
        assert_eq!(apply(&braced, &back), text);
        let for_at = text.find("for").unwrap();
        let looped = apply(text, find(&intentions(ImportStyle::Kotlin, text, for_at), "Add braces to 'for' statement").unwrap());
        assert!(looped.contains("for (x in xs) {\n        use(x)\n    }\n}"), "{looped}");
        // `if (a) b else c` on one line: the `if` part.
        let inline = "if (a) b() else c()";
        assert_eq!(apply(inline, find(&intentions(ImportStyle::Kotlin, inline, 1), "Add braces to 'if' statement").unwrap()), "if (a) {\n    b()\n} else c()");
        let java = "    } else {\n        x = 1;\n    }";
        assert_eq!(apply(java, find(&intentions(ImportStyle::Java, java, 7), "Remove braces from 'else' statement").unwrap()), "    } else x = 1;");
        // A declaration keeps its braces.
        assert!(find(&intentions(ImportStyle::Kotlin, "if (a) {\n    val x = 1\n}", 1), "Remove braces from 'if' statement").is_none());
    }

    #[test]
    fn puts_arguments_on_lines() {
        let text = "    foo(1, \"a, b\", bar(2, 3))";
        let all = intentions(ImportStyle::Kotlin, text, 9);
        let split = apply(text, find(&all, "Put arguments on separate lines").unwrap());
        assert_eq!(split, "    foo(\n        1,\n        \"a, b\",\n        bar(2, 3)\n    )");
        let back = find(&intentions(ImportStyle::Kotlin, &split, 9), "Put arguments on one line").cloned().unwrap();
        assert_eq!(apply(&split, &back), text);
        let decl = "fun f(a: Int, b: String) {}";
        assert!(find(&intentions(ImportStyle::Kotlin, decl, 2), "Put parameters on separate lines").is_some());
        assert!(find(&intentions(ImportStyle::Kotlin, "foo(1)", 4), "Put arguments on separate lines").is_none());
    }
}

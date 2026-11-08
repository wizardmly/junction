//! Android Studio's "Android String References" code folding: an app
//! resource reference shown as its value, collapsed by default
//! (`getString(R.string.hello, name)` → `"Hello, {name}!"`,
//! `android:text="@string/app_name"` → `"My App"`). The rules follow AOSP's
//! `ResourceFoldingBuilder` / `InlinedResource`.

use std::ops::Range;

use crate::domain::android_resources::ResourceRef;
use crate::usecase::android_resources::ResourceTable;

/// AS's `FOLD_MAX_LENGTH`.
const FOLD_MAX_LENGTH: usize = 60;

/// Calls folded whole with their resource argument (and their receiver).
fn folds_call(name: &str) -> bool {
    matches!(name, "getString" | "getText" | "getInteger") || name.starts_with("getDimension") || name.starts_with("getQuantityString")
}

static CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\bR\.(string|dimen|integer|plurals)\.(\w+)").unwrap());
static XML: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#""@(string|dimen|integer|bool)/([\w.]+)""#).unwrap());

/// The folded regions of a Kotlin, Java or XML file and their placeholders.
/// Each region is on one line; regions are sorted and disjoint.
pub fn folds(table: &ResourceTable, path: &str, text: &str) -> Vec<(Range<usize>, String)> {
    let code = path.ends_with(".kt") || path.ends_with(".java");
    if !code && !path.ends_with(".xml") {
        return Vec::new();
    }
    let mut out: Vec<(Range<usize>, String)> = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        let regex = if code { &*CODE } else { &*XML };
        for c in regex.captures_iter(line) {
            let (whole, kind, name) = (c.get(0).unwrap(), &c[1], &c[2]);
            // The framework's `android.R.string.ok` and another package's
            // `com.x.R` are not folded.
            if code && line[..whole.start()].ends_with('.') {
                continue;
            }
            let range = whole.range();
            let Some(value) = table.value(&ResourceRef::new(kind, name)) else { continue };
            let (range, value) = match code.then(|| call_around(line, range.clone())).flatten() {
                Some((call, args)) => {
                    let args = if args.len() >= 3 && line[call.clone()].contains("getQuantityString") { &args[1..] } else { &args[..] };
                    (call, insert_arguments(&value, args))
                }
                None => (range, value),
            };
            let label = placeholder(kind, name, &value, !code);
            let range = line_start + range.start..line_start + range.end;
            if out.last().is_some_and(|(last, _)| last.end > range.start) {
                continue;
            }
            out.push((range, label));
        }
        line_start += line.len() + 1;
    }
    out
}

/// AS's placeholder: strings (and anything in XML) quoted, integers and
/// one-character values after their name, the rest as is.
fn placeholder(kind: &str, name: &str, value: &str, xml: bool) -> String {
    let value = value.replace('\n', "\\n").replace('\t', "\\t");
    if kind == "bool" {
        value
    } else if kind == "string" || kind == "plurals" || xml {
        format!("\"{}\"", shorten(&value, FOLD_MAX_LENGTH - 2))
    } else if kind == "integer" || value.chars().count() <= 1 {
        format!("{name}: {value}")
    } else {
        shorten(&value, FOLD_MAX_LENGTH)
    }
}

/// IntelliJ's `StringUtil.shortenTextWithEllipsis(text, max, 0)`.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// When `reference` is the first argument of a `getString(…)`-like call on
/// this line: the call with its receiver chain (`context.getString(…)`,
/// `resources.getQuantityString(…)`), and its arguments' text.
fn call_around(line: &str, reference: Range<usize>) -> Option<(Range<usize>, Vec<String>)> {
    let before = line[..reference.start].trim_end();
    let open = before.strip_suffix('(')?.len();
    let name_start = line[..open].trim_end().char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last()?.0;
    let name = line[name_start..open].trim_end();
    if !folds_call(name) {
        return None;
    }
    let (args, close) = arguments(line, open)?;
    Some((receiver_start(line, name_start)..close + 1, args))
}

/// The top-level arguments of the call whose `(` is at `open`, and where its
/// `)` is.
fn arguments(line: &str, open: usize) -> Option<(Vec<String>, usize)> {
    let mut depth = 0;
    let mut args = Vec::new();
    let mut from = open + 1;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line[open..].char_indices().map(|(i, c)| (open + i, c)) {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    let last = line[from..i].trim();
                    if !last.is_empty() {
                        args.push(last.to_string());
                    }
                    return Some((args, i));
                }
            }
            ',' if depth == 1 => {
                args.push(line[from..i].trim().to_string());
                from = i + 1;
            }
            _ => {}
        }
    }
    None
}

/// Where the receiver chain before a call's name starts: `a.b().c.` is
/// folded with it, like the qualified expression AS folds.
fn receiver_start(line: &str, name_start: usize) -> usize {
    let bytes = line.as_bytes();
    let mut start = name_start;
    loop {
        let mut i = start;
        // `?.` or `.` before the name.
        if i == 0 || bytes[i - 1] != b'.' {
            return start;
        }
        i -= 1;
        if i > 0 && bytes[i - 1] == b'?' {
            i -= 1;
        }
        // A call's parentheses, balanced.
        if i > 0 && bytes[i - 1] == b')' {
            let mut depth = 0;
            let mut j = i;
            while j > 0 {
                j -= 1;
                match bytes[j] {
                    b')' => depth += 1,
                    b'(' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 {
                return start;
            }
            i = j;
        }
        let ident = line[..i].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last().map(|(k, _)| k);
        match ident {
            Some(k) if k < i => start = k,
            _ => return start,
        }
    }
}

/// AOSP's `InlinedResource.insertArguments`: each `%s` / `%1$d` of the
/// format string replaced with `{argument}` (`args[0]` is the resource).
fn insert_arguments(format: &str, args: &[String]) -> String {
    static FORMAT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"%(\d+\$)?([-+#, 0(<]*)?(\d+)?(\.\d+)?([tT])?([a-zA-Z%])").unwrap());
    if !format.contains('%') || args.is_empty() {
        return format.to_string();
    }
    let mut out = String::with_capacity(format.len() * 2);
    let mut start = 0;
    let mut next = 1;
    let mut index = 0;
    while let Some(m) = FORMAT.captures_at(format, index) {
        let whole = m.get(0).unwrap();
        index = whole.end();
        if &m[6] == "%" {
            continue;
        }
        // An escaped `\%`.
        if format[..whole.start()].ends_with('\\') {
            continue;
        }
        let number = match m.get(1) {
            Some(n) => {
                let n: usize = n.as_str().trim_end_matches('$').parse().unwrap_or(0);
                next = n + 1;
                n
            }
            None => {
                next += 1;
                next - 1
            }
        };
        if number > 0 && number < args.len() {
            out.push_str(&format[start..whole.start()]);
            out.push('{');
            out.push_str(constant_text(&args[number]));
            out.push('}');
            start = index;
        }
    }
    out.push_str(&format[start..]);
    out
}

/// A string literal argument's text (AS's `tryEvaluateConstantAsText`),
/// otherwise the expression as written.
fn constant_text(arg: &str) -> &str {
    arg.strip_prefix('"').and_then(|a| a.strip_suffix('"')).filter(|a| !a.contains('"') && !a.contains('$')).unwrap_or(arg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::android_resources::{Declaration, ResourceSource};

    struct Fake(Vec<Declaration>);
    impl ResourceSource for Fake {
        fn scan(&self) -> Vec<Declaration> {
            self.0.clone()
        }
    }

    fn table() -> ResourceTable {
        let decl = |kind: &str, name: &str, value: &str| Declaration { resource: ResourceRef::new(kind, name), path: "app/src/main/res/values/x.xml".into(), folder: "values".into(), line: 1, col: 1, value: Some(value.into()) };
        ResourceTable::build(&Fake(vec![
            decl("string", "app_name", "My App"),
            decl("string", "hello", "Hello, %1$s! You have %2$d messages"),
            decl("string", "title", "@string/app_name"),
            decl("string", "long", &"x".repeat(80)),
            decl("dimen", "margin", "16dp"),
            decl("integer", "max", "3"),
            decl("bool", "tablet", "true"),
        ]))
    }

    fn labels(path: &str, text: &str) -> Vec<(String, String)> {
        folds(&table(), path, text).into_iter().map(|(r, l)| (text[r].to_string(), l)).collect()
    }

    #[test]
    fn folds_kotlin_and_java_references() {
        let kt = "val a = R.string.app_name\nval b = getString(R.string.title)\nval c = requireContext().getString(R.string.hello, user.name, \"5\")\nval d = resources.getDimension(R.dimen.margin)\nval e = R.integer.max\nval f = android.R.string.ok\nval g = R.string.missing\n";
        assert_eq!(
            labels("A.kt", kt),
            vec![
                ("R.string.app_name".into(), "\"My App\"".into()),
                ("getString(R.string.title)".into(), "\"My App\"".into()),
                ("requireContext().getString(R.string.hello, user.name, \"5\")".into(), "\"Hello, {user.name}! You have {5} messages\"".into()),
                ("resources.getDimension(R.dimen.margin)".into(), "16dp".into()),
                ("R.integer.max".into(), "max: 3".into()),
            ]
        );
        let java = "String s = context.getString(R.string.app_name);\nsetTitle(R.string.app_name);\n";
        assert_eq!(labels("A.java", java), vec![("context.getString(R.string.app_name)".into(), "\"My App\"".into()), ("R.string.app_name".into(), "\"My App\"".into())]);
    }

    #[test]
    fn folds_xml_attributes_and_shortens() {
        let xml = "<TextView android:text=\"@string/app_name\" android:padding=\"@dimen/margin\"\n  app:on=\"@bool/tablet\" android:hint=\"@string/long\" android:label=\"@android:string/ok\"/>";
        let got = labels("res/layout/a.xml", xml);
        assert_eq!(got[0], ("\"@string/app_name\"".into(), "\"My App\"".into()));
        assert_eq!(got[1], ("\"@dimen/margin\"".into(), "\"16dp\"".into()));
        assert_eq!(got[2], ("\"@bool/tablet\"".into(), "true".into()));
        assert_eq!(got[3].1, format!("\"{}…\"", "x".repeat(57)));
        assert_eq!(got.len(), 4);
    }
}

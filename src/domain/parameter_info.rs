//! Parameter Info (Ctrl+P) as IntelliJ shows it: the call around the
//! caret and the argument it's in, and each declaration's parameters as
//! its source writes them ("x: Int, y: String = \"\"" in Kotlin, "int x,
//! String y" in Java), the current one bold.

/// The call the caret is in: where its name and `(` are, and the
/// argument the caret is in (by position, or by a Kotlin named argument).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The callee's name and where it starts.
    pub name: String,
    pub name_at: usize,
    /// The `(`.
    pub open: usize,
    /// 0-based, counting the commas before the caret.
    pub argument: usize,
    /// `name = ` before the caret in its argument (Kotlin).
    pub named: Option<String>,
}

/// The innermost call whose parentheses hold `caret`; none inside a
/// lambda's braces or a string.
pub fn call_at(text: &str, caret: usize, kotlin: bool) -> Option<Call> {
    call_from(text, 0, caret, kotlin)
}

/// `call_at`, scanning from `from` (a call's `(` already known: what the
/// caret moved to is found without reading the file before it).
pub fn call_from(text: &str, from: usize, caret: usize, kotlin: bool) -> Option<Call> {
    let caret = caret.min(text.len());
    // Open brackets: (byte, char, commas so far, start of the current argument).
    let mut stack: Vec<(usize, char, usize, usize)> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = from;
    while i < caret {
        let rest = &text[i..];
        let c = bytes[i];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |e| e + 2);
            continue;
        }
        if rest.starts_with("\"\"\"") {
            i += 3 + rest[3..].find("\"\"\"").map_or(rest.len() - 3, |e| e + 3);
            continue;
        }
        match c {
            b'"' | b'\'' => {
                // To the closing quote, past escapes; `${…}` is skipped whole.
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != c && bytes[j] != b'\n' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                if j >= caret {
                    // The caret is in the string.
                    return None;
                }
                i = j + 1;
                continue;
            }
            b'(' | b'[' | b'{' => stack.push((i, c as char, 0, i + 1)),
            b')' | b']' | b'}' => {
                stack.pop();
            }
            b',' => {
                if let Some(top) = stack.last_mut() {
                    top.2 += 1;
                    top.3 = i + 1;
                }
            }
            _ => {}
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    let &(open, kind, argument, arg_start) = stack.last()?;
    if kind != '(' {
        return None;
    }
    // The callee: the word before `(`, past generic arguments.
    let mut end = text[..open].trim_end().len();
    if text[..end].ends_with('>') {
        let mut depth = 0;
        for (j, ch) in text[..end].char_indices().rev() {
            match ch {
                '>' => depth += 1,
                '<' => {
                    depth -= 1;
                    if depth == 0 {
                        end = text[..j].trim_end().len();
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let start = text[..end].char_indices().rev().take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_' || *ch == '$').last().map(|(j, _)| j)?;
    let name = text[start..end].to_owned();
    // `if (`, `for (`… aren't calls.
    const KEYWORDS: &[&str] = &["if", "for", "while", "when", "catch", "switch", "synchronized", "return", "fun", "class", "constructor", "init"];
    if name.is_empty() || KEYWORDS.contains(&name.as_str()) || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let current = text[arg_start..caret].trim_start();
    let named = kotlin
        .then(|| {
            let (n, _) = current.split_once('=')?;
            let n = n.trim();
            (!n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_') && !current.contains("==")).then(|| n.to_owned())
        })
        .flatten();
    Some(Call { name, name_at: start, open, argument, named })
}

/// A declaration's parameters, from its name at `at` in its source: the
/// list after it (past type parameters and, for a class, `constructor`),
/// each with whitespace collapsed and Kotlin's `val` / `var` dropped.
/// `Some(vec![])` for no parameters, none when there's no list (a class
/// without a primary constructor).
pub fn parameters(source: &str, at: usize) -> Option<Vec<String>> {
    let rest = source.get(at..)?;
    let name_len = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').map(char::len_utf8).sum::<usize>();
    let mut i = at + name_len;
    let bytes = source.as_bytes();
    // Type parameters, an annotated or private primary constructor.
    loop {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        let rest = &source[i..];
        if rest.starts_with('<') {
            let mut depth = 0;
            let mut j = i;
            for (k, ch) in rest.char_indices() {
                match ch {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            j = i + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i = j;
            continue;
        }
        let word: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '@').collect();
        if matches!(word.as_str(), "private" | "internal" | "protected" | "public" | "constructor") || word.starts_with('@') {
            i += word.len();
            // An annotation's own arguments.
            if word.starts_with('@') && source[i..].starts_with('(') {
                i += source[i..].find(')').map_or(0, |e| e + 1);
            }
            continue;
        }
        break;
    }
    if !source[i..].starts_with('(') {
        return None;
    }
    // To the matching `)`, splitting at top-level commas.
    let mut params = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let mut chars = source[i..].char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '(' | '[' | '{' | '<' if depth > 0 || ch == '(' => {
                depth += 1;
                if depth > 1 {
                    current.push(ch);
                }
            }
            ')' | ']' | '}' | '>' if depth > 1 || ch == ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                current.push(ch);
            }
            ',' if depth == 1 => params.push(std::mem::take(&mut current)),
            '"' | '\'' => {
                current.push(ch);
                while let Some((_, c2)) = chars.next() {
                    current.push(c2);
                    if c2 == '\\' {
                        if let Some((_, c3)) = chars.next() {
                            current.push(c3);
                        }
                    } else if c2 == ch {
                        break;
                    }
                }
            }
            '/' if chars.peek().is_some_and(|(_, n)| *n == '/') => {
                // A line comment in the list.
                for (_, c2) in chars.by_ref() {
                    if c2 == '\n' {
                        break;
                    }
                }
                current.push(' ');
            }
            _ => current.push(ch),
        }
    }
    params.push(current);
    Some(
        params
            .into_iter()
            .map(|p| {
                let p = p.split_whitespace().collect::<Vec<_>>().join(" ");
                let mut p = p.as_str();
                for prefix in ["override ", "private ", "protected ", "internal ", "public ", "val ", "var "] {
                    p = p.strip_prefix(prefix).unwrap_or(p);
                }
                p.to_owned()
            })
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// The parameter a call's argument goes to: by name, else by position, a
/// vararg taking the rest.
pub fn current_parameter(params: &[String], call: &Call) -> Option<usize> {
    let name_of = |p: &str| -> String {
        let p = p.trim_start_matches("vararg ").trim_start_matches("noinline ").trim_start_matches("crossinline ");
        p.split([':', ' ']).next().unwrap_or("").to_owned()
    };
    if let Some(named) = &call.named {
        return params.iter().position(|p| name_of(p) == *named);
    }
    if call.argument < params.len() {
        return Some(call.argument);
    }
    params.iter().position(|p| p.starts_with("vararg ") || p.contains("..."))
}

/// A declaration whose parameter count can't take the call's arguments,
/// grayed out as IntelliJ grays them.
pub fn fits(params: &[String], call: &Call) -> bool {
    call.argument < params.len().max(1) || params.iter().any(|p| p.starts_with("vararg ") || p.contains("..."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_call() {
        let text = "fun f() { foo(1, bar(\"a,(b\", 2), x) }";
        let at = text.find(" x)").unwrap() + 1;
        let call = call_at(text, at, true).unwrap();
        assert_eq!((call.name.as_str(), call.argument, call.open), ("foo", 2, text.find("foo(").unwrap() + 3));
        // In the inner call's second argument.
        let inner = call_at(text, text.find("2)").unwrap(), true).unwrap();
        assert_eq!((inner.name.as_str(), inner.argument), ("bar", 1));
        // In a string, a lambda, or outside any call: nothing.
        assert_eq!(call_at(text, text.find("a,(b").unwrap() + 2, true), None);
        assert_eq!(call_at("list.map { it }", 11, true), None);
        assert_eq!(call_at("if (x) y", 4, true), None);
        // Generic arguments, a named argument, Java's `new`.
        let generic = call_at("listOf<String>(a, ", 18, true).unwrap();
        assert_eq!((generic.name.as_str(), generic.argument), ("listOf", 1));
        assert_eq!(call_at("Foo(x = 1, y = ", 15, true).unwrap().named.as_deref(), Some("y"));
        assert_eq!(call_at("new StringBuilder(", 18, false).unwrap().name, "StringBuilder");
        // From a known `(`: the same call.
        assert_eq!(call_from(text, text.find("foo(").unwrap() + 3, at, true), Some(call));
    }

    #[test]
    fn reads_parameters() {
        let kt = "fun greet(name: String, times: Int = 1, vararg extra: Any) {}";
        assert_eq!(parameters(kt, 4), Some(vec!["name: String".to_owned(), "times: Int = 1".into(), "vararg extra: Any".into()]));
        let class = "data class Point<T>(\n    val x: Int, // across\n    private var y: Map<String, Int>,\n)";
        assert_eq!(parameters(class, class.find("Point").unwrap()), Some(vec!["x: Int".to_owned(), "y: Map<String, Int>".into()]));
        assert_eq!(parameters("class A private constructor(a: Int)", 6), Some(vec!["a: Int".to_owned()]));
        assert_eq!(parameters("class Plain {", 6), None);
        assert_eq!(parameters("fun none() = 1", 4), Some(vec![]));
        let java = "public static <T> List<T> of(T first, String s, int... rest) {";
        assert_eq!(parameters(java, java.find("of(").unwrap()), Some(vec!["T first".to_owned(), "String s".into(), "int... rest".into()]));
        assert_eq!(parameters("void log(String fmt, Object a) {}", 5), Some(vec!["String fmt".to_owned(), "Object a".into()]));
    }

    #[test]
    fn picks_the_current_parameter() {
        let params: Vec<String> = vec!["name: String".into(), "times: Int = 1".into(), "vararg extra: Any".into()];
        let call = |text: &str| call_at(text, text.len(), true).unwrap();
        assert_eq!(current_parameter(&params, &call("greet(")), Some(0));
        assert_eq!(current_parameter(&params, &call("greet(a, b, c, d, ")), Some(2));
        assert_eq!(current_parameter(&params, &call("greet(times = ")), Some(1));
        assert!(fits(&params, &call("greet(a, b, c, d, ")));
        let two: Vec<String> = vec!["a: Int".into(), "b: Int".into()];
        assert!(!fits(&two, &call("f(1, 2, ")) && fits(&[], &call("f(")));
    }
}

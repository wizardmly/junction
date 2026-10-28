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
    /// `name = ` before the caret in its argument (Kotlin, annotations);
    /// a Java annotation's unnamed argument is its `value`.
    pub named: Option<String>,
    /// `@Name(`: an annotation's arguments.
    pub annotation: bool,
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
    let annotation = text[..start].ends_with('@');
    let current = text[arg_start..caret].trim_start();
    let named = (kotlin || annotation)
        .then(|| {
            let (n, _) = current.split_once('=')?;
            let n = n.trim();
            (!n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_') && !current.contains("==")).then(|| n.to_owned())
        })
        .flatten()
        .or_else(|| (annotation && !kotlin && argument == 0).then(|| "value".to_owned()));
    Some(Call { name, name_at: start, open, argument, named, annotation })
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
        // A Java annotation's `int value() default 0`.
        if let Some((head, _)) = p.split_once("()") {
            return head.rsplit(' ').next().unwrap_or("").to_owned();
        }
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
    call.argument < params.len().max(1) || params.iter().any(|p| p.starts_with("vararg ") || p.contains("...") || p.contains("()"))
}

/// Past a comment or string starting at `i`, or None when none starts there.
fn skip_literal(text: &str, i: usize) -> Option<usize> {
    let rest = &text[i..];
    if rest.starts_with("//") {
        return Some(i + rest.find('\n').unwrap_or(rest.len()));
    }
    if rest.starts_with("/*") {
        return Some(i + rest.find("*/").map_or(rest.len(), |e| e + 2));
    }
    if let Some(body) = rest.strip_prefix("\"\"\"") {
        return Some(i + 3 + body.find("\"\"\"").map_or(body.len(), |e| e + 3));
    }
    let bytes = text.as_bytes();
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] != quote && bytes[j] != b'\n' {
        j += if bytes[j] == b'\\' { 2 } else { 1 };
    }
    Some((j + 1).min(text.len()))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The word before `at`, past whitespace.
fn word_before(text: &str, at: usize) -> &str {
    let head = text[..at].trim_end();
    let start = head.char_indices().rev().take_while(|(_, c)| is_word(*c) || *c == '@').last().map_or(head.len(), |(j, _)| j);
    &head[start..]
}

/// The class body's `{` after a class header from `from`, past type
/// parameters, a primary constructor and supertypes; None for a header
/// that ends without a body (a Kotlin `class A(val x: Int)`).
fn class_body(source: &str, from: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = from;
    while i < source.len() {
        if let Some(end) = skip_literal(source, i) {
            i = end;
            continue;
        }
        let c = source[i..].chars().next()?;
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            '{' if depth <= 0 => return Some(i),
            ';' | '}' | '=' if depth <= 0 => return None,
            '\n' if depth <= 0 => {
                // Kotlin ends a header at a line break unless the next line
                // (or this one) carries it on.
                let prev = source[..i].trim_end().chars().last();
                let next = source[i..].trim_start();
                let goes_on = matches!(prev, Some(':' | ',' | '<' | '(')) || next.starts_with([':', ',', '{', '<', '(']) || ["extends", "implements", "where", "throws", "permits"].iter().any(|w| next.starts_with(w));
                if !goes_on {
                    return None;
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    None
}

/// What a class's constructors take, from its name at `at`: Kotlin's
/// primary and `constructor(…)`s, Java's `Name(…)`s (a class declaring
/// none has the default one), a Java `@interface`'s elements one per row
/// (`String value() default ""`, as IntelliJ lists them). None when `at`
/// isn't a class's name.
pub fn constructors(source: &str, at: usize) -> Option<Vec<Vec<String>>> {
    let keyword = word_before(source, at);
    if !matches!(keyword, "class" | "record" | "@interface" | "enum") {
        return None;
    }
    let name: String = source[at..].chars().take_while(|c| is_word(*c)).collect();
    let mut out = Vec::new();
    if let Some(primary) = parameters(source, at) {
        out.push(primary);
    }
    let Some(body) = class_body(source, at + name.len()) else {
        return Some(if out.is_empty() { vec![Vec::new()] } else { out });
    };
    let mut depth = 0;
    let mut i = body;
    while i < source.len() {
        if let Some(end) = skip_literal(source, i) {
            i = end;
            continue;
        }
        let c = source[i..].chars().next()?;
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            c if depth == 1 && is_word(c) && !source[..i].ends_with(is_word) => {
                let word: String = source[i..].chars().take_while(|c| is_word(*c)).collect();
                let after = source[i + word.len()..].trim_start();
                let before = source[..i].trim_end();
                if keyword == "@interface" && after.starts_with("()") && !before.ends_with(['.', '=']) && word_before(source, i) != "new" {
                    // `Type name() [default value];`: its type is the text since the last `;` / `{`.
                    let start = before.rfind([';', '{', '}']).map_or(0, |s| s + 1);
                    let ty = before[start..].split_whitespace().filter(|w| !w.starts_with('@') && !matches!(*w, "public" | "abstract")).collect::<Vec<_>>().join(" ");
                    let tail = after[2..].split(';').next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
                    let element = if tail.is_empty() { format!("{ty} {word}()") } else { format!("{ty} {word}() {tail}") };
                    out.push(vec![element]);
                } else if after.starts_with('(')
                    && (word == "constructor" || (word == name && declares(source, i)))
                    && let Some(params) = parameters(source, i)
                {
                    out.push(params);
                }
                i += word.len();
                continue;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    if out.is_empty() && keyword != "@interface" {
        out.push(Vec::new());
    }
    Some(out)
}

/// Whether a Java `Name(` at `i` starts a declaration (a constructor),
/// not a call: after a modifier, an annotation or a statement's end.
fn declares(source: &str, i: usize) -> bool {
    let before = source[..i].trim_end();
    let prev = word_before(source, i);
    before.is_empty() || before.ends_with([';', '{', '}']) || matches!(prev, "public" | "private" | "protected") || prev.starts_with('@')
}

/// A declaration's parameter lists: a class's constructors, else its own.
pub fn declaration_parameters(source: &str, at: usize) -> Vec<Vec<String>> {
    constructors(source, at).unwrap_or_else(|| vec![parameters(source, at).unwrap_or_default()])
}

/// The name of the class whose body holds `at` (for `this(`).
pub fn enclosing_class(text: &str, at: usize) -> Option<usize> {
    let mut braces = Vec::new();
    let mut i = 0;
    let at = at.min(text.len());
    while i < at {
        if let Some(end) = skip_literal(text, i) {
            i = end;
            continue;
        }
        match text.as_bytes()[i] {
            b'{' => braces.push(i),
            b'}' => {
                braces.pop();
            }
            _ => {}
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    let header = regex::Regex::new(r"(?:^|[^\w@])(?:class|interface|record|enum)\s+([A-Za-z_$][\w$]*)").ok()?;
    braces.iter().rev().find_map(|&brace| {
        let start = text[..brace].rfind([';', '{', '}']).map_or(0, |s| s + 1);
        let found = header.captures_iter(&text[start..brace]).last()?;
        let name = found.get(1)?;
        (class_body(text, start + name.end()) == Some(brace)).then_some(start + name.start())
    })
}

/// Where the superclass is named in the header of the class named at
/// `at` (for `super(`): Java's `extends X`; Kotlin's supertype with a
/// constructor call (`: Base(x), I`), else its first.
pub fn superclass(text: &str, at: usize) -> Option<usize> {
    let name_end = at + text[at..].chars().take_while(|c| is_word(*c)).map(char::len_utf8).sum::<usize>();
    let end = class_body(text, name_end).unwrap_or_else(|| text[name_end..].find('\n').map_or(text.len(), |e| name_end + e));
    let header = &text[name_end..end];
    // Top-level `:` / `extends`, outside type parameters and the constructor.
    let mut depth = 0i32;
    let mut list = None;
    for (j, c) in header.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ':' if depth == 0 => {
                list = Some(j + 1);
                break;
            }
            'e' if depth == 0 && header[j..].starts_with("extends") && !header[..j].ends_with(is_word) && !header[j + 7..].starts_with(is_word) => {
                list = Some(j + 7);
                break;
            }
            _ => {}
        }
    }
    let list = list?;
    // The supertypes, split at top-level commas: (start of the name, followed by `(`).
    let mut items: Vec<(usize, bool)> = Vec::new();
    let mut depth = 0i32;
    let mut item = list;
    let push = |from: usize, to: usize, items: &mut Vec<(usize, bool)>| {
        let part = &header[from..to];
        let lead = part.len() - part.trim_start().len();
        let mut k = from + lead;
        // Past annotations.
        while header[k..].starts_with('@') {
            k += 1 + header[k + 1..].chars().take_while(|c| is_word(*c) || *c == '.').map(char::len_utf8).sum::<usize>();
            k += header[k..].len() - header[k..].trim_start().len();
        }
        let dotted = header[k..].chars().take_while(|c| is_word(*c) || *c == '.').map(char::len_utf8).sum::<usize>();
        if dotted == 0 {
            return;
        }
        let name = &header[k..k + dotted];
        let last = k + name.rfind('.').map_or(0, |d| d + 1);
        items.push((last, header[k + dotted..to].contains('(')));
    };
    for (j, c) in header[list..].char_indices() {
        let j = j + list;
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                push(item, j, &mut items);
                item = j + 1;
            }
            _ => {}
        }
        if depth == 0 && header[j..].starts_with("implements") && !header[..j].ends_with(is_word) {
            break;
        }
    }
    let tail = header[item..].find("implements").map_or(header.len(), |e| item + e);
    push(item, tail.max(item), &mut items);
    let (found, _) = items.iter().find(|(_, call)| *call).or(items.first())?;
    Some(name_end + found)
}

/// A call's argument expressions as written, to its `)` (or as far as
/// the text goes).
pub fn arguments(text: &str, open: usize) -> Vec<String> {
    argument_ranges(text, open).into_iter().map(|r| text[r].to_owned()).collect()
}

/// Where each of a call's arguments is, trimmed.
pub fn argument_ranges(text: &str, open: usize) -> Vec<std::ops::Range<usize>> {
    let trimmed = |r: std::ops::Range<usize>| {
        let s = &text[r.clone()];
        let start = r.start + (s.len() - s.trim_start().len());
        start..start + s.trim().len()
    };
    let mut args = Vec::new();
    let mut depth = 0;
    let mut current = open + 1;
    let mut i = open;
    while i < text.len() {
        if let Some(end) = skip_literal(text, i) {
            i = end;
            continue;
        }
        let c = text[i..].chars().next().unwrap_or(' ');
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    args.push(trimmed(current..i));
                    break;
                }
            }
            ',' if depth == 1 => {
                args.push(trimmed(current..i));
                current = i + 1;
            }
            ';' if depth == 1 => {
                args.push(trimmed(current..i));
                break;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    if i >= text.len() {
        args.push(trimmed(current.min(text.len())..text.len()));
    }
    if args.len() == 1 && args[0].is_empty() {
        args.clear();
    }
    args
}

/// Marks an argument rewritten to its declared type (`\u{1}String`).
const TYPED: char = '\u{1}';
/// Follows a typed argument's type with all its supertypes, when they are
/// known (`\u{1}Dog\u{2}Pet,Animal`).
const SUPERS: char = '\u{2}';

/// An argument whose type is known (from more than its declaration, e.g.
/// a call's declared return type), with its supertypes when all are known.
pub fn typed(ty: &str, supertypes: Option<&[String]>) -> String {
    match supertypes {
        Some(s) => format!("{TYPED}{ty}{SUPERS}{}", s.join(",")),
        None => format!("{TYPED}{ty}"),
    }
}

/// A typed argument's type (without its supertypes); None for one as
/// written.
pub fn typed_type(arg: &str) -> Option<&str> {
    arg.strip_prefix(TYPED).map(|t| t.split(SUPERS).next().unwrap_or(t))
}

/// `arguments`, with a bare variable replaced by what its nearest
/// declaration before the call says it is: its declared type, or the
/// literal it's initialized with (`val n = 3` → `3`), or the class it's
/// constructed from (`val p = Point(…)`); anything else stays as written.
pub fn typed_arguments(text: &str, open: usize, kotlin: bool) -> Vec<String> {
    let before = &text[..open.min(text.len())];
    arguments(text, open)
        .into_iter()
        .map(|arg| {
            let is_name = arg.starts_with(|c: char| c.is_alphabetic() || c == '_') && arg.chars().all(|c| c.is_alphanumeric() || c == '_');
            if !is_name || matches!(arg.as_str(), "true" | "false" | "null" | "this") {
                return arg;
            }
            declared_type(before, &arg, kotlin).unwrap_or(arg)
        })
        .collect()
}

/// The last declaration of `name` in `text`: a `TYPED` type, or a literal.
fn declared_type(text: &str, name: &str, kotlin: bool) -> Option<String> {
    let name = regex::escape(name);
    let pattern = if kotlin {
        // `val name = value`, or `val name: Type` and a parameter `name: Type`.
        format!(r"\b(?:val|var)\s+{name}\s*=\s*([^\s;]+)|\b{name}\s*:\s*([A-Za-z_][\w.]*(?:<[^=\n;]*?>)?\??)")
    } else {
        format!(r"(?:^|[^\w.])((?:[A-Za-z_][\w.]*(?:<[^;=()]*?>)?)(?:\[\])*)\s+{name}\s*([=;,)])")
    };
    let re = regex::Regex::new(&pattern).ok()?;
    let caps = re.captures_iter(text).last()?;
    if kotlin {
        return match (caps.get(1), caps.get(2)) {
            (_, Some(ty)) => Some(format!("{TYPED}{}", ty.as_str())),
            (Some(value), _) => value_type(value.as_str()),
            _ => None,
        };
    }
    let ty = caps.get(1)?.as_str();
    if matches!(ty, "return" | "new" | "throw" | "else" | "case" | "instanceof" | "yield") {
        return None;
    }
    // `var x = …`: the value's.
    if ty == "var" {
        let rest = &text[caps.get(0)?.end()..];
        return value_type(rest.split([';', '\n']).next()?.trim().trim_start_matches("new "));
    }
    Some(format!("{TYPED}{ty}"))
}

/// What an initializer shows of its type: a literal as is, `Foo(`'s class.
fn value_type(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with(['"', '\'']) || value.starts_with(|c: char| c.is_ascii_digit()) || matches!(value, "true" | "false") {
        let end = value.find([',', ')', ';']).unwrap_or(value.len());
        return Some(value[..end].trim().to_owned());
    }
    let class = value.split(['(', '<']).next()?;
    let simple = class.rsplit('.').next()?;
    (value.len() > class.len() && simple.starts_with(|c: char| c.is_uppercase()) && simple.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| format!("{TYPED}{class}"))
}

/// Whether a value of a declared type can go to a parameter of this type,
/// as far as names show: the same type (boxed or not), a Java primitive
/// widening, a supertype of every String or number; a class of the
/// project may be a subtype of any other.
fn accepts_type(ty: &str, arg: &str) -> bool {
    let (arg, supers) = match arg.split_once(SUPERS) {
        Some((a, s)) => (a, Some(s.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>())),
        None => (arg, None),
    };
    let base = |t: &str| {
        let t = t.trim().trim_end_matches('?');
        let t = t.split('<').next().unwrap_or(t);
        t.rsplit('.').next().unwrap_or(t).trim().to_owned()
    };
    let (param, value) = (base(ty), base(arg));
    if param.is_empty() || value.is_empty() || matches!(param.as_str(), "Any" | "Object") {
        return true;
    }
    if ty.contains("[]") != arg.contains("[]") {
        return false;
    }
    let primitive = |t: &str| -> Option<&'static str> {
        Some(match t {
            "int" | "Int" | "Integer" => "int",
            "long" | "Long" => "long",
            "short" | "Short" => "short",
            "byte" | "Byte" => "byte",
            "char" | "Char" | "Character" => "char",
            "boolean" | "Boolean" => "boolean",
            "float" | "Float" => "float",
            "double" | "Double" => "double",
            "String" => "String",
            _ => return None,
        })
    };
    match (primitive(&param), primitive(&value)) {
        (Some(p), Some(v)) if p == v => true,
        (Some(p), Some(v)) => {
            // Java widens a primitive to a wider one (not a boxed type, nor in Kotlin).
            let order = ["byte", "short", "int", "long", "float", "double"];
            let rank = |t: &str| order.iter().position(|o| *o == t);
            let java_primitive = param.chars().next().is_some_and(|c| c.is_ascii_lowercase());
            java_primitive && (rank(p).zip(rank(v)).is_some_and(|(p, v)| v < p) || (v == "char" && rank(p).is_some_and(|p| p >= 2)))
        }
        (None, Some(v)) => match param.as_str() {
            "Any" | "Object" | "Comparable" | "Serializable" => true,
            "CharSequence" => v == "String",
            "Number" => !matches!(v, "String" | "char" | "boolean"),
            _ => false,
        },
        (Some(_), None) => false,
        // A class: its own or a supertype, when they're all known.
        (None, None) => param == value || supers.is_none_or(|s| s.contains(&param.as_str())),
    }
}

/// Whether an argument as written can go to a parameter of this type, as
/// far as a literal shows (anything else may).
fn accepts(ty: &str, arg: &str) -> bool {
    let ty = ty.trim();
    let base = ty.trim_end_matches('?').rsplit('.').next().unwrap_or(ty);
    let base = base.split('<').next().unwrap_or(base).trim();
    if base.is_empty() || matches!(base, "Any" | "Object") || (base.len() == 1 && base.chars().all(|c| c.is_ascii_uppercase())) {
        return true;
    }
    let arg = arg.trim();
    if let Some(declared) = arg.strip_prefix(TYPED) {
        return accepts_type(ty, declared);
    }
    let numeric = |kinds: &[&str]| kinds.contains(&base);
    if arg == "null" {
        return ty.ends_with('?') || !matches!(base, "int" | "long" | "short" | "byte" | "char" | "boolean" | "float" | "double" | "Int" | "Long" | "Short" | "Byte" | "Char" | "Boolean" | "Float" | "Double");
    }
    if arg == "true" || arg == "false" {
        return matches!(base, "Boolean" | "boolean");
    }
    if arg.starts_with('"') {
        return matches!(base, "String" | "CharSequence" | "Comparable" | "Serializable");
    }
    if arg.starts_with('\'') && arg.len() <= 4 {
        return matches!(base, "Char" | "char" | "int" | "long" | "Character");
    }
    if arg.starts_with(|c: char| c.is_ascii_digit()) && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
        let lower = arg.to_ascii_lowercase();
        let hex = lower.starts_with("0x") || lower.starts_with("0b");
        if !hex && (lower.contains('.') || lower.ends_with('f') || lower.ends_with('d') || lower.contains('e')) {
            return if lower.ends_with('f') { numeric(&["Float", "float", "double", "Double", "Number"]) } else { numeric(&["Double", "double", "Number"]) };
        }
        if lower.ends_with('l') {
            return numeric(&["Long", "long", "float", "double", "Number"]);
        }
        return numeric(&["Int", "Long", "Short", "Byte", "int", "long", "short", "byte", "char", "float", "double", "Integer", "Number"]);
    }
    if arg.starts_with('{') {
        return ty.contains("->") || !matches!(base, "String" | "Int" | "Long" | "Boolean" | "Double" | "Float" | "Char" | "int" | "long" | "boolean" | "double" | "float" | "char");
    }
    true
}

/// A parameter's type: Kotlin's after `:`, Java's before the name.
fn parameter_type(param: &str) -> &str {
    if let Some((_, ty)) = param.split_once(':') {
        return ty.split(" = ").next().unwrap_or(ty);
    }
    let param = param.trim();
    param.rsplit_once(' ').map_or("", |(ty, _)| ty).trim_start_matches("final ")
}

/// Which overload the call's written arguments pick, as IntelliJ
/// highlights the one the call resolves to: the first that takes their
/// number and whose types the literals fit. None for a single overload or
/// when none (or, with nothing written, several) fit.
pub fn best_overload(overloads: &[Vec<String>], args: &[String]) -> Option<usize> {
    if overloads.len() < 2 {
        return None;
    }
    let takes = |params: &[String]| takes_arguments(params, args);
    let fitting: Vec<usize> = (0..overloads.len()).filter(|&i| takes(&overloads[i])).collect();
    // Of several, the one whose types the variables have exactly (a
    // project class may or may not implement a library interface).
    // The nearer a supertype, the better (IntelliJ's most specific).
    let exact = |params: &[String]| {
        let simple = |t: &str| t.trim().trim_end_matches('?').split('<').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_owned();
        args.iter()
            .zip(params)
            .map(|(a, p)| {
                let param = simple(parameter_type(p));
                let Some(t) = typed_type(a) else { return 0 };
                if simple(t) == param {
                    return 1000;
                }
                let supers = a.split_once(SUPERS).map_or("", |(_, s)| s);
                supers.split(',').position(|s| s == param).map_or(0, |i| 999 - i.min(998))
            })
            .sum::<usize>()
    };
    let best = fitting.iter().copied().rev().max_by_key(|&i| exact(&overloads[i]));
    match fitting.as_slice() {
        [one] => Some(*one),
        [_, ..] if !args.is_empty() => best,
        _ => None,
    }
}

/// Whether a declaration with these parameters takes the call's written
/// arguments: their number (defaults, vararg), Kotlin names, and the types
/// the literals and variables show.
pub fn takes_arguments(params: &[String], args: &[String]) -> bool {
    let vararg = params.iter().any(|p| p.starts_with("vararg ") || p.contains("..."));
    let required = params.iter().filter(|p| !p.contains(" = ") && !p.starts_with("vararg ") && !p.contains("...")).count();
    if args.iter().any(|a| named_argument(a).is_some()) {
        return args.iter().all(|a| named_argument(a).is_none_or(|n| params.iter().any(|p| p.trim_start_matches("vararg ").split([':', ' ']).next() == Some(n))));
    }
    if args.len() < required || (!vararg && args.len() > params.len()) {
        return false;
    }
    args.iter().enumerate().all(|(i, a)| {
        let p = params.get(i).or_else(|| params.last());
        p.is_none_or(|p| accepts(parameter_type(p).trim_end_matches("...").trim(), a))
    })
}

/// `name = value` → `name` (a Kotlin named argument).
fn named_argument(arg: &str) -> Option<&str> {
    let (n, rest) = arg.split_once('=')?;
    let n = n.trim();
    (!rest.starts_with('=') && !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(n)
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

    #[test]
    fn constructors_and_delegation() {
        let kt = "class Point(val x: Int) : Base(x), Shape {\n    constructor(x: Int, y: Int) : this(x)\n    fun f() = Point(1)\n}\n";
        let at = kt.find("Point").unwrap();
        assert_eq!(constructors(kt, at), Some(vec![vec!["x: Int".to_owned()], vec!["x: Int".to_owned(), "y: Int".into()]]));
        assert_eq!(enclosing_class(kt, kt.find("this(x").unwrap() + 5), Some(at));
        assert_eq!(&kt[superclass(kt, at).unwrap()..][..4], "Base");
        let java = "public class Foo extends a.Bar implements Baz {\n  private final Foo self = new Foo(1);\n  public Foo(int a) { super(a); }\n  Foo(String s, int b) throws IOException { this(b); }\n  void foo() { Foo(); }\n}";
        let at = java.find("Foo").unwrap();
        assert_eq!(constructors(java, at), Some(vec![vec!["int a".to_owned()], vec!["String s".to_owned(), "int b".into()]]));
        assert_eq!(enclosing_class(java, java.find("super(a").unwrap()), Some(at));
        assert_eq!(&java[superclass(java, at).unwrap()..][..3], "Bar");
        // No constructor written: the default one; a function isn't a class.
        assert_eq!(constructors("class Plain {\n  void f() {}\n}", 6), Some(vec![vec![]]));
        assert_eq!(constructors("class A(val x: Int)\nfun main() { A(1) }", 6), Some(vec![vec!["x: Int".to_owned()]]));
        assert_eq!(constructors("fun f(a: Int) {}", 4), None);
        assert_eq!(declaration_parameters("fun f(a: Int) {}", 4), vec![vec!["a: Int".to_owned()]]);
        // Kotlin: a class with no primary constructor calls `super(` on its supertype.
        let sec = "class V : View {\n    constructor(c: Context) : super(c)\n}";
        assert_eq!(&sec[superclass(sec, 6).unwrap()..][..4], "View");
    }

    #[test]
    fn annotation_elements() {
        let java = "public @interface Route {\n  String value() default \"\";\n  int[] codes();\n}";
        let rows = constructors(java, java.find("Route").unwrap()).unwrap();
        assert_eq!(rows, vec![vec!["String value() default \"\"".to_owned()], vec!["int[] codes()".to_owned()]]);
        let call = call_at("@Route(", 7, false).unwrap();
        assert!(call.annotation && call.named.as_deref() == Some("value"));
        assert_eq!(current_parameter(&rows[0], &call), Some(0));
        assert_eq!(current_parameter(&rows[1], &call), None);
        let named = call_at("@Route(codes = ", 15, false).unwrap();
        assert_eq!(current_parameter(&rows[1], &named), Some(0));
        assert!(fits(&rows[1], &call_at("@Route(\"a\", codes = ", 20, false).unwrap()));
    }

    #[test]
    fn picks_the_overload() {
        let ov: Vec<Vec<String>> = vec![vec!["int x".into()], vec!["String s".into()], vec!["String s".into(), "int n".into()]];
        assert_eq!(best_overload(&ov, &arguments("f(\"a\")", 1)), Some(1));
        assert_eq!(best_overload(&ov, &arguments("f(\"a\", 2)", 1)), Some(2));
        assert_eq!(best_overload(&ov, &arguments("f(3", 1)), Some(0));
        assert_eq!(best_overload(&ov, &arguments("f()", 1)), None);
        assert_eq!(best_overload(&ov[..1], &arguments("f(1)", 1)), None);
        let kt: Vec<Vec<String>> = vec![vec!["a: Int".into()], vec!["b: String = \"\"".into(), "c: Boolean".into()]];
        assert_eq!(best_overload(&kt, &arguments("f(c = true)", 1)), Some(1));
        assert_eq!(best_overload(&kt, &arguments("f(x, true)", 1)), Some(1));
        // A variable: what its declaration says.
        let java = "void g(String name, long big) {\n  int n = 3;\n  f(name); f(n); f(big); f(name, n);\n  return n;\n}";
        let at = |call: &str| java.find(call).unwrap() + 1;
        assert_eq!(best_overload(&ov, &typed_arguments(java, at("f(name)"), false)), Some(1));
        assert_eq!(best_overload(&ov, &typed_arguments(java, at("f(n)"), false)), Some(0));
        assert_eq!(best_overload(&ov, &typed_arguments(java, at("f(big)"), false)), None);
        assert_eq!(best_overload(&ov, &typed_arguments(java, at("f(name, n)"), false)), Some(2));
        let longs: Vec<Vec<String>> = vec![vec!["String s".into()], vec!["long x".into()], vec!["Object o".into()]];
        assert_eq!(best_overload(&longs, &typed_arguments(java, at("f(n)"), false)), Some(1));
        let kotlin = "fun g(flag: Boolean) {\n    val n = 3\n    var s: String? = null\n    val p = Point(1, 2)\n    f(s); f(n); f(flag); f(p); f(other)\n}";
        let at = |call: &str| kotlin.find(call).unwrap() + 1;
        let kt: Vec<Vec<String>> = vec![vec!["x: Int".into()], vec!["s: CharSequence?".into()], vec!["b: Boolean".into()], vec!["p: Point".into()]];
        assert_eq!(best_overload(&kt, &typed_arguments(kotlin, at("f(s)"), true)), Some(1));
        assert_eq!(best_overload(&kt, &typed_arguments(kotlin, at("f(n)"), true)), Some(0));
        assert_eq!(best_overload(&kt, &typed_arguments(kotlin, at("f(flag)"), true)), Some(2));
        assert_eq!(best_overload(&kt, &typed_arguments(kotlin, at("f(p)"), true)), Some(3));
        // Unknown: the first that takes one argument, as before.
        assert_eq!(best_overload(&kt, &typed_arguments(kotlin, at("f(other)"), true)), Some(0));
        assert_eq!(arguments("f(a, g(b, c), \"),\")", 1), vec!["a".to_owned(), "g(b, c)".into(), "\"),\"".into()]);
        let text = "f( a ,  g(b) )";
        assert_eq!(argument_ranges(text, 1).into_iter().map(|r| &text[r]).collect::<Vec<_>>(), vec!["a", "g(b)"]);
        // An inferred type with its supertypes: the most specific overload
        // that takes it, none that doesn't.
        let pets: Vec<Vec<String>> = vec![vec!["c: Car".into()], vec!["a: Animal".into()], vec!["p: Pet".into()], vec!["s: String".into()]];
        let dog = typed("Dog", Some(&["Pet".to_owned(), "Animal".into()]));
        assert_eq!(best_overload(&pets, std::slice::from_ref(&dog)), Some(2));
        assert!(!takes_arguments(&pets[0], std::slice::from_ref(&dog)));
        assert_eq!(best_overload(&pets[..2], std::slice::from_ref(&dog)), Some(1));
        // Supertypes unknown: any class may take it.
        assert!(takes_arguments(&pets[0], &[typed("Dog", None)]));
        assert_eq!(typed_type(&dog), Some("Dog"));
    }
}

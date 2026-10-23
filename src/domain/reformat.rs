//! Code › Reformat Code (Ctrl+Alt+L) for Kotlin and Java with IntelliJ's
//! default code style: lines re-indented by the brackets they're in (the
//! file's indent unit, else 4 spaces; Kotlin's official style puts arguments and chained calls one
//! level deeper, Java a continuation of 8), `case` bodies under their
//! label, a space after commas, after `if` / `for` / `while`… and before
//! `{`, single spaces around assignments and comparisons, at most 2 blank
//! lines kept, no trailing spaces. Strings, comments and raw strings keep
//! their text.

use std::ops::Range;

use crate::domain::imports::ImportStyle;

/// IntelliJ keeps up to 2 blank lines in code and declarations.
const KEEP_BLANK_LINES: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Code,
    Str,
    /// `"""…"""`: a Kotlin raw string or a Java text block.
    Raw,
    Char,
    LineComment,
    /// Kotlin's block comments nest.
    Block(usize),
    /// `${…}` in a Kotlin string: its brace depth.
    Template(usize),
}

/// What the scan found on a line.
#[derive(Default, Debug)]
struct Line {
    /// The state the line starts in (`Code` when outside everything).
    starts_in: Option<State>,
    /// Which bytes are code (not in a string or comment).
    code: Vec<bool>,
    /// Which bytes are in a comment.
    comment: Vec<bool>,
    /// Brackets of code: (byte, char).
    brackets: Vec<(usize, char)>,
}

/// The byte length of the char at `i` (a char boundary).
fn char_len(text: &str, i: usize) -> usize {
    text[i..].chars().next().map_or(1, char::len_utf8)
}

fn scan(text: &str, kotlin: bool) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut stack = vec![State::Code];
    for raw in text.split('\n') {
        let mut line = Line { starts_in: stack.last().copied(), code: vec![false; raw.len()], comment: vec![false; raw.len()], ..Default::default() };
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            let rest = &raw[i..];
            let top = *stack.last().unwrap_or(&State::Code);
            match top {
                State::Code | State::Template(_) => {
                    if rest.starts_with("//") {
                        stack.push(State::LineComment);
                        line.comment[i..].iter_mut().for_each(|c| *c = true);
                        i = bytes.len();
                        continue;
                    }
                    if rest.starts_with("/*") {
                        stack.push(State::Block(1));
                        line.comment[i] = true;
                        line.comment[i + 1] = true;
                        i += 2;
                        continue;
                    }
                    if rest.starts_with("\"\"\"") {
                        stack.push(State::Raw);
                        i += 3;
                        continue;
                    }
                    match c {
                        b'"' => stack.push(State::Str),
                        b'\'' => stack.push(State::Char),
                        b'{' if top != State::Code => {
                            if let Some(State::Template(d)) = stack.last_mut() {
                                *d += 1;
                            }
                        }
                        b'}' if top != State::Code => {
                            if let Some(State::Template(d)) = stack.last_mut() {
                                *d -= 1;
                                if *d == 0 {
                                    stack.pop();
                                }
                            }
                        }
                        b'(' | b'[' | b'{' | b')' | b']' | b'}' if top == State::Code => {
                            line.brackets.push((i, c as char));
                            line.code[i] = true;
                        }
                        // A template's expression (`${a + b}`) is spaced as code;
                        // its brackets don't indent.
                        _ => {
                            let n = char_len(raw, i);
                            line.code[i..i + n].iter_mut().for_each(|b| *b = true);
                        }
                    }
                    i += char_len(raw, i);
                }
                State::Str | State::Char => {
                    if c == b'\\' {
                        i += 1;
                        if i < bytes.len() {
                            i += char_len(raw, i);
                        }
                        continue;
                    }
                    if kotlin && top == State::Str && rest.starts_with("${") {
                        stack.push(State::Template(1));
                        i += 2;
                        continue;
                    }
                    if (top == State::Str && c == b'"') || (top == State::Char && c == b'\'') {
                        stack.pop();
                    }
                    i += char_len(raw, i);
                }
                State::Raw => {
                    if rest.starts_with("\"\"\"") {
                        // `""""` ends with the last three quotes.
                        let quotes = rest.bytes().take_while(|b| *b == b'"').count();
                        stack.pop();
                        i += quotes;
                        continue;
                    }
                    if kotlin && rest.starts_with("${") {
                        stack.push(State::Template(1));
                        i += 2;
                        continue;
                    }
                    i += char_len(raw, i);
                }
                State::LineComment => i = bytes.len(),
                State::Block(depth) => {
                    let n = char_len(raw, i);
                    line.comment[i..i + n].iter_mut().for_each(|b| *b = true);
                    if rest.starts_with("*/") {
                        line.comment[i + 1] = true;
                        stack.pop();
                        if depth > 1 {
                            stack.push(State::Block(depth - 1));
                        }
                        i += 2;
                    } else if kotlin && rest.starts_with("/*") {
                        stack.pop();
                        stack.push(State::Block(depth + 1));
                        i += 2;
                    } else {
                        i += n;
                    }
                }
            }
        }
        // A line comment, or a string left open (bad code), ends here.
        while matches!(stack.last(), Some(State::LineComment | State::Str | State::Char)) {
            stack.pop();
        }
        if stack.is_empty() {
            stack.push(State::Code);
        }
        lines.push(line);
    }
    lines
}

/// Why a line continues the one before.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reason {
    None,
    /// `.call()`, `?.call()`.
    Chain,
    /// `?: fallback`.
    Elvis,
    /// `&& b`, `|| b`, a line after one ending with them; Java's `? :`.
    Binary,
    /// After `=` or Kotlin's `->`.
    Assign,
    /// A Kotlin property's `get()` / `set(…)`.
    Accessor,
    /// A Kotlin class header's supertypes after `:`.
    Header,
}

/// The last code line: its code (strings as `_`, no comments), whether it
/// left a bracket open, the stack depth after it, why it continued the
/// one before, and its indent.
struct Previous {
    code: String,
    opened: bool,
    depth: usize,
    reason: Reason,
    indent: usize,
}

/// An open bracket: the indent of its line and of its content, in
/// columns, the line it's on, and whether a Java `case` body is open in it.
#[derive(Clone, Copy, Debug)]
struct Open {
    line_indent: usize,
    content: usize,
    line: usize,
    case_body: bool,
    /// A Kotlin class's or constructor's parameters: `) : Base()` after.
    header: bool,
}

/// The reformatted text. `only`: the 0-based lines to change (a
/// selection); the others keep their text.
pub fn reformat(style: ImportStyle, text: &str, only: Option<Range<usize>>) -> String {
    reformat_indented(style, text, only, None)
}

/// `reformat` with the indent unit given (`.editorconfig`'s), else the
/// file's.
pub fn reformat_indented(style: ImportStyle, text: &str, only: Option<Range<usize>>, indent: Option<&str>) -> String {
    let kotlin = style == ImportStyle::Kotlin;
    // The project's code style is the file's: its indent unit (4 spaces
    // when it has none yet). Columns count units when it's a tab.
    let lines = scan(text, kotlin);
    let unit = indent.map_or_else(|| indent_unit(text, &lines), str::to_owned);
    let tabs = unit == "\t";
    let step = if tabs { 1 } else { unit.len() };
    // Kotlin's official style continues by one indent; Java by 8 (two).
    let cont = if kotlin { step } else { 2 * step };
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut stack: Vec<Open> = Vec::new();
    // The last code line.
    let mut previous: Option<Previous> = None;
    let mut blank_run = 0;
    let pad = |cols: usize| if tabs { "\t".repeat(cols) } else { " ".repeat(cols) };
    for (ix, (text_line, info)) in raw.iter().zip(&lines).enumerate() {
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let starts = info.starts_in.unwrap_or(State::Code);
        let trimmed = text_line.trim();
        // Inside a raw string or a template: the text is the string's.
        if !matches!(starts, State::Code | State::Block(_)) {
            out.push((*text_line).to_owned());
            let lead = text_line.len() - text_line.trim_start().len();
            apply_brackets(&mut stack, &info.brackets, 0, ix, lead, lead, cont, step, None, text_line);
            blank_run = 0;
            continue;
        }
        if trimmed.is_empty() {
            blank_run += 1;
            if selected && blank_run > KEEP_BLANK_LINES && matches!(starts, State::Code) {
                continue;
            }
            out.push(if selected { String::new() } else { (*text_line).to_owned() });
            continue;
        }
        blank_run = 0;
        let lead = text_line.len() - text_line.trim_start().len();
        // Closers first on the line close before it's indented.
        let mut leading_closers = 0;
        for (at, c) in &info.brackets {
            let between = &text_line[lead..*at];
            if matches!(c, ')' | ']' | '}') && between.chars().all(|ch| ch.is_whitespace() || matches!(ch, ')' | ']' | '}')) {
                leading_closers += 1;
            } else {
                break;
            }
        }
        let mut closed_to = None;
        let mut closes_header = false;
        for _ in 0..leading_closers {
            if let Some(open) = stack.pop() {
                closed_to = Some(open.line_indent);
                closes_header = open.header;
            }
        }
        let mut indent = match (closed_to, stack.last()) {
            (Some(line_indent), _) => line_indent,
            (None, Some(open)) => open.content + if open.case_body { step } else { 0 },
            (None, None) => 0,
        };
        let code_start = trimmed;
        let is_code_line = matches!(starts, State::Code);
        let mut continued = Reason::None;
        let mut header = false;
        if is_code_line && closed_to.is_none() {
            // A Java `case` label sits at the switch's level; its body below.
            let label = !kotlin && (code_start.starts_with("case ") || code_start.starts_with("default:") || code_start.starts_with("default ->"));
            if label {
                if let Some(open) = stack.last_mut() {
                    if open.case_body {
                        open.case_body = false;
                        indent -= step;
                    }
                }
            }
            // A continuation: a chained call or an operator starting the
            // line, or an operator ending the line before. Kotlin's
            // official style continues by one indent, binary operators by
            // two; Java by two.
            let reason = if (code_start.starts_with('.') && !code_start.starts_with("..")) || code_start.starts_with("?.") {
                Reason::Chain
            } else if code_start.starts_with("?:") {
                Reason::Elvis
            } else if kotlin && code_start.starts_with("->") {
                // A `when` branch's arrow on its own line: one indent.
                Reason::Elvis
            } else if code_start.starts_with("&&") || code_start.starts_with("||") || !kotlin && (code_start.starts_with("? ") || code_start.starts_with(": ")) {
                Reason::Binary
            } else if kotlin && (["get(", "get() ", "set(", "private set", "protected set", "internal set"].iter().any(|a| code_start.starts_with(a)) || code_start == "get()") {
                // A Kotlin property's accessors sit under it.
                Reason::Accessor
            } else {
                match &previous {
                    Some(p) if !p.opened && p.depth == stack.len() => {
                        let prev = p.code.as_str();
                        if prev.ends_with("&&") || prev.ends_with("||") {
                            Reason::Binary
                        } else if prev.ends_with('=') && !prev.ends_with("==") || kotlin && prev.ends_with("->") {
                            Reason::Assign
                        } else if kotlin && prev.ends_with(':') && !prev.ends_with("::") || p.reason == Reason::Header && prev.ends_with(',') {
                            Reason::Header
                        } else {
                            Reason::None
                        }
                    }
                    _ => Reason::None,
                }
            };
            if reason != Reason::None {
                let size = match reason {
                    _ if !kotlin => 2 * step,
                    Reason::Binary => 2 * step,
                    _ => step,
                };
                // Under a continuation of the same statement: a sibling
                // (another `.call`, `&& b`) lines up with it; anything else
                // goes one deeper.
                indent = match &previous {
                    Some(p) if !p.opened && p.depth == stack.len() && p.reason != Reason::None && reason != Reason::Header => {
                        if p.reason == reason && reason != Reason::Assign { p.indent } else { p.indent + size }
                    }
                    _ => indent + size,
                };
            }
            continued = reason;
            // A class header's supertypes: its body belongs to the class.
            header = reason == Reason::Header;
        }
        let body: String = if is_code_line && selected { normalize(text_line, info, lead, kotlin, closes_header) } else { text_line.trim_start().to_owned() };
        let new_line = match starts {
            // A block comment's later lines: ` * …` under its `/*`.
            State::Block(_) if body.starts_with('*') => format!("{} {body}", pad(indent)),
            State::Block(_) => (*text_line).trim_end().to_owned(),
            _ => format!("{}{body}", pad(indent)),
        };
        // Trailing spaces go, but not a comment's own.
        let ends_in_comment = info.comment.last().copied().unwrap_or(false) || matches!(starts, State::Block(_)) && !text_line.contains("*/");
        let line_out = match (selected, ends_in_comment) {
            (true, false) => new_line.trim_end().to_owned(),
            (true, true) => new_line,
            (false, _) => (*text_line).to_owned(),
        };
        // The rest of the line's brackets, where they are in the new line.
        let depth = stack.len();
        if is_code_line {
            let placed = scan(&line_out, kotlin).into_iter().next().map(|l| l.brackets).unwrap_or_default();
            let lead_out = line_out.len() - line_out.trim_start().len();
            // An unselected line keeps its indent: its brackets go by it.
            let at = if selected { indent } else { lead_out };
            let base = if header { at.saturating_sub(step * if kotlin { 1 } else { 2 }) } else { at };
            let align = (!kotlin && !tabs).then(|| declaration_paren(&line_out)).flatten();
            apply_brackets(&mut stack, &placed, leading_closers, ix, at, base, cont, step, align, &line_out);
        } else {
            apply_brackets(&mut stack, &info.brackets, leading_closers, ix, indent, indent, cont, step, None, text_line);
        }
        out.push(line_out);
        if is_code_line {
            let code_text: String = text_line
                .char_indices()
                .filter(|(i, _)| !info.comment.get(*i).copied().unwrap_or(false))
                .map(|(i, c)| if info.code.get(i).copied().unwrap_or(false) { c } else { '_' })
                .collect();
            previous = Some(Previous { code: code_text.trim_end().to_owned(), opened: stack.len() > depth, depth: stack.len(), reason: continued, indent });
            // A `case …:` label: what follows is its body.
            if !kotlin && (code_start.starts_with("case ") || code_start.starts_with("default")) && code_text.trim_end().ends_with(':') {
                if let Some(open) = stack.last_mut() {
                    open.case_body = true;
                }
            }
        }
    }
    out.join("\n")
}

/// The file's indent unit: a tab, else 4 spaces unless the code's indents
/// are mostly not multiples of 4 (a 2-space file); comments' ` * ` don't
/// count.
fn indent_unit(text: &str, lines: &[Line]) -> String {
    // Code lines only: not a string's or a comment's.
    let code: Vec<&str> = text.split('\n').zip(lines).filter(|(_, l)| l.starts_in == Some(State::Code)).map(|(t, _)| t).collect();
    if code.iter().any(|l| l.starts_with('\t')) {
        return "\t".into();
    }
    let indents: Vec<usize> = code
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('*'))
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .filter(|n| *n > 0)
        .collect();
    // A 2-space file has many lines at 2, 6, 10… and next to none at odd
    // columns; a messy file has those too, and gets IntelliJ's 4.
    let odd_twos = indents.iter().filter(|n| *n % 4 == 2).count();
    let odd = indents.iter().filter(|n| *n % 2 == 1).count();
    if !indents.is_empty() && odd_twos * 5 >= indents.len() && odd * 20 <= indents.len() { "  ".into() } else { "    ".into() }
}

/// Java aligns a declaration's parameters under the first one: the column
/// after `(` when the line is a method or constructor header with a
/// parameter after it.
fn declaration_paren(line: &str) -> Option<usize> {
    use std::sync::LazyLock;
    static HEADER: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Modifiers then a name (a constructor), or a type then a name.
        regex::Regex::new(concat!(
            r"^\s*(?:@[\w.]+(?:\([^)]*\))?\s+)*",
            r"(?:(?:(?:public|private|protected|static|final|abstract|synchronized|native|default|strictfp)\s+)+(?:<[^>]+>\s+)?(?:[\w.<>\[\]?, ]+\s+)?",
            r"|(?:<[^>]+>\s+)?[\w.]+(?:<[^()]*>)?(?:\[\])*\s+)",
            r"\w+\s*$"
        ))
        .unwrap()
    });
    let open = line.find('(')?;
    let before = &line[..open];
    let after = line[open + 1..].trim();
    if after.is_empty() || before.contains('=') || ["return ", "new ", "throw ", "if ", "for ", "while ", "switch ", "catch ", "synchronized "].iter().any(|k| before.trim_start().starts_with(k) || before.contains(&format!(" {k}"))) {
        return None;
    }
    HEADER.is_match(before).then(|| line[..open + 1].chars().count())
}

/// Applies the line's brackets after the first `skip` (already closed):
/// `{` content one step past `base` (the line, or its statement when the
/// line closed a bracket opened earlier); `(` / `[` a continuation past the
/// line, or under the first parameter (`align`).
#[allow(clippy::too_many_arguments)]
fn apply_brackets(stack: &mut Vec<Open>, brackets: &[(usize, char)], skip: usize, line: usize, line_indent: usize, base: usize, cont: usize, step: usize, align: Option<usize>, text: &str) {
    let mut base = base;
    let mut first_paren = true;
    for (at, c) in brackets.iter().skip(skip) {
        match c {
            '{' => stack.push(Open { line_indent: base, content: base + step, line, case_body: false, header: false }),
            '(' | '[' => {
                let content = match align.filter(|_| *c == '(' && first_paren) {
                    Some(col) => col,
                    None => line_indent + cont,
                };
                if *c == '(' {
                    first_paren = false;
                }
                let header = *c == '(' && header_paren(text.get(..*at).unwrap_or(""));
                stack.push(Open { line_indent, content, line, case_body: false, header });
            }
            _ => {
                if let Some(open) = stack.pop() {
                    // `@NotNull Runnable r) {`: the body is the header's.
                    if open.line < line {
                        base = base.min(open.line_indent);
                    }
                }
            }
        }
    }
}

/// The line's code with IntelliJ's spacing; strings and comments as written.
fn normalize(line: &str, info: &Line, lead: usize, kotlin: bool, closes_header: bool) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().filter(|(i, _)| *i >= lead).collect();
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    let mut out = String::with_capacity(line.len() + 8);
    let mut k = 0;
    let push_space = |out: &mut String| {
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    };
    while k < chars.len() {
        let (i, c) = chars[k];
        if !is_code(i) {
            out.push(c);
            k += 1;
            continue;
        }
        let next = chars.get(k + 1).map(|(j, ch)| (*j, *ch));
        let next_code = |n: usize| chars.get(k + n).filter(|(j, _)| is_code(*j)).map(|(_, ch)| *ch);
        match c {
            ' ' | '\t' => {
                // Runs of spaces become one; none before `,` `)` `;` `.`.
                let mut m = k;
                while m < chars.len() && is_code(chars[m].0) && matches!(chars[m].1, ' ' | '\t') {
                    m += 1;
                }
                let following = chars.get(m).map(|(_, ch)| *ch);
                let following_code = chars.get(m).is_some_and(|(j, _)| is_code(*j));
                // None inside parentheses and brackets (`foo( a )` → `foo(a)`).
                let after_open = out.ends_with('(') || out.ends_with('[');
                if m < chars.len() && !(following_code && matches!(following, Some(',' | ';' | ')' | ']'))) && !(after_open && following_code) {
                    push_space(&mut out);
                }
                k = m;
                continue;
            }
            ',' => {
                out.push(',');
                if next.is_some_and(|(j, ch)| !(is_code(j) && matches!(ch, ' ' | '\t' | ')' | ']'))) {
                    out.push(' ');
                }
            }
            '{' => {
                let prev = out.chars().last();
                if prev.is_some_and(|p| p.is_alphanumeric() || p == ')' || p == '_' || (p == '>' && !out.ends_with("->"))) {
                    out.push(' ');
                }
                out.push('{');
            }
            '}' => {
                out.push('}');
                // `}else`, `}catch`, `}finally`, `}while`.
                if next.is_some_and(|(j, ch)| is_code(j) && ch.is_alphabetic()) {
                    out.push(' ');
                }
            }
            '(' => {
                // `if(` → `if (`.
                const KEYWORDS: &[&str] = &["if", "for", "while", "when", "catch", "switch", "synchronized", "try"];
                let word: String = out.chars().rev().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect::<Vec<_>>().into_iter().rev().collect();
                let before_word = out.len() - word.len();
                let boundary = out[..before_word].chars().last().is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '.'));
                if KEYWORDS.contains(&word.as_str()) && boundary && !(kotlin && word == "try") {
                    out.push(' ');
                }
                out.push('(');
            }
            ':' if kotlin && !out.ends_with(':') && !out.ends_with('?') && next.is_none_or(|(_, ch)| ch != ':') => {
                // `name: Type`, `fun f(): Int`; supertypes `A : B`, bounds.
                let typed = out.chars().last().is_some_and(|p| p.is_alphanumeric() || matches!(p, '_' | ')' | '>' | '?' | '`' | ' '));
                if !typed {
                    out.push(':');
                } else {
                    if spaced_colon(&out, closes_header) {
                        push_space(&mut out);
                    } else {
                        while out.ends_with(' ') {
                            out.pop();
                        }
                    }
                    out.push(':');
                    if next.is_some_and(|(j, ch)| !(is_code(j) && ch == ' ')) {
                        out.push(' ');
                    }
                }
            }
            '<' | '>' if comparison(&chars, k, &is_code) => {
                let op = if chars.get(k + 1).is_some_and(|(j, ch)| is_code(*j) && *ch == '=') { if c == '<' { "<=" } else { ">=" } } else if c == '<' { "<" } else { ">" };
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(' ');
                out.push_str(op);
                k += op.len();
                while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                    k += 1;
                }
                if k < chars.len() {
                    out.push(' ');
                }
                continue;
            }
            '=' | '!' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '<' | '>' => {
                let op = operator(&chars, k, &is_code);
                match op {
                    Some(op) => {
                        push_space(&mut out);
                        out.push_str(op);
                        k += op.chars().count();
                        if chars.get(k).is_some_and(|(j, ch)| !(is_code(*j) && *ch == ' ')) {
                            out.push(' ');
                        } else {
                            // Several spaces after: one.
                            while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                                k += 1;
                            }
                            if k < chars.len() {
                                out.push(' ');
                            }
                        }
                        continue;
                    }
                    None => out.push(c),
                }
            }
            _ => {
                // `else{` / `try{` handled by `{`; keywords before `(` by `(`.
                let _ = next_code;
                out.push(c);
            }
        }
        k += 1;
    }
    out
}

/// Whether a Kotlin `:` takes a space before it, as IntelliJ spaces it:
/// supertypes (`class A : B`, `object : X`, `) : Base()` after a class's
/// parameters), `constructor(…) : this(…)`, type parameter bounds
/// (`<E : Bound>`, `where T : Bound`); not a declaration's or a function's
/// type. `before`: the line's code so far.
fn spaced_colon(before: &str, closes_header: bool) -> bool {
    let trimmed = before.trim_end();
    let word_before = |text: &str| text.trim_end().rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_')).next().unwrap_or("").to_owned();
    if word_before(trimmed) == "object" {
        return true;
    }
    // Inside a type parameter list.
    let angles = trimmed.matches('<').count() as i32 - (trimmed.matches('>').count() - trimmed.matches("->").count()) as i32;
    if angles > 0 {
        return true;
    }
    if trimmed.ends_with(')') {
        // Its `(`: on this line, or on an earlier one.
        let mut depth = 0;
        for (i, ch) in trimmed.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        return header_paren(&trimmed[..i]);
                    }
                }
                _ => {}
            }
        }
        return closes_header;
    }
    let parens = trimmed.matches('(').count() as i32 - trimmed.matches(')').count() as i32;
    parens <= 0 && trimmed.split(|ch: char| !ch.is_alphanumeric()).any(|w| matches!(w, "class" | "object" | "interface" | "where"))
}

/// A `(` opening a class's or a constructor's parameters (after a
/// supertype list's `:` follows): `class A(`, `constructor(`.
fn header_paren(before: &str) -> bool {
    let words: Vec<&str> = before.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).filter(|w| !w.is_empty()).collect();
    // Not inside another call's parentheses.
    let parens = before.matches('(').count() as i32 - before.matches(')').count() as i32;
    words.last() == Some(&"constructor") || parens <= 0 && words.iter().any(|w| matches!(*w, "class" | "object" | "interface"))
}

/// A `<` / `>` (or `<=` / `>=`) comparing, not a generic's bracket: a
/// number on either side, or lower-case names (`i<n`, `a.size>0`), never
/// `List<Int>` or `listOf<String>`.
fn comparison(chars: &[(usize, char)], k: usize, is_code: &dyn Fn(usize) -> bool) -> bool {
    let c = chars[k].1;
    let next = chars.get(k + 1).filter(|(j, _)| is_code(*j)).map(|(_, ch)| *ch);
    let prev = k.checked_sub(1).and_then(|p| chars.get(p)).filter(|(j, _)| is_code(*j)).map(|(_, ch)| *ch);
    // `->`, `<<`, `>>`, `..<`, `<>`.
    if c == '>' && prev == Some('-') || next == Some(c) || prev == Some(c) || prev == Some('.') || c == '<' && next == Some('>') {
        return false;
    }
    // The words around it, skipping spaces and a following `=`.
    let mut left = String::new();
    for (j, ch) in chars[..k].iter().rev() {
        if !is_code(*j) {
            break;
        }
        if ch.is_alphanumeric() || *ch == '_' || *ch == ')' || *ch == ']' {
            left.insert(0, *ch);
        } else if *ch == ' ' && left.is_empty() {
            continue;
        } else {
            break;
        }
    }
    let mut at = k + 1;
    if chars.get(at).is_some_and(|(_, ch)| *ch == '=') {
        at += 1;
    }
    let mut right = String::new();
    for (j, ch) in &chars[at..] {
        if !is_code(*j) && right.is_empty() {
            // A string or char literal compared: `c>'a'`.
            return !left.is_empty();
        }
        if ch.is_alphanumeric() || *ch == '_' || *ch == '(' || *ch == '-' && right.is_empty() {
            right.push(*ch);
            if *ch == '(' {
                break;
            }
        } else if *ch == ' ' && right.is_empty() {
            continue;
        } else {
            break;
        }
    }
    let number = |w: &str| w.trim_start_matches('-').starts_with(|ch: char| ch.is_ascii_digit());
    let lower = |w: &str| w.starts_with(|ch: char| ch.is_lowercase() || ch == '_' || ch == '(') || w.ends_with(')') || w.ends_with(']');
    if left.is_empty() || right.is_empty() {
        return false;
    }
    number(&left) || number(&right) || lower(&left) && lower(&right) && !right.ends_with('(')
}

/// The binary operator at `k` that gets spaces around it: assignments,
/// `==` / `!=` (and `===` / `!==`), `&&`, `||`.
fn operator(chars: &[(usize, char)], k: usize, is_code: &dyn Fn(usize) -> bool) -> Option<&'static str> {
    let at = |n: usize| chars.get(k + n).filter(|(j, _)| is_code(*j)).map(|(_, c)| *c);
    let prev = k.checked_sub(1).and_then(|p| chars.get(p)).filter(|(j, _)| is_code(*j)).map(|(_, c)| *c);
    const OPS: &[&str] = &["===", "!==", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&&", "||", "="];
    for op in OPS {
        let matches = op.chars().enumerate().all(|(n, c)| at(n) == Some(c));
        if !matches {
            continue;
        }
        let after = at(op.chars().count());
        // Part of a longer operator: `<=`, `>=`, `=>`, `->`, `==` inside `===`.
        if matches!(prev, Some('=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' | '&' | '|')) || matches!(after, Some('=' | '>')) {
            return None;
        }
        return Some(op);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kt(text: &str) -> String {
        reformat(ImportStyle::Kotlin, text, None)
    }

    fn java(text: &str) -> String {
        reformat(ImportStyle::Java, text, None)
    }

    #[test]
    fn reindents_kotlin() {
        let messy = "class A {\nfun f(x:Int) {\nif(x>1){\nprintln(x)\n        }else{\n   foo(1,2)\n}\n    }\n}\n";
        assert_eq!(kt(messy), "class A {\n    fun f(x: Int) {\n        if (x > 1) {\n            println(x)\n        } else {\n            foo(1, 2)\n        }\n    }\n}\n");
    }

    #[test]
    fn lambdas_arguments_and_chains() {
        let text = "fun f() {\nitems.filter {\nit > 1\n}\n.map { x ->\nx * 2\n}\nval y = foo(\na,\nb\n)\nbar(baz {\nqux()\n})\nval z =\n1 + 2\n}";
        assert_eq!(
            kt(text),
            "fun f() {\n    items.filter {\n        it > 1\n    }\n        .map { x ->\n            x * 2\n        }\n    val y = foo(\n        a,\n        b\n    )\n    bar(baz {\n        qux()\n    })\n    val z =\n        1 + 2\n}"
        );
    }

    #[test]
    fn when_branches_and_elvis() {
        let text = "val s = when (x) {\n1 ->\n\"one\"\nelse -> \"many\"\n}\nval n = a\n?: b";
        assert_eq!(kt(text), "val s = when (x) {\n    1 ->\n        \"one\"\n    else -> \"many\"\n}\nval n = a\n    ?: b");
    }

    #[test]
    fn spacing() {
        assert_eq!(kt("val a=b==c&&d!=e"), "val a = b == c && d != e");
        assert_eq!(kt("x+=1;y-=2"), "x += 1;y -= 2");
        assert_eq!(kt("val f = { a: Int -> a >= 1 && a <= 2 }"), "val f = { a: Int -> a >= 1 && a <= 2 }");
        assert_eq!(kt("foo(a ,  b)   // keep  this"), "foo(a, b) // keep  this");
        assert_eq!(kt("val m: Map<String,Int> = mapOf(\"a,b\" to 1)"), "val m: Map<String, Int> = mapOf(\"a,b\" to 1)");
        assert_eq!(kt("list.forEach{ println(it) }"), "list.forEach { println(it) }");
        assert_eq!(kt("val ok = a === b || c !== d"), "val ok = a === b || c !== d");
        assert_eq!(kt("fun f(x: Int=1) = x"), "fun f(x: Int = 1) = x");
        assert_eq!(java("int[] a = new int[]{1,2};"), "int[] a = new int[]{1, 2};");
        assert_eq!(kt("if (i>0&&i<=n||a.size>=b) x"), "if (i > 0 && i <= n || a.size >= b) x");
        assert_eq!(kt("val l: List<Int> = listOf<String>(); val m = Map<String, List<Int>>()"), "val l: List<Int> = listOf<String>(); val m = Map<String, List<Int>>()");
        assert_eq!(kt("x shl 1; a->b; 1..<n; c>'a'"), "x shl 1; a->b; 1..<n; c > 'a'");
        assert_eq!(kt("class A(val x:Int):B(), C {"), "class A(val x: Int) : B(), C {");
        assert_eq!(kt("fun f(a:Int):String? = a?:b ?: c; val r = String::class"), "fun f(a: Int): String? = a?:b ?: c; val r = String::class");
        assert_eq!(kt("object O:Runnable { val m = mapOf(\"a:b\" to 1) }"), "object O : Runnable { val m = mapOf(\"a:b\" to 1) }");
        assert_eq!(kt("class G<E:Any, T:List<E>>: Base(), X"), "class G<E : Any, T : List<E>> : Base(), X");
        assert_eq!(kt("constructor(p: Project):this(p)"), "constructor(p: Project) : this(p)");
        assert_eq!(kt("abstract class B<E : Any>():"), "abstract class B<E : Any>() :");
        assert_eq!(kt("run(object:Runnable { })"), "run(object : Runnable { })");
        assert_eq!(kt("class A(\nval x: Int\n): Base() {\n}\nfun f(\na: Int\n) : String {\n}"), "class A(\n    val x: Int\n) : Base() {\n}\nfun f(\n    a: Int\n): String {\n}");
    }

    #[test]
    fn parentheses_and_templates() {
        assert_eq!(kt("fun a( x:Int ):Int = foo( x [ 0 ] , y )"), "fun a(x: Int): Int = foo(x [0], y)");
        assert_eq!(java("if ( x ) { f( /* c */ a ); }"), "if (x) { f( /* c */ a); }");
        assert_eq!(kt("val s = \"${a==b} ${foo( a,b )} ${map[\"k\"]} $x { y }\""), "val s = \"${a == b} ${foo(a, b)} ${map[\"k\"]} $x { y }\"");
        assert_eq!(kt("val s = \"${if (a>0) \"p\" else \"n\"}\""), "val s = \"${if (a > 0) \"p\" else \"n\"}\"");
    }

    #[test]
    fn strings_and_comments_keep_their_text() {
        let text = "fun f() {\nval s = \"a=b,c  {\"\nval t = \"\"\"\n  raw=1\n    {\n\"\"\"\n/* a=b\n   * x\n */\nval u = \"${map[\"k\"]}=\"\n}";
        assert_eq!(kt(text), "fun f() {\n    val s = \"a=b,c  {\"\n    val t = \"\"\"\n  raw=1\n    {\n\"\"\"\n    /* a=b\n     * x\n     */\n    val u = \"${map[\"k\"]}=\"\n}");
    }

    #[test]
    fn blank_lines_and_trailing_spaces() {
        assert_eq!(kt("a()   \n\n\n\n\nb()\n"), "a()\n\n\nb()\n");
    }

    #[test]
    fn java_continuations_and_switch() {
        let text = "class A {\nvoid f(int x) {\nswitch (x) {\ncase 1:\nfoo();\nbreak;\ndefault:\nbar(1,\n2);\n}\nString s = new StringBuilder()\n.append(1)\n.toString();\n}\n}";
        assert_eq!(
            java(text),
            "class A {\n    void f(int x) {\n        switch (x) {\n            case 1:\n                foo();\n                break;\n            default:\n                bar(1,\n                        2);\n        }\n        String s = new StringBuilder()\n                .append(1)\n                .toString();\n    }\n}"
        );
    }

    #[test]
    fn editorconfig_indent() {
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\nval a = 1\n}", None, Some("\t")), "fun f() {\n\tval a = 1\n}");
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\n    val a = 1\n}", None, Some("  ")), "fun f() {\n  val a = 1\n}");
    }

    #[test]
    fn only_the_selected_lines() {
        let text = "fun f() {\nval a=1\nval b=2\n}";
        assert_eq!(reformat(ImportStyle::Kotlin, text, Some(2..3)), "fun f() {\nval a=1\n    val b = 2\n}");
    }
}

#[cfg(test)]
mod corpus {
    /// Formatted sources reformat to themselves (set JUNCTION_FMT_DIR).
    #[test]
    #[ignore]
    fn formatted_sources_are_stable() {
        let Ok(dir) = std::env::var("JUNCTION_FMT_DIR") else { return };
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let text = std::fs::read_to_string(&path).unwrap();
            let style = if path.extension().is_some_and(|e| e == "kt") { super::ImportStyle::Kotlin } else { super::ImportStyle::Java };
            let out = super::reformat(style, &text, None);
            let changed: Vec<(usize, &str, &str)> = text.lines().zip(out.lines()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, b))| (i + 1, a, b)).collect();
            println!("{} {} changed", path.file_name().unwrap().to_string_lossy(), changed.len());
            for (line, a, b) in changed.iter().take(6) {
                println!("  {line}: {a:?}\n   → {b:?}");
            }
        }
    }
}

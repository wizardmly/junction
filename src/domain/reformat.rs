//! Code › Reformat Code (Ctrl+Alt+L) for Kotlin and Java with IntelliJ's
//! default code style: lines re-indented by the brackets they're in (the
//! file's indent unit, else 4 spaces; Kotlin's official style puts arguments and chained calls one
//! level deeper, Java a continuation of 8), `case` bodies under their
//! label, a space after commas, after `if` / `for` / `while`… and before
//! `{`, single spaces around assignments and comparisons, at most 2 blank
//! lines kept, no trailing spaces. Strings, comments and raw strings keep
//! their text. Gradle's Groovy scripts take the same with Groovy's
//! defaults (`reformat_groovy`).

use std::ops::Range;

use crate::domain::imports::ImportStyle;

/// Settings › Code Style › Blank Lines › "Keep maximum blank lines":
/// how many blank lines a reformat leaves in a row between members of a
/// class (or at the top level), between statements, and before a `}`.
/// IntelliJ's are 2 each, for Java, Kotlin and Groovy alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeepBlankLines {
    pub declarations: usize,
    pub code: usize,
    pub before_rbrace: usize,
}

impl Default for KeepBlankLines {
    fn default() -> Self {
        Self { declarations: 2, code: 2, before_rbrace: 2 }
    }
}

/// Settings › Code Style › Blank Lines › "Minimum blank lines": how many
/// blank lines a reformat puts between members at least. Java shows all
/// but the last; Kotlin "After class header" (before a first function or
/// property) and the last, its other minimums being fixed at 1
/// (`kotlinSpacingRules.kt`). IntelliJ's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinBlankLines {
    pub after_package: usize,
    pub before_imports: usize,
    pub after_imports: usize,
    pub around_class: usize,
    pub after_class_header: usize,
    pub around_field_in_interface: usize,
    pub around_field: usize,
    pub around_method_in_interface: usize,
    pub around_method: usize,
    pub around_initializer: usize,
    pub before_class_end: usize,
    /// Kotlin's "Before declaration with comment or annotation".
    pub before_commented_declaration: usize,
}

impl Default for MinBlankLines {
    fn default() -> Self {
        Self::from_values([1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 0, 1])
    }
}

impl MinBlankLines {
    /// The settings' labels, in `values` order.
    pub const LABELS: [&'static str; 12] = [
        "After package:",
        "Before imports:",
        "After imports:",
        "Around class:",
        "After class header:",
        "Around field in interface:",
        "Around field:",
        "Around method in interface:",
        "Around method:",
        "Around initializer:",
        "Before class end:",
        "Before declaration with comment or annotation:",
    ];
    /// Which of `values` each language's page shows.
    pub const JAVA: [usize; 11] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    pub const KOTLIN: [usize; 2] = [4, 11];

    pub fn values(&self) -> [usize; 12] {
        [
            self.after_package,
            self.before_imports,
            self.after_imports,
            self.around_class,
            self.after_class_header,
            self.around_field_in_interface,
            self.around_field,
            self.around_method_in_interface,
            self.around_method,
            self.around_initializer,
            self.before_class_end,
            self.before_commented_declaration,
        ]
    }

    pub fn from_values(v: [usize; 12]) -> Self {
        let [after_package, before_imports, after_imports, around_class, after_class_header, around_field_in_interface, around_field, around_method_in_interface, around_method, around_initializer, before_class_end, before_commented_declaration] = v;
        Self { after_package, before_imports, after_imports, around_class, after_class_header, around_field_in_interface, around_field, around_method_in_interface, around_method, around_initializer, before_class_end, before_commented_declaration }
    }
}

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

/// Which bytes of a Java file are code (not in a string, char or comment).
pub(crate) fn java_code_mask(text: &str) -> Vec<bool> {
    let mut mask = Vec::with_capacity(text.len());
    for (ix, line) in scan(text, false).into_iter().enumerate() {
        if ix > 0 {
            mask.push(false);
        }
        mask.extend(line.code);
    }
    mask
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
    /// A line after one ending with an arithmetic operator: `"a" +`.
    Operator,
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
    /// A `{`, not a `(` or `[`.
    brace: bool,
    /// A class's (or object's, enum's…) body: its blank lines are in
    /// declarations.
    declarations: bool,
}

/// The language a reformat follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flavor {
    Java,
    Kotlin,
    /// Groovy (Gradle scripts): Java's indents and continuations, Kotlin's
    /// `${…}` templates and `name: value` colons (map entries, named
    /// arguments).
    Groovy,
}

/// What a reformat indents with (Settings › Code Style, `.editorconfig`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndentStyle {
    /// The indent unit; `None`: the file's own.
    pub unit: Option<String>,
    /// The unit for a file without one of its own (4 spaces when `None`).
    pub fallback: Option<String>,
    /// The continuation indent and the indent it goes with, in columns:
    /// a continuation is that many indents (`None`: the language's).
    pub continuation: Option<(usize, usize)>,
    /// Where long lines wrap (`None`: 120).
    pub right_margin: Option<usize>,
    pub keep_blank: KeepBlankLines,
    /// Java's and Kotlin's (Groovy keeps its blank lines as they are).
    pub min_blank: MinBlankLines,
}

/// The reformatted text. `only`: the 0-based lines to change (a
/// selection); the others keep their text.
#[cfg(test)]
pub fn reformat(style: ImportStyle, text: &str, only: Option<Range<usize>>) -> String {
    reformat_indented(style, text, only, &IndentStyle::default())
}

/// `reformat` with the code style's indents.
pub fn reformat_indented(style: ImportStyle, text: &str, only: Option<Range<usize>>, indents: &IndentStyle) -> String {
    reformat_with(if style == ImportStyle::Kotlin { Flavor::Kotlin } else { Flavor::Java }, text, only, indents)
}

/// A Gradle Groovy script (`build.gradle`, `settings.gradle`) reformatted
/// with IntelliJ's Groovy defaults; `indents` as in `reformat_indented`.
pub fn reformat_groovy(text: &str, only: Option<Range<usize>>, indents: &IndentStyle) -> String {
    reformat_with(Flavor::Groovy, text, only, indents)
}

fn reformat_with(flavor: Flavor, text: &str, only: Option<Range<usize>>, indents: &IndentStyle) -> String {
    let kotlin = flavor == Flavor::Kotlin;
    // Kotlin's and Groovy's strings and colons.
    let templates = flavor != Flavor::Java;
    // The project's code style is the file's: its indent unit (4 spaces
    // when it has none yet). Columns count units when it's a tab.
    let lines = scan(text, templates);
    let unit = indents.unit.clone().unwrap_or_else(|| indent_unit(text, &lines, indents.fallback.as_deref()));
    let tabs = unit == "\t";
    let step = if tabs { 1 } else { unit.len() };
    // Kotlin's official style continues by one indent; Java by 8 (two);
    // the code style's in proportion to the unit.
    let cont = match indents.continuation {
        Some((continuation, indent)) if indent > 0 => ((continuation * step) as f32 / indent as f32).round() as usize,
        _ if kotlin => step,
        _ => 2 * step,
    };
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    // Per line out: code the selection reformats, which may be chopped.
    let mut choppable: Vec<bool> = Vec::with_capacity(raw.len());
    let mut stack: Vec<Open> = Vec::new();
    // The last code line.
    let mut previous: Option<Previous> = None;
    let mut blank_run = 0;
    let bodies = declaration_braces(&raw, &lines);
    let keep = indents.keep_blank;
    let need = if flavor == Flavor::Groovy { vec![0; raw.len()] } else { minimum_blank_lines(&raw, &lines, &bodies, kotlin, &indents.min_blank) };
    // Blank lines just put out.
    let mut kept = 0;
    let pad = |cols: usize| if tabs { "\t".repeat(cols) } else { " ".repeat(cols) };
    for (ix, (text_line, info)) in raw.iter().zip(&lines).enumerate() {
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let starts = info.starts_in.unwrap_or(State::Code);
        let trimmed = text_line.trim();
        // Inside a raw string or a template: the text is the string's.
        if !matches!(starts, State::Code | State::Block(_)) {
            out.push((*text_line).to_owned());
            choppable.push(false);
            let lead = text_line.len() - text_line.trim_start().len();
            apply_brackets(&mut stack, &info.brackets, 0, ix, lead, lead, cont, step, None, text_line, &bodies[ix]);
            blank_run = 0;
            kept = 0;
            continue;
        }
        if trimmed.is_empty() {
            blank_run += 1;
            if selected && matches!(starts, State::Code) {
                // Before a `}`, in a class body (or the file), or in code.
                let next = raw.iter().zip(&lines).skip(ix + 1).find(|(l, _)| !l.trim().is_empty());
                let limit = if next.is_some_and(|(l, i)| l.trim_start().starts_with('}') && i.starts_in == Some(State::Code)) {
                    keep.before_rbrace
                } else if stack.last().is_none_or(|open| open.declarations) {
                    keep.declarations
                } else {
                    keep.code
                };
                if blank_run > limit {
                    continue;
                }
            }
            out.push(if selected { String::new() } else { (*text_line).to_owned() });
            choppable.push(false);
            kept += 1;
            continue;
        }
        blank_run = 0;
        // Blank Lines › Minimum blank lines before a member.
        if selected && !out.is_empty() {
            for _ in kept..need[ix] {
                out.push(String::new());
                choppable.push(false);
            }
        }
        kept = 0;
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
                        } else if ends_with_operator(prev) {
                            Reason::Operator
                        } else if kotlin && prev.ends_with(':') && !prev.ends_with("::") || p.reason == Reason::Header && prev.ends_with(',') {
                            Reason::Header
                        } else if flavor == Flavor::Groovy && prev.ends_with(',') && stack.last().is_none_or(|o| o.brace) {
                            // A command's arguments without parentheses:
                            // `proguardFiles a,` then `b`.
                            Reason::Binary
                        } else {
                            Reason::None
                        }
                    }
                    _ => Reason::None,
                }
            };
            if reason != Reason::None {
                let size = match reason {
                    _ if !kotlin => cont,
                    Reason::Binary => 2 * cont,
                    _ => cont,
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
        let body: String = if is_code_line && selected { normalize(text_line, info, lead, flavor, closes_header) } else { text_line.trim_start().to_owned() };
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
            let placed = scan(&line_out, templates).into_iter().next().map(|l| l.brackets).unwrap_or_default();
            let lead_out = line_out.len() - line_out.trim_start().len();
            // An unselected line keeps its indent: its brackets go by it.
            let at = if selected { indent } else { lead_out };
            let base = if header { at.saturating_sub(cont) } else { at };
            let align = (flavor == Flavor::Java && !tabs).then(|| declaration_paren(&line_out)).flatten();
            apply_brackets(&mut stack, &placed, leading_closers, ix, at, base, cont, step, align, &line_out, &bodies[ix]);
        } else {
            apply_brackets(&mut stack, &info.brackets, leading_closers, ix, indent, indent, cont, step, None, text_line, &bodies[ix]);
        }
        out.push(line_out);
        choppable.push(selected && is_code_line);
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
    if kotlin {
        out = chop_long_lines(out, choppable, &unit, indents.right_margin.unwrap_or(RIGHT_MARGIN));
    }
    out.join("\n")
}

/// IntelliJ's default right margin (Code Style › General › Hard wrap at).
pub const RIGHT_MARGIN: usize = 120;

/// Kotlin's official style chops a list of arguments or parameters down
/// when it runs past the right margin (CALL_PARAMETERS_WRAP and
/// METHOD_PARAMETERS_WRAP "chop down if long", `(` and `)` on their own
/// lines, no trailing comma): the outermost such list first, then each
/// line it makes that is still long.
fn chop_long_lines(lines: Vec<String>, choppable: Vec<bool>, unit: &str, margin: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut todo: Vec<(String, bool)> = lines.into_iter().zip(choppable).rev().collect();
    while let Some((line, choppable)) = todo.pop() {
        match choppable.then(|| chop(&line, unit, margin)).flatten() {
            Some(parts) => todo.extend(parts.into_iter().rev().map(|p| (p, true))),
            None => out.push(line),
        }
    }
    out
}

/// Columns, a tab as 4.
fn width(text: &str) -> usize {
    text.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// A code line split at its outermost argument or parameter list whose
/// last item ends past the margin; `None` when there is none.
fn chop(line: &str, unit: &str, margin: usize) -> Option<Vec<String>> {
    if width(line) <= margin {
        return None;
    }
    let info = scan(line, true).into_iter().next()?;
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    // (depth, open, close) of the list to chop.
    let mut best: Option<(usize, usize, usize)> = None;
    let mut stack: Vec<(usize, char)> = Vec::new();
    for &(at, c) in &info.brackets {
        if matches!(c, '(' | '[' | '{') {
            stack.push((at, c));
            continue;
        }
        let Some((open, opener)) = stack.pop() else { continue };
        let inner = &line[open + 1..at];
        let past = width(&line[..at]) > margin;
        let commented = info.comment[open..at].iter().any(|c| *c);
        if opener == '(' && c == ')' && past && !inner.trim().is_empty() && !commented && is_call_or_declaration(&line[..open]) {
            let depth = stack.len();
            if best.is_none_or(|(d, o, _)| depth < d || depth == d && open < o) {
                best = Some((depth, open, at));
            }
        }
    }
    // A declaration whose parameters fit but not what follows them
    // (`): Result<String> {`): its parameters.
    let declaration = || {
        let (open, close) = paren_pair(&info.brackets)?;
        let head = &line[..open];
        let declares = head.split_whitespace().any(|w| matches!(w, "fun" | "class" | "constructor" | "interface") || w.starts_with("fun<"));
        (declares && !line[open + 1..close].trim().is_empty() && width(head) < margin).then_some((0, open, close))
    };
    let Some((_, open, close)) = best.or_else(declaration) else {
        return chain_break(line, &info, unit, margin);
    };
    // The items: commas outside nested brackets, strings and `<…>`.
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut angle = 0usize;
    let mut start = open + 1;
    for (i, c) in line[open + 1..close].char_indices().map(|(i, c)| (i + open + 1, c)) {
        if !is_code(i) {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '<' if line[..i].ends_with(|p: char| p.is_alphanumeric() || p == '_') => angle += 1,
            '>' if angle > 0 && !line[..i].ends_with('-') => angle -= 1,
            ',' if depth == 0 && angle == 0 => {
                items.push(line[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(line[start..close].trim());
    let indent = &line[..line.len() - line.trim_start().len()];
    let mut parts = vec![line[..=open].trim_end().to_owned()];
    let last = items.len() - 1;
    for (n, item) in items.into_iter().enumerate() {
        parts.push(format!("{indent}{unit}{item}{}", if n < last { "," } else { "" }));
    }
    parts.push(format!("{indent}{}", &line[close..]));
    Some(parts)
}

/// METHOD_CALL_CHAIN_WRAP "wrap if long": the line broken before its last
/// chained call (`.name(` or `?.name {`, outside brackets) that starts
/// within the margin, the call an indent deeper.
fn chain_break(line: &str, info: &Line, unit: &str, margin: usize) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let mut depth = 0usize;
    let mut cut = None;
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '.' if depth == 0 && i > lead && width(&line[..i]) < margin => {
                let at = if line[..i].ends_with('?') { i - 1 } else { i };
                let after_receiver = line[..at].ends_with(|p: char| p.is_alphanumeric() || p == '_' || p == ')' || p == '}');
                let name: String = line[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let calls = !name.is_empty() && line[i + 1 + name.len()..].trim_start().starts_with(['(', '{']);
                if after_receiver && calls && at > lead {
                    cut = Some(at);
                }
            }
            _ => {}
        }
    }
    let Some(cut) = cut else { return assignment_break(line, info, unit, margin) };
    let indent = &line[..lead];
    Some(vec![line[..cut].trim_end().to_owned(), format!("{indent}{unit}{}", &line[cut..])])
}

/// ASSIGNMENT_WRAP and WRAP_EXPRESSION_BODY_FUNCTIONS "wrap if long": the
/// line broken after its first `=` outside brackets, the value an indent
/// deeper, when the value then fits.
fn assignment_break(line: &str, info: &Line, unit: &str, margin: usize) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let bytes = line.as_bytes();
    let mut depth = 0usize;
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => {
                let before = if i > 0 { bytes[i - 1] } else { b' ' };
                let after = bytes.get(i + 1).copied().unwrap_or(b' ');
                // `==`, `!=`, `<=`, `>=`, `=>`, `+=` and the like stay.
                if matches!(before, b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%') || matches!(after, b'=' | b'>') {
                    return None;
                }
                let value = line[i + 1..].trim();
                let wrapped = format!("{}{unit}{value}", &line[..lead]);
                // Only where it helps: a long string stays on its line.
                if value.is_empty() || width(&wrapped) > margin {
                    return None;
                }
                return Some(vec![line[..=i].to_owned(), wrapped]);
            }
            _ => {}
        }
    }
    None
}

/// The line's first `(` and its `)`, when they pair on the line.
fn paren_pair(brackets: &[(usize, char)]) -> Option<(usize, usize)> {
    let first = brackets.iter().position(|(_, c)| *c == '(')?;
    let mut depth = 0;
    for &(at, c) in &brackets[first..] {
        match c {
            '(' | '[' | '{' => depth += 1,
            _ => {
                depth -= 1;
                if depth == 0 {
                    return (c == ')').then_some((brackets[first].0, at));
                }
            }
        }
    }
    None
}

/// Whether a `(` after this text opens a call's arguments or a
/// declaration's parameters: after a name, `>` or `)`, not a keyword's
/// condition.
fn is_call_or_declaration(before: &str) -> bool {
    let word = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
    let ends_name = before.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == ')') || before.ends_with('>') && !before.ends_with("->");
    ends_name && !matches!(word, "if" | "when" | "for" | "while" | "catch" | "return" | "in" | "is" | "else" | "throw")
}

/// The file's indent unit: a tab, else 4 spaces unless the code's indents
/// are mostly not multiples of 4 (a 2-space file); comments' ` * ` don't
/// count.
fn indent_unit(text: &str, lines: &[Line], fallback: Option<&str>) -> String {
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
    if !indents.is_empty() && odd_twos * 5 >= indents.len() && odd * 20 <= indents.len() { "  ".into() } else { fallback.unwrap_or("    ").into() }
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

/// A member of a class body or of the file, for its minimum blank lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Member {
    Package,
    Imports,
    /// Kotlin's `@file:` annotations.
    FileAnnotations,
    Class,
    /// Kotlin's `object` (and `companion object`).
    Object,
    Fun,
    Property,
    Init,
    Constructor,
    TypeAlias,
    /// Java's fields and enum constants.
    Field,
    /// Java's methods and constructors.
    Method,
    /// Java's `{ … }` and `static { … }`.
    Initializer,
    /// Anything else (a script's statement, Kotlin's enum entries).
    Other,
}

/// Kotlin's and Java's modifiers before a declaration's keyword.
const MODIFIERS: [&str; 37] = [
    "public", "private", "protected", "internal", "open", "final", "abstract", "override", "sealed", "data", "inner", "enum", "annotation", "companion", "inline", "value", "const",
    "lateinit", "suspend", "operator", "infix", "tailrec", "external", "expect", "actual", "static", "synchronized", "native", "transient", "volatile", "strictfp", "default",
    "non-sealed", "noinline", "crossinline", "reified", "vararg",
];

/// The words of a line of code after its annotations (`@Foo`, `@Foo(…)`)
/// and modifiers.
fn declaration_words(code: &str) -> Vec<&str> {
    let mut rest = code.trim_start();
    // Annotations on the line: `@Inject`, `@field:Rule`, `@Suppress("x")`.
    while let Some(after) = rest.strip_prefix('@') {
        if after.starts_with("interface") {
            break;
        }
        let name_end = after.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':'))).unwrap_or(after.len());
        let mut tail = &after[name_end..];
        if tail.starts_with('(') {
            let mut depth = 0;
            let close = tail.char_indices().find(|(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            });
            tail = close.map(|(i, _)| &tail[i + 1..]).unwrap_or("");
        }
        rest = tail.trim_start();
    }
    rest.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '@'))).filter(|w| !w.is_empty()).skip_while(|w| MODIFIERS.contains(w)).take(2).collect()
}

/// What a line of a class body (or the file) declares; `None`: not a
/// declaration's first line (Kotlin's `get() = …`, `: Base()`, `where …`).
fn member_kind(code: &str, kotlin: bool) -> Option<Member> {
    let code = code.trim();
    let first = code.chars().next()?;
    if code.starts_with("@file:") {
        return Some(Member::FileAnnotations);
    }
    if !(first.is_alphanumeric() || matches!(first, '_' | '@' | '`')) && !(first == '{' && !kotlin) {
        return None;
    }
    let words = declaration_words(code);
    let word = words.first().copied().unwrap_or("");
    let kind = match word {
        "package" => Member::Package,
        "import" => Member::Imports,
        "class" | "interface" | "@interface" | "record" => Member::Class,
        "enum" if !kotlin => Member::Class,
        "fun" if words.get(1) == Some(&"interface") => Member::Class,
        "object" if kotlin => Member::Object,
        "fun" if kotlin => Member::Fun,
        "val" | "var" if kotlin => Member::Property,
        "init" if kotlin => Member::Init,
        "constructor" if kotlin => Member::Constructor,
        "typealias" if kotlin => Member::TypeAlias,
        "get" | "set" | "where" | "by" | "else" | "catch" | "finally" if kotlin => return None,
        _ if kotlin => Member::Other,
        _ if code.trim_start_matches("static").trim_start().starts_with('{') => Member::Initializer,
        _ => match (code.find('('), code.find('=')) {
            (Some(paren), eq) if eq.is_none_or(|eq| paren < eq) && !code.trim_end().ends_with(',') => Member::Method,
            _ => Member::Field,
        },
    };
    Some(kind)
}

/// The blank lines Java's code style puts between two members at least.
fn java_blank_lines(prev: Member, next: Member, interface: bool, min: &MinBlankLines) -> usize {
    let around = |m: Member| match m {
        Member::Class => min.around_class,
        Member::Method if interface => min.around_method_in_interface,
        Member::Method => min.around_method,
        Member::Field if interface => min.around_field_in_interface,
        Member::Field => min.around_field,
        Member::Initializer => min.around_initializer,
        _ => 0,
    };
    match (prev, next) {
        (Member::Package, _) => min.after_package.max(if next == Member::Imports { min.before_imports } else { 0 }),
        (_, Member::Imports) => min.before_imports,
        (Member::Imports, _) => min.after_imports,
        _ => around(prev).max(around(next)),
    }
}

/// Kotlin's (`kotlinSpacingRules.kt`): one after the package, imports and
/// `@file:` annotations; before a declaration with a comment or annotation
/// on its own line, the setting; between functions, classes, objects (and
/// a property and a function) one when the first takes more than a line;
/// none between properties; one otherwise.
fn kotlin_blank_lines(prev: Member, prev_multiline: bool, next: Member, commented: bool, min: &MinBlankLines) -> usize {
    use Member::*;
    if matches!(prev, Package | Imports | FileAnnotations) {
        return 1;
    }
    let declaration = |m: Member| matches!(m, Class | Object | Fun | Property | Init | Constructor | TypeAlias);
    if !declaration(prev) || !declaration(next) {
        return 0;
    }
    if commented {
        return min.before_commented_declaration;
    }
    match (prev, next) {
        (Class | Object, Class | Object) | (Fun, Fun | Property | Class) | (Property, Fun) | (Constructor, Constructor) | (TypeAlias, TypeAlias) => usize::from(prev_multiline),
        (Property, Property) => 0,
        _ => 1,
    }
}

/// A class body (or the file) for `minimum_blank_lines`.
struct Members {
    /// The bracket depth its members are at.
    depth: usize,
    class_body: bool,
    interface: bool,
    /// The last member: what it is, its declaration's line, whether it's an interface.
    last: Option<(Member, usize, bool)>,
    /// The last line of the last member.
    end: usize,
    /// Java: the last member's `;` or `}` hasn't come yet.
    open: bool,
    /// A comment or annotation on its own line: the next member's start.
    pending: Option<usize>,
}

/// Per line: the blank lines the code style wants before it at least,
/// for a member (from its comments and annotations) of a class body or
/// the file, and Java's `}` of a class.
fn minimum_blank_lines(raw: &[&str], lines: &[Line], bodies: &[Vec<bool>], kotlin: bool, min: &MinBlankLines) -> Vec<usize> {
    let mut need = vec![0; raw.len()];
    // Open brackets: whether each is a class body.
    let mut brackets: Vec<bool> = Vec::new();
    let mut scopes = vec![Members { depth: 0, class_body: false, interface: false, last: None, end: 0, open: false, pending: None }];
    let mut previous_code = String::new();
    for (ix, (text, info)) in raw.iter().zip(lines).enumerate() {
        let in_code = info.starts_in.unwrap_or(State::Code) == State::Code;
        // Strings as `_`, comments gone.
        let code: String = text
            .char_indices()
            .map(|(i, c)| match (info.code.get(i).copied().unwrap_or(false), info.comment.get(i).copied().unwrap_or(false)) {
                (true, _) => c,
                (false, true) => ' ',
                (false, false) => '_',
            })
            .collect();
        let code = code.trim();
        let blank = text.trim().is_empty();
        let depth = brackets.len();
        let mut pending_line = false;
        let scope = scopes.last_mut().expect("the file's scope stays");
        if in_code && depth == scope.depth {
            if blank {
                scope.pending = None;
            } else if code.starts_with('}') {
                if !kotlin && scope.class_body && scope.last.is_some() {
                    need[ix] = min.before_class_end;
                }
                scope.pending = None;
            } else if code.is_empty() || (code.starts_with('@') && !code.starts_with("@file:") && !code.starts_with("@interface") && declaration_words(code).is_empty()) {
                // A comment or an annotation on its own line.
                if kotlin || !scope.open {
                    scope.pending.get_or_insert(ix);
                    pending_line = true;
                }
            } else {
                let kind = member_kind(code, kotlin);
                let continues = previous_code.ends_with("->") || previous_code.ends_with(|c: char| matches!(c, '=' | ',' | '(' | '.' | ':' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '?' | '!'));
                let starts = match kind {
                    None => false,
                    Some(_) if kotlin => !continues,
                    Some(_) => !scope.open,
                };
                let kind = kind.unwrap_or(Member::Other);
                // Imports (and `@file:` annotations) are one list.
                let merged = scope.last.is_some_and(|(k, _, _)| k == kind && matches!(kind, Member::Imports | Member::FileAnnotations));
                if starts && !merged {
                    let start = scope.pending.take().unwrap_or(ix);
                    let n = match scope.last {
                        Some((prev, line, _)) if kotlin => kotlin_blank_lines(prev, scope.end > line, kind, start < ix, min),
                        Some((prev, _, _)) => java_blank_lines(prev, kind, scope.interface, min),
                        None if scope.class_body && (!kotlin || matches!(kind, Member::Fun | Member::Property)) => min.after_class_header,
                        None => 0,
                    };
                    need[start] = need[start].max(n);
                    let interface = kind == Member::Class && declaration_words(code).iter().any(|w| *w == "interface" || *w == "@interface");
                    scope.last = Some((kind, ix, interface));
                }
                scope.pending = None;
                scope.open = true;
            }
        }
        if !blank {
            let innermost = scopes.len() - 1;
            for (i, scope) in scopes.iter_mut().enumerate() {
                if i < innermost || !pending_line {
                    scope.end = ix;
                }
            }
        }
        let mut body = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => {
                    let class_body = body.next().copied().unwrap_or(false);
                    brackets.push(class_body);
                    if class_body {
                        let interface = scopes.last().and_then(|s| s.last).is_some_and(|(_, _, interface)| interface);
                        scopes.push(Members { depth: brackets.len(), class_body: true, interface, last: None, end: ix, open: false, pending: None });
                    }
                }
                '(' | '[' => brackets.push(false),
                _ => {
                    if brackets.pop() == Some(true) && scopes.len() > 1 {
                        scopes.pop();
                    }
                }
            }
        }
        if in_code && !code.is_empty() {
            previous_code = code.to_owned();
            // Java: a member ends at its `;` or `}`.
            if let Some(scope) = scopes.last_mut() {
                if !kotlin && scope.depth == brackets.len() && (code.ends_with(';') || code.ends_with('}')) {
                    scope.open = false;
                }
            }
        }
    }
    need
}

/// Per line, for each of its `{` in order: whether it opens a class body
/// (`class`, `interface`, `object`, `enum`, `record` or Java's `new X() {`
/// since the last statement), where blank lines are between declarations.
fn declaration_braces(raw: &[&str], lines: &[Line]) -> Vec<Vec<bool>> {
    const KEYWORDS: [&str; 6] = ["class", "interface", "object", "enum", "record", "new"];
    let mut out = Vec::with_capacity(lines.len());
    // The bracket depth a keyword was seen at, until its body opens.
    let mut pending: Option<usize> = None;
    let mut depth = 0usize;
    for (text, info) in raw.iter().zip(lines) {
        let mut braces = Vec::new();
        let bytes = text.as_bytes();
        let mut word_start = None;
        for i in 0..=bytes.len() {
            let b = bytes.get(i).copied().unwrap_or(b' ');
            let code = info.code.get(i).copied().unwrap_or(false);
            let wordy = code && (b.is_ascii_alphanumeric() || b == b'_');
            if wordy && word_start.is_none() {
                word_start = Some(i);
            }
            if !wordy {
                if let Some(start) = word_start.take() {
                    let word = &text[start..i];
                    // `X.class`, `foo.new`: not declarations.
                    let qualified = start > 0 && bytes[start - 1] == b'.';
                    if !qualified && KEYWORDS.contains(&word) {
                        pending = Some(depth);
                    } else if !qualified && pending == Some(depth) && matches!(word, "fun" | "val" | "var" | "return" | "if" | "when" | "for" | "while") {
                        pending = None;
                    }
                }
            }
            if !code {
                continue;
            }
            match b {
                b'(' | b'[' => depth += 1,
                b')' | b']' => {
                    depth = depth.saturating_sub(1);
                    if pending.is_some_and(|d| d > depth) {
                        pending = None;
                    }
                }
                b'{' => {
                    let body = pending == Some(depth);
                    if body {
                        pending = None;
                    }
                    braces.push(body);
                }
                b'}' | b';' | b'=' if pending == Some(depth) => pending = None,
                _ => {}
            }
        }
        out.push(braces);
    }
    out
}

/// Applies the line's brackets after the first `skip` (already closed):
/// `{` content one step past `base` (the line, or its statement when the
/// line closed a bracket opened earlier); `(` / `[` a continuation past the
/// line, or under the first parameter (`align`).
#[allow(clippy::too_many_arguments)]
fn apply_brackets(stack: &mut Vec<Open>, brackets: &[(usize, char)], skip: usize, line: usize, line_indent: usize, base: usize, cont: usize, step: usize, align: Option<usize>, text: &str, bodies: &[bool]) {
    let mut base = base;
    let mut first_paren = true;
    let mut bodies = bodies.iter();
    for (at, c) in brackets.iter().skip(skip) {
        match c {
            '{' => stack.push(Open { line_indent: base, content: base + step, line, case_body: false, header: false, brace: true, declarations: bodies.next().copied().unwrap_or(false) }),
            '(' | '[' => {
                let content = match align.filter(|_| *c == '(' && first_paren) {
                    Some(col) => col,
                    None => line_indent + cont,
                };
                if *c == '(' {
                    first_paren = false;
                }
                let header = *c == '(' && header_paren(text.get(..*at).unwrap_or(""));
                stack.push(Open { line_indent, content, line, case_body: false, header, brace: false, declarations: false });
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
fn normalize(line: &str, info: &Line, lead: usize, flavor: Flavor, closes_header: bool) -> String {
    let kotlin = flavor == Flavor::Kotlin;
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
            // Groovy's `a ? b : c`: spaced as an operator.
            ':' if flavor == Flavor::Groovy && out.contains(" ? ") => {
                push_space(&mut out);
                out.push(':');
                if next.is_some_and(|(j, ch)| !(is_code(j) && ch == ' ')) {
                    out.push(' ');
                }
            }
            ':' if flavor != Flavor::Java && !out.ends_with(':') && !out.ends_with('?') && next.is_none_or(|(_, ch)| ch != ':') => {
                // `name: Type`, `fun f(): Int`; supertypes `A : B`, bounds.
                // An annotation's use-site target: `@get:Rule`.
                let word = out.trim_end_matches(|p: char| p.is_alphanumeric() || p == '_');
                let use_site = word.ends_with('@') && word.len() < out.len();
                let typed = !use_site && out.chars().last().is_some_and(|p| p.is_alphanumeric() || matches!(p, '_' | ')' | '>' | '?' | '`' | ' '));
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
/// A line ending with a binary `+ - * / %`, not `++`, `--`, `->`, an
/// `import a.*` or a comment's end.
fn ends_with_operator(line: &str) -> bool {
    let Some(last) = line.chars().last() else { return false };
    let before = line[..line.len() - 1].chars().last();
    match last {
        '+' => before != Some('+'),
        '-' => before != Some('-'),
        '*' => !matches!(before, Some('.' | '/')),
        '/' => before != Some('*') && before != Some('/'),
        '%' => true,
        _ => false,
    }
}

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
    // A declaration's type parameters: `fun <reified T>`, `class <T>`.
    if left.is_empty() || right.is_empty() || matches!(left.as_str(), "fun" | "class" | "interface" | "typealias" | "val" | "var") {
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
    fn gradle_groovy() {
        let text = "plugins {\nid 'com.android.application'\n}\nandroid {\ncompileSdk=34\ndefaultConfig{\nminSdk 24\nversionName \"${major}.0\"\n}\n}\ndependencies {\nimplementation fileTree(dir:'libs',include:['*.jar'])\nimplementation 'a:b:1'\n}\ntask clean(type:Delete){\ndelete rootProject.buildDir\n}\ndef v = a ? 'x' :'y'";
        assert_eq!(
            reformat_groovy(text, None, &IndentStyle::default()),
            "plugins {\n    id 'com.android.application'\n}\nandroid {\n    compileSdk = 34\n    defaultConfig {\n        minSdk 24\n        versionName \"${major}.0\"\n    }\n}\ndependencies {\n    implementation fileTree(dir: 'libs', include: ['*.jar'])\n    implementation 'a:b:1'\n}\ntask clean(type: Delete) {\n    delete rootProject.buildDir\n}\ndef v = a ? 'x' : 'y'"
        );
        assert_eq!(reformat_groovy("release {\nproguardFiles a,\n'b',\n'c'\nminify true\n}", None, &IndentStyle::default()), "release {\n    proguardFiles a,\n            'b',\n            'c'\n    minify true\n}");
        // Java's continuation for an argument list.
        assert_eq!(reformat_groovy("foo(\na,\nb)", None, &IndentStyle::default()), "foo(\n        a,\n        b)");
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
        assert_eq!(kt("class A(\nval x: Int\n): Base() {\n}\nfun f(\na: Int\n) : String {\n}"), "class A(\n    val x: Int\n) : Base() {\n}\n\nfun f(\n    a: Int\n): String {\n}");
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
    fn keeps_the_code_styles_blank_lines() {
        let keep = |declarations, code, before_rbrace| IndentStyle { keep_blank: KeepBlankLines { declarations, code, before_rbrace }, ..Default::default() };
        let kotlin = "class A {\n\n\n    val a = 1\n\n\n    fun f() {\n        g()\n\n\n        h()\n\n\n    }\n\n\n}\n\n\nfun top() {}\n";
        // Declarations keep 1, code 0, nothing before `}`.
        assert_eq!(
            reformat_indented(ImportStyle::Kotlin, kotlin, None, &keep(1, 0, 0)),
            "class A {\n\n    val a = 1\n\n    fun f() {\n        g()\n        h()\n    }\n}\n\nfun top() {}\n"
        );
        // The defaults keep 2 everywhere.
        assert_eq!(reformat(ImportStyle::Kotlin, kotlin, None), kotlin);
        // An object expression's and an anonymous class's bodies are declarations; a lambda's is code.
        let java = "class A {\n    void f() {\n        run(new Runnable() {\n\n\n            void x() {}\n        });\n        list.forEach(x -> {\n\n\n            g(x);\n        });\n    }\n}";
        assert_eq!(
            reformat_indented(ImportStyle::Java, java, None, &keep(2, 0, 2)),
            "class A {\n    void f() {\n        run(new Runnable() {\n\n\n            void x() {}\n        });\n        list.forEach(x -> {\n            g(x);\n        });\n    }\n}"
        );
        let object = "val r = object : Runnable {\n\n    override fun run() {}\n}\nfun f() {\n    val c = A::class.java\n\n    g()\n}";
        assert_eq!(reformat_indented(ImportStyle::Kotlin, object, None, &keep(1, 0, 2)), "val r = object : Runnable {\n\n    override fun run() {}\n}\n\nfun f() {\n    val c = A::class.java\n    g()\n}");
    }

    #[test]
    fn puts_the_minimum_blank_lines_between_members() {
        // Java: after the package and imports, around classes, methods and
        // initializers (with their comments and annotations); fields together.
        let members = "package a;\nimport b.C;\nimport b.D;\nclass A {\n    int x;\n    int y = f(\n        1);\n    /** Doc. */\n    @Override\n    void f() {\n    }\n    void g() {}\n    static {\n    }\n    interface I {\n        int Z = 1;\n        void h();\n        void i();\n    }\n    enum E {\n        A, B;\n        void j() {}\n    }\n}\nclass B {}";
        assert_eq!(
            java(members),
            "package a;\n\nimport b.C;\nimport b.D;\n\nclass A {\n    int x;\n    int y = f(\n            1);\n\n    /** Doc. */\n    @Override\n    void f() {\n    }\n\n    void g() {}\n\n    static {\n    }\n\n    interface I {\n        int Z = 1;\n\n        void h();\n\n        void i();\n    }\n\n    enum E {\n        A, B;\n\n        void j() {}\n    }\n}\n\nclass B {}"
        );
        // Statements in a method and a lambda's body keep theirs.
        let code = "class A {\n    void f() {\n        int a = 1;\n        run(() -> {\n            g();\n        });\n        h();\n    }\n}";
        assert_eq!(java(code), code);
        // The settings: around fields 1, methods 0, after the class header 1, before its end 1.
        let min = MinBlankLines { around_field: 1, around_method: 0, after_class_header: 1, before_class_end: 1, ..Default::default() };
        let style = IndentStyle { min_blank: min, ..Default::default() };
        assert_eq!(
            reformat_indented(ImportStyle::Java, "class A {\n    int x;\n    int y;\n    void f() {}\n    void g() {}\n}", None, &style),
            "class A {\n\n    int x;\n\n    int y;\n\n    void f() {}\n    void g() {}\n\n}"
        );
        // Kotlin: functions and classes taking more than a line get one
        // after them, one-liners and properties don't; a comment or
        // annotation on its own line gets one before it.
        let kotlin = "package a\nimport b.C\nclass A {\n    val x = 1\n    val y: Int\n        get() = 2\n    fun f() = 1\n    fun g() = 2\n    fun h() {\n    }\n    val z = if (a) b\n    else c\n    // Why.\n    val w = 3\n    @Inject\n    lateinit var v: V\n    init {\n    }\n    companion object {\n        const val K = \"=\"\n    }\n}\nfun top() = A()\n    .x\nobject O";
        assert_eq!(
            kt(kotlin),
            "package a\n\nimport b.C\n\nclass A {\n    val x = 1\n    val y: Int\n        get() = 2\n\n    fun f() = 1\n    fun g() = 2\n    fun h() {\n    }\n\n    val z = if (a) b\n    else c\n\n    // Why.\n    val w = 3\n\n    @Inject\n    lateinit var v: V\n\n    init {\n    }\n\n    companion object {\n        const val K = \"=\"\n    }\n}\n\nfun top() = A()\n    .x\n\nobject O"
        );
        // A lambda's statements and Gradle scripts' aren't members.
        let script = "plugins {\n    id(\"a\")\n}\nandroid {\n    namespace = \"b\"\n}";
        assert_eq!(kt(script), script);
        // Only the selected lines get them.
        assert_eq!(reformat(ImportStyle::Java, "class A {\n    void f() {}\n    void g() {}\n}", Some(0..2)), "class A {\n    void f() {}\n    void g() {}\n}");
    }

    #[test]
    fn blank_lines_and_trailing_spaces() {
        assert_eq!(kt("a()   \n\n\n\n\nb()\n"), "a()\n\n\nb()\n");
    }

    #[test]
    fn chops_long_argument_and_parameter_lists() {
        let long = format!("fun f() {{\nval result = repository.loadEverything(firstArgument, \"{}\", listOf(1, 2), third = mapOf<String, Int>())\n}}", "x".repeat(40));
        assert_eq!(
            kt(&long),
            format!("fun f() {{\n    val result = repository.loadEverything(\n        firstArgument,\n        \"{}\",\n        listOf(1, 2),\n        third = mapOf<String, Int>()\n    )\n}}", "x".repeat(40))
        );
        // Reformatting again changes nothing.
        assert_eq!(kt(&kt(&long)), kt(&long));
        let decl = "class Repository {\nfun loadEverything(firstArgument: String, secondArgument: Map<String, Int>, thirdArgument: Int = 1): Result<String> {\nreturn x\n}\n}";
        assert_eq!(
            kt(decl),
            "class Repository {\n    fun loadEverything(\n        firstArgument: String,\n        secondArgument: Map<String, Int>,\n        thirdArgument: Int = 1\n    ): Result<String> {\n        return x\n    }\n}"
        );
        // A chained call past the margin goes to the next line.
        let chain = format!("fun f() {{\ncomposeTestRule.onNodeWithText(getString(R.string.{})).assertIsSelected()\n}}", "a".repeat(60));
        assert_eq!(kt(&chain), format!("fun f() {{\n    composeTestRule.onNodeWithText(getString(R.string.{}))\n        .assertIsSelected()\n}}", "a".repeat(60)));
        assert_eq!(kt(&kt(&chain)), kt(&chain));
        // Else after `=`: an expression body, a property's value.
        let body = format!("fun f(): Flow<Int> = combine(topics, news) {{ topics, news -> topics.size + news.size + {} }}", "n".repeat(40));
        assert_eq!(kt(&body), format!("fun f(): Flow<Int> =\n    combine(topics, news) {{ topics, news -> topics.size + news.size + {} }}", "n".repeat(40)));
        assert_eq!(kt(&kt(&body)), kt(&body));
        // A long string or a condition isn't a list to chop; Java never chops.
        let string = format!("val s = \"{}\"", "y".repeat(130));
        assert_eq!(kt(&string), string);
        let java_line = format!("class A {{\n    void f() {{\n        call(aaaa, \"{}\");\n    }}\n}}", "z".repeat(120));
        assert_eq!(java(&java_line), java_line);
    }

    #[test]
    fn chops_at_the_configured_margin() {
        let text = "fun f() {\nval result = repository.load(firstArgument, secondArgument, third)\n}";
        assert_eq!(kt(text), "fun f() {\n    val result = repository.load(firstArgument, secondArgument, third)\n}");
        let style = IndentStyle { right_margin: Some(60), ..Default::default() };
        assert_eq!(
            reformat_indented(ImportStyle::Kotlin, text, None, &style),
            "fun f() {\n    val result = repository.load(\n        firstArgument,\n        secondArgument,\n        third\n    )\n}"
        );
    }

    #[test]
    fn annotation_targets_and_type_parameters_keep_their_text() {
        let text = "class A {\n@get:Rule\nval r = 1\n@field:JvmField val f = 2\n}\nprivate inline fun <reified T : Any> Project.f() = g<T>()\nfun <T> h(a: Int) = a < b";
        assert_eq!(kt(text), "class A {\n    @get:Rule\n    val r = 1\n    @field:JvmField val f = 2\n}\n\nprivate inline fun <reified T : Any> Project.f() = g<T>()\nfun <T> h(a: Int) = a < b");
    }

    #[test]
    fn operator_at_line_end_continues() {
        let text = "fun f() {\nval s = \"a\" +\n\"b\" +\n\"c\"\ni++\nfoo()\n}";
        assert_eq!(kt(text), "fun f() {\n    val s = \"a\" +\n        \"b\" +\n        \"c\"\n    i++\n    foo()\n}");
        assert_eq!(java("class A {\nint n = 1 *\n2;\nint m = 3;\n}"), "class A {\n    int n = 1 *\n            2;\n    int m = 3;\n}");
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
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\nval a = 1\n}", None, &IndentStyle { unit: Some("\t".into()), ..Default::default() }), "fun f() {\n\tval a = 1\n}");
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\n    val a = 1\n}", None, &IndentStyle { unit: Some("  ".into()), ..Default::default() }), "fun f() {\n  val a = 1\n}");
        // The code style's: Kotlin continuing by 8, a new file's unit 2.
        let style = IndentStyle { unit: None, fallback: Some("  ".into()), continuation: Some((8, 4)), ..Default::default() };
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\nval a =\n1\n}", None, &style), "fun f() {\n  val a =\n      1\n}");
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


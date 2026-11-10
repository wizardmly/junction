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
/// but the Kotlin ones; Kotlin "After class header" (before a first
/// function or property), "Around 'when' branches with {}" and "Before
/// declaration with comment or annotation", its other minimums being fixed
/// at 1 (`kotlinSpacingRules.kt`). IntelliJ's defaults. The settings added
/// later come last in `values`, so older settings files still read.
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
    /// Java's, when a comment comes before the `package`.
    pub before_package: usize,
    /// Java's, before the first member of `new X() { … }`.
    pub after_anonymous_class_header: usize,
    /// Java's, before a method's first statement.
    pub before_method_body: usize,
    /// Kotlin's, between `when` branches when one of them is a `{ … }` block.
    pub around_when_branch_with_block: usize,
}

impl Default for MinBlankLines {
    fn default() -> Self {
        Self::from_values([1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0])
    }
}

impl MinBlankLines {
    /// The settings' labels, in `values` order.
    pub const LABELS: [&'static str; 16] = [
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
        "Before package:",
        "After anonymous class header:",
        "Before method body:",
        "Around 'when' branches with {}:",
    ];
    /// Which of `values` each language's page shows, in IntelliJ's order.
    pub const JAVA: [usize; 14] = [12, 0, 1, 2, 3, 4, 13, 5, 6, 7, 8, 14, 9, 10];
    pub const KOTLIN: [usize; 3] = [4, 15, 11];

    pub fn values(&self) -> [usize; 16] {
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
            self.before_package,
            self.after_anonymous_class_header,
            self.before_method_body,
            self.around_when_branch_with_block,
        ]
    }

    pub fn from_values(v: [usize; 16]) -> Self {
        let [after_package, before_imports, after_imports, around_class, after_class_header, around_field_in_interface, around_field, around_method_in_interface, around_method, around_initializer, before_class_end, before_commented_declaration, before_package, after_anonymous_class_header, before_method_body, around_when_branch_with_block] = v;
        Self {
            after_package,
            before_imports,
            after_imports,
            around_class,
            after_class_header,
            around_field_in_interface,
            around_field,
            around_method_in_interface,
            around_method,
            around_initializer,
            before_class_end,
            before_commented_declaration,
            before_package,
            after_anonymous_class_header,
            before_method_body,
            around_when_branch_with_block,
        }
    }

    /// `values`, or a prefix of them (an older settings file's) with the
    /// rest the defaults.
    pub fn from_prefix(v: &[usize]) -> Option<Self> {
        let mut values = Self::default().values();
        (v.len() >= 12 && v.len() <= values.len()).then(|| {
            values[..v.len()].copy_from_slice(v);
            Self::from_values(values)
        })
    }
}

/// Settings › Code Style › Spaces: which spaces a reformat puts (or
/// takes out) before keywords' parentheses, around operators and at
/// commas. IntelliJ's defaults: all but "Before comma".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spaces {
    /// Before Parentheses, in `values` order: `if`, `for`, `while`,
    /// `switch`, `when`, `try`, `catch`, `synchronized`.
    pub before_parens: [bool; 8],
    /// `=`, `+=`, `-=`…
    pub around_assignment: bool,
    /// `&&`, `||`.
    pub around_logical: bool,
    /// `==`, `!=`, `===`, `!==`.
    pub around_equality: bool,
    /// `<`, `>`, `<=`, `>=`.
    pub around_relational: bool,
    pub before_comma: bool,
    pub after_comma: bool,
    /// Java's Before Keywords: `} else`, `} while`, `} catch`, `} finally`.
    pub before_keywords: [bool; 4],
    /// Kotlin's colons: before / after a declaration's (`name: Type`,
    /// default no / yes), before / after a new type's (`class A : B`,
    /// `<T : Any>`, both yes).
    pub colons: [bool; 4],
    /// Binary `+`, `-`.
    pub around_additive: bool,
    /// `*`, `/`, `%`.
    pub around_multiplicative: bool,
    /// Java's Before Left Brace, in `BEFORE_BRACE` order: class, method,
    /// `if`, `else`, `for`, `while`, `do`, `switch`, `try`, `catch`,
    /// `finally`, `synchronized`.
    pub before_left_brace: [bool; 12],
    /// Java's In Ternary Operator (?:): before / after `?`, before / after `:`.
    pub ternary: [bool; 4],
    /// Arrows: Java's Around operators › Lambda arrow; Kotlin's Around
    /// arrow in function types, Around arrow in "when" clause and Before
    /// lambda arrow (after it there's always a space). All on by default.
    pub arrows: [bool; 4],
    /// Kotlin's Unary operator: `! a`, `- 1`, `i ++` (default off).
    pub around_unary: bool,
    /// Kotlin's Range operator: `a .. b`, `a ..< b` (default off).
    pub around_range: bool,
    /// Kotlin's In simple one line methods: `{ x }` for a lambda or a
    /// function body on one line (default on; other blocks always `{ x }`).
    pub simple_braces: bool,
    /// Java's Within, in `Within` order (all off by default).
    pub within: [bool; 22],
}

/// How many values `Spaces::values` has.
pub const SPACES_LEN: usize = 69;

/// Java's Within options, in `Spaces::within` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Within {
    Brackets,
    ArrayInitializer,
    EmptyArrayInitializer,
    Grouping,
    MethodDeclaration,
    EmptyMethodDeclaration,
    MethodCall,
    EmptyMethodCall,
    If,
    For,
    While,
    Switch,
    Try,
    Catch,
    Synchronized,
    Cast,
    Annotation,
    /// Code braces: a one-line block's `{ x }` (`{ }` when empty).
    CodeBraces,
    AngleBrackets,
    RecordHeader,
    Deconstruction,
    /// Spaces inside block braces when body is present.
    BlockBody,
}

impl Default for Spaces {
    fn default() -> Self {
        Self {
            before_parens: [true; 8],
            around_assignment: true,
            around_logical: true,
            around_equality: true,
            around_relational: true,
            before_comma: false,
            after_comma: true,
            before_keywords: [true; 4],
            colons: [false, true, true, true],
            around_additive: true,
            around_multiplicative: true,
            before_left_brace: [true; 12],
            ternary: [true; 4],
            arrows: [true; 4],
            around_unary: false,
            around_range: false,
            simple_braces: true,
            within: [false; 22],
        }
    }
}

impl Spaces {
    /// The keywords of `before_parens`, in order.
    const KEYWORDS: [&'static str; 8] = ["if", "for", "while", "switch", "when", "try", "catch", "synchronized"];
    /// The keywords of `before_keywords`, in order.
    const AFTER_BRACE: [&'static str; 4] = ["else", "while", "catch", "finally"];
    /// The constructs of `before_left_brace`, in order.
    const BEFORE_BRACE: [&'static str; 12] = ["class", "method", "if", "else", "for", "while", "do", "switch", "try", "catch", "finally", "synchronized"];
    /// The settings' labels, in `values` order.
    pub const LABELS: [&'static str; SPACES_LEN] = [
        "Before parentheses: 'if'",
        "Before parentheses: 'for'",
        "Before parentheses: 'while'",
        "Before parentheses: 'switch'",
        "Before parentheses: 'when'",
        "Before parentheses: 'try'",
        "Before parentheses: 'catch'",
        "Before parentheses: 'synchronized'",
        "Around operators: Assignment (=, +=, ...)",
        "Around operators: Logical (&&, ||)",
        "Around operators: Equality (==, !=)",
        "Around operators: Relational (<, >, <=, >=)",
        "Other: Before comma",
        "Other: After comma",
        "Before keywords: 'else'",
        "Before keywords: 'while'",
        "Before keywords: 'catch'",
        "Before keywords: 'finally'",
        "Other: Before colon, after declaration name",
        "Other: After colon, before declaration type",
        "Other: Before colon in new type definition",
        "Other: After colon in new type definition",
        "Around operators: Additive (+, -)",
        "Around operators: Multiplicative (*, /, %)",
        "Before left brace: Class left brace",
        "Before left brace: Method left brace",
        "Before left brace: 'if' left brace",
        "Before left brace: 'else' left brace",
        "Before left brace: 'for' left brace",
        "Before left brace: 'while' left brace",
        "Before left brace: 'do' left brace",
        "Before left brace: 'switch' left brace",
        "Before left brace: 'try' left brace",
        "Before left brace: 'catch' left brace",
        "Before left brace: 'finally' left brace",
        "Before left brace: 'synchronized' left brace",
        "In ternary operator (?:): Before '?'",
        "In ternary operator (?:): After '?'",
        "In ternary operator (?:): Before ':'",
        "In ternary operator (?:): After ':'",
        "Around operators: Lambda arrow",
        "Around operators: Around arrow in function types",
        "Around operators: Around arrow in 'when' clause",
        "Other: Before lambda arrow",
        "Around operators: Unary operator (!, -, +, ++, --)",
        "Around operators: Range operator (.., ..<)",
        "Other: In simple one line methods",
        "Within: Brackets",
        "Within: Array initializer braces",
        "Within: Empty array initializer braces",
        "Within: Grouping parentheses",
        "Within: Method declaration parentheses",
        "Within: Empty method declaration parentheses",
        "Within: Method call parentheses",
        "Within: Empty method call parentheses",
        "Within: 'if' parentheses",
        "Within: 'for' parentheses",
        "Within: 'while' parentheses",
        "Within: 'switch' parentheses",
        "Within: 'try' parentheses",
        "Within: 'catch' parentheses",
        "Within: 'synchronized' parentheses",
        "Within: Type cast parentheses",
        "Within: Annotation parentheses",
        "Within: Code braces",
        "Within: Angle brackets",
        "Within: Record header",
        "Within: Deconstruction list",
        "Within: Spaces inside block braces when body is present",
    ];
    /// Which of `values` each language's page shows, in IntelliJ's order.
    pub const JAVA: [usize; 59] = [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 22, 23, 44, 40, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 14, 15, 16, 17, 64, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 65, 66, 67, 68, 36, 37, 38, 39, 12, 13,
    ];
    pub const KOTLIN: [usize; 23] = [0, 1, 2, 4, 6, 8, 9, 10, 11, 22, 23, 44, 45, 41, 42, 12, 13, 18, 19, 20, 21, 46, 43];

    pub fn values(&self) -> [bool; SPACES_LEN] {
        let (p, k, c, b, t, a, w) = (self.before_parens, self.before_keywords, self.colons, self.before_left_brace, self.ternary, self.arrows, self.within);
        [
            p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7],
            self.around_assignment, self.around_logical, self.around_equality, self.around_relational, self.before_comma, self.after_comma,
            k[0], k[1], k[2], k[3], c[0], c[1], c[2], c[3],
            self.around_additive, self.around_multiplicative,
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11],
            t[0], t[1], t[2], t[3],
            a[0], a[1], a[2], a[3],
            self.around_unary, self.around_range, self.simple_braces,
            w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7], w[8], w[9], w[10], w[11], w[12], w[13], w[14], w[15], w[16],
            w[17], w[18], w[19], w[20], w[21],
        ]
    }

    pub fn from_values(v: [bool; SPACES_LEN]) -> Self {
        Self {
            before_parens: [v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]],
            around_assignment: v[8],
            around_logical: v[9],
            around_equality: v[10],
            around_relational: v[11],
            before_comma: v[12],
            after_comma: v[13],
            before_keywords: [v[14], v[15], v[16], v[17]],
            colons: [v[18], v[19], v[20], v[21]],
            around_additive: v[22],
            around_multiplicative: v[23],
            before_left_brace: [v[24], v[25], v[26], v[27], v[28], v[29], v[30], v[31], v[32], v[33], v[34], v[35]],
            ternary: [v[36], v[37], v[38], v[39]],
            arrows: [v[40], v[41], v[42], v[43]],
            around_unary: v[44],
            around_range: v[45],
            simple_braces: v[46],
            within: [v[47], v[48], v[49], v[50], v[51], v[52], v[53], v[54], v[55], v[56], v[57], v[58], v[59], v[60], v[61], v[62], v[63], v[64], v[65], v[66], v[67], v[68]],
        }
    }

    /// From saved values, older settings having fewer (the rest default).
    pub fn from_prefix(values: &[bool]) -> Option<Self> {
        let mut all = Self::default().values();
        if !(14..=all.len()).contains(&values.len()) {
            return None;
        }
        all[..values.len()].copy_from_slice(values);
        Some(Self::from_values(all))
    }

    /// Whether `word` after a `}` takes a space (Java's Before Keywords).
    fn before_keyword(&self, word: &str, kotlin: bool) -> bool {
        kotlin || Self::AFTER_BRACE.iter().position(|k| *k == word).is_none_or(|i| self.before_keywords[i])
    }

    /// Whether a `(` after `word` takes a space; `None`: not a keyword's
    /// (Kotlin's `synchronized(…)` is a function, Java's `when(…)` a method).
    fn before_paren(&self, word: &str, kotlin: bool) -> Option<bool> {
        let shown: &[usize] = if kotlin { &Self::KOTLIN } else { &Self::JAVA };
        Self::KEYWORDS.iter().position(|k| *k == word).filter(|i| shown.contains(i)).map(|i| self.before_parens[i])
    }

    /// Whether a Java `{` after `before` (the line so far) takes a space:
    /// Before Left Brace for the construct it opens.
    fn before_brace(&self, before: &str) -> bool {
        let before = before.trim_end();
        let construct = if before.ends_with(')') {
            // The word before the matching `(`: a keyword's, else a method's.
            let mut depth = 0;
            let mut open = None;
            for (i, ch) in before.char_indices().rev() {
                match ch {
                    ')' => depth += 1,
                    '(' => {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match open.map(|o| last_word(before[..o].trim_end())) {
                Some(w @ ("if" | "for" | "while" | "switch" | "try" | "catch" | "synchronized")) => w,
                // `new Runnable() {`: an anonymous class.
                Some(_) if before[..open.unwrap_or(0)].contains("new ") && !before.contains("->") => "class",
                _ => "method",
            }
        } else {
            match last_word(before) {
                w @ ("else" | "do" | "try" | "finally") => w,
                // `throws IOException {`: a method.
                _ if mentions_word(before, "throws") => "method",
                _ if ["class", "interface", "enum", "record", "@interface"].iter().any(|k| mentions_word(before, k)) => "class",
                _ => return true,
            }
        };
        Self::BEFORE_BRACE.iter().position(|k| *k == construct).is_none_or(|i| self.before_left_brace[i])
    }

    /// Whether an operator of `normalize`'s takes spaces around it.
    fn around(&self, op: &str) -> bool {
        match op {
            "&&" | "||" => self.around_logical,
            "==" | "!=" | "===" | "!==" => self.around_equality,
            "<" | ">" | "<=" | ">=" => self.around_relational,
            "+" | "-" => self.around_additive,
            "*" | "/" | "%" => self.around_multiplicative,
            _ => self.around_assignment,
        }
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
    /// Ends with a statement's head without its body: `if (…)`, `else`,
    /// `for (…)`, `while (…)`, `do`. The next line is the body, one
    /// indent in from this (the head's first line's indent).
    head: Option<usize>,
    /// Its text as laid out.
    line: String,
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
    /// Java's and Kotlin's (Groovy's are IntelliJ's defaults).
    pub spaces: Spaces,
    /// Wrapping and Braces (`None`: the language's defaults).
    pub wrapping: Option<Wrapping>,
    /// Java's Chained method calls › Builder methods: calls by these names
    /// always start a line of their own.
    pub builders: Vec<String>,
}

/// Code Style › Wrapping and Braces: how a long construct wraps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wrap {
    DoNotWrap,
    /// As many items on a line as fit.
    WrapIfLong,
    /// One item a line when the construct is long.
    ChopIfLong,
}

impl Wrap {
    pub const LABELS: [&'static str; 3] = ["Do not wrap", "Wrap if long", "Chop down if long"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        [Wrap::DoNotWrap, Wrap::WrapIfLong, Wrap::ChopIfLong].get(i).copied()
    }
}

/// The Wrapping and Braces options Reformat Code applies past the right
/// margin (IntelliJ's CALL_PARAMETERS_WRAP, METHOD_PARAMETERS_WRAP,
/// METHOD_CALL_CHAIN_WRAP, EXTENDS_LIST_WRAP, ASSIGNMENT_WRAP).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wrapping {
    pub call_args: Wrap,
    pub params: Wrap,
    pub chain: Wrap,
    /// Kotlin's supertype list (Java's isn't wrapped).
    pub extends: Wrap,
    /// Assignments, initializers and Kotlin's expression bodies.
    pub assignment: Wrap,
    /// Keep when reformatting › Line breaks: off joins the lines of a
    /// wrapped list or expression (the options above wrap it again).
    pub keep_line_breaks: bool,
    pub braces: Braces,
    /// Java's binary expressions and ternaries.
    pub ops: Operations,
    /// Java's class, method and field annotations.
    pub annotations: [AnnotationWrap; 3],
    /// Java's enum constants (Enum constants; IntelliJ's default Do not wrap).
    pub enum_constants: AnnotationWrap,
    /// Kotlin's Elvis expressions (`?:` starts the next line).
    pub elvis: Wrap,
    /// Java's local variable annotations (IntelliJ's VARIABLE_ANNOTATION_WRAP).
    pub local_annotations: AnnotationWrap,
    /// Java's array initializers (IntelliJ's ARRAY_INITIALIZER_WRAP),
    /// annotations' `{…}` too.
    pub array_init: Wrap,
    /// Java's Throws list (THROWS_LIST_WRAP).
    pub throws_list: Wrap,
    /// Java's Throws keyword (THROWS_KEYWORD_WRAP): `throws` starts the next line.
    pub throws_keyword: Wrap,
    /// Java's parameter annotations (PARAMETER_ANNOTATION_WRAP): a
    /// declaration parameter's annotations each end a line.
    pub param_annotations: AnnotationWrap,
    /// Java's Keep when reformatting: simple blocks, methods, lambdas and
    /// classes, and control statements, in one line.
    pub keep_one_line: KeepOneLine,
    /// Java's Special 'else if' treatment (SPECIAL_ELSE_IF_TREATMENT, on by
    /// default): `else if` on one line; off, `if` starts the next line.
    pub special_else_if: bool,
    /// Java's 'for()' statement (FOR_STATEMENT_WRAP): the header's parts
    /// after `;`.
    pub for_wrap: Wrap,
    /// Java's 'for()' statement › Align when multiline (ALIGN_MULTILINE_FOR,
    /// on by default): the parts under the first.
    pub align_for: bool,
    /// Java's 'for()' statement › New line after '(' and Place ')' on new
    /// line (FOR_STATEMENT_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE), for a
    /// header over lines.
    pub for_parens: [bool; 2],
    /// Java's 'try-with-resources' (RESOURCE_LIST_WRAP): the resources after `;`.
    pub resource_wrap: Wrap,
    /// Its Align when multiline (ALIGN_MULTILINE_RESOURCES, on by default).
    pub align_resources: bool,
    /// Its New line after '(' and Place ')' on new line
    /// (RESOURCE_LIST_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE).
    pub resource_parens: [bool; 2],
    /// Java's 'assert' statement (ASSERT_STATEMENT_WRAP): a long one breaks
    /// before its message.
    pub assert_wrap: Wrap,
    /// Its Place ':' on new line (ASSERT_STATEMENT_COLON_ON_NEXT_LINE): the
    /// break before `:` instead of after it.
    pub assert_colon_next: bool,
    /// Method call arguments › New line after '(' and Place ')' on new line
    /// (CALL_PARAMETERS_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE), for
    /// arguments over lines; Kotlin's chopped lists too.
    pub call_parens: [bool; 2],
    /// Method declaration parameters › the same
    /// (METHOD_PARAMETERS_LPAREN_ON_NEXT_LINE / RPAREN_ON_NEXT_LINE).
    pub param_parens: [bool; 2],
    /// Java's Parenthesized expression › New line after '(' and Place ')' on
    /// new line (PARENTHESES_EXPRESSION_LPAREN_WRAP / RPAREN_WRAP).
    pub expr_parens: [bool; 2],
    /// Java's Method declaration parameters › Align when multiline
    /// (ALIGN_MULTILINE_PARAMETERS, on by default): under the first.
    pub align_params: bool,
    /// Java's Method call arguments › Align when multiline
    /// (ALIGN_MULTILINE_PARAMETERS_IN_CALLS, off by default).
    pub align_args: bool,
    /// Java's Record components (RECORD_COMPONENTS_WRAP): a record header's
    /// components, instead of the method parameters' option.
    pub record_wrap: Wrap,
    /// Its Align when multiline (ALIGN_MULTILINE_RECORDS, on by default).
    pub align_records: bool,
    /// Its New line after '(' and Place ')' on new line
    /// (NEW_LINE_AFTER_LPAREN_IN_RECORD_HEADER / RPAREN_ON_NEW_LINE_IN_RECORD_HEADER).
    pub record_parens: [bool; 2],
    /// Java's Binary expressions › Align when multiline
    /// (ALIGN_MULTILINE_BINARY_OPERATION, off by default): operands under the first.
    pub align_binary: bool,
    /// Java's Ternary operation › Align when multiline (ALIGN_MULTILINE_TERNARY_OPERATION, off).
    pub align_ternary: bool,
    /// Java's Throws list › Align when multiline (ALIGN_MULTILINE_THROWS_LIST, off).
    pub align_throws: bool,
    /// Java's Extends / implements list › Align when multiline (ALIGN_MULTILINE_EXTENDS_LIST, off).
    pub align_extends: bool,
    /// Java's Assignment statement › Align when multiline
    /// (ALIGN_MULTILINE_ASSIGNMENT, off): `a =` / `b = c` with `b` under `a`.
    pub align_assignment: bool,
    /// Java's Chained method calls › Align when multiline
    /// (ALIGN_MULTILINE_CHAINED_METHODS, off): `.call()` under the first call's dot.
    pub align_chains: bool,
    /// Java's Array initializer › Align when multiline
    /// (ALIGN_MULTILINE_ARRAY_INITIALIZER_EXPRESSION, off): elements under the first.
    pub align_arrays: bool,
    /// Java's Annotation parameters (ANNOTATION_PARAMETER_WRAP).
    pub annotation_params: Wrap,
    /// Kotlin's 'when' statements › Align 'when' branches in columns
    /// (ALIGN_IN_COLUMNS_CASE_BRANCH, off): the branches' `->` in a column.
    pub when_align: bool,
    /// Kotlin's 'when' statements › Line break after multiline 'when' entry
    /// (LINE_BREAK_AFTER_MULTILINE_WHEN_ENTRY, on): a blank line after a
    /// branch over lines.
    pub when_break: bool,
    /// Java's Deconstruction list (DECONSTRUCTION_LIST_WRAP): a record
    /// pattern's components (`instanceof P(int x, int y)`, `case P(…)`).
    pub decon_wrap: Wrap,
    /// Its Align when multiline (ALIGN_MULTILINE_DECONSTRUCTION_LIST_COMPONENTS, on).
    pub align_decon: bool,
    /// Its New line after '(' and Place ')' on new line
    /// (NEW_LINE_AFTER_LPAREN_IN_DECONSTRUCTION_PATTERN / RPAREN_ON_NEW_LINE_IN_DECONSTRUCTION_PATTERN, both on).
    pub decon_parens: [bool; 2],
    /// Kotlin's Use continuation indent, in order: parameters, arguments, expression bodies, chained calls, supertypes, `if` conditions, Elvis, context parameters
    /// (CONTINUATION_INDENT_IN_PARAMETER_LISTS and the like; the official
    /// style's are all off: a normal indent).
    pub kotlin_cont: [bool; 8],
    /// Keep when reformatting › Comment at first column (KEEP_FIRST_COLUMN_COMMENT,
    /// on): a comment starting a line at column 0 stays there (Kotlin's line comments).
    pub keep_first_column: bool,
    /// Java's 'switch' statement › Indent 'case' branches (INDENT_CASE_FROM_SWITCH,
    /// on): off, `case` labels at the `switch`'s level.
    pub indent_case: bool,
    /// Chained method calls › Wrap first call (WRAP_FIRST_METHOD_IN_CALL_CHAIN,
    /// off): the chain's first call may wrap too, when another call follows it.
    pub wrap_first_call: bool,
    /// Java's Group declarations › Align fields / variables / assignments in
    /// columns (ALIGN_GROUP_FIELD_DECLARATIONS, ALIGN_CONSECUTIVE_VARIABLE_DECLARATIONS,
    /// ALIGN_CONSECUTIVE_ASSIGNMENTS; all off): consecutive one-line ones in columns.
    pub group_align: [bool; 3],
    /// Java's Keep when reformatting › Multiple expressions in one line
    /// (KEEP_MULTIPLE_EXPRESSIONS_IN_ONE_LINE, off): off, `a(); b();` in a
    /// block goes a statement a line.
    pub keep_multiple: bool,
    /// Java's Modifier list › Wrap after modifier list (MODIFIER_LIST_WRAP,
    /// off): a class member's modifiers on a line of their own.
    pub modifier_wrap: bool,
    /// Java's Group declarations › Align simple methods in columns
    /// (ALIGN_SUBSEQUENT_SIMPLE_METHODS, off): consecutive one-line methods' `{` in a column.
    pub align_simple_methods: bool,
    /// Java's Assignment statement › Assignment sign on next line
    /// (PLACE_ASSIGNMENT_SIGN_ON_NEXT_LINE, off): a wrapped assignment's `=` starts the value's line.
    pub assign_sign_next: bool,
    /// Java's 'switch' statement › Indent 'break' from 'case' (INDENT_BREAK_FROM_CASE,
    /// on): off, a case body's `break` at the label's level.
    pub indent_break: bool,
    /// Java's Array initializer › New line after '{' and Place '}' on new line
    /// (ARRAY_INITIALIZER_LBRACE_ON_NEXT_LINE / RBRACE_ON_NEXT_LINE, off), for an initializer over lines.
    pub array_parens: [bool; 2],
    /// Java's Field / Parameter annotations › Do not wrap after single annotation
    /// (DO_NOT_WRAP_AFTER_SINGLE_ANNOTATION / _IN_PARAMETER, off): one annotation stays on the line.
    pub single_annotation: [bool; 2],
}


/// Wrapping and Braces › Class / Method / Field annotations (IntelliJ's
/// CLASS_ANNOTATION_WRAP and the like). IntelliJ's default is Wrap always;
/// ours is Do not wrap, so `@Test public void f()` stays as written.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AnnotationWrap {
    #[default]
    DoNotWrap,
    WrapIfLong,
    ChopIfLong,
    WrapAlways,
}

impl AnnotationWrap {
    pub const LABELS: [&'static str; 4] = ["Do not wrap", "Wrap if long", "Chop down if long", "Wrap always"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        [AnnotationWrap::DoNotWrap, AnnotationWrap::WrapIfLong, AnnotationWrap::ChopIfLong, AnnotationWrap::WrapAlways].get(i).copied()
    }
}

/// Wrapping and Braces › Binary expressions and Ternary operation, with
/// their Operation sign on next line options (IntelliJ's
/// BINARY_OPERATION_WRAP, BINARY_OPERATION_SIGN_ON_NEXT_LINE,
/// TERNARY_OPERATION_WRAP, TERNARY_OPERATION_SIGNS_ON_NEXT_LINE). Java's;
/// none wrap by default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Operations {
    pub binary: Wrap,
    pub binary_sign_next: bool,
    pub ternary: Wrap,
    pub ternary_signs_next: bool,
}

impl Operations {
    pub const NONE: Operations = Operations { binary: Wrap::DoNotWrap, binary_sign_next: false, ternary: Wrap::DoNotWrap, ternary_signs_next: false };
}

/// Wrapping and Braces › Braces placement and the 'else' / 'while' /
/// 'catch' / 'finally' on new line options (IntelliJ's CLASS_BRACE_STYLE,
/// METHOD_BRACE_STYLE, BRACE_STYLE: off is End of line, on Next line;
/// ELSE_ON_NEW_LINE and the like). All off by default; Kotlin has only
/// the keyword ones but 'while'.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Braces {
    pub else_new_line: bool,
    pub while_new_line: bool,
    pub catch_new_line: bool,
    pub finally_new_line: bool,
    pub class_next_line: bool,
    pub method_next_line: bool,
    /// Kotlin's Put left brace on new line, for all its braces.
    pub other_next_line: bool,
    /// Force braces for Java's 'if', 'for' and 'while'.
    pub force: [Force; 3],
}

/// Wrapping and Braces › Keep when reformatting (IntelliJ's
/// KEEP_SIMPLE_BLOCKS_IN_ONE_LINE, KEEP_SIMPLE_METHODS_IN_ONE_LINE,
/// KEEP_SIMPLE_LAMBDAS_IN_ONE_LINE, KEEP_SIMPLE_CLASSES_IN_ONE_LINE and
/// KEEP_CONTROL_STATEMENT_IN_ONE_LINE): what may stay on one line. IntelliJ
/// keeps only control statements; Junction keeps all, as the file has them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeepOneLine {
    /// `if (a) { b(); }`, `{}` of a statement.
    pub blocks: bool,
    /// `void f() { g(); }`, `void f() {}`.
    pub methods: bool,
    /// `() -> { g(); }`.
    pub lambdas: bool,
    /// `class A {}`.
    pub classes: bool,
    /// `if (a) return;`.
    pub control: bool,
}

impl KeepOneLine {
    pub const ALL: KeepOneLine = KeepOneLine { blocks: true, methods: true, lambdas: true, classes: true, control: true };
    pub const LABELS: [&'static str; 5] = ["Simple blocks in one line", "Simple methods in one line", "Simple lambdas in one line", "Simple classes in one line", "Control statement in one line"];

    pub fn values(&self) -> [bool; 5] {
        [self.blocks, self.methods, self.lambdas, self.classes, self.control]
    }

    pub fn from_values(v: [bool; 5]) -> Self {
        KeepOneLine { blocks: v[0], methods: v[1], lambdas: v[2], classes: v[3], control: v[4] }
    }
}

impl Default for KeepOneLine {
    fn default() -> Self {
        Self::ALL
    }
}

/// Wrapping and Braces › Force braces (IntelliJ's IF_BRACE_FORCE and the like).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Force {
    #[default]
    DoNotForce,
    /// When the statement is on its own line.
    WhenMultiline,
    Always,
}

impl Force {
    pub const LABELS: [&'static str; 3] = ["Do not force", "When multiline", "Always"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        [Force::DoNotForce, Force::WhenMultiline, Force::Always].get(i).copied()
    }
}

impl Braces {
    pub const LABELS: [&'static str; 7] = ["'else' on new line", "'while' on new line", "'catch' on new line", "'finally' on new line", "Braces placement: In class declaration: Next line", "Braces placement: In method declaration: Next line", "Braces placement: Other: Next line"];

    pub fn values(&self) -> [bool; 7] {
        [self.else_new_line, self.while_new_line, self.catch_new_line, self.finally_new_line, self.class_next_line, self.method_next_line, self.other_next_line]
    }

    pub const NONE: Braces = Braces { else_new_line: false, while_new_line: false, catch_new_line: false, finally_new_line: false, class_next_line: false, method_next_line: false, other_next_line: false, force: [Force::DoNotForce; 3] };

    /// The seven switches; `force` stays.
    pub fn with_values(self, v: [bool; 7]) -> Self {
        Braces { else_new_line: v[0], while_new_line: v[1], catch_new_line: v[2], finally_new_line: v[3], class_next_line: v[4], method_next_line: v[5], other_next_line: v[6], ..self }
    }

    pub fn from_values(v: [bool; 7]) -> Self {
        Braces::NONE.with_values(v)
    }
}

impl Wrapping {
    /// IntelliJ's Java defaults: nothing wraps.
    pub const JAVA: Wrapping = Wrapping { call_args: Wrap::DoNotWrap, params: Wrap::DoNotWrap, chain: Wrap::DoNotWrap, extends: Wrap::DoNotWrap, assignment: Wrap::DoNotWrap, keep_line_breaks: true, braces: Braces::NONE, ops: Operations::NONE, annotations: [AnnotationWrap::DoNotWrap; 3], enum_constants: AnnotationWrap::DoNotWrap, elvis: Wrap::DoNotWrap, local_annotations: AnnotationWrap::DoNotWrap, array_init: Wrap::DoNotWrap, throws_list: Wrap::DoNotWrap, throws_keyword: Wrap::DoNotWrap, annotation_params: Wrap::DoNotWrap, param_annotations: AnnotationWrap::DoNotWrap, keep_one_line: KeepOneLine::ALL, special_else_if: true, for_wrap: Wrap::DoNotWrap, align_for: true, for_parens: [false; 2], resource_wrap: Wrap::DoNotWrap, align_resources: true, resource_parens: [false; 2], assert_wrap: Wrap::DoNotWrap, assert_colon_next: false, call_parens: [false; 2], param_parens: [false; 2], expr_parens: [false; 2], align_params: true, align_args: false, record_wrap: Wrap::DoNotWrap, align_records: true, record_parens: [false; 2], align_binary: false, align_ternary: false, align_throws: false, align_extends: false, align_assignment: false, align_chains: false, align_arrays: false, when_align: false, when_break: true, decon_wrap: Wrap::DoNotWrap, align_decon: true, decon_parens: [true; 2], kotlin_cont: [false; 8], keep_first_column: true, indent_case: true, wrap_first_call: false, group_align: [false; 3], keep_multiple: false, modifier_wrap: false, align_simple_methods: false, assign_sign_next: false, indent_break: true, array_parens: [false; 2], single_annotation: [false; 2] };
    /// Kotlin's official style.
    pub const KOTLIN: Wrapping = Wrapping { call_args: Wrap::ChopIfLong, params: Wrap::ChopIfLong, chain: Wrap::WrapIfLong, extends: Wrap::WrapIfLong, assignment: Wrap::WrapIfLong, keep_line_breaks: true, braces: Braces::NONE, ops: Operations::NONE, annotations: [AnnotationWrap::DoNotWrap; 3], enum_constants: AnnotationWrap::DoNotWrap, elvis: Wrap::WrapIfLong, local_annotations: AnnotationWrap::DoNotWrap, array_init: Wrap::DoNotWrap, throws_list: Wrap::DoNotWrap, throws_keyword: Wrap::DoNotWrap, annotation_params: Wrap::DoNotWrap, param_annotations: AnnotationWrap::DoNotWrap, keep_one_line: KeepOneLine::ALL, special_else_if: true, for_wrap: Wrap::DoNotWrap, align_for: true, for_parens: [false; 2], resource_wrap: Wrap::DoNotWrap, align_resources: true, resource_parens: [false; 2], assert_wrap: Wrap::DoNotWrap, assert_colon_next: false, call_parens: [true; 2], param_parens: [true; 2], expr_parens: [false; 2], align_params: true, align_args: false, record_wrap: Wrap::DoNotWrap, align_records: true, record_parens: [false; 2], align_binary: false, align_ternary: false, align_throws: false, align_extends: false, align_assignment: false, align_chains: false, align_arrays: false, when_align: false, when_break: true, decon_wrap: Wrap::DoNotWrap, align_decon: true, decon_parens: [true; 2], kotlin_cont: [false; 8], keep_first_column: true, indent_case: true, wrap_first_call: false, group_align: [false; 3], keep_multiple: false, modifier_wrap: false, align_simple_methods: false, assign_sign_next: false, indent_break: true, array_parens: [false; 2], single_annotation: [false; 2] };

    pub fn values(&self) -> [Wrap; 5] {
        [self.call_args, self.params, self.chain, self.extends, self.assignment]
    }

    pub fn from_values(v: [Wrap; 5]) -> Self {
        Wrapping { call_args: v[0], params: v[1], chain: v[2], extends: v[3], assignment: v[4], keep_line_breaks: true, braces: Braces::default(), ops: Operations::NONE, annotations: [AnnotationWrap::DoNotWrap; 3], enum_constants: AnnotationWrap::DoNotWrap, elvis: Wrap::DoNotWrap, local_annotations: AnnotationWrap::DoNotWrap, array_init: Wrap::DoNotWrap, throws_list: Wrap::DoNotWrap, throws_keyword: Wrap::DoNotWrap, annotation_params: Wrap::DoNotWrap, param_annotations: AnnotationWrap::DoNotWrap, keep_one_line: KeepOneLine::ALL, special_else_if: true, for_wrap: Wrap::DoNotWrap, align_for: true, for_parens: [false; 2], resource_wrap: Wrap::DoNotWrap, align_resources: true, resource_parens: [false; 2], assert_wrap: Wrap::DoNotWrap, assert_colon_next: false, call_parens: [false; 2], param_parens: [false; 2], expr_parens: [false; 2], align_params: true, align_args: false, record_wrap: Wrap::DoNotWrap, align_records: true, record_parens: [false; 2], align_binary: false, align_ternary: false, align_throws: false, align_extends: false, align_assignment: false, align_chains: false, align_arrays: false, when_align: false, when_break: true, decon_wrap: Wrap::DoNotWrap, align_decon: true, decon_parens: [true; 2], kotlin_cont: [false; 8], keep_first_column: true, indent_case: true, wrap_first_call: false, group_align: [false; 3], keep_multiple: false, modifier_wrap: false, align_simple_methods: false, assign_sign_next: false, indent_break: true, array_parens: [false; 2], single_annotation: [false; 2] }
    }

    /// The five wraps changed; the other options stay.
    pub fn with_values(self, v: [Wrap; 5]) -> Self {
        Wrapping { keep_line_breaks: self.keep_line_breaks, braces: self.braces, ops: self.ops, annotations: self.annotations, enum_constants: self.enum_constants, elvis: self.elvis, local_annotations: self.local_annotations, array_init: self.array_init, throws_list: self.throws_list, throws_keyword: self.throws_keyword, annotation_params: self.annotation_params, param_annotations: self.param_annotations, keep_one_line: self.keep_one_line, special_else_if: self.special_else_if, for_wrap: self.for_wrap, align_for: self.align_for, for_parens: self.for_parens, resource_wrap: self.resource_wrap, align_resources: self.align_resources, resource_parens: self.resource_parens, assert_wrap: self.assert_wrap, assert_colon_next: self.assert_colon_next, call_parens: self.call_parens, param_parens: self.param_parens, expr_parens: self.expr_parens, align_params: self.align_params, align_args: self.align_args, record_wrap: self.record_wrap, align_records: self.align_records, record_parens: self.record_parens, align_binary: self.align_binary, align_ternary: self.align_ternary, align_throws: self.align_throws, align_extends: self.align_extends, align_assignment: self.align_assignment, align_chains: self.align_chains, align_arrays: self.align_arrays, when_align: self.when_align, when_break: self.when_break, decon_wrap: self.decon_wrap, align_decon: self.align_decon, decon_parens: self.decon_parens, kotlin_cont: self.kotlin_cont, keep_first_column: self.keep_first_column, indent_case: self.indent_case, wrap_first_call: self.wrap_first_call, group_align: self.group_align, keep_multiple: self.keep_multiple, modifier_wrap: self.modifier_wrap, align_simple_methods: self.align_simple_methods, assign_sign_next: self.assign_sign_next, indent_break: self.indent_break, array_parens: self.array_parens, single_annotation: self.single_annotation, ..Wrapping::from_values(v) }
    }

    fn any(&self) -> bool {
        self.values().iter().any(|w| *w != Wrap::DoNotWrap) || self.ops.binary != Wrap::DoNotWrap || self.ops.ternary != Wrap::DoNotWrap || self.elvis != Wrap::DoNotWrap || self.array_init != Wrap::DoNotWrap || self.throws_list != Wrap::DoNotWrap || self.throws_keyword != Wrap::DoNotWrap || self.annotation_params != Wrap::DoNotWrap || self.assert_wrap != Wrap::DoNotWrap || self.record_wrap != Wrap::DoNotWrap || self.decon_wrap != Wrap::DoNotWrap
    }
}

/// What long lines are wrapped with.
struct Wrapper<'a> {
    kotlin: bool,
    /// One indent, and Java's continuation (Kotlin's is the indent).
    unit: &'a str,
    cont: String,
    margin: usize,
    w: Wrapping,
    tabs: bool,
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
    reformat_pass(flavor, text, only, indents, 0)
}

/// One pass of `reformat_with`; `nested`: how many passes after chopping
/// came before (a pass that chops again goes once more, up to two).
fn reformat_pass(flavor: Flavor, text: &str, only: Option<Range<usize>>, indents: &IndentStyle, nested: u8) -> String {
    let kotlin = flavor == Flavor::Kotlin;
    // Kotlin's and Groovy's strings and colons.
    let templates = flavor != Flavor::Java;
    let wrapping = match flavor {
        Flavor::Groovy => None,
        Flavor::Kotlin => Some(indents.wrapping.unwrap_or(Wrapping::KOTLIN)),
        Flavor::Java => Some(indents.wrapping.unwrap_or(Wrapping::JAVA)),
    };
    let laid;
    let (text, only) = match wrapping {
        Some(w) => {
            // Force braces first: the braces it adds are laid out too.
            let (forced, only) = if kotlin { (text.to_owned(), only) } else { force_braces(text, &w.braces.force, only) };
            let wrapped_annotations;
            let (forced, only) = if kotlin {
                let margin = indents.right_margin.unwrap_or(RIGHT_MARGIN);
                let [class, method, field] = w.annotations;
                let (a, only) = wrap_annotations(&forced, true, &[class, method, field, w.local_annotations], false, only, margin);
                let (a, only) = wrap_parameter_annotations(&a, true, w.param_annotations, false, only, margin);
                let (a, only) = list_parens(&a, true, w.call_parens, w.param_parens, [true; 2], [false; 2], [false; 2], [false; 2], only);
                wrapped_annotations = a;
                (wrapped_annotations, only)
            } else {
                let margin = indents.right_margin.unwrap_or(RIGHT_MARGIN);
                let [class, method, field] = w.annotations;
                let (a, only) = wrap_annotations(&forced, false, &[class, method, field, w.local_annotations], w.single_annotation[0], only, margin);
                let (a, only) = wrap_enum_constants(&a, w.enum_constants, only, margin);
                let (a, only) = wrap_parameter_annotations(&a, false, w.param_annotations, w.single_annotation[1], only, margin);
                let (a, only) = split_one_line_bodies(&a, w.keep_one_line, w.keep_multiple, only);
                let (a, only) = else_if_layout(&a, w.special_else_if, only);
                let (a, only) = wrap_for_statements(&a, "for", w.for_wrap, w.align_for, only, margin);
                let (a, only) = wrap_for_statements(&a, "try", w.resource_wrap, w.align_resources, only, margin);
                let (a, only) = header_parens(&a, "for", w.for_parens, only);
                let (a, only) = header_parens(&a, "try", w.resource_parens, only);
                let (a, only) = wrap_builder_calls(&a, &indents.builders, w.wrap_first_call, only);
                let (a, only) = list_parens(&a, false, w.call_parens, w.param_parens, w.record_parens, w.expr_parens, w.decon_parens, w.array_parens, only);
                wrapped_annotations = a;
                (wrapped_annotations, only)
            };
            // Twice: a `} else {` split puts `else {` on a line whose `{` may move too.
            let (once, only) = brace_layout(&forced, kotlin, &w.braces, only);
            let (l, only) = brace_layout(&once, kotlin, &w.braces, only);
            laid = l;
            (laid.as_str(), only)
        }
        None => (text, only),
    };
    let joined;
    let (text, only) = match wrapping {
        Some(w) if !w.keep_line_breaks => {
            let (j, only) = join_line_breaks(text, kotlin, only, indents.right_margin.unwrap_or(RIGHT_MARGIN));
            joined = j;
            (joined.as_str(), only)
        }
        _ => (text, only),
    };
    // The project's code style is the file's: its indent unit (4 spaces
    // when it has none yet). Columns count units when it's a tab.
    let lines = scan(text, templates);
    let unit = indents.unit.clone().unwrap_or_else(|| indent_unit(text, &lines, indents.fallback.as_deref()));
    // Java aligns a multi-line `for (` header under its first part unless Align when multiline is off.
    let align_for = wrapping.is_none_or(|w| w.align_for);
    let align_resources = wrapping.is_none_or(|w| w.align_resources);
    let align_params = wrapping.is_none_or(|w| w.align_params);
    let align_records = wrapping.is_none_or(|w| w.align_records);
    let align_args = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_args);
    let align_binary = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_binary);
    let align_ternary = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_ternary);
    let align_throws = wrapping.is_some_and(|w| w.align_throws);
    let align_extends = wrapping.is_some_and(|w| w.align_extends);
    let align_assignment = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_assignment);
    let align_chains = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_chains);
    let align_arrays = flavor == Flavor::Java && wrapping.is_some_and(|w| w.align_arrays);
    let align_decon = wrapping.is_none_or(|w| w.align_decon);
    let kotlin_cont = wrapping.filter(|_| kotlin).map_or([false; 8], |w| w.kotlin_cont);
    // The header list's lines so far are aligned.
    let mut header_aligned = false;
    // A header line's indent before that alignment: its body goes by it.
    let mut header_unaligned = 0;
    let tabs = unit == "\t";
    let step = if tabs { 1 } else { unit.len() };
    // Kotlin's official style continues by one indent; Java by 8 (two);
    // the code style's in proportion to the unit.
    let cont = match indents.continuation {
        Some((continuation, indent)) if indent > 0 => ((continuation * step) as f32 / indent as f32).round() as usize,
        _ if kotlin => step,
        _ => 2 * step,
    };
    // Kotlin's Use continuation indent: the continuation where it's on, a
    // normal indent where it's off (the official style's).
    let kc = |i: usize| if !kotlin || kotlin_cont[i] { cont } else { step };
    let bracket_sizes = kotlin.then(|| [kc(0), kc(1), kc(5), kc(7)]);
    let keep_first_column = wrapping.is_none_or(|w| w.keep_first_column);
    let flat_switch = flavor == Flavor::Java && wrapping.is_some_and(|w| !w.indent_case);
    let flat_break = flavor == Flavor::Java && wrapping.is_some_and(|w| !w.indent_break);
    // A block comment kept at column 0: its later lines stay too.
    let mut kept_block = false;
    let header_size = kc(4);
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    // Per line out: code the selection reformats, which may be chopped.
    let mut choppable: Vec<bool> = Vec::with_capacity(raw.len());
    let mut stack: Vec<Open> = Vec::new();
    // The last code line.
    let mut previous: Option<Previous> = None;
    let mut blank_run = 0;
    // A statement head left open on its line: the depth before it and its indent.
    let mut split_head: Option<(usize, usize)> = None;
    let bodies = declaration_braces(&raw, &lines);
    let keep = indents.keep_blank;
    let spaces = if flavor == Flavor::Groovy { Spaces::default() } else { indents.spaces };
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
            apply_brackets(&mut stack, &info.brackets, 0, ix, lead, lead, cont, step, None, [false; 4], text_line, &bodies[ix], flavor == Flavor::Java, bracket_sizes);
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
        let mut body_line = false;
        let mut in_opened = false;
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
            let reason = if (code_start.starts_with('.') && !code_start.starts_with("..") && !code_start[1..].starts_with(|c: char| c.is_ascii_digit())) || code_start.starts_with("?.") {
                Reason::Chain
            } else if code_start.starts_with("?:") {
                Reason::Elvis
            } else if kotlin && code_start.starts_with("->") {
                // A `when` branch's arrow on its own line: one indent.
                Reason::Elvis
            } else if code_start.starts_with("&&") || code_start.starts_with("||") || !kotlin && (code_start.starts_with("? ") || code_start.starts_with(": ")) {
                Reason::Binary
            } else if flavor == Flavor::Java && ["+ ", "- ", "* ", "/ ", "% "].iter().any(|op| code_start.starts_with(op)) {
                // Operation sign on next line: `+ b`.
                Reason::Operator
            } else if flavor == Flavor::Java && code_start.starts_with("= ") {
                // Assignment sign on next line: `= value`.
                Reason::Assign
            } else if flavor == Flavor::Java && ["throws ", "implements ", "extends "].iter().any(|k| code_start.starts_with(k)) {
                // Throws keyword / Extends keyword on next line: the header goes on.
                Reason::Header
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
                        } else if kotlin && prev.ends_with(':') && !prev.ends_with("::") || (p.reason == Reason::Header || kotlin && supertypes_wrapped(prev) || flavor == Flavor::Java && java_header_list(prev)) && prev.ends_with(',') {
                            Reason::Header
                        } else if flavor == Flavor::Groovy && prev.ends_with(',') && stack.last().is_none_or(|o| o.brace) {
                            // A command's arguments without parentheses:
                            // `proguardFiles a,` then `b`.
                            Reason::Binary
                        } else if flavor == Flavor::Java && java_ternary_end(prev) {
                            // `cond ?` / `one :`.
                            Reason::Binary
                        } else {
                            Reason::None
                        }
                    }
                    // Java: an expression that goes on in the bracket its
                    // line opened (`if (a &&`): the bracket's indent is the
                    // continuation; the operands after line up with it.
                    Some(p) if p.opened && p.depth == stack.len() && flavor == Flavor::Java => {
                        let prev = p.code.as_str();
                        if prev.ends_with("&&") || prev.ends_with("||") || java_ternary_end(prev) {
                            in_opened = true;
                            Reason::Binary
                        } else if ends_with_operator(prev) {
                            in_opened = true;
                            Reason::Operator
                        } else {
                            Reason::None
                        }
                    }
                    _ => Reason::None,
                }
            };
            // `&& b` first in a bracket the line before opened: likewise.
            if flavor == Flavor::Java && matches!(reason, Reason::Binary | Reason::Operator) && previous.as_ref().is_some_and(|p| p.opened && p.depth == stack.len()) {
                in_opened = true;
            }
            if reason != Reason::None && !in_opened {
                let size = match reason {
                    _ if !kotlin => cont,
                    Reason::Binary => 2 * cont,
                    Reason::Chain => kc(3),
                    Reason::Elvis if code_start.starts_with("?:") => kc(6),
                    Reason::Assign => kc(2),
                    Reason::Header => kc(4),
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
            // Java's Binary expressions › Align when multiline: an operand
            // under the expression's first one (not a ternary's).
            let ternary = code_start.starts_with("? ") || code_start.starts_with(": ") || previous.as_ref().is_some_and(|p| java_ternary_end(&p.code));
            // Ternary operation › Align when multiline: `? a` / `: b` under the condition.
            let aligned = if ternary { align_ternary } else { align_binary };
            if aligned && matches!(reason, Reason::Binary | Reason::Operator) {
                if let Some(p) = previous.as_ref().filter(|p| p.depth == stack.len() && (p.reason != reason || p.opened)) {
                    if let Some(col) = binary_start(&p.line, in_opened && p.opened) {
                        indent = col;
                    }
                }
            }
            // Chained method calls › Align when multiline: the first `.call`
            // continuing a line under the dot of that line's trailing calls.
            if align_chains && reason == Reason::Chain && !tabs {
                if let Some(col) = previous.as_ref().filter(|p| p.depth == stack.len() && !p.opened && p.reason != Reason::Chain).and_then(|p| chain_start(&p.line)) {
                    indent = col;
                }
            }
            // Assignment statement › Align when multiline: `b = c` after
            // `a =`, `b` under `a`.
            if align_assignment && reason == Reason::Assign && !tabs && assignment_target(code_start).is_some() {
                if let Some(p) = previous.as_ref().filter(|p| p.depth == stack.len() && !p.opened && assignment_target(p.line.trim_start()).is_some()) {
                    indent = p.line.len() - p.line.trim_start().len();
                }
            }
            // Throws list / Extends list › Align when multiline: a type after
            // `,` under the first after the keyword.
            header_unaligned = indent;
            if reason == Reason::Header && flavor == Flavor::Java && (align_throws || align_extends) && !tabs {
                if let Some(p) = previous.as_ref().filter(|p| p.depth == stack.len() && p.code.ends_with(',')) {
                    match header_list_start(&p.line) {
                        Some((col, throws)) if if throws { align_throws } else { align_extends } => {
                            indent = col;
                            header_aligned = true;
                        }
                        // A later type: under the one before.
                        None if p.reason == Reason::Header && header_aligned => indent = p.indent,
                        _ => {}
                    }
                }
            }
            if reason != Reason::Header {
                header_aligned = false;
            }
            continued = reason;
            // A class header's supertypes: its body belongs to the class.
            header = reason == Reason::Header;
            // A body without braces under its `if (…)`, `else`, `for (…)`…: one indent deeper.
            if reason == Reason::None && !code_start.starts_with('{') {
                if let Some(head) = previous.as_ref().filter(|p| !p.opened && p.depth == stack.len()).and_then(|p| p.head) {
                    indent = head + step;
                    body_line = true;
                }
            }
            // Indent 'break' from 'case' off: a case body's `break` at its label's level.
            if flat_break && !body_line && reason == Reason::None && (code_start.starts_with("break;") || code_start.starts_with("break ")) && stack.last().is_some_and(|o| o.case_body) {
                indent = indent.saturating_sub(step);
            }
        }
        let body: String = if is_code_line && selected { normalize(text_line, info, lead, flavor, closes_header, &spaces) } else { text_line.trim_start().to_owned() };
        // Keep when reformatting › Comment at first column: as written
        // (Java's block comments too, with their later lines).
        let block_start = !kotlin && text_line.starts_with("/*") && text_line.find("*/").is_none_or(|end| text_line[end + 2..].trim().is_empty());
        let first_column = keep_first_column && selected && match starts {
            State::Code => text_line.starts_with("//") || block_start,
            State::Block(_) => kept_block,
            _ => false,
        };
        kept_block = match starts {
            State::Code => first_column && block_start && !text_line.contains("*/"),
            State::Block(_) => kept_block && !text_line.contains("*/"),
            _ => false,
        };
        let new_line = match starts {
            _ if first_column => (*text_line).to_owned(),
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
            let base = if header { if selected { header_unaligned } else { at }.saturating_sub(header_size) } else { at };
            // A constructor after its modifiers' own line (Wrap after modifier list): `Name(` declares too.
            let after_modifiers = flavor == Flavor::Java && previous.as_ref().is_some_and(|p| p.depth == stack.len() && modifier_list_line(&p.code));
            let declared = || declaration_paren(&line_out).or_else(|| after_modifiers.then(|| constructor_paren(&line_out)).flatten()).filter(|_| if record_header(&line_out) { align_records } else { align_params });
            let align = (flavor == Flavor::Java && !tabs).then(|| declared().or_else(|| for_paren(&line_out).filter(|_| align_for)).or_else(|| keyword_paren(&line_out, "try").filter(|_| align_resources))).flatten();
            apply_brackets(&mut stack, &placed, leading_closers, ix, at, base, cont, step, align, [align_args && !tabs, align_arrays && !tabs, align_decon && !tabs, flat_switch], &line_out, &bodies[ix], flavor == Flavor::Java, bracket_sizes);
        } else {
            apply_brackets(&mut stack, &info.brackets, leading_closers, ix, indent, indent, cont, step, None, [false; 4], text_line, &bodies[ix], flavor == Flavor::Java, bracket_sizes);
        }
        out.push(line_out);
        choppable.push(selected && is_code_line);
        if is_code_line {
            let code_text: String = text_line
                .char_indices()
                .filter(|(i, _)| !info.comment.get(*i).copied().unwrap_or(false))
                .map(|(i, c)| if info.code.get(i).copied().unwrap_or(false) { c } else { '_' })
                .collect();
            // A comment between a head and its body: the body is still to come.
            if !(body_line && code_text.trim().is_empty()) {
                let code = code_text.trim();
                let mut head = statement_head(code).then_some(indent);
                // A head over lines: `if (a &&` … `b)`.
                if let Some((at_depth, at_indent)) = split_head.filter(|(d, _)| stack.len() <= *d) {
                    split_head = None;
                    if stack.len() == at_depth && code.ends_with(')') {
                        head = Some(at_indent);
                    }
                }
                if head.is_none() && stack.len() > depth && opens_head(code) {
                    split_head = Some((depth, indent));
                }
                previous = Some(Previous { code: code_text.trim_end().to_owned(), opened: stack.len() > depth, depth: stack.len(), reason: continued, indent, head, line: out.last().cloned().unwrap_or_default() });
            }
            // A `case …:` label: what follows is its body.
            if !kotlin && (code_start.starts_with("case ") || code_start.starts_with("default")) && code_text.trim_end().ends_with(':') {
                if let Some(open) = stack.last_mut() {
                    open.case_body = true;
                }
            }
        }
    }
    if let Some(w) = wrapping.filter(|w| kotlin && (w.when_align || w.when_break)) {
        when_entries(&mut out, &mut choppable, w.when_align, w.when_break);
    }
    if let Some(w) = wrapping.filter(|w| flavor == Flavor::Java && (w.group_align.contains(&true) || w.align_simple_methods)) {
        let [fields, variables, assignments] = w.group_align;
        group_declarations(&mut out, &choppable, [fields, variables, assignments, w.align_simple_methods && w.keep_one_line.methods]);
    }
    if wrapping.is_some_and(|w| flavor == Flavor::Java && w.modifier_wrap) {
        let before = out.len();
        wrap_modifier_lists(&mut out, &mut choppable);
        let added = out.len() - before;
        // Lines it split: lay them out again, so what was aligned after the
        // modifiers (parameters under the first) follows them.
        if added > 0 && nested < 2 {
            let only = only.map(|r| r.start..r.end + added);
            let same = IndentStyle { unit: Some(unit.clone()), ..indents.clone() };
            return reformat_pass(flavor, &out.join("\n"), only, &same, nested + 1);
        }
    }
    if let Some(w) = wrapping.filter(Wrapping::any) {
        let wrapper = Wrapper { kotlin, unit: &unit, cont: pad(cont), margin: indents.right_margin.unwrap_or(RIGHT_MARGIN), w, tabs };
        let before = out.len();
        let chopped = chop_long_lines(out, choppable, &wrapper);
        let added = chopped.len() - before;
        let text = chopped.join("\n");
        // Lines it split: lay the result out once more, so what follows a
        // new continuation line (a lambda's body, the blank lines around a
        // now multi-line declaration) is indented and spaced for it, as a
        // second Reformat Code would. The selection grows by the new lines.
        if added > 0 && nested < 2 {
            let only = only.map(|r| r.start..r.end + added);
            // With this pass's indent unit: the new lines mustn't change what's detected.
            let same = IndentStyle { unit: Some(unit.clone()), ..indents.clone() };
            return reformat_pass(flavor, &text, only, &same, nested + 1);
        }
        return text;
    }
    out.join("\n")
}

/// Kotlin's 'when' statements options over laid-out lines: Align 'when'
/// branches in columns pads the selected branches' `->` to the furthest;
/// Line break after multiline 'when' entry puts a blank line between a
/// branch over lines and the next (none when a comment is between).
fn when_entries(out: &mut Vec<String>, choppable: &mut Vec<bool>, align: bool, line_break: bool) {
    struct When {
        depth: usize,
        // Per branch: its first and last line, and whether a comment line came before it.
        entries: Vec<(usize, usize, bool)>,
        comment: bool,
        // The last line at the branches' level ends the branch's head (`a,`, `->`).
        open: bool,
        arrows: Vec<(usize, usize, usize)>,
    }
    let joined = out.join("\n");
    let lines = scan(&joined, true);
    let mut stack: Vec<Option<usize>> = Vec::new();
    let mut whens: Vec<When> = Vec::new();
    let mut done: Vec<When> = Vec::new();
    for (i, (text, info)) in out.iter().zip(&lines).enumerate() {
        let depth = stack.len();
        // Byte for byte: strings and comments blanked.
        let code: String = text.char_indices().map(|(k, c)| if info.comment.get(k).copied().unwrap_or(false) { " ".repeat(c.len_utf8()) } else if info.code.get(k).copied().unwrap_or(false) { c.to_string() } else { "_".repeat(c.len_utf8()) }).collect();
        let trimmed = code.trim();
        let comment_only = trimmed.is_empty() && !text.trim().is_empty();
        if matches!(info.starts_in, Some(State::Code) | None) && (!trimmed.is_empty() || comment_only) {
            if let Some(w) = whens.last_mut().filter(|w| depth >= w.depth) {
                let lead_closer = trimmed.starts_with('}') || trimmed.starts_with(')');
                if depth == w.depth && comment_only {
                    w.comment = true;
                } else if depth == w.depth && !lead_closer {
                    let continues = w.open || ["->", ".", "?.", "?:", "&&", "||"].iter().any(|p| trimmed.starts_with(p));
                    if continues {
                        if let Some(e) = w.entries.last_mut() {
                            e.1 = i;
                        }
                    } else {
                        w.entries.push((i, i, w.comment));
                        w.comment = false;
                    }
                    // The first `->` at the line's own level.
                    if !trimmed.starts_with("->") {
                        let bytes = code.as_bytes();
                        let mut level = 0i32;
                        let mut k = 0;
                        while k + 1 < bytes.len() {
                            match bytes[k] {
                                b'(' | b'[' | b'{' => level += 1,
                                b')' | b']' | b'}' => level -= 1,
                                b'-' if level == 0 && bytes[k + 1] == b'>' => {
                                    if choppable.get(i).copied().unwrap_or(false) {
                                        w.arrows.push((i, k, text[..k].chars().count()));
                                    }
                                    break;
                                }
                                _ => {}
                            }
                            k += 1;
                        }
                    }
                    w.open = [",", "->", "&&", "||", "="].iter().any(|p| trimmed.ends_with(p));
                } else if !comment_only {
                    if let Some(e) = w.entries.last_mut() {
                        e.1 = i;
                    }
                }
            }
        }
        for (at, c) in &info.brackets {
            match c {
                '{' => {
                    let before = code.get(..*at).unwrap_or("").trim_end();
                    let head = match before.strip_suffix(')') {
                        Some(_) => {
                            let mut level = 0;
                            let mut open = None;
                            for (k, ch) in before.char_indices().rev() {
                                match ch {
                                    ')' => level += 1,
                                    '(' => {
                                        level -= 1;
                                        if level == 0 {
                                            open = Some(k);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            open.map_or("", |k| before[..k].trim_end())
                        }
                        None => before,
                    };
                    let is_when = head.ends_with("when") && !head[..head.len() - 4].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
                    if is_when {
                        whens.push(When { depth: stack.len() + 1, entries: Vec::new(), comment: false, open: false, arrows: Vec::new() });
                        stack.push(Some(whens.len() - 1));
                    } else {
                        stack.push(None);
                    }
                }
                '(' | '[' => stack.push(None),
                _ => {
                    if let Some(Some(_)) = stack.pop() {
                        if let Some(w) = whens.pop() {
                            done.push(w);
                        }
                    }
                }
            }
        }
    }
    done.extend(whens);
    if align {
        for w in &done {
            let Some(max) = w.arrows.iter().map(|a| a.2).max() else { continue };
            for &(line, at, col) in &w.arrows {
                if col < max {
                    out[line].insert_str(at, &" ".repeat(max - col));
                }
            }
        }
    }
    if line_break {
        let mut blank_before: Vec<usize> = Vec::new();
        for w in &done {
            for pair in w.entries.windows(2) {
                let ((start, end, _), (next, _, comment)) = (pair[0], pair[1]);
                if end > start && next == end + 1 && !comment && choppable.get(next).copied().unwrap_or(false) {
                    blank_before.push(next);
                }
            }
        }
        blank_before.sort_unstable();
        blank_before.dedup();
        for &line in blank_before.iter().rev() {
            out.insert(line, String::new());
            choppable.insert(line, false);
        }
    }
}

/// What `group_declarations` aligns on a line: the kind (0 a field, 1 a
/// local variable, 2 an assignment), the byte where each column starts
/// (the type for a field, then the name, then `=`; `None` when absent),
/// and whether a field has modifiers.
struct Grouped {
    kind: usize,
    columns: [Option<usize>; 3],
    modifiers: bool,
}

/// A one-line field, local variable declaration or assignment statement
/// (`in_class`: a class body's line), as `group_declarations` aligns it.
fn grouped_line(code: &str, in_class: bool) -> Option<Grouped> {
    if in_class {
        if let Some(brace) = simple_method_brace(code) {
            return Some(Grouped { kind: 3, columns: [None, None, Some(brace)], modifiers: false });
        }
    }
    const MODIFIERS: [&str; 14] = ["public", "private", "protected", "static", "final", "transient", "volatile", "abstract", "default", "synchronized", "native", "strictfp", "sealed", "non-sealed"];
    let lead = code.len() - code.trim_start().len();
    let body = code.trim_end();
    let semi = body.strip_suffix(';')?.len();
    let bytes = code.as_bytes();
    // Brackets balanced on the line; the first `=` outside them.
    let mut depth = 0i32;
    let mut eq = None;
    let mut compound = false;
    for (i, &b) in bytes.iter().enumerate().take(semi).skip(lead) {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b'=' if depth == 0 && eq.is_none() && !compound => {
                let before = if i > 0 { bytes[i - 1] } else { b' ' };
                let after = bytes.get(i + 1).copied().unwrap_or(b' ');
                if after == b'=' || matches!(before, b'=' | b'!' | b'<' | b'>') {
                    // A comparison: not a declaration's or assignment's.
                    return None;
                }
                if matches!(before, b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^') {
                    compound = true;
                } else {
                    eq = Some(i);
                }
            }
            b';' if depth == 0 => return None,
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let head_end = eq.unwrap_or(semi);
    let head = &code[lead..head_end];
    if head.contains(['(', '{']) {
        return None;
    }
    // Words outside `<…>`, with their byte offsets.
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut angle = 0i32;
    let mut start = None;
    for (i, c) in head.char_indices().chain([(head.len(), ' ')]) {
        match c {
            '<' => angle += 1,
            '>' => angle -= 1,
            ',' if angle == 0 => return None,
            _ => {}
        }
        if c.is_whitespace() && angle == 0 {
            if let Some(st) = start.take() {
                words.push((lead + st, &head[st..i]));
            }
        } else if start.is_none() && !c.is_whitespace() {
            start = Some(i);
        }
    }
    let ident = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '[' | ']')) && !w.starts_with(|c: char| c.is_ascii_digit() || c == '.' || c == '[');
    let type_like = |w: &str| ident(w.split(['<', '>', '?', ',', ' ', '&']).next().unwrap_or("")) && w.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '[' | ']' | '<' | '>' | '?' | ',' | ' ' | '&'));
    let keyword = |w: &str| matches!(w, "return" | "throw" | "yield" | "assert" | "package" | "import" | "case" | "default" | "else" | "do" | "new" | "break" | "continue" | "goto" | "var" | "this" | "super" | "instanceof");
    if !in_class && words.len() == 1 && eq.is_some() || compound && words.len() == 1 {
        // An assignment: `a = b;`, `this.a = b;`, `a[i] = b;` (`+=` and the like join the group, unaligned).
        let (_, w) = words[0];
        if in_class || !ident(w) || keyword(w) && w != "this" {
            return None;
        }
        return Some(Grouped { kind: 2, columns: [None, None, eq], modifiers: false });
    }
    if compound || words.len() < 2 {
        return None;
    }
    let (name_at, name) = *words.last()?;
    if !ident(name) || name.contains('.') || keyword(name) {
        return None;
    }
    let mods = words[..words.len() - 1].iter().take_while(|(_, w)| MODIFIERS.contains(w) || w.starts_with('@')).count();
    let types = &words[mods..words.len() - 1];
    // `var x = …` is a declaration too.
    if types.len() != 1 || !(type_like(types[0].1) || types[0].1 == "var") || keyword(types[0].1) && types[0].1 != "var" {
        return None;
    }
    if !in_class && words[..mods].iter().any(|(_, w)| *w != "final" && !w.starts_with('@')) {
        return None;
    }
    if in_class {
        Some(Grouped { kind: 0, columns: [Some(types[0].0), Some(name_at), eq], modifiers: mods > 0 })
    } else {
        Some(Grouped { kind: 1, columns: [None, Some(name_at), eq], modifiers: mods > 0 })
    }
}

/// A one-line method (`int f() { return 1; }`): its body's `{`.
fn simple_method_brace(code: &str) -> Option<usize> {
    let body = code.trim_end();
    if !body.ends_with('}') {
        return None;
    }
    let open = code.find('(')?;
    if !declaration_head(&code[..open]) {
        return None;
    }
    let mut depth = 0i32;
    let mut brace = None;
    for (i, b) in code.bytes().enumerate().skip(open) {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 && brace.is_none() => {
                brace = Some(i);
                depth += 1;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                // The body closes the line.
                if depth == 0 && brace.is_some() && i + 1 < body.len() {
                    return None;
                }
            }
            _ => {}
        }
    }
    brace.filter(|_| depth == 0)
}

/// Java's Group declarations over laid-out lines (`align`: fields,
/// variables, assignments in columns): consecutive one-line fields of a
/// class body get their types, names and `=` in columns (comment lines
/// between them don't end the group; fields with and without modifiers
/// don't share one, as IntelliJ avoids long indents); consecutive local
/// variable declarations of a block their names and `=`; consecutive
/// assignment statements their `=`. A blank line or another statement
/// ends a group.
fn group_declarations(out: &mut [String], choppable: &[bool], align: [bool; 4]) {
    let joined = out.join("\n");
    let lines = scan(&joined, false);
    let raw: Vec<&str> = out.iter().map(String::as_str).collect();
    let bodies = declaration_braces(&raw, &lines);
    // Per line: the group's key it joins (kind, depth) and its columns.
    let mut found: Vec<Option<(usize, Grouped)>> = Vec::with_capacity(out.len());
    // Per open bracket: whether it's a class body.
    let mut stack: Vec<bool> = Vec::new();
    for (ix, (text, info)) in out.iter().zip(&lines).enumerate() {
        let in_class = stack.last().copied().unwrap_or(false);
        let in_block = stack.last().is_some_and(|c| !c);
        let ends_in_code = lines.get(ix + 1).is_none_or(|l| matches!(l.starts_in, Some(State::Code) | None));
        let mut entry = None;
        if choppable.get(ix).copied().unwrap_or(false) && matches!(info.starts_in, Some(State::Code) | None) && ends_in_code && (in_class || in_block) {
            let code: String = text.char_indices().map(|(k, c)| if info.comment.get(k).copied().unwrap_or(false) { " ".repeat(c.len_utf8()) } else if info.code.get(k).copied().unwrap_or(false) { c.to_string() } else { "_".repeat(c.len_utf8()) }).collect();
            if let Some(g) = grouped_line(&code, in_class).filter(|g| align[g.kind]) {
                entry = Some((stack.len(), g));
            }
        }
        found.push(entry);
        let mut braces = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => stack.push(braces.next().copied().unwrap_or(false)),
                '(' | '[' => stack.push(false),
                _ => {
                    stack.pop();
                }
            }
        }
    }
    let mut ix = 0;
    while ix < out.len() {
        let Some((depth, first)) = &found[ix] else {
            ix += 1;
            continue;
        };
        let (depth, kind, modifiers) = (*depth, first.kind, first.modifiers);
        let mut members = vec![ix];
        let mut next = ix + 1;
        while next < out.len() {
            match &found[next] {
                Some((d, g)) if *d == depth && g.kind == kind && (kind != 0 || g.modifiers == modifiers) => {
                    members.push(next);
                    next += 1;
                }
                // Simple methods go on past blank lines (IntelliJ's aligner
                // sees only the class's members).
                None if kind == 3 && out[next].trim().is_empty() => next += 1,
                // A field group goes on past comment lines.
                None if kind == 0 && {
                    let t = out[next].trim_start();
                    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
                } => next += 1,
                _ => break,
            }
        }
        if members.len() > 1 {
            // Column by column: pad each line so the column starts where the furthest one does.
            for col in 0..3 {
                let target = members.iter().filter_map(|&m| {
                    let g = &found[m].as_ref()?.1;
                    let at = g.columns[col]?;
                    let before = out[m][..at].trim_end();
                    Some(before.chars().count() + 1)
                }).max();
                let Some(target) = target else { continue };
                for &m in &members {
                    let Some((_, g)) = found[m].as_mut() else { continue };
                    let Some(at) = g.columns[col] else { continue };
                    let line = &mut out[m];
                    let before_len = line[..at].trim_end().len();
                    let have = line[..before_len].chars().count();
                    let pad = target.saturating_sub(have).max(1);
                    let spaces = " ".repeat(pad);
                    line.replace_range(before_len..at, &spaces);
                    let shift = before_len as isize + pad as isize - at as isize;
                    for c in g.columns.iter_mut().flatten() {
                        if *c >= at {
                            *c = (*c as isize + shift) as usize;
                        }
                    }
                }
            }
        }
        ix = next.max(ix + 1);
    }
}

/// Whether a line's code (strings blanked, comments gone) is a statement's
/// head with its body on the next line: `if (…)`, `} else`, `else if (…)`,
/// `for (…)`, `while (…)`, `do`; or ends with Kotlin's `if (…)`
/// expression: `return if (…)`, `val x = if (…)`.
fn statement_head(code: &str) -> bool {
    if head_at_start(code) {
        return true;
    }
    code.match_indices("if").any(|(i, _)| {
        let before = code[..i].chars().next_back();
        i > 0 && before.is_some_and(|c| c == ' ' || c == '(' || c == '=') && head_at_start(&code[i..])
    })
}

/// Whether a line starts a statement head its line doesn't close: `if (a &&`.
fn opens_head(code: &str) -> bool {
    let closes = code.starts_with('}');
    let body = code.strip_prefix('}').map(str::trim_start).unwrap_or(code);
    let body = body.strip_prefix("else ").map(str::trim_start).unwrap_or(body);
    let word = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
    if !matches!(word, "if" | "for" | "while") || closes && word == "while" {
        return false;
    }
    let rest = body[word.len()..].trim_start();
    let mut depth = 0i32;
    for c in rest.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    rest.starts_with('(') && depth > 0
}

fn head_at_start(code: &str) -> bool {
    let closes = code.starts_with('}');
    let body = code.strip_prefix('}').map(str::trim_start).unwrap_or(code);
    let word = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
    let rest = body[word.len()..].trim_start();
    match word {
        "do" => rest.is_empty(),
        "else" => rest.is_empty() || rest.starts_with("if") && head_at_start(rest),
        // `} while (…)` ends a do-while.
        "while" if closes => false,
        "if" | "for" | "while" => {
            if !rest.starts_with('(') {
                return false;
            }
            let mut depth = 0;
            for (i, c) in rest.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1 == rest.len();
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

/// Java's Class / Method / Field / Local variable annotations (`wraps` in
/// that order): the annotations before a declaration in a class body (or
/// a class at the top), or before a local variable in a code block, on
/// lines of their own (Wrap always; Chop down if long when the line is
/// too long) or where the line would pass the margin (Wrap if long).
/// Lines with comments, enum constants and local classes stay. Kotlin:
/// classes and objects, functions and constructors, properties (local
/// ones in code blocks).
/// `single_field`: Java's Do not wrap after single annotation, for fields.
fn wrap_annotations(text: &str, kotlin: bool, wraps: &[AnnotationWrap; 4], single_field: bool, only: Option<Range<usize>>, margin: usize) -> (String, Option<Range<usize>>) {
    if wraps.iter().all(|w| *w == AnnotationWrap::DoNotWrap) {
        return (text.to_owned(), only);
    }
    let lines = scan(text, kotlin);
    let raw: Vec<&str> = text.split('\n').collect();
    let bodies = declaration_braces(&raw, &lines);
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    // Per open bracket: a class body, a code block, or `(` / `[`.
    #[derive(PartialEq)]
    enum Open {
        Class,
        Block,
        Paren,
    }
    let mut stack: Vec<Open> = Vec::new();
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let plain = info.starts_in.unwrap_or(State::Code) == State::Code && !info.comment.iter().any(|c| *c);
        let in_class = stack.last().is_none_or(|o| *o == Open::Class);
        let in_block = stack.last() == Some(&Open::Block);
        let mut parts = None;
        if selected && plain && (in_class || in_block) {
            let trimmed = line.trim_start();
            let lead = &line[..line.len() - trimmed.len()];
            let (annotations, rest) = leading_annotations(trimmed);
            // A method header over lines ends with `,`: its `(` tells.
            let kind = member_kind(rest, kotlin).map(|k| if k == Member::Field && declaration_paren(rest).is_some() { Member::Method } else { k });
            let wrap = if kotlin {
                // Kotlin's: a class or object, a function or constructor, a
                // property (a local one in a code block).
                match kind {
                    Some(Member::Property) if in_block => wraps[3],
                    _ if in_block => AnnotationWrap::DoNotWrap,
                    Some(Member::Class | Member::Object) => wraps[0],
                    Some(Member::Fun | Member::Constructor) => wraps[1],
                    Some(Member::Property) => wraps[2],
                    _ => AnnotationWrap::DoNotWrap,
                }
            } else {
                match kind {
                // A local variable: `@SuppressWarnings("x") List<T> l = …;`.
                Some(Member::Field) if in_block => wraps[3],
                _ if in_block => AnnotationWrap::DoNotWrap,
                Some(Member::Class) => wraps[0],
                Some(Member::Method) => wraps[1],
                Some(Member::Field) if single_field && annotations.len() == 1 => AnnotationWrap::DoNotWrap,
                Some(Member::Field) if !rest.trim_end().ends_with(',') => wraps[2],
                _ => AnnotationWrap::DoNotWrap,
                }
            };
            let long = width(line) > margin;
            if !annotations.is_empty() && !rest.is_empty() {
                let mut tokens: Vec<&str> = annotations.clone();
                tokens.push(rest);
                parts = match wrap {
                    AnnotationWrap::WrapAlways => Some(tokens.iter().map(|t| format!("{lead}{t}")).collect::<Vec<_>>()),
                    AnnotationWrap::ChopIfLong if long => Some(tokens.iter().map(|t| format!("{lead}{t}")).collect()),
                    AnnotationWrap::WrapIfLong if long => {
                        let mut v: Vec<String> = Vec::new();
                        let mut current = format!("{lead}{}", tokens[0]);
                        for t in &tokens[1..] {
                            let joined = format!("{current} {t}");
                            if width(&joined) > margin {
                                v.push(current);
                                current = format!("{lead}{t}");
                            } else {
                                current = joined;
                            }
                        }
                        v.push(current);
                        (v.len() > 1).then_some(v)
                    }
                    _ => None,
                };
            }
        }
        let mut braces = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => stack.push(if braces.next().copied().unwrap_or(false) { Open::Class } else { Open::Block }),
                '(' | '[' => stack.push(Open::Paren),
                _ => {
                    stack.pop();
                }
            }
        }
        match parts {
            Some(p) => {
                delta += p.len() as isize - 1;
                out.extend(p);
            }
            None => out.push((*line).to_owned()),
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// Java's Enum constants: a line of constants first in an enum's body
/// (`A, B(1), C;`) one constant a line (Wrap always; Chop down if long
/// when the line is too long) or as many as fit (Wrap if long). A
/// constant with a body (`A {`) or a comment keeps its line.
fn wrap_enum_constants(text: &str, wrap: AnnotationWrap, only: Option<Range<usize>>, margin: usize) -> (String, Option<Range<usize>>) {
    if wrap == AnnotationWrap::DoNotWrap {
        return (text.to_owned(), only);
    }
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let bodies = declaration_braces(&raw, &lines);
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    // Per open bracket: an enum body whose constants are still to come.
    let mut stack: Vec<bool> = Vec::new();
    // The last declaration keyword seen was `enum`.
    let mut enum_next = false;
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let plain = info.starts_in.unwrap_or(State::Code) == State::Code && !info.comment.iter().any(|c| *c);
        let mut parts = None;
        let trimmed = line.trim();
        if selected && plain && stack.last() == Some(&true) && info.brackets.iter().all(|(_, c)| matches!(c, '(' | ')')) && !trimmed.is_empty() {
            let lead = &line[..line.len() - line.trim_start().len()];
            let code = |i: usize| info.code.get(i).copied().unwrap_or(false);
            // The constants: commas outside brackets.
            let mut items: Vec<String> = Vec::new();
            let mut depth = 0;
            let mut start = lead.len();
            for (i, c) in line.char_indices().skip(lead.len()) {
                if !code(i) {
                    continue;
                }
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 0 => {
                        items.push(line[start..=i].trim().to_owned());
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            let last = line[start..].trim();
            if !last.is_empty() {
                items.push(last.to_owned());
            }
            let long = width(line) > margin;
            if items.len() > 1 {
                parts = match wrap {
                    AnnotationWrap::WrapAlways => Some(items.iter().map(|t| format!("{lead}{t}")).collect::<Vec<_>>()),
                    AnnotationWrap::ChopIfLong if long => Some(items.iter().map(|t| format!("{lead}{t}")).collect()),
                    AnnotationWrap::WrapIfLong if long => {
                        let mut v: Vec<String> = Vec::new();
                        let mut current = format!("{lead}{}", items[0]);
                        for t in &items[1..] {
                            let joined = format!("{current} {t}");
                            if width(&joined) > margin {
                                v.push(current);
                                current = format!("{lead}{t}");
                            } else {
                                current = joined;
                            }
                        }
                        v.push(current);
                        (v.len() > 1).then_some(v)
                    }
                    _ => None,
                };
            }
        }
        // `enum` before a body; the constants end at the body's first `;`.
        for word in trimmed.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            match word {
                "enum" => enum_next = true,
                "class" | "interface" | "record" => enum_next = false,
                _ => {}
            }
        }
        let mut braces = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => {
                    let body = braces.next().copied().unwrap_or(false);
                    stack.push(body && enum_next);
                    if body {
                        enum_next = false;
                    }
                }
                '(' | '[' => stack.push(false),
                _ => {
                    stack.pop();
                }
            }
        }
        if stack.last() == Some(&true) && line.char_indices().any(|(i, ch)| ch == ';' && info.code.get(i).copied().unwrap_or(false)) {
            if let Some(top) = stack.last_mut() {
                *top = false;
            }
        }
        match parts {
            Some(p) => {
                delta += p.len() as isize - 1;
                out.extend(p);
            }
            None => out.push((*line).to_owned()),
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// Parameter annotations (PARAMETER_ANNOTATION_WRAP): in a method's or
/// constructor's parameters, a line break after each annotation of a
/// parameter (`@NonNull String a` → `@NonNull` / `String a`); the indenter
/// aligns the parts under the parameters. Kotlin's also break before a
/// later parameter's annotations, and a declaration on one line gets its
/// parameters chopped (`fun f(` / `@A` / `x: Int` / `)`), as its official
/// style does once they span lines.
/// `single`: Java's Do not wrap after single annotation in parameters.
fn wrap_parameter_annotations(text: &str, kotlin: bool, wrap: AnnotationWrap, single: bool, only: Option<Range<usize>>, margin: usize) -> (String, Option<Range<usize>>) {
    if wrap == AnnotationWrap::DoNotWrap {
        return (text.to_owned(), only);
    }
    let lines = scan(text, kotlin);
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    // Inside a declaration's parameters: the bracket depth, whether the next
    // token starts a parameter, and the parameter's index.
    let mut params: Option<(usize, bool, usize)> = None;
    for (ix, line) in raw.iter().enumerate() {
        // Where the parts go: under the first parameter, as the indenter
        // aligns them; a line inside the parameters keeps its indent.
        let lead = line.len() - line.trim_start().len();
        let mut align = if kotlin { lead + 4 } else { lead };
        let info = &lines[ix];
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let code = |i: usize| info.code.get(i).copied().unwrap_or(false);
        let mut start = 0;
        // Kotlin's declaration with all its parameters on this line: `(` and `)`.
        let mut whole: Option<(usize, usize)> = None;
        if params.is_none() && info.starts_in.unwrap_or(State::Code) == State::Code {
            let open = if kotlin { kotlin_declaration_open(line, info) } else { declaration_paren(line).and(line.find('(')) };
            if let Some(open) = open {
                params = Some((1, true, 0));
                start = open + 1;
                if kotlin {
                    whole = paren_pair(&info.brackets).filter(|(o, _)| *o == open);
                } else {
                    align = line[..start].chars().count();
                }
            }
        }
        let mut points: Vec<usize> = Vec::new();
        // Per parameter on this line: where its annotations start and end.
        let mut annotated: Vec<(usize, usize)> = Vec::new();
        if let Some((mut depth, mut fresh, mut index)) = params {
            let bytes = line.as_bytes();
            let mut i = start;
            while i < bytes.len() {
                let c = bytes[i];
                if !code(i) {
                    i += 1;
                    continue;
                }
                if fresh && c.is_ascii_whitespace() {
                    i += 1;
                    continue;
                }
                if fresh && c == b'@' && depth == 1 {
                    fresh = false;
                    let (annotations, rest) = leading_annotations(&line[i..]);
                    if annotations.is_empty() {
                        i += 1;
                        continue;
                    }
                    let base = line.as_ptr() as usize;
                    let starts: Vec<usize> = annotations.iter().map(|a| a.as_ptr() as usize - base).chain((!rest.is_empty()).then(|| rest.as_ptr() as usize - base)).collect();
                    // Kotlin: a later parameter's annotations start a line.
                    if kotlin && index > 0 && !line[..i].trim().is_empty() {
                        points.push(i);
                    }
                    // One annotation stays with its parameter (Do not wrap after single annotation).
                    if !(single && annotations.len() == 1) {
                        points.extend(starts.iter().skip(1));
                        annotated.push((i, *starts.last().unwrap_or(&i)));
                    }
                    i = if rest.is_empty() { line.len() } else { rest.as_ptr() as usize - base };
                    continue;
                }
                fresh = false;
                match c {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    b',' if depth == 1 => {
                        fresh = true;
                        index += 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            params = (depth > 0).then_some((depth, fresh, index));
        }
        let long = width(line) > margin;
        // Kotlin's one-line declaration: chopped, when its annotations must wrap.
        if let (true, Some((open, close)), false) = (selected && kotlin && wrap == AnnotationWrap::WrapAlways, whole, annotated.is_empty()) {
            let is_code = |i: usize| code(i);
            let pad = " ".repeat(align);
            let mut parts = vec![line[..=open].trim_end().to_owned()];
            let items = list_items(line, open, close, &is_code);
            let last = items.len() - 1;
            for (n, item) in items.iter().enumerate() {
                let (annotations, rest) = leading_annotations(item);
                for a in annotations {
                    parts.push(format!("{pad}{}", a.trim()));
                }
                let comma = if n < last { "," } else { "" };
                parts.push(format!("{pad}{}{comma}", rest.trim()));
            }
            parts.push(format!("{}{}", &line[..lead], &line[close..]));
            delta += parts.len() as isize - 1;
            out.extend(parts);
            continue;
        }
        let parts = if selected && !points.is_empty() {
            let cont = " ".repeat(align);
            let mut pieces: Vec<&str> = Vec::new();
            let mut from = 0;
            for p in &points {
                pieces.push(line[from..*p].trim_end());
                from = *p;
            }
            pieces.push(&line[from..]);
            match wrap {
                AnnotationWrap::WrapAlways => Some(pieces.iter().enumerate().map(|(n, t)| if n == 0 { t.to_string() } else { format!("{cont}{t}") }).collect::<Vec<_>>()),
                AnnotationWrap::ChopIfLong if long => Some(pieces.iter().enumerate().map(|(n, t)| if n == 0 { t.to_string() } else { format!("{cont}{t}") }).collect()),
                AnnotationWrap::WrapIfLong if long => {
                    let mut v: Vec<String> = Vec::new();
                    let mut current = pieces[0].to_string();
                    for t in &pieces[1..] {
                        let joined = format!("{current} {t}");
                        if width(&joined) > margin {
                            v.push(current);
                            current = format!("{cont}{t}");
                        } else {
                            current = joined;
                        }
                    }
                    v.push(current);
                    (v.len() > 1).then_some(v)
                }
                _ => None,
            }
        } else {
            None
        };
        match parts {
            Some(p) => {
                delta += p.len() as isize - 1;
                out.extend(p);
            }
            None => out.push((*line).to_owned()),
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// The `(` of a Kotlin `fun`'s, constructor's or class's parameters on
/// this line.
fn kotlin_declaration_open(line: &str, info: &Line) -> Option<usize> {
    let open = info.brackets.iter().find(|(_, c)| *c == '(').map(|(at, _)| *at)?;
    let head = &line[..open];
    let declares = head.split_whitespace().any(|w| matches!(w, "fun" | "class" | "constructor") || w.starts_with("fun<")) || head.trim_end().ends_with("constructor");
    (declares && is_call_or_declaration(head)).then_some(open)
}

/// Java's Keep when reformatting turned off: a body opened and closed on
/// one line (`void f() { g(); }`, `() -> {}`, `if (a) { b(); }`, `class
/// A {}`) gets its own lines, a statement a line; a control statement's
/// body (`if (a) return;`) the next line. The indenter indents them after.
/// `multiple`: Keep when reformatting › Multiple expressions in one line;
/// off, a block's line of statements is split a statement a line.
fn split_one_line_bodies(text: &str, keep: KeepOneLine, multiple: bool, only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    if keep == KeepOneLine::ALL && multiple {
        return (text.to_owned(), only);
    }
    let mut text = text.to_owned();
    let mut only = only;
    // A split line can hold another body: split until nothing changes.
    for _ in 0..64 {
        let (next, next_only, changed) = split_bodies_once(&text, keep, multiple, only.clone());
        text = next;
        only = next_only;
        if !changed {
            break;
        }
    }
    (text, only)
}

#[derive(PartialEq, Clone, Copy)]
enum BodyOpen {
    Class,
    Block,
    Paren,
}

fn split_bodies_once(text: &str, keep: KeepOneLine, multiple: bool, only: Option<Range<usize>>) -> (String, Option<Range<usize>>, bool) {
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let bodies = declaration_braces(&raw, &lines);
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    let mut changed = false;
    let mut stack: Vec<BodyOpen> = Vec::new();
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let plain = info.starts_in.unwrap_or(State::Code) == State::Code && !info.comment.iter().any(|c| *c);
        let mut parts = None;
        if selected && plain {
            parts = split_one_line_body(line, info, &bodies[ix], &stack, keep);
            if parts.is_none() && !keep.control && stack.last() == Some(&BodyOpen::Block) {
                parts = split_control_statement(line, info);
            }
            if parts.is_none() && !multiple && stack.last() == Some(&BodyOpen::Block) {
                parts = split_statements(line, info);
            }
        }
        let mut braces = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => stack.push(if braces.next().copied().unwrap_or(false) { BodyOpen::Class } else { BodyOpen::Block }),
                '(' | '[' => stack.push(BodyOpen::Paren),
                _ => {
                    stack.pop();
                }
            }
        }
        match parts {
            Some(p) => {
                delta += p.len() as isize - 1;
                changed = true;
                out.extend(p);
            }
            None => out.push((*line).to_owned()),
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only, changed)
}

/// A block's line of statements (`a(); b();`) split after its first `;`
/// outside brackets that more code follows: IntelliJ's line feed between
/// statements when Multiple expressions in one line is off. A `case`
/// label's line and a line closing a brace first stay.
fn split_statements(line: &str, info: &Line) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let code = &line[lead..];
    if code.starts_with("case ") || code.starts_with("default") || code.starts_with('}') {
        return None;
    }
    let mut depth = 0i32;
    for (i, c) in line.char_indices().skip(lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            ';' if depth == 0 => {
                let rest = line[i + 1..].trim_start();
                if rest.is_empty() || rest.starts_with(['}', ';']) {
                    return None;
                }
                return Some(vec![line[..=i].to_owned(), format!("{}{rest}", &line[..lead])]);
            }
            _ => {}
        }
    }
    None
}

/// Modifier list › Wrap after modifier list: a class member's (or a
/// top-level class's) leading modifiers and annotations end their line;
/// the rest (`void f() {`, `int x;`, `class A {`) goes on the next at the
/// same indent.
/// Over laid-out lines (`choppable`: the selection's code lines), after the
/// blank lines between members are settled.
fn wrap_modifier_lists(out: &mut Vec<String>, choppable: &mut Vec<bool>) {
    const MODIFIERS: [&str; 14] = ["public", "private", "protected", "static", "final", "transient", "volatile", "abstract", "default", "synchronized", "native", "strictfp", "sealed", "non-sealed"];
    let text = out.join("\n");
    let lines = scan(&text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let bodies = declaration_braces(&raw, &lines);
    let mut wrapped: Vec<String> = Vec::with_capacity(raw.len());
    let mut selection: Vec<bool> = Vec::with_capacity(raw.len());
    let mut stack: Vec<BodyOpen> = Vec::new();
    // A member over lines: its later lines aren't members.
    let mut continued = false;
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let selected = choppable.get(ix).copied().unwrap_or(false);
        let member_level = stack.last().is_none_or(|o| *o == BodyOpen::Class);
        let mut split = None;
        if selected && member_level && !continued && matches!(info.starts_in, Some(State::Code) | None) {
            let lead = line.len() - line.trim_start().len();
            let bytes = line.as_bytes();
            let mut at = lead;
            let mut end = None;
            loop {
                let rest = &line[at..];
                if !info.code.get(at).copied().unwrap_or(false) {
                    break;
                }
                let word_len = rest.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '@' | '.'))).unwrap_or(rest.len());
                let word = &rest[..word_len];
                let mut next = at + word_len;
                if word.starts_with('@') && word.len() > 1 && word != "@interface" {
                    // An annotation, with its arguments.
                    if bytes.get(next) == Some(&b'(') {
                        let Some(close) = close_paren(line, next, &info.code) else { break };
                        next = close + 1;
                    }
                } else if !MODIFIERS.contains(&word) {
                    break;
                }
                end = Some(next);
                at = next + (line[next..].len() - line[next..].trim_start().len());
                if at >= line.len() {
                    break;
                }
            }
            if let Some(end) = end {
                let rest = line[end..].trim_start();
                let starts = rest.chars().next();
                if starts.is_some_and(|c| c.is_alphabetic() || c == '_' || c == '<') && !rest.starts_with("//") {
                    split = Some(vec![line[..end].to_owned(), format!("{}{rest}", &line[..lead])]);
                }
            }
        }
        let mut braces = bodies[ix].iter();
        for (_, c) in &info.brackets {
            match c {
                '{' => stack.push(if braces.next().copied().unwrap_or(false) { BodyOpen::Class } else { BodyOpen::Block }),
                '(' | '[' => stack.push(BodyOpen::Paren),
                _ => {
                    stack.pop();
                }
            }
        }
        // Until the member ends: `;`, `{` or `}` last on the line.
        let code_end: String = line.char_indices().filter(|(k, _)| info.code.get(*k).copied().unwrap_or(false)).map(|(_, c)| c).collect();
        let code_end = code_end.trim_end();
        continued = !(code_end.is_empty() || code_end.ends_with([';', '{', '}']) || code_end.trim_start().starts_with('@'));
        match split {
            Some(parts) => {
                selection.extend([selected; 2]);
                wrapped.extend(parts);
            }
            None => {
                selection.push(selected);
                wrapped.push((*line).to_owned());
            }
        }
    }
    *out = wrapped;
    *choppable = selection;
}

/// The first body on the line that opens and closes there and isn't kept.
fn split_one_line_body(line: &str, info: &Line, bodies: &[bool], stack: &[BodyOpen], keep: KeepOneLine) -> Option<Vec<String>> {
    let mut open: Vec<BodyOpen> = stack.to_vec();
    let mut flags = bodies.iter();
    let brackets = &info.brackets;
    let mut i = 0;
    while i < brackets.len() {
        let (pos, c) = brackets[i];
        match c {
            '{' => {
                let class = flags.next().copied().unwrap_or(false);
                let mut depth = 0;
                let mut close = None;
                for (j, &(at, b)) in brackets.iter().enumerate().skip(i) {
                    match b {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some((j, at));
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let Some((j, end)) = close else {
                    open.push(if class { BodyOpen::Class } else { BodyOpen::Block });
                    i += 1;
                    continue;
                };
                let before = line[..pos].trim_end();
                let enclosing = open.last().copied();
                let kept = if before.is_empty() || is_array_initializer(before) {
                    None
                } else if class {
                    Some(keep.classes)
                } else if before.ends_with("->") {
                    Some(keep.lambdas)
                } else if before.contains("switch (") || before.contains("switch(") {
                    None
                } else if enclosing.is_none_or(|o| o == BodyOpen::Class) {
                    Some(if member_kind(before, false) == Some(Member::Method) { keep.methods } else { keep.blocks })
                } else if enclosing == Some(BodyOpen::Block) {
                    Some(keep.blocks)
                } else {
                    None
                };
                if kept == Some(false) {
                    return Some(split_body_at(line, info, pos, end));
                }
                // Kept: past its `}`, its own bodies with it.
                for b in &brackets[i + 1..=j] {
                    if b.1 == '{' {
                        flags.next();
                    }
                }
                i = j + 1;
                continue;
            }
            '(' | '[' => open.push(BodyOpen::Paren),
            _ => {
                open.pop();
            }
        }
        i += 1;
    }
    None
}

/// `head {` / a statement a line / `}` and what follows it.
fn split_body_at(line: &str, info: &Line, open: usize, close: usize) -> Vec<String> {
    let lead = &line[..line.len() - line.trim_start().len()];
    let mut out = vec![line[..=open].trim_end().to_owned()];
    let code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    let mut depth = 0i32;
    let mut start = open + 1;
    let bytes = line.as_bytes();
    let mut i = open + 1;
    while i < close {
        if code(i) {
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' => depth -= 1,
                b'}' => {
                    depth -= 1;
                    // A nested block ends a statement unless `else` and the like go on.
                    let next = line[i + 1..close].trim_start();
                    let goes_on = ["else", "catch", "finally", "while"].iter().any(|k| next.starts_with(k)) || next.starts_with(|c: char| !(c.is_alphabetic() || c == '_' || c == '@'));
                    if depth == 0 && !next.is_empty() && !goes_on {
                        out.push(format!("{lead}    {}", line[start..=i].trim()));
                        start = i + 1;
                    }
                }
                b';' if depth == 0 => {
                    out.push(format!("{lead}    {}", line[start..=i].trim()));
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    let last = line[start..close].trim();
    if !last.is_empty() {
        out.push(format!("{lead}    {last}"));
    }
    out.push(format!("{lead}{}", line[close..].trim_end()));
    out
}

/// `if (a) return;` → `if (a)` / `return;`; `for`, `while`, `else` too.
fn split_control_statement(line: &str, info: &Line) -> Option<Vec<String>> {
    let lead_len = line.len() - line.trim_start().len();
    let mut at = lead_len;
    let mut rest = &line[at..];
    if let Some(r) = rest.strip_prefix('}') {
        at += 1 + (r.len() - r.trim_start().len());
        rest = r.trim_start();
    }
    let keyword_at = |s: &str, k: &str| s.strip_prefix(k).is_some_and(|r| r.starts_with([' ', '(']));
    let split = if keyword_at(rest, "else") && !keyword_at(rest[4..].trim_start(), "if") {
        let body = rest[4..].trim_start();
        (!body.is_empty() && !body.starts_with('{')).then(|| at + 4)
    } else {
        let mut r = rest;
        let mut a = at;
        if keyword_at(r, "else") {
            let skip = 4 + (r[4..].len() - r[4..].trim_start().len());
            a += skip;
            r = &r[skip..];
        }
        if !["if", "for", "while"].iter().any(|k| keyword_at(r, k)) {
            return None;
        }
        let paren = a + r.find('(')?;
        let mut depth = 0;
        let mut close = None;
        for (p, c) in &info.brackets {
            if *p < paren {
                continue;
            }
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(*p);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let body = line[close + 1..].trim();
        (!body.is_empty() && !body.starts_with('{') && body != ";").then_some(close + 1)
    }?;
    let lead = &line[..lead_len];
    Some(vec![line[..split].trim_end().to_owned(), format!("{lead}    {}", line[split..].trim())])
}

/// Java's Special 'else if' treatment: on, `else` alone on a line before
/// `if (…)` joins it (`} else if (…) {`); off, `else if` splits and the
/// `if` goes on the next line (the indenter steps it in).
fn else_if_layout(text: &str, special: bool, only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    let plain = |ix: usize| lines[ix].starts_in.unwrap_or(State::Code) == State::Code && !lines[ix].comment.iter().any(|c| *c);
    let selected = |ix: usize| only.as_ref().is_none_or(|r| r.contains(&ix));
    let starts_if = |t: &str| t.strip_prefix("if").is_some_and(|r| r.starts_with([' ', '(']));
    let mut ix = 0;
    while ix < raw.len() {
        let line = raw[ix];
        let trimmed = line.trim();
        if special && selected(ix) && plain(ix) && (trimmed == "else" || trimmed == "} else") && ix + 1 < raw.len() && selected(ix + 1) && plain(ix + 1) && starts_if(raw[ix + 1].trim_start()) {
            out.push(format!("{} {}", line.trim_end(), raw[ix + 1].trim()));
            delta -= 1;
            ix += 2;
            continue;
        }
        if !special && selected(ix) && plain(ix) {
            let code = |i: usize| lines[ix].code.get(i).copied().unwrap_or(false);
            let found = line.match_indices("else").find(|(i, _)| {
                let after = &line[i + 4..];
                let before_ok = *i == 0 || !line[..*i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
                let gap = after.len() - after.trim_start().len();
                before_ok && code(*i) && gap > 0 && starts_if(after.trim_start())
            });
            if let Some((i, _)) = found {
                let lead = &line[..line.len() - line.trim_start().len()];
                out.push(line[..i + 4].trim_end().to_owned());
                out.push(format!("{lead}{}", line[i + 4..].trim()));
                delta += 1;
                ix += 1;
                continue;
            }
        }
        out.push(line.to_owned());
        ix += 1;
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// The column after a `for (`'s parenthesis, when the line is one.
fn for_paren(line: &str) -> Option<usize> {
    keyword_paren(line, "for")
}

/// The column after the parenthesis of a statement starting the line with
/// `keyword (` (`for`, `try`), when something follows it.
fn keyword_paren(line: &str, keyword: &str) -> Option<usize> {
    let open = keyword_open(line, keyword)?;
    (!line[open + 1..].trim().is_empty()).then(|| line[..open + 1].chars().count())
}

/// The byte of that `(`.
fn keyword_open(line: &str, keyword: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix(keyword)?;
    let gap = rest.len() - rest.trim_start().len();
    rest.trim_start().starts_with('(').then(|| line.len() - trimmed.len() + keyword.len() + gap)
}

/// New line after '(' and Place ')' on new line for a `for (` / `try (`
/// header over lines: its first part leaves the `(` line, its `)` starts
/// one.
fn header_parens(text: &str, keyword: &str, parens: [bool; 2], only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    if parens == [false; 2] {
        return (text.to_owned(), only);
    }
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    // Per line: a split after this byte (`(`) and one before this byte (`)`).
    let mut after: Vec<Option<usize>> = vec![None; raw.len()];
    let mut before: Vec<Option<usize>> = vec![None; raw.len()];
    let selected = |ix: usize| only.as_ref().is_none_or(|r| r.contains(&ix));
    let plain = |ix: usize| lines[ix].starts_in.unwrap_or(State::Code) == State::Code && !lines[ix].comment.iter().any(|c| *c);
    for ix in 0..raw.len() {
        if !selected(ix) || !plain(ix) {
            continue;
        }
        let Some(open) = keyword_open(raw[ix], keyword) else { continue };
        // Its `)`: on a later line, or the header is on one line.
        let mut depth = 0;
        let mut close = None;
        'lines: for (j, info) in lines.iter().enumerate().skip(ix) {
            for (p, c) in &info.brackets {
                if j == ix && *p < open {
                    continue;
                }
                match c {
                    '(' | '[' | '{' => depth += 1,
                    _ => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some((j, *p));
                            break 'lines;
                        }
                    }
                }
            }
            if j > ix + 50 {
                break;
            }
        }
        let Some((cl, cp)) = close else { continue };
        if cl == ix {
            continue;
        }
        if parens[0] && !raw[ix][open + 1..].trim().is_empty() {
            after[ix] = Some(open + 1);
        }
        if parens[1] && selected(cl) && plain(cl) && !raw[cl][..cp].trim().is_empty() {
            before[cl] = Some(cp);
        }
    }
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    for (ix, line) in raw.iter().enumerate() {
        let lead = &line[..line.len() - line.trim_start().len()];
        match (after[ix], before[ix]) {
            (None, None) => out.push((*line).to_owned()),
            (a, b) => {
                let mut cuts: Vec<usize> = a.into_iter().chain(b).collect();
                cuts.sort_unstable();
                let mut from = 0;
                for c in cuts {
                    let piece = &line[from..c];
                    out.push(if from == 0 { piece.trim_end().to_owned() } else { format!("{lead}        {}", piece.trim()) });
                    from = c;
                    delta += 1;
                }
                out.push(if from == 0 { (*line).to_owned() } else { format!("{lead}{}", line[from..].trim()) });
            }
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// New line after '(' and Place ')' on new line for method call arguments
/// (`call`), declaration parameters (`decl`) and Java's parenthesized
/// expressions (`expr`), as IntelliJ's dependent line feeds do.
///
/// Java: a list whose items span lines (a declaration's single parameter,
/// or an expression: whose parentheses do) gets its first item off the `(`
/// line and its `)` on a line of its own. Kotlin's calls the same, when the
/// arguments span lines outside their lambdas and objects; Kotlin's
/// declarations keep their line breaks when on and lose them when off.
/// `array`: Java's Array initializer `{` and `}`, as the parentheses' options.
#[allow(clippy::too_many_arguments)]
fn list_parens(text: &str, kotlin: bool, call: [bool; 2], decl: [bool; 2], record: [bool; 2], expr: [bool; 2], decon: [bool; 2], array: [bool; 2], only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    let decon = if text.contains("instanceof") || text.contains("case ") { decon } else { [false; 2] };
    if !kotlin && call == [false; 2] && decl == [false; 2] && record == [false; 2] && expr == [false; 2] && decon == [false; 2] && array == [false; 2] {
        return (text.to_owned(), only);
    }
    if kotlin && call == [false; 2] && decl == [true; 2] {
        return (text.to_owned(), only);
    }
    let lines = scan(text, kotlin);
    let raw: Vec<&str> = text.split('\n').collect();
    let selected = |ix: usize| only.as_ref().is_none_or(|r| r.contains(&ix));
    let plain = |ix: usize| lines[ix].starts_in.unwrap_or(State::Code) == State::Code;
    let code = |j: usize, i: usize| lines[j].code.get(i).copied().unwrap_or(false) && !lines[j].comment.get(i).copied().unwrap_or(false);
    // Every bracket pair `(…)`: (line, byte) of each.
    let flat: Vec<(usize, usize, char)> = lines.iter().enumerate().flat_map(|(j, l)| l.brackets.iter().map(move |(p, c)| (j, *p, *c))).collect();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    // Braces that may be array initializers.
    let mut arrays: Vec<(usize, usize)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (n, &(_, _, c)) in flat.iter().enumerate() {
        if matches!(c, '(' | '[' | '{') {
            stack.push(n);
        } else if let Some(o) = stack.pop() {
            if flat[o].2 == '(' && c == ')' {
                pairs.push((o, n));
            } else if flat[o].2 == '{' && c == '}' && !kotlin && array != [false; 2] {
                arrays.push((o, n));
            }
        }
    }
    let mut cuts: Vec<Vec<usize>> = vec![Vec::new(); raw.len()];
    // Lines that join the line before them (Kotlin's declarations, off).
    let mut join: Vec<bool> = vec![false; raw.len()];
    for (o, n) in pairs {
        let ((ol, op, _), (cl, cp, _)) = (flat[o], flat[n]);
        if ol == cl || !plain(ol) || !plain(cl) {
            continue;
        }
        let before = &raw[ol][..op];
        let word = before.trim_end().rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        let keyword = matches!(word, "if" | "for" | "while" | "switch" | "synchronized" | "try" | "catch" | "when" | "assert");
        let after_close = raw[cl][cp + 1..].trim_start();
        let callee = is_call_or_declaration(before) && !annotation_paren(before) && !keyword;
        let (declares, [lparen, rparen]) = if callee {
            let declares = if kotlin { kotlin_declaration_open(raw[ol], &lines[ol]) == Some(op) } else { raw[ol].find('(') == Some(op) && declaration_head(before) };
            (declares, if declares && !kotlin && record_header(raw[ol]) { record } else if declares { decl } else if !kotlin && deconstruction(before) { decon } else { call })
        } else if !kotlin && !keyword && !before.trim_end().ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == ')' || c == ']' || c == '>') && !after_close.starts_with("->") {
            (false, expr)
        } else {
            continue;
        };
        // Kotlin's declarations: their line breaks kept (on) or joined (off).
        if kotlin && declares {
            let after = raw[ol][op + 1..].trim();
            if !lparen && after.is_empty() && selected(ol + 1) && plain(ol + 1) && ol + 1 < cl {
                join[ol + 1] = true;
            }
            let prev_comment = lines[cl - 1].comment.iter().any(|c| *c);
            if !rparen && raw[cl][..cp].trim().is_empty() && selected(cl) && !prev_comment && !(cl == ol + 1 && after.is_empty()) {
                join[cl] = true;
            }
            continue;
        }
        if !lparen && !rparen {
            continue;
        }
        // The items: top-level commas, where the first starts and the last
        // ends, and (Kotlin) whether a line ends outside lambdas and objects.
        let (mut depth, mut angle, mut commas, mut braces) = (0usize, 0usize, 0usize, 0usize);
        let (mut first, mut last): (Option<usize>, Option<usize>) = (None, None);
        let mut bare_break = false;
        let mut object = false;
        for j in ol..=cl {
            let line = raw[j];
            let from = if j == ol { op + 1 } else { 0 };
            let to = if j == cl { cp } else { line.len() };
            for (i, ch) in line[from..to].char_indices().map(|(i, ch)| (i + from, ch)) {
                if lines[j].comment.get(i).copied().unwrap_or(false) || ch.is_whitespace() {
                    continue;
                }
                if first.is_none() && kotlin && line[i..].starts_with("object") {
                    object = true;
                }
                first.get_or_insert(j);
                last = Some(j);
                if !code(j, i) {
                    continue;
                }
                match ch {
                    '{' => {
                        depth += 1;
                        braces += 1;
                    }
                    '}' => {
                        depth = depth.saturating_sub(1);
                        braces = braces.saturating_sub(1);
                    }
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth = depth.saturating_sub(1),
                    '<' if line[..i].ends_with(|p: char| p.is_alphanumeric() || p == '_') => angle += 1,
                    '>' if angle > 0 && !line[..i].ends_with('-') => angle -= 1,
                    ',' if depth == 0 && angle == 0 => commas += 1,
                    _ => {}
                }
            }
            if j < cl && braces == 0 {
                bare_break = true;
            }
        }
        let (Some(first), Some(last)) = (first, last) else { continue };
        // Java: a call's (two or more) arguments or a declaration's parameters
        // from the first to the last; one parameter or an expression: the parentheses.
        let spans = if kotlin {
            bare_break
        } else if callee {
            if commas > 0 { first != last } else { declares }
        } else {
            true
        };
        if !spans {
            continue;
        }
        let after = raw[ol][op + 1..].trim();
        if lparen && !(kotlin && object && commas == 0) && selected(ol) && !after.is_empty() && !after.starts_with("//") && !after.starts_with("/*") {
            cuts[ol].push(op + 1);
        }
        if rparen && selected(cl) && !raw[cl][..cp].trim().is_empty() {
            cuts[cl].push(cp);
        }
    }
    // Array initializers over lines: after `{` and before `}`.
    for (o, n) in arrays {
        let ((ol, op, _), (cl, cp, _)) = (flat[o], flat[n]);
        if ol == cl || !plain(ol) || !plain(cl) || !is_array_initializer(&raw[ol][..op]) {
            continue;
        }
        let after = raw[ol][op + 1..].trim();
        if array[0] && selected(ol) && !after.is_empty() && !after.starts_with("//") && !after.starts_with("/*") {
            cuts[ol].push(op + 1);
        }
        if array[1] && selected(cl) && !raw[cl][..cp].trim().is_empty() {
            cuts[cl].push(cp);
        }
    }
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    for (ix, line) in raw.iter().enumerate() {
        let mut points = std::mem::take(&mut cuts[ix]);
        points.sort_unstable();
        points.dedup();
        let lead = &line[..line.len() - line.trim_start().len()];
        let mut pieces: Vec<String> = Vec::new();
        let mut from = 0;
        if points.is_empty() {
            pieces.push((*line).to_owned());
        }
        for c in points.iter().copied().chain([line.len()]).filter(|_| !points.is_empty()) {
            let piece = line[from..c].trim();
            if from == 0 {
                pieces.push(line[..c].trim_end().to_owned());
            } else if !piece.is_empty() {
                pieces.push(format!("{lead}        {piece}"));
            }
            from = c;
        }
        if join[ix] && !out.is_empty() {
            let previous = out.pop().unwrap_or_default();
            let glue = if previous.trim_end().ends_with('(') || pieces[0].trim_start().starts_with(')') { "" } else { " " };
            pieces[0] = format!("{}{glue}{}", previous.trim_end(), pieces[0].trim_start());
            delta -= 1;
        }
        delta += pieces.len() as isize - 1;
        out.extend(pieces);
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// Java's 'for()' statement wrap: a header past the margin breaks after its
/// `;`s (Chop down if long: after each; Wrap if long: where it must), the
/// parts under the first with Align when multiline.
fn wrap_for_statements(text: &str, keyword: &str, wrap: Wrap, align: bool, only: Option<Range<usize>>, margin: usize) -> (String, Option<Range<usize>>) {
    if wrap == Wrap::DoNotWrap {
        return (text.to_owned(), only);
    }
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let selected = only.as_ref().is_none_or(|r| r.contains(&ix));
        let plain = info.starts_in.unwrap_or(State::Code) == State::Code && !info.comment.iter().any(|c| *c);
        let mut parts = None;
        if let (true, true, Some(col)) = (selected, plain, keyword_paren(line, keyword)) {
            if width(line) > margin {
                // The `;`s in the header's own parentheses.
                let open = line.len() - line.trim_start().len() + line.trim_start().find('(').unwrap_or(0);
                let mut depth = 0;
                let mut points = Vec::new();
                for (i, b) in line.bytes().enumerate().skip(open) {
                    if !info.code.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    match b {
                        b'(' | b'[' | b'{' => depth += 1,
                        b')' | b']' | b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        b';' if depth == 1 => points.push(i + 1),
                        _ => {}
                    }
                }
                if !points.is_empty() {
                    let lead = &line[..line.len() - line.trim_start().len()];
                    let cont = if align { " ".repeat(col) } else { format!("{lead}        ") };
                    let mut pieces: Vec<&str> = Vec::new();
                    let mut from = 0;
                    for p in &points {
                        pieces.push(line[from..*p].trim());
                        from = *p;
                    }
                    pieces.push(line[from..].trim());
                    pieces[0] = line[..points[0]].trim_end();
                    let v: Vec<String> = match wrap {
                        Wrap::ChopIfLong => pieces.iter().enumerate().map(|(n, t)| if n == 0 { t.to_string() } else { format!("{cont}{t}") }).collect(),
                        _ => {
                            let mut v: Vec<String> = Vec::new();
                            let mut current = pieces[0].to_string();
                            for t in &pieces[1..] {
                                let joined = format!("{current} {t}");
                                if width(&joined) > margin {
                                    v.push(current);
                                    current = format!("{cont}{t}");
                                } else {
                                    current = joined;
                                }
                            }
                            v.push(current);
                            v
                        }
                    };
                    parts = (v.len() > 1).then_some(v);
                }
            }
        }
        match parts {
            Some(p) => {
                delta += p.len() as isize - 1;
                out.extend(p);
            }
            None => out.push((*line).to_owned()),
        }
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// A line's leading annotations (`@A`, `@a.B(x = "(")`) and the rest; not
/// `@interface`.
fn leading_annotations(line: &str) -> (Vec<&str>, &str) {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(after) = rest.strip_prefix('@') {
        let mut name_len = after.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(after.len());
        // Kotlin's use-site target: `@get:Rule`, `@field:JvmField`.
        if after[name_len..].starts_with(':') && after[name_len + 1..].starts_with(|c: char| c.is_alphabetic()) {
            name_len += 1 + after[name_len + 1..].find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(after.len() - name_len - 1);
        }
        if name_len == 0 || &after[..name_len] == "interface" {
            break;
        }
        let mut end = 1 + name_len;
        if rest[end..].starts_with('(') {
            let mut depth = 0;
            let mut quote: Option<char> = None;
            let mut close = None;
            let mut escaped = false;
            for (i, c) in rest[end..].char_indices() {
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
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(end + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match close {
                Some(c) => end = c,
                None => break,
            }
        }
        out.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    (out, rest)
}

/// Java's Force braces: `if (c) x();` (Always) and `if (c)` with `x();`
/// on the next line (When multiline, Always) get `{ }`; likewise `else`,
/// `for` and `while` (`force`: if, for, while; `else` goes with if). Only
/// a one-line body that is a plain statement (not another `if`, `for`…);
/// lines with comments stay.
fn force_braces(text: &str, force: &[Force; 3], only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    if force.iter().all(|f| *f == Force::DoNotForce) {
        return (text.to_owned(), only);
    }
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let selected = |ix: usize| only.as_ref().is_none_or(|r| r.contains(&ix));
    let plain = |ix: usize| lines[ix].starts_in.unwrap_or(State::Code) == State::Code && !lines[ix].comment.iter().any(|c| *c) && lines[ix].code.first().copied().unwrap_or(true);
    // Where a statement's head ends: after `if (…)`, `for (…)`, `while (…)`, or `else`; with its option.
    let head_end = |line: &str, code: &[bool]| -> Option<(usize, Force)> {
        let trimmed = line.trim_start();
        let body = trimmed.strip_prefix("} ").unwrap_or(trimmed);
        let at = line.len() - body.len();
        let word = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        let chosen = match word {
            "if" | "else" => force[0],
            "for" => force[1],
            // `} while (…);` ends a do-while.
            "while" if !trimmed.starts_with('}') => force[2],
            _ => return None,
        };
        if word == "else" {
            let rest = body[4..].trim_start();
            if rest.starts_with("if") && !rest[2..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                // `else if (…)`: the `if`'s head.
                let open = at + body.find('(')?;
                return close_paren(line, open, code).map(|c| (c + 1, force[0]));
            }
            return Some((at + 4, chosen));
        }
        let open = at + word.len() + body[word.len()..].find('(')?;
        if !line[at + word.len()..open].trim().is_empty() {
            return None;
        }
        close_paren(line, open, code).map(|c| (c + 1, chosen))
    };
    // A body that may get braces: one statement, not a block or another statement with a body.
    let simple = |stmt: &str| {
        let stmt = stmt.trim();
        let word = stmt.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        stmt.ends_with(';') && !stmt.starts_with('{') && !matches!(word, "if" | "else" | "for" | "while" | "do" | "try" | "switch" | "synchronized")
    };
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    let mut ix = 0;
    while ix < raw.len() {
        let line = raw[ix];
        let lead = &line[..line.len() - line.trim_start().len()];
        if selected(ix) && plain(ix) {
            if let Some((end, f)) = head_end(line, &lines[ix].code) {
                let rest = line[end..].trim();
                if !rest.is_empty() && f == Force::Always && simple(rest) {
                    // `if (c) x();` → three lines.
                    out.push(format!("{} {{", line[..end].trim_end()));
                    out.push(format!("{lead}    {rest}"));
                    out.push(format!("{lead}}}"));
                    delta += 2;
                    ix += 1;
                    continue;
                }
                if rest.is_empty() && f != Force::DoNotForce && ix + 1 < raw.len() && selected(ix + 1) && plain(ix + 1) && simple(raw[ix + 1]) {
                    // `if (c)` + `x();` → the body in braces.
                    out.push(format!("{} {{", line.trim_end()));
                    out.push(raw[ix + 1].to_owned());
                    out.push(format!("{lead}}}"));
                    delta += 1;
                    ix += 2;
                    continue;
                }
            }
        }
        out.push(line.to_owned());
        ix += 1;
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// The `)` matching the `(` at `open`, strings skipped (`code`: the line's code bytes).
fn close_paren(line: &str, open: usize, code: &[bool]) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in line[open..].char_indices() {
        if !code.get(open + i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Braces placement and 'else' / 'while' / 'catch' / 'finally' on new
/// line: a `}` and the keyword after it split or joined, a declaration's
/// or statement's `{` moved to the next line or back. Lines in comments
/// or strings, and lines with a comment where they would join, stay.
/// `only`: the lines that may change, returned with its end moved by the
/// lines added or removed.
fn brace_layout(text: &str, kotlin: bool, braces: &Braces, only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    let lines = scan(text, kotlin);
    let raw: Vec<&str> = text.split('\n').collect();
    let code_of = |ix: usize| -> String {
        let info = &lines[ix];
        raw[ix]
            .char_indices()
            .filter(|(i, _)| !info.comment.get(*i).copied().unwrap_or(false))
            .map(|(i, c)| if info.code.get(i).copied().unwrap_or(false) { c } else { '_' })
            .collect::<String>()
            .trim()
            .to_owned()
    };
    let commented = |ix: usize| lines[ix].comment.iter().any(|c| *c);
    let selected = |ix: usize| only.as_ref().is_none_or(|r| r.contains(&ix));
    let in_code = |ix: usize| lines[ix].starts_in.unwrap_or(State::Code) == State::Code;
    // Whether a keyword after `}` goes on its own line; `None`: not one.
    let keyword_new_line = |rest: &str, closes_do: bool| -> Option<bool> {
        let word = rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        match word {
            // A `when` branch's `else ->` isn't one.
            "else" if !rest[4..].trim_start().starts_with("->") => Some(braces.else_new_line),
            "catch" => Some(braces.catch_new_line),
            "finally" => Some(braces.finally_new_line),
            "while" if !kotlin && closes_do => Some(braces.while_new_line),
            _ => None,
        }
    };
    // Java: a line's `{` kind: Some(next line) for a class, a method or a statement.
    let brace_next_line = |head: &str| -> Option<bool> {
        let head = head.trim();
        if kotlin {
            // Put left brace on new line: a declaration's or a statement's
            // `{`, not a lambda's (`run {`, `= foo {`).
            let bare = head.trim_start_matches('}').trim_start();
            let first = bare.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
            let control = matches!(first, "if" | "else" | "for" | "while" | "do" | "try" | "catch" | "finally" | "when");
            if control {
                return (head.ends_with(')') || matches!(bare, "else" | "try" | "finally" | "do" | "when")).then_some(braces.other_next_line);
            }
            let declares = matches!(declaration_words(bare).first().copied(), Some("class" | "interface" | "object" | "fun" | "constructor" | "init"));
            // `=` outside parentheses: an expression body or initializer.
            let mut depth = 0i32;
            let assigns = head.chars().any(|c| {
                match c {
                    '(' | '[' | '<' => depth += 1,
                    ')' | ']' | '>' => depth -= 1,
                    _ => {}
                }
                c == '=' && depth <= 0
            });
            return (declares && !assigns && !head.contains("->")).then_some(braces.other_next_line);
        }
        let first = head.trim_start_matches('}').trim_start().split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        if matches!(first, "if" | "else" | "for" | "while" | "do" | "try" | "catch" | "finally" | "switch" | "synchronized") {
            return Some(braces.other_next_line);
        }
        if head.contains("->") || head.ends_with('=') || head.ends_with(']') || head.contains("new ") {
            return None;
        }
        match declaration_words(head).first().copied() {
            Some("class" | "interface" | "@interface" | "record" | "enum") => Some(braces.class_next_line),
            _ if (head.ends_with(')') || head.contains(") throws ")) && declaration_paren(head).is_some() => Some(braces.method_next_line),
            _ => None,
        }
    };
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    // Whether each open `{` opened a `do` block.
    let mut stack: Vec<bool> = Vec::new();
    let mut ix = 0;
    while ix < raw.len() {
        let line = raw[ix];
        let code = code_of(ix);
        let lead = &line[..line.len() - line.trim_start().len()];
        let closes_do = code.starts_with('}') && stack.last().copied().unwrap_or(false);
        let mut done = false;
        if selected(ix) && in_code(ix) {
            // `} else {` on one line: split when the option says so.
            if let Some(rest) = code.strip_prefix('}').map(str::trim_start) {
                if keyword_new_line(rest, closes_do) == Some(true) && !rest.is_empty() {
                    let at = line.find('}').unwrap() + 1;
                    out.push(line[..at].trim_end().to_owned());
                    out.push(format!("{lead}{}", line[at..].trim_start()));
                    delta += 1;
                    done = true;
                }
            }
            // `}` then a line with the keyword: join when it doesn't.
            if !done && code == "}" && !commented(ix) && ix + 1 < raw.len() && selected(ix + 1) && in_code(ix + 1) {
                let next = code_of(ix + 1);
                if keyword_new_line(&next, closes_do) == Some(false) {
                    out.push(format!("{} {}", line.trim_end(), raw[ix + 1].trim()));
                    delta -= 1;
                    // The `}` line's brackets, then the next line's.
                    stack.pop();
                    ix += 1;
                    for &(_, c) in &lines[ix].brackets {
                        match c {
                            '{' => stack.push(code_of(ix).trim_start_matches('}').trim_start().starts_with("do")),
                            '}' => drop(stack.pop()),
                            _ => {}
                        }
                    }
                    ix += 1;
                    continue;
                }
            }
            // `head {`: the `{` to the next line.
            if !done && code.ends_with('{') && code.len() > 1 && !commented(ix) {
                let head = code[..code.len() - 1].trim_end();
                if brace_next_line(head) == Some(true) {
                    let at = line.rfind('{').unwrap();
                    out.push(line[..at].trim_end().to_owned());
                    out.push(format!("{lead}{{"));
                    delta += 1;
                    done = true;
                }
            }
            // A lone `{` after its head: back up when End of line.
            if !done && code == "{" && !commented(ix) && ix > 0 && selected(ix - 1) && out.last().is_some_and(|l| !l.trim().is_empty()) && !commented(ix - 1) && in_code(ix - 1) {
                let head = code_of(ix - 1);
                if !head.ends_with([';', '{', '}', ',']) && brace_next_line(&head) == Some(false) {
                    let last = out.last_mut().unwrap();
                    *last = format!("{} {{", last.trim_end());
                    delta -= 1;
                    done = true;
                }
            }
        }
        if !done {
            out.push(line.to_owned());
        }
        let opens_do = code.trim_start_matches('}').trim_start().starts_with("do") || code == "{" && ix > 0 && code_of(ix - 1).trim_start_matches('}').trim() == "do";
        for &(_, c) in &lines[ix].brackets {
            match c {
                '{' => stack.push(opens_do),
                '}' => drop(stack.pop()),
                _ => {}
            }
        }
        ix += 1;
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// Keep when reformatting › Line breaks off: a line that continues a
/// list (inside `(` or `[`) or an expression (after a binary operator or
/// `=`, or starting with `.`, `?.`, `?:`, `&&`, `||`) joins the line
/// before it. Not across comments, blank lines or strings; not a list
/// with a trailing comma (Kotlin keeps it chopped). The lines join only
/// when the result fits within `margin`. `only`: the lines that may
/// join, returned shrunk by the lines joined.
fn join_line_breaks(text: &str, kotlin: bool, only: Option<Range<usize>>, margin: usize) -> (String, Option<Range<usize>>) {
    let lines = scan(text, kotlin);
    let raw: Vec<&str> = text.split('\n').collect();
    let is_code = |ix: usize, i: usize| lines[ix].code.get(i).copied().unwrap_or(false);
    // The lists whose last item has a trailing comma: (line, byte) of `(`/`[`.
    let mut trailing = std::collections::HashSet::new();
    {
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut last = ' ';
        for (ix, line) in raw.iter().enumerate() {
            for (i, c) in line.char_indices() {
                if !is_code(ix, i) || c.is_whitespace() {
                    continue;
                }
                match c {
                    '(' | '[' | '{' => stack.push((ix, i)),
                    ')' | ']' | '}' => {
                        if let Some(open) = stack.pop() {
                            if last == ',' && c != '}' {
                                trailing.insert(open);
                            }
                        }
                    }
                    _ => {}
                }
                last = c;
            }
        }
    }
    // The code of a line: strings as `_`, comments left out, trimmed.
    let code_of = |ix: usize| -> String {
        let info = &lines[ix];
        raw[ix]
            .char_indices()
            .filter(|(i, _)| !info.comment.get(*i).copied().unwrap_or(false))
            .map(|(i, c)| if is_code(ix, i) { c } else { '_' })
            .collect::<String>()
            .trim()
            .to_owned()
    };
    // Whether each line may join the one before it.
    let mut joins = vec![false; raw.len()];
    // Open brackets at the start of the line: (char, its list has a trailing comma).
    let mut stack: Vec<(char, bool)> = Vec::new();
    for (ix, info) in lines.iter().enumerate() {
        let may = ix > 0 && only.as_ref().is_none_or(|r| r.contains(&ix) && r.contains(&(ix - 1)));
        if may {
            let prev_code = code_of(ix - 1);
            let code = code_of(ix);
            let commented = |ix: usize| lines[ix].comment.iter().any(|c| *c);
            let in_code = info.starts_in.unwrap_or(State::Code) == State::Code;
            let in_list = matches!(stack.last(), Some(('(' | '[', false)));
            let operator = ["&&", "||", "+", "-", "*", "/", "%", "="].iter().any(|op| prev_code.ends_with(op))
                && !["++", "--", "->"].iter().any(|op| prev_code.ends_with(op))
                && !["@", "import ", "package "].iter().any(|w| prev_code.starts_with(w));
            let leading = (code.starts_with('.') && !code.starts_with("..")) || ["?.", "?:", "&&", "||"].iter().any(|op| code.starts_with(op));
            joins[ix] = in_code
                && !code.is_empty()
                && !prev_code.is_empty()
                && !commented(ix)
                && !commented(ix - 1)
                && !prev_code.ends_with([';', '{', '}'])
                && !prev_code.ends_with("->")
                && (in_list && !code.starts_with('}') || operator || leading);
        }
        for &(at, c) in &info.brackets {
            match c {
                '(' | '[' | '{' => stack.push((c, trailing.contains(&(ix, at)))),
                _ => drop(stack.pop()),
            }
        }
    }
    // Each run of lines that join becomes one line, when it fits.
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut joins_in = 0;
    let mut ix = 0;
    while ix < raw.len() {
        let mut end = ix + 1;
        while end < raw.len() && joins[end] {
            end += 1;
        }
        let mut line = raw[ix].trim_end().to_owned();
        for next in &raw[ix + 1..end] {
            let next = next.trim();
            let tight = next.starts_with([')', ']', '.']) || next.starts_with("?.") || line.ends_with(['(', '[']);
            line = format!("{line}{}{next}", if tight { "" } else { " " });
        }
        if end > ix + 1 && width(&line) <= margin {
            out.push(line);
            joins_in += only.as_ref().map_or(0, |r| (ix + 1..end).filter(|i| r.contains(i)).count());
        } else {
            out.extend(raw[ix..end].iter().map(|l| (*l).to_owned()));
        }
        ix = end;
    }
    (out.join("\n"), only.map(|r| r.start..r.end.saturating_sub(joins_in)))
}

/// IntelliJ's default right margin (Code Style › General › Hard wrap at).
pub const RIGHT_MARGIN: usize = 120;

/// Kotlin's official style chops a list of arguments or parameters down
/// when it runs past the right margin (CALL_PARAMETERS_WRAP and
/// METHOD_PARAMETERS_WRAP "chop down if long", `(` and `)` on their own
/// lines, no trailing comma): the outermost such list first, then each
/// line it makes that is still long.
fn chop_long_lines(lines: Vec<String>, choppable: Vec<bool>, wrapper: &Wrapper) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut todo: Vec<(String, bool)> = lines.into_iter().zip(choppable).rev().collect();
    while let Some((line, choppable)) = todo.pop() {
        // A chop that changes nothing would come back forever.
        match choppable.then(|| chop(&line, wrapper)).flatten().filter(|parts| parts.len() > 1) {
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
/// last item ends past the margin (as the list's Wrapping option says);
/// else at its supertypes, chained calls or assignment; `None` when
/// nothing wraps.
fn chop(line: &str, wr: &Wrapper) -> Option<Vec<String>> {
    let margin = wr.margin;
    if width(line) <= margin {
        return None;
    }
    let info = scan(line, wr.kotlin).into_iter().next()?;
    // Kotlin's Elvis expressions: the outermost expression first, unless
    // that's an assignment whose value fits on a line of its own (then
    // only instead of chopping an argument list, below).
    // Kotlin's Use continuation indent, per place (off: a normal indent).
    let kc = |i: usize| if !wr.kotlin || wr.w.kotlin_cont[i] { wr.cont.as_str() } else { wr.unit };
    let assigned = wr.w.assignment != Wrap::DoNotWrap && assignment_break(line, &info, kc(2), margin, !wr.kotlin && wr.w.assign_sign_next).is_some();
    if wr.kotlin && wr.w.elvis != Wrap::DoNotWrap && !assigned {
        if let Some(parts) = elvis_break(line, &info, kc(6), margin, wr.w.elvis) {
            return Some(parts);
        }
    }
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    if !wr.kotlin && wr.w.assert_wrap != Wrap::DoNotWrap {
        if let Some(parts) = assert_break(line, &is_code, &wr.cont, wr.w.assert_colon_next) {
            return Some(parts);
        }
    }
    let first_paren = info.brackets.iter().find(|(_, c)| *c == '(').map(|(at, _)| *at);
    let java_declaration = !wr.kotlin && declaration_paren(line).is_some();
    // Whether the list opened at `open` (`depth` brackets in) is a
    // declaration's parameters.
    let declares = |open: usize, depth: usize| {
        if wr.kotlin {
            let head = &line[..open];
            depth == 0 && head.split_whitespace().any(|w| matches!(w, "fun" | "class" | "constructor" | "interface") || w.starts_with("fun<"))
        } else {
            java_declaration && Some(open) == first_paren
        }
    };
    let record = !wr.kotlin && record_header(line);
    let mode = |open: usize, depth: usize| {
        if line.as_bytes()[open] == b'{' {
            wr.w.array_init
        } else if !wr.kotlin && annotation_paren(&line[..open]) {
            wr.w.annotation_params
        } else if declares(open, depth) && record {
            wr.w.record_wrap
        } else if declares(open, depth) {
            wr.w.params
        } else if !wr.kotlin && deconstruction(&line[..open]) {
            wr.w.decon_wrap
        } else {
            wr.w.call_args
        }
    };
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
        let depth = stack.len();
        // Java keeps the first item after `(`: one item can't wrap.
        let wraps = mode(open, depth) != Wrap::DoNotWrap && (wr.kotlin || list_items(line, open, at, &is_code).len() > 1);
        // Java's Array initializer: `new int[]{…}`, `= {…}`.
        let array = !wr.kotlin && opener == '{' && c == '}' && is_array_initializer(&line[..open]);
        if (opener == '(' && c == ')' && is_call_or_declaration(&line[..open]) || array) && past && wraps && !inner.trim().is_empty() && !commented {
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
        (wr.kotlin && wr.w.params != Wrap::DoNotWrap && declares(open, 0) && !line[open + 1..close].trim().is_empty() && width(head) < margin).then_some((0, open, close))
    };
    if best.is_none() && wr.kotlin && wr.w.extends != Wrap::DoNotWrap {
        if let Some(parts) = supertype_break(line, &info, kc(4), margin) {
            return Some(parts);
        }
    }
    if best.is_none() && !wr.kotlin && wr.w.throws_keyword != Wrap::DoNotWrap {
        if let Some(at) = throws_at(line, &info) {
            let head = line[..at].trim_end();
            // Only when the header before it then fits.
            if width(head) <= margin {
                let lead = &line[..line.len() - line.trim_start().len()];
                return Some(vec![head.to_owned(), format!("{lead}{}{}", wr.cont, &line[at..])]);
            }
        }
    }
    if best.is_none() && !wr.kotlin && wr.w.throws_list != Wrap::DoNotWrap {
        if let Some(parts) = throws_break(line, &info, &wr.cont, margin, wr.w.throws_list) {
            return Some(parts);
        }
    }
    // An assignment that would chop an argument list: its Elvis instead,
    // when that leaves every line within the margin.
    if assigned && best.is_some() && wr.kotlin && wr.w.elvis != Wrap::DoNotWrap {
        if let Some(parts) = elvis_break(line, &info, kc(6), margin, wr.w.elvis).filter(|parts| parts.iter().all(|p| width(p) <= margin)) {
            return Some(parts);
        }
    }
    let continuation = if wr.kotlin { wr.unit } else { wr.cont.as_str() };
    let Some((depth, open, close)) = best.or_else(declaration) else {
        if !wr.kotlin {
            if let Some(parts) = operation_break(line, &info, continuation, margin, &wr.w.ops) {
                return Some(parts);
            }
        }
        let chain = (wr.w.chain != Wrap::DoNotWrap).then(|| chain_break(line, &info, if wr.kotlin { kc(3) } else { continuation }, margin, wr.w.wrap_first_call)).flatten();
        return chain.or_else(|| (wr.w.assignment != Wrap::DoNotWrap).then(|| assignment_break(line, &info, if wr.kotlin { kc(2) } else { continuation }, margin, !wr.kotlin && wr.w.assign_sign_next)).flatten());
    };
    let items = list_items(line, open, close, &is_code);
    let indent = &line[..line.len() - line.trim_start().len()];
    let last = items.len() - 1;
    let comma = |n: usize| if n < last { "," } else { "" };
    let fill = |parts: &mut Vec<String>, mut current: String, rest: Vec<String>, prefix: &str| {
        for item in rest {
            let joined = format!("{current} {item}");
            if current.trim().is_empty() {
                current = format!("{current}{item}");
            } else if width(&joined) > margin {
                parts.push(current);
                current = format!("{prefix}{item}");
            } else {
                current = joined;
            }
        }
        parts.push(current);
    };
    if wr.kotlin {
        // Kotlin's official style: `(` and `)` on their own lines (New line
        // after '(' and Place ')' on new line; off, the first item stays
        // after `(` and `)` after the last).
        let [lparen, rparen] = if declares(open, depth) { wr.w.param_parens } else { wr.w.call_parens };
        let prefix = format!("{indent}{}", if declares(open, depth) { kc(0) } else { kc(1) });
        let mut texts: Vec<String> = items.iter().enumerate().map(|(n, item)| format!("{item}{}", comma(n))).collect();
        if !rparen {
            if let Some(t) = texts.last_mut() {
                t.push_str(&line[close..]);
            }
        }
        let mut parts = Vec::new();
        let (start, rest) = if lparen { (prefix.clone(), texts) } else { (format!("{}{}", &line[..=open], texts[0]), texts[1..].to_vec()) };
        if lparen {
            parts.push(line[..=open].trim_end().to_owned());
        }
        match mode(open, depth) {
            Wrap::WrapIfLong => fill(&mut parts, start, rest, &prefix),
            _ if lparen => parts.extend(rest.into_iter().map(|t| format!("{prefix}{t}"))),
            _ => {
                parts.push(start);
                parts.extend(rest.into_iter().map(|t| format!("{prefix}{t}")));
            }
        }
        if rparen {
            parts.push(format!("{indent}{}", &line[close..]));
        }
        return Some(parts);
    }
    // Java: the first item stays after `(`, the rest continue (a
    // declaration's aligned under the first), `)` after the last.
    let aligned = if declares(open, depth) { if record { wr.w.align_records } else { wr.w.align_params } } else if deconstruction(&line[..open]) { wr.w.align_decon } else { wr.w.align_args };
    let prefix = if aligned && !wr.tabs { " ".repeat(line[..=open].chars().count()) } else { format!("{indent}{}", wr.cont) };
    let texts: Vec<String> = items.iter().enumerate().map(|(n, item)| if n < last { format!("{item},") } else { format!("{item}{}", &line[close..]) }).collect();
    let first = format!("{}{}", &line[..=open], texts[0]);
    let mut parts = Vec::new();
    match mode(open, depth) {
        Wrap::WrapIfLong => fill(&mut parts, first, texts[1..].to_vec(), &prefix),
        _ => {
            parts.push(first);
            parts.extend(texts[1..].iter().map(|t| format!("{prefix}{t}")));
        }
    }
    Some(parts)
}

/// Java's 'assert' statement past the margin (ASSERT_STATEMENT_WRAP): its
/// message continues the next line, after the `:` or, with Place ':' on
/// new line, with it.
fn assert_break(line: &str, is_code: &dyn Fn(usize) -> bool, cont: &str, colon_next: bool) -> Option<Vec<String>> {
    let trimmed = line.trim_start();
    let lead = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("assert")?;
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '(') {
        return None;
    }
    // Its `:`: outside brackets, not a ternary's, not `::`.
    let bytes = line.as_bytes();
    let (mut depth, mut ternary) = (0usize, 0usize);
    let mut colon = None;
    for i in line.len() - rest.len()..line.len() {
        if !is_code(i) {
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'?' if depth == 0 && bytes[i - 1] != b'<' => ternary += 1,
            b':' if bytes.get(i + 1) == Some(&b':') || i > 0 && bytes[i - 1] == b':' => {}
            b':' if depth == 0 && ternary > 0 => ternary -= 1,
            b':' if depth == 0 => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, tail) = if colon_next { (line[..colon].trim_end(), &line[colon..]) } else { (line[..=colon].trim_end(), line[colon + 1..].trim_start()) };
    (!tail.trim().is_empty()).then(|| vec![head.to_owned(), format!("{lead}{cont}{}", tail.trim_end())])
}

/// A list's items between `open` and `close`: commas outside nested
/// brackets, strings and `<…>`.
fn list_items<'a>(line: &'a str, open: usize, close: usize, is_code: &dyn Fn(usize) -> bool) -> Vec<&'a str> {
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
    items
}

/// EXTENDS_LIST_WRAP "wrap if long": a class, interface or object
/// header's supertypes (after its `:`) put on as few lines as fit, each
/// next line an indent deeper; `None` when the header has none or no
/// break helps.
/// Where a Java method header's `throws` starts (after its `) `).
fn throws_at(line: &str, info: &Line) -> Option<usize> {
    let lead = line.len() - line.trim_start().len();
    let mut depth = 0usize;
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            't' if depth == 0 && line[..i].ends_with(") ") && line[i..].starts_with("throws ") => return Some(i),
            _ => {}
        }
    }
    None
}

/// Java's Throws list (THROWS_LIST_WRAP): a method header's `throws A,
/// B, C` past the margin, broken after its commas; the first type stays
/// after `throws`, the others continue (Chop down if long: one a line;
/// Wrap if long: as many as fit).
fn throws_break(line: &str, info: &Line, cont: &str, margin: usize, mode: Wrap) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    let mut depth = 0usize;
    let mut angle = 0usize;
    let mut throws = None;
    let mut end = line.trim_end().len();
    let mut commas = Vec::new();
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !is_code(i) {
            continue;
        }
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '<' if line[..i].ends_with(|p: char| p.is_alphanumeric() || p == '_') => angle += 1,
            '>' if angle > 0 => angle -= 1,
            '{' | ';' if depth == 0 && angle == 0 => {
                end = i;
                break;
            }
            't' if depth == 0 && throws.is_none() && line[..i].ends_with(") ") && line[i..].starts_with("throws ") => throws = Some(i + "throws".len()),
            ',' if depth == 0 && angle == 0 && throws.is_some() => commas.push(i),
            _ => {}
        }
    }
    let throws = throws?;
    if commas.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    let mut start = throws;
    for comma in &commas {
        items.push(format!("{},", line[start..*comma].trim()));
        start = comma + 1;
    }
    let tail = line[end..].trim();
    let last = line[start..end].trim();
    items.push(if tail.is_empty() { last.to_owned() } else { format!("{last} {tail}") });
    if items.iter().any(|i| i.is_empty() || i.starts_with(',')) {
        return None;
    }
    let prefix = format!("{}{cont}", &line[..lead]);
    let mut parts = Vec::new();
    let mut current = format!("{} {}", line[..throws].trim_end(), items[0]);
    for item in &items[1..] {
        let joined = format!("{current} {item}");
        if mode == Wrap::ChopIfLong || width(&joined) > margin {
            parts.push(current);
            current = format!("{prefix}{item}");
        } else {
            current = joined;
        }
    }
    parts.push(current);
    (parts.len() > 1).then_some(parts)
}

fn supertype_break(line: &str, info: &Line, unit: &str, margin: usize) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let words = declaration_words(&line[lead..]);
    let declares = match words.as_slice() {
        ["class" | "interface" | "object", ..] => true,
        ["fun", "interface"] => true,
        _ => false,
    };
    if !declares {
        return None;
    }
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    // The supertypes: from the header's `:` to its `{` (or `where`, or the line's end).
    let mut depth = 0usize;
    let mut angle = 0usize;
    let mut colon = None;
    let mut end = line.trim_end().len();
    let mut commas = Vec::new();
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !is_code(i) {
            continue;
        }
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '<' if line[..i].ends_with(|p: char| p.is_alphanumeric() || p == '_') => angle += 1,
            '>' if angle > 0 && !line[..i].ends_with('-') => angle -= 1,
            '{' if depth == 0 && angle == 0 => {
                end = i;
                break;
            }
            ':' if depth == 0 && angle == 0 && colon.is_none() && !line[i + 1..].starts_with(':') && !line[..i].ends_with(':') => colon = Some(i),
            ',' if depth == 0 && angle == 0 && colon.is_some() => commas.push(i),
            _ => {}
        }
    }
    let colon = colon?;
    if let Some(at) = line[colon..end].find(" where ") {
        end = colon + at;
        commas.retain(|c| *c < end);
    }
    let mut items = Vec::new();
    let mut start = colon + 1;
    for comma in commas {
        items.push(format!("{},", line[start..comma].trim()));
        start = comma + 1;
    }
    items.push(line[start..end].trim().to_owned());
    if items.iter().any(|i| i.is_empty() || i == ",") {
        return None;
    }
    let indent = &line[..lead];
    let tail = line[end..].trim_end();
    let mut parts = Vec::new();
    let mut current = line[..=colon].to_owned();
    for item in items {
        let joined = format!("{current} {item}");
        if width(&joined) > margin {
            parts.push(current);
            current = format!("{indent}{unit}{item}");
        } else {
            current = joined;
        }
    }
    if !tail.is_empty() {
        current = format!("{current} {}", tail.trim_start());
    }
    parts.push(current);
    (parts.len() > 1).then_some(parts)
}

/// Whether a line is a class header whose supertypes go on past it
/// (`class A : B(),`).
/// A Java method header's `throws A,` or a class header's `implements A,`
/// going on to the next line.
fn java_header_list(code: &str) -> bool {
    code.ends_with(',') && (code.contains(") throws ") || matches!(declaration_words(code).as_slice(), ["class" | "interface" | "enum" | "record", ..]) && (code.contains(" implements ") || code.contains(" extends ")))
}

fn supertypes_wrapped(code: &str) -> bool {
    matches!(declaration_words(code).as_slice(), ["class" | "interface" | "object", ..] | ["fun", "interface"]) && code.contains(':') && code.ends_with(',')
}

/// METHOD_CALL_CHAIN_WRAP "wrap if long": the line broken before its last
/// chained call (`.name(` or `?.name {`, outside brackets) that starts
/// within the margin, the call an indent deeper.
/// `first`: Wrap first call, a chain's first call may be the one too
/// when another call follows it in the chain.
fn chain_break(line: &str, info: &Line, unit: &str, margin: usize, first: bool) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let mut depth = 0usize;
    let mut cut = None;
    // Calls so far in the chain at hand: the first isn't wrapped unless
    // WRAP_FIRST_METHOD_IN_CALL_CHAIN is on.
    let mut calls_before = 0;
    // That first call's dot, until a call after it shows it's a chain.
    let mut first_cut = None;
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        match c {
            '(' | '{' if depth == 0 && line[..i].trim_end().ends_with(|p: char| p.is_alphanumeric() || p == '_') => {
                calls_before += 1;
                depth += 1;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '.' if depth == 0 && i > lead => {
                let at = if line[..i].ends_with('?') { i - 1 } else { i };
                let after_receiver = line[..at].ends_with(|p: char| p.is_alphanumeric() || p == '_' || p == ')' || p == '}');
                let name: String = line[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let calls = !name.is_empty() && line[i + 1 + name.len()..].trim_start().starts_with(['(', '{']);
                if after_receiver && calls && at > lead && calls_before > 0 {
                    if let Some(f) = first_cut.take() {
                        cut = Some(f);
                    }
                }
                if after_receiver && calls && at > lead && calls_before > 0 && width(&line[..i]) < margin {
                    cut = Some(at);
                } else if first && after_receiver && calls && at > lead && calls_before == 0 && width(&line[..i]) < margin {
                    first_cut = Some(at);
                }
            }
            // Another expression starts: a new chain.
            c if depth == 0 && !(c.is_alphanumeric() || matches!(c, '_' | '?' | '!')) => {
                calls_before = 0;
                first_cut = None;
            }
            _ => {}
        }
    }
    let cut = cut?;
    let indent = &line[..lead];
    Some(vec![line[..cut].trim_end().to_owned(), format!("{indent}{unit}{}", &line[cut..])])
}

/// Java's Chained method calls › Builder methods (BUILDER_METHODS): a
/// chained call by one of `names` always wraps, as IntelliJ's
/// `ChainMethodCallsBlockBuilder` does, except the chain's first call
/// (unless Wrap first call is on and a call follows it). Only a line's
/// outermost chain, not one inside an argument.
fn wrap_builder_calls(text: &str, names: &[String], first: bool, only: Option<Range<usize>>) -> (String, Option<Range<usize>>) {
    if names.is_empty() || !names.iter().any(|n| text.contains(&format!(".{n}("))) {
        return (text.to_owned(), only);
    }
    let lines = scan(text, false);
    let raw: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut delta: isize = 0;
    for (ix, line) in raw.iter().enumerate() {
        let info = &lines[ix];
        let plain = info.starts_in.unwrap_or(State::Code) == State::Code;
        let cuts = if plain && only.as_ref().is_none_or(|r| r.contains(&ix)) { builder_cuts(line, info, names, first) } else { Vec::new() };
        if cuts.is_empty() {
            out.push((*line).to_owned());
            continue;
        }
        let lead = &line[..line.len() - line.trim_start().len()];
        let mut from = 0;
        for c in cuts {
            let piece = &line[from..c];
            out.push(if from == 0 { piece.trim_end().to_owned() } else { format!("{lead}{}", piece.trim()) });
            from = c;
            delta += 1;
        }
        out.push(format!("{lead}{}", line[from..].trim()));
    }
    let only = only.map(|r| r.start..(r.end as isize + delta).max(r.start as isize) as usize);
    (out.join("\n"), only)
}

/// Where `wrap_builder_calls` breaks `line`: before each builder call's `.`.
fn builder_cuts(line: &str, info: &Line, names: &[String], first: bool) -> Vec<usize> {
    let lead = line.len() - line.trim_start().len();
    let mut cuts = Vec::new();
    // Per bracket depth: calls so far in the chain at hand, and its first
    // call's dot when that is a builder (cut once another call follows).
    let mut chains: Vec<(usize, Option<usize>)> = vec![(usize::from(line[lead..].starts_with('.')), None)];
    for (i, c) in line.char_indices().skip(lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            continue;
        }
        let ident_before = line[..i].trim_end().ends_with(|p: char| p.is_alphanumeric() || p == '_');
        match c {
            '(' | '[' | '{' => {
                if c == '(' && ident_before {
                    if let Some(top) = chains.last_mut() {
                        top.0 += 1;
                    }
                }
                chains.push((0, None));
            }
            ')' | ']' | '}' => {
                if chains.len() > 1 {
                    chains.pop();
                }
            }
            '.' if i > lead => {
                let after_receiver = line[..i].ends_with(|p: char| p.is_alphanumeric() || p == '_' || p == ')');
                let name: String = line[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let calls = !name.is_empty() && line[i + 1 + name.len()..].trim_start().starts_with('(');
                // Only the line's own chain: one in brackets stays (the
                // indenter has no continuation inside a wrapped argument).
                let outer = chains.len() == 1;
                let Some(top) = chains.last_mut() else { continue };
                if !(after_receiver && calls && outer) {
                    continue;
                }
                if top.0 > 0 {
                    if let Some(f) = top.1.take() {
                        cuts.push(f);
                    }
                }
                if names.iter().any(|n| *n == name) {
                    if top.0 > 0 {
                        cuts.push(i);
                    } else if first {
                        top.1 = Some(i);
                    }
                }
            }
            // Another expression starts: a new chain.
            c if !(c.is_alphanumeric() || matches!(c, '_' | ' ' | '\t' | '.')) => {
                if let Some(top) = chains.last_mut() {
                    *top = (0, None);
                }
            }
            _ => {}
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

/// ASSIGNMENT_WRAP and WRAP_EXPRESSION_BODY_FUNCTIONS "wrap if long": the
/// line broken after its first `=` outside brackets, the value an indent
/// deeper, when the value then fits.
/// `sign_next`: Java's Assignment sign on next line, the `=` starting the value's line.
fn assignment_break(line: &str, info: &Line, unit: &str, margin: usize, sign_next: bool) -> Option<Vec<String>> {
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
                let sign = if sign_next { "= " } else { "" };
                let wrapped = format!("{}{unit}{sign}{value}", &line[..lead]);
                // Only where it helps: a long string stays on its line.
                if value.is_empty() || width(&wrapped) > margin {
                    return None;
                }
                let head = if sign_next { line[..i].trim_end().to_owned() } else { line[..=i].to_owned() };
                return Some(vec![head, wrapped]);
            }
            _ => {}
        }
    }
    None
}

/// Kotlin's Elvis expressions: a long line broken before its `?:`s
/// outside brackets (and strings), each starting a line one indent in.
/// Chop down breaks before each; Wrap if long where the line would pass
/// the margin.
fn elvis_break(line: &str, info: &Line, unit: &str, margin: usize, mode: Wrap) -> Option<Vec<String>> {
    let lead = line.len() - line.trim_start().len();
    let bytes = line.as_bytes();
    let mut depth = 0i32;
    // Open `${…}` templates: their braces aren't code, their insides are.
    let mut templates = 0;
    let mut cuts = Vec::new();
    for (i, c) in line.char_indices().skip_while(|(i, _)| *i < lead) {
        if !info.code.get(i).copied().unwrap_or(false) {
            if c == '{' && i > 0 && bytes[i - 1] == b'$' {
                templates += 1;
            } else if c == '}' && templates > 0 {
                templates -= 1;
            }
            continue;
        }
        if templates > 0 {
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '?' if depth == 0 && i > lead && bytes[i - 1] == b' ' && line[i..].starts_with("?: ") && info.code.get(i + 1).copied().unwrap_or(false) => cuts.push(i),
            _ => {}
        }
    }
    if cuts.is_empty() || depth != 0 {
        return None;
    }
    let mut pieces = vec![line[..cuts[0]].trim_end().to_owned()];
    for (n, at) in cuts.iter().enumerate() {
        let end = cuts.get(n + 1).copied().unwrap_or(line.len());
        pieces.push(line[*at..end].trim().to_owned());
    }
    let prefix = format!("{}{unit}", &line[..lead]);
    let mut parts: Vec<String> = Vec::new();
    let mut current = pieces[0].clone();
    for piece in &pieces[1..] {
        let joined = format!("{current} {piece}");
        if mode == Wrap::ChopIfLong || width(&joined) > margin {
            parts.push(current);
            current = format!("{prefix}{piece}");
        } else {
            current = joined;
        }
    }
    parts.push(current);
    (parts.len() > 1).then_some(parts)
}

/// Java's Binary expressions and Ternary operation: a long line broken
/// at the operators of its outermost expression (the first bracket level
/// with any, the loosest operators there: `||`, then `&&`, `+ -`,
/// `* / %`, with `? :` looser than all). Chop down breaks at each; Wrap
/// if long only where the next part would pass the margin. The sign ends
/// the line, or starts the next with Operation sign on next line.
fn operation_break(line: &str, info: &Line, unit: &str, margin: usize, ops: &Operations) -> Option<Vec<String>> {
    if ops.binary == Wrap::DoNotWrap && ops.ternary == Wrap::DoNotWrap {
        return None;
    }
    let lead = line.len() - line.trim_start().len();
    let bytes = line.as_bytes();
    let code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    // (at, len, depth, region: the open bracket's position, rank: 0 `?:`, 1 `||` … 4 `* / %`)
    let mut found: Vec<(usize, usize, usize, usize, u8)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    // `?` waiting for its `:` per bracket level.
    let mut pending: Vec<usize> = vec![0];
    let mut i = lead;
    while i < line.len() {
        let c = bytes[i];
        if !code(i) {
            i += 1;
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => {
                stack.push(i);
                pending.push(0);
            }
            b')' | b']' | b'}' => {
                stack.pop();
                if pending.len() > 1 {
                    pending.pop();
                }
            }
            _ => {}
        }
        let spaced = |len: usize| i > lead && bytes[i - 1] == b' ' && bytes.get(i + len) == Some(&b' ');
        let region = stack.last().copied().unwrap_or(0);
        let rank = match c {
            b'|' if bytes.get(i + 1) == Some(&b'|') && spaced(2) => Some((2, 1)),
            b'&' if bytes.get(i + 1) == Some(&b'&') && spaced(2) => Some((2, 2)),
            b'+' | b'-' if spaced(1) => Some((1, 3)),
            b'*' | b'/' | b'%' if spaced(1) => Some((1, 4)),
            b'?' if spaced(1) => {
                *pending.last_mut().unwrap() += 1;
                Some((1, 0))
            }
            // A ternary's `:`, not a label's, a `for (x : xs)`'s or an `assert`'s.
            b':' if spaced(1) && *pending.last().unwrap() > 0 => {
                *pending.last_mut().unwrap() -= 1;
                Some((1, 0))
            }
            _ => None,
        };
        if let Some((len, rank)) = rank {
            // Operators whose option is off don't count.
            if (if rank == 0 { ops.ternary } else { ops.binary }) != Wrap::DoNotWrap {
                found.push((i, len, stack.len(), region, rank));
            }
            i += len;
            continue;
        }
        i += 1;
    }
    // Outermost first: the lowest level, its first bracket, its loosest operators.
    let depth = found.iter().map(|f| f.2).min()?;
    let region = found.iter().find(|f| f.2 == depth)?.3;
    let in_region: Vec<_> = found.iter().filter(|f| f.2 == depth && f.3 == region).collect();
    let rank = in_region.iter().map(|f| f.4).min()?;
    let mode = if rank == 0 { ops.ternary } else { ops.binary };
    let sign_next = if rank == 0 { ops.ternary_signs_next } else { ops.binary_sign_next };
    let cuts: Vec<(usize, usize)> = in_region.iter().filter(|f| f.4 == rank).map(|f| (f.0, f.1)).collect();
    // The pieces between the cuts, each with its sign where it goes.
    let mut pieces: Vec<String> = Vec::new();
    let mut start = 0;
    for (n, (at, len)) in cuts.iter().enumerate() {
        let end = if sign_next { *at } else { at + len };
        let piece = if n == 0 { line[..end].trim_end().to_owned() } else { line[start..end].trim().to_owned() };
        pieces.push(piece);
        start = end;
    }
    pieces.push(line[start..].trim().to_owned());
    let prefix = format!("{}{unit}", &line[..lead]);
    let mut parts: Vec<String> = Vec::new();
    let mut current = pieces[0].clone();
    for piece in &pieces[1..] {
        let joined = format!("{current} {piece}");
        if mode == Wrap::ChopIfLong || width(&joined) > margin {
            parts.push(current);
            current = format!("{prefix}{piece}");
        } else {
            current = joined;
        }
    }
    parts.push(current);
    (parts.len() > 1).then_some(parts)
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
/// Whether a `(` after `before` opens a Java annotation's parameters: `@Name(`.
fn annotation_paren(before: &str) -> bool {
    let name = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
    name.ends_with('@') && name.len() < before.len()
}

/// Whether a `{` after `before` opens a Java array initializer:
/// `new int[] {`, `String[] a = {`, an annotation's `@A({` / `@A(x = {`.
fn is_array_initializer(before: &str) -> bool {
    let head = before.trim_end();
    if head.ends_with(']') && head.contains("new ") {
        return true;
    }
    if head.ends_with('=') && !head.ends_with("==") && head.contains('[') {
        return true;
    }
    if let Some(open) = head.strip_suffix('(') {
        return annotation_paren(open);
    }
    if let Some(eq) = head.strip_suffix('=').filter(|h| !h.ends_with(|c: char| matches!(c, '=' | '!' | '<' | '>'))) {
        let key = eq.trim_end().trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
        return key.trim_end().strip_suffix('(').is_some_and(annotation_paren);
    }
    false
}

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
    let messy = indents.is_empty() || odd * 20 > indents.len();
    // What a `{` at a line's end adds to the next line: a block's indent,
    // which continuation lines (wrapped arguments at 8) don't blur.
    let (mut blocks_of_two, mut blocks_of_four) = (0, 0);
    let width_of = |l: &str| l.len() - l.trim_start_matches(' ').len();
    let mut lines_iter = code.iter().filter(|l| !l.trim().is_empty()).peekable();
    while let Some(line) = lines_iter.next() {
        if line.trim_end().ends_with('{') {
            if let Some(next) = lines_iter.peek() {
                match width_of(next).checked_sub(width_of(line)) {
                    Some(2) => blocks_of_two += 1,
                    Some(4) => blocks_of_four += 1,
                    _ => {}
                }
            }
        }
    }
    // Blocks decide when there are enough of them: aligned parameters at odd
    // columns make a 2-space file look messy otherwise.
    let two = if blocks_of_two != blocks_of_four { blocks_of_two > blocks_of_four } else { !messy && odd_twos * 5 >= indents.len() };
    if two { "  ".into() } else { fallback.unwrap_or("    ").into() }
}

/// Java aligns a declaration's parameters under the first one: the column
/// after `(` when the line is a method or constructor header with a
/// parameter after it.
fn declaration_paren(line: &str) -> Option<usize> {
    let open = line.find('(')?;
    let before = &line[..open];
    let after = line[open + 1..].trim();
    if after.is_empty() || !declaration_head(before) {
        return None;
    }
    Some(line[..open + 1].chars().count())
}

/// Where a Java header's last `throws` / `extends` / `implements` list
/// starts on a laid-out line, in columns, and whether it's `throws`'.
fn header_list_start(line: &str) -> Option<(usize, bool)> {
    let info = scan(line, false).into_iter().next()?;
    let mut best: Option<(usize, bool)> = None;
    for keyword in ["throws", "extends", "implements"] {
        let mut from = 0;
        while let Some(found) = line[from..].find(keyword) {
            let at = from + found;
            let end = at + keyword.len();
            let word = (at == 0 || !line[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_')) && line[end..].starts_with(' ');
            if word && info.code.get(at).copied().unwrap_or(false) && best.is_none_or(|(b, _)| at > b) {
                best = Some((at, keyword == "throws"));
            }
            from = end;
        }
    }
    let (at, throws) = best?;
    let keyword_end = at + line[at..].find(' ')?;
    let start = line.len() - line[keyword_end..].trim_start().len();
    (start < line.len()).then(|| (line[..start].chars().count(), throws))
}

/// Where a binary expression's first operand starts on a laid-out line, in
/// columns: after the last `(` left open (`in_paren`), else after a
/// top-level `=` or `return`.
fn binary_start(line: &str, in_paren: bool) -> Option<usize> {
    let info = scan(line, false).into_iter().next()?;
    let code = |i: usize| info.code.get(i).copied().unwrap_or(false) && !info.comment.get(i).copied().unwrap_or(false);
    let bytes = line.as_bytes();
    let mut start = None;
    if in_paren {
        let mut opens: Vec<usize> = Vec::new();
        for (at, c) in &info.brackets {
            match c {
                '(' | '[' | '{' => opens.push(*at),
                _ => {
                    opens.pop();
                }
            }
        }
        start = opens.last().map(|at| at + 1);
    } else {
        let mut depth = 0usize;
        for i in 0..bytes.len() {
            if !code(i) {
                continue;
            }
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b'=' if depth == 0 && bytes.get(i + 1) != Some(&b'=') && !(i > 0 && matches!(bytes[i - 1], b'=' | b'!' | b'<' | b'>')) => start = Some(i + 1),
                _ => {}
            }
        }
        if start.is_none() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("return ") {
                start = Some(line.len() - trimmed.len() + "return".len());
            }
        }
    }
    let mut s = start?;
    while s < bytes.len() && bytes[s] == b' ' {
        s += 1;
    }
    (s < bytes.len()).then(|| line[..s].chars().count())
}

/// The column of a Java line's trailing method calls' first dot: `b.c().d()`
/// → the dot before `c` (IntelliJ aligns the chain's calls after the
/// qualifier); None when the line doesn't end in a call.
fn chain_start(line: &str) -> Option<usize> {
    let info = scan(line, false).into_iter().next()?;
    let code = |i: usize| info.code.get(i).copied().unwrap_or(false) && !info.comment.get(i).copied().unwrap_or(false);
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    while end > 0 && (!code(end - 1) || bytes[end - 1] == b' ' || bytes[end - 1] == b';') {
        end -= 1;
    }
    if line.trim_start().starts_with('.') {
        return None;
    }
    // Dots at the end's level, back to the expression's start.
    let mut dots = Vec::new();
    let mut i = end;
    let mut start = end;
    while i > 0 {
        let c = bytes[i - 1];
        if !code(i - 1) {
            break;
        }
        match c {
            b')' | b']' | b'>' if !(c == b'>' && i >= 2 && bytes[i - 2] == b'-') => {
                let open = match c { b')' => b'(', b']' => b'[', _ => b'<' };
                let mut depth = 0;
                let mut j = i;
                loop {
                    if j == 0 {
                        return None;
                    }
                    j -= 1;
                    if !code(j) {
                        continue;
                    }
                    if bytes[j] == c {
                        depth += 1;
                    } else if bytes[j] == open {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                i = j;
            }
            b'.' if !(i >= 2 && bytes[i - 2] == b'.') => {
                dots.push(i - 1);
                i -= 1;
            }
            _ if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' => i -= 1,
            _ => break,
        }
        start = i;
    }
    dots.reverse();
    // `new A.B()`: the constructor's type is the qualifier's.
    if line[..start].trim_end().ends_with("new") {
        let paren = line[start..end].find('(').map_or(end, |p| start + p);
        dots.retain(|d| *d > paren);
        return dots.first().filter(|_| dots.iter().enumerate().all(|(k, d)| line[*d..dots.get(k + 1).copied().unwrap_or(end)].ends_with(')'))).map(|d| line[..*d].chars().count());
    }
    // The qualifier comes first; then the calls.
    if dots.first().is_none_or(|d| *d == start) {
        return None;
    }
    let call = |k: usize| line[dots[k]..dots.get(k + 1).copied().unwrap_or(end)].ends_with(')');
    let mut first = None;
    for k in (0..dots.len()).rev() {
        if !call(k) {
            break;
        }
        first = Some(dots[k]);
    }
    first.map(|d| line[..d].chars().count())
}

/// A Java assignment's target when the line starts with one: `a.b[i] =`,
/// `x +=` (not a declaration's `int x =`).
fn assignment_target(code: &str) -> Option<&str> {
    let bytes = code.as_bytes();
    if !bytes.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_' || *c == b'$') {
        return None;
    }
    let mut depth = 0usize;
    for (i, c) in bytes.iter().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.checked_sub(1)?,
            b'=' if depth == 0 => {
                if bytes.get(i + 1) == Some(&b'=') {
                    return None;
                }
                let target = code[..i].trim_end().trim_end_matches(['+', '-', '*', '/', '%', '&', '|', '^']).trim_end_matches("<<").trim_end_matches(">>>").trim_end_matches(">>").trim_end();
                return (!target.is_empty() && !target.ends_with(['!', '<', '>']) && !target.contains(' ') && !["return", "throw", "yield"].contains(&target)).then_some(target);
            }
            b' ' if depth == 0 && !code[i..].trim_start().starts_with(|c: char| "=+-*/%&|^<>".contains(c)) => return None,
            _ if depth == 0 && !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'.' | b' ' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>')) => return None,
            _ => {}
        }
    }
    None
}

/// Whether the text before a Kotlin `(` names a declaration's parameters:
/// `fun name`, `class Name`, `constructor`.
/// Whether a Java `{` opens a `switch`'s body: `switch (x) {`.
fn switch_brace(before: &str) -> bool {
    let head = before.trim_end();
    let Some(head) = head.strip_suffix(')') else { return false };
    let mut depth = 1;
    for (i, c) in head.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    let word = head[..i].trim_end();
                    return word.strip_suffix("switch").is_some_and(|b| !b.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_'));
                }
            }
            _ => {}
        }
    }
    false
}

fn kotlin_declares(before: &str) -> bool {
    let word_at = |w: &str| {
        before.match_indices(w).filter(|(i, _)| *i == 0 || before[..*i].ends_with(|c: char| c == ' ' || c == '\t' || c == '(' || c == '{')).map(|(i, _)| i).last()
    };
    if before.ends_with("constructor") {
        return true;
    }
    ["fun ", "class ", "interface "].iter().filter_map(|w| word_at(w)).max().is_some_and(|i| !before[i..].contains(['(', '=', '{', ')']))
}

/// Whether the text before a Java `(` ends with a record pattern's type:
/// `o instanceof Point`, `case Point`, or a component of an outer one
/// (`case Line(Point`).
fn deconstruction(before: &str) -> bool {
    let head = before.trim_end();
    let name = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == '$');
    let type_name = &head[name.len()..];
    // Generic arguments: `Box<String>(`.
    let name = if type_name.is_empty() && head.ends_with('>') {
        let Some(open) = head.rfind('<') else { return false };
        let before_args = &head[..open];
        let n = before_args.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == '$');
        if n.len() == before_args.len() {
            return false;
        }
        if !before_args[n.len()..].starts_with(|c: char| c.is_ascii_uppercase()) {
            return false;
        }
        n
    } else {
        if !type_name.rsplit('.').next().is_some_and(|t| t.starts_with(|c: char| c.is_ascii_uppercase())) {
            return false;
        }
        name
    };
    let rest = name.trim_end();
    let word_end = |w: &str| rest.strip_suffix(w).is_some_and(|b| !b.ends_with(|c: char| c.is_alphanumeric() || c == '_')) && rest.len() < name.len();
    if word_end("instanceof") || word_end("case") {
        return true;
    }
    // A component: the outer list's `(` or a `,` in it.
    let mut depth = 0usize;
    for (i, c) in rest.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => return deconstruction(&rest[..i]),
            '(' => depth -= 1,
            ',' if depth == 0 && i + 1 == rest.len() => continue,
            ',' | ' ' | '_' | '<' | '>' | '.' | '[' | ']' => {}
            _ if c.is_alphanumeric() || c == '$' => {}
            _ => return false,
        }
        if depth == 0 && i + 1 == rest.len() && c != ',' && c != '(' {
            return false;
        }
    }
    false
}

/// Whether a Java line is a record's header: `record Name(` (modifiers and
/// annotations before it).
fn record_header(line: &str) -> bool {
    let Some(open) = line.find('(') else { return false };
    let words: Vec<&str> = line[..open].split_whitespace().collect();
    words.len() >= 2 && words[words.len() - 2] == "record"
}

/// Whether a line's text before its first `(` is a Java method's or
/// constructor's header.
/// A line of only modifiers and annotations (`public static`, `@Override public`):
/// Wrap after modifier list's first line.
fn modifier_list_line(code: &str) -> bool {
    const MODIFIERS: [&str; 14] = ["public", "private", "protected", "static", "final", "transient", "volatile", "abstract", "default", "synchronized", "native", "strictfp", "sealed", "non-sealed"];
    let words: Vec<&str> = code.split_whitespace().collect();
    words.iter().any(|w| MODIFIERS.contains(w)) && words.iter().all(|w| MODIFIERS.contains(w) || w.starts_with('@') && !w.contains('('))
}

/// `Name(` starting the line, items after it: a constructor's parameters'
/// column, as `declaration_paren`.
fn constructor_paren(line: &str) -> Option<usize> {
    let code = line.trim_start();
    let name_len = code.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))?;
    if name_len == 0 || !code.starts_with(|c: char| c.is_uppercase()) || code.as_bytes()[name_len] != b'(' {
        return None;
    }
    let open = line.len() - code.len() + name_len;
    if line[open + 1..].trim().is_empty() {
        return None;
    }
    Some(line[..open + 1].chars().count())
}

fn declaration_head(before: &str) -> bool {
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
    if before.contains('=') || ["return ", "new ", "throw ", "if ", "for ", "while ", "switch ", "catch ", "synchronized ", "case ", "yield "].iter().any(|k| before.trim_start().starts_with(k) || before.contains(&format!(" {k}"))) {
        return false;
    }
    HEADER.is_match(before)
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
    /// Java's `new X() { … }`.
    anonymous: bool,
    /// The last member: what it is, its declaration's line, whether it's an interface.
    last: Option<(Member, usize, bool)>,
    /// The last line of the last member.
    end: usize,
    /// Java: the last member's `;` or `}` hasn't come yet.
    open: bool,
    /// A comment or annotation on its own line: the next member's start.
    pending: Option<usize>,
}

/// A Kotlin `when`'s branches for "Around 'when' branches with {}".
struct WhenBranches {
    /// The bracket depth its branches are at.
    depth: usize,
    /// Whether the last branch's body is a `{ … }` block.
    last_block: Option<bool>,
    /// A comment on its own line: the next branch's start.
    pending: Option<usize>,
}

/// Whether the code before a `{` (`line[..at]`) is a `when`'s head.
fn when_head(line: &str, info: &Line, at: usize) -> bool {
    let head = line[..at].trim_end();
    let Some(start) = head.rfind("when") else { return false };
    let word = |i: usize| head[i..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
    info.code.get(start).copied().unwrap_or(false)
        && (start == 0 || !head[..start].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.'))
        && (head.len() == start + 4 || (!word(start + 4) && head.ends_with(')')))
}

/// Per line: the blank lines the code style wants before it at least,
/// for a member (from its comments and annotations) of a class body or
/// the file, and Java's `}` of a class.
fn minimum_blank_lines(raw: &[&str], lines: &[Line], bodies: &[Vec<bool>], kotlin: bool, min: &MinBlankLines) -> Vec<usize> {
    let mut need = vec![0; raw.len()];
    // Open brackets: whether each is a class body.
    let mut brackets: Vec<bool> = Vec::new();
    let mut scopes = vec![Members { depth: 0, class_body: false, interface: false, anonymous: false, last: None, end: 0, open: false, pending: None }];
    let mut whens: Vec<WhenBranches> = Vec::new();
    // Java: the depth of a method body just opened, until its first line.
    let mut method_body: Option<usize> = None;
    // Java: the last of `class`, `new`… seen, for whether a body is anonymous.
    let mut last_keyword = "";
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
        let continues = previous_code.ends_with("->") || previous_code.ends_with(|c: char| matches!(c, '=' | ',' | '(' | '.' | ':' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '?' | '!'));
        if !blank {
            if let Some(d) = method_body.take() {
                if in_code && depth == d && !code.starts_with('}') {
                    need[ix] = need[ix].max(min.before_method_body);
                }
            }
        }
        if let Some(w) = whens.last_mut().filter(|w| in_code && w.depth == depth) {
            if blank {
                w.pending = None;
            } else if code.is_empty() {
                w.pending.get_or_insert(ix);
            } else if !code.starts_with('}') && !continues {
                let block = code.contains("->") && code.ends_with('{');
                let start = w.pending.take().unwrap_or(ix);
                if w.last_block.is_some_and(|last| last || block) {
                    need[start] = need[start].max(min.around_when_branch_with_block);
                }
                w.last_block = Some(block);
            } else {
                w.pending = None;
            }
        }
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
                        None if scope.anonymous => min.after_anonymous_class_header,
                        None if scope.class_body && (!kotlin || matches!(kind, Member::Fun | Member::Property)) => min.after_class_header,
                        None => 0,
                    };
                    need[start] = need[start].max(n);
                    // Java's "Before package": after the file's header comment.
                    if !kotlin && kind == Member::Package && !scope.class_body && raw[..ix].iter().any(|l| !l.trim().is_empty()) {
                        need[ix] = need[ix].max(min.before_package);
                    }
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
        for (at, c) in &info.brackets {
            match c {
                '{' => {
                    let class_body = body.next().copied().unwrap_or(false);
                    brackets.push(class_body);
                    if class_body {
                        let interface = scopes.last().and_then(|s| s.last).is_some_and(|(_, _, interface)| interface);
                        let anonymous = !kotlin && last_declaration_keyword(text, info, *at).unwrap_or(last_keyword) == "new";
                        scopes.push(Members { depth: brackets.len(), class_body: true, interface, anonymous, last: None, end: ix, open: false, pending: None });
                    } else if kotlin && in_code && when_head(text, info, *at) {
                        whens.push(WhenBranches { depth: brackets.len(), last_block: None, pending: None });
                    } else if !kotlin {
                        let scope = scopes.last().expect("the file's scope stays");
                        let method = scope.class_body && scope.depth + 1 == brackets.len() && scope.last.is_some_and(|(k, _, _)| k == Member::Method);
                        method_body = (method && text[*at + 1..].trim().is_empty()).then_some(brackets.len());
                    }
                }
                '(' | '[' => brackets.push(false),
                _ => {
                    if whens.last().is_some_and(|w| w.depth == brackets.len()) {
                        whens.pop();
                    }
                    if brackets.pop() == Some(true) && scopes.len() > 1 {
                        scopes.pop();
                    }
                }
            }
        }
        if let Some(word) = last_declaration_keyword(text, info, text.len()) {
            last_keyword = word;
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

/// The last of `class`, `interface`, `enum`, `record` and `new` in the
/// code of `line[..end]`.
fn last_declaration_keyword(line: &str, info: &Line, end: usize) -> Option<&'static str> {
    const KEYWORDS: [&str; 5] = ["class", "interface", "enum", "record", "new"];
    let bytes = line.as_bytes();
    let mut found = None;
    let mut start = None;
    for i in 0..=end.min(bytes.len()) {
        let wordy = i < end && info.code.get(i).copied().unwrap_or(false) && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_');
        if wordy {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            let qualified = s > 0 && bytes[s - 1] == b'.';
            if let Some(k) = KEYWORDS.iter().find(|k| **k == &line[s..i]).filter(|_| !qualified) {
                found = Some(*k);
            }
        }
    }
    found
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
fn apply_brackets(stack: &mut Vec<Open>, brackets: &[(usize, char)], skip: usize, line: usize, line_indent: usize, base: usize, cont: usize, step: usize, align: Option<usize>, [align_calls, align_arrays, align_decon, flat_switch]: [bool; 4], text: &str, bodies: &[bool], java: bool, kotlin_sizes: Option<[usize; 4]>) {
    let mut base = base;
    let mut first_paren = true;
    let mut bodies = bodies.iter();
    for (at, c) in brackets.iter().skip(skip) {
        match c {
            '{' => {
                // Java's array initializer continues its line (`new int[] {`).
                let array = java && is_array_initializer(text.get(..*at).unwrap_or(""));
                let rest = text.get(at + 1..).unwrap_or("");
                // Array initializer › Align when multiline: under the first element.
                let content = if array && align_arrays && !rest.trim().is_empty() && !rest.trim_start().starts_with("//") {
                    text[..=*at].chars().count() + (rest.len() - rest.trim_start().len())
                } else if flat_switch && switch_brace(text.get(..*at).unwrap_or("")) {
                    // Indent 'case' branches off: the labels at the switch's level.
                    base
                } else {
                    base + if array { cont } else { step }
                };
                stack.push(Open { line_indent: base, content, line, case_body: false, header: false, brace: true, declarations: bodies.next().copied().unwrap_or(false) })
            }
            '(' | '[' => {
                // Java's Align when multiline for a call: under its first argument.
                let call = || {
                    let before = text.get(..*at).unwrap_or("");
                    let aligned = if java && deconstruction(before) { align_decon } else { align_calls };
                    (aligned && *c == '(' && is_call_or_declaration(before) && !annotation_paren(before) && !(text.find('(') == Some(*at) && declaration_head(before)) && !text.get(at + 1..).unwrap_or("").trim().is_empty()).then(|| text[..=*at].chars().count())
                };
                let content = match align.filter(|_| *c == '(' && first_paren).or_else(call) {
                    Some(col) => col,
                    // Kotlin's Use continuation indent: parameters, arguments, `if` conditions.
                    None => line_indent + kotlin_sizes.map_or(cont, |[params, args, conditions, context]| {
                        let before = text.get(..*at).unwrap_or("").trim_end();
                        if *c == '(' && before.strip_suffix("context").is_some_and(|b| !b.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.')) {
                            context
                        } else if *c == '(' && before.strip_suffix("if").is_some_and(|b| !b.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_')) {
                            conditions
                        } else if *c == '(' && kotlin_declares(before) {
                            params
                        } else {
                            args
                        }
                    }),
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
fn normalize(line: &str, info: &Line, lead: usize, flavor: Flavor, closes_header: bool, spaces: &Spaces) -> String {
    let kotlin = flavor == Flavor::Kotlin;
    let chars: Vec<(usize, char)> = line.char_indices().filter(|(i, _)| *i >= lead).collect();
    let is_code = |i: usize| info.code.get(i).copied().unwrap_or(false);
    let mut out = String::with_capacity(line.len() + 8);
    let mut k = 0;
    // Java ternaries whose `:` is still to come on this line.
    let mut ternaries = 0;
    // Kotlin `{`s closed later on this line: whether In simple one line
    // methods governs their spaces.
    let mut braces: Vec<bool> = Vec::new();
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
                // Spaces › After comma off; Before Parentheses off for a keyword.
                let after_comma = out.ends_with(',') && !spaces.after_comma;
                let keyword_paren = following_code && following == Some('(') && spaces.before_paren(last_word(&out), kotlin) == Some(false);
                // Before Keywords off: `}else`.
                let word: String = chars[m..].iter().take_while(|(j, ch)| is_code(*j) && ch.is_alphanumeric()).map(|(_, ch)| *ch).collect();
                let brace_keyword = out.ends_with('}') && !spaces.before_keyword(&word, kotlin);
                if m < chars.len() && !(following_code && matches!(following, Some(',' | ';' | ')' | ']'))) && !(after_open && following_code) && !(after_comma && following_code) && !keyword_paren && !brace_keyword {
                    push_space(&mut out);
                }
                k = m;
                continue;
            }
            ',' => {
                if spaces.before_comma {
                    push_space(&mut out);
                }
                out.push(',');
                if spaces.after_comma && next.is_some_and(|(j, ch)| !(is_code(j) && matches!(ch, ' ' | '\t' | ')' | ']'))) {
                    out.push(' ');
                }
            }
            '{' => {
                // Java's Before Left Brace off for this construct: `if (a){`.
                if flavor == Flavor::Java && !spaces.before_brace(&out) {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                } else {
                    let prev = out.chars().last();
                    if prev.is_some_and(|p| p.is_alphanumeric() || p == ')' || p == '_' || (p == '>' && !out.ends_with("->"))) {
                        out.push(' ');
                    }
                }
                out.push('{');
                if kotlin {
                    // `{x}` → `{ x }`, `{x ->` → `{ x ->`.
                    let mut depth = 0;
                    let mut closes = false;
                    let mut params = false;
                    for (n, (j, ch)) in chars.iter().enumerate().skip(k + 1) {
                        if !is_code(*j) {
                            continue;
                        }
                        match ch {
                            '{' => depth += 1,
                            '}' if depth == 0 => {
                                closes = true;
                                break;
                            }
                            '}' => depth -= 1,
                            '-' if depth == 0 && chars.get(n + 1).is_some_and(|(_, c)| *c == '>') => params = true,
                            _ => {}
                        }
                    }
                    let mut m = k + 1;
                    while chars.get(m).is_some_and(|(j, ch)| is_code(*j) && matches!(ch, ' ' | '\t')) {
                        m += 1;
                    }
                    let empty = chars.get(m).is_none_or(|(j, ch)| is_code(*j) && *ch == '}');
                    if (closes || params) && !empty {
                        let governed = simple_brace(&out[..out.len() - 1]);
                        if closes {
                            braces.push(governed);
                        }
                        if spaces.simple_braces || !governed {
                            out.push(' ');
                        }
                        k = m;
                        continue;
                    } else if closes {
                        braces.push(true);
                    }
                }
            }
            // Java's ternary `a ? b : c` (not a wildcard `<?`, `? extends`).
            '?' if flavor == Flavor::Java && ternary_question(&chars, k, &is_code, &out) => {
                ternaries += 1;
                let [before, after, _, _] = spaces.ternary;
                spaced_operator(&mut out, &chars, &mut k, &is_code, "?", before, after);
                continue;
            }
            ':' if flavor == Flavor::Java && ternaries > 0 && !out.ends_with(':') && next.is_none_or(|(_, ch)| ch != ':') => {
                ternaries -= 1;
                let [_, _, before, after] = spaces.ternary;
                spaced_operator(&mut out, &chars, &mut k, &is_code, ":", before, after);
                continue;
            }
            '}' => {
                if let Some(governed) = braces.pop().filter(|_| kotlin && !out.trim_end().ends_with('{')) {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    if spaces.simple_braces || !governed {
                        out.push(' ');
                    }
                }
                out.push('}');
                // `}else`, `}catch`, `}finally`, `}while`.
                let word: String = chars[k + 1..].iter().take_while(|(j, ch)| is_code(*j) && ch.is_alphanumeric()).map(|(_, ch)| *ch).collect();
                if next.is_some_and(|(j, ch)| is_code(j) && ch.is_alphabetic()) && spaces.before_keyword(&word, kotlin) {
                    out.push(' ');
                }
            }
            '(' => {
                // `if(` → `if (`.
                let word = last_word(&out);
                let before_word = out.len() - word.len();
                let boundary = out[..before_word].chars().last().is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '.'));
                if spaces.before_paren(word, kotlin) == Some(true) && boundary {
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
                    // A new type's colon (`class A : B`, `<T : Any>`) or a declaration's.
                    let new_type = spaced_colon(&out, closes_header);
                    let (before, after) = if new_type { (spaces.colons[2], spaces.colons[3]) } else { (spaces.colons[0], spaces.colons[1]) };
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    if before {
                        push_space(&mut out);
                    }
                    out.push(':');
                    if !after {
                        while chars.get(k + 1).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                            k += 1;
                        }
                    } else if next.is_some_and(|(j, ch)| !(is_code(j) && ch == ' ')) {
                        out.push(' ');
                    }
                }
            }
            '<' | '>' if comparison(&chars, k, &is_code) => {
                let op = if chars.get(k + 1).is_some_and(|(j, ch)| is_code(*j) && *ch == '=') { if c == '<' { "<=" } else { ">=" } } else if c == '<' { "<" } else { ">" };
                let spaced = spaces.around(op);
                while out.ends_with(' ') {
                    out.pop();
                }
                if spaced {
                    out.push(' ');
                }
                out.push_str(op);
                k += op.len();
                while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                    k += 1;
                }
                if k < chars.len() && spaced {
                    out.push(' ');
                }
                continue;
            }
            // `->`: a lambda's, a `when` branch's, a function type's.
            '-' if flavor != Flavor::Groovy && next_code(1) == Some('>') && !out.ends_with('-') => {
                let (before, after) = arrow_spaces(&out, kotlin, spaces);
                while out.ends_with(' ') {
                    out.pop();
                }
                if before && !out.is_empty() {
                    out.push(' ');
                }
                out.push_str("->");
                k += 2;
                while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                    k += 1;
                }
                if k < chars.len() && after {
                    out.push(' ');
                }
                continue;
            }
            // Kotlin's `..` / `..<`.
            '.' if kotlin && next_code(1) == Some('.') && !out.ends_with('.') => {
                let op = if next_code(2) == Some('<') { "..<" } else { ".." };
                spaced_operator(&mut out, &chars, &mut k, &is_code, op, spaces.around_range, spaces.around_range);
                continue;
            }
            // Prefix `!`, signs, `++` / `--` (Java's `~` too).
            '!' | '+' | '-' | '~' if flavor != Flavor::Groovy && (c != '~' || !kotlin) && !(!kotlin && out.is_empty() && next_code(1) == Some(' ')) && let Some((op, prefix)) = unary(&chars, k, &is_code, &out, kotlin) => {
                if prefix {
                    out.push_str(op);
                    k += op.len();
                    while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                        k += 1;
                    }
                    // Never `- -x` into `--x`.
                    let joins = chars.get(k).is_some_and(|(j, ch)| is_code(*j) && op.ends_with(*ch));
                    if (spaces.around_unary || joins) && k < chars.len() {
                        out.push(' ');
                    }
                } else {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    if spaces.around_unary {
                        out.push(' ');
                    }
                    out.push_str(op);
                    k += op.len();
                }
                continue;
            }
            '+' | '-' | '*' | '/' | '%' if flavor != Flavor::Groovy && arithmetic(&chars, k, &is_code, &out) => {
                // A binary `a + b`: spaced or not as the setting says, but
                // never so that `a - -b` runs together into `--`.
                let spaced = spaces.around(&c.to_string());
                while out.ends_with(' ') {
                    out.pop();
                }
                if spaced || out.ends_with(c) {
                    out.push(' ');
                }
                out.push(c);
                k += 1;
                while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                    k += 1;
                }
                let joins_after = chars.get(k).is_some_and(|(j, ch)| is_code(*j) && (*ch == c || (c == '-' && *ch == '>') || (c == '/' && matches!(*ch, '/' | '*'))));
                if k < chars.len() && (spaced || joins_after) {
                    out.push(' ');
                }
                continue;
            }
            '=' | '!' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '<' | '>' => {
                let op = operator(&chars, k, &is_code);
                match op {
                    Some(op) if !spaces.around(op) => {
                        while out.ends_with(' ') {
                            out.pop();
                        }
                        out.push_str(op);
                        k += op.chars().count();
                        while chars.get(k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
                            k += 1;
                        }
                        continue;
                    }
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
    if flavor == Flavor::Java { java_within(&out, spaces) } else { out }
}

/// Java's Within options on a normalized line: a space inside each pair
/// of parentheses, brackets and array initializer braces opened and
/// closed on it whose option is on, none when it is off (IntelliJ's
/// `SPACE_WITHIN_*`).
fn java_within(line: &str, spaces: &Spaces) -> String {
    let chars: Vec<char> = line.chars().collect();
    // The code characters: not in strings, chars or comments.
    let mut code = vec![true; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' => {
                let q = chars[i];
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != q {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                for c in code.iter_mut().take((i + 1).min(chars.len())).skip(start) {
                    *c = false;
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                for c in code.iter_mut().skip(i) {
                    *c = false;
                }
                break;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let start = i;
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 1;
                for c in code.iter_mut().take((i + 1).min(chars.len())).skip(start) {
                    *c = false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // Each opening's kind, then each pair's: whether a space goes inside.
    let mut open: Vec<(usize, Option<Within>)> = Vec::new();
    let mut after_open: Vec<Option<bool>> = vec![None; chars.len()];
    let mut before_close: Vec<Option<bool>> = vec![None; chars.len()];
    let mut last_closed: Option<Within> = None;
    // Type arguments and parameters: `List< String >`, not `a < b`.
    for (i, &c) in chars.iter().enumerate() {
        if c != '<' || !code[i] || i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '_' | '.' | ' ')) {
            continue;
        }
        if chars[i - 1] == ' ' {
            // `public <T> T f(`: after a modifier or at the start.
            let before: String = chars[..i].iter().collect();
            if !matches!(last_word(before.trim_end()), "public" | "private" | "protected" | "static" | "final" | "abstract" | "synchronized" | "default" | "native") {
                continue;
            }
        }
        let mut depth = 0;
        let mut close = None;
        for (j, &d) in chars.iter().enumerate().skip(i) {
            match d {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ if d.is_alphanumeric() || matches!(d, '_' | ' ' | ',' | '.' | '?' | '&' | '[' | ']' | '@' | '$') => {}
                _ => break,
            }
        }
        let Some(close) = close else { continue };
        if chars[i + 1..close].windows(2).any(|w| w == ['&', '&']) {
            continue;
        }
        if chars[i + 1..close].iter().all(|c| *c == ' ') {
            // A diamond stays `<>`.
            after_open[i] = Some(false);
            before_close[close] = Some(false);
            continue;
        }
        // `a < b > c` isn't one: a type starts with a letter or `?`.
        let first = chars[i + 1..close].iter().find(|c| **c != ' ');
        if first.is_some_and(|f| f.is_alphabetic() || *f == '?' || *f == '@') {
            let on = spaces.within[Within::AngleBrackets as usize];
            after_open[i] = Some(on);
            before_close[close] = Some(on);
        }
    }
    for (i, &c) in chars.iter().enumerate() {
        if !code[i] || !matches!(c, '(' | '[' | '{' | ')' | ']' | '}') {
            continue;
        }
        let before: String = chars[..i].iter().collect();
        let before = before.trim_end();
        match c {
            '(' => {
                let word = last_word(before);
                let previous = last_word(before[..before.len() - word.len()].trim_end());
                let enclosing = open.last().and_then(|(_, k)| *k);
                let pattern = word.starts_with(|c: char| c.is_uppercase()) && (matches!(previous, "instanceof" | "case") || enclosing == Some(Within::Deconstruction));
                let kind = if previous == "record" {
                    Within::RecordHeader
                } else if pattern {
                    Within::Deconstruction
                } else {
                    paren_kind(before)
                };
                open.push((i, Some(kind)));
            }
            '[' => open.push((i, before.ends_with(|p: char| p.is_alphanumeric() || matches!(p, '_' | ')' | ']')).then_some(Within::Brackets))),
            '{' => {
                let enclosing = open.last().and_then(|(_, k)| *k);
                let initializer = before.ends_with('=') || before.ends_with(']') || (before.ends_with([',', '{', '(']) && matches!(enclosing, Some(Within::ArrayInitializer | Within::Annotation)));
                // A code block: after a statement's or method's `)` (not a
                // record's header), a lambda's `->`, `else`, `try`, ….
                let block = before.ends_with(')') && !matches!(last_closed, Some(Within::RecordHeader | Within::Annotation)) || before.ends_with("->") || matches!(last_word(before), "else" | "try" | "finally" | "do" | "static");
                open.push((i, if initializer { Some(Within::ArrayInitializer) } else if block { Some(Within::CodeBraces) } else { None }));
            }
            ')' | ']' | '}' => {
                let Some((at, kind)) = open.pop() else { continue };
                last_closed = kind;
                let Some(mut kind) = kind else { continue };
                let empty = chars[at + 1..i].iter().all(|c| *c == ' ');
                if kind == Within::CodeBraces {
                    // Off (with Spaces inside block braces off too): kept as
                    // written, as with array initializers.
                    let on = spaces.within[Within::CodeBraces as usize] || !empty && spaces.within[Within::BlockBody as usize];
                    if on {
                        after_open[at] = Some(true);
                        before_close[i] = Some(true);
                    }
                    continue;
                }
                if kind == Within::Cast {
                    // `(Type) x`: a cast only when an operand follows.
                    let next = chars[i + 1..].iter().find(|c| **c != ' ');
                    let inside: String = chars[at + 1..i].iter().collect();
                    let inside = inside.trim();
                    let primitive = matches!(inside.trim_end_matches("[]"), "int" | "long" | "short" | "byte" | "char" | "float" | "double" | "boolean");
                    let simple = inside.split('<').next().unwrap_or("").trim_end_matches("[]");
                    let named = simple.rsplit('.').next().is_some_and(|n| n.starts_with(|c: char| c.is_uppercase())) && simple.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$'));
                    if !(primitive || named && next.is_some_and(|n| n.is_alphanumeric() || matches!(n, '_' | '(' | '"' | '\'' | '!' | '~'))) {
                        kind = Within::Grouping;
                    }
                }
                if empty {
                    kind = match kind {
                        Within::MethodCall => Within::EmptyMethodCall,
                        Within::MethodDeclaration => Within::EmptyMethodDeclaration,
                        Within::ArrayInitializer => Within::EmptyArrayInitializer,
                        _ => continue,
                    };
                }
                // A lambda's parameters (`(a, b) ->`) aren't grouping.
                if kind == Within::Grouping && chars[i + 1..].iter().collect::<String>().trim_start().starts_with("->") {
                    continue;
                }
                let on = spaces.within[kind as usize];
                // Array initializer braces off: kept as written (Signal's
                // `new int[] { 1 }` is common; AS's default would join them).
                if !on && matches!(kind, Within::ArrayInitializer | Within::EmptyArrayInitializer) {
                    continue;
                }
                after_open[at] = Some(on);
                before_close[i] = Some(on);
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if let Some(on) = before_close[k] {
            while out.ends_with(' ') {
                out.pop();
            }
            if on {
                out.push(' ');
            }
        }
        out.push(c);
        k += 1;
        if let Some(on) = after_open[k - 1] {
            let mut m = k;
            while chars.get(m) == Some(&' ') {
                m += 1;
            }
            // Before a comment the spaces stay: `f( /* c */ a)`.
            if m < chars.len() && code[m] {
                k = m;
                if on {
                    out.push(' ');
                }
            }
        }
    }
    out
}

/// What a Java `(` after `before` opens, for its Within option.
fn paren_kind(before: &str) -> Within {
    let word = last_word(before);
    match word {
        "if" => return Within::If,
        "for" => return Within::For,
        "while" => return Within::While,
        "switch" => return Within::Switch,
        "try" => return Within::Try,
        "catch" => return Within::Catch,
        "synchronized" => return Within::Synchronized,
        _ => {}
    }
    if before.ends_with('>') && !before.ends_with("->") {
        // `new ArrayList<>(`, `new Foo<Bar>(`.
        return Within::MethodCall;
    }
    if word.is_empty() || !before.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$') {
        return Within::Cast;
    }
    let head = before[..before.len() - word.len()].trim_end();
    // `@Name(`, `@a.b.Name(`.
    let qualified = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
    if qualified.ends_with('@') || head.ends_with('@') {
        return Within::Annotation;
    }
    if matches!(word, "super" | "this") || head.ends_with('.') {
        return Within::MethodCall;
    }
    // `void f(`, `public Foo(`, `List<T> f(`, `int[] f(`: a declaration.
    let previous = last_word(head);
    let declares = head.ends_with(']') || head.ends_with('>') && !head.ends_with(" >") && !head.ends_with("->") || head.ends_with(|c: char| c.is_alphanumeric() || c == '_') && !matches!(previous, "return" | "new" | "throw" | "else" | "case" | "assert" | "yield" | "do" | "instanceof");
    if declares { Within::MethodDeclaration } else { Within::MethodCall }
}

/// The spaces before and after a `->` that follows `out`. Kotlin: a
/// function type's (`(Int) -> Unit`) when what comes before is a `(…)`
/// that isn't a lambda's destructuring, a lambda's when an unclosed `{`
/// on the line opens it, else a `when` branch's. Java: the lambda arrow's
/// (switch rules' too).
fn arrow_spaces(out: &str, kotlin: bool, spaces: &Spaces) -> (bool, bool) {
    if !kotlin {
        return (spaces.arrows[0], spaces.arrows[0]);
    }
    let before = out.trim_end();
    if before.ends_with(')') {
        let mut depth = 0;
        let mut open = None;
        for (i, ch) in before.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let head = open.map_or("", |o| before[..o].trim_end());
        // `{ (a, b) -> …`, `{ x, (a, b) -> …`: destructuring.
        if !(head.ends_with('{') || head.ends_with(',') && unclosed_brace(head)) {
            return (spaces.arrows[1], spaces.arrows[1]);
        }
    }
    if unclosed_brace(before) {
        (spaces.arrows[3], true)
    } else {
        (spaces.arrows[2], spaces.arrows[2])
    }
}

/// Whether a Kotlin `{` after `before` (the line so far) opens a lambda
/// or a function's body, whose spaces inside In simple one line methods
/// sets; not a class body or a control statement's block.
fn simple_brace(before: &str) -> bool {
    let segment = before.rsplit(['{', '}', ';']).next().unwrap_or("").trim_end();
    if matches!(last_word(segment), "else" | "try" | "finally" | "do" | "init" | "when") {
        return false;
    }
    if segment.ends_with(')') {
        let mut depth = 0;
        for (i, ch) in segment.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        if matches!(last_word(segment[..i].trim_end()), "if" | "for" | "while" | "when" | "catch") {
                            return false;
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    mentions_word(segment, "fun") || !["class", "object", "interface", "enum"].iter().any(|k| mentions_word(segment, k))
}

/// A Kotlin unary operator at `k` (prefix `!`, a sign, `++` / `--`) and
/// whether it's a prefix one; not `!in`, `!is`, `!!`, `!=`, `->`, `-=`.
/// Java's too, where `step`, `to` and the like are names.
fn unary(chars: &[(usize, char)], k: usize, is_code: &dyn Fn(usize) -> bool, out: &str, kotlin: bool) -> Option<(&'static str, bool)> {
    let c = chars[k].1;
    let next = chars.get(k + 1).filter(|(j, _)| is_code(*j)).map(|(_, ch)| *ch);
    let before = out.trim_end();
    let last = before.chars().last();
    let word = last_word(before);
    let keyword = matches!(word, "return" | "else" | "if" | "while" | "throw" | "case" | "yield" | "assert" | "default") || kotlin && matches!(word, "in" | "is" | "when" | "to" | "until" | "downTo" | "step" | "and" | "or" | "as");
    let cast = last == Some(')') && sign_after_paren(before);
    let operand = last.is_some_and(|l| l.is_alphanumeric() || matches!(l, '_' | ')' | ']' | '}' | '"' | '\'' | '`' | '!')) && !keyword && !cast || before.ends_with("++") || before.ends_with("--");
    match c {
        '!' if out.ends_with('!') || operand || matches!(next, Some('=') | None) => None,
        '!' => Some(("!", true)),
        '~' => (!operand && next.is_some()).then_some(("~", true)),
        _ if next == Some(c) => {
            // `++i` / `i++`; not a third `-` (`a---b`).
            if out.ends_with(c) {
                return None;
            }
            let op = if c == '+' { "++" } else { "--" };
            Some((op, !operand))
        }
        _ if operand || matches!(next, Some('=' | '>') | None) || out.ends_with(c) => None,
        _ => Some((if c == '+' { "+" } else { "-" }, true)),
    }
}

/// After `if (…)` / `while (…)` (Kotlin's `if (x) -1 else 1`) or a
/// primitive cast (`(int) -x`) that `before` ends with, a `+` / `-` is a sign.
fn sign_after_paren(before: &str) -> bool {
    let mut depth = 0;
    for (i, ch) in before.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    let inside = before[i + 1..before.len() - 1].trim();
                    let head = last_word(before[..i].trim_end());
                    return matches!(head, "if" | "while" | "for" | "when") || matches!(inside, "int" | "long" | "short" | "byte" | "char" | "float" | "double" | "boolean");
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether `text` has a `{` that isn't closed after it.
fn unclosed_brace(text: &str) -> bool {
    let mut depth = 0i32;
    for ch in text.chars().rev() {
        match ch {
            '}' => depth += 1,
            '{' if depth == 0 => return true,
            '{' => depth -= 1,
            _ => {}
        }
    }
    false
}

/// Whether the `+ - * / %` at `k` is a binary operator on its own (not
/// `++`, `+=`, `->`, a sign, `1e-5`, `<*>`, `import a.*`): what the code
/// before it ends with is an operand, not a keyword.
fn arithmetic(chars: &[(usize, char)], k: usize, is_code: &dyn Fn(usize) -> bool, out: &str) -> bool {
    let c = chars[k].1;
    let at = |n: isize| chars.get(k.checked_add_signed(n)?).filter(|(j, _)| is_code(*j)).map(|(_, ch)| *ch);
    let next = at(1);
    if next == Some('=') || (matches!(c, '+' | '-') && (next == Some(c) || at(-1) == Some(c))) || (c == '-' && next == Some('>')) {
        return false;
    }
    let before = out.trim_end();
    let Some(last) = before.chars().last() else { return false };
    if !(last.is_alphanumeric() || matches!(last, '_' | ')' | ']' | '"' | '\'' | '`')) {
        return false;
    }
    let word = last_word(before);
    // `1e-5`, `0x1p+3`.
    let number = word.starts_with(|d: char| d.is_ascii_digit());
    if matches!(c, '+' | '-') && number && (word.ends_with(['e', 'E']) && !word.starts_with("0x") || word.ends_with(['p', 'P'])) {
        return false;
    }
    const KEYWORDS: &[&str] = &["return", "case", "else", "in", "is", "throw", "yield", "when", "if", "while", "do", "to", "until", "downTo", "step", "and", "or", "assert", "default", "new", "instanceof", "as"];
    if KEYWORDS.contains(&word) {
        return false;
    }
    !(last == ')' && sign_after_paren(before))
}

/// Whether Java's `?` at `k` is a ternary's, not a wildcard's (`<?>`,
/// `<? extends T>`, `Map<?, ?>`).
fn ternary_question(chars: &[(usize, char)], k: usize, is_code: &dyn Fn(usize) -> bool, out: &str) -> bool {
    let before = out.trim_end();
    if before.ends_with('<') || before.ends_with(',') && before.rfind('<') > before.rfind('>') {
        return false;
    }
    let after: String = chars[k + 1..].iter().take_while(|(j, _)| is_code(*j)).map(|(_, ch)| *ch).collect();
    let after = after.trim_start();
    !(after.starts_with('>') || after.starts_with(',') || after.starts_with("extends") || after.starts_with("super"))
}

/// `op` at `k`, the spaces around it as `before` / `after` say; `k`
/// moves past it and the spaces after it.
fn spaced_operator(out: &mut String, chars: &[(usize, char)], k: &mut usize, is_code: &dyn Fn(usize) -> bool, op: &str, before: bool, after: bool) {
    while out.ends_with(' ') {
        out.pop();
    }
    if before && !out.is_empty() {
        out.push(' ');
    }
    out.push_str(op);
    *k += op.chars().count();
    while chars.get(*k).is_some_and(|(j, ch)| is_code(*j) && *ch == ' ') {
        *k += 1;
    }
    if after && *k < chars.len() {
        out.push(' ');
    }
}

/// Whether `text` has the whole word `word`.
fn mentions_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// The word (letters, digits, `_`) `text` ends with.
fn last_word(text: &str) -> &str {
    let start = text.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    &text[start..]
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
/// A Java line ending with a ternary's sign: `cond ?`, `one :` (not a
/// `case A:` label or a `for (x :`).
fn java_ternary_end(line: &str) -> bool {
    let start = line.trim_start();
    (line.ends_with(" ?") || line.ends_with(" :")) && !start.starts_with("case ") && !start.starts_with("default") && !start.starts_with("for ")
}

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

    #[test]
    fn braces_range_and_unary_spaces() {
        let text = "fun f() {return 1}\nval a = xs.map {it * 2}.filter {x->x > 0}\nval b = xs.map {x ->\n    x\n}\nclass A {val x = 1}\nval s = \"${x}\"\nval e = {}\nval r = 0 .. 9 + (1 ..< n).count()\nval n = ! done && - 1 < x\nval c = i ++ + -- j + k++ - 1\nval d = - -x\nval m = a !in b && c!! == d\n";
        assert_eq!(kt(text), "fun f() { return 1 }\nval a = xs.map { it * 2 }.filter { x -> x > 0 }\nval b = xs.map { x ->\n    x\n}\n\nclass A { val x = 1 }\n\nval s = \"${x}\"\nval e = {}\nval r = 0..9 + (1..<n).count()\nval n = !done && -1 < x\nval c = i++ + --j + k++ - 1\nval d = - -x\nval m = a !in b && c!! == d\n");
        let with = |text: &str, spaces: Spaces| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { spaces, ..Default::default() });
        let off = Spaces { simple_braces: false, around_range: true, around_unary: true, ..Default::default() };
        assert_eq!(with("fun f() { return 1 }\nval a = xs.map { it }\nif (a) { b() } else { c() }\nclass A { val x = 1 }\n", off), "fun f() {return 1}\nval a = xs.map {it}\nif (a) { b() } else { c() }\nclass A { val x = 1 }\n");
        assert_eq!(with("val r = 0..9\nval n = !done && -1 < x\nval c = i++\n", off), "val r = 0 .. 9\nval n = ! done && - 1 < x\nval c = i ++\n");
    }

    #[test]
    fn java_within_and_unary_spaces() {
        let java = |text: &str, spaces: Spaces| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { spaces, ..Default::default() });
        let text = "class A {\n    @Ann(x = 1)\n    int f(int a, int[] b) {\n        if (a > 0 && (b[a] + 1) > 2) {\n            g((String) o, foo(), new int[] {1}, new int[] {});\n        }\n        for (int i : b) {\n            synchronized (this) {\n                i = - i + ~ a - ++ a + a ++;\n            }\n        }\n        return ! done ? -1 : (int) - a;\n    }\n}\n";
        // The defaults: none inside, unary operators joined.
        assert_eq!(java(text, Spaces::default()), text.replace("- i", "-i").replace("~ a", "~a").replace("++ a", "++a").replace("a ++", "a++").replace("! done", "!done").replace("(int) - a", "(int) -a"));
        let on = Spaces { within: [true; 22], around_unary: true, ..Default::default() };
        assert_eq!(
            java(text, on),
            "class A {\n    @Ann( x = 1 )\n    int f( int a, int[] b ) {\n        if ( a > 0 && ( b[ a ] + 1 ) > 2 ) {\n            g( ( String ) o, foo( ), new int[] { 1 }, new int[] { } );\n        }\n        for ( int i : b ) {\n            synchronized ( this ) {\n                i = - i + ~ a - ++ a + a ++;\n            }\n        }\n        return ! done ? - 1 : ( int ) - a;\n    }\n}\n"
        );
        // A continued line's leading operator and a lambda's parameters stay.
        let continued = "class A {\n    void f() {\n        int n = a\n                + b;\n        Runnable r = (x) -> x;\n    }\n}\n";
        let plain = java(continued, Spaces::default());
        assert!(plain.contains("\n+ b;") || plain.contains(" + b;"));
        assert_eq!(java(continued, on), plain.replace("void f()", "void f( )"));
        assert_eq!(java("f( /* c */ a);\n", Spaces::default()), "f( /* c */ a);\n");
    }

    #[test]
    fn java_within_braces_angles_records() {
        let java = |text: &str, spaces: Spaces| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { spaces, ..Default::default() });
        let text = "class A {\n    void f() {return;}\n\n    void e() {}\n\n    List< byte[] > a = new ArrayList< >();\n\n    record P(int x, int y) {}\n\n    void g() {\n        if (o instanceof P(int x, int y) && a < b && c > d) {}\n        Runnable r = () -> { go(); };\n    }\n}\n";
        // The defaults: no spaces in type arguments; braces as written.
        assert_eq!(java(text, Spaces::default()), text.replace("List< byte[] > a = new ArrayList< >()", "List<byte[]> a = new ArrayList<>()"));
        let mut within = [false; 22];
        within[Within::CodeBraces as usize] = true;
        within[Within::AngleBrackets as usize] = true;
        within[Within::RecordHeader as usize] = true;
        within[Within::Deconstruction as usize] = true;
        assert_eq!(
            java(text, Spaces { within, ..Default::default() }),
            "class A {\n    void f() { return; }\n\n    void e() { }\n\n    List< byte[] > a = new ArrayList<>();\n\n    record P( int x, int y ) {}\n\n    void g() {\n        if (o instanceof P( int x, int y ) && a < b && c > d) { }\n        Runnable r = () -> { go(); };\n    }\n}\n"
        );
        // Spaces inside block braces when body is present: not the empty ones.
        let mut body = [false; 22];
        body[Within::BlockBody as usize] = true;
        assert_eq!(java("class A {\n    void f() {return;}\n\n    void e() {}\n}\n", Spaces { within: body, ..Default::default() }), "class A {\n    void f() { return; }\n\n    void e() {}\n}\n");
    }

    #[test]
    fn spaces_around_arrows() {
        let text = "val g: (Int)->Int = { x->x }\nval h = xs.map { (a, b)->a }\nwhen (x) {\n    1->a()\n    else  ->   b()\n}\nval f = { a, b ->\n    a\n}\n";
        assert_eq!(kt(text), "val g: (Int) -> Int = { x -> x }\nval h = xs.map { (a, b) -> a }\nwhen (x) {\n    1 -> a()\n    else -> b()\n}\nval f = { a, b ->\n    a\n}\n");
        let style = |i: usize| {
            let mut s = Spaces::default();
            s.arrows[i] = false;
            s
        };
        let with = |text: &str, spaces: Spaces| {
            let mut indents = IndentStyle::default();
            indents.spaces = spaces;
            reformat_indented(ImportStyle::Kotlin, text, None, &indents)
        };
        assert_eq!(with("val g: (Int) -> Int = { x -> x }\n", style(1)), "val g: (Int)->Int = { x -> x }\n");
        assert_eq!(with("val g: (Int) -> Int = { x -> x }\n", style(3)), "val g: (Int) -> Int = { x-> x }\n");
        assert_eq!(with("when (x) {\n    1 -> a()\n}\n", style(2)), "when (x) {\n    1->a()\n}\n");
        let java = |text: &str, spaces: Spaces| {
            let mut indents = IndentStyle::default();
            indents.spaces = spaces;
            reformat_indented(ImportStyle::Java, text, None, &indents)
        };
        assert_eq!(java("class A {\n    Runnable r = ()->go();\n}\n", Spaces::default()), "class A {\n    Runnable r = () -> go();\n}\n");
        assert_eq!(java("class A {\n    Runnable r = () -> go();\n}\n", style(0)), "class A {\n    Runnable r = ()->go();\n}\n");
    }

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
        assert_eq!(kt(text), "val s = when (x) {\n    1 ->\n        \"one\"\n\n    else -> \"many\"\n}\nval n = a\n    ?: b");
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
        assert_eq!(kt("x shl 1; a->b; 1..<n; c>'a'"), "x shl 1; a -> b; 1..<n; c > 'a'");
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
    fn the_later_minimum_blank_lines_settings() {
        // Java: before the package (after a header comment), after an
        // anonymous class's header, before a method's first statement.
        let min = MinBlankLines { before_package: 1, after_anonymous_class_header: 1, before_method_body: 1, ..Default::default() };
        let style = IndentStyle { min_blank: min, ..Default::default() };
        let text = "/* Header. */\npackage a;\nclass A {\n    void f() {\n        run(new Runnable() {\n            public void run() {\n                g();\n            }\n        });\n    }\n    void e() {\n    }\n}";
        assert_eq!(
            reformat_indented(ImportStyle::Java, text, None, &style),
            "/* Header. */\n\npackage a;\n\nclass A {\n    void f() {\n\n        run(new Runnable() {\n\n            public void run() {\n\n                g();\n            }\n        });\n    }\n\n    void e() {\n    }\n}"
        );
        // The defaults change nothing; a package on the first line gets none.
        assert_eq!(java(text), "/* Header. */\npackage a;\n\nclass A {\n    void f() {\n        run(new Runnable() {\n            public void run() {\n                g();\n            }\n        });\n    }\n\n    void e() {\n    }\n}");
        assert_eq!(reformat_indented(ImportStyle::Java, "package a;\nclass A {}", None, &style), "package a;\n\nclass A {}");
        // Kotlin: around `when` branches with a block, not between one-liners.
        let style = IndentStyle { min_blank: MinBlankLines { around_when_branch_with_block: 1, ..Default::default() }, ..Default::default() };
        let text = "fun f(x: Int) {\n    when (x) {\n        1 -> a()\n        2 -> b()\n        // Three.\n        3 -> {\n            c()\n        }\n        else -> d()\n    }\n    when {\n        x > 0 -> e()\n        else -> g()\n    }\n}";
        assert_eq!(
            reformat_indented(ImportStyle::Kotlin, text, None, &style),
            "fun f(x: Int) {\n    when (x) {\n        1 -> a()\n        2 -> b()\n\n        // Three.\n        3 -> {\n            c()\n        }\n\n        else -> d()\n    }\n    when {\n        x > 0 -> e()\n        else -> g()\n    }\n}"
        );
        // Line break after multiline 'when' entry: a blank line after the block.
        assert_eq!(kt(text), text.replace("        }\n        else", "        }\n\n        else"));
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
    fn wraps_a_long_supertype_list() {
        let style = IndentStyle { right_margin: Some(60), ..Default::default() };
        let text = "class SearchViewModel(repo: Repo) : ViewModel(), SearchActions, Closeable, Comparable<SearchViewModel> {\nval x = 1\n}";
        let wrapped = "class SearchViewModel(repo: Repo) : ViewModel(),\n    SearchActions, Closeable, Comparable<SearchViewModel> {\n    val x = 1\n}";
        assert_eq!(reformat_indented(ImportStyle::Kotlin, text, None, &style), wrapped);
        assert_eq!(reformat_indented(ImportStyle::Kotlin, wrapped, None, &style), wrapped);
        // An object's and an interface's too; a short one and Java's stay.
        let object = "object DefaultSearchActionsImplementation : SearchActions, Closeable, Runnable";
        assert_eq!(reformat_indented(ImportStyle::Kotlin, object, None, &style), "object DefaultSearchActionsImplementation : SearchActions,\n    Closeable, Runnable");
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "class A : B(), C", None, &style), "class A : B(), C");
        let java = "class SearchViewModel extends ViewModel implements SearchActions, Closeable, Comparable<SearchViewModel> {\n}";
        assert_eq!(reformat_indented(ImportStyle::Java, java, None, &style), java);
    }

    #[test]
    fn left_brace_and_ternary_spaces() {
        // IntelliJ's defaults: a space before every `{`, `a ? b : c`.
        let text = "class A{\nint f(boolean c) throws E{\nif(c){\nreturn c?1:2;\n}else{\ntry{\n}finally{\n}\n}\nList<? extends T> xs = m.get(c ? a : b);\nMap<?, ?> m;\nRunnable r = String::valueOf;\nreturn x;\n}\n}";
        assert_eq!(
            java(text),
            "class A {\n    int f(boolean c) throws E {\n        if (c) {\n            return c ? 1 : 2;\n        } else {\n            try {\n            } finally {\n            }\n        }\n        List<? extends T> xs = m.get(c ? a : b);\n        Map<?, ?> m;\n        Runnable r = String::valueOf;\n        return x;\n    }\n}"
        );
        // Turned off: only that construct's brace; ternary sides each.
        let mut b = [true; 12];
        b[2] = false; // if
        b[0] = false; // class
        let style = IndentStyle { spaces: Spaces { before_left_brace: b, ternary: [false, true, true, false], ..Default::default() }, ..Default::default() };
        assert_eq!(
            reformat_indented(ImportStyle::Java, "class A {\n    void f() {\n        if (c) {\n            x = c ? 1 : 2;\n        } else {\n        }\n        new Runnable() {\n        };\n    }\n}", None, &style),
            "class A{\n    void f() {\n        if (c){\n            x = c? 1 :2;\n        } else {\n        }\n        new Runnable(){\n        };\n    }\n}"
        );
        // Kotlin is untouched by them.
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "class A {\n    fun f() = if (c) 1 else 2\n}", None, &style), "class A {\n    fun f() = if (c) 1 else 2\n}");
    }

    #[test]
    fn spaces_settings() {
        // IntelliJ's defaults: `if (`, `a == b`, `f(a, b)`; Mockito's `when(` in
        // Java and Kotlin's `synchronized(` aren't keywords.
        assert_eq!(java("class A {\nvoid f() {\nif(a==b&&c<d) x=f(a,b);\nwhen(m.get()).thenReturn(1);\n}\n}"), "class A {\n    void f() {\n        if (a == b && c < d) x = f(a, b);\n        when(m.get()).thenReturn(1);\n    }\n}");
        assert_eq!(kt("fun f() {\nsynchronized(lock) {}\nwhen(x) {}\n}"), "fun f() {\n    synchronized(lock) {}\n    when (x) {}\n}");
        // Each turned the other way.
        let mut values = [false; SPACES_LEN];
        values[12] = true;
        let all_off = Spaces::from_values(values);
        let style = IndentStyle { spaces: all_off, ..Default::default() };
        assert_eq!(
            reformat_indented(ImportStyle::Java, "class A {\n    void f() {\n        if (a == b && c < d) x += f(a, b);\n        while (i >= 0) i -= 1;\n    }\n}", None, &style),
            "class A{\n    void f(){\n        if(a==b&&c<d) x+=f(a ,b);\n        while(i>=0) i-=1;\n    }\n}"
        );
        let style = IndentStyle { spaces: Spaces { before_parens: [true, true, true, true, false, true, true, true], around_relational: false, ..Default::default() }, ..Default::default() };
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "fun f() {\n    when (x) {}\n    if (a < b) g()\n}", None, &style), "fun f() {\n    when(x) {}\n    if (a<b) g()\n}");
    }

    #[test]
    fn spaces_around_arithmetic() {
        assert_eq!(
            java("class A {\nint f(int a,int b) {\nreturn a+b*2-(a%b)/-1+x++ + (int) -d+1e-5;\n}\n}"),
            "class A {\n    int f(int a, int b) {\n        return a + b * 2 - (a % b) / -1 + x++ + (int) -d + 1e-5;\n    }\n}"
        );
        assert_eq!(kt("fun f(n: Int) = if (n>0) -1 else n-1\nval l: List<*> = listOf(*args)\nval s = \"a\"+1"), "fun f(n: Int) = if (n > 0) -1 else n - 1\nval l: List<*> = listOf(*args)\nval s = \"a\" + 1");
        let style = IndentStyle { spaces: Spaces { around_additive: false, around_multiplicative: false, ..Default::default() }, ..Default::default() };
        assert_eq!(reformat_indented(ImportStyle::Kotlin, "val x = a + b * c - -d / e", None, &style), "val x = a+b*c- -d/e");
    }

    #[test]
    fn before_keywords_and_colon_spaces() {
        let text = "class A {\n    void f() {\n        try {\n            g();\n        } catch (Exception e) {\n        } finally {\n        }\n        if (a) {\n        } else {\n        }\n    }\n}";
        let style = IndentStyle { spaces: Spaces { before_keywords: [false, true, true, false], ..Default::default() }, ..Default::default() };
        assert_eq!(
            reformat_indented(ImportStyle::Java, text, None, &style),
            "class A {\n    void f() {\n        try {\n            g();\n        } catch (Exception e) {\n        }finally {\n        }\n        if (a) {\n        }else {\n        }\n    }\n}"
        );
        // Kotlin: IntelliJ's defaults, then each colon setting the other way.
        let text = "class A<T : Any>(val name: String) : B() {\n    fun f(): Int = 1\n}";
        assert_eq!(kt(text), text);
        let style = IndentStyle { spaces: Spaces { colons: [true, false, false, false], ..Default::default() }, ..Default::default() };
        assert_eq!(reformat_indented(ImportStyle::Kotlin, text, None, &style), "class A<T:Any>(val name :String):B() {\n    fun f() :Int = 1\n}");
        // Older settings had 14 values.
        assert_eq!(Spaces::from_prefix(&[true; 14]).map(|s| s.colons), Some([false, true, true, true]));
    }

    #[test]
    fn wrapping_options() {
        let w = |kotlin: bool, wrapping: Wrapping, text: &str| {
            let style = IndentStyle { right_margin: Some(60), wrapping: Some(wrapping), ..Default::default() };
            let once = reformat_indented(if kotlin { ImportStyle::Kotlin } else { ImportStyle::Java }, text, None, &style);
            assert_eq!(reformat_indented(if kotlin { ImportStyle::Kotlin } else { ImportStyle::Java }, &once, None, &style), once, "stable");
            once
        };
        let java = "class A {\nvoid run(String firstArgument, String secondArgument, int third) {\nresult = repository.load(firstArgument, secondArgument, third);\n}\n}";
        // IntelliJ's Java defaults: nothing wraps.
        assert_eq!(w(false, Wrapping::JAVA, java), "class A {\n    void run(String firstArgument, String secondArgument, int third) {\n        result = repository.load(firstArgument, secondArgument, third);\n    }\n}");
        let chop = Wrapping { call_args: Wrap::ChopIfLong, params: Wrap::ChopIfLong, ..Wrapping::JAVA };
        assert_eq!(
            w(false, chop, java),
            "class A {\n    void run(String firstArgument,\n             String secondArgument,\n             int third) {\n        result = repository.load(firstArgument,\n                secondArgument,\n                third);\n    }\n}"
        );
        let fill = Wrapping { call_args: Wrap::WrapIfLong, params: Wrap::WrapIfLong, ..Wrapping::JAVA };
        assert_eq!(
            w(false, fill, java),
            "class A {\n    void run(String firstArgument, String secondArgument,\n             int third) {\n        result = repository.load(firstArgument,\n                secondArgument, third);\n    }\n}"
        );
        let chain = "class A {\nvoid f() {\nString s = builder.append(firstPart).append(secondPart).build();\n}\n}";
        assert_eq!(w(false, Wrapping { chain: Wrap::WrapIfLong, ..Wrapping::JAVA }, chain), "class A {\n    void f() {\n        String s = builder.append(firstPart)\n                .append(secondPart).build();\n    }\n}");
        let assign = "class A {\nvoid f() {\nString descriptionOfTheThing = formatter.format(firstValue, locale);\n}\n}";
        assert_eq!(w(false, Wrapping { assignment: Wrap::WrapIfLong, ..Wrapping::JAVA }, assign), "class A {\n    void f() {\n        String descriptionOfTheThing =\n                formatter.format(firstValue, locale);\n    }\n}");
        // Kotlin: wrap if long fills the lines; do not wrap leaves it.
        let kt = "fun f() {\nval result = repository.load(firstArgument, secondArgument, third)\n}";
        assert_eq!(w(true, Wrapping { call_args: Wrap::WrapIfLong, ..Wrapping::KOTLIN }, kt), "fun f() {\n    val result = repository.load(\n        firstArgument, secondArgument, third\n    )\n}");
        assert_eq!(w(true, Wrapping::JAVA, kt), "fun f() {\n    val result = repository.load(firstArgument, secondArgument, third)\n}");
    }

    #[test]
    fn brace_placement_and_keywords_on_new_lines() {
        let f = |kotlin: bool, braces: Braces, text: &str| {
            let w = if kotlin { Wrapping::KOTLIN } else { Wrapping::JAVA };
            let style = IndentStyle { wrapping: Some(Wrapping { braces, ..w }), ..Default::default() };
            let st = if kotlin { ImportStyle::Kotlin } else { ImportStyle::Java };
            let once = reformat_indented(st, text, None, &style);
            assert_eq!(reformat_indented(st, &once, None, &style), once, "stable");
            once
        };
        let java = "class A\n{\n    void f(int a)\n    {\n        if (a > 0)\n        {\n            g();\n        }\n        else {\n            h();\n        }\n        do {\n            a--;\n        }\n        while (a > 0);\n        if (a > 1) {\n        }\n        while (a < 0) {\n        }\n        try {\n        } catch (Exception e) {\n        } finally {\n        }\n        Runnable r = () ->\n        {\n        };\n    }\n}";
        // IntelliJ's defaults: End of line, keywords after `}`.
        assert_eq!(
            f(false, Braces::default(), java),
            "class A {\n    void f(int a) {\n        if (a > 0) {\n            g();\n        } else {\n            h();\n        }\n        do {\n            a--;\n        } while (a > 0);\n        if (a > 1) {\n        }\n        while (a < 0) {\n        }\n        try {\n        } catch (Exception e) {\n        } finally {\n        }\n        Runnable r = () ->\n        {\n        };\n    }\n}"
        );
        let all = Braces::from_values([true; 7]);
        assert_eq!(
            f(false, all, "class A {\n    void f(int a) {\n        if (a > 0) {\n            g();\n        } else {\n            h();\n        }\n        try {\n        } catch (Exception e) {\n        } finally {\n        }\n    }\n}"),
            "class A\n{\n    void f(int a)\n    {\n        if (a > 0)\n        {\n            g();\n        }\n        else\n        {\n            h();\n        }\n        try\n        {\n        }\n        catch (Exception e)\n        {\n        }\n        finally\n        {\n        }\n    }\n}"
        );
        let kt = "fun f(a: Int) {\n    if (a > 0) {\n        g()\n    }\n    else {\n        h()\n    }\n    try {\n    } catch (e: Exception) {\n    }\n}";
        assert_eq!(f(true, Braces::default(), kt), "fun f(a: Int) {\n    if (a > 0) {\n        g()\n    } else {\n        h()\n    }\n    try {\n    } catch (e: Exception) {\n    }\n}");
        assert_eq!(f(true, Braces { catch_new_line: true, ..Default::default() }, kt), "fun f(a: Int) {\n    if (a > 0) {\n        g()\n    } else {\n        h()\n    }\n    try {\n    }\n    catch (e: Exception) {\n    }\n}");
    }

    #[test]
    fn force_braces_and_kotlin_left_brace() {
        let f = |kotlin: bool, braces: Braces, text: &str| {
            let w = if kotlin { Wrapping::KOTLIN } else { Wrapping::JAVA };
            let style = IndentStyle { wrapping: Some(Wrapping { braces, ..w }), ..Default::default() };
            let st = if kotlin { ImportStyle::Kotlin } else { ImportStyle::Java };
            let once = reformat_indented(st, text, None, &style);
            assert_eq!(reformat_indented(st, &once, None, &style), once, "stable");
            once
        };
        let java = "class A {\n    int f(int a) {\n        if (a > 0) return log(\")\");\n        if (a < 0)\n            a++;\n        else\n            a--;\n        for (int i = 0; i < a; i++) g(i);\n        while (a > 9)\n            a /= 2;\n        if (a == 1) // one\n            a = 2;\n        if (a == 2)\n            if (a > 1) a = 3;\n        return a;\n    }\n}";
        // Off: nothing added.
        let none = f(false, Braces::NONE, java);
        assert!(!none.contains("{\n            a++;"));
        let multiline = Braces { force: [Force::WhenMultiline; 3], ..Braces::NONE };
        assert_eq!(
            f(false, multiline, java),
            "class A {\n    int f(int a) {\n        if (a > 0) return log(\")\");\n        if (a < 0) {\n            a++;\n        } else {\n            a--;\n        }\n        for (int i = 0; i < a; i++) g(i);\n        while (a > 9) {\n            a /= 2;\n        }\n        if (a == 1) // one\n            a = 2;\n        if (a == 2)\n            if (a > 1) a = 3;\n        return a;\n    }\n}"
        );
        let always = Braces { force: [Force::Always, Force::Always, Force::DoNotForce], ..Braces::NONE };
        assert_eq!(
            f(false, always, java),
            "class A {\n    int f(int a) {\n        if (a > 0) {\n            return log(\")\");\n        }\n        if (a < 0) {\n            a++;\n        } else {\n            a--;\n        }\n        for (int i = 0; i < a; i++) {\n            g(i);\n        }\n        while (a > 9)\n            a /= 2;\n        if (a == 1) // one\n            a = 2;\n        if (a == 2)\n            if (a > 1) {\n                a = 3;\n            }\n        return a;\n    }\n}"
        );
        // Kotlin: Put left brace on new line, not a lambda's; off joins it back.
        let kt = "class A {\n    fun f(x: Int): Int {\n        if (x > 0) {\n            run {\n                g()\n            }\n        } else {\n        }\n        return x\n    }\n    fun g() = run {\n    }\n}";
        let next = f(true, Braces { other_next_line: true, ..Braces::NONE }, kt);
        assert_eq!(next, "class A\n{\n    fun f(x: Int): Int\n    {\n        if (x > 0)\n        {\n            run {\n                g()\n            }\n        } else\n        {\n        }\n        return x\n    }\n\n    fun g() = run {\n    }\n}");
        assert_eq!(f(true, Braces::NONE, &next), "class A {\n    fun f(x: Int): Int {\n        if (x > 0) {\n            run {\n                g()\n            }\n        } else {\n        }\n        return x\n    }\n\n    fun g() = run {\n    }\n}");
    }

    #[test]
    fn indents_bodies_without_braces() {
        assert_eq!(
            java("class A {\nvoid f(int a) {\nif (a > 0)\nreturn;\nelse if (a < 0)\n // negative\na++;\nelse\nfor (int i = 0; i < a; i++)\nif (i > 1)\ng(i,\n1);\nwhile (a > 9)\na /= 2;\ndo\na--;\nwhile (a > 0);\nif (a == 1)\n{\na = 2;\n}\nh(\")\");\nif (a > 1 &&\na < 9)\na = 0;\nwhile (f(a,\nb) > 0) {\na--;\n}\ng();\n}\n}"),
            "class A {\n    void f(int a) {\n        if (a > 0)\n            return;\n        else if (a < 0)\n            // negative\n            a++;\n        else\n            for (int i = 0; i < a; i++)\n                if (i > 1)\n                    g(i,\n                            1);\n        while (a > 9)\n            a /= 2;\n        do\n            a--;\n        while (a > 0);\n        if (a == 1) {\n            a = 2;\n        }\n        h(\")\");\n        if (a > 1 &&\n                a < 9)\n            a = 0;\n        while (f(a,\n                b) > 0) {\n            a--;\n        }\n        g();\n    }\n}"
        );
        assert_eq!(kt("fun f(x: Int) {\nif (x > 0)\nprintln(x)\nelse\nreturn\nfor (i in 0..x)\ng(i)\nval y = if (x > 1) 2 else 3\ndo {\ng(x)\n} while (x > 9)\nh()\nreturn if (x > 2)\nx\nelse\n0\n}"), "fun f(x: Int) {\n    if (x > 0)\n        println(x)\n    else\n        return\n    for (i in 0..x)\n        g(i)\n    val y = if (x > 1) 2 else 3\n    do {\n        g(x)\n    } while (x > 9)\n    h()\n    return if (x > 2)\n        x\n    else\n        0\n}");
    }

    #[test]
    fn wraps_binary_and_ternary_expressions() {
        let w = |ops: Operations, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { ops, ..Wrapping::JAVA }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "class A {\n    boolean f() {\n        if (firstConditionHoldsFor(alpha) && secondConditionHoldsFor(beta) || thirdConditionHoldsFor(gamma, delta, epsilon)) {\n            return true;\n        }\n        int total = firstValueOfTheLongName + secondValueOfTheLongName * factor + thirdValueOfTheLongName - fourthValueOfIt;\n        String label = isTheSelectedItemVisible(position) ? resources.getString(R.string.visible_item) : fallbackLabelText;\n        return a + b;\n    }\n}";
        assert_eq!(w(Operations { binary: Wrap::ChopIfLong, ..Operations::NONE }, text), "class A {\n    boolean f() {\n        if (firstConditionHoldsFor(alpha) && secondConditionHoldsFor(beta) ||\n                thirdConditionHoldsFor(gamma, delta, epsilon)) {\n            return true;\n        }\n        int total = firstValueOfTheLongName +\n                secondValueOfTheLongName * factor +\n                thirdValueOfTheLongName -\n                fourthValueOfIt;\n        String label = isTheSelectedItemVisible(position) ? resources.getString(R.string.visible_item) : fallbackLabelText;\n        return a + b;\n    }\n}");
        // Wrap if long: only where the line would pass the margin; the sign starts the next line.
        assert_eq!(w(Operations { binary: Wrap::WrapIfLong, binary_sign_next: true, ternary: Wrap::ChopIfLong, ..Operations::NONE }, text), "class A {\n    boolean f() {\n        if (firstConditionHoldsFor(alpha) && secondConditionHoldsFor(beta)\n                || thirdConditionHoldsFor(gamma, delta, epsilon)) {\n            return true;\n        }\n        int total = firstValueOfTheLongName + secondValueOfTheLongName * factor + thirdValueOfTheLongName\n                - fourthValueOfIt;\n        String label = isTheSelectedItemVisible(position) ?\n                resources.getString(R.string.visible_item) :\n                fallbackLabelText;\n        return a + b;\n    }\n}");
        assert_eq!(w(Operations { ternary: Wrap::WrapIfLong, ternary_signs_next: true, ..Operations::NONE }, text), "class A {\n    boolean f() {\n        if (firstConditionHoldsFor(alpha) && secondConditionHoldsFor(beta) || thirdConditionHoldsFor(gamma, delta, epsilon)) {\n            return true;\n        }\n        int total = firstValueOfTheLongName + secondValueOfTheLongName * factor + thirdValueOfTheLongName - fourthValueOfIt;\n        String label = isTheSelectedItemVisible(position) ? resources.getString(R.string.visible_item)\n                : fallbackLabelText;\n        return a + b;\n    }\n}");
        assert_eq!(w(Operations::NONE, text), text);
        // Only binary expressions wrap: a ternary's branches are cut at their operators.
        let ternary = "class A {\n    int f() {\n        return total > 0 ? alphaValueOfTheThing + betaValueOfTheThing + gammaValueOfTheThing : deltaValue - total - extra;\n    }\n}";
        assert_eq!(w(Operations { binary: Wrap::WrapIfLong, ..Operations::NONE }, ternary), "class A {\n    int f() {\n        return total > 0 ? alphaValueOfTheThing + betaValueOfTheThing + gammaValueOfTheThing : deltaValue - total -\n                extra;\n    }\n}");
    }

    #[test]
    fn wraps_annotations() {
        let w = |annotations: [AnnotationWrap; 3], text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { annotations, ..Wrapping::JAVA }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "@Deprecated @SuppressWarnings(\"a)\") public class A {\n    @Inject Foo foo;\n\n    @Inject @Named(\"b\") Bar bar, baz;\n\n    @Override public void f(@NonNull String s,\n                            @NonNull Locale locale) {\n        @SuppressWarnings(\"x\") int n = 1;\n    }\n\n    // @Test public void g() {}\n    @interface Ann {\n    }\n}\n\nenum E {\n    @Deprecated ONE,\n    TWO\n}";
        assert_eq!(w([AnnotationWrap::DoNotWrap; 3], text), text);
        assert_eq!(
            w([AnnotationWrap::WrapAlways; 3], text),
            "@Deprecated\n@SuppressWarnings(\"a)\")\npublic class A {\n    @Inject\n    Foo foo;\n\n    @Inject\n    @Named(\"b\")\n    Bar bar, baz;\n\n    @Override\n    public void f(@NonNull String s,\n                  @NonNull Locale locale) {\n        @SuppressWarnings(\"x\") int n = 1;\n    }\n\n    // @Test public void g() {}\n    @interface Ann {\n    }\n}\n\nenum E {\n    @Deprecated ONE,\n    TWO\n}"
        );
        // If long: only a line past the margin, as many on a line as fit.
        let long = "class A {\n    @Override @SuppressWarnings(\"unchecked\") @VisibleForTesting public void aMethodWithAVeryLongNameThatPassesTheMargin(int a) {\n    }\n    @Override public void f() {\n    }\n}";
        assert_eq!(
            w([AnnotationWrap::DoNotWrap, AnnotationWrap::WrapIfLong, AnnotationWrap::DoNotWrap], long),
            "class A {\n    @Override @SuppressWarnings(\"unchecked\") @VisibleForTesting\n    public void aMethodWithAVeryLongNameThatPassesTheMargin(int a) {\n    }\n\n    @Override public void f() {\n    }\n}"
        );
        assert_eq!(
            w([AnnotationWrap::DoNotWrap, AnnotationWrap::ChopIfLong, AnnotationWrap::DoNotWrap], long),
            "class A {\n    @Override\n    @SuppressWarnings(\"unchecked\")\n    @VisibleForTesting\n    public void aMethodWithAVeryLongNameThatPassesTheMargin(int a) {\n    }\n\n    @Override public void f() {\n    }\n}"
        );
    }

    #[test]
    fn wraps_local_variable_annotations() {
        let w = |wrap: AnnotationWrap, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { local_annotations: wrap, ..Wrapping::JAVA }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "class A {\n    @Nullable String name;\n\n    void f(@NonNull String s) {\n        @SuppressWarnings(\"unchecked\") List<String> l = (List<String>) o;\n        for (@NonNull String x : l) {\n            @Deprecated final int n = 1;\n        }\n        @Deprecated class Local {\n        }\n        run(() -> {\n            @Nullable Object v = null; // why\n        });\n    }\n}";
        assert_eq!(w(AnnotationWrap::DoNotWrap, text), text);
        assert_eq!(
            w(AnnotationWrap::WrapAlways, text),
            "class A {\n    @Nullable String name;\n\n    void f(@NonNull String s) {\n        @SuppressWarnings(\"unchecked\")\n        List<String> l = (List<String>) o;\n        for (@NonNull String x : l) {\n            @Deprecated\n            final int n = 1;\n        }\n        @Deprecated class Local {\n        }\n        run(() -> {\n            @Nullable Object v = null; // why\n        });\n    }\n}"
        );
        assert_eq!(w(AnnotationWrap::ChopIfLong, text), text);
    }

    #[test]
    fn wraps_array_initializers() {
        let w = |wrap: Wrap, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { array_init: wrap, ..Wrapping::JAVA }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "class A {\n    private static final String[] PROJECTION = new String[]{ID, ADDRESS, DATE_SENT, DATE_RECEIVED, BODY, TYPE, THREAD_ID, STATUS, READ};\n    int[] small = {1, 2, 3};\n\n    void f() {\n        String[] columns = {\"recipient_id\", \"group_id\", \"profile_key\", \"profile_given_name\", \"system_display_name\", \"extra\"};\n        g(new int[] {1, 2}, h(a, b));\n    }\n}";
        assert_eq!(w(Wrap::DoNotWrap, text), text);
        assert_eq!(
            w(Wrap::WrapIfLong, text),
            "class A {\n    private static final String[] PROJECTION = new String[]{ID, ADDRESS, DATE_SENT, DATE_RECEIVED, BODY, TYPE,\n            THREAD_ID, STATUS, READ};\n    int[] small = {1, 2, 3};\n\n    void f() {\n        String[] columns = {\"recipient_id\", \"group_id\", \"profile_key\", \"profile_given_name\", \"system_display_name\",\n                \"extra\"};\n        g(new int[] {1, 2}, h(a, b));\n    }\n}"
        );
        assert_eq!(
            w(Wrap::ChopIfLong, "class A {\n    private static final String[] PROJECTION = new String[]{ID, ADDRESS, DATE_SENT, DATE_RECEIVED, BODY, TYPE, THREAD_ID, STATUS};\n}"),
            "class A {\n    private static final String[] PROJECTION = new String[]{ID,\n            ADDRESS,\n            DATE_SENT,\n            DATE_RECEIVED,\n            BODY,\n            TYPE,\n            THREAD_ID,\n            STATUS};\n}"
        );
        // Elements on lines of their own continue the line that opens them.
        let laid = "class A {\n    static final String[] COLUMNS = {\n        A,\n        B\n    };\n}";
        assert_eq!(w(Wrap::DoNotWrap, laid), "class A {\n    static final String[] COLUMNS = {\n            A,\n            B\n    };\n}");
    }

    #[test]
    fn wraps_throws_lists_and_annotation_arrays() {
        let w = |wrapping: Wrapping, text: &str| {
            let style = IndentStyle { wrapping: Some(wrapping), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "interface A {\n    void download(String url, File destination) throws IOException, InterruptedException, TimeoutException, GeneralSecurityException;\n\n    void read() throws IOException, InterruptedException {\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, text), text);
        assert_eq!(
            w(Wrapping { throws_list: Wrap::WrapIfLong, ..Wrapping::JAVA }, text),
            "interface A {\n    void download(String url, File destination) throws IOException, InterruptedException, TimeoutException,\n            GeneralSecurityException;\n\n    void read() throws IOException, InterruptedException {\n    }\n}"
        );
        assert_eq!(
            w(Wrapping { throws_list: Wrap::ChopIfLong, ..Wrapping::JAVA }, text),
            "interface A {\n    void download(String url, File destination) throws IOException,\n            InterruptedException,\n            TimeoutException,\n            GeneralSecurityException;\n\n    void read() throws IOException, InterruptedException {\n    }\n}"
        );
        // Written over lines: the types continue, the body is the method's.
        let laid = "class A implements Runnable,\nCloseable {\n    void f() throws IOException,\n    TimeoutException {\n        g();\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, laid), "class A implements Runnable,\n        Closeable {\n    void f() throws IOException,\n            TimeoutException {\n        g();\n    }\n}");
        // An annotation's array follows Array initializer.
        let annotated = "class A {\n    @SuppressWarnings({\"unchecked\", \"rawtypes\", \"deprecation\", \"unused\", \"ConstantConditions\", \"FieldCanBeLocal\", \"SpellCheckingInspection\"})\n    @Config(sdk = {Build.VERSION_CODES.P, Build.VERSION_CODES.Q, Build.VERSION_CODES.R, Build.VERSION_CODES.S, Build.VERSION_CODES.TIRAMISU})\n    void f() {\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, annotated), annotated);
        assert_eq!(
            w(Wrapping { array_init: Wrap::ChopIfLong, ..Wrapping::JAVA }, annotated),
            "class A {\n    @SuppressWarnings({\"unchecked\",\n            \"rawtypes\",\n            \"deprecation\",\n            \"unused\",\n            \"ConstantConditions\",\n            \"FieldCanBeLocal\",\n            \"SpellCheckingInspection\"})\n    @Config(sdk = {Build.VERSION_CODES.P,\n            Build.VERSION_CODES.Q,\n            Build.VERSION_CODES.R,\n            Build.VERSION_CODES.S,\n            Build.VERSION_CODES.TIRAMISU})\n    void f() {\n    }\n}"
        );
    }

    #[test]
    fn java_names_like_kotlin_keywords_take_binary_operators() {
        let text = "class A {\n    void k() {\n        i += step + step - to * in;\n        int as = until - 1;\n    }\n}\n";
        assert_eq!(reformat(ImportStyle::Java, text, None), text);
        let kt = "fun f() {\n    for (i in 0 until -1 step -2) {\n    }\n}\n";
        assert_eq!(reformat(ImportStyle::Kotlin, kt, None), kt);
    }

    #[test]
    fn wraps_resources_and_header_parentheses() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let res = format!("try (InputStream input = openTheInputStream{}(); OutputStream output = openTheOutputStream(target)) {{", "X".repeat(25));
        let text = format!("class A {{\n    void k() {{\n        {res}\n            copy(input, output);\n        }}\n    }}\n}}\n");
        assert_eq!(java(Wrapping::JAVA, &text), text);
        // try-with-resources: Chop down if long, aligned.
        let chop = java(Wrapping { resource_wrap: Wrap::ChopIfLong, ..Wrapping::JAVA }, &text);
        assert_eq!(chop, format!("class A {{\n    void k() {{\n        try (InputStream input = openTheInputStream{}();\n             OutputStream output = openTheOutputStream(target)) {{\n            copy(input, output);\n        }}\n    }}\n}}\n", "X".repeat(25)));
        // New line after '(' and ')' on its own line: continuation indent.
        let parens = Wrapping { resource_wrap: Wrap::ChopIfLong, resource_parens: [true; 2], ..Wrapping::JAVA };
        let out = java(parens, &text);
        assert_eq!(out, format!("class A {{\n    void k() {{\n        try (\n                InputStream input = openTheInputStream{}();\n                OutputStream output = openTheOutputStream(target)\n        ) {{\n            copy(input, output);\n        }}\n    }}\n}}\n", "X".repeat(25)));
        assert_eq!(java(parens, &out), out);
        // The same for a `for` over lines; one on a line stays.
        let for_text = "class A {\n    void k() {\n        for (int i = 0;\n             i < n;\n             i++) {\n            go(i);\n        }\n        for (int j = 0; j < n; j++) {\n        }\n    }\n}\n";
        let for_out = java(Wrapping { for_parens: [true; 2], ..Wrapping::JAVA }, for_text);
        assert_eq!(for_out, "class A {\n    void k() {\n        for (\n                int i = 0;\n                i < n;\n                i++\n        ) {\n            go(i);\n        }\n        for (int j = 0; j < n; j++) {\n        }\n    }\n}\n");
    }

    #[test]
    fn wraps_asserts_and_list_parentheses() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let a = format!("class A {{\n    void k() {{\n        assert index >= 0 && index < size{} : \"index out of range: \" + index + \" of \" + size;\n    }}\n}}\n", "X".repeat(40));
        assert_eq!(java(Wrapping::JAVA, &a), a);
        // 'assert' statement: the message continues the next line, after `:` or with it.
        let w = Wrapping { assert_wrap: Wrap::WrapIfLong, ..Wrapping::JAVA };
        let after = java(w, &a);
        assert_eq!(after, format!("class A {{\n    void k() {{\n        assert index >= 0 && index < size{} :\n                \"index out of range: \" + index + \" of \" + size;\n    }}\n}}\n", "X".repeat(40)));
        assert_eq!(java(w, &after), after);
        let next = java(Wrapping { assert_colon_next: true, ..w }, &a);
        assert_eq!(next, format!("class A {{\n    void k() {{\n        assert index >= 0 && index < size{}\n                : \"index out of range: \" + index + \" of \" + size;\n    }}\n}}\n", "X".repeat(40)));
        // Method call arguments / declaration parameters over lines: `(` and `)` lines;
        // a call's single argument and lists on one line stay.
        let calls = "class A {\n    void k(int a,\n           int b) {\n        go(a,\n                b);\n        run(x -> {\n            y();\n        });\n        one(a, b);\n    }\n\n    void m(\n            int a) {\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, calls), calls);
        let both = Wrapping { call_parens: [true; 2], param_parens: [true; 2], ..Wrapping::JAVA };
        let out = java(both, calls);
        assert_eq!(out, "class A {\n    void k(\n            int a,\n            int b\n    ) {\n        go(\n                a,\n                b\n        );\n        run(x -> {\n            y();\n        });\n        one(a, b);\n    }\n\n    void m(\n            int a\n    ) {\n    }\n}\n");
        assert_eq!(java(both, &out), out);
        // Only ')' for calls: the first argument stays after `(`.
        let close = java(Wrapping { call_parens: [false, true], ..Wrapping::JAVA }, calls);
        assert!(close.contains("        go(a,\n                b\n        );\n"), "{close}");
        // A first argument `.5f` isn't a chained call.
        let num = "class A {\n    void k() {\n        go(new S(.5f, 1f,\n                2f));\n    }\n}\n";
        assert_eq!(java(both, num), "class A {\n    void k() {\n        go(new S(\n                .5f, 1f,\n                2f\n        ));\n    }\n}\n");
        // Kotlin's parameter annotations, Wrap always: the parameters chopped.
        let k = "class A(@Inject val a: A, b: B) {\n    fun f(@Suppress(\"x\") x: Int, @A @B y: Int) {\n    }\n\n    fun g(\n        @A a: Int,\n        b: Int\n    ) = 1\n}\n";
        assert_eq!(kotlin(Wrapping::KOTLIN, k), k);
        let kw = Wrapping { param_annotations: AnnotationWrap::WrapAlways, ..Wrapping::KOTLIN };
        let kout = kotlin(kw, k);
        assert_eq!(kout, "class A(\n    @Inject\n    val a: A,\n    b: B\n) {\n    fun f(\n        @Suppress(\"x\")\n        x: Int,\n        @A\n        @B\n        y: Int\n    ) {\n    }\n\n    fun g(\n        @A\n        a: Int,\n        b: Int\n    ) = 1\n}\n");
        assert_eq!(kotlin(kw, &kout), kout);
        // Calls keep their annotations.
        let call = "fun f() {\n    g(@A x, y)\n}\n";
        assert_eq!(kotlin(kw, call), call);
    }

    #[test]
    fn kotlin_list_parentheses_and_java_expressions() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let k = "fun f() {\n    go(a,\n        b)\n    run(a) {\n        x()\n    }\n    send(object : Runnable {\n        override fun run() {}\n    })\n    call(a, {\n        b()\n    })\n}\n\nfun g(\n    a: Int,\n    b: Int\n) = 1\n";
        // Kotlin's official style (on): arguments over lines get `(` and `)`
        // lines; lambdas and objects don't count; declarations keep theirs.
        let out = kotlin(Wrapping::KOTLIN, k);
        assert_eq!(out, "fun f() {\n    go(\n        a,\n        b\n    )\n    run(a) {\n        x()\n    }\n    send(object : Runnable {\n        override fun run() {}\n    })\n    call(a, {\n        b()\n    })\n}\n\nfun g(\n    a: Int,\n    b: Int\n) = 1\n");
        assert_eq!(kotlin(Wrapping::KOTLIN, &out), out);
        // Off: a declaration's breaks after `(` and before `)` go; chopped
        // lists keep the first item after `(` and `)` after the last.
        let off = Wrapping { call_parens: [false; 2], param_parens: [false; 2], ..Wrapping::KOTLIN };
        let o2 = kotlin(off, k);
        assert!(o2.contains("    go(a,\n        b)\n") && o2.ends_with("fun g(a: Int,\n    b: Int) = 1\n"), "{o2}");
        assert_eq!(kotlin(off, &o2), o2);
        let long = format!("fun f() {{\n    val result = computeSomething(firstArgument{}, secondArgument, thirdArgument, fourth)\n}}\n", "X".repeat(50));
        let lo = kotlin(off, &long);
        assert_eq!(lo, format!("fun f() {{\n    val result = computeSomething(firstArgument{},\n        secondArgument,\n        thirdArgument,\n        fourth)\n}}\n", "X".repeat(50)));
        assert_eq!(kotlin(off, &lo), lo);
        // Java's Parenthesized expression over lines; a condition's parentheses aren't one.
        let e = "class A {\n    int k() {\n        int x = (a +\n                b) * 2;\n        if (a &&\n                b) {\n        }\n        return x;\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, e), e);
        let ew = Wrapping { expr_parens: [true; 2], ..Wrapping::JAVA };
        let eo = java(ew, e);
        assert!(eo.contains("        int x = (\n") && eo.contains("        ) * 2;\n") && eo.contains("        if (a &&\n                b) {\n"), "{eo}");
        assert_eq!(java(ew, &eo), eo);
    }

    #[test]
    fn aligns_parameters_arguments_and_records() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "class A {\n    void k(int a,\n           int b) {\n        go(a,\n                b);\n    }\n\n    record R(int a,\n             int b) {\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        // Align when multiline: parameters and record components off (continuation), arguments on.
        let w = Wrapping { align_params: false, align_args: true, align_records: false, ..Wrapping::JAVA };
        let o = java(w, t);
        assert_eq!(o, "class A {\n    void k(int a,\n            int b) {\n        go(a,\n           b);\n    }\n\n    record R(int a,\n            int b) {\n    }\n}\n");
        assert_eq!(java(w, &o), o);
        // Chopped arguments aligned; Record components wrapped by their own option, parentheses on lines.
        let long = format!("class A {{\n    void k() {{\n        go(first{}, second, third, fourth);\n    }}\n\n    record Point(int horizontalCoordinate{}, int verticalCoordinate, int depth) {{\n    }}\n}}\n", "X".repeat(90), "X".repeat(60));
        let lw = Wrapping { call_args: Wrap::ChopIfLong, record_wrap: Wrap::ChopIfLong, align_args: true, record_parens: [true; 2], ..Wrapping::JAVA };
        let lo = java(lw, &long);
        assert_eq!(lo, format!("class A {{\n    void k() {{\n        go(first{},\n           second,\n           third,\n           fourth);\n    }}\n\n    record Point(\n            int horizontalCoordinate{},\n            int verticalCoordinate,\n            int depth\n    ) {{\n    }}\n}}\n", "X".repeat(90), "X".repeat(60)));
        assert_eq!(java(lw, &lo), lo);
        // The method parameters' option doesn't wrap a record.
        assert_eq!(java(Wrapping { params: Wrap::ChopIfLong, ..Wrapping::JAVA }, &long).matches("record Point(int").count(), 1);
    }

    #[test]
    fn aligns_binary_operands() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "class A {\n    boolean k() {\n        boolean x = first &&\n                second &&\n                third;\n        if (a ||\n                b) {\n        }\n        int y = c ? d\n                : e;\n        return one +\n                two;\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let w = Wrapping { align_binary: true, ..Wrapping::JAVA };
        let o = java(w, t);
        // Operands under the first after `=`, `(` and `return`; a ternary keeps its continuation.
        assert_eq!(o, "class A {\n    boolean k() {\n        boolean x = first &&\n                    second &&\n                    third;\n        if (a ||\n            b) {\n        }\n        int y = c ? d\n                : e;\n        return one +\n               two;\n    }\n}\n");
        assert_eq!(java(w, &o), o);
    }

    #[test]
    fn aligns_ternaries_and_header_lists() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "class A extends B implements C,\n        D,\n        E {\n    void k() throws IOException,\n            InterruptedException {\n        String value = c\n                ? d\n                : e;\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let w = Wrapping { align_ternary: true, align_throws: true, align_extends: true, ..Wrapping::JAVA };
        let o = java(w, t);
        assert_eq!(o, "class A extends B implements C,\n                             D,\n                             E {\n    void k() throws IOException,\n                    InterruptedException {\n        String value = c\n                       ? d\n                       : e;\n    }\n}\n");
        assert_eq!(java(w, &o), o);
        // Only Throws list: the class header keeps its continuation.
        let only = java(Wrapping { align_throws: true, ..Wrapping::JAVA }, t);
        assert!(only.starts_with("class A extends B implements C,\n        D,\n        E {\n    void k() throws IOException,\n                    InterruptedException {\n"), "{only}");
    }

    #[test]
    fn aligns_chains_assignments_and_array_initializers() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "class A {\n    void k() {\n        String s = this.builder.append(a)\n                .append(b)\n                .toString();\n        x =\n                y = 5;\n        int[] v = {1, 2,\n                3};\n        list.stream()\n                .map(f);\n        stream\n                .map(f);\n        x = new A.B()\n                .c();\n        y = new A().b()\n                .c();\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let w = Wrapping { align_assignment: true, align_chains: true, align_arrays: true, ..Wrapping::JAVA };
        let o = java(w, t);
        assert_eq!(o, "class A {\n    void k() {\n        String s = this.builder.append(a)\n                               .append(b)\n                               .toString();\n        x =\n        y = 5;\n        int[] v = {1, 2,\n                   3};\n        list.stream()\n            .map(f);\n        stream\n                .map(f);\n        x = new A.B()\n                .c();\n        y = new A().b()\n                   .c();\n    }\n}\n");
        assert_eq!(java(w, &o), o);
    }

    #[test]
    fn lays_out_when_branches() {
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "fun f(x: Int) = when (x) {\n    1 -> \"one\"\n    10, 11 -> {\n        g { a -> a }\n    }\n    // comment\n    100 -> \"x\"\n    else -> \"y\"\n}\n";
        // Line break after multiline 'when' entry (on): a blank line after the block.
        let broken = "fun f(x: Int) = when (x) {\n    1 -> \"one\"\n    10, 11 -> {\n        g { a -> a }\n    }\n    // comment\n    100 -> \"x\"\n    else -> \"y\"\n}\n";
        assert_eq!(kotlin(Wrapping::KOTLIN, t), broken);
        let u = "val y = when {\n    a -> b(\n        c,\n    )\n    else -> d\n}\n";
        let o = kotlin(Wrapping::KOTLIN, u);
        assert_eq!(o, "val y = when {\n    a -> b(\n        c,\n    )\n\n    else -> d\n}\n");
        assert_eq!(kotlin(Wrapping::KOTLIN, &o), o);
        let off = Wrapping { when_break: false, ..Wrapping::KOTLIN };
        assert_eq!(kotlin(off, u), u);
        // Align 'when' branches in columns: the arrows in a column.
        let w = Wrapping { when_align: true, ..off };
        let a = kotlin(w, t);
        assert_eq!(a, "fun f(x: Int) = when (x) {\n    1      -> \"one\"\n    10, 11 -> {\n        g { a -> a }\n    }\n    // comment\n    100    -> \"x\"\n    else   -> \"y\"\n}\n");
        assert_eq!(kotlin(w, &a), a);
    }

    #[test]
    fn lays_out_deconstruction_lists() {
        assert!(deconstruction("if (o instanceof Point") && deconstruction("case Point") && deconstruction("case Line(Point(int x, int y), Point"));
        assert!(deconstruction("case Box<String>") && !deconstruction("foo(Bar") && !deconstruction("new Point") && !deconstruction("x instanceof point"));
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Align when multiline (on): components under the first; a call's arguments keep their continuation.
        let t = "class A {\n    void k(Object o) {\n        if (o instanceof Point(int x,\n                int y)) {\n            go(x,\n                    y);\n        }\n    }\n}\n";
        let off = Wrapping { decon_parens: [false; 2], ..Wrapping::JAVA };
        let aligned = "class A {\n    void k(Object o) {\n        if (o instanceof Point(int x,\n                               int y)) {\n            go(x,\n                    y);\n        }\n    }\n}\n";
        assert_eq!(java(off, t), aligned);
        assert_eq!(java(Wrapping { align_decon: false, ..off }, t), t);
        // New line after '(' and Place ')' on new line (on by default).
        let o = java(Wrapping::JAVA, t);
        assert_eq!(o, "class A {\n    void k(Object o) {\n        if (o instanceof Point(\n                int x,\n                int y\n        )) {\n            go(x,\n                    y);\n        }\n    }\n}\n");
        assert_eq!(java(Wrapping::JAVA, &o), o);
        // Deconstruction list: chop a long one.
        let long = format!("class A {{\n    void k(Object o) {{\n        switch (o) {{\n            case Point(int horizontal{}, int vertical, int depth) -> go();\n            default -> {{}}\n        }}\n    }}\n}}\n", "X".repeat(70));
        let c = java(Wrapping { decon_wrap: Wrap::ChopIfLong, ..off }, &long);
        assert!(c.contains(&format!("            case Point(int horizontal{},\n                       int vertical,\n                       int depth) -> go();\n", "X".repeat(70))), "{c}");
    }

    #[test]
    fn kotlin_use_continuation_indent() {
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), continuation: Some((8, 4)), ..Default::default() });
        let t = "class A(\n    val a: Int,\n) {\n    fun f(\n        x: Int,\n    ) = g(\n        x,\n    )\n\n    fun h() =\n        items\n            .map { it }\n\n    fun e() = a\n        ?: b\n\n    fun k() {\n        if (\n            a\n        ) {\n            go()\n        }\n    }\n}\n";
        // The official style's: all off, a normal indent even with a continuation of 8.
        assert_eq!(kotlin(Wrapping::KOTLIN, t), t);
        let on = Wrapping { kotlin_cont: [true; 8], ..Wrapping::KOTLIN };
        let o = kotlin(on, t);
        assert_eq!(o, "class A(\n        val a: Int,\n) {\n    fun f(\n            x: Int,\n    ) = g(\n            x,\n    )\n\n    fun h() =\n            items\n                    .map { it }\n\n    fun e() = a\n            ?: b\n\n    fun k() {\n        if (\n                a\n        ) {\n            go()\n        }\n    }\n}\n");
        assert_eq!(kotlin(on, &o), o);
        let header = "class D : B(),\n    C {\n}\n";
        assert_eq!(kotlin(Wrapping::KOTLIN, header), header);
        assert_eq!(kotlin(on, header), "class D : B(),\n        C {\n}\n");
        // Only Function call arguments.
        let args = kotlin(Wrapping { kotlin_cont: [false, true, false, false, false, false, false, false], ..Wrapping::KOTLIN }, t);
        assert!(args.contains("    fun f(\n        x: Int,\n    ) = g(\n            x,\n    )\n"), "{args}");
    }

    #[test]
    fn keeps_first_column_comments_and_flat_switches() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let t = "class A {\n    void k(int x) {\n//        go();\n/* a\n   b */\n        // here\n      // moved\n        switch (x) {\n            case 1:\n                go();\n                break;\n            default:\n                stop();\n        }\n    }\n}\n";
        // Comment at first column (on): those stay; an indented one goes with the code.
        let o = java(Wrapping::JAVA, t);
        assert_eq!(o, t.replace("      // moved", "        // moved"));
        let off = java(Wrapping { keep_first_column: false, ..Wrapping::JAVA }, t);
        assert!(off.contains("    void k(int x) {\n        //        go();\n        /* a\n   b */\n"), "{off}");
        // Indent 'case' branches off: the labels at the switch's level.
        let flat = java(Wrapping { indent_case: false, ..Wrapping::JAVA }, &o);
        assert!(flat.contains("        switch (x) {\n        case 1:\n            go();\n            break;\n        default:\n            stop();\n        }\n"), "{flat}");
        assert_eq!(java(Wrapping { indent_case: false, ..Wrapping::JAVA }, &flat), flat);
        // Kotlin: line comments only; context parameters' continuation.
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), continuation: Some((8, 4)), ..Default::default() });
        let k = "fun f() {\n//    go()\n    go()\n}\n";
        assert_eq!(kotlin(Wrapping::KOTLIN, k), k);
        let c = "context(\n    a: A,\n)\nfun g() {}\n";
        assert_eq!(kotlin(Wrapping::KOTLIN, c), c);
        let mut cont = [false; 8];
        cont[7] = true;
        assert_eq!(kotlin(Wrapping { kotlin_cont: cont, ..Wrapping::KOTLIN }, c), "context(\n        a: A,\n)\nfun g() {}\n");
    }

    #[test]
    fn wraps_first_calls_and_aligns_declaration_groups() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Wrap first call: the chain's first call may wrap when another follows.
        // Wrap if long: the first call's dot when no later one is within the margin.
        let long = "a".repeat(100);
        let chain = format!("class A {{\n    void f() {{\n        builder.append({long}).append(b);\n    }}\n}}\n");
        let on = Wrapping { chain: Wrap::WrapIfLong, wrap_first_call: true, ..Wrapping::JAVA };
        assert_eq!(java(on, &chain), format!("class A {{\n    void f() {{\n        builder\n                .append({long}).append(b);\n    }}\n}}\n"));
        assert_eq!(java(on, &java(on, &chain)), java(on, &chain));
        // Off (the default): the first call stays.
        assert_eq!(java(Wrapping { chain: Wrap::WrapIfLong, ..Wrapping::JAVA }, &chain), chain);
        // One call only: nothing to chain.
        let single = format!("class A {{\n    void f() {{\n        builder.append({long}, b);\n    }}\n}}\n");
        assert_eq!(java(on, &single), single);
        // Kotlin's too.
        let kotlin = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        let k = format!("fun f() {{\n    builder.append({long}).append(b)\n}}\n");
        let kon = Wrapping { call_args: Wrap::DoNotWrap, wrap_first_call: true, ..Wrapping::KOTLIN };
        assert_eq!(kotlin(kon, &k), format!("fun f() {{\n    builder\n        .append({long})\n        .append(b)\n}}\n"));
        assert_eq!(kotlin(Wrapping { wrap_first_call: false, ..kon }, &k), k);
        // Group declarations.
        let t = "class A {\n    private int a = 1;\n    private static final String name;\n    // note\n    protected List<String> items = new ArrayList<>();\n    int bare;\n\n    private long far = 2;\n\n    void f() {\n        int i = 0;\n        final String text = \"a = b\";\n        Map<String, Integer> m;\n        i = 1;\n        text2 = \"x\";\n        i += 2;\n        go();\n        long j = 3;\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let all = Wrapping { group_align: [true; 3], ..Wrapping::JAVA };
        let o = java(all, t);
        assert_eq!(o, "class A {\n    private              int          a     = 1;\n    private static final String       name;\n    // note\n    protected            List<String> items = new ArrayList<>();\n    int bare;\n\n    private long far = 2;\n\n    void f() {\n        int                  i    = 0;\n        final String         text = \"a = b\";\n        Map<String, Integer> m;\n        i     = 1;\n        text2 = \"x\";\n        i += 2;\n        go();\n        long j = 3;\n    }\n}\n");
        assert_eq!(java(all, &o), o);
        // Back to single spaces with the options off.
        assert_eq!(java(Wrapping::JAVA, &o), t);
        // One option at a time.
        let fields = java(Wrapping { group_align: [true, false, false], ..Wrapping::JAVA }, t);
        assert!(fields.contains("        int i = 0;\n") && fields.contains("    private              int          a     = 1;\n"), "{fields}");
    }

    #[test]
    fn splits_statements_and_wraps_modifier_lists() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Multiple expressions in one line off (the default): a statement a line.
        let t = "class A {\n    void f() {\n        int i = 1; go(i); stop();\n        for (int j = 0; j < 2; j++) run(() -> { a(); b(); });\n        x(); // done\n    }\n}\n";
        let o = java(Wrapping::JAVA, t);
        assert_eq!(o, "class A {\n    void f() {\n        int i = 1;\n        go(i);\n        stop();\n        for (int j = 0; j < 2; j++) run(() -> { a(); b(); });\n        x(); // done\n    }\n}\n");
        assert_eq!(java(Wrapping::JAVA, &o), o);
        // On: kept.
        assert_eq!(java(Wrapping { keep_multiple: true, ..Wrapping::JAVA }, t), t);
        // Wrap after modifier list.
        let m = "public class A {\n    @Override public void f() {}\n\n    private static final int X = 1;\n\n    public <T> T g(T t) {\n        final int y = 2;\n        return t;\n    }\n    static {\n    }\n}\n";
        let w = Wrapping { modifier_wrap: true, ..Wrapping::JAVA };
        let wrapped = java(w, m);
        assert_eq!(wrapped, "public\nclass A {\n    @Override public\n    void f() {}\n\n    private static final\n    int X = 1;\n\n    public\n    <T> T g(T t) {\n        final int y = 2;\n        return t;\n    }\n\n    static {\n    }\n}\n");
        assert_eq!(java(w, &wrapped), wrapped);
        assert_eq!(java(Wrapping::JAVA, m), m.replace("    }\n    static {", "    }\n\n    static {"));
        // A constructor's parameters stay under the first.
        let c = "class A {\n    public A(int a,\n             int b) {\n    }\n}\n";
        let cw = java(w, c);
        assert_eq!(cw, "class A {\n    public\n    A(int a,\n      int b) {\n    }\n}\n");
        assert_eq!(java(w, &cw), cw);
    }

    #[test]
    fn aligns_simple_methods_signs_and_breaks() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Align simple methods in columns: the bodies' `{`, past blank lines.
        let t = "class A {\n    int a() { return 1; }\n\n    public String name() { return n; }\n\n    // other\n    void go() { run(); }\n\n    void stop() { halt(); }\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let w = Wrapping { align_simple_methods: true, ..Wrapping::JAVA };
        let o = java(w, t);
        assert_eq!(o, "class A {\n    int a()              { return 1; }\n\n    public String name() { return n; }\n\n    // other\n    void go()   { run(); }\n\n    void stop() { halt(); }\n}\n");
        assert_eq!(java(w, &o), o);
        assert_eq!(java(Wrapping::JAVA, &o), t);
        // Assignment sign on next line.
        let long = "a".repeat(90);
        let a = format!("class A {{\n    void f() {{\n        String value = compute({long});\n    }}\n}}\n");
        let sign = Wrapping { assignment: Wrap::WrapIfLong, assign_sign_next: true, ..Wrapping::JAVA };
        let o = java(sign, &a);
        assert_eq!(o, format!("class A {{\n    void f() {{\n        String value\n                = compute({long});\n    }}\n}}\n"));
        assert_eq!(java(sign, &o), o);
        assert_eq!(java(Wrapping { assign_sign_next: false, ..sign }, &a), format!("class A {{\n    void f() {{\n        String value =\n                compute({long});\n    }}\n}}\n"));
        // Indent 'break' from 'case' off: `break` at the label's level.
        let b = "class A {\n    void f(int x) {\n        switch (x) {\n            case 1:\n                go();\n                if (y) break;\n                break;\n            default:\n                stop();\n        }\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, b), b);
        let flat = java(Wrapping { indent_break: false, ..Wrapping::JAVA }, b);
        assert_eq!(flat, b.replace("                break;\n", "            break;\n"));
        assert_eq!(java(Wrapping { indent_break: false, ..Wrapping::JAVA }, &flat), flat);
    }

    #[test]
    fn array_braces_and_single_annotations() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Array initializer › New line after '{' / Place '}' on new line.
        let t = "class A {\n    int[] a = {1, 2,\n            3};\n    int[] b = {1, 2};\n}\n";
        assert_eq!(java(Wrapping::JAVA, t), t);
        let w = Wrapping { array_parens: [true; 2], ..Wrapping::JAVA };
        let o = java(w, t);
        assert_eq!(o, "class A {\n    int[] a = {\n            1, 2,\n            3\n    };\n    int[] b = {1, 2};\n}\n");
        assert_eq!(java(w, &o), o);
        // Do not wrap after single annotation: fields and parameters.
        let f = "class A {\n    @Nullable String a;\n    @Nullable @Inject String b;\n\n    void f(@Nullable String x, @A @B int y) {\n    }\n}\n";
        let always = Wrapping { annotations: [AnnotationWrap::DoNotWrap, AnnotationWrap::DoNotWrap, AnnotationWrap::WrapAlways], param_annotations: AnnotationWrap::WrapAlways, ..Wrapping::JAVA };
        let wrapped = java(always, f);
        assert!(wrapped.contains("    @Nullable\n    String a;\n") && wrapped.contains("    void f(@Nullable\n           String x, @A\n"), "{wrapped}");
        let single = Wrapping { single_annotation: [true; 2], ..always };
        let kept = java(single, f);
        assert!(kept.contains("    @Nullable String a;\n    @Nullable\n    @Inject\n    String b;\n") && kept.contains("    void f(@Nullable String x, @A\n           @B\n           int y)"), "{kept}");
        assert_eq!(java(single, &kept), kept);
    }

    #[test]
    fn wraps_builder_methods() {
        let java = |w: Wrapping, names: &[&str], text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), builders: names.iter().map(|n| n.to_string()).collect(), ..Default::default() });
        let t = "class A {\n    void f() {\n        Foo f = Foo.builder().name(\"a\").add(g(1).add(2)).build();\n        list.add(1);\n        b.add(1).add(2);\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, &[], t), t);
        // The first call stays, every builder call after it wraps.
        let o = java(Wrapping::JAVA, &["add", "build"], t);
        assert_eq!(o, "class A {\n    void f() {\n        Foo f = Foo.builder().name(\"a\")\n                .add(g(1).add(2))\n                .build();\n        list.add(1);\n        b.add(1)\n                .add(2);\n    }\n}\n");
        assert_eq!(java(Wrapping::JAVA, &["add", "build"], &o), o);
        // Wrap first call: the first too, when a call follows it.
        let first = Wrapping { wrap_first_call: true, ..Wrapping::JAVA };
        let f = java(first, &["add"], t);
        assert!(f.contains("        list.add(1);\n        b\n                .add(1)\n                .add(2);\n"), "{f}");
        assert_eq!(java(first, &["add"], &f), f);
    }

    #[test]
    fn lays_out_else_if_and_for_statements() {
        let java = |w: Wrapping, text: &str| reformat_indented(ImportStyle::Java, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() });
        // Special 'else if' treatment on (the default): `else` / `if` join.
        let split = "class A {\n    void k() {\n        if (a) {\n            x();\n        } else\n        if (b) {\n            y();\n        } else {\n            z();\n        }\n    }\n}\n";
        let joined = "class A {\n    void k() {\n        if (a) {\n            x();\n        } else if (b) {\n            y();\n        } else {\n            z();\n        }\n    }\n}\n";
        assert_eq!(java(Wrapping::JAVA, split), joined);
        assert_eq!(java(Wrapping::JAVA, joined), joined);
        // Off: `if` on the next line, stepped in.
        let off = Wrapping { special_else_if: false, ..Wrapping::JAVA };
        let stepped = "class A {\n    void k() {\n        if (a) {\n            x();\n        } else\n            if (b) {\n                y();\n            } else {\n                z();\n            }\n    }\n}\n";
        assert_eq!(java(off, joined), stepped);
        assert_eq!(java(off, stepped), stepped);
        // 'for()' statement: Chop down if long, aligned under the first part.
        let head = format!("for (int index = startOfTheRange{}; index < endOfTheRangeThatIsLong; index += stepBetweenTheItems) {{", "X".repeat(30));
        let text = format!("class A {{\n    void k() {{\n        {head}\n            go(index);\n        }}\n    }}\n}}\n");
        assert_eq!(java(Wrapping::JAVA, &text), text);
        let chop = java(Wrapping { for_wrap: Wrap::ChopIfLong, ..Wrapping::JAVA }, &text);
        assert_eq!(chop, format!("class A {{\n    void k() {{\n        for (int index = startOfTheRange{};\n             index < endOfTheRangeThatIsLong;\n             index += stepBetweenTheItems) {{\n            go(index);\n        }}\n    }}\n}}\n", "X".repeat(30)));
        assert_eq!(java(Wrapping { for_wrap: Wrap::ChopIfLong, ..Wrapping::JAVA }, &chop), chop);
        // Wrap if long keeps what fits; without alignment, a continuation indent.
        let wrapped = java(Wrapping { for_wrap: Wrap::WrapIfLong, align_for: false, ..Wrapping::JAVA }, &text);
        assert!(wrapped.contains(&format!("startOfTheRange{}; index < endOfTheRangeThatIsLong;\n                index += stepBetweenTheItems) {{", "X".repeat(30))), "{wrapped}");
    }

    #[test]
    fn splits_simple_bodies_when_not_kept() {
        let java = |keep: [bool; 5], text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { keep_one_line: KeepOneLine::from_values(keep), ..Wrapping::JAVA }), ..Default::default() };
            reformat_indented(ImportStyle::Java, text, None, &style)
        };
        let text = "class A {\n    void f() { g(); h(); }\n\n    void e() {}\n\n    void k() {\n        x(new String[] {a}, new int[]{1});\n        if (a) { b(); } else { c(); }\n        run(() -> { go(); });\n        if (a) return;\n        for (int i = 0; i < n; i++) go(i);\n        int[] x = {1, 2};\n        class B {}\n    }\n}\n";
        assert_eq!(java([true; 5], text), text);
        // IntelliJ's defaults: only control statements stay.
        let all = java([false, false, false, false, true], text);
        assert_eq!(
            all,
            "class A {\n    void f() {\n        g();\n        h();\n    }\n\n    void e() {\n    }\n\n    void k() {\n        x(new String[] {a}, new int[]{1});\n        if (a) {\n            b();\n        } else {\n            c();\n        }\n        run(() -> {\n            go();\n        });\n        if (a) return;\n        for (int i = 0; i < n; i++) go(i);\n        int[] x = {1, 2};\n        class B {\n        }\n    }\n}\n"
        );
        assert_eq!(java([false, false, false, false, true], &all), all);
        // One kind at a time.
        assert_eq!(java([true, false, true, true, true], text), text.replace("void f() { g(); h(); }", "void f() {\n        g();\n        h();\n    }").replace("void e() {}", "void e() {\n    }"));
        assert_eq!(java([true, true, false, true, true], text), text.replace("run(() -> { go(); });", "run(() -> {\n            go();\n        });"));
        let control = java([true, true, true, true, false], text);
        assert!(control.contains("        if (a)\n            return;\n        for (int i = 0; i < n; i++)\n            go(i);\n"), "{control}");
        assert_eq!(java([true, true, true, true, false], &control), control);
        let chain = "class A {\n    void k() {\n        if (a) b();\n        else if (c) d();\n        else e();\n    }\n}\n";
        assert_eq!(java([true, true, true, true, false], chain), "class A {\n    void k() {\n        if (a)\n            b();\n        else if (c)\n            d();\n        else\n            e();\n    }\n}\n");
    }

    #[test]
    fn wraps_kotlin_annotations() {
        let kotlin = |wraps: [AnnotationWrap; 4], text: &str| {
            let w = Wrapping { annotations: [wraps[0], wraps[1], wraps[2]], local_annotations: wraps[3], ..Wrapping::KOTLIN };
            reformat_indented(ImportStyle::Kotlin, text, None, &IndentStyle { wrapping: Some(w), ..Default::default() })
        };
        let text = "@Parcelize data class A(@Inject val b: Int) {\n    @get:Rule @JvmField val rule = Rule()\n\n    @Composable override fun f() {\n        @Suppress(\"x\") val y = 1\n        @OptIn(X::class) go()\n    }\n}\n\n@Test fun top() {\n}\n";
        use AnnotationWrap::*;
        assert_eq!(kotlin([DoNotWrap; 4], text), text);
        assert_eq!(
            kotlin([WrapAlways; 4], text),
            "@Parcelize\ndata class A(@Inject val b: Int) {\n    @get:Rule\n    @JvmField\n    val rule = Rule()\n\n    @Composable\n    override fun f() {\n        @Suppress(\"x\")\n        val y = 1\n        @OptIn(X::class) go()\n    }\n}\n\n@Test\nfun top() {\n}\n"
        );
        // Each kind its own option: only properties here.
        assert_eq!(
            kotlin([DoNotWrap, DoNotWrap, WrapAlways, DoNotWrap], text),
            text.replace("@get:Rule @JvmField val", "@get:Rule\n    @JvmField\n    val")
        );
        let out = kotlin([WrapAlways; 4], text);
        assert_eq!(kotlin([WrapAlways; 4], &out), out);
    }

    #[test]
    fn wraps_parameter_annotations() {
        let java = |wrap: AnnotationWrap, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { param_annotations: wrap, ..Wrapping::JAVA }), ..Default::default() };
            reformat_indented(ImportStyle::Java, text, None, &style)
        };
        let text = "class A {\n    void f(@NonNull String a, @Nullable @Size(max = 3) String b, int c) {\n        go();\n    }\n}\n";
        assert_eq!(java(AnnotationWrap::DoNotWrap, text), text);
        // Short: only Wrap always breaks; a single parameter's annotations each end a line.
        assert_eq!(java(AnnotationWrap::ChopIfLong, text), text);
        assert_eq!(
            java(AnnotationWrap::WrapAlways, text),
            "class A {\n    void f(@NonNull\n           String a, @Nullable\n           @Size(max = 3)\n           String b, int c) {\n        go();\n    }\n}\n"
        );
        // Chopped parameters: each one's annotation on its own line.
        let chopped = "class A {\n    public A(@NonNull String a,\n             @Named(\"b, c\") int b) {\n    }\n}\n";
        assert_eq!(java(AnnotationWrap::WrapAlways, chopped), "class A {\n    public A(@NonNull\n             String a,\n             @Named(\"b, c\")\n             int b) {\n    }\n}\n");
        // Long: Wrap if long breaks where the line passes the margin.
        let long = format!("class A {{\n    void loadEverything(@NonNull String firstArgument, @Nullable String secondArgument, @Named(\"{}\") int c) {{\n    }}\n}}\n", "x".repeat(20));
        let out = java(AnnotationWrap::WrapIfLong, &long);
        assert!(out.lines().all(|l| l.len() <= 120), "{out}");
        assert_eq!(java(AnnotationWrap::WrapIfLong, &out), out);
        // Calls and annotations elsewhere don't change.
        let call = "class A {\n    void f() {\n        g(@A x, y);\n    }\n}\n";
        assert_eq!(java(AnnotationWrap::WrapAlways, call), call);
    }

    #[test]
    fn keeps_two_spaces_with_aligned_parameters() {
        // Parameters aligned at odd columns don't make a 2-space file 4.
        let text = "class A {\n  void draw(int a,\n            int b) {\n    go();\n  }\n\n  void x(int c,\n         int d) {\n    go();\n  }\n\n  void y() {\n    if (a) {\n      go();\n    }\n  }\n}\n";
        assert_eq!(reformat(ImportStyle::Java, text, None), text);
    }

    #[test]
    fn wraps_throws_keyword_and_annotation_parameters() {
        let w = |wrapping: Wrapping, text: &str| {
            let style = IndentStyle { wrapping: Some(wrapping), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "class A {\n    public static Bitmap decodeSampledBitmapFromStream(InputStream stream, int width, int height, int sample) throws IOException {\n        g();\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, text), text);
        assert_eq!(
            w(Wrapping { throws_keyword: Wrap::WrapIfLong, ..Wrapping::JAVA }, text),
            "class A {\n    public static Bitmap decodeSampledBitmapFromStream(InputStream stream, int width, int height, int sample)\n            throws IOException {\n        g();\n    }\n}"
        );
        // Written that way: kept, the body the method's.
        let laid = "class A {\n    void f()\n    throws IOException {\n        g();\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, laid), "class A {\n    void f()\n            throws IOException {\n        g();\n    }\n}");
        let annotated = "class A {\n    @Query(value = \"SELECT * FROM messages WHERE thread_id = :threadId\", nativeQuery = false, countQuery = \"SELECT COUNT(*)\")\n    void f() {\n    }\n}";
        assert_eq!(w(Wrapping::JAVA, annotated), annotated);
        // Method call arguments don't reach annotations; their own option does.
        assert_eq!(w(Wrapping { call_args: Wrap::ChopIfLong, ..Wrapping::JAVA }, annotated), annotated);
        assert_eq!(
            w(Wrapping { annotation_params: Wrap::ChopIfLong, ..Wrapping::JAVA }, annotated),
            "class A {\n    @Query(value = \"SELECT * FROM messages WHERE thread_id = :threadId\",\n            nativeQuery = false,\n            countQuery = \"SELECT COUNT(*)\")\n    void f() {\n    }\n}"
        );
    }

    #[test]
    fn wraps_enum_constants() {
        let w = |wrap: AnnotationWrap, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { enum_constants: wrap, ..Wrapping::JAVA }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Java, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Java, &once, None, &style), once, "stable");
            once
        };
        let text = "enum Color {\n    RED(\"r, g\"), GREEN(1, 2), BLUE;\n\n    void f() {\n        g(a, b);\n    }\n}\n\nclass A {\n    enum Size { S, M }\n\n    enum Mode {\n        ON, OFF, // two\n        AUTO { }, MANUAL\n    }\n}";
        assert_eq!(w(AnnotationWrap::DoNotWrap, text), text);
        assert_eq!(
            w(AnnotationWrap::WrapAlways, text),
            "enum Color {\n    RED(\"r, g\"),\n    GREEN(1, 2),\n    BLUE;\n\n    void f() {\n        g(a, b);\n    }\n}\n\nclass A {\n    enum Size { S, M }\n\n    enum Mode {\n        ON, OFF, // two\n        AUTO { }, MANUAL\n    }\n}"
        );
        let long = "enum Permission {\n    READ_EXTERNAL_STORAGE, WRITE_EXTERNAL_STORAGE, ACCESS_FINE_LOCATION, ACCESS_COARSE_LOCATION, CAMERA, RECORD_AUDIO, READ_CONTACTS\n}";
        assert_eq!(w(AnnotationWrap::WrapAlways, "enum E {\n    A, B\n}"), "enum E {\n    A,\n    B\n}");
        assert_eq!(w(AnnotationWrap::ChopIfLong, "enum E {\n    A, B\n}"), "enum E {\n    A, B\n}");
        assert_eq!(
            w(AnnotationWrap::WrapIfLong, long),
            "enum Permission {\n    READ_EXTERNAL_STORAGE, WRITE_EXTERNAL_STORAGE, ACCESS_FINE_LOCATION, ACCESS_COARSE_LOCATION, CAMERA, RECORD_AUDIO,\n    READ_CONTACTS\n}"
        );
    }

    #[test]
    fn lays_out_again_after_chopping() {
        // Wrapped after `=`: the lambda's body follows the new continuation
        // line, and a second pass changes nothing.
        let text = "class A {\n  private val mediaProjectionLauncher = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->\n    if (result.resultCode == RESULT_OK) {\n      start(result.data!!)\n    }\n  }\n}";
        let style = IndentStyle { unit: Some("  ".into()), ..Default::default() };
        let once = reformat_indented(ImportStyle::Kotlin, text, None, &style);
        assert_eq!(
            once,
            "class A {\n  private val mediaProjectionLauncher =\n    registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->\n      if (result.resultCode == RESULT_OK) {\n        start(result.data!!)\n      }\n    }\n}"
        );
        assert_eq!(reformat_indented(ImportStyle::Kotlin, &once, None, &style), once);
        // A selection grows by the lines chopping adds.
        assert_eq!(reformat_indented(ImportStyle::Kotlin, text, Some(0..7), &style), once);
    }

    #[test]
    fn wraps_elvis_expressions() {
        let w = |wrap: Wrap, text: &str| {
            let style = IndentStyle { wrapping: Some(Wrapping { elvis: wrap, ..Wrapping::KOTLIN }), ..Default::default() };
            let once = reformat_indented(ImportStyle::Kotlin, text, None, &style);
            assert_eq!(reformat_indented(ImportStyle::Kotlin, &once, None, &style), once, "stable");
            once
        };
        let long = "fun f() {\n    val name = intent.getStringExtra(EXTRA_DISPLAY_NAME) ?: savedInstanceState?.getString(KEY_NAME) ?: defaultName(context)\n}";
        // Off: the argument list chops as before.
        assert_eq!(w(Wrap::DoNotWrap, long), "fun f() {\n    val name = intent.getStringExtra(EXTRA_DISPLAY_NAME) ?: savedInstanceState?.getString(KEY_NAME) ?: defaultName(\n        context\n    )\n}");
        assert_eq!(
            w(Wrap::WrapIfLong, long),
            "fun f() {\n    val name = intent.getStringExtra(EXTRA_DISPLAY_NAME) ?: savedInstanceState?.getString(KEY_NAME)\n        ?: defaultName(context)\n}"
        );
        assert_eq!(
            w(Wrap::ChopIfLong, long),
            "fun f() {\n    val name = intent.getStringExtra(EXTRA_DISPLAY_NAME)\n        ?: savedInstanceState?.getString(KEY_NAME)\n        ?: defaultName(context)\n}"
        );
        // Short lines, and `?:` inside brackets or a string template, stay.
        let short = "fun f() {\n    val a = b ?: c\n    log(\"${x ?: \"none\"} ${aVeryLongTemplateExpressionName.withSomeMoreCalls().andEvenMore()} padding padding padding\")\n}";
        assert_eq!(w(Wrap::ChopIfLong, short), short);
    }

    #[test]
    fn joins_line_breaks_when_not_kept() {
        let f = |kotlin: bool, w: Wrapping, text: &str, only: Option<Range<usize>>| {
            let style = IndentStyle { wrapping: Some(Wrapping { keep_line_breaks: false, ..w }), ..Default::default() };
            let st = if kotlin { ImportStyle::Kotlin } else { ImportStyle::Java };
            let once = reformat_indented(st, text, only.clone(), &style);
            if only.is_none() {
                assert_eq!(reformat_indented(st, &once, None, &style), once, "stable");
            }
            once
        };
        let java = "class A {\n    void f(int a,\n           int b) {\n        int x = a +\n                b;\n        g(a, // why\n          b);\n        s.append(a)\n                .append(b);\n        if (a > 0 &&\n                b > 0) {\n            h();\n        }\n    }\n}";
        assert_eq!(
            f(false, Wrapping::JAVA, java, None),
            "class A {\n    void f(int a, int b) {\n        int x = a + b;\n        g(a, // why\n                b);\n        s.append(a).append(b);\n        if (a > 0 && b > 0) {\n            h();\n        }\n    }\n}"
        );
        // Only the selected lines.
        assert_eq!(f(false, Wrapping::JAVA, java, Some(1..3)), "class A {\n    void f(int a, int b) {\n        int x = a +\n                b;\n        g(a, // why\n          b);\n        s.append(a)\n                .append(b);\n        if (a > 0 &&\n                b > 0) {\n            h();\n        }\n    }\n}");
        // Kotlin: joined, then chopped again when long; a trailing comma keeps the list as it is.
        let kt = "fun f() {\n    val x =\n        listOf(\n            1,\n            2\n        )\n    val y = listOf(\n        1,\n        2,\n    )\n    val z = s\n        ?.trim()\n        ?: \"\"\n    val t: String?\n    import()\n}";
        assert_eq!(f(true, Wrapping::KOTLIN, kt, None), "fun f() {\n    val x = listOf(1, 2)\n    val y = listOf(\n        1,\n        2,\n    )\n    val z = s?.trim() ?: \"\"\n    val t: String?\n    import()\n}");
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
        // The code style's: Kotlin continuing by 8 (Expression body functions' Use continuation indent on), a new file's unit 2.
        let wrapping = Some(Wrapping { kotlin_cont: [false, false, true, false, false, false, false, false], ..Wrapping::KOTLIN });
        let style = IndentStyle { unit: None, fallback: Some("  ".into()), continuation: Some((8, 4)), wrapping, ..Default::default() };
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
    /// Formatted sources reformat to themselves (set JUNCTION_FMT_DIR;
    /// JUNCTION_FMT_OUT writes the results to a directory to diff).
    #[test]
    #[ignore]
    fn formatted_sources_are_stable() {
        let Ok(dir) = std::env::var("JUNCTION_FMT_DIR") else { return };
        let mut unstable: Vec<(String, usize)> = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let text = std::fs::read_to_string(&path).unwrap();
            let style = if path.extension().is_some_and(|e| e == "kt") { super::ImportStyle::Kotlin } else { super::ImportStyle::Java };
            let out = super::reformat(style, &text, None);
            // JUNCTION_FMT_OUT: also write each result there, to diff.
            if let Ok(out_dir) = std::env::var("JUNCTION_FMT_OUT") {
                std::fs::write(std::path::Path::new(&out_dir).join(path.file_name().unwrap()), &out).unwrap();
            }
            let changed: Vec<(usize, &str, &str)> = text.lines().zip(out.lines()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, b))| (i + 1, a, b)).collect();
            println!("{} {} changed", path.file_name().unwrap().to_string_lossy(), changed.len());
            for (line, a, b) in changed.iter().take(6) {
                println!("  {line}: {a:?}\n   → {b:?}");
            }
            // The second pass must change nothing.
            let again = super::reformat(style, &out, None);
            if again != out {
                let moved: Vec<(usize, &str, &str)> = out.lines().zip(again.lines()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, b))| (i + 1, a, b)).collect();
                let moved_count = moved.len().max(1);
                println!("UNSTABLE {} {moved_count} lines", path.file_name().unwrap().to_string_lossy());
                for (line, a, b) in moved.iter().take(6) {
                    println!("  {line}: {a:?}\n   → {b:?}");
                }
                unstable.push((path.file_name().unwrap().to_string_lossy().into_owned(), moved_count));
            }
        }
        let total: usize = unstable.iter().map(|(_, n)| n).sum();
        println!("unstable: {} files, {total} lines", unstable.len());
        assert!(unstable.is_empty(), "a second Reformat Code pass changed {total} lines in {} files", unstable.len());
    }
}


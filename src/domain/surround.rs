//! Surround With (Ctrl+Alt+T), as IntelliJ's: the surrounders its popup
//! lists for statements and for an expression, in its order and with its
//! names (KotlinStatementSurroundDescriptor, JavaStatementsSurroundDescriptor,
//! the expression descriptors), and the edit wrapping the lines or the
//! expression, the body indented one level, the caret or selection where
//! the template's first placeholder is.

use std::ops::Range;

use super::imports::ImportStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surrounder {
    If,
    IfElse,
    Run,
    TryFinally,
    TryCatchFinally,
    TryCatch,
    While,
    DoWhile,
    For,
    Synchronized,
    Runnable,
    Block,
    /// Expressions.
    Not,
    StringTemplate,
    Parentheses,
    Cast,
    NotInstanceof,
    /// `if (expr) {…}` for a whole statement (Java) or a Boolean (Kotlin).
    IfExpression,
    IfElseExpression,
    NullCheck,
    When,
    TryCatchExpression,
    TryCatchFinallyExpression,
    TryFinallyExpression,
    /// Kotlin `if () expr else` / `if () { expr } else {}`.
    IfAround,
    IfAroundBraces,
}

impl Surrounder {
    /// The popup's text for it.
    pub fn label(self, style: ImportStyle) -> &'static str {
        let kotlin = style == ImportStyle::Kotlin;
        match self {
            Surrounder::If => "if",
            Surrounder::IfElse => "if / else",
            Surrounder::Run => "run { }",
            Surrounder::TryFinally => "try / finally",
            Surrounder::TryCatchFinally => "try / catch / finally",
            Surrounder::TryCatch => "try / catch",
            Surrounder::While => "while",
            Surrounder::DoWhile => "do / while",
            Surrounder::For => "for",
            Surrounder::Synchronized => "synchronized",
            Surrounder::Runnable => "Runnable",
            Surrounder::Block => "{ }",
            Surrounder::Not => "!(expr)",
            Surrounder::StringTemplate => "\"${expr}\"",
            Surrounder::Parentheses => "(expr)",
            Surrounder::Cast => "((Type)expr)",
            Surrounder::NotInstanceof => "!(expr instanceof Type)",
            Surrounder::IfExpression if kotlin => "if (expr) { ... }",
            Surrounder::IfExpression => "if (expr) {...}",
            Surrounder::IfElseExpression if kotlin => "if (expr) { ... } else { ... }",
            Surrounder::IfElseExpression => "if (expr) {...} else {...}",
            Surrounder::NullCheck => "if (expr != null) {...}",
            Surrounder::When => "when (expr) {}",
            Surrounder::TryCatchExpression => "try { expr } catch {}",
            Surrounder::TryCatchFinallyExpression => "try { expr } catch {} finally {}",
            Surrounder::TryFinallyExpression => "try { expr } finally {}",
            Surrounder::IfAround => "if () expr else",
            Surrounder::IfAroundBraces => "if () { expr } else {}",
        }
    }
}

/// The surrounders for whole statements (lines), in IntelliJ's order.
pub fn statement_surrounders(style: ImportStyle) -> &'static [Surrounder] {
    use Surrounder::*;
    match style {
        ImportStyle::Kotlin => &[If, IfElse, Run, TryFinally, TryCatchFinally, TryCatch, While, DoWhile, For],
        ImportStyle::Java => &[If, IfElse, While, DoWhile, For, TryCatch, TryFinally, TryCatchFinally, Synchronized, Runnable, Block],
    }
}

/// What the expression surrounders need to know of the expression.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpressionContext {
    /// Its type, when known (`Boolean`, `int`, `String`).
    pub ty: Option<String>,
    /// The statement it is all of, in a block: Java's with its `;`
    /// (`if (expr) {...}` replaces it); Kotlin's its own range (a value
    /// not used, so no `try` / `if () expr else` around it).
    pub statement: Option<Range<usize>>,
    /// Java: the expression statement in a block it is part of
    /// (`if (expr != null) {...}` wraps it).
    pub enclosing: Option<Range<usize>>,
    /// Kotlin: an `if` / `when` / `try` in its place needs parentheses
    /// (`(if (a) {…}).equals(true)`); Java: a cast needs them inside
    /// (`((Type) (a + b))`).
    pub parenthesize: bool,
}

impl ExpressionContext {
    fn boolean(&self, kotlin: bool) -> bool {
        // Unknown types aren't ruled out.
        self.ty.as_deref().is_none_or(|t| if kotlin { t == "Boolean" } else { t == "boolean" || t == "Boolean" })
    }

    fn primitive(&self) -> bool {
        self.ty.as_deref().is_some_and(|t| matches!(t, "boolean" | "int" | "long" | "short" | "byte" | "char" | "float" | "double" | "void"))
    }
}

/// The surrounders for an expression, in IntelliJ's order, those that
/// apply to it (KotlinExpressionSurroundDescriptor,
/// JavaExpressionSurroundDescriptor); without a context (the selection
/// isn't one expression) the ones that only wrap its text.
pub fn expression_surrounders(style: ImportStyle, context: Option<&ExpressionContext>) -> Vec<Surrounder> {
    use Surrounder::*;
    let kotlin = style == ImportStyle::Kotlin;
    let Some(c) = context else {
        return if kotlin { vec![Not, StringTemplate, Parentheses] } else { vec![Parentheses, Not] };
    };
    let all: &[Surrounder] = if kotlin {
        &[Not, StringTemplate, Parentheses, When, IfExpression, IfElseExpression, TryCatchExpression, TryCatchFinallyExpression, TryFinallyExpression, IfAround, IfAroundBraces]
    } else {
        &[Parentheses, Cast, Not, NotInstanceof, IfExpression, IfElseExpression, NullCheck]
    };
    all.iter()
        .copied()
        .filter(|s| match s {
            Not | IfExpression | IfElseExpression if kotlin => c.boolean(true),
            Not => c.boolean(false),
            IfExpression | IfElseExpression => c.statement.is_some() && c.boolean(false),
            Cast => c.ty.as_deref() != Some("void"),
            NotInstanceof => !c.primitive(),
            NullCheck => !c.primitive() && c.statement.is_none() && c.enclosing.is_some(),
            // A value not used: KotlinControlFlowExpressionSurrounderBase.
            TryCatchExpression | TryCatchFinallyExpression | TryFinallyExpression | IfAround | IfAroundBraces => c.statement.is_none(),
            _ => true,
        })
        .collect()
}

/// IntelliJ's popup numbering: 1–9, 0, then letters.
pub fn mnemonic(index: usize) -> char {
    match index {
        0..=8 => (b'1' + index as u8) as char,
        9 => '0',
        n => (b'A' + (n - 10) as u8) as char,
    }
}

/// The edit: the range it replaces, the new text, and what to select in
/// the new text afterwards (empty: the caret).
#[derive(Debug, PartialEq, Eq)]
pub struct Surrounded {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
    /// More than one place to fill in (`for (item in items)`, Java's
    /// `for (; ; )`): each one, in order, Tab going from one to the next as
    /// in a live template, and where the caret ends (the header's end).
    pub stops: Vec<Range<usize>>,
    pub end: usize,
}

/// The whole lines the selection touches (to their line break), or the
/// caret's line.
pub fn lines_of(text: &str, selection: Range<usize>) -> Range<usize> {
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    // A selection ending at a line start doesn't take that line.
    let last = if selection.end > selection.start && text[..selection.end].ends_with('\n') { selection.end - 1 } else { selection.end };
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i + 1);
    start..end
}

/// The indentation step of the file: a tab, or the smallest indentation.
pub fn indent_unit(text: &str) -> String {
    if text.lines().any(|l| l.starts_with('\t')) {
        return "\t".into();
    }
    let smallest = text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start_matches(' ').len()).filter(|n| *n > 0).min();
    " ".repeat(smallest.unwrap_or(4))
}

/// Wraps the lines `range` (whole lines) with a statement surrounder.
pub fn surround_lines(style: ImportStyle, surrounder: Surrounder, text: &str, range: Range<usize>, unit: &str) -> Surrounded {
    let block = &text[range.clone()];
    let base: String = block.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take_while(|c| c.is_whitespace()).collect();
    let deeper = |extra: &str| -> String {
        block
            .lines()
            .map(|l| if l.trim().is_empty() { String::new() } else { format!("{extra}{l}") })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let kotlin = style == ImportStyle::Kotlin;
    // (header, footer, placeholder in header or footer, body indent).
    let catch = if kotlin { format!("catch (e: Exception) {{\n{base}{unit}throw e\n{base}}}") } else { format!("catch (Exception e) {{\n{base}{unit}throw new RuntimeException(e);\n{base}}}") };
    let empty_block = |word: &str| format!("{word} {{\n{base}{unit}\n{base}}}");
    let (header, footer, placeholder, extra): (String, String, &str, String) = match surrounder {
        Surrounder::If => ("if () {".into(), "}".into(), "()", unit.into()),
        Surrounder::IfElse => ("if () {".into(), format!("}} {}", empty_block("else")), "()", unit.into()),
        Surrounder::Run => ("run {".into(), "}".into(), "", unit.into()),
        Surrounder::TryFinally => ("try {".into(), format!("}} {}", empty_block("finally")), "", unit.into()),
        Surrounder::TryCatchFinally => ("try {".into(), format!("}} {catch} {}", empty_block("finally")), "Exception", unit.into()),
        Surrounder::TryCatch => ("try {".into(), format!("}} {catch}"), "Exception", unit.into()),
        Surrounder::While => ("while (true) {".into(), "}".into(), "true", unit.into()),
        Surrounder::DoWhile => ("do {".into(), if kotlin { "} while (true)".into() } else { "} while (true);".into() }, "true", unit.into()),
        Surrounder::For => {
            if kotlin { ("for (item in items) {".into(), "}".into(), "item in items", unit.into()) } else { ("for (; ; ) {".into(), "}".into(), "(", unit.into()) }
        }
        Surrounder::Synchronized => ("synchronized () {".into(), "}".into(), "()", unit.into()),
        Surrounder::Runnable => (
            format!("Runnable runnable = new Runnable() {{\n{base}{unit}public void run() {{"),
            format!("{unit}}}\n{base}}};"),
            "runnable",
            format!("{unit}{unit}"),
        ),
        Surrounder::Block => ("{".into(), "}".into(), "", unit.into()),
        // Not statement surrounders.
        _ => ("".into(), "".into(), "", String::new()),
    };
    let body = deeper(&extra);
    let mut out = format!("{base}{header}\n{body}\n{base}{footer}");
    if block.ends_with('\n') {
        out.push('\n');
    }
    // The placeholder: inside `()` (the caret), or the word selected; with
    // none, the caret at the end of the header's line.
    let header_at = range.start + base.len();
    let select = match placeholder {
        "()" | "(" => {
            let at = header_at + header.find('(').map_or(0, |i| i + 1);
            at..at
        }
        "" => {
            let at = header_at + header.len();
            at..at
        }
        word => match out.find(word) {
            Some(i) => range.start + i..range.start + i + word.len(),
            None => header_at..header_at,
        },
    };
    let header_end = header_at + header.find('\n').unwrap_or(header.len());
    let stops: Vec<Range<usize>> = match (surrounder, kotlin) {
        (Surrounder::For, true) => {
            let at = header_at + "for (".len();
            vec![at..at + "item".len(), at + "item in ".len()..at + "item in items".len()]
        }
        (Surrounder::For, false) => {
            let at = header_at + "for (".len();
            vec![at..at, at + "; ".len()..at + "; ".len(), at + "; ; ".len()..at + "; ; ".len()]
        }
        _ => Vec::new(),
    };
    Surrounded { range, text: out, select, stops, end: header_end }
}

/// Whether a statement surrounder moves the declarations used after the
/// lines out in front of them (IntelliJ's MoveDeclarationsOutHelper and
/// SurroundWithUtil.moveDeclarationsOut), and whether a `var` (a non-final
/// Java variable) moved out keeps a default value (`0`, `false`, `null`)
/// as its initializer: `Some(default)`, or None for no move.
pub fn moves_declarations(style: ImportStyle, surrounder: Surrounder) -> Option<bool> {
    use Surrounder::*;
    match (style, surrounder) {
        (ImportStyle::Kotlin, If | Run | TryFinally | TryCatchFinally | TryCatch) => Some(true),
        (ImportStyle::Kotlin, IfElse) => Some(false),
        // The Kotlin loops don't move them (KotlinLoopSurrounderBase).
        (ImportStyle::Kotlin, _) => None,
        (ImportStyle::Java, If | While | For | TryCatch | TryCatchFinally) => Some(true),
        (ImportStyle::Java, IfElse | DoWhile | TryFinally | Synchronized | Block) => Some(false),
        (ImportStyle::Java, _) => None,
    }
}

/// The declarations a surrounder moves out: the lines it wraps instead
/// (`a = 1` for `val a = 1`), the declarations in front of the construct
/// in order (`val a: Int`), and Kotlin's `val b = ` before it when the
/// last line declared what is used after (`val b = if (…) { … }`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclarationsOut {
    pub block: String,
    pub declarations: Vec<String>,
    pub prefix: String,
}

/// `surround_lines` with the declarations moved out.
pub fn surround_lines_moving(style: ImportStyle, surrounder: Surrounder, text: &str, range: Range<usize>, unit: &str, out: &DeclarationsOut) -> Surrounded {
    let mut moved = String::with_capacity(text.len());
    moved.push_str(&text[..range.start]);
    moved.push_str(&out.block);
    moved.push_str(&text[range.end..]);
    let done = surround_lines(style, surrounder, &moved, range.start..range.start + out.block.len(), unit);
    let base: String = done.text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut front: String = out.declarations.iter().map(|d| format!("{base}{d}\n")).collect();
    front.push_str(&base);
    front.push_str(&out.prefix);
    let shift = front.len() - base.len();
    let at = |r: &Range<usize>| r.start + shift..r.end + shift;
    Surrounded {
        range,
        text: format!("{front}{}", &done.text[base.len()..]),
        select: at(&done.select),
        stops: done.stops.iter().map(at).collect(),
        end: done.end + shift,
    }
}

/// Wraps an expression (within a line) with an expression surrounder.
pub fn surround_expression(style: ImportStyle, surrounder: Surrounder, text: &str, range: Range<usize>, unit: &str, context: &ExpressionContext) -> Surrounded {
    let expr = &text[range.clone()];
    let line = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let base: String = text[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let inner = format!("{base}{unit}");
    // (the text, where the caret goes or what is selected, in it).
    let (out, select): (String, Range<usize>) = match surrounder {
        Surrounder::Not => (format!("!({expr})"), usize::MAX..usize::MAX),
        Surrounder::StringTemplate => (format!("\"${{{expr}}}\""), usize::MAX..usize::MAX),
        Surrounder::Cast => {
            let expr = if context.parenthesize { format!("({expr})") } else { expr.to_owned() };
            (format!("(() {expr})"), 2..2)
        }
        Surrounder::NotInstanceof => {
            let out = format!("!({expr} instanceof )");
            let at = out.len() - 1;
            (out, at..at)
        }
        Surrounder::IfExpression | Surrounder::IfElseExpression if style == ImportStyle::Java => {
            let statement = context.statement.clone().unwrap_or(range.clone());
            let otherwise = if surrounder == Surrounder::IfElseExpression { format!(" else {{\n{base}}}") } else { String::new() };
            let out = format!("if ({expr}) {{\n{inner}\n{base}}}{otherwise}");
            let at = out.find('\n').unwrap() + 1 + inner.len();
            return expression_edit(statement, out, at..at);
        }
        Surrounder::NullCheck => {
            let statement = context.enclosing.clone().unwrap_or(range.clone());
            let out = format!("if ({expr} != null) {{\n{inner}{}\n{base}}}", &text[statement.clone()]);
            let at = out.rfind('\n').unwrap();
            return expression_edit(statement, out, at..at);
        }
        Surrounder::When => {
            let (branches, at) = match context.ty.as_deref() {
                // computeMissingCases: the caret on the first branch's value.
                Some("Boolean") => (format!("{inner}true -> TODO()\n{inner}false -> TODO()\n"), inner.len() + "true -> ".len()),
                _ => (format!("{inner} -> {{}}\n{inner}else -> {{}}\n"), inner.len()),
            };
            let head = format!("when ({expr}) {{\n");
            let at = head.len() + at;
            (format!("{head}{branches}{base}}}"), at..at)
        }
        Surrounder::IfExpression | Surrounder::IfElseExpression => {
            let otherwise = if surrounder == Surrounder::IfElseExpression { format!(" else {{\n{base}}}") } else { String::new() };
            let out = format!("if ({expr}) {{\n{inner}\n{base}}}{otherwise}");
            let at = out.find('\n').unwrap() + 1 + inner.len();
            (out, at..at)
        }
        Surrounder::TryCatchExpression | Surrounder::TryCatchFinallyExpression => {
            let finally = if surrounder == Surrounder::TryCatchFinallyExpression { format!(" finally {{\n{base}}}") } else { String::new() };
            let out = format!("try {{\n{inner}{expr}\n{base}}} catch (e: Exception) {{\n{inner}throw e\n{base}}}{finally}");
            let at = out.find("Exception").unwrap();
            (out, at..at + "Exception".len())
        }
        Surrounder::TryFinallyExpression => {
            let out = format!("try {{\n{inner}{expr}\n{base}}} finally {{\n{inner}\n{base}}}");
            let at = out.len() - 1 - base.len() - 1;
            (out, at..at)
        }
        Surrounder::IfAround => {
            let out = format!("if () {expr} else");
            (out, 4..4)
        }
        Surrounder::IfAroundBraces => (format!("if () {{\n{inner}{expr}\n{base}}} else {{\n{base}}}"), 4..4),
        _ => (format!("({expr})"), usize::MAX..usize::MAX),
    };
    // A Kotlin `if` / `when` / `try` taking an operand's place.
    let wrapped = style == ImportStyle::Kotlin && context.parenthesize && !matches!(surrounder, Surrounder::Not | Surrounder::StringTemplate | Surrounder::Parentheses);
    let (out, shift) = if wrapped { (format!("({out})"), 1) } else { (out, 0) };
    // The caret after the new text unless placed.
    let select = if select.start == usize::MAX { out.len()..out.len() } else { select.start + shift..select.end + shift };
    expression_edit(range, out, select)
}

/// The edit replacing `range` with `text`, `select` in it.
fn expression_edit(range: Range<usize>, text: String, select: Range<usize>) -> Surrounded {
    let select = range.start + select.start..range.start + select.end;
    let end = select.end;
    Surrounded { range, text, select, stops: Vec::new(), end }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, s: &Surrounded) -> (String, String) {
        let mut out = text.to_owned();
        out.replace_range(s.range.clone(), &s.text);
        let selected = out[s.select.clone()].to_owned();
        (out, selected)
    }

    #[test]
    fn surrounds_kotlin_lines() {
        let text = "fun f() {\n    val a = 1\n    println(a)\n}\n";
        let lines = lines_of(text, text.find("val").unwrap()..text.find("println").unwrap() + 3);
        assert_eq!(&text[lines.clone()], "    val a = 1\n    println(a)\n");
        let unit = indent_unit(text);
        let (out, _) = apply(text, &surround_lines(ImportStyle::Kotlin, Surrounder::If, text, lines.clone(), &unit));
        assert_eq!(out, "fun f() {\n    if () {\n        val a = 1\n        println(a)\n    }\n}\n");
        let (out, selected) = apply(text, &surround_lines(ImportStyle::Kotlin, Surrounder::TryCatch, text, lines.clone(), &unit));
        assert_eq!(out, "fun f() {\n    try {\n        val a = 1\n        println(a)\n    } catch (e: Exception) {\n        throw e\n    }\n}\n");
        assert_eq!(selected, "Exception");
        let (out, selected) = apply(text, &surround_lines(ImportStyle::Kotlin, Surrounder::DoWhile, text, lines, &unit));
        assert_eq!(out, "fun f() {\n    do {\n        val a = 1\n        println(a)\n    } while (true)\n}\n");
        assert_eq!(selected, "true");
        let s = surround_lines(ImportStyle::Kotlin, Surrounder::For, text, lines_of(text, 14..14), &unit);
        let (out, selected) = apply(text, &s);
        assert_eq!(selected, "item in items");
        assert_eq!(s.stops.iter().map(|r| &out[r.clone()]).collect::<Vec<_>>(), ["item", "items"]);
    }

    #[test]
    fn surrounds_java_lines() {
        let text = "  void f() {\n    run();\n  }\n";
        let caret = text.find("run").unwrap();
        let lines = lines_of(text, caret..caret);
        let unit = indent_unit(text);
        assert_eq!(unit, "  ");
        let s = surround_lines(ImportStyle::Java, Surrounder::For, text, lines.clone(), &unit);
        let (out, _) = apply(text, &s);
        assert_eq!(out, "  void f() {\n    for (; ; ) {\n      run();\n    }\n  }\n");
        // Tab: the initializer, the condition, the update, then the header's end.
        let header = out.find("for").unwrap();
        assert_eq!(s.stops.iter().map(|r| r.start - header).collect::<Vec<_>>(), [5, 7, 9]);
        assert_eq!(&out[s.end - 3..s.end], ") {");
        assert_eq!(&out[s.select.start..s.select.start + 1], ";");
        let (out, _) = apply(text, &surround_lines(ImportStyle::Java, Surrounder::Runnable, text, lines, &unit));
        assert_eq!(out, "  void f() {\n    Runnable runnable = new Runnable() {\n      public void run() {\n        run();\n      }\n    };\n  }\n");
    }

    #[test]
    fn surrounds_expressions() {
        let text = "val ok = a && b\n";
        let at = text.find("a && b").unwrap();
        let c = ExpressionContext::default();
        let (out, _) = apply(text, &surround_expression(ImportStyle::Kotlin, Surrounder::Not, text, at..at + 6, "    ", &c));
        assert_eq!(out, "val ok = !(a && b)\n");
        let (out, _) = apply(text, &surround_expression(ImportStyle::Kotlin, Surrounder::StringTemplate, text, at..at + 6, "    ", &c));
        assert_eq!(out, "val ok = \"${a && b}\"\n");
        assert_eq!((mnemonic(0), mnemonic(9), mnemonic(10)), ('1', '0', 'A'));
    }

    #[test]
    fn surrounds_expressions_as_intellij() {
        let at = |text: &str, what: &str| {
            let start = text.rfind(what).unwrap();
            start..start + what.len()
        };
        let run = |style, s, text: &str, what: &str, c: ExpressionContext| {
            let done = surround_expression(style, s, text, at(text, what), "    ", &c);
            let (mut out, _) = apply(text, &done);
            out.insert_str(done.select.end, "]");
            out.insert_str(done.select.start, "[");
            out
        };
        let k = ImportStyle::Kotlin;
        let none = ExpressionContext::default;
        // withIfExpression/complexBoolean.kt, withIfElseExpression/complexBoolean.kt.
        let text = "fun foo(x: Boolean, y: Boolean) {\n    x || y && x\n}\n";
        let statement = || ExpressionContext { ty: Some("Boolean".into()), statement: Some(at(text, "x || y && x")), ..Default::default() };
        assert_eq!(run(k, Surrounder::IfExpression, text, "x || y && x", statement()), "fun foo(x: Boolean, y: Boolean) {\n    if (x || y && x) {\n        []\n    }\n}\n");
        assert_eq!(run(k, Surrounder::IfElseExpression, text, "x || y && x", statement()), "fun foo(x: Boolean, y: Boolean) {\n    if (x || y && x) {\n        []\n    } else {\n    }\n}\n");
        // withIfElseExpression/replaceWithParenthesized.kt.
        let text = "fun test(some: Boolean) {\n    some.equals(true)\n}";
        let c = ExpressionContext { ty: Some("Boolean".into()), parenthesize: true, ..Default::default() };
        assert_eq!(run(k, Surrounder::IfElseExpression, text, "some", c), "fun test(some: Boolean) {\n    (if (some) {\n        []\n    } else {\n    }).equals(true)\n}");
        // when/multiExpression.kt.
        let text = "fun foo() {\n    val a = \"aa\"\n    a.capitalize()\n}";
        assert_eq!(run(k, Surrounder::When, text, "a.capitalize()", none()), "fun foo() {\n    val a = \"aa\"\n    when (a.capitalize()) {\n        [] -> {}\n        else -> {}\n    }\n}");
        // ifElseExpression/paranthesized.kt, ifElseExpressionBraces/singleExpression.kt.
        let text = "fun foo() {\n    \"aaa\" + 1\n}";
        assert_eq!(run(k, Surrounder::IfAround, text, "\"aaa\"", ExpressionContext { parenthesize: true, ..none() }), "fun foo() {\n    (if ([]) \"aaa\" else) + 1\n}");
        let text = "fun foo() {\n    val a = \"aaa\"\n}";
        assert_eq!(run(k, Surrounder::IfAroundBraces, text, "\"aaa\"", none()), "fun foo() {\n    val a = if ([]) {\n        \"aaa\"\n    } else {\n    }\n}");
        // tryCatchExpression/singleExpression.kt, tryFinallyExpression/singleExpression.kt.
        assert_eq!(run(k, Surrounder::TryCatchExpression, text, "\"aaa\"", none()), "fun foo() {\n    val a = try {\n        \"aaa\"\n    } catch (e: [Exception]) {\n        throw e\n    }\n}");
        assert_eq!(run(k, Surrounder::TryFinallyExpression, text, "\"aaa\"", none()), "fun foo() {\n    val a = try {\n        \"aaa\"\n    } finally {\n        []\n    }\n}");

        let j = ImportStyle::Java;
        // SurroundBinaryWithCast.
        let text = "    if (a + b + 1) {\n";
        assert_eq!(run(j, Surrounder::Cast, text, "a + b + 1", ExpressionContext { parenthesize: true, ..none() }), "    if ((([]) (a + b + 1))) {\n");
        assert_eq!(run(j, Surrounder::NotInstanceof, text, "a + b + 1", none()), "    if (!(a + b + 1 instanceof [])) {\n");
        // SurroundWithNotNullCheck.
        let text = "  void bar() {\n    foo().toLowerCase();\n  }\n";
        let c = ExpressionContext { enclosing: Some(at(text, "foo().toLowerCase();")), ..none() };
        assert_eq!(run(j, Surrounder::NullCheck, text, "foo()", c), "  void bar() {\n    if (foo() != null) {\n        foo().toLowerCase();[]\n    }\n  }\n");
        let text = "  void bar() {\n    ok();\n  }\n";
        let c = ExpressionContext { statement: Some(at(text, "ok();")), ..none() };
        assert_eq!(run(j, Surrounder::IfElseExpression, text, "ok()", c), "  void bar() {\n    if (ok()) {\n        []\n    } else {\n    }\n  }\n");
    }

    #[test]
    fn offers_the_expression_surrounders_that_apply() {
        use Surrounder::*;
        let boolean = ExpressionContext { ty: Some("Boolean".into()), statement: Some(0..1), ..Default::default() };
        assert_eq!(expression_surrounders(ImportStyle::Kotlin, Some(&boolean)), [Not, StringTemplate, Parentheses, When, IfExpression, IfElseExpression]);
        let string = ExpressionContext { ty: Some("String".into()), ..Default::default() };
        assert_eq!(
            expression_surrounders(ImportStyle::Kotlin, Some(&string)),
            [StringTemplate, Parentheses, When, TryCatchExpression, TryCatchFinallyExpression, TryFinallyExpression, IfAround, IfAroundBraces]
        );
        let int = ExpressionContext { ty: Some("int".into()), enclosing: Some(0..1), ..Default::default() };
        assert_eq!(expression_surrounders(ImportStyle::Java, Some(&int)), [Parentheses, Cast]);
        let unknown = ExpressionContext { enclosing: Some(0..1), ..Default::default() };
        assert_eq!(expression_surrounders(ImportStyle::Java, Some(&unknown)), [Parentheses, Cast, Not, NotInstanceof, NullCheck]);
        assert_eq!(expression_surrounders(ImportStyle::Java, None), [Parentheses, Not]);
    }
}

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
}

impl Surrounder {
    /// The popup's text for it.
    pub fn label(self) -> &'static str {
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

/// The surrounders for an expression within a line.
pub fn expression_surrounders(style: ImportStyle) -> &'static [Surrounder] {
    use Surrounder::*;
    match style {
        ImportStyle::Kotlin => &[Not, StringTemplate, Parentheses],
        ImportStyle::Java => &[Parentheses, Not],
    }
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
    Surrounded { range, text: out, select }
}

/// Wraps an expression (within a line) with an expression surrounder.
pub fn surround_expression(surrounder: Surrounder, text: &str, range: Range<usize>) -> Surrounded {
    let expr = &text[range.clone()];
    let out = match surrounder {
        Surrounder::Not => format!("!({expr})"),
        Surrounder::StringTemplate => format!("\"${{{expr}}}\""),
        _ => format!("({expr})"),
    };
    let end = range.start + out.len();
    Surrounded { range, text: out, select: end..end }
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
        assert_eq!(&out[s.select.start..s.select.start + 1], ";");
        let (out, _) = apply(text, &surround_lines(ImportStyle::Java, Surrounder::Runnable, text, lines, &unit));
        assert_eq!(out, "  void f() {\n    Runnable runnable = new Runnable() {\n      public void run() {\n        run();\n      }\n    };\n  }\n");
    }

    #[test]
    fn surrounds_expressions() {
        let text = "val ok = a && b\n";
        let at = text.find("a && b").unwrap();
        let (out, _) = apply(text, &surround_expression(Surrounder::Not, text, at..at + 6));
        assert_eq!(out, "val ok = !(a && b)\n");
        let (out, _) = apply(text, &surround_expression(Surrounder::StringTemplate, text, at..at + 6));
        assert_eq!(out, "val ok = \"${a && b}\"\n");
        assert_eq!((mnemonic(0), mnemonic(9), mnemonic(10)), ('1', '0', 'A'));
    }
}

//! Introduce Variable (Ctrl+Alt+V) and Introduce Constant (Ctrl+Alt+C),
//! as IntelliJ's: the names they suggest for an expression
//! (KotlinNameSuggester's: the called or selected name without `get` /
//! `is` / `set` and its camel-case tails, else one for the literal's type;
//! a constant's in NameUtil.capitalizeAndUnderscore's `UPPER_CASE`), and
//! the edits declaring it (a local before the statement, a constant at the
//! top level or first in its class) and putting its name where the
//! expression was.

use std::ops::Range;

use super::imports::ImportStyle;

/// What an expression's name can come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameSource {
    /// `user.getName()` → `getName`; `point.x` → `x`; `Foo()` → `Foo`.
    Name(String),
    Literal(Literal),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Literal {
    Int,
    Long,
    Float,
    Double,
    Boolean,
    Char,
    String,
}

impl Literal {
    /// IntelliJ's names for the type (a primitive's own name skipped).
    fn names(self) -> &'static [&'static str] {
        match self {
            Literal::Int => &["i", "n"],
            Literal::Long => &["lng", "l"],
            Literal::Float => &["f"],
            Literal::Double => &["d"],
            Literal::Boolean => &["bool", "b"],
            Literal::Char => &["ch", "c"],
            Literal::String => &["string", "str", "s", "text"],
        }
    }

    /// The Java type a declaration of it takes.
    pub fn java_type(self) -> &'static str {
        match self {
            Literal::Int => "int",
            Literal::Long => "long",
            Literal::Float => "float",
            Literal::Double => "double",
            Literal::Boolean => "boolean",
            Literal::Char => "char",
            Literal::String => "String",
        }
    }
}

const KOTLIN_HARD_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null", "object", "package", "return", "super", "this", "throw",
    "true", "try", "typealias", "typeof", "val", "var", "when", "while",
];

/// `getUserName` → `UserName`, `isEmpty` → `Empty`; else the name.
fn cut_accessor_prefix(name: &str) -> &str {
    for prefix in ["get", "is", "set"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.chars().next().is_some_and(char::is_uppercase) {
                return rest;
            }
        }
    }
    name
}

fn decapitalize(name: &str) -> String {
    // `URLConnection` → `urlConnection`, as IntelliJ's decapitalizeSmart.
    let chars: Vec<char> = name.chars().collect();
    let upper = chars.iter().take_while(|c| c.is_uppercase()).count();
    let lower = match upper {
        0 => 0,
        n if n == chars.len() => n,
        1 => 1,
        n => n - 1,
    };
    chars.iter().enumerate().map(|(i, c)| if i < lower { c.to_lowercase().next().unwrap_or(*c) } else { *c }).collect()
}

/// `UserName` → [`userName`, `name`].
fn camel_names(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut out = Vec::new();
    let mut upper_before = false;
    for (i, c) in chars.iter().enumerate() {
        let upper = c.is_uppercase();
        if i == 0 || upper && !upper_before {
            let tail: String = chars[i..].iter().collect();
            out.push(decapitalize(&tail));
        }
        upper_before = upper;
    }
    out
}

/// A name not `taken`: the name, else the name with 2, 3….
fn unique(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    (2..).map(|i| format!("{name}{i}")).find(|n| !taken(n)).unwrap_or_default()
}

/// The names suggested for an expression, best first; none of them
/// `taken` (names already used where the variable goes), none a keyword.
pub fn suggest_names(style: ImportStyle, source: &NameSource, taken: &dyn Fn(&str) -> bool) -> Vec<String> {
    let keyword = |n: &str| match style {
        ImportStyle::Kotlin => KOTLIN_HARD_KEYWORDS.contains(&n),
        ImportStyle::Java => super::rename::check_name("java", n).is_err(),
    };
    let base: Vec<String> = match source {
        NameSource::Name(name) => camel_names(cut_accessor_prefix(name)).into_iter().filter(|n| n.len() > 1 || n.chars().all(char::is_alphabetic)).collect(),
        NameSource::Literal(literal) => literal.names().iter().map(|s| s.to_string()).collect(),
        NameSource::Unknown => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for name in base.into_iter().chain(std::iter::once("value".to_owned())) {
        let name = if keyword(&name) {
            // `class` → `klass`, as IntelliJ's escaper.
            match name.as_str() {
                "class" => "klass".to_owned(),
                "fun" => "function".to_owned(),
                "this" => "self".to_owned(),
                "package" => "pkg".to_owned(),
                _ => continue,
            }
        } else {
            name
        };
        let name = unique(&name, taken);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Where an expression to extract is, found in the file's syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extraction {
    /// Every occurrence of the expression where the variable can be
    /// used, in order; each with its statement (a direct child of the
    /// block the variable goes into) and whether it is that statement.
    pub occurrences: Vec<Occurrence>,
    /// The one the caret or selection is on.
    pub selected: usize,
    pub source: NameSource,
    /// Java: the declaration's type, if known from the expression.
    pub java_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub range: Range<usize>,
    pub statement: Range<usize>,
    /// The expression is the whole statement (`foo()` on its own line):
    /// the declaration takes its place.
    pub whole: bool,
}

/// The edit: the byte range it replaces, its text, and where the new
/// name's declaration is in the new text (to select it).
#[derive(Debug, PartialEq, Eq)]
pub struct Introduced {
    pub range: Range<usize>,
    pub text: String,
    pub name_at: Range<usize>,
}

/// Declares `name` for the expression before the statement of the first
/// occurrence replaced (`all`, or the selected one only), with that
/// statement's indentation, and puts the name in each. `modifier`: a
/// Kotlin `var`, a Java `final`.
pub fn introduce(style: ImportStyle, text: &str, extraction: &Extraction, all: bool, name: &str, modifier: bool) -> Result<Introduced, String> {
    let chosen: Vec<&Occurrence> = if all { extraction.occurrences.iter().collect() } else { extraction.occurrences.get(extraction.selected).into_iter().collect() };
    let first = *chosen.first().ok_or("Cannot find an expression to introduce")?;
    let expression = &text[first.range.clone()];
    let declaration = match style {
        ImportStyle::Kotlin => format!("{} {name} = {expression}", if modifier { "var" } else { "val" }),
        ImportStyle::Java => {
            let ty = extraction.java_type.as_deref().ok_or("Cannot perform refactoring without a type")?;
            format!("{}{ty} {name} = {expression};", if modifier { "final " } else { "" })
        }
    };
    // (range, replacement, where the name is in the replacement if it
    // declares it), in order.
    let mut edits: Vec<(Range<usize>, String, Option<usize>)> = Vec::new();
    let name_in_decl = declaration.find(&format!(" {name} ")).map_or(0, |i| i + 1);
    if first.whole {
        edits.push((first.statement.clone(), declaration, Some(name_in_decl)));
    } else {
        let line_start = text[..first.statement.start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..first.statement.start].chars().take_while(|c| c.is_whitespace()).collect();
        edits.push((line_start..line_start, format!("{indent}{declaration}\n"), Some(indent.len() + name_in_decl)));
        edits.push((first.range.clone(), name.to_owned(), None));
    }
    for o in chosen.iter().skip(1) {
        edits.push((o.range.clone(), name.to_owned(), None));
    }
    Ok(apply_edits(text, edits, name))
}

/// The words of a name or text: `userName` → [user, Name], `URLConnection`
/// → [URL, Connection], `"Hello, world"` → [Hello, world].
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in text.split(|c: char| !c.is_alphanumeric()).filter(|c| !c.is_empty()) {
        let chars: Vec<char> = chunk.chars().collect();
        let mut word = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let starts = c.is_uppercase()
                && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit() || p.is_uppercase() && next.is_some_and(char::is_lowercase));
            if starts && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}

/// IntelliJ's NameUtil.capitalizeAndUnderscore: `userName` → `USER_NAME`.
pub fn constant_name(text: &str) -> Option<String> {
    let name = words(text).iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join("_");
    match name.chars().next() {
        None => None,
        Some(c) if c.is_ascii_digit() => Some(format!("_{name}")),
        Some(_) => Some(name),
    }
}

/// A constant's names: a string's own words first (`"Hello"` →
/// `HELLO`), then the variable names in upper case.
pub fn suggest_constant_names(style: ImportStyle, source: &NameSource, string_content: Option<&str>, taken: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Long texts make poor names: their first few words.
    let from_text = string_content.and_then(|t| constant_name(&words(t).into_iter().take(4).collect::<Vec<_>>().join(" ")));
    for name in from_text.into_iter().chain(suggest_names(style, source, &|_| false).iter().filter_map(|n| constant_name(n))) {
        let name = unique(&name, taken);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Where Introduce Constant puts the constant, and the expression's
/// occurrences in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstantPlan {
    pub occurrences: Vec<Range<usize>>,
    pub selected: usize,
    /// The start of the line the declaration goes before, and its
    /// indentation.
    pub insert_at: usize,
    pub indent: String,
    pub source: NameSource,
    /// Java: the field's type.
    pub java_type: Option<String>,
    /// A string literal's text, for its name.
    pub string_content: Option<String>,
}

/// Declares the constant (Kotlin `private const val` before the top-level
/// declaration, Java `public static final` first in the class, a blank
/// line after it) and puts its name at the occurrences replaced.
pub fn introduce_constant(style: ImportStyle, text: &str, plan: &ConstantPlan, all: bool, name: &str) -> Result<Introduced, String> {
    let chosen: Vec<&Range<usize>> = if all { plan.occurrences.iter().collect() } else { plan.occurrences.get(plan.selected).into_iter().collect() };
    let first = *chosen.first().ok_or("Cannot find an expression to introduce")?;
    let expression = &text[first.clone()];
    let declaration = match style {
        ImportStyle::Kotlin => format!("private const val {name} = {expression}"),
        ImportStyle::Java => {
            let ty = plan.java_type.as_deref().ok_or("Cannot perform refactoring without a type")?;
            format!("public static final {ty} {name} = {expression};")
        }
    };
    let name_in_decl = plan.indent.len() + declaration.find(&format!(" {name} ")).map_or(0, |i| i + 1);
    let mut edits: Vec<(Range<usize>, String, Option<usize>)> = vec![(plan.insert_at..plan.insert_at, format!("{}{declaration}\n\n", plan.indent), Some(name_in_decl))];
    edits.extend(chosen.iter().map(|r| ((*r).clone(), name.to_owned(), None)));
    Ok(apply_edits(text, edits, name))
}

/// One edit covering `edits` (range, replacement, where the declared
/// name is in the replacement); an insertion goes before a replacement
/// at the same place.
fn apply_edits(text: &str, mut edits: Vec<(Range<usize>, String, Option<usize>)>, name: &str) -> Introduced {
    edits.sort_by_key(|(r, _, decl)| (r.start, decl.is_none(), r.end));
    let start = edits.first().map_or(0, |(r, _, _)| r.start);
    let end = edits.iter().map(|(r, _, _)| r.end).max().unwrap_or(start);
    let mut out = String::new();
    let mut last = start;
    let mut name_at = 0..0;
    for (range, with, decl) in &edits {
        out.push_str(&text[last..range.start]);
        if let Some(at) = decl {
            let at = start + out.len() + at;
            name_at = at..at + name.len();
        }
        out.push_str(with);
        last = range.end;
    }
    out.push_str(&text[last..end]);
    Introduced { range: start..end, text: out, name_at }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Introduced) -> String {
        let mut out = text.to_owned();
        out.replace_range(i.range.clone(), &i.text);
        out
    }

    #[test]
    fn suggests_constant_names() {
        assert_eq!(constant_name("userName").as_deref(), Some("USER_NAME"));
        assert_eq!(constant_name("URLConnection").as_deref(), Some("URL_CONNECTION"));
        assert_eq!(constant_name("Hello, world!").as_deref(), Some("HELLO_WORLD"));
        assert_eq!(constant_name("2 items").as_deref(), Some("_2_ITEMS"));
        assert_eq!(constant_name("..."), None);
        let free = |_: &str| false;
        assert_eq!(suggest_constant_names(ImportStyle::Kotlin, &NameSource::Literal(Literal::String), Some("Hello world"), &free), ["HELLO_WORLD", "STRING", "STR", "S", "TEXT", "VALUE"]);
        assert_eq!(suggest_constant_names(ImportStyle::Kotlin, &NameSource::Literal(Literal::Int), None, &|n| n == "I"), ["I2", "N", "VALUE"]);
    }

    #[test]
    fn introduces_constants() {
        let text = "val a = 1\n\nfun foo(): Int {\n    val x = 2 + 3\n    return 2\n}\n";
        let at = |nth: usize| { let i = text.match_indices('2').nth(nth).unwrap().0; i..i + 1 };
        let plan = ConstantPlan {
            occurrences: vec![at(0), at(1)],
            selected: 1,
            insert_at: text.find("fun foo").unwrap(),
            indent: String::new(),
            source: NameSource::Literal(Literal::Int),
            java_type: None,
            string_content: None,
        };
        let done = introduce_constant(ImportStyle::Kotlin, text, &plan, true, "I").unwrap();
        let mut out = text.to_owned();
        out.replace_range(done.range.clone(), &done.text);
        // As IntelliJ's test data (introduceConstant/extractInteger.kt).
        assert_eq!(out, "val a = 1\n\nprivate const val I = 2\n\nfun foo(): Int {\n    val x = I + 3\n    return I\n}\n");
        assert_eq!(&out[done.name_at.clone()], "I");
        let java = "class Test {\n\n  int get() {\n    return 42;\n  }\n}\n";
        let i = java.find("42").unwrap();
        let plan = ConstantPlan {
            occurrences: vec![i..i + 2],
            selected: 0,
            insert_at: java.find("  int get").unwrap(),
            indent: "  ".into(),
            source: NameSource::Literal(Literal::Int),
            java_type: Some("int".into()),
            string_content: None,
        };
        let done = introduce_constant(ImportStyle::Java, java, &plan, false, "ANSWER").unwrap();
        let mut out = java.to_owned();
        out.replace_range(done.range.clone(), &done.text);
        assert_eq!(out, "class Test {\n\n  public static final int ANSWER = 42;\n\n  int get() {\n    return ANSWER;\n  }\n}\n");
    }

    #[test]
    fn suggests_names() {
        let free = |_: &str| false;
        let names = |s: &NameSource| suggest_names(ImportStyle::Kotlin, s, &free);
        assert_eq!(names(&NameSource::Name("getUserName".into())), ["userName", "name", "value"]);
        assert_eq!(names(&NameSource::Name("isEmpty".into())), ["empty", "value"]);
        // As IntelliJ's getCamelNames: no word starts right after a capital.
        assert_eq!(names(&NameSource::Name("URLConnection".into())), ["urlConnection", "value"]);
        assert_eq!(names(&NameSource::Literal(Literal::String)), ["string", "str", "s", "text", "value"]);
        assert_eq!(names(&NameSource::Literal(Literal::Int)), ["i", "n", "value"]);
        assert_eq!(names(&NameSource::Unknown), ["value"]);
        // Taken names get a number; keywords are escaped.
        let taken = |n: &str| n == "name" || n == "value";
        assert_eq!(suggest_names(ImportStyle::Kotlin, &NameSource::Name("name".into()), &taken), ["name2", "value2"]);
        assert_eq!(suggest_names(ImportStyle::Kotlin, &NameSource::Name("getClass".into()), &free), ["klass", "value"]);
    }

    fn occurrence(text: &str, expr: &str, nth: usize, statement: &str) -> Occurrence {
        let at = text.match_indices(expr).nth(nth).unwrap().0;
        let s = text.find(statement).unwrap();
        Occurrence { range: at..at + expr.len(), statement: s..s + statement.len(), whole: false }
    }

    #[test]
    fn introduces_a_kotlin_variable() {
        let text = "fun f(p: Point) {\n    println(p.x + 1)\n    show(p.x)\n}\n";
        let ex = Extraction {
            occurrences: vec![occurrence(text, "p.x", 0, "println(p.x + 1)"), occurrence(text, "p.x", 1, "show(p.x)")],
            selected: 1,
            source: NameSource::Name("x".into()),
            java_type: None,
        };
        let all = introduce(ImportStyle::Kotlin, text, &ex, true, "x", false).unwrap();
        let out = apply(text, &all);
        assert_eq!(out, "fun f(p: Point) {\n    val x = p.x\n    println(x + 1)\n    show(x)\n}\n");
        // `name_at` is in the new text: the declared name.
        assert_eq!(out.find("val x").unwrap() + 4, all.name_at.start);
        assert_eq!(&out[all.name_at.clone()], "x");
        // Only the selected one: declared before its own statement.
        let one = introduce(ImportStyle::Kotlin, text, &ex, false, "x", true).unwrap();
        assert_eq!(apply(text, &one), "fun f(p: Point) {\n    println(p.x + 1)\n    var x = p.x\n    show(x)\n}\n");
    }

    #[test]
    fn introduces_a_java_variable() {
        let text = "void f() {\n  foo(\"a\" + b);\n  bar();\n}\n";
        let at = text.find("\"a\" + b").unwrap();
        let s = text.find("foo(").unwrap();
        let ex = Extraction {
            occurrences: vec![Occurrence { range: at..at + 7, statement: s..s + 14, whole: false }],
            selected: 0,
            source: NameSource::Literal(Literal::String),
            java_type: Some("String".into()),
        };
        assert_eq!(apply(text, &introduce(ImportStyle::Java, text, &ex, false, "s", false).unwrap()), "void f() {\n  String s = \"a\" + b;\n  foo(s);\n  bar();\n}\n");
        assert!(apply(text, &introduce(ImportStyle::Java, text, &ex, false, "s", true).unwrap()).contains("  final String s = "));
        // The whole statement becomes the declaration.
        let at = text.find("bar()").unwrap();
        let ex = Extraction {
            occurrences: vec![Occurrence { range: at..at + 5, statement: at..at + 6, whole: true }],
            selected: 0,
            source: NameSource::Name("bar".into()),
            java_type: Some("int".into()),
        };
        let done = introduce(ImportStyle::Java, text, &ex, false, "bar", false).unwrap();
        let out = apply(text, &done);
        assert_eq!(out, "void f() {\n  foo(\"a\" + b);\n  int bar = bar();\n}\n");
        assert_eq!(&out[done.name_at.clone()], "bar");
        let ex = Extraction { java_type: None, ..ex };
        assert_eq!(introduce(ImportStyle::Java, text, &ex, false, "bar", false), Err("Cannot perform refactoring without a type".into()));
    }
}

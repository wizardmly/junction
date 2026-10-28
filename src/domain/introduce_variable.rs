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

    /// The Kotlin type a declaration of it takes.
    pub fn kotlin_type(self) -> &'static str {
        match self {
            Literal::Int => "Int",
            Literal::Long => "Long",
            Literal::Float => "Float",
            Literal::Double => "Double",
            Literal::Boolean => "Boolean",
            Literal::Char => "Char",
            Literal::String => "String",
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
    /// Kotlin: the type "Specify type explicitly" writes, if known.
    pub kotlin_type: Option<String>,
    /// Kotlin: the expression is in an expression body (`fun f() = …`),
    /// which becomes a block body to declare the variable in.
    pub body: Option<ExpressionBody>,
}

/// A Kotlin function's expression body, as IntelliJ converts it to a
/// block (ConvertToBlockBodyIntention): `= expression` becomes
/// `{ return expression }`, with the return type written if it wasn't.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionBody {
    /// From the `=` to the expression's end.
    pub range: Range<usize>,
    pub expression: Range<usize>,
    /// The function's line's indentation.
    pub indent: String,
    /// `: Type` to insert at that offset (after the parameters) when the
    /// function doesn't declare its return type.
    pub return_type: Option<(usize, String)>,
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
/// Kotlin `var`, a Java `final`. `explicit_type`: Kotlin's "Specify
/// type explicitly", `val name: Type = …`.
pub fn introduce(style: ImportStyle, text: &str, extraction: &Extraction, all: bool, name: &str, modifier: bool, explicit_type: bool) -> Result<Introduced, String> {
    let chosen: Vec<&Occurrence> = if all { extraction.occurrences.iter().collect() } else { extraction.occurrences.get(extraction.selected).into_iter().collect() };
    let first = *chosen.first().ok_or("Cannot find an expression to introduce")?;
    let expression = &text[first.range.clone()];
    // The declaration, and where the name is in it.
    let (declaration, name_in_decl) = match style {
        ImportStyle::Kotlin => {
            let ty = extraction.kotlin_type.as_deref().filter(|_| explicit_type).map(|t| format!(": {t}")).unwrap_or_default();
            (format!("{} {name}{ty} = {expression}", if modifier { "var" } else { "val" }), 4)
        }
        ImportStyle::Java => {
            let ty = extraction.java_type.as_deref().ok_or("Cannot perform refactoring without a type")?;
            let prefix = format!("{}{ty} ", if modifier { "final " } else { "" });
            (format!("{prefix}{name} = {expression};"), prefix.len())
        }
    };
    // (range, replacement, where the name is in the replacement if it
    // declares it), in order.
    let mut edits: Vec<(Range<usize>, String, Option<usize>)> = Vec::new();
    if let Some(body) = &extraction.body {
        return Ok(into_block(text, body, &chosen, &declaration, name_in_decl, name));
    }
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

/// Declares the variable in the block an expression body becomes:
/// `= a + b` → `{\n    val x = a\n    return x + b\n}`.
fn into_block(text: &str, body: &ExpressionBody, chosen: &[&Occurrence], declaration: &str, name_in_decl: usize, name: &str) -> Introduced {
    let unit = super::surround::indent_unit(text);
    let inner = format!("{}{unit}", body.indent);
    let mut expression = String::new();
    let mut last = body.expression.start;
    for o in chosen {
        expression.push_str(&text[last..o.range.start]);
        expression.push_str(name);
        last = o.range.end;
    }
    expression.push_str(&text[last..body.expression.end]);
    let block = format!("{{\n{inner}{declaration}\n{inner}return {expression}\n{}}}", body.indent);
    let mut edits = vec![(body.range.clone(), block, Some(2 + inner.len() + name_in_decl))];
    if let Some((at, ty)) = &body.return_type {
        edits.push((*at..*at, format!(": {ty}"), None));
    }
    apply_edits(text, edits, name)
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
    /// Java: the constant goes into an interface (or annotation type),
    /// where fields are implicitly `public static final`.
    pub interface: bool,
    /// Java: the class it goes into, and that class's fields the
    /// expression uses (qualified when the constant moves elsewhere).
    pub class_name: String,
    pub class_fields: Vec<String>,
}

/// Where a constant goes in a Java class: the start of the line it goes
/// before and its indentation; `blank_after` false in an empty body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstantSlot {
    pub insert_at: usize,
    pub indent: String,
    pub interface: bool,
    pub blank_after: bool,
    /// After the declaration's line: the closing brace's indentation when
    /// the body was `{}` on one line.
    pub after: String,
}

/// A Java constant's visibility, as IntelliJ's JavaVisibilityPanel offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    PackageLocal,
    Private,
}

impl Visibility {
    pub const ALL: [Visibility; 4] = [Visibility::Public, Visibility::Protected, Visibility::PackageLocal, Visibility::Private];

    pub fn label(self) -> &'static str {
        match self {
            Visibility::Public => "Public",
            Visibility::Protected => "Protected",
            Visibility::PackageLocal => "Package private",
            Visibility::Private => "Private",
        }
    }

    /// The modifier and its space (none for package-private).
    fn modifier(self) -> &'static str {
        match self {
            Visibility::Public => "public ",
            Visibility::Protected => "protected ",
            Visibility::PackageLocal => "",
            Visibility::Private => "private ",
        }
    }
}

/// Declares the constant (Kotlin `private const val` before the top-level
/// declaration, Java `<visibility> static final` in the class, a blank
/// line after it) and puts its name at the occurrences replaced.
pub fn introduce_constant(style: ImportStyle, text: &str, plan: &ConstantPlan, all: bool, name: &str, visibility: Visibility) -> Result<Introduced, String> {
    let chosen: Vec<&Range<usize>> = if all { plan.occurrences.iter().collect() } else { plan.occurrences.get(plan.selected).into_iter().collect() };
    let first = *chosen.first().ok_or("Cannot find an expression to introduce")?;
    let expression = &text[first.clone()];
    let declaration = match style {
        ImportStyle::Kotlin => format!("private const val {name} = {expression}"),
        ImportStyle::Java => {
            let ty = plan.java_type.as_deref().ok_or("Cannot perform refactoring without a type")?;
            if plan.interface {
                format!("{ty} {name} = {expression};")
            } else {
                format!("{}static final {ty} {name} = {expression};", visibility.modifier())
            }
        }
    };
    let name_in_decl = plan.indent.len() + declaration.find(&format!(" {name} = ")).map_or(0, |i| i + 1);
    // A blank line after it, unless one is already there.
    let blank = if text[plan.insert_at..].starts_with('\n') { "" } else { "\n" };
    let mut edits: Vec<(Range<usize>, String, Option<usize>)> = vec![(plan.insert_at..plan.insert_at, format!("{}{declaration}\n{blank}", plan.indent), Some(name_in_decl))];
    edits.extend(chosen.iter().map(|r| ((*r).clone(), name.to_owned(), None)));
    Ok(apply_edits(text, edits, name))
}

/// Java's "Move to another class": the declaration for `class` (the
/// expression's uses of its own class's fields qualified, `Calc.MAX`)
/// and the file's occurrences replaced by `Class.NAME`.
pub fn moved_constant(text: &str, plan: &ConstantPlan, all: bool, name: &str, visibility: Visibility, class: &str, interface: bool) -> Result<(String, Introduced), String> {
    let chosen: Vec<&Range<usize>> = if all { plan.occurrences.iter().collect() } else { plan.occurrences.get(plan.selected).into_iter().collect() };
    let first = *chosen.first().ok_or("Cannot find an expression to introduce")?;
    let ty = plan.java_type.as_deref().ok_or("Cannot perform refactoring without a type")?;
    let mut expression = String::new();
    let source = &text[first.clone()];
    let mut rest = source;
    // Whole words only, not after a `.` (already qualified).
    while let Some(i) = rest.find(|c: char| c.is_alphanumeric() || c == '_') {
        let len = rest[i..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len() - i);
        let word = &rest[i..i + len];
        expression.push_str(&rest[..i]);
        let qualified = plan.class_fields.iter().any(|f| f == word) && !expression.trim_end().ends_with('.') && class != plan.class_name;
        if qualified {
            expression.push_str(&format!("{}.", plan.class_name));
        }
        expression.push_str(word);
        rest = &rest[i + len..];
    }
    expression.push_str(rest);
    let declaration = if interface { format!("{ty} {name} = {expression};") } else { format!("{}static final {ty} {name} = {expression};", visibility.modifier()) };
    let reference = format!("{class}.{name}");
    let edits = chosen.iter().map(|r| ((*r).clone(), reference.clone(), None)).collect();
    let mut done = apply_edits(text, edits, name);
    // The caret after the first reference.
    let end = done.range.start + done.text.find(&reference).map_or(0, |i| i + reference.len());
    done.name_at = end - name.len()..end;
    Ok((declaration, done))
}

/// The declaration put at `slot` in a class's text, a blank line after
/// it (unless the body was empty or one is there).
pub fn insert_constant(text: &str, slot: &ConstantSlot, declaration: &str) -> String {
    let blank = if !slot.blank_after || text[slot.insert_at..].starts_with('\n') { "" } else { "\n" };
    let mut out = text.to_owned();
    out.insert_str(slot.insert_at, &format!("{}{declaration}\n{blank}{}", slot.indent, slot.after));
    out
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
            interface: false,
            class_name: String::new(),
            class_fields: Vec::new(),
        };
        let done = introduce_constant(ImportStyle::Kotlin, text, &plan, true, "I", Visibility::Public).unwrap();
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
            interface: false,
            class_name: String::new(),
            class_fields: Vec::new(),
        };
        let done = introduce_constant(ImportStyle::Java, java, &plan, false, "ANSWER", Visibility::Public).unwrap();
        let mut out = java.to_owned();
        out.replace_range(done.range.clone(), &done.text);
        assert_eq!(out, "class Test {\n\n  public static final int ANSWER = 42;\n\n  int get() {\n    return ANSWER;\n  }\n}\n");
        let java_out = |plan: &ConstantPlan, v| {
            let done = introduce_constant(ImportStyle::Java, java, plan, false, "ANSWER", v).unwrap();
            let mut out = java.to_owned();
            out.replace_range(done.range.clone(), &done.text);
            assert_eq!(&out[done.name_at.clone()], "ANSWER");
            out
        };
        assert!(java_out(&plan, Visibility::PackageLocal).contains("\n  static final int ANSWER = 42;"));
        assert!(java_out(&plan, Visibility::Private).contains("\n  private static final int ANSWER = 42;"));
        // An interface's fields are implicitly public static final.
        let plan = ConstantPlan { interface: true, ..plan };
        assert!(java_out(&plan, Visibility::Private).contains("\n  int ANSWER = 42;"));
    }

    #[test]
    fn moves_constants() {
        let java = "class Calc {\n  static int MAX = 9;\n  int f() {\n    return MAX * 2 + Calc.MAX;\n  }\n}\n";
        let i = java.find("MAX * 2").unwrap();
        let plan = ConstantPlan {
            occurrences: vec![i..i + "MAX * 2 + Calc.MAX".len()],
            selected: 0,
            insert_at: 0,
            indent: "  ".into(),
            source: NameSource::Literal(Literal::Int),
            java_type: Some("int".into()),
            string_content: None,
            interface: false,
            class_name: "Calc".into(),
            class_fields: vec!["MAX".into()],
        };
        let (decl, done) = moved_constant(java, &plan, true, "LIMIT", Visibility::Public, "Limits", false).unwrap();
        assert_eq!(decl, "public static final int LIMIT = Calc.MAX * 2 + Calc.MAX;");
        let mut out = java.to_owned();
        out.replace_range(done.range.clone(), &done.text);
        assert!(out.contains("return Limits.LIMIT;"));
        assert_eq!(&out[done.name_at.clone()], "LIMIT");
        assert_eq!(moved_constant(java, &plan, true, "LIMIT", Visibility::Private, "Calc", true).unwrap().0, "int LIMIT = MAX * 2 + Calc.MAX;");

        let target = "class Limits {\n  void g() {}\n}\n";
        let slot = ConstantSlot { insert_at: target.find("  void").unwrap(), indent: "  ".into(), interface: false, blank_after: true, after: String::new() };
        assert_eq!(insert_constant(target, &slot, "static final int A = 1;"), "class Limits {\n  static final int A = 1;\n\n  void g() {}\n}\n");
        let empty = "class Limits {\n}\n";
        let slot = ConstantSlot { insert_at: empty.find('}').unwrap(), indent: "  ".into(), interface: false, blank_after: false, after: String::new() };
        assert_eq!(insert_constant(empty, &slot, "static final int A = 1;"), "class Limits {\n  static final int A = 1;\n}\n");
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
            kotlin_type: Some("Int".into()),
            body: None,
        };
        let all = introduce(ImportStyle::Kotlin, text, &ex, true, "x", false, false).unwrap();
        let out = apply(text, &all);
        assert_eq!(out, "fun f(p: Point) {\n    val x = p.x\n    println(x + 1)\n    show(x)\n}\n");
        // `name_at` is in the new text: the declared name.
        assert_eq!(out.find("val x").unwrap() + 4, all.name_at.start);
        assert_eq!(&out[all.name_at.clone()], "x");
        // Only the selected one: declared before its own statement.
        let one = introduce(ImportStyle::Kotlin, text, &ex, false, "x", true, false).unwrap();
        assert_eq!(apply(text, &one), "fun f(p: Point) {\n    println(p.x + 1)\n    var x = p.x\n    show(x)\n}\n");
        let typed = introduce(ImportStyle::Kotlin, text, &ex, true, "x", false, true).unwrap();
        let out = apply(text, &typed);
        assert!(out.contains("    val x: Int = p.x\n"));
        assert_eq!(&out[typed.name_at.clone()], "x");
    }

    #[test]
    fn turns_an_expression_body_into_a_block() {
        let text = "class A {\n    fun f(a: Int) = a * 2 + a * 2\n}\n";
        let eq = text.find("= a").unwrap();
        let end = text.find("\n}").unwrap();
        let start = eq + 2;
        let ex = Extraction {
            occurrences: vec![occurrence(text, "a * 2", 0, "a * 2 + a * 2"), occurrence(text, "a * 2", 1, "a * 2 + a * 2")],
            selected: 0,
            source: NameSource::Unknown,
            java_type: None,
            kotlin_type: None,
            body: Some(ExpressionBody { range: eq..end, expression: start..end, indent: "    ".into(), return_type: Some((text.find(") =").unwrap() + 1, "Int".into())) }),
        };
        let done = introduce(ImportStyle::Kotlin, text, &ex, true, "value", false, false).unwrap();
        let out = apply(text, &done);
        assert_eq!(out, "class A {\n    fun f(a: Int): Int {\n        val value = a * 2\n        return value + value\n    }\n}\n");
        assert_eq!(&out[done.name_at.clone()], "value");
        assert_eq!(out.find("val value").unwrap() + 4, done.name_at.start);
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
            kotlin_type: None,
            body: None,
        };
        assert_eq!(apply(text, &introduce(ImportStyle::Java, text, &ex, false, "s", false, false).unwrap()), "void f() {\n  String s = \"a\" + b;\n  foo(s);\n  bar();\n}\n");
        assert!(apply(text, &introduce(ImportStyle::Java, text, &ex, false, "s", true, false).unwrap()).contains("  final String s = "));
        // The whole statement becomes the declaration.
        let at = text.find("bar()").unwrap();
        let ex = Extraction {
            occurrences: vec![Occurrence { range: at..at + 5, statement: at..at + 6, whole: true }],
            selected: 0,
            source: NameSource::Name("bar".into()),
            java_type: Some("int".into()),
            kotlin_type: None,
            body: None,
        };
        let done = introduce(ImportStyle::Java, text, &ex, false, "bar", false, false).unwrap();
        let out = apply(text, &done);
        assert_eq!(out, "void f() {\n  foo(\"a\" + b);\n  int bar = bar();\n}\n");
        assert_eq!(&out[done.name_at.clone()], "bar");
        let ex = Extraction { java_type: None, ..ex };
        assert_eq!(introduce(ImportStyle::Java, text, &ex, false, "bar", false, false), Err("Cannot perform refactoring without a type".into()));
    }
}

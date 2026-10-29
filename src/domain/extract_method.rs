//! Extract Method / Extract Function (Ctrl+Alt+M) as IntelliJ writes it:
//! the selected statements or expression become a private method after
//! the one they're in, the variables they read from outside its
//! parameters, the one they produce for the code after them its result;
//! the selection becomes the call. The analysis (`index::extract_method`)
//! finds the parts; this writes them.

use std::ops::Range;

use crate::domain::imports::ImportStyle;

/// A variable the extracted code reads from outside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    /// As written where it's declared, or inferred; empty when unknown.
    pub ty: String,
    /// Where the extracted code uses it, relative to the code.
    pub uses: Vec<Range<usize>>,
    /// The code assigns it (Kotlin's parameters are `val`: copied to a `var`).
    pub assigned: bool,
}

/// The value the code after the selection needs from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Declared in the selection (`val total = …`): declared at the call.
    Declared { name: String, ty: String, mutable: bool },
    /// Declared before and assigned in the selection: assigned at the call.
    Assigned { name: String, ty: String },
}

impl Output {
    pub fn name(&self) -> &str {
        match self {
            Output::Declared { name, .. } | Output::Assigned { name, .. } => name,
        }
    }

    pub fn ty(&self) -> &str {
        match self {
            Output::Declared { ty, .. } | Output::Assigned { ty, .. } => ty,
        }
    }
}

/// What the analysis found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodPlan {
    /// The selected statements (whole) or expression.
    pub range: Range<usize>,
    pub expression: bool,
    pub parameters: Vec<Parameter>,
    pub output: Option<Output>,
    /// The selection ends the method with `return …`: the call is returned,
    /// and the new method returns the old one's type.
    pub returns: Option<String>,
    /// An expression's type, when it shows it; empty when unknown.
    pub expression_type: String,
    /// After the enclosing method's last character.
    pub insert_at: usize,
    /// The enclosing method's line indent.
    pub member_indent: String,
    /// The enclosing method is static (Java).
    pub is_static: bool,
    /// Function names in the file, for a name that isn't taken.
    pub taken: Vec<String>,
}

impl MethodPlan {
    /// The result type the new method needs: the returned type, the
    /// output's, the expression's; empty when unknown, none for no result.
    pub fn result_type(&self) -> Option<&str> {
        if let Some(r) = &self.returns {
            return Some(r);
        }
        if let Some(o) = &self.output {
            return Some(o.ty());
        }
        self.expression.then_some(self.expression_type.as_str())
    }

    /// Whether the dialog must ask for the result type: Java always needs
    /// one, Kotlin only for a block body.
    pub fn needs_result_type(&self, style: ImportStyle) -> bool {
        match self.result_type() {
            Some("") => style == ImportStyle::Java || !self.expression,
            _ => false,
        }
    }

    /// IntelliJ's default name: "extracted", numbered when taken.
    pub fn suggested_name(&self) -> String {
        let taken = |n: &str| self.taken.iter().any(|t| t == n);
        if !taken("extracted") {
            return "extracted".into();
        }
        (1..).map(|i| format!("extracted{i}")).find(|n| !taken(n)).unwrap_or_default()
    }
}

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractRequest {
    pub name: String,
    /// "private", "public"… ("" for Java's package-private).
    pub visibility: String,
    pub is_static: bool,
    /// The parameters' names and types, in the plan's order (renamable).
    pub parameters: Vec<(String, String)>,
    /// The result type when the plan's was unknown.
    pub result_type: String,
    /// The parameters' order in the signature, as indexes into the plan's
    /// (the dialog's Move Up / Move Down); empty for the plan's order.
    pub order: Vec<usize>,
}

impl ExtractRequest {
    /// The plan's parameter indexes in signature order.
    pub fn ordered(&self) -> Vec<usize> {
        let n = self.parameters.len();
        let mut sorted = self.order.clone();
        sorted.sort_unstable();
        if sorted == (0..n).collect::<Vec<_>>() { self.order.clone() } else { (0..n).collect() }
    }
}

/// Another place in the file with the same code (IntelliJ's duplicates):
/// its parameters bound to other expressions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub range: Range<usize>,
    /// The arguments, in the plan's parameter order.
    pub args: Vec<String>,
    /// The variable an `Output::Assigned` result goes to here.
    pub assigned: Option<String>,
}

/// The edit: `range` of the old text becomes `text`; `select` is the new
/// method's name in the new text, absolute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
}

/// The new method's signature, as the dialog previews it.
pub fn signature(style: ImportStyle, plan: &MethodPlan, request: &ExtractRequest) -> String {
    let result = match plan.result_type() {
        Some("") => request.result_type.trim().to_owned(),
        Some(t) => t.to_owned(),
        None => String::new(),
    };
    let visibility = if request.visibility.is_empty() { String::new() } else { format!("{} ", request.visibility) };
    match style {
        ImportStyle::Java => {
            let params: Vec<String> = request.ordered().into_iter().map(|i| format!("{} {}", request.parameters[i].1, request.parameters[i].0)).collect();
            let result = if result.is_empty() { "void".to_owned() } else { result };
            format!("{visibility}{}{result} {}({})", if request.is_static { "static " } else { "" }, request.name, params.join(", "))
        }
        ImportStyle::Kotlin => {
            let params: Vec<String> = request.ordered().into_iter().map(|i| format!("{}: {}", request.parameters[i].0, request.parameters[i].1)).collect();
            let result = if result.is_empty() || result == "Unit" { String::new() } else { format!(": {result}") };
            format!("{visibility}fun {}({}){result}", request.name, params.join(", "))
        }
    }
}

/// Whether a name is an identifier.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// The request's problem, as the dialog shows it.
pub fn check(style: ImportStyle, plan: &MethodPlan, request: &ExtractRequest) -> Result<(), String> {
    if !is_identifier(&request.name) {
        return Err(format!("'{}' is not a valid name", request.name));
    }
    if plan.taken.iter().any(|t| t == &request.name) && request.parameters.is_empty() {
        return Err(format!("Method '{}' already exists", request.name));
    }
    for (i, (name, ty)) in request.parameters.iter().enumerate() {
        if !is_identifier(name) {
            return Err(format!("'{name}' is not a valid parameter name"));
        }
        if request.parameters[..i].iter().any(|(n, _)| n == name) {
            return Err(format!("Duplicate parameter name '{name}'"));
        }
        if ty.trim().is_empty() {
            return Err(format!("Specify the type of '{name}'"));
        }
    }
    if plan.needs_result_type(style) && request.result_type.trim().is_empty() {
        return Err("Specify the return type".into());
    }
    Ok(())
}

/// The lines of `code` with the first line's indent `from` taken off each
/// (as far as they have it) and `to` put on.
fn reindent(code: &str, from: &str, to: &str) -> String {
    code.lines()
        .enumerate()
        .map(|(i, line)| {
            if line.trim().is_empty() {
                return String::new();
            }
            let line = if i == 0 { line } else { line.strip_prefix(from).unwrap_or_else(|| line.trim_start()) };
            format!("{to}{line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The extraction: the call in place of the selection (and of the
/// duplicates), the new method after the enclosing one.
pub fn extract(style: ImportStyle, text: &str, plan: &MethodPlan, request: &ExtractRequest, unit: &str, duplicates: &[Duplicate]) -> Result<Extracted, String> {
    check(style, plan, request)?;
    let java = style == ImportStyle::Java;
    // The code, its parameters renamed.
    let mut code = text[plan.range.clone()].to_owned();
    let mut renames: Vec<(Range<usize>, &str)> = Vec::new();
    for (p, (name, _)) in plan.parameters.iter().zip(&request.parameters) {
        if &p.name != name {
            renames.extend(p.uses.iter().map(|u| (u.clone(), name.as_str())));
        }
    }
    renames.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (r, name) in renames {
        code.replace_range(r, name);
    }
    let line_start = text[..plan.range.start].rfind('\n').map_or(0, |i| i + 1);
    let statement_indent: String = text[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let inner = format!("{}{unit}", plan.member_indent);
    let order = request.ordered();
    let args: Vec<&str> = order.iter().map(|i| plan.parameters[*i].name.as_str()).collect();
    let call_text = call(java, plan, request, &args, None);
    let semi = if java { ";" } else { "" };
    // The new method.
    let sig = signature(style, plan, request);
    let mut method = format!("\n\n{}{sig}", plan.member_indent);
    if plan.expression && !java {
        let _ = std::fmt::Write::write_fmt(&mut method, format_args!(" = {}", reindent(&code, &statement_indent, "").trim_start()));
    } else {
        method.push_str(" {\n");
        // Kotlin's parameters can't be assigned: a `var` copy first.
        if !java {
            for (p, (name, _)) in plan.parameters.iter().zip(&request.parameters) {
                if p.assigned {
                    method.push_str(&format!("{inner}var {name} = {name}\n"));
                }
            }
        }
        if plan.expression {
            method.push_str(&format!("{inner}return {}{semi}\n", reindent(&code, &statement_indent, "").trim_start()));
        } else {
            method.push_str(&reindent(&code, &statement_indent, &inner));
            method.push('\n');
            if let (Some(output), None) = (&plan.output, &plan.returns) {
                method.push_str(&format!("{inner}return {}{semi}\n", output.name()));
            }
        }
        method.push_str(&format!("{}}}", plan.member_indent));
    }
    // One edit from the first change to the last: the selection, the
    // duplicates and the new method's place.
    let name_in_method = method.find(&format!("{}(", request.name)).unwrap_or(0);
    let mut edits: Vec<(Range<usize>, String, bool)> = vec![(plan.range.clone(), call_text, false), (plan.insert_at..plan.insert_at, method, true)];
    for d in duplicates {
        let args: Vec<&str> = order.iter().filter_map(|i| d.args.get(*i)).map(String::as_str).collect();
        edits.push((d.range.clone(), call(java, plan, request, &args, d.assigned.as_deref()), false));
    }
    edits.sort_by_key(|(r, _, _)| (r.start, r.end));
    let (start, end) = (edits[0].0.start, edits.iter().map(|(r, _, _)| r.end).max().unwrap_or(plan.insert_at));
    let mut replacement = String::new();
    let mut at = start;
    let mut name_offset = 0;
    for (r, t, is_method) in edits {
        if r.start < at {
            return Err("The duplicates overlap".into());
        }
        replacement.push_str(&text[at..r.start]);
        if is_method {
            name_offset = replacement.len() + name_in_method;
        }
        replacement.push_str(&t);
        at = r.end;
    }
    replacement.push_str(&text[at..end]);
    Ok(Extracted { range: start..end, text: replacement, select: start + name_offset..start + name_offset + request.name.len() })
}

/// The call that takes the code's place: an expression, a returned call,
/// the result declared or assigned, or a statement.
fn call(java: bool, plan: &MethodPlan, request: &ExtractRequest, args: &[&str], assigned: Option<&str>) -> String {
    let call = format!("{}({})", request.name, args.join(", "));
    let semi = if java { ";" } else { "" };
    if plan.expression {
        call
    } else if plan.returns.is_some() {
        format!("return {call}{semi}")
    } else {
        match &plan.output {
            None => format!("{call}{semi}"),
            Some(Output::Declared { name, ty, mutable }) => {
                let ty = if ty.is_empty() { request.result_type.trim() } else { ty.as_str() };
                if java { format!("{ty} {name} = {call};") } else { format!("{} {name} = {call}", if *mutable { "var" } else { "val" }) }
            }
            Some(Output::Assigned { name, .. }) => format!("{} = {call}{semi}", assigned.unwrap_or(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, e: &Extracted) -> String {
        format!("{}{}{}", &text[..e.range.start], e.text, &text[e.range.end..])
    }

    fn request(name: &str, params: &[(&str, &str)], result: &str) -> ExtractRequest {
        ExtractRequest {
            name: name.into(),
            visibility: "private".into(),
            is_static: false,
            parameters: params.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
            result_type: result.into(),
            order: Vec::new(),
        }
    }

    #[test]
    fn extracts_kotlin_statements_with_an_output() {
        let text = "class A {\n    fun f(n: Int) {\n        val a = n + 1\n        val b = a * 2\n        println(b)\n    }\n}\n";
        let start = text.find("val a").unwrap();
        let end = text.find("a * 2").unwrap() + 5;
        let plan = MethodPlan {
            range: start..end,
            expression: false,
            parameters: vec![Parameter { name: "n".into(), ty: "Int".into(), uses: vec![8..9], assigned: false }],
            output: Some(Output::Declared { name: "b".into(), ty: "Int".into(), mutable: false }),
            returns: None,
            expression_type: String::new(),
            insert_at: text.find("    }\n}").unwrap() + 5,
            member_indent: "    ".into(),
            is_static: false,
            taken: vec!["f".into()],
        };
        assert_eq!(plan.suggested_name(), "extracted");
        let e = extract(ImportStyle::Kotlin, text, &plan, &request("twice", &[("count", "Int")], ""), "    ", &[]).unwrap();
        let out = apply(text, &e);
        assert_eq!(
            out,
            "class A {\n    fun f(n: Int) {\n        val b = twice(n)\n        println(b)\n    }\n\n    private fun twice(count: Int): Int {\n        val a = count + 1\n        val b = a * 2\n        return b\n    }\n}\n"
        );
        assert_eq!(&out[e.select.clone()], "twice");
    }

    #[test]
    fn extracts_expressions_returns_and_assignments() {
        // A Kotlin expression: an expression body.
        let text = "fun f(x: Int) = println(x * 3 + 1)\n";
        let start = text.find("x * 3").unwrap();
        let plan = MethodPlan {
            range: start..start + 9,
            expression: true,
            parameters: vec![Parameter { name: "x".into(), ty: "Int".into(), uses: vec![0..1], assigned: false }],
            output: None,
            returns: None,
            expression_type: String::new(),
            insert_at: text.len() - 1,
            member_indent: String::new(),
            is_static: false,
            taken: vec!["f".into(), "extracted".into()],
        };
        assert_eq!(plan.suggested_name(), "extracted1");
        assert!(!plan.needs_result_type(ImportStyle::Kotlin) && plan.needs_result_type(ImportStyle::Java));
        let e = extract(ImportStyle::Kotlin, text, &plan, &request("calc", &[("x", "Int")], ""), "    ", &[]).unwrap();
        assert_eq!(apply(text, &e), "fun f(x: Int) = println(calc(x))\n\nprivate fun calc(x: Int) = x * 3 + 1\n");
        // Java: the last statements return; static kept.
        let text = "class A {\n  static int f(int n) {\n    int a = n + 1;\n    return a * 2;\n  }\n}\n";
        let start = text.find("int a").unwrap();
        let plan = MethodPlan {
            range: start..text.find("a * 2;").unwrap() + 6,
            expression: false,
            parameters: vec![Parameter { name: "n".into(), ty: "int".into(), uses: vec![8..9], assigned: false }],
            output: None,
            returns: Some("int".into()),
            expression_type: String::new(),
            insert_at: text.find("  }\n}").unwrap() + 3,
            member_indent: "  ".into(),
            is_static: true,
            taken: vec![],
        };
        let mut r = request("compute", &[("n", "int")], "");
        r.is_static = true;
        assert_eq!(signature(ImportStyle::Java, &plan, &r), "private static int compute(int n)");
        let e = extract(ImportStyle::Java, text, &plan, &r, "  ", &[]).unwrap();
        assert_eq!(apply(text, &e), "class A {\n  static int f(int n) {\n    return compute(n);\n  }\n\n  private static int compute(int n) {\n    int a = n + 1;\n    return a * 2;\n  }\n}\n");
        // Kotlin: an outer `var` assigned and used after; its parameter copied.
        let text = "fun f() {\n    var s = 0\n    s += 2\n    println(s)\n}\n";
        let start = text.find("s += 2").unwrap();
        let plan = MethodPlan {
            range: start..start + 6,
            expression: false,
            parameters: vec![Parameter { name: "s".into(), ty: "Int".into(), uses: vec![0..1], assigned: true }],
            output: Some(Output::Assigned { name: "s".into(), ty: "Int".into() }),
            returns: None,
            expression_type: String::new(),
            insert_at: text.len() - 1,
            member_indent: String::new(),
            is_static: false,
            taken: vec![],
        };
        let e = extract(ImportStyle::Kotlin, text, &plan, &request("add", &[("s", "Int")], ""), "    ", &[]).unwrap();
        assert_eq!(apply(text, &e), "fun f() {\n    var s = 0\n    s = add(s)\n    println(s)\n}\n\nprivate fun add(s: Int): Int {\n    var s = s\n    s += 2\n    return s\n}\n");
        // Checks.
        assert_eq!(check(ImportStyle::Kotlin, &plan, &request("add", &[("s", "")], "")), Err("Specify the type of 's'".into()));
        assert_eq!(check(ImportStyle::Kotlin, &plan, &request("1x", &[("s", "Int")], "")), Err("'1x' is not a valid name".into()));
    }

    #[test]
    fn parameters_in_the_chosen_order() {
        let text = "fun f(a: Int, b: Int) = println(a - b)\n";
        let start = text.find("a - b").unwrap();
        let plan = MethodPlan {
            range: start..start + 5,
            expression: true,
            parameters: vec![
                Parameter { name: "a".into(), ty: "Int".into(), uses: vec![0..1], assigned: false },
                Parameter { name: "b".into(), ty: "Int".into(), uses: vec![4..5], assigned: false },
            ],
            output: None,
            returns: None,
            expression_type: "Int".into(),
            insert_at: text.len() - 1,
            member_indent: String::new(),
            is_static: false,
            taken: vec![],
        };
        let mut r = request("sub", &[("x", "Int"), ("y", "Long")], "");
        r.order = vec![1, 0];
        assert_eq!(signature(ImportStyle::Kotlin, &plan, &r), "private fun sub(y: Long, x: Int): Int");
        let duplicate = Duplicate { range: 0..0, args: vec!["3".into(), "4".into()], assigned: None };
        let e = extract(ImportStyle::Kotlin, text, &plan, &r, "    ", &[duplicate]).unwrap();
        assert_eq!(apply(text, &e), "sub(4, 3)fun f(a: Int, b: Int) = println(sub(b, a))\n\nprivate fun sub(y: Long, x: Int): Int = x - y\n");
        // Not a permutation: the plan's order.
        r.order = vec![1, 1];
        assert_eq!(r.ordered(), [0, 1]);
    }
}

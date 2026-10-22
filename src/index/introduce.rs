//! Introduce Variable's view of the syntax tree (Kotlin and Java): the
//! expressions at the caret IntelliJ's "Expressions" popup lists, and for
//! one of them its occurrences in the enclosing block, the statement each
//! is in, what to name it after and, in Java, its type when the
//! expression shows it.

use std::ops::Range;

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;
use crate::domain::introduce_variable::{ConstantPlan, ExpressionBody, Extraction, Literal, NameSource, Occurrence};

const NO_EXPRESSION: &str = "Cannot find an expression to introduce";
const NOT_AN_EXPRESSION: &str = "Selected block should represent an expression";
const NO_CONTAINER: &str = "Cannot refactor in this place";
const NO_RETURN_TYPE: &str = "Cannot convert to a block body without the function's return type";

fn literal(lang: Lang, node: Node, text: &str) -> Option<Literal> {
    let t = &text[node.byte_range()];
    Some(match (lang, node.kind()) {
        (_, "string_literal") => Literal::String,
        (_, "character_literal") => Literal::Char,
        (Lang::Kotlin, "boolean_literal") | (Lang::Java, "true" | "false") => Literal::Boolean,
        (Lang::Kotlin, "long_literal") => Literal::Long,
        (Lang::Kotlin, "integer_literal" | "hex_literal" | "bin_literal" | "unsigned_literal") => Literal::Int,
        (Lang::Java, "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal") => {
            if t.ends_with(['l', 'L']) { Literal::Long } else { Literal::Int }
        }
        (Lang::Kotlin, "real_literal") | (Lang::Java, "decimal_floating_point_literal" | "hex_floating_point_literal") => {
            if t.ends_with(['f', 'F']) { Literal::Float } else { Literal::Double }
        }
        _ => return None,
    })
}

/// A value expression the variable can stand for (not a callee, a
/// declared name, an assignment's target or a type).
fn is_expression(lang: Lang, node: Node, text: &str) -> bool {
    let Some(parent) = node.parent() else { return false };
    let kind = node.kind();
    if literal(lang, node, text).is_some() {
        return true;
    }
    match lang {
        Lang::Kotlin => {
            // The callee of a call is not a value: `foo` of `foo()`.
            if parent.kind() == "call_expression" && parent.named_child(0).is_some_and(|c| c.id() == node.id()) {
                return false;
            }
            match kind {
                "simple_identifier" => {
                    let named_argument = parent.kind() == "value_argument" && node.next_sibling().is_some_and(|s| s.kind() == "=");
                    !named_argument
                        && (parent.kind().ends_with("_expression")
                            || matches!(parent.kind(), "value_argument" | "statements" | "control_structure_body" | "property_declaration"))
                }
                "call_expression" | "navigation_expression" | "additive_expression" | "multiplicative_expression" | "comparison_expression"
                | "equality_expression" | "conjunction_expression" | "disjunction_expression" | "elvis_expression" | "infix_expression"
                | "range_expression" | "prefix_expression" | "postfix_expression" | "as_expression" | "check_expression"
                | "parenthesized_expression" | "indexing_expression" | "lambda_literal" | "object_literal" | "this_expression" => true,
                _ => false,
            }
        }
        Lang::Java => match kind {
            "identifier" => {
                let field = |name: &str| parent.child_by_field_name(name).is_some_and(|c| c.id() == node.id());
                match parent.kind() {
                    "binary_expression" | "unary_expression" | "argument_list" | "parenthesized_expression" | "ternary_expression" | "array_access"
                    | "return_statement" | "array_initializer" => true,
                    "field_access" | "method_invocation" => field("object"),
                    "variable_declarator" => field("value"),
                    "assignment_expression" => field("right"),
                    "cast_expression" => field("value"),
                    _ => false,
                }
            }
            "parenthesized_expression" => !matches!(parent.kind(), "if_statement" | "while_statement" | "do_statement" | "switch_expression" | "synchronized_statement"),
            "method_invocation" | "field_access" | "binary_expression" | "unary_expression" | "object_creation_expression" | "array_access"
            | "cast_expression" | "ternary_expression" | "lambda_expression" | "array_creation_expression" | "instanceof_expression"
            | "method_reference" | "this" | "null_literal" => true,
            _ => false,
        },
        _ => false,
    }
}

/// The node spanning exactly `range` that is an expression.
pub(crate) fn expression_node<'t>(lang: Lang, root: Node<'t>, text: &str, range: Range<usize>) -> Option<Node<'t>> {
    let mut node = root.descendant_for_byte_range(range.start, range.end)?;
    loop {
        if node.byte_range() == range && is_expression(lang, node, text) {
            return Some(node);
        }
        if node.start_byte() < range.start || node.end_byte() > range.end {
            return None;
        }
        node = node.parent()?;
    }
}

fn supported(lang: Lang) -> bool {
    matches!(lang, Lang::Kotlin | Lang::Java)
}

/// The expressions Introduce Variable can take: the selected one, or with
/// no selection the ones around the caret, innermost first (IntelliJ's
/// "Expressions" popup).
pub fn expressions_at(lang: Lang, text: &str, selection: Range<usize>) -> Result<Vec<Range<usize>>, String> {
    if !supported(lang) {
        return Err(NO_EXPRESSION.into());
    }
    let tree = symbols::parse(lang, text).ok_or(NO_EXPRESSION)?;
    let root = tree.root_node();
    if !selection.is_empty() {
        let s = &text[selection.clone()];
        let start = selection.start + (s.len() - s.trim_start().len());
        let end = selection.end - (s.len() - s.trim_end().len());
        return expression_node(lang, root, text, start..end).map(|n| vec![n.byte_range()]).ok_or_else(|| NOT_AN_EXPRESSION.into());
    }
    let mut out: Vec<Range<usize>> = Vec::new();
    // The caret right after a name still means it.
    for at in [selection.start, selection.start.saturating_sub(1)] {
        let Some(mut node) = root.descendant_for_byte_range(at, at) else { continue };
        loop {
            if is_expression(lang, node, text) && !out.contains(&node.byte_range()) {
                out.push(node.byte_range());
            }
            match node.parent() {
                Some(p) if !matches!(p.kind(), "statements" | "block" | "class_body" | "source_file" | "program") => node = p,
                _ => break,
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    if out.is_empty() { Err(NO_EXPRESSION.into()) } else { Ok(out) }
}

fn is_block(lang: Lang, node: Node) -> bool {
    match lang {
        Lang::Kotlin => node.kind() == "statements",
        _ => node.kind() == "block",
    }
}

/// The statement of `node`: its ancestor directly in a block.
fn statement_of<'t>(lang: Lang, node: Node<'t>) -> Option<Node<'t>> {
    let mut n = node;
    loop {
        let parent = n.parent()?;
        if is_block(lang, parent) {
            return Some(n);
        }
        n = parent;
    }
}

/// The expression is its statement (`foo()` on its own).
fn is_whole(lang: Lang, node: Node, statement: Node) -> bool {
    match lang {
        Lang::Kotlin => node.id() == statement.id(),
        _ => statement.kind() == "expression_statement" && statement.named_child(0).is_some_and(|c| c.id() == node.id()),
    }
}

fn normalized(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn name_source(lang: Lang, node: Node, text: &str) -> NameSource {
    let t = |n: Node| text[n.byte_range()].to_owned();
    if let Some(l) = literal(lang, node, text) {
        return NameSource::Literal(l);
    }
    // The last name in a dotted chain: `a.b.getC` → `getC`.
    let last_name = |n: Node| {
        let s = t(n);
        s.rsplit('.').next().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
    };
    let source = match (lang, node.kind()) {
        (Lang::Kotlin, "call_expression") => node.named_child(0).and_then(last_name),
        (Lang::Kotlin, "navigation_expression") | (_, "simple_identifier" | "identifier") => last_name(node),
        (Lang::Kotlin | Lang::Java, "parenthesized_expression") => {
            return node.named_child(0).map_or(NameSource::Unknown, |inner| name_source(lang, inner, text));
        }
        (Lang::Java, "method_invocation") => node.child_by_field_name("name").map(t),
        (Lang::Java, "field_access") => node.child_by_field_name("field").map(t),
        (Lang::Java, "object_creation_expression" | "cast_expression") => {
            node.child_by_field_name("type").map(t).map(|ty| ty.split('<').next().unwrap_or(&ty).rsplit('.').next().unwrap_or_default().to_owned())
        }
        _ => None,
    };
    if let Some(name) = source {
        return NameSource::Name(name);
    }
    match boolean_or_string(lang, node, text) {
        Some(l) => NameSource::Literal(l),
        None => NameSource::Unknown,
    }
}

/// Comparisons and logic are booleans; `+` with a string is a string.
fn boolean_or_string(lang: Lang, node: Node, text: &str) -> Option<Literal> {
    match node.kind() {
        "comparison_expression" | "equality_expression" | "conjunction_expression" | "disjunction_expression" | "check_expression"
        | "instanceof_expression" => Some(Literal::Boolean),
        "binary_expression" => {
            let op = node.child_by_field_name("operator").map(|o| &text[o.byte_range()])?;
            match op {
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" => Some(Literal::Boolean),
                "+" if has_string(lang, node, text) => Some(Literal::String),
                _ => None,
            }
        }
        "additive_expression" if has_string(lang, node, text) => Some(Literal::String),
        _ => None,
    }
}

fn has_string(lang: Lang, node: Node, text: &str) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter().any(|c| literal(lang, c, text) == Some(Literal::String) || boolean_or_string(lang, c, text) == Some(Literal::String))
}

pub(crate) fn java_type(node: Node, text: &str) -> Option<String> {
    if let Some(l) = literal(Lang::Java, node, text) {
        return Some(l.java_type().to_owned());
    }
    match node.kind() {
        "object_creation_expression" | "cast_expression" => node.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned()),
        "parenthesized_expression" => node.named_child(0).and_then(|n| java_type(n, text)),
        _ => boolean_or_string(Lang::Java, node, text).map(|l| l.java_type().to_owned()),
    }
}

/// The Kotlin type an expression shows: a literal's, a constructor
/// call's (`Foo<Int>(…)`; not in Compose files, whose capitalized calls
/// are composables), a cast's, a comparison's or a string concatenation's.
pub(crate) fn kotlin_type(node: Node, text: &str) -> Option<String> {
    if node.kind() == "unsigned_literal" {
        return None;
    }
    if let Some(l) = literal(Lang::Kotlin, node, text) {
        return Some(l.kotlin_type().to_owned());
    }
    match node.kind() {
        "call_expression" => {
            let callee = node.named_child(0).filter(|c| c.kind() == "simple_identifier")?;
            let name = &text[callee.byte_range()];
            if !name.starts_with(|c: char| c.is_uppercase()) || text.contains("import androidx.compose") {
                return None;
            }
            let suffix = node.named_child(1)?;
            let mut cursor = suffix.walk();
            let arguments = suffix.named_children(&mut cursor).find(|c| c.kind() == "type_arguments").map(|t| &text[t.byte_range()]).unwrap_or_default();
            Some(format!("{name}{arguments}"))
        }
        "as_expression" => {
            let ty = node.named_child((node.named_child_count() as u32).checked_sub(1)?)?;
            let safe = text[node.byte_range()].contains("as?");
            Some(format!("{}{}", &text[ty.byte_range()], if safe { "?" } else { "" }))
        }
        "parenthesized_expression" => node.named_child(0).and_then(|n| kotlin_type(n, text)),
        "simple_identifier" => parameter_type(node, text),
        "additive_expression" | "multiplicative_expression" if boolean_or_string(Lang::Kotlin, node, text).is_none() => {
            // Arithmetic on numbers: the widest of its operands' types.
            const NUMBERS: [&str; 4] = ["Int", "Long", "Float", "Double"];
            let (left, right) = (node.named_child(0)?, node.named_child(1)?);
            let rank = |n: Node| kotlin_type(n, text).and_then(|t| NUMBERS.iter().position(|x| *x == t));
            Some(NUMBERS[rank(left)?.max(rank(right)?)].to_owned())
        }
        _ => boolean_or_string(Lang::Kotlin, node, text).map(|l| l.kotlin_type().to_owned()),
    }
}

/// The declared type of the parameter of the enclosing function `name` is.
fn parameter_type(name: Node, text: &str) -> Option<String> {
    let wanted = &text[name.byte_range()];
    let mut function = name.parent()?;
    while function.kind() != "function_declaration" {
        if matches!(function.kind(), "lambda_literal" | "class_body") {
            return None;
        }
        function = function.parent()?;
    }
    let mut cursor = function.walk();
    let parameters = function.named_children(&mut cursor).find(|c| c.kind() == "function_value_parameters")?;
    let mut cursor = parameters.walk();
    let found = parameters.named_children(&mut cursor).filter(|p| p.kind() == "parameter").find_map(|p| {
        let (n, t) = (p.named_child(0)?, p.named_child(1)?);
        (&text[n.byte_range()] == wanted).then(|| text[t.byte_range()].to_owned())
    });
    found
}

/// Kotlin: the expression body (`fun f() = …`) `node` is in, unless a
/// block around it is closer.
fn expression_body<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let mut n = node;
    while let Some(p) = n.parent() {
        match p.kind() {
            "statements" | "class_body" | "lambda_literal" => return None,
            "function_body" => return (p.child(0).is_some_and(|c| c.kind() == "=") && p.parent().is_some_and(|f| f.kind() == "function_declaration")).then_some(p),
            _ => n = p,
        }
    }
    None
}

/// The block body an expression body becomes: its range, expression,
/// the function's indentation and the return type to write.
fn block_body(body: Node, text: &str) -> Result<ExpressionBody, String> {
    let function = body.parent().ok_or(NO_CONTAINER)?;
    let expression = body.named_child(0).ok_or(NO_CONTAINER)?;
    let line = text[..function.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let indent = text[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut cursor = function.walk();
    let parameters = function.named_children(&mut cursor).find(|c| c.kind() == "function_value_parameters").ok_or(NO_CONTAINER)?;
    let declared = text[parameters.end_byte()..body.start_byte()].contains(':');
    let return_type = if declared { None } else { Some((parameters.end_byte(), kotlin_type(expression, text).ok_or(NO_RETURN_TYPE)?)) };
    Ok(ExpressionBody { range: body.byte_range(), expression: expression.byte_range(), indent, return_type })
}

/// Names declared in the function around `node` (parameters and locals):
/// the new variable must not clash with them.
fn declared_names(lang: Lang, node: Node, text: &str) -> Vec<String> {
    let mut scope = node;
    while let Some(p) = scope.parent() {
        scope = p;
        if matches!(scope.kind(), "function_declaration" | "method_declaration" | "constructor_declaration" | "anonymous_initializer" | "secondary_constructor") {
            break;
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![scope];
    while let Some(n) = stack.pop() {
        let declares = match (lang, n.kind()) {
            (Lang::Kotlin, "simple_identifier") => n.parent().is_some_and(|p| matches!(p.kind(), "variable_declaration" | "parameter" | "lambda_parameter" | "class_parameter")),
            (Lang::Java, "identifier") => n.parent().is_some_and(|p| {
                matches!(p.kind(), "variable_declarator" | "formal_parameter" | "catch_formal_parameter" | "enhanced_for_statement" | "inferred_parameters" | "lambda_expression")
                    && (p.child_by_field_name("name").is_some_and(|c| c.id() == n.id()) || matches!(p.kind(), "inferred_parameters" | "lambda_expression"))
            }),
            _ => false,
        };
        if declares {
            out.push(text[n.byte_range()].to_owned());
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    out
}

/// Introduce Variable for the expression at `range`: its occurrences in
/// the block around it, what to name it after, its Java type, and the
/// names already declared there.
pub fn extraction(lang: Lang, text: &str, range: Range<usize>) -> Result<(Extraction, Vec<String>), String> {
    let tree = symbols::parse(lang, text).ok_or(NO_EXPRESSION)?;
    let root = tree.root_node();
    let node = expression_node(lang, root, text, range).ok_or(NOT_AN_EXPRESSION)?;
    let body = if lang == Lang::Kotlin { expression_body(node) } else { None };
    let block = match body {
        Some(body) => body,
        None => statement_of(lang, node).and_then(|s| s.parent()).ok_or(NO_CONTAINER)?,
    };
    let body = body.map(|b| block_body(b, text)).transpose()?;
    let wanted = (node.kind(), normalized(&text[node.byte_range()]));
    let mut occurrences = Vec::new();
    let mut stack = vec![block];
    let mut found: Vec<Node> = Vec::new();
    while let Some(n) = stack.pop() {
        if n.kind() == wanted.0 && is_expression(lang, n, text) && normalized(&text[n.byte_range()]) == wanted.1 {
            found.push(n);
            continue;
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    found.sort_by_key(|n| n.start_byte());
    let mut selected = 0;
    for n in found {
        // Its statement in this block.
        let mut s = n;
        while let Some(p) = s.parent() {
            if p.id() == block.id() {
                break;
            }
            s = p;
        }
        if n.id() == node.id() {
            selected = occurrences.len();
        }
        let whole = if body.is_some() { s.id() == n.id() } else { is_whole(lang, n, s) };
        occurrences.push(Occurrence { range: n.byte_range(), statement: s.byte_range(), whole });
    }
    let extraction = Extraction {
        occurrences,
        selected,
        source: name_source(lang, node, text),
        java_type: (lang == Lang::Java).then(|| java_type(node, text)).flatten(),
        kotlin_type: (lang == Lang::Kotlin).then(|| kotlin_type(node, text)).flatten(),
        body,
    };
    Ok((extraction, declared_names(lang, node, text)))
}

const NON_CONSTANT_KOTLIN: &str = "Can't introduce constant for non constant expression";
const NON_CONSTANT_JAVA: &str = "Selected expression cannot be a constant initializer";

/// Kotlin conversions and operators a constant expression may call
/// (`1.toByte()`, `0xFF.and(0x0F)`): Kotlin evaluates them at compile time.
const KOTLIN_CONST_CALLS: &[&str] = &[
    "toByte", "toShort", "toInt", "toLong", "toFloat", "toDouble", "toChar", "toUByte", "toUShort", "toUInt", "toULong", "and", "or", "xor", "inv", "shl", "shr",
    "ushr", "plus", "minus", "times", "div", "rem", "compareTo", "equals", "not", "unaryMinus", "unaryPlus",
];

/// Kotlin's names declared `const val` in the file.
fn kotlin_consts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices("const val ") {
        if text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let name: String = text[i + 10..].trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// Whether Kotlin can evaluate the expression at compile time (IntelliJ
/// asks the compiler; this is the same rule on the syntax).
fn kotlin_constant(node: Node, text: &str, consts: &[String]) -> bool {
    let named: Vec<Node> = {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).collect()
    };
    match node.kind() {
        // Templates (`"$x"`) are not constant here.
        "string_literal" => named.iter().all(|c| matches!(c.kind(), "string_content" | "escape_sequence" | "character_escape_seq")),
        _ if literal(Lang::Kotlin, node, text).is_some() => true,
        "simple_identifier" => consts.iter().any(|c| c == &text[node.byte_range()]),
        "parenthesized_expression" | "prefix_expression" | "additive_expression" | "multiplicative_expression" | "comparison_expression"
        | "equality_expression" | "conjunction_expression" | "disjunction_expression" => named.iter().all(|c| kotlin_constant(*c, text, consts)),
        // `a shl 2`: its operator is a name.
        "infix_expression" => named.len() == 3 && kotlin_constant(named[0], text, consts) && kotlin_constant(named[2], text, consts),
        "call_expression" => {
            let (Some(callee), Some(suffix)) = (named.first(), named.get(1)) else { return false };
            if callee.kind() != "navigation_expression" {
                return false;
            }
            let receiver = callee.named_child(0);
            let method = text[callee.byte_range()].rsplit('.').next().unwrap_or_default().trim();
            let mut args = Vec::new();
            let mut stack = vec![*suffix];
            while let Some(n) = stack.pop() {
                if n.kind() == "value_argument" {
                    args.extend(n.named_child((n.named_child_count() as u32).saturating_sub(1)));
                    continue;
                }
                let mut cursor = n.walk();
                stack.extend(n.named_children(&mut cursor));
            }
            KOTLIN_CONST_CALLS.contains(&method)
                && receiver.is_some_and(|r| kotlin_constant(r, text, consts))
                && args.iter().all(|a| kotlin_constant(*a, text, consts))
        }
        _ => false,
    }
}

/// Whether a Java expression can initialize a static field: it uses no
/// local, parameter, `this` or instance field (an unqualified lower-case
/// name not declared static in the file).
fn java_static_safe(node: Node, text: &str, locals: &[String]) -> bool {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "this" | "super" => return false,
            "identifier" => {
                let parent = n.parent();
                let named_part = parent.is_some_and(|p| {
                    ["name", "field", "type"].iter().any(|f| p.child_by_field_name(f).is_some_and(|c| c.id() == n.id()))
                });
                let name = &text[n.byte_range()];
                // Declared `static … name =` / `;` somewhere in the file.
                let static_field = text.match_indices(name).any(|(i, _)| {
                    let line_start = text[..i].rfind('\n').map_or(0, |j| j + 1);
                    let line = &text[line_start..i];
                    line.contains("static ") && !line.contains('(') && text[i + name.len()..].trim_start().starts_with(['=', ';'])
                });
                let class_like = name.chars().next().is_some_and(char::is_uppercase);
                if !named_part && (locals.iter().any(|l| l == name) || !class_like && !static_field) {
                    return false;
                }
            }
            _ => {}
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    true
}

/// Introduce Constant for the expression at `range`: whether it can be
/// one, its occurrences in the file, and where the constant goes.
pub fn constant(lang: Lang, text: &str, range: Range<usize>) -> Result<(ConstantPlan, Vec<String>), String> {
    let tree = symbols::parse(lang, text).ok_or(NO_EXPRESSION)?;
    let root = tree.root_node();
    let node = expression_node(lang, root, text, range).ok_or(NOT_AN_EXPRESSION)?;
    match lang {
        Lang::Kotlin if !kotlin_constant(node, text, &kotlin_consts(text)) => return Err(NON_CONSTANT_KOTLIN.into()),
        Lang::Java if !java_static_safe(node, text, &declared_names(lang, node, text)) => return Err(NON_CONSTANT_JAVA.into()),
        _ => {}
    }
    let line_start = |at: usize| text[..at].rfind('\n').map_or(0, |i| i + 1);
    let (insert_at, indent) = match lang {
        Lang::Kotlin => {
            // Before the top-level declaration holding it, its doc comment
            // staying with it.
            let mut top = node;
            while let Some(p) = top.parent() {
                if p.parent().is_none() {
                    break;
                }
                top = p;
            }
            let mut start = top;
            while let Some(prev) = start.prev_sibling().filter(|p| p.kind().contains("comment") && text[p.end_byte()..start.start_byte()].matches('\n').count() <= 1) {
                start = prev;
            }
            (line_start(start.start_byte()), String::new())
        }
        _ => {
            // First in the nearest class that can hold a static field.
            let mut n = node;
            let body = loop {
                let p = n.parent().ok_or(NO_CONTAINER)?;
                if p.kind() == "class_body" || p.kind() == "interface_body" || p.kind() == "enum_body" {
                    let class = p.parent().ok_or(NO_CONTAINER)?;
                    let top_level = class.parent().is_some_and(|g| g.kind() == "program");
                    let is_static = class.named_child(0).is_some_and(|m| m.kind() == "modifiers" && text[m.byte_range()].split_whitespace().any(|w| w == "static"));
                    if top_level || is_static || class.kind() != "class_declaration" {
                        break p;
                    }
                }
                n = p;
            };
            let first = body.named_child(0).ok_or(NO_CONTAINER)?;
            let at = line_start(first.start_byte());
            let indent: String = text[at..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            (at, indent)
        }
    };
    let wanted = (node.kind(), normalized(&text[node.byte_range()]));
    let mut found: Vec<Node> = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == wanted.0 && is_expression(lang, n, text) && normalized(&text[n.byte_range()]) == wanted.1 {
            found.push(n);
            continue;
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    found.sort_by_key(|n| n.start_byte());
    let selected = found.iter().position(|n| n.id() == node.id()).unwrap_or(0);
    let string_content = (node.kind() == "string_literal").then(|| text[node.byte_range()].trim_matches('"').to_owned());
    let plan = ConstantPlan {
        occurrences: found.iter().map(|n| n.byte_range()).collect(),
        selected,
        insert_at,
        indent,
        source: name_source(lang, node, text),
        java_type: (lang == Lang::Java).then(|| java_type(node, text)).flatten(),
        string_content,
    };
    // Every name in the file: a new constant must not shadow or clash.
    let taken = text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_owned).collect();
    Ok((plan, taken))
}

/// Where a live template's abbreviation starting at `offset` is, as
/// IntelliJ's template contexts see it: a statement or expression in a
/// body, a class (or Kotlin `object`) member, the Kotlin top level, or
/// Java's place after an `if` block. None in comments and strings.
pub fn template_contexts(lang: Lang, text: &str, offset: usize) -> Vec<crate::domain::live_templates::Context> {
    use crate::domain::live_templates::Context;
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let Some(node) = tree.root_node().descendant_for_byte_range(offset, offset) else { return Vec::new() };
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let before = text[line_start..offset].trim();
    let statement_start = before.is_empty() || before.ends_with(['{', ';', '}']);
    let mut n = Some(node);
    while let Some(x) = n {
        let kind = x.kind();
        if kind.contains("comment") || kind.contains("string") {
            return Vec::new();
        }
        match (lang, kind) {
            // `object O { maino }` with nothing else in it parses as `object`
            // infix `O` applied to a lambda: it is an object body.
            (Lang::Kotlin, "statements" | "lambda_literal")
                if [Some(x), x.parent()].into_iter().flatten().any(|l| {
                    l.kind() == "lambda_literal"
                        && l.parent().is_some_and(|p| p.kind() == "infix_expression" && p.named_child(0).is_some_and(|c| c.kind() == "object_literal"))
                }) =>
            {
                return vec![Context::Class, Context::Object];
            }
            (Lang::Kotlin, "statements" | "function_body" | "lambda_literal" | "control_structure_body") | (Lang::Java, "block" | "switch_block_statement_group") => {
                let mut out = vec![Context::Expression];
                if statement_start {
                    out.push(Context::Statement);
                }
                if lang == Lang::Java && text[..offset].trim_end().ends_with('}') {
                    out.push(Context::Else);
                }
                return out;
            }
            (_, "class_body" | "interface_body" | "enum_body" | "enum_class_body") => {
                let object = x.parent().is_some_and(|p| matches!(p.kind(), "object_declaration" | "companion_object"));
                return if object { vec![Context::Class, Context::Object] } else { vec![Context::Class] };
            }
            _ => {}
        }
        n = x.parent();
    }
    if lang == Lang::Kotlin { vec![Context::TopLevel] } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, needle: &str) -> Range<usize> {
        let i = text.find(needle).unwrap();
        i..i + needle.len()
    }

    #[test]
    fn lists_expressions_at_the_caret() {
        let kt = "fun f(p: Point) {\n    println(p.getX() + 1)\n}\n";
        let caret = kt.find("getX").unwrap() + 1;
        let found: Vec<&str> = expressions_at(Lang::Kotlin, kt, caret..caret).unwrap().into_iter().map(|r| &kt[r]).collect();
        assert_eq!(found, ["p.getX()", "p.getX() + 1", "println(p.getX() + 1)"]);
        let java = "class A { void f(Point p) { foo(new Bar(1), p.x); } }\n";
        let caret = java.find("Bar").unwrap();
        let found: Vec<&str> = expressions_at(Lang::Java, java, caret..caret).unwrap().into_iter().map(|r| &java[r]).collect();
        assert_eq!(found, ["new Bar(1)", "foo(new Bar(1), p.x)"]);
        // A selection must be an expression.
        assert_eq!(expressions_at(Lang::Kotlin, kt, at(kt, "p.getX() ")).unwrap(), vec![at(kt, "p.getX()")]);
        assert_eq!(expressions_at(Lang::Kotlin, kt, at(kt, "getX() +")), Err(NOT_AN_EXPRESSION.into()));
    }

    #[test]
    fn finds_template_contexts() {
        use crate::domain::live_templates::Context;
        let kt = "fun f() {\n    sout\n    val x = ifn\n}\nclass A {\n    fun0\n}\nobject O {\n    maino\n}\nmain\n// sout\n";
        let at = |w: &str| kt.find(w).unwrap();
        assert_eq!(template_contexts(Lang::Kotlin, kt, at("sout")), vec![Context::Expression, Context::Statement]);
        assert_eq!(template_contexts(Lang::Kotlin, kt, at("ifn")), vec![Context::Expression]);
        assert_eq!(template_contexts(Lang::Kotlin, kt, at("fun0")), vec![Context::Class]);
        assert_eq!(template_contexts(Lang::Kotlin, kt, at("maino")), vec![Context::Class, Context::Object]);
        assert_eq!(template_contexts(Lang::Kotlin, kt, kt.rfind("main").unwrap() - 0), vec![Context::TopLevel]);
        assert_eq!(template_contexts(Lang::Kotlin, kt, kt.rfind("sout").unwrap()), vec![]);
        let java = "class A {\n  psvm\n  void f() {\n    if (a) {\n    } else-if\n    sout\n  }\n}\n";
        assert_eq!(template_contexts(Lang::Java, java, java.find("psvm").unwrap()), vec![Context::Class]);
        assert!(template_contexts(Lang::Java, java, java.find("else-if").unwrap()).contains(&Context::Else));
        assert!(template_contexts(Lang::Java, java, java.find("sout").unwrap()).contains(&Context::Statement));
    }

    #[test]
    fn plans_constants() {
        let kt = "val a = 1\n\n/** Doc. */\nfun foo(): Int {\n    val x = 2 + 3\n    println(\"Hi $a\")\n    return 2\n}\n\nconst val K = 4\nfun bar() = K * 2 + a\n";
        let (plan, _) = constant(Lang::Kotlin, kt, at(kt, "2 + 3")).unwrap();
        assert_eq!(plan.insert_at, kt.find("/** Doc.").unwrap());
        let two = kt.find("return 2").unwrap() + 7;
        let (plan, _) = constant(Lang::Kotlin, kt, two..two + 1).unwrap();
        // Every `2` in the file: in `2 + 3`, `return 2` and `K * 2`.
        assert_eq!(plan.occurrences.len(), 3);
        assert_eq!(plan.selected, 1);
        assert!(constant(Lang::Kotlin, kt, at(kt, "K * 2")).is_ok());
        assert_eq!(constant(Lang::Kotlin, kt, at(kt, "K * 2 + a")).unwrap_err(), NON_CONSTANT_KOTLIN);
        assert_eq!(constant(Lang::Kotlin, kt, at(kt, "\"Hi $a\"")).unwrap_err(), NON_CONSTANT_KOTLIN);
        let java = "class Test {\n\n  int count;\n  static final int MAX = 3;\n\n  int get(int n) {\n    return Math.max(MAX, 4) + n + count;\n  }\n}\n";
        let (plan, _) = constant(Lang::Java, java, at(java, "Math.max(MAX, 4)")).unwrap();
        assert_eq!(plan.insert_at, java.find("  int count").unwrap());
        assert_eq!(plan.indent, "  ");
        assert_eq!(constant(Lang::Java, java, at(java, "Math.max(MAX, 4) + n")).unwrap_err(), NON_CONSTANT_JAVA);
        assert_eq!(constant(Lang::Java, java, at(java, "Math.max(MAX, 4) + n + count")).unwrap_err(), NON_CONSTANT_JAVA);
    }

    #[test]
    fn finds_occurrences_and_names() {
        let kt = "fun f(p: Point, x: Int) {\n    println(p.x + 1)\n    if (ok) show(p.x)\n    val q = 2\n}\n";
        let first = at(kt, "p.x");
        let (ex, taken) = extraction(Lang::Kotlin, kt, first.clone()).unwrap();
        assert_eq!(ex.occurrences.len(), 2);
        assert_eq!(&kt[ex.occurrences[1].statement.clone()], "if (ok) show(p.x)");
        assert_eq!(ex.source, NameSource::Name("x".into()));
        assert!(taken.contains(&"x".to_owned()) && taken.contains(&"q".to_owned()) && taken.contains(&"p".to_owned()));
        // A call on its own line is its whole statement.
        let (ex, _) = extraction(Lang::Kotlin, kt, at(kt, "println(p.x + 1)")).unwrap();
        assert!(ex.occurrences[0].whole);
        // Java types from what the expression shows.
        let java = "class A { void f() { foo(\"a\" + b, new Bar<String>(1), c > 2, d.get()); } }\n";
        let ty = |e: &str| extraction(Lang::Java, java, at(java, e)).unwrap().0.java_type;
        assert_eq!(ty("\"a\" + b").as_deref(), Some("String"));
        assert_eq!(ty("new Bar<String>(1)").as_deref(), Some("Bar<String>"));
        assert_eq!(ty("c > 2").as_deref(), Some("boolean"));
        assert_eq!(ty("d.get()"), None);
        assert_eq!(extraction(Lang::Java, java, at(java, "new Bar<String>(1)")).unwrap().0.source, NameSource::Name("Bar".into()));
        // Outside a block: nowhere to declare it.
        let field = "class A { val x = 1 + 2 }\n";
        assert_eq!(extraction(Lang::Kotlin, field, at(field, "1 + 2")).unwrap_err(), NO_CONTAINER);
        // Kotlin types for "Specify type explicitly".
        let kt = "fun f() {\n    g(\"a\" + b, Box<Int>(1), c > 2, d as? Foo, 1L, h(), 2u)\n}\n";
        let ty = |e: &str| extraction(Lang::Kotlin, kt, at(kt, e)).unwrap().0.kotlin_type;
        assert_eq!(ty("\"a\" + b").as_deref(), Some("String"));
        assert_eq!(ty("Box<Int>(1)").as_deref(), Some("Box<Int>"));
        assert_eq!(ty("c > 2").as_deref(), Some("Boolean"));
        assert_eq!(ty("d as? Foo").as_deref(), Some("Foo?"));
        assert_eq!(ty("1L").as_deref(), Some("Long"));
        assert_eq!(ty("h()"), None);
        assert_eq!(ty("2u"), None);
        let kt = "fun f(a: Int, d: Double) = a * 2 + a * d\n";
        let ty = |e: &str| extraction(Lang::Kotlin, kt, at(kt, e)).unwrap().0.kotlin_type;
        assert_eq!(ty("a * 2").as_deref(), Some("Int"));
        assert_eq!(ty("a * 2 + a * d").as_deref(), Some("Double"));
    }

    #[test]
    fn plans_expression_bodies() {
        let kt = "class A {\n    fun f(a: Int): Int = a * 2 + a * 2\n    fun g(s: String) = \"<\" + s\n    fun h() = run { 1 + 2 }\n    fun k(x: X) = x.y + 1\n}\n";
        let (ex, _) = extraction(Lang::Kotlin, kt, at(kt, "a * 2")).unwrap();
        assert_eq!(ex.occurrences.len(), 2);
        let body = ex.body.unwrap();
        assert_eq!(&kt[body.range.clone()], "= a * 2 + a * 2");
        assert_eq!(&kt[body.expression.clone()], "a * 2 + a * 2");
        assert_eq!((body.indent.as_str(), body.return_type), ("    ", None));
        let (ex, _) = extraction(Lang::Kotlin, kt, at(kt, "\"<\"")).unwrap();
        let (at_, ty) = ex.body.unwrap().return_type.unwrap();
        assert_eq!((&kt[..at_].rsplit('\n').next().unwrap()[4..], ty.as_str()), ("fun g(s: String)", "String"));
        // In a lambda: its block, not the body.
        assert!(extraction(Lang::Kotlin, kt, at(kt, "1 + 2")).unwrap().0.body.is_none());
        // The return type isn't known.
        assert_eq!(extraction(Lang::Kotlin, kt, at(kt, "x.y")).unwrap_err(), NO_RETURN_TYPE);
    }
}

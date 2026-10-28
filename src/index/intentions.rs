//! Alt+Enter intentions that read the syntax tree (Kotlin), as IntelliJ's
//! Kotlin plugin names them: Convert concatenation to template / Convert
//! template to concatenated string, Specify type explicitly / Specify
//! return type explicitly (when the expression or the declarations of
//! its calls show its type), and Add
//! names to call arguments (given the callee's parameters).

use tree_sitter::Node;

use super::extract_method::children;
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

fn innermost<'t>(root: Node<'t>, at: usize) -> Option<Node<'t>> {
    root.descendant_for_byte_range(at, at)
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(Some(node), |n| n.parent())
}

/// A non-raw string literal.
fn plain_string(node: Node, text: &str) -> bool {
    node.kind() == "string_literal" && !text[node.byte_range()].starts_with("\"\"\"")
}

/// The intentions at `caret` in a Kotlin file.
pub fn intentions(text: &str, caret: usize) -> Vec<Intention> {
    let Some(tree) = symbols::parse(Lang::Kotlin, text) else { return Vec::new() };
    let root = tree.root_node();
    let mut out = Vec::new();
    let shown = |n: Node| super::introduce::kotlin_type(n, text).filter(|t| !t.is_empty()).map(|t| (t, Vec::new()));
    out.extend(specify_type(root, text, caret, &shown).map(|(i, _)| i));
    out.extend(to_template(root, text, caret));
    out.extend(to_concatenation(root, text, caret));
    out.extend(remove_loop_indices(root, text, caret));
    out
}

/// Specify type explicitly / Specify return type explicitly where only
/// the declarations of the calls show the type (`val a = user.getAddress()`
/// → `val a: Address = …`), with the classes to import; None where the
/// expression shows it itself (those are among `intentions`).
pub fn specify_inferred_type(text: &str, caret: usize, classes: super::catches::Classes) -> Option<(Intention, Vec<String>)> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let declared = |n: Node| {
        if super::introduce::kotlin_type(n, text).is_some_and(|t| !t.is_empty()) {
            return None;
        }
        super::types::type_of(Lang::Kotlin, text, n.byte_range(), classes).map(|t| (t.ty, t.imports))
    };
    specify_type(tree.root_node(), text, caret, &declared)
}

/// Iterate over 'List<User>' (IntelliJ's IterateExpressionIntention): an
/// expression statement of a collection's type, the caret at its start or
/// end, becomes `for (user in users) {` with the caret in the body. The
/// variable is named after the expression (`users` → `user`) or the
/// element type (`User` → `user`, `Int` → `i`, `String` → `s`), else `e`.
pub fn iterate_over(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let root = tree.root_node();
    let statement = [caret, caret.saturating_sub(1)]
        .into_iter()
        .filter_map(|at| innermost(root, at))
        .flat_map(ancestors)
        .find(|n| n.parent().is_some_and(|p| p.kind() == "statements") && (n.start_byte() == caret || n.end_byte() == caret))?;
    if statement.kind().ends_with("declaration") || matches!(statement.kind(), "assignment" | "for_statement" | "while_statement" | "do_while_statement") {
        return None;
    }
    let ty = super::introduce::kotlin_type(statement, text)
        .filter(|t| !t.is_empty())
        .or_else(|| super::types::type_of(Lang::Kotlin, text, statement.byte_range(), classes).map(|t| t.ty))?;
    let element = element_type(&ty)?;
    let expression = &text[statement.byte_range()];
    let name = iteration_name(expression, &element);
    let line = text[..statement.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let unit = if indent.starts_with('\t') { "\t" } else { "    " };
    let head = format!("for ({name} in {expression}) {{\n{indent}{unit}");
    let replacement = format!("{head}\n{indent}}}");
    Some(Intention { title: format!("Iterate over '{ty}'"), range: statement.byte_range(), caret: statement.start_byte() + head.len(), replacement })
}

/// What a `for` over a value of the type goes through; None when it
/// isn't iterable (as far as the name shows).
fn element_type(ty: &str) -> Option<String> {
    let ty = ty.trim_end_matches('?');
    let (head, args) = match ty.find('<') {
        Some(i) if ty.ends_with('>') => (&ty[..i], &ty[i + 1..ty.len() - 1]),
        Some(_) => return None,
        None => (ty, ""),
    };
    let head = head.rsplit('.').next().unwrap_or(head);
    let primitive = |p: &str| Some(p.to_owned());
    match head {
        "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "MutableCollection" | "Iterable" | "MutableIterable" | "Sequence" | "Array" | "Iterator" | "ArrayDeque"
            if !args.is_empty() =>
        {
            Some(args.trim().trim_start_matches("out ").trim_start_matches("in ").to_owned())
        }
        "Map" | "MutableMap" | "HashMap" | "LinkedHashMap" if !args.is_empty() => Some(format!("Map.Entry<{args}>")),
        "String" | "CharSequence" | "CharArray" | "CharRange" => primitive("Char"),
        "IntArray" | "IntRange" | "IntProgression" => primitive("Int"),
        "LongArray" | "LongRange" | "LongProgression" => primitive("Long"),
        "ShortArray" => primitive("Short"),
        "ByteArray" => primitive("Byte"),
        "DoubleArray" => primitive("Double"),
        "FloatArray" => primitive("Float"),
        "BooleanArray" => primitive("Boolean"),
        _ => None,
    }
}

/// The loop variable's name, as Kotlin's name suggester gives it.
fn iteration_name(expression: &str, element: &str) -> String {
    // `users`, `repo.users`, `getUsers()`: the singular.
    let last = expression.trim_end_matches("()").rsplit(['.', ' ']).next().unwrap_or("");
    let last = last.strip_prefix("get").filter(|r| r.starts_with(|c: char| c.is_uppercase())).unwrap_or(last);
    if last.chars().all(|c| c.is_alphanumeric() || c == '_') && last.starts_with(|c: char| c.is_alphabetic()) {
        let singular = if let Some(stem) = last.strip_suffix("ies") {
            Some(format!("{stem}y"))
        } else if last.ends_with("ss") {
            None
        } else {
            last.strip_suffix('s').map(str::to_owned)
        };
        if let Some(s) = singular.filter(|s| s.len() > 1) {
            let mut chars = s.chars();
            let first = chars.next().unwrap_or('e').to_lowercase().collect::<String>();
            return first + chars.as_str();
        }
    }
    let base = element.split('<').next().unwrap_or(element).trim_end_matches('?');
    let base = base.rsplit('.').next().unwrap_or(base);
    match base {
        "Int" => "i".into(),
        "Long" => "l".into(),
        "Short" | "String" => "s".into(),
        "Byte" | "Boolean" => "b".into(),
        "Char" => "c".into(),
        "Double" => "d".into(),
        "Float" => "f".into(),
        "Entry" => "entry".into(),
        _ if base.starts_with(|c: char| c.is_uppercase()) && base.chars().all(|c| c.is_alphanumeric()) => {
            let mut chars = base.chars();
            let first = chars.next().unwrap_or('e').to_lowercase().collect::<String>();
            first + chars.as_str()
        }
        _ => "e".into(),
    }
}

/// The `for` loop whose header (before its body) holds `caret`, with its
/// variable(s), the iterated expression and the body.
fn for_header<'t>(root: Node<'t>, caret: usize) -> Option<(Node<'t>, Node<'t>, Node<'t>, Option<Node<'t>>)> {
    let at = innermost(root, caret)?;
    let n = ancestors(at).find(|n| matches!(n.kind(), "for_statement" | "statements" | "lambda_literal" | "class_body"))?;
    if n.kind() != "for_statement" {
        return None;
    }
    let mut cursor = n.walk();
    let named: Vec<Node> = n.named_children(&mut cursor).collect();
    let variable = *named.first()?;
    let iterated = *named.get(1)?;
    let body = named.get(2).copied();
    if body.is_some_and(|b| caret >= b.start_byte()) {
        return None;
    }
    Some((variable, iterated, n, body))
}

/// Whether `name` is a word in `text`.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// Add indices to 'for' loop (IntelliJ's AddForLoopIndicesIntention):
/// `for (x in list)` → `for ((index, x) in list.withIndex())`, when the
/// loop goes through a collection, array, sequence or string.
pub fn add_loop_indices(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (variable, iterated, statement, _) = for_header(tree.root_node(), caret)?;
    if variable.kind() != "variable_declaration" {
        return None;
    }
    let ty = super::introduce::kotlin_type(iterated, text)
        .filter(|t| !t.is_empty())
        .or_else(|| super::types::type_of(Lang::Kotlin, text, iterated.byte_range(), classes).map(|t| t.ty))?;
    // A map has no `withIndex()`.
    if element_type(&ty)?.starts_with("Map.Entry") {
        return None;
    }
    let loop_text = &text[statement.byte_range()];
    let index = std::iter::once("index".to_owned()).chain((1..).map(|n| format!("index{n}"))).find(|name| !mentions(loop_text, name))?;
    let expression = &text[iterated.byte_range()];
    let expression = if matches!(iterated.kind(), "simple_identifier" | "navigation_expression" | "call_expression" | "parenthesized_expression" | "this_expression") {
        expression.to_owned()
    } else {
        format!("({expression})")
    };
    let middle = &text[variable.end_byte()..iterated.start_byte()];
    let start = variable.start_byte();
    Some(Intention {
        title: "Add indices to 'for' loop".into(),
        range: start..iterated.end_byte(),
        replacement: format!("({index}, {}){middle}{expression}.withIndex()", &text[variable.byte_range()]),
        caret: start + 1 + index.len(),
    })
}

/// Remove indices in 'for' loop: `for ((index, x) in list.withIndex())`
/// → `for (x in list)` when the loop doesn't use `index`.
fn remove_loop_indices(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let (variables, iterated, _, body) = for_header(root, caret)?;
    if variables.kind() != "multi_variable_declaration" || variables.named_child_count() != 2 {
        return None;
    }
    let index = &text[variables.named_child(0)?.byte_range()];
    let value = &text[variables.named_child(1)?.byte_range()];
    // `….withIndex()`
    let call = text[iterated.byte_range()].strip_suffix(".withIndex()")?;
    if iterated.kind() != "call_expression" || index != "_" && body.is_some_and(|b| mentions(&text[b.byte_range()], index)) {
        return None;
    }
    let middle = &text[variables.end_byte()..iterated.start_byte()];
    let call = call.strip_prefix('(').and_then(|c| c.strip_suffix(')')).filter(|c| !c.contains(['(', ')'])).unwrap_or(call);
    let start = variables.start_byte();
    Some(Intention { title: "Remove indices in 'for' loop".into(), range: start..iterated.end_byte(), replacement: format!("{value}{middle}{call}"), caret: start })
}

/// Replace 'for each' loop with indexed 'for' loop (Java, IntelliJ's
/// ReplaceForEachLoopWithIndexedForLoopIntention): over a `List` or an
/// array named by a variable or field, `for (String s : names) {` →
/// `for (int i = 0; i < names.size(); i++) {` with `String s = names.get(i);`
/// (`names.length`, `names[i]` for an array) first in the body.
pub fn indexed_for_loop(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let at = innermost(tree.root_node(), caret)?;
    let n = ancestors(at).find(|n| matches!(n.kind(), "enhanced_for_statement" | "block" | "class_body"))?;
    if n.kind() != "enhanced_for_statement" {
        return None;
    }
    let ty = n.child_by_field_name("type")?;
    let name = n.child_by_field_name("name")?;
    let value = n.child_by_field_name("value")?;
    let body = n.child_by_field_name("body")?;
    if caret >= body.start_byte() || !matches!(value.kind(), "identifier" | "field_access") {
        return None;
    }
    let collection = super::surround::expression_type(Lang::Java, value, text).or_else(|| super::types::type_of(Lang::Java, text, value.byte_range(), classes).map(|t| t.ty))?;
    let head = collection.split('<').next().unwrap_or(&collection);
    let expression = &text[value.byte_range()];
    let (bound, element) = if collection.ends_with("[]") {
        (format!("{expression}.length"), format!("{expression}[{{i}}]"))
    } else if matches!(head, "List" | "ArrayList" | "LinkedList" | "MutableList" | "java.util.List" | "CopyOnWriteArrayList" | "Vector") {
        (format!("{expression}.size()"), format!("{expression}.get({{i}})"))
    } else {
        return None;
    };
    // An index name the loop doesn't use: i, j, k…
    let loop_text = &text[n.byte_range()];
    let index = ["i", "j", "k", "m", "n"].into_iter().map(str::to_owned).chain((1..).map(|k| format!("i{k}"))).find(|c| !mentions(loop_text, c))?;
    let element = element.replace("{i}", &index);
    let line = text[..n.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let unit = if indent.starts_with('\t') { "\t" } else { "    " };
    let declaration = format!("{} {} = {element};", &text[ty.byte_range()], &text[name.byte_range()]);
    let header = format!("for (int {index} = 0; {index} < {bound}; {index}++) ");
    let body_text = &text[body.byte_range()];
    let new_body = if body.kind() == "block" {
        // After the `{`, on a line of its own.
        let inner = &body_text[1..];
        format!("{{\n{indent}{unit}{declaration}{}", if inner.trim_start().starts_with('}') { format!("\n{indent}}}") } else { format!("\n{indent}{unit}{}", inner.trim_start()) })
    } else {
        format!("{{\n{indent}{unit}{declaration}\n{indent}{unit}{body_text}\n{indent}}}")
    };
    Some(Intention { title: "Replace 'for each' loop with indexed 'for' loop".into(), range: n.byte_range(), replacement: format!("{header}{new_body}"), caret: n.start_byte() })
}

type Typing<'a> = dyn Fn(Node) -> Option<(String, Vec<String>)> + 'a;

/// `val x = 1` → `val x: Int = 1`; `fun f() = 1` → `fun f(): Int = 1`;
/// the type (and its imports) from `typing`.
fn specify_type(root: Node, text: &str, caret: usize, typing: &Typing) -> Option<(Intention, Vec<String>)> {
    let at = innermost(root, caret)?;
    for n in ancestors(at) {
        match n.kind() {
            "property_declaration" => {
                let parts = children(n);
                let variable = parts.iter().find(|c| c.kind() == "variable_declaration")?;
                // Typed already, or a destructuring declaration.
                if children(*variable).len() != 1 {
                    return None;
                }
                let eq = {
                    let mut cursor = n.walk();
                    n.children(&mut cursor).find(|c| c.kind() == "=")?
                };
                // On the declaration, not in its initializer.
                if caret > eq.start_byte() {
                    return None;
                }
                let init = eq.next_named_sibling()?;
                let (ty, imports) = typing(init)?;
                let end = variable.end_byte();
                return Some((Intention { title: "Specify type explicitly".into(), range: end..end, replacement: format!(": {ty}"), caret: end + 2 + ty.len() }, imports));
            }
            "function_declaration" => {
                let parts = children(n);
                let params = parts.iter().find(|c| c.kind() == "function_value_parameters")?;
                // A declared return type follows the parameters.
                if parts.iter().any(|c| c.start_byte() >= params.end_byte() && matches!(c.kind(), "user_type" | "nullable_type" | "function_type")) {
                    return None;
                }
                let body = parts.iter().find(|c| c.kind() == "function_body")?;
                if caret >= body.start_byte() || !text[body.byte_range()].starts_with('=') {
                    return None;
                }
                let expression = body.named_child(0)?;
                let (ty, imports) = typing(expression)?;
                let end = params.end_byte();
                return Some((Intention { title: "Specify return type explicitly".into(), range: end..end, replacement: format!(": {ty}"), caret: end + 2 + ty.len() }, imports));
            }
            "statements" | "class_body" | "lambda_literal" => return None,
            _ => {}
        }
    }
    None
}

/// The operands of a `+` chain (none when it has another operator).
fn plus_operands<'t>(chain: Node<'t>, text: &str, out: &mut Vec<Node<'t>>) -> bool {
    if chain.kind() != "additive_expression" {
        out.push(chain);
        return true;
    }
    let mut cursor = chain.walk();
    let all: Vec<Node> = chain.children(&mut cursor).collect();
    let [left, op, right] = all.as_slice() else { return false };
    &text[op.byte_range()] == "+" && plus_operands(*left, text, out) && plus_operands(*right, text, out)
}

/// `"a" + x + "b"` → `"a${x}b"`: a `+` chain starting with a string, so
/// every `+` in it joins strings.
fn to_template(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let at = innermost(root, caret)?;
    // The outermost `+` chain around the caret.
    let chain = ancestors(at).take_while(|n| n.kind() == "additive_expression" || n == &at || n.parent().is_some_and(|p| p.kind() == "additive_expression")).filter(|n| n.kind() == "additive_expression").last()?;
    let mut operands = Vec::new();
    if !plus_operands(chain, text, &mut operands) || !operands.first().is_some_and(|o| plain_string(*o, text)) {
        return None;
    }
    if operands.iter().any(|o| o.kind() == "string_literal" && !plain_string(*o, text)) {
        return None;
    }
    let mut body = String::new();
    // `$name` waits for what follows: a letter or digit makes it `${name}`.
    let mut pending: Option<String> = None;
    let push = |body: &mut String, pending: &mut Option<String>, piece: &str| {
        if let Some(name) = pending.take() {
            if piece.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                body.push_str(&format!("${{{name}}}"));
            } else {
                body.push('$');
                body.push_str(&name);
            }
        }
        body.push_str(piece);
    };
    for o in &operands {
        let t = &text[o.byte_range()];
        match o.kind() {
            "string_literal" => push(&mut body, &mut pending, &t[1..t.len() - 1]),
            "simple_identifier" => {
                push(&mut body, &mut pending, "");
                pending = Some(t.to_owned());
            }
            "integer_literal" | "boolean_literal" if !t.contains(['_', 'x', 'X', 'b', 'B']) => push(&mut body, &mut pending, t),
            "null" => push(&mut body, &mut pending, "null"),
            "character_literal" => {
                let inner = &t[1..t.len() - 1];
                let piece = match inner {
                    "\"" => "\\\"",
                    "$" => "\\$",
                    "\\'" => "'",
                    other => other,
                };
                push(&mut body, &mut pending, piece);
            }
            _ => push(&mut body, &mut pending, &format!("${{{t}}}")),
        }
    }
    push(&mut body, &mut pending, "");
    let replacement = format!("\"{body}\"");
    Some(Intention { title: "Convert concatenation to template".into(), range: chain.byte_range(), caret: chain.start_byte() + replacement.len(), replacement })
}

/// Whether an expression needs parentheses as an operand of `+`.
fn needs_parens(node: Node) -> bool {
    !matches!(
        node.kind(),
        "simple_identifier" | "call_expression" | "navigation_expression" | "parenthesized_expression" | "string_literal" | "integer_literal" | "real_literal" | "boolean_literal"
            | "character_literal" | "indexing_expression" | "this_expression" | "postfix_expression" | "multiplicative_expression"
    )
}

/// `"a${x}b"` → `"a" + x + "b"`.
fn to_concatenation(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let literal = ancestors(innermost(root, caret)?).find(|n| n.kind() == "string_literal")?;
    if !plain_string(literal, text) {
        return None;
    }
    let mut cursor = literal.walk();
    let entries: Vec<Node> = literal.children(&mut cursor).collect();
    if !entries.iter().any(|e| matches!(e.kind(), "interpolated_identifier" | "interpolated_expression")) {
        return None;
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut literal_run = String::new();
    let mut literal_first = None;
    for e in &entries {
        match e.kind() {
            "interpolated_identifier" | "interpolated_expression" => {
                if !literal_run.is_empty() {
                    pieces.push(format!("\"{literal_run}\""));
                    literal_run.clear();
                }
                literal_first.get_or_insert(false);
                let t = &text[e.byte_range()];
                let inner = if e.kind() == "interpolated_expression" { e.named_child(0) } else { None };
                pieces.push(if inner.is_some_and(needs_parens) { format!("({t})") } else { t.to_owned() });
            }
            "interpolation_identifier_start" | "interpolation_expression_start" | "interpolation_expression_end" | "\"" => {}
            _ => {
                literal_first.get_or_insert(true);
                literal_run.push_str(&text[e.byte_range()]);
            }
        }
    }
    if !literal_run.is_empty() {
        pieces.push(format!("\"{literal_run}\""));
    }
    // Starting with a value: `"" +` keeps the `+` a string's.
    if literal_first == Some(false) && pieces.len() < 2 || literal_first == Some(false) && !pieces[1].starts_with('"') {
        pieces.insert(0, "\"\"".into());
    }
    let replacement = pieces.join(" + ");
    Some(Intention { title: "Convert template to concatenated string".into(), range: literal.byte_range(), caret: literal.start_byte(), replacement })
}

/// A parameter's name, from its source ("x: Int = 1", "vararg xs: Int");
/// none for a vararg or a Java parameter ("int x").
fn parameter_name(param: &str) -> Option<String> {
    let (head, _) = param.split_once(':')?;
    let words: Vec<&str> = head.split_whitespace().filter(|w| !w.starts_with('@')).collect();
    if words.contains(&"vararg") {
        return None;
    }
    words.last().map(|w| w.trim_matches('`').to_owned())
}

/// Add names to call arguments: the call whose `(` is at `open`, its
/// positional arguments named after `params` (the chosen declaration's).
pub fn add_argument_names(text: &str, open: usize, params: &[String]) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let args = innermost(tree.root_node(), open).and_then(|n| ancestors(n).find(|a| a.kind() == "value_arguments" && a.start_byte() == open))?;
    let arguments: Vec<Node> = children(args).into_iter().filter(|a| a.kind() == "value_argument").collect();
    let mut edits: Vec<(usize, String)> = Vec::new();
    let mut seen_named = false;
    for (i, a) in arguments.iter().enumerate() {
        let named = a.named_child(0).is_some_and(|c| c.kind() == "simple_identifier" && c.next_sibling().is_some_and(|s| s.kind() == "="));
        if named {
            seen_named = true;
            continue;
        }
        // A positional argument after a named one, or a spread: leave it.
        if seen_named || text[a.byte_range()].starts_with('*') {
            return None;
        }
        let name = parameter_name(params.get(i)?)?;
        edits.push((a.start_byte(), format!("{name} = ")));
    }
    if edits.is_empty() {
        return None;
    }
    let range = args.byte_range();
    let mut replacement = text[range.clone()].to_owned();
    for (at, insert) in edits.iter().rev() {
        replacement.insert_str(at - range.start, insert);
    }
    Some(Intention { title: "Add names to call arguments".into(), caret: range.start, range, replacement })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        let mut out = text.to_owned();
        out.replace_range(i.range.clone(), &i.replacement);
        out
    }

    fn find(text: &str, at: &str, title: &str) -> Option<String> {
        let caret = text.find(at).unwrap();
        intentions(text, caret).into_iter().find(|i| i.title == title).map(|i| apply(text, &i))
    }

    #[test]
    fn converts_concatenation_to_a_template() {
        let text = "fun f(x: Int, name: String) {\n    val a = \"a\" + x + \"b\\n\" + name.length + 'c' + 1 + name + \"!\"\n}\n";
        assert_eq!(
            find(text, "+ x", "Convert concatenation to template").unwrap(),
            "fun f(x: Int, name: String) {\n    val a = \"a${x}b\\n${name.length}c1$name!\"\n}\n"
        );
        // A value first: maybe a number's `+`.
        let text = "fun f(x: Int) = x + \"a\"\n";
        assert_eq!(find(text, "+", "Convert concatenation to template"), None);
        let text = "fun f(x: Int) = \"$\" + '$' + x + '\"'\n";
        assert_eq!(find(text, "+ x", "Convert concatenation to template").unwrap(), "fun f(x: Int) = \"$\\$$x\\\"\"\n");
    }

    #[test]
    fn converts_a_template_to_concatenation() {
        let text = "fun f(x: Int, n: String) = \"x=$x, n=${n.length} \\$ ${x + 1}\"\n";
        assert_eq!(
            find(text, "x=", "Convert template to concatenated string").unwrap(),
            "fun f(x: Int, n: String) = \"x=\" + x + \", n=\" + n.length + \" \\$ \" + (x + 1)\n"
        );
        let text = "fun f(x: Int) = \"$x\"\n";
        assert_eq!(find(text, "$x", "Convert template to concatenated string").unwrap(), "fun f(x: Int) = \"\" + x\n");
        let text = "val s = \"plain\"\n";
        assert_eq!(find(text, "plain", "Convert template to concatenated string"), None);
    }

    #[test]
    fn specifies_types() {
        let text = "fun f() {\n    val n = 1\n    val s = \"a\" + n\n    val p = Point(1, 2)\n    val q: Int = 2\n    val u = g()\n}\n";
        assert_eq!(find(text, "n = 1", "Specify type explicitly").unwrap(), text.replace("val n = 1", "val n: Int = 1"));
        assert_eq!(find(text, "s =", "Specify type explicitly").unwrap(), text.replace("val s =", "val s: String ="));
        assert_eq!(find(text, "p =", "Specify type explicitly").unwrap(), text.replace("val p =", "val p: Point ="));
        assert_eq!(find(text, "q:", "Specify type explicitly"), None);
        assert_eq!(find(text, "u =", "Specify type explicitly"), None);
        let text = "fun twice(x: Int) = x * 2\n";
        assert_eq!(find(text, "twice", "Specify return type explicitly").unwrap(), "fun twice(x: Int): Int = x * 2\n");
        let text = "fun twice(x: Int): Int = x * 2\n";
        assert_eq!(find(text, "twice", "Specify return type explicitly"), None);        // From the declarations of the calls, with their imports.
        let user = "package com.model\n\nclass User {\n    fun address(): Address = Address()\n}\n\nclass Address\n";
        let classes = |name: &str| -> Vec<String> { if name == "User" || name == "Address" { vec![user.to_owned()] } else { Vec::new() } };
        let text = "package app\n\nimport com.model.User\n\nfun f(user: User) {\n    val a = user.address()\n    val n = 1\n}\n\nfun g(user: User) = user.address()\n";
        let at = |s: &str| text.find(s).unwrap();
        let (i, imports) = specify_inferred_type(text, at("a ="), &classes).unwrap();
        assert_eq!((i.title.as_str(), i.replacement.as_str(), imports), ("Specify type explicitly", ": Address", vec!["com.model.Address".to_owned()]));
        let (i, _) = specify_inferred_type(text, at("g(user"), &classes).unwrap();
        assert_eq!((i.title.as_str(), i.range.start), ("Specify return type explicitly", at(") = user") + 1));
        // What the expression shows is offered already.
        assert_eq!(specify_inferred_type(text, at("n = 1"), &classes), None);
    }

    #[test]
    fn iterates_over_a_collection() {
        let user = "package com.model\n\nclass User\n\nclass Repo {\n    fun all(): List<User> = emptyList()\n}\n";
        let classes = |name: &str| -> Vec<String> { if name == "User" || name == "Repo" { vec![user.to_owned()] } else { Vec::new() } };
        let text = "fun f(repo: Repo, users: List<User>, names: Array<String>, word: String, n: Int) {\n    users\n    repo.all()\n    names\n    word\n    n\n    val x = users\n}\n";
        let at = |s: &str| text.find(s).unwrap();
        let i = iterate_over(text, at("users\n"), &classes).unwrap();
        assert_eq!(i.title, "Iterate over 'List<User>'");
        let out = apply(text, &i);
        assert!(out.contains("    for (user in users) {\n        \n    }\n    repo.all()"), "{out}");
        assert_eq!(&out[i.caret - 8..i.caret], "        ");
        // At the end of the statement too; named after the element type.
        let i = iterate_over(text, at("repo.all()") + "repo.all()".len(), &classes).unwrap();
        assert!(apply(text, &i).contains("for (user in repo.all()) {"));
        assert!(apply(text, &iterate_over(text, at("names\n"), &classes).unwrap()).contains("for (name in names) {"));
        assert!(apply(text, &iterate_over(text, at("word\n"), &classes).unwrap()).contains("for (c in word) {"));
        // Not an Int, not inside the statement, not a declaration.
        assert_eq!(iterate_over(text, at("n\n    val"), &classes), None);
        assert_eq!(iterate_over(text, at("all()"), &classes), None);
        assert_eq!(iterate_over(text, at("val x"), &classes), None);
    }

    #[test]
    fn adds_and_removes_loop_indices() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(list: List<String>, m: Map<String, Int>, a: IntArray) {\n    for (x in list) { println(x) }\n    for (e in m) {}\n    for (n in a) { val index = n }\n}\n";
        let at = |s: &str| text.find(s).unwrap();
        let i = add_loop_indices(text, at("x in"), &classes).unwrap();
        assert_eq!(i.title, "Add indices to 'for' loop");
        let out = apply(text, &i);
        assert!(out.contains("for ((index, x) in list.withIndex()) { println(x) }"), "{out}");
        assert_eq!(&out[i.caret - 5..i.caret], "index");
        // Not a map's; a name the loop uses already gets a number; not in the body.
        assert_eq!(add_loop_indices(text, at("e in"), &classes), None);
        assert!(apply(text, &add_loop_indices(text, at("n in"), &classes).unwrap()).contains("for ((index1, n) in a.withIndex())"));
        assert_eq!(add_loop_indices(text, at("println"), &classes), None);
        // Back, when the index isn't used.
        assert_eq!(find(&out, "index,", "Remove indices in 'for' loop").unwrap(), text);
        let used = "fun f(list: List<String>) {\n    for ((i, x) in list.withIndex()) { println(i) }\n}\n";
        assert_eq!(find(used, "i,", "Remove indices in 'for' loop"), None);
    }

    #[test]
    fn replaces_for_each_with_an_indexed_loop() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "import java.util.List;\n\nclass A {\n    void f(List<String> names, int[] ns, Iterable<String> it) {\n        for (String s : names) {\n            System.out.println(s);\n        }\n        for (int x : ns) total(x, i);\n        for (String t : it) {}\n    }\n}\n";
        let at = |s: &str| text.find(s).unwrap();
        let i = indexed_for_loop(text, at("s : names"), &classes).unwrap();
        assert_eq!(i.title, "Replace 'for each' loop with indexed 'for' loop");
        assert!(apply(text, &i).contains("        for (int i = 0; i < names.size(); i++) {\n            String s = names.get(i);\n            System.out.println(s);\n        }\n"), "{}", apply(text, &i));
        // An array; `i` is taken; a statement body gets braces.
        let out = apply(text, &indexed_for_loop(text, at("x : ns"), &classes).unwrap());
        assert!(out.contains("        for (int j = 0; j < ns.length; j++) {\n            int x = ns[j];\n            total(x, i);\n        }\n"), "{out}");
        // Not an Iterable; not in the body.
        assert_eq!(indexed_for_loop(text, at("t : it"), &classes), None);
        assert_eq!(indexed_for_loop(text, at("println"), &classes), None);
    }

    #[test]
    fn adds_names_to_call_arguments() {
        let text = "fun f() {\n    g(1, \"a\", flag = true)\n}\n";
        let open = text.find('(').unwrap();
        let open = text[open + 1..].find('(').unwrap() + open + 1;
        let params = vec!["count: Int".to_owned(), "label: String = \"\"".into(), "flag: Boolean".into()];
        let i = add_argument_names(text, open, &params).unwrap();
        assert_eq!(apply(text, &i), "fun f() {\n    g(count = 1, label = \"a\", flag = true)\n}\n");
        // Java parameters and varargs have no names to use.
        assert_eq!(add_argument_names(text, open, &["int count".to_owned(), "String label".into()]), None);
        assert_eq!(add_argument_names(text, open, &["vararg xs: Int".to_owned()]), None);
    }
}

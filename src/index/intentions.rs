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
    out.extend(to_for_each(root, text, caret));
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
        if let Some(s) = singular.filter(|s| !s.is_empty()) {
            let mut chars = s.chars();
            let first = chars.next().unwrap_or('e').to_lowercase().collect::<String>();
            return first + chars.as_str();
        }
    }
    let base = element.split('<').next().unwrap_or(element).trim_end_matches('?');
    let base = base.rsplit('.').next().unwrap_or(base);
    match base {
        "Int" | "int" | "Integer" => "i".into(),
        "Long" | "long" => "l".into(),
        "Short" | "short" | "String" => "s".into(),
        "Byte" | "byte" | "Boolean" | "boolean" => "b".into(),
        "Char" | "char" | "Character" => "c".into(),
        "Double" | "double" => "d".into(),
        "Float" | "float" => "f".into(),
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
    let expression = receiver(iterated, text);
    let middle = &text[variable.end_byte()..iterated.start_byte()];
    let start = variable.start_byte();
    Some(Intention {
        title: "Add indices to 'for' loop".into(),
        range: start..iterated.end_byte(),
        replacement: format!("({index}, {}){middle}{expression}.withIndex()", &text[variable.byte_range()]),
        caret: start + 1 + index.len(),
    })
}

/// The expression as the receiver of a call: `(1..10)`, `list`.
fn receiver(node: Node, text: &str) -> String {
    let expression = &text[node.byte_range()];
    if matches!(node.kind(), "simple_identifier" | "navigation_expression" | "call_expression" | "parenthesized_expression" | "this_expression" | "indexing_expression") {
        expression.to_owned()
    } else {
        format!("({expression})")
    }
}

/// The `break` / `continue` in `node` that leave or repeat the loop
/// labelled `label` (or unlabelled, outside `nested` loops and lambdas).
fn loop_jumps<'t>(node: Node<'t>, text: &str, label: Option<&str>, nested: bool, out: &mut Vec<Node<'t>>) {
    for child in children(node) {
        if child.kind() == "jump_expression" {
            let t = &text[child.byte_range()];
            if t.starts_with("break") || t.starts_with("continue") {
                let target = t.split_once('@').map(|(_, l)| l.trim());
                if target.is_none() && !nested || target.is_some() && target == label {
                    out.push(child);
                }
            }
        }
        let inner = nested || matches!(child.kind(), "for_statement" | "while_statement" | "do_while_statement" | "lambda_literal" | "anonymous_function" | "function_declaration" | "class_declaration" | "object_literal");
        loop_jumps(child, text, label, inner, out);
    }
}

/// `text[range]` with the ranges in `edits` replaced by `with`.
fn replaced(text: &str, range: std::ops::Range<usize>, edits: &[Node], with: &str) -> String {
    let mut out = String::new();
    let mut at = range.start;
    for edit in edits {
        out.push_str(&text[at..edit.start_byte()]);
        out.push_str(with);
        at = edit.end_byte();
    }
    out.push_str(&text[at..range.end]);
    out
}

/// Replace with a 'forEach' function call (IntelliJ's
/// ConvertToForEachFunctionCallIntention): `for (x in xs) { … }` →
/// `xs.forEach { x -> … }`, the loop's `continue` becoming
/// `return@forEach`; not when the loop has a `break`.
fn to_for_each(root: Node, text: &str, caret: usize) -> Option<Intention> {
    let (variable, iterated, statement, body) = for_header(root, caret)?;
    let body = body?;
    let label = statement.prev_named_sibling().filter(|l| l.kind() == "label" && text[l.end_byte()..statement.start_byte()].trim().is_empty());
    let name = label.map(|l| text[l.byte_range()].trim().trim_end_matches('@'));
    let mut jumps = Vec::new();
    loop_jumps(body, text, name, false, &mut jumps);
    if jumps.iter().any(|j| text[j.byte_range()].starts_with("break")) {
        return None;
    }
    let block = &text[body.byte_range()];
    let inner = if block.starts_with('{') && block.ends_with('}') {
        replaced(text, body.start_byte() + 1..body.end_byte() - 1, &jumps, "return@forEach")
    } else {
        format!(" {} ", replaced(text, body.byte_range(), &jumps, "return@forEach"))
    };
    let inner = if inner.trim().is_empty() { " ".to_owned() } else { inner };
    let start = label.map_or(statement.start_byte(), |l| l.start_byte());
    Some(Intention {
        title: "Replace with a 'forEach' function call".into(),
        range: start..statement.end_byte(),
        replacement: format!("{}.forEach {{ {} ->{inner}}}", receiver(iterated, text), &text[variable.byte_range()]),
        caret: start,
    })
}

/// Replace with a 'for' loop (IntelliJ's ConvertForEachToForLoopIntention):
/// the statement `xs.forEach { x -> … }` (`it` when the lambda names no
/// parameter) → `for (x in xs) { … }`, its `return@forEach` becoming
/// `continue`, when `xs` is a collection, array, sequence, map or string.
pub fn for_each_to_loop(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let call = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "call_expression")?;
    if call.parent()?.kind() != "statements" {
        return None;
    }
    let [callee, suffix] = children(call).into_iter().filter(|c| c.is_named()).collect::<Vec<_>>()[..] else { return None };
    let [annotated] = children(suffix).into_iter().filter(|c| c.is_named()).collect::<Vec<_>>()[..] else { return None };
    let [lambda] = children(annotated).into_iter().filter(|c| c.is_named()).collect::<Vec<_>>()[..] else { return None };
    if callee.kind() != "navigation_expression" || lambda.kind() != "lambda_literal" {
        return None;
    }
    let target = callee.named_child(0)?;
    let name = callee.named_child(1)?;
    if text[name.byte_range()].trim() != ".forEach" {
        return None;
    }
    let typed = if target.kind() == "parenthesized_expression" { target.named_child(0)? } else { target };
    // `1..3`, `0 until n`: a range.
    let range = typed.kind() == "range_expression" || typed.kind() == "infix_expression" && [" until ", " downTo "].iter().any(|w| text[typed.byte_range()].contains(w));
    let ty = super::introduce::kotlin_type(typed, text)
        .filter(|t| !t.is_empty())
        .or_else(|| range.then(|| "IntRange".to_owned()))
        .or_else(|| super::types::type_of(Lang::Kotlin, text, typed.byte_range(), classes).map(|t| t.ty))?;
    element_type(&ty)?;
    let mut cursor = lambda.walk();
    let parts: Vec<Node> = lambda.children(&mut cursor).collect();
    let parameters = parts.iter().find(|c| c.kind() == "lambda_parameters");
    if parameters.is_some_and(|p| p.named_child_count() != 1) {
        return None;
    }
    let variable = parameters.map_or("it", |p| &text[p.byte_range()]);
    let open = parts.iter().find(|c| c.kind() == "->").or(parts.first())?.end_byte();
    let close = lambda.end_byte() - 1;
    // `return@forEach` becomes `continue`; not inside a loop or lambda of
    // its own, where it would mean something else.
    let mut returns = Vec::new();
    let mut misplaced = false;
    fn walk<'t>(node: Node<'t>, text: &str, nested: bool, returns: &mut Vec<Node<'t>>, misplaced: &mut bool) {
        for child in children(node) {
            if child.kind() == "jump_expression" && text[child.byte_range()].starts_with("return@forEach") {
                if nested || text[child.byte_range()].trim() != "return@forEach" {
                    *misplaced = true;
                }
                returns.push(child);
            }
            let inner = nested || matches!(child.kind(), "for_statement" | "while_statement" | "do_while_statement" | "lambda_literal" | "anonymous_function" | "function_declaration" | "class_declaration" | "object_literal");
            walk(child, text, inner, returns, misplaced);
        }
    }
    walk(lambda, text, false, &mut returns, &mut misplaced);
    if misplaced {
        return None;
    }
    let inner = replaced(text, open..close, &returns, "continue");
    let inner = if inner.trim().is_empty() { " ".to_owned() } else { inner };
    let range_text = &text[target.byte_range()];
    let range_text = if target.kind() == "parenthesized_expression" { range_text[1..range_text.len() - 1].trim() } else { range_text };
    Some(Intention {
        title: "Replace with a 'for' loop".into(),
        range: call.byte_range(),
        replacement: format!("for ({variable} in {range_text}) {{{inner}}}"),
        caret: call.start_byte(),
    })
}

/// The call `x.name(…)` whose `.name` holds `caret`, with `x`, `name`
/// and the arguments; None for `?.`, a trailing lambda, named or spread
/// arguments.
fn method_call<'t>(root: Node<'t>, text: &'t str, caret: usize) -> Option<(Node<'t>, Node<'t>, &'t str, Vec<Node<'t>>)> {
    let call = innermost(root, caret).into_iter().flat_map(ancestors).find(|n| {
        n.kind() == "call_expression" && n.named_child(0).is_some_and(|c| c.kind() == "navigation_expression" && c.named_child(1).is_some_and(|s| s.start_byte() <= caret))
            && n.named_child(1).is_some_and(|s| s.start_byte() >= caret)
    })?;
    let callee = call.named_child(0)?;
    let target = callee.named_child(0)?;
    let name = text[callee.named_child(1)?.byte_range()].trim().strip_prefix('.')?;
    let [arguments] = children(call.named_child(1)?)[..] else { return None };
    if arguments.kind() != "value_arguments" {
        return None;
    }
    let values = children(arguments);
    let plain = values.iter().all(|v| {
        let mut cursor = v.walk();
        let tokens: Vec<Node> = v.children(&mut cursor).collect();
        tokens.len() == 1 && tokens[0].is_named()
    });
    plain.then_some((call, target, name, values))
}

/// The type of the receiver `node`, from what it shows or the
/// declarations of its calls.
fn receiver_type(node: Node, text: &str, classes: super::catches::Classes) -> Option<String> {
    super::introduce::kotlin_type(node, text)
        .filter(|t| !t.is_empty())
        .or_else(|| super::types::type_of(Lang::Kotlin, text, node.byte_range(), classes).map(|t| t.ty))
}

/// `List` for `kotlin.collections.List<String>?`.
fn type_head(ty: &str) -> &str {
    let head = ty.split('<').next().unwrap_or(ty).trim_end_matches('?');
    head.rsplit('.').next().unwrap_or(head)
}

/// Replace with '==' / 'in' / '!in' / '+' … (IntelliJ's
/// ReplaceCallWithBinaryOperatorInspection and its 'contains' twin): the
/// caret on the name, `a.equals(b)` → `a == b` (`!a.equals(b)` → `a != b`),
/// `xs.contains(x)` → `x in xs` (`!…` → `x !in xs`), `a.plus(b)` → `a + b`
/// and `minus` / `times` / `div` / `rem` likewise, when the receiver's type
/// has those as operators: not nullable; `equals` not on Double / Float
/// (`==` differs for NaN and -0.0); `contains` on collections, maps,
/// arrays, ranges and strings; arithmetic on numbers, `plus` on strings,
/// `plus` / `minus` on collections.
pub fn binary_operator(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
    let [argument] = values[..] else { return None };
    let ty = receiver_type(target, text, classes)?;
    if ty.ends_with('?') {
        return None;
    }
    let head = type_head(&ty);
    let number = matches!(head, "Int" | "Long" | "Short" | "Byte" | "Double" | "Float" | "BigDecimal" | "BigInteger");
    let collection = matches!(head, "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "Iterable" | "Sequence");
    let negated = call.parent().filter(|p| p.kind() == "prefix_expression" && text[p.byte_range()].starts_with('!'));
    let operator = match name {
        "equals" if !matches!(head, "Double" | "Float") => if negated.is_some() { "!=" } else { "==" },
        "contains" if collection || matches!(head, "Map" | "MutableMap" | "HashMap" | "LinkedHashMap" | "String" | "CharSequence" | "IntRange" | "LongRange" | "CharRange") || element_type(&ty).is_some() && head.ends_with("Array") => {
            if negated.is_some() { "!in" } else { "in" }
        }
        "plus" if number || collection || matches!(head, "String" | "Map" | "MutableMap" | "Array") => "+",
        "minus" if number || collection || matches!(head, "Map" | "MutableMap") => "-",
        "times" if number => "*",
        "div" if number => "/",
        "rem" if number => "%",
        _ => return None,
    };
    let whole = if matches!(operator, "!=" | "!in") { negated? } else { call };
    let operand = |n: Node| {
        let t = &text[n.byte_range()];
        let primary = matches!(
            n.kind(),
            "simple_identifier" | "integer_literal" | "long_literal" | "real_literal" | "string_literal" | "character_literal" | "boolean_literal" | "call_expression" | "navigation_expression" | "parenthesized_expression" | "indexing_expression" | "this_expression"
        ) || t == "null";
        if primary { t.to_owned() } else { format!("({t})") }
    };
    let (left, right) = (operand(target), operand(argument.named_child(0)?));
    let expression = if operator.ends_with("in") { format!("{right} {operator} {left}") } else { format!("{left} {operator} {right}") };
    let alone = whole.parent().is_some_and(|p| {
        matches!(
            p.kind(),
            "statements" | "property_declaration" | "value_argument" | "assignment" | "parenthesized_expression" | "jump_expression" | "control_structure_body" | "function_body" | "if_expression" | "while_statement" | "do_while_statement" | "when_condition" | "conjunction_expression" | "disjunction_expression"
        )
    });
    Some(Intention {
        title: format!("Replace with '{operator}'"),
        range: whole.byte_range(),
        caret: whole.start_byte(),
        replacement: if alone { expression } else { format!("({expression})") },
    })
}

/// Replace with 'orEmpty()' (the fix of IntelliJ's Kotlin inspection for
/// an elvis with an empty value): `s ?: ""` → `s.orEmpty()`, and
/// `?: emptyList()` / `emptySet()` / `emptyMap()` / `emptySequence()` /
/// `emptyArray()` likewise, when the left side's type is the nullable
/// String / List / Set / Map / Sequence / Array.
pub fn or_empty(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let elvis = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "elvis_expression" && n.named_child_count() == 2)?;
    let (left, right) = (elvis.named_child(0)?, elvis.named_child(1)?);
    let wanted = match text[right.byte_range()].trim() {
        "\"\"" => "String",
        "emptyList()" => "List",
        "emptySet()" => "Set",
        "emptyMap()" => "Map",
        "emptySequence()" => "Sequence",
        "emptyArray()" => "Array",
        _ => return None,
    };
    let ty = receiver_type(left, text, classes)?;
    if !ty.ends_with('?') || type_head(&ty) != wanted {
        return None;
    }
    let receiver = &text[left.byte_range()];
    let primary = matches!(left.kind(), "simple_identifier" | "call_expression" | "navigation_expression" | "parenthesized_expression" | "indexing_expression" | "this_expression");
    let receiver = if primary { receiver.to_owned() } else { format!("({receiver})") };
    Some(Intention { title: "Replace with 'orEmpty()'".into(), range: elvis.byte_range(), caret: elvis.start_byte(), replacement: format!("{receiver}.orEmpty()") })
}

/// Replace negated 'isEmpty' with 'isNotEmpty' (IntelliJ's
/// ReplaceNegatedIsEmptyWithIsNotEmptyInspection): `!xs.isEmpty()` →
/// `xs.isNotEmpty()`, and `isNotEmpty` / `isBlank` / `isNotBlank` the other
/// way, when `xs` is a collection, map, array or string (only strings for
/// the blank checks).
pub fn negated_emptiness(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let negation = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "prefix_expression" && text[n.byte_range()].starts_with('!'))?;
    let call = children(negation).into_iter().last()?;
    let [callee, suffix] = children(call)[..] else { return None };
    if call.kind() != "call_expression" || callee.kind() != "navigation_expression" || text[suffix.byte_range()].replace(' ', "") != "()" {
        return None;
    }
    let target = callee.named_child(0)?;
    let name = text[callee.named_child(1)?.byte_range()].trim().strip_prefix('.')?;
    let opposite = match name {
        "isEmpty" => "isNotEmpty",
        "isNotEmpty" => "isEmpty",
        "isBlank" => "isNotBlank",
        "isNotBlank" => "isBlank",
        _ => return None,
    };
    let ty = receiver_type(target, text, classes)?;
    let head = type_head(&ty);
    let string = matches!(head, "String" | "CharSequence" | "StringBuilder");
    let sized = string
        || matches!(head, "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "MutableCollection" | "Map" | "MutableMap" | "HashMap" | "LinkedHashMap")
        || head.ends_with("Array");
    if ty.ends_with('?') || !(if name.contains("Blank") { string } else { sized }) {
        return None;
    }
    let short = |n: &str| n.trim_end_matches("()").to_owned();
    Some(Intention {
        title: format!("Replace negated '{}' with '{}'", short(name), short(opposite)),
        range: negation.byte_range(),
        caret: negation.start_byte(),
        replacement: format!("{}.{opposite}()", &text[target.byte_range()]),
    })
}

/// Replace with 'isNullOrEmpty()' (IntelliJ's
/// ReplaceIsEmptyWithIsNullOrEmpty twin): `s == null || s.isEmpty()` →
/// `s.isNullOrEmpty()`, `s != null && s.isNotEmpty()` →
/// `!s.isNullOrEmpty()`, and `isBlank` / `isNotBlank` → `isNullOrBlank`,
/// when `s` is a nullable string, collection, map or array; the caret
/// anywhere on the check. The null check may end a longer `||` / `&&`.
pub fn null_or_empty(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let operator = |left: Node, right: Node| text[left.end_byte()..right.start_byte()].trim().to_owned();
    for node in innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
    {
        let (or, wanted) = match node.kind() {
            "disjunction_expression" => (true, "=="),
            "conjunction_expression" => (false, "!="),
            _ => continue,
        };
        let [left, call] = children(node)[..] else { continue };
        // `a || s == null || s.isEmpty()`: the null check is the left's last operand.
        let check = if left.kind() == node.kind() { *children(left).last()? } else { left };
        let [a, b] = children(check)[..] else { continue };
        if check.kind() != "equality_expression" || operator(a, b) != wanted {
            continue;
        }
        let subject = match (text[a.byte_range()].trim(), text[b.byte_range()].trim()) {
            ("null", s) | (s, "null") if s != "null" => s,
            _ => continue,
        };
        let [callee, suffix] = children(call)[..] else { continue };
        if call.kind() != "call_expression" || callee.kind() != "navigation_expression" || text[suffix.byte_range()].replace(' ', "") != "()" {
            continue;
        }
        let target = callee.named_child(0)?;
        let name = text[callee.named_child(1)?.byte_range()].trim().strip_prefix('.')?;
        let replaced = match (or, name) {
            (true, "isEmpty") | (false, "isNotEmpty") => "isNullOrEmpty",
            (true, "isBlank") | (false, "isNotBlank") => "isNullOrBlank",
            _ => continue,
        };
        if text[target.byte_range()].trim() != subject {
            continue;
        }
        let ty = receiver_type(target, text, classes).or_else(|| receiver_type(if subject == text[a.byte_range()].trim() { a } else { b }, text, classes))?;
        let head = type_head(&ty);
        let string = matches!(head, "String" | "CharSequence");
        let fits = if replaced == "isNullOrBlank" {
            string
        } else {
            string || matches!(head, "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "MutableCollection" | "Map" | "MutableMap" | "HashMap" | "LinkedHashMap" | "Array")
        };
        if !ty.ends_with('?') || !fits {
            continue;
        }
        return Some(Intention {
            title: format!("Replace with '{replaced}()'"),
            range: check.start_byte()..call.end_byte(),
            caret: check.start_byte(),
            replacement: format!("{}{subject}.{replaced}()", if or { "" } else { "!" }),
        });
    }
    None
}

/// Convert two comparisons to range check (IntelliJ's
/// ConvertTwoComparisonsToRangeCheckInspection): `x >= a && x <= b` →
/// `x in a..b` (either order, either side of each comparison), and
/// `x >= a && x < b` → `x in a until b` for whole numbers and chars, when
/// `x` is a non-null number or char; the caret anywhere on the check.
pub fn range_check(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let check = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "conjunction_expression")?;
    let [first, second] = children(check)[..] else { return None };
    // A comparison as (subject, bound, is a lower bound, inclusive).
    fn bound<'t>(c: Node<'t>, text: &str) -> Option<(Node<'t>, Node<'t>, bool, bool)> {
        let [a, b] = children(c)[..] else { return None };
        if c.kind() != "comparison_expression" {
            return None;
        }
        let (flipped, inclusive) = match text[a.end_byte()..b.start_byte()].trim() {
            ">=" => (false, true),
            "<=" => (true, true),
            ">" => (false, false),
            "<" => (true, false),
            _ => return None,
        };
        // `x >= a`: x is the subject on the left and a is its lower bound.
        Some((a, b, !flipped, inclusive))
    }
    // A comparison read either way round: (subject, bound, lower, inclusive).
    fn ways<'t>(a: Node<'t>, b: Node<'t>, lower: bool, inclusive: bool) -> [(Node<'t>, Node<'t>, bool, bool); 2] {
        [(a, b, lower, inclusive), (b, a, !lower, inclusive)]
    }
    let (a0, b0, lower0, inclusive0) = bound(first, text)?;
    let (a1, b1, lower1, inclusive1) = bound(second, text)?;
    let same = |x: Node, y: Node| text[x.byte_range()].trim() == text[y.byte_range()].trim();
    let (subject, low, high) = ways(a0, b0, lower0, inclusive0).into_iter().find_map(|(s0, x0, l0, i0)| {
        let (_, x1, _, i1) = ways(a1, b1, lower1, inclusive1).into_iter().find(|&(s1, _, l1, _)| same(s0, s1) && l1 != l0)?;
        Some(if l0 { (s0, (x0, i0), (x1, i1)) } else { (s0, (x1, i1), (x0, i0)) })
    })?;
    if !low.1 || subject.kind() == "integer_literal" {
        return None;
    }
    let ty = receiver_type(subject, text, classes)?;
    let whole = matches!(ty.as_str(), "Int" | "Long" | "Short" | "Byte" | "Char");
    if !(whole || matches!(ty.as_str(), "Double" | "Float")) || !high.1 && !whole {
        return None;
    }
    let operand = |n: Node| {
        let t = text[n.byte_range()].trim();
        if matches!(n.kind(), "simple_identifier" | "integer_literal" | "character_literal" | "real_literal" | "call_expression" | "navigation_expression" | "parenthesized_expression") { t.to_owned() } else { format!("({t})") }
    };
    let range = if high.1 { format!("{}..{}", operand(low.0), operand(high.0)) } else { format!("{} until {}", operand(low.0), operand(high.0)) };
    Some(Intention {
        title: "Convert two comparisons to range check".into(),
        range: check.byte_range(),
        caret: check.start_byte(),
        replacement: format!("{} in {range}", text[subject.byte_range()].trim()),
    })
}

/// Replace 'associate' with 'associateWith' / 'associateBy' (IntelliJ's
/// ReplaceAssociateFunctionInspection): `xs.associate { it to f(it) }` →
/// `xs.associateWith { f(it) }`, `xs.associate { f(it) to it }` →
/// `xs.associateBy { f(it) }` (a named parameter `x ->` too), when `xs` is
/// a collection, array or sequence; the caret on the call, outside the lambda.
pub fn associate_function(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let call = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "call_expression")?;
    fn named(n: Node) -> Vec<Node> {
        children(n).into_iter().filter(|c| c.is_named()).collect()
    }
    let [callee, suffix] = named(call)[..] else { return None };
    let [annotated] = named(suffix)[..] else { return None };
    let [lambda] = named(annotated)[..] else { return None };
    if callee.kind() != "navigation_expression" || lambda.kind() != "lambda_literal" || text[callee.named_child(1)?.byte_range()].trim() != ".associate" {
        return None;
    }
    let parts = named(lambda);
    let parameters = parts.iter().find(|c| c.kind() == "lambda_parameters");
    if parameters.is_some_and(|p| p.named_child_count() != 1 || p.named_child(0).is_some_and(|v| v.kind() != "variable_declaration" || v.named_child_count() != 1)) {
        return None;
    }
    let variable = parameters.map_or("it", |p| text[p.byte_range()].trim());
    let [body] = named(*parts.iter().find(|c| c.kind() == "statements")?)[..] else { return None };
    let [key, to, value] = named(body)[..] else { return None };
    if body.kind() != "infix_expression" || &text[to.byte_range()] != "to" {
        return None;
    }
    let (function, kept) = if text[key.byte_range()].trim() == variable {
        ("associateWith", value)
    } else if text[value.byte_range()].trim() == variable {
        ("associateBy", key)
    } else {
        return None;
    };
    let target = callee.named_child(0)?;
    let ty = receiver_type(target, text, classes)?;
    if ty.ends_with('?') || element_type(&ty).is_none_or(|e| e.starts_with("Map.Entry")) {
        return None;
    }
    let head = parameters.map_or(String::new(), |p| format!("{} -> ", &text[p.byte_range()]));
    Some(Intention {
        title: format!("Replace 'associate' with '{function}'"),
        range: call.byte_range(),
        caret: call.start_byte(),
        replacement: format!("{}.{function} {{ {head}{} }}", &text[target.byte_range()], &text[kept.byte_range()]),
    })
}

/// Merge call chain to 'first' (IntelliJ's SimplifiableCallChainInspection):
/// `xs.filter { … }.first()` → `xs.first { … }`, and likewise `firstOrNull`,
/// `last`, `lastOrNull`, `single`, `singleOrNull`, `count`, `any`, `none`,
/// `isNotEmpty()` → `any`, `isEmpty()` → `none`, `map { … }.filterNotNull()`
/// → `mapNotNull { … }`, `map { }.sum()` → `sumOf { }`, `map { }.joinToString()`
/// → `joinToString { }`, `sortedBy { }.first()` → `minBy { }` (and
/// `last` → `maxBy`, `…OrNull`, `sortedByDescending` the other way), when `xs` is a collection, array, sequence or
/// string; the caret on the chain, outside the lambda.
pub fn merge_call_chain(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    // Right after the chain's `)` too.
    let calls = [caret, caret.saturating_sub(1)]
        .into_iter()
        .filter_map(|at| innermost(tree.root_node(), at))
        .flat_map(|n| ancestors(n).take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body")))
        .filter(|n| n.kind() == "call_expression");
    for outer in calls {
        let [callee, suffix] = children(outer)[..] else { continue };
        if callee.kind() != "navigation_expression" || text[suffix.byte_range()].replace(' ', "") != "()" {
            continue;
        }
        let Some(inner) = callee.named_child(0).filter(|n| n.kind() == "call_expression") else { continue };
        let second = text[callee.named_child(1)?.byte_range()].trim();
        let [inner_callee, inner_suffix] = children(inner)[..] else { continue };
        let [lambda] = children(inner_suffix)[..] else { continue };
        // `listOf(a, b).filterNotNull()` → `listOfNotNull(a, b)`.
        if inner_callee.kind() == "simple_identifier" && &text[inner_callee.byte_range()] == "listOf" && second == ".filterNotNull" && lambda.kind() == "value_arguments" {
            return Some(Intention {
                title: "Merge call chain to 'listOfNotNull'".into(),
                range: outer.byte_range(),
                caret: outer.start_byte(),
                replacement: format!("listOfNotNull{}", &text[lambda.byte_range()]),
            });
        }
        if inner_callee.kind() != "navigation_expression" {
            continue;
        }
        let first = text[inner_callee.named_child(1)?.byte_range()].trim();
        // `xs.sorted().first()` → `xs.min()`.
        if lambda.kind() == "value_arguments" && lambda.named_child_count() == 0 {
            let merged = match (first, second) {
                (".sorted", ".first") | (".sortedDescending", ".last") => "min",
                (".sorted", ".firstOrNull") | (".sortedDescending", ".lastOrNull") => "minOrNull",
                (".sorted", ".last") | (".sortedDescending", ".first") => "max",
                (".sorted", ".lastOrNull") | (".sortedDescending", ".firstOrNull") => "maxOrNull",
                _ => continue,
            };
            let target = inner_callee.named_child(0)?;
            let ty = receiver_type(target, text, classes)?;
            if ty.ends_with('?') || element_type(&ty).is_none_or(|e| e.starts_with("Map.Entry")) {
                return None;
            }
            return Some(Intention {
                title: format!("Merge call chain to '{merged}'"),
                range: outer.byte_range(),
                caret: outer.start_byte(),
                replacement: format!("{}.{merged}()", &text[target.byte_range()]),
            });
        }
        if lambda.kind() != "annotated_lambda" {
            continue;
        }
        let merged = match (first, second) {
            (".filter", ".first" | ".firstOrNull" | ".last" | ".lastOrNull" | ".single" | ".singleOrNull" | ".count" | ".any" | ".none") => &second[1..],
            (".filter", ".isNotEmpty") => "any",
            (".filter", ".isEmpty") => "none",
            (".map", ".filterNotNull") => "mapNotNull",
            (".map", ".sum") => "sumOf",
            (".map", ".joinToString") => "joinToString",
            (".map", ".toMap") => "associate",
            (".sortedBy", ".first") | (".sortedByDescending", ".last") => "minBy",
            (".sortedBy", ".firstOrNull") | (".sortedByDescending", ".lastOrNull") => "minByOrNull",
            (".sortedBy", ".last") | (".sortedByDescending", ".first") => "maxBy",
            (".sortedBy", ".lastOrNull") | (".sortedByDescending", ".firstOrNull") => "maxByOrNull",
            _ => continue,
        };
        let target = inner_callee.named_child(0)?;
        let ty = receiver_type(target, text, classes)?;
        if ty.ends_with('?') || element_type(&ty).is_none_or(|e| e.starts_with("Map.Entry")) {
            return None;
        }
        return Some(Intention {
            title: format!("Merge call chain to '{merged}'"),
            range: outer.byte_range(),
            caret: outer.start_byte(),
            replacement: format!("{}.{merged} {}", &text[target.byte_range()], &text[lambda.byte_range()]),
        });
    }
    None
}

/// Remove redundant call (Kotlin, IntelliJ's
/// RemoveRedundantCallsOfConversionMethodsInspection): `s.toString()` on a
/// String, `n.toInt()` on an Int, and the other conversions to the type the
/// receiver already has, become the receiver; Java's Remove redundant
/// 'toString()' call (RedundantStringOperationInspection) does the same for
/// `s.toString()` on a String. Not on a nullable receiver, whose
/// `toString()` gives "null".
pub fn redundant_conversion(text: &str, caret: usize, kotlin: bool, classes: super::catches::Classes) -> Option<Intention> {
    let (range, target, name) = if kotlin {
        let tree = symbols::parse(Lang::Kotlin, text)?;
        let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
        if !values.is_empty() {
            return None;
        }
        let ty = receiver_type(target, text, classes)?;
        let wanted = match name {
            "toString" => "String",
            "toInt" => "Int",
            "toLong" => "Long",
            "toShort" => "Short",
            "toByte" => "Byte",
            "toDouble" => "Double",
            "toFloat" => "Float",
            "toChar" => "Char",
            _ => return None,
        };
        if ty.ends_with('?') || type_head(&ty) != wanted {
            return None;
        }
        (call.byte_range(), target.byte_range(), "Remove redundant call".to_owned())
    } else {
        let tree = symbols::parse(Lang::Java, text)?;
        let call = ancestors(innermost(tree.root_node(), caret)?)
            .take_while(|n| !matches!(n.kind(), "block" | "class_body" | "expression_statement" | "lambda_expression"))
            .find(|n| n.kind() == "method_invocation" && n.child_by_field_name("name").is_some_and(|m| m.start_byte() <= caret && caret <= m.end_byte()))?;
        let object = call.child_by_field_name("object")?;
        let arguments = call.child_by_field_name("arguments")?;
        let string = |n: Node| java_type(n, text, classes).is_some_and(|t| t == "String" || t == "java.lang.String");
        match &text[call.child_by_field_name("name")?.byte_range()] {
            "toString" if arguments.named_child_count() == 0 && string(object) => (call.byte_range(), object.byte_range(), "Remove redundant 'toString()' call".to_owned()),
            // `String.valueOf(s)` of a string, or of anything but a `char[]`
            // added to a string.
            "valueOf" if &text[object.byte_range()] == "String" && arguments.named_child_count() == 1 => {
                let value = arguments.named_child(0)?;
                let parent = call.parent()?;
                let concatenated = parent.kind() == "binary_expression"
                    && parent.child_by_field_name("operator").is_some_and(|o| &text[o.byte_range()] == "+")
                    && [parent.child_by_field_name("left"), parent.child_by_field_name("right")].into_iter().flatten().find(|n| n.id() != call.id()).is_some_and(string)
                    && match java_type(value, text, classes) {
                        Some(t) => t != "char[]",
                        // An arithmetic expression or a literal isn't a `char[]`.
                        None => value.kind() == "binary_expression" || value.kind().ends_with("_literal"),
                    };
                if !(string(value) || concatenated) {
                    return None;
                }
                let title = "Remove redundant 'String.valueOf()' call".to_owned();
                // `"n=" + String.valueOf(a + b)` keeps the sum's parentheses.
                if concatenated && !string(value) && matches!(value.kind(), "binary_expression" | "ternary_expression" | "assignment_expression" | "lambda_expression" | "instanceof_expression") {
                    return Some(Intention { title, caret: call.start_byte(), replacement: format!("({})", &text[value.byte_range()]), range: call.byte_range() });
                }
                (call.byte_range(), value.byte_range(), title)
            }
            _ => return None,
        }
    };
    Some(Intention { title: name, caret: range.start, replacement: text[target].to_owned(), range })
}

/// Replace with indices (IntelliJ's ReplaceManualRangeWithIndicesCalls):
/// `0 until xs.size` / `0..xs.size - 1` → `xs.indices` (`length` for a
/// string), when `xs` is a collection, array or string.
pub fn range_to_indices(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let range = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| {
            n.kind() == "range_expression"
                || n.kind() == "infix_expression" && n.named_child(1).is_some_and(|m| &text[m.byte_range()] == "until")
        })?;
    let start = range.named_child(0)?;
    let end = *children(range).last()?;
    if &text[start.byte_range()] != "0" {
        return None;
    }
    // The size: `xs.size` after `until`, `xs.size - 1` after `..`.
    let size = if range.kind() == "range_expression" {
        let t = &text[end.byte_range()];
        let minus = t.strip_suffix('1').map(str::trim_end).and_then(|t| t.strip_suffix('-'))?;
        if end.kind() != "additive_expression" {
            return None;
        }
        let size = end.named_child(0)?;
        (minus.trim_end() == &text[size.byte_range()]).then_some(size)?
    } else {
        end
    };
    if size.kind() != "navigation_expression" {
        return None;
    }
    let target = size.named_child(0)?;
    let member = text[size.named_child(1)?.byte_range()].trim();
    let ty = receiver_type(target, text, classes)?;
    let head = type_head(&ty);
    let string = matches!(head, "String" | "CharSequence" | "StringBuilder");
    let fits = match member {
        ".size" => !string && element_type(&ty).is_some_and(|e| !e.starts_with("Map.Entry")) && !matches!(head, "Sequence" | "Iterable" | "IntRange" | "LongRange" | "CharRange"),
        ".length" => string,
        _ => false,
    };
    if ty.ends_with('?') || !fits {
        return None;
    }
    Some(Intention { title: "Replace with indices".into(), range: range.byte_range(), caret: range.start_byte(), replacement: format!("{}.indices", &text[target.byte_range()]) })
}

/// Replace with 'maxOf' function (IntelliJ's
/// ReplaceJavaStaticMethodWithKotlinAnalogInspection): `Math.max(a, b)` →
/// `maxOf(a, b)` (`min` → `minOf`), `Integer.parseInt(s)` → `s.toInt()`
/// (and `Long` / `Short` / `Byte` / `Double` / `Float`), `String.valueOf(x)`
/// / `Integer.toString(x)` → `x.toString()` (not for a `CharArray`), and
/// `System.out.println(x)` → `println(x)`; the caret on the method's
/// name, when the file doesn't import a `Math` (…) of its own.
pub fn kotlin_analog(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
    let class = &text[target.byte_range()];
    let own = text.lines().any(|l| l.trim_start().starts_with("import ") && (l.trim_end().ends_with(&format!(".{class}")) || l.contains(&format!(" as {class}"))));
    if own {
        return None;
    }
    let value = |i: usize| values.get(i).map(|v| &text[v.byte_range()]);
    // A value written before `.toInt()` and the like.
    let operand = |v: Node| {
        let inner = v.named_child(0).unwrap_or(v);
        if inner.kind().ends_with("_literal") || inner.kind() == "line_string_literal" { text[v.byte_range()].to_owned() } else { receiver(inner, text) }
    };
    let (function, replacement) = match (class, name, values.len()) {
        ("Math", "max" | "min", 2) => {
            let f = if name == "max" { "maxOf" } else { "minOf" };
            (f, format!("{f}({}, {})", value(0)?, value(1)?))
        }
        ("Integer" | "Long" | "Short" | "Byte" | "Double" | "Float", _, 1) => {
            let to = match (class, name) {
                ("Integer", "parseInt") => "toInt",
                ("Long", "parseLong") => "toLong",
                ("Short", "parseShort") => "toShort",
                ("Byte", "parseByte") => "toByte",
                ("Double", "parseDouble") => "toDouble",
                ("Float", "parseFloat") => "toFloat",
                (_, "toString") => "toString",
                _ => return None,
            };
            (to, format!("{}.{to}()", operand(values[0])))
        }
        ("String", "valueOf", 1) => {
            let v = values[0];
            if receiver_type(v.named_child(0).unwrap_or(v), text, classes).is_some_and(|t| type_head(&t) == "CharArray") {
                return None;
            }
            ("toString", format!("{}.toString()", operand(v)))
        }
        ("System.out", "println" | "print", 0 | 1) => (name, format!("{name}({})", value(0).unwrap_or(""))),
        _ => return None,
    };
    Some(Intention { title: format!("Replace with '{function}' function"), range: call.byte_range(), caret: call.start_byte(), replacement })
}

/// Replace 'count' with 'size' (IntelliJ's ReplaceCollectionCountWithSize
/// inspection): `xs.count()` → `xs.size` on a collection, map or array
/// (not an Iterable or a Sequence, whose `count()` iterates), and
/// `s.count()` → `s.length` on a string; the caret on `count`.
pub fn count_to_size(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
    if name != "count" || !values.is_empty() {
        return None;
    }
    let ty = receiver_type(target, text, classes)?;
    let head = type_head(&ty);
    let property = if matches!(head, "String" | "CharSequence" | "StringBuilder") {
        "length"
    } else if matches!(head, "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "MutableCollection" | "Map" | "MutableMap" | "HashMap" | "LinkedHashMap") || head.ends_with("Array") {
        "size"
    } else {
        return None;
    };
    if ty.ends_with('?') {
        return None;
    }
    Some(Intention { title: format!("Replace 'count' with '{property}'"), range: call.byte_range(), caret: call.start_byte(), replacement: format!("{}.{property}", &text[target.byte_range()]) })
}

/// Replace with 'lastIndex' (IntelliJ's ReplaceWithLastIndex inspection):
/// `xs.size - 1` → `xs.lastIndex` when `xs` is a list or an array, and
/// `s.length - 1` when `s` is a string; the caret on the subtraction.
pub fn last_index(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let minus = innermost(tree.root_node(), caret)
        .into_iter()
        .flat_map(ancestors)
        .take_while(|n| !matches!(n.kind(), "statements" | "lambda_literal" | "class_body"))
        .find(|n| n.kind() == "additive_expression")?;
    let [size, one] = children(minus)[..] else { return None };
    if &text[one.byte_range()] != "1" || text[size.end_byte()..one.start_byte()].trim() != "-" || size.kind() != "navigation_expression" {
        return None;
    }
    let target = size.named_child(0)?;
    let member = text[size.named_child(1)?.byte_range()].trim();
    let ty = receiver_type(target, text, classes)?;
    let head = type_head(&ty);
    let fits = match member {
        ".size" => matches!(head, "List" | "MutableList" | "ArrayList") || head.ends_with("Array"),
        ".length" => matches!(head, "String" | "CharSequence" | "StringBuilder"),
        _ => false,
    };
    if ty.ends_with('?') || !fits {
        return None;
    }
    Some(Intention { title: "Replace with 'lastIndex'".into(), range: minus.byte_range(), caret: minus.start_byte(), replacement: format!("{}.lastIndex", &text[target.byte_range()]) })
}

/// Replace 'substring' call with 'take' call / with 'dropLast' call
/// (IntelliJ's ReplaceSubstringWithTakeInspection and
/// ReplaceSubstringWithDropLastInspection): `s.substring(0, n)` →
/// `s.take(n)`, `s.substring(0, s.length - n)` → `s.dropLast(n)`, and
/// (ReplaceSubstringWithSubstringBeforeInspection) `s.substring(0,
/// s.indexOf(x))` → `s.substringBefore(x)` (`lastIndexOf` →
/// `substringBeforeLast`), when `s` is a String; the caret on `substring`.
pub fn substring_call(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
    let [start, end] = values[..] else { return None };
    if name != "substring" || &text[start.byte_range()] != "0" {
        return None;
    }
    let ty = receiver_type(target, text, classes)?;
    if ty != "String" {
        return None;
    }
    let receiver = &text[target.byte_range()];
    let end_text = &text[end.byte_range()];
    let inner = end.named_child(0).unwrap_or(end);
    let drop_last = (inner.kind() == "additive_expression")
        .then(|| children(inner))
        .and_then(|parts| match parts[..] {
            [length, count] if text[length.byte_range()] == format!("{receiver}.length") && text[length.end_byte()..count.start_byte()].trim() == "-" => Some(count),
            _ => None,
        });
    // `s.substring(0, s.indexOf(x))` → `s.substringBefore(x)`.
    let before = (inner.kind() == "call_expression")
        .then(|| children(inner))
        .and_then(|parts| match parts[..] {
            [callee, suffix] if callee.kind() == "navigation_expression" && callee.named_child(0).is_some_and(|r| &text[r.byte_range()] == receiver) => {
                let function = match text[callee.named_child(1)?.byte_range()].trim() {
                    ".indexOf" => "substringBefore",
                    ".lastIndexOf" => "substringBeforeLast",
                    _ => return None,
                };
                let [arguments] = children(suffix)[..] else { return None };
                let [value] = children(arguments)[..] else { return None };
                (arguments.kind() == "value_arguments").then_some((function, &text[value.byte_range()]))
            }
            _ => None,
        });
    let (function, argument) = match (drop_last, before) {
        (Some(count), _) => ("dropLast", &text[count.byte_range()]),
        (_, Some(found)) => found,
        _ => ("take", end_text),
    };
    Some(Intention {
        title: format!("Replace 'substring' call with '{function}' call"),
        range: call.byte_range(),
        caret: call.start_byte(),
        replacement: format!("{receiver}.{function}({argument})"),
    })
}

/// Replace 'get' call with indexing operator / Replace 'set' call with
/// indexing operator (IntelliJ's ReplaceGetOrSetInspection): `xs.get(i)`
/// → `xs[i]`, the statement `m.set(k, v)` → `m[k] = v`, when the
/// receiver is a list, map, array or string (whose `get` / `set` are
/// operators); the caret on `get` / `set`.
pub fn indexing_operator(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let (call, target, name, values) = method_call(tree.root_node(), text, caret)?;
    let set = match name {
        "get" => false,
        "set" => true,
        _ => return None,
    };
    if values.len() < 1 + set as usize {
        return None;
    }
    if set && call.parent()?.kind() != "statements" {
        return None;
    }
    let ty = receiver_type(target, text, classes)?;
    let head = type_head(&ty);
    let readable = matches!(head, "List" | "Map" | "String" | "CharSequence" | "StringBuilder");
    let writable = matches!(head, "MutableList" | "ArrayList" | "MutableMap" | "HashMap" | "LinkedHashMap" | "Array")
        || head.ends_with("Array") && head != "Array" && head.len() > 5;
    if ty.ends_with('?') || !(writable || readable && !set) {
        return None;
    }
    let (keys, value) = if set { values.split_at(values.len() - 1) } else { (&values[..], &[][..]) };
    let keys = keys.iter().map(|k| &text[k.byte_range()]).collect::<Vec<_>>().join(", ");
    let mut replacement = format!("{}[{keys}]", &text[target.byte_range()]);
    if let Some(v) = value.first() {
        replacement.push_str(&format!(" = {}", &text[v.byte_range()]));
    }
    let word = if set { "set" } else { "get" };
    Some(Intention { title: format!("Replace '{word}' call with indexing operator"), range: call.byte_range(), caret: call.start_byte(), replacement })
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
    let head = head.rsplit('.').next().unwrap_or(head);
    let expression = &text[value.byte_range()];
    let (bound, element) = if collection.ends_with("[]") {
        (format!("{expression}.length"), format!("{expression}[{{i}}]"))
    } else if matches!(head, "List" | "ArrayList" | "LinkedList" | "CopyOnWriteArrayList" | "Vector") {
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

/// Replace with enhanced 'for' (Java, IntelliJ's ForLoopReplaceableByForEach
/// fix): `for (int i = 0; i < names.size(); i++)` (or `< names.length`)
/// whose body reads `i` only in `names.get(i)` (`names[i]`) → `for (String
/// name : names)`, a leading `String s = names.get(i);` giving the name.
pub fn enhanced_for_loop(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let at = innermost(tree.root_node(), caret)?;
    let n = ancestors(at).find(|n| matches!(n.kind(), "for_statement" | "block" | "class_body"))?;
    let body = n.child_by_field_name("body").filter(|b| b.kind() == "block")?;
    if n.kind() != "for_statement" || caret >= body.start_byte() {
        return None;
    }
    let field = |name: &str| n.child_by_field_name(name).map(|c| text[c.byte_range()].split_whitespace().collect::<String>());
    // `int i = 0`, `i < x.size()`, `i++`
    let init = field("init")?;
    let index = init.strip_prefix("int")?.strip_suffix("=0;").or_else(|| init.strip_prefix("int")?.strip_suffix("=0"))?.to_owned();
    let condition = field("condition")?;
    let bound = condition.strip_prefix(&format!("{index}<"))?;
    let (collection, array) = match (bound.strip_suffix(".size()"), bound.strip_suffix(".length")) {
        (Some(c), _) => (c.to_owned(), false),
        (_, Some(c)) => (c.to_owned(), true),
        _ => return None,
    };
    if !collection.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') || collection.is_empty() {
        return None;
    }
    let update = field("update")?;
    if ![format!("{index}++"), format!("++{index}"), format!("{index}+=1")].contains(&update) {
        return None;
    }
    // The collection's element type, from its declaration.
    let value = n.child_by_field_name("condition")?.child_by_field_name("right")?.child_by_field_name("object")?;
    let declared = super::surround::expression_type(Lang::Java, value, text).or_else(|| super::types::type_of(Lang::Java, text, value.byte_range(), classes).map(|t| t.ty))?;
    let element = if array {
        declared.strip_suffix("[]")?.to_owned()
    } else {
        let (head, args) = declared.split_once('<')?;
        if !matches!(head.rsplit('.').next().unwrap_or(head), "List" | "ArrayList" | "LinkedList" | "Vector" | "CopyOnWriteArrayList") {
            return None;
        }
        args.strip_suffix('>')?.trim().to_owned()
    };
    let access = if array { format!("{collection}[{index}]") } else { format!("{collection}.get({index})") };
    let body_text = &text[body.byte_range()];
    // Every use of the index is an access; the list isn't changed.
    let squeezed: String = body_text.split_whitespace().collect();
    let uses = squeezed.match_indices(&index).filter(|(i, _)| {
        let before = squeezed[..*i].chars().next_back();
        let after = squeezed[i + index.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    });
    if uses.count() != squeezed.matches(&access).count() || ["add", "remove", "set", "clear", "addAll", "removeIf"].iter().any(|m| squeezed.contains(&format!("{collection}.{m}("))) {
        return None;
    }
    // A leading `T name = access;` names the loop's variable.
    let inner = body_text[1..body_text.len() - 1].to_owned();
    let first = inner.trim_start();
    let leading = first.split_once(';').and_then(|(statement, _)| {
        let (decl, value) = statement.split_once('=')?;
        let name = decl.split_whitespace().last()?.to_owned();
        (value.split_whitespace().collect::<String>() == access && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| (name, statement.len() + 1))
    });
    let (name, inner) = match leading {
        Some((name, len)) => {
            let rest = &first[len..];
            // Drop the declaration's line.
            let rest = rest.strip_prefix(|c: char| c == ' ' || c == '\t').map_or(rest, |r| r).trim_start_matches([' ', '\t']);
            let rest = rest.strip_prefix('\n').unwrap_or(rest);
            let lead = &inner[..inner.len() - first.len()];
            let lead = lead.rfind('\n').map_or("", |i| &lead[..=i]);
            (name, format!("{lead}{rest}"))
        }
        None => {
            let base = iteration_name(&collection, &element);
            let name = std::iter::once(base.clone()).chain((1..).map(|k| format!("{base}{k}"))).find(|c| !mentions(&inner, c))?;
            (name, inner)
        }
    };
    let inner = replace_access(&inner, &collection, &index, array, &name);
    Some(Intention { title: "Replace with enhanced 'for'".into(), range: n.byte_range(), replacement: format!("for ({element} {name} : {collection}) {{{inner}}}"), caret: n.start_byte() })
}

/// `names.get(i)` / `names.get( i )` / `names[i]` → `name`.
fn replace_access(text: &str, collection: &str, index: &str, array: bool, name: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let (open, close) = if array { ('[', ']') } else { ('(', ')') };
    let prefix = if array { collection.to_owned() } else { format!("{collection}.get") };
    while let Some(i) = rest.find(&prefix) {
        let after = &rest[i + prefix.len()..];
        let boundary = !rest[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
        let arg = after.trim_start().strip_prefix(open).and_then(|a| a.split_once(close)).filter(|(a, _)| a.trim() == index);
        match arg {
            Some((_, tail)) if boundary => {
                out.push_str(&rest[..i]);
                out.push_str(name);
                rest = tail;
            }
            _ => {
                out.push_str(&rest[..i + prefix.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A size compared with 0 (or 1) → `isEmpty()` / `isNotEmpty()`: Kotlin's
/// ReplaceSizeZeroCheckWithIsEmpty and ReplaceSizeCheckWithIsNotEmpty
/// fixes (`list.size == 0`, `text.length > 0`), Java's
/// SizeReplaceableByIsEmpty fix (`list.size() == 0` → `list.isEmpty()`,
/// `!= 0` / `> 0` → `!list.isEmpty()`), when the receiver is declared a
/// collection, map, array (Kotlin) or string.
pub fn size_check(text: &str, caret: usize, kotlin: bool, classes: super::catches::Classes) -> Option<Intention> {
    let lang = if kotlin { Lang::Kotlin } else { Lang::Java };
    let tree = symbols::parse(lang, text)?;
    let at = innermost(tree.root_node(), caret)?;
    let comparison = ancestors(at)
        .take_while(|n| !matches!(n.kind(), "statements" | "block" | "class_body" | "lambda_literal"))
        .find(|n| matches!(n.kind(), "equality_expression" | "comparison_expression" | "binary_expression") && n.named_child_count() == 2)?;
    let (left, right) = (comparison.named_child(0)?, comparison.named_child(1)?);
    let op = text[left.end_byte()..right.start_byte()].trim();
    let number = |n: Node| -> Option<u32> { matches!(n.kind(), "integer_literal" | "decimal_integer_literal").then(|| text[n.byte_range()].parse().ok()).flatten() };
    // `size` on the left, the number on the right.
    let (access, op, value) = match (number(left), number(right)) {
        (None, Some(v)) => (left, op.to_owned(), v),
        (Some(v), None) => (right, match op { "<" => ">", ">" => "<", "<=" => ">=", ">=" => "<=", o => o }.to_owned(), v),
        _ => return None,
    };
    let empty = match (op.as_str(), value) {
        ("==", 0) | ("<", 1) => true,
        ("!=", 0) | (">", 0) | (">=", 1) => false,
        _ => return None,
    };
    // `x.size` / `x.length` (Kotlin), `x.size()` / `x.length()` (Java).
    let (receiver, member) = if kotlin {
        if access.kind() != "navigation_expression" || text[access.byte_range()].contains("?.") {
            return None;
        }
        let suffix = access.named_child(1)?;
        (access.named_child(0)?, text[suffix.byte_range()].trim_start_matches('.').to_owned())
    } else {
        if access.kind() != "method_invocation" || access.child_by_field_name("arguments").is_some_and(|a| a.named_child_count() > 0) {
            return None;
        }
        (access.child_by_field_name("object")?, text[access.child_by_field_name("name")?.byte_range()].to_owned())
    };
    let declared = if kotlin {
        super::introduce::kotlin_type(receiver, text).filter(|t| !t.is_empty())
    } else {
        super::surround::expression_type(Lang::Java, receiver, text)
    }
    .or_else(|| super::types::type_of(lang, text, receiver.byte_range(), classes).map(|t| t.ty))?;
    if declared.ends_with('?') {
        return None;
    }
    let head = declared.split('<').next().unwrap_or(&declared);
    let head = head.rsplit('.').next().unwrap_or(head);
    let sized = matches!(
        head,
        "List" | "MutableList" | "ArrayList" | "LinkedList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "TreeSet" | "Collection" | "MutableCollection" | "Map" | "MutableMap" | "HashMap"
            | "LinkedHashMap" | "TreeMap"
    ) || kotlin && (head == "Array" || head.ends_with("Array"));
    let ok = match member.as_str() {
        "size" => sized,
        "length" => head == "String" || kotlin && matches!(head, "CharSequence" | "StringBuilder"),
        _ => false,
    };
    if !ok || !kotlin && declared.ends_with("[]") {
        return None;
    }
    let receiver_text = &text[receiver.byte_range()];
    let (title, replacement) = match (kotlin, empty) {
        (true, true) => ("Replace size zero check with 'isEmpty()'", format!("{receiver_text}.isEmpty()")),
        (true, false) => ("Replace size check with 'isNotEmpty()'", format!("{receiver_text}.isNotEmpty()")),
        (false, true) => ("Replace with 'isEmpty()'", format!("{receiver_text}.isEmpty()")),
        (false, false) => ("Replace with '!isEmpty()'", format!("!{receiver_text}.isEmpty()")),
    };
    Some(Intention { title: title.into(), range: comparison.byte_range(), caret: comparison.start_byte() + replacement.len(), replacement })
}

/// The type of a Java expression, from what it shows or the declarations.
fn java_type(node: Node, text: &str, classes: super::catches::Classes) -> Option<String> {
    super::surround::expression_type(Lang::Java, node, text).or_else(|| super::types::type_of(Lang::Java, text, node.byte_range(), classes).map(|t| t.ty))
}

/// Java string fixes: Replace '==' with 'equals()' / Replace '!=' with
/// '!equals()' (IntelliJ's StringEqualityInspection) where both sides are
/// strings, `a == b` → `a.equals(b)`; and Replace with 'isEmpty()'
/// (StringEqualsEmptyStringInspection), `s.equals("")` → `s.isEmpty()`.
pub fn string_comparison(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let at = innermost(tree.root_node(), caret)?;
    let string = |n: Node| java_type(n, text, classes).is_some_and(|t| t == "String" || t == "java.lang.String");
    let near = || ancestors(at).take_while(|n| !matches!(n.kind(), "block" | "class_body" | "expression_statement" | "lambda_expression"));
    if let Some(call) = near().find(|n| n.kind() == "method_invocation") {
        let name = call.child_by_field_name("name").map(|n| &text[n.byte_range()]);
        let arguments = call.child_by_field_name("arguments").map(children).unwrap_or_default();
        if name == Some("equals") && arguments.len() == 1 && &text[arguments[0].byte_range()] == "\"\"" {
            let object = call.child_by_field_name("object")?;
            if string(object) {
                let replacement = format!("{}.isEmpty()", &text[object.byte_range()]);
                return Some(Intention { title: "Replace with 'isEmpty()'".into(), range: call.byte_range(), caret: call.start_byte(), replacement });
            }
        }
    }
    let comparison = near().find(|n| n.kind() == "binary_expression" && n.named_child_count() == 2 && matches!(n.child_by_field_name("operator").map(|o| o.kind()), Some("==" | "!=")))?;
    let (left, right) = (comparison.named_child(0)?, comparison.named_child(1)?);
    let not = comparison.child_by_field_name("operator")?.kind() == "!=";
    if [left, right].iter().any(|n| n.kind() == "null_literal") || !string(left) || !string(right) {
        return None;
    }
    let object = &text[left.byte_range()];
    let primary = matches!(left.kind(), "identifier" | "string_literal" | "method_invocation" | "field_access" | "parenthesized_expression" | "array_access" | "this");
    let object = if primary { object.to_owned() } else { format!("({object})") };
    let replacement = format!("{}{object}.equals({})", if not { "!" } else { "" }, &text[right.byte_range()]);
    let title = if not { "Replace '!=' with '!equals()'" } else { "Replace '==' with 'equals()'" };
    Some(Intention { title: title.into(), range: comparison.byte_range(), caret: comparison.start_byte(), replacement })
}

/// Replace 'indexOf()' with 'contains()' (Java, IntelliJ's
/// IndexOfReplaceableByContainsInspection): `s.indexOf(x) != -1` (or
/// `>= 0`, `> -1`) → `s.contains(x)`, `s.indexOf(x) == -1` (or `< 0`) →
/// `!s.contains(x)`, when `s` is a String (not with a `char`) or a List;
/// the caret on the comparison.
pub fn index_of_contains(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let comparison = ancestors(innermost(tree.root_node(), caret)?)
        .take_while(|n| !matches!(n.kind(), "block" | "class_body" | "expression_statement" | "lambda_expression"))
        .find(|n| n.kind() == "binary_expression" && n.child_by_field_name("left").is_some_and(|l| l.kind() == "method_invocation"))?;
    let call = comparison.child_by_field_name("left")?;
    let bound = text[comparison.child_by_field_name("right")?.byte_range()].replace(' ', "");
    let negated = match (comparison.child_by_field_name("operator")?.kind(), bound.as_str()) {
        ("!=" | ">", "-1") | (">=", "0") => false,
        ("==", "-1") | ("<", "0") => true,
        _ => return None,
    };
    let object = call.child_by_field_name("object")?;
    let arguments = children(call.child_by_field_name("arguments")?);
    if &text[call.child_by_field_name("name")?.byte_range()] != "indexOf" || arguments.len() != 1 {
        return None;
    }
    let ty = java_type(object, text, classes)?;
    let head = ty.split('<').next().unwrap_or(&ty).rsplit('.').next().unwrap_or(&ty).to_owned();
    let fits = match head.as_str() {
        "String" => arguments[0].kind() != "character_literal" && java_type(arguments[0], text, classes).is_none_or(|t| t != "char" && t != "int"),
        "List" | "ArrayList" | "LinkedList" => true,
        _ => false,
    };
    if !fits {
        return None;
    }
    Some(Intention {
        title: "Replace 'indexOf()' with 'contains()'".into(),
        range: comparison.byte_range(),
        caret: comparison.start_byte(),
        replacement: format!("{}{}.contains({})", if negated { "!" } else { "" }, &text[object.byte_range()], &text[arguments[0].byte_range()]),
    })
}

/// Replace with 'getOrDefault' method call (Java, IntelliJ's
/// Java8MapApiInspection): `m.containsKey(k) ? m.get(k) : d` (or
/// `!m.containsKey(k) ? d : m.get(k)`) → `m.getOrDefault(k, d)`, when `m`
/// is a Map; the caret on the conditional.
pub fn get_or_default(text: &str, caret: usize, classes: super::catches::Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let ternary = ancestors(innermost(tree.root_node(), caret)?)
        .take_while(|n| !matches!(n.kind(), "block" | "class_body" | "expression_statement" | "lambda_expression"))
        .find(|n| n.kind() == "ternary_expression")?;
    let mut condition = ternary.child_by_field_name("condition")?;
    while condition.kind() == "parenthesized_expression" {
        condition = condition.named_child(0)?;
    }
    let (mut found, mut missing) = (ternary.child_by_field_name("consequence")?, ternary.child_by_field_name("alternative")?);
    if condition.kind() == "unary_expression" && text[condition.byte_range()].starts_with('!') {
        condition = condition.named_child(0)?;
        std::mem::swap(&mut found, &mut missing);
    }
    // `m.containsKey(k)` and `m.get(k)`: the map and the key.
    let call = |n: Node, name: &str| -> Option<(String, String)> {
        let arguments = children(n.child_by_field_name("arguments")?);
        if n.kind() != "method_invocation" || &text[n.child_by_field_name("name")?.byte_range()] != name || arguments.len() != 1 {
            return None;
        }
        Some((text[n.child_by_field_name("object")?.byte_range()].to_owned(), text[arguments[0].byte_range()].to_owned()))
    };
    let (map, key) = call(condition, "containsKey")?;
    if call(found, "get")? != (map.clone(), key.clone()) {
        return None;
    }
    let object = condition.child_by_field_name("object")?;
    let ty = java_type(object, text, classes)?;
    let head = ty.split('<').next().unwrap_or(&ty).rsplit('.').next().unwrap_or(&ty).to_owned();
    if !matches!(head.as_str(), "Map" | "HashMap" | "LinkedHashMap" | "TreeMap" | "SortedMap" | "NavigableMap" | "ConcurrentMap" | "ConcurrentHashMap") {
        return None;
    }
    Some(Intention {
        title: "Replace with 'getOrDefault' method call".into(),
        range: ternary.byte_range(),
        caret: ternary.start_byte(),
        replacement: format!("{map}.getOrDefault({key}, {})", &text[missing.byte_range()]),
    })
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
    fn converts_between_for_loops_and_for_each() {
        let text = "fun f(xs: List<Int>) {\n    for (x in xs) {\n        if (x < 0) continue\n        for (y in xs) { if (y > x) break; continue }\n        println(x)\n    }\n    for (i in 1..10) println(i)\n    for (x in xs) { if (x > 2) break }\n    outer@ for (x in xs) { for (y in xs) { if (y == x) continue@outer } }\n}\n";
        let out = find(text, "x in xs) {\n", "Replace with a 'forEach' function call").unwrap();
        assert!(out.contains("    xs.forEach { x ->\n        if (x < 0) return@forEach\n        for (y in xs) { if (y > x) break; continue }\n        println(x)\n    }\n"), "{out}");
        assert!(find(text, "i in", "Replace with a 'forEach' function call").unwrap().contains("    (1..10).forEach { i -> println(i) }\n"));
        // A `break` out of the loop can't be kept; a labelled `continue` can.
        assert_eq!(find(text, "x in xs) { if", "Replace with a 'forEach' function call"), None);
        assert!(find(text, "x in xs) { for", "Replace with a 'forEach' function call").unwrap().contains("    xs.forEach { x -> for (y in xs) { if (y == x) return@forEach } }\n"));
        // Not in the body.
        assert_eq!(find(text, "println(x)", "Replace with a 'forEach' function call"), None);

        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let back = |text: &str, at: &str| for_each_to_loop(text, text.find(at).unwrap(), &classes).map(|i| apply(text, &i));
        assert_eq!(back(&out, "forEach").unwrap(), text);
        let text = "fun f(xs: List<Int>, m: Map<String, Int>, n: Int) {\n    (1..3).forEach { i -> if (i == 2) return@forEach }\n    xs.forEach { println(it) }\n    m.forEach { (k, v) -> println(k) }\n    m.forEach { k, v -> println(k) }\n    xs.forEach { x -> xs.map { return@forEach } }\n    val u = xs.forEach { }\n}\n";
        assert!(back(text, "xs.forEach { p").unwrap().contains("    for (it in xs) { println(it) }\n"));
        assert!(back(text, "forEach { (k").unwrap().contains("    for ((k, v) in m) { println(k) }\n"));
        assert!(back(text, "(1..3)").unwrap().contains("    for (i in 1..3) { if (i == 2) continue }\n"));
        // Two parameters (Java's BiConsumer), a return from a lambda inside, not a statement, inside the lambda.
        assert_eq!(back(text, "forEach { k,"), None);
        assert_eq!(back(text, "forEach { x"), None);
        assert_eq!(back(text, "forEach { }"), None);
        assert_eq!(back(text, "println(it)"), None);
    }

    #[test]
    fn replaces_get_and_set_with_indexing() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, m: MutableMap<String, Int>, s: String, set: Set<Int>, a: IntArray) {\n    val x = xs.get(0) + s.get(1).code\n    m.set(\"a\", xs.get(1))\n    a.set(0, 2)\n    val y = m.get(key = \"a\")\n    xs.set(0, 1)\n    set.get(0)\n    val z = m.set(\"b\", 1)\n}\n";
        let run = |at: &str| indexing_operator(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("get(0)").unwrap();
        assert_eq!(title, "Replace 'get' call with indexing operator");
        assert!(out.contains("val x = xs[0] + s.get(1)"), "{out}");
        assert!(run("get(1).code").unwrap().1.contains("xs.get(0) + s[1].code"));
        let (title, out) = run("set(\"a\"").unwrap();
        assert_eq!(title, "Replace 'set' call with indexing operator");
        assert!(out.contains("    m[\"a\"] = xs.get(1)\n"), "{out}");
        // The inner call when the caret is on its name.
        assert!(run("get(1))").unwrap().1.contains("m.set(\"a\", xs[1])"));
        assert!(run("set(0, 2)").unwrap().1.contains("    a[0] = 2\n"));
        // Named arguments, a read-only list's set, a set's get, a set used as a value, inside the arguments.
        assert_eq!(run("get(key"), None);
        assert_eq!(run("set(0, 1)"), None);
        assert_eq!(run("get(0)\n"), None);
        assert_eq!(run("set(\"b\""), None);
        assert_eq!(run("0) +"), None);
    }

    #[test]
    fn replaces_calls_with_binary_operators() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(a: String, b: String, xs: List<Int>, n: Int, d: Double, m: Map<String, Int>, o: String?) {\n    if (a.equals(b)) return\n    val c = !a.equals(b)\n    val e = xs.contains(n + 1)\n    if (!m.contains(a)) return\n    val g = n.plus(2).toString()\n    val h = n.times(n - 1)\n    val i = d.equals(1.0)\n    val j = o.equals(a)\n    val k = a.plus(b, c)\n}\n";
        let run = |at: &str| binary_operator(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("equals(b)) return").unwrap();
        assert_eq!(title, "Replace with '=='");
        assert!(out.contains("    if (a == b) return\n"), "{out}");
        let (title, out) = run("equals(b)\n").unwrap();
        assert_eq!(title, "Replace with '!='");
        assert!(out.contains("    val c = a != b\n"), "{out}");
        assert!(run("contains(n").unwrap().1.contains("    val e = (n + 1) in xs\n"));
        let (title, out) = run("contains(a)").unwrap();
        assert_eq!(title, "Replace with '!in'");
        assert!(out.contains("    if (a !in m) return\n"), "{out}");
        assert!(run("plus(2)").unwrap().1.contains("    val g = (n + 2).toString()\n"));
        assert!(run("times").unwrap().1.contains("    val h = n * (n - 1)\n"));
        // Double's equals isn't ==; a nullable receiver; two arguments; not on the receiver.
        assert_eq!(run("equals(1.0)"), None);
        assert_eq!(run("equals(a)"), None);
        assert_eq!(run("plus(b, c)"), None);
        assert_eq!(run("a.equals(b)) return"), None);
    }

    #[test]
    fn compares_java_strings_with_equals() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "class A {\n    boolean f(String a, String b, Object o, int n) {\n        if (a == b) return true;\n        boolean c = a + \"x\" != b;\n        boolean d = a == null || o == a || n == 1;\n        return a.equals(\"\") && o.equals(\"\");\n    }\n}\n";
        let run = |at: &str| string_comparison(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("a == b").unwrap();
        assert_eq!(title, "Replace '==' with 'equals()'");
        assert!(out.contains("        if (a.equals(b)) return true;\n"), "{out}");
        let (title, out) = run("!= b").unwrap();
        assert_eq!(title, "Replace '!=' with '!equals()'");
        assert!(out.contains("        boolean c = !(a + \"x\").equals(b);\n"), "{out}");
        let (title, out) = run("equals(\"\") &&").unwrap();
        assert_eq!(title, "Replace with 'isEmpty()'");
        assert!(out.contains("        return a.isEmpty() && o.equals(\"\");\n"), "{out}");
        // null, an Object, numbers.
        assert_eq!(run("a == null"), None);
        assert_eq!(run("o == a"), None);
        assert_eq!(run("n == 1"), None);
        assert_eq!(run("o.equals"), None);
    }

    #[test]
    fn replaces_null_checks_with_is_null_or_empty() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(s: String?, xs: List<Int>?, t: String, ys: List<Int>?, a: Boolean) {\n    if (s == null || s.isEmpty()) return\n    val b = xs != null && xs.isNotEmpty()\n    val c = a || null == s || s.isBlank()\n    val d = t == null || t.isEmpty()\n    val e = xs == null || xs.isBlank()\n    val g = ys == null || xs.isEmpty()\n}\n";
        let run = |at: &str| null_or_empty(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("s.isEmpty").unwrap();
        assert_eq!(title, "Replace with 'isNullOrEmpty()'");
        assert!(out.contains("    if (s.isNullOrEmpty()) return\n"), "{out}");
        assert!(run("xs != null").unwrap().1.contains("    val b = !xs.isNullOrEmpty()\n"));
        let (title, out) = run("null == s").unwrap();
        assert_eq!(title, "Replace with 'isNullOrBlank()'");
        assert!(out.contains("    val c = a || s.isNullOrBlank()\n"), "{out}");
        // Not nullable; a list isn't blank; another variable.
        assert_eq!(run("t == null"), None);
        assert_eq!(run("xs == null || xs.isBlank"), None);
        assert_eq!(run("ys == null"), None);
    }

    #[test]
    fn replaces_elvis_with_or_empty_and_negations() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(s: String?, xs: List<Int>?, t: String, ys: List<Int>, m: Map<String, Int>?) {\n    val a = s ?: \"\"\n    val b = xs ?: emptyList()\n    val c = t ?: \"\"\n    val d = xs ?: emptySet()\n    val e = m ?: emptyMap()\n    if (!ys.isEmpty()) return\n    val g = !t.isNotBlank()\n    val h = !s.isEmpty()\n    val i = !ys.isBlank()\n}\n";
        let run = |at: &str, f: fn(&str, usize, super::super::catches::Classes) -> Option<Intention>| f(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("?: \"\"", or_empty).unwrap();
        assert_eq!(title, "Replace with 'orEmpty()'");
        assert!(out.contains("    val a = s.orEmpty()\n"), "{out}");
        assert!(run("emptyList", or_empty).unwrap().1.contains("    val b = xs.orEmpty()\n"));
        assert!(run("emptyMap", or_empty).unwrap().1.contains("    val e = m.orEmpty()\n"));
        // Not nullable; the wrong empty value.
        assert_eq!(run("t ?:", or_empty), None);
        assert_eq!(run("emptySet", or_empty), None);
        let (title, out) = run("!ys.isEmpty", negated_emptiness).unwrap();
        assert_eq!(title, "Replace negated 'isEmpty' with 'isNotEmpty'");
        assert!(out.contains("    if (ys.isNotEmpty()) return\n"), "{out}");
        assert!(run("isNotBlank", negated_emptiness).unwrap().1.contains("    val g = t.isBlank()\n"));
        // A nullable receiver; a list isn't blank.
        assert_eq!(run("!s.isEmpty", negated_emptiness), None);
        assert_eq!(run("isBlank", negated_emptiness), None);
    }

    #[test]
    fn merges_sorted_and_list_of_chains() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, a: String?, b: String?) {\n    val m = xs.sorted().first()\n    val n = xs.sortedDescending().firstOrNull()\n    val o = listOf(a, b).filterNotNull()\n    val p = xs.map { it to it }.toMap()\n    val q = xs.sorted().take(2)\n}\n";
        let run = |at: &str| merge_call_chain(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run(".first()").unwrap();
        assert_eq!(title, "Merge call chain to 'min'");
        assert!(out.contains("    val m = xs.min()\n"), "{out}");
        assert!(run(".firstOrNull").unwrap().1.contains("    val n = xs.maxOrNull()\n"));
        assert!(run(".filterNotNull").unwrap().1.contains("    val o = listOfNotNull(a, b)\n"));
        assert!(run(".toMap").unwrap().1.contains("    val p = xs.associate { it to it }\n"));
        assert_eq!(run(".take"), None);
    }

    #[test]
    fn merges_call_chains() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, m: Map<String, Int>, s: String, q: List<Int>?) {\n    val a = xs.filter { it > 1 }.first()\n    val b = xs.map { it.toString() }.filterNotNull().size\n    if (s.filter { it.isDigit() }.isNotEmpty()) return\n    val c = m.filter { it.value > 1 }.count()\n    val d = xs.filter { it > 1 }.sum()\n    val e = q?.filter { it > 1 }?.first()\n}\n";
        let run = |at: &str| merge_call_chain(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("first()").unwrap();
        assert_eq!(title, "Merge call chain to 'first'");
        assert!(out.contains("    val a = xs.first { it > 1 }\n"), "{out}");
        // On the first call too; not in the lambda.
        assert!(run("filter { it > 1 }.first").is_some());
        assert_eq!(run("it > 1 }.first"), None);
        assert!(run("filterNotNull").unwrap().1.contains("    val b = xs.mapNotNull { it.toString() }.size\n"));
        assert!(run("isNotEmpty").unwrap().1.contains("    if (s.any { it.isDigit() }) return\n"));
        let more = "fun g(xs: List<Int>) {\n    val a = xs.map { it * 2 }.sum()\n    val b = xs.sortedBy { -it }.lastOrNull()\n    val c = xs.map { \"<$it>\" }.joinToString()\n    val d = xs.map { it }.joinToString(\", \")\n}\n";
        let more_run = |at: &str| merge_call_chain(more, more.find(at).unwrap(), &classes).map(|i| apply(more, &i));
        assert!(more_run("sum()").unwrap().contains("    val a = xs.sumOf { it * 2 }\n"));
        // Right after the last `)`.
        let end = more.find("sum()").unwrap() + "sum()".len();
        assert!(merge_call_chain(more, end, &classes).is_some());
        assert!(more_run("lastOrNull").unwrap().contains("    val b = xs.maxByOrNull { -it }\n"));
        assert!(more_run("joinToString()").unwrap().contains("    val c = xs.joinToString { \"<$it>\" }\n"));
        // joinToString with a separator keeps its argument.
        assert_eq!(more_run("joinToString(\","), None);
        // A map's filter is a map; sum has no predicate form; a nullable receiver.
        assert_eq!(run("count()"), None);
        assert_eq!(run("sum()"), None);
        assert_eq!(run("?.first"), None);
    }

    #[test]
    fn removes_redundant_string_value_of() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "class A {\n    String f(String s, int n, int m, char[] cs) {\n        String a = String.valueOf(s);\n        String b = \"n=\" + String.valueOf(n);\n        String c = \"n=\" + String.valueOf(n + m);\n        String d = String.valueOf(n);\n        String e = \"c=\" + String.valueOf(cs);\n        int g = 1 + Integer.valueOf(n);\n        return a;\n    }\n}\n";
        let run = |at: &str| redundant_conversion(text, text.find(at).unwrap() + at.find("valueOf").unwrap_or(0), false, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("String.valueOf(s)").unwrap();
        assert_eq!(title, "Remove redundant 'String.valueOf()' call");
        assert!(out.contains("        String a = s;\n"), "{out}");
        assert!(run("String.valueOf(n);\n        String c").unwrap().1.contains("        String b = \"n=\" + n;\n"));
        assert!(run("String.valueOf(n + m)").unwrap().1.contains("        String c = \"n=\" + (n + m);\n"));
        // Converts an int on its own; a char[] is its chars; not String's.
        assert_eq!(run("String.valueOf(n);\n        String e"), None);
        assert_eq!(run("String.valueOf(cs)"), None);
        assert_eq!(run("Integer.valueOf"), None);
    }

    #[test]
    fn removes_redundant_conversions() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(s: String, n: Int, l: Long, q: String?) {\n    val a = s.toString().length\n    val b = n.toInt() + l.toInt()\n    val c = q.toString()\n}\n";
        let run = |at: &str| redundant_conversion(text, text.find(at).unwrap(), true, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("toString().length").unwrap();
        assert_eq!(title, "Remove redundant call");
        assert!(out.contains("    val a = s.length\n"), "{out}");
        assert!(run("toInt() +").unwrap().1.contains("    val b = n + l.toInt()\n"));
        // A Long's toInt converts; a nullable's toString gives \"null\".
        assert_eq!(run("toInt()\n"), None);
        assert_eq!(run("toString()\n"), None);

        let text = "class A {\n    int f(String s, Object o) {\n        return s.toString().length() + o.toString().length();\n    }\n}\n";
        let run = |at: &str| redundant_conversion(text, text.find(at).unwrap(), false, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("toString().length() +").unwrap();
        assert_eq!(title, "Remove redundant 'toString()' call");
        assert!(out.contains("        return s.length() + o.toString().length();\n"), "{out}");
        assert_eq!(run("toString().length();"), None);
    }

    #[test]
    fn replaces_java_static_methods_with_kotlin_analogs() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(a: Int, b: Int, s: String, cs: CharArray) {\n    val m = Math.max(a, b)\n    val n = Integer.parseInt(s)\n    val o = Integer.parseInt(\"12\")\n    val p = String.valueOf(a + b)\n    val q = String.valueOf(cs)\n    System.out.println(m)\n    val r = Math.abs(a)\n}\n";
        let run = |at: &str| kotlin_analog(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("max(").unwrap();
        assert_eq!(title, "Replace with 'maxOf' function");
        assert!(out.contains("    val m = maxOf(a, b)\n"), "{out}");
        assert!(run("parseInt(s").unwrap().1.contains("    val n = s.toInt()\n"));
        assert!(run("parseInt(\"").unwrap().1.contains("    val o = \"12\".toInt()\n"));
        assert!(run("valueOf(a").unwrap().1.contains("    val p = (a + b).toString()\n"));
        assert!(run("println").unwrap().1.contains("    println(m)\n"));
        // A CharArray's chars; a function without an analog here.
        assert_eq!(run("valueOf(cs"), None);
        assert_eq!(run("abs"), None);
        // The file's own Math.
        let own = "import com.example.Math\nval m = Math.max(1, 2)\n";
        assert_eq!(kotlin_analog(own, own.find("max").unwrap(), &classes), None);
    }

    #[test]
    fn replaces_substring_with_take_and_drop_last() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(s: String, t: String?, xs: List<Int>, n: Int) {\n    val a = s.substring(0, n)\n    val b = s.substring(0, s.length - 2)\n    val c = s.substring(1, n)\n    val d = t?.substring(0, n)\n    val e = s.substring(n)\n}\n";
        let run = |at: &str| substring_call(text, text.find(at).unwrap() + 2, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("s.substring(0, n)").unwrap();
        assert_eq!(title, "Replace 'substring' call with 'take' call");
        assert!(out.contains("    val a = s.take(n)\n"), "{out}");
        let (title, out) = run("s.substring(0, s.length").unwrap();
        assert_eq!(title, "Replace 'substring' call with 'dropLast' call");
        assert!(out.contains("    val b = s.dropLast(2)\n"), "{out}");
        let text2 = "fun g(s: String, t: String) {\n    val a = s.substring(0, s.indexOf('.'))\n    val b = s.substring(0, s.lastIndexOf(\"/\"))\n    val c = s.substring(0, t.indexOf('.'))\n}\n";
        let run2 = |at: &str| substring_call(text2, text2.find(at).unwrap() + 2, &classes).map(|i| (i.title.clone(), apply(text2, &i)));
        let (title, out) = run2("s.substring(0, s.indexOf").unwrap();
        assert_eq!(title, "Replace 'substring' call with 'substringBefore' call");
        assert!(out.contains("    val a = s.substringBefore('.')\n"), "{out}");
        assert!(run2("s.substring(0, s.lastIndexOf").unwrap().1.contains("    val b = s.substringBeforeLast(\"/\")\n"));
        // Another string's index: take.
        assert_eq!(run2("s.substring(0, t.indexOf").unwrap().0, "Replace 'substring' call with 'take' call");
        // Not from 0; a nullable receiver; one argument.
        assert_eq!(run("s.substring(1"), None);
        assert_eq!(run("t?.substring"), None);
        assert_eq!(run("s.substring(n)"), None);
    }

    #[test]
    fn converts_two_comparisons_to_range_check() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(x: Int, d: Double, n: Int?, s: String, lo: Int, hi: Int) {\n    if (x >= 1 && x <= 10) return\n    if (10 >= x && 1 <= x) return\n    if (x >= lo && x < hi + 1) return\n    if (d >= 0.0 && d <= 1.0) return\n    if (d >= 0.0 && d < 1.0) return\n    if (x > 1 && x <= 10) return\n    if (n >= 1 && n <= 10) return\n    if (x >= 1 && lo <= 10) return\n}\n";
        let run = |at: &str| range_check(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("x >= 1 && x <= 10").unwrap();
        assert_eq!(title, "Convert two comparisons to range check");
        assert!(out.contains("    if (x in 1..10) return\n"), "{out}");
        assert!(run("10 >= x").unwrap().1.contains("    if (x in 1..10) return\n"));
        assert!(run("x >= lo").unwrap().1.contains("    if (x in lo until (hi + 1)) return\n"));
        assert!(run("d >= 0.0 && d <= 1.0").unwrap().1.contains("    if (d in 0.0..1.0) return\n"));
        // No `until` for doubles; a strict lower bound; nullable; different subjects.
        assert_eq!(run("d >= 0.0 && d < 1.0"), None);
        assert_eq!(run("x > 1"), None);
        assert_eq!(run("n >= 1"), None);
        assert_eq!(run("lo <= 10"), None);
    }

    #[test]
    fn replaces_associate_with_associate_with_and_by() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<String>, m: Map<String, Int>, n: List<String>?) {\n    val a = xs.associate { it to it.length }\n    val b = xs.associate { s -> s.length to s }\n    val c = xs.associate { it.length to it.first() }\n    val d = m.associate { it to 1 }\n    val e = n?.associate { it to 1 }\n}\n";
        let run = |at: &str| associate_function(text, text.find(at).unwrap() + 4, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("xs.associate { it to").unwrap();
        assert_eq!(title, "Replace 'associate' with 'associateWith'");
        assert!(out.contains("    val a = xs.associateWith { it.length }\n"), "{out}");
        let (title, out) = run("xs.associate { s").unwrap();
        assert_eq!(title, "Replace 'associate' with 'associateBy'");
        assert!(out.contains("    val b = xs.associateBy { s -> s.length }\n"), "{out}");
        // Neither side is the element; a map; nullable.
        assert_eq!(run("xs.associate { it.length"), None);
        assert_eq!(run("m.associate"), None);
        assert_eq!(run("n?.associate"), None);
    }

    #[test]
    fn replaces_index_of_with_contains() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "class A {\n    void f(String s, List<String> xs, int[] a) {\n        if (s.indexOf(\"x\") != -1) return;\n        if (s.indexOf(\"y\") < 0) return;\n        if (xs.indexOf(s) >= 0) return;\n        if (s.indexOf('c') != -1) return;\n        if (s.indexOf(\"z\") > 0) return;\n    }\n}\n";
        let run = |at: &str| index_of_contains(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("s.indexOf(\"x").unwrap();
        assert_eq!(title, "Replace 'indexOf()' with 'contains()'");
        assert!(out.contains("        if (s.contains(\"x\")) return;\n"), "{out}");
        assert!(run("s.indexOf(\"y").unwrap().1.contains("        if (!s.contains(\"y\")) return;\n"));
        assert!(run("xs.indexOf").unwrap().1.contains("        if (xs.contains(s)) return;\n"));
        // A char; `> 0` isn't "found".
        assert_eq!(run("s.indexOf('c"), None);
        assert_eq!(run("s.indexOf(\"z"), None);
    }

    #[test]
    fn replaces_contains_key_conditional_with_get_or_default() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "class A {\n    int f(Map<String, Integer> m, Map<String, Integer> o, String k) {\n        int a = m.containsKey(k) ? m.get(k) : 0;\n        int b = !m.containsKey(k) ? -1 : m.get(k);\n        int c = m.containsKey(k) ? o.get(k) : 0;\n        int d = m.containsKey(k) ? m.get(\"x\") : 0;\n        return a;\n    }\n}\n";
        let run = |at: &str| get_or_default(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("m.containsKey(k) ? m.get(k)").unwrap();
        assert_eq!(title, "Replace with 'getOrDefault' method call");
        assert!(out.contains("        int a = m.getOrDefault(k, 0);\n"), "{out}");
        assert!(run("!m.containsKey").unwrap().1.contains("        int b = m.getOrDefault(k, -1);\n"));
        // Another map; another key.
        assert_eq!(run("m.containsKey(k) ? o.get"), None);
        assert_eq!(run("m.containsKey(k) ? m.get(\"x"), None);
    }

    #[test]
    fn replaces_count_with_size() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, s: String, q: Sequence<Int>, m: Map<String, Int>, n: List<Int>?) {\n    val a = xs.count()\n    val b = s.count()\n    val c = q.count()\n    val d = m.count()\n    val e = xs.count { it > 1 }\n    val g = n?.count()\n}\n";
        let run = |at: &str| count_to_size(text, text.find(at).unwrap() + 3, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("xs.count()").unwrap();
        assert_eq!(title, "Replace 'count' with 'size'");
        assert!(out.contains("    val a = xs.size\n"), "{out}");
        let (title, out) = run(" s.count()").unwrap();
        assert_eq!(title, "Replace 'count' with 'length'");
        assert!(out.contains("    val b = s.length\n"), "{out}");
        assert!(run("m.count()").unwrap().1.contains("    val d = m.size\n"));
        // A sequence iterates; a predicate; nullable.
        assert_eq!(run("q.count()"), None);
        assert_eq!(run("xs.count {"), None);
        assert_eq!(run("n?.count"), None);
    }

    #[test]
    fn replaces_size_minus_one_with_last_index() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, a: IntArray, s: String, set: Set<Int>, n: Int) {\n    val i = xs.size - 1\n    val j = a.size - 1\n    val k = s.length - 1\n    val l = set.size - 1\n    val m = n - 1\n    val o = xs.size - 2\n}\n";
        let run = |at: &str| last_index(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("xs.size - 1").unwrap();
        assert_eq!(title, "Replace with 'lastIndex'");
        assert!(out.contains("    val i = xs.lastIndex\n"), "{out}");
        assert!(run("a.size").unwrap().1.contains("    val j = a.lastIndex\n"));
        assert!(run("s.length").unwrap().1.contains("    val k = s.lastIndex\n"));
        // A set has no lastIndex; not size; not 1.
        assert_eq!(run("set.size"), None);
        assert_eq!(run("n - 1"), None);
        assert_eq!(run("xs.size - 2"), None);
    }

    #[test]
    fn replaces_manual_ranges_with_indices() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "fun f(xs: List<Int>, s: String, a: IntArray, n: Int) {\n    for (i in 0 until xs.size) println(xs[i])\n    for (i in 0..s.length - 1) println(s[i])\n    for (i in 0..a.size - 2) println(a[i])\n    for (i in 1 until a.size) println(a[i])\n    for (i in 0 until n) println(i)\n    for (i in 0 until a.size) println(a[i])\n}\n";
        let run = |at: &str| range_to_indices(text, text.find(at).unwrap(), &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = run("until xs").unwrap();
        assert_eq!(title, "Replace with indices");
        assert!(out.contains("    for (i in xs.indices) println(xs[i])\n"), "{out}");
        assert!(run("0..s").unwrap().1.contains("    for (i in s.indices) println(s[i])\n"));
        assert!(run("until a.size) println(a[i])\n}").unwrap().1.contains("    for (i in a.indices) println(a[i])\n}"));
        // Not the last index; not from 0; not a size.
        assert_eq!(run("0..a"), None);
        assert_eq!(run("1 until"), None);
        assert_eq!(run("until n"), None);
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
    fn replaces_an_indexed_loop_with_for_each() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let text = "class A {\n    void f(java.util.List<String> names, int[] ns, java.util.List<String> other) {\n        for (int i = 0; i < names.size(); i++) {\n            String s = names.get(i);\n            print(s);\n        }\n        for (int j = 0; j < ns.length; j++) {\n            total += ns[j] * ns[ j ];\n        }\n        for (int k = 0; k < other.size(); k++) {\n            print(k + other.get(k));\n        }\n    }\n}\n";
        let at = |s: &str| text.find(s).unwrap();
        let i = enhanced_for_loop(text, at("int i"), &classes).unwrap();
        assert_eq!(i.title, "Replace with enhanced 'for'");
        assert!(apply(text, &i).contains("        for (String s : names) {\n            print(s);\n        }\n"), "{}", apply(text, &i));
        let out = apply(text, &enhanced_for_loop(text, at("int j"), &classes).unwrap());
        assert!(out.contains("        for (int n : ns) {\n            total += n * n;\n        }\n"), "{out}");
        // The index used on its own.
        assert_eq!(enhanced_for_loop(text, at("int k"), &classes), None);
        // Back and forth with the indexed loop.
        let each = "class A {\n    void f(java.util.List<String> names) {\n        for (String name : names) {\n            print(name);\n        }\n    }\n}\n";
        let indexed = apply(each, &indexed_for_loop(each, each.find("name :").unwrap(), &classes).unwrap());
        assert_eq!(apply(&indexed, &enhanced_for_loop(&indexed, indexed.find("int i").unwrap(), &classes).unwrap()), each);
    }

    #[test]
    fn replaces_size_checks() {
        let classes = |_: &str| -> Vec<String> { Vec::new() };
        let kt = "fun f(l: List<Int>, s: String, a: IntArray, n: Int, m: List<Int>?) {\n    if (l.size == 0 || 0 < s.length || a.size >= 1 || n == 0 || l.size == 2) {}\n    m?.size == 0\n}\n";
        let at = |text: &str, s: &str| text.find(s).unwrap();
        let get = |text: &str, s: &str, kotlin: bool| size_check(text, at(text, s), kotlin, &classes).map(|i| (i.title.clone(), apply(text, &i)));
        let (title, out) = get(kt, "l.size == 0", true).unwrap();
        assert_eq!(title, "Replace size zero check with 'isEmpty()'");
        assert!(out.contains("if (l.isEmpty() || 0 < s.length"));
        let (title, out) = get(kt, "s.length", true).unwrap();
        assert_eq!(title, "Replace size check with 'isNotEmpty()'");
        assert!(out.contains("|| s.isNotEmpty() ||"));
        assert!(get(kt, "a.size", true).unwrap().1.contains("|| a.isNotEmpty() ||"));
        assert_eq!(get(kt, "n == 0", true), None);
        assert_eq!(get(kt, "l.size == 2", true), None);
        assert_eq!(get(kt, "m?.size", true), None);
        let java = "class A {\n    void f(java.util.List<String> l, String s, int[] a) {\n        if (l.size() == 0 || s.length() > 0 || a.length == 0) {}\n    }\n}\n";
        let (title, out) = get(java, "l.size()", false).unwrap();
        assert_eq!(title, "Replace with 'isEmpty()'");
        assert!(out.contains("if (l.isEmpty() ||"));
        let (title, out) = get(java, "s.length()", false).unwrap();
        assert_eq!(title, "Replace with '!isEmpty()'");
        assert!(out.contains("|| !s.isEmpty() ||"));
        assert_eq!(get(java, "a.length", false), None);
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

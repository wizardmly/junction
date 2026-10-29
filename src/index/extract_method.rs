//! Extract Method's view of the syntax tree (Kotlin and Java), as
//! IntelliJ's `ExtractMethodProcessor` / Kotlin's `ExtractionData` see the
//! selection: whole statements of one block or one expression; the
//! variables declared before it that it reads (the parameters, with their
//! declared or literal types), the one declared or assigned in it that the
//! code after it reads (the result), and the jumps it can't take along.

use std::ops::Range;

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;
use crate::domain::extract_method::{Duplicate, MethodPlan, Output, Parameter};

const NOT_STATEMENTS: &str = "Selected block should represent a set of statements or an expression";
const NO_FUNCTION: &str = "Cannot extract a method from here: the selection is not in a method";
const MULTIPLE_OUTPUTS: &str = "There are multiple output values for the selected code fragment";
const RETURN_INSIDE: &str = "The selection contains a return that doesn't end the method";
const JUMP_OUTSIDE: &str = "The selection contains a break or continue whose loop is outside it";

/// A variable declared in the method.
#[derive(Debug)]
pub(super) struct Decl {
    pub(super) name: String,
    pub(super) ty: String,
    /// The end of its name: references after it see it.
    pub(super) at: usize,
    /// Where it's visible.
    pub(super) scope: Range<usize>,
    pub(super) mutable: bool,
}

pub(super) fn children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

pub(super) fn descendants<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(children(n).into_iter().rev());
    }
    out
}

pub(super) fn inside(inner: &Range<usize>, outer: &Range<usize>) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

pub(super) fn is_function(lang: Lang, node: Node) -> bool {
    match lang {
        Lang::Kotlin => node.kind() == "function_declaration",
        _ => matches!(node.kind(), "method_declaration" | "constructor_declaration"),
    }
}

/// Lambdas, local functions and classes: their returns aren't the method's.
fn is_nested_body(lang: Lang, node: Node) -> bool {
    match lang {
        Lang::Kotlin => matches!(node.kind(), "lambda_literal" | "anonymous_function" | "function_declaration" | "class_declaration" | "object_literal"),
        _ => matches!(node.kind(), "lambda_expression" | "class_body" | "method_declaration"),
    }
}

/// A Kotlin expression's type when it shows it.
pub(super) fn kotlin_type(node: Node, text: &str, known: &dyn Fn(&str) -> String) -> String {
    let t = &text[node.byte_range()];
    match node.kind() {
        "integer_literal" | "hex_literal" | "bin_literal" => "Int",
        "long_literal" => "Long",
        "real_literal" => {
            if t.ends_with(['f', 'F']) {
                "Float"
            } else {
                "Double"
            }
        }
        "boolean_literal" | "comparison_expression" | "equality_expression" | "conjunction_expression" | "disjunction_expression" | "check_expression" => "Boolean",
        "string_literal" => "String",
        "character_literal" => "Char",
        "prefix_expression" if t.starts_with('!') => "Boolean",
        "parenthesized_expression" => return node.named_child(0).map(|n| kotlin_type(n, text, known)).unwrap_or_default(),
        // A variable: its declared type.
        "simple_identifier" => return known(t),
        // `Point(1, 2)`: a constructor.
        "call_expression" => {
            let callee = node.named_child(0).filter(|c| c.kind() == "simple_identifier").map(|c| &text[c.byte_range()]).unwrap_or_default();
            return if callee.starts_with(|c: char| c.is_uppercase()) { callee.to_owned() } else { String::new() };
        }
        "additive_expression" | "multiplicative_expression" => {
            let types: Vec<String> = children(node).iter().map(|c| kotlin_type(*c, text, known)).collect();
            return if types.iter().any(|t| t == "String") && node.kind() == "additive_expression" {
                "String".into()
            } else if types.iter().all(|t| t == "Int") {
                "Int".into()
            } else if types.iter().all(|t| matches!(t.as_str(), "Int" | "Double")) {
                "Double".into()
            } else {
                String::new()
            };
        }
        _ => "",
    }
    .to_owned()
}

/// The declarations in the method, Kotlin.
pub(super) fn kotlin_decls(function: Node, text: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    fn name_of(n: Node) -> Option<Node> {
        children(n).into_iter().find(|c| c.kind() == "simple_identifier")
    }
    let type_after = |n: Node| children(n).into_iter().skip_while(|c| c.kind() != "simple_identifier").nth(1).map(|t| text[t.byte_range()].to_owned()).unwrap_or_default();
    for n in descendants(function) {
        let parent = n.parent();
        match n.kind() {
            "parameter" => {
                if let Some(name) = name_of(n) {
                    // This method's or a nested function's: visible in that function.
                    let scope = n.parent().and_then(|p| p.parent()).map_or(function.byte_range(), |f| f.byte_range());
                    out.push(Decl { name: text[name.byte_range()].into(), ty: type_after(n), at: name.end_byte(), scope, mutable: false });
                }
            }
            "variable_declaration" => {
                let Some(name) = name_of(n) else { continue };
                let Some(parent) = parent else { continue };
                let mut ty = type_after(n);
                let (scope, mutable) = match parent.kind() {
                    "property_declaration" => {
                        let mutable = children(parent).first().is_some_and(|k| &text[k.byte_range()] == "var");
                        if ty.is_empty() {
                            // The initializer's type when it shows it.
                            if let Some(init) = children(parent).into_iter().skip_while(|c| c.id() != n.id()).nth(1) {
                                ty = kotlin_type(init, text, &|name| out.iter().rev().find(|d: &&Decl| d.name == name).map(|d| d.ty.clone()).unwrap_or_default());
                            }
                        }
                        (parent.parent().map_or(parent.byte_range(), |c| parent.start_byte()..c.end_byte()), mutable)
                    }
                    "for_statement" => {
                        if ty.is_empty() && children(parent).iter().any(|c| c.kind() == "range_expression") {
                            ty = "Int".into();
                        }
                        (parent.byte_range(), false)
                    }
                    // A lambda's parameters.
                    "lambda_parameters" => (parent.parent().map_or(parent.byte_range(), |l| l.byte_range()), false),
                    _ => continue,
                };
                out.push(Decl { name: text[name.byte_range()].into(), ty, at: name.end_byte(), scope, mutable });
            }
            "catch_block" => {
                if let Some(name) = name_of(n) {
                    out.push(Decl { name: text[name.byte_range()].into(), ty: type_after(n), at: name.end_byte(), scope: n.byte_range(), mutable: false });
                }
            }
            _ => {}
        }
    }
    out
}

/// The declarations in the method, Java.
pub(super) fn java_decls(function: Node, text: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    let field = |n: Node, f: &str| n.child_by_field_name(f).map(|c| text[c.byte_range()].to_owned()).unwrap_or_default();
    for n in descendants(function) {
        match n.kind() {
            "formal_parameter" | "spread_parameter" => {
                if let Some(name) = n.child_by_field_name("name").or_else(|| children(n).into_iter().rev().find(|c| c.kind() == "identifier")) {
                    let scope = n.parent().and_then(|p| p.parent()).map_or(function.byte_range(), |f| f.byte_range());
                    out.push(Decl { name: text[name.byte_range()].into(), ty: field(n, "type"), at: name.end_byte(), scope, mutable: true });
                }
            }
            "local_variable_declaration" => {
                let ty = field(n, "type");
                let ty = if ty == "var" { String::new() } else { ty };
                let Some(parent) = n.parent() else { continue };
                let scope = if parent.kind() == "for_statement" { parent.byte_range() } else { n.start_byte()..parent.end_byte() };
                for d in children(n).into_iter().filter(|c| c.kind() == "variable_declarator") {
                    if let Some(name) = d.child_by_field_name("name") {
                        let dims = d.child_by_field_name("dimensions").map(|x| text[x.byte_range()].to_owned()).unwrap_or_default();
                        out.push(Decl { name: text[name.byte_range()].into(), ty: format!("{ty}{dims}"), at: name.end_byte(), scope: scope.clone(), mutable: true });
                    }
                }
            }
            "enhanced_for_statement" => {
                if let Some(name) = n.child_by_field_name("name") {
                    out.push(Decl { name: text[name.byte_range()].into(), ty: field(n, "type"), at: name.end_byte(), scope: n.byte_range(), mutable: true });
                }
            }
            "catch_formal_parameter" => {
                if let Some(name) = n.child_by_field_name("name") {
                    let ty = children(n).into_iter().find(|c| c.kind() == "catch_type").map(|c| text[c.byte_range()].to_owned()).unwrap_or_default();
                    let scope = n.parent().map_or(n.byte_range(), |c| c.byte_range());
                    out.push(Decl { name: text[name.byte_range()].into(), ty, at: name.end_byte(), scope, mutable: true });
                }
            }
            "lambda_expression" => {
                if let Some(params) = n.child_by_field_name("parameters") {
                    let names: Vec<Node> = if params.kind() == "identifier" { vec![params] } else { children(params).into_iter().filter(|c| c.kind() == "identifier").collect() };
                    for name in names {
                        out.push(Decl { name: text[name.byte_range()].into(), ty: String::new(), at: name.end_byte(), scope: n.byte_range(), mutable: true });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A name read or written as a variable (not a member after `.`, a
/// method's name, a declared name, a named argument).
pub(super) fn is_reference(lang: Lang, node: Node, text: &str) -> bool {
    let Some(parent) = node.parent() else { return false };
    match lang {
        Lang::Kotlin => {
            matches!(node.kind(), "simple_identifier" | "interpolated_identifier")
                && !matches!(parent.kind(), "navigation_suffix" | "variable_declaration" | "parameter" | "function_declaration" | "class_declaration" | "catch_block" | "label" | "type_identifier")
                && !(parent.kind() == "value_argument" && node.next_sibling().is_some_and(|s| s.kind() == "="))
                && !text[node.byte_range()].starts_with('@')
        }
        _ => {
            node.kind() == "identifier"
                && !["name", "field", "label"].iter().any(|f| parent.child_by_field_name(f).is_some_and(|c| c.id() == node.id()) && parent.kind() != "assignment_expression")
                && !matches!(parent.kind(), "inferred_parameters" | "lambda_expression" | "catch_formal_parameter" | "labeled_statement" | "break_statement" | "continue_statement")
        }
    }
}

/// Whether the reference is written: assigned, incremented.
pub(super) fn is_assigned(lang: Lang, node: Node, text: &str) -> bool {
    let Some(parent) = node.parent() else { return false };
    match lang {
        Lang::Kotlin => {
            (parent.kind() == "directly_assignable_expression" && parent.parent().is_some_and(|a| a.kind() == "assignment"))
                || (matches!(parent.kind(), "postfix_expression" | "prefix_expression") && {
                    let t = &text[parent.byte_range()];
                    t.contains("++") || t.contains("--")
                })
        }
        _ => {
            (parent.kind() == "assignment_expression" && parent.child_by_field_name("left").is_some_and(|l| l.id() == node.id())) || parent.kind() == "update_expression"
        }
    }
}

/// The method's declared result type.
fn function_type(lang: Lang, function: Node, text: &str) -> String {
    match lang {
        Lang::Kotlin => {
            let named = children(function);
            let params = named.iter().position(|c| c.kind() == "function_value_parameters");
            params
                .and_then(|i| named.get(i + 1))
                .filter(|t| !matches!(t.kind(), "function_body" | "type_constraints"))
                .map(|t| text[t.byte_range()].to_owned())
                .unwrap_or_default()
        }
        _ => function.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned()).unwrap_or_default(),
    }
}

/// The selected statements: whole children of one block.
fn statements<'t>(lang: Lang, function: Node<'t>, range: &Range<usize>) -> Option<Vec<Node<'t>>> {
    let mut node = function.descendant_for_byte_range(range.start, range.end)?;
    loop {
        let container = match lang {
            Lang::Kotlin => node.kind() == "statements",
            _ => matches!(node.kind(), "block" | "constructor_body"),
        };
        if container {
            let overlapping: Vec<Node> = children(node).into_iter().filter(|c| c.end_byte() > range.start && c.start_byte() < range.end).collect();
            if !overlapping.is_empty() && overlapping.iter().all(|c| inside(&c.byte_range(), range)) {
                return Some(overlapping);
            }
        }
        if node.id() == function.id() {
            return None;
        }
        node = node.parent()?;
    }
}

/// Extract Method for the selection.
pub fn plan(lang: Lang, text: &str, selection: Range<usize>) -> Result<MethodPlan, String> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err("Extract Method works in Kotlin and Java files".into());
    }
    // The selection without its surrounding blanks.
    let selected = &text[selection.clone()];
    let start = selection.start + (selected.len() - selected.trim_start().len());
    let end = (selection.start + selected.trim_end().len()).max(start);
    if start == end {
        return Err("Select the statements or the expression to extract".into());
    }
    let tree = symbols::parse(lang, text).ok_or(NOT_STATEMENTS)?;
    let root = tree.root_node();
    let mut function = root.descendant_for_byte_range(start, end).ok_or(NO_FUNCTION)?;
    while !is_function(lang, function) {
        function = function.parent().ok_or(NO_FUNCTION)?;
    }
    let (range, expression, expression_node) = match statements(lang, function, &(start..end)) {
        Some(nodes) => (nodes[0].start_byte()..nodes[nodes.len() - 1].end_byte(), false, None),
        None => {
            let node = super::introduce::expression_node(lang, root, text, start..end).ok_or(NOT_STATEMENTS)?;
            (node.byte_range(), true, Some(node))
        }
    };
    let decls = match lang {
        Lang::Kotlin => kotlin_decls(function, text),
        _ => java_decls(function, text),
    };
    // A reference's declaration: the latest visible one with its name.
    let resolve = |name: &str, at: usize| decls.iter().enumerate().filter(|(_, d)| d.name == name && d.at <= at && inside(&(at..at), &d.scope)).max_by_key(|(_, d)| d.at).map(|(i, _)| i);
    let refs: Vec<(usize, Range<usize>, bool)> = descendants(function)
        .into_iter()
        .filter(|n| is_reference(lang, *n, text))
        .filter_map(|n| {
            let name = text[n.byte_range()].trim_start_matches('$');
            Some((resolve(name, n.start_byte())?, n.byte_range(), is_assigned(lang, n, text)))
        })
        .collect();
    // Parameters: declared before the selection, used in it.
    let mut parameters: Vec<(usize, Parameter)> = Vec::new();
    for (decl, r, assigned) in refs.iter().filter(|(_, r, _)| inside(r, &range)) {
        let d = &decls[*decl];
        if d.at > range.start {
            continue;
        }
        let rel = r.start - range.start..r.end - range.start;
        let rel = if text[r.clone()].starts_with('$') { rel.start + 1..rel.end } else { rel };
        match parameters.iter_mut().find(|(i, _)| i == decl) {
            Some((_, p)) => {
                p.uses.push(rel);
                p.assigned |= *assigned;
            }
            None => parameters.push((*decl, Parameter { name: d.name.clone(), ty: d.ty.clone(), uses: vec![rel], assigned: *assigned })),
        }
    }
    // The result: what the code after the selection reads of it.
    let mut outputs: Vec<Output> = Vec::new();
    if !expression {
        let after = |decl: usize| refs.iter().any(|(d, r, _)| *d == decl && r.start >= range.end && r.end <= function.end_byte());
        for (i, d) in decls.iter().enumerate() {
            if inside(&(d.at..d.at), &range) && d.scope.end > range.end && after(i) {
                outputs.push(Output::Declared { name: d.name.clone(), ty: d.ty.clone(), mutable: d.mutable });
            }
        }
        for (i, p) in &parameters {
            if p.assigned && after(*i) {
                outputs.push(Output::Assigned { name: p.name.clone(), ty: p.ty.clone() });
            }
        }
    }
    if outputs.len() > 1 {
        return Err(MULTIPLE_OUTPUTS.into());
    }
    // Jumps out of the selection.
    let mut returns = None;
    let selection_nodes: Vec<Node> = descendants(function).into_iter().filter(|n| inside(&n.byte_range(), &range)).collect();
    let nested = |n: Node| {
        let mut p = n.parent();
        while let Some(x) = p {
            if !inside(&x.byte_range(), &range) {
                return false;
            }
            if is_nested_body(lang, x) {
                return true;
            }
            p = x.parent();
        }
        false
    };
    for n in &selection_nodes {
        let t = &text[n.byte_range()];
        let (is_return, is_jump) = match lang {
            Lang::Kotlin if n.kind() == "jump_expression" => (t.starts_with("return") && !t.starts_with("return@"), t.starts_with("break") || t.starts_with("continue")),
            Lang::Java => (n.kind() == "return_statement", matches!(n.kind(), "break_statement" | "continue_statement")),
            _ => (false, false),
        };
        if is_return && !nested(*n) {
            // Only a return ending the method: the call is returned.
            let last_in_method = n.end_byte() == range.end && {
                let body_end = text[..function.end_byte()].trim_end().trim_end_matches('}').trim_end().len();
                range.end >= body_end
            };
            if !last_in_method || expression || returns.is_some() {
                return Err(RETURN_INSIDE.into());
            }
            returns = Some(function_type(lang, function, text));
        }
        if is_jump && !t.contains('@') {
            let mut p = n.parent();
            let mut target_inside = false;
            while let Some(x) = p {
                if matches!(x.kind(), "for_statement" | "while_statement" | "do_while_statement" | "do_statement" | "enhanced_for_statement" | "switch_expression" | "switch_block") {
                    target_inside = inside(&x.byte_range(), &range);
                    break;
                }
                p = x.parent();
            }
            if !target_inside {
                return Err(JUMP_OUTSIDE.into());
            }
        }
    }
    if returns.is_some() && !outputs.is_empty() {
        return Err(MULTIPLE_OUTPUTS.into());
    }
    let expression_type = match (lang, expression_node) {
        (Lang::Java, Some(n)) => super::introduce::java_type(n, text).unwrap_or_default(),
        (Lang::Kotlin, Some(n)) => kotlin_type(n, text, &|name| resolve(name, n.start_byte()).map(|i| decls[i].ty.clone()).unwrap_or_default()),
        _ => String::new(),
    };
    let line_start = text[..function.start_byte()].rfind('\n').map_or(0, |i| i + 1);
    let member_indent: String = text[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let is_static = lang == Lang::Java && children(function).iter().any(|c| c.kind() == "modifiers" && text[c.byte_range()].split_whitespace().any(|w| w == "static"));
    let taken = descendants(root)
        .into_iter()
        .filter(|n| is_function(lang, *n))
        .filter_map(|f| match lang {
            Lang::Kotlin => children(f).into_iter().find(|c| c.kind() == "simple_identifier").map(|c| text[c.byte_range()].to_owned()),
            _ => f.child_by_field_name("name").map(|c| text[c.byte_range()].to_owned()),
        })
        .collect();
    Ok(MethodPlan {
        range,
        expression,
        parameters: parameters.into_iter().map(|(_, p)| p).collect(),
        output: outputs.pop(),
        returns,
        expression_type,
        insert_at: function.end_byte(),
        member_indent,
        is_static,
        taken,
    })
}

/// The other places in the file with the same code as the plan's (as
/// IntelliJ's `DuplicatesFinder` finds them): the same syntax, the
/// parameters standing for any expression (the same one each time),
/// in a method of the same class that can call the new one, and not
/// leaving a variable the code after them reads undeclared.
pub fn duplicates(lang: Lang, text: &str, plan: &MethodPlan, is_static: bool) -> Vec<Duplicate> {
    // A returned call can't stand anywhere else; a written parameter other
    // than the result would go unwritten.
    if plan.returns.is_some() || plan.parameters.iter().any(|p| p.assigned && plan.output.as_ref().is_none_or(|o| !matches!(o, Output::Assigned { name, .. } if *name == p.name))) {
        return Vec::new();
    }
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let root = tree.root_node();
    let Some(mut function) = root.descendant_for_byte_range(plan.range.start, plan.range.end) else { return Vec::new() };
    while !is_function(lang, function) {
        let Some(p) = function.parent() else { return Vec::new() };
        function = p;
    }
    let pattern: Vec<Node> = if plan.expression {
        match super::introduce::expression_node(lang, root, text, plan.range.clone()) {
            Some(n) => vec![n],
            None => return Vec::new(),
        }
    } else {
        match statements(lang, function, &plan.range) {
            Some(nodes) => nodes,
            None => return Vec::new(),
        }
    };
    // A lone parameter isn't worth a call.
    if plan.expression && plan.parameters.iter().any(|p| p.uses.iter().any(|u| u.len() == plan.range.len())) {
        return Vec::new();
    }
    let class = |n: Node| {
        let mut p = n.parent();
        while let Some(x) = p {
            if matches!(x.kind(), "class_body" | "enum_class_body" | "enum_body" | "interface_body") {
                return Some(x.id());
            }
            p = x.parent();
        }
        None
    };
    let home = class(function);
    // Candidates: an expression of the same kind, or as many statements
    // in a row of one block.
    let mut candidates: Vec<Vec<Node>> = Vec::new();
    for n in descendants(root) {
        if plan.expression {
            if n.kind() == pattern[0].kind() {
                candidates.push(vec![n]);
            }
        } else if n.kind() == pattern[0].parent().map_or("", |p| p.kind()) {
            let kids = children(n);
            for w in kids.windows(pattern.len()) {
                candidates.push(w.to_vec());
            }
        }
    }
    let mut out: Vec<Duplicate> = Vec::new();
    for c in candidates {
        let range = c[0].start_byte()..c[c.len() - 1].end_byte();
        if range.start < plan.range.end && plan.range.start < range.end || out.iter().any(|d| range.start < d.range.end && d.range.start < range.end) {
            continue;
        }
        let Some(f) = enclosing(lang, c[0]) else { continue };
        if class(f) != home {
            continue;
        }
        if lang == Lang::Java && !is_static && children(f).iter().any(|m| m.kind() == "modifiers" && text[m.byte_range()].split_whitespace().any(|w| w == "static")) {
            continue;
        }
        let mut bound: Vec<Option<String>> = vec![None; plan.parameters.len()];
        if !pattern.iter().zip(&c).all(|(p, n)| same(lang, text, plan, *p, *n, &mut bound)) {
            continue;
        }
        let args: Vec<String> = bound.into_iter().map(Option::unwrap_or_default).collect();
        let assigned = match &plan.output {
            Some(Output::Assigned { name, .. }) => plan.parameters.iter().position(|p| &p.name == name).map(|i| args[i].clone()),
            _ => None,
        };
        // The variables declared in it that the code after it reads: only
        // the result can be.
        if !plan.expression {
            let decls = match lang {
                Lang::Kotlin => kotlin_decls(f, text),
                _ => java_decls(f, text),
            };
            let result = match &plan.output {
                Some(Output::Declared { name, .. }) => Some(name.as_str()),
                _ => None,
            };
            let read_after = |d: &Decl| {
                descendants(f).into_iter().any(|n| n.start_byte() >= range.end && n.end_byte() <= d.scope.end && is_reference(lang, n, text) && text[n.byte_range()].trim_start_matches('$') == d.name)
            };
            if decls.iter().any(|d| inside(&(d.at..d.at), &range) && Some(d.name.as_str()) != result && read_after(d)) {
                continue;
            }
        }
        out.push(Duplicate { range, args, assigned });
    }
    out
}

/// The method a node is in.
fn enclosing(lang: Lang, node: Node) -> Option<Node> {
    let mut f = node;
    while !is_function(lang, f) {
        f = f.parent()?;
    }
    Some(f)
}

/// Whether the candidate has the pattern's syntax, binding each parameter
/// use to the candidate's expression there (the same one each time).
fn same(lang: Lang, text: &str, plan: &MethodPlan, p: Node, c: Node, bound: &mut [Option<String>]) -> bool {
    if is_reference(lang, p, text) {
        let start = p.start_byte() + usize::from(text[p.byte_range()].starts_with('$')) - plan.range.start;
        let rel = start..p.end_byte() - plan.range.start;
        if let Some(i) = plan.parameters.iter().position(|q| q.uses.contains(&rel)) {
            // In a string template or written: only a variable.
            let variable = p.kind() == "interpolated_identifier" || plan.parameters[i].assigned;
            if variable && c.kind() != p.kind() {
                return false;
            }
            if !c.is_named() || c.kind().ends_with("comment") {
                return false;
            }
            let arg = text[c.byte_range()].trim_start_matches(|ch| variable && ch == '$');
            let arg: String = arg.split_whitespace().collect::<Vec<_>>().join(" ");
            return match &bound[i] {
                Some(b) => *b == arg,
                None => {
                    bound[i] = Some(arg);
                    true
                }
            };
        }
    }
    if p.kind() != c.kind() || p.child_count() != c.child_count() {
        return false;
    }
    if p.child_count() == 0 {
        return text[p.byte_range()] == text[c.byte_range()];
    }
    let mut pc = p.walk();
    let mut cc = c.walk();
    let (ps, cs): (Vec<Node>, Vec<Node>) = (p.children(&mut pc).collect(), c.children(&mut cc).collect());
    ps.into_iter().zip(cs).all(|(a, b)| same(lang, text, plan, a, b, bound))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KT: &str = "class A {\n    fun f(n: Int, s: String): Int {\n        val a = n + 1\n        var b = 2\n        b += a\n        val unused = s.length\n        for (i in 0..n) { println(i + b) }\n        return a * b\n    }\n}\n";

    fn span(text: &str, from: &str, to: &str) -> Range<usize> {
        let start = text.find(from).unwrap();
        start..text[start..].find(to).unwrap() + start + to.len()
    }

    #[test]
    fn plans_kotlin() {
        // `b += a`: reads a and b, assigns b, used after: b is the result.
        let plan = plan(Lang::Kotlin, KT, span(KT, "b += a", "b += a")).unwrap();
        assert_eq!(plan.parameters.iter().map(|p| (p.name.as_str(), p.ty.as_str(), p.assigned)).collect::<Vec<_>>(), [("b", "Int", true), ("a", "Int", false)]);
        assert_eq!(plan.output, Some(Output::Assigned { name: "b".into(), ty: "Int".into() }));
        assert_eq!(plan.member_indent, "    ");
        assert_eq!(plan.taken, ["f"]);
        // From `val a` through `b += a`: a and b are declared inside; both used after.
        assert_eq!(super::plan(Lang::Kotlin, KT, span(KT, "val a", "b += a")).unwrap_err(), MULTIPLE_OUTPUTS);
        // The loop: n and b in, nothing out.
        let loop_plan = super::plan(Lang::Kotlin, KT, span(KT, "for (i", "b) }")).unwrap();
        assert_eq!(loop_plan.parameters.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["n", "b"]);
        assert_eq!(loop_plan.output, None);
        // The last statements return: the call is returned with f's type.
        let tail = super::plan(Lang::Kotlin, KT, span(KT, "for (i", "a * b")).unwrap();
        assert_eq!(tail.returns.as_deref(), Some("Int"));
        // A return in the middle can't go.
        let text = "fun f(x: Int): Int {\n    if (x > 0) return 1\n    println(x)\n    return 0\n}\n";
        assert_eq!(super::plan(Lang::Kotlin, text, span(text, "if (x", "println(x)")).unwrap_err(), RETURN_INSIDE);
        // An expression; a partial statement is neither.
        let e = super::plan(Lang::Kotlin, KT, span(KT, "n + 1", "n + 1")).unwrap();
        assert!(e.expression && e.expression_type == "Int" && e.parameters.len() == 1);
        assert_eq!(super::plan(Lang::Kotlin, KT, span(KT, "1\n        var", "var b")).unwrap_err(), NOT_STATEMENTS);
        // A string template's variable is a parameter.
        let text = "fun f(name: String) {\n    println(\"Hi $name\")\n}\n";
        let t = super::plan(Lang::Kotlin, text, span(text, "println", "\")")).unwrap();
        assert_eq!(t.parameters.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["name"]);
    }

    #[test]
    fn plans_java() {
        let text = "class A {\n  static int f(int n, String s) {\n    int a = n + 1;\n    long b = 2;\n    for (int i = 0; i < n; i++) { if (i > 3) break; b += i; }\n    return (int) b + a;\n  }\n}\n";
        let p = plan(Lang::Java, text, span(text, "long b", "b += i; }")).unwrap();
        assert_eq!(p.parameters.iter().map(|p| (p.name.as_str(), p.ty.as_str())).collect::<Vec<_>>(), [("n", "int")]);
        assert_eq!(p.output, Some(Output::Declared { name: "b".into(), ty: "long".into(), mutable: true }));
        assert!(p.is_static);
        // A break whose loop is outside.
        assert_eq!(plan(Lang::Java, text, span(text, "if (i > 3) break;", "break;")).unwrap_err(), JUMP_OUTSIDE);
        let e = plan(Lang::Java, text, span(text, "n + 1", "n + 1")).unwrap();
        assert!(e.expression && e.parameters.len() == 1);
        assert_eq!(plan(Lang::Java, text, 0..5).unwrap_err(), NO_FUNCTION);
    }

    #[test]
    fn finds_duplicates() {
        use crate::domain::extract_method::{extract, ExtractRequest};
        use crate::domain::imports::ImportStyle;
        // Kotlin statements with a result: the other copy's parameter is
        // another expression; a copy whose local is read after isn't one.
        let text = "class A {\n    fun f(n: Int) {\n        val a = n + 1\n        val b = a * 2\n        println(b)\n    }\n\n    fun g(m: Int) {\n        val a = (m - 1) + 1\n        val b = a * 2\n        println(b + a)\n    }\n\n    fun h(k: Int) {\n        val a = k + 1\n        val b = a * 2\n        println(b)\n    }\n}\n";
        let p = plan(Lang::Kotlin, text, span(text, "val a = n", "a * 2")).unwrap();
        let d = duplicates(Lang::Kotlin, text, &p, false);
        assert_eq!(d.iter().map(|d| d.args.clone()).collect::<Vec<_>>(), [vec!["k".to_owned()]]);
        let request = ExtractRequest { name: "twice".into(), visibility: "private".into(), is_static: false, parameters: vec![("n".into(), "Int".into())], result_type: String::new(), order: Vec::new() };
        let e = extract(ImportStyle::Kotlin, text, &p, &request, "    ", &d).unwrap();
        let out = format!("{}{}{}", &text[..e.range.start], e.text, &text[e.range.end..]);
        assert!(out.contains("    fun h(k: Int) {\n        val b = twice(k)\n        println(b)"), "{out}");
        assert!(out.contains("val a = (m - 1) + 1"));
        assert_eq!(&out[e.select.clone()], "twice");
        // An expression: the same parameter must stand for the same thing.
        let text = "class A {\n  int f(int x, int y) {\n    return x * x + 1;\n  }\n  int g(int z) {\n    int q = (z + 2) * (z + 2) + 1;\n    return z * y() + 1 + q;\n  }\n  static int s(int w) { return w * w + 1; }\n  int y() { return 0; }\n}\n";
        let p = plan(Lang::Java, text, span(text, "x * x + 1", "x * x + 1")).unwrap();
        let d = duplicates(Lang::Java, text, &p, false);
        assert_eq!(d.iter().map(|d| (&text[d.range.clone()], d.args.clone())).collect::<Vec<_>>(), [("(z + 2) * (z + 2) + 1", vec!["(z + 2)".to_owned()])]);
        // A static method can call a static one.
        assert_eq!(duplicates(Lang::Java, text, &p, true).len(), 2);
        // An assigned result goes to the copy's variable.
        let text = "fun f() {\n    var s = 0\n    s += 2\n    println(s)\n    var t = 1\n    t += 2\n    println(t)\n}\n";
        let p = plan(Lang::Kotlin, text, span(text, "s += 2", "s += 2")).unwrap();
        let d = duplicates(Lang::Kotlin, text, &p, false);
        assert_eq!(d.iter().map(|d| d.assigned.clone()).collect::<Vec<_>>(), [Some("t".to_owned())]);
        let request = ExtractRequest { name: "add".into(), visibility: "private".into(), is_static: false, parameters: vec![("s".into(), "Int".into())], result_type: String::new(), order: Vec::new() };
        let e = extract(ImportStyle::Kotlin, text, &p, &request, "    ", &d).unwrap();
        let out = format!("{}{}{}", &text[..e.range.start], e.text, &text[e.range.end..]);
        assert!(out.starts_with("fun f() {\n    var s = 0\n    s = add(s)\n    println(s)\n    var t = 1\n    t = add(t)\n    println(t)\n}\n\nprivate fun add(s: Int): Int {"), "{out}");
    }
}

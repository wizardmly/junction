//! Inline Function's view of the syntax tree (Kotlin and Java): the
//! function named at the caret (its declaration or a call in the same
//! file), the one expression its body is (or its statements), and the
//! parameters and members it uses, as Introduce Parameter analyses an
//! expression.

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::{children, descendants, inside, is_assigned, is_function, is_reference, java_decls, kotlin_decls};
use super::lang::Lang;
use super::symbols;
use crate::domain::inline_function::Block;
use crate::domain::introduce_parameter::ParameterPlan;
use crate::domain::line_edit::declaration_lines;

const NOT_HERE: &str = "Cannot perform refactoring. Caret should be positioned at the name of element to be inlined.";
const NOT_ONE_EXPRESSION: &str = "Cannot perform refactoring. Only a function with a body can be inlined.";
const RETURN_INSIDE: &str = "Cannot perform refactoring. Only a function whose only return is its last statement can be inlined.";

fn name_of<'t>(lang: Lang, function: Node<'t>) -> Option<Node<'t>> {
    match lang {
        Lang::Kotlin => children(function).into_iter().find(|c| c.kind() == "simple_identifier"),
        _ => function.child_by_field_name("name"),
    }
}

/// The one expression a function's body is.
fn body_expression<'t>(lang: Lang, function: Node<'t>, text: &str) -> Option<Node<'t>> {
    let returned = |statement: Node<'t>| -> Option<Node<'t>> {
        match statement.kind() {
            "jump_expression" | "return_statement" => children(statement).into_iter().find(|c| c.is_named() && !c.kind().contains("comment")),
            _ => None,
        }
    };
    match lang {
        Lang::Kotlin => {
            let body = children(function).into_iter().find(|c| c.kind() == "function_body")?;
            let parts = children(body);
            // `= expression`, not a block.
            if !text[body.byte_range()].trim_start().starts_with('{') {
                return parts.into_iter().find(|p| p.is_named());
            }
            let statements = parts.into_iter().find(|p| p.kind() == "statements")?;
            let named: Vec<Node> = children(statements).into_iter().filter(|c| c.is_named() && !c.kind().contains("comment")).collect();
            (named.len() == 1).then(|| returned(named[0])).flatten()
        }
        _ => {
            let body = function.child_by_field_name("body")?;
            let named: Vec<Node> = children(body).into_iter().filter(|c| c.is_named() && !c.kind().contains("comment")).collect();
            (named.len() == 1).then(|| returned(named[0])).flatten()
        }
    }
}

/// The function to inline at `offset`: its plan (the body as the
/// expression) and its declaration's whole lines.
pub fn plan_at(lang: Lang, text: &str, offset: usize) -> Result<(ParameterPlan, Range<usize>), String> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err(NOT_HERE.into());
    }
    let (word, _) = super::nav::word_at(text, offset).ok_or(NOT_HERE)?;
    let tree = symbols::parse(lang, text).ok_or(NOT_HERE)?;
    let functions: Vec<Node> = descendants(tree.root_node()).into_iter().filter(|n| is_function(lang, *n) && name_of(lang, *n).is_some_and(|name| text[name.byte_range()] == word)).collect();
    // On a declaration's name: that one; on a call: the file's only one of that name.
    let on_name = functions.iter().find(|f| name_of(lang, **f).is_some_and(|n| n.start_byte() <= offset && offset <= n.end_byte()));
    let function = match (on_name, functions.as_slice()) {
        (Some(f), _) => *f,
        (None, [only]) => *only,
        _ => return Err(NOT_HERE.into()),
    };
    let Some(body) = body_expression(lang, function, text) else {
        let plan = block_plan(lang, function, text, &word)?;
        return Ok((plan, declaration_lines(text, function.byte_range())));
    };
    if descendants(body).iter().any(|n| matches!(n.kind(), "simple_identifier" | "identifier") && text[n.byte_range()] == word && n.parent().is_some_and(|p| matches!(p.kind(), "call_expression" | "method_invocation"))) {
        return Err(format!("Cannot perform refactoring. {word} is recursive"));
    }
    let (plan, _) = super::introduce_parameter::plan(lang, text, body.byte_range())?;
    Ok((plan, declaration_lines(text, function.byte_range())))
}

/// The plan for a body of statements: the statements, the parameters and
/// members they use, the variables they declare, and the last `return`.
fn block_plan(lang: Lang, function: Node, text: &str, word: &str) -> Result<ParameterPlan, String> {
    let statements: Vec<Node> = match lang {
        Lang::Kotlin => {
            let body = children(function).into_iter().find(|c| c.kind() == "function_body").ok_or(NOT_ONE_EXPRESSION)?;
            let block = children(body).into_iter().find(|c| c.kind() == "statements");
            block.map(|b| children(b).into_iter().filter(|c| !c.kind().contains("comment")).collect()).unwrap_or_default()
        }
        _ => {
            let body = function.child_by_field_name("body").ok_or(NOT_ONE_EXPRESSION)?;
            children(body).into_iter().filter(|c| !c.kind().contains("comment")).collect()
        }
    };
    let (Some(first), Some(last)) = (statements.first(), statements.last()) else { return Err(NOT_ONE_EXPRESSION.into()) };
    let range = first.start_byte()..last.end_byte();
    let rel = |r: Range<usize>| r.start - range.start..r.end - range.start;
    let is_return = |n: Node| match lang {
        Lang::Kotlin => n.kind() == "jump_expression" && text[n.byte_range()].starts_with("return") && !text[n.byte_range()].starts_with("return@"),
        _ => n.kind() == "return_statement",
    };
    // The last statement's return; no other return of the function.
    let nested = |n: Node| {
        std::iter::successors(n.parent(), |p| p.parent())
            .take_while(|p| p.id() != function.id())
            .any(|p| matches!(p.kind(), "lambda_literal" | "anonymous_function" | "function_declaration" | "class_declaration" | "object_literal" | "lambda_expression" | "class_body" | "method_declaration"))
    };
    let all: Vec<Node> = statements.iter().flat_map(|s| descendants(*s)).collect();
    if all.iter().any(|n| is_return(*n) && !nested(*n) && n.id() != last.id()) {
        return Err(RETURN_INSIDE.into());
    }
    if all.iter().any(|n| matches!(n.kind(), "simple_identifier" | "identifier") && &text[n.byte_range()] == word && n.parent().is_some_and(|p| matches!(p.kind(), "call_expression" | "method_invocation"))) {
        return Err(format!("Cannot perform refactoring. {word} is recursive"));
    }
    let (return_start, result) = if is_return(*last) {
        let value = children(*last).into_iter().find(|c| c.is_named() && !c.kind().contains("comment"));
        (Some(last.start_byte() - range.start), value.map(|v| rel(v.byte_range())))
    } else {
        (None, None)
    };
    let params_node = super::introduce_parameter::params_node(lang, function).ok_or(NOT_ONE_EXPRESSION)?;
    let decls = match lang {
        Lang::Kotlin => kotlin_decls(function, text),
        _ => java_decls(function, text),
    };
    let is_param = |d: &super::extract_method::Decl| d.scope == function.byte_range() && inside(&(d.at..d.at), &params_node.byte_range());
    let params: Vec<&super::extract_method::Decl> = decls.iter().filter(|d| is_param(d)).collect();
    let resolve = |name: &str, at: usize| decls.iter().filter(|d| d.name == name && d.at <= at && inside(&(at..at), &d.scope)).max_by_key(|d| d.at);
    // The class's own members: what a call on another object qualifies.
    let class = std::iter::successors(function.parent(), |p| p.parent()).find(|p| matches!(p.kind(), "class_body" | "enum_class_body" | "enum_body"));
    let private = |m: Node| children(m).into_iter().any(|c| c.kind() == "modifiers" && text[c.byte_range()].split_whitespace().any(|w| w == "private"));
    let private_members: Vec<String> = class
        .map(|c| {
            children(c)
                .into_iter()
                .filter(|m| private(*m))
                .flat_map(|m| match m.kind() {
                    "function_declaration" => children(m).into_iter().filter(|x| x.kind() == "simple_identifier").take(1).map(|x| text[x.byte_range()].to_owned()).collect::<Vec<_>>(),
                    "property_declaration" => descendants(m).into_iter().filter(|x| x.kind() == "variable_declaration").flat_map(children).filter(|x| x.kind() == "simple_identifier").map(|x| text[x.byte_range()].to_owned()).collect(),
                    "method_declaration" => m.child_by_field_name("name").map(|x| text[x.byte_range()].to_owned()).into_iter().collect(),
                    "field_declaration" => children(m).into_iter().filter(|x| x.kind() == "variable_declarator").filter_map(|d| d.child_by_field_name("name")).map(|x| text[x.byte_range()].to_owned()).collect(),
                    _ => Vec::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let members_of_class: Vec<String> = class
        .map(|c| {
            children(c)
                .into_iter()
                .flat_map(|m| match m.kind() {
                    "function_declaration" => children(m).into_iter().filter(|x| x.kind() == "simple_identifier").take(1).map(|x| text[x.byte_range()].to_owned()).collect::<Vec<_>>(),
                    "property_declaration" => descendants(m).into_iter().filter(|x| x.kind() == "variable_declaration").flat_map(children).filter(|x| x.kind() == "simple_identifier").map(|x| text[x.byte_range()].to_owned()).collect(),
                    "method_declaration" => m.child_by_field_name("name").map(|x| text[x.byte_range()].to_owned()).into_iter().collect(),
                    "field_declaration" => children(m).into_iter().filter(|x| x.kind() == "variable_declarator").filter_map(|d| d.child_by_field_name("name")).map(|x| text[x.byte_range()].to_owned()).collect(),
                    _ => Vec::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    // Kotlin's primary constructor properties are members too.
    let mut members_of_class = members_of_class;
    if let Some(class_decl) = class.and_then(|c| c.parent()) {
        for p in descendants(class_decl).into_iter().filter(|p| p.kind() == "class_parameter") {
            if let Some(id) = children(p).into_iter().find(|x| x.kind() == "simple_identifier") {
                members_of_class.push(text[id.byte_range()].to_owned());
            }
        }
    }
    let mut uses = Vec::new();
    let mut members = Vec::new();
    let mut assigned = Vec::new();
    let mut uses_private = false;
    for n in all.iter().filter(|n| is_reference(lang, **n, text)) {
        let raw = &text[n.byte_range()];
        let name = raw.trim_start_matches('$');
        let r = rel(n.byte_range());
        let r = if raw.starts_with('$') { r.start + 1..r.end } else { r };
        match resolve(name, n.start_byte()) {
            Some(d) if is_param(d) => {
                let ix = params.iter().position(|p| p.name == d.name).unwrap_or(0);
                uses.push((r, ix));
                if is_assigned(lang, *n, text) && !assigned.contains(&ix) {
                    assigned.push(ix);
                }
            }
            Some(_) => {}
            None if members_of_class.iter().any(|m| m == name) && !raw.starts_with('$') => {
                uses_private |= private_members.iter().any(|m| m == name);
                members.push(r);
            }
            None => {}
        }
    }
    let locals = decls.iter().filter(|d| inside(&(d.at..d.at), &range)).map(|d| d.name.clone()).collect();
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line_start..range.start].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    Ok(ParameterPlan {
        function: super::introduce_parameter::function_name(lang, function, text),
        expression: text[range].to_owned(),
        occurrences: Vec::new(),
        selected: 0,
        params_close: params_node.end_byte() - 1,
        params: params.iter().map(|p| p.name.clone()).collect(),
        uses,
        members,
        ty: String::new(),
        taken: Vec::new(),
        block: Some(Block { return_start, result, indent, locals, assigned, types: params.iter().map(|p| p.ty.clone()).collect(), java: lang == Lang::Java, private_members: uses_private }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::inline_function::edits;
    use crate::domain::introduce_parameter::apply;

    #[test]
    fn inlines_a_kotlin_function_everywhere_in_its_file() {
        let text = "class Cart(val rate: Double) {\n    fun total(price: Double, count: Int): Double {\n        return price * rate * count\n    }\n\n    fun use() = total(1.0, 2) + 1\n}\n";
        let (plan, decl) = plan_at(Lang::Kotlin, text, text.find("total(1.0").unwrap()).unwrap();
        assert_eq!(plan.expression, "price * rate * count");
        let calls = super::super::introduce_parameter::call_sites(Lang::Kotlin, text, &plan.function, plan.params.len());
        let out = apply(text, edits(&plan, text, decl, &calls).unwrap());
        assert_eq!(out, "class Cart(val rate: Double) {\n    fun use() = (1.0 * rate * 2) + 1\n}\n");
    }

    #[test]
    fn refuses_what_it_cannot_inline() {
        let text = "fun a(x: Int): Int {\n    if (x > 0) return 1\n    return 0\n}\nfun b(n: Int): Int = if (n > 0) b(n - 1) else 0\n";
        assert_eq!(plan_at(Lang::Kotlin, text, text.find("a(").unwrap()).unwrap_err(), RETURN_INSIDE);
        assert!(plan_at(Lang::Kotlin, text, text.find("b(n:").unwrap()).unwrap_err().contains("recursive"));
        let java = "class A {\n  int twice(int v) { return v * 2; }\n  int f() { return twice(3); }\n}\n";
        let (plan, _) = plan_at(Lang::Java, java, java.find("twice(3)").unwrap()).unwrap();
        assert_eq!((plan.expression.as_str(), plan.params.len()), ("v * 2", 1));
    }

    #[test]
    fn inlines_statements() {
        use crate::domain::inline_function::inline_calls;
        let text = "class Cart(val rate: Double) {\n    fun total(price: Double, count: Int): Double {\n        val net = price * count\n        log(net)\n        return net * rate\n    }\n\n    fun log(x: Double) {}\n\n    fun use(n: Int) {\n        val t = total(2.0, n + 1)\n        println(t)\n    }\n\n    fun other(cart: Cart) {\n        cart.total(3.0, 2)\n    }\n}\n";
        let (plan, decl) = plan_at(Lang::Kotlin, text, text.find("total(price").unwrap()).unwrap();
        let block = plan.block.clone().unwrap();
        assert_eq!((block.locals.clone(), plan.members.len()), (vec!["net".to_owned()], 2));
        let calls = super::super::introduce_parameter::call_sites(Lang::Kotlin, text, "total", 2);
        let out = apply(text, edits(&plan, text, decl, &calls).unwrap());
        assert_eq!(
            out,
            "class Cart(val rate: Double) {\n    fun log(x: Double) {}\n\n    fun use(n: Int) {\n        val count = n + 1\n        val net = 2.0 * count\n        log(net)\n        val t = net * rate\n        println(t)\n    }\n\n    fun other(cart: Cart) {\n        val net = 3.0 * 2\n        cart.log(net)\n    }\n}\n"
        );
        // Java: in an argument; a local of the caller's name stops it.
        let java = "class A {\n  int total;\n  int add(int a, int b) {\n    int sum = a + b;\n    total += sum;\n    return sum;\n  }\n  void use(int x) {\n    System.out.println(add(x, 7));\n  }\n}\n";
        let (plan, _) = plan_at(Lang::Java, java, java.find("add(int").unwrap()).unwrap();
        let calls = super::super::introduce_parameter::call_sites(Lang::Java, java, "add", 2);
        let calls: Vec<_> = calls.into_iter().filter(|c| c.name_range.start > java.find("void use").unwrap()).collect();
        assert_eq!(inline_calls(&plan, java, &calls).unwrap(), java.replace("    System.out.println(add(x, 7));", "    int sum = x + 7;\n    total += sum;\n    System.out.println(sum);"));
        let clash = java.replace("void use(int x) {\n", "void use(int x) {\n    int sum = 0;\n");
        let calls = super::super::introduce_parameter::call_sites(Lang::Java, &clash, "add", 2);
        let calls: Vec<_> = calls.into_iter().filter(|c| c.name_range.start > clash.find("void use").unwrap()).collect();
        assert!(inline_calls(&plan, &clash, &calls).unwrap_err().contains("'sum' is already defined"));
        // Not before a loop's condition.
        let looped = java.replace("System.out.println(add(x, 7));", "while (add(x, 7) > 0) {}");
        let calls = super::super::introduce_parameter::call_sites(Lang::Java, &looped, "add", 2);
        let calls: Vec<_> = calls.into_iter().filter(|c| c.name_range.start > looped.find("void use").unwrap()).collect();
        assert!(inline_calls(&plan, &looped, &calls).unwrap_err().contains("isn't a statement of a block"));
    }
}

//! Inline Function's view of the syntax tree (Kotlin and Java): the
//! function named at the caret (its declaration or a call in the same
//! file), the one expression its body is, and the parameters and members
//! that expression uses, as Introduce Parameter analyses an expression.

use std::ops::Range;

use tree_sitter::Node;

use super::extract_method::{children, descendants, is_function};
use super::lang::Lang;
use super::symbols;
use crate::domain::introduce_parameter::ParameterPlan;
use crate::domain::line_edit::{line_end, line_start};

const NOT_HERE: &str = "Cannot perform refactoring. Caret should be positioned at the name of element to be inlined.";
const NOT_ONE_EXPRESSION: &str = "Cannot perform refactoring. Only a function whose body is one expression or one return statement can be inlined.";

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
    let body = body_expression(lang, function, text).ok_or(NOT_ONE_EXPRESSION)?;
    if descendants(body).iter().any(|n| matches!(n.kind(), "simple_identifier" | "identifier") && text[n.byte_range()] == word && n.parent().is_some_and(|p| matches!(p.kind(), "call_expression" | "method_invocation"))) {
        return Err(format!("Cannot perform refactoring. {word} is recursive"));
    }
    let (plan, _) = super::introduce_parameter::plan(lang, text, body.byte_range())?;
    // The declaration's lines, and the blank line after it when the line
    // before is blank or opens the block.
    let start = line_start(text, function.start_byte());
    let mut end = (line_end(text, function.end_byte()) + 1).min(text.len());
    let before = text[..start.saturating_sub(1)].rsplit('\n').next().unwrap_or_default().trim();
    if (before.is_empty() || before.ends_with('{')) && text[end..].starts_with('\n') {
        end += 1;
    }
    Ok((plan, start..end))
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
        let text = "fun a(x: Int): Int {\n    val y = x\n    return y\n}\nfun b(n: Int): Int = if (n > 0) b(n - 1) else 0\n";
        assert!(plan_at(Lang::Kotlin, text, text.find("a(").unwrap()).unwrap_err().contains("one expression"));
        assert!(plan_at(Lang::Kotlin, text, text.find("b(n:").unwrap()).unwrap_err().contains("recursive"));
        let java = "class A {\n  int twice(int v) { return v * 2; }\n  int f() { return twice(3); }\n}\n";
        let (plan, _) = plan_at(Lang::Java, java, java.find("twice(3)").unwrap()).unwrap();
        assert_eq!((plan.expression.as_str(), plan.params.len()), ("v * 2", 1));
    }
}

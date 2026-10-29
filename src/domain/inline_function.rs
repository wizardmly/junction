//! Inline Function (Ctrl+Alt+N on a function), as IntelliJ's for a Kotlin
//! or Java function whose body is one expression (`= …`, or a block with
//! one `return …`): each call becomes that expression, its parameters
//! replaced by the call's arguments and its members qualified by the
//! call's receiver, then the declaration goes. The analysis is
//! `index::inline_function`.

use std::ops::Range;

use super::introduce_parameter::{apply, argument_for, CallSite, Edit, ParameterPlan};

/// A replacement is one operand where it lands: a name, a call, a literal.
fn is_simple(expr: &str) -> bool {
    let mut depth = 0i32;
    for c in expr.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            // An operator or a space outside brackets: more than one operand.
            c if depth == 0 && (c.is_whitespace() || "+-*/%<>=!&|?:".contains(c)) => return false,
            _ => {}
        }
    }
    true
}

/// Places where anything fits: between `(` / `,` / `=` / `return` and
/// `)` / `,` / the line's end.
fn fits_anything(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].trim_end();
    let after = text[range.end..].trim_start_matches([' ', '\t']);
    let open = before.ends_with(['(', ',', '=', '{']) || before.ends_with("return") || before.is_empty() || before.ends_with('\n');
    let close = after.starts_with([')', ',', '\n', ';', '}']) || after.is_empty() || after.starts_with("\r\n");
    open && close
}

/// The edit replacing one call with the body.
pub fn call_edit(plan: &ParameterPlan, text: &str, call: &CallSite) -> Result<Edit, String> {
    // Every parameter needs an argument (a default value isn't inlined).
    for (ix, name) in plan.params.iter().enumerate() {
        if call.value_for(&plan.params, ix).is_none() {
            return Err(format!("Cannot perform refactoring. A call of {} leaves out '{name}'", plan.function));
        }
    }
    let start = match &call.receiver {
        Some(receiver) => {
            let dot = call.name_range.start.checked_sub(1).filter(|d| text.as_bytes().get(*d) == Some(&b'.'));
            let start = dot.and_then(|d| d.checked_sub(receiver.len())).filter(|s| text.get(*s..call.name_range.start - 1) == Some(receiver.as_str()));
            start.ok_or_else(|| format!("Cannot perform refactoring. Unsupported call of {}", plan.function))?
        }
        None => call.name_range.start,
    };
    let range = start..call.end();
    let body = argument_for(plan, call);
    let body = if is_simple(&body) || fits_anything(text, &range) { body } else { format!("({body})") };
    Ok((range, body))
}

/// The function's file's edits: its calls there, and the declaration
/// (`declaration`: its whole lines) removed.
pub fn edits(plan: &ParameterPlan, text: &str, declaration: Range<usize>, calls_here: &[CallSite]) -> Result<Vec<Edit>, String> {
    let mut out = Vec::new();
    for call in calls_here.iter().filter(|c| !(declaration.start <= c.name_range.start && c.name_range.end <= declaration.end)) {
        out.push(call_edit(plan, text, call)?);
    }
    out.push((declaration, String::new()));
    Ok(out)
}

/// The text with the edits applied (for a file elsewhere).
pub fn inline_calls(plan: &ParameterPlan, text: &str, calls: &[CallSite]) -> Result<String, String> {
    let edits = calls.iter().map(|c| call_edit(plan, text, c)).collect::<Result<Vec<_>, _>>()?;
    Ok(apply(text, edits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ParameterPlan {
        // fun total(price: Double, count: Int) = price * rate * count
        let expression = "price * rate * count".to_owned();
        ParameterPlan {
            function: "total".into(),
            occurrences: Vec::new(),
            selected: 0,
            params_close: 0,
            params: vec!["price".into(), "count".into()],
            uses: vec![(0..5, 0), (15..20, 1)],
            members: vec![8..12],
            ty: String::new(),
            taken: Vec::new(),
            expression,
        }
    }

    fn call(text: &str, receiver: Option<&str>, args: &[(Option<&str>, &str)]) -> CallSite {
        let name = text.find("total").unwrap();
        CallSite {
            name_range: name..name + 5,
            args_open: name + 5,
            args_close: text.rfind(')').unwrap(),
            args: args.iter().map(|(n, v)| (n.map(str::to_owned), (*v).to_owned())).collect(),
            receiver: receiver.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn inlines_calls() {
        let text = "val x = cart.total(10.0, n + 1)";
        let (range, body) = call_edit(&plan(), text, &call(text, Some("cart"), &[(None, "10.0"), (None, "n + 1")])).unwrap();
        assert_eq!(apply(text, vec![(range, body)]), "val x = 10.0 * cart.rate * (n + 1)");
        // In an operator expression the body takes parentheses.
        let text = "val y = 2 + total(a, b) * 3";
        let mut c = call(text, None, &[(Some("count"), "b"), (Some("price"), "a")]);
        c.args_close = text.find(") *").unwrap();
        assert_eq!(apply(text, vec![call_edit(&plan(), text, &c).unwrap()]), "val y = 2 + (a * rate * b) * 3");
        // A left-out argument (a default value) isn't inlined.
        let text = "total(1.0)";
        assert!(call_edit(&plan(), text, &call(text, None, &[(None, "1.0")])).is_err());
    }

    #[test]
    fn removes_the_declaration() {
        let text = "fun total(price: Double, count: Int) = price * rate * count\nfun use() = total(1.0, 2)\n";
        let decl = 0..text.find("fun use").unwrap();
        let here = vec![call(&text[decl.end..], None, &[(None, "1.0"), (None, "2")])].into_iter().map(|mut c| {
            c.name_range = c.name_range.start + decl.end..c.name_range.end + decl.end;
            c.args_open += decl.end;
            c.args_close += decl.end;
            c
        }).collect::<Vec<_>>();
        let out = apply(text, edits(&plan(), text, decl, &here).unwrap());
        assert_eq!(out, "fun use() = 1.0 * rate * 2\n");
    }
}

//! Inline Function (Ctrl+Alt+N on a function), as IntelliJ's for a Kotlin
//! or Java function whose body is one expression (`= …`, or a block with
//! one `return …`): each call becomes that expression, its parameters
//! replaced by the call's arguments and its members qualified by the
//! call's receiver, then the declaration goes. A body of statements goes
//! before the statement holding the call, the call becoming what the last
//! `return` returns. The analysis is `index::inline_function`.

use std::ops::Range;

use super::introduce_parameter::{apply, argument_for, CallSite, Edit, ParameterPlan};

/// A body of statements (the plan's `expression`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// Where the last `return` starts, and what it returns (relative to the
    /// statements); none when the body doesn't end with one.
    pub return_start: Option<usize>,
    pub result: Option<Range<usize>>,
    /// The statements' indent.
    pub indent: String,
    /// The variables the body declares.
    pub locals: Vec<String>,
    /// The parameters the body assigns, by index.
    pub assigned: Vec<usize>,
    /// The parameters' declared types (Java needs them for a local).
    pub types: Vec<String>,
    pub java: bool,
    /// The statements use private members of the class: a call on another
    /// object can't reach them.
    pub private_members: bool,
}

/// An argument that can stand for its parameter as it is: a name, a
/// literal, `a.b`.
fn is_atom(value: &str) -> bool {
    let v = value.trim();
    !v.is_empty() && (v.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') || (v.starts_with('"') && v.ends_with('"') && v.len() >= 2 && !v[1..v.len() - 1].contains('"')))
}

/// The edit inlining a call of a function with statements: the statements
/// (the parameters replaced by the arguments, or declared from them) go
/// before the call's statement, the call becomes the returned value (or,
/// as a statement of its own, goes).
fn block_call_edit(plan: &ParameterPlan, block: &Block, text: &str, call: &CallSite, start: usize) -> Result<Edit, String> {
    let f = &plan.function;
    let site = call.statement.as_ref().ok_or_else(|| format!("Cannot perform refactoring. A call of {f}() isn't a statement of a block, where the statements of {f}() could go"))?;
    if call.receiver.is_some() && block.private_members {
        return Err(format!("Cannot perform refactoring. {f}() uses private members of its class, which a call on another object can't reach"));
    }
    if !site.whole && block.result.is_none() {
        return Err(format!("Cannot perform refactoring. {f}() returns no value"));
    }
    let semi = if block.java { ";" } else { "" };
    // Arguments that are names or literals stand in for their parameters;
    // others are declared first, under the parameter's name.
    let mut declared: Vec<String> = Vec::new();
    let mut edits: Vec<Edit> = Vec::new();
    for (ix, name) in plan.params.iter().enumerate() {
        let value = call.value_for(&plan.params, ix).ok_or_else(|| format!("Cannot perform refactoring. A call of {f} leaves out '{name}'"))?;
        let used = plan.uses.iter().any(|(_, p)| *p == ix);
        if is_atom(value) && !block.assigned.contains(&ix) {
            edits.extend(plan.uses.iter().filter(|(_, p)| *p == ix).map(|(r, _)| (r.clone(), value.trim().to_owned())));
        } else if used || !is_atom(value) {
            let mutable = block.assigned.contains(&ix);
            declared.push(if block.java {
                format!("{} {name} = {value};", block.types.get(ix).map_or("var", String::as_str))
            } else {
                format!("{} {name} = {value}", if mutable { "var" } else { "val" })
            });
            if site.taken.contains(name) {
                return Err(format!("Cannot perform refactoring. Variable '{name}' is already defined in the scope"));
            }
        }
    }
    if let Some(local) = block.locals.iter().find(|l| site.taken.contains(l)) {
        return Err(format!("Cannot perform refactoring. Variable '{local}' is already defined in the scope"));
    }
    if let Some(receiver) = &call.receiver {
        edits.extend(plan.members.iter().map(|r| (r.start..r.start, format!("{receiver}."))));
    }
    // The statements before the return, and the returned value, each with
    // its edits.
    let split = block.return_start.unwrap_or(plan.expression.len());
    let part = |range: Range<usize>| {
        let inside: Vec<Edit> = edits.iter().filter(|(r, _)| r.start >= range.start && r.end <= range.end).map(|(r, t)| (r.start - range.start..r.end - range.start, t.clone())).collect();
        apply(&plan.expression[range], inside)
    };
    let prelude = part(0..split);
    let result = block.result.clone().map(part);
    let mut lines: Vec<String> = declared;
    for (i, line) in prelude.trim_end().lines().enumerate() {
        let line = if i == 0 { line } else { line.strip_prefix(block.indent.as_str()).unwrap_or_else(|| line.trim_start()) };
        lines.push(line.to_owned());
    }
    // Blank lines stay blank (no indent).
    let joined = lines.iter().enumerate().map(|(i, l)| if l.trim().is_empty() { String::new() } else if i == 0 { l.clone() } else { format!("{}{l}", site.indent) }).collect::<Vec<_>>().join("\n");
    if site.whole {
        // The call alone: its statements; a returned call stays as a statement.
        let end = call.end() + usize::from(block.java && text[call.end()..].starts_with(';'));
        let kept = result.filter(|r| r.contains('('));
        let mut out = joined;
        if let Some(r) = kept {
            if !out.is_empty() {
                out.push_str(&format!("\n{}", site.indent));
            }
            out.push_str(&format!("{r}{semi}"));
        }
        return Ok((site.start..end, out));
    }
    let range = start..call.end();
    let result = result.unwrap_or_default();
    let result = if is_simple(&result) || fits_anything(text, &range) { result } else { format!("({result})") };
    let before = if joined.is_empty() { String::new() } else { format!("{joined}\n{}", site.indent) };
    Ok((site.start..call.end(), format!("{before}{}{result}", &text[site.start..start])))
}

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
    if call.reference {
        return Err(format!("Cannot perform refactoring. {} is used as a reference (::{})", plan.function, plan.function));
    }
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
    if let Some(block) = &plan.block {
        return block_call_edit(plan, block, text, call, start);
    }
    let range = start..call.end();
    let body = argument_for(plan, call);
    let body = if is_simple(&body) || fits_anything(text, &range) { body } else { format!("({body})") };
    Ok((range, body))
}

/// The function's file's edits: its calls there, and the declaration
/// (`declaration`: its whole lines) removed.
pub fn edits(plan: &ParameterPlan, text: &str, declaration: Range<usize>, calls_here: &[CallSite]) -> Result<Vec<Edit>, String> {
    let calls: Vec<CallSite> = calls_here.iter().filter(|c| !(declaration.start <= c.name_range.start && c.name_range.end <= declaration.end)).cloned().collect();
    let mut out = call_edits(plan, text, &calls)?;
    out.push((declaration, String::new()));
    Ok(out)
}

/// The calls' edits; with statements, two calls in one statement, or in
/// one function when the statements declare variables, can't both go.
fn call_edits(plan: &ParameterPlan, text: &str, calls: &[CallSite]) -> Result<Vec<Edit>, String> {
    let edits = calls.iter().map(|c| call_edit(plan, text, c)).collect::<Result<Vec<_>, _>>()?;
    if let Some(block) = &plan.block {
        let mut ranges: Vec<&Range<usize>> = edits.iter().map(|(r, _)| r).collect();
        ranges.sort_by_key(|r| r.start);
        if ranges.windows(2).any(|w| w[1].start < w[0].end) {
            return Err(format!("Cannot perform refactoring. Two calls of {}() are in one statement", plan.function));
        }
        let declares = !block.locals.is_empty() || plan.params.iter().enumerate().any(|(ix, _)| calls.iter().any(|c| c.value_for(&plan.params, ix).is_some_and(|v| !is_atom(v) || block.assigned.contains(&ix))));
        let functions: Vec<&Range<usize>> = calls.iter().filter_map(|c| c.statement.as_ref().map(|s| &s.function)).collect();
        if declares && functions.iter().enumerate().any(|(i, f)| functions[..i].contains(f)) {
            return Err(format!("Cannot perform refactoring. {}() declares variables and is called twice in one function", plan.function));
        }
    }
    Ok(edits)
}

/// The text with the edits applied (for a file elsewhere).
pub fn inline_calls(plan: &ParameterPlan, text: &str, calls: &[CallSite]) -> Result<String, String> {
    Ok(apply(text, call_edits(plan, text, calls)?))
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
            block: None,
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

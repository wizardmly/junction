//! Introduce Parameter (Ctrl+Alt+P) as IntelliJ does it for Kotlin and
//! Java: the selected expression of a function becomes a new last
//! parameter, its occurrences there the parameter's name; each call
//! passes the expression, the function's parameters in it replaced by the
//! call's arguments and its members qualified by the call's receiver; a
//! Kotlin parameter can take the expression as its default value instead,
//! the calls left alone. The analysis is `index::introduce_parameter`.

use std::ops::Range;

use crate::domain::imports::ImportStyle;

/// What the analysis found in the function's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterPlan {
    pub function: String,
    /// The selected expression's text.
    pub expression: String,
    /// Its occurrences in the function, the selected one at `selected`.
    pub occurrences: Vec<Range<usize>>,
    pub selected: usize,
    /// The `)` ending the parameter list.
    pub params_close: usize,
    /// The existing parameters' names, in order.
    pub params: Vec<String>,
    /// The function's parameters used in the expression: where (relative
    /// to it) and which.
    pub uses: Vec<(Range<usize>, usize)>,
    /// Names in it that aren't parameters (fields, methods of the class):
    /// a call on another object qualifies them.
    pub members: Vec<Range<usize>>,
    /// Its type when it shows it; empty when unknown.
    pub ty: String,
    /// Names the parameter mustn't take.
    pub taken: Vec<String>,
}

/// A call of the function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallSite {
    /// The function's name in it.
    pub name_range: Range<usize>,
    /// The `(` starting its arguments.
    pub args_open: usize,
    /// The `)` ending its arguments.
    pub args_close: usize,
    /// Its arguments: Kotlin's name when named, and the value.
    pub args: Vec<(Option<String>, String)>,
    /// `repo` of `repo.load(…)`; none for `load(…)` and `this.load(…)`.
    pub receiver: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterRequest {
    pub name: String,
    pub ty: String,
    pub replace_all: bool,
    /// Kotlin: `name: Type = expression`, the calls unchanged.
    pub default_value: bool,
}

/// A text edit.
pub type Edit = (Range<usize>, String);

/// The expression a call passes: the function's parameters in it replaced
/// by the call's arguments, its members qualified by the call's receiver.
pub fn argument_for(plan: &ParameterPlan, call: &CallSite) -> String {
    let mut edits: Vec<Edit> = Vec::new();
    for (range, param) in &plan.uses {
        let name = &plan.params[*param];
        // A named argument by name, else by position.
        let arg = call.args.iter().find(|(n, _)| n.as_deref() == Some(name.as_str())).or_else(|| call.args.get(*param).filter(|(n, _)| n.is_none()));
        if let Some((_, value)) = arg {
            let simple = value.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
            edits.push((range.clone(), if simple { value.clone() } else { format!("({value})") }));
        }
    }
    if let Some(receiver) = &call.receiver {
        for range in &plan.members {
            edits.push((range.start..range.start, format!("{receiver}.")));
        }
    }
    apply(&plan.expression, edits)
}

/// Applies edits (non-overlapping) to a text.
pub fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|(r, _)| std::cmp::Reverse((r.start, r.end)));
    let mut out = text.to_owned();
    for (r, s) in edits {
        out.replace_range(r, &s);
    }
    out
}

pub fn check(style: ImportStyle, plan: &ParameterPlan, request: &ParameterRequest) -> Result<(), String> {
    crate::domain::rename::check_name(if style == ImportStyle::Java { "java" } else { "kt" }, &request.name)?;
    if plan.params.contains(&request.name) || plan.taken.contains(&request.name) {
        return Err(format!("Variable '{}' is already defined in the scope", request.name));
    }
    if request.ty.trim().is_empty() {
        return Err("Specify the parameter's type".into());
    }
    Ok(())
}

/// The edits in the function's file (its calls there included) and the
/// new argument for each call elsewhere.
pub fn edits(style: ImportStyle, plan: &ParameterPlan, calls_here: &[CallSite], request: &ParameterRequest) -> Result<Vec<Edit>, String> {
    check(style, plan, request)?;
    let java = style == ImportStyle::Java;
    let mut out: Vec<Edit> = Vec::new();
    // The declaration.
    let sep = if plan.params.is_empty() { "" } else { ", " };
    let declared = if java {
        format!("{sep}{} {}", request.ty.trim(), request.name)
    } else if request.default_value {
        format!("{sep}{}: {} = {}", request.name, request.ty.trim(), plan.expression)
    } else {
        format!("{sep}{}: {}", request.name, request.ty.trim())
    };
    out.push((plan.params_close..plan.params_close, declared));
    // The expression, once or everywhere in the function.
    for (i, r) in plan.occurrences.iter().enumerate() {
        if request.replace_all || i == plan.selected {
            out.push((r.clone(), request.name.clone()));
        }
    }
    if !(request.default_value && !java) {
        for call in calls_here {
            out.extend(call_edit(plan, call, &request.name));
        }
    }
    Ok(out)
}

/// The new argument added to a call: named when the call names its
/// arguments or leaves some to their defaults.
pub fn call_edit(plan: &ParameterPlan, call: &CallSite, name: &str) -> Option<Edit> {
    let value = argument_for(plan, call);
    let named = call.args.iter().any(|(n, _)| n.is_some()) || call.args.len() < plan.params.len();
    let sep = if call.args.is_empty() { "" } else { ", " };
    let arg = if named { format!("{sep}{name} = {value}") } else { format!("{sep}{value}") };
    Some((call.args_close..call.args_close, arg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str, expr: &str) -> ParameterPlan {
        let at = text.find(expr).unwrap();
        ParameterPlan {
            function: "total".into(),
            expression: expr.into(),
            occurrences: vec![at..at + expr.len()],
            selected: 0,
            params_close: text.find(')').unwrap(),
            params: vec!["price".into()],
            uses: vec![(0..5, 0)],
            members: vec![expr.find("rate").unwrap()..expr.find("rate").unwrap() + 4],
            ty: "Double".into(),
            taken: vec![],
        }
    }

    #[test]
    fn introduces_a_parameter() {
        let text = "fun total(price: Double): Double {\n    return price * rate\n}\nfun use() = total(2.0) + cart.total(base + 1)\n";
        let p = plan(text, "price * rate");
        let calls = [
            CallSite { args_close: text.find("2.0)").unwrap() + 3, args: vec![(None, "2.0".into())], receiver: None, ..Default::default() },
            CallSite { args_close: text.find("1)").unwrap() + 1, args: vec![(None, "base + 1".into())], receiver: Some("cart".into()), ..Default::default() },
        ];
        let request = ParameterRequest { name: "amount".into(), ty: "Double".into(), replace_all: true, default_value: false };
        let out = apply(text, edits(ImportStyle::Kotlin, &p, &calls, &request).unwrap());
        assert_eq!(out, "fun total(price: Double, amount: Double): Double {\n    return amount\n}\nfun use() = total(2.0, 2.0 * rate) + cart.total(base + 1, (base + 1) * cart.rate)\n");
        // A default value: the calls stay.
        let request = ParameterRequest { default_value: true, ..request };
        let out = apply(text, edits(ImportStyle::Kotlin, &p, &calls, &request).unwrap());
        assert!(out.starts_with("fun total(price: Double, amount: Double = price * rate): Double {") && out.contains("total(2.0) + cart.total(base + 1)"), "{out}");
        // A taken name; a call with named arguments gets a named one.
        assert!(check(ImportStyle::Kotlin, &p, &ParameterRequest { name: "price".into(), ty: "Double".into(), replace_all: true, default_value: false }).is_err());
        let named = CallSite { args_close: 10, args: vec![(Some("price".into()), "4.0".into())], receiver: None, ..Default::default() };
        assert_eq!(call_edit(&p, &named, "amount"), Some((10..10, ", amount = 4.0 * rate".into())));
    }

    #[test]
    fn java_has_no_default() {
        let text = "int total(int price) { return price * rate; }\nint use() { return total(3); }\n";
        let mut p = plan(text, "price * rate");
        p.ty = "int".into();
        let calls = [CallSite { args_close: text.find("3)").unwrap() + 1, args: vec![(None, "3".into())], receiver: None, ..Default::default() }];
        let request = ParameterRequest { name: "amount".into(), ty: "int".into(), replace_all: false, default_value: true };
        let out = apply(text, edits(ImportStyle::Java, &p, &calls, &request).unwrap());
        assert_eq!(out, "int total(int price, int amount) { return amount; }\nint use() { return total(3, 3 * rate); }\n");
    }
}

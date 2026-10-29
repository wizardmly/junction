//! Change Signature (Ctrl+F6) as IntelliJ does it for Kotlin and Java
//! functions: a new name and return type, the parameters renamed, retyped,
//! reordered, removed or added (with the value the calls pass); the
//! declaration, the parameters' uses in the body and every call follow.
//! The analysis is `index::change_signature`.

use std::ops::Range;

use crate::domain::imports::ImportStyle;
use crate::domain::introduce_parameter::{CallSite, Edit};

/// A parameter as declared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: String,
    /// Its modifiers and annotations (`vararg`, `final`, `@NonNull`).
    pub prefix: String,
    /// Kotlin's default value, without the `=`.
    pub default: Option<String>,
    /// The parameter's text in the list.
    pub text: String,
    /// Its uses in the body.
    pub uses: Vec<Range<usize>>,
    /// A Kotlin constructor's `val` / `var` parameter: also a property.
    pub property: bool,
}

/// The function as the analysis found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub name_range: Range<usize>,
    pub params: Vec<Param>,
    /// Between the parentheses of the parameter list.
    pub params_range: Range<usize>,
    /// The return type as written: Kotlin's after `:`, Java's before the
    /// name; empty for Kotlin's none.
    pub return_type: String,
    /// Kotlin: from the `)` to the type's end when written, else the `)`'s
    /// end (where `: Type` goes). Java: the type.
    pub return_range: Range<usize>,
    /// A constructor: no name or return type to change.
    pub constructor: bool,
}

/// A parameter of the new signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewParam {
    /// Which parameter it was; none for an added one.
    pub old: Option<usize>,
    pub name: String,
    pub ty: String,
    /// An added parameter's value at the calls.
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureRequest {
    pub name: String,
    pub return_type: String,
    pub params: Vec<NewParam>,
}

impl SignatureRequest {
    /// The signature unchanged.
    #[cfg(test)]
    pub fn of(signature: &Signature) -> Self {
        Self {
            name: signature.name.clone(),
            return_type: signature.return_type.clone(),
            params: signature.params.iter().enumerate().map(|(i, p)| NewParam { old: Some(i), name: p.name.clone(), ty: p.ty.clone(), value: String::new() }).collect(),
        }
    }
}

fn lang_of(style: ImportStyle) -> &'static str {
    if style == ImportStyle::Java { "java" } else { "kt" }
}

pub fn check(style: ImportStyle, signature: &Signature, request: &SignatureRequest) -> Result<(), String> {
    if !signature.constructor {
        crate::domain::rename::check_name(lang_of(style), &request.name)?;
    }
    // A property is used beyond the constructor: Rename / Safe Delete handle it.
    for (i, old) in signature.params.iter().enumerate().filter(|(_, p)| p.property) {
        match request.params.iter().find(|p| p.old == Some(i)) {
            None => return Err(format!("Property '{}' can't be removed here; use Safe Delete", old.name)),
            Some(p) if p.name != old.name => return Err(format!("Property '{}' can't be renamed here; use Rename (Shift+F6)", old.name)),
            _ => {}
        }
    }
    for (i, p) in request.params.iter().enumerate() {
        crate::domain::rename::check_name(lang_of(style), &p.name).map_err(|e| format!("Parameter {}: {e}", i + 1))?;
        if request.params[..i].iter().any(|q| q.name == p.name) {
            return Err(format!("Duplicated parameter name: {}", p.name));
        }
        if p.ty.trim().is_empty() {
            return Err(format!("Specify the type of parameter '{}'", p.name));
        }
        if p.old.is_none() && p.value.trim().is_empty() {
            return Err(format!("Specify the default value of parameter '{}' for the calls", p.name));
        }
    }
    if style == ImportStyle::Java && !signature.constructor && request.return_type.trim().is_empty() {
        return Err("Specify the return type".into());
    }
    Ok(())
}

fn render_param(style: ImportStyle, old: Option<&Param>, p: &NewParam) -> String {
    if let Some(o) = old.filter(|o| o.name == p.name && o.ty == p.ty.trim()) {
        return o.text.clone();
    }
    let prefix = old.map(|o| o.prefix.as_str()).unwrap_or("");
    let ty = p.ty.trim();
    match style {
        ImportStyle::Java => format!("{prefix}{ty} {}", p.name),
        ImportStyle::Kotlin => {
            let default = old.and_then(|o| o.default.as_ref()).map(|d| format!(" = {d}")).unwrap_or_default();
            format!("{prefix}{}: {ty}{default}", p.name)
        }
    }
}

/// The declaration's text: what `(…)` holds (one line, or one per line
/// when it was so).
pub fn parameter_list(style: ImportStyle, signature: &Signature, request: &SignatureRequest, original: &str) -> String {
    let rendered: Vec<String> = request.params.iter().map(|p| render_param(style, p.old.and_then(|i| signature.params.get(i)), p)).collect();
    // Kept on separate lines when they were.
    if original.contains('\n') && !rendered.is_empty() {
        let indent = original.lines().nth(1).map(|l| l.len() - l.trim_start().len()).unwrap_or(4);
        let close_indent = original.lines().last().filter(|l| l.trim().is_empty()).map(|l| l.len()).unwrap_or(0);
        let pad = " ".repeat(indent);
        let trailing = if original.trim_end().ends_with(',') { "," } else { "" };
        return format!("\n{}{trailing}\n{}", rendered.iter().map(|r| format!("{pad}{r}")).collect::<Vec<_>>().join(",\n"), " ".repeat(close_indent));
    }
    rendered.join(", ")
}

/// A call's new arguments, each rendered (named when it must be), and
/// whether the last one is the old last parameter (a trailing lambda can
/// stay); none when the call can't be rewritten.
fn argument_list(style: ImportStyle, signature: &Signature, request: &SignatureRequest, call: &CallSite) -> Option<(Vec<String>, bool)> {
    let kotlin = style == ImportStyle::Kotlin;
    let names: Vec<String> = signature.params.iter().map(|p| p.name.clone()).collect();
    let mut named = false;
    let mut out = Vec::new();
    for (ix, p) in request.params.iter().enumerate() {
        let value = match p.old {
            Some(i) => match call.value_for(&names, i) {
                Some(v) => v.to_owned(),
                // Left to its default: the ones after are named.
                None if kotlin => {
                    named = true;
                    continue;
                }
                None => return None,
            },
            None => p.value.trim().to_owned(),
        };
        // Kotlin: once the order differs from the call's, names keep it right.
        if kotlin && !named {
            let was_named = p.old.is_some_and(|i| call.args.iter().any(|(n, _)| n.as_deref() == Some(signature.params[i].name.as_str())));
            let moved = p.old.is_some_and(|i| i != ix);
            named = was_named || (moved && call.args.iter().any(|(n, _)| n.is_some()));
        }
        out.push(if named { format!("{} = {value}", p.name) } else { value });
    }
    let last_kept = signature.params.len().checked_sub(1).is_some_and(|last| request.params.last().is_some_and(|p| p.old == Some(last)));
    Some((out, last_kept))
}

/// A call's new argument list (between the parentheses); none when the
/// call can't be rewritten.
#[cfg(test)]
pub fn call_arguments(style: ImportStyle, signature: &Signature, request: &SignatureRequest, call: &CallSite) -> Option<String> {
    argument_list(style, signature, request, call).map(|(args, _)| args.join(", "))
}

/// The edits in the function's file: the declaration, the parameters'
/// uses, and its calls there.
pub fn edits(style: ImportStyle, text: &str, signature: &Signature, calls: &[CallSite], request: &SignatureRequest) -> Result<Vec<Edit>, String> {
    check(style, signature, request)?;
    let mut out: Vec<Edit> = Vec::new();
    if !signature.constructor && request.name != signature.name {
        out.push((signature.name_range.clone(), request.name.clone()));
    }
    let original = &text[signature.params_range.clone()];
    out.push((signature.params_range.clone(), parameter_list(style, signature, request, original)));
    // The return type.
    let ty = request.return_type.trim();
    if !signature.constructor && ty != signature.return_type {
        match style {
            ImportStyle::Java => out.push((signature.return_range.clone(), ty.to_owned())),
            ImportStyle::Kotlin if ty.is_empty() => out.push((signature.return_range.clone(), String::new())),
            ImportStyle::Kotlin if signature.return_type.is_empty() => out.push((signature.return_range.clone(), format!(": {ty}"))),
            ImportStyle::Kotlin => out.push((signature.return_range.clone(), format!(": {ty}"))),
        }
    }
    // Renamed parameters' uses.
    for p in &request.params {
        let Some(old) = p.old.and_then(|i| signature.params.get(i)) else { continue };
        if old.name != p.name {
            out.extend(old.uses.iter().map(|r| (r.clone(), p.name.clone())));
        }
    }
    out.extend(calls.iter().flat_map(|c| call_edits(style, signature, request, c)));
    Ok(out)
}

/// A call's edits: its name and arguments.
pub fn call_edits(style: ImportStyle, signature: &Signature, request: &SignatureRequest, call: &CallSite) -> Vec<Edit> {
    let mut out = Vec::new();
    if !signature.constructor && request.name != signature.name && !call.name_range.is_empty() {
        out.push((call.name_range.clone(), request.name.clone()));
    }
    let Some((mut args, last_kept)) = argument_list(style, signature, request, call) else { return out };
    match &call.lambda {
        None => out.push((call.args_open + 1..call.args_close, args.join(", "))),
        // Still the last parameter: the lambda stays outside the parentheses.
        Some(lambda) if last_kept => {
            args.pop();
            let inside = args.join(", ");
            if call.has_parens() {
                out.push((call.args_open + 1..call.args_close, inside));
            } else if !inside.is_empty() {
                out.push((call.args_open..call.args_open, format!("({inside})")));
            }
        }
        // Moved or removed: it goes inside the parentheses, as an argument.
        Some(lambda) => out.push((call.args_open..lambda.end, format!("({})", args.join(", ")))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::introduce_parameter::apply;

    fn kotlin() -> (String, Signature) {
        let text = "fun total(price: Double, count: Int = 1): Double {\n    return price * count\n}\n".to_owned();
        let p = |name: &str, ty: &str, default: Option<&str>, text: &str, uses: Vec<Range<usize>>| Param { name: name.into(), ty: ty.into(), prefix: String::new(), default: default.map(Into::into), text: text.into(), uses, property: false };
        let price = text.find("price *").unwrap();
        let count = text.find("count\n").unwrap();
        let open = text.find('(').unwrap();
        let close = text.find(')').unwrap();
        let ret = text.find(": Double {").unwrap();
        let signature = Signature {
            name: "total".into(),
            name_range: 4..9,
            params: vec![p("price", "Double", None, "price: Double", vec![price..price + 5]), p("count", "Int", Some("1"), "count: Int = 1", vec![count..count + 5])],
            params_range: open + 1..close,
            return_type: "Double".into(),
            return_range: close + 1..ret + 8,
            constructor: false,
        };
        (text, signature)
    }

    #[test]
    fn changes_a_kotlin_signature() {
        let (text, s) = kotlin();
        let mut request = SignatureRequest::of(&s);
        request.name = "sum".into();
        request.params.swap(0, 1);
        request.params[1].name = "amount".into();
        request.params.push(NewParam { old: None, name: "tax".into(), ty: "Double".into(), value: "0.2".into() });
        let calls = [
            CallSite { name_range: 0..5, args_open: 5, args_close: 14, args: vec![(None, "2.0".into()), (None, "3".into())], receiver: None, ..Default::default() },
            CallSite { name_range: 0..5, args_open: 5, args_close: 10, args: vec![(None, "4.0".into())], receiver: None, ..Default::default() },
        ];
        let out = apply(&text, edits(ImportStyle::Kotlin, &text, &s, &[], &request).unwrap());
        assert_eq!(out, "fun sum(count: Int = 1, amount: Double, tax: Double): Double {\n    return amount * count\n}\n");
        // Positional calls follow the order; one that left `count` to its default names the rest.
        assert_eq!(call_arguments(ImportStyle::Kotlin, &s, &request, &calls[0]).as_deref(), Some("3, 2.0, 0.2"));
        assert_eq!(call_arguments(ImportStyle::Kotlin, &s, &request, &calls[1]).as_deref(), Some("amount = 4.0, tax = 0.2"));
        // The return type goes, then comes back.
        request.return_type = String::new();
        let out = apply(&text, edits(ImportStyle::Kotlin, &text, &s, &[], &request).unwrap());
        assert!(out.starts_with("fun sum(count: Int = 1, amount: Double, tax: Double) {"), "{out}");
        request.params[2].value.clear();
        assert!(check(ImportStyle::Kotlin, &s, &request).unwrap_err().contains("default value"));
    }

    #[test]
    fn changes_a_java_signature() {
        let text = "int total(int price, int count) { return price * count; }";
        let s = Signature {
            name: "total".into(),
            name_range: 4..9,
            params: vec![
                Param { name: "price".into(), ty: "int".into(), text: "int price".into(), ..Default::default() },
                Param { name: "count".into(), ty: "int".into(), text: "int count".into(), ..Default::default() },
            ],
            params_range: 10..30,
            return_type: "int".into(),
            return_range: 0..3,
            constructor: false,
        };
        let mut request = SignatureRequest::of(&s);
        request.params.remove(1);
        request.params[0].ty = "long".into();
        request.return_type = "long".into();
        let out = apply(text, edits(ImportStyle::Java, text, &s, &[], &request).unwrap());
        assert_eq!(out, "long total(long price) { return price * count; }");
        let call = CallSite { name_range: 0..5, args_open: 5, args_close: 11, args: vec![(None, "1".into()), (None, "2".into())], receiver: None, ..Default::default() };
        assert_eq!(call_arguments(ImportStyle::Java, &s, &request, &call).as_deref(), Some("1"));
    }

    #[test]
    fn keeps_parameters_on_their_lines() {
        let (_, s) = kotlin();
        let request = SignatureRequest::of(&s);
        let list = parameter_list(ImportStyle::Kotlin, &s, &request, "\n    price: Double,\n    count: Int = 1,\n");
        assert_eq!(list, "\n    price: Double,\n    count: Int = 1,\n");
    }
}

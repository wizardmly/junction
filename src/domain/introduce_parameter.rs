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
    /// The function's own `this` in it (its object, or a Kotlin extension
    /// function's receiver): a call on another object passes that object.
    pub this_refs: Vec<Range<usize>>,
    /// Its type when it shows it; empty when unknown.
    pub ty: String,
    /// Names the parameter mustn't take.
    pub taken: Vec<String>,
    /// Inline Function of a function with statements: `expression` is
    /// its body's statements.
    pub block: Option<crate::domain::inline_function::Block>,
    /// A Java method or constructor with a body: where an overload
    /// delegating to it can go.
    pub delegate: Option<DelegateSite>,
    /// Java: the class's fields with getters used in the expression.
    pub fields: Vec<FieldUse>,
    /// The top-level class the function is in (private members are
    /// accessible there), and the file's package.
    pub class_range: Range<usize>,
    pub package: String,
}

/// A field used in the expression that has a getter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldUse {
    /// Where (relative to the expression): `rate` or `this.rate`.
    pub range: Range<usize>,
    /// `getRate`.
    pub getter: String,
    pub access: Access,
}

/// A member's visibility, as far as calls elsewhere see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Private,
    /// Package-private or protected: visible in its package.
    Package,
    Public,
}

/// IntelliJ's "Replace fields used in expressions with their getters".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldGetters {
    #[default]
    None,
    /// Where the field isn't accessible from the call.
    Inaccessible,
    All,
}

/// Where a call is, as far as the fields' access goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallPlace {
    /// Outside the function's top-level class.
    pub outside: bool,
    /// The calling file's package.
    pub package: String,
}

impl CallPlace {
    /// A call in another file.
    pub fn elsewhere(text: &str) -> Self {
        CallPlace { outside: true, package: crate::domain::debugger::package_of(text) }
    }
}

/// What "Delegate via overloading method" needs of a Java method: IntelliJ
/// copies it before itself with a body calling the new signature.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DelegateSite {
    /// Where the declaration (its modifiers) starts.
    pub start: usize,
    /// The declaration up to its body's `{`.
    pub header: String,
    /// Its line's indent, and one indent step.
    pub indent: String,
    pub unit: String,
    /// It returns a value (not `void`, not a constructor).
    pub returns: bool,
    pub constructor: bool,
    /// Its `@Override` annotations, with the space after them: the
    /// original no longer overrides anything.
    pub overrides: Vec<Range<usize>>,
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
    /// Kotlin's trailing lambda `{ … }`, the last parameter's value. With
    /// no parentheses (`run { … }`) `args_open` and `args_close` are both
    /// right after the name.
    pub lambda: Option<Range<usize>>,
    /// The trailing lambda's text.
    pub lambda_text: String,
    /// A reference (`::name`, `Type::name`), not a call: no arguments.
    pub reference: bool,
    /// The statement the call is in, when statements can go before it.
    pub statement: Option<StatementSite>,
}

/// The statement a call is in, in a block: where statements inlined
/// before it go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatementSite {
    /// The statement's start (after its line's indent).
    pub start: usize,
    pub indent: String,
    /// The call is the whole statement (`f(x)` / `f(x);`).
    pub whole: bool,
    /// The function the call is in, and the names declared there.
    pub function: Range<usize>,
    pub taken: Vec<String>,
}

impl CallSite {
    /// The value passed for parameter `ix` of `params`: by name, by
    /// position, or the trailing lambda for the last one.
    pub fn value_for<'a>(&'a self, params: &[String], ix: usize) -> Option<&'a str> {
        let name = params.get(ix)?;
        let positional = self.args.iter().take_while(|(n, _)| n.is_none()).count();
        if let Some((_, v)) = self.args.iter().find(|(n, _)| n.as_deref() == Some(name.as_str())) {
            return Some(v);
        }
        if ix < positional {
            return Some(&self.args[ix].1);
        }
        (self.lambda.is_some() && ix + 1 == params.len()).then_some(self.lambda_text.as_str())
    }

    /// Whether the call has parentheses (`run { … }` has none).
    pub fn has_parens(&self) -> bool {
        self.lambda.is_none() || self.args_open != self.args_close
    }

    /// Where the call's arguments end: the trailing lambda's end, else
    /// past the `)`.
    pub fn end(&self) -> usize {
        self.lambda.as_ref().map_or(self.args_close + 1, |l| l.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterRequest {
    pub name: String,
    pub ty: String,
    pub replace_all: bool,
    /// Kotlin: `name: Type = expression`, the calls unchanged.
    pub default_value: bool,
    /// Java: `final Type name`.
    pub declare_final: bool,
    /// Java: an overload with the old signature passes the expression;
    /// the calls unchanged.
    pub delegate: bool,
    pub getters: FieldGetters,
}

impl ParameterRequest {
    /// The calls are left as they are.
    pub fn keeps_calls(&self) -> bool {
        self.default_value || self.delegate
    }
}

/// A text edit.
pub type Edit = (Range<usize>, String);

/// The expression a call passes: the function's parameters in it replaced
/// by the call's arguments, its members qualified by the call's receiver.
pub fn argument_for(plan: &ParameterPlan, call: &CallSite) -> String {
    argument_at(plan, call, FieldGetters::None, &CallPlace::default())
}

/// `argument_for`, the fields replaced by their getters as asked for a
/// call at `place`.
pub fn argument_at(plan: &ParameterPlan, call: &CallSite, getters: FieldGetters, place: &CallPlace) -> String {
    let mut edits: Vec<Edit> = Vec::new();
    let replaced: Vec<&FieldUse> = plan
        .fields
        .iter()
        .filter(|f| match getters {
            FieldGetters::None => false,
            FieldGetters::All => true,
            FieldGetters::Inaccessible => match f.access {
                Access::Private => place.outside,
                Access::Package => place.outside && place.package != plan.package,
                Access::Public => false,
            },
        })
        .collect();
    let receiver = call.receiver.as_ref().map(|r| if r.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') { r.clone() } else { format!("({r})") });
    for f in &replaced {
        let prefix = receiver.as_ref().map_or_else(String::new, |r| format!("{r}."));
        edits.push((f.range.clone(), format!("{prefix}{}()", f.getter)));
    }
    let in_replaced = |r: &Range<usize>| replaced.iter().any(|f| f.range.start <= r.start && r.end <= f.range.end);
    for (range, param) in &plan.uses {
        // A named argument by name, else by position (or the trailing lambda).
        if let Some(value) = call.value_for(&plan.params, *param) {
            let simple = value.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
            edits.push((range.clone(), if simple { value.to_owned() } else { format!("({value})") }));
        }
    }
    if let Some(receiver) = &receiver {
        for range in plan.members.iter().filter(|r| !in_replaced(r)) {
            edits.push((range.start..range.start, format!("{receiver}.")));
        }
        for range in plan.this_refs.iter().filter(|r| !in_replaced(r)) {
            edits.push((range.clone(), receiver.clone()));
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
        let fin = if request.declare_final { "final " } else { "" };
        format!("{sep}{fin}{} {}", request.ty.trim(), request.name)
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
    if request.delegate {
        let site = plan.delegate.as_ref().ok_or("The method has no body to delegate from")?;
        let args: Vec<&str> = plan.params.iter().map(String::as_str).chain([plan.expression.as_str()]).collect();
        let call = format!("{}({})", if site.constructor { "this" } else { plan.function.as_str() }, args.join(", "));
        let call = if site.returns { format!("return {call}") } else { call };
        let (indent, unit) = (&site.indent, &site.unit);
        out.push((site.start..site.start, format!("{} {{\n{indent}{unit}{call};\n{indent}}}\n\n{indent}", site.header)));
        for r in &site.overrides {
            out.push((r.clone(), String::new()));
        }
    } else if !(request.default_value && !java) {
        for call in calls_here {
            let place = CallPlace { outside: !(plan.class_range.start <= call.name_range.start && call.name_range.start < plan.class_range.end), package: plan.package.clone() };
            out.extend(call_edit_at(plan, call, &request.name, request.getters, &place));
        }
    }
    Ok(out)
}

/// The new argument added to a call: named when the call names its
/// arguments or leaves some to their defaults; the fields replaced by
/// their getters as asked for a call at `place`.
pub fn call_edit_at(plan: &ParameterPlan, call: &CallSite, name: &str, getters: FieldGetters, place: &CallPlace) -> Option<Edit> {
    if call.reference {
        return None;
    }
    let value = argument_at(plan, call, getters, place);
    let passed = call.args.len() + usize::from(call.lambda.is_some());
    let named = call.args.iter().any(|(n, _)| n.is_some()) || passed < plan.params.len();
    // The new last parameter takes the trailing lambda's place: the lambda
    // goes inside the parentheses.
    if let (Some(lambda), Some(last)) = (&call.lambda, plan.params.last()) {
        let mut args: Vec<String> = call.args.iter().map(|(n, v)| n.as_ref().map_or_else(|| v.clone(), |n| format!("{n} = {v}"))).collect();
        args.push(if named { format!("{last} = {}", call.lambda_text) } else { call.lambda_text.clone() });
        args.push(if named { format!("{name} = {value}") } else { value });
        return Some((call.args_open..lambda.end, format!("({})", args.join(", "))));
    }
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
            this_refs: vec![],
            ty: "Double".into(),
            taken: vec![],
            block: None,
            delegate: None,
            fields: vec![],
            class_range: 0..text.len(),
            package: String::new(),
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
        let request = ParameterRequest { name: "amount".into(), ty: "Double".into(), replace_all: true, default_value: false, declare_final: false, delegate: false, getters: FieldGetters::None };
        let out = apply(text, edits(ImportStyle::Kotlin, &p, &calls, &request).unwrap());
        assert_eq!(out, "fun total(price: Double, amount: Double): Double {\n    return amount\n}\nfun use() = total(2.0, 2.0 * rate) + cart.total(base + 1, (base + 1) * cart.rate)\n");
        // A default value: the calls stay.
        let request = ParameterRequest { default_value: true, ..request };
        let out = apply(text, edits(ImportStyle::Kotlin, &p, &calls, &request).unwrap());
        assert!(out.starts_with("fun total(price: Double, amount: Double = price * rate): Double {") && out.contains("total(2.0) + cart.total(base + 1)"), "{out}");
        // A taken name; a call with named arguments gets a named one.
        assert!(check(ImportStyle::Kotlin, &p, &ParameterRequest { name: "price".into(), ty: "Double".into(), replace_all: true, default_value: false, declare_final: false, delegate: false, getters: FieldGetters::None }).is_err());
        let named = CallSite { args_close: 10, args: vec![(Some("price".into()), "4.0".into())], receiver: None, ..Default::default() };
        assert_eq!(call_edit_at(&p, &named, "amount", FieldGetters::None, &CallPlace::default()), Some((10..10, ", amount = 4.0 * rate".into())));
        // A trailing lambda isn't last any more: it goes inside.
        let mut p = p.clone();
        p.params.push("block".into());
        let call = "total(2.0) { }";
        let lambda = CallSite { args_open: 5, args_close: 9, args: vec![(None, "2.0".into())], lambda: Some(11..14), lambda_text: "{ }".into(), ..Default::default() };
        assert_eq!(apply(call, call_edit_at(&p, &lambda, "amount", FieldGetters::None, &CallPlace::default()).into_iter().collect()), "total(2.0, { }, 2.0 * rate)");
    }

    #[test]
    fn java_has_no_default() {
        let text = "int total(int price) { return price * rate; }\nint use() { return total(3); }\n";
        let mut p = plan(text, "price * rate");
        p.ty = "int".into();
        let calls = [CallSite { args_close: text.find("3)").unwrap() + 1, args: vec![(None, "3".into())], receiver: None, ..Default::default() }];
        let request = ParameterRequest { name: "amount".into(), ty: "int".into(), replace_all: false, default_value: true, declare_final: false, delegate: false, getters: FieldGetters::None };
        let out = apply(text, edits(ImportStyle::Java, &p, &calls, &request).unwrap());
        assert_eq!(out, "int total(int price, int amount) { return amount; }\nint use() { return total(3, 3 * rate); }\n");
    }

    #[test]
    fn java_final_and_delegate() {
        let text = "class C {\n    @Override\n    public int total(int price) {\n        return price * rate;\n    }\n    int use() { return total(3); }\n}\n";
        let mut p = plan(text, "price * rate");
        p.params_close = text.find("price)").unwrap() + 5;
        p.ty = "int".into();
        let start = text.find("@Override").unwrap();
        p.delegate = Some(DelegateSite {
            start,
            header: "@Override\n    public int total(int price)".into(),
            indent: "    ".into(),
            unit: "    ".into(),
            returns: true,
            constructor: false,
            overrides: vec![start..start + 14],
        });
        let calls = [CallSite { args_close: text.find("3)").unwrap() + 1, args: vec![(None, "3".into())], receiver: None, ..Default::default() }];
        let request = ParameterRequest { name: "amount".into(), ty: "int".into(), replace_all: true, default_value: false, declare_final: true, delegate: true, getters: FieldGetters::None };
        let out = apply(text, edits(ImportStyle::Java, &p, &calls, &request).unwrap());
        assert_eq!(
            out,
            "class C {\n    @Override\n    public int total(int price) {\n        return total(price, price * rate);\n    }\n\n    public int total(int price, final int amount) {\n        return amount;\n    }\n    int use() { return total(3); }\n}\n"
        );
        // A constructor calls this(…); a void method has no return.
        let site = p.delegate.as_mut().unwrap();
        site.constructor = true;
        site.returns = false;
        let out = apply(text, edits(ImportStyle::Java, &p, &calls, &ParameterRequest { declare_final: false, ..request }).unwrap());
        assert!(out.contains("{\n        this(price, price * rate);\n    }") && out.contains("total(int price, int amount)"), "{out}");
    }

    #[test]
    fn fields_become_getters() {
        // return price * rate + this.base; rate is private, base package-private.
        let text = "class C {\n    int total(int price) { return price * rate + this.base; }\n    void use(C c) { total(1); c.total(2); }\n}\nclass D { void use(C c) { c.total(3); } }\n";
        let expr = "price * rate + this.base";
        let mut p = plan(text, expr);
        p.class_range = 0..text.find("class D").unwrap();
        p.this_refs = vec![15..19];
        p.fields = vec![
            FieldUse { range: 8..12, getter: "getRate".into(), access: Access::Private },
            FieldUse { range: 15..24, getter: "getBase".into(), access: Access::Package },
        ];
        let call = |at: &str, receiver: Option<&str>| {
            let start = text.find(at).unwrap();
            CallSite { name_range: start..start + 5, args_close: start + at.len() - 1, args: vec![(None, "9".into())], receiver: receiver.map(Into::into), ..Default::default() }
        };
        let inside = CallPlace { outside: false, package: String::new() };
        let outside = CallPlace { outside: true, package: String::new() };
        let abroad = CallPlace { outside: true, package: "other".into() };
        let c = call("total(2)", Some("c"));
        assert_eq!(argument_at(&p, &c, FieldGetters::None, &outside), "9 * c.rate + c.base");
        assert_eq!(argument_at(&p, &c, FieldGetters::Inaccessible, &inside), "9 * c.rate + c.base");
        assert_eq!(argument_at(&p, &c, FieldGetters::Inaccessible, &outside), "9 * c.getRate() + c.base");
        assert_eq!(argument_at(&p, &c, FieldGetters::Inaccessible, &abroad), "9 * c.getRate() + c.getBase()");
        assert_eq!(argument_at(&p, &c, FieldGetters::All, &inside), "9 * c.getRate() + c.getBase()");
        assert_eq!(argument_at(&p, &call("total(1)", None), FieldGetters::All, &inside), "9 * getRate() + getBase()");
        // In the function's file: the calls in its class keep the fields, others get getters.
        let calls = [call("total(1)", None), call("total(2)", Some("c")), call("total(3)", Some("c"))];
        let request = ParameterRequest { name: "amount".into(), ty: "int".into(), replace_all: true, default_value: false, declare_final: false, delegate: false, getters: FieldGetters::Inaccessible };
        let out = apply(text, edits(ImportStyle::Java, &p, &calls, &request).unwrap());
        assert!(out.contains("total(1, 9 * rate + this.base); c.total(2, 9 * c.rate + c.base)") && out.contains("c.total(3, 9 * c.getRate() + c.base)"), "{out}");
    }
}

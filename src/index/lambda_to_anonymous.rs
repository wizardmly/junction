//! Replace lambda with anonymous class (Java, IntelliJ's
//! LambdaToAnonymousIntention): `Function<String, Integer> f = s -> s.length();`
//! → `new Function<String, Integer>() { @Override public Integer apply(String s) { return s.length(); } }`.
//! The functional interface comes from the declared type the lambda is
//! assigned to (a local, a field, or the method's return type); its method
//! from the JDK's well-known interfaces or the interface's declaration the
//! index finds.

use tree_sitter::Node;

use super::catches::Classes;
use super::extract_method::{children, descendants};
use super::lang::Lang;
use super::lambdas::{line_indent, reindent};
use super::symbols;
use crate::domain::intentions::Intention;

/// A functional interface's method: its name, parameter types and return
/// type in terms of the interface's type parameters, and whether it
/// throws (`Callable.call() throws Exception`).
struct Method {
    name: String,
    parameters: Vec<String>,
    returns: String,
    type_parameters: Vec<String>,
    throws: Option<String>,
}

/// java.util.function and friends.
fn known(simple: &str) -> Option<Method> {
    let m = |name: &str, parameters: &[&str], returns: &str, type_parameters: &[&str]| Method {
        name: name.into(),
        parameters: parameters.iter().map(|p| p.to_string()).collect(),
        returns: returns.into(),
        type_parameters: type_parameters.iter().map(|p| p.to_string()).collect(),
        throws: None,
    };
    Some(match simple {
        "Runnable" => m("run", &[], "void", &[]),
        "Callable" => Method { throws: Some("Exception".into()), ..m("call", &[], "V", &["V"]) },
        "Comparator" => m("compare", &["T", "T"], "int", &["T"]),
        "Supplier" => m("get", &[], "T", &["T"]),
        "Consumer" => m("accept", &["T"], "void", &["T"]),
        "BiConsumer" => m("accept", &["T", "U"], "void", &["T", "U"]),
        "Function" => m("apply", &["T"], "R", &["T", "R"]),
        "BiFunction" => m("apply", &["T", "U"], "R", &["T", "U", "R"]),
        "Predicate" => m("test", &["T"], "boolean", &["T"]),
        "BiPredicate" => m("test", &["T", "U"], "boolean", &["T", "U"]),
        "UnaryOperator" => m("apply", &["T"], "T", &["T"]),
        "BinaryOperator" => m("apply", &["T", "T"], "T", &["T"]),
        "IntPredicate" => m("test", &["int"], "boolean", &[]),
        "IntFunction" => m("apply", &["int"], "R", &["R"]),
        "ToIntFunction" => m("applyAsInt", &["T"], "int", &["T"]),
        "IntConsumer" => m("accept", &["int"], "void", &[]),
        "BooleanSupplier" => m("getAsBoolean", &[], "boolean", &[]),
        "IntSupplier" => m("getAsInt", &[], "int", &[]),
        "OnClickListener" | "View.OnClickListener" => m("onClick", &["View"], "void", &[]),
        "OnLongClickListener" | "View.OnLongClickListener" => m("onLongClick", &["View"], "boolean", &[]),
        _ => return None,
    })
}

/// The single abstract method of a Java interface declared in `source`.
fn declared(source: &str, simple: &str) -> Option<Method> {
    let tree = symbols::parse(Lang::Java, source)?;
    let declaration = descendants(tree.root_node())
        .into_iter()
        .find(|n| n.kind() == "interface_declaration" && n.child_by_field_name("name").is_some_and(|c| &source[c.byte_range()] == simple))?;
    if children(declaration).iter().any(|c| c.kind() == "extends_interfaces") {
        return None;
    }
    let type_parameters = declaration
        .child_by_field_name("type_parameters")
        .map(|t| children(t).into_iter().filter_map(|p| children(p).into_iter().find(|c| c.kind() == "type_identifier" || c.kind() == "identifier").map(|n| source[n.byte_range()].to_owned())).collect())
        .unwrap_or_default();
    let body = declaration.child_by_field_name("body")?;
    let methods: Vec<Node> = children(body)
        .into_iter()
        .filter(|m| m.kind() == "method_declaration" && m.child_by_field_name("body").is_none())
        .filter(|m| {
            let modifiers = children(*m).into_iter().find(|c| c.kind() == "modifiers").map_or("", |c| &source[c.byte_range()]);
            !modifiers.contains("static") && !modifiers.contains("default")
        })
        .collect();
    let [method] = methods[..] else { return None };
    let parameters = children(method.child_by_field_name("parameters")?)
        .into_iter()
        .filter(|p| p.kind() == "formal_parameter")
        .filter_map(|p| p.child_by_field_name("type").map(|t| source[t.byte_range()].to_owned()))
        .collect();
    let throws = children(method).into_iter().find(|c| c.kind() == "throws").map(|t| source[t.byte_range()].trim_start_matches("throws").trim().to_owned());
    Some(Method { name: source[method.child_by_field_name("name")?.byte_range()].to_owned(), parameters, returns: source[method.child_by_field_name("type")?.byte_range()].to_owned(), type_parameters, throws })
}

/// `? super T` / `? extends T` → `T`, `?` → `Object`.
fn bound(argument: &str) -> String {
    let a = argument.trim();
    match a.strip_prefix('?') {
        Some(rest) => {
            let rest = rest.trim();
            rest.strip_prefix("super").or_else(|| rest.strip_prefix("extends")).map_or("Object".to_owned(), |t| t.trim().to_owned())
        }
        None => a.to_owned(),
    }
}

/// `type` with each of the interface's type parameters replaced by its
/// argument (whole words only).
fn substitute(ty: &str, names: &[String], arguments: &[String]) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match names.iter().position(|n| n == word) {
            Some(i) => out.push_str(arguments.get(i).map_or("Object", |a| a.as_str())),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in ty.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Replace lambda with anonymous class, the caret on the lambda's
/// parameters or arrow. An expression body becomes `return …;` (a
/// statement for a void method); `this` / `super` inside stop it.
pub fn lambda_to_anonymous(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let mut node = tree.root_node().descendant_for_byte_range(caret, caret)?;
    let lambda = loop {
        if node.kind() == "lambda_expression" {
            break node;
        }
        node = node.parent()?;
    };
    let body = lambda.child_by_field_name("body")?;
    // On the parameters or the arrow, not in the body.
    if caret > body.start_byte() || caret == body.start_byte() && body.kind() != "block" {
        return None;
    }
    // The declared type it is assigned to.
    let declarator = lambda.parent().filter(|p| p.kind() == "variable_declarator");
    let ty = match declarator {
        Some(d) => d.parent().filter(|p| matches!(p.kind(), "local_variable_declaration" | "field_declaration"))?.child_by_field_name("type")?,
        None => {
            let ret = lambda.parent().filter(|p| p.kind() == "return_statement")?;
            std::iter::successors(Some(ret), |n| n.parent()).find(|n| n.kind() == "method_declaration")?.child_by_field_name("type")?
        }
    };
    let ty_text = &text[ty.byte_range()];
    let (raw, arguments) = match ty.kind() {
        "generic_type" => {
            let raw = children(ty).into_iter().find(|c| matches!(c.kind(), "type_identifier" | "scoped_type_identifier"))?;
            let args = children(ty).into_iter().find(|c| c.kind() == "type_arguments")?;
            (&text[raw.byte_range()], children(args).into_iter().map(|a| bound(&text[a.byte_range()])).collect::<Vec<_>>())
        }
        "type_identifier" | "scoped_type_identifier" => (ty_text, Vec::new()),
        _ => return None,
    };
    let simple = raw.rsplit('.').next()?;
    let method = classes(simple).iter().find_map(|s| declared(s, simple)).or_else(|| known(raw)).or_else(|| known(simple))?;
    // Lambda parameters: `x`, `(x, y)`, `(String x)`.
    let params = lambda.child_by_field_name("parameters")?;
    let names: Vec<(String, Option<String>)> = match params.kind() {
        "identifier" => vec![(text[params.byte_range()].to_owned(), None)],
        "inferred_parameters" => children(params).into_iter().map(|p| (text[p.byte_range()].to_owned(), None)).collect(),
        "formal_parameters" => children(params)
            .into_iter()
            .filter(|p| p.kind() == "formal_parameter")
            .map(|p| Some((text[p.child_by_field_name("name")?.byte_range()].to_owned(), p.child_by_field_name("type").map(|t| text[t.byte_range()].to_owned()))))
            .collect::<Option<_>>()?,
        _ => return None,
    };
    if names.len() != method.parameters.len() {
        return None;
    }
    // `this` and `super` would mean the anonymous class.
    if descendants(body).iter().any(|d| d.kind() == "this" || d.kind() == "super") {
        return None;
    }
    let returns = substitute(&method.returns, &method.type_parameters, &arguments);
    let parameters: Vec<String> = names
        .iter()
        .zip(&method.parameters)
        .map(|((name, own), ty)| format!("{} {name}", own.clone().unwrap_or_else(|| substitute(ty, &method.type_parameters, &arguments))))
        .collect();
    let base = line_indent(text, lambda.start_byte()).to_owned();
    let unit = "    ";
    let inner = format!("{base}{unit}");
    let body_text = if body.kind() == "block" {
        reindent(&text[body.byte_range()], &base, &inner)
    } else {
        let expression = &text[body.byte_range()];
        let statement = if returns == "void" { format!("{expression};") } else { format!("return {expression};") };
        format!("{{\n{inner}{unit}{statement}\n{inner}}}")
    };
    let created = if arguments.is_empty() && ty.kind() != "generic_type" { raw.to_owned() } else { ty_text.to_owned() };
    // `new Comparator<? super T>()` can't be written; use the bounds.
    let created = if created.contains('?') { format!("{raw}<{}>", arguments.join(", ")) } else { created };
    let throws = method.throws.as_ref().map_or(String::new(), |t| format!(" throws {t}"));
    let replacement = format!(
        "new {created}() {{\n{inner}@Override\n{inner}public {returns} {}({}){throws} {body_text}\n{base}}}",
        method.name,
        parameters.join(", ")
    );
    Some(Intention { title: "Replace lambda with anonymous class".into(), range: lambda.byte_range(), replacement, caret: lambda.start_byte() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(text: &str, at: &str, classes: Classes) -> Option<String> {
        lambda_to_anonymous(text, text.find(at).unwrap(), classes).map(|i| format!("{}{}{}", &text[..i.range.start], i.replacement, &text[i.range.end..]))
    }

    #[test]
    fn replaces_lambda_with_anonymous_class() {
        let none = |_: &str| Vec::new();
        let text = "class A {\n    void f() {\n        Function<String, Integer> f = s -> s.length();\n    }\n}";
        assert_eq!(
            convert(text, "s ->", &none).unwrap(),
            "class A {\n    void f() {\n        Function<String, Integer> f = new Function<String, Integer>() {\n            @Override\n            public Integer apply(String s) {\n                return s.length();\n            }\n        };\n    }\n}"
        );
        let text = "class A {\n    void f() {\n        Runnable r = () -> {\n            go();\n        };\n    }\n}";
        assert_eq!(
            convert(text, "() ->", &none).unwrap(),
            "class A {\n    void f() {\n        Runnable r = new Runnable() {\n            @Override\n            public void run() {\n                go();\n            }\n        };\n    }\n}"
        );
        let text = "class A {\n    Comparator<? super User> c = (a, b) -> a.age - b.age;\n}";
        assert!(convert(text, "(a, b)", &none).unwrap().contains("new Comparator<User>() {\n        @Override\n        public int compare(User a, User b) {\n            return a.age - b.age;\n        }\n    };"));
        // A project interface, from its declaration.
        let classes = |n: &str| if n == "Listener" { vec!["interface Listener<T> { void changed(T value, int index); }".to_owned()] } else { Vec::new() };
        let text = "class A {\n    Listener<String> l = (v, i) -> show(v);\n}";
        assert!(convert(text, "(v, i)", &classes).unwrap().contains("public void changed(String v, int i) {\n            show(v);\n        }"));
        // `this` inside, an unknown type, the caret in the body: none.
        assert_eq!(convert("class A { Runnable r = () -> this.go(); }", "() ->", &none), None);
        assert_eq!(convert("class A { Unknown r = () -> go(); }", "() ->", &none), None);
        assert_eq!(convert("class A { Runnable r = () -> go(); }", "go", &none), None);
    }
}

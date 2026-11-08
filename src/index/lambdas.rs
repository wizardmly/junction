//! Alt+Enter intentions between lambdas and what they stand for, as
//! IntelliJ offers them: Replace with lambda (Java, an anonymous class of a
//! functional interface, AnonymousCanBeLambdaInspection), Replace lambda
//! with method reference (Java, LambdaCanBeMethodReferenceInspection) and
//! Convert lambda to reference (Kotlin, ConvertLambdaToReferenceIntention).
//! Whether a type is a functional interface, and what a lambda's `it` is,
//! come from the declarations the index finds.

use tree_sitter::Node;

use super::catches::Classes;
use super::extract_method::{children, descendants};
use super::lang::Lang;
use super::symbols;
use crate::domain::intentions::Intention;

fn innermost<'t>(root: Node<'t>, at: usize) -> Option<Node<'t>> {
    root.descendant_for_byte_range(at, at)
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(Some(node), |n| n.parent())
}

/// A whole word `name` in `text`.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
    })
}

/// Interfaces with one abstract method that the JDK and the Android SDK
/// declare, for when their sources aren't there.
const FUNCTIONAL: &[&str] = &[
    "Runnable", "Callable", "Comparator", "Supplier", "Consumer", "BiConsumer", "Function", "BiFunction", "Predicate", "BiPredicate", "UnaryOperator", "BinaryOperator",
    "IntFunction", "IntPredicate", "IntConsumer", "IntSupplier", "IntUnaryOperator", "IntBinaryOperator", "ToIntFunction", "ToLongFunction", "ToDoubleFunction", "LongFunction",
    "DoubleFunction", "BooleanSupplier", "Executor", "ThreadFactory", "FileFilter", "FilenameFilter", "PathMatcher", "UncaughtExceptionHandler", "ActionListener",
    "OnClickListener", "OnLongClickListener", "OnTouchListener", "OnFocusChangeListener", "OnKeyListener", "OnCheckedChangeListener", "OnItemClickListener",
    "OnItemLongClickListener", "OnEditorActionListener", "OnMenuItemClickListener", "OnDismissListener", "OnCancelListener", "OnShowListener", "OnRefreshListener",
    "OnScrollChangeListener", "OnLayoutChangeListener", "OnApplyWindowInsetsListener", "OnPreDrawListener", "OnGlobalLayoutListener", "OnCompletionListener",
    "OnPreparedListener", "OnErrorListener", "Observer", "ValueCallback",
];

/// Whether the interface named `simple` has a single abstract method: from
/// its declaration (Java, or a Kotlin `fun interface`) when the index has
/// it, else from the known list.
fn functional(simple: &str, classes: Classes) -> bool {
    for source in classes(simple) {
        if let Some(found) = functional_in(&source, simple) {
            return found;
        }
    }
    FUNCTIONAL.contains(&simple)
}

fn functional_in(source: &str, simple: &str) -> Option<bool> {
    if source.contains("fun interface") || source.contains("interface ") && !source.contains(';') && source.contains("fun ") {
        // Kotlin: only a `fun interface` takes a Java lambda.
        let tree = symbols::parse(Lang::Kotlin, source)?;
        let declaration = descendants(tree.root_node()).into_iter().find(|n| {
            n.kind() == "class_declaration" && children(*n).iter().any(|c| c.kind() == "type_identifier" && &source[c.byte_range()] == simple)
        })?;
        let head = &source[declaration.start_byte()..declaration.end_byte().min(declaration.start_byte() + 200)];
        return Some(head.contains("fun interface"));
    }
    let tree = symbols::parse(Lang::Java, source)?;
    let declaration = descendants(tree.root_node()).into_iter().find(|n| {
        matches!(n.kind(), "interface_declaration" | "class_declaration" | "enum_declaration" | "record_declaration")
            && n.child_by_field_name("name").is_some_and(|c| &source[c.byte_range()] == simple)
    })?;
    if declaration.kind() != "interface_declaration" {
        return Some(false);
    }
    // Inherited abstract methods aren't known.
    if children(declaration).iter().any(|c| c.kind() == "extends_interfaces") {
        return None;
    }
    let body = declaration.child_by_field_name("body")?;
    let abstract_methods = children(body)
        .into_iter()
        .filter(|m| m.kind() == "method_declaration" && m.child_by_field_name("body").is_none())
        .filter(|m| {
            let modifiers = children(*m).into_iter().find(|c| c.kind() == "modifiers").map_or("", |c| &source[c.byte_range()]);
            let name = m.child_by_field_name("name").map_or("", |n| &source[n.byte_range()]);
            !mentions(modifiers, "static") && !mentions(modifiers, "default") && !matches!(name, "equals" | "hashCode" | "toString")
        })
        .count();
    Some(abstract_methods == 1)
}

/// The indent of the line `at` is on.
fn line_indent(text: &str, at: usize) -> &str {
    let line = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let rest = &text[line..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

/// A block's text moved from a line indented `from` to one indented `to`.
fn reindent(block: &str, from: &str, to: &str) -> String {
    block
        .split('\n')
        .enumerate()
        .map(|(i, line)| match (i, line.strip_prefix(from)) {
            (0, _) => line.to_owned(),
            (_, Some(rest)) => format!("{to}{rest}"),
            _ => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A Java block's statements without comments.
fn statements(block: Node) -> Vec<Node> {
    children(block).into_iter().filter(|c| !c.kind().ends_with("comment")).collect()
}

/// Replace with lambda (Java): `new View.OnClickListener() { @Override
/// public void onClick(View v) { … } }` → `v -> …`, for an anonymous class
/// of a functional interface with nothing but that one method, which
/// doesn't use `this` or `super` (they'd mean the outer class in a
/// lambda) and whose parameters no local of the enclosing method shadows.
/// A body of one `return x;` or one expression statement becomes an
/// expression lambda.
pub fn anonymous_to_lambda(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let creation = ancestors(innermost(tree.root_node(), caret)?).find(|n| n.kind() == "object_creation_expression")?;
    let class_body = children(creation).into_iter().find(|c| c.kind() == "class_body")?;
    // On `new Type()`, not in the body.
    if caret > class_body.start_byte() {
        return None;
    }
    let arguments = creation.child_by_field_name("arguments")?;
    if !children(arguments).is_empty() {
        return None;
    }
    let ty = creation.child_by_field_name("type")?;
    let ty_text = &text[ty.byte_range()];
    let simple = ty_text.split('<').next()?.rsplit('.').next()?.trim();
    let members: Vec<Node> = children(class_body).into_iter().filter(|c| !c.kind().ends_with("comment")).collect();
    let [method] = members[..] else { return None };
    if method.kind() != "method_declaration" || method.child_by_field_name("type_parameters").is_some() {
        return None;
    }
    let body = method.child_by_field_name("body")?;
    if descendants(body).iter().any(|n| matches!(n.kind(), "this" | "super")) {
        return None;
    }
    let mut names = Vec::new();
    for p in children(method.child_by_field_name("parameters")?) {
        if p.kind() != "formal_parameter" {
            return None;
        }
        names.push(&text[p.child_by_field_name("name")?.byte_range()]);
    }
    // A lambda parameter may not shadow a local of the enclosing method.
    if let Some(scope) = ancestors(creation).skip(1).find(|n| matches!(n.kind(), "method_declaration" | "constructor_declaration" | "lambda_expression")) {
        let outside = format!("{}{}", &text[scope.start_byte()..creation.start_byte()], &text[creation.end_byte()..scope.end_byte()]);
        if names.iter().any(|n| mentions(&outside, n)) {
            return None;
        }
    }
    if !functional(simple, classes) {
        return None;
    }
    let parameters = match &names[..] {
        [one] => (*one).to_owned(),
        _ => format!("({})", names.join(", ")),
    };
    let lambda_body = match &statements(body)[..] {
        [only] if only.kind() == "return_statement" && only.named_child_count() == 1 => text[only.named_child(0)?.byte_range()].to_owned(),
        [only] if only.kind() == "expression_statement" => text[only.named_child(0)?.byte_range()].to_owned(),
        _ => reindent(&text[body.byte_range()], line_indent(text, method.start_byte()), line_indent(text, creation.start_byte())),
    };
    Some(Intention { title: "Replace with lambda".into(), range: creation.byte_range(), replacement: format!("{parameters} -> {lambda_body}"), caret: creation.start_byte() })
}

/// The arguments' texts of a Java call.
fn argument_texts<'t>(call: Node, text: &'t str) -> Option<Vec<&'t str>> {
    Some(children(call.child_by_field_name("arguments")?).iter().map(|a| &text[a.byte_range()]).collect())
}

/// Replace lambda with method reference (Java): `x -> foo(x)` →
/// `this::foo` (`Outer::foo` for a static method of the class),
/// `s -> System.out.println(s)` → `System.out::println`, `x -> new Item(x)`
/// → `Item::new`, and `(String s) -> s.trim()` → `String::trim` where the
/// first parameter's declared type is the receiver.
pub fn lambda_to_method_reference(text: &str, caret: usize) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let lambda = ancestors(innermost(tree.root_node(), caret)?).find(|n| n.kind() == "lambda_expression")?;
    let params = lambda.child_by_field_name("parameters")?;
    // Each parameter: its name and declared type.
    let mut names: Vec<(&str, Option<&str>)> = Vec::new();
    match params.kind() {
        "identifier" => names.push((&text[params.byte_range()], None)),
        "inferred_parameters" => names.extend(children(params).iter().map(|p| (&text[p.byte_range()], None))),
        "formal_parameters" => {
            for p in children(params) {
                if p.kind() != "formal_parameter" {
                    return None;
                }
                names.push((&text[p.child_by_field_name("name")?.byte_range()], p.child_by_field_name("type").map(|t| &text[t.byte_range()])));
            }
        }
        _ => return None,
    }
    let mut body = lambda.child_by_field_name("body")?;
    if body.kind() == "block" {
        let [only] = statements(body)[..] else { return None };
        if !matches!(only.kind(), "return_statement" | "expression_statement") || only.named_child_count() != 1 {
            return None;
        }
        body = only.named_child(0)?;
    }
    let plain: Vec<&str> = names.iter().map(|(n, _)| *n).collect();
    let reference = match body.kind() {
        "method_invocation" => {
            let name = &text[body.child_by_field_name("name")?.byte_range()];
            if body.child_by_field_name("type_arguments").is_some() {
                return None;
            }
            let args = argument_texts(body, text)?;
            match body.child_by_field_name("object") {
                // `foo(a, b)`: a method of this class.
                None if args == plain => {
                    let class = ancestors(lambda).find(|n| matches!(n.kind(), "class_declaration" | "enum_declaration" | "record_declaration"))?;
                    let class_name = &text[class.child_by_field_name("name")?.byte_range()];
                    let declared = descendants(class.child_by_field_name("body")?)
                        .into_iter()
                        .find(|m| m.kind() == "method_declaration" && m.child_by_field_name("name").is_some_and(|n| &text[n.byte_range()] == name))?;
                    let modifiers = children(declared).into_iter().find(|c| c.kind() == "modifiers").map_or("", |c| &text[c.byte_range()]);
                    let in_static = ancestors(lambda)
                        .find(|n| matches!(n.kind(), "method_declaration" | "field_declaration"))
                        .and_then(|m| children(m).into_iter().find(|c| c.kind() == "modifiers"))
                        .is_some_and(|m| mentions(&text[m.byte_range()], "static"));
                    if mentions(modifiers, "static") {
                        format!("{class_name}::{name}")
                    } else if in_static {
                        return None;
                    } else {
                        format!("this::{name}")
                    }
                }
                None => return None,
                Some(object) => {
                    let object_text = &text[object.byte_range()];
                    if args == plain && !plain.iter().any(|p| mentions(object_text, p)) && !descendants(object).iter().any(|n| matches!(n.kind(), "method_invocation" | "object_creation_expression")) {
                        // `System.out.println(s)`: a bound reference.
                        format!("{object_text}::{name}")
                    } else if let (Some((first, Some(ty))), true) = (names.first(), object_text == names.first()?.0) {
                        // `(String s) -> s.trim()`: the first parameter receives the call.
                        if args[..] != plain[1..] || ty.contains('<') || *first != object_text {
                            return None;
                        }
                        format!("{ty}::{name}")
                    } else {
                        return None;
                    }
                }
            }
        }
        "object_creation_expression" if children(body).iter().all(|c| c.kind() != "class_body") => {
            if argument_texts(body, text)? != plain {
                return None;
            }
            let ty = &text[body.child_by_field_name("type")?.byte_range()];
            format!("{}::new", ty.split('<').next()?)
        }
        _ => return None,
    };
    Some(Intention { title: "Replace lambda with method reference".into(), range: lambda.byte_range(), replacement: reference, caret: lambda.start_byte() })
}

/// The functions on collections whose lambda takes one element.
const ELEMENT_FUNCTIONS: &[&str] = &[
    "map", "mapNotNull", "filter", "filterNot", "forEach", "onEach", "any", "all", "none", "count", "first", "firstOrNull", "last", "lastOrNull", "find", "findLast",
    "sortedBy", "sortedByDescending", "sortBy", "sortByDescending", "groupBy", "associateBy", "associateWith", "flatMap", "sumOf", "maxBy", "minBy", "maxByOrNull",
    "minByOrNull", "maxOf", "minOf", "takeWhile", "dropWhile", "partition", "distinctBy", "single", "singleOrNull", "indexOfFirst", "indexOfLast",
];

use super::extract_method::children as named;

/// Convert lambda to reference (Kotlin): `users.map { it.name }` →
/// `users.map(User::name)` where the receiver's type shows the element,
/// `{ u: User -> u.name }` → `User::name`, `{ x -> show(x) }` →
/// `::show` (`this::show` for a member of the enclosing class), with the
/// caret in the lambda. A trailing lambda moves into the parentheses.
pub fn lambda_to_reference(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Kotlin, text)?;
    let lambda = ancestors(innermost(tree.root_node(), caret)?).find(|n| n.kind() == "lambda_literal")?;
    let parts = named(lambda);
    let parameters: Vec<Node> = parts.iter().find(|c| c.kind() == "lambda_parameters").map(|p| named(*p)).unwrap_or_default();
    let statements = parts.iter().find(|c| c.kind() == "statements")?;
    let [statement] = named(*statements)[..] else { return None };
    // Each parameter's name and declared type; `it` when there are none.
    let mut params: Vec<(String, Option<String>)> = Vec::new();
    for p in &parameters {
        if p.kind() != "variable_declaration" {
            return None;
        }
        let pieces = named(*p);
        let name = pieces.iter().find(|c| c.kind() == "simple_identifier")?;
        let ty = pieces.iter().find(|c| c.kind().ends_with("type")).map(|t| text[t.byte_range()].to_owned());
        params.push((text[name.byte_range()].to_owned(), ty));
    }
    if parameters.is_empty() {
        params.push(("it".into(), None));
    }
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    // Where the lambda goes: its call, for the element type and the parentheses.
    let annotated = lambda.parent().filter(|p| p.kind() == "annotated_lambda");
    let suffix = annotated.and_then(|a| a.parent()).filter(|p| p.kind() == "call_suffix");
    let call = suffix.and_then(|s| s.parent()).filter(|c| c.kind() == "call_expression");
    let member = member_access(statement, text);
    let reference = match statement.kind() {
        // `show(x)`: a function of the file or a member of this class.
        "call_expression" if member.is_none() => {
            let [callee, call_suffix] = named(statement)[..] else { return None };
            if callee.kind() != "simple_identifier" {
                return None;
            }
            let [arguments] = named(call_suffix)[..] else { return None };
            if arguments.kind() != "value_arguments" {
                return None;
            }
            let args: Vec<&str> = named(arguments).iter().map(|a| text[a.byte_range()].trim()).collect();
            if args != names || (parameters.is_empty() && args.is_empty()) {
                return None;
            }
            let name = &text[callee.byte_range()];
            let class = ancestors(lambda).find(|n| matches!(n.kind(), "class_body" | "enum_class_body"));
            let declares = |scope: Node| {
                named(scope).into_iter().any(|d| d.kind() == "function_declaration" && named(d).iter().any(|c| c.kind() == "simple_identifier" && &text[c.byte_range()] == name))
            };
            if class.is_some_and(declares) {
                format!("this::{name}")
            } else if declares(tree.root_node()) {
                format!("::{name}")
            } else {
                return None;
            }
        }
        // `it.name` / `it.name()`: the element type's member.
        _ => {
            let (target, member) = member?;
            let [(param, declared)] = &params[..] else { return None };
            if &target != param {
                return None;
            }
            let ty = match declared {
                Some(ty) => ty.clone(),
                None => {
                    let call = call?;
                    let callee = call.named_child(0)?;
                    if callee.kind() != "navigation_expression" {
                        return None;
                    }
                    let function = text[callee.named_child(1)?.byte_range()].trim().trim_start_matches('.');
                    if !ELEMENT_FUNCTIONS.contains(&function) {
                        return None;
                    }
                    let receiver = callee.named_child(0)?;
                    let ty = super::introduce::kotlin_type(receiver, text)
                        .filter(|t| !t.is_empty())
                        .or_else(|| super::types::type_of(Lang::Kotlin, text, receiver.byte_range(), classes).map(|t| t.ty))?;
                    collection_element(&ty)?
                }
            };
            if ty.contains(['<', '?', '(']) {
                return None;
            }
            format!("{ty}::{member}")
        }
    };
    // `xs.map { … }` → `xs.map(Ref)`; `f(a) { … }` → `f(a, Ref)`.
    match (annotated, suffix) {
        (Some(annotated), Some(suffix)) => {
            let arguments = named(suffix).into_iter().find(|c| c.kind() == "value_arguments");
            if named(suffix).iter().any(|c| c.kind() == "type_arguments") {
                return None;
            }
            let (start, replacement) = match arguments {
                Some(a) => {
                    let inner = text[a.start_byte() + 1..a.end_byte() - 1].trim();
                    let joined = if inner.is_empty() { reference.clone() } else { format!("{inner}, {reference}") };
                    (a.start_byte(), format!("({joined})"))
                }
                // From the end of the callee, past the space before `{`.
                None => (call.and_then(|c| c.named_child(0)).map_or(suffix.start_byte(), |c| c.end_byte()), format!("({reference})")),
            };
            Some(Intention { title: "Convert lambda to reference".into(), range: start..annotated.end_byte(), replacement, caret: start })
        }
        _ => Some(Intention { title: "Convert lambda to reference".into(), range: lambda.byte_range(), replacement: reference, caret: lambda.start_byte() }),
    }
}

/// `x.name` or `x.name()`: `x` and `name`.
fn member_access(node: Node, text: &str) -> Option<(String, String)> {
    let navigation = match node.kind() {
        "navigation_expression" => node,
        "call_expression" => {
            let [callee, suffix] = named(node)[..] else { return None };
            let [arguments] = named(suffix)[..] else { return None };
            if arguments.kind() != "value_arguments" || !named(arguments).is_empty() {
                return None;
            }
            callee
        }
        _ => return None,
    };
    if navigation.kind() != "navigation_expression" {
        return None;
    }
    let target = navigation.named_child(0)?;
    if target.kind() != "simple_identifier" {
        return None;
    }
    let suffix = &text[navigation.named_child(1)?.byte_range()];
    let member = suffix.trim().strip_prefix('.')?.trim();
    member.chars().all(|c| c.is_alphanumeric() || c == '_').then(|| (text[target.byte_range()].to_owned(), member.to_owned()))
}

/// What a collection, array or sequence of this type holds.
fn collection_element(ty: &str) -> Option<String> {
    let ty = ty.trim_end_matches('?');
    let open = ty.find('<')?;
    let head = ty[..open].rsplit('.').next()?;
    if !matches!(
        head,
        "List" | "MutableList" | "ArrayList" | "Set" | "MutableSet" | "HashSet" | "LinkedHashSet" | "Collection" | "MutableCollection" | "Iterable" | "Sequence" | "Array"
    ) {
        return None;
    }
    Some(ty[open + 1..ty.len().checked_sub(1)?].trim().trim_start_matches("out ").to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, i: &Intention) -> String {
        format!("{}{}{}", &text[..i.range.start], i.replacement, &text[i.range.end..])
    }

    #[test]
    fn anonymous_class_to_lambda() {
        let none = |_: &str| Vec::new();
        let text = "class A {\n    void f(View b) {\n        b.setOnClickListener(new View.OnClickListener() {\n            @Override\n            public void onClick(View v) {\n                show(v);\n            }\n        });\n    }\n}\n";
        let i = anonymous_to_lambda(text, text.find("new View").unwrap() + 4, &none).unwrap();
        assert_eq!(i.title, "Replace with lambda");
        assert!(apply(text, &i).contains("b.setOnClickListener(v -> show(v));"), "{}", apply(text, &i));
        // Several statements stay a block, moved to the lambda's indent.
        let text = "class A {\n    void f() {\n        run(new Runnable() {\n            public void run() {\n                a();\n                b();\n            }\n        });\n    }\n}\n";
        let i = anonymous_to_lambda(text, text.find("Runnable").unwrap(), &none).unwrap();
        assert_eq!(apply(text, &i), "class A {\n    void f() {\n        run(() -> {\n            a();\n            b();\n        });\n    }\n}\n");
        // A return: an expression lambda.
        let text = "class A { Comparator<String> c = new Comparator<String>() { public int compare(String a, String b) { return a.length() - b.length(); } }; }";
        let i = anonymous_to_lambda(text, text.find("new").unwrap(), &none).unwrap();
        assert_eq!(i.replacement, "(a, b) -> a.length() - b.length()");
        // `this`, a field, an unknown interface, an abstract class from source: none.
        let uses_this = "class A { void f() { run(new Runnable() { public void run() { this.a(); } }); } }";
        assert!(anonymous_to_lambda(uses_this, uses_this.find("Runnable").unwrap(), &none).is_none());
        let field = "class A { void f() { run(new Runnable() { int n; public void run() { a(); } }); } }";
        assert!(anonymous_to_lambda(field, field.find("Runnable").unwrap(), &none).is_none());
        let unknown = "class A { void f() { run(new Task() { public void run() { a(); } }); } }";
        assert!(anonymous_to_lambda(unknown, unknown.find("Task").unwrap(), &none).is_none());
        let task = |n: &str| if n == "Task" { vec!["interface Task { void run(); default void stop() {} }".to_owned()] } else { Vec::new() };
        assert!(anonymous_to_lambda(unknown, unknown.find("Task").unwrap(), &task).is_some());
        let abstract_class = |n: &str| if n == "Task" { vec!["abstract class Task { abstract void run(); }".to_owned()] } else { Vec::new() };
        assert!(anonymous_to_lambda(unknown, unknown.find("Task").unwrap(), &abstract_class).is_none());
        let kotlin_fun = |n: &str| if n == "Task" { vec!["package p\n\nfun interface Task {\n    fun run()\n}\n".to_owned()] } else { Vec::new() };
        assert!(anonymous_to_lambda(unknown, unknown.find("Task").unwrap(), &kotlin_fun).is_some());
        // A parameter named like a local of the method: none.
        let shadow = "class A { void f() { View v = null; v.setOnClickListener(new View.OnClickListener() { public void onClick(View v) { g(); } }); } }";
        assert!(anonymous_to_lambda(shadow, shadow.find("new View").unwrap(), &none).is_none());
        // The caret in the body: none.
        let text = "class A { void f() { run(new Runnable() { public void run() { a(); } }); } }";
        assert!(anonymous_to_lambda(text, text.find("a()").unwrap(), &none).is_none());
    }

    #[test]
    fn lambda_to_java_method_reference() {
        let call = |text: &str, at: &str| lambda_to_method_reference(text, text.find(at).unwrap()).map(|i| apply(text, &i));
        let text = "class A { void f(List<String> xs) { xs.forEach(s -> System.out.println(s)); } }";
        assert_eq!(call(text, "s ->").unwrap(), "class A { void f(List<String> xs) { xs.forEach(System.out::println); } }");
        let text = "class A { void f(List<String> xs) { xs.forEach(s -> show(s)); } void show(String s) {} }";
        assert!(call(text, "s ->").unwrap().contains("xs.forEach(this::show)"));
        let text = "class A { static void f(List<String> xs) { xs.forEach(s -> show(s)); } static void show(String s) {} }";
        assert!(call(text, "s ->").unwrap().contains("xs.forEach(A::show)"));
        let text = "class A { static void f(List<String> xs) { xs.forEach(s -> show(s)); } void show(String s) {} }";
        assert_eq!(call(text, "s ->"), None);
        let text = "class A { Object f() { return x -> new Item(x); } }";
        assert!(call(text, "x ->").unwrap().contains("return Item::new;"));
        let text = "class A { Object f() { return (String s) -> s.trim(); } }";
        assert!(call(text, "(String").unwrap().contains("return String::trim;"));
        // The parameter used otherwise, or the receiver computed: none.
        assert_eq!(call("class A { Object f() { return s -> s.trim(); } }", "s ->"), None);
        assert_eq!(call("class A { Object f() { return s -> g(s, s); } }", "s ->"), None);
        assert_eq!(call("class A { Object f() { return s -> make().run(s); } }", "s ->"), None);
    }

    #[test]
    fn kotlin_lambda_to_reference() {
        let users = |n: &str| if n == "Repo" { vec!["class Repo {\n    fun users(): List<User> = emptyList()\n}\n".to_owned()] } else { Vec::new() };
        let convert = |text: &str, classes: Classes| lambda_to_reference(text, text.rfind("{ ").unwrap() + 1, classes).map(|i| apply(text, &i));
        let text = "fun f(xs: List<User>) = xs.map { it.name }";
        assert_eq!(convert(text, &users).unwrap(), "fun f(xs: List<User>) = xs.map(User::name)");
        let text = "fun f(xs: List<User>) = xs.sortedBy { it.age() }";
        assert_eq!(convert(text, &users).unwrap(), "fun f(xs: List<User>) = xs.sortedBy(User::age)");
        let text = "fun f(r: Repo) = r.users().filter { it.active }";
        assert_eq!(convert(text, &users).unwrap(), "fun f(r: Repo) = r.users().filter(User::active)");
        let text = "val g = { u: User -> u.name }";
        assert_eq!(convert(text, &users).unwrap(), "val g = User::name");
        let text = "fun show(x: Int) {}\nfun f(xs: List<Int>) { xs.forEach { x -> show(x) } }";
        assert_eq!(convert(text, &users).unwrap(), "fun show(x: Int) {}\nfun f(xs: List<Int>) { xs.forEach(::show) }");
        let text = "class A {\n    fun show(x: Int) {}\n    fun f(xs: List<Int>) = xs.forEach { show(it) }\n}";
        assert!(convert(text, &users).unwrap().contains("xs.forEach(this::show)"));
        let text = "fun f(xs: List<Int>) = run(1) { show(it) }\nfun show(x: Int) {}";
        assert!(convert(text, &users).unwrap().contains("run(1, ::show)"));
        // Unknown element type, generic element, an unknown function, two uses: none.
        assert_eq!(convert("fun f(xs: Foo) = xs.map { it.name }", &users), None);
        assert_eq!(convert("fun f(xs: List<List<Int>>) = xs.map { it.size }", &users), None);
        assert_eq!(convert("fun f(xs: List<Int>) = xs.forEach { show(it) }", &users), None);
        assert_eq!(convert("fun f(xs: List<User>) = xs.map { it.name + it.name }", &users), None);
    }
}

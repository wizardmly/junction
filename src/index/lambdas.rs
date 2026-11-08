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
pub(super) fn line_indent(text: &str, at: usize) -> &str {
    let line = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let rest = &text[line..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

/// A block's text moved from a line indented `from` to one indented `to`.
pub(super) fn reindent(block: &str, from: &str, to: &str) -> String {
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
pub(super) fn collection_element(ty: &str) -> Option<String> {
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

/// What a Kotlin class or file declares under a name: a function and its
/// parameters' names, or a property.
#[derive(Debug, PartialEq)]
enum KotlinMember {
    Function(Vec<String>),
    Property,
}

/// The parameters' names of a Kotlin `function_value_parameters` or
/// `class_parameters` node.
fn kotlin_parameter_names(parameters: Node, text: &str) -> Vec<String> {
    named(parameters)
        .into_iter()
        .filter(|p| matches!(p.kind(), "parameter" | "class_parameter"))
        .filter_map(|p| named(p).into_iter().find(|c| c.kind() == "simple_identifier").map(|n| text[n.byte_range()].to_owned()))
        .collect()
}

/// `name` among the declarations directly in `scope` (a file, a class
/// body); a function overloaded with different parameter counts is none.
fn kotlin_member(scope: Node, text: &str, name: &str) -> Option<KotlinMember> {
    let mut found: Option<KotlinMember> = None;
    for d in named(scope) {
        let named_here = |d: Node| named(d).iter().any(|c| c.kind() == "simple_identifier" && &text[c.byte_range()] == name);
        let member = match d.kind() {
            "function_declaration" if named_here(d) => {
                let params = named(d).into_iter().find(|c| c.kind() == "function_value_parameters").map(|p| kotlin_parameter_names(p, text)).unwrap_or_default();
                KotlinMember::Function(params)
            }
            "property_declaration" if named(d).iter().any(|v| v.kind() == "variable_declaration" && named_here(*v)) => KotlinMember::Property,
            _ => continue,
        };
        match (&found, &member) {
            (Some(KotlinMember::Function(a)), KotlinMember::Function(b)) if a.len() != b.len() => return None,
            (Some(_), _) => {}
            (None, _) => found = Some(member),
        }
    }
    found
}

/// The Kotlin class `class_name` in `source`: its body's member `name`, or
/// a `val` / `var` of its primary constructor.
fn kotlin_class_member(source: &str, class_name: &str, name: &str) -> Option<KotlinMember> {
    let tree = symbols::parse(Lang::Kotlin, source)?;
    let class = descendants(tree.root_node())
        .into_iter()
        .find(|n| matches!(n.kind(), "class_declaration" | "object_declaration") && named(*n).iter().any(|c| c.kind() == "type_identifier" && &source[c.byte_range()] == class_name))?;
    let primary = named(class).into_iter().find(|c| c.kind() == "primary_constructor");
    let property = primary.is_some_and(|p| {
        named(p).into_iter().flat_map(|c| if c.kind() == "class_parameters" { named(c) } else { vec![c] }).any(|c| {
            c.kind() == "class_parameter"
                && named(c).iter().any(|n| n.kind() == "simple_identifier" && &source[n.byte_range()] == name)
                && named(c).iter().any(|n| n.kind() == "binding_pattern_kind")
        })
    });
    if property {
        return Some(KotlinMember::Property);
    }
    let body = named(class).into_iter().find(|c| matches!(c.kind(), "class_body" | "enum_class_body"))?;
    kotlin_member(body, source, name)
}

/// The Kotlin class `class_name`'s primary constructor parameters, from
/// `source`.
fn kotlin_constructor(source: &str, class_name: &str) -> Option<Vec<String>> {
    let tree = symbols::parse(Lang::Kotlin, source)?;
    let class = descendants(tree.root_node())
        .into_iter()
        .find(|n| n.kind() == "class_declaration" && named(*n).iter().any(|c| c.kind() == "type_identifier" && &source[c.byte_range()] == class_name))?;
    // Secondary constructors could take other arguments.
    let body = named(class).into_iter().find(|c| c.kind() == "class_body");
    if body.is_some_and(|b| named(b).iter().any(|c| c.kind() == "secondary_constructor")) {
        return None;
    }
    Some(match named(class).into_iter().find(|c| c.kind() == "primary_constructor") {
        Some(p) => named(p).into_iter().find(|c| c.kind() == "class_parameters").map_or_else(|| kotlin_parameter_names(p, source), |c| kotlin_parameter_names(c, source)),
        None => Vec::new(),
    })
}

/// Kotlin standard functions (not properties) a `Type::name` reference
/// often names, for when the type's source isn't there.
const KOTLIN_FUNCTIONS: &[&str] = &[
    "trim", "lowercase", "uppercase", "toInt", "toLong", "toDouble", "toFloat", "toIntOrNull", "toLongOrNull", "toDoubleOrNull", "toString", "toList", "toSet",
    "toMutableList", "isEmpty", "isNotEmpty", "isBlank", "isNotBlank", "isNullOrEmpty", "isNullOrBlank", "hashCode", "reversed", "sorted", "first", "last", "capitalize",
];

/// A lambda's body as an argument goes outside the parentheses when it is
/// the last one (IntelliJ moves it out after converting): the edit over
/// `node` in `call(…, node)`, else over `node` alone.
fn place_lambda(node: Node, text: &str, lambda: String, title: &str) -> Intention {
    let argument = node.parent().filter(|p| p.kind() == "value_argument" && named(*p).len() == 1);
    let arguments = argument.and_then(|a| a.parent()).filter(|p| p.kind() == "value_arguments");
    let trailing = arguments.zip(argument).filter(|(args, arg)| named(*args).last().is_some_and(|l| l.id() == arg.id())).filter(|(args, _)| {
        // Not `f(…)(…)` or a named argument.
        !text[args.byte_range()].contains('=') && args.parent().is_some_and(|s| s.kind() == "call_suffix" && named(s).len() == 1)
    });
    match trailing {
        Some((arguments, _)) => {
            let others: Vec<&str> = named(arguments).iter().rev().skip(1).rev().map(|a| text[a.byte_range()].trim()).collect();
            let replacement = if others.is_empty() { format!(" {lambda}") } else { format!("({}) {lambda}", others.join(", ")) };
            Intention { title: title.into(), range: arguments.byte_range(), replacement, caret: arguments.start_byte() }
        }
        None => Intention { title: title.into(), range: node.byte_range(), replacement: lambda, caret: node.start_byte() },
    }
}

/// A Kotlin lambda calling `call` with these parameters: `{ show(it) }`,
/// `{ a, b -> show(a, b) }`, `{ show() }`.
fn kotlin_call_lambda(call: &str, params: &[String]) -> String {
    match params {
        [] => format!("{{ {call}() }}"),
        [_] => format!("{{ {call}(it) }}"),
        _ => format!("{{ {} -> {call}({}) }}", params.join(", "), params.join(", ")),
    }
}

/// Convert reference to lambda (Kotlin, IntelliJ's
/// ConvertReferenceToLambdaIntention): `User::name` → `{ it.name }`
/// (`{ it.name() }` for a function), `::show` / `this::show` →
/// `{ show(it) }`, `::User` → `{ User(it) }`, the parameters from the
/// declaration the index finds; as the last argument the lambda moves
/// out of the parentheses (`xs.map { it.name }`).
pub fn reference_to_lambda(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    const TITLE: &str = "Convert reference to lambda";
    let tree = symbols::parse(Lang::Kotlin, text)?;
    // `User::name` and `this::show` also parse as a navigation with a
    // `::name` suffix.
    let node = ancestors(innermost(tree.root_node(), caret)?).find(|n| {
        n.kind() == "callable_reference"
            || n.kind() == "navigation_expression"
                && n.named_child(0).is_some_and(|t| matches!(t.kind(), "this_expression" | "simple_identifier"))
                && n.named_child(1).is_some_and(|s| text[s.byte_range()].trim_start().starts_with("::"))
    })?;
    let parts = named(node);
    let name_node = parts.iter().rev().find(|c| c.kind() == "simple_identifier").copied().filter(|_| node.kind() == "callable_reference").or_else(|| parts.last().and_then(|s| named(*s).into_iter().find(|c| c.kind() == "simple_identifier")))?;
    let name = &text[name_node.byte_range()];
    let receiver = match node.kind() {
        "callable_reference" => parts.iter().find(|c| c.kind() == "type_identifier" || c.kind() == "user_type").map(|t| text[t.byte_range()].trim().to_owned()),
        _ => match parts.first() {
            Some(t) if t.kind() == "simple_identifier" => {
                let ty = text[t.byte_range()].to_owned();
                // `obj::run`, a bound reference: its receiver's type isn't known.
                if !ty.starts_with(char::is_uppercase) {
                    return None;
                }
                Some(ty)
            }
            _ => None,
        },
    };
    let sources = |class: &str| std::iter::once(text.to_owned()).chain(classes(class)).collect::<Vec<_>>();
    let lambda = match receiver {
        // `User::name`: a member of the type.
        Some(ty) => {
            if ty.contains(['<', '?', '.']) {
                return None;
            }
            let member = sources(&ty).iter().find_map(|s| kotlin_class_member(s, &ty, name));
            match member {
                Some(KotlinMember::Function(params)) if params.is_empty() => format!("{{ it.{name}() }}"),
                Some(KotlinMember::Function(params)) => {
                    let receiver = type_variable_name(&ty);
                    if params.contains(&receiver) {
                        return None;
                    }
                    format!("{{ {receiver}, {} -> {receiver}.{name}({}) }}", params.join(", "), params.join(", "))
                }
                Some(KotlinMember::Property) => format!("{{ it.{name} }}"),
                None if KOTLIN_FUNCTIONS.contains(&name) => format!("{{ it.{name}() }}"),
                None => format!("{{ it.{name} }}"),
            }
        }
        // `::show`, `::User`, `this::show`.
        None => {
            let class_body = ancestors(node).find(|n| matches!(n.kind(), "class_body" | "enum_class_body"));
            let declared = class_body.and_then(|b| kotlin_member(b, text, name)).or_else(|| kotlin_member(tree.root_node(), text, name));
            match declared {
                Some(KotlinMember::Function(params)) => kotlin_call_lambda(name, &params),
                Some(KotlinMember::Property) => return None,
                None if name.starts_with(char::is_uppercase) => {
                    let params = sources(name).iter().find_map(|s| kotlin_constructor(s, name))?;
                    kotlin_call_lambda(name, &params)
                }
                None => return None,
            }
        }
    };
    Some(place_lambda(node, text, lambda, TITLE))
}

/// A Java method's or constructor's parameter names and whether it is
/// static, from the class `class_name` in `source`; overloads with
/// different parameter counts give none. `name` is `new` for a
/// constructor (none declared: no parameters).
fn java_method(source: &str, class_name: &str, name: &str) -> Option<(Vec<String>, bool)> {
    let tree = symbols::parse(Lang::Java, source)?;
    let class = descendants(tree.root_node()).into_iter().find(|n| {
        matches!(n.kind(), "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration") && n.child_by_field_name("name").is_some_and(|c| &source[c.byte_range()] == class_name)
    })?;
    let body = class.child_by_field_name("body")?;
    let kind = if name == "new" { "constructor_declaration" } else { "method_declaration" };
    let mut found: Option<(Vec<String>, bool)> = None;
    for m in children(body).into_iter().filter(|m| m.kind() == kind) {
        if name != "new" && m.child_by_field_name("name").is_none_or(|n| &source[n.byte_range()] != name) {
            continue;
        }
        let params: Vec<String> = children(m.child_by_field_name("parameters")?)
            .into_iter()
            .filter(|p| matches!(p.kind(), "formal_parameter" | "spread_parameter"))
            .filter_map(|p| {
                let n = p.child_by_field_name("name").or_else(|| children(p).into_iter().rev().find(|c| matches!(c.kind(), "identifier" | "variable_declarator")))?;
                let n = n.child_by_field_name("name").unwrap_or(n);
                Some(source[n.byte_range()].to_owned())
            })
            .collect();
        let modifiers = children(m).into_iter().find(|c| c.kind() == "modifiers").map_or("", |c| &source[c.byte_range()]);
        let entry = (params, mentions(modifiers, "static"));
        match &found {
            Some((a, _)) if a.len() != entry.0.len() => return None,
            Some(_) => {}
            None => found = Some(entry),
        }
    }
    if found.is_none() && name == "new" {
        return Some((Vec::new(), false));
    }
    found
}

/// JDK methods a method reference often names, for when their sources
/// aren't there: (type, method, parameters, static).
const JAVA_KNOWN: &[(&str, &str, &[&str], bool)] = &[
    ("String", "trim", &[], false),
    ("String", "length", &[], false),
    ("String", "isEmpty", &[], false),
    ("String", "isBlank", &[], false),
    ("String", "toUpperCase", &[], false),
    ("String", "toLowerCase", &[], false),
    ("String", "strip", &[], false),
    ("String", "valueOf", &["obj"], true),
    ("Object", "toString", &[], false),
    ("Object", "hashCode", &[], false),
    ("Integer", "parseInt", &["s"], true),
    ("Integer", "valueOf", &["i"], true),
    ("Long", "parseLong", &["s"], true),
    ("Double", "parseDouble", &["s"], true),
    ("Objects", "nonNull", &["obj"], true),
    ("Objects", "isNull", &["obj"], true),
    ("Objects", "toString", &["o"], true),
];

/// IntelliJ's first name for a variable of a type: `s` for a String,
/// `o` for an Object, else the type's name decapitalized.
fn type_variable_name(ty: &str) -> String {
    match ty {
        "String" => "s".into(),
        "Object" | "Any" => "o".into(),
        _ => {
            let mut c = ty.chars();
            c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// A name not used in `scope`, from `base` (`s`, `s1`, …).
fn unused_name(base: &str, scope: &str) -> String {
    (0..).map(|i| if i == 0 { base.to_owned() } else { format!("{base}{i}") }).find(|n| !mentions(scope, n)).unwrap_or_default()
}

/// Replace method reference with lambda (Java, IntelliJ's
/// MethodRefCanBeReplacedWithLambdaInspection): `this::show` →
/// `s -> show(s)`, `Item::new` → `name -> new Item(name)`, `String::trim`
/// → `s -> s.trim()`, `Integer::parseInt` → `s -> Integer.parseInt(s)`,
/// `System.out::println` → `x -> System.out.println(x)`; the parameters
/// are the declaration's (the index's), renamed where the enclosing
/// method already uses the name.
pub fn method_reference_to_lambda(text: &str, caret: usize, classes: Classes) -> Option<Intention> {
    let tree = symbols::parse(Lang::Java, text)?;
    let reference = ancestors(innermost(tree.root_node(), caret)?).find(|n| n.kind() == "method_reference")?;
    let parts = children(reference);
    let qualifier = *parts.first()?;
    let qualifier_text = &text[qualifier.byte_range()];
    let name = if text[reference.byte_range()].trim_end().ends_with("::new") { "new" } else { &text[parts.get(1)?.byte_range()] };
    if text[reference.byte_range()].contains('<') {
        return None;
    }
    let enclosing = ancestors(reference).find(|n| matches!(n.kind(), "class_declaration" | "enum_declaration" | "record_declaration"));
    let enclosing_name = enclosing.and_then(|c| c.child_by_field_name("name")).map(|n| &text[n.byte_range()]);
    let lookup = |class: &str| -> Option<(Vec<String>, bool)> {
        std::iter::once(text.to_owned())
            .chain(classes(class))
            .find_map(|s| java_method(&s, class, name))
            .or_else(|| JAVA_KNOWN.iter().find(|(t, m, _, _)| *t == class && *m == name).map(|(_, _, p, st)| (p.iter().map(|s| (*s).to_owned()).collect(), *st)))
    };
    let scope = ancestors(reference).find(|n| matches!(n.kind(), "method_declaration" | "constructor_declaration" | "lambda_expression" | "field_declaration"));
    let scope_text = scope.map_or("", |s| &text[s.byte_range()]);
    let rename = |params: Vec<String>| params.iter().map(|p| unused_name(p, scope_text)).collect::<Vec<_>>();
    let lambda = |params: &[String], body: String| match params {
        [one] => format!("{one} -> {body}"),
        _ => format!("({}) -> {body}", params.join(", ")),
    };
    let replacement = match qualifier.kind() {
        // `this::show`: a method of this class.
        "this" => {
            let (params, _) = java_method(text, enclosing_name?, name)?;
            let params = rename(params);
            lambda(&params, format!("{name}({})", params.join(", ")))
        }
        // `System.out::println`: a bound receiver.
        "field_access" if qualifier_text == "System.out" || qualifier_text == "System.err" => {
            if !matches!(name, "println" | "print") {
                return None;
            }
            let x = unused_name("x", scope_text);
            format!("{x} -> {qualifier_text}.{name}({x})")
        }
        "identifier" | "scoped_type_identifier" | "type_identifier" | "field_access" if qualifier_text.starts_with(char::is_uppercase) || qualifier_text.contains('.') => {
            let class = qualifier_text.rsplit('.').next()?.trim();
            if !class.starts_with(char::is_uppercase) {
                return None;
            }
            let (params, is_static) = lookup(class)?;
            let params = rename(params);
            let args = params.join(", ");
            if name == "new" {
                lambda(&params, format!("new {qualifier_text}({args})"))
            } else if is_static {
                // A static method of this class needs no qualifier.
                let call = if Some(class) == enclosing_name { format!("{name}({args})") } else { format!("{qualifier_text}.{name}({args})") };
                lambda(&params, call)
            } else {
                // `String::trim`: the first parameter receives the call.
                let receiver = unused_name(&type_variable_name(class), scope_text);
                let mut all = vec![receiver.clone()];
                all.extend(params.iter().cloned());
                lambda(&all, format!("{receiver}.{name}({args})"))
            }
        }
        _ => return None,
    };
    Some(Intention { title: "Replace method reference with lambda".into(), range: reference.byte_range(), replacement, caret: reference.start_byte() })
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

    #[test]
    fn kotlin_reference_to_lambda() {
        let user = |n: &str| if n == "User" { vec!["class User(val name: String, age: Int) {\n    fun age(): Int = 0\n    fun greet(other: User): String = \"\"\n}\n".to_owned()] } else { Vec::new() };
        let convert = |text: &str, at: &str, classes: Classes| reference_to_lambda(text, text.find(at).unwrap() + 2, classes).map(|i| apply(text, &i));
        assert_eq!(convert("val a = xs.map(User::name)", "User::", &user).unwrap(), "val a = xs.map { it.name }");
        assert_eq!(convert("val a = xs.sortedBy(User::age)", "User::", &user).unwrap(), "val a = xs.sortedBy { it.age() }");
        assert_eq!(convert("val a = xs.map(String::trim)", "String::", &user).unwrap(), "val a = xs.map { it.trim() }");
        assert_eq!(convert("val g = User::greet", "User::", &user).unwrap(), "val g = { user, other -> user.greet(other) }");
        let text = "fun show(x: Int) {}\nfun f(xs: List<Int>) = run(1, ::show)";
        assert!(convert(text, "::show", &user).unwrap().contains("run(1) { show(it) }"));
        let text = "class A {\n    fun show(a: Int, b: Int) {}\n    fun f() = g(this::show)\n}";
        assert!(convert(text, "this::", &user).unwrap().contains("g { a, b -> show(a, b) }"));
        assert_eq!(convert("val a = pairs.map(::User)", "::User", &user).unwrap(), "val a = pairs.map { name, age -> User(name, age) }");
        let one = |n: &str| if n == "Item" { vec!["class Item(val id: Int)".to_owned()] } else { Vec::new() };
        assert_eq!(convert("val a = ids.map(::Item)", "::Item", &one).unwrap(), "val a = ids.map { Item(it) }");
        // Not the last argument: in place.
        let text = "fun show(x: Int) {}\nval a = f(::show, 1)";
        assert!(convert(text, "::show", &user).unwrap().contains("f({ show(it) }, 1)"));
        // Unknown function: none.
        assert_eq!(convert("val a = xs.forEach(::nope)", "::nope", &user), None);
    }

    #[test]
    fn java_method_reference_to_lambda() {
        let item = |n: &str| if n == "Item" { vec!["class Item { Item(String name) {} static Item of(String name, int n) { return null; } }".to_owned()] } else { Vec::new() };
        let convert = |text: &str, at: &str| method_reference_to_lambda(text, text.find(at).unwrap() + 1, &item).map(|i| apply(text, &i));
        let text = "class A { void f(List<String> xs) { xs.forEach(System.out::println); } }";
        assert!(convert(text, "System").unwrap().contains("xs.forEach(x -> System.out.println(x));"));
        let text = "class A { void f(List<String> xs) { xs.forEach(this::show); } void show(String line) {} }";
        assert!(convert(text, "this").unwrap().contains("xs.forEach(line -> show(line));"));
        let text = "class A { Object f() { return Item::new; } }";
        assert!(convert(text, "Item").unwrap().contains("return name -> new Item(name);"));
        let text = "class A { Object f() { return Item::of; } }";
        assert!(convert(text, "Item").unwrap().contains("return (name, n) -> Item.of(name, n);"));
        let text = "class A { Object f() { return String::trim; } }";
        assert!(convert(text, "String").unwrap().contains("return s -> s.trim();"));
        let text = "class A { Object f(String s) { return Integer::parseInt; } }";
        assert!(convert(text, "Integer").unwrap().contains("return s1 -> Integer.parseInt(s1);"));
        let text = "class A { static Object f() { return A::g; } static int g(int v) { return v; } }";
        assert!(convert(text, "A::").unwrap().contains("return v -> g(v);"));
        // Unknown method or receiver: none.
        assert_eq!(convert("class A { Object f() { return Foo::bar; } }", "Foo"), None);
        assert_eq!(convert("class A { Object f(B b) { return b::run; } }", "b::"), None);
    }
}

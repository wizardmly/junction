//! The exceptions a try / catch surrounder catches, as IntelliJ's: Java's
//! JavaWithTryCatchSurrounder catches what the wrapped code throws and
//! doesn't handle (ExceptionUtil.getUnhandledExceptions: the checked
//! exceptions in the `throws` of the methods and constructors it calls and
//! of its `throw new X(…)`, else the unchecked ones), Kotlin's
//! TryCatchExceptionUtil what the functions it calls declare (a Java
//! method's `throws`, a Kotlin `@Throws(X::class)`). The callee is found
//! by its receiver: none or `this` (the enclosing class and its
//! superclasses), a class name (`Thread.sleep`), a local variable or
//! parameter, or a field of the class, by their declared type.

use std::ops::Range;

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;
use crate::domain::imports::header;

/// An exception to catch: as the code names it, and qualified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thrown {
    pub name: String,
    pub qualified: String,
}

/// The texts of the files declaring a class of this simple name: the
/// project's, then the libraries'.
pub type Classes<'a> = &'a dyn Fn(&str) -> Vec<String>;

/// The JDK's unchecked exceptions, for when their sources aren't there.
const UNCHECKED: &[&str] = &[
    "RuntimeException", "Error", "IllegalArgumentException", "IllegalStateException", "NullPointerException", "IndexOutOfBoundsException",
    "ArrayIndexOutOfBoundsException", "StringIndexOutOfBoundsException", "ClassCastException", "ArithmeticException", "UnsupportedOperationException",
    "NumberFormatException", "ConcurrentModificationException", "NoSuchElementException", "SecurityException", "UncheckedIOException",
    "DateTimeException", "AssertionError", "OutOfMemoryError", "StackOverflowError",
];

/// Common JDK checked exceptions, (name, package, superclass), for when
/// the JDK's sources aren't there.
const JDK_CHECKED: &[(&str, &str, &str)] = &[
    ("Exception", "java.lang", "Throwable"),
    ("InterruptedException", "java.lang", "Exception"),
    ("CloneNotSupportedException", "java.lang", "Exception"),
    ("ReflectiveOperationException", "java.lang", "Exception"),
    ("ClassNotFoundException", "java.lang", "ReflectiveOperationException"),
    ("NoSuchMethodException", "java.lang", "ReflectiveOperationException"),
    ("NoSuchFieldException", "java.lang", "ReflectiveOperationException"),
    ("IllegalAccessException", "java.lang", "ReflectiveOperationException"),
    ("InstantiationException", "java.lang", "ReflectiveOperationException"),
    ("IOException", "java.io", "Exception"),
    ("FileNotFoundException", "java.io", "IOException"),
    ("EOFException", "java.io", "IOException"),
    ("MalformedURLException", "java.net", "IOException"),
    ("SocketException", "java.net", "IOException"),
    ("UnknownHostException", "java.net", "IOException"),
    ("URISyntaxException", "java.net", "Exception"),
    ("SQLException", "java.sql", "Exception"),
    ("ParseException", "java.text", "Exception"),
    ("ExecutionException", "java.util.concurrent", "Exception"),
    ("TimeoutException", "java.util.concurrent", "Exception"),
    ("GeneralSecurityException", "java.security", "Exception"),
    ("NoSuchAlgorithmException", "java.security", "GeneralSecurityException"),
];

/// What the whole lines (or the expression) `range` of `text` throw to be
/// caught, in IntelliJ's order (a subclass before its superclass); empty
/// for a plain `Exception`.
pub fn caught_exceptions(lang: Lang, text: &str, range: Range<usize>, classes: Classes) -> Vec<Thrown> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let finder = Finder { classes, own: text };
    let mut found: Vec<Thrown> = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.end_byte() <= range.start || node.start_byte() >= range.end {
            continue;
        }
        if node.start_byte() >= range.start && node.end_byte() <= range.end {
            for t in finder.thrown_in(lang, node) {
                if !found.contains(&t) {
                    found.push(t);
                }
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    if lang == Lang::Java {
        // The checked ones, else all (getThrownExceptions).
        let checked: Vec<Thrown> = found.iter().filter(|t| finder.checked(&t.qualified)).cloned().collect();
        if !checked.is_empty() {
            found = checked;
        }
    }
    // A subclass before its superclass (ExceptionUtil.sortExceptionsByHierarchy).
    let ancestors: Vec<Vec<String>> = found.iter().map(|t| finder.ancestors(&t.qualified)).collect();
    let mut order: Vec<usize> = (0..found.len()).collect();
    for i in 0..order.len() {
        for j in i + 1..order.len() {
            let (a, b) = (order[i], order[j]);
            if ancestors[b].contains(&found[a].name) {
                order.swap(i, j);
            }
        }
    }
    order.into_iter().map(|i| found[i].clone()).collect()
}

struct Finder<'a> {
    classes: Classes<'a>,
    own: &'a str,
}

/// The class declarations' node kinds.
fn is_class(kind: &str) -> bool {
    matches!(kind, "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "object_declaration")
}

fn class_name<'t>(node: Node<'t>, text: &'t str) -> Option<&'t str> {
    let name = node.child_by_field_name("name").or_else(|| {
        let mut c = node.walk();
        node.named_children(&mut c).find(|n| n.kind() == "type_identifier")
    })?;
    Some(&text[name.byte_range()])
}

/// The class named `name` in a file.
fn find_class<'t>(root: Node<'t>, text: &str, name: &str) -> Option<Node<'t>> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if is_class(n.kind()) && class_name(n, text) == Some(name) {
            return Some(n);
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    None
}

/// `Foo<Bar>[]` → `Foo`; `a.b.Foo` stays qualified.
fn bare_type(ty: &str) -> &str {
    let ty = ty.split('<').next().unwrap_or(ty);
    ty.trim_end_matches("[]").trim_end_matches('?').trim()
}

fn argument_count(arguments: Option<Node>) -> usize {
    arguments.map_or(0, |a| {
        let mut c = a.walk();
        a.named_children(&mut c).filter(|n| !n.kind().contains("comment") && n.kind() != "annotated_lambda").count()
    })
}

impl Finder<'_> {
    /// The text of the class `name` as `context` (a file's text) sees it,
    /// with that class's simple name.
    fn class_text(&self, name: &str, context: &str) -> Option<String> {
        let simple = name.rsplit('.').next().unwrap_or(name);
        if symbols::parse(lang_of(context), context).is_some_and(|t| find_class(t.root_node(), context, simple).is_some()) {
            return Some(context.to_owned());
        }
        let qualified = self.qualify(name, context);
        let package = qualified.rsplit_once('.').map_or("", |(p, _)| p);
        let candidates = (self.classes)(simple);
        let found = candidates.iter().find(|t| crate::domain::debugger::package_of(t) == package).or(candidates.first());
        found.cloned()
    }

    /// The qualified name `name` has in `context`: its import, its file's
    /// package, java.lang or a wildcard import's package where a class of
    /// that name is.
    fn qualify(&self, name: &str, context: &str) -> String {
        if name.contains('.') {
            return name.to_owned();
        }
        let h = header(context);
        if let Some((_, q)) = h.imports.iter().find(|(n, _)| n == name) {
            return q.clone();
        }
        let in_package = |p: &str| if p.is_empty() { name.to_owned() } else { format!("{p}.{name}") };
        let packages: Vec<String> = (self.classes)(name).iter().map(|t| crate::domain::debugger::package_of(t)).collect();
        if packages.contains(&h.package) || symbols::parse(lang_of(context), context).is_some_and(|t| find_class(t.root_node(), context, name).is_some()) {
            return in_package(&h.package);
        }
        if let Some(p) = packages.iter().find(|p| *p == "java.lang" || h.wildcards.contains(p)) {
            return in_package(p);
        }
        if packages.is_empty() {
            if let Some((_, p, _)) = JDK_CHECKED.iter().find(|(n, _, _)| *n == name) {
                return in_package(p);
            }
            if UNCHECKED.contains(&name) {
                return in_package("java.lang");
            }
        }
        packages.first().map_or_else(|| in_package(&h.package), |p| in_package(p))
    }

    /// The simple names of a class's superclasses, nearest first.
    fn ancestors(&self, qualified: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = qualified.to_owned();
        let mut context = self.own.to_owned();
        for _ in 0..12 {
            let simple = current.rsplit('.').next().unwrap_or(&current).to_owned();
            let Some(text) = self.class_text(&current, &context) else {
                // A JDK exception without its sources.
                let mut name = simple;
                while let Some((_, _, parent)) = JDK_CHECKED.iter().find(|(n, _, _)| *n == name) {
                    out.push(parent.to_string());
                    name = parent.to_string();
                }
                break;
            };
            let Some(tree) = symbols::parse(Lang::Java, &text) else { break };
            let Some(class) = find_class(tree.root_node(), &text, &simple) else { break };
            let Some(superclass) = class.child_by_field_name("superclass").and_then(|s| s.named_child(0)) else { break };
            let name = bare_type(&text[superclass.byte_range()]).to_owned();
            current = self.qualify(&name, &text);
            out.push(name.rsplit('.').next().unwrap_or(&name).to_owned());
            context = text;
        }
        out
    }

    /// Whether an exception must be caught or declared.
    fn checked(&self, qualified: &str) -> bool {
        let simple = qualified.rsplit('.').next().unwrap_or(qualified);
        if UNCHECKED.contains(&simple) {
            return false;
        }
        let ancestors = self.ancestors(qualified);
        if ancestors.iter().any(|a| a == "RuntimeException" || a == "Error") {
            return false;
        }
        // Unknown: unchecked when its superclass is a known unchecked one.
        !ancestors.last().is_some_and(|a| UNCHECKED.contains(&a.as_str()))
    }

    /// The exceptions a statement or expression `node` of the own text
    /// throws.
    fn thrown_in(&self, lang: Lang, node: Node) -> Vec<Thrown> {
        let text = self.own;
        let mut out: Vec<Thrown> = Vec::new();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            match n.kind() {
                // Not run here.
                "lambda_expression" | "class_body" | "lambda_literal" | "anonymous_function" | "function_declaration" if n.id() != node.id() => continue,
                "try_statement" if lang == Lang::Java => {
                    // What its catches take isn't thrown on.
                    let mut c = n.walk();
                    let caught: Vec<String> = n
                        .named_children(&mut c)
                        .filter(|k| k.kind() == "catch_clause")
                        .flat_map(|k| {
                            let p = k.named_children(&mut k.walk()).find(|p| p.kind() == "catch_formal_parameter");
                            p.and_then(|p| p.named_children(&mut p.walk()).find(|t| t.kind() == "catch_type")).map(|t| text[t.byte_range()].split('|').map(|s| bare_type(s).to_owned()).collect::<Vec<_>>()).unwrap_or_default()
                        })
                        .collect();
                    if let Some(body) = n.child_by_field_name("body") {
                        for t in self.thrown_in(lang, body) {
                            let catches_all = caught.iter().any(|c| c == "Exception" || c == "Throwable");
                            let ancestors = self.ancestors(&t.qualified);
                            if !catches_all && !caught.contains(&t.name) && !ancestors.iter().any(|a| caught.contains(a)) {
                                out.push(t);
                            }
                        }
                    }
                    let mut c = n.walk();
                    let rest: Vec<Node> = n.named_children(&mut c).filter(|k| Some(k.id()) != n.child_by_field_name("body").map(|b| b.id())).collect();
                    stack.extend(rest.into_iter().rev());
                    continue;
                }
                "throw_statement" if lang == Lang::Java => {
                    if let Some(created) = n.named_child(0).filter(|e| e.kind() == "object_creation_expression") {
                        if let Some(ty) = created.child_by_field_name("type") {
                            let name = bare_type(&text[ty.byte_range()]).to_owned();
                            out.push(Thrown { qualified: self.qualify(&name, text), name });
                        }
                    }
                }
                "method_invocation" => out.extend(self.java_call(n)),
                "object_creation_expression" => {
                    if let Some(ty) = n.child_by_field_name("type") {
                        let class = bare_type(&text[ty.byte_range()]).to_owned();
                        let arity = argument_count(n.child_by_field_name("arguments"));
                        out.extend(self.declared(&class, text, None, arity));
                    }
                }
                "call_expression" if lang == Lang::Kotlin => out.extend(self.kotlin_call(n)),
                _ => {}
            }
            // In source order.
            let mut c = n.walk();
            let children: Vec<Node> = n.named_children(&mut c).collect();
            stack.extend(children.into_iter().rev());
        }
        out
    }

    /// A Java call's callee's `throws`.
    fn java_call(&self, call: Node) -> Vec<Thrown> {
        let text = self.own;
        let Some(name) = call.child_by_field_name("name").map(|n| &text[n.byte_range()]) else { return Vec::new() };
        let arity = argument_count(call.child_by_field_name("arguments"));
        let enclosing = enclosing_class(call, text);
        let class = match call.child_by_field_name("object") {
            None => enclosing,
            Some(o) if o.kind() == "this" => enclosing,
            Some(o) if o.kind() == "super" => {
                let Some(own) = enclosing else { return Vec::new() };
                return self.superclass_of(&own, text).map(|s| self.declared(&s, text, Some(name), arity)).unwrap_or_default();
            }
            Some(o) => self.receiver_type(Lang::Java, o, call),
        };
        class.map(|c| self.declared(&c, text, Some(name), arity)).unwrap_or_default()
    }

    /// A Kotlin call's callee's `throws` or `@Throws`.
    fn kotlin_call(&self, call: Node) -> Vec<Thrown> {
        let text = self.own;
        let Some(callee) = call.named_child(0) else { return Vec::new() };
        let suffix = call.named_child(1);
        let arguments = suffix.and_then(|s| {
            let mut c = s.walk();
            s.named_children(&mut c).find(|a| a.kind() == "value_arguments")
        });
        let arity = argument_count(arguments);
        match callee.kind() {
            "simple_identifier" => {
                let name = &text[callee.byte_range()];
                if name.starts_with(|c: char| c.is_uppercase()) {
                    return self.declared(name, text, None, arity);
                }
                // A function of this file (or its class).
                let Some(tree) = symbols::parse(Lang::Kotlin, text) else { return Vec::new() };
                kotlin_functions(tree.root_node(), text, name, arity).into_iter().flat_map(|f| kotlin_throws(f, text)).map(|n| Thrown { qualified: self.qualify(&n, text), name: n }).collect()
            }
            "navigation_expression" => {
                let Some(receiver) = callee.named_child(0) else { return Vec::new() };
                let Some(name) = callee.named_child(1).and_then(|s| s.named_child(0)).map(|n| &text[n.byte_range()]) else { return Vec::new() };
                self.receiver_type(Lang::Kotlin, receiver, call).map(|c| self.declared(&c, text, Some(name), arity)).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// The class a receiver expression is, by name or declared type.
    fn receiver_type(&self, lang: Lang, receiver: Node, call: Node) -> Option<String> {
        let text = self.own;
        let name = &text[receiver.byte_range()];
        if matches!(receiver.kind(), "identifier" | "simple_identifier") {
            if super::locals::resolve(lang, text, receiver.start_byte()).is_some() {
                return super::surround::expression_type(lang, receiver, text).map(|t| bare_type(&t).to_owned());
            }
            if name.starts_with(|c: char| c.is_uppercase()) {
                return Some(name.to_owned());
            }
            // A field of the enclosing class.
            let class = enclosing_class_node(call)?;
            return field_type(lang, class, text, name);
        }
        if matches!(receiver.kind(), "field_access" | "scoped_identifier") && name.starts_with(|c: char| c.is_lowercase()) && name.contains('.') {
            // A qualified class name (`java.lang.Thread`).
            let last = name.rsplit('.').next()?;
            return last.starts_with(|c: char| c.is_uppercase()).then(|| name.to_owned());
        }
        let ty = if lang == Lang::Kotlin { super::introduce::kotlin_type(receiver, text) } else { super::introduce::java_type(receiver, text) };
        ty.map(|t| bare_type(&t).to_owned())
    }

    fn superclass_of(&self, class: &str, context: &str) -> Option<String> {
        let text = self.class_text(class, context)?;
        let tree = symbols::parse(Lang::Java, &text)?;
        let node = find_class(tree.root_node(), &text, class.rsplit('.').next()?)?;
        let superclass = node.child_by_field_name("superclass")?.named_child(0)?;
        Some(self.qualify(bare_type(&text[superclass.byte_range()]), &text))
    }

    /// What a method (`name`; a constructor without one) of `class` with
    /// `arity` arguments declares it throws, looking in its superclasses
    /// too.
    fn declared(&self, class: &str, context: &str, name: Option<&str>, arity: usize) -> Vec<Thrown> {
        let mut class = class.to_owned();
        let mut context = context.to_owned();
        for _ in 0..8 {
            let Some(text) = self.class_text(&class, &context) else { return Vec::new() };
            let simple = class.rsplit('.').next().unwrap_or(&class).to_owned();
            let lang = lang_of(&text);
            let kotlin = lang == Lang::Kotlin;
            let Some(tree) = symbols::parse(lang, &text) else { return Vec::new() };
            let Some(node) = find_class(tree.root_node(), &text, &simple) else { return Vec::new() };
            let names: Option<Vec<String>> = if kotlin {
                let found = name.map(|n| kotlin_functions(node, &text, n, arity)).unwrap_or_default();
                (!found.is_empty()).then(|| found.into_iter().flat_map(|f| kotlin_throws(f, &text)).collect())
            } else {
                java_throws(node, &text, name, arity)
            };
            if let Some(names) = names {
                return names.into_iter().map(|n| Thrown { qualified: self.qualify(&n, &text), name: n.rsplit('.').next().unwrap_or(&n).to_owned() }).collect();
            }
            // Constructors aren't inherited.
            let Some(superclass) = name.and(node.child_by_field_name("superclass")).and_then(|s| s.named_child(0)) else { return Vec::new() };
            class = self.qualify(bare_type(&text[superclass.byte_range()]), &text);
            context = text;
        }
        Vec::new()
    }
}

/// Java or Kotlin, by the file's text (the callers have no path).
fn lang_of(text: &str) -> Lang {
    let kotlin = text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("fun ") || l.contains(" fun ") || l.starts_with("val ") || l.starts_with("object ") || l.starts_with("import ") && !l.ends_with(';') || l.starts_with("package ") && !l.ends_with(';')
    });
    if kotlin { Lang::Kotlin } else { Lang::Java }
}

fn enclosing_class_node(node: Node) -> Option<Node> {
    let mut n = node.parent();
    while let Some(p) = n {
        if is_class(p.kind()) {
            return Some(p);
        }
        n = p.parent();
    }
    None
}

fn enclosing_class(node: Node, text: &str) -> Option<String> {
    enclosing_class_node(node).and_then(|c| class_name(c, text)).map(str::to_owned)
}

/// A field's declared type in a class.
fn field_type(lang: Lang, class: Node, text: &str, name: &str) -> Option<String> {
    let body = class.child_by_field_name("body").or_else(|| {
        let mut c = class.walk();
        class.named_children(&mut c).find(|b| b.kind() == "class_body")
    })?;
    let mut c = body.walk();
    for member in body.named_children(&mut c) {
        match (lang, member.kind()) {
            (Lang::Java, "field_declaration") => {
                let mut d = member.walk();
                if member.children_by_field_name("declarator", &mut d).any(|v| v.child_by_field_name("name").is_some_and(|n| &text[n.byte_range()] == name)) {
                    return member.child_by_field_name("type").map(|t| bare_type(&text[t.byte_range()]).to_owned());
                }
            }
            (Lang::Kotlin, "property_declaration") => {
                let variable = member.named_children(&mut member.walk()).find(|v| v.kind() == "variable_declaration")?;
                if variable.named_child(0).is_some_and(|n| &text[n.byte_range()] == name) {
                    return text[variable.byte_range()].split_once(':').map(|(_, t)| bare_type(t).to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// The `throws` of the Java methods (or constructors, `name` None) of a
/// class with `arity` parameters: None when it declares none such.
fn java_throws(class: Node, text: &str, name: Option<&str>, arity: usize) -> Option<Vec<String>> {
    let body = class.child_by_field_name("body")?;
    let mut c = body.walk();
    let kind = if name.is_some() { "method_declaration" } else { "constructor_declaration" };
    let mut matched = false;
    let mut out = Vec::new();
    for member in body.named_children(&mut c).filter(|m| m.kind() == kind) {
        if name.is_some_and(|n| member.child_by_field_name("name").is_none_or(|m| &text[m.byte_range()] != n)) {
            continue;
        }
        let Some(parameters) = member.child_by_field_name("parameters") else { continue };
        let mut p = parameters.walk();
        let params: Vec<Node> = parameters.named_children(&mut p).filter(|n| matches!(n.kind(), "formal_parameter" | "spread_parameter")).collect();
        let varargs = params.last().is_some_and(|l| l.kind() == "spread_parameter");
        if !(params.len() == arity || varargs && arity + 1 >= params.len()) {
            continue;
        }
        matched = true;
        let mut m = member.walk();
        if let Some(throws) = member.named_children(&mut m).find(|t| t.kind() == "throws") {
            let mut t = throws.walk();
            for ty in throws.named_children(&mut t) {
                let n = bare_type(&text[ty.byte_range()]).to_owned();
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        break;
    }
    // A class without the constructor written has the default one.
    (matched || name.is_none()).then_some(out)
}

/// The Kotlin functions named `name` with `arity` parameters under `scope`.
fn kotlin_functions<'t>(scope: Node<'t>, text: &str, name: &str, arity: usize) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![scope];
    while let Some(n) = stack.pop() {
        if n.kind() == "function_declaration" {
            let mut c = n.walk();
            let children: Vec<Node> = n.named_children(&mut c).collect();
            let named = children.iter().any(|c| c.kind() == "simple_identifier" && &text[c.byte_range()] == name);
            let count = children.iter().find(|c| c.kind() == "function_value_parameters").map_or(0, |p| {
                let mut c = p.walk();
                p.named_children(&mut c).filter(|x| x.kind() == "parameter").count()
            });
            if named && count == arity {
                out.push(n);
            }
            continue;
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out
}

/// The classes of a Kotlin function's `@Throws(X::class, …)`.
fn kotlin_throws(function: Node, text: &str) -> Vec<String> {
    let mut c = function.walk();
    let Some(modifiers) = function.named_children(&mut c).find(|m| m.kind() == "modifiers") else { return Vec::new() };
    let mut out = Vec::new();
    for annotation in text[modifiers.byte_range()].split('@').skip(1) {
        let annotation = annotation.trim();
        if !(annotation.starts_with("Throws(") || annotation.starts_with("kotlin.jvm.Throws(") || annotation.starts_with("kotlin.Throws(")) {
            continue;
        }
        let args = annotation.split_once('(').map_or("", |(_, a)| a);
        for part in args.split(&[',', ')'][..]) {
            if let Some(class) = part.trim().strip_suffix("::class") {
                out.push(class.trim().to_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caught(lang: Lang, text: &str, from: &str, to: &str, files: &[&str]) -> Vec<String> {
        let start = text.find(from).unwrap();
        let end = text.find(to).unwrap() + to.len();
        let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        let classes = |name: &str| -> Vec<String> {
            files.iter().filter(|t| t.contains(&format!("class {name}")) || t.contains(&format!("interface {name}"))).cloned().collect()
        };
        caught_exceptions(lang, text, start..end, &classes).into_iter().map(|t| t.qualified).collect()
    }

    const THREAD: &str = "package java.lang;\npublic class Thread {\n    public static void sleep(long millis) throws InterruptedException {}\n}\n";
    const INTERRUPTED: &str = "package java.lang;\npublic class InterruptedException extends Exception {}\n";
    const IO: &str = "package java.io;\npublic class IOException extends Exception {}\n";
    const NOT_FOUND: &str = "package java.io;\npublic class FileNotFoundException extends IOException {}\n";
    const READER: &str = "package java.io;\npublic class FileReader {\n    public FileReader(String name) throws FileNotFoundException {}\n    public int read() throws IOException { return 0; }\n}\n";
    const ILLEGAL: &str = "package java.lang;\npublic class IllegalArgumentException extends RuntimeException {}\n";

    #[test]
    fn catches_what_java_calls_throw() {
        let files = [THREAD, INTERRUPTED, IO, NOT_FOUND, READER, ILLEGAL];
        let text = "package app;\n\nimport java.io.FileReader;\n\nclass A {\n    private FileReader field;\n\n    void load() throws IOException {}\n\n    void f(String name) {\n        Thread.sleep(10);\n        FileReader r = new FileReader(name);\n        r.read();\n        load();\n        field.read();\n        check(1);\n    }\n\n    void check(int n) throws IllegalArgumentException {}\n}\n";
        assert_eq!(caught(Lang::Java, text, "Thread.sleep", "r.read();", &files), ["java.lang.InterruptedException", "java.io.FileNotFoundException", "java.io.IOException"]);
        assert_eq!(caught(Lang::Java, text, "load();", "load();", &files), ["java.io.IOException"]);
        // Only unchecked ones: those (getThrownExceptions).
        assert_eq!(caught(Lang::Java, text, "check(1)", "check(1);", &files), ["java.lang.IllegalArgumentException"]);
        // Caught inside, or in a lambda: nothing.
        let text = "class A {\n    void f() {\n        try {\n            Thread.sleep(1);\n        } catch (InterruptedException e) {\n        }\n        Runnable r = () -> Thread.sleep(1);\n        throw new IOException();\n    }\n}\n";
        assert_eq!(caught(Lang::Java, text, "try {", "throw new IOException();", &files), ["java.io.IOException"]);
    }

    #[test]
    fn knows_jdk_exceptions_without_their_sources() {
        let text = "package app;\n\nclass A {\n    void read() throws java.io.IOException, FileNotFoundException {}\n    void f() {\n        read();\n    }\n}\n";
        assert_eq!(caught(Lang::Java, text, "read();", "read();", &[]), ["java.io.FileNotFoundException", "java.io.IOException"]);
    }

    #[test]
    fn catches_what_kotlin_calls_declare() {
        let files = [THREAD, INTERRUPTED, IO];
        let text = "package app\n\nimport java.io.IOException\n\n@Throws(IOException::class)\nfun load() {}\n\nfun f() {\n    Thread.sleep(10)\n    load()\n    println(1)\n}\n";
        assert_eq!(caught(Lang::Kotlin, text, "Thread.sleep", "println(1)", &files), ["java.lang.InterruptedException", "java.io.IOException"]);
        assert!(caught(Lang::Kotlin, text, "println(1)", "println(1)", &files).is_empty());
    }
}

//! Names whose declaration isn't a symbol of their own, as IntelliJ resolves
//! them: `this` / `self` / `Self` (the enclosing type), `super` (its
//! supertype), Kotlin's `it` and Swift's `$0` (the lambda), Kotlin's
//! `field` (the property), and named arguments (the callee's parameter).

use tree_sitter::Node;

use super::lang::Lang;
use super::nav::{self, Target};
use super::store::ProjectIndex;
use super::symbols::{self, Symbol, SymbolKind};

/// What the word at an offset is, when it is one of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special {
    /// `this`, `this@Label`, `self`, `Self`.
    This { label: Option<String> },
    /// `super`, `super<Type>`, `super@Label`.
    Super { explicit: Option<String> },
    /// A lambda's implicit parameter: byte offset of the lambda's `{`.
    Lambda(usize),
    /// A Kotlin accessor's `field`: byte offset of the property's name.
    Field(usize),
    /// `name = …` / `name: …` in a call: the byte offset of the callee's
    /// name, and the argument's name.
    NamedArg { callee: usize, name: String },
}

fn this_word(lang: Lang, word: &str) -> bool {
    match lang {
        Lang::Kotlin | Lang::Java | Lang::Dart | Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Cpp => word == "this",
        Lang::Swift => word == "self" || word == "Self",
        Lang::ObjC => word == "self",
        Lang::Rust => word == "Self",
        _ => false,
    }
}

fn super_word(lang: Lang, word: &str) -> bool {
    word == "super"
        && matches!(lang, Lang::Kotlin | Lang::Java | Lang::Dart | Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Swift | Lang::ObjC | Lang::Python)
}

/// `@Label` or `<Type>` right after `end`.
fn suffix(text: &str, end: usize, open: char, close: Option<char>) -> Option<String> {
    let rest = text[end..].strip_prefix(open)?;
    let len = rest.find(|c: char| !nav::is_word_char(c)).unwrap_or(rest.len());
    if len == 0 || close.is_some_and(|c| !rest[len..].starts_with(c)) {
        return None;
    }
    Some(rest[..len].to_owned())
}

pub fn classify(lang: Lang, text: &str, offset: usize) -> Option<Special> {
    let (word, range) = nav::word_at(text, offset)?;
    let qualified = {
        let before = text[..range.start].trim_end();
        before.ends_with('.') || before.ends_with("::") || before.ends_with("->") && matches!(lang, Lang::C | Lang::Cpp | Lang::ObjC)
    };
    if !qualified && this_word(lang, &word) {
        return Some(Special::This { label: (lang == Lang::Kotlin).then(|| suffix(text, range.end, '@', None)).flatten() });
    }
    if !qualified && super_word(lang, &word) {
        let explicit = if lang == Lang::Kotlin { suffix(text, range.end, '<', Some('>')) } else { None };
        return Some(Special::Super { explicit });
    }
    let implicit = match lang {
        Lang::Kotlin => matches!(word.as_str(), "it" | "field"),
        Lang::Swift => word.starts_with('$') && word[1..].chars().all(|c| c.is_ascii_digit()) && word.len() > 1,
        _ => false,
    };
    let named = match lang {
        // `f(x = 1)`, not `f(x == 1)`.
        Lang::Kotlin | Lang::Python => {
            let after = text[range.end..].trim_start();
            after.starts_with('=') && !after.starts_with("==")
        }
        Lang::Swift | Lang::Dart => {
            let after = text[range.end..].trim_start();
            after.starts_with(':') && !after.starts_with("::")
        }
        _ => false,
    };
    if !implicit && !named || qualified {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    let at = tree.root_node().descendant_for_byte_range(range.start, range.end)?;
    if implicit {
        if word == "field" {
            let accessor = ancestors(at).find(|n| matches!(n.kind(), "getter" | "setter"))?;
            // The grammar puts accessors after the property, not inside it.
            let property = ancestors(accessor).find(|n| n.kind() == "property_declaration").or_else(|| {
                std::iter::successors(accessor.prev_named_sibling(), |n| n.prev_named_sibling())
                    .find(|n| !matches!(n.kind(), "getter" | "setter"))
                    .filter(|n| n.kind() == "property_declaration")
            })?;
            let name = first_named_ident(property)?;
            return Some(Special::Field(name.start_byte()));
        }
        // Kotlin's `it` belongs to the nearest lambda without parameters.
        let lambda = ancestors(at).find(|n| n.kind() == "lambda_literal" && (lang == Lang::Swift || !has_child(*n, "lambda_parameters")))?;
        return Some(Special::Lambda(lambda.start_byte()));
    }
    // The argument node and the call around it.
    let argument = ancestors(at).take(3).find(|n| matches!(n.kind(), "value_argument" | "keyword_argument" | "named_argument"))?;
    let call = ancestors(argument).find(|n| matches!(n.kind(), "call_expression" | "call" | "constructor_invocation" | "new_expression" | "delegation_specifier"))?;
    let callee = callee_name(call, text)?;
    Some(Special::NamedArg { callee, name: word })
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(node.parent(), |n| n.parent())
}

fn has_child(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).any(|c| c.kind() == kind);
    found
}

/// The first identifier in a node, depth first.
fn first_named_ident(node: Node) -> Option<Node> {
    if matches!(node.kind(), "simple_identifier" | "identifier") {
        return Some(node);
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter().filter(|c| !matches!(c.kind(), "modifiers" | "binding_pattern_kind" | "annotation")).find_map(first_named_ident)
}

/// Byte offset of the name a call calls: `f`, `a.b.f`, `Foo` in `Foo(…)`.
fn callee_name(call: Node, text: &str) -> Option<usize> {
    let function = call.child_by_field_name("function").or_else(|| call.named_child(0))?;
    // The last identifier before the arguments.
    let end = function.end_byte();
    let before = text[..end].trim_end_matches(|c: char| c == '>' || c.is_whitespace());
    let before = if before.len() < end && text[before.len()..end].contains('>') {
        // `Foo<T>(…)`: skip the type arguments.
        let open = text[..end].rfind('<')?;
        text[..open].trim_end()
    } else {
        before
    };
    let start = before.rfind(|c: char| !nav::is_word_char(c)).map_or(0, |i| i + 1);
    (start < before.len()).then_some(start)
}

/// The innermost type declared around `offset`; for a member outside any
/// type (a Rust `impl` method, a Kotlin extension function), the type
/// it belongs to, by name.
pub enum Enclosing {
    Symbol(Symbol),
    Named(String),
}

pub fn enclosing_type(lang: Lang, text: &str, offset: usize, label: Option<&str>) -> Option<Enclosing> {
    let (line, col) = nav::position(text, offset);
    let symbols = symbols::extract(lang, text);
    let around = |s: &&Symbol| (s.line < line || s.line == line && s.col <= col) && line <= s.end_line;
    let innermost = |a: &&Symbol, b: &&Symbol| a.line.cmp(&b.line).then(b.end_line.cmp(&a.end_line));
    let ty = symbols
        .iter()
        .filter(|s| (s.kind.is_type() || s.kind == SymbolKind::Extension) && around(s))
        .filter(|s| label.is_none_or(|l| s.name == l))
        .max_by(innermost);
    let function = symbols.iter().filter(|s| s.kind.is_callable() && around(s)).max_by(innermost);
    if label.is_none() {
        if let Some(function) = function {
            // A function nested deeper than the type: an extension's receiver.
            if ty.is_none_or(|t| function.line > t.line) {
                let decl_line = text.lines().nth(function.line as usize).unwrap_or("");
                if let Some(receiver) = extension_receiver(lang, decl_line, &function.name) {
                    return Some(Enclosing::Named(receiver));
                }
                if ty.is_none() {
                    if let Some(container) = &function.container {
                        return Some(Enclosing::Named(container.rsplit(['.', ':']).next()?.to_owned()));
                    }
                }
            }
        }
    }
    // `extension Foo` stands for Foo.
    match ty? {
        s if s.kind == SymbolKind::Extension => Some(Enclosing::Named(s.name.clone())),
        s => Some(Enclosing::Symbol(s.clone())),
    }
}

/// `fun Foo.bar(`, `fun <T> List<T>.bar(` → `Foo`, `List`.
fn extension_receiver(lang: Lang, line: &str, name: &str) -> Option<String> {
    if lang != Lang::Kotlin {
        return None;
    }
    let pattern = format!(r"\bfun\s+(?:<[^>]*>\s*)?([\w.]+?)(?:<[^>]*>)?\??\.{}\s*[(<]", regex::escape(name));
    let captures = regex::Regex::new(&pattern).ok()?.captures(line)?;
    Some(captures[1].rsplit('.').next()?.to_owned())
}

/// Type declarations named `name`: the project's first, then libraries.
pub fn type_targets(index: &ProjectIndex, name: &str, path: &str) -> Vec<Target> {
    let mut out: Vec<(i32, Target)> = index
        .symbols_named(name)
        .filter(|(_, _, s)| s.kind.is_type())
        .map(|(p, e, s)| {
            let score = if p == path { 2 } else if ProjectIndex::is_external(p) { 0 } else { 1 };
            (score, nav::symbol_target(p, e.lang, s))
        })
        .collect();
    let best = out.iter().map(|(s, _)| *s).max().unwrap_or(0);
    out.retain(|(s, _)| *s == best);
    out.into_iter().map(|(_, t)| t).collect()
}

/// The supertypes of a type, nearest first: the type's own supers, then
/// theirs. Classes come before interfaces at each level.
pub fn super_chain(index: &ProjectIndex, direct: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut level: Vec<String> = direct.to_vec();
    while !level.is_empty() && out.len() < 32 {
        let mut next = Vec::new();
        let class_first = |n: &String| !index.symbols_named(n).any(|(_, _, s)| matches!(s.kind, SymbolKind::Class | SymbolKind::Struct));
        level.sort_by_key(class_first);
        for name in level {
            if out.contains(&name) {
                continue;
            }
            for (_, _, s) in index.symbols_named(&name).filter(|(_, _, s)| s.kind.is_type()) {
                next.extend(s.supers.iter().cloned());
            }
            out.push(name);
        }
        level = next;
    }
    out
}

/// The supertypes `super` names in a type: its superclass when it has
/// one, else every supertype.
fn super_of(index: &ProjectIndex, supers: &[String]) -> Vec<String> {
    let class = supers.iter().find(|n| index.symbols_named(n).any(|(_, _, s)| matches!(s.kind, SymbolKind::Class | SymbolKind::Struct)));
    match class {
        Some(c) => vec![c.clone()],
        None => supers.to_vec(),
    }
}

/// The type names `this.x` / `super.x` / an unqualified member call may
/// mean, nearest first, for ranking members.
pub fn receivers(index: &ProjectIndex, lang: Lang, text: &str, qualifier: &str, at: usize) -> Vec<String> {
    let this = this_word(lang, qualifier) || qualifier.is_empty() || qualifier == "self" && matches!(lang, Lang::Python | Lang::Rust);
    let sup = super_word(lang, qualifier);
    if !this && !sup {
        return Vec::new();
    }
    let (own, supers) = match enclosing_type(lang, text, at, None) {
        Some(Enclosing::Symbol(s)) => (s.name.clone(), s.supers.clone()),
        Some(Enclosing::Named(n)) => {
            let supers = index.symbols_named(&n).filter(|(_, _, s)| s.kind.is_type()).flat_map(|(_, _, s)| s.supers.clone()).collect();
            (n, supers)
        }
        None => return Vec::new(),
    };
    if sup {
        return super_chain(index, &supers);
    }
    let mut out = vec![own];
    out.extend(super_chain(index, &supers));
    out
}

/// Where a special name leads.
pub fn targets(index: &ProjectIndex, root: &std::path::Path, path: &str, lang: Lang, text: &str, offset: usize, special: Special) -> Vec<Target> {
    let here = |at: usize, name: &str, label: &str| {
        let (line, col) = nav::position(text, at);
        vec![Target { path: path.to_owned(), line, col, name: name.to_owned(), label: format!("{label} · {}", lang.name()), container: None }]
    };
    match special {
        Special::This { label } => match enclosing_type(lang, text, offset, label.as_deref()) {
            Some(Enclosing::Symbol(s)) => vec![nav::symbol_target(path, lang, &s)],
            Some(Enclosing::Named(n)) => type_targets(index, &n, path),
            None => Vec::new(),
        },
        Special::Super { explicit } => {
            let supers = match explicit {
                Some(t) => vec![t],
                None => match enclosing_type(lang, text, offset, None) {
                    Some(Enclosing::Symbol(s)) => super_of(index, &s.supers),
                    Some(Enclosing::Named(n)) => {
                        let direct: Vec<String> = index.symbols_named(&n).filter(|(_, _, s)| s.kind.is_type()).flat_map(|(_, _, s)| s.supers.clone()).collect();
                        super_of(index, &direct)
                    }
                    None => Vec::new(),
                },
            };
            supers.iter().flat_map(|s| type_targets(index, s, path)).collect()
        }
        Special::Lambda(at) => here(at, "lambda", "lambda"),
        Special::Field(at) => {
            let name = nav::word_at(text, at).map(|(w, _)| w).unwrap_or_default();
            here(at, &name, "property")
        }
        Special::NamedArg { callee, name } => {
            let callees = nav::definitions(index, path, lang, text, callee);
            let mut out = Vec::new();
            for target in &callees {
                let source = if target.path == path {
                    Some(text.to_owned())
                } else {
                    let file = if ProjectIndex::is_external(&target.path) { std::path::PathBuf::from(&target.path) } else { root.join(&target.path) };
                    std::fs::read_to_string(file).ok()
                };
                let Some(source) = source else { continue };
                let target_lang = index.file(&target.path).map(|e| e.lang).unwrap_or(lang);
                if let Some(at) = parameter(target_lang, &source, target.line, target.col, &name) {
                    let (line, col) = nav::position(&source, at);
                    out.push(Target { path: target.path.clone(), line, col, name: name.clone(), label: format!("parameter · {}", target_lang.name()), container: Some(target.name.clone()) });
                }
            }
            if out.is_empty() { callees } else { out }
        }
    }
}

/// Byte offset of the parameter `name` of the declaration named at
/// (`line`, `col`): a function's, or a class's constructors'.
pub fn parameter(lang: Lang, text: &str, line: u32, col: u32, name: &str) -> Option<usize> {
    let tree = symbols::parse(lang, text)?;
    let offset = byte_offset(text, line, col)?;
    let at = tree.root_node().descendant_for_byte_range(offset, offset)?;
    // The declaration: the nearest ancestor holding parameters.
    for decl in ancestors(at).take(6) {
        if let Some(found) = find_param(decl, text, name, false) {
            return Some(found);
        }
        if is_type_decl(decl.kind()) {
            // A class: its constructors' parameters.
            return find_param(decl, text, name, true);
        }
    }
    None
}

fn is_type_decl(kind: &str) -> bool {
    matches!(kind, "class_declaration" | "class_definition" | "struct_declaration" | "object_declaration" | "enum_declaration" | "protocol_declaration")
}

fn is_body(kind: &str) -> bool {
    matches!(
        kind,
        "function_body" | "class_body" | "block" | "statements" | "compound_statement" | "lambda_literal" | "enum_class_body" | "protocol_body"
    )
}

fn is_constructor(node: Node, text: &str) -> bool {
    let kind = node.kind();
    kind.contains("constructor") || kind == "init_declaration" || kind == "primary_constructor"
        || kind == "function_definition" && node.child_by_field_name("name").and_then(|n| n.utf8_text(text.as_bytes()).ok()) == Some("__init__")
}

/// A parameter named `name` under `node`, not inside bodies; in
/// constructors only when `constructors`.
fn find_param(node: Node, text: &str, name: &str, constructors: bool) -> Option<usize> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    for child in children {
        let kind = child.kind();
        if matches!(kind, "identifier" | "simple_identifier") && child.utf8_text(text.as_bytes()).ok() == Some(name) {
            let param = ancestors(child).take(4).find(|n| super::locals::is_parameter(n.kind()));
            if param.is_some() && (!constructors || ancestors(child).any(|n| is_constructor(n, text))) {
                return Some(child.start_byte());
            }
            continue;
        }
        // Nested declarations have parameters of their own.
        if is_body(kind) && !(constructors && matches!(kind, "class_body" | "block")) || kind == "lambda_literal" {
            continue;
        }
        if constructors && !is_constructor(child, text) && (kind.contains("function") || kind.contains("method")) && kind != "function_value_parameters" {
            if !(kind == "function_definition" && is_constructor(child, text)) {
                continue;
            }
        }
        if let Some(found) = find_param(child, text, name, constructors) {
            return Some(found);
        }
    }
    None
}

fn byte_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    let start = if line == 0 { 0 } else { text.match_indices('\n').nth(line as usize - 1)?.0 + 1 };
    let rest = &text[start..];
    let mut units = 0;
    for (i, c) in rest.char_indices() {
        if units >= col as usize || c == '\n' {
            return Some(start + i);
        }
        units += c.len_utf16();
    }
    Some(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::store::{FileEntry, ProjectIndex};

    fn nth(text: &str, word: &str, n: usize) -> usize {
        text.match_indices(word)
            .filter(|(i, _)| {
                let before = text[..*i].chars().next_back();
                let after = text[i + word.len()..].chars().next();
                !before.is_some_and(nav::is_word_char) && !after.is_some_and(nav::is_word_char)
            })
            .nth(n)
            .unwrap()
            .0
            + 1
    }

    fn index_of(files: &[(&str, Lang, &str)]) -> ProjectIndex {
        let mut index = ProjectIndex::default();
        for (path, lang, text) in files {
            let entry = FileEntry { lang: *lang, mtime: 0, size: 0, symbols: symbols::extract(*lang, text), bridges: Vec::new() };
            index.insert_for_test(path, entry);
        }
        index
    }

    /// (line, name) of each target for the `n`th `word`.
    fn go(files: &[(&str, Lang, &str)], file: usize, word: &str, n: usize) -> Vec<(String, u32)> {
        let index = index_of(files);
        let (path, lang, text) = files[file];
        let offset = nth(text, word, n);
        let Some(special) = classify(lang, text, offset) else { return Vec::new() };
        targets(&index, std::path::Path::new("/nonexistent"), path, lang, text, offset, special).into_iter().map(|t| (t.path, t.line)).collect()
    }

    const KT: &str = "open class Base {\n    open fun area() = 1.0\n}\nclass Square(val side: Double, val tag: String = \"\") : Base(), Shape {\n    override fun area(): Double {\n        val s = this.side\n        return super.area() + list.map { it * 2 }.sum()\n    }\n    inner class In { fun f() = this@Square.side }\n    var p: Int = 0\n        get() = field\n}\nfun Square.ext() = this.side\nfun use() = Square(side = 2.0, tag = \"x\")\nfun g(k: Int) = 1\nfun h() = g(k = 1)\ninterface Shape\n";

    #[test]
    fn kotlin_specials() {
        let one = "open class Base { open fun area() = 1.0 }\nclass Ring : Base() {\n    override fun area() = super.area() + this.hashCode()\n}\n";
        assert_eq!(go(&[("r.kt", Lang::Kotlin, one)], 0, "super", 0), vec![("r.kt".into(), 0)]);
        assert_eq!(go(&[("r.kt", Lang::Kotlin, one)], 0, "this", 0), vec![("r.kt".into(), 1)]);
        let files = [("a.kt", Lang::Kotlin, KT)];
        assert_eq!(go(&files, 0, "this", 0), vec![("a.kt".into(), 3)]);
        assert_eq!(go(&files, 0, "super", 0), vec![("a.kt".into(), 0)]);
        assert_eq!(go(&files, 0, "it", 0), vec![("a.kt".into(), 6)]);
        assert_eq!(go(&files, 0, "this", 1), vec![("a.kt".into(), 3)]);
        assert_eq!(go(&files, 0, "field", 0), vec![("a.kt".into(), 9)]);
        // An extension function's `this` is its receiver.
        assert_eq!(go(&files, 0, "this", 2), vec![("a.kt".into(), 3)]);
        // Named arguments: the constructor's and the function's parameters.
        assert_eq!(go(&files, 0, "side", 4), vec![("a.kt".into(), 3)]);
        assert_eq!(go(&files, 0, "tag", 1), vec![("a.kt".into(), 3)]);
        assert_eq!(go(&files, 0, "k", 1), vec![("a.kt".into(), 14)]);
    }

    #[test]
    fn members_through_this_and_super() {
        let kt = "open class Base {\n    open fun area() = 1.0\n    fun name() = \"b\"\n}\nclass Square : Base() {\n    override fun area() = 2.0\n    fun f() = this.area() + super.area() + name() + this.name()\n}\nfun name() = 3\n";
        let index = index_of(&[("a.kt", Lang::Kotlin, kt)]);
        let lines = |word: &str, n: usize| nav::definitions(&index, "a.kt", Lang::Kotlin, kt, nth(kt, word, n)).into_iter().map(|t| t.line).collect::<Vec<_>>();
        assert_eq!(lines("area", 2), vec![5]);
        assert_eq!(lines("area", 3), vec![1]);
        // Inherited members: the class's own before a top-level function.
        assert_eq!(lines("name", 1), vec![2]);
        assert_eq!(lines("name", 2), vec![2]);
    }

    #[test]
    fn other_languages() {
        let py = "class A:\n    def __init__(self, x, y=1):\n        pass\n\nclass B(A):\n    def __init__(self):\n        super().__init__(x=1)\n\ndef f(a, b=2):\n    return a\n\nf(1, b=3)\nA(y=2)\n";
        let files = [("a.py", Lang::Python, py)];
        assert_eq!(go(&files, 0, "super", 0), vec![("a.py".into(), 0)]);
        assert_eq!(go(&files, 0, "b", 1), vec![("a.py".into(), 8)]);
        assert_eq!(go(&files, 0, "y", 1), vec![("a.py".into(), 1)]);

        let swift = "class A {\n    func move(by amount: Int) {}\n}\nclass B: A {\n    func f() {\n        self.move(by: 1)\n        super.move(by: 2)\n        let xs = [1].map { $0 + 1 }\n    }\n}\n";
        let files = [("a.swift", Lang::Swift, swift)];
        assert_eq!(go(&files, 0, "self", 0), vec![("a.swift".into(), 3)]);
        assert_eq!(go(&files, 0, "super", 0), vec![("a.swift".into(), 0)]);
        assert_eq!(go(&files, 0, "$0", 0), vec![("a.swift".into(), 7)]);
        assert_eq!(go(&files, 0, "by", 1), vec![("a.swift".into(), 1)]);

        let dart = "class A {\n  void f({int x = 0}) {}\n}\nclass B extends A {\n  void g() {\n    this.f(x: 1);\n    super.f(x: 2);\n  }\n}\n";
        let files = [("a.dart", Lang::Dart, dart)];
        assert_eq!(go(&files, 0, "this", 0), vec![("a.dart".into(), 3)]);
        assert_eq!(go(&files, 0, "super", 0), vec![("a.dart".into(), 0)]);
        assert_eq!(go(&files, 0, "x", 1), vec![("a.dart".into(), 1)]);

        let java = "class A { void f() {} }\nclass B extends A implements I {\n  void g() { this.g(); super.f(); }\n}\ninterface I {}\n";
        let files = [("A.java", Lang::Java, java)];
        assert_eq!(go(&files, 0, "this", 0), vec![("A.java".into(), 1)]);
        assert_eq!(go(&files, 0, "super", 0), vec![("A.java".into(), 0)]);

        let rust = "struct P;\nimpl P {\n    fn new() -> Self { Self }\n}\n";
        let files = [("a.rs", Lang::Rust, rust)];
        assert_eq!(go(&files, 0, "Self", 0), vec![("a.rs".into(), 0)]);

        let ts = "class A { f() {} }\nclass B extends A {\n  g() { this.f(); super.f(); }\n}\n";
        let files = [("a.ts", Lang::TypeScript, ts)];
        assert_eq!(go(&files, 0, "this", 0), vec![("a.ts".into(), 1)]);
        assert_eq!(go(&files, 0, "super", 0), vec![("a.ts".into(), 0)]);

        let cpp = "class A {\n  int x;\n  int f() { return this->x; }\n};\n";
        let files = [("a.cpp", Lang::Cpp, cpp)];
        assert_eq!(go(&files, 0, "this", 0), vec![("a.cpp".into(), 0)]);
    }
}

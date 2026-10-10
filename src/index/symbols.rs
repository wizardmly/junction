//! Symbol extraction: one tree walk per file with per-language rules naming
//! the declaration nodes, how to read their names, and which ones contain
//! others (classes, namespaces, impl blocks).

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser, Tree};

use super::lang::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Namespace,
    Class,
    Struct,
    Interface,
    Protocol,
    Trait,
    Enum,
    EnumMember,
    Extension,
    Function,
    Method,
    Constructor,
    Field,
    Property,
    Constant,
    Variable,
    TypeAlias,
    Macro,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Interface => "interface",
            SymbolKind::Protocol => "protocol",
            SymbolKind::Trait => "trait",
            SymbolKind::Enum => "enum",
            SymbolKind::EnumMember => "enum member",
            SymbolKind::Extension => "extension",
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Field => "field",
            SymbolKind::Property => "property",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::TypeAlias => "type alias",
            SymbolKind::Macro => "macro",
        }
    }

    /// Types and namespaces, which Go to Class lists.
    pub fn is_type(self) -> bool {
        matches!(
            self,
            SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Interface
                | SymbolKind::Protocol
                | SymbolKind::Trait
                | SymbolKind::Enum
                | SymbolKind::TypeAlias
        )
    }

    fn is_callable(self) -> bool {
        matches!(self, SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 0-based line and UTF-16 column of the name.
    pub line: u32,
    pub col: u32,
    /// Last line of the whole declaration.
    pub end_line: u32,
    /// The enclosing class / namespace / impl, innermost last joined by ".".
    pub container: Option<String>,
    /// A declaration without a body (prototype, forward declaration,
    /// abstract or external member); definitions rank first.
    pub decl: bool,
    /// Extra text: an Objective-C selector, a Rust impl's trait.
    pub detail: Option<String>,
}

/// How a rule reads the declared name.
#[derive(Clone, Copy)]
enum Name {
    /// The node's `name` field (or another field).
    Field(&'static str),
    /// The first named child of one of these kinds.
    Child(&'static [&'static str]),
    /// The innermost name of a C-style declarator chain.
    Declarator,
    /// An Objective-C method: the selector's first piece.
    Selector,
    /// A Swift `class_declaration`, whose `name` may be a `user_type`.
    SwiftType,
    /// The identifiers bound by a pattern (Swift property).
    Pattern,
}

#[derive(Clone, Copy)]
struct Rule {
    node: &'static str,
    kind: SymbolKind,
    name: Name,
    /// Its children get it as their container.
    container: bool,
    /// Only outside function bodies (variables).
    top_only: bool,
}

const fn rule(node: &'static str, kind: SymbolKind, name: Name) -> Rule {
    Rule { node, kind, name, container: false, top_only: false }
}
const fn holder(node: &'static str, kind: SymbolKind, name: Name) -> Rule {
    Rule { node, kind, name, container: true, top_only: false }
}
const fn top(node: &'static str, kind: SymbolKind, name: Name) -> Rule {
    Rule { node, kind, name, container: false, top_only: true }
}

use Name::*;
use SymbolKind as K;

const C_RULES: &[Rule] = &[
    rule("function_definition", K::Function, Declarator),
    top("declaration", K::Function, Declarator),
    rule("struct_specifier", K::Struct, Field("name")),
    rule("union_specifier", K::Struct, Field("name")),
    rule("enum_specifier", K::Enum, Field("name")),
    rule("enumerator", K::EnumMember, Field("name")),
    rule("type_definition", K::TypeAlias, Declarator),
    rule("preproc_def", K::Macro, Field("name")),
    rule("preproc_function_def", K::Macro, Field("name")),
];

const CPP_RULES: &[Rule] = &[
    holder("namespace_definition", K::Namespace, Field("name")),
    holder("class_specifier", K::Class, Field("name")),
    holder("struct_specifier", K::Struct, Field("name")),
    rule("union_specifier", K::Struct, Field("name")),
    rule("enum_specifier", K::Enum, Field("name")),
    rule("enumerator", K::EnumMember, Field("name")),
    rule("function_definition", K::Function, Declarator),
    top("declaration", K::Function, Declarator),
    rule("field_declaration", K::Method, Declarator),
    rule("type_definition", K::TypeAlias, Declarator),
    rule("alias_declaration", K::TypeAlias, Field("name")),
    rule("concept_definition", K::TypeAlias, Field("name")),
    rule("preproc_def", K::Macro, Field("name")),
    rule("preproc_function_def", K::Macro, Field("name")),
];

const OBJC_RULES: &[Rule] = &[
    holder("class_interface", K::Class, Child(&["identifier"])),
    holder("class_implementation", K::Class, Child(&["identifier"])),
    holder("category_interface", K::Extension, Child(&["identifier"])),
    holder("category_implementation", K::Extension, Child(&["identifier"])),
    holder("protocol_declaration", K::Protocol, Child(&["identifier"])),
    rule("method_declaration", K::Method, Selector),
    rule("method_definition", K::Method, Selector),
    rule("property_declaration", K::Property, Declarator),
    rule("function_definition", K::Function, Declarator),
    top("declaration", K::Function, Declarator),
    rule("struct_specifier", K::Struct, Field("name")),
    rule("enum_specifier", K::Enum, Field("name")),
    rule("enumerator", K::EnumMember, Field("name")),
    rule("type_definition", K::TypeAlias, Declarator),
    rule("preproc_def", K::Macro, Field("name")),
    rule("preproc_function_def", K::Macro, Field("name")),
];

const RUST_RULES: &[Rule] = &[
    holder("mod_item", K::Module, Field("name")),
    rule("function_item", K::Function, Field("name")),
    rule("function_signature_item", K::Function, Field("name")),
    holder("struct_item", K::Struct, Field("name")),
    holder("union_item", K::Struct, Field("name")),
    holder("enum_item", K::Enum, Field("name")),
    rule("enum_variant", K::EnumMember, Field("name")),
    holder("trait_item", K::Trait, Field("name")),
    holder("impl_item", K::Class, Field("type")),
    rule("type_item", K::TypeAlias, Field("name")),
    rule("associated_type", K::TypeAlias, Field("name")),
    rule("const_item", K::Constant, Field("name")),
    rule("static_item", K::Variable, Field("name")),
    rule("macro_definition", K::Macro, Field("name")),
    rule("field_declaration", K::Field, Field("name")),
];

const GO_RULES: &[Rule] = &[
    rule("function_declaration", K::Function, Field("name")),
    rule("method_declaration", K::Method, Field("name")),
    rule("type_spec", K::Struct, Field("name")),
    rule("type_alias", K::TypeAlias, Field("name")),
    top("const_spec", K::Constant, Field("name")),
    top("var_spec", K::Variable, Field("name")),
    rule("method_elem", K::Method, Field("name")),
    rule("method_spec", K::Method, Field("name")),
    rule("field_declaration", K::Field, Field("name")),
];

const JAVA_RULES: &[Rule] = &[
    holder("class_declaration", K::Class, Field("name")),
    holder("interface_declaration", K::Interface, Field("name")),
    holder("enum_declaration", K::Enum, Field("name")),
    holder("record_declaration", K::Class, Field("name")),
    holder("annotation_type_declaration", K::Interface, Field("name")),
    rule("enum_constant", K::EnumMember, Field("name")),
    rule("method_declaration", K::Method, Field("name")),
    rule("constructor_declaration", K::Constructor, Field("name")),
    rule("field_declaration", K::Field, Declarator),
    rule("constant_declaration", K::Constant, Declarator),
];

const KOTLIN_RULES: &[Rule] = &[
    holder("class_declaration", K::Class, Child(&["type_identifier"])),
    holder("object_declaration", K::Class, Child(&["type_identifier"])),
    holder("companion_object", K::Class, Child(&["type_identifier"])),
    rule("function_declaration", K::Function, Child(&["simple_identifier"])),
    top("property_declaration", K::Property, Declarator),
    rule("type_alias", K::TypeAlias, Child(&["type_identifier"])),
    rule("enum_entry", K::EnumMember, Child(&["simple_identifier"])),
];

const SWIFT_RULES: &[Rule] = &[
    holder("class_declaration", K::Class, SwiftType),
    holder("protocol_declaration", K::Protocol, Field("name")),
    rule("function_declaration", K::Function, Field("name")),
    rule("protocol_function_declaration", K::Method, Field("name")),
    rule("init_declaration", K::Constructor, Child(&["init"])),
    top("property_declaration", K::Property, Pattern),
    rule("protocol_property_declaration", K::Property, Pattern),
    rule("typealias_declaration", K::TypeAlias, Field("name")),
    rule("associatedtype_declaration", K::TypeAlias, Field("name")),
    rule("enum_entry", K::EnumMember, Field("name")),
];

const DART_RULES: &[Rule] = &[
    holder("class_declaration", K::Class, Field("name")),
    holder("class_definition", K::Class, Field("name")),
    holder("mixin_declaration", K::Class, Field("name")),
    holder("extension_declaration", K::Extension, Field("name")),
    holder("enum_declaration", K::Enum, Field("name")),
    rule("enum_constant", K::EnumMember, Field("name")),
    rule("function_signature", K::Function, Field("name")),
    rule("getter_signature", K::Property, Field("name")),
    rule("setter_signature", K::Property, Field("name")),
    rule("constructor_signature", K::Constructor, Field("name")),
    rule("factory_constructor_signature", K::Constructor, Child(&["identifier"])),
    rule("type_alias", K::TypeAlias, Child(&["type_identifier"])),
    top("initialized_identifier", K::Field, Field("name")),
    top("static_final_declaration", K::Variable, Field("name")),
];

const V_RULES: &[Rule] = &[
    rule("function_declaration", K::Function, Field("name")),
    holder("struct_declaration", K::Struct, Field("name")),
    holder("enum_declaration", K::Enum, Field("name")),
    holder("interface_declaration", K::Interface, Field("name")),
    rule("enum_field_definition", K::EnumMember, Field("name")),
    rule("struct_field_declaration", K::Field, Field("name")),
    rule("interface_method_definition", K::Method, Field("name")),
    rule("const_definition", K::Constant, Field("name")),
    rule("type_declaration", K::TypeAlias, Field("name")),
    rule("global_var_definition", K::Variable, Field("name")),
];

const JS_RULES: &[Rule] = &[
    rule("function_declaration", K::Function, Field("name")),
    rule("generator_function_declaration", K::Function, Field("name")),
    holder("class_declaration", K::Class, Field("name")),
    holder("abstract_class_declaration", K::Class, Field("name")),
    rule("method_definition", K::Method, Field("name")),
    rule("abstract_method_signature", K::Method, Field("name")),
    rule("method_signature", K::Method, Field("name")),
    top("variable_declarator", K::Variable, Field("name")),
    holder("interface_declaration", K::Interface, Field("name")),
    rule("type_alias_declaration", K::TypeAlias, Field("name")),
    holder("enum_declaration", K::Enum, Field("name")),
    holder("internal_module", K::Namespace, Field("name")),
    rule("public_field_definition", K::Field, Field("name")),
    rule("field_definition", K::Field, Field("property")),
];

const PYTHON_RULES: &[Rule] = &[
    holder("class_definition", K::Class, Field("name")),
    rule("function_definition", K::Function, Field("name")),
];

fn rules(lang: Lang) -> &'static [Rule] {
    match lang {
        Lang::C => C_RULES,
        Lang::Cpp => CPP_RULES,
        Lang::ObjC => OBJC_RULES,
        Lang::Rust => RUST_RULES,
        Lang::Go => GO_RULES,
        Lang::Java => JAVA_RULES,
        Lang::Kotlin => KOTLIN_RULES,
        Lang::Swift => SWIFT_RULES,
        Lang::Dart => DART_RULES,
        Lang::V => V_RULES,
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => JS_RULES,
        Lang::Python => PYTHON_RULES,
    }
}

/// Function bodies, inside which only nested declarations count.
fn is_body(kind: &str) -> bool {
    matches!(kind, "compound_statement" | "block" | "function_body" | "statement_block" | "statements" | "lambda_literal")
}

pub fn parse(lang: Lang, text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    parser.parse(text, None)
}

pub fn extract(lang: Lang, text: &str) -> Vec<Symbol> {
    let Some(tree) = parse(lang, text) else { return Vec::new() };
    extract_from(lang, text, &tree)
}

pub fn extract_from(lang: Lang, text: &str, tree: &Tree) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut walker = Walker { lang, text, rules: rules(lang), containers: Vec::new(), out: &mut out };
    walker.walk(tree.root_node(), false);
    out
}

struct Walker<'a> {
    lang: Lang,
    text: &'a str,
    rules: &'static [Rule],
    containers: Vec<String>,
    out: &'a mut Vec<Symbol>,
}

impl Walker<'_> {
    fn walk(&mut self, node: Node, in_body: bool) {
        let mut pushed = false;
        if let Some(rule) = self.rules.iter().find(|r| r.node == node.kind()).copied() {
            if !(rule.top_only && in_body) {
                let names = self.names(node, rule.name);
                let kind = self.refine(node, rule.kind);
                for (name_node, name) in &names {
                    self.push(node, *name_node, name.clone(), kind, rule);
                }
                if rule.container {
                    if let Some((_, name)) = names.first() {
                        self.containers.push(name.clone());
                        pushed = true;
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, in_body || is_body(child.kind()));
        }
        if pushed {
            self.containers.pop();
        }
    }

    /// The declaration carries this modifier (`abstract`, `annotation`…).
    fn declared_with(&self, node: Node, modifier: &str) -> bool {
        let mut cursor = node.walk();
        let found = node.children(&mut cursor).any(|c| c.kind() == "modifiers" && self.text_of(c).split_whitespace().any(|m| m == modifier));
        found
    }

    fn text_of(&self, node: Node) -> &str {
        node.utf8_text(self.text.as_bytes()).unwrap_or_default()
    }

    /// Kinds the node kind alone can't tell: Swift's class_declaration covers
    /// struct / enum / extension; a function inside a class is a method.
    fn refine(&self, node: Node, kind: SymbolKind) -> SymbolKind {
        if self.lang == Lang::Swift && node.kind() == "class_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "struct" => return K::Struct,
                    "enum" => return K::Enum,
                    "extension" => return K::Extension,
                    "class" | "actor" => return K::Class,
                    _ => {}
                }
            }
        }
        // Kotlin's class_declaration covers interfaces and enum classes.
        if self.lang == Lang::Kotlin && node.kind() == "class_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "interface" => return K::Interface,
                    "enum" | "enum_class_body" => return K::Enum,
                    _ => {}
                }
            }
        }
        if self.lang == Lang::Go && node.kind() == "type_spec" {
            let ty = node.child_by_field_name("type").map(|t| t.kind());
            return match ty {
                Some("interface_type") => K::Interface,
                Some("struct_type") => K::Struct,
                _ => K::TypeAlias,
            };
        }
        if kind == K::Function && !self.containers.is_empty() && !matches!(self.lang, Lang::C | Lang::Python) {
            return K::Method;
        }
        if self.lang == Lang::Python && kind == K::Function && !self.containers.is_empty() {
            return K::Method;
        }
        kind
    }

    fn push(&mut self, node: Node, name_node: Node, name: String, kind: SymbolKind, rule: Rule) {
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '~' || c == '$') {
            return;
        }
        let pos = name_node.start_position();
        let line_start = self.text[..name_node.start_byte()].rfind('\n').map_or(0, |i| i + 1);
        let col = self.text[line_start..name_node.start_byte()].encode_utf16().count() as u32;
        let mut container = (!self.containers.is_empty()).then(|| self.containers.join("."));
        let mut name = name;
        // `A::b` defines b in A.
        if let Some((qualifier, last)) = name.rsplit_once("::") {
            let qualifier = qualifier.replace("::", ".");
            container = Some(match container {
                Some(c) => format!("{c}.{qualifier}"),
                None => qualifier,
            });
            name = last.to_owned();
        }
        let decl = self.is_declaration(node, kind, rule);
        let detail = match rule.name {
            Selector => Some(selector(node, self.text)),
            _ if node.kind() == "impl_item" => node.child_by_field_name("trait").map(|t| format!("impl {}", self.text_of(t))),
            // What the Project view's class icons tell apart.
            _ if self.lang == Lang::Kotlin && node.kind() == "object_declaration" => Some("object".into()),
            _ if node.kind() == "annotation_type_declaration" => Some("annotation".into()),
            _ if self.lang == Lang::Kotlin && kind == K::Class && self.declared_with(node, "annotation") => Some("annotation".into()),
            _ if self.declared_with(node, "abstract") && kind == K::Class => Some("abstract".into()),
            _ => None,
        };
        // An impl block is a container, not a symbol of its own; a bodiless
        // `struct S` is a use of the type.
        if node.kind() == "impl_item" || (decl && matches!(node.kind(), "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier")) {
            return;
        }
        self.out.push(Symbol { name, kind, line: pos.row as u32, col, end_line: node.end_position().row as u32, container, decl, detail });
    }

    fn is_declaration(&self, node: Node, kind: SymbolKind, rule: Rule) -> bool {
        match node.kind() {
            "declaration" | "field_declaration" if kind.is_callable() => true,
            "function_signature_item" | "method_declaration" if matches!(self.lang, Lang::Rust | Lang::ObjC) => true,
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => node.child_by_field_name("body").is_none(),
            "method_declaration" if self.lang == Lang::Java => node.child_by_field_name("body").is_none(),
            "function_declaration" if self.lang == Lang::Kotlin || self.lang == Lang::Swift => {
                let mut cursor = node.walk();
                !node.named_children(&mut cursor).any(|c| c.kind() == "function_body")
            }
            "function_signature" => {
                // Dart: a bare signature (abstract or external) has no body sibling.
                let parent = node.parent().map(|p| if p.kind() == "method_signature" { p.parent().unwrap_or(p) } else { p });
                parent.is_some_and(|p| p.kind() != "function_declaration" && p.kind() != "method_declaration" || {
                    let mut cursor = p.walk();
                    !p.named_children(&mut cursor).any(|c| c.kind() == "function_body")
                })
            }
            "protocol_function_declaration" | "method_signature" | "abstract_method_signature" | "method_elem" | "method_spec" | "interface_method_definition" => true,
            _ => {
                let _ = rule;
                false
            }
        }
    }

    fn names<'t>(&self, node: Node<'t>, name: Name) -> Vec<(Node<'t>, String)> {
        let one = |n: Option<Node<'t>>| n.map(|n| vec![(n, self.text_of(n).to_owned())]).unwrap_or_default();
        match name {
            Field(field) => {
                // Go's const/var specs bind several names.
                let mut cursor = node.walk();
                let all: Vec<Node> = node.children_by_field_name(field, &mut cursor).collect();
                // A Dart named constructor `Foo.named` is called by its last name.
                if all.len() > 1 && node.kind() == "constructor_signature" {
                    return one(all.last().copied());
                }
                if all.len() > 1 && self.lang == Lang::Go {
                    return all.into_iter().map(|n| (n, self.text_of(n).to_owned())).collect();
                }
                let n = all.into_iter().next();
                // A Rust impl's type may be generic: `Foo<T>` names Foo.
                match n {
                    Some(n) if n.kind() == "generic_type" => one(n.child_by_field_name("type")),
                    Some(n) if n.kind() == "scoped_type_identifier" => one(n.child_by_field_name("name")),
                    other => one(other),
                }
            }
            Child(kinds) => {
                let mut cursor = node.walk();
                let found = node.children(&mut cursor).find(|c| kinds.contains(&c.kind()));
                one(found)
            }
            Declarator => self.declarator_names(node),
            Selector => {
                let mut cursor = node.walk();
                one(node.named_children(&mut cursor).find(|c| c.kind() == "identifier"))
            }
            SwiftType => match node.child_by_field_name("name") {
                Some(n) if n.kind() == "user_type" => {
                    let mut cursor = n.walk();
                    let last = n.named_children(&mut cursor).filter(|c| c.kind() == "type_identifier").last();
                    one(last)
                }
                other => one(other),
            },
            Pattern => {
                let mut cursor = node.walk();
                let mut out = Vec::new();
                for child in node.children_by_field_name("name", &mut cursor) {
                    if let Some(id) = child.child_by_field_name("bound_identifier") {
                        out.push((id, self.text_of(id).to_owned()));
                    } else if child.kind() == "simple_identifier" {
                        out.push((child, self.text_of(child).to_owned()));
                    }
                }
                out
            }
        }
    }

    /// Names in a C-style declarator: functions only for `declaration` and
    /// `field_declaration` (prototypes and methods), type names for typedefs.
    fn declarator_names<'t>(&self, node: Node<'t>) -> Vec<(Node<'t>, String)> {
        let kind = node.kind();
        let mut out = Vec::new();
        let mut cursor = node.walk();
        let declarators: Vec<Node> = match kind {
            // Kotlin / Java / ObjC hold names in child declarators.
            "property_declaration" if self.lang == Lang::Kotlin => {
                let mut c = node.walk();
                let mut found = Vec::new();
                for child in node.named_children(&mut c) {
                    if child.kind() == "variable_declaration" {
                        let mut c2 = child.walk();
                        if let Some(id) = child.named_children(&mut c2).find(|n| n.kind() == "simple_identifier") {
                            found.push((id, self.text_of(id).to_owned()));
                        }
                    }
                }
                return found;
            }
            "property_declaration" => {
                // ObjC: @property (...) int count; → struct_declaration > struct_declarator > identifier
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    if child.kind() == "struct_declaration" {
                        let mut c2 = child.walk();
                        for d in child.named_children(&mut c2).filter(|n| n.kind() == "struct_declarator") {
                            if let Some(id) = innermost(d) {
                                out.push((id, self.text_of(id).to_owned()));
                            }
                        }
                    }
                }
                return out;
            }
            _ => node.children_by_field_name("declarator", &mut cursor).collect(),
        };
        for d in declarators {
            let is_function = contains_function_declarator(d);
            let wanted = match kind {
                "declaration" | "field_declaration" if self.lang != Lang::Java => is_function,
                _ => true,
            };
            if !wanted {
                continue;
            }
            // Java: variable_declarator name.
            let id = if d.kind() == "variable_declarator" { d.child_by_field_name("name") } else { innermost(d) };
            if let Some(id) = id {
                out.push((id, self.text_of(id).to_owned()));
            }
        }
        out
    }
}

fn contains_function_declarator(node: Node) -> bool {
    let mut n = Some(node);
    while let Some(cur) = n {
        if cur.kind() == "function_declarator" {
            return true;
        }
        n = cur.child_by_field_name("declarator");
    }
    false
}

/// The name at the bottom of a declarator chain (`*(*f)(int)` → `f`).
fn innermost(node: Node) -> Option<Node> {
    let mut cur = node;
    loop {
        match cur.kind() {
            "identifier" | "field_identifier" | "type_identifier" | "qualified_identifier" | "destructor_name" | "operator_name"
            | "primitive_type" => return Some(cur),
            _ => {}
        }
        if let Some(next) = cur.child_by_field_name("declarator") {
            cur = next;
            continue;
        }
        let mut cursor = cur.walk();
        let next = cur
            .named_children(&mut cursor)
            .find(|c| matches!(c.kind(), "identifier" | "field_identifier" | "type_identifier" | "qualified_identifier" | "parenthesized_declarator" | "pointer_declarator" | "function_declarator" | "array_declarator" | "reference_declarator"));
        cur = next?;
    }
}

/// An Objective-C method's full selector, `initWithName:age:`.
pub fn selector(node: Node, text: &str) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    let mut has_params = false;
    let mut pending: Option<String> = None;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Some(p) = pending.take() {
                    out.push_str(&p);
                }
                pending = Some(child.utf8_text(text.as_bytes()).unwrap_or_default().to_owned());
            }
            "method_parameter" => {
                has_params = true;
                if let Some(p) = pending.take() {
                    out.push_str(&p);
                }
                out.push(':');
            }
            _ => {}
        }
    }
    if let Some(p) = pending {
        if !has_params {
            out.push_str(&p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(lang: Lang, src: &str) -> Vec<String> {
        extract(lang, src)
            .into_iter()
            .map(|s| {
                let mut out = format!("{}:{}", s.kind.label(), s.name);
                if let Some(c) = s.container {
                    out = format!("{out}@{c}");
                }
                if s.decl {
                    out.push_str(" decl");
                }
                out
            })
            .collect()
    }

    #[test]
    fn c_and_cpp() {
        let c = names(Lang::C, "#define MAX 3\nint add(int a, int b);\nstatic int add(int a, int b) { int local = 1; return a + b; }\nstruct S { int x; };\ntypedef struct S S_t;\nenum E { A, B };\nint counter = 0;\n");
        assert_eq!(c, ["macro:MAX", "function:add decl", "function:add", "struct:S", "type alias:S_t", "enum:E", "enum member:A", "enum member:B"]);
        let cpp = names(Lang::Cpp, "namespace ns { class Foo { public: void bar(); int x; }; }\nvoid ns::Foo::bar() {}\ntemplate<typename T> T id(T t) { return t; }\nusing Alias = int;\n");
        assert_eq!(cpp, ["namespace:ns", "class:Foo@ns", "method:bar@ns.Foo decl", "function:bar@ns.Foo", "function:id", "type alias:Alias"]);
    }

    #[test]
    fn objc() {
        let src = "@interface Foo : NSObject\n- (instancetype)initWithName:(NSString *)name age:(int)age;\n+ (void)shared;\n@property (nonatomic) int count;\n@end\n@implementation Foo\n- (void)run:(int)x { }\n@end\n@protocol P\n- (void)go;\n@end\nint cfunc(int a) { return a; }\n";
        let syms = extract(Lang::ObjC, src);
        let init = syms.iter().find(|s| s.name == "initWithName").unwrap();
        assert_eq!(init.detail.as_deref(), Some("initWithName:age:"));
        assert_eq!(init.container.as_deref(), Some("Foo"));
        assert_eq!(
            names(Lang::ObjC, src),
            ["class:Foo", "method:initWithName@Foo decl", "method:shared@Foo decl", "property:count@Foo", "class:Foo", "method:run@Foo", "protocol:P", "method:go@P decl", "function:cfunc"]
        );
    }

    #[test]
    fn rust() {
        let src = "mod m { pub fn inner() {} }\npub struct Point { x: i32 }\nimpl Point { pub fn len(&self) -> i32 { let y = 1; 0 } }\nimpl<T> Display for Wrapper<T> { fn fmt(&self) {} }\ntrait Shape { fn area(&self) -> f64; }\nenum E { A }\nconst N: u8 = 1;\nmacro_rules! m { () => {} }\nextern \"C\" { fn c_add(a: i32) -> i32; }\n";
        assert_eq!(
            names(Lang::Rust, src),
            ["module:m", "method:inner@m", "struct:Point", "field:x@Point", "method:len@Point", "method:fmt@Wrapper", "trait:Shape", "method:area@Shape decl", "enum:E", "enum member:A@E", "constant:N", "macro:m", "function:c_add decl"]
        );
    }

    #[test]
    fn go_java_kotlin() {
        assert_eq!(
            names(Lang::Go, "package p\ntype Point struct { X int }\ntype Shape interface { Area() float64 }\nfunc (p Point) Len() int { x := 1; return x }\nfunc Add(a, b int) int { return a + b }\nconst A, B = 1, 2\n"),
            ["struct:Point", "field:X", "interface:Shape", "method:Area decl", "method:Len", "function:Add", "constant:A", "constant:B"]
        );
        assert_eq!(
            names(Lang::Java, "package a; public class Foo { private int x, y; public native int nat(int a); Foo() {} void bar() { int local = 0; } interface I { void m(); } enum E { A } }"),
            ["class:Foo", "field:x@Foo", "field:y@Foo", "method:nat@Foo decl", "constructor:Foo@Foo", "method:bar@Foo", "interface:I@Foo", "method:m@Foo.I decl", "enum:E@Foo", "enum member:A@Foo.E"]
        );
        assert_eq!(
            names(Lang::Kotlin, "package a.b\nclass Foo {\n  external fun nat(x: Int): Int\n  fun bar() { val local = 1 }\n  val p = 1\n}\nobject O\nfun top(): Int = 1\ntypealias T = Int\n"),
            ["class:Foo", "method:nat@Foo decl", "method:bar@Foo", "property:p@Foo", "class:O", "function:top", "type alias:T"]
        );
    }

    #[test]
    fn swift_dart_v() {
        assert_eq!(
            names(Lang::Swift, "class Foo: NSObject { func bar(x: Int) {} \n var p = 1 }\nstruct S { let a: Int }\nprotocol P { func go() }\nenum E { case a }\nextension Foo { func baz() {} }\nfunc add(_ a: Int) -> Int { a }\n"),
            ["class:Foo", "method:bar@Foo", "property:p@Foo", "struct:S", "property:a@S", "protocol:P", "method:go@P decl", "enum:E", "enum member:a@E", "extension:Foo", "method:baz@Foo", "function:add"]
        );
        assert_eq!(
            names(Lang::Dart, "class Foo extends Bar { int x = 0; void run() { var y = 1; } Foo.named(); }\nint top(int a) => a;\nexternal int twice(int x);\nenum E { a }\nmixin M {}\n"),
            ["class:Foo", "field:x@Foo", "method:run@Foo", "constructor:named@Foo", "function:top", "function:twice decl", "enum:E", "enum member:a@E", "class:M"]
        );
        assert_eq!(
            names(Lang::V, "module main\npub fn add(a int, b int) int { return a }\nstruct Point { x int }\nfn (p Point) len() int { return 0 }\nenum Color { red }\ninterface Shape { area() f64 }\nconst pi = 3.14\ntype MyInt = int\n"),
            ["function:add", "struct:Point", "field:x@Point", "function:len", "enum:Color", "enum member:red@Color", "interface:Shape", "method:area@Shape decl", "constant:pi", "type alias:MyInt"]
        );
    }

    #[test]
    fn js_ts_python() {
        assert_eq!(
            names(Lang::TypeScript, "export class Foo { bar(): void {} }\nfunction add(a: number) { const local = 1; return a }\nconst handler = () => 1;\ninterface I { m(): void }\ntype T = string;\nenum E { A }\n"),
            ["class:Foo", "method:bar@Foo", "function:add", "variable:handler", "interface:I", "method:m@I decl", "type alias:T", "enum:E"]
        );
        assert_eq!(names(Lang::Python, "class A:\n    def m(self):\n        pass\ndef f():\n    pass\n"), ["class:A", "method:m@A", "function:f"]);
    }
}

#[cfg(test)]
mod kotlin_kinds {
    use super::*;

    #[test]
    fn kinds_and_tags() {
        let src = "package a\npublic sealed interface Map<K> { }\npublic enum class Mode { A, B }\nannotation class Exp\nabstract class Base\ninternal object Utils { }\nclass Plain\n";
        let got: Vec<String> = extract(Lang::Kotlin, src).into_iter().filter(|s| s.kind.is_type()).map(|s| format!("{}:{}:{}", s.kind.label(), s.name, s.detail.unwrap_or_default())).collect();
        assert_eq!(got, ["interface:Map:", "enum:Mode:", "class:Exp:annotation", "class:Base:abstract", "class:Utils:object", "class:Plain:"]);
    }
}

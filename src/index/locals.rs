//! Local names: parameters, local variables, lambda and loop variables,
//! Kotlin / Swift constructor properties. The symbol index only knows
//! declarations a file exports; these resolve from the file's own syntax
//! tree, as IntelliJ does: Ctrl+click on a use goes to its declaration, on
//! the declaration it lists the uses in its scope.

use tree_sitter::Node;

use super::lang::Lang;
use super::symbols;

/// What the identifier at an offset is, locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    /// Byte offset of the declaring identifier.
    pub decl: usize,
    /// The identifier at the offset is the declaration itself.
    pub on_decl: bool,
    /// Byte offsets of every use in the declaration's scope (not the
    /// declaration).
    pub uses: Vec<usize>,
    /// A class member (Kotlin `class A(val x: Int)`): used outside the
    /// class too, so Find Usages searches the project.
    pub member: bool,
}

/// Identifier node kinds that can name a local.
fn is_ident(kind: &str) -> bool {
    // Kotlin's `"$x"`: a use in a string template.
    matches!(kind, "identifier" | "simple_identifier" | "shorthand_property_identifier_pattern" | "interpolated_identifier")
}

/// Nodes wrapping a declared name on its way up to the declaration
/// (`*p`, `a[]`, `&r`, `(x, y)`, Swift's `let x` pattern).
fn is_wrapper(kind: &str) -> bool {
    matches!(
        kind,
        "pointer_declarator" | "array_declarator" | "reference_declarator" | "init_declarator" | "tuple_pattern" | "pattern"
            | "value_binding_pattern" | "mut_pattern" | "reference_pattern" | "expression_list" | "pattern_list" | "tuple_pattern_list"
            | "list_splat_pattern" | "typed_parameter" | "default_parameter" | "typed_default_parameter" | "object_pattern"
            | "array_pattern" | "assignment_pattern" | "variable_declarator" | "multi_variable_declaration" | "inferred_parameters"
            | "lambda_parameters" | "binding_pattern"
    )
}

/// Nodes whose (first) identifier declares a local.
fn is_binding(kind: &str) -> bool {
    matches!(
        kind,
        "parameter" | "class_parameter" | "formal_parameter" | "variable_declaration" | "variable_declarator" | "init_declarator"
            | "parameter_declaration" | "optional_parameter_declaration" | "variadic_parameter_declaration" | "catch_formal_parameter"
            | "enhanced_for_statement" | "let_declaration" | "closure_parameters" | "short_var_declaration" | "range_clause"
            | "var_spec" | "required_parameter" | "optional_parameter" | "lambda_parameter" | "formal_parameters" | "parameters"
            | "initialized_variable_definition" | "initialized_identifier" | "catch_block" | "catch_clause" | "catch_parameter"
            | "for_statement" | "for_in_statement" | "for_expression" | "if_let_expression" | "let_condition" | "property_declaration"
            | "lambda_function_type_parameters" | "method_parameter" | "self_parameter" | "local_variable_declaration"
            | "lexical_declaration" | "variable_declaration_list" | "assignment" | "arrow_function" | "resource" | "match_pattern"
            | "inferred_parameters" | "lambda_parameters" | "typed_parameter" | "default_parameter" | "for_in_clause"
            | "var_declaration" | "simple_parameter" | "constructor_param"
    )
}

/// Where a local's name is visible: the nearest of these around its declaration.
fn is_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration" | "function_definition" | "method_declaration" | "constructor_declaration" | "function_item"
            | "closure_expression" | "lambda_literal" | "lambda_expression" | "arrow_function" | "function_expression"
            | "method_definition" | "func_literal" | "function_signature_item" | "init_declaration" | "secondary_constructor"
            | "anonymous_function" | "lambda" | "block" | "compound_statement" | "statements" | "function_body" | "statement_block"
            | "for_statement" | "enhanced_for_statement" | "for_expression" | "for_in_statement" | "catch_block" | "catch_clause"
            | "match_arm" | "if_expression" | "if_statement" | "while_statement" | "class_declaration" | "class_body"
            | "method_definition_body" | "local_function_statement" | "getter" | "setter" | "subscript_declaration"
            | "list_comprehension" | "generator_expression" | "dictionary_comprehension" | "set_comprehension"
            | "function_literal" | "method_signature"
    )
}

/// A function's parameters are visible in its whole body, not only after
/// them: they bind at the function, whatever their order in the text.
fn binds_whole_scope(binding: &str) -> bool {
    matches!(
        binding,
        "parameter" | "class_parameter" | "formal_parameter" | "parameter_declaration" | "optional_parameter_declaration"
            | "variadic_parameter_declaration" | "required_parameter" | "optional_parameter" | "formal_parameters" | "parameters"
            | "closure_parameters" | "lambda_parameter" | "method_parameter" | "simple_parameter" | "constructor_param"
            | "typed_parameter" | "default_parameter" | "inferred_parameters" | "lambda_parameters"
    )
}

/// A node kind that declares a function's (or lambda's) parameter.
pub fn is_parameter(kind: &str) -> bool {
    binds_whole_scope(kind) && !matches!(kind, "formal_parameters" | "parameters" | "closure_parameters" | "lambda_parameters" | "inferred_parameters")
}

/// The declaration node an identifier names, when it declares a local.
fn binding_of(node: Node) -> Option<Node> {
    let mut cur = node;
    let mut parent = node.parent()?;
    while is_wrapper(parent.kind()) && !is_binding(parent.kind()) {
        cur = parent;
        parent = parent.parent()?;
    }
    if !is_binding(parent.kind()) {
        // `int x = …` reaches its declaration through init_declarator /
        // variable_declarator, which are bindings themselves.
        return None;
    }
    // The value side of `x = y`, `let a = b`: only the name side binds.
    for field in ["value", "right", "default_value", "type", "body", "condition", "initializer"] {
        if parent.child_by_field_name(field).is_some_and(|v| v.id() == cur.id() || v.start_byte() <= cur.start_byte() && cur.end_byte() <= v.end_byte()) {
            return None;
        }
    }
    match parent.kind() {
        // Only the first name is the declared one: `x: T = y`, `for (x in y)`.
        "variable_declaration" | "parameter" | "class_parameter" | "formal_parameter" | "catch_block" | "lambda_parameter"
        | "for_statement" | "for_in_statement" | "for_expression" | "enhanced_for_statement" | "simple_parameter" | "property_declaration"
        | "initialized_variable_definition" | "catch_clause" | "method_parameter" | "for_in_clause" => {
            (first_ident(parent)?.id() == node.id()).then_some(parent)
        }
        // `a = 1` binds in Python only, and only the left side.
        "assignment" => parent.child_by_field_name("left").filter(|l| l.start_byte() <= node.start_byte() && node.end_byte() <= l.end_byte()).map(|_| parent),
        _ => Some(parent),
    }
}

/// Whether `node` is in the value a declaration starts with (`let x =
/// x(1)`: the outer `x`), where the name isn't bound yet.
fn in_own_value(binding: Node, node: Node) -> bool {
    ["value", "initializer", "right", "default_value"]
        .iter()
        .any(|f| binding.child_by_field_name(f).is_some_and(|v| v.start_byte() <= node.start_byte() && node.end_byte() <= v.end_byte()))
}

/// The first identifier under a node, skipping modifiers and annotations.
fn first_ident(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "modifiers" | "annotation" | "parameter_modifiers" | "parameter_modifier" | "type_annotation" | "user_type" | "attribute"
                | "binding_pattern_kind" | "mutable_specifier" | "value_binding_pattern" | "final_builtin" | "const_builtin" | "inferred_type"
        ) {
            continue;
        }
        if is_ident(child.kind()) {
            return Some(child);
        }
        if is_wrapper(child.kind()) || matches!(child.kind(), "variable_declaration" | "simple_identifier") {
            if let Some(found) = first_ident(child) {
                return Some(found);
            }
        }
        // A type or expression comes before the name only in C-like
        // declarations, which name through `declarator` fields.
        if !matches!(child.kind(), "type_identifier" | "scoped_type_identifier" | "primitive_type" | "type") && !child.kind().ends_with("_type") {
            return None;
        }
    }
    None
}

fn scope_of(binding: Node) -> Option<Node> {
    let mut cur = binding.parent()?;
    loop {
        if is_scope(cur.kind()) {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// A member access (`x.name`, `x->name`), not a local.
fn is_member_access(text: &str, start: usize) -> bool {
    let before = text[..start].trim_end();
    (before.ends_with('.') && !before.ends_with("..")) || before.ends_with("->") || before.ends_with("::")
}

fn collect<'t>(node: Node<'t>, word: &str, text: &str, out: &mut Vec<Node<'t>>) {
    if is_ident(node.kind()) {
        if node.utf8_text(text.as_bytes()).is_ok_and(|t| t == word) {
            out.push(node);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, word, text, out);
    }
}

/// The locals visible at `offset`, nearest scope first: (name, whether
/// it is a parameter). For completion.
pub fn visible(lang: Lang, text: &str, offset: usize) -> Vec<(String, bool)> {
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let mut idents: Vec<Node> = Vec::new();
    collect_all(tree.root_node(), &mut idents);
    // (scope size, name, parameter): smaller scopes are nearer.
    let mut found: Vec<(usize, String, bool)> = Vec::new();
    for ident in idents {
        if ident.start_byte() <= offset && offset <= ident.end_byte() {
            continue;
        }
        let Some(binding) = binding_of(ident) else { continue };
        let Some(scope) = scope_of(binding) else { continue };
        if !(scope.start_byte() <= offset && offset <= scope.end_byte()) || matches!(scope.kind(), "class_declaration" | "class_body") {
            continue;
        }
        if ident.start_byte() > offset && !binds_whole_scope(binding.kind()) {
            continue;
        }
        let Ok(name) = ident.utf8_text(text.as_bytes()) else { continue };
        found.push((scope.end_byte() - scope.start_byte(), name.to_owned(), is_parameter(binding.kind())));
    }
    found.sort_by_key(|f| f.0);
    let mut seen = std::collections::HashSet::new();
    found.into_iter().filter(|f| seen.insert(f.1.clone())).map(|(_, name, param)| (name, param)).collect()
}

fn collect_all<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if is_ident(node.kind()) {
        out.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_all(child, out);
    }
}

/// Resolves the identifier at `offset` to a local declaration.
pub fn resolve(lang: Lang, text: &str, offset: usize) -> Option<Local> {
    let (mut word, mut range) = super::nav::word_at(text, offset)?;
    // Kotlin's `"$x"`: x.
    if lang == Lang::Kotlin && word.len() > 1 && word.starts_with('$') {
        word.remove(0);
        range.start += 1;
    }
    if is_member_access(text, range.start) {
        return None;
    }
    let tree = symbols::parse(lang, text)?;
    let root = tree.root_node();
    let at = root.descendant_for_byte_range(range.start, range.end)?;
    if !is_ident(at.kind()) {
        return None;
    }
    let mut same: Vec<Node> = Vec::new();
    collect(root, &word, text, &mut same);
    // Declarations visible here, innermost scope first, then the nearest
    // one before the use.
    let mut best: Option<(Node, Node)> = None;
    for ident in &same {
        let Some(binding) = binding_of(*ident) else { continue };
        let Some(scope) = scope_of(binding) else { continue };
        if !(scope.start_byte() <= at.start_byte() && at.end_byte() <= scope.end_byte()) {
            continue;
        }
        if ident.start_byte() > at.start_byte() && !binds_whole_scope(binding.kind()) || in_own_value(binding, at) {
            continue;
        }
        let better = match best {
            None => true,
            Some((b_ident, b_scope)) => {
                let inner = scope.end_byte() - scope.start_byte() < b_scope.end_byte() - b_scope.start_byte();
                let same_scope = scope.id() == b_scope.id();
                inner || same_scope && ident.start_byte() > b_ident.start_byte() && ident.start_byte() <= at.start_byte()
            }
        };
        if better {
            best = Some((*ident, scope));
        }
    }
    let (decl, scope) = best?;
    let own = binding_of(decl);
    let uses = same
        .iter()
        .filter(|n| n.id() != decl.id() && scope.start_byte() <= n.start_byte() && n.end_byte() <= scope.end_byte() && n.start_byte() >= decl.start_byte().min(scope.start_byte()))
        .filter(|n| !is_member_access(text, n.start_byte()))
        // A nearer declaration of the same name shadows this one.
        .filter(|n| binding_of(**n).is_none())
        .filter(|n| !own.is_some_and(|b| in_own_value(b, **n)))
        .map(|n| n.start_byte())
        .collect();
    let member = matches!(scope.kind(), "class_declaration" | "class_body");
    Some(Local { decl: decl.start_byte(), on_decl: decl.id() == at.id(), uses, member })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_bound_in_its_own_value() {
        let text = "fn main() {\n    let area = area(3);\n    println!(\"{}\", area);\n}\n";
        // The call is the function's, not the variable's.
        assert!(resolve(Lang::Rust, text, text.find("area(").unwrap()).is_none());
        let local = resolve(Lang::Rust, text, text.find("area =").unwrap()).unwrap();
        assert_eq!(local.uses, [text.find("area);").unwrap()]);
    }

    #[test]
    fn string_template_uses() {
        let text = "fun f() {\n    val n = 1\n    log(\"$n and ${n}\")\n}\n";
        let local = resolve(Lang::Kotlin, text, text.find("n =").unwrap()).unwrap();
        let uses: Vec<usize> = local.uses.clone();
        assert_eq!(uses, [text.find("$n").unwrap() + 1, text.find("{n}").unwrap() + 1]);
        // And back from the template.
        assert_eq!(resolve(Lang::Kotlin, text, text.find("$n").unwrap() + 1).unwrap().decl, text.find("n =").unwrap());
    }

    #[test]
    fn visible_locals() {
        let text = "fun f(size: Int) {\n    val label = 1\n    run { x -> \n    }\n    \n    val later = 2\n}\nfun g(other: Int) {}\n";
        let at = text.find("    \n    val later").unwrap() + 4;
        let names: Vec<String> = visible(Lang::Kotlin, text, at).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"size".to_owned()) && names.contains(&"label".to_owned()), "{names:?}");
        assert!(!names.contains(&"later".to_owned()) && !names.contains(&"other".to_owned()) && !names.contains(&"x".to_owned()), "{names:?}");
    }

    /// (declaration line, on it, use lines) for the `n`th occurrence of `word`.
    fn at(lang: Lang, text: &str, word: &str, n: usize) -> Option<(usize, bool, Vec<usize>)> {
        let offset = text.match_indices(word).filter(|(i, _)| {
            let before = text[..*i].chars().next_back();
            let after = text[i + word.len()..].chars().next();
            !before.is_some_and(super::super::nav::is_word_char) && !after.is_some_and(super::super::nav::is_word_char)
        }).nth(n)?.0;
        let line = |o: usize| text[..o].matches('\n').count();
        let local = resolve(lang, text, offset + 1)?;
        Some((line(local.decl), local.on_decl, local.uses.iter().map(|o| line(*o)).collect()))
    }

    #[test]
    fn kotlin_locals() {
        let src = "class Square(val side: Double) : Base() {\n    fun area(k: Int): Double {\n        val twice = side * k\n        list.forEach { item -> println(item + twice) }\n        return twice\n    }\n}\n";
        assert_eq!(at(Lang::Kotlin, src, "side", 1), Some((0, false, vec![2])));
        assert_eq!(at(Lang::Kotlin, src, "side", 0), Some((0, true, vec![2])));
        assert_eq!(at(Lang::Kotlin, src, "k", 1), Some((1, false, vec![2])));
        assert_eq!(at(Lang::Kotlin, src, "twice", 2), Some((2, false, vec![3, 4])));
        assert_eq!(at(Lang::Kotlin, src, "item", 1), Some((3, false, vec![3])));
        assert_eq!(at(Lang::Kotlin, src, "area", 0), None);
        assert_eq!(at(Lang::Kotlin, src, "list", 0), None);
    }

    #[test]
    fn java_c_rust_locals() {
        let java = "class A {\n  int f(int a, String s) {\n    int b = a + 1;\n    for (String x : list) { use(x, b); }\n    try { } catch (Exception e) { log(e); }\n    return b;\n  }\n}\n";
        assert_eq!(at(Lang::Java, java, "a", 1), Some((1, false, vec![2])));
        assert_eq!(at(Lang::Java, java, "b", 2), Some((2, false, vec![3, 5])));
        assert_eq!(at(Lang::Java, java, "x", 1), Some((3, false, vec![3])));
        assert_eq!(at(Lang::Java, java, "e", 1), Some((4, false, vec![4])));
        let c = "int f(int *p, int n) {\n  int sum = 0;\n  for (int i = 0; i < n; i++) sum += p[i];\n  return sum;\n}\n";
        assert_eq!(at(Lang::C, c, "p", 1), Some((0, false, vec![2])));
        assert_eq!(at(Lang::C, c, "sum", 1), Some((1, false, vec![2, 3])));
        assert_eq!(at(Lang::C, c, "i", 2), Some((2, false, vec![2, 2, 2])));
        let rust = "fn f(v: &[i32], mut n: usize) -> i32 {\n    let (a, b) = (1, 2);\n    let total = v.iter().map(|x| x + a).sum::<i32>();\n    n += 1;\n    total + b\n}\n";
        assert_eq!(at(Lang::Rust, rust, "v", 1), Some((0, false, vec![2])));
        assert_eq!(at(Lang::Rust, rust, "n", 1), Some((0, false, vec![3])));
        assert_eq!(at(Lang::Rust, rust, "b", 1), Some((1, false, vec![4])));
        assert_eq!(at(Lang::Rust, rust, "x", 1), Some((2, false, vec![2])));
        assert_eq!(at(Lang::Rust, rust, "total", 1), Some((2, false, vec![4])));
    }

    #[test]
    fn swift_dart_go_ts_python_locals() {
        let swift = "func f(count: Int) -> Int {\n    let twice = count * 2\n    return twice\n}\n";
        assert_eq!(at(Lang::Swift, swift, "count", 1), Some((0, false, vec![1])));
        assert_eq!(at(Lang::Swift, swift, "twice", 1), Some((1, false, vec![2])));
        let dart = "int f(int a) {\n  var b = a + 1;\n  return b;\n}\n";
        assert_eq!(at(Lang::Dart, dart, "a", 1), Some((0, false, vec![1])));
        assert_eq!(at(Lang::Dart, dart, "b", 1), Some((1, false, vec![2])));
        let go = "func f(a int) int {\n\tb := a + 1\n\tfor i, v := range xs { b += i + v }\n\treturn b\n}\n";
        assert_eq!(at(Lang::Go, go, "a", 1), Some((0, false, vec![1])));
        assert_eq!(at(Lang::Go, go, "b", 1), Some((1, false, vec![2, 3])));
        assert_eq!(at(Lang::Go, go, "v", 1), Some((2, false, vec![2])));
        let ts = "function f(a: number) {\n  const b = a + 1;\n  return [1].map(x => x + b);\n}\n";
        assert_eq!(at(Lang::TypeScript, ts, "a", 1), Some((0, false, vec![1])));
        assert_eq!(at(Lang::TypeScript, ts, "x", 1), Some((2, false, vec![2])));
        assert_eq!(at(Lang::TypeScript, ts, "b", 1), Some((1, false, vec![2])));
        let py = "def f(a, b=1):\n    c = a + b\n    for x in c:\n        print(x)\n    return c\n";
        assert_eq!(at(Lang::Python, py, "a", 1), Some((0, false, vec![1])));
        assert_eq!(at(Lang::Python, py, "c", 1), Some((1, false, vec![2, 4])));
        assert_eq!(at(Lang::Python, py, "x", 1), Some((2, false, vec![3])));
    }
}



//! Change Signature's view of the syntax tree (Kotlin and Java): the
//! function at the caret, its parameters as written (modifiers, type,
//! Kotlin's default value) and their uses in its body, and its return type.

use tree_sitter::Node;

use super::extract_method::{children, descendants, inside, is_function, is_reference, java_decls, kotlin_decls};
use super::lang::Lang;
use super::symbols;
use crate::domain::change_signature::{Param, Signature};

const NO_FUNCTION: &str = "Cannot perform refactoring.\nThe caret should be positioned at the name of the method to be refactored";

fn class_name(class: Node) -> Option<Node> {
    children(class).into_iter().find(|c| c.kind() == "type_identifier")
}

fn trimmed(text: &str, start: usize, end: usize) -> std::ops::Range<usize> {
    let s = &text[start..end];
    let lead = s.len() - s.trim_start().len();
    start + lead..start + s.trim_end().len()
}

/// The function whose declaration or body holds `offset`.
pub fn signature_at(lang: Lang, text: &str, offset: usize) -> Result<Signature, String> {
    if !matches!(lang, Lang::Kotlin | Lang::Java) {
        return Err("Change Signature works in Kotlin and Java files".into());
    }
    let tree = symbols::parse(lang, text).ok_or(NO_FUNCTION)?;
    // The innermost function there; Kotlin's constructors too: a secondary
    // one, or the primary one with the caret in the class's header.
    let span = |n: &Node| -> std::ops::Range<usize> {
        if n.kind() == "class_declaration" {
            let body = children(*n).into_iter().find(|c| matches!(c.kind(), "class_body" | "enum_class_body"));
            n.start_byte()..body.map_or(n.end_byte(), |b| b.start_byte())
        } else {
            n.byte_range()
        }
    };
    let candidate = |n: &Node| match n.kind() {
        "secondary_constructor" => lang == Lang::Kotlin,
        "class_declaration" => lang == Lang::Kotlin && children(*n).iter().any(|c| c.kind() == "primary_constructor"),
        _ => is_function(lang, *n),
    };
    let function = descendants(tree.root_node())
        .into_iter()
        .filter(|n| candidate(n) && span(n).start <= offset && offset <= span(n).end)
        .min_by_key(|n| span(n).len())
        .ok_or(NO_FUNCTION)?;
    let constructor = matches!(function.kind(), "constructor_declaration" | "secondary_constructor" | "class_declaration");
    // The name's node, its text (a constructor's is its class's) and the parameters' node.
    let (name, name_text, params_node) = match function.kind() {
        "class_declaration" => {
            let name = class_name(function).ok_or(NO_FUNCTION)?;
            (name, text[name.byte_range()].to_owned(), children(function).into_iter().find(|c| c.kind() == "primary_constructor").ok_or(NO_FUNCTION)?)
        }
        "secondary_constructor" => {
            let class = std::iter::successors(function.parent(), |p| p.parent()).find(|p| p.kind() == "class_declaration").and_then(class_name).ok_or(NO_FUNCTION)?;
            let mut cursor = function.walk();
            let keyword = function.children(&mut cursor).find(|c| c.kind() == "constructor").ok_or(NO_FUNCTION)?;
            (keyword, text[class.byte_range()].to_owned(), children(function).into_iter().find(|c| c.kind() == "function_value_parameters").ok_or(NO_FUNCTION)?)
        }
        _ => {
            let (name, params) = match lang {
                Lang::Kotlin => (
                    children(function).into_iter().find(|c| c.kind() == "simple_identifier").ok_or(NO_FUNCTION)?,
                    children(function).into_iter().find(|c| c.kind() == "function_value_parameters").ok_or(NO_FUNCTION)?,
                ),
                _ => (function.child_by_field_name("name").ok_or(NO_FUNCTION)?, function.child_by_field_name("parameters").ok_or(NO_FUNCTION)?),
            };
            (name, text[name.byte_range()].to_owned(), params)
        }
    };
    // The parameters: the list's nodes between commas, inside the parentheses.
    let mut segments: Vec<Vec<Node>> = vec![Vec::new()];
    let (mut open, mut close) = (None, None);
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "(" if open.is_none() => open = Some(child.end_byte()),
            ")" => close = Some(child.start_byte()),
            _ if open.is_none() || close.is_some() => {}
            "," => segments.push(Vec::new()),
            k if k.ends_with("comment") => {}
            _ => segments.last_mut().unwrap().push(child),
        }
    }
    let (Some(open), Some(close)) = (open, close) else { return Err(NO_FUNCTION.into()) };
    let primary = function.kind() == "class_declaration";
    let decls = match lang {
        _ if primary => Vec::new(),
        Lang::Kotlin => kotlin_decls(function, text),
        _ => java_decls(function, text),
    };
    // Where a primary constructor's parameters are seen: the supertypes'
    // arguments, property initializers and `init` blocks.
    let initializers: Vec<Node> = if primary {
        let body = children(function).into_iter().find(|c| c.kind() == "class_body").map(children).unwrap_or_default();
        children(function).into_iter().filter(|c| c.kind() == "delegation_specifier").chain(body.into_iter().filter(|c| matches!(c.kind(), "property_declaration" | "anonymous_initializer"))).collect()
    } else {
        Vec::new()
    };
    let mut params = Vec::new();
    for seg in segments.into_iter().filter(|s| !s.is_empty()) {
        let (start, end) = (seg[0].start_byte(), seg.last().unwrap().end_byte());
        let mut property = false;
        let (name_node, ty, prefix_end, default) = match lang {
            Lang::Kotlin if seg[0].kind() == "class_parameter" => {
                let p = seg[0];
                let Some(id) = children(p).into_iter().find(|c| c.kind() == "simple_identifier") else { continue };
                property = children(p).iter().any(|c| c.kind() == "binding_pattern_kind");
                let mut cursor = p.walk();
                let equals = p.children(&mut cursor).find(|c| c.kind() == "=");
                let ty_end = equals.map_or(p.end_byte(), |e| e.start_byte());
                let ty = text[id.end_byte()..ty_end].trim().trim_start_matches(':').trim().to_owned();
                let default = equals.map(|e| text[e.end_byte()..p.end_byte()].trim().to_owned());
                (id, ty, id.start_byte(), default)
            }
            Lang::Kotlin => {
                let Some(p) = seg.iter().find(|n| n.kind() == "parameter") else { continue };
                let Some(id) = children(*p).into_iter().find(|c| c.kind() == "simple_identifier") else { continue };
                let ty = text[id.end_byte()..p.end_byte()].trim().trim_start_matches(':').trim().to_owned();
                let default = seg.iter().position(|n| n.kind() == "=").and_then(|i| seg.get(i + 1)).map(|n| text[n.start_byte()..end].trim().to_owned());
                (id, ty, p.start_byte(), default)
            }
            _ => {
                let p = seg[0];
                let id = p.child_by_field_name("name").or_else(|| children(p).into_iter().find(|c| c.kind() == "variable_declarator").and_then(|d| d.child_by_field_name("name")));
                let Some(id) = id else { continue };
                let modifiers = children(p).into_iter().find(|c| c.kind() == "modifiers");
                let ty_start = modifiers.map_or(p.start_byte(), |m| m.end_byte());
                let ty = text[ty_start..id.start_byte()].trim().to_owned();
                // `String... args`: the type without the dots, which `prefix` can't hold.
                (id, ty, ty_start, None)
            }
        };
        let pname = text[name_node.byte_range()].to_owned();
        let prefix = text[start..prefix_end].trim().to_owned();
        let prefix = if prefix.is_empty() { prefix } else { format!("{prefix} ") };
        // Its uses: references in the function resolving to it.
        let decl = decls.iter().find(|d| d.name == pname && inside(&(d.at..d.at), &params_node.byte_range()));
        let scopes = if primary { if property { Vec::new() } else { initializers.clone() } } else { vec![function] };
        let uses = scopes
            .iter()
            .flat_map(|n| descendants(*n))
            .filter(|n| !inside(&n.byte_range(), &params_node.byte_range()) && is_reference(lang, *n, text) && text[n.byte_range()].trim_start_matches('$') == pname)
            .filter(|n| {
                // The innermost declaration of the name before it is the parameter.
                let at = n.start_byte();
                let nearest = decls.iter().filter(|d| d.name == pname && d.at <= at && inside(&(at..at), &d.scope)).max_by_key(|d| d.at);
                nearest.map(|d| d.at) == decl.map(|d| d.at)
            })
            .map(|n| {
                let r = n.byte_range();
                if text[r.clone()].starts_with('$') { r.start + 1..r.end } else { r }
            })
            .collect();
        params.push(Param { name: pname, ty, prefix, default, text: text[trimmed(text, start, end)].to_owned(), uses, property, name_range: name_node.byte_range() });
    }
    // The return type.
    let (return_type, return_range) = match lang {
        Lang::Kotlin if constructor => (String::new(), close..close),
        Lang::Kotlin => {
            let named = children(function);
            let after = named.iter().position(|c| c.id() == params_node.id()).map(|i| &named[i + 1..]).unwrap_or(&[]);
            match after.first().filter(|c| !matches!(c.kind(), "function_body" | "type_constraints")) {
                Some(ty) => (text[ty.byte_range()].to_owned(), params_node.end_byte()..ty.end_byte()),
                None => (String::new(), params_node.end_byte()..params_node.end_byte()),
            }
        }
        _ => match function.child_by_field_name("type") {
            Some(ty) => (text[ty.byte_range()].to_owned(), ty.byte_range()),
            None => (String::new(), name.start_byte()..name.start_byte()),
        },
    };
    let (visibility, visibility_range) = visibility_of(function, text);
    // Where a delegate's header ends: the body (Java's `{`, Kotlin's `{` or
    // `=`), or a secondary constructor's `: this(…)`.
    let body_start = match function.kind() {
        "class_declaration" => None,
        _ if lang == Lang::Java => function.child_by_field_name("body").map(|b| b.start_byte()),
        "secondary_constructor" => Some(children(function).into_iter().find(|c| matches!(c.kind(), "constructor_delegation_call" | "statements" | "block")).map_or(function.end_byte(), |n| n.start_byte())),
        _ => children(function).into_iter().find(|c| c.kind() == "function_body").map(|b| b.start_byte()),
    };
    Ok(Signature {
        decl_start: function.start_byte(),
        body_start,
        visibility,
        visibility_range,
        name: name_text,
        name_range: name.byte_range(),
        params,
        params_range: open..close,
        return_type,
        return_range,
        constructor,
    })
}

/// The function's visibility modifier and its range, else none and where
/// one goes: before its other modifiers, after its annotations. A Kotlin
/// primary constructor's can't change here.
fn visibility_of(function: Node, text: &str) -> (String, Option<std::ops::Range<usize>>) {
    if function.kind() == "class_declaration" {
        return (String::new(), None);
    }
    const VISIBILITIES: [&str; 4] = ["public", "protected", "private", "internal"];
    let modifiers = {
        let mut cursor = function.walk();
        let found = function.children(&mut cursor).find(|c| c.kind() == "modifiers");
        found
    };
    let mut first_modifier = None;
    if let Some(modifiers) = modifiers {
        let mut cursor = modifiers.walk();
        for m in modifiers.children(&mut cursor) {
            if m.kind().contains("annotation") {
                continue;
            }
            let word = &text[m.byte_range()];
            if VISIBILITIES.contains(&word) {
                return (word.to_owned(), Some(m.byte_range()));
            }
            first_modifier.get_or_insert(m.start_byte());
        }
    }
    // No modifier: before the `fun` / `constructor` keyword, Java's type
    // parameters, type or name.
    let at = first_modifier.unwrap_or_else(|| {
        let mut cursor = function.walk();
        let after_modifiers: Vec<Node> = function.children(&mut cursor).filter(|c| c.kind() != "modifiers" && !c.kind().contains("comment")).collect();
        after_modifiers.first().map_or(function.start_byte(), |n| n.start_byte())
    });
    (String::new(), Some(at..at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::change_signature::{NewParam, SignatureRequest, edits};
    use crate::domain::imports::ImportStyle;
    use crate::domain::introduce_parameter::apply;

    #[test]
    fn reads_and_changes_a_kotlin_function() {
        let text = "class Cart {\n    fun total(price: Double, vararg counts: Int = intArrayOf()): Double {\n        val price2 = price\n        return price * counts.size\n    }\n    fun use() = total(1.0, 2, 3)\n}\n";
        let at = text.find("return").unwrap();
        let s = signature_at(Lang::Kotlin, text, at).unwrap();
        assert_eq!((s.name.as_str(), s.return_type.as_str(), s.params.len()), ("total", "Double", 2));
        assert_eq!((s.params[1].prefix.as_str(), s.params[1].ty.as_str(), s.params[1].default.as_deref()), ("vararg ", "Int", Some("intArrayOf()")));
        assert_eq!(s.params[0].uses.len(), 2);
        let mut request = SignatureRequest::of(&s);
        request.params[0].name = "amount".into();
        request.params.push(NewParam { old: None, name: "tax".into(), ty: "Double".into(), value: "0.0".into() });
        request.return_type = "Int".into();
        let out = apply(text, edits(ImportStyle::Kotlin, text, &s, &[], &request).unwrap());
        assert!(out.contains("fun total(amount: Double, vararg counts: Int = intArrayOf(), tax: Double): Int {"), "{out}");
        assert!(out.contains("val price2 = amount\n        return amount * counts.size"), "{out}");
    }

    #[test]
    fn reads_a_java_method() {
        let text = "class A {\n  public static int total(final int price, String... names) { return price; }\n}\n";
        let s = signature_at(Lang::Java, text, text.find("return").unwrap()).unwrap();
        assert_eq!((s.return_type.as_str(), s.params[0].prefix.as_str(), s.params[0].ty.as_str(), s.params[1].ty.as_str()), ("int", "final ", "int", "String..."));
        assert_eq!(&text[s.params_range.clone()], "final int price, String... names");
        assert!(signature_at(Lang::Java, text, 2).is_err());
    }

    #[test]
    fn changes_kotlin_constructors() {
        use crate::domain::change_signature::{call_edits, check};
        use crate::index::introduce_parameter::call_sites;
        let text = "open class Cart(private val rate: Double, count: Int = 2) {\n    constructor(label: String) : this(1.0, label.length)\n    val n = count * 2\n}\nclass Sub : Cart(2.0, 1)\nval c = Cart(3.0)\n";
        let s = signature_at(Lang::Kotlin, text, text.find("Cart(").unwrap() + 1).unwrap();
        assert_eq!((s.name.as_str(), s.constructor, s.params.len()), ("Cart", true, 2));
        assert_eq!((s.params[0].property, s.params[0].prefix.as_str(), s.params[1].default.as_deref()), (true, "private val ", Some("2")));
        assert_eq!(s.params[1].uses.len(), 1);
        let calls = call_sites(Lang::Kotlin, text, "Cart", 2);
        assert_eq!(calls.iter().map(|c| &text[c.name_range.clone()]).collect::<Vec<_>>(), ["this", "Cart", "Cart"]);
        // Rename `count`, add `tax` at the end.
        let mut request = SignatureRequest::of(&s);
        request.params[1].name = "items".into();
        request.params.push(NewParam { old: None, name: "tax".into(), ty: "Double".into(), value: "0.0".into() });
        let mut changes = edits(ImportStyle::Kotlin, text, &s, &[], &request).unwrap();
        changes.extend(calls.iter().flat_map(|c| call_edits(ImportStyle::Kotlin, &s, &request, c)));
        assert_eq!(
            apply(text, changes),
            "open class Cart(private val rate: Double, items: Int = 2, tax: Double) {\n    constructor(label: String) : this(1.0, label.length, 0.0)\n    val n = items * 2\n}\nclass Sub : Cart(2.0, 1, 0.0)\nval c = Cart(3.0, tax = 0.0)\n"
        );
        // The property is the class's: not renamed or removed here.
        let mut rename = SignatureRequest::of(&s);
        rename.params[0].name = "ratio".into();
        assert!(check(ImportStyle::Kotlin, &s, &rename).unwrap_err().contains("Rename"));
        // A secondary constructor.
        let second = signature_at(Lang::Kotlin, text, text.find("label: String").unwrap()).unwrap();
        assert_eq!((second.name.as_str(), second.params.len(), &text[second.name_range.clone()]), ("Cart", 1, "constructor"));
    }

    #[test]
    fn renames_references_too() {
        use crate::domain::change_signature::call_edits;
        use crate::index::introduce_parameter::call_sites;
        let kotlin = "class Cart {\n    fun total(n: Int) = n\n}\nfun use(c: Cart) = listOf(1).map(c::total) + listOf(Cart()).map(Cart::total) + c.total(3)\n";
        let java = "class Cart {\n  int total(int n) { return n; }\n  void use(java.util.List<Integer> l) { l.forEach(this::total); total(1); java.util.function.Supplier<Cart> s = Cart::new; }\n}\n";
        for (lang, style, text, expected) in [
            (Lang::Kotlin, ImportStyle::Kotlin, kotlin, "fun use(c: Cart) = listOf(1).map(c::sum) + listOf(Cart()).map(Cart::sum) + c.sum(3, 0)"),
            (Lang::Java, ImportStyle::Java, java, "  void use(java.util.List<Integer> l) { l.forEach(this::sum); sum(1, 0); java.util.function.Supplier<Cart> s = Cart::new; }"),
        ] {
            let s = signature_at(lang, text, text.find("total(").unwrap() + 1).unwrap();
            let mut request = SignatureRequest::of(&s);
            request.name = "sum".into();
            request.params.push(NewParam { old: None, name: "m".into(), ty: "int".into(), value: "0".into() });
            let calls = call_sites(lang, text, "total", 1);
            let out = apply(text, calls.iter().flat_map(|c| call_edits(style, &s, &request, c)).collect());
            assert!(out.contains(expected), "{out}");
        }
    }

    #[test]
    fn keeps_kotlin_trailing_lambdas_last() {
        use crate::domain::change_signature::call_edits;
        use crate::index::introduce_parameter::call_sites;
        let text = "fun repeat(times: Int, block: () -> Unit) {}\nfun each(block: (Int) -> Unit) {}\nfun use() {\n    repeat(2) { println() }\n    repeat(times = 1) { }\n    repeat(3, { })\n    each { println(it) }\n}\n";
        let change = |function: &str, f: &dyn Fn(&mut SignatureRequest)| {
            let s = signature_at(Lang::Kotlin, text, text.find(&format!("fun {function}")).unwrap() + 5).unwrap();
            let mut request = SignatureRequest::of(&s);
            f(&mut request);
            let calls = call_sites(Lang::Kotlin, text, function, s.params.len());
            let out = apply(text, calls.iter().flat_map(|c| call_edits(ImportStyle::Kotlin, &s, &request, c)).collect());
            out.lines().skip(3).take(4).map(str::to_owned).collect::<Vec<_>>().join("\n")
        };
        // A new parameter before the lambda: it stays trailing.
        let added = change("repeat", &|r| r.params.insert(1, NewParam { old: None, name: "gap".into(), ty: "Long".into(), value: "0L".into() }));
        assert_eq!(added, "    repeat(2, 0L) { println() }\n    repeat(times = 1, gap = 0L) { }\n    repeat(3, 0L, { })\n    each { println(it) }");
        // The lambda moved first: it goes inside the parentheses.
        let swapped = change("repeat", &|r| r.params.swap(0, 1));
        assert_eq!(swapped, "    repeat({ println() }, 2)\n    repeat(block = { }, times = 1)\n    repeat({ }, 3)\n    each { println(it) }");
        // No parentheses: they're added when needed.
        let each = change("each", &|r| r.params.insert(0, NewParam { old: None, name: "from".into(), ty: "Int".into(), value: "0".into() }));
        assert_eq!(each, "    repeat(2) { println() }\n    repeat(times = 1) { }\n    repeat(3, { })\n    each(0) { println(it) }");
    }

    #[test]
    fn changes_the_visibility() {
        let change = |lang: Lang, text: &str, at: &str, to: &str| {
            let s = signature_at(lang, text, text.find(at).unwrap()).unwrap();
            let mut request = SignatureRequest::of(&s);
            request.visibility = Some(to.into());
            let style = if lang == Lang::Java { ImportStyle::Java } else { ImportStyle::Kotlin };
            apply(text, edits(style, text, &s, &[], &request).unwrap())
        };
        let kt = "class A {\n    @Deprecated(\"x\")\n    override suspend fun go() {}\n    private fun stop() {}\n    fun run() {}\n}\n";
        // Before the other modifiers, after the annotations.
        assert!(change(Lang::Kotlin, kt, "go", "internal").contains("@Deprecated(\"x\")\n    internal override suspend fun go()"));
        assert!(change(Lang::Kotlin, kt, "stop", "public").contains("    fun stop()"));
        assert!(change(Lang::Kotlin, kt, "stop", "protected").contains("    protected fun stop()"));
        assert!(change(Lang::Kotlin, kt, "run", "private").contains("    private fun run()"));
        assert_eq!(change(Lang::Kotlin, kt, "run", "public"), kt);
        let java = "class A {\n    @Override\n    public static int go() { return 1; }\n    void run() {}\n    <T> T pick() { return null; }\n    A() {}\n}\n";
        assert!(change(Lang::Java, java, "go", "package-private").contains("@Override\n    static int go()"));
        assert!(change(Lang::Java, java, "run", "private").contains("    private void run()"));
        assert!(change(Lang::Java, java, "pick", "protected").contains("    protected <T> T pick()"));
        assert!(change(Lang::Java, java, "A()", "public").contains("    public A() {}"));
        // A primary constructor's stays.
        let primary = "class P(val x: Int)\n";
        assert_eq!(signature_at(Lang::Kotlin, primary, 8).unwrap().visibility_range, None);
    }

    #[test]
    fn delegates_via_an_overloading_method() {
        use crate::domain::change_signature::call_edits;
        use crate::index::introduce_parameter::call_sites;
        let text = "class Calc {\n    @Deprecated\n    public int add(int a, int b) {\n        return a + b;\n    }\n    void use() { add(1, 2); }\n    Calc(int x) {}\n}\n";
        let s = signature_at(Lang::Java, text, text.find("add").unwrap()).unwrap();
        let mut request = SignatureRequest::of(&s);
        request.params.insert(1, NewParam { old: None, name: "scale".into(), ty: "int".into(), value: "1".into() });
        request.params.swap(0, 2);
        request.delegate = true;
        let calls = call_sites(Lang::Java, text, "add", 2);
        assert!(calls.iter().all(|c| call_edits(ImportStyle::Java, &s, &request, c).is_empty()));
        let out = apply(text, edits(ImportStyle::Java, text, &s, &calls, &request).unwrap());
        assert!(out.contains("    @Deprecated\n    public int add(int a, int b) {\n        return add(b, 1, a);\n    }\n\n    @Deprecated\n    public int add(int b, int scale, int a) {"), "{out}");
        assert!(out.contains("void use() { add(1, 2); }"));
        // A constructor calls this(…); with a visibility change too.
        let s = signature_at(Lang::Java, text, text.find("Calc(int").unwrap()).unwrap();
        let mut request = SignatureRequest::of(&s);
        request.params.push(NewParam { old: None, name: "y".into(), ty: "int".into(), value: "0".into() });
        (request.delegate, request.visibility) = (true, Some("private".into()));
        let out = apply(text, edits(ImportStyle::Java, text, &s, &[], &request).unwrap());
        assert!(out.contains("    Calc(int x) {\n        this(x, 0);\n    }\n\n    private Calc(int x, int y) {}"), "{out}");
    }
}

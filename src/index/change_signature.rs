//! Change Signature's view of the syntax tree (Kotlin and Java): the
//! function at the caret, its parameters as written (modifiers, type,
//! Kotlin's default value) and their uses in its body, and its return type.

use tree_sitter::Node;

use super::extract_method::{children, descendants, inside, is_function, is_reference, java_decls, kotlin_decls};
use super::lang::Lang;
use super::symbols;
use crate::domain::change_signature::{Param, Signature};

const NO_FUNCTION: &str = "Cannot perform refactoring.\nThe caret should be positioned at the name of the method to be refactored";

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
    let function = descendants(tree.root_node())
        .into_iter()
        .filter(|n| is_function(lang, *n) && n.start_byte() <= offset && offset <= n.end_byte())
        .min_by_key(|n| n.end_byte() - n.start_byte())
        .ok_or(NO_FUNCTION)?;
    let constructor = function.kind() == "constructor_declaration";
    let (name, params_node) = match lang {
        Lang::Kotlin => (
            children(function).into_iter().find(|c| c.kind() == "simple_identifier").ok_or(NO_FUNCTION)?,
            children(function).into_iter().find(|c| c.kind() == "function_value_parameters").ok_or(NO_FUNCTION)?,
        ),
        _ => (function.child_by_field_name("name").ok_or(NO_FUNCTION)?, function.child_by_field_name("parameters").ok_or(NO_FUNCTION)?),
    };
    // The parameters: the list's nodes between commas.
    let mut segments: Vec<Vec<Node>> = vec![Vec::new()];
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "(" | ")" => {}
            "," => segments.push(Vec::new()),
            k if k.ends_with("comment") => {}
            _ => segments.last_mut().unwrap().push(child),
        }
    }
    let decls = match lang {
        Lang::Kotlin => kotlin_decls(function, text),
        _ => java_decls(function, text),
    };
    let mut params = Vec::new();
    for seg in segments.into_iter().filter(|s| !s.is_empty()) {
        let (start, end) = (seg[0].start_byte(), seg.last().unwrap().end_byte());
        let (name_node, ty, prefix_end, default) = match lang {
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
        let uses = descendants(function)
            .into_iter()
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
        params.push(Param { name: pname, ty, prefix, default, text: text[trimmed(text, start, end)].to_owned(), uses });
    }
    // The return type.
    let (return_type, return_range) = match lang {
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
    Ok(Signature {
        name: text[name.byte_range()].to_owned(),
        name_range: name.byte_range(),
        params,
        params_range: params_node.start_byte() + 1..params_node.end_byte() - 1,
        return_type,
        return_range,
        constructor,
    })
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
}

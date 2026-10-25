//! Alt+Enter › Import function / Import extension function (Kotlin), as
//! IntelliJ offers them on a call the file can't resolve: `foo()` with
//! no visible `foo`, or `x.shout()` where `shout` is an extension
//! function declared in a package the file doesn't import. This part
//! reads the call at the caret and a declaration's receiver; the index
//! finds the declarations.

/// The call at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    /// `x.name(…)` (or `x?.name`): only an extension function fits.
    pub member: bool,
    /// Where the name starts.
    pub start: usize,
}

/// The function called at `offset`: a lower-case name followed by `(` or
/// a trailing lambda's `{`; not one being declared (`fun foo(`, `fun
/// String.foo(`), not `::foo`.
pub fn call_at(text: &str, offset: usize) -> Option<Call> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let offset = offset.min(text.len());
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(offset, |(i, _)| i);
    let end = offset + text[offset..].chars().take_while(|c| word(*c)).map(char::len_utf8).sum::<usize>();
    let name = &text[start..end];
    if !name.starts_with(|c: char| c.is_lowercase()) {
        return None;
    }
    let after = text[end..].trim_start_matches([' ', '\t']);
    if !(after.starts_with('(') || after.starts_with('{')) {
        return None;
    }
    let before = text[..start].trim_end_matches([' ', '\t']);
    if before.ends_with("::") {
        return None;
    }
    // A declaration: `fun` then at most type parameters and a receiver.
    let line = &text[text[..start].rfind('\n').map_or(0, |i| i + 1)..start];
    if let Some(at) = line.rfind("fun ") {
        let between = &line[at + 4..];
        if (at == 0 || !line[..at].ends_with(word)) && !between.contains(['(', '=', '{', ')']) {
            return None;
        }
    }
    let member = before.ends_with('.');
    Some(Call { name: name.to_owned(), member, start })
}

/// A Kotlin function's receiver type from its declaration (`fun <T>
/// List<T>.second(): T` → `List<T>`); none for a plain function.
pub fn receiver(decl: &str, name: &str) -> Option<String> {
    let rest = decl[decl.find("fun ")? + 4..].trim_start();
    // Type parameters.
    let rest = match rest.strip_prefix('<') {
        Some(inner) => {
            let mut depth = 1;
            let close = inner.char_indices().find(|(_, c)| {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })?;
            inner[close.0 + 1..].trim_start()
        }
        None => rest,
    };
    let head = rest[..rest.find('(')?].trim_end();
    let receiver = head.strip_suffix(name)?.trim_end().strip_suffix('.')?.trim();
    (!receiver.is_empty()).then(|| receiver.to_owned())
}

/// A receiver type's class name: `List<T>?` → `List`, `a.b.Item` → `Item`.
pub fn simple_name(ty: &str) -> &str {
    let ty = ty.trim().trim_end_matches('?');
    let ty = ty.split('<').next().unwrap_or(ty).trim();
    ty.rsplit('.').next().unwrap_or(ty)
}

/// The declaration's type parameters' names (`fun <T : Any, R>` → T, R).
pub fn type_params(decl: &str) -> Vec<String> {
    let Some(rest) = decl.find("fun ").map(|i| decl[i + 4..].trim_start()) else { return Vec::new() };
    let Some(inner) = rest.strip_prefix('<') else { return Vec::new() };
    let inner = inner.split('>').next().unwrap_or("");
    inner
        .split(',')
        .filter_map(|p| p.split([':', ' ']).find(|w| !w.is_empty() && !matches!(*w, "in" | "out" | "reified")).map(str::to_owned))
        .collect()
}

/// Whether an extension on `receiver` applies to a value whose type and
/// supertypes are `types` (empty: unknown, so any receiver might).
pub fn receiver_fits(receiver: &str, type_params: &[String], types: &[String]) -> bool {
    let name = simple_name(receiver);
    types.is_empty() || name == "Any" || type_params.iter().any(|p| p == name) || types.iter().any(|t| t == name)
}

/// The Alt+Enter item: `Import function 'foo'`, `Import extension
/// function 'String.shout'`.
pub fn title(name: &str, receiver: Option<&str>) -> String {
    match receiver {
        Some(r) => format!("Import extension function '{}.{name}'", simple_name(r)),
        None => format!("Import function '{name}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_calls() {
        let text = "fun main() {\n    greet(\"a\")\n    \"x\".shout()\n    list?.second { }\n    Foo()\n    val f = ::greet\n    x.size\n}\nfun String.shout() = 1\n";
        let at = |s: &str| text.find(s).unwrap() + 1;
        assert_eq!(call_at(text, at("greet(")), Some(Call { name: "greet".into(), member: false, start: at("greet(") - 1 }));
        assert!(call_at(text, at("shout()")).is_some_and(|c| c.member));
        assert!(call_at(text, at("second")).is_some_and(|c| c.member && c.name == "second"));
        for word in ["Foo()", "greet\n", "size", "main", "shout() ="] {
            assert_eq!(call_at(text, at(word)), None, "{word}");
        }
        // `fun` earlier on the line, the call inside the body.
        let one = "fun f() = greet(1)";
        assert!(call_at(one, one.find("greet").unwrap()).is_some());
    }

    #[test]
    fn reads_receivers() {
        assert_eq!(receiver("fun String.shout(): String = uppercase()", "shout").as_deref(), Some("String"));
        assert_eq!(receiver("inline fun <T> List<T>.second(): T", "second").as_deref(), Some("List<T>"));
        assert_eq!(receiver("fun greet(name: String)", "greet"), None);
        assert_eq!(type_params("inline fun <T : Any, reified R> T.cast(): R"), ["T", "R"]);
        assert_eq!(simple_name("kotlin.collections.List<T>?"), "List");
        let types = ["ArrayList".to_owned(), "List".to_owned()];
        assert!(receiver_fits("List<T>", &[], &types));
        assert!(receiver_fits("T", &["T".into()], &types));
        assert!(!receiver_fits("String", &[], &types));
        assert!(receiver_fits("String", &[], &[]));
        assert_eq!(title("shout", Some("String?")), "Import extension function 'String.shout'");
        assert_eq!(title("greet", None), "Import function 'greet'");
    }
}

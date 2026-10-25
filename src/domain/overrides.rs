//! Code › Override Methods (Ctrl+O) / Implement Methods (Ctrl+I), as
//! IntelliJ generates them: a class header's supertypes (with their type
//! arguments), which of a supertype's members can be overridden and
//! which must be implemented, and the member written in IntelliJ's
//! default templates (Kotlin's `TODO("Not yet implemented")` or a
//! `super` call; Java's `@Override` with a default return value or a
//! `super` call), type parameters replaced by the class's arguments.

/// A supertype in a class header: its simple name and type arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuperType {
    pub name: String,
    pub args: Vec<String>,
}

/// What a member header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// The parameters as written (Kotlin defaults dropped).
    pub params: Vec<String>,
    /// The return or property type; none for Kotlin's implicit Unit.
    pub ty: Option<String>,
    pub kind: MemberKind,
    /// Modifiers kept on the override: `suspend`, `operator`, `protected`…
    pub keep: Vec<String>,
    /// Java's `throws …` clause.
    pub throws: Option<String>,
    pub is_abstract: bool,
    /// Its doc comment (`/** … */`), unindented, for Copy JavaDoc.
    pub doc: Option<String>,
    /// Qualified names of the classes its signature names, as the
    /// supertype's file imports them, for the subclass to import.
    pub imports: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Fun,
    Val,
    Var,
}

/// Splits at top-level commas (outside `<>`, `()`, `[]`).
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut current).trim().to_owned());
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

/// A type reference: `a.b.Comparable<Item>()` → `Comparable` and `[Item]`.
fn super_type(text: &str) -> Option<SuperType> {
    let text = text.trim();
    let text = text.split(" by ").next().unwrap_or(text).trim();
    let base = text.split('(').next().unwrap_or(text).trim();
    let (name, args) = match base.find('<') {
        Some(lt) => (&base[..lt], split_top(base[lt + 1..].trim_end().strip_suffix('>').unwrap_or(&base[lt + 1..]))),
        None => (base, Vec::new()),
    };
    let name = name.trim().rsplit('.').next().unwrap_or("").trim();
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| SuperType { name: name.to_owned(), args })
}

/// A class header's supertypes: Kotlin's after `:`, Java's `extends` and
/// `implements`.
pub fn supertypes(header: &str, kotlin: bool) -> Vec<SuperType> {
    let header = header.split('{').next().unwrap_or(header);
    if kotlin {
        // The `:` after the primary constructor and type parameters.
        let mut depth = 0;
        let mut colon = None;
        for (i, c) in header.char_indices() {
            match c {
                '(' | '<' => depth += 1,
                ')' | '>' => depth -= 1,
                ':' if depth == 0 => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(colon) = colon else { return Vec::new() };
        let list = header[colon + 1..].split(" where ").next().unwrap_or("");
        split_top(list).iter().filter_map(|s| super_type(s)).collect()
    } else {
        let mut out = Vec::new();
        for keyword in ["extends", "implements"] {
            if let Some(at) = header.find(&format!(" {keyword} ")) {
                let rest = &header[at + keyword.len() + 2..];
                let end = ["implements", "extends", "permits"].iter().filter_map(|k| rest.find(&format!(" {k} "))).min().unwrap_or(rest.len());
                out.extend(split_top(&rest[..end]).iter().filter_map(|s| super_type(s)));
            }
        }
        out
    }
}

/// A type's own type parameters: `interface Comparable<in T>` → `[T]`.
pub fn type_params(header: &str, name: &str) -> Vec<String> {
    let Some(at) = header.find(name) else { return Vec::new() };
    let rest = header[at + name.len()..].trim_start();
    let Some(rest) = rest.strip_prefix('<') else { return Vec::new() };
    let mut depth = 1;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    split_top(&rest[..end])
        .into_iter()
        .filter_map(|p| p.split([':', ' ']).filter(|w| !w.is_empty() && !matches!(*w, "in" | "out" | "reified")).next().map(str::to_owned))
        .collect()
}

/// Replaces whole-word type parameters with the class's arguments.
pub fn substitute(text: &str, map: &[(String, String)]) -> String {
    if map.is_empty() {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        let replaced = map.iter().find(|(p, _)| p == word).map_or(word.as_str(), |(_, a)| a.as_str()).to_owned();
        out.push_str(&replaced);
        word.clear();
    };
    for c in text.chars() {
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

/// A member's header (its declaration up to its body): what it is and
/// whether it can be overridden from a subclass (none: final, private,
/// static). `in_interface`: the owner is an interface; `has_body`: the
/// declaration has a body or an initializer.
pub fn member(header: &str, kotlin: bool, in_interface: bool, has_body: bool) -> Option<Member> {
    let header = header.split_whitespace().collect::<Vec<_>>().join(" ");
    let header = header.trim_end_matches(['{', ';', '=']).trim();
    let paren = header.find('(');
    let words: Vec<&str> = header[..paren.unwrap_or(header.len())].split(' ').filter(|w| !w.starts_with('@')).collect();
    let has = |m: &str| words.contains(&m);
    if has("private") || has("static") || has("final") && !kotlin {
        return None;
    }
    if kotlin {
        let kind = if has("fun") { MemberKind::Fun } else if has("var") { MemberKind::Var } else if has("val") { MemberKind::Val } else { return None };
        // Kotlin members are final unless open, abstract, an override or an interface's.
        let is_abstract = has("abstract") || in_interface && !has_body;
        if !(in_interface || has("open") || has("abstract") || has("override")) || has("final") {
            return None;
        }
        let keep: Vec<String> = ["protected", "internal", "suspend", "operator", "infix", "inline", "tailrec"].iter().filter(|m| has(m)).map(|m| m.to_string()).collect();
        match kind {
            MemberKind::Fun => {
                let open = paren?;
                let name = header[..open].rsplit(' ').next()?.rsplit('.').next()?.to_owned();
                let params = crate::domain::parameter_info::parameters(header, header[..open].len() - name.len())?;
                let params = params.into_iter().map(|p| p.split(" = ").next().unwrap_or(&p).trim().to_owned()).collect();
                let close = crate::domain::intentions::matching(header, open)?;
                let ty = header[close + 1..].trim().strip_prefix(':').map(|t| t.split(" where ").next().unwrap_or(t).trim().to_owned()).filter(|t| !t.is_empty());
                Some(Member { name, params, ty, kind, keep, throws: None, is_abstract, doc: None, imports: Vec::new() })
            }
            _ => {
                let decl = header.split(if kind == MemberKind::Var { " var " } else { " val " }).last().unwrap_or(header);
                let decl = decl.strip_prefix("var ").or_else(|| decl.strip_prefix("val ")).unwrap_or(decl);
                let (name, ty) = match decl.split_once(':') {
                    Some((n, t)) => (n.trim(), Some(t.split(" = ").next().unwrap_or(t).split(" by ").next().unwrap_or(t).trim().to_owned())),
                    None => (decl.split(" = ").next().unwrap_or(decl).trim(), None),
                };
                Some(Member { name: name.to_owned(), params: Vec::new(), ty, kind, keep, throws: None, is_abstract, doc: None, imports: Vec::new() })
            }
        }
    } else {
        let open = paren?;
        let close = crate::domain::intentions::matching(header, open)?;
        let before: Vec<&str> = header[..open].split(' ').filter(|w| !w.is_empty() && !w.starts_with('@')).collect();
        let name = (*before.last()?).to_owned();
        // The return type: the word before the name, past modifiers.
        const MODIFIERS: &[&str] = &["public", "protected", "abstract", "default", "synchronized", "native", "strictfp"];
        let ty: Vec<&str> = before[..before.len() - 1].iter().copied().filter(|w| !MODIFIERS.contains(w) && !w.starts_with('<')).collect();
        let ty = ty.join(" ");
        if ty.is_empty() {
            // A constructor.
            return None;
        }
        let params = crate::domain::parameter_info::parameters(header, header[..open].len() - name.len())?;
        let throws = header[close + 1..].trim().strip_prefix("throws ").map(|t| t.trim().to_owned());
        let is_abstract = before.contains(&"abstract") || in_interface && !has_body && !before.contains(&"default");
        // An interface's members are public.
        let visibility = if in_interface || before.contains(&"public") { "public" } else if before.contains(&"protected") { "protected" } else { "" };
        let keep = (!visibility.is_empty()).then(|| vec![visibility.to_owned()]).unwrap_or_default();
        Some(Member { name, params, ty: Some(ty), kind: MemberKind::Fun, keep, throws, is_abstract, doc: None, imports: Vec::new() })
    }
}

/// Kotlin `Any`'s and Java `Object`'s overridable members.
pub fn root_members(kotlin: bool) -> Vec<(String, Member)> {
    let make = |name: &str, params: &[&str], ty: &str, keep: &[&str]| Member {
        name: name.into(),
        params: params.iter().map(|p| p.to_string()).collect(),
        ty: Some(ty.into()),
        kind: MemberKind::Fun,
        keep: keep.iter().map(|k| k.to_string()).collect(),
        throws: None,
        is_abstract: false,
        doc: None,
        imports: Vec::new(),
    };
    if kotlin {
        vec![
            ("Any".into(), make("equals", &["other: Any?"], "Boolean", &[])),
            ("Any".into(), make("hashCode", &[], "Int", &[])),
            ("Any".into(), make("toString", &[], "String", &[])),
        ]
    } else {
        vec![
            ("Object".into(), make("equals", &["Object obj"], "boolean", &["public"])),
            ("Object".into(), make("hashCode", &[], "int", &["public"])),
            ("Object".into(), make("toString", &[], "String", &["public"])),
            ("Object".into(), Member { throws: Some("CloneNotSupportedException".into()), ..make("clone", &[], "Object", &["protected"]) }),
        ]
    }
}

/// How the chooser lists a member: `toString(): String`, Java's
/// `toString():String`.
pub fn label(m: &Member, kotlin: bool) -> String {
    let params = |kotlin: bool| -> String {
        m.params
            .iter()
            .map(|p| if kotlin { p.trim_start_matches("vararg ").split(':').next().unwrap_or(p).trim().to_owned() + ": " + p.split_once(':').map_or("", |(_, t)| t.trim()) } else { p.split(' ').rev().nth(1).unwrap_or(p).to_owned() })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (m.kind, kotlin) {
        (MemberKind::Fun, true) => format!("{}({}){}", m.name, params(true), m.ty.as_ref().map(|t| format!(": {t}")).unwrap_or_default()),
        (MemberKind::Fun, false) => format!("{}({}):{}", m.name, params(false), m.ty.clone().unwrap_or_default()),
        (_, _) => format!("{}{}", m.name, m.ty.as_ref().map(|t| format!(": {t}")).unwrap_or_default()),
    }
}

/// An argument list passing a member's parameters on (`super.f(a, b)`).
fn arguments(m: &Member, kotlin: bool) -> String {
    m.params
        .iter()
        .map(|p| {
            if kotlin {
                let name = p.trim_start_matches("vararg ").split(':').next().unwrap_or(p).trim();
                if p.starts_with("vararg ") { format!("*{name}") } else { name.to_owned() }
            } else {
                p.rsplit(' ').next().unwrap_or(p).trim_start_matches("...").to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Java's default return value for a type, as IntelliJ's Implemented
/// Method Body template writes it.
fn default_value(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "void" => return None,
        "boolean" => "false",
        "byte" | "short" | "int" | "long" | "char" => "0",
        "float" => "0",
        "double" => "0",
        _ => "null",
    })
}

/// A parameter's type as overloads differ by it: Kotlin's after the
/// `:` (with `vararg`), Java's without the name and `final`; package
/// qualifiers and spaces dropped (`java.util.List<String>` is
/// `List<String>`).
fn parameter_type(p: &str, kotlin: bool) -> String {
    let p = p.split(" = ").next().unwrap_or(p);
    let ty = if kotlin {
        let ty = p.split_once(':').map_or(p, |(_, t)| t).trim().to_owned();
        if p.trim_start().starts_with("vararg ") { format!("vararg {ty}") } else { ty }
    } else {
        let words: Vec<&str> = p.split_whitespace().filter(|w| *w != "final" && !w.starts_with('@')).collect();
        words[..words.len().saturating_sub(1)].join(" ")
    };
    // `a.b.C` → `C`, inside type arguments too.
    let mut out = String::new();
    let mut word = String::new();
    for c in ty.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            word.push(c);
        } else {
            out.push_str(word.rsplit('.').next().unwrap_or(&word));
            word.clear();
            if !c.is_whitespace() {
                out.push(c);
            }
        }
    }
    out
}

/// The name and parameter types of a method the class declares, from its
/// source (annotations, modifiers and body included); none for a
/// property.
pub fn declared_signature(source: &str) -> Option<(String, Vec<String>, bool)> {
    let open = source.find('(')?;
    let before = source[..open].trim_end();
    let name_len = before.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '`').map(char::len_utf8).sum::<usize>();
    let name = before[before.len() - name_len..].trim_matches('`').to_owned();
    let kotlin = before.split_whitespace().any(|w| w == "fun");
    if name.is_empty() || !kotlin && before.split_whitespace().count() < 2 {
        return None;
    }
    let params = crate::domain::parameter_info::parameters(source, before.len() - name_len)?;
    Some((name, params.iter().map(|p| parameter_type(p, kotlin)).collect(), kotlin))
}

/// Whether a declared method (`declared_signature`) overrides `m`: the
/// same name and parameter types.
pub fn overrides(declared: &(String, Vec<String>, bool), m: &Member, kotlin: bool) -> bool {
    m.kind == MemberKind::Fun && declared.0 == m.name && declared.1 == m.params.iter().map(|p| parameter_type(p, kotlin)).collect::<Vec<_>>()
}

/// What the chooser sets for the members it writes: Java's Insert
/// @Override (on by default), Copy JavaDoc (off by default), and
/// Kotlin's `super<Interface>` when more than one direct supertype could
/// be meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub insert_override: bool,
    pub copy_doc: bool,
    pub qualifier: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self { insert_override: true, copy_doc: false, qualifier: None }
    }
}

/// The class names a member's signature uses (`Map<Key, List<Item>>` →
/// Map, Key, List, Item), not ones after a `.` (nested or qualified).
pub fn type_names(m: &Member, kotlin: bool) -> Vec<String> {
    let mut texts: Vec<&str> = m.params.iter().map(|p| if kotlin { p.split_once(':').map_or("", |(_, t)| t) } else { p.trim_end().rsplit_once(' ').map_or("", |(t, _)| t) }).collect();
    texts.extend(m.ty.as_deref());
    texts.extend(m.throws.as_deref());
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        let mut at = 0;
        for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            let qualified = text.get(..at).is_some_and(|t| t.ends_with('.'));
            at += word.len() + 1;
            if !qualified && word.starts_with(|c: char| c.is_uppercase()) && !out.iter().any(|w| w == word) {
                out.push(word.to_owned());
            }
        }
    }
    out
}

/// The doc comment (`/** … */`) ending just above line `line`, past
/// annotations, unindented with its `*` lines one space in.
pub fn doc_comment(lines: &[&str], line: usize) -> Option<String> {
    let mut end = line.min(lines.len());
    while end > 0 && lines[end - 1].trim_start().starts_with('@') {
        end -= 1;
    }
    if end == 0 || !lines[end - 1].trim_end().ends_with("*/") {
        return None;
    }
    let mut start = end - 1;
    while !lines[start].trim_start().starts_with("/**") {
        // Another comment's end, or a plain `/* */` comment.
        if start == 0 || start + 1 < end && lines[start].contains("*/") || lines[start].trim_start().starts_with("/*") || end - start > 200 {
            return None;
        }
        start -= 1;
    }
    let doc: Vec<String> = lines[start..end].iter().map(|l| if l.trim_start().starts_with('*') { format!(" {}", l.trim()) } else { l.trim().to_owned() }).collect();
    Some(doc.join("\n"))
}

/// The override, unindented (one `unit` inside), in IntelliJ's templates,
/// with the default options.
#[cfg(test)]
pub fn generate(m: &Member, kotlin: bool, unit: &str) -> String {
    generate_with(m, kotlin, unit, &Options::default())
}

/// The override, unindented (one `unit` inside), in IntelliJ's templates,
/// with the chooser's options: its doc comment first with Copy JavaDoc.
pub fn generate_with(m: &Member, kotlin: bool, unit: &str, options: &Options) -> String {
    let code = generate_code(m, kotlin, unit, options);
    match m.doc.as_deref().filter(|_| options.copy_doc) {
        Some(doc) => format!("{doc}\n{code}"),
        None => code,
    }
}

fn generate_code(m: &Member, kotlin: bool, unit: &str, options: &Options) -> String {
    let sup = options.qualifier.as_ref().map_or_else(|| "super".to_owned(), |q| format!("super<{q}>"));
    let keep = if m.keep.is_empty() { String::new() } else { format!("{} ", m.keep.join(" ")) };
    if kotlin {
        let todo = "TODO(\"Not yet implemented\")";
        match m.kind {
            MemberKind::Fun => {
                let ty = m.ty.as_ref().map(|t| format!(": {t}")).unwrap_or_default();
                let unit_returning = m.ty.as_deref().is_none_or(|t| t == "Unit");
                let body = if m.is_abstract {
                    todo.to_owned()
                } else if unit_returning {
                    format!("{sup}.{}({})", m.name, arguments(m, true))
                } else {
                    format!("return {sup}.{}({})", m.name, arguments(m, true))
                };
                format!("{keep}override fun {}({}){ty} {{\n{unit}{body}\n}}", m.name, m.params.join(", "))
            }
            MemberKind::Val | MemberKind::Var => {
                let keyword = if m.kind == MemberKind::Var { "var" } else { "val" };
                let ty = m.ty.as_ref().map(|t| format!(": {t}")).unwrap_or_default();
                let getter = if m.is_abstract { todo.to_owned() } else { format!("{sup}.{}", m.name) };
                let mut out = format!("{keep}override {keyword} {}{ty}\n{unit}get() = {getter}", m.name);
                if m.kind == MemberKind::Var {
                    let setter = if m.is_abstract { "{}".to_owned() } else { format!("{{\n{unit}{unit}{sup}.{} = value\n{unit}}}", m.name) };
                    out.push_str(&format!("\n{unit}set(value) {setter}"));
                }
                out
            }
        }
    } else {
        let ty = m.ty.clone().unwrap_or_else(|| "void".into());
        let throws = m.throws.as_ref().map(|t| format!(" throws {t}")).unwrap_or_default();
        let call = format!("super.{}({})", m.name, arguments(m, false));
        let body = match (m.is_abstract, default_value(&ty)) {
            (true, Some(v)) => format!("return {v};"),
            (true, None) => String::new(),
            (false, Some(_)) => format!("return {call};"),
            (false, None) => format!("{call};"),
        };
        let body = if body.is_empty() { String::new() } else { format!("{unit}{body}\n") };
        let annotation = if options.insert_override { "@Override\n" } else { "" };
        format!("{annotation}{keep}{ty} {}({}){throws} {{\n{body}}}", m.name, m.params.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_supertypes() {
        let kt = "class Shop(val n: Int) : Base(n), Comparable<Shop>, java.io.Serializable, Runnable by r {";
        let names: Vec<(String, Vec<String>)> = supertypes(kt, true).into_iter().map(|s| (s.name, s.args)).collect();
        assert_eq!(names, [("Base".into(), vec![]), ("Comparable".into(), vec!["Shop".to_owned()]), ("Serializable".into(), vec![]), ("Runnable".into(), vec![])]);
        let java = "public class A<T> extends Base<T> implements Runnable, Comparable<A<T>> {";
        let names: Vec<String> = supertypes(java, false).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Base", "Runnable", "Comparable"]);
        assert_eq!(type_params("public interface Comparable<in T : Any>", "Comparable"), ["T"]);
        assert_eq!(substitute("fun compareTo(other: T): Int", &[("T".into(), "Shop".into())]), "fun compareTo(other: Shop): Int");
    }

    #[test]
    fn reads_members() {
        let m = member("abstract fun area(scale: Double = 1.0): Double", true, false, false).unwrap();
        assert_eq!((m.name.as_str(), m.params.clone(), m.ty.as_deref(), m.is_abstract), ("area", vec!["scale: Double".to_owned()], Some("Double"), true));
        // A class's plain member is final; an interface's is open.
        assert!(member("fun close()", true, false, true).is_none());
        let open = member("fun close() {", true, true, true).unwrap();
        assert!(!open.is_abstract && open.ty.is_none());
        assert!(member("private open fun x()", true, false, true).is_none());
        let prop = member("val label: String", true, true, false).unwrap();
        assert_eq!((prop.kind, prop.ty.as_deref(), prop.is_abstract), (MemberKind::Val, Some("String"), true));
        let java = member("public abstract int compute(int a, String b) throws IOException;", false, false, false).unwrap();
        assert_eq!((java.name.as_str(), java.ty.as_deref(), java.throws.as_deref(), java.is_abstract), ("compute", Some("int"), Some("IOException"), true));
        assert!(member("public static void util()", false, false, true).is_none());
        assert!(member("default void run() {", false, true, true).is_some_and(|m| !m.is_abstract));
        assert_eq!(label(&java, false), "compute(a, b):int".replace("a, b", "int, String"));
        assert_eq!(label(&m, true), "area(scale: Double): Double");
    }

    #[test]
    fn generates_overrides() {
        let area = member("abstract fun area(scale: Double = 1.0): Double", true, false, false).unwrap();
        assert_eq!(generate(&area, true, "    "), "override fun area(scale: Double): Double {\n    TODO(\"Not yet implemented\")\n}");
        let (_, to_string) = root_members(true).into_iter().find(|(_, m)| m.name == "toString").unwrap();
        assert_eq!(generate(&to_string, true, "    "), "override fun toString(): String {\n    return super.toString()\n}");
        let run = member("fun run(vararg xs: Int) {", true, true, true).unwrap();
        assert_eq!(generate(&run, true, "    "), "override fun run(vararg xs: Int) {\n    super.run(*xs)\n}");
        let label = member("val label: String", true, true, false).unwrap();
        assert_eq!(generate(&label, true, "    "), "override val label: String\n    get() = TODO(\"Not yet implemented\")");
        let java = member("int compute(int a, String b);", false, true, false).unwrap();
        assert_eq!(generate(&java, false, "    "), "@Override\npublic int compute(int a, String b) {\n    return 0;\n}");
        let (_, equals) = root_members(false).into_iter().find(|(_, m)| m.name == "equals").unwrap();
        assert_eq!(generate(&equals, false, "    "), "@Override\npublic boolean equals(Object obj) {\n    return super.equals(obj);\n}");
        // Options: no @Override; Kotlin's super<Interface>.
        let no_annotation = Options { insert_override: false, ..Options::default() };
        assert!(generate_with(&java, false, "    ", &no_annotation).starts_with("public int compute("));
        let qualified = Options { qualifier: Some("Runnable".into()), ..Options::default() };
        assert_eq!(generate_with(&run, true, "    ", &qualified), "override fun run(vararg xs: Int) {\n    super<Runnable>.run(*xs)\n}");
        let void = member("public abstract void reset();", false, false, false).unwrap();
        assert_eq!(generate(&void, false, "    "), "@Override\npublic void reset() {\n}");
    }

    #[test]
    fn copies_docs_and_names_types() {
        let lines = ["interface Shape {", "    /**", "     * The area.", "     */", "    @Deprecated(\"x\")", "    fun area(scale: Double): Double", "    /* plain */", "    fun other()", "}"];
        assert_eq!(doc_comment(&lines, 5).as_deref(), Some("/**\n * The area.\n */"));
        assert_eq!(doc_comment(&lines, 7), None);
        assert_eq!(doc_comment(&["/** One line. */", "fun f()"], 1).as_deref(), Some("/** One line. */"));
        let mut area = member("abstract fun area(scale: Double): Double", true, false, false).unwrap();
        area.doc = doc_comment(&lines, 5);
        let copied = Options { copy_doc: true, ..Options::default() };
        assert!(generate_with(&area, true, "    ", &copied).starts_with("/**\n * The area.\n */\noverride fun area("));
        assert!(generate_with(&area, true, "    ", &Options::default()).starts_with("override fun"));
        let m = member("fun load(key: Key, into: MutableMap<String, List<Item>>): Result<a.b.Nested>", true, true, false).unwrap();
        assert_eq!(type_names(&m, true), ["Key", "MutableMap", "String", "List", "Item", "Result"]);
        let java = member("Item find(Key key) throws IOException;", false, true, false).unwrap();
        assert_eq!(type_names(&java, false), ["Key", "Item", "IOException"]);
    }

    #[test]
    fn tells_overloads_apart() {
        let by_int = member("open fun f(x: Int): Int", true, false, true).unwrap();
        let by_list = member("open fun f(items: List<String>)", true, false, true).unwrap();
        let declared = declared_signature("    override fun f(n: Int): Int {\n        return 1\n    }").unwrap();
        assert!(overrides(&declared, &by_int, true));
        assert!(!overrides(&declared, &by_list, true));
        let declared = declared_signature("override fun f(items: kotlin.collections.List<String>) = Unit").unwrap();
        assert!(overrides(&declared, &by_list, true));
        let java = member("public void put(String key, java.util.Map<String, Integer> m);", false, true, false).unwrap();
        let declared = declared_signature("@Override\npublic void put(final String k, Map<String, Integer> values) {}").unwrap();
        assert!(overrides(&declared, &java, false));
        let other = declared_signature("public void put(int k, Map<String, Integer> values) {}").unwrap();
        assert!(!overrides(&other, &java, false));
    }
}

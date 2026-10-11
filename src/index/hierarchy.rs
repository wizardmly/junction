//! Type hierarchy over the built-in index, for the editor gutter's
//! implementation markers as IntelliJ draws them: "Is implemented in" /
//! "Is overridden in" next to an interface, a class with subclasses or a
//! method with overrides, and "Implements" / "Overrides" next to a method
//! that implements or overrides one in a supertype.
//!
//! Names are matched by simple name, as the rest of the index does: good
//! enough to point at the right places, without resolving imports.

use std::collections::{HashMap, HashSet, VecDeque};

use super::nav::Target;
use super::store::{FileEntry, ProjectIndex};
use super::symbols::{Symbol, SymbolKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// An interface (or abstract member) with implementations: green I↓.
    Implemented,
    /// A class with subclasses, a method with overrides: blue O↓.
    Overridden,
    /// Implements an interface member: green I↑.
    Implementing,
    /// Overrides a superclass member: blue O↑.
    Overriding,
}

impl MarkKind {
    pub fn goes_down(self) -> bool {
        matches!(self, MarkKind::Implemented | MarkKind::Overridden)
    }
}

#[derive(Clone, Debug)]
pub struct GutterMark {
    /// 0-based line of the declaration's name.
    pub line: u32,
    pub kind: MarkKind,
    /// The declaration's name, for the popup title.
    pub name: String,
    /// Its parameter types, "(Bundle?)", for a function; filled from the
    /// file's text.
    pub signature: String,
    /// Where the marker leads: implementations, or the super declaration.
    pub targets: Vec<Target>,
}

impl GutterMark {
    /// IntelliJ's popup title, Kotlin's wording in a Kotlin file:
    /// "Choose Implementation of onCreate(Bundle?) (6 found)".
    pub fn title(&self, kotlin: bool) -> String {
        let types = self.targets.first().is_some_and(|t| t.label.split(' ').next().is_some_and(|k| k != "method" && k != "function" && k != "property" && k != "field"));
        let name = format!("{}{}", self.name, self.signature);
        let n = self.targets.len();
        let what = match self.kind {
            MarkKind::Implemented if types => "Choose Implementation of",
            MarkKind::Overridden if types => "Choose Subclass of",
            MarkKind::Implemented => "Choose Implementation of",
            MarkKind::Overridden if kotlin => "Choose Implementation of",
            MarkKind::Overridden => "Choose Overriding Method of",
            MarkKind::Implementing | MarkKind::Overriding => "Choose Super Method of",
        };
        format!("{what} {name} ({n} found)")
    }
}

/// "(Bundle?, Int)": the parameter types of the function named `name` whose
/// declaration starts on `line`.
pub fn signature(text: &str, line: u32, name: &str) -> String {
    let Some(start) = (if line == 0 { Some(0) } else { text.match_indices('\n').nth(line as usize - 1).map(|(i, _)| i + 1) }) else { return String::new() };
    let rest = &text[start..];
    let Some(at) = rest.find(&format!("{name}(")).or_else(|| rest.find(&format!("{name} ("))).filter(|i| *i < rest.find('\n').unwrap_or(rest.len())) else { return String::new() };
    let open = start + at + rest[at..].find('(').unwrap_or(0);
    let bytes = text.as_bytes();
    let (mut depth, mut end) = (0i32, open);
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            b')' | b'>' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
        if i - open > 2000 {
            return String::new();
        }
    }
    if end <= open {
        return String::new();
    }
    // Split at top-level commas.
    let inner = &text[open + 1..end];
    let mut params = Vec::new();
    let (mut depth, mut from) = (0i32, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | '>' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                params.push(&inner[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    params.push(&inner[from..]);
    let types: Vec<String> = params
        .iter()
        .map(|p| p.split('=').next().unwrap_or(p).trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            // Drop annotations.
            let p = p.split_whitespace().filter(|w| !w.starts_with('@')).collect::<Vec<_>>().join(" ");
            match p.split_once(':') {
                // Kotlin / Swift / TS: `name: Type`.
                Some((_, ty)) => ty.trim().to_owned(),
                // Java / C: `final Type name`.
                None => {
                    let words: Vec<&str> = p.split_whitespace().filter(|w| !matches!(*w, "final" | "const" | "vararg")).collect();
                    match words.len() {
                        0 => String::new(),
                        1 => words[0].to_owned(),
                        n => words[..n - 1].join(" "),
                    }
                }
            }
        })
        .collect();
    format!("({})", types.join(", "))
}

const LIMIT: usize = 500;

fn is_abstract(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Interface | SymbolKind::Protocol | SymbolKind::Trait)
}

/// Types that can have subtypes.
fn is_class_like(kind: SymbolKind) -> bool {
    kind.is_type() && kind != SymbolKind::TypeAlias
}

fn is_member(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Method | SymbolKind::Function | SymbolKind::Property | SymbolKind::Field)
}

fn last_segment(container: &str) -> &str {
    container.rsplit('.').next().unwrap_or(container)
}

fn owner_of(s: &Symbol) -> Option<&str> {
    s.container.as_deref().map(last_segment)
}

fn target(path: &str, entry: &FileEntry, s: &Symbol) -> Target {
    Target {
        path: path.to_owned(),
        line: s.line,
        col: s.col,
        name: s.name.clone(),
        label: format!("{} · {}", s.kind.label(), entry.lang.name()),
        container: s.container.clone(),
    }
}

/// Every project type extending `name`, directly or further down.
fn subtypes<'a>(index: &'a ProjectIndex, name: &str) -> Vec<(&'a str, &'a FileEntry, &'a Symbol)> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    let mut names: HashSet<String> = HashSet::from([name.to_owned()]);
    let mut queue = VecDeque::from([name.to_owned()]);
    while let Some(parent) = queue.pop_front() {
        for (path, entry, s) in index.subtypes_of(&parent) {
            if !is_class_like(s.kind) || !seen.insert((path.to_owned(), s.line)) {
                continue;
            }
            out.push((path, entry, s));
            if names.insert(s.name.clone()) {
                queue.push_back(s.name.clone());
            }
            if out.len() >= LIMIT {
                return out;
            }
        }
    }
    out
}

/// As IntelliJ lists them: the project's before libraries', by class
/// name, then location.
fn sort(targets: &mut [Target]) {
    targets.sort_by(|a, b| {
        // The project's first, then libraries'.
        let key = |t: &Target| (ProjectIndex::is_external(&t.path), t.container.clone().unwrap_or_default(), t.name.clone(), t.path.clone(), t.line);
        key(a).cmp(&key(b))
    });
}

/// Members of the types below one type, by member name, built once per
/// type and shared by all of its members.
type MembersBelow<'a> = HashMap<&'a str, Vec<(&'a str, &'a FileEntry, &'a Symbol)>>;

/// The members named `name` with each owner, built once per member name:
/// walking up the supertypes then looks each one up instead of scanning
/// every namesake again.
type ByOwner<'a> = HashMap<&'a str, Vec<(&'a str, &'a FileEntry, &'a Symbol)>>;

/// Per supertype name: whether it is abstract, and its own supertypes.
struct Parent {
    is_abstract: bool,
    supers: Vec<String>,
}

/// The gutter markers of a file, from its current symbols.
pub fn gutter_marks(index: &ProjectIndex, path: &str, symbols: &[Symbol]) -> Vec<GutterMark> {
    let mut out = Vec::new();
    // Subtypes per type name, shared by the type and its members.
    let mut below: HashMap<String, Vec<(&str, &FileEntry, &Symbol)>> = HashMap::new();
    let mut members_below: HashMap<String, MembersBelow> = HashMap::new();
    let mut by_owner: HashMap<&str, ByOwner> = HashMap::new();
    let mut parents: HashMap<String, Parent> = HashMap::new();
    let types_here: HashMap<&str, &Symbol> = symbols.iter().filter(|s| is_class_like(s.kind)).map(|s| (s.name.as_str(), s)).collect();

    for s in symbols {
        if is_class_like(s.kind) {
            let subs = below.entry(s.name.clone()).or_insert_with(|| subtypes(index, &s.name));
            let mut targets: Vec<Target> = subs.iter().map(|(p, e, sub)| target(p, e, sub)).collect();
            // A Rust trait is implemented by `impl Trait for T` blocks, known
            // by their methods: one entry per implementing type.
            if s.kind == SymbolKind::Trait {
                let mut types = HashSet::new();
                for (p, e, m) in index.subtypes_of(&s.name) {
                    if is_member(m.kind) && types.insert((p.to_owned(), m.container.clone())) {
                        let mut t = target(p, e, m);
                        t.name = owner_of(m).unwrap_or(&m.name).to_owned();
                        t.container = None;
                        targets.push(t);
                    }
                }
            }
            if !targets.is_empty() {
                sort(&mut targets);
                let kind = if is_abstract(s.kind) { MarkKind::Implemented } else { MarkKind::Overridden };
                out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets });
            }
            continue;
        }
        if !is_member(s.kind) || s.kind == SymbolKind::Field && s.supers.is_empty() {
            continue;
        }
        let Some(owner) = owner_of(s) else { continue };
        let owner_type = types_here.get(owner).copied();
        let abstract_owner = owner_type.is_some_and(|t| is_abstract(t.kind));

        // Down: the same member in subtypes (Rust: in impls of the trait).
        let members = members_below.entry(owner.to_owned()).or_insert_with(|| {
            let subs = below.entry(owner.to_owned()).or_insert_with(|| subtypes(index, owner));
            let mut members: MembersBelow = HashMap::new();
            for (p, _, sub) in subs.iter() {
                let Some(entry) = index.file(p) else { continue };
                for m in entry.symbols.iter().filter(|m| is_member(m.kind) && owner_of(m) == Some(sub.name.as_str())) {
                    members.entry(m.name.as_str()).or_default().push((p, entry, m));
                }
            }
            if owner_type.is_some_and(|t| t.kind == SymbolKind::Trait) {
                for (p, e, m) in index.subtypes_of(owner) {
                    if is_member(m.kind) {
                        members.entry(m.name.as_str()).or_default().push((p, e, m));
                    }
                }
            }
            members
        });
        let mut targets: Vec<Target> = members.get(s.name.as_str()).into_iter().flatten().map(|(p, e, m)| target(p, e, m)).collect();
        if !targets.is_empty() {
            sort(&mut targets);
            let kind = if s.decl || abstract_owner { MarkKind::Implemented } else { MarkKind::Overridden };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets });
        }

        // Up: the nearest supertype declaring the same member.
        let mut queue: VecDeque<String> = owner_type.map(|t| t.supers.clone()).unwrap_or_default().into();
        queue.extend(s.supers.iter().cloned());
        if queue.is_empty() {
            continue;
        }
        let namesakes = by_owner.entry(s.name.as_str()).or_insert_with(|| {
            let mut map: ByOwner = HashMap::new();
            for (p, e, m) in index.symbols_named(&s.name) {
                if is_member(m.kind) {
                    if let Some(owner) = owner_of(m) {
                        map.entry(owner).or_default().push((p, e, m));
                    }
                }
            }
            map
        });
        let mut seen: HashSet<String> = HashSet::from([owner.to_owned()]);
        let mut found: Vec<(Target, bool)> = Vec::new();
        while let Some(parent) = queue.pop_front() {
            if !seen.insert(parent.clone()) || seen.len() > 64 {
                continue;
            }
            let info = parents.entry(parent.clone()).or_insert_with(|| {
                let mut info = Parent { is_abstract: false, supers: Vec::new() };
                for (_, _, t) in index.symbols_named(&parent) {
                    if is_class_like(t.kind) {
                        info.is_abstract |= is_abstract(t.kind);
                        info.supers.extend(t.supers.iter().cloned());
                    }
                }
                info
            });
            for (p, e, m) in namesakes.get(parent.as_str()).into_iter().flatten() {
                if !(*p == path && m.line == s.line) {
                    found.push((target(p, e, m), m.decl || info.is_abstract));
                }
            }
            if !found.is_empty() {
                break;
            }
            queue.extend(info.supers.iter().cloned());
        }
        if !found.is_empty() {
            let kind = if found.iter().any(|(_, implements)| *implements) { MarkKind::Implementing } else { MarkKind::Overriding };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), signature: String::new(), targets: found.into_iter().map(|(t, _)| t).collect() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::lang::Lang;
    use crate::index::symbols;

    fn index_of(files: &[(&str, Lang, &str)]) -> ProjectIndex {
        let mut index = ProjectIndex::default();
        for (path, lang, text) in files {
            let entry = FileEntry { lang: *lang, mtime: 0, size: 0, symbols: symbols::extract(*lang, text), bridges: Vec::new() };
            index.insert_for_test(path, entry);
        }
        index
    }

    fn marks(index: &ProjectIndex, path: &str) -> Vec<(u32, MarkKind, Vec<String>)> {
        let symbols = index.files[path].symbols.clone();
        gutter_marks(index, path, &symbols)
            .into_iter()
            .map(|m| (m.line, m.kind, m.targets.iter().map(|t| format!("{}:{}", t.path, t.line)).collect()))
            .collect()
    }

    #[test]
    fn kotlin_interface_and_overrides() {
        let index = index_of(&[
            ("Shape.kt", Lang::Kotlin, "interface Shape {\n    fun area(): Double\n}\nopen class Base : Shape {\n    override fun area() = 1.0\n}\n"),
            ("Square.kt", Lang::Kotlin, "class Square : Base() {\n    override fun area() = 2.0\n}\n"),
        ]);
        assert_eq!(
            marks(&index, "Shape.kt"),
            [
                (0, MarkKind::Implemented, vec!["Shape.kt:3".to_owned(), "Square.kt:0".to_owned()]),
                (1, MarkKind::Implemented, vec!["Shape.kt:4".to_owned(), "Square.kt:1".to_owned()]),
                (3, MarkKind::Overridden, vec!["Square.kt:0".to_owned()]),
                (4, MarkKind::Overridden, vec!["Square.kt:1".to_owned()]),
                (4, MarkKind::Implementing, vec!["Shape.kt:1".to_owned()]),
            ]
        );
        assert_eq!(marks(&index, "Square.kt"), [(1, MarkKind::Overriding, vec!["Shape.kt:4".to_owned()])]);
    }

    #[test]
    fn java_swift_cpp_rust() {
        let java = index_of(&[("A.java", Lang::Java, "interface I { void run(); }\nclass A extends java.util.Base<String> implements I, J<X> { public void run() {} }\n")]);
        assert_eq!(java.files["A.java"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["Base", "I", "J"]);
        assert_eq!(marks(&java, "A.java").len(), 3);

        let swift = index_of(&[("a.swift", Lang::Swift, "protocol P { func go() }\nclass A: NSObject, P {\n    func go() {}\n}\n")]);
        assert_eq!(swift.files["a.swift"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["NSObject", "P"]);
        assert_eq!(marks(&swift, "a.swift").iter().map(|m| m.1).collect::<Vec<_>>(), [MarkKind::Implemented, MarkKind::Implemented, MarkKind::Implementing]);

        let cpp = index_of(&[("a.h", Lang::Cpp, "class B { public: virtual void f() = 0; };\nclass A : public B, private ns::C<int> { public: void f() override; };\n")]);
        assert_eq!(cpp.files["a.h"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "C"]);

        let rust = index_of(&[("a.rs", Lang::Rust, "trait Shape { fn area(&self) -> f64; }\nstruct Sq;\nimpl Shape for Sq {\n    fn area(&self) -> f64 { 1.0 }\n}\n")]);
        assert_eq!(
            marks(&rust, "a.rs"),
            [
                (0, MarkKind::Implemented, vec!["a.rs:3".to_owned()]),
                (0, MarkKind::Implemented, vec!["a.rs:3".to_owned()]),
                (3, MarkKind::Implementing, vec!["a.rs:0".to_owned()]),
            ]
        );
    }

    #[test]
    fn dart_ts_python_objc() {
        let dart = index_of(&[("a.dart", Lang::Dart, "abstract class I { void m(); }\nclass A extends B with M implements I, J<T> { void m() {} }\n")]);
        assert_eq!(dart.files["a.dart"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "M", "I", "J"]);
        let ts = index_of(&[("a.ts", Lang::TypeScript, "class A extends B<T> implements I { m() {} }\ninterface I extends K, L { m(): void }\n")]);
        assert_eq!(ts.files["a.ts"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "I"]);
        assert_eq!(ts.files["a.ts"].symbols.iter().find(|s| s.name == "I").unwrap().supers, ["K", "L"]);
        let py = index_of(&[("a.py", Lang::Python, "class A(B, mod.C):\n    def m(self): pass\n")]);
        assert_eq!(py.files["a.py"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["B", "C"]);
        let objc = index_of(&[("a.m", Lang::ObjC, "@interface A : NSObject <P, Q>\n@end\n@protocol P <R>\n@end\n")]);
        assert_eq!(objc.files["a.m"].symbols.iter().find(|s| s.name == "A").unwrap().supers, ["NSObject", "P", "Q"]);
        assert_eq!(objc.files["a.m"].symbols.iter().find(|s| s.name == "P").unwrap().supers, ["R"]);
    }
}

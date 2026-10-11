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
    /// Where the marker leads: implementations, or the super declaration.
    pub targets: Vec<Target>,
}

impl GutterMark {
    /// IntelliJ's popup title.
    pub fn title(&self) -> String {
        let types = self.targets.first().is_some_and(|t| t.label.split(' ').next().is_some_and(|k| k != "method" && k != "function" && k != "property" && k != "field"));
        match self.kind {
            MarkKind::Implemented => format!("Choose Implementation of {}", self.name),
            MarkKind::Overridden if types => format!("Choose Subclass of {}", self.name),
            MarkKind::Overridden => format!("Choose Overriding Method of {}", self.name),
            MarkKind::Implementing | MarkKind::Overriding => format!("Choose Super Method of {}", self.name),
        }
    }
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

/// As IntelliJ lists them: by class name, then location.
fn sort(targets: &mut [Target]) {
    targets.sort_by(|a, b| {
        let key = |t: &Target| (t.container.clone().unwrap_or_default(), t.name.clone(), t.path.clone(), t.line);
        key(a).cmp(&key(b))
    });
}

/// The gutter markers of a file, from its current symbols.
pub fn gutter_marks(index: &ProjectIndex, path: &str, symbols: &[Symbol]) -> Vec<GutterMark> {
    let mut out = Vec::new();
    // Subtypes per type name, shared by the type and its members.
    let mut below: HashMap<String, Vec<(&str, &FileEntry, &Symbol)>> = HashMap::new();
    let types_here: HashMap<&str, &Symbol> = symbols.iter().filter(|s| is_class_like(s.kind)).map(|s| (s.name.as_str(), s)).collect();

    for s in symbols {
        if is_class_like(s.kind) {
            let subs = below.entry(s.name.clone()).or_insert_with(|| subtypes(index, &s.name)).clone();
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
                out.push(GutterMark { line: s.line, kind, name: s.name.clone(), targets });
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
        let subs = below.entry(owner.to_owned()).or_insert_with(|| subtypes(index, owner)).clone();
        let mut targets: Vec<Target> = Vec::new();
        for (p, _, sub) in &subs {
            let Some(entry) = index.file(p) else { continue };
            for m in entry.symbols.iter().filter(|m| m.name == s.name && is_member(m.kind) && owner_of(m) == Some(sub.name.as_str())) {
                targets.push(target(p, entry, m));
            }
        }
        if owner_type.is_some_and(|t| t.kind == SymbolKind::Trait) {
            for (p, e, m) in index.subtypes_of(owner) {
                if m.name == s.name && is_member(m.kind) {
                    targets.push(target(p, e, m));
                }
            }
        }
        if !targets.is_empty() {
            sort(&mut targets);
            let kind = if s.decl || abstract_owner { MarkKind::Implemented } else { MarkKind::Overridden };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), targets });
        }

        // Up: the nearest supertype declaring the same member.
        let mut queue: VecDeque<String> = owner_type.map(|t| t.supers.clone()).unwrap_or_default().into();
        queue.extend(s.supers.iter().cloned());
        let mut seen: HashSet<String> = HashSet::from([owner.to_owned()]);
        let mut found: Vec<(Target, bool)> = Vec::new();
        while let Some(parent) = queue.pop_front() {
            if !seen.insert(parent.clone()) || seen.len() > 64 {
                continue;
            }
            let parent_abstract = index.symbols_named(&parent).any(|(_, _, t)| is_class_like(t.kind) && is_abstract(t.kind));
            for (p, e, m) in index.symbols_named(&s.name) {
                if is_member(m.kind) && owner_of(m) == Some(parent.as_str()) && !(p == path && m.line == s.line) {
                    found.push((target(p, e, m), m.decl || parent_abstract));
                }
            }
            if !found.is_empty() {
                break;
            }
            for (_, _, t) in index.symbols_named(&parent) {
                if is_class_like(t.kind) {
                    queue.extend(t.supers.iter().cloned());
                }
            }
        }
        if !found.is_empty() {
            let kind = if found.iter().any(|(_, implements)| *implements) { MarkKind::Implementing } else { MarkKind::Overriding };
            out.push(GutterMark { line: s.line, kind, name: s.name.clone(), targets: found.into_iter().map(|(t, _)| t).collect() });
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

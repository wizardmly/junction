//! Alt+Enter › Import class, as IntelliJ offers it on a class name the
//! file doesn't see: the project's and libraries' classes of that simple
//! name, to import one ("Class to Import" when there are several).

use crate::domain::completion::{CandidateKind, CodeModel};
use crate::domain::imports;

/// The class name at `offset` and the qualified names it could import;
/// none when the file already sees a class of that name, or the word is a
/// member (`x.Name`), a local, or one of the file's declarations.
pub fn candidates(model: &dyn CodeModel, text: &str, offset: usize) -> Option<(String, Vec<String>)> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(offset, |(i, _)| i);
    let end = offset + text[offset..].chars().take_while(|c| word(*c)).map(char::len_utf8).sum::<usize>();
    let name = &text[start..end];
    if name.is_empty() || !name.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    let ctx = model.context(text, end);
    if ctx.member_access || ctx.in_comment_or_string || text[..start].trim_end().ends_with("import") {
        return None;
    }
    // The file's own: a declaration, a local, a type parameter.
    if model.file_symbols(text, end).iter().chain(model.locals(text, end).iter()).any(|c| c.name == name) {
        return None;
    }
    let types = [CandidateKind::Class, CandidateKind::Interface, CandidateKind::Enum, CandidateKind::TypeAlias];
    let found: Vec<_> = model.declarations(name, 500).into_iter().filter(|c| c.name == name && types.contains(&c.kind)).collect();
    let mut qualified: Vec<String> = Vec::new();
    for c in &found {
        match &c.import {
            // Seen already (same package, imported, implicit): nothing to fix.
            None => return None,
            Some(q) if imports::is_visible(q, ctx.package.as_deref(), &ctx.imports, &ctx.implicit_packages) => return None,
            Some(q) => {
                if !qualified.contains(q) {
                    qualified.push(q.clone());
                }
            }
        }
    }
    // IntelliJ lists them by name.
    qualified.sort();
    (!qualified.is_empty()).then(|| (name.to_owned(), qualified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::completion::{Candidate, CodeContext, Scope};

    struct Model;

    impl CodeModel for Model {
        fn context(&self, text: &str, offset: usize) -> CodeContext {
            CodeContext {
                prefix: offset..offset,
                member_access: text[..offset].trim_end_matches(|c: char| c.is_alphanumeric()).ends_with('.'),
                package: Some("com.example".into()),
                imports: vec!["java.util.List".into()],
                implicit_packages: vec!["java.lang".into()],
                ..Default::default()
            }
        }
        fn locals(&self, _: &str, _: usize) -> Vec<Candidate> {
            vec![Candidate::new("Local", CandidateKind::Local, Scope::Local)]
        }
        fn file_symbols(&self, _: &str, _: usize) -> Vec<Candidate> {
            vec![Candidate::new("Mine", CandidateKind::Class, Scope::File)]
        }
        fn members(&self, _: &[String]) -> Vec<Candidate> {
            Vec::new()
        }
        fn declarations(&self, prefix: &str, _: usize) -> Vec<Candidate> {
            let class = |name: &str, q: &str| Candidate { import: Some(q.into()), ..Candidate::new(name, CandidateKind::Class, Scope::Project) };
            [class("Date", "java.util.Date"), class("Date", "java.sql.Date"), class("DateFormat", "java.text.DateFormat"), class("List", "java.util.List"), class("List", "java.awt.List"), class("String", "java.lang.String"), class("Helper", "com.example.Helper")]
                .into_iter()
                .filter(|c| c.name.starts_with(prefix))
                .collect()
        }
        fn keywords(&self) -> &'static [&'static str] {
            &[]
        }
    }

    #[test]
    fn offers_classes_the_file_does_not_see() {
        let text = "val d = Date()";
        assert_eq!(candidates(&Model, text, 9), Some(("Date".to_owned(), vec!["java.sql.Date".to_owned(), "java.util.Date".into()])));
        // Imported, implicit, same package, the file's own, a local, a member, lower case.
        for (text, at) in [("val l: List<Int>", 8), ("val s: String", 8), ("Helper()", 2), ("Mine()", 1), ("Local", 1), ("x.Date", 3), ("date", 1)] {
            assert_eq!(candidates(&Model, text, at), None, "{text}");
        }
    }
}

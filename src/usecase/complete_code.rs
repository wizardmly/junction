//! Basic completion (IntelliJ's Ctrl+Space and autopopup) from the code
//! model: members after `x.`, otherwise locals, the file's and project's
//! declarations, library declarations and keywords; a picked class not yet
//! visible to the file brings its import.

use std::ops::Range;

use crate::domain::completion::{self, Candidate, CodeModel, Scope};
use crate::domain::imports::{self, ImportStyle};

/// The popup's content: what the pick replaces, and the candidates with
/// the import each one adds.
#[derive(Debug, Default)]
pub struct Completions {
    pub replace: Range<usize>,
    pub items: Vec<Completion>,
}

#[derive(Debug)]
pub struct Completion {
    pub candidate: Candidate,
    /// (byte offset, text) to insert for the candidate's import.
    pub import: Option<(usize, String)>,
}

/// How many candidates the popup lists.
const LIMIT: usize = 100;

/// `explicit`: Ctrl+Space, which lists candidates for an empty prefix too.
pub fn complete(model: &dyn CodeModel, text: &str, offset: usize, explicit: bool, style: Option<ImportStyle>) -> Completions {
    let ctx = model.context(text, offset);
    let prefix = &text[ctx.prefix.clone()];
    let empty = Completions { replace: ctx.prefix.clone(), items: Vec::new() };
    if ctx.in_comment_or_string || (prefix.is_empty() && !explicit && !ctx.member_access) {
        return empty;
    }
    let candidates: Vec<Candidate> = if ctx.member_access {
        if ctx.receivers.is_empty() {
            // An unknown receiver: any member of that name, once typed.
            if prefix.is_empty() {
                return empty;
            }
            model.declarations(prefix, LIMIT * 4).into_iter().filter(|c| c.owner.is_some()).collect()
        } else {
            model.members(&ctx.receivers)
        }
    } else {
        let mut all = model.locals(text, offset);
        all.extend(model.file_symbols(text, offset));
        all.extend(model.keywords().iter().map(|k| Candidate::new(*k, completion::CandidateKind::Keyword, Scope::Keyword)));
        if !prefix.is_empty() {
            // Members of other types need a receiver; top-level ones don't.
            all.extend(model.declarations(prefix, LIMIT * 4).into_iter().filter(|c| c.owner.is_none() || c.kind.is_type()));
        }
        all
    };
    let ranked = completion::rank(prefix, candidates, LIMIT);
    let items = ranked
        .into_iter()
        .map(|candidate| {
            let import = match (style, &candidate.import) {
                (Some(style), Some(qualified)) if !ctx.member_access && !imports::is_visible(qualified, ctx.package.as_deref(), &ctx.imports, &ctx.implicit_packages) => {
                    Some(imports::import_edit(style, text, qualified))
                }
                _ => None,
            };
            Completion { candidate, import }
        })
        .collect();
    Completions { replace: ctx.prefix, items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::completion::{CandidateKind, CodeContext};

    struct Model {
        ctx: CodeContext,
    }

    impl CodeModel for Model {
        fn context(&self, _: &str, _: usize) -> CodeContext {
            self.ctx.clone()
        }
        fn locals(&self, _: &str, _: usize) -> Vec<Candidate> {
            vec![Candidate::new("textSize", CandidateKind::Local, Scope::Local)]
        }
        fn file_symbols(&self, _: &str, _: usize) -> Vec<Candidate> {
            vec![Candidate::new("render", CandidateKind::Method, Scope::Member)]
        }
        fn members(&self, types: &[String]) -> Vec<Candidate> {
            assert_eq!(types, ["TextView"]);
            vec![Candidate::new("setText", CandidateKind::Method, Scope::Library), Candidate::new("getText", CandidateKind::Method, Scope::Library)]
        }
        fn declarations(&self, _: &str, _: usize) -> Vec<Candidate> {
            let mut view = Candidate::new("TextView", CandidateKind::Class, Scope::Library);
            view.import = Some("android.widget.TextView".to_owned());
            vec![view]
        }
        fn keywords(&self) -> &'static [&'static str] {
            &["true", "try", "typealias"]
        }
    }

    #[test]
    fn unqualified_with_import() {
        let text = "package a\n\nfun f() { te }";
        let at = text.find("te }").unwrap() + 2;
        let model = Model { ctx: CodeContext { prefix: at - 2..at, package: Some("a".into()), ..Default::default() } };
        let out = complete(&model, text, at, false, Some(ImportStyle::Kotlin));
        let names: Vec<&str> = out.items.iter().map(|c| c.candidate.name.as_str()).collect();
        assert_eq!(names, ["textSize", "TextView"]);
        assert_eq!(out.items[1].import.as_ref().unwrap().1, "\nimport android.widget.TextView\n");
    }

    #[test]
    fn members_after_dot() {
        let text = "view.";
        let model = Model { ctx: CodeContext { prefix: 5..5, member_access: true, receivers: vec!["TextView".into()], ..Default::default() } };
        let out = complete(&model, text, 5, false, None);
        let names: Vec<&str> = out.items.iter().map(|c| c.candidate.name.as_str()).collect();
        assert_eq!(names, ["getText", "setText"]);
    }

    #[test]
    fn nothing_in_comments_or_before_typing() {
        let model = Model { ctx: CodeContext { prefix: 3..5, in_comment_or_string: true, ..Default::default() } };
        assert!(complete(&model, "// te", 5, true, None).items.is_empty());
        let model = Model { ctx: CodeContext { prefix: 0..0, ..Default::default() } };
        assert!(complete(&model, "", 0, false, None).items.is_empty());
    }
}

//! The editor's problem highlighting: syntax errors re-checked a moment
//! after typing, plus the compiler's problems from the last build, as
//! IntelliJ's wavy underlines with the message on hover.

use gpui_kit::component::input::{Diagnostic, DiagnosticSeverity, RopeExt as _};
use gpui_kit::{AppContext as _, Context};

use super::FileEditor;
use crate::domain::diagnostics::{Origin, Problem, Severity};
use crate::index::service::CodeIndex;
use crate::ui::compile_service::{CompileEvent, CompileService};
use crate::infra::tree_sitter_syntax::TreeSitterSyntax;
use crate::usecase::check_syntax;

impl FileEditor {
    /// Re-checks the syntax (after a pause when `debounce`), then shows it
    /// with the compiler's problems.
    pub(super) fn update_problems(&mut self, debounce: bool, cx: &mut Context<Self>) {
        if self.read_only() || self.image.is_some() {
            return;
        }
        let text = self.text(cx);
        let Some(lang) = CodeIndex::lang_of(&self.path, &text) else { return };
        self.problems_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            }
            let problems = cx.background_spawn(async move { check_syntax::check(&TreeSitterSyntax { lang }, &text) }).await;
            this.update(cx, |this, cx| {
                this.syntax_problems = problems;
                this.show_problems(cx);
            })
            .ok();
        }));
    }

    /// Follows compile on save: this file's problems from its module's
    /// last compile.
    pub(super) fn attach_compiler(&mut self, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let service = CompileService::global(cx);
        let file = self.repository.root().join(&self.own);
        let problems = service.read(cx).problems(&file);
        self.set_compiler_problems(problems, cx);
        self._subscriptions.push(cx.subscribe(&service, move |this, service, event: &CompileEvent, cx| {
            if matches!(event, CompileEvent::ProblemsChanged) {
                let problems = service.read(cx).problems(&file);
                this.set_compiler_problems(problems, cx);
            }
        }));
    }

    /// The compiler's problems for this file, from the last build.
    pub fn set_compiler_problems(&mut self, problems: Vec<Problem>, cx: &mut Context<Self>) {
        if self.compiler_problems != problems {
            self.compiler_problems = problems;
            self.show_problems(cx);
        }
    }

    fn show_problems(&mut self, cx: &mut Context<Self>) {
        let mut all: Vec<Problem> = self.syntax_problems.iter().chain(&self.compiler_problems).cloned().collect();
        all.sort_by_key(|p| (p.start, p.end));
        self.state.update(cx, |state, cx| {
            let text = state.text().clone();
            let Some(set) = state.diagnostics_mut() else { return };
            set.reset(&text);
            let lines = text.lines_len() as u32;
            for p in all.iter().filter(|p| p.start.line < lines) {
                let start = lsp_types::Position::new(p.start.line, p.start.col);
                let end = lsp_types::Position::new(p.end.line.min(lines - 1), p.end.col);
                let (s, e) = (text.position_to_offset(&start), text.position_to_offset(&end));
                // The compiler gives a start only: underline the token there
                // (an identifier or a string literal), as IntelliJ does.
                let e = if p.origin == Origin::Compiler { token_end(&text, s).max(e) } else { e };
                // A problem at a line's end still underlines one character.
                let end = text.offset_to_position(if e <= s { (s + 1).min(text.len()) } else { e });
                set.push(Diagnostic::new(start..end, p.message.clone()).with_severity(severity(p.severity)));
            }
            cx.notify();
        });
        cx.notify();
    }
}

fn severity(s: Severity) -> DiagnosticSeverity {
    match s {
        Severity::Error => DiagnosticSeverity::Error,
        Severity::Warning => DiagnosticSeverity::Warning,
    }
}

/// The end of the token starting at `at`: an identifier, or a string
/// literal up to its closing quote on the same line.
fn token_end(text: &gpui_kit::component::input::Rope, at: usize) -> usize {
    let line_end = text.line_end_offset(text.offset_to_point(at).row);
    let rest = text.slice(at..line_end).to_string();
    let mut chars = rest.char_indices();
    let len = match chars.next() {
        Some((_, q @ ('"' | '\''))) => chars.find(|(_, c)| *c == q).map_or(rest.len(), |(i, c)| i + c.len_utf8()),
        Some((_, c)) if c.is_alphanumeric() || c == '_' => rest.char_indices().find(|(_, c)| !(c.is_alphanumeric() || *c == '_')).map_or(rest.len(), |(i, _)| i),
        _ => 0,
    };
    at + len
}

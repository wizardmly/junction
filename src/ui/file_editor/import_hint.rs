//! The auto-import hint, as IntelliJ's ShowAutoImportPass shows it: on the
//! caret's line, a class name the file doesn't see but the project or a
//! library declares gets a small popup over it, "Import class...
//! com.example.Item?  Alt+Enter"; Alt+Enter (or a click on the link)
//! imports it, or opens "Class to Import" when several classes have the
//! name. Esc dismisses it for that name; it goes when the caret leaves the
//! line or the name gets resolved.

use gpui_kit::component::h_flex;
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px,
};

use super::FileEditor;
use crate::theme::ActivePalette as _;

/// The hint shown: the name's range and the classes it can import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImportHint {
    pub range: std::ops::Range<usize>,
    pub name: String,
    pub classes: Vec<String>,
}

/// After typing or moving stops, as IntelliJ's daemon pass runs.
const DELAY: std::time::Duration = std::time::Duration::from_millis(500);

impl FileEditor {
    /// The caret moved or the text changed: the hint goes unless it's
    /// still right, and the caret's line is looked at again after a pause.
    pub(super) fn follow_import_hint(&mut self, cx: &mut Context<Self>) {
        if self.read_only() || self.jvm_style().is_none() {
            return;
        }
        let state = self.state.read(cx);
        let text = state.value();
        let caret = state.cursor();
        let key = (text.len(), caret);
        if self.import_hint_key == key {
            return;
        }
        self.import_hint_key = key;
        let line = |at: usize| text[..at.min(text.len())].matches('\n').count();
        if self.import_hint.as_ref().is_some_and(|h| text.get(h.range.clone()) != Some(h.name.as_str()) || line(h.range.start) != line(caret)) {
            self.import_hint = None;
            cx.notify();
        }
        self.import_hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELAY).await;
            this.update(cx, |this, cx| this.find_import_hint(cx)).ok();
        }));
    }

    /// The first class name on the caret's line that could be imported.
    fn find_import_hint(&mut self, cx: &mut Context<Self>) {
        let Some((lang, _)) = self.jvm_style() else { return };
        let Some(index) = self.code_index.clone() else { return };
        let index = index.read(cx).index.clone();
        let (text, caret) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.cursor())
        };
        let start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
        let end = text[caret..].find('\n').map_or(text.len(), |i| caret + i);
        let Ok(index) = index.read() else { return };
        let model = crate::infra::index_code_model::IndexCodeModel::new(&index, lang, &self.path);
        let mut found = None;
        let mut at = start;
        while at < end {
            let rest = &text[at..end];
            let Some(offset) = rest.find(|c: char| c.is_uppercase()) else { break };
            let word_start = at + offset;
            let len = text[word_start..end].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(end - word_start);
            let after_word = word_start > 0 && text[..word_start].ends_with(|c: char| c.is_alphanumeric() || c == '_');
            let name = &text[word_start..word_start + len];
            if !after_word && !self.import_hint_dismissed.iter().any(|d| d == name) {
                if let Some((name, classes)) = crate::usecase::import_class::candidates(&model, &text, word_start) {
                    found = Some(ImportHint { range: word_start..word_start + len, name, classes });
                    break;
                }
            }
            at = word_start + len.max(1);
        }
        if self.import_hint != found {
            self.import_hint = found;
            cx.notify();
        }
    }

    /// Alt+Enter while the hint shows: its import. Whether there was one.
    pub(super) fn accept_import_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(hint) = self.import_hint.take() else { return false };
        let at = self.state.read(cx).range_to_bounds(&(hint.range.start..hint.range.start)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.import_one_of(hint.classes, at, window, cx);
        true
    }

    /// Esc: the hint goes, and stays away for that name.
    pub(super) fn dismiss_import_hint(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(hint) = self.import_hint.take() else { return false };
        self.import_hint_dismissed.push(hint.name);
        cx.notify();
        true
    }

    /// The popup over the name.
    pub(super) fn render_import_hint(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let hint = self.import_hint.as_ref()?;
        let at = self.state.read(cx).range_to_bounds(&(hint.range.start..hint.range.start))?;
        let palette = cx.palette().clone();
        let question = format!("{}?", hint.classes[0]);
        Some(
            deferred(
                anchored().position(point(at.left() - px(4.), at.top() - px(30.))).snap_to_window_with_margin(px(8.)).child(
                    h_flex()
                        .id("import-hint")
                        .gap_2()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .whitespace_nowrap()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_md()
                        .child(
                            div().id("import-hint-link").text_color(palette.link).cursor_pointer().child("Import class...").on_click(cx.listener(|this, _, window, cx| {
                                this.accept_import_hint(window, cx);
                            })),
                        )
                        .child(question)
                        .when(hint.classes.len() > 1, |el| el.child(div().text_color(palette.text_secondary).child("(multiple choices...)")))
                        .child(div().text_color(palette.text_secondary).child("Alt+Enter")),
                ),
            )
            .with_priority(1),
        )
    }
}

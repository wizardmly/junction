//! Live Templates in the editor, as IntelliJ's: an abbreviation then Tab
//! expands it (`sout`, `fori`, `iter`…, where its context allows); in the
//! template, Tab or Enter goes to the next variable and Shift+Tab back,
//! a variable's copies follow it as it is typed, and Escape or Tab past
//! the last one leaves the caret at its end. The current variable is
//! outlined.

use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, Window, anchored, deferred, div, px};

use super::FileEditor;
use crate::domain::live_templates::{expand, find, Session};
use crate::theme::ActivePalette as _;

pub(super) struct TemplateState {
    session: Session,
    /// The text as the session last saw it.
    text: String,
    /// The copies' update is ours: its change isn't an edit.
    syncing: bool,
}

impl FileEditor {
    /// Tab (or Shift+Tab, `forward` false): in a template, to the next (or
    /// previous) variable; else expands the abbreviation before the caret.
    /// Whether it did something (else Tab indents as usual).
    pub(super) fn template_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let cursor = self.state.read(cx).cursor();
        if let Some(t) = &mut self.template {
            // The caret left the template's current variable: it's over.
            let inside = t.session.stops.get(t.session.current).is_some_and(|g| g.iter().any(|r| r.start <= cursor && cursor <= r.end));
            if inside {
                let next = if forward { t.session.next() } else { t.session.previous() };
                let done = next.is_none();
                let range = next.unwrap_or(t.session.end..t.session.end);
                if done {
                    self.template = None;
                }
                self.state.update(cx, |state, cx| state.set_selected_range(range, cx));
                cx.notify();
                return true;
            }
            self.template = None;
        }
        if !forward || self.read_only() || self.state.read(cx).is_completion_menu_open(cx) || !self.state.read(cx).selected_range().is_empty() {
            return false;
        }
        self.expand_template(window, cx)
    }

    /// Expands the abbreviation before the caret, if a template has it here.
    pub(super) fn expand_template(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((lang, style)) = self.jvm_style() else { return false };
        let text = self.text(cx);
        let cursor = self.state.read(cx).cursor();
        let start = text[..cursor].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '-' || *c == '_').last().map_or(cursor, |(i, _)| i);
        if start == cursor {
            return false;
        }
        let contexts = crate::index::introduce::template_contexts(lang, &text, start);
        let kotlin = style == crate::domain::imports::ImportStyle::Kotlin;
        let Some(template) = find(kotlin, &text[start..cursor], &contexts) else { return false };
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..start].chars().take_while(|c| c.is_whitespace()).collect();
        let expansion = expand(template, &indent, &crate::domain::surround::indent_unit(&text));
        let mut new_text = text.clone();
        new_text.replace_range(start..cursor, &expansion.text);
        let session = Session::new(&expansion, start);
        let selection = session.selection().unwrap_or(session.end..session.end);
        self.state.update(cx, |state, cx| {
            state.set_selected_range(start..cursor, cx);
            state.replace(expansion.text.clone(), window, cx);
            state.set_selected_range(selection, cx);
        });
        self.template = (!session.stops.is_empty()).then_some(TemplateState { session, text: new_text, syncing: false });
        cx.notify();
        true
    }

    /// Enter in a template: to the next variable, as Tab.
    pub(super) fn template_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.template.is_some() && self.template_tab(true, window, cx)
    }

    /// Escape: the template ends, the caret where it is.
    pub(super) fn template_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let active = self.template.take().is_some();
        if active {
            cx.notify();
        }
        active
    }

    /// The text changed: an edit in the current variable updates its
    /// copies; one anywhere else ends the template.
    pub(super) fn template_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = &mut self.template else { return };
        let text = self.state.read(cx).value().to_string();
        if t.syncing {
            t.syncing = false;
            t.text = text;
            return;
        }
        let cursor = self.state.read(cx).cursor();
        match t.session.edited(&t.text, &text, cursor) {
            None => self.template = None,
            Some(None) => t.text = text,
            Some(Some(mirror)) => {
                let mut after = text.clone();
                after.replace_range(mirror.range.clone(), &mirror.text);
                t.text = after;
                t.syncing = true;
                self.state.update(cx, |state, cx| {
                    state.set_selected_range(mirror.range.clone(), cx);
                    state.replace(mirror.text.clone(), window, cx);
                    state.set_selected_range(mirror.caret..mirror.caret, cx);
                });
            }
        }
        cx.notify();
    }

    /// The current variable's places, outlined.
    pub(super) fn render_template_stops(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = self.template.as_ref()?;
        let group = t.session.stops.get(t.session.current)?;
        let state = self.state.read(cx);
        let boxes: Vec<_> = group
            .iter()
            .filter_map(|r| {
                let a = state.range_to_bounds(&(r.start..r.start))?;
                let b = state.range_to_bounds(&(r.end..r.end))?;
                (a.top() == b.top()).then(|| (a.origin, b.left() - a.left(), a.size.height))
            })
            .collect();
        let color = cx.palette().accent;
        Some(
            deferred(div().children(boxes.into_iter().map(move |(origin, width, height)| {
                anchored().position(origin).child(div().w(width.max(px(2.))).h(height).border_1().border_color(color).rounded_sm())
            })))
            .with_priority(1),
        )
    }
}

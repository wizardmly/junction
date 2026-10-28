//! Live Templates in the editor, as IntelliJ's: an abbreviation then Tab
//! expands it (`sout`, `fori`, `iter`…, where its context allows); in the
//! template, Tab or Enter goes to the next variable and Shift+Tab back,
//! a variable's copies follow it as it is typed, and Escape or Tab past
//! the last one leaves the caret at its end. The current variable is
//! outlined.

use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, Window, anchored, deferred, div, px};

use super::FileEditor;
use crate::domain::live_templates::{expand, expand_around, find, resolved, shortened, Expansion, Session, Template};
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
        if !forward || self.read_only() || !self.state.read(cx).selected_range().is_empty() {
            return false;
        }
        // An abbreviation typed in full expands even with the completion
        // popup open (it lists the template too), as IntelliJ's Tab does.
        let expanded = self.expand_template(window, cx);
        if expanded {
            self.state.update(cx, |state, cx| state.dismiss_completion_overlay(cx));
        }
        expanded
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
        // Its computed variables from the code here, its qualified names
        // shortened (imported).
        let template = resolved(&template, &crate::index::introduce::template_scope(lang, &text, start, &|n| self.class_texts(n, false, cx)));
        let (template, imports) = shortened(&template);
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..start].chars().take_while(|c| c.is_whitespace()).collect();
        let expansion = expand(&template, &indent, &crate::domain::surround::indent_unit(&text));
        self.insert_template(start..cursor, expansion.text.clone(), 0, &expansion, imports, window, cx);
        true
    }

    /// Writes a template's `insert` (its expansion at `lead` bytes in) over
    /// `range`, imports its classes, in one edit, and starts filling in
    /// its variables.
    #[allow(clippy::too_many_arguments)]
    fn insert_template(&mut self, range: std::ops::Range<usize>, insert: String, lead: usize, expansion: &Expansion, imports: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, style)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let mut new_text = text.clone();
        new_text.replace_range(range.clone(), &insert);
        let mut at = range.start + lead;
        for qualified in imports {
            let header = crate::domain::imports::header(&new_text);
            let imported: Vec<String> = header.imports.iter().map(|(_, q)| q.clone()).chain(header.wildcards.iter().map(|w| format!("{w}.*"))).collect();
            if crate::domain::imports::is_visible(&qualified, Some(&header.package), &imported, &[]) {
                continue;
            }
            let (offset, line) = crate::domain::imports::import_edit(style, &new_text, &qualified);
            new_text.insert_str(offset, &line);
            if offset <= at {
                at += line.len();
            }
        }
        // One edit from the first change to the last.
        let prefix = text.bytes().zip(new_text.bytes()).take_while(|(a, b)| a == b).count().min(range.start);
        let suffix = text.bytes().rev().zip(new_text.bytes().rev()).take_while(|(a, b)| a == b).count().min(text.len() - range.end);
        let replaced = prefix..text.len() - suffix;
        let replacement = new_text[prefix..new_text.len() - suffix].to_owned();
        let session = Session::new(expansion, at);
        let selection = session.selection().unwrap_or(session.end..session.end);
        self.state.update(cx, |state, cx| {
            state.set_selected_range(replaced, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(selection, cx);
        });
        self.start_template(session, cx);
    }

    /// Fills in `session`'s places one by one, as a template's (the text
    /// already written, the first place selected).
    pub(super) fn start_template(&mut self, session: Session, cx: &mut Context<Self>) {
        let text = self.state.read(cx).value().to_string();
        self.template = (!session.stops.is_empty()).then_some(TemplateState { session, text, syncing: false });
        cx.notify();
    }

    /// Surrounds `range` (whole lines, or text within a line) with a
    /// surround template, its qualified names shortened and imported, and
    /// fills in its variables.
    pub(super) fn surround_with_template(&mut self, template: Template, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, _)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let template = resolved(&template, &crate::index::introduce::template_scope(lang, &text, range.start, &|n| self.class_texts(n, false, cx)));
        let (template, imports) = shortened(&template);
        let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let whole_lines = range.start == line_start && (range.end == text.len() || text[..range.end].ends_with('\n'));
        let selection = &text[range.clone()];
        let indent: String = text[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let expansion = expand_around(&template, selection, &indent, &crate::domain::surround::indent_unit(&text));
        let lead = if whole_lines { indent.as_str() } else { "" };
        let mut insert = format!("{lead}{}", expansion.text);
        if whole_lines && selection.ends_with('\n') && !insert.ends_with('\n') {
            insert.push('\n');
        }
        self.insert_template(range, insert, lead.len(), &expansion, imports, window, cx);
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

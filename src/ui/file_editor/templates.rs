//! Live Templates in the editor, as IntelliJ's: an abbreviation then Tab
//! expands it (`sout`, `fori`, `iter`…, where its context allows); in the
//! template, Tab or Enter goes to the next variable and Shift+Tab back,
//! a variable's copies follow it as it is typed, and Escape or Tab past
//! the last one leaves the caret at its end. The current variable is
//! outlined.

use gpui_kit::{Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, prelude::FluentBuilder as _, px};

use super::FileEditor;
use crate::domain::live_templates::{choice_variables, expand, expand_around, find, resolved, shortened, subtype_variables, Expansion, Session, Template};
use crate::theme::ActivePalette as _;

pub(super) struct TemplateState {
    session: Session,
    /// The text as the session last saw it.
    text: String,
    /// The copies' update is ours: its change isn't an edit.
    syncing: bool,
    /// Each variable's lookup: the values to pick from (none for most).
    lookups: Vec<Vec<String>>,
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
        let choices = choice_variables(&template);
        let template = resolved(&template, &self.template_scope(lang, &text, start, cx));
        let (template, imports) = shortened(&template);
        // `subtypes(…)`: the type and its subclasses to pick from; `enum(…)`: its values.
        let subtypes: Vec<(String, Vec<String>)> = subtype_variables(&template)
            .into_iter()
            .map(|(name, ty)| (name.to_owned(), std::iter::once(ty.to_owned()).chain(self.subclass_names(ty, cx)).collect()))
            .chain(choices)
            .collect();
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..start].chars().take_while(|c| c.is_whitespace()).collect();
        let expansion = expand(&template, &indent, &crate::domain::surround::indent_unit(&text));
        let lookups = expansion.names.iter().map(|n| subtypes.iter().find(|(v, _)| v == n).map(|(_, l)| l.clone()).unwrap_or_default()).collect();
        self.insert_template(start..cursor, expansion.text.clone(), 0, &expansion, imports, lookups, window, cx);
        true
    }

    /// What the code at `offset` declares, with the file's name, package,
    /// line and the clipboard, for the templates' expressions.
    fn template_scope(&self, lang: crate::index::lang::Lang, text: &str, offset: usize, cx: &mut Context<Self>) -> crate::domain::live_templates::Scope {
        let mut scope = crate::index::introduce::template_scope(lang, text, offset, &|n| self.class_texts(n, false, cx));
        scope.file_name = self.path.rsplit('/').next().unwrap_or_default().to_owned();
        scope.package = crate::domain::imports::header(text).package;
        scope.line = text[..offset.min(text.len())].matches('\n').count() + 1;
        scope.clipboard = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        scope
    }

    /// Writes a template's `insert` (its expansion at `lead` bytes in) over
    /// `range`, imports its classes, in one edit, and starts filling in
    /// its variables.
    #[allow(clippy::too_many_arguments)]
    fn insert_template(&mut self, range: std::ops::Range<usize>, insert: String, lead: usize, expansion: &Expansion, imports: Vec<String>, lookups: Vec<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
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
        self.start_template_with_lookup(session, lookups, cx);
    }

    /// Fills in `session`'s places one by one, as a template's (the text
    /// already written, the first place selected).
    pub(super) fn start_template(&mut self, session: Session, cx: &mut Context<Self>) {
        self.start_template_with_lookup(session, Vec::new(), cx);
    }

    /// [`Self::start_template`], with values to pick each variable's from
    /// (Up / Down, or a click), as IntelliJ's template lookups.
    pub(super) fn start_template_with_lookup(&mut self, session: Session, lookups: Vec<Vec<String>>, cx: &mut Context<Self>) {
        let text = self.state.read(cx).value().to_string();
        self.template = (!session.stops.is_empty()).then_some(TemplateState { session, text, syncing: false, lookups });
        self.in_place = None;
        cx.notify();
    }

    /// The current variable's lookup, if it has one to pick from.
    fn lookup(&self) -> Option<&[String]> {
        let t = self.template.as_ref()?;
        t.lookups.get(t.session.current).map(Vec::as_slice).filter(|l| l.len() > 1)
    }

    /// Puts a lookup value in the current variable, selected.
    fn pick_lookup(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.template_places().and_then(|p| p.first().cloned()) else { return };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(range.clone(), cx);
            state.replace(value.clone(), window, cx);
        });
        // The template follows the edit; then the value is selected again.
        self.template_edited(window, cx);
        if let Some(range) = self.template_places().and_then(|p| p.first().cloned()) {
            self.state.update(cx, |state, cx| state.set_selected_range(range, cx));
        }
        cx.notify();
    }

    /// Up / Down in a variable with a lookup: the previous or next value.
    pub(super) fn lookup_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.state.read(cx).is_completion_menu_open(cx) {
            return false;
        }
        let Some(values) = self.lookup() else { return false };
        let Some(range) = self.template_places().and_then(|p| p.first().cloned()) else { return false };
        let current = self.state.read(cx).value().get(range).unwrap_or_default().to_owned();
        let count = values.len() as isize;
        let at = values.iter().position(|v| *v == current).map_or(if delta > 0 { 0 } else { count - 1 }, |i| (i as isize + delta).rem_euclid(count));
        let value = values[at as usize].clone();
        self.pick_lookup(value, window, cx);
        true
    }

    /// Surrounds `range` (whole lines, or text within a line) with a
    /// surround template, its qualified names shortened and imported, and
    /// fills in its variables.
    pub(super) fn surround_with_template(&mut self, template: Template, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, _)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let template = resolved(&template, &self.template_scope(lang, &text, range.start, cx));
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
        self.insert_template(range, insert, lead.len(), &expansion, imports, Vec::new(), window, cx);
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

    /// The current variable's places, the first one first.
    pub(super) fn template_places(&self) -> Option<&[std::ops::Range<usize>]> {
        let t = self.template.as_ref()?;
        t.session.stops.get(t.session.current).map(Vec::as_slice)
    }

    /// The current variable's lookup, under it, its value highlighted.
    pub(super) fn render_template_lookup(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let values = self.lookup()?.to_vec();
        let range = self.template_places()?.first()?.clone();
        let state = self.state.read(cx);
        let current = state.value().get(range.clone()).unwrap_or_default().to_owned();
        let bounds = state.range_to_bounds(&(range.start..range.start))?;
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let rows = values.into_iter().enumerate().map(|(ix, value)| {
            let entity = entity.clone();
            let chosen = value == current;
            gpui_kit::component::h_flex()
                .id(("template-lookup", ix))
                .px_1()
                .rounded_sm()
                .when(chosen, |el| el.bg(palette.selection_inactive))
                .child(value.clone())
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.pick_lookup(value.clone(), window, cx);
                        this.focus(window, cx);
                    })
                })
        });
        Some(
            deferred(
                anchored().position(gpui_kit::point(bounds.left() - px(4.), bounds.bottom() + px(2.))).snap_to_window_with_margin(px(8.)).child(
                    gpui_kit::component::v_flex()
                        .p_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_md()
                        .children(rows),
                ),
            )
            .with_priority(2),
        )
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

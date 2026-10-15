//! Alt+Enter (Show Context Actions) in the editor: the quick fixes of the
//! problems at the caret, as IntelliJ's popup lists them ("Change to
//! 1.13.1"). Enter or a click applies one; Escape closes it. The same
//! popup is Introduce Variable's "Expressions" chooser.

use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Point, StatefulInteractiveElement as _, Styled as _, Window, actions, anchored, deferred, div,
    point, prelude::FluentBuilder as _, px,
};

use super::FileEditor;
use crate::domain::diagnostics::{Fix, Pos};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;

actions!(context_actions, [ShowContextActions, CloseContextActions, SelectPrevContextAction, SelectNextContextAction, ApplyContextAction]);

const POPUP_CONTEXT: &str = "ContextActions";

pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("alt-enter", ShowContextActions, Some(super::CONTEXT)),
        KeyBinding::new("escape", CloseContextActions, Some(POPUP_CONTEXT)),
        KeyBinding::new("up", SelectPrevContextAction, Some(POPUP_CONTEXT)),
        KeyBinding::new("down", SelectNextContextAction, Some(POPUP_CONTEXT)),
        KeyBinding::new("enter", ApplyContextAction, Some(POPUP_CONTEXT)),
    ]);
}

pub(super) struct ContextActions {
    /// Each fix and the byte range it replaces.
    items: Vec<(std::ops::Range<usize>, Choice)>,
    /// "Expressions" over the list.
    title: Option<&'static str>,
    selected: usize,
    at: Point<Pixels>,
    /// The selection to put back when the popup closes unused.
    caret: Option<std::ops::Range<usize>>,
}

#[derive(Clone)]
pub(super) enum Choice {
    Fix(Fix),
    /// An expression to introduce a variable or constant for (its text,
    /// one line).
    Expression(String, crate::ui::introduce_dialog::Introduce),
    /// Generate: what to generate.
    Generate(crate::domain::generate::Member),
    /// Surround With: the surrounder, numbered as in IntelliJ's popup;
    /// the range is the lines or the expression it wraps.
    Surround(crate::domain::surround::Surrounder, char, bool),
}

/// The byte offset of a line and character column.
fn offset_of(text: &str, pos: Pos) -> usize {
    let line_start = text.split_inclusive('\n').take(pos.line as usize).map(str::len).sum::<usize>();
    let line = &text[line_start.min(text.len())..];
    line_start + line.char_indices().nth(pos.col as usize).map_or(line.find('\n').unwrap_or(line.len()), |(i, _)| i)
}

impl FileEditor {
    pub(super) fn show_context_actions(&mut self, _: &ShowContextActions, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        let cursor = self.state.read(cx).cursor();
        let cursor_line = Pos::of(&text, cursor).line;
        let mut items: Vec<(std::ops::Range<usize>, Choice)> = Vec::new();
        // The problems under the caret first, then the rest of its line.
        let mut problems = self.problems();
        problems.sort_by_key(|p| {
            let (s, e) = (offset_of(&text, p.start), offset_of(&text, p.end));
            !(s <= cursor && cursor <= e)
        });
        for p in problems.iter().filter(|p| p.start.line == cursor_line) {
            if let Some(fix) = &p.fix {
                items.push((offset_of(&text, p.start)..offset_of(&text, p.end), Choice::Fix(fix.clone())));
            }
        }
        if items.is_empty() {
            return;
        }
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, title: None, selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// Introduce Variable's "Expressions" popup at the caret: the one
    /// chosen is highlighted in the text as the list moves.
    pub(super) fn choose_expression(&mut self, kind: crate::ui::introduce_dialog::Introduce, text: &str, ranges: Vec<std::ops::Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        let items = ranges
            .into_iter()
            .map(|r| {
                let one_line = text[r.clone()].split_whitespace().collect::<Vec<_>>().join(" ");
                let label = if one_line.chars().count() > 60 { format!("{}…", one_line.chars().take(60).collect::<String>()) } else { one_line };
                (r, Choice::Expression(label, kind))
            })
            .collect();
        let caret = Some(self.state.read(cx).selected_range());
        self.context_actions = Some(ContextActions { items, title: Some("Expressions"), selected: 0, at, caret });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }


    /// Surround With's popup at the caret: the statement surrounders for
    /// the lines, then (for an expression selected within a line) the
    /// expression ones, numbered; a number applies its item.
    pub(super) fn choose_surrounder(&mut self, style: crate::domain::imports::ImportStyle, lines: std::ops::Range<usize>, expression: Option<std::ops::Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::surround::{expression_surrounders, mnemonic, statement_surrounders};
        let mut items: Vec<(std::ops::Range<usize>, Choice)> = Vec::new();
        if let Some(expr) = &expression {
            for s in expression_surrounders(style) {
                items.push((expr.clone(), Choice::Surround(*s, mnemonic(items.len()), true)));
            }
        }
        for s in statement_surrounders(style) {
            items.push((lines.clone(), Choice::Surround(*s, mnemonic(items.len()), false)));
        }
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, title: Some("Surround With"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    /// Generate's popup at the caret.
    pub(super) fn choose_generate(&mut self, items: Vec<crate::domain::generate::Member>, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        let items = items.into_iter().map(|m| (cursor..cursor, Choice::Generate(m))).collect();
        self.context_actions = Some(ContextActions { items, title: Some("Generate"), selected: 0, at, caret: None });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    fn close_context_actions(&mut self, _: &CloseContextActions, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(caret) = self.context_actions.take().and_then(|p| p.caret) {
            self.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn step_context_action(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.context_actions {
            popup.selected = (popup.selected as isize + delta).rem_euclid(popup.items.len() as isize) as usize;
            cx.notify();
        }
    }

    fn apply_context_action(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.context_actions.take() else { return };
        let Some((range, choice)) = popup.items.get(ix).cloned() else { return };
        self.focus(window, cx);
        match choice {
            Choice::Fix(fix) => self.state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(&fix.replacement, window, cx);
            }),
            Choice::Surround(surrounder, _, expression) => self.surround(surrounder, range, expression, window, cx),
            Choice::Generate(member) => self.generate_member(member, window, cx),
            Choice::Expression(_, kind) => {
                if let Some(caret) = popup.caret {
                    self.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
                }
                self.introduce_for(kind, range, window, cx);
            }
        }
        cx.notify();
    }

    /// The expression the "Expressions" popup has selected, highlighted
    /// in the text as IntelliJ does: a box per line it spans.
    pub(super) fn render_expression_highlight(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.context_actions.as_ref()?;
        let (range, Choice::Expression(..)) = popup.items.get(popup.selected)? else { return None };
        let state = self.state.read(cx);
        let text = state.value().to_string();
        let mut boxes = Vec::new();
        let mut line_start = range.start;
        while line_start < range.end {
            let line_end = text[line_start..range.end].find('\n').map_or(range.end, |i| line_start + i);
            if let (Some(a), Some(b)) = (state.range_to_bounds(&(line_start..line_start)), state.range_to_bounds(&(line_end..line_end))) {
                boxes.push((a.origin, b.left() - a.left(), a.size.height));
            }
            line_start = line_end + 1;
        }
        let color = cx.palette().selection.opacity(0.6);
        Some(
            deferred(div().children(boxes.into_iter().map(move |(origin, width, height)| {
                anchored().position(origin).child(div().w(width.max(px(2.))).h(height).bg(color).rounded_sm())
            })))
            .with_priority(1),
        )
    }

    pub(super) fn render_context_actions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.context_actions.as_ref()?;
        let palette = cx.palette().clone();
        let rows = popup.items.iter().enumerate().map(|(ix, (_, choice))| {
            let selected = ix == popup.selected;
            let (icon, label) = match choice {
                Choice::Fix(fix) => (Some(icons::INTENTION_BULB), fix.title.clone()),
                Choice::Expression(text, _) => (None, text.clone()),
                Choice::Surround(s, key, _) => (None, format!("{key}. {}", s.label())),
                Choice::Generate(m) => (None, m.label().to_owned()),
            };
            h_flex()
                .id(("context-action", ix))
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .when(selected, |el| el.bg(palette.selection))
                .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                .when_some(icon, |el, icon| el.child(Icon::new(icon).xsmall()))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.apply_context_action(ix, window, cx)))
        });
        Some(
            deferred(
                anchored().position(popup.at).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .id("context-actions")
                        .key_context(POPUP_CONTEXT)
                        .track_focus(&self.context_focus)
                        .on_action(cx.listener(Self::close_context_actions))
                        .on_action(cx.listener(|this, _: &SelectPrevContextAction, _, cx| this.step_context_action(-1, cx)))
                        .on_action(cx.listener(|this, _: &SelectNextContextAction, _, cx| this.step_context_action(1, cx)))
                        .on_action(cx.listener(|this, _: &ApplyContextAction, window, cx| {
                            let ix = this.context_actions.as_ref().map_or(0, |p| p.selected);
                            this.apply_context_action(ix, window, cx);
                        }))
                        .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                            // Surround With's numbers and letters choose an item.
                            let Some(key) = e.keystroke.key.chars().next().filter(|_| e.keystroke.key.chars().count() == 1) else { return };
                            let key = key.to_ascii_uppercase();
                            let ix = this.context_actions.as_ref().and_then(|p| p.items.iter().position(|(_, c)| matches!(c, Choice::Surround(_, k, _) if *k == key)));
                            if let Some(ix) = ix {
                                cx.stop_propagation();
                                this.apply_context_action(ix, window, cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if let Some(caret) = this.context_actions.take().and_then(|p| p.caret) {
                                this.state.update(cx, |state, cx| state.set_selected_range(caret, cx));
                            }
                            cx.notify();
                        }))
                        .min_w(px(220.))
                        .p_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_lg()
                        .when_some(popup.title, |el, title| {
                            el.child(div().px_2().pb_1().text_xs().text_color(palette.text_secondary).child(title))
                        })
                        .children(rows)
                        .child(div().h(px(0.))),
                ),
            )
            .with_priority(2),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets_from_positions() {
        let text = "ab\ncdé\nf";
        assert_eq!(super::offset_of(text, super::Pos::new(1, 2)), 5);
        assert_eq!(super::offset_of(text, super::Pos::new(1, 3)), 7);
        assert_eq!(super::offset_of(text, super::Pos::new(2, 1)), 9);
    }
}

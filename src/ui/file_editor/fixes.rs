//! Alt+Enter (Show Context Actions) in the editor: the quick fixes of the
//! problems at the caret, as IntelliJ's popup lists them ("Change to
//! 1.13.1"). Enter or a click applies one; Escape closes it.

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
    items: Vec<(std::ops::Range<usize>, Fix)>,
    selected: usize,
    at: Point<Pixels>,
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
        let mut items: Vec<(std::ops::Range<usize>, Fix)> = Vec::new();
        // The problems under the caret first, then the rest of its line.
        let mut problems = self.problems();
        problems.sort_by_key(|p| {
            let (s, e) = (offset_of(&text, p.start), offset_of(&text, p.end));
            !(s <= cursor && cursor <= e)
        });
        for p in problems.iter().filter(|p| p.start.line == cursor_line) {
            if let Some(fix) = &p.fix {
                items.push((offset_of(&text, p.start)..offset_of(&text, p.end), fix.clone()));
            }
        }
        if items.is_empty() {
            return;
        }
        let at = self.state.read(cx).range_to_bounds(&(cursor..cursor)).map_or(point(px(80.), px(80.)), |b| b.bottom_left() + point(px(0.), px(4.)));
        self.context_actions = Some(ContextActions { items, selected: 0, at });
        window.focus(&self.context_focus, cx);
        cx.notify();
    }

    fn close_context_actions(&mut self, _: &CloseContextActions, window: &mut Window, cx: &mut Context<Self>) {
        self.context_actions = None;
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
        let Some((range, fix)) = popup.items.get(ix).cloned() else { return };
        self.focus(window, cx);
        self.state.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(&fix.replacement, window, cx);
        });
        cx.notify();
    }

    pub(super) fn render_context_actions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let popup = self.context_actions.as_ref()?;
        let palette = cx.palette().clone();
        let rows = popup.items.iter().enumerate().map(|(ix, (_, fix))| {
            let selected = ix == popup.selected;
            h_flex()
                .id(("context-action", ix))
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .when(selected, |el| el.bg(palette.selection))
                .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                .child(Icon::new(icons::INTENTION_BULB).xsmall())
                .child(fix.title.clone())
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
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.context_actions = None;
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

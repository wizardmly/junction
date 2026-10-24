//! View › Parameter Info (Ctrl+P), as IntelliJ's: above the call around
//! the caret, each declaration's parameters (overloads one per line), the
//! one the caret's argument goes to bold, declarations that can't take
//! the arguments grayed; it follows the caret and closes when the caret
//! leaves the call or on Escape. It also pops up a moment after `(` is
//! typed after a name, as IntelliJ's autopopup does.

use gpui_kit::{
    AppContext as _, Context, FontWeight, IntoElement, ParentElement as _, Styled as _, Subscription, Window, anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};
use gpui_kit::component::{h_flex, v_flex};

use super::FileEditor;
use crate::domain::parameter_info::{self as pi, Call};
use crate::index::service::Nav;
use crate::theme::ActivePalette as _;

/// IntelliJ's "Autopopup in (ms)" for parameter info.
const AUTOPOPUP_MS: u64 = 1000;

pub(super) struct ParameterInfo {
    /// The call's `(`.
    open: usize,
    kotlin: bool,
    /// Each declaration's parameters.
    overloads: Vec<Vec<String>>,
    _observe: Subscription,
}

impl FileEditor {
    /// Ctrl+P: the parameters of the call at the caret.
    pub(super) fn show_parameter_info(&mut self, explicit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, style)) = self.jvm_style() else { return };
        let kotlin = style == crate::domain::imports::ImportStyle::Kotlin;
        let Some(index) = self.code_index.clone() else { return };
        let text = self.text(cx);
        let caret = self.state.read(cx).cursor();
        let Some(call) = pi::call_at(&text, caret, kotlin) else { return };
        let root = index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        // `this(`: this class's constructors; `super(`: its superclass's.
        let class = matches!(call.name.as_str(), "this" | "super").then(|| pi::enclosing_class(&text, call.name_at)).flatten();
        let local = (call.name == "this").then(|| class.map(|at| pi::declaration_parameters(&text, at))).flatten();
        let lookup = match call.name.as_str() {
            "this" => None,
            "super" => class.and_then(|at| pi::superclass(&text, at)),
            _ => Some(call.name_at),
        };
        let task = lookup.map(|at| index.read(cx).go_to(self.path.clone(), text.clone(), at, cx));
        self.parameter_task = Some(cx.spawn_in(window, async move |this, cx| {
            let targets = match task {
                Some(task) => match task.await {
                    Nav::Targets(targets) => targets,
                    Nav::Declaration => Vec::new(),
                },
                None => Vec::new(),
            };
            // Each declaration's parameters, from its source.
            let overloads = cx
                .background_spawn(async move {
                    let mut out: Vec<Vec<String>> = Vec::new();
                    for params in local.into_iter().flatten() {
                        if !out.contains(&params) {
                            out.push(params);
                        }
                    }
                    for t in targets {
                        let Ok(source) = std::fs::read_to_string(root.join(&t.path)) else { continue };
                        let Some(at) = offset_of(&source, t.line, t.col) else { continue };
                        // A class: its constructors (none written takes none).
                        for params in pi::declaration_parameters(&source, at) {
                            if !out.contains(&params) {
                                out.push(params);
                            }
                        }
                    }
                    out
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if overloads.is_empty() {
                    if explicit {
                        Self::hint("Parameter Info", "No parameters", window, cx);
                    }
                    return;
                }
                // Still in the same call.
                let text = this.text(cx);
                let caret = this.state.read(cx).cursor();
                if pi::call_from(&text, call.open, caret, kotlin).is_none_or(|c| c.open != call.open) {
                    return;
                }
                let _observe = cx.observe(&this.state, |this, _, cx| this.follow_parameter_info(cx));
                this.parameter_info = Some(ParameterInfo { open: call.open, kotlin, overloads, _observe });
                cx.notify();
            })
            .ok();
        }));
    }

    /// The caret moved or the text changed: closes once out of the call.
    fn follow_parameter_info(&mut self, cx: &mut Context<Self>) {
        let Some(info) = &self.parameter_info else { return };
        let (open, kotlin) = (info.open, info.kotlin);
        let state = self.state.read(cx);
        let caret = state.cursor();
        let text = state.value();
        let still = caret > open && text.as_bytes().get(open) == Some(&b'(') && pi::call_from(&text, open, caret, kotlin).is_some_and(|c| c.open == open);
        if !still {
            self.parameter_info = None;
        }
        cx.notify();
    }

    /// After a `(` typed after a name: Parameter Info a moment later, if
    /// the caret is still in that call.
    pub(super) fn parameter_info_autopopup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.parameter_info.is_some() || self.jvm_style().is_none() {
            return;
        }
        let kotlin = self.jvm_style().is_some_and(|(_, s)| s == crate::domain::imports::ImportStyle::Kotlin);
        // Detached: typing on mustn't cancel a popup a `(` scheduled.
        cx.spawn_in(window, async move |this, cx| {
            // The edit's caret is placed after the event.
            cx.background_executor().timer(std::time::Duration::from_millis(1)).await;
            this.update_in(cx, |this, window, cx| {
                let state = this.state.read(cx);
                let caret = state.cursor();
                let text = state.value();
                let typed_paren = caret > 0 && text.as_bytes().get(caret - 1) == Some(&b'(') && text[..caret - 1].trim_end().ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '>');
                if !typed_paren {
                    return;
                }
                let open = caret - 1;
                this.parameter_task = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(AUTOPOPUP_MS)).await;
                    this.update_in(cx, |this, window, cx| {
                        let state = this.state.read(cx);
                        let (caret, text) = (state.cursor(), state.value());
                        if this.parameter_info.is_none() && caret > open && pi::call_from(&text, open, caret, kotlin).is_some_and(|c| c.open == open) {
                            this.show_parameter_info(false, window, cx);
                        }
                    })
                    .ok();
                }));
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_parameter_info(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.parameter_info.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// The popup, above the call's line at its `(`.
    pub(super) fn render_parameter_info(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let info = self.parameter_info.as_ref()?;
        let state = self.state.read(cx);
        let caret = state.cursor();
        let text = state.value();
        let call: Call = pi::call_from(&text, info.open, caret, info.kotlin)?;
        let at = state.range_to_bounds(&(info.open..info.open))?;
        let palette = cx.palette().clone();
        // The overload the written arguments pick, highlighted.
        let best = pi::best_overload(&info.overloads, &pi::arguments(&text, info.open));
        let mut rows = v_flex().gap_0p5();
        for (ix, params) in info.overloads.iter().enumerate() {
            let fits = pi::fits(params, &call);
            let current = pi::current_parameter(params, &call).filter(|_| fits);
            let color = if fits { palette.text } else { palette.text_disabled };
            let mut row = h_flex().text_color(color).whitespace_nowrap().px_1().rounded_sm().when(best == Some(ix), |el| el.bg(palette.selection_inactive));
            if params.is_empty() {
                row = row.child("<no parameters>");
            }
            for (i, p) in params.iter().enumerate() {
                row = row.child(div().when(current == Some(i), |el| el.font_weight(FontWeight::BOLD)).child(p.clone()));
                if i + 1 < params.len() {
                    row = row.child(div().pr_1().child(","));
                }
            }
            rows = rows.child(row);
        }
        let height = px(8.) + px(20.) * info.overloads.len() as f32;
        Some(
            deferred(
                anchored().position(point(at.left() - px(8.), at.top() - height - px(2.))).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .px_1()
                        .py_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_md()
                        .child(rows),
                ),
            )
            .with_priority(2),
        )
    }
}

/// A 0-based line and UTF-16 column as a byte offset.
pub(super) fn offset_of(text: &str, line: u32, col: u32) -> Option<usize> {
    let start: usize = text.split_inclusive('\n').take(line as usize).map(str::len).sum();
    let line_text = text.get(start..)?.split('\n').next()?;
    let mut units = 0;
    for (i, ch) in line_text.char_indices() {
        if units >= col {
            return Some(start + i);
        }
        units += ch.len_utf16() as u32;
    }
    Some(start + line_text.len())
}

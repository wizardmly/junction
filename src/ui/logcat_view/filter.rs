//! Logcat's filter field, as Android Studio's: completion while a key or a
//! value is typed (Ctrl+Space asks for it), the history button with the
//! favorites and the latest filters and how many entries each lets
//! through, and the star that makes the filter a favorite.

use std::collections::BTreeSet;

use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, FontWeight, SharedString, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div,
    point, prelude::FluentBuilder as _, px,
};

use super::LogcatView;
use crate::domain::logcat::{self, Filter};
use crate::domain::logcat_filter::{self, Completion, Known};
use crate::infra::logcat_filter_store;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;

pub(super) struct CompletionPopup {
    completion: Completion,
    selected: usize,
    scroll: gpui_kit::ScrollHandle,
}

impl LogcatView {
    /// The tags, packages and processes the log has shown.
    fn known_values(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let (mut tags, mut packages, mut processes) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for e in self.entries.iter().filter(|e| !e.system) {
            if !e.tag.is_empty() {
                tags.insert(e.tag.as_str());
            }
            if e.package.starts_with("pid-") {
                continue;
            }
            processes.insert(e.package.as_str());
            if e.package.contains('.') && !e.package.contains([':', '/']) {
                packages.insert(e.package.as_str());
            }
        }
        let owned = |set: BTreeSet<&str>| set.into_iter().map(str::to_owned).collect();
        (owned(tags), owned(packages), owned(processes))
    }

    /// The completion for the caret's term; `invoked` by Ctrl+Space.
    pub(super) fn update_completion(&mut self, invoked: bool, cx: &mut Context<Self>) {
        let (text, caret) = {
            let state = self.query.read(cx);
            (state.value().to_string(), state.cursor())
        };
        let (tags, packages, processes) = self.known_values();
        let known = Known { tags: &tags, packages: &packages, processes: &processes };
        self.completion = logcat_filter::complete(&text, caret, &known, invoked).map(|completion| CompletionPopup { completion, selected: 0, scroll: gpui_kit::ScrollHandle::new() });
        cx.notify();
    }

    /// The query changed: refilter, and complete what's being typed.
    pub(super) fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.refilter(cx);
        if std::mem::take(&mut self.skip_completion) {
            self.completion = None;
        } else {
            self.update_completion(false, cx);
        }
    }

    /// Writes the chosen item into the query; a key goes on to its values.
    fn accept_completion(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.completion.take() else { return };
        let Some(item) = popup.completion.items.get(ix.unwrap_or(popup.selected)).cloned() else { return };
        self.skip_completion = !item.ends_with(':');
        let range = popup.completion.replace.clone();
        self.query.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(item, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn step_completion(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popup) = self.completion.as_mut() {
            let n = popup.completion.items.len() as isize;
            popup.selected = (popup.selected as isize + delta).rem_euclid(n) as usize;
            popup.scroll.scroll_to_item(popup.selected);
            cx.notify();
        }
    }

    /// The filter was applied (Enter, leaving the field): into the history.
    pub(super) fn remember_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        let before = self.history.clone();
        self.history.remember(&query);
        if self.history != before {
            logcat_filter_store::save(&self.history);
        }
    }

    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.history.toggle_favorite(&query);
        logcat_filter_store::save(&self.history);
        cx.notify();
    }

    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.completion = None;
        self.history_open = !self.history_open;
        if self.history_open {
            self.remember_filter(cx);
        }
        cx.notify();
    }

    /// A filter picked in the history popup.
    fn use_filter(&mut self, filter: String, window: &mut Window, cx: &mut Context<Self>) {
        self.history_open = false;
        self.query.update(cx, |state, cx| {
            state.set_value(filter, window, cx);
            state.focus(window, cx);
        });
        self.remember_filter(cx);
        self.refilter(cx);
    }

    fn forget_filter(&mut self, filter: &str, cx: &mut Context<Self>) {
        self.history.remove(filter);
        logcat_filter_store::save(&self.history);
        cx.notify();
    }

    /// How many entries a filter lets through, as the history shows.
    fn count(&self, filter: &str) -> usize {
        let filter = Filter::parse(filter, &self.mine);
        let now = self.entries.back().map_or(0, |e| e.millis);
        self.entries.iter().filter(|e| !e.system && filter.matches(e, now)).count()
    }

    /// The query field with its history button and star, and the popups.
    pub(super) fn render_filter(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let palette = cx.palette().clone();
        let query = self.query.read(cx).value().to_string();
        let favorite = self.history.is_favorite(&query);
        let open = self.completion.is_some();
        let history_button = tool_button("logcat-history", icons::SEARCH_HISTORY, "Show history").on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx)));
        let star = tool_button("logcat-favorite", if favorite { icons::STAR } else { icons::STAR_EMPTY }, if favorite { "Remove from favorites" } else { "Add to favorites" })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx)));
        let field = div()
            .flex_1()
            .min_w(px(120.))
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                let k = &e.keystroke;
                if k.key == "space" && k.modifiers.control {
                    cx.stop_propagation();
                    this.history_open = false;
                    this.update_completion(true, cx);
                }
            }))
            .when(open, |el| {
                el.capture_action(cx.listener(|this, _: &gpui_kit::component::input::MoveUp, _, cx| {
                    cx.stop_propagation();
                    this.step_completion(-1, cx);
                }))
                .capture_action(cx.listener(|this, _: &gpui_kit::component::input::MoveDown, _, cx| {
                    cx.stop_propagation();
                    this.step_completion(1, cx);
                }))
                .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Enter, window, cx| {
                    cx.stop_propagation();
                    this.accept_completion(None, window, cx);
                }))
                .capture_action(cx.listener(|this, _: &gpui_kit::component::input::IndentInline, window, cx| {
                    cx.stop_propagation();
                    this.accept_completion(None, window, cx);
                }))
                .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| {
                    cx.stop_propagation();
                    this.completion = None;
                    cx.notify();
                }))
            })
            .child(gpui_kit::component::input::Input::new(&self.query).small().cleanable(true).prefix(history_button).suffix(star));
        let bounds = self.query.read(cx).input_bounds();
        // The history under the field; a completion under its term, as
        // IntelliJ's lookup.
        let term_x = self.completion.as_ref().and_then(|c| self.query.read(cx).range_to_bounds(&(c.completion.replace.start..c.completion.replace.start))).map(|b| b.left() - px(9.));
        let at = point(term_x.unwrap_or(bounds.left() - px(28.)), bounds.bottom() + px(6.));
        let popup = |id: &'static str| {
            v_flex()
                .id(id)
                .occlude()
                .min_w(px(260.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .py_1()
                .text_sm()
                .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                .border_1()
                .border_color(palette.border)
                .rounded_md()
                .shadow_lg()
        };
        let mut overlay: Option<AnyElement> = None;
        if let Some(c) = &self.completion {
            let typed_len = c.completion.replace.len();
            let mut list = popup("logcat-completion").track_scroll(&c.scroll);
            for (ix, item) in c.completion.items.iter().enumerate() {
                let selected = ix == c.selected;
                // The typed part in bold, as IntelliJ's lookup shows it.
                let typed = query.get(c.completion.replace.clone()).unwrap_or("").trim_start_matches('"').to_lowercase();
                let (bold, rest) = if typed_len > 0 && item.to_lowercase().trim_start_matches('"').starts_with(&typed) {
                    let n = typed.len() + usize::from(item.starts_with('"'));
                    item.split_at(n.min(item.len()))
                } else {
                    ("", item.as_str())
                };
                list = list.child(
                    h_flex()
                        .id(("logcat-completion-item", ix))
                        .px_2()
                        .h(px(22.))
                        .when(selected, |el| el.bg(palette.selection))
                        .hover(|el| el.bg(palette.hover))
                        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.accept_completion(Some(ix), window, cx);
                        }))
                        .child(div().font_weight(gpui_kit::FontWeight::BOLD).text_color(palette.link).child(bold.to_owned()))
                        .child(rest.to_owned()),
                );
            }
            overlay = Some(list.into_any_element());
        } else if self.history_open {
            let mut list = popup("logcat-history-popup").on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.history_open = false;
                cx.notify();
            }));
            let section = |title: &'static str| div().px_2().pt_1().text_xs().text_color(palette.text_secondary).child(title);
            // A named filter shows its name in bold, the query as its
            // tooltip; after it when another filter has the same name.
            let names: Vec<Option<String>> = self.history.favorites.iter().chain(&self.history.recent).map(|f| logcat::filter_name(f)).collect();
            let label = |ix: usize, filter: &String| {
                let el = div().id(("logcat-history-label", ix)).flex_1().whitespace_nowrap();
                let Some(name) = logcat::filter_name(filter) else { return el.child(filter.clone()) };
                let rest: SharedString = logcat::remove_filter_names(filter).into();
                if names.iter().filter(|n| n.as_deref() == Some(name.as_str())).count() > 1 {
                    el.child(h_flex().child(div().font_weight(FontWeight::BOLD).child(name)).child(format!(": {rest}")))
                } else {
                    el.child(div().font_weight(FontWeight::BOLD).child(name)).tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(rest.clone()).build(w, cx))
                }
            };
            let row = |ix: usize, filter: &String, starred: bool, cx: &mut Context<Self>| {
                let (pick, forget) = (filter.clone(), filter.clone());
                h_flex()
                    .id(("logcat-history-item", ix))
                    .px_2()
                    .h(px(24.))
                    .gap_2()
                    .hover(|el| el.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, window, cx| this.use_filter(pick.clone(), window, cx)))
                    .child(Icon::new(if starred { icons::STAR } else { icons::HISTORY }).xsmall())
                    .child(label(ix, filter))
                    .child(div().text_color(palette.text_secondary).child(format!("({})", self.count(filter))))
                    .child(tool_button(("logcat-history-remove", ix), icons::CLOSE_SMALL, "Remove from history").on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.forget_filter(&forget, cx);
                    })))
            };
            if self.history.favorites.is_empty() && self.history.recent.is_empty() {
                list = list.child(div().px_2().text_color(palette.text_secondary).child("No filters yet"));
            }
            if !self.history.favorites.is_empty() {
                list = list.child(section("Favorites"));
                for (ix, f) in self.history.favorites.iter().enumerate() {
                    list = list.child(row(ix, f, true, cx));
                }
            }
            if !self.history.recent.is_empty() {
                list = list.child(section("History"));
                for (ix, f) in self.history.recent.iter().enumerate() {
                    list = list.child(row(1000 + ix, f, false, cx));
                }
            }
            overlay = Some(list.into_any_element());
        }
        let _ = window;
        div()
            .flex_1()
            .min_w(px(120.))
            .child(field)
            .when_some(overlay, |el, overlay| el.child(deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(overlay)).with_priority(2)))
    }
}

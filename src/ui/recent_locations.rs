//! Recent Locations (Ctrl+Shift+E), as IntelliJ's popup: the places
//! visited and changed lately, newest first, each with its file, the
//! function or class it is in, and a few lines of code around it. Typing
//! filters by file name and code; "Show changed only" (or Ctrl+Shift+E
//! again) keeps the places edited.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Sizable as _, WindowExt as _, checkbox::Checkbox, h_flex, scroll::ScrollableElement as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::index::call_hierarchy::caller_at;
use crate::index::lang::Lang;
use crate::index::nav::Target;
use crate::theme::ActivePalette as _;
use crate::ui::common;

/// One place, ready to show.
pub struct Location {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub changed: bool,
    /// The function or class around it: "Cart.total".
    pub crumb: String,
    /// The lines around it, their common indent removed, and the first one's
    /// number.
    pub snippet: String,
    pub first_line: u32,
    pub styles: Vec<(Range<usize>, HighlightStyle)>,
}

impl Location {
    /// A place in a file's text, with its snippet colored.
    pub fn new(path: String, line: u32, col: u32, changed: bool, text: &str, dark: bool, highlighters: &mut HashMap<&'static str, gpui_kit::component::highlighter::SyntaxHighlighter>) -> Self {
        let lines: Vec<&str> = text.lines().collect();
        let at = (line as usize).min(lines.len().saturating_sub(1));
        let (from, to) = (at.saturating_sub(1), (at + 2).min(lines.len()));
        let chunk = lines.get(from..to).unwrap_or_default();
        let indent = chunk.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
        let snippet = chunk.iter().map(|l| l.get(indent..).unwrap_or(l.trim_start())).collect::<Vec<_>>().join("\n");
        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        let crumb = Lang::from_path(&path)
            .and_then(|lang| {
                let offset = crate::index::call_hierarchy::offset_of(text, line, col);
                caller_at(lang, text, offset, &file_name)
            })
            .map(|c| c.name)
            .filter(|n| *n != file_name)
            .unwrap_or_default();
        let language = crate::ui::file_editor::language_for(&path);
        let highlighter = highlighters.entry(language).or_insert_with(|| gpui_kit::component::highlighter::SyntaxHighlighter::new(language));
        highlighter.update(None, &gpui_kit::component::Rope::from_str(&snippet), None);
        let styles = highlighter.styles(&(0..snippet.len()), &crate::theme::editor_colors(dark));
        Self { path, line, col, changed, crumb, snippet, first_line: from as u32, styles }
    }

    fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

pub struct RecentLocations {
    locations: Vec<Location>,
    shown: Vec<usize>,
    changed_only: bool,
    input: Entity<InputState>,
    selected: usize,
    scroll: ScrollHandle,
    focus: FocusHandle,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    _subscriptions: Vec<Subscription>,
}

impl RecentLocations {
    pub fn open(locations: Vec<Location>, on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>, window: &mut Window, cx: &mut App) {
        let view = cx.new(|cx| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter by file name or code"));
            let sub = cx.subscribe_in(&input, window, |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.filter(cx);
                }
            });
            let mut this = Self {
                locations,
                shown: Vec::new(),
                changed_only: false,
                input,
                selected: 0,
                scroll: ScrollHandle::new(),
                focus: cx.focus_handle(),
                on_pick,
                _subscriptions: vec![sub],
            };
            this.filter(cx);
            // IntelliJ preselects the place before the current one.
            if this.shown.len() > 1 {
                this.selected = 1;
            }
            this
        });
        let (confirm, shown) = (view.clone(), view.clone());
        window.open_dialog(cx, move |dialog, _, cx| {
            let confirm = confirm.clone();
            let changed_only = shown.read(cx).changed_only;
            dialog.title(if changed_only { "Recently Changed Locations" } else { "Recent Locations" }).w(px(640.)).child(shown.clone()).on_ok(move |_, window, cx| {
                confirm.update(cx, |this, cx| this.pick(this.selected, window, cx));
                false
            })
        });
        let input = view.read(cx).input.clone();
        crate::ui::dialogs::focus_input(&input, window, cx);
    }

    /// Ctrl+Shift+E again, or the checkbox.
    pub fn toggle_changed(&mut self, cx: &mut Context<Self>) {
        self.changed_only = !self.changed_only;
        self.filter(cx);
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_lowercase();
        self.shown = (0..self.locations.len())
            .filter(|&i| {
                let l = &self.locations[i];
                (!self.changed_only || l.changed)
                    && (query.is_empty() || l.file_name().to_lowercase().contains(&query) || l.snippet.to_lowercase().contains(&query) || l.crumb.to_lowercase().contains(&query))
            })
            .collect();
        self.selected = 0;
        cx.notify();
    }

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.shown.len().max(1) as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn pick(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(l) = self.shown.get(row).and_then(|i| self.locations.get(*i)) else { return };
        let target = Target { path: l.path.clone(), line: l.line, col: l.col, name: l.file_name().to_owned(), label: String::new(), container: None };
        window.close_dialog(cx);
        (self.on_pick)(target, window, cx);
    }
}

impl Render for RecentLocations {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone();
        let mut rows = Vec::new();
        for (row, &i) in self.shown.iter().enumerate() {
            let l = &self.locations[i];
            let selected = row == self.selected;
            let numbers = (0..l.snippet.lines().count().max(1)).map(|k| (l.first_line as usize + k + 1).to_string()).collect::<Vec<_>>().join("\n");
            rows.push(
                v_flex()
                    .id(("recent-location", row))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(palette.border)
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_sm()
                            .child(common::icon(common::file_icon(&l.path)))
                            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(l.file_name().to_owned()))
                            .when(!l.crumb.is_empty(), |el| el.child(div().text_color(palette.text_secondary).child(l.crumb.clone())))
                            .when(l.changed, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("changed"))),
                    )
                    .child(
                        h_flex()
                            .items_start()
                            .gap_3()
                            .font_family(mono.clone())
                            .text_xs()
                            .child(div().flex_none().text_right().min_w(px(24.)).text_color(palette.text_secondary).child(numbers))
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(StyledText::new(SharedString::from(l.snippet.clone())).with_highlights(l.styles.clone()))),
                    )
                    .on_click(cx.listener(move |this, e: &gpui_kit::ClickEvent, window, cx| {
                        this.selected = row;
                        let _ = e;
                        this.pick(row, window, cx);
                    })),
            );
        }
        v_flex()
            .key_context("RecentLocations")
            .track_focus(&self.focus)
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                let k = &event.keystroke;
                match k.key.as_str() {
                    "down" => this.select(1, cx),
                    "up" => this.select(-1, cx),
                    "e" if k.modifiers.secondary() && k.modifiers.shift => this.toggle_changed(cx),
                    _ => {}
                }
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.input).small()))
                    .child(
                        Checkbox::new("recent-changed-only")
                            .label("Show changed only")
                            .checked(self.changed_only)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_changed(cx))),
                    ),
            )
            .child(
                div()
                    .relative()
                    .when(self.shown.is_empty(), |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child(if self.changed_only { "No changed locations" } else { "No recent locations" })))
                    .child(v_flex().id("recent-locations").max_h(px(480.)).overflow_y_scroll().track_scroll(&self.scroll).children(rows))
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

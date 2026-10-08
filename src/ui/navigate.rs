//! Code navigation UI: the "Choose Declaration" list, Recent Files and
//! File Structure pickers.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    WindowExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::index::nav::Target;
use crate::theme::ActivePalette as _;
use crate::ui::common::{self};

/// Something to open: a path and position.
#[derive(Clone)]
pub struct OpenTarget(pub Target);

pub fn symbol_icon(kind: crate::index::symbols::SymbolKind) -> IconName {
    use crate::index::symbols::SymbolKind as K;
    match kind {
        K::Class | K::Struct | K::Interface | K::Protocol | K::Trait | K::Extension => IconName::Layers,
        K::Enum | K::EnumMember => IconName::Rows3,
        K::Function | K::Method | K::Constructor => IconName::CircleDot,
        K::Field | K::Property | K::Variable | K::Constant => IconName::Circle,
        K::Module | K::Namespace => IconName::FolderClosed,
        K::TypeAlias => IconName::Hash,
        K::Macro => IconName::Hash,
    }
}

/// "Choose Declaration": several targets for one name.
pub fn choose_target(targets: Vec<Target>, on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>, window: &mut Window, cx: &mut App) {
    let targets = Rc::new(targets);
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for (ix, t) in targets.iter().enumerate() {
            let (t2, on_pick) = (t.clone(), on_pick.clone());
            let name = if t.name.is_empty() { t.path.rsplit('/').next().unwrap_or(&t.path).to_owned() } else { t.name.clone() };
            list = list.child(
                h_flex()
                    .id(("choose", ix))
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|el| el.bg(palette.hover))
                    .child(common::icon(common::file_icon(&t.path)).text_color(palette.text_secondary))
                    .child(div().text_sm().child(match &t.container {
                        Some(c) => format!("{c}.{name}"),
                        None => name,
                    }))
                    .child(div().text_xs().text_color(palette.text_secondary).child(t.label.clone()))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(palette.text_secondary).child(format!("{}:{}", t.path, t.line + 1)))
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        on_pick(t2.clone(), window, cx);
                    }),
            );
        }
        dialog.title("Choose Declaration").w(px(640.)).child(div().id("choose-list").max_h(px(420.)).overflow_y_scrollbar().child(list))
    });
}

/// A filterable list popup: Recent Files (Ctrl+E), File Structure (Ctrl+F12).
pub struct ListPicker {
    items: Vec<crate::ui::find_view::FoundItem>,
    shown: Vec<usize>,
    input: Entity<InputState>,
    selected: usize,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    /// Indent per item (File Structure nesting).
    indents: Vec<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ListPicker {
    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_owned();
        let mut scored: Vec<(usize, i32)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| crate::index::nav::fuzzy_score(&query, &it.title).map(|s| (i, s)))
            .collect();
        if !query.is_empty() {
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        }
        self.shown = scored.into_iter().map(|(i, _)| i).collect();
        self.selected = 0;
        cx.notify();
    }

    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.shown.get(ix).and_then(|i| self.items.get(*i)) else { return };
        let target = item.target.clone();
        window.close_dialog(cx);
        (self.on_pick)(target, window, cx);
    }
}

impl Render for ListPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex();
        for (row, &i) in self.shown.iter().enumerate() {
            let item = &self.items[i];
            let indent = if self.input.read(cx).value().is_empty() { self.indents.get(i).copied().unwrap_or(0) } else { 0 };
            list = list.child(
                h_flex()
                    .id(("pick", row))
                    .h(px(26.))
                    .pl(px(8. + 16. * indent as f32))
                    .pr_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(row == self.selected, |el| el.bg(palette.selection))
                    .hover(|el| el.bg(palette.hover))
                    .child(common::icon(item.icon).text_color(palette.text_secondary))
                    .child(div().text_sm().whitespace_nowrap().child(item.title.clone()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(item.detail.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(row, window, cx))),
            );
        }
        v_flex()
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                let n = this.shown.len().max(1);
                match event.keystroke.key.as_str() {
                    "down" => this.selected = (this.selected + 1) % n,
                    "up" => this.selected = (this.selected + n - 1) % n,
                    _ => return,
                }
                cx.notify();
            }))
            .child(Input::new(&self.input))
            .child(
                div()
                    .id("pick-list")
                    .max_h(px(420.))
                    .overflow_y_scrollbar()
                    .when(self.shown.is_empty(), |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child("Nothing found")))
                    .child(list),
            )
    }
}

/// Opens a ListPicker dialog.
pub fn pick_from_list(
    title: &'static str,
    items: Vec<crate::ui::find_view::FoundItem>,
    indents: Vec<usize>,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter"));
        let subscriptions = vec![cx.subscribe_in(&input, window, |this: &mut ListPicker, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.filter(cx),
            InputEvent::PressEnter { .. } => this.pick(this.selected, window, cx),
            _ => {}
        })];
        let shown = (0..items.len()).collect();
        ListPicker { items, shown, input, selected: 0, on_pick, indents, _subscriptions: subscriptions }
    });
    let focus = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| dialog.title(title).w(px(560.)).child(view.clone()));
    crate::ui::dialogs::focus_input(&focus, window, cx);
}

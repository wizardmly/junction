//! Code navigation UI: the "Choose Declaration" and Show Usages lists,
//! Recent Files and File Structure pickers.

use std::rc::Rc;
use crate::ui::as_icons as icons;

use gpui_kit::component::{
    WindowExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::index::nav::Target;
use crate::theme::ActivePalette as _;
use gpui_kit::component::Icon;
use crate::ui::common::{self};
use crate::ui::find_view::FoundItem;

/// Something to open: a path and position.
#[derive(Clone)]
pub struct OpenTarget(pub Target);

pub fn symbol_icon(kind: crate::index::symbols::SymbolKind) -> Icon {
    use crate::index::symbols::SymbolKind as K;
    // Android Studio's structure icons.
    match kind {
        K::Class | K::Struct | K::Extension => icons::CLASS,
        K::Interface | K::Protocol | K::Trait => icons::INTERFACE,
        K::Enum => icons::ENUM,
        K::Function | K::Macro => icons::FUNCTION,
        K::Method | K::Constructor => icons::METHOD,
        K::Field | K::EnumMember => icons::FIELD,
        K::Property => icons::PROPERTY,
        K::Constant => icons::CONSTANT,
        K::Variable => icons::VARIABLE,
        K::Module | K::Namespace => icons::PACKAGE,
        K::TypeAlias => icons::TYPE_ALIAS,
    }
    .into()
}

/// "Choose Declaration": several targets for one name. `locations` is what
/// each row shows on the right (file and line, or library and path).
pub fn choose_target(
    title: &'static str,
    targets: Vec<Target>,
    locations: Vec<String>,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let items: Vec<FoundItem> = targets
        .into_iter()
        .map(|t| {
            let name = if t.name.is_empty() { t.path.rsplit('/').next().unwrap_or(&t.path).to_owned() } else { t.name.clone() };
            FoundItem {
                title: match &t.container {
                    Some(c) => format!("{c}.{name}"),
                    None => name,
                },
                detail: t.label.clone(),
                icon: common::file_icon(&t.path),
                target: t,
            }
        })
        .collect();
    let n = items.len();
    open_picker(title, items, vec![0; n], locations, false, 640., on_pick, window, cx);
}

/// A list popup: Choose Declaration, Show Usages, Recent Files (Ctrl+E),
/// File Structure (Ctrl+F12). ↑↓ select, Enter opens.
pub struct ListPicker {
    items: Vec<FoundItem>,
    shown: Vec<usize>,
    /// The filter field; none for Choose Declaration, which takes the keys itself.
    input: Option<Entity<InputState>>,
    focus: FocusHandle,
    selected: usize,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    /// Indent per item (File Structure nesting).
    indents: Vec<usize>,
    /// Right-aligned text per item (a location).
    locations: Vec<String>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ListPicker {
    fn query(&self, cx: &App) -> String {
        self.input.as_ref().map(|i| i.read(cx).value().trim().to_owned()).unwrap_or_default()
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
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

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.shown.len().max(1) as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        self.scroll.scroll_to_item(self.selected);
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
        let filtered = !self.query(cx).is_empty();
        let mut rows = Vec::new();
        for (row, &i) in self.shown.iter().enumerate() {
            let item = &self.items[i];
            let indent = if filtered { 0 } else { self.indents.get(i).copied().unwrap_or(0) };
            let location = self.locations.get(i).cloned().unwrap_or_default();
            rows.push(
                h_flex()
                    .id(("pick", row))
                    .flex_none()
                    .h(px(26.))
                    .pl(px(8. + 16. * indent as f32))
                    .pr_2()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(row == self.selected, |el| el.bg(palette.selection))
                    .when(row != self.selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .child(common::icon(item.icon.clone()).text_color(palette.text_secondary))
                    .child(div().text_sm().whitespace_nowrap().child(item.title.clone()))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(item.detail.clone()))
                    .when(!location.is_empty(), |el| {
                        el.child(div().flex_shrink(1.).min_w_0().max_w(px(360.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(location))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(row, window, cx))),
            );
        }
        v_flex()
            .track_focus(&self.focus)
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "down" => this.select(1, cx),
                "up" => this.select(-1, cx),
                "pagedown" => this.select(10.min(this.shown.len().saturating_sub(1 + this.selected)) as isize, cx),
                "pageup" => this.select(-(10.min(this.selected) as isize), cx),
                _ => {}
            }))
            .when_some(self.input.as_ref(), |el, input| el.child(Input::new(input)))
            .child(
                div()
                    .relative()
                    .when(self.shown.is_empty(), |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child("Nothing found")))
                    .child(v_flex().id("pick-list").max_h(px(420.)).overflow_y_scroll().track_scroll(&self.scroll).children(rows))
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

/// Opens a filterable ListPicker dialog.
pub fn pick_from_list(
    title: &'static str,
    items: Vec<FoundItem>,
    indents: Vec<usize>,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    open_picker(title, items, indents, Vec::new(), true, 560., on_pick, window, cx);
}

#[allow(clippy::too_many_arguments)]
fn open_picker(
    title: &'static str,
    items: Vec<FoundItem>,
    indents: Vec<usize>,
    locations: Vec<String>,
    filterable: bool,
    width: f32,
    on_pick: Rc<dyn Fn(Target, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let mut subscriptions = Vec::new();
        let input = filterable.then(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type to filter"));
            subscriptions.push(cx.subscribe_in(&input, window, |this: &mut ListPicker, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.filter(cx)
                }
            }));
            input
        });
        let shown = (0..items.len()).collect();
        ListPicker { items, shown, input, focus: cx.focus_handle(), selected: 0, on_pick, indents, locations, scroll: ScrollHandle::new(), _subscriptions: subscriptions }
    });
    // Enter reaches the dialog as its Confirm: open the selected row (the
    // pick closes the dialog itself, before the target opens and takes focus).
    let (confirm, shown) = (view.clone(), view.clone());
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = confirm.clone();
        dialog.title(title).w(px(width)).child(shown.clone()).on_ok(move |_, window, cx| {
            confirm.update(cx, |this, cx| this.pick(this.selected, window, cx));
            false
        })
    });
    let (input, focus) = { let v = view.read(cx); (v.input.clone(), v.focus.clone()) };
    match input {
        Some(input) => crate::ui::dialogs::focus_input(&input, window, cx),
        None => window.defer(cx, move |window, cx| window.focus(&focus, cx)),
    }
}

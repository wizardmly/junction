//! A long branch list inside a popup menu (the Log's Branch filter): only
//! the rows in view are drawn, so moving the mouse over a few hundred
//! remote branches stays as smooth as over a short menu.

use std::ops::Range;
use crate::ui::as_icons as icons;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, SharedString, UniformListScrollHandle, Window, div, px, uniform_list};

/// Menus longer than this get the virtual list.
pub const LONG: usize = 30;
const ROW: f32 = 26.;
const MAX_HEIGHT: f32 = 420.;

pub struct RefRow {
    pub label: SharedString,
    pub value: String,
    pub favorite: bool,
    pub checked: bool,
}

type Pick = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub struct RefList {
    rows: Rc<Vec<RefRow>>,
    pick: Pick,
    scroll: UniformListScrollHandle,
    width: f32,
}

impl RefList {
    fn render_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let theme = cx.theme();
        let (accent, accent_text) = (theme.tokens.accent, theme.accent_foreground);
        range
            .map(|ix| {
                let row = &self.rows[ix];
                let (pick, value) = (self.pick.clone(), row.value.clone());
                div()
                    .id(ix)
                    .h(px(ROW))
                    .w_full()
                    .px_2()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap_x_1()
                    .text_sm()
                    .hover(|s| s.bg(accent).text_color(accent_text))
                    .child(div().w(px(16.)).flex_none().children(row.favorite.then(|| Icon::new(icons::STAR).xsmall())))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(row.label.clone()))
                    .children(row.checked.then(|| Icon::new(icons::CHECKED).xsmall()))
                    .on_click(move |_, window, cx| {
                        pick(value.clone(), window, cx);
                        // Closes the menu and its parent, as a picked item does.
                        window.dispatch_action(Box::new(gpui_kit::component::dialog::Cancel), cx);
                    })
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for RefList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (self.rows.len() as f32 * ROW).min(MAX_HEIGHT);
        div()
            .w(px(self.width))
            .h(px(height))
            .text_color(cx.theme().foreground)
            .child(uniform_list("ref-list", self.rows.len(), cx.processor(Self::render_rows)).track_scroll(&self.scroll).size_full())
    }
}

/// Adds `rows` to `menu`: as menu items when few, else as one virtual list.
pub fn add_rows(mut menu: PopupMenu, rows: Vec<RefRow>, pick: Pick, cx: &mut Context<PopupMenu>) -> PopupMenu {
    if rows.len() <= LONG {
        for row in rows {
            let (pick, value) = (pick.clone(), row.value.clone());
            let item = PopupMenuItem::new(row.label).checked(row.checked).on_click(move |_, window, cx| pick(value.clone(), window, cx));
            menu = menu.item(if row.favorite { item.icon(icons::STAR) } else { item });
        }
        return menu;
    }
    let longest = rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0);
    let width = (longest as f32 * 7.5 + 64.).clamp(180., 460.);
    let list: Entity<RefList> = cx.new(|_| RefList { rows: Rc::new(rows), pick, scroll: UniformListScrollHandle::new(), width });
    // Disabled: the item itself takes no hover highlight; its rows do.
    menu.item(PopupMenuItem::element(move |_, _| list.clone()).disabled(true))
}

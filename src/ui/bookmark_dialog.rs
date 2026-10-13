//! Add Mnemonic Bookmark (Ctrl+F11): IntelliJ's chooser of 1–9, 0 and A–Z.
//! A mnemonic another bookmark has is marked; picking it moves it here.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, ElementId, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::bookmarks::{Bookmarks, MNEMONICS};
use crate::theme::ActivePalette as _;

/// `line`: `None` for a file bookmark.
pub fn choose_mnemonic(file: String, line: Option<u32>, text: String, window: &mut Window, cx: &mut App) {
    let current = Bookmarks::get(cx).at(&file, line).cloned();
    let editing = current.as_ref().is_some_and(|b| b.mnemonic.is_some());
    let picked = Rc::new(Cell::new(current.as_ref().and_then(|b| b.mnemonic)));
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let used: Vec<char> = Bookmarks::get(cx).list.iter().filter(|b| !(b.file == file && b.place() == line)).filter_map(|b| b.mnemonic).collect();
        let cell = |c: char| {
            let selected = picked.get() == Some(c);
            let taken = used.contains(&c);
            let pick = picked.clone();
            div()
                .id(ElementId::NamedInteger("mnemonic".into(), c as u64))
                .w(px(28.))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if selected { palette.accent } else { palette.border })
                .when(selected, |el| el.bg(palette.selection))
                .when(taken && !selected, |el| el.bg(palette.hover))
                .text_color(if taken && !selected { palette.text_secondary } else { palette.text })
                .cursor_pointer()
                .child(c.to_string())
                .on_click(move |_, window, _| {
                    pick.set(if pick.get() == Some(c) { None } else { Some(c) });
                    window.refresh();
                })
        };
        let row = |chars: &str| h_flex().gap_1().children(chars.chars().map(&cell));
        let body = v_flex()
            .gap_1()
            .child(row(&MNEMONICS[..10]))
            .child(row(&MNEMONICS[10..20]))
            .child(row(&MNEMONICS[20..30]))
            .child(row(&MNEMONICS[30..]))
            .child(div().pt_2().text_xs().text_color(palette.text_secondary).child("Marked mnemonics belong to other bookmarks; choosing one moves it here."));
        let (file, text, pick) = (file.clone(), text.clone(), picked.clone());
        dialog
            .title(if editing { "Edit Mnemonic" } else { "Add Mnemonic Bookmark" })
            .w(px(360.))
            .child(body)
            .on_ok(move |_, _, cx| {
                let mnemonic = pick.get();
                Bookmarks::update(cx, |b| b.set(&file, line, &text, mnemonic));
                true
            })
            .footer(crate::ui::dialogs::footer("OK"))
    });
}

//! Generate's member chooser, as IntelliJ's: the class's properties or
//! fields with their types, all checked, Select All / Select None; OK
//! generates with the checked ones.

use std::rc::Rc;

use gpui_kit::component::{Sizable as _, WindowExt as _, button::{Button, ButtonVariants as _}, checkbox::Checkbox, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::domain::generate::Field;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

pub struct ChooserView {
    fields: Vec<Field>,
    checked: Vec<bool>,
}

impl Render for ChooserView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("generate-fields").max_h(px(280.)).min_h(px(60.)).overflow_y_scroll().p_1().border_1().border_color(palette.border).rounded_md();
        for (ix, f) in self.fields.iter().enumerate() {
            list = list.child(
                h_flex()
                    .gap_2()
                    .child(Checkbox::new(SharedString::from(format!("generate-{ix}"))).checked(self.checked[ix]).on_change(cx.listener(move |this, v: &bool, _, cx| {
                        this.checked[ix] = *v;
                        cx.notify();
                    })))
                    .child(div().text_sm().child(f.name.clone()))
                    .when(!f.ty.is_empty(), |el| el.child(div().text_sm().text_color(palette.text_secondary).child(f.ty.clone()))),
            );
        }
        v_flex()
            .gap_2()
            .child(list)
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("generate-all").label("Select All").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                        this.checked.iter_mut().for_each(|c| *c = true);
                        cx.notify();
                    })))
                    .child(Button::new("generate-none").label("Select None").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                        this.checked.iter_mut().for_each(|c| *c = false);
                        cx.notify();
                    }))),
            )
    }
}

/// Opens the chooser; OK hands the checked fields to `on_ok`.
pub fn choose(title: &'static str, fields: Vec<Field>, on_ok: impl Fn(Vec<Field>, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let checked = vec![true; fields.len()];
    let view = cx.new(|_| ChooserView { fields, checked });
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title(title).w(px(540.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, window, cx| {
            let v = view_ok.read(cx);
            let chosen: Vec<Field> = v.fields.iter().zip(&v.checked).filter(|(_, c)| **c).map(|(f, _)| f.clone()).collect();
            // After this dialog closes: what follows may open another one.
            let on_ok = on_ok.clone();
            window.defer(cx, move |window, cx| on_ok(chosen, window, cx));
            true
        })
    });
}

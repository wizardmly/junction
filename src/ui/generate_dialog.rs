//! Generate's member chooser, as IntelliJ's: the class's properties or
//! fields with their types, all checked, Select All / Select None; OK
//! generates with the checked ones. Override / Implement Methods lists
//! the members under their classes, as IntelliJ's tree.

use std::rc::Rc;

use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, button::{Button, ButtonVariants as _}, checkbox::Checkbox, h_flex, v_flex};
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
    /// Checkboxes under the list (Override Methods' "Insert @Override").
    options: Vec<(&'static str, bool)>,
    /// Each field's `ty` is its group (a member's class): a heading row
    /// over each group, the rows under it indented.
    grouped: bool,
}

impl Render for ChooserView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("generate-fields").max_h(px(280.)).min_h(px(60.)).overflow_y_scroll().p_1().border_1().border_color(palette.border).rounded_md();
        for (ix, f) in self.fields.iter().enumerate() {
            if self.grouped && (ix == 0 || self.fields[ix - 1].ty != f.ty) {
                list = list.child(h_flex().gap_1p5().py_0p5().child(Icon::new(crate::ui::as_icons::CLASS).xsmall()).child(div().text_sm().child(f.ty.clone())));
            }
            list = list.child(
                h_flex()
                    .gap_2()
                    .when(self.grouped, |el| el.pl_5())
                    .child(Checkbox::new(SharedString::from(format!("generate-{ix}"))).checked(self.checked[ix]).on_change(cx.listener(move |this, v: &bool, _, cx| {
                        this.checked[ix] = *v;
                        cx.notify();
                    })))
                    .child(div().text_sm().child(f.name.clone()))
                    .when(!self.grouped && !f.ty.is_empty(), |el| el.child(div().text_sm().text_color(palette.text_secondary).child(f.ty.clone()))),
            );
        }
        let mut options = h_flex().gap_3();
        for (ix, (label, on)) in self.options.iter().enumerate() {
            options = options.child(Checkbox::new(SharedString::from(format!("generate-option-{ix}"))).label(*label).checked(*on).on_change(cx.listener(move |this, v: &bool, _, cx| {
                this.options[ix].1 = *v;
                cx.notify();
            })));
        }
        v_flex()
            .gap_2()
            .child(list)
            .when(!self.options.is_empty(), |el| el.child(options))
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
    choose_checked(title, fields, true, on_ok, window, cx);
}

/// `choose`, the items checked or not to start with (Override Methods
/// starts with none, Implement Methods with all, as IntelliJ's).
pub fn choose_checked(title: &'static str, fields: Vec<Field>, all_checked: bool, on_ok: impl Fn(Vec<Field>, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    open(title, fields, all_checked, Vec::new(), false, move |chosen, _, window, cx| on_ok(chosen, window, cx), window, cx);
}

/// `choose_checked` with checkboxes under the list (`on_ok` gets their
/// states in order too), the fields listed under their groups (`ty`,
/// consecutive fields of a group together).
pub fn choose_grouped(
    title: &'static str,
    fields: Vec<Field>,
    all_checked: bool,
    options: Vec<(&'static str, bool)>,
    on_ok: impl Fn(Vec<Field>, Vec<bool>, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    open(title, fields, all_checked, options, true, on_ok, window, cx);
}

#[allow(clippy::too_many_arguments)]
fn open(
    title: &'static str,
    fields: Vec<Field>,
    all_checked: bool,
    options: Vec<(&'static str, bool)>,
    grouped: bool,
    on_ok: impl Fn(Vec<Field>, Vec<bool>, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let checked = vec![all_checked; fields.len()];
    let view = cx.new(|_| ChooserView { fields, checked, options, grouped });
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title(title).w(px(540.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, window, cx| {
            let v = view_ok.read(cx);
            let chosen: Vec<Field> = v.fields.iter().zip(&v.checked).filter(|(_, c)| **c).map(|(f, _)| f.clone()).collect();
            let options: Vec<bool> = v.options.iter().map(|(_, on)| *on).collect();
            // After this dialog closes: what follows may open another one.
            let on_ok = on_ok.clone();
            window.defer(cx, move |window, cx| on_ok(chosen, options, window, cx));
            true
        })
    });
}

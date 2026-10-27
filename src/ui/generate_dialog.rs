//! Generate's member chooser, as IntelliJ's: the class's properties or
//! fields with their types, all checked, Select All / Select None; OK
//! generates with the checked ones. Override / Implement Methods lists
//! the members under their classes, as IntelliJ's tree, with its toolbar's
//! Sort Alphabetically and Show Classes toggles (remembered, as IntelliJ
//! keeps them: unsorted and grouped at first).

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{Icon, Selectable as _, Sizable as _, WindowExt as _, button::{Button, ButtonVariants as _}, checkbox::Checkbox, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::domain::generate::Field;
use crate::theme::ActivePalette as _;
use crate::ui::common::tool_button;
use crate::ui::dialogs;

pub struct ChooserView {
    fields: Vec<Field>,
    checked: Vec<bool>,
    /// Checkboxes under the list (Override Methods' "Insert @Override").
    options: Vec<(&'static str, bool)>,
    /// Each field's `ty` is its group (a member's class): a heading row
    /// over each group, the rows under it indented.
    grouped: bool,
    /// The grouped list's toggles: Sort Alphabetically, Show Classes.
    sorted: bool,
    show_classes: bool,
}

thread_local! {
    /// The toggles as last left, for the next chooser.
    static TOGGLES: Cell<(bool, bool)> = const { Cell::new((false, true)) };
}

impl ChooserView {
    /// The fields in the order shown: by group (when shown), then by name
    /// when sorted, else as given.
    fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.fields.len()).collect();
        if !self.grouped {
            return order;
        }
        let group = |ix: usize| if self.show_classes { self.fields.iter().position(|f| f.ty == self.fields[ix].ty).unwrap_or(ix) } else { 0 };
        if self.sorted {
            order.sort_by(|&a, &b| group(a).cmp(&group(b)).then_with(|| self.fields[a].name.to_lowercase().cmp(&self.fields[b].name.to_lowercase())));
        } else {
            order.sort_by_key(|&ix| group(ix));
        }
        order
    }
}

impl Render for ChooserView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("generate-fields").max_h(px(280.)).min_h(px(60.)).overflow_y_scroll().p_1().border_1().border_color(palette.border).rounded_md();
        let headings = self.grouped && self.show_classes;
        let mut previous: Option<&str> = None;
        for ix in self.order() {
            let f = &self.fields[ix];
            if headings && previous != Some(f.ty.as_str()) {
                list = list.child(h_flex().gap_1p5().py_0p5().child(Icon::new(crate::ui::as_icons::CLASS).xsmall()).child(div().text_sm().child(f.ty.clone())));
            }
            previous = Some(f.ty.as_str());
            list = list.child(
                h_flex()
                    .gap_2()
                    .when(headings, |el| el.pl_5())
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
        let toolbar = h_flex()
            .gap_0p5()
            .child(tool_button("generate-sort", crate::ui::as_icons::SORT_ALPHABETICALLY, "Sort Alphabetically").selected(self.sorted).on_click(cx.listener(|this, _, _, cx| {
                this.sorted = !this.sorted;
                TOGGLES.set((this.sorted, this.show_classes));
                cx.notify();
            })))
            .child(tool_button("generate-classes", crate::ui::as_icons::CLASS, "Show Classes").selected(self.show_classes).on_click(cx.listener(|this, _, _, cx| {
                this.show_classes = !this.show_classes;
                TOGGLES.set((this.sorted, this.show_classes));
                cx.notify();
            })));
        v_flex()
            .gap_2()
            .when(self.grouped, |el| el.child(toolbar))
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
    let (sorted, show_classes) = TOGGLES.get();
    let view = cx.new(|_| ChooserView { fields, checked, options, grouped, sorted, show_classes });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_and_groups() {
        let field = |name: &str, ty: &str| Field { name: name.into(), ty: ty.into(), is_static: false, is_final: false };
        let fields = vec![field("zeta()", "Shape"), field("area()", "Shape"), field("toString()", "Object"), field("equals(Object)", "Object")];
        let view = |sorted, show_classes| ChooserView { checked: vec![false; 4], fields: fields.clone(), options: Vec::new(), grouped: true, sorted, show_classes };
        assert_eq!(view(false, true).order(), [0, 1, 2, 3]);
        assert_eq!(view(true, true).order(), [1, 0, 3, 2]);
        assert_eq!(view(true, false).order(), [1, 3, 2, 0]);
        assert_eq!(view(false, false).order(), [0, 1, 2, 3]);
    }
}

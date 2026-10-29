//! IntelliJ's Extract Method (Java) / Extract Function (Kotlin) dialog:
//! the name, the visibility, "Declare static" in Java, the return type
//! when the code doesn't show it, the parameters (renamable, their types
//! editable, reordered with Move Up / Move Down) and the signature's preview.

use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::extract_method::{ExtractRequest, MethodPlan, check, signature};
use crate::domain::imports::ImportStyle;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::dialogs;

pub struct ExtractMethodView {
    style: ImportStyle,
    plan: MethodPlan,
    name: Entity<InputState>,
    visibility: String,
    is_static: bool,
    result_type: Option<Entity<InputState>>,
    /// Each parameter's name and type fields, in the plan's order.
    parameters: Vec<(Entity<InputState>, Entity<InputState>)>,
    /// The signature's order (indexes into `parameters`) and the selected row.
    order: Vec<usize>,
    selected: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ExtractMethodView {
    fn request(&self, cx: &App) -> ExtractRequest {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        ExtractRequest {
            name: value(&self.name),
            visibility: self.visibility.clone(),
            is_static: self.is_static,
            parameters: self.parameters.iter().map(|(n, t)| (value(n), value(t))).collect(),
            result_type: self.result_type.as_ref().map(value).unwrap_or_default(),
            order: self.order.clone(),
        }
    }

    /// Move Up / Move Down: the selected parameter one place.
    fn shift(&mut self, up: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let to = if up { ix.checked_sub(1) } else { (ix + 1 < self.order.len()).then_some(ix + 1) };
        if let Some(to) = to {
            self.order.swap(ix, to);
            self.selected = Some(to);
            cx.notify();
        }
    }

    /// The request, if it's valid.
    fn valid(&self, cx: &App) -> Result<ExtractRequest, String> {
        let request = self.request(cx);
        check(self.style, &self.plan, &request)?;
        Ok(request)
    }
}

impl Render for ExtractMethodView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let request = self.request(cx);
        let error = check(self.style, &self.plan, &request).err();
        let java = self.style == ImportStyle::Java;
        let label = |text: &'static str| div().w(px(90.)).flex_none().text_sm().text_color(palette.text_secondary).child(text);
        let options: &[(&str, &str)] = if java {
            &[("private", "private"), ("protected", "protected"), ("", "package-private"), ("public", "public")]
        } else {
            &[("private", "private"), ("internal", "internal"), ("protected", "protected"), ("public", "public")]
        };
        let current = options.iter().find(|(v, _)| *v == self.visibility).map_or("private", |(_, l)| *l);
        let this = cx.entity().downgrade();
        let visibility = Button::new("extract-visibility").outline().small().label(current).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            for (value, label) in options {
                let (value, this) = (value.to_string(), this.clone());
                menu = menu.item(PopupMenuItem::new(*label).on_click(move |_, _, cx| {
                    this.update(cx, |v, cx| {
                        v.visibility = value.clone();
                        cx.notify();
                    })
                    .ok();
                }));
            }
            menu
        });
        let mut params = v_flex().gap_1();
        for (row, i) in self.order.iter().enumerate() {
            let (name, ty) = &self.parameters[*i];
            params = params.child(
                h_flex()
                    .id(("extract-param", row))
                    .gap_2()
                    .p_0p5()
                    .rounded_md()
                    .when(self.selected == Some(row), |el| el.bg(palette.selection))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(row);
                        cx.notify();
                    }))
                    .child(div().w(px(180.)).child(Input::new(name).small()))
                    .child(div().flex_1().child(Input::new(ty).small())),
            );
        }
        let has = self.selected.is_some() && self.order.len() > 1;
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(label("Name:")).child(div().flex_1().child(Input::new(&self.name))))
            .child(
                h_flex().gap_2().child(label("Visibility:")).child(visibility).when(java, |el| {
                    el.child(div().w(px(16.))).child(
                        Checkbox::new("extract-static").label("Declare static").checked(self.is_static || self.plan.is_static).disabled(self.plan.is_static).on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.is_static = *checked;
                            cx.notify();
                        })),
                    )
                }),
            )
            .when_some(self.result_type.clone(), |el, input| el.child(h_flex().gap_2().child(label("Return type:")).child(div().flex_1().child(Input::new(&input)))))
            .when(!self.parameters.is_empty(), |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .child(div().text_sm().text_color(palette.text_secondary).child("Parameters:"))
                        .child(div().flex_1())
                        .child(tool_button("extract-up", icons::MOVE_UP, "Move Up").disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(true, cx))))
                        .child(tool_button("extract-down", icons::MOVE_DOWN, "Move Down").disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(false, cx)))),
                )
                    .child(h_flex().gap_2().text_xs().text_color(palette.text_secondary).child(div().w(px(180.)).child("Name")).child(div().child("Type")))
                    .child(params)
            })
            .child(div().text_sm().text_color(palette.text_secondary).child("Signature Preview:"))
            .child(div().p_2().border_1().border_color(palette.border).rounded_md().bg(palette.panel).font_family(mono).text_sm().child(signature(self.style, &self.plan, &request)))
            .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_conflict).child(e)))
    }
}

/// Opens the dialog for the plan; Refactor hands the request to `on_ok`.
pub fn open(style: ImportStyle, plan: MethodPlan, on_ok: impl Fn(ExtractRequest, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let title = if style == ImportStyle::Java { "Extract Method" } else { "Extract Function" };
    let view = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(plan.suggested_name()));
        let result_type = plan.needs_result_type(style).then(|| cx.new(|cx| InputState::new(window, cx).placeholder(if style == ImportStyle::Java { "e.g. int" } else { "e.g. Int" })));
        let parameters: Vec<_> = plan
            .parameters
            .iter()
            .map(|p| {
                let ty = cx.new(|cx| InputState::new(window, cx).default_value(p.ty.clone()).placeholder("Type"));
                (cx.new(|cx| InputState::new(window, cx).default_value(p.name.clone())), ty)
            })
            .collect();
        let mut _subscriptions = Vec::new();
        let mut watch = |input: &Entity<InputState>, cx: &mut Context<ExtractMethodView>| {
            _subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        };
        watch(&name, cx);
        if let Some(r) = &result_type {
            watch(r, cx);
        }
        for (n, t) in &parameters {
            watch(n, cx);
            watch(t, cx);
        }
        let order = (0..parameters.len()).collect();
        let selected = (!parameters.is_empty()).then_some(0);
        ExtractMethodView { style, is_static: plan.is_static, plan, name, visibility: "private".into(), result_type, parameters, order, selected, _subscriptions }
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title(title).w(px(600.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).valid(cx) else { return false };
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
    focus.update(cx, |s, cx| s.select_all(window, cx));
}

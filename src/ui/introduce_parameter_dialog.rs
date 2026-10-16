//! IntelliJ's Introduce Parameter dialog: the name (its suggestions in a
//! dropdown), the type, "Replace all N occurrences" and, in Kotlin,
//! "Introduce default value" (the calls then left alone).

use std::rc::Rc;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::imports::ImportStyle;
use crate::domain::introduce_parameter::{ParameterPlan, ParameterRequest, check};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::dialogs;

pub struct IntroduceParameterView {
    style: ImportStyle,
    plan: ParameterPlan,
    /// The calls it will change, in all files.
    calls: usize,
    suggestions: Vec<String>,
    name: Entity<InputState>,
    ty: Entity<InputState>,
    replace_all: bool,
    default_value: bool,
    _subscriptions: Vec<Subscription>,
}

impl IntroduceParameterView {
    fn request(&self, cx: &App) -> ParameterRequest {
        ParameterRequest {
            name: self.name.read(cx).value().trim().to_owned(),
            ty: self.ty.read(cx).value().trim().to_owned(),
            replace_all: self.replace_all,
            default_value: self.default_value && self.style == ImportStyle::Kotlin,
        }
    }

    fn valid(&self, cx: &App) -> Result<ParameterRequest, String> {
        let request = self.request(cx);
        check(self.style, &self.plan, &request)?;
        Ok(request)
    }
}

impl Render for IntroduceParameterView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let palette = cx.palette().clone();
        let request = self.request(cx);
        let error = check(self.style, &self.plan, &request).err();
        let label = |text: &'static str| div().w(px(60.)).flex_none().text_sm().text_color(palette.text_secondary).child(text);
        let this = cx.entity().downgrade();
        let suggestions = self.suggestions.clone();
        let names = Button::new("parameter-names").outline().small().child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            for s in &suggestions {
                let (s, this) = (s.clone(), this.clone());
                menu = menu.item(PopupMenuItem::new(s.clone()).on_click(move |_, window, cx| {
                    this.update(cx, |v, cx| v.name.update(cx, |n, cx| n.set_value(s.clone(), window, cx))).ok();
                }));
            }
            menu
        });
        let occurrences = self.plan.occurrences.len();
        let kotlin = self.style == ImportStyle::Kotlin;
        let calls = if request.default_value { 0 } else { self.calls };
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(label("Name:")).child(div().flex_1().child(Input::new(&self.name))).when(self.suggestions.len() > 1, |el| el.child(names)))
            .child(h_flex().gap_2().child(label("Type:")).child(div().flex_1().child(Input::new(&self.ty))))
            .when(occurrences > 1, |el| {
                el.child(Checkbox::new("parameter-replace-all").label(format!("Replace all {occurrences} occurrences")).checked(self.replace_all).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.replace_all = *checked;
                    cx.notify();
                })))
            })
            .when(kotlin, |el| {
                el.child(Checkbox::new("parameter-default").label("Introduce default value").checked(self.default_value).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.default_value = *checked;
                    cx.notify();
                })))
            })
            .child(div().text_xs().text_color(palette.text_secondary).child(match calls {
                0 => format!("The calls of {}() stay as they are", self.plan.function),
                1 => format!("1 call of {}() gets the new argument", self.plan.function),
                n => format!("{n} calls of {}() get the new argument", self.plan.function),
            }))
            .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_conflict).child(e)))
    }
}

/// Opens the dialog; Refactor hands the request to `on_ok`.
pub fn open(
    style: ImportStyle,
    plan: ParameterPlan,
    suggestions: Vec<String>,
    calls: usize,
    on_ok: impl Fn(ParameterRequest, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let first = suggestions.first().cloned().unwrap_or_else(|| "value".into());
        let name = cx.new(|cx| InputState::new(window, cx).default_value(first));
        let placeholder = if style == ImportStyle::Java { "e.g. int" } else { "e.g. Int" };
        let ty = cx.new(|cx| InputState::new(window, cx).default_value(plan.ty.clone()).placeholder(placeholder));
        let mut _subscriptions = Vec::new();
        for input in [&name, &ty] {
            _subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        IntroduceParameterView { style, plan, calls, suggestions, name, ty, replace_all: true, default_value: false, _subscriptions }
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title("Introduce Parameter").w(px(480.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).valid(cx) else { return false };
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
    focus.update(cx, |s, cx| s.select_all(window, cx));
}

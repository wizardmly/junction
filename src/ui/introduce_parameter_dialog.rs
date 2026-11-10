//! IntelliJ's Introduce Parameter dialog: the name (its suggestions in a
//! dropdown), the type, "Replace all N occurrences" and, in Kotlin,
//! "Introduce default value" (the calls then left alone); in Java,
//! "Declare final" and "Delegate via overloading method", remembered as
//! IntelliJ's `JavaRefactoringSettings` does, and the fields with getters
//! the expression uses: "Replace fields used in expressions with their
//! getters".

use std::rc::Rc;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::{Radio, RadioGroup},
    v_flex,
};
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::imports::ImportStyle;
use crate::domain::introduce_parameter::{FieldGetters, ParameterPlan, ParameterRequest, check};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::dialogs;

thread_local! {
    /// Java's "Declare final" and "Delegate via overloading method", as last chosen.
    static LAST_JAVA: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
    /// "Replace fields used in expressions with their getters", as last chosen.
    static LAST_GETTERS: std::cell::Cell<FieldGetters> = const { std::cell::Cell::new(FieldGetters::None) };
}

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
    declare_final: bool,
    delegate: bool,
    getters: FieldGetters,
    _subscriptions: Vec<Subscription>,
}

impl IntroduceParameterView {
    fn request(&self, cx: &App) -> ParameterRequest {
        ParameterRequest {
            name: self.name.read(cx).value().trim().to_owned(),
            ty: self.ty.read(cx).value().trim().to_owned(),
            replace_all: self.replace_all,
            default_value: self.default_value && self.style == ImportStyle::Kotlin,
            declare_final: self.declare_final && self.style == ImportStyle::Java,
            delegate: self.delegate && self.style == ImportStyle::Java && self.plan.delegate.is_some(),
            getters: if self.plan.fields.is_empty() { FieldGetters::None } else { self.getters },
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
        let java = self.style == ImportStyle::Java;
        let can_delegate = java && self.plan.delegate.is_some();
        let calls = if request.keeps_calls() { 0 } else { self.calls };
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
            .when(java, |el| {
                el.child(Checkbox::new("parameter-final").label("Declare final").checked(self.declare_final).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.declare_final = *checked;
                    cx.notify();
                })))
            })
            .when(can_delegate, |el| {
                el.child(Checkbox::new("parameter-delegate").label("Delegate via overloading method").checked(self.delegate).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.delegate = *checked;
                    cx.notify();
                })))
            })
            .when(java && !self.plan.fields.is_empty() && !request.keeps_calls(), |el| {
                const MODES: [FieldGetters; 3] = [FieldGetters::None, FieldGetters::Inaccessible, FieldGetters::All];
                el.child(div().text_sm().text_color(palette.text_secondary).child("Replace fields used in expressions with their getters")).child(
                    RadioGroup::vertical("parameter-getters")
                        .child(Radio::new("parameter-getters-none").label("Do not replace"))
                        .child(Radio::new("parameter-getters-inaccessible").label("Replace fields inaccessible in usage context"))
                        .child(Radio::new("parameter-getters-all").label("Replace all fields"))
                        .selected_index(MODES.iter().position(|m| *m == self.getters))
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.getters = MODES[*ix];
                            cx.notify();
                        })),
                )
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
        let (declare_final, delegate) = LAST_JAVA.with(|l| l.get());
        let getters = LAST_GETTERS.with(|l| l.get());
        IntroduceParameterView { style, plan, calls, suggestions, name, ty, replace_all: true, default_value: false, declare_final, delegate, getters, _subscriptions }
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title("Introduce Parameter").w(px(480.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).valid(cx) else { return false };
            if style == ImportStyle::Java {
                let view = view_ok.read(cx);
                LAST_JAVA.with(|l| l.set((view.declare_final, view.delegate)));
                if !view.plan.fields.is_empty() {
                    LAST_GETTERS.with(|l| l.set(view.getters));
                }
            }
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
    focus.update(cx, |s, cx| s.select_all(window, cx));
}

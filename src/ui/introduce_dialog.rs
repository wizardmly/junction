//! Android Studio's Introduce Variable (Ctrl+Alt+V) and Introduce
//! Constant (Ctrl+Alt+C) dialogs: the name, with the names IntelliJ
//! suggests for the expression; "Replace all occurrences" when the
//! expression is there more than once; for a variable "Declare with var"
//! in Kotlin, "Declare final" in Java; Kotlin's "Specify type
//! explicitly" when the expression shows its type; a Java constant's
//! visibility (the last one chosen, as JavaRefactoringSettings'
//! INTRODUCE_CONSTANT_VISIBILITY, for this session); a Java constant's
//! "Move to another class" with its "To class:" (the class last moved
//! to, for this session).

use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _, h_flex,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::imports::ImportStyle;
use crate::domain::introduce_variable::Visibility;
use crate::domain::rename::check_name;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

/// What Refactor asks for.
pub struct IntroduceRequest {
    pub name: String,
    pub all: bool,
    /// Kotlin `var`, Java `final`.
    pub modifier: bool,
    /// Kotlin `val name: Type`.
    pub explicit_type: bool,
    /// A Java constant's.
    pub visibility: Visibility,
    /// A Java constant's "Move to another class": the class typed.
    pub move_to: Option<String>,
}

thread_local! {
    /// The visibility last chosen for a Java constant.
    static LAST_VISIBILITY: std::cell::Cell<Visibility> = const { std::cell::Cell::new(Visibility::Public) };
    /// The class a constant last moved to.
    static LAST_CLASS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub struct IntroduceView {
    style: ImportStyle,
    /// "Declare with var" / "Declare final"; none for a constant.
    modifier_label: Option<&'static str>,
    name: Entity<InputState>,
    suggestions: Vec<String>,
    occurrences: usize,
    all: bool,
    modifier: bool,
    /// "Specify type explicitly" offered: the type is known.
    typed: bool,
    explicit_type: bool,
    /// The visibility panel shown (a Java constant outside an interface).
    choose_visibility: bool,
    visibility: Visibility,
    /// "Move to another class" offered (a Java constant), checked, and
    /// its class.
    can_move: bool,
    move_to: bool,
    to_class: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl IntroduceView {
    fn extension(&self) -> &'static str {
        if self.style == ImportStyle::Java { "java" } else { "kt" }
    }

    fn request(&self, cx: &App) -> Result<IntroduceRequest, String> {
        let name = self.name.read(cx).value().trim().to_owned();
        check_name(self.extension(), &name)?;
        let move_to = if self.move_to {
            let class = self.to_class.read(cx).value().trim().to_owned();
            if class.is_empty() {
                return Err("Target class name is not specified".into());
            }
            crate::domain::create_test::check_package(self.extension(), &class).map_err(|_| format!("'{class}' is not a valid class name"))?;
            Some(class)
        } else {
            None
        };
        Ok(IntroduceRequest { name, all: self.all, modifier: self.modifier, explicit_type: self.explicit_type, visibility: self.visibility, move_to })
    }
}

impl Render for IntroduceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let error = self.request(cx).err();
        let suggestions = self.suggestions.iter().enumerate().map(|(ix, s)| {
            let s = s.clone();
            div()
                .id(("introduce-suggestion", ix))
                .px_1p5()
                .rounded_sm()
                .text_sm()
                .text_color(palette.link)
                .cursor_pointer()
                .hover(|el| el.bg(palette.hover))
                .child(s.clone())
                .on_click(cx.listener(move |this, _, window, cx| {
                    let s = s.clone();
                    this.name.update(cx, |name, cx| name.set_value(s, window, cx));
                    cx.notify();
                }))
        });
        v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(div().text_sm().child("Name:")).child(div().flex_1().child(Input::new(&self.name))))
            .when(self.suggestions.len() > 1, |el| {
                el.child(h_flex().flex_wrap().gap_1().child(div().text_xs().text_color(palette.text_secondary).child("Suggested:")).children(suggestions))
            })
            .when_some(error.filter(|_| !self.name.read(cx).value().is_empty()), |el, e| {
                el.child(div().text_xs().text_color(palette.status_conflict).child(e))
            })
            .when(self.occurrences > 1, |el| {
                el.child(
                    Checkbox::new("introduce-all")
                        .label(SharedString::from(format!("Replace all occurrences ({} occurrences)", self.occurrences)))
                        .checked(self.all)
                        .on_change(cx.listener(|this, v: &bool, _, cx| {
                            this.all = *v;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.modifier_label, |el, label| {
                el.child(Checkbox::new("introduce-modifier").label(label).checked(self.modifier).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.modifier = *v;
                    cx.notify();
                })))
            })
            .when(self.can_move, |el| {
                el.child(Checkbox::new("introduce-move").label("Move to another class").checked(self.move_to).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.move_to = *v;
                    cx.notify();
                })))
                .when(self.move_to, |el| el.child(h_flex().gap_2().child(div().text_sm().child("To class:")).child(div().flex_1().child(Input::new(&self.to_class)))))
            })
            .when(self.choose_visibility, |el| {
                let selected = Visibility::ALL.iter().position(|v| *v == self.visibility);
                el.child(div().text_sm().child("Visibility")).child(
                    RadioGroup::horizontal("introduce-visibility").children(Visibility::ALL.map(Visibility::label)).selected_index(selected).on_change(cx.listener(
                        |this, ix: &usize, _, cx| {
                            this.visibility = Visibility::ALL[*ix];
                            cx.notify();
                        },
                    )),
                )
            })
            .when(self.typed, |el| {
                el.child(Checkbox::new("introduce-type").label("Specify type explicitly").checked(self.explicit_type).on_change(cx.listener(|this, v: &bool, _, cx| {
                    this.explicit_type = *v;
                    cx.notify();
                })))
            })
    }
}

/// Which dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Introduce {
    Variable,
    Constant,
}

/// Opens Introduce Variable or Constant; Refactor hands the request to
/// `on_ok`.
pub fn open(
    kind: Introduce,
    style: ImportStyle,
    suggestions: Vec<String>,
    occurrences: usize,
    typed: bool,
    choose_visibility: bool,
    can_move: bool,
    on_ok: impl Fn(IntroduceRequest, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let first = suggestions.first().cloned().unwrap_or_else(|| "value".into());
        let name = cx.new(|cx| InputState::new(window, cx).default_value(first));
        let to_class = cx.new(|cx| InputState::new(window, cx).default_value(LAST_CLASS.with(|c| c.borrow().clone())));
        let _subscriptions = [&name, &to_class]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        let modifier_label = match (kind, style) {
            (Introduce::Constant, _) => None,
            (_, ImportStyle::Java) => Some("Declare final"),
            _ => Some("Declare with var"),
        };
        // Android Studio's defaults: replace all, except a Java constant's
        // (JavaRefactoringSettings.INTRODUCE_CONSTANT_REPLACE_ALL).
        let all = occurrences > 1 && !(kind == Introduce::Constant && style == ImportStyle::Java);
        let visibility = LAST_VISIBILITY.with(|v| v.get());
        IntroduceView { style, modifier_label, name, suggestions, occurrences, all, modifier: false, typed, explicit_type: false, choose_visibility, visibility, can_move, move_to: false, to_class, _subscriptions }
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title(if kind == Introduce::Constant { "Introduce Constant" } else { "Introduce Variable" }).w(px(440.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).request(cx) else { return false };
            if view_ok.read(cx).choose_visibility {
                LAST_VISIBILITY.with(|v| v.set(request.visibility));
            }
            if let Some(class) = &request.move_to {
                LAST_CLASS.with(|c| *c.borrow_mut() = class.clone());
            }
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}

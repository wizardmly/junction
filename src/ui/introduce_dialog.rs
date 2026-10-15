//! Android Studio's Introduce Variable dialog (Ctrl+Alt+V): the name, with
//! the names IntelliJ suggests for the expression; "Replace all
//! occurrences" when the expression is there more than once; "Declare
//! with var" in Kotlin, "Declare final" in Java.

use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _, h_flex,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::imports::ImportStyle;
use crate::domain::rename::check_name;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

/// What Refactor asks for.
pub struct IntroduceRequest {
    pub name: String,
    pub all: bool,
    /// Kotlin `var`, Java `final`.
    pub modifier: bool,
}

pub struct IntroduceView {
    style: ImportStyle,
    name: Entity<InputState>,
    suggestions: Vec<String>,
    occurrences: usize,
    all: bool,
    modifier: bool,
    _subscription: Subscription,
}

impl IntroduceView {
    fn extension(&self) -> &'static str {
        if self.style == ImportStyle::Java { "java" } else { "kt" }
    }

    fn request(&self, cx: &App) -> Result<IntroduceRequest, String> {
        let name = self.name.read(cx).value().trim().to_owned();
        check_name(self.extension(), &name)?;
        Ok(IntroduceRequest { name, all: self.all, modifier: self.modifier })
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
        let modifier_label = if self.style == ImportStyle::Java { "Declare final" } else { "Declare with var" };
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
            .child(Checkbox::new("introduce-modifier").label(modifier_label).checked(self.modifier).on_change(cx.listener(|this, v: &bool, _, cx| {
                this.modifier = *v;
                cx.notify();
            })))
    }
}

/// Opens Introduce Variable; Refactor hands the request to `on_ok`.
pub fn open(
    style: ImportStyle,
    suggestions: Vec<String>,
    occurrences: usize,
    on_ok: impl Fn(IntroduceRequest, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| {
        let first = suggestions.first().cloned().unwrap_or_else(|| "value".into());
        let name = cx.new(|cx| InputState::new(window, cx).default_value(first));
        let _subscription = cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        // Android Studio's default: replace all when there are several.
        IntroduceView { style, name, suggestions, occurrences, all: occurrences > 1, modifier: false, _subscription }
    });
    let focus = view.read(cx).name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_ok) = (view.clone(), on_ok.clone());
        dialog.title("Introduce Variable").w(px(440.)).child(view.clone()).footer(dialogs::footer("Refactor")).on_ok(move |_, window, cx| {
            let Ok(request) = view_ok.read(cx).request(cx) else { return false };
            on_ok(request, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}

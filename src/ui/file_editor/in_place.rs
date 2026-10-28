//! Introduce Variable in the editor, IntelliJ's in-place mode (Settings ›
//! Editor › Code Editing › Refactorings › In the editor): the variable is
//! declared under its first suggested name, which is then edited where it
//! stands, every use following. Below the declaration: the other names
//! (Up / Down), and the options the dialog has (Kotlin's "Declare with
//! var" Alt+V and "Specify type explicitly" Alt+T, Java's "Declare final"
//! Alt+F), each redoing the declaration as it changes.

use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, point, prelude::FluentBuilder as _, px};

use super::FileEditor;
use crate::domain::imports::ImportStyle;
use crate::domain::introduce_variable::Introduced;
use crate::theme::ActivePalette as _;
use crate::ui::introduce_dialog::IntroduceRequest;

/// In-place Introduce Variable's plan: its edit, its names (the first one
/// to start with), the classes the declaration's type needs imported,
/// whether Kotlin can write the type, and how many occurrences there are.
pub(super) struct InPlaceIntroduce {
    pub(super) edit: Box<dyn Fn(&str, &IntroduceRequest) -> Result<Introduced, String>>,
    pub(super) name: String,
    pub(super) suggestions: Vec<String>,
    pub(super) imports: Vec<String>,
    pub(super) style: ImportStyle,
    pub(super) typed: bool,
    pub(super) occurrences: usize,
}

/// The variable being named: its plan, the text before it was declared,
/// and the options it is declared with.
pub(super) struct InPlace {
    plan: Rc<InPlaceIntroduce>,
    all: bool,
    original: String,
    modifier: bool,
    explicit_type: bool,
    /// The suggestion Up / Down last put in.
    selected: usize,
}

impl FileEditor {
    /// Declares the variable under the plan's first name and starts naming
    /// it in place.
    pub(super) fn introduce_in_place(&mut self, plan: Rc<InPlaceIntroduce>, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = plan.name.clone();
        let state = InPlace { plan, all, original: self.text(cx), modifier: false, explicit_type: false, selected: 0 };
        self.declare_in_place(state, &name, window, cx);
    }

    /// The declaration for `state`'s options under `name`, written over
    /// what is there now, its name selected with every use following.
    fn declare_in_place(&mut self, state: InPlace, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let plan = state.plan.clone();
        let request = IntroduceRequest {
            name: name.to_owned(),
            all: state.all,
            modifier: state.modifier,
            explicit_type: state.explicit_type,
            visibility: crate::domain::introduce_variable::Visibility::Public,
            move_to: None,
            enum_constant: false,
        };
        let done = match (plan.edit)(&state.original, &request) {
            Ok(done) => done,
            Err(message) => return Self::hint("Introduce Variable", format!("Cannot perform refactoring.\n{message}"), window, cx),
        };
        // Kotlin writes the type only when asked to.
        let imports: &[String] = if plan.style == ImportStyle::Java || state.explicit_type { &plan.imports } else { &[] };
        let (replaced, replacement, shift) = super::with_imports(plan.style, &state.original, done.range.clone(), &done.text, imports);
        let mut target = state.original.clone();
        target.replace_range(replaced, &replacement);
        // One edit from the text as it is now.
        let current = self.text(cx);
        let prefix = current.bytes().zip(target.bytes()).take_while(|(a, b)| a == b).count();
        let suffix = current.bytes().rev().zip(target.bytes().rev()).take_while(|(a, b)| a == b).count().min(current.len() - prefix).min(target.len() - prefix);
        let prefix = floor(&current, &target, prefix);
        let mut suffix = suffix;
        while suffix > 0 && (!current.is_char_boundary(current.len() - suffix) || !target.is_char_boundary(target.len() - suffix)) {
            suffix -= 1;
        }
        let moved = |r: &std::ops::Range<usize>| r.start + shift..r.end + shift;
        let name_at = moved(&done.name_at);
        self.template = None;
        self.state.update(cx, |s, cx| {
            s.set_selected_range(prefix..current.len() - suffix, cx);
            s.replace(&target[prefix..target.len() - suffix], window, cx);
            s.set_selected_range(name_at.clone(), cx);
        });
        let stops = vec![std::iter::once(name_at.clone()).chain(done.uses.iter().map(moved)).collect()];
        self.start_template(crate::domain::live_templates::Session { stops, current: 0, end: name_at.end }, cx);
        self.in_place = Some(state);
    }

    /// Whether the variable is being named (its template still on).
    fn naming(&self) -> bool {
        self.in_place.is_some() && self.template.is_some()
    }

    /// The name as typed so far.
    fn name_in_place(&self, cx: &Context<Self>) -> Option<String> {
        let r = self.template_places()?.first()?.clone();
        self.state.read(cx).value().get(r).map(str::to_owned)
    }

    /// An option toggled: the declaration redone with the name as it is.
    fn toggle_in_place(&mut self, toggle: impl FnOnce(&mut InPlace), window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.name_in_place(cx) else { return };
        let Some(mut state) = self.in_place.take() else { return };
        toggle(&mut state);
        self.declare_in_place(state, &name, window, cx);
    }

    /// Alt+V (Kotlin `var`), Alt+F (Java `final`), Alt+T (Kotlin's type).
    pub(super) fn in_place_key(&mut self, key: &gpui_kit::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.naming() || !key.modifiers.alt || key.modifiers.control || key.modifiers.platform {
            return false;
        }
        let Some(state) = &self.in_place else { return false };
        let kotlin = state.plan.style == ImportStyle::Kotlin;
        match key.key.as_str() {
            "v" if kotlin => self.toggle_in_place(|s| s.modifier = !s.modifier, window, cx),
            "f" if !kotlin => self.toggle_in_place(|s| s.modifier = !s.modifier, window, cx),
            "t" if kotlin && state.plan.typed => self.toggle_in_place(|s| s.explicit_type = !s.explicit_type, window, cx),
            _ => return false,
        }
        true
    }

    /// Up / Down: the previous or next suggested name.
    pub(super) fn in_place_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.naming() || self.state.read(cx).is_completion_menu_open(cx) {
            return false;
        }
        let Some(mut state) = self.in_place.take() else { return false };
        let count = state.plan.suggestions.len() as isize;
        if count < 2 {
            self.in_place = Some(state);
            return false;
        }
        state.selected = (state.selected as isize + delta).rem_euclid(count) as usize;
        let name = state.plan.suggestions[state.selected].clone();
        self.declare_in_place(state, &name, window, cx);
        true
    }

    /// The names and options, under the declared name.
    pub(super) fn render_in_place(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if !self.naming() {
            return None;
        }
        let state = self.in_place.as_ref()?;
        let name_at = self.template_places()?.first()?.clone();
        let bounds = self.state.read(cx).range_to_bounds(&(name_at.start..name_at.start))?;
        let palette = cx.palette().clone();
        let current = self.name_in_place(cx).unwrap_or_default();
        let entity = cx.entity();
        let names = state.plan.suggestions.iter().enumerate().map(|(ix, n)| {
            let entity = entity.clone();
            let chosen = *n == current;
            h_flex()
                .id(("in-place-name", ix))
                .px_1()
                .rounded_sm()
                .when(chosen, |el| el.bg(palette.selection_inactive))
                .child(n.clone())
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        if let Some(mut state) = this.in_place.take() {
                            state.selected = ix;
                            let name = state.plan.suggestions[ix].clone();
                            this.declare_in_place(state, &name, window, cx);
                            this.focus(window, cx);
                        }
                    })
                })
        });
        let kotlin = state.plan.style == ImportStyle::Kotlin;
        let option = |id: &'static str, label: &'static str, checked: bool, toggle: fn(&mut InPlace)| {
            let entity = entity.clone();
            Checkbox::new(id).label(label).checked(checked).on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    this.toggle_in_place(toggle, window, cx);
                    this.focus(window, cx);
                })
            })
        };
        let options = v_flex()
            .gap_1()
            .pt_1()
            .border_t_1()
            .border_color(palette.border)
            .when(kotlin, |el| el.child(option("in-place-var", "Declare with var  Alt+V", state.modifier, |s| s.modifier = !s.modifier)))
            .when(kotlin && state.plan.typed, |el| el.child(option("in-place-type", "Specify type explicitly  Alt+T", state.explicit_type, |s| s.explicit_type = !s.explicit_type)))
            .when(!kotlin, |el| el.child(option("in-place-final", "Declare final  Alt+F", state.modifier, |s| s.modifier = !s.modifier)));
        Some(
            deferred(
                anchored().position(point(bounds.left() - px(4.), bounds.bottom() + px(2.))).snap_to_window_with_margin(px(8.)).child(
                    v_flex()
                        .gap_1()
                        .p_1()
                        .text_sm()
                        .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                        .border_1()
                        .border_color(palette.border)
                        .rounded_md()
                        .shadow_md()
                        .children(names)
                        .child(options),
                ),
            )
            .with_priority(2),
        )
    }
}

/// `at` moved back to a character boundary of both texts.
fn floor(a: &str, b: &str, mut at: usize) -> usize {
    while at > 0 && (!a.is_char_boundary(at) || !b.is_char_boundary(at)) {
        at -= 1;
    }
    at
}

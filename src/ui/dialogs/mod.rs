//! Modal dialogs matching IntelliJ's, one file per area; shared footer and
//! confirm helpers live here.

use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    App, Entity, InteractiveElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

mod branch;
mod push;
mod update;
mod stash;
mod merge_rebase;
mod compare;
mod about;

pub use branch::*;
pub use push::*;
pub use update::*;
pub use stash::*;
pub use merge_rebase::*;
pub use compare::*;
pub use about::*;

/// Focuses a dialog's input once the dialog is open (opening it moves focus).
pub fn focus_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let input = input.clone();
    window.defer(cx, move |window, cx| input.update(cx, |state, cx| state.focus(window, cx)));
}

pub fn footer(ok_label: &'static str) -> DialogFooter {
    footer_enabled(ok_label, true)
}

/// A footer whose OK button is greyed out while the input is invalid
/// (IntelliJ's validation disables the default action).
pub fn footer_enabled(ok_label: &'static str, enabled: bool) -> DialogFooter {
    DialogFooter::new()
        .gap_2()
        .child(DialogClose::new().child(Button::new("cancel").label("Cancel").outline()))
        .child(DialogAction::new().child(Button::new("ok").label(ok_label).primary().disabled(!enabled)))
}

/// A choice between several actions plus Cancel, like IntelliJ's
/// `Messages.showDialog` with custom buttons. The last option is the default.
pub fn choose(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    details: Vec<String>,
    cancel_label: &'static str,
    options: Vec<(&'static str, Rc<dyn Fn(&mut Window, &mut App)>)>,
    window: &mut Window,
    cx: &mut App,
) {
    let (title, message) = (title.into(), message.into());
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let last = options.len().saturating_sub(1);
        let mut footer = DialogFooter::new()
            .gap_2()
            .child(DialogClose::new().child(Button::new("choose-cancel").label(cancel_label).outline()));
        for (ix, (label, run)) in options.iter().enumerate() {
            let run = run.clone();
            // A button with its own click handler isn't closed by DialogClose; close first.
            let button = Button::new(("choose-option", ix)).label(*label).on_click(move |_, window, cx| {
                window.close_dialog(cx);
                run(window, cx)
            });
            footer = footer.child(if ix == last { button.primary() } else { button.outline() });
        }
        // Files or commits the message refers to, one per line.
        let list = v_flex()
            .id("choose-details")
            .max_h(px(200.))
            .overflow_y_scroll()
            .px_2()
            .text_sm()
            .text_color(palette.text_secondary)
            .children(details.iter().map(|line| div().child(line.clone())));
        dialog
            .title(title.clone())
            .w(px(520.))
            .child(v_flex().gap_2().child(div().text_sm().child(message.clone())).when(!details.is_empty(), |el| el.child(list)))
            .footer(footer)
    });
}

/// A Yes / No confirmation, like IntelliJ's `Messages.showYesNoDialog`.
pub fn confirm(title: impl Into<SharedString>, message: impl Into<SharedString>, ok_label: &'static str, on_ok: impl Fn(&mut App) + 'static, window: &mut Window, cx: &mut App) {
    let (title, message) = (title.into(), message.into());
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let on_ok = on_ok.clone();
        dialog
            .title(title.clone())
            .w(px(420.))
            .child(div().text_sm().child(message.clone()))
            .footer(footer(ok_label))
            .on_ok(move |_, _, cx| {
                on_ok(cx);
                true
            })
    });
}

/// A branch name input with a dropdown of local and remote branches.
pub(super) fn branch_picker(id: &'static str, input: &Entity<InputState>, model: &Entity<RepoModel>, cx: &App) -> impl gpui_kit::IntoElement {
    use gpui_kit::component::{h_flex, menu::{DropdownMenu as _, PopupMenuItem}};
    let refs = model.read(cx).refs().clone();
    let current = refs.current_branch.clone();
    let input_for_menu = input.clone();
    h_flex()
        .gap_1()
        .child(div().flex_1().child(Input::new(input)))
        .child(Button::new(id).outline().small().label("Branches").dropdown_menu(move |mut menu, _, _| {
            for reference in refs.local_branches().chain(refs.remote_branches()) {
                if Some(&reference.name) == current.as_ref() {
                    continue;
                }
                let name = reference.name.clone();
                let input = input_for_menu.clone();
                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| {
                    input.update(cx, |state, cx| state.set_value(name.clone(), window, cx));
                }));
            }
            menu
        }))
}

/// IntelliJ's "Git Root" combo of a multi-root project's dialogs (Merge,
/// Rebase, Pull, Stash, Tag, Reset): picking another root makes it the
/// active one and opens the dialog again for it. `None` with a single root.
pub(crate) fn git_root_row(
    model: &Entity<RepoModel>,
    reopen: fn(Entity<RepoModel>, &mut Window, &mut App),
    cx: &App,
) -> Option<gpui_kit::AnyElement> {
    use gpui_kit::IntoElement as _;
    use gpui_kit::component::{h_flex, menu::{DropdownMenu as _, PopupMenuItem}};
    let m = model.read(cx);
    if !m.is_multi_root() {
        return None;
    }
    let active = m.repository().map(|r| r.root().to_path_buf());
    let roots: Vec<(std::path::PathBuf, String)> = m.roots().iter().map(|r| (r.path.clone(), m.root_label(&r.path))).collect();
    let label = roots.iter().find(|(p, _)| Some(p) == active.as_ref()).map(|(_, l)| l.clone()).unwrap_or_default();
    let secondary = cx.palette().text_secondary;
    let model = model.clone();
    Some(
        h_flex()
            .gap_2()
            .text_sm()
            .child(div().text_color(secondary).child("Git Root:"))
            .child(Button::new("git-root").outline().small().label(label).dropdown_caret(true).dropdown_menu(move |mut menu, _, _| {
                for (path, label) in &roots {
                    let (path, model) = (path.clone(), model.clone());
                    let checked = Some(&path) == active.as_ref();
                    menu = menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(move |_, window, cx| {
                        if checked {
                            return;
                        }
                        model.update(cx, |m, cx| m.switch_root(path.clone(), cx));
                        window.close_dialog(cx);
                        reopen(model.clone(), window, cx);
                    }));
                }
                menu
            }))
            .into_any_element(),
    )
}

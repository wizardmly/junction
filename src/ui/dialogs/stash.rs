//! Stash Changes and Unstash As.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Sizable as _, WindowExt as _,
    checkbox::Checkbox,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, ParentElement as _,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

use super::{focus_input, footer};

/// Stash Changes dialog.
pub fn stash(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, StashRequest};
    let message = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
    let keep_index = Rc::new(Cell::new(false));
    let untracked = Rc::new(Cell::new(false));
    let branch = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    let focus_target = message.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let keep_cell = keep_index.clone();
        let untracked_cell = untracked.clone();
        let (keep_ok, untracked_ok, message_ok, model) = (keep_index.clone(), untracked.clone(), message.clone(), model.clone());
        dialog
            .title("Stash")
            .w(px(440.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child(format!("Current branch: {branch}")))
                    .child(Input::new(&message))
                    .child(
                        Checkbox::new("keep-index").label("Keep index").checked(keep_index.get()).on_change(move |v, window, _| {
                            keep_cell.set(*v);
                            window.refresh();
                        }),
                    )
                    .child(
                        Checkbox::new("include-untracked")
                            .label("Include untracked files")
                            .checked(untracked.get())
                            .on_change(move |v, window, _| {
                                untracked_cell.set(*v);
                                window.refresh();
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let request = StashRequest {
                    message: message_ok.read(cx).value().to_string(),
                    keep_index: keep_ok.get(),
                    include_untracked: untracked_ok.get(),
                };
                model.update(cx, |model, cx| model.run_operation("Stash", move |repo| ops::stash_save(repo, &request), cx));
                true
            })
            .footer(footer("Create Stash"))
    });
    focus_input(&focus_target, window, cx);
}

/// Unstash Changes: pop or apply, optionally reinstating the index or
/// turning the stash into a new branch (`git stash branch`).
pub fn unstash_as(model: Entity<RepoModel>, stash: String, message: String, window: &mut Window, cx: &mut App) {
    let branch = cx.new(|cx| InputState::new(window, cx).placeholder("As new branch (optional)"));
    let pop = Rc::new(Cell::new(false));
    let index = Rc::new(Cell::new(false));
    let focus = branch.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let has_branch = !branch.read(cx).value().trim().is_empty();
        let (pop_cell, index_cell) = (pop.clone(), index.clone());
        let (pop_ok, index_ok, branch_ok, model, stash, message) =
            (pop.clone(), index.clone(), branch.clone(), model.clone(), stash.clone(), message.clone());
        dialog
            .title("Unstash Changes")
            .w(px(460.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child(format!("{stash}: {message}")))
                    .child(
                        Checkbox::new("unstash-pop").label("Pop stash").checked(pop.get() || has_branch).disabled(has_branch).on_change(
                            move |v, window, _| {
                                pop_cell.set(*v);
                                window.refresh();
                            },
                        ),
                    )
                    .child(Checkbox::new("unstash-index").label("Reinstate index").checked(index.get() || has_branch).disabled(has_branch).on_change(
                        move |v, window, _| {
                            index_cell.set(*v);
                            window.refresh();
                        },
                    ))
                    .child(Input::new(&branch).small())
                    .child(div().text_xs().text_color(secondary).child("A new branch is created at the commit the stash was made on; the stash is dropped.")),
            )
            .footer(footer("Unstash"))
            .on_ok(move |_, _, cx| {
                let branch = branch_ok.read(cx).value().trim().to_owned();
                let (pop, index, stash) = (pop_ok.get(), index_ok.get(), stash.clone());
                model.update(cx, |m, cx| {
                    m.run_operation("Unstash", move |repo| {
                        if !branch.is_empty() {
                            repo.run(["stash", "branch", &branch, &stash])?;
                            return Ok(format!("Unstashed to new branch {branch}"));
                        }
                        let mut args = vec!["stash", if pop { "pop" } else { "apply" }];
                        if index {
                            args.push("--index");
                        }
                        args.push(&stash);
                        repo.run(&args)?;
                        Ok("Changes unstashed".into())
                    }, cx)
                });
                true
            })
    });
    focus_input(&focus, window, cx);
}

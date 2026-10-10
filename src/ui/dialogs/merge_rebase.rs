//! Merge, Rebase, Conflicts and Abort.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogClose, DialogFooter},
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

use super::{branch_picker, confirm, focus_input, footer};

/// Settings › Version Control › Git, plus Appearance. Changes apply on OK.
/// Opens the merge tool for a conflict (handed in by the workspace).
pub type OpenMerge = Rc<dyn Fn(crate::git::merge::Conflict, &mut Window, &mut App)>;

/// IntelliJ's Conflicts dialog: conflicted files with what each side did,
/// and Accept Yours / Accept Theirs / Merge…. It stays open, refreshing as
/// files are resolved, until the user closes it.
pub fn conflicts(model: Entity<RepoModel>, open_merge: OpenMerge, window: &mut Window, cx: &mut App) {
    use crate::git::merge;
    use gpui_kit::component::h_flex;
    use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _, prelude::FluentBuilder as _};

    let selected = Rc::new(Cell::new(0usize));
    // The sides named by branch ("Changes from feature"), read once.
    let titles = {
        let model = model.read(cx);
        let state = model.state();
        model.repository().map_or_else(
            || {
                let (left, right) = merge::side_titles(state);
                (left.to_owned(), right.to_owned())
            },
            |repository| merge::branch_titles(repository, state),
        )
    };
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let (ours_title, theirs_title) = titles.clone();
        let conflicts = merge::conflicts(model.read(cx).status());
        let current = selected.get().min(conflicts.len().saturating_sub(1));
        let chosen = conflicts.get(current).cloned();

        let mut list = v_flex().border_1().border_color(palette.border).rounded_md().h(px(220.)).overflow_hidden().child(
            h_flex()
                .px_2()
                .h(px(24.))
                .text_xs()
                .text_color(palette.text_secondary)
                .border_b_1()
                .border_color(palette.border)
                .child(div().flex_1().child("Name"))
                .child(div().w(px(90.)).child("Yours"))
                .child(div().w(px(90.)).child("Theirs")),
        );
        for (ix, conflict) in conflicts.iter().enumerate() {
            let (yours, theirs) = conflict.kind.sides();
            let select = selected.clone();
            let open = open_merge.clone();
            let merge_conflict = conflict.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("conflict-{ix}")))
                    .px_2()
                    .h(px(24.))
                    .text_sm()
                    .when(ix == current, |el| el.bg(palette.selection))
                    .on_click(move |event, window, cx| {
                        select.set(ix);
                        // Double-click opens the merge tool, as in IntelliJ.
                        if event.click_count() >= 2 && merge_conflict.kind.can_merge() {
                            window.close_dialog(cx);
                            open(merge_conflict.clone(), window, cx);
                        }
                        window.refresh();
                    })
                    .child(div().flex_1().text_color(palette.status_conflict).child(conflict.path.clone()))
                    .child(div().w(px(90.)).text_color(palette.text_secondary).child(yours))
                    .child(div().w(px(90.)).text_color(palette.text_secondary).child(theirs)),
            );
        }
        if conflicts.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(palette.text_secondary).child("All conflicts have been resolved"));
        }

        let accept = |ours: bool| {
            let model = model.clone();
            let chosen = chosen.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                let Some(conflict) = chosen.clone() else { return };
                model.update(cx, |m, cx| {
                    m.run_operation(if ours { "Accept Yours" } else { "Accept Theirs" }, move |repo| {
                        merge::accept(repo, &conflict, ours)?;
                        Ok(String::new())
                    }, cx)
                });
            }
        };
        let open = open_merge.clone();
        let merge_target = chosen.clone();
        dialog
            .title("Conflicts")
            .w(px(620.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(palette.text_secondary).child(format!("Left: {ours_title}  ·  Right: {theirs_title}")))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(div().flex_1().child(list))
                            .child(
                                v_flex()
                                    .w(px(130.))
                                    .gap_2()
                                    .child(Button::new("conflict-yours").outline().small().w_full().label("Accept Yours").disabled(chosen.is_none()).on_click(accept(true)))
                                    .child(Button::new("conflict-theirs").outline().small().w_full().label("Accept Theirs").disabled(chosen.is_none()).on_click(accept(false)))
                                    .child(
                                        Button::new("conflict-merge")
                                            .primary()
                                            .small()
                                            .w_full()
                                            .label("Merge…")
                                            .disabled(!chosen.as_ref().is_some_and(|c| c.kind.can_merge()))
                                            .on_click(move |_, window, cx| {
                                                if let Some(conflict) = merge_target.clone() {
                                                    window.close_dialog(cx);
                                                    open(conflict, window, cx);
                                                }
                                            }),
                                    ),
                            ),
                    ),
            )
            .footer(DialogFooter::new().child(DialogClose::new().child(Button::new("conflicts-close").label("Close").outline())))
    });
}

/// Git › Merge…: the branch to merge into the current one, and git's merge options.
pub fn merge(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let branch = cx.new(|cx| InputState::new(window, cx).placeholder("Branch to merge"));
    let message = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message (optional)"));
    // --no-ff, --ff-only, --squash, --no-commit, --no-verify, --allow-unrelated-histories
    let flags = Rc::new(std::cell::RefCell::new([false; 6]));
    let current = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    let focus_target = branch.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let set = flags.borrow().clone();
        let options = ["--no-ff", "--ff-only", "--squash", "--no-commit", "--no-verify", "--allow-unrelated-histories"];
        let mut grid = v_flex().gap_1();
        for (ix, option) in options.iter().enumerate() {
            let flags = flags.clone();
            grid = grid.child(Checkbox::new(SharedString::from(format!("merge-opt-{ix}"))).label(*option).checked(set[ix]).on_change(
                move |v, window, _| {
                    let mut f = flags.borrow_mut();
                    f[ix] = *v;
                    // --ff-only excludes --no-ff and --squash, as IntelliJ greys them out.
                    if ix == 1 && *v {
                        f[0] = false;
                        f[2] = false;
                    }
                    if (ix == 0 || ix == 2) && *v {
                        f[1] = false;
                    }
                    window.refresh();
                },
            ));
        }
        let (branch_ok, message_ok, flags_ok, model_ok) = (branch.clone(), message.clone(), flags.clone(), model.clone());
        dialog
            .title(format!("Merge into {current}"))
            .w(px(480.))
            .child(
                v_flex()
                    .gap_3()
                    .child(branch_picker("merge-branches", &branch, &model, cx))
                    .child(div().text_sm().text_color(secondary).child("Options"))
                    .child(grid)
                    .child(Input::new(&message)),
            )
            .footer(footer("Merge"))
            .on_ok(move |_, _, cx| {
                let target = branch_ok.read(cx).value().trim().to_owned();
                if target.is_empty() {
                    return false;
                }
                let message = message_ok.read(cx).value().trim().to_owned();
                let set = flags_ok.borrow().clone();
                // Like IntelliJ's smart merge: local changes are stashed and restored.
                let mut args = vec!["merge".to_owned(), "--autostash".to_owned()];
                args.extend(options.iter().zip(set).filter(|(_, on)| *on).map(|(o, _)| o.to_string()));
                if !message.is_empty() {
                    args.push("-m".into());
                    args.push(message);
                }
                args.push(target.clone());
                model_ok.update(cx, |m, cx| {
                    m.run_operation("Merge", move |repo| {
                        repo.run(&args)?;
                        Ok(format!("Merged {target}"))
                    }, cx)
                });
                true
            })
    });
    focus_input(&focus_target, window, cx);
}

/// Git › Rebase…: onto a branch, with --onto / --interactive / --rebase-merges.
pub fn rebase(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let onto = cx.new(|cx| InputState::new(window, cx).placeholder("Branch or commit to rebase onto"));
    let upstream = cx.new(|cx| InputState::new(window, cx).placeholder("Upstream (for --onto: commits after it are moved)"));
    // --interactive, --rebase-merges, --onto, --keep-empty, --autostash
    let flags = Rc::new(std::cell::RefCell::new([false, false, false, false, true]));
    let current = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    let focus_target = onto.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let set = flags.borrow().clone();
        let labels = ["--interactive", "--rebase-merges", "--onto", "--keep-empty", "--autostash"];
        let mut grid = v_flex().gap_1();
        for (ix, label) in labels.iter().enumerate() {
            let flags = flags.clone();
            grid = grid.child(Checkbox::new(SharedString::from(format!("rebase-opt-{ix}"))).label(*label).checked(set[ix]).on_change(
                move |v, window, _| {
                    flags.borrow_mut()[ix] = *v;
                    window.refresh();
                },
            ));
        }
        let (onto_ok, upstream_ok, flags_ok, model_ok) = (onto.clone(), upstream.clone(), flags.clone(), model.clone());
        dialog
            .title(format!("Rebase {current}"))
            .w(px(480.))
            .child(
                v_flex()
                    .gap_3()
                    .child(branch_picker("rebase-branches", &onto, &model, cx))
                    .child(grid)
                    .children(set[2].then(|| Input::new(&upstream))),
            )
            .footer(footer("Rebase"))
            .on_ok(move |_, window, cx| {
                let target = onto_ok.read(cx).value().trim().to_owned();
                if target.is_empty() {
                    return false;
                }
                let set = flags_ok.borrow().clone();
                if set[0] && !set[2] {
                    // Interactive: our own editor instead of git's todo file.
                    crate::ui::rebase_dialog::open_onto(model_ok.clone(), target, window, cx);
                    return true;
                }
                let mut args = vec!["rebase".to_owned()];
                for (ix, flag) in ["", "--rebase-merges", "", "--keep-empty", "--autostash"].iter().enumerate() {
                    if set[ix] && !flag.is_empty() {
                        args.push(flag.to_string());
                    }
                }
                if set[2] {
                    let upstream = upstream_ok.read(cx).value().trim().to_owned();
                    if upstream.is_empty() {
                        return false;
                    }
                    args.extend(["--onto".to_owned(), target.clone(), upstream]);
                } else {
                    args.push(target.clone());
                }
                model_ok.update(cx, |m, cx| {
                    m.run_operation("Rebase", move |repo| {
                        repo.run(&args)?;
                        Ok(format!("Rebased onto {target}"))
                    }, cx)
                });
                true
            })
    });
    focus_input(&focus_target, window, cx);
}

/// Abort Rebase / Merge / Cherry-Pick / Revert, after IntelliJ's confirmation.
pub fn abort_operation(model: Entity<RepoModel>, state: crate::git::RepositoryState, window: &mut Window, cx: &mut App) {
    use crate::git::RepositoryState::*;
    let (title, what) = match state {
        Rebasing => ("Abort Rebase", "rebase"),
        Merging => ("Abort Merge", "merge"),
        CherryPicking => ("Abort Cherry-Pick", "cherry-pick"),
        Reverting => ("Abort Revert", "revert"),
        Normal => return,
    };
    confirm(
        title,
        format!("Are you sure you want to abort the {what}? Changes made during it will be lost."),
        "Abort",
        move |cx| {
            model.update(cx, |m, cx| {
                m.run_operation("Git", move |repo| crate::git::merge::step(repo, state, crate::git::merge::OperationStep::Abort), cx)
            })
        },
        window,
        cx,
    );
}

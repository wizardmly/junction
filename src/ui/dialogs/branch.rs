//! Branch dialogs: Create New Branch, New Tag, Git Reset, Rename, Checkout Revision.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    checkbox::Checkbox,
    input::{Input, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, ParentElement as _,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::settings::Settings;
use crate::theme::ActivePalette as _;

use super::{focus_input, footer};

/// Git › New Branch: name, "Checkout branch" (on by default), start point.
pub fn new_branch(model: Entity<RepoModel>, start_point: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Branch name"));
    let checkout = Rc::new(Cell::new(true));
    let short_start = start_point[..start_point.len().min(10)].to_owned();
    let focus_target = name.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let checkout_value = checkout.get();
        let checkout_cell = checkout.clone();
        let name_for_ok = name.clone();
        let model = model.clone();
        let start = start_point.clone();
        let checkout_ok = checkout.clone();
        dialog
            .title("Create New Branch")
            .w(px(420.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child(format!("From {short_start}")))
                    .child(Input::new(&name))
                    .child(
                        Checkbox::new("checkout")
                            .label("Checkout branch")
                            .checked(checkout_value)
                            .on_change(move |value, window, _| {
                                checkout_cell.set(*value);
                                window.refresh();
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let branch = name_for_ok.read(cx).value().trim().to_owned();
                if branch.is_empty() {
                    return false;
                }
                let start = start.clone();
                let checkout = checkout_ok.get();
                model.update(cx, |model, cx| {
                    // Synchronous branch control: the new branch is created in every root,
                    // each from its own HEAD.
                    let others = if Settings::get(cx).sync_branches { model.other_roots() } else { Vec::new() };
                    model.run_operation("New Branch", move |repo| {
                        crate::git::status::create_branch(repo, &branch, &start, checkout)?;
                        let synced = crate::ui::branches_popup::sync_to_roots(repo, &others, |other| {
                            crate::git::status::create_branch(other, &branch, "HEAD", checkout).is_ok()
                        });
                        let base = if checkout { format!("Checked out new branch {branch}") } else { format!("Created branch {branch}") };
                        Ok(if synced > 0 { format!("{base} in {} repositories", synced + 1) } else { base })
                    }, cx)
                });
                true
            })
            .footer(footer("Create"))
    });
    focus_input(&focus_target, window, cx);
}

/// Git › New Tag.
pub fn new_tag(model: Entity<RepoModel>, target: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Tag name"));
    let message = cx.new(|cx| InputState::new(window, cx).placeholder("Message (optional, creates an annotated tag)"));
    let focus_target = name.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let name_for_ok = name.clone();
        let message_for_ok = message.clone();
        let model = model.clone();
        let target = target.clone();
        dialog
            .title("Create New Tag")
            .w(px(420.))
            .child(v_flex().gap_3().child(Input::new(&name)).child(Input::new(&message)))
            .on_ok(move |_, _, cx| {
                let tag = name_for_ok.read(cx).value().trim().to_owned();
                if tag.is_empty() {
                    return false;
                }
                let message = message_for_ok.read(cx).value().trim().to_owned();
                let target = target.clone();
                model.update(cx, |model, cx| {
                    model.run_operation("New Tag", move |repo| {
                        if message.is_empty() {
                            repo.run(["tag", &tag, &target])?;
                        } else {
                            repo.run(["tag", "-a", &tag, "-m", &message, &target])?;
                        }
                        Ok(format!("Created tag {tag}"))
                    }, cx)
                });
                true
            })
            .footer(footer("Create Tag"))
    });
    focus_input(&focus_target, window, cx);
}

const RESET_MODES: [(&str, &str, &str); 4] = [
    ("Soft", "--soft", "Files won't change, differences will be staged for commit."),
    ("Mixed", "--mixed", "Files won't change, differences won't be staged."),
    ("Hard", "--hard", "Files will be reverted to the state of the selected commit. Warning: any local changes will be lost."),
    ("Keep", "--keep", "Files whose changes differ between the current and the selected commit will be reverted. Local changes are kept."),
];

/// Git Reset dialog, from "Reset Current Branch to Here…".
pub fn reset_to(model: Entity<RepoModel>, target: String, window: &mut Window, cx: &mut App) {
    let mode = Rc::new(Cell::new(1usize));
    let branch = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let selected = mode.get();
        let mode_cell = mode.clone();
        let mode_ok = mode.clone();
        let model = model.clone();
        let target = target.clone();
        dialog
            .title("Git Reset")
            .w(px(480.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().child(format!(
                        "This will reset the current branch head ({branch}) to the selected commit {}.",
                        &target[..target.len().min(8)]
                    )))
                    .child(
                        RadioGroup::new("reset-mode")
                            .children(RESET_MODES.iter().map(|(label, _, _)| *label))
                            .selected_index(Some(selected))
                            .on_change(move |ix, window, _| {
                                mode_cell.set(*ix);
                                window.refresh();
                            }),
                    )
                    .child(div().text_sm().text_color(secondary).child(RESET_MODES[selected].2)),
            )
            .on_ok(move |_, _, cx| {
                let (label, flag, _) = RESET_MODES[mode_ok.get()];
                let target = target.clone();
                model.update(cx, |model, cx| {
                    model.run_operation("Reset", move |repo| {
                        repo.run(["reset", flag, &target])?;
                        Ok(format!("{label} reset to {}", &target[..target.len().min(8)]))
                    }, cx)
                });
                true
            })
            .footer(footer("Reset"))
    });
}

/// Rename Branch dialog.
pub fn rename_branch(model: Entity<RepoModel>, branch: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).default_value(branch.clone()));
    let focus_target = name.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (name_ok, model, old) = (name.clone(), model.clone(), branch.clone());
        dialog
            .title(format!("Rename Branch '{branch}'"))
            .w(px(400.))
            .child(Input::new(&name))
            .on_ok(move |_, _, cx| {
                let new = name_ok.read(cx).value().trim().to_owned();
                if new.is_empty() || new == old {
                    return false;
                }
                let old = old.clone();
                model.update(cx, |model, cx| {
                    model.run_operation("Rename Branch", move |repo| {
                        repo.run(["branch", "-m", &old, &new])?;
                        Ok(format!("Renamed {old} to {new}"))
                    }, cx)
                });
                true
            })
            .footer(footer("Rename"))
    });
    focus_input(&focus_target, window, cx);
}

/// Checkout Tag or Revision…: detaches HEAD at a tag, branch or hash.
pub fn checkout_revision(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Tag, branch or commit hash"));
    let focus_target = input.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (ok_input, model) = (input.clone(), model.clone());
        dialog
            .title("Checkout Tag or Revision")
            .w(px(400.))
            .child(Input::new(&input))
            .on_ok(move |_, _, cx| {
                let revision = ok_input.read(cx).value().trim().to_owned();
                if revision.is_empty() {
                    return false;
                }
                model.update(cx, |model, cx| {
                    model.run_operation("Checkout", move |repo| {
                        let commit = repo.run(["rev-parse", "--verify", "--quiet", &format!("{revision}^{{commit}}")])
                            .map_err(|_| anyhow::anyhow!("Unknown revision: {revision}"))?;
                        repo.run(["checkout", "--detach", commit.trim()])?;
                        Ok(format!("Checked out {revision}"))
                    }, cx)
                });
                true
            })
            .footer(footer("Checkout"))
    });
    focus_input(&focus_target, window, cx);
}

//! Modal dialogs matching IntelliJ's: Create New Branch, New Tag, Git Reset.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window, div, px};

use crate::model::RepoModel;
use crate::theme::ActivePalette as _;

fn footer(ok_label: &'static str) -> DialogFooter {
    DialogFooter::new()
        .gap_2()
        .child(DialogClose::new().child(Button::new("cancel").label("Cancel").outline()))
        .child(DialogAction::new().child(Button::new("ok").label(ok_label).primary()))
}

/// Git › New Branch: name, "Checkout branch" (on by default), start point.
pub fn new_branch(model: Entity<RepoModel>, start_point: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Branch name"));
    let checkout = Rc::new(Cell::new(true));
    let short_start = start_point[..start_point.len().min(10)].to_owned();
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
                    model.run_operation("New Branch", move |repo| {
                        crate::git::status::create_branch(repo, &branch, &start, checkout)?;
                        Ok(if checkout { format!("Checked out new branch {branch}") } else { format!("Created branch {branch}") })
                    }, cx)
                });
                true
            })
            .footer(footer("Create"))
    });
}

/// Git › New Tag.
pub fn new_tag(model: Entity<RepoModel>, target: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Tag name"));
    let message = cx.new(|cx| InputState::new(window, cx).placeholder("Message (optional, creates an annotated tag)"));
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

/// The Push dialog: commits that will be pushed, the editable target branch,
/// and IntelliJ's options (force push with lease, push tags, run hooks).
pub fn push(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, PushRequest, PushTags};
    use gpui_kit::component::{ActiveTheme as _, h_flex, scroll::ScrollableElement as _};
    use gpui_kit::{InteractiveElement as _, prelude::FluentBuilder as _};
    use std::cell::RefCell;

    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let preview = match ops::push_preview(&repository) {
        Ok(preview) => preview,
        Err(error) => {
            window.push_notification(gpui_kit::component::notification::Notification::error(error.to_string()), cx);
            return;
        }
    };
    let Some(branch) = preview.branch.clone() else {
        window.push_notification(
            gpui_kit::component::notification::Notification::warning("Cannot push: HEAD is detached"),
            cx,
        );
        return;
    };
    let target = cx.new(|cx| InputState::new(window, cx).default_value(preview.target.clone()));
    #[derive(Clone, Copy)]
    struct Options {
        force: bool,
        tags: Option<usize>,
        hooks: bool,
        remote: usize,
    }
    let remote_ix = preview.remotes.iter().position(|r| *r == preview.remote).unwrap_or(0);
    let options = Rc::new(RefCell::new(Options { force: false, tags: None, hooks: true, remote: remote_ix }));
    let repo_name = repository.name();

    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let current = *options.borrow();
        let remote = preview.remotes.get(current.remote).cloned().unwrap_or_else(|| preview.remote.clone());
        let mut commits = v_flex().gap_px();
        for commit in &preview.commits {
            commits = commits.child(
                h_flex()
                    .h(px(22.))
                    .gap_2()
                    .text_sm()
                    .child(div().font_family(mono.clone()).text_color(palette.text_secondary).child(commit.short_hash().to_owned()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.subject.clone()))
                    .child(div().text_color(palette.text_secondary).child(commit.author_name.clone())),
            );
        }
        if preview.commits.is_empty() {
            commits = commits.child(div().text_sm().text_color(palette.text_secondary).child("Nothing to push"));
        }

        let set = |options: &Rc<RefCell<Options>>, edit: fn(&mut Options, bool)| {
            let options = options.clone();
            move |value: &bool, window: &mut Window, _: &mut App| {
                edit(&mut options.borrow_mut(), *value);
                window.refresh();
            }
        };
        let tags_options = options.clone();
        let remote_options = options.clone();
        let remotes = preview.remotes.clone();

        let ok_options = options.clone();
        let ok_target = target.clone();
        let ok_branch = branch.clone();
        let ok_model = model.clone();
        let ok_remotes = preview.remotes.clone();
        let ok_default_remote = preview.remote.clone();
        let has_upstream = !preview.new_branch;

        dialog
            .title(format!("Push Commits to {repo_name}"))
            .w(px(620.))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(branch.clone()))
                            .child("→")
                            .child(
                                Button::new("push-remote")
                                    .ghost()
                                    .xsmall()
                                    .label(remote.clone())
                                    .when(remotes.len() > 1, |b| {
                                        b.on_click(move |_, window, _| {
                                            let mut o = remote_options.borrow_mut();
                                            o.remote = (o.remote + 1) % remotes.len().max(1);
                                            window.refresh();
                                        })
                                    }),
                            )
                            .child(":")
                            .child(div().w(px(220.)).child(Input::new(&target).xsmall()))
                            .when(preview.new_branch, |el| {
                                el.child(
                                    div()
                                        .px_1()
                                        .rounded(px(3.))
                                        .bg(palette.status_added)
                                        .text_xs()
                                        .text_color(gpui_kit::white())
                                        .child("New"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("push-commits")
                            .h(px(220.))
                            .p_2()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(palette.border)
                            .overflow_y_scrollbar()
                            .child(commits),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                Checkbox::new("push-tags")
                                    .label("Push tags:")
                                    .checked(current.tags.is_some())
                                    .on_change(set(&options, |o, v| o.tags = v.then_some(0))),
                            )
                            .when(current.tags.is_some(), |el| {
                                el.child(
                                    RadioGroup::horizontal("push-tags-mode")
                                        .children(["All", "Current Branch"])
                                        .selected_index(current.tags)
                                        .on_change(move |ix, window, _| {
                                            tags_options.borrow_mut().tags = Some(*ix);
                                            window.refresh();
                                        }),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                Checkbox::new("push-hooks")
                                    .label("Run Git hooks")
                                    .checked(current.hooks)
                                    .on_change(set(&options, |o, v| o.hooks = v)),
                            )
                            .child(
                                Checkbox::new("push-force")
                                    .label("Force push (--force-with-lease)")
                                    .checked(current.force)
                                    .on_change(set(&options, |o, v| o.force = v)),
                            ),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let o = *ok_options.borrow();
                let target = ok_target.read(cx).value().trim().to_owned();
                if target.is_empty() {
                    return false;
                }
                let remote = ok_remotes.get(o.remote).cloned().unwrap_or_else(|| ok_default_remote.clone());
                let request = PushRequest {
                    remote,
                    branch: ok_branch.clone(),
                    target,
                    force_with_lease: o.force,
                    tags: match o.tags {
                        None => PushTags::None,
                        Some(0) => PushTags::All,
                        Some(_) => PushTags::CurrentBranch,
                    },
                    set_upstream: !has_upstream,
                    run_hooks: o.hooks,
                };
                ok_model.update(cx, |model, cx| {
                    model.run_operation(if request.force_with_lease { "Force Push" } else { "Push" }, move |repo| ops::push(repo, &request), cx)
                });
                true
            })
            .footer(footer(if current.force { "Force Push" } else { "Push" }))
    });
}

/// Update Project (`Ctrl+T`): fetch, then merge or rebase, stashing local changes.
pub fn update_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let rebase = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let selected = if rebase.get() { 1 } else { 0 };
        let rebase_cell = rebase.clone();
        let rebase_ok = rebase.clone();
        let model = model.clone();
        dialog
            .title("Update Project")
            .w(px(420.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child("Update type"))
                    .child(
                        RadioGroup::new("update-type")
                            .children(["Merge incoming changes into the current branch", "Rebase the current branch on top of incoming changes"])
                            .selected_index(Some(selected))
                            .on_change(move |ix, window, _| {
                                rebase_cell.set(*ix == 1);
                                window.refresh();
                            }),
                    )
                    .child(div().text_sm().text_color(secondary).child("Local changes are stashed before updating and restored afterwards.")),
            )
            .on_ok(move |_, _, cx| {
                let rebase = rebase_ok.get();
                model.update(cx, |model, cx| {
                    model.run_operation("Update Project", move |repo| {
                        repo.run(["fetch", "--all", "--prune"])?;
                        let has_upstream = repo.run(["rev-parse", "--abbrev-ref", "@{upstream}"]).is_ok();
                        if !has_upstream {
                            anyhow::bail!("the current branch has no tracked branch");
                        }
                        let before = repo.run(["rev-parse", "HEAD"])?;
                        repo.run(["pull", if rebase { "--rebase" } else { "--no-rebase" }, "--autostash"])?;
                        let after = repo.run(["rev-parse", "HEAD"])?;
                        if before == after {
                            return Ok("All files are up to date".into());
                        }
                        let count = repo
                            .run(["rev-list", "--count", &format!("{}..{}", before.trim(), after.trim())])
                            .unwrap_or_default();
                        Ok(format!("{} commits pulled", count.trim()))
                    }, cx)
                });
                true
            })
            .footer(footer("OK"))
    });
}

/// Stash Changes dialog.
pub fn stash(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, StashRequest};
    let message = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
    let keep_index = Rc::new(Cell::new(false));
    let untracked = Rc::new(Cell::new(false));
    let branch = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
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
}

/// Rename Branch dialog.
pub fn rename_branch(model: Entity<RepoModel>, branch: String, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).default_value(branch.clone()));
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
}

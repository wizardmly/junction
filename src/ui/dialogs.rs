//! Modal dialogs matching IntelliJ's: Create New Branch, New Tag, Git Reset.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, AppContext as _, Entity, ParentElement as _, SharedString, Styled as _, Window, div, px};

use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;

/// Focuses a dialog's input once the dialog is open (opening it moves focus).
pub fn focus_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let input = input.clone();
    window.defer(cx, move |window, cx| input.update(cx, |state, cx| state.focus(window, cx)));
}

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
                    model.run_operation("New Branch", move |repo| {
                        crate::git::status::create_branch(repo, &branch, &start, checkout)?;
                        Ok(if checkout { format!("Checked out new branch {branch}") } else { format!("Created branch {branch}") })
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
        // Settings › Git › Protected branches: no force push to them.
        let protected = Settings::get(cx).is_protected(target.read(cx).value().trim());

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
                                    .disabled(protected)
                                    .on_change(set(&options, |o, v| o.force = v)),
                            )
                            .when(protected, |el| {
                                el.child(div().text_xs().text_color(palette.text_secondary).child("Force push is disabled: protected branch"))
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let o = *ok_options.borrow();
                let target = ok_target.read(cx).value().trim().to_owned();
                if target.is_empty() {
                    return false;
                }
                let remote = ok_remotes.get(o.remote).cloned().unwrap_or_else(|| ok_default_remote.clone());
                let force = o.force && !Settings::get(cx).is_protected(&target);
                let request = PushRequest {
                    remote,
                    branch: ok_branch.clone(),
                    target,
                    force_with_lease: force,
                    tags: match o.tags {
                        None => PushTags::None,
                        Some(0) => PushTags::All,
                        Some(_) => PushTags::CurrentBranch,
                    },
                    set_upstream: !has_upstream,
                    run_hooks: o.hooks,
                };
                let settings = Settings::get(cx);
                let auto_update = settings.auto_update_on_push_rejected;
                let rebase = settings.update_method == UpdateMethod::Rebase;
                ok_model.update(cx, |model, cx| {
                    model.run_operation(if request.force_with_lease { "Force Push" } else { "Push" }, move |repo| {
                        if auto_update {
                            return ops::push_with_auto_update(repo, &request, rebase);
                        }
                        ops::push(repo, &request).map_err(|error| {
                            if ops::is_rejected(&error) {
                                anyhow::anyhow!("Push rejected: the remote has commits that aren't in {}. Update Project (Ctrl+T), then push again.", request.branch)
                            } else {
                                error
                            }
                        })
                    }, cx)
                });
                true
            })
            .footer(footer(if current.force && !protected { "Force Push" } else { "Push" }))
    });
}

/// Update Project (`Ctrl+T`): fetch, then merge or rebase, stashing local changes.
pub fn update_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    // IntelliJ remembers the last update method chosen here.
    let rebase = Rc::new(Cell::new(Settings::get(cx).update_method == UpdateMethod::Rebase));
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
                let method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                if Settings::get(cx).update_method != method {
                    Settings::update(cx, |s| s.update_method = method);
                }
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

/// Settings › Version Control › Git, plus Appearance. Changes apply on OK.
pub fn settings(window: &mut Window, cx: &mut App) {
    let draft = Rc::new(std::cell::RefCell::new(Settings::get(cx).clone()));
    let initial = Settings::get(cx).clone();
    let protected = cx.new(|cx| InputState::new(window, cx).default_value(initial.protected_branches.clone()));
    let margin = cx.new(|cx| InputState::new(window, cx).default_value(initial.commit_subject_limit.to_string()));
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let current = draft.borrow().clone();
        let section = |title: &'static str| {
            div().pt_1().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.text).child(title)
        };
        let check = |id: &'static str, label: &'static str, value: bool, set: fn(&mut Settings, bool)| {
            let draft = draft.clone();
            Checkbox::new(id).label(label).checked(value).on_change(move |v, window, _| {
                set(&mut draft.borrow_mut(), *v);
                window.refresh();
            })
        };
        let theme_draft = draft.clone();
        let update_draft = draft.clone();
        let ok_draft = draft.clone();
        let (ok_protected, ok_margin) = (protected.clone(), margin.clone());
        dialog
            .title("Settings")
            .w(px(520.))
            .child(
                v_flex()
                    .gap_2()
                    .child(section("Appearance"))
                    .child(
                        RadioGroup::horizontal("settings-theme")
                            .children(["Dark", "Light"])
                            .selected_index(Some(if current.dark { 0 } else { 1 }))
                            .on_change(move |ix, window, _| {
                                theme_draft.borrow_mut().dark = *ix == 0;
                                window.refresh();
                            }),
                    )
                    .child(section("Version Control › Git"))
                    .child(check("settings-staging", "Enable staging area", current.staging_area, |s, v| s.staging_area = v))
                    .child(
                        div()
                            .pl_6()
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .child("Show Staged and Unstaged changes in the Commit tool window instead of changelists"),
                    )
                    .child(check(
                        "settings-auto-update",
                        "Auto-update if push of the current branch was rejected",
                        current.auto_update_on_push_rejected,
                        |s, v| s.auto_update_on_push_rejected = v,
                    ))
                    .child(div().pt_1().text_sm().text_color(palette.text_secondary).child("Update method"))
                    .child(
                        RadioGroup::horizontal("settings-update-method")
                            .children(["Merge", "Rebase"])
                            .selected_index(Some(if current.update_method == UpdateMethod::Rebase { 1 } else { 0 }))
                            .on_change(move |ix, window, _| {
                                update_draft.borrow_mut().update_method =
                                    if *ix == 1 { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                                window.refresh();
                            }),
                    )
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .pt_1()
                            .text_sm()
                            .child("Protected branches:")
                            .child(div().flex_1().child(Input::new(&protected).small())),
                    )
                    .child(check(
                        "settings-crlf",
                        "Warn if CRLF line separators are about to be committed",
                        current.warn_crlf,
                        |s, v| s.warn_crlf = v,
                    ))
                    .child(check(
                        "settings-detached",
                        "Warn when committing in detached HEAD or during rebase",
                        current.warn_detached_head,
                        |s, v| s.warn_detached_head = v,
                    ))
                    .child(section("Version Control › Commit"))
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .text_sm()
                            .child("Subject line length limit:")
                            .child(div().w(px(70.)).child(Input::new(&margin).small())),
                    ),
            )
            .footer(footer("OK"))
            .on_ok(move |_, window, cx| {
                let mut next = ok_draft.borrow().clone();
                next.protected_branches = ok_protected.read(cx).value().trim().to_owned();
                if let Ok(limit) = ok_margin.read(cx).value().trim().parse::<usize>() {
                    next.commit_subject_limit = limit.clamp(20, 200);
                }
                if next.dark != Settings::get(cx).dark {
                    crate::theme::apply(next.dark, cx);
                }
                Settings::update(cx, |s| *s = next);
                window.refresh();
                true
            })
    });
}

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
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let state = model.read(cx).state();
        let (ours_title, theirs_title) = merge::side_titles(state);
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

/// A branch name input with a dropdown of local and remote branches.
fn branch_picker(id: &'static str, input: &Entity<InputState>, model: &Entity<RepoModel>, cx: &App) -> impl gpui_kit::IntoElement {
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

pub type OpenDiff = Rc<dyn Fn(crate::ui::diff_view::DiffSource, &mut Window, &mut App)>;

/// "Show Diff with Working Tree" / "Compare with Local": the files that differ
/// between a revision and the working tree (or another revision). Double-click
/// opens the file's diff.
pub fn compare_files(model: Entity<RepoModel>, old: String, new: Option<String>, open_diff: OpenDiff, window: &mut Window, cx: &mut App) {
    use crate::ui::common;
    use crate::ui::diff_view::DiffSource;
    use gpui_kit::component::{Icon, h_flex};
    use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _, prelude::FluentBuilder as _};

    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let files = crate::git::diff::changed_files(&repository, &old, new.as_deref());
    let short = |r: &str| if r.len() == 40 { r[..8].to_owned() } else { r.to_owned() };
    let title = match &new {
        None => format!("Difference between '{}' and the current working tree", short(&old)),
        Some(new) => format!("Difference between '{}' and '{}'", short(&old), short(new)),
    };
    let selected = Rc::new(Cell::new(0usize));
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mut list = v_flex().id("compare-files").border_1().border_color(palette.border).rounded_md().h(px(320.)).overflow_y_scroll();
        match &files {
            Err(error) => list = list.child(div().p_3().text_sm().text_color(palette.status_conflict).child(error.to_string())),
            Ok(files) if files.is_empty() => {
                list = list.child(div().p_3().text_sm().text_color(palette.text_secondary).child("No differences"))
            }
            Ok(files) => {
                for (ix, file) in files.iter().enumerate() {
                    let select = selected.clone();
                    let open = open_diff.clone();
                    let source = DiffSource::Between { old: old.clone(), new: new.clone(), path: file.path.clone(), old_path: file.old_path.clone() };
                    list = list.child(
                        h_flex()
                            .id(SharedString::from(format!("compare-{ix}")))
                            .px_2()
                            .gap_1()
                            .h(px(24.))
                            .flex_shrink_0()
                            .text_sm()
                            .when(ix == selected.get(), |el| el.bg(palette.selection))
                            .on_click(move |event, window, cx| {
                                select.set(ix);
                                if event.click_count() >= 2 {
                                    window.close_dialog(cx);
                                    open(source.clone(), window, cx);
                                }
                                window.refresh();
                            })
                            .child(Icon::new(common::file_icon(&file.path)).small().text_color(palette.text_secondary))
                            .child(div().text_color(common::change_color(file.kind, &palette)).child(file.path.clone()))
                            .when_some(file.old_path.clone(), |el, old| {
                                el.child(div().text_xs().text_color(palette.text_secondary).child(format!("from {old}")))
                            }),
                    );
                }
            }
        }
        let count = files.as_ref().map_or(0, |f| f.len());
        let open = open_diff.clone();
        let chosen = files.as_ref().ok().and_then(|f| f.get(selected.get())).map(|file| DiffSource::Between {
            old: old.clone(),
            new: new.clone(),
            path: file.path.clone(),
            old_path: file.old_path.clone(),
        });
        dialog
            .title(title.clone())
            .w(px(640.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(palette.text_secondary).child(format!(
                        "{count} {} changed · double-click a file to see its diff",
                        if count == 1 { "file" } else { "files" }
                    )))
                    .child(list),
            )
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("compare-close").label("Close").outline()))
                    .child(DialogClose::new().child(
                        Button::new("compare-diff")
                            .label("Show Diff")
                            .primary()
                            .disabled(chosen.is_none())
                            .on_click(move |_, window, cx| {
                                if let Some(source) = chosen.clone() {
                                    open(source, window, cx);
                                }
                            }),
                    )),
            )
    });
}

//! Modal dialogs matching IntelliJ's: Create New Branch, New Tag, Git Reset.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, AppContext as _, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _, Window, div, px};

use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;

/// Focuses a dialog's input once the dialog is open (opening it moves focus).
pub fn focus_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let input = input.clone();
    window.defer(cx, move |window, cx| input.update(cx, |state, cx| state.focus(window, cx)));
}

pub fn footer(ok_label: &'static str) -> DialogFooter {
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

/// The Push dialog: commits that will be pushed, the editable target branch,
/// and IntelliJ's options (force push with lease, push tags, run hooks).
pub fn push(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, PushRequest, PushTags};
    use gpui_kit::component::{ActiveTheme as _, h_flex, scroll::ScrollableElement as _};
    use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _, prelude::FluentBuilder as _};
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
        /// The commit whose files the change tree shows; all commits when `None`.
        selected: Option<usize>,
    }
    let remote_ix = preview.remotes.iter().position(|r| *r == preview.remote).unwrap_or(0);
    let options = Rc::new(RefCell::new(Options { force: false, tags: None, hooks: true, remote: remote_ix, selected: None }));
    // The right-hand change tree: each commit's files, and their union.
    let commit_files: Vec<Vec<crate::git::log::FileChange>> = preview
        .commits
        .iter()
        .map(|c| crate::git::log::load_details(&repository, &c.hash).map(|d| d.changes).unwrap_or_default())
        .collect();
    let all_files: Vec<crate::git::log::FileChange> = {
        let mut seen = std::collections::BTreeMap::new();
        // Oldest first, so the newest change to a path wins.
        for files in commit_files.iter().rev() {
            for file in files {
                seen.insert(file.path.clone(), file.clone());
            }
        }
        seen.into_values().collect()
    };
    let repo_name = repository.name();

    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let current = *options.borrow();
        let remote = preview.remotes.get(current.remote).cloned().unwrap_or_else(|| preview.remote.clone());
        let mut commits = v_flex().gap_px();
        for (ix, commit) in preview.commits.iter().enumerate() {
            let is_selected = current.selected == Some(ix);
            let select_options = options.clone();
            commits = commits.child(
                h_flex()
                    .id(("push-commit", ix))
                    .h(px(22.))
                    .px_1()
                    .gap_2()
                    .rounded(px(3.))
                    .text_sm()
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(palette.selection))
                    .on_click(move |_, window, _| {
                        let mut o = select_options.borrow_mut();
                        o.selected = if o.selected == Some(ix) { None } else { Some(ix) };
                        window.refresh();
                    })
                    .child(div().font_family(mono.clone()).text_color(palette.text_secondary).child(commit.short_hash().to_owned()))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.subject.clone()))
                    .child(div().text_color(palette.text_secondary).child(commit.author_name.clone())),
            );
        }
        if preview.commits.is_empty() {
            commits = commits.child(div().text_sm().text_color(palette.text_secondary).child("Nothing to push"));
        }
        let shown_files = current.selected.and_then(|ix| commit_files.get(ix)).unwrap_or(&all_files);
        let mut files = v_flex()
            .gap_px()
            .child(div().pb_1().text_xs().text_color(palette.text_secondary).child(format!(
                "{} file{} changed",
                shown_files.len(),
                if shown_files.len() == 1 { "" } else { "s" }
            )));
        for file in shown_files {
            let (dir, name) = file.path.rsplit_once('/').map_or(("", file.path.as_str()), |(d, n)| (d, n));
            files = files.child(
                h_flex()
                    .h(px(22.))
                    .gap_1p5()
                    .text_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::File).xsmall().text_color(palette.text_secondary))
                    .child(div().text_color(crate::ui::common::change_color(file.kind, &palette)).child(name.to_owned()))
                    .child(div().text_xs().text_color(palette.text_secondary).text_ellipsis().child(dir.to_owned())),
            );
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
            .w(px(760.))
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
                        h_flex()
                            .h(px(240.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(palette.border)
                            .child(div().id("push-commits").flex_1().h_full().p_1().overflow_y_scrollbar().child(commits))
                            .child(
                                div()
                                    .id("push-files")
                                    .w(px(260.))
                                    .h_full()
                                    .p_2()
                                    .border_l_1()
                                    .border_color(palette.border)
                                    .overflow_y_scrollbar()
                                    .child(files),
                            ),
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

/// Update Project (`Ctrl+T`): fetch, then merge or rebase, cleaning the
/// working tree with stash or shelve. Both choices are remembered.
pub fn update_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let settings = Settings::get(cx);
    let rebase = Rc::new(Cell::new(settings.update_method == UpdateMethod::Rebase));
    let shelve = Rc::new(Cell::new(settings.update_shelve));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (rebase_cell, rebase_ok) = (rebase.clone(), rebase.clone());
        let (shelve_cell, shelve_ok) = (shelve.clone(), shelve.clone());
        let model = model.clone();
        dialog
            .title("Update Project")
            .w(px(440.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(secondary).child("Update type"))
                    .child(
                        RadioGroup::new("update-type")
                            .children(["Merge incoming changes into the current branch", "Rebase the current branch on top of incoming changes"])
                            .selected_index(Some(if rebase.get() { 1 } else { 0 }))
                            .on_change(move |ix, window, _| {
                                rebase_cell.set(*ix == 1);
                                window.refresh();
                            }),
                    )
                    .child(div().text_sm().text_color(secondary).child("Clean working tree before update"))
                    .child(
                        RadioGroup::new("update-clean")
                            .children(["Using Stash", "Using Shelve"])
                            .selected_index(Some(if shelve.get() { 1 } else { 0 }))
                            .on_change(move |ix, window, _| {
                                shelve_cell.set(*ix == 1);
                                window.refresh();
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| {
                let rebase = rebase_ok.get();
                let shelve = shelve_ok.get();
                let method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                let settings = Settings::get(cx);
                if settings.update_method != method || settings.update_shelve != shelve {
                    Settings::update(cx, |s| {
                        s.update_method = method;
                        s.update_shelve = shelve;
                    });
                }
                let clean = if shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
                model.update(cx, |model, cx| {
                    // Multi-root projects update every root, as IntelliJ does.
                    let roots: Vec<std::path::PathBuf> = model.roots().iter().map(|r| r.path.clone()).collect();
                    let project = model.project_root().map(std::path::Path::to_path_buf);
                    model.run_operation("Update Project", move |repo| {
                        if roots.len() < 2 {
                            return crate::git::ops::update_project(repo, rebase, clean);
                        }
                        let project = project.unwrap_or_else(|| repo.root().to_path_buf());
                        let mut lines = Vec::new();
                        let mut failed = Vec::new();
                        for root in &roots {
                            let label = crate::git::roots::label(&project, root);
                            let result = repo.nested(&root.to_string_lossy()).and_then(|r| crate::git::ops::update_project(&r, rebase, clean));
                            match result {
                                Ok(message) => lines.push(format!("{label}: {}", message.split('\u{1f}').next().unwrap_or_default())),
                                Err(error) => failed.push(format!("{label}: {error}")),
                            }
                        }
                        if !failed.is_empty() && lines.is_empty() {
                            anyhow::bail!("{}", failed.join("\n"));
                        }
                        lines.extend(failed.into_iter().map(|f| format!("{f} (failed)")));
                        Ok(lines.join("\n"))
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

/// Settings › Version Control › Git, plus Appearance. Changes apply on OK.
pub fn settings(window: &mut Window, cx: &mut App) {
    let draft = Rc::new(std::cell::RefCell::new(Settings::get(cx).clone()));
    let initial = Settings::get(cx).clone();
    let protected = cx.new(|cx| InputState::new(window, cx).default_value(initial.protected_branches.clone()));
    let margin = cx.new(|cx| InputState::new(window, cx).default_value(initial.commit_subject_limit.to_string()));
    let fetch_interval = cx.new(|cx| InputState::new(window, cx).default_value(initial.fetch_interval_minutes.max(1).to_string()));
    let git_path = cx.new(|cx| InputState::new(window, cx).placeholder(match crate::git::detected_executable() {
        Some(git) => format!("Auto-detected: {}", git.display()),
        None => "Git not found: install Git or enter its path".to_owned(),
    }).default_value(initial.git_executable.clone()));
    // Settings › Languages & Frameworks: one server command per language,
    // with what would run when left empty.
    let servers: Vec<(crate::index::lang::Lang, Entity<InputState>, Option<String>)> = crate::index::lang::Lang::ALL
        .into_iter()
        .filter(|l| *l != crate::index::lang::Lang::Tsx)
        .map(|lang| {
            let detected = crate::index::lsp::detected(lang);
            let placeholder = match &detected {
                Some(command) => format!("Auto: {command}"),
                None => format!("Not found: {}", lang.default_servers().join(" / ")),
            };
            let value = initial.language_servers.get(lang.key()).cloned().unwrap_or_default();
            (lang, cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value)), detected)
        })
        .collect();
    let servers = Rc::new(servers);
    // The page shown, from the list on the left.
    let page: Rc<Cell<usize>> = Rc::default();
    // The Test button's result line.
    let git_test: Rc<std::cell::RefCell<Option<Result<String, String>>>> = Rc::default();
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let current = draft.borrow().clone();
        let fetch_draft = draft.clone();
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
        let (ok_protected, ok_margin, ok_fetch, ok_git_path) = (protected.clone(), margin.clone(), fetch_interval.clone(), git_path.clone());
        let (test_path, test_result) = (git_path.clone(), git_test.clone());
        let test_line = git_test.borrow().clone();
        let ok_servers = servers.clone();
        let server_rows = servers.iter().map(|(lang, input, detected)| {
            let status = match (input.read(cx).value().trim(), detected) {
                ("off", _) => ("Off", palette.text_secondary),
                (custom, _) if !custom.is_empty() => {
                    let found = custom.split_whitespace().next().and_then(crate::index::lsp::find_program).is_some();
                    if found { ("Custom", palette.status_added) } else { ("Not found", palette.status_conflict) }
                }
                (_, Some(_)) => ("Installed", palette.status_added),
                (_, None) => ("Index only", palette.text_secondary),
            };
            gpui_kit::component::h_flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(90.)).child(lang.name()))
                .child(div().flex_1().child(Input::new(input).small().disabled(!current.use_language_servers)))
                .child(div().w(px(70.)).text_xs().text_color(status.1).child(status.0))
        });
        let page_index = page.get();
        let nav = |ix: usize, label: &'static str| {
            let page = page.clone();
            Button::new(("settings-page", ix)).small().ghost().w_full().justify_start().selected(page_index == ix).label(label).on_click(move |_, window, _| {
                page.set(ix);
                window.refresh();
            })
        };
        let navigation = v_flex().w(px(170.)).gap_1().child(nav(0, "Version Control")).child(nav(1, "Languages & Frameworks"));
        let general = v_flex()
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
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .text_sm()
                            .child("Path to Git executable:")
                            .child(div().flex_1().child(Input::new(&git_path).small()))
                            .child(Button::new("settings-git-test").small().label("Test").on_click(move |_, window, cx| {
                                let path = test_path.read(cx).value().to_string();
                                *test_result.borrow_mut() = Some(crate::git::executable_version(&path).map_err(|e| e.to_string()));
                                window.refresh();
                            })),
                    )
                    .children(test_line.map(|result| {
                        let (text, color) = match result {
                            Ok(version) => (version, palette.status_added),
                            Err(error) => (error, palette.status_conflict),
                        };
                        div().pl_6().text_xs().text_color(color).child(text)
                    }))
                    .child(check(
                        "settings-credential-helper",
                        "Use credential helper",
                        current.use_credential_helper,
                        |s, v| s.use_credential_helper = v,
                    ))
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
                    .child(div().pt_1().text_sm().text_color(palette.text_secondary).child("Clean working tree using"))
                    .child(
                        RadioGroup::horizontal("settings-update-clean")
                            .children(["Stash", "Shelve"])
                            .selected_index(Some(if current.update_shelve { 1 } else { 0 }))
                            .on_change({
                                let draft = draft.clone();
                                move |ix, window, _| {
                                    draft.borrow_mut().update_shelve = *ix == 1;
                                    window.refresh();
                                }
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
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .text_sm()
                            .child(Checkbox::new("settings-fetch").label("Update branch info: fetch every").checked(current.fetch_interval_minutes > 0).on_change(
                                move |v, window, _| {
                                    // The minutes come from the input on OK; 1 marks "on".
                                    fetch_draft.borrow_mut().fetch_interval_minutes = *v as u32;
                                    window.refresh();
                                },
                            ))
                            .child(div().w(px(50.)).child(Input::new(&fetch_interval).small().disabled(current.fetch_interval_minutes == 0)))
                            .child("minutes"),
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
                    );
        let languages = v_flex()
                    .gap_2()
                    .child(section("Languages & Frameworks › Code Navigation"))
                    .child(check(
                        "settings-lsp",
                        "Use language servers for Go to Declaration, Quick Documentation and Find Usages",
                        current.use_language_servers,
                        |s, v| s.use_language_servers = v,
                    ))
                    .child(div().pl_6().text_xs().text_color(palette.text_secondary).child(
                        "The built-in index always answers, and resolves calls across JNI, Dart FFI, extern \"C\" and Swift/Objective-C bridges. \
                         Leave a command empty to use the detected server, or type off to disable one.",
                    ))
                    .children(server_rows);
        dialog
            .title("Settings")
            .w(px(780.))
            .child(
                gpui_kit::component::h_flex()
                    .items_start()
                    .gap_4()
                    .child(navigation)
                    .child(div().flex_1().min_w_0().child(if page_index == 0 { general.into_any_element() } else { languages.into_any_element() })),
            )
            .footer(footer("OK"))
            .on_ok(move |_, window, cx| {
                let mut next = ok_draft.borrow().clone();
                next.protected_branches = ok_protected.read(cx).value().trim().to_owned();
                next.git_executable = ok_git_path.read(cx).value().trim().to_owned();
                next.language_servers = ok_servers
                    .iter()
                    .filter_map(|(lang, input, _)| {
                        let command = input.read(cx).value().trim().to_owned();
                        (!command.is_empty()).then(|| (lang.key().to_owned(), command))
                    })
                    .collect();
                if let Ok(limit) = ok_margin.read(cx).value().trim().parse::<usize>() {
                    next.commit_subject_limit = limit.clamp(20, 200);
                }
                if next.fetch_interval_minutes > 0 {
                    next.fetch_interval_minutes = ok_fetch.read(cx).value().trim().parse::<u32>().unwrap_or(10).clamp(1, 1440);
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

/// "Compare with Branch…" / "Compare with Revision…" for one file: pick a
/// branch, tag or revision, then diff that version against the working tree.
pub fn compare_file_with(model: Entity<RepoModel>, path: String, open_diff: OpenDiff, window: &mut Window, cx: &mut App) {
    let revision = cx.new(|cx| InputState::new(window, cx).placeholder("Branch, tag or revision"));
    let focus_target = revision.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let (revision_ok, path_ok, open) = (revision.clone(), path.clone(), open_diff.clone());
        let repository = model.read(cx).repository().cloned();
        dialog
            .title(format!("Compare {path} with…"))
            .w(px(460.))
            .child(branch_picker("compare-file-branches", &revision, &model, cx))
            .on_ok(move |_, window, cx| {
                let revision = revision_ok.read(cx).value().trim().to_owned();
                let valid = repository.as_ref().is_some_and(|repo| {
                    repo.run(["rev-parse", "--verify", "-q", &format!("{revision}^{{commit}}")]).is_ok()
                });
                if revision.is_empty() || !valid {
                    return false;
                }
                let source = crate::ui::diff_view::DiffSource::Between { old: revision, new: None, path: path_ok.clone(), old_path: None };
                open(source, window, cx);
                true
            })
            .footer(footer("Compare"))
    });
    focus_input(&focus_target, window, cx);
}

/// Configure GPG Key: sign commits with one of the user's secret keys (repository config).
pub fn configure_gpg(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let keys = crate::git::gpg::secret_keys(&repository);
    let (sign, current) = crate::git::gpg::signing_config(&repository);
    let sign = Rc::new(Cell::new(sign));
    let selected = Rc::new(Cell::new(keys.iter().position(|k| current.ends_with(&k.id) || k.id.ends_with(&current)).unwrap_or(0)));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (sign_set, sign_ok) = (sign.clone(), sign.clone());
        let (key_set, key_ok) = (selected.clone(), selected.clone());
        let keys_ok = keys.clone();
        let repository = repository.clone();
        let model = model.clone();
        let body = v_flex()
            .gap_3()
            .child(Checkbox::new("gpg-sign").label("Sign commits with GPG key").checked(sign.get()).on_change(
                move |value, window, _| {
                    sign_set.set(*value);
                    window.refresh();
                },
            ))
            .child(if keys.is_empty() {
                div()
                    .text_sm()
                    .text_color(secondary)
                    .child("No secret keys found. Install GnuPG and create a key with gpg --full-generate-key.")
                    .into_any_element()
            } else {
                RadioGroup::new("gpg-key")
                    .children(keys.iter().map(|k| format!("{}  {}", k.id, k.user)))
                    .selected_index(Some(selected.get()))
                    .disabled(!sign.get())
                    .on_change(move |ix, window, _| {
                        key_set.set(*ix);
                        window.refresh();
                    })
                    .into_any_element()
            })
            .child(div().text_xs().text_color(secondary).child("Saved to this repository's config (commit.gpgSign, user.signingKey)."));
        dialog
            .title("Configure GPG Key")
            .w(px(520.))
            .child(body)
            .on_ok(move |_, _, cx| {
                let sign = sign_ok.get();
                let key = keys_ok.get(key_ok.get()).map(|k| k.id.clone()).unwrap_or_default();
                let result = crate::git::gpg::set_signing_config(&repository, sign, &key);
                let (title, message, error) = match result {
                    Ok(()) if sign => ("GPG", format!("Commits will be signed with {key}"), false),
                    Ok(()) => ("GPG", "Commit signing turned off".to_owned(), false),
                    Err(error) => ("GPG", error.to_string(), true),
                };
                model.update(cx, |m, cx| m.notify(title, message, error, cx));
                true
            })
            .footer(footer("OK"))
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

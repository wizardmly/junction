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
use gpui_kit::{
    App, AppContext as _, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;

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
/// and IntelliJ's options (push tags, run hooks); Force Push (with lease)
/// is in the Push button's dropdown.
pub fn push(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    push_up_to(model, None, window, cx)
}

/// After Commit and Push: the Push dialog, or a direct push of the current
/// branch when Settings › Git turns the dialog off (or keeps it for
/// protected branches only and the target isn't one).
pub fn push_after_commit(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, PushRequest, PushTags};
    let settings = Settings::get(cx).clone();
    let preview = model.read(cx).repository().and_then(|repo| ops::push_preview(repo).ok()).filter(|p| p.branch.is_some() && !p.remotes.is_empty());
    let Some(preview) = preview else { return push(model, window, cx) };
    let dialog = settings.commit_push_dialog && (!settings.commit_push_dialog_protected_only || settings.is_protected(&preview.target));
    if dialog {
        return push(model, window, cx);
    }
    let request = PushRequest {
        remote: preview.remote,
        branch: preview.branch.unwrap_or_default(),
        target: preview.target,
        force_with_lease: false,
        tags: PushTags::None,
        set_upstream: !preview.has_upstream,
        run_hooks: settings.run_hooks,
        up_to: None,
    };
    run_push(model, request, cx);
}

/// The Push dialog for the current branch, or (Push All up to Here) for
/// its commits up to `up_to`.
pub fn push_up_to(model: Entity<RepoModel>, up_to: Option<String>, window: &mut Window, cx: &mut App) {
    use crate::git::ops::{self, PushRequest, PushTags};
    use gpui_kit::component::{ActiveTheme as _, h_flex, menu::{DropdownMenu as _, PopupMenuItem}, scroll::ScrollableElement as _};
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
        tags: Option<usize>,
        hooks: bool,
        remote: usize,
        /// The commit whose files the change tree shows; all commits when `None`.
        selected: Option<usize>,
    }
    let remote_ix = preview.remotes.iter().position(|r| *r == preview.remote).unwrap_or(0);
    let options = Rc::new(RefCell::new(Options { tags: None, hooks: true, remote: remote_ix, selected: None }));
    /// What the push would send to one remote branch, worked out again
    /// whenever the remote or the target branch changes.
    struct Outgoing {
        remote: String,
        target: String,
        new_branch: bool,
        commits: Vec<crate::git::Commit>,
        /// Each commit's files, and their union for the change tree.
        commit_files: Vec<Vec<crate::git::log::FileChange>>,
        all_files: Vec<crate::git::log::FileChange>,
    }
    let up_to_ok = up_to.clone();
    let outgoing = move |repository: &crate::git::Repository, remote: &str, target: &str| {
        let (new_branch, mut commits) = ops::push_commits(repository, remote, target).unwrap_or_default();
        // Newest first: the ones above `up_to` stay behind.
        if let Some(pos) = up_to.as_ref().and_then(|h| commits.iter().position(|c| &c.hash == h)) {
            commits.drain(..pos);
        }
        let commit_files: Vec<Vec<crate::git::log::FileChange>> = commits
            .iter()
            .map(|c| crate::git::log::load_details(repository, &c.hash).map(|d| d.changes).unwrap_or_default())
            .collect();
        let mut seen = std::collections::BTreeMap::new();
        // Oldest first, so the newest change to a path wins.
        for files in commit_files.iter().rev() {
            for file in files {
                seen.insert(file.path.clone(), file.clone());
            }
        }
        Outgoing { remote: remote.to_owned(), target: target.to_owned(), new_branch, commits, commit_files, all_files: seen.into_values().collect() }
    };
    let computed = Rc::new(RefCell::new(Outgoing {
        remote: preview.remote.clone(),
        target: preview.target.clone(),
        new_branch: preview.new_branch,
        commits: Vec::new(),
        commit_files: Vec::new(),
        all_files: Vec::new(),
    }));
    *computed.borrow_mut() = outgoing(&repository, &preview.remote, &preview.target);
    let repo_name = repository.name();

    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let remote = preview.remotes.get(options.borrow().remote).cloned().unwrap_or_else(|| preview.remote.clone());
        let target_name = target.read(cx).value().trim().to_owned();
        // IntelliJ re-checks the target as it is edited: "New", and the
        // commits that remote doesn't have.
        if computed.borrow().remote != remote || computed.borrow().target != target_name {
            *computed.borrow_mut() = outgoing(&repository, &remote, &target_name);
            options.borrow_mut().selected = None;
        }
        let current = *options.borrow();
        let out = computed.borrow();
        let mut commits = v_flex().gap_px();
        for (ix, commit) in out.commits.iter().enumerate() {
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
                    .overflow_hidden()
                    .when(is_selected, |el| el.bg(palette.selection))
                    .on_click(move |_, window, _| {
                        let mut o = select_options.borrow_mut();
                        o.selected = if o.selected == Some(ix) { None } else { Some(ix) };
                        window.refresh();
                    })
                    .child(div().flex_shrink_0().font_family(mono.clone()).text_color(palette.text_secondary).child(commit.short_hash().to_owned()))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.subject.clone()))
                    .child(div().flex_shrink_0().text_color(palette.text_secondary).child(commit.author_name.to_string())),
            );
        }
        if out.commits.is_empty() {
            commits = commits.child(div().text_sm().text_color(palette.text_secondary).child("Nothing to push"));
        }
        let shown_files = current.selected.and_then(|ix| out.commit_files.get(ix)).unwrap_or(&out.all_files);
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
                    .child(div().flex_shrink_0().text_color(crate::ui::common::change_color(file.kind, &palette)).child(name.to_owned()))
                    .child(div().min_w_0().text_xs().text_color(palette.text_secondary).text_ellipsis().child(dir.to_owned())),
            );
        }
        let new_branch = out.new_branch;
        // Nothing to send: IntelliJ greys out Push (tags still count).
        let nothing = out.commits.is_empty() && !new_branch && current.tags.is_none();
        drop(out);

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
        let ok_up_to = up_to_ok.clone();
        let has_upstream = preview.has_upstream;
        // Settings › Git › Protected branches: no force push to them.
        let protected = Settings::get(cx).is_protected(&target_name);
        // Where a force push would go, for its confirmation.
        let force_target = {
            let (options, target, remotes, default_remote) =
                (options.clone(), target.clone(), preview.remotes.clone(), preview.remote.clone());
            move |cx: &App| {
                let remote = remotes.get(options.borrow().remote).cloned().unwrap_or_else(|| default_remote.clone());
                format!("{remote}/{}", target.read(cx).value().trim())
            }
        };
        let force_branch = branch.clone();
        let push = Rc::new(move |force: bool, cx: &mut App| -> bool {
            let o = *ok_options.borrow();
            let target = ok_target.read(cx).value().trim().to_owned();
            if target.is_empty() || nothing {
                return false;
            }
            let remote = ok_remotes.get(o.remote).cloned().unwrap_or_else(|| ok_default_remote.clone());
            let force = force && !Settings::get(cx).is_protected(&target);
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
                up_to: ok_up_to.clone(),
            };
            run_push(ok_model.clone(), request, cx);
            true
        });
        let (ok_push, button_push, force_push) = (push.clone(), push.clone(), push);

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
                            .child({
                                // IntelliJ's remote link opens the list of remotes.
                                let button = Button::new("push-remote").ghost().xsmall().label(remote.clone());
                                if remotes.len() > 1 {
                                    button
                                        .dropdown_caret(true)
                                        .dropdown_menu(move |mut menu, _, _| {
                                            for (ix, name) in remotes.iter().enumerate() {
                                                let options = remote_options.clone();
                                                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, _| {
                                                    options.borrow_mut().remote = ix;
                                                    window.refresh();
                                                }));
                                            }
                                            menu
                                        })
                                        .into_any_element()
                                } else {
                                    button.into_any_element()
                                }
                            })
                            .child(":")
                            .child(div().w(px(220.)).child(Input::new(&target).xsmall()))
                            .when(new_branch, |el| {
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
                            .child(div().id("push-commits").flex_1().min_w_0().h_full().p_1().overflow_y_scrollbar().child(commits))
                            .child(
                                div()
                                    .id("push-files")
                                    .w(px(260.))
                                    .flex_shrink_0()
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
                            .when(protected, |el| {
                                el.child(div().text_xs().text_color(palette.text_secondary).child("Force push is disabled: protected branch"))
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| ok_push(false, cx))
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("cancel").label("Cancel").outline()))
                    .child(
                        // IntelliJ's Push ▾: Force Push (--force-with-lease) in the dropdown.
                        gpui_kit::component::button::DropdownButton::new("push-split")
                            .primary()
                            .disabled(nothing)
                            .button(Button::new("ok").label("Push").on_click(move |_, window, cx| {
                                if button_push(false, cx) {
                                    window.close_dialog(cx);
                                }
                            }))
                            .dropdown_menu(move |menu, _, _| {
                                let (force_push, force_target, force_branch) = (force_push.clone(), force_target.clone(), force_branch.clone());
                                menu.item(
                                    PopupMenuItem::new(if protected { "Force Push (protected branch)" } else { "Force Push" })
                                        .disabled(protected)
                                        .on_click(move |_, window, cx| {
                                            // IntelliJ asks first: a force push can overwrite
                                            // commits at the remote.
                                            let message = format!(
                                                "You're going to force push \"{force_branch}\" to \"{}\". It may overwrite commits at the remote. Are you sure you want to proceed?",
                                                force_target(cx)
                                            );
                                            // Over the Push dialog: Cancel goes back to it.
                                            let (force_push, message) = (force_push.clone(), SharedString::from(message));
                                            window.open_dialog(cx, move |dialog, _, _| {
                                                let force_push = force_push.clone();
                                                dialog
                                                    .title("Force Push")
                                                    .w(px(460.))
                                                    .child(div().text_sm().child(message.clone()))
                                                    .footer(footer("Force Push"))
                                                    .on_ok(move |_, window, cx| {
                                                        if force_push(true, cx) {
                                                            // The Push dialog goes too.
                                                            window.defer(cx, |window, cx| window.close_dialog(cx));
                                                        }
                                                        true
                                                    })
                                            });
                                        }),
                                )
                            }),
                    ),
            )
    });
}

/// The push a rejection interrupted, for the Push Rejected dialog's
/// Merge / Rebase to finish.
static REJECTED_PUSH: std::sync::Mutex<Option<crate::git::ops::PushRequest>> = std::sync::Mutex::new(None);

/// Runs a push; with "Auto-update if push was rejected" a rejection
/// updates and retries, otherwise it leads to the Push Rejected dialog.
fn run_push(model: Entity<RepoModel>, request: crate::git::ops::PushRequest, cx: &mut App) {
    use crate::git::ops;
    let settings = Settings::get(cx);
    let auto_update = settings.auto_update_on_push_rejected;
    let rebase = settings.update_method == UpdateMethod::Rebase;
    model.update(cx, |model, cx| {
        model.run_operation(if request.force_with_lease { "Force Push" } else { "Push" }, move |repo| {
            if auto_update {
                return ops::push_with_auto_update(repo, &request, rebase);
            }
            ops::push(repo, &request).map_err(|error| {
                if ops::is_rejected(&error) {
                    let message = anyhow::anyhow!("{PUSH_REJECTED} the remote has commits that aren't in {}.", request.branch);
                    *REJECTED_PUSH.lock().unwrap() = Some(request.clone());
                    message
                } else {
                    error
                }
            })
        }, cx)
    });
}

/// How a rejected push's error message starts; the workspace answers it
/// with [`push_rejected`].
pub const PUSH_REJECTED: &str = "Push rejected:";

/// IntelliJ's Push Rejected dialog: the remote has commits the branch
/// doesn't, so merge or rebase onto them and push again.
pub fn push_rejected(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let Some(request) = REJECTED_PUSH.lock().unwrap().take() else { return };
    let settings = Settings::get(cx);
    let clean = if settings.update_shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
    let update = |rebase: bool| -> Rc<dyn Fn(&mut Window, &mut App)> {
        let (model, request) = (model.clone(), request.clone());
        Rc::new(move |_, cx| {
            let request = request.clone();
            model.update(cx, |model, cx| {
                model.run_operation("Push", move |repo| {
                    crate::git::ops::update_after_rejected_push(repo, &request, rebase, clean)
                }, cx)
            });
        })
    };
    // The configured update method is the default button.
    let mut options = vec![("Rebase", true, update(true)), ("Merge", false, update(false))];
    if settings.update_method == UpdateMethod::Rebase {
        options.reverse();
    }
    let message: SharedString = format!(
        "Push of current branch {} was rejected. Remote changes need to be merged before pushing.",
        request.branch
    )
    .into();
    // IntelliJ's "Remember the update method choice and silently update in
    // future": turns on auto-update with the chosen method.
    let remember = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, _, _| {
        let last = options.len().saturating_sub(1);
        let mut footer = DialogFooter::new()
            .gap_2()
            .child(DialogClose::new().child(Button::new("rejected-cancel").label("Cancel").outline()));
        for (ix, (label, rebase, run)) in options.iter().enumerate() {
            let (run, rebase, remember) = (run.clone(), *rebase, remember.clone());
            let button = Button::new(("rejected-option", ix)).label(*label).on_click(move |_, window, cx| {
                window.close_dialog(cx);
                if remember.get() {
                    Settings::update(cx, |s| {
                        s.auto_update_on_push_rejected = true;
                        s.update_method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                    });
                }
                run(window, cx)
            });
            footer = footer.child(if ix == last { button.primary() } else { button.outline() });
        }
        let remember_cell = remember.clone();
        dialog
            .title("Push Rejected")
            .w(px(520.))
            .child(
                v_flex().gap_3().child(div().text_sm().child(message.clone())).child(
                    Checkbox::new("rejected-remember")
                        .label("Remember the update method choice and silently update in future")
                        .checked(remember.get())
                        .on_change(move |value, window, _| {
                            remember_cell.set(*value);
                            window.refresh();
                        }),
                ),
            )
            .footer(footer)
    });
}

/// Update Project (`Ctrl+T`): fetch, then merge or rebase, cleaning the
/// working tree with stash or shelve. Both choices are remembered; after
/// "Don't show again" Ctrl+T updates with them straight away.
pub fn update_project(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let settings = Settings::get(cx);
    if !settings.update_dialog {
        let clean = if settings.update_shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
        run_update(model, settings.update_method == UpdateMethod::Rebase, clean, cx);
        return;
    }
    let rebase = Rc::new(Cell::new(settings.update_method == UpdateMethod::Rebase));
    let shelve = Rc::new(Cell::new(settings.update_shelve));
    let hide = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let (rebase_cell, rebase_ok) = (rebase.clone(), rebase.clone());
        let (shelve_cell, shelve_ok) = (shelve.clone(), shelve.clone());
        let (hide_cell, hide_ok) = (hide.clone(), hide.clone());
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
                    )
                    .child(Checkbox::new("update-hide").label("Don't show again").checked(hide.get()).on_change(move |v, window, _| {
                        hide_cell.set(*v);
                        window.refresh();
                    })),
            )
            .on_ok(move |_, _, cx| {
                let rebase = rebase_ok.get();
                let shelve = shelve_ok.get();
                let method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge };
                let settings = Settings::get(cx);
                if settings.update_method != method || settings.update_shelve != shelve || hide_ok.get() {
                    let hide = hide_ok.get();
                    Settings::update(cx, |s| {
                        s.update_method = method;
                        s.update_shelve = shelve;
                        if hide {
                            s.update_dialog = false;
                        }
                    });
                }
                let clean = if shelve { crate::git::ops::CleanWith::Shelve } else { crate::git::ops::CleanWith::Stash };
                run_update(model.clone(), rebase, clean, cx);
                true
            })
            .footer(footer("OK"))
    });
}

/// Starts an operation's message when it partly failed (some roots of
/// Update Project): the balloon is a warning, not a success.
pub const PARTIAL_FAILURE: char = '\u{1d}';

/// Runs Update Project; multi-root projects update every root, as IntelliJ does.
fn run_update(model: Entity<RepoModel>, rebase: bool, clean: crate::git::ops::CleanWith, cx: &mut App) {
    model.update(cx, |model, cx| {
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
            let partial = !failed.is_empty();
            lines.extend(failed.into_iter().map(|f| format!("{f} (failed)")));
            let message = lines.join("\n");
            Ok(if partial { format!("{PARTIAL_FAILURE}{message}") } else { message })
        }, cx)
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

/// One revision of a file, for Compare with Revision.
#[derive(Clone)]
struct FileRevision {
    hash: String,
    author: String,
    time: i64,
    subject: String,
    /// The file's path in that revision (it may have been renamed since).
    path: String,
}

/// The commits that changed a file, newest first, following renames.
fn file_revisions(repository: &crate::git::Repository, path: &str) -> Vec<FileRevision> {
    let Ok(out) = repository.run(["log", "--follow", "-n", "1000", "--format=%x1e%H%x1f%an%x1f%at%x1f%s", "--name-only", "--", path]) else {
        return Vec::new();
    };
    out.split('\x1e')
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut fields = lines.next()?.split('\x1f');
            let (hash, author, time, subject) = (fields.next()?, fields.next()?, fields.next()?, fields.next().unwrap_or_default());
            let at = lines.map(str::trim).find(|l| !l.is_empty()).unwrap_or(path);
            Some(FileRevision { hash: hash.to_owned(), author: author.to_owned(), time: time.parse().unwrap_or(0), subject: subject.to_owned(), path: at.to_owned() })
        })
        .collect()
}

/// Compare with Revision…: IntelliJ's list of the file's revisions (hash,
/// date, author, message), filtered as you type; the chosen one opens
/// against the local file.
pub fn compare_file_with_revision(model: Entity<RepoModel>, path: String, open_diff: OpenDiff, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::h_flex;
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let revisions = Rc::new(file_revisions(&repository, &path));
    let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by hash, author or message"));
    let focus_target = filter.clone();
    let selected = Rc::new(Cell::new(0usize));
    let last_query = Rc::new(std::cell::RefCell::new(String::new()));
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let query = filter.read(cx).value().trim().to_lowercase();
        // A new filter selects its first match.
        if *last_query.borrow() != query {
            selected.set(0);
            *last_query.borrow_mut() = query.clone();
        }
        let shown: Vec<FileRevision> = revisions
            .iter()
            .filter(|r| query.is_empty() || [&r.hash, &r.author, &r.subject].iter().any(|f| f.to_lowercase().contains(&query)))
            .cloned()
            .collect();
        let current = selected.get().min(shown.len().saturating_sub(1));
        let path = path.clone();
        let source_for = {
            let path = path.clone();
            move |r: &FileRevision| crate::ui::diff_view::DiffSource::Between {
                old: r.hash.clone(),
                new: None,
                path: path.clone(),
                old_path: (r.path != path).then(|| r.path.clone()),
            }
        };
        let mut list = v_flex().id("file-revisions").h(px(300.)).overflow_y_scroll().border_1().border_color(palette.border).rounded_md();
        for (ix, revision) in shown.iter().enumerate() {
            let (select, open, source) = (selected.clone(), open_diff.clone(), source_for(revision));
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("file-revision-{ix}")))
                    .flex_shrink_0()
                    .px_2()
                    .h(px(24.))
                    .gap_3()
                    .text_sm()
                    .when(ix == current, |el| el.bg(palette.selection))
                    .on_click(move |event, window, cx| {
                        select.set(ix);
                        if event.click_count() >= 2 {
                            window.close_dialog(cx);
                            open(source.clone(), window, cx);
                        }
                        window.refresh();
                    })
                    .child(div().w(px(70.)).flex_shrink_0().font_family("monospace").child(revision.hash[..revision.hash.len().min(8)].to_owned()))
                    .child(div().w(px(130.)).flex_shrink_0().text_color(palette.text_secondary).child(crate::ui::common::format_date(revision.time)))
                    .child(div().w(px(110.)).flex_shrink_0().truncate().text_color(palette.text_secondary).child(revision.author.clone()))
                    .child(div().flex_1().min_w_0().truncate().child(revision.subject.clone())),
            );
        }
        if shown.is_empty() {
            list = list.child(div().p_3().text_sm().text_color(palette.text_secondary).child("No revisions"));
        }
        let (select_keys, count) = (selected.clone(), shown.len());
        let chosen = shown.get(current).map(&source_for);
        let open = open_diff.clone();
        dialog
            .title(format!("Compare {path} with Revision"))
            .w(px(720.))
            .child(
                v_flex()
                    .gap_2()
                    // Up / Down move through the list while typing a filter.
                    .child(div().on_key_down(move |event, window, cx| {
                        let step: isize = match event.keystroke.key.as_str() {
                            "up" => -1,
                            "down" => 1,
                            _ => return,
                        };
                        let at = (select_keys.get().min(count.saturating_sub(1)) as isize + step).clamp(0, count.saturating_sub(1) as isize);
                        select_keys.set(at as usize);
                        cx.stop_propagation();
                        window.refresh();
                    }).child(Input::new(&filter)))
                    .child(list),
            )
            .on_ok(move |_, window, cx| {
                let Some(source) = chosen.clone() else { return false };
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

/// Help › About.
pub fn about(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        let palette = cx.palette().clone();
        dialog.title(format!("About {}", crate::ui::workspace::APP_NAME)).w(px(380.)).child(
            v_flex()
                .gap_1()
                .text_sm()
                .child(div().text_lg().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(crate::ui::workspace::APP_NAME))
                .child(div().text_color(palette.text_secondary).child("Git & Code Navigator"))
                .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .child(div().text_color(palette.text_secondary).child("Spell checking uses the SCOWL word list.")),
        )
    });
}

/// IntelliJ's default keymap as this client implements it:
/// (group, action, Windows / Linux, macOS).
pub const KEYMAP: &[(&str, &str, &str, &str)] = &[
    ("Git", "Commit…", "Ctrl+K", "⌘K"),
    ("Git", "Push…", "Ctrl+Shift+K", "⇧⌘K"),
    ("Git", "Update Project…", "Ctrl+T", "⌘T"),
    ("Git", "Branches…", "Ctrl+Shift+`", "⇧⌘`"),
    ("Git", "VCS Operations Popup", "Alt+`", "⌃V"),
    ("Git", "Rollback", "Ctrl+Alt+Z", "⌥⌘Z"),
    ("Git", "Show Diff", "Ctrl+D", "⌘D"),
    ("Git", "Add to VCS", "Ctrl+Alt+A", "⌥⌘A"),
    ("Git", "Move to Another Changelist", "Alt+Shift+M", "⇧⌘M"),
    ("Git", "Commit Message History", "Ctrl+M", "⌃M"),
    ("Git", "Refresh", "Ctrl+Alt+Y", "⌥⌘Y"),
    ("Diff", "Next / Previous Difference", "F7 / Shift+F7", "F7 / ⇧F7"),
    ("Diff", "Compare Next / Previous File", "Alt+Right / Alt+Left", "⌥→ / ⌥←"),
    ("Diff", "Jump to Source", "F4", "⌘↓"),
    ("Editor", "Next / Previous Change", "Ctrl+Alt+Shift+Down / Up", "⌃⌥⇧↓ / ↑"),
    ("Editor", "Go to Declaration", "Ctrl+B", "⌘B"),
    ("Editor", "Find Usages", "Alt+F7", "⌥F7"),
    ("Editor", "Find / Replace", "Ctrl+F / Ctrl+R", "⌘F / ⌘R"),
    ("Editor", "Find Next / Previous", "F3 / Shift+F3", "⌘G / ⇧⌘G"),
    ("Editor", "Show Context Actions (spelling)", "Alt+Enter", "⌥↩"),
    ("Navigate", "Search Everywhere", "Double Shift", "Double ⇧"),
    ("Navigate", "Find Action", "Ctrl+Shift+A", "⇧⌘A"),
    ("Navigate", "Class / File / Symbol", "Ctrl+N / Ctrl+Shift+N / Ctrl+Alt+Shift+N", "⌘O / ⇧⌘O / ⌥⌘O"),
    ("Navigate", "Line/Column", "Ctrl+G", "⌘L"),
    ("Navigate", "Recent Files", "Ctrl+E", "⌘E"),
    ("Navigate", "File Structure", "Ctrl+F12", "⌘F12"),
    ("Navigate", "Back / Forward", "Ctrl+Alt+Left / Right", "⌘[ / ⌘]"),
    ("Navigate", "Select In Project View", "Alt+F1", "⌥F1"),
    ("Navigate", "Find / Replace in Files", "Ctrl+Shift+F / Ctrl+Shift+R", "⇧⌘F / ⇧⌘R"),
    ("Window", "Project / Find / Git / Commit", "Alt+1 / Alt+3 / Alt+9 / Alt+0", "⌘1 / ⌘3 / ⌘9 / ⌘0"),
    ("Window", "Hide Active Tool Window", "Shift+Escape", "⇧⎋"),
    ("Window", "Hide All Tool Windows", "Ctrl+Shift+F12", "⇧⌘F12"),
    ("Window", "Close Tab", "Ctrl+F4", "⌘W"),
    ("Window", "Select Next / Previous Tab", "Alt+Right / Alt+Left", "⇧⌘] / ⇧⌘["),
    ("Window", "Settings", "Ctrl+Alt+S", "⌘,"),
];

/// Help › Keyboard Shortcuts: the table above for this platform.
pub fn keymap_reference(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        let palette = cx.palette().clone();
        use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _};
        let mut list = v_flex().id("keymap-list").max_h(px(520.)).overflow_y_scroll().gap_px().text_sm();
        let mut group = "";
        for (g, action, pc, mac) in KEYMAP {
            if *g != group {
                group = g;
                list = list.child(div().pt_2().pb_0p5().text_xs().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.text_secondary).child(*g));
            }
            let keys = if cfg!(target_os = "macos") { *mac } else { *pc };
            list = list.child(
                gpui_kit::component::h_flex()
                    .gap_4()
                    .child(div().flex_1().child(*action))
                    .child(div().text_color(palette.text_secondary).child(keys)),
            );
        }
        dialog.title("Keyboard Shortcuts").w(px(560.)).child(list)
    });
}

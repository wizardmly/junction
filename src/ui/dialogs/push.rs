//! The Push dialog and what follows a rejected push.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _,
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogClose, DialogFooter},
    input::{Input, InputState},
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};

use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;

use super::footer;

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
    let remote_ix = preview.remotes.iter().position(|r| *r == preview.remote).unwrap_or(0);
    let options = Rc::new(RefCell::new(Options { tags: None, hooks: true, remote: remote_ix, selected: None }));
    let up_to_ok = up_to.clone();
    let outgoing = move |repository: &crate::git::Repository, remote: &str, target: &str| Outgoing::compute(repository, remote, target, up_to.as_deref());
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
        let commits = commit_list(&out, current, &options, &palette, &mono);
        let shown_files = current.selected.and_then(|ix| out.commit_files.get(ix)).unwrap_or(&out.all_files);
        let files = file_list(shown_files, &palette);
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
                                        .on_click(move |_, window, cx| confirm_force_push(&force_branch, &force_target(cx), force_push.clone(), window, cx)),
                                )
                            }),
                    ),
            )
    });
}

/// Push dialog state the controls change.
#[derive(Clone, Copy)]
struct Options {
    tags: Option<usize>,
    hooks: bool,
    remote: usize,
    /// The commit whose files the change tree shows; all commits when `None`.
    selected: Option<usize>,
}

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

impl Outgoing {
    /// The commits `remote`/`target` doesn't have (from `up_to` down, when
    /// given) and the files each one changes.
    fn compute(repository: &crate::git::Repository, remote: &str, target: &str, up_to: Option<&str>) -> Self {
        let (new_branch, mut commits) = crate::git::ops::push_commits(repository, remote, target).unwrap_or_default();
        // Newest first: the ones above `up_to` stay behind.
        if let Some(pos) = up_to.and_then(|h| commits.iter().position(|c| c.hash == h)) {
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
    }
}

/// The outgoing commits; a click selects one to show only its files.
fn commit_list(
    out: &Outgoing,
    current: Options,
    options: &Rc<std::cell::RefCell<Options>>,
    palette: &crate::theme::Palette,
    mono: &SharedString,
) -> gpui_kit::Div {
    use gpui_kit::component::h_flex;
    use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _, prelude::FluentBuilder as _};
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
    commits
}

/// The files the shown commits change.
fn file_list(shown_files: &[crate::git::log::FileChange], palette: &crate::theme::Palette) -> gpui_kit::Div {
    use gpui_kit::component::h_flex;
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
    files
}

/// Asks before a force push (it can overwrite commits at the remote), over
/// the Push dialog: Cancel goes back to it.
fn confirm_force_push(force_branch: &str, force_target: &str, force_push: Rc<dyn Fn(bool, &mut App) -> bool>, window: &mut Window, cx: &mut App) {
    let message = SharedString::from(format!(
        "You're going to force push \"{force_branch}\" to \"{force_target}\". It may overwrite commits at the remote. Are you sure you want to proceed?"
    ));
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

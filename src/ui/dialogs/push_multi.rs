//! The Push dialog of a multi-root project: IntelliJ's tree with one node
//! per repository (checkbox, branch → remote : target) and its outgoing
//! commits under it; Push sends every checked repository.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogClose, DialogFooter},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::Repository;
use crate::git::ops::{self, PushRequest, PushTags};
use crate::model::RepoModel;
use crate::settings::Settings;
use crate::theme::ActivePalette as _;

use super::push::{Outgoing, file_list, run_push_all};

/// One repository's node.
struct RootPush {
    repository: Repository,
    label: String,
    branch: String,
    remotes: Vec<String>,
    remote: usize,
    default_remote: String,
    target: Entity<InputState>,
    has_upstream: bool,
    checked: bool,
    out: Outgoing,
}

/// What the change tree on the right shows.
#[derive(Clone, Copy, PartialEq)]
enum Shown {
    Root(usize),
    Commit(usize, usize),
}

struct State {
    roots: Vec<RootPush>,
    shown: Option<Shown>,
    tags: Option<usize>,
    hooks: bool,
}

/// Roots that can't push (detached HEAD, no remote) are listed greyed out
/// with the reason, as IntelliJ lists them.
pub fn push_multi(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let m = model.read(cx);
    let repositories = m.repositories();
    let labels: Vec<String> = repositories.iter().map(|r| m.root_label(r.root())).collect();
    let active = m.repository().map(|r| r.root().to_path_buf());
    let project = m.project_root().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut roots = Vec::new();
    let mut skipped = Vec::new();
    for (repository, label) in repositories.into_iter().zip(labels) {
        let preview = match ops::push_preview(&repository) {
            Ok(p) => p,
            Err(error) => {
                skipped.push((label, error.to_string()));
                continue;
            }
        };
        let Some(branch) = preview.branch.clone() else {
            skipped.push((label, "HEAD is detached".into()));
            continue;
        };
        if preview.remotes.is_empty() {
            skipped.push((label, "No remotes".into()));
            continue;
        }
        let out = Outgoing::compute(&repository, &preview.remote, &preview.target, None);
        // IntelliJ checks the repositories that have something to push.
        let checked = !out.commits.is_empty() || out.new_branch;
        let target = cx.new(|cx| InputState::new(window, cx).default_value(preview.target.clone()));
        let remote = preview.remotes.iter().position(|r| *r == preview.remote).unwrap_or(0);
        roots.push(RootPush {
            repository,
            label,
            branch,
            remotes: preview.remotes,
            remote,
            default_remote: preview.remote,
            target,
            has_upstream: preview.has_upstream,
            checked,
            out,
        });
    }
    if roots.is_empty() {
        let reason = skipped.iter().map(|(l, r)| format!("{l}: {r}")).collect::<Vec<_>>().join("\n");
        window.push_notification(gpui_kit::component::notification::Notification::warning(format!("Nothing to push\n{reason}")), cx);
        return;
    }
    // The active root's node is shown first in the change tree.
    let first = roots.iter().position(|r| Some(r.repository.root()) == active.as_deref()).unwrap_or(0);
    let state = Rc::new(RefCell::new(State { roots, shown: Some(Shown::Root(first)), tags: None, hooks: true }));

    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        // The target branch is re-checked as it is edited ("New", commits).
        {
            let mut s = state.borrow_mut();
            let mut reset = false;
            for root in &mut s.roots {
                let remote = root.remotes.get(root.remote).cloned().unwrap_or_else(|| root.default_remote.clone());
                let target = root.target.read(cx).value().trim().to_owned();
                if root.out.remote != remote || root.out.target != target {
                    root.out = Outgoing::compute(&root.repository, &remote, &target, None);
                    reset = true;
                }
            }
            if reset && matches!(s.shown, Some(Shown::Commit(..))) {
                s.shown = None;
            }
        }
        let s = state.borrow();
        let mut tree = v_flex().gap_px();
        for (ix, root) in s.roots.iter().enumerate() {
            let remote = root.remotes.get(root.remote).cloned().unwrap_or_else(|| root.default_remote.clone());
            let selected = s.shown == Some(Shown::Root(ix));
            let (check_state, select_state, remote_state) = (state.clone(), state.clone(), state.clone());
            let remotes = root.remotes.clone();
            tree = tree.child(
                h_flex()
                    .id(("push-root", ix))
                    .h(px(26.))
                    .px_1()
                    .gap_1p5()
                    .text_sm()
                    .rounded(px(3.))
                    .when(selected, |el| el.bg(palette.selection))
                    .on_click(move |_, window, _| {
                        select_state.borrow_mut().shown = Some(Shown::Root(ix));
                        window.refresh();
                    })
                    .child(Checkbox::new(("push-root-check", ix)).checked(root.checked).on_change(move |v, window, _| {
                        check_state.borrow_mut().roots[ix].checked = *v;
                        window.refresh();
                    }))
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(format!("{}:", root.label)))
                    .child(div().child(root.branch.clone()))
                    .child("→")
                    .child({
                        let button = Button::new(("push-root-remote", ix)).ghost().xsmall().label(remote.clone());
                        if remotes.len() > 1 {
                            button
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (rix, name) in remotes.iter().enumerate() {
                                        let state = remote_state.clone();
                                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, _| {
                                            state.borrow_mut().roots[ix].remote = rix;
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
                    .child(div().w(px(160.)).child(Input::new(&root.target).xsmall()))
                    .when(root.out.new_branch, |el| {
                        el.child(div().px_1().rounded(px(3.)).bg(palette.status_added).text_xs().text_color(gpui_kit::white()).child("New"))
                    }),
            );
            if root.out.commits.is_empty() {
                tree = tree.child(div().pl(px(30.)).h(px(22.)).text_sm().text_color(palette.text_secondary).child("Nothing to push"));
            }
            for (cix, commit) in root.out.commits.iter().enumerate() {
                let is_selected = s.shown == Some(Shown::Commit(ix, cix));
                let select = state.clone();
                tree = tree.child(
                    h_flex()
                        .id(SharedString::from(format!("push-root-{ix}-commit-{cix}")))
                        .h(px(22.))
                        .pl(px(30.))
                        .pr_1()
                        .gap_2()
                        .rounded(px(3.))
                        .text_sm()
                        .cursor_pointer()
                        .overflow_hidden()
                        .when(is_selected, |el| el.bg(palette.selection))
                        .on_click(move |_, window, _| {
                            select.borrow_mut().shown = Some(Shown::Commit(ix, cix));
                            window.refresh();
                        })
                        .child(div().flex_shrink_0().font_family(mono.clone()).text_color(palette.text_secondary).child(commit.short_hash().to_owned()))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(commit.subject.clone()))
                        .child(div().flex_shrink_0().text_color(palette.text_secondary).child(commit.author_name.to_string())),
                );
            }
        }
        for (label, reason) in &skipped {
            tree = tree.child(
                h_flex()
                    .h(px(26.))
                    .px_1()
                    .gap_1p5()
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(format!("{label}:")))
                    .child(reason.clone()),
            );
        }
        let shown_files = match s.shown {
            Some(Shown::Root(ix)) => s.roots.get(ix).map(|r| r.out.all_files.clone()).unwrap_or_default(),
            Some(Shown::Commit(ix, cix)) => s.roots.get(ix).and_then(|r| r.out.commit_files.get(cix).cloned()).unwrap_or_default(),
            None => Vec::new(),
        };
        let files = file_list(&shown_files, &palette);
        let tags = s.tags;
        let hooks = s.hooks;
        // Something to send in a checked root (tags count too).
        let nothing = !s.roots.iter().any(|r| r.checked && (!r.out.commits.is_empty() || r.out.new_branch || tags.is_some()));
        let protected: Vec<String> = s
            .roots
            .iter()
            .filter(|r| r.checked)
            .map(|r| r.target.read(cx).value().trim().to_owned())
            .filter(|t| Settings::get(cx).is_protected(t))
            .collect();
        drop(s);

        let push = {
            let (state, model) = (state.clone(), model.clone());
            Rc::new(move |force: bool, cx: &mut App| -> bool {
                if nothing {
                    return false;
                }
                let s = state.borrow();
                let mut requests: Vec<(Repository, PushRequest)> = Vec::new();
                for root in s.roots.iter().filter(|r| r.checked) {
                    let target = root.target.read(cx).value().trim().to_owned();
                    if target.is_empty() {
                        continue;
                    }
                    let remote = root.remotes.get(root.remote).cloned().unwrap_or_else(|| root.default_remote.clone());
                    let force = force && !Settings::get(cx).is_protected(&target);
                    requests.push((root.repository.clone(), PushRequest {
                        remote,
                        branch: root.branch.clone(),
                        target,
                        force_with_lease: force,
                        tags: match s.tags {
                            None => PushTags::None,
                            Some(0) => PushTags::All,
                            Some(_) => PushTags::CurrentBranch,
                        },
                        set_upstream: !root.has_upstream,
                        run_hooks: s.hooks,
                        up_to: None,
                    }));
                }
                drop(s);
                if requests.is_empty() {
                    return false;
                }
                run_push_all(model.clone(), requests, cx);
                true
            })
        };
        let (ok_push, button_push, force_push) = (push.clone(), push.clone(), push);
        let (tags_state, tags_mode_state, hooks_state) = (state.clone(), state.clone(), state.clone());
        let force_names: Vec<String> = {
            let s = state.borrow();
            s.roots.iter().filter(|r| r.checked).map(|r| format!("{} ({})", r.label, r.branch)).collect()
        };

        dialog
            .title(format!("Push Commits to {project}"))
            .w(px(820.))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .h(px(300.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(palette.border)
                            .child(div().id("push-tree").flex_1().min_w_0().h_full().p_1().overflow_y_scrollbar().child(tree))
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
                            .child(Checkbox::new("push-tags").label("Push tags:").checked(tags.is_some()).on_change(move |v, window, _| {
                                tags_state.borrow_mut().tags = v.then_some(0);
                                window.refresh();
                            }))
                            .when(tags.is_some(), |el| {
                                el.child(RadioGroup::horizontal("push-tags-mode").children(["All", "Current Branch"]).selected_index(tags).on_change(
                                    move |ix, window, _| {
                                        tags_mode_state.borrow_mut().tags = Some(*ix);
                                        window.refresh();
                                    },
                                ))
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(Checkbox::new("push-hooks").label("Run Git hooks").checked(hooks).on_change(move |v, window, _| {
                                hooks_state.borrow_mut().hooks = *v;
                                window.refresh();
                            }))
                            .when(!protected.is_empty(), |el| {
                                el.child(div().text_xs().text_color(palette.text_secondary).child(format!("Force push is disabled for protected branch {}", protected.join(", "))))
                            }),
                    ),
            )
            .on_ok(move |_, _, cx| ok_push(false, cx))
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("cancel").label("Cancel").outline()))
                    .child(
                        gpui_kit::component::button::DropdownButton::new("push-split")
                            .primary()
                            .disabled(nothing)
                            .button(Button::new("ok").label("Push").on_click(move |_, window, cx| {
                                if button_push(false, cx) {
                                    window.close_dialog(cx);
                                }
                            }))
                            .dropdown_menu(move |menu, _, _| {
                                let (force_push, names) = (force_push.clone(), force_names.join(", "));
                                menu.item(PopupMenuItem::new("Force Push").on_click(move |_, window, cx| {
                                    super::push::confirm_force_push(&names, "their remote branches", force_push.clone(), window, cx)
                                }))
                            }),
                    ),
            )
    });
}

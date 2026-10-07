//! The branches popup opened from the title bar's VCS widget
//! (IntelliJ: Git › Branches…, `Ctrl+Shift+\``). Clicking a branch expands
//! its actions inline, where IntelliJ shows them in a side submenu.

use std::rc::Rc;

use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::{RefKind, RefName, status};
use crate::model::RepoModel;
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::dialogs;

pub(crate) type Run = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct BranchAction {
    pub label: String,
    pub enabled: bool,
    pub run: Run,
}

pub struct BranchesPopup {
    model: Entity<RepoModel>,
    search: Entity<InputState>,
    expanded: Option<String>,
    /// Collapsed group headers; Tags starts collapsed, as in IntelliJ.
    collapsed: std::collections::HashSet<&'static str>,
    /// Commit… focuses the Commit tool window, which the workspace owns.
    pub on_commit: Option<Run>,
    _subscriptions: Vec<Subscription>,
}

impl BranchesPopup {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search for branches and actions"));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
        ];
        Self { model, search, expanded: None, collapsed: ["Tags"].into_iter().collect(), on_commit: None, _subscriptions: subscriptions }
    }
}

fn git_op(model: &Entity<RepoModel>, title: &'static str, args: Vec<String>, done: String) -> Run {
    let model = model.clone();
    Rc::new(move |_, cx| {
        let args = args.clone();
        let done = done.clone();
        model.update(cx, |model, cx| {
            model.run_operation(title, move |repo| {
                repo.run(&args)?;
                Ok(done)
            }, cx)
        });
    })
}

/// Runs `op` in each other root; returns how many succeeded.
pub(crate) fn sync_to_roots(repo: &crate::git::Repository, roots: &[std::path::PathBuf], op: impl Fn(&crate::git::Repository) -> bool) -> usize {
    roots
        .iter()
        .filter_map(|root| repo.nested(&root.to_string_lossy()).ok())
        .filter(|other| op(other))
        .count()
}

fn action(label: impl Into<String>, enabled: bool, run: Run) -> BranchAction {
    BranchAction { label: label.into(), enabled, run }
}

/// The actions IntelliJ offers for a branch, in its order.
pub(crate) fn branch_actions(model: &Entity<RepoModel>, reference: &RefName, current: Option<&str>, remotes: &[String]) -> Vec<BranchAction> {
    let name = reference.name.clone();
    let is_current = reference.kind == RefKind::LocalBranch && Some(name.as_str()) == current;
    let current_name = current.unwrap_or("HEAD").to_owned();
    let mut actions = Vec::new();

    let checkout_ref = reference.clone();
    let checkout_model = model.clone();
    actions.push(action(
        "Checkout",
        !is_current,
        Rc::new(move |_, cx| {
            let reference = checkout_ref.clone();
            // Synchronous branch control: the same branch in every root that has it.
            let others = if crate::settings::Settings::get(cx).sync_branches { checkout_model.read(cx).other_roots() } else { Vec::new() };
            checkout_model.update(cx, |model, cx| {
                model.run_operation("Checkout", move |repo| {
                    status::checkout(repo, &reference)?;
                    let synced = sync_to_roots(repo, &others, |other| {
                        other.run(["rev-parse", "--verify", "-q", &reference.full_name]).is_ok() && status::checkout(other, &reference).is_ok()
                    });
                    Ok(match synced {
                        0 => format!("Checked out {}", reference.name),
                        n => format!("Checked out {} in {} repositories", reference.name, n + 1),
                    })
                }, cx)
            });
        }),
    ));
    let new_model = model.clone();
    let new_from = name.clone();
    actions.push(action(
        format!("New Branch from '{name}'…"),
        true,
        Rc::new(move |window, cx| dialogs::new_branch(new_model.clone(), new_from.clone(), window, cx)),
    ));
    if !is_current {
        // Shows the commits this branch has that the current one doesn't.
        let compare_model = model.clone();
        let range = format!("{current_name}..{name}");
        actions.push(action(
            format!("Compare with '{current_name}'"),
            true,
            Rc::new(move |_, cx| {
                let range = range.clone();
                compare_model.update(cx, |model, cx| {
                    let filter = crate::git::LogFilter { branches: vec![range.clone()], ..Default::default() };
                    model.open_log_tab(range, filter, cx);
                });
            }),
        ));
        let diff_model = model.clone();
        let diff_name = name.clone();
        actions.push(action(
            "Show Diff with Working Tree",
            true,
            Rc::new(move |_, cx| {
                let name = diff_name.clone();
                diff_model.update(cx, |model, cx| model.compare(name, None, cx));
            }),
        ));
        actions.push(action(
            format!("Checkout and Rebase onto '{current_name}'"),
            reference.kind == RefKind::LocalBranch,
            git_op(model, "Checkout and Rebase", vec!["rebase".into(), current_name.clone(), name.clone()], format!("Rebased {name} onto {current_name}")),
        ));
        actions.push(action(
            format!("Rebase '{current_name}' onto '{name}'"),
            true,
            git_op(model, "Rebase", vec!["rebase".into(), name.clone()], format!("Rebased {current_name} onto {name}")),
        ));
        actions.push(action(
            format!("Merge '{name}' into '{current_name}'"),
            true,
            git_op(model, "Merge", vec!["merge".into(), name.clone()], format!("Merged {name} into {current_name}")),
        ));
    }
    match reference.kind {
        RefKind::RemoteBranch => {
            let branch = reference.branch_without_remote().to_owned();
            let remote = reference.remote().unwrap_or("origin").to_owned();
            actions.push(action(
                format!("Pull into '{current_name}' Using Rebase"),
                true,
                git_op(model, "Pull", vec!["pull".into(), "--rebase".into(), remote.clone(), branch.clone()], format!("Pulled {name} with rebase")),
            ));
            actions.push(action(
                format!("Pull into '{current_name}' Using Merge"),
                true,
                git_op(model, "Pull", vec!["pull".into(), "--no-rebase".into(), remote.clone(), branch.clone()], format!("Pulled {name} with merge")),
            ));
            actions.push(action(
                "Delete",
                true,
                git_op(model, "Delete Remote Branch", vec!["push".into(), remote, "--delete".into(), branch], format!("Deleted remote branch {name}")),
            ));
        }
        RefKind::LocalBranch => {
            if let Some(upstream) = &reference.upstream {
                let (remote, remote_branch) = upstream.split_once('/').unwrap_or(("origin", upstream.as_str()));
                let args = if is_current {
                    vec!["pull".into(), "--ff-only".into()]
                } else {
                    vec!["fetch".into(), remote.to_owned(), format!("{remote_branch}:{name}")]
                };
                actions.push(action("Update", true, git_op(model, "Update", args, format!("Updated {name}"))));
            }
            let push_args = if reference.upstream.is_some() {
                vec!["push".into(), "origin".into(), name.clone()]
            } else {
                vec!["push".into(), "-u".into(), "origin".into(), name.clone()]
            };
            actions.push(action("Push…", true, git_op(model, "Push", push_args, format!("Pushed {name}"))));
            let rename_model = model.clone();
            let rename_name = name.clone();
            actions.push(action(
                "Rename…",
                true,
                Rc::new(move |window, cx| dialogs::rename_branch(rename_model.clone(), rename_name.clone(), window, cx)),
            ));
            let tracking_model = model.clone();
            let (tracking_name, tracking_upstream) = (name.clone(), reference.upstream.clone());
            actions.push(action(
                "Edit Tracking Branch…",
                true,
                Rc::new(move |window, cx| {
                    crate::ui::remote_dialogs::edit_tracking_branch(tracking_model.clone(), tracking_name.clone(), tracking_upstream.clone(), window, cx)
                }),
            ));
            actions.push(action(
                "Delete",
                !is_current,
                git_op(model, "Delete Branch", vec!["branch".into(), "-d".into(), name.clone()], format!("Deleted branch {name}")),
            ));
        }
        RefKind::Tag => {
            for remote in remotes {
                actions.push(action(
                    format!("Push to {remote}"),
                    true,
                    git_op(model, "Push Tag", vec!["push".into(), remote.clone(), format!("refs/tags/{name}")], format!("Pushed tag {name} to {remote}")),
                ));
            }
            actions.push(action(
                "Delete",
                true,
                git_op(model, "Delete Tag", vec!["tag".into(), "-d".into(), name.clone()], format!("Deleted tag {name}")),
            ));
            for remote in remotes {
                actions.push(action(
                    format!("Delete on {remote}"),
                    true,
                    git_op(model, "Delete Tag on Remote", vec!["push".into(), remote.clone(), "--delete".into(), format!("refs/tags/{name}")], format!("Deleted tag {name} on {remote}")),
                ));
            }
        }
    }
    actions
}

fn row(id: impl Into<SharedString>, palette: &Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    let hover = palette.hover;
    h_flex()
        .id(gpui_kit::ElementId::Name(id.into()))
        .h(px(26.))
        .px_2()
        .gap_1p5()
        .rounded(px(4.))
        .text_sm()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

impl BranchesPopup {
    pub fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.focus(window, cx));
    }

    pub fn reset_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded = None;
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
    }
}

impl gpui_kit::EventEmitter<gpui_kit::DismissEvent> for BranchesPopup {}

impl Render for BranchesPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let query = self.search.read(cx).value().to_lowercase();
        let model = self.model.read(cx);
        let refs = model.refs().clone();
        let current = refs.current_branch.clone();
        let remote_names = remote_names(&refs);
        let head = refs.head_commit.clone().unwrap_or_default();
        let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        let entity = cx.entity();

        let top_action = |id: &'static str, icon: IconName, label: &'static str, run: Run| {
            row(id, &palette)
                .child(Icon::new(icon).small().text_color(palette.text_secondary))
                .child(label)
                .on_click({
                    let entity = entity.clone();
                    move |_, window, cx| {
                        entity.update(cx, |_, cx| cx.emit(gpui_kit::DismissEvent));
                        run(window, cx)
                    }
                })
        };

        let searching = !query.is_empty();
        let collapsed = self.collapsed.clone();
        let section = |title: &'static str, count: usize| {
            let entity = entity.clone();
            let is_collapsed = collapsed.contains(title) && !searching;
            h_flex()
                .id(SharedString::from(format!("bp-group-{title}")))
                .px_2()
                .pt_2()
                .pb_0p5()
                .gap_1()
                .text_xs()
                .text_color(palette.text_secondary)
                .cursor_pointer()
                .child(Icon::new(if is_collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).xsmall())
                .child(title)
                .when(is_collapsed, |el| el.child(div().child(format!("({count})"))))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        if !this.collapsed.remove(title) {
                            this.collapsed.insert(title);
                        }
                        cx.notify();
                    })
                })
        };

        let mut list = v_flex().gap_px();
        if query.is_empty() {
            let new_model = self.model.clone();
            let new_head = head.clone();
            list = list
                .child(top_action("bp-update", IconName::ArrowDownToLine, "Update Project…", {
                    let model = self.model.clone();
                    Rc::new(move |window, cx| dialogs::update_project(model.clone(), window, cx))
                }))
                .when_some(self.on_commit.clone(), |list, run| list.child(top_action("bp-commit", IconName::Check, "Commit…", run)))
                .child(top_action("bp-push", IconName::ArrowUpFromLine, "Push…", {
                    let model = self.model.clone();
                    Rc::new(move |window, cx| dialogs::push(model.clone(), window, cx))
                }))
                .child(top_action(
                    "bp-new",
                    IconName::Plus,
                    "New Branch…",
                    Rc::new(move |window, cx| dialogs::new_branch(new_model.clone(), new_head.clone(), window, cx)),
                ))
                .child(top_action("bp-checkout-rev", IconName::Tag, "Checkout Tag or Revision…", {
                    let model = self.model.clone();
                    Rc::new(move |window, cx| dialogs::checkout_revision(model.clone(), window, cx))
                }));
            // Multi-root projects: pick the repository whose branches are listed
            // (and that branch operations target), plus synchronous control.
            if model.is_multi_root() {
                let project = model.project_root().map(std::path::Path::to_path_buf).unwrap_or_default();
                let active = model.repository().map(|r| r.root().to_path_buf());
                list = list.child(
                    div().px_2().pt_2().pb_0p5().text_xs().text_color(palette.text_secondary).child("Repositories"),
                );
                for (ix, root) in model.roots().iter().enumerate() {
                    let is_active = active.as_ref() == Some(&root.path);
                    let path = root.path.clone();
                    let switch_model = self.model.clone();
                    list = list.child(
                        row(SharedString::from(format!("bp-root-{ix}")), &palette)
                            .child(Icon::new(IconName::FolderGit2).small().text_color(palette.text_secondary))
                            .child(div().when(is_active, |el| el.font_weight(gpui_kit::FontWeight::SEMIBOLD)).child(crate::git::roots::label(&project, &root.path)))
                            .child(div().flex_1())
                            .child(
                                h_flex()
                                    .gap_0p5()
                                    .text_xs()
                                    .text_color(palette.text_secondary)
                                    .child(Icon::new(IconName::GitBranch).xsmall())
                                    .child(root.branch.clone().unwrap_or_else(|| "detached".into())),
                            )
                            .when(is_active, |el| el.child(Icon::new(IconName::Check).xsmall().text_color(palette.accent)))
                            .on_click(move |_, _, cx| {
                                let path = path.clone();
                                switch_model.update(cx, |m, cx| m.switch_root(path, cx));
                            }),
                    );
                }
                let sync = crate::settings::Settings::get(cx).sync_branches;
                list = list.child(
                    row("bp-sync", &palette)
                        .child(Icon::new(if sync { IconName::Check } else { IconName::Circle }).xsmall().text_color(if sync { palette.accent } else { gpui_kit::transparent_black() }))
                        .child(div().text_color(palette.text_secondary).child("Execute branch operations on all roots"))
                        .on_click(move |_, _, cx| crate::settings::Settings::update(cx, |s| s.sync_branches = !s.sync_branches)),
                );
            }
        }

        let favorites = refs.favorites.clone();
        let add_group = |list: gpui_kit::Div, title: &'static str, mut refs_in_group: Vec<RefName>| -> gpui_kit::Div {
            if refs_in_group.is_empty() {
                return list;
            }
            if title != "Recent" {
                // Favorites first, the rest keeps its order.
                refs_in_group.sort_by_key(|r| !favorites.contains(&r.full_name));
            }
            let count = refs_in_group.len();
            let mut list = list.child(section(title, count));
            if collapsed.contains(title) && !searching {
                return list;
            }
            for reference in refs_in_group {
                let is_current = reference.kind == RefKind::LocalBranch && Some(&reference.name) == current.as_ref();
                let color: Hsla = match reference.kind {
                    _ if is_current => palette.ref_head,
                    RefKind::RemoteBranch => palette.ref_remote,
                    RefKind::Tag => palette.ref_tag,
                    _ => palette.ref_local,
                };
                // Keyed by group too: a branch can be listed under Recent and Local.
                let key = format!("{title}\0{}", reference.full_name);
                let expanded = self.expanded.as_deref() == Some(key.as_str());
                let toggle_name = key;
                let toggle_entity = entity.clone();
                let is_favorite = favorites.contains(&reference.full_name);
                let star_model = self.model.clone();
                let star_name = reference.full_name.clone();
                let star = div()
                    .id(SharedString::from(format!("star-{title}-{}", reference.full_name)))
                    .px_0p5()
                    .child(
                        Icon::new(if is_favorite { IconName::StarOff } else { IconName::Star })
                            .xsmall()
                            .text_color(palette.text_secondary),
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let name = star_name.clone();
                        star_model.update(cx, |model, cx| {
                            if let Some(repo) = model.repository() {
                                let _ = crate::git::refs::set_favorite(repo, &name, !is_favorite);
                            }
                            model.reload(cx);
                        });
                    });
                list = list.child(
                    row(format!("branch-{title}-{}", reference.full_name), &palette)
                        .when(expanded, |el| el.bg(palette.selection))
                        .child(
                            Icon::new(if is_favorite {
                                IconName::Star
                            } else if reference.kind == RefKind::Tag {
                                IconName::Tag
                            } else {
                                IconName::GitBranch
                            })
                            .small()
                            .text_color(if is_favorite { palette.ref_head } else { color }),
                        )
                        .child(div().when(is_current, |el| el.font_weight(FontWeight::SEMIBOLD)).child(reference.name.clone()))
                        .when(reference.behind > 0, |el| el.child(div().text_xs().text_color(palette.link).child(format!("↓{}", reference.behind))))
                        .when(reference.ahead > 0, |el| el.child(div().text_xs().text_color(palette.status_added).child(format!("↑{}", reference.ahead))))
                        .child(div().flex_1())
                        .when_some(reference.upstream.clone(), |el, upstream| {
                            el.child(div().text_xs().text_color(palette.text_secondary).child(upstream))
                        })
                        .child(star)
                        .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall().text_color(palette.text_secondary))
                        .on_click(move |_, _, cx| {
                            let name = toggle_name.clone();
                            toggle_entity.update(cx, |this, cx| {
                                this.expanded = if this.expanded.as_deref() == Some(name.as_str()) { None } else { Some(name) };
                                cx.notify();
                            });
                        }),
                );
                if expanded {
                    for (ix, branch_action) in branch_actions(&self.model, &reference, current.as_deref(), &remote_names).into_iter().enumerate() {
                        let run = branch_action.run.clone();
                        let close_entity = entity.clone();
                        list = list.child(
                            row(format!("branch-action-{title}-{}-{ix}", reference.full_name), &palette)
                                .pl(px(30.))
                                .when(!branch_action.enabled, |el| el.text_color(palette.text_disabled).cursor_default())
                                .child(branch_action.label)
                                .when(branch_action.enabled, |el| {
                                    el.on_click(move |_, window, cx| {
                                        close_entity.update(cx, |this, cx| {
                                            this.expanded = None;
                                            cx.emit(gpui_kit::DismissEvent);
                                            cx.notify();
                                        });
                                        run(window, cx)
                                    })
                                }),
                        );
                    }
                }
            }
            list
        };

        if !searching {
            let recent: Vec<RefName> = refs
                .recent
                .iter()
                .filter(|name| Some(*name) != current.as_ref())
                .filter_map(|name| refs.find(&format!("refs/heads/{name}")).cloned())
                .collect();
            list = add_group(list, "Recent", recent);
        }
        list = add_group(list, "Local", refs.local_branches().filter(|r| matches(&r.name)).cloned().collect());
        list = add_group(list, "Remote", refs.remote_branches().filter(|r| matches(&r.name)).cloned().collect());
        {
            list = add_group(list, "Tags", refs.tags().filter(|r| matches(&r.name)).cloned().collect());
        }

        v_flex()
            .w(px(400.))
            .gap_1()
            .child(Input::new(&self.search).small().prefix(Icon::new(IconName::Search).xsmall()))
            .child(div().id("branches-popup-list").max_h(px(520.)).overflow_y_scrollbar().child(list))
    }
}

/// Remotes tags push to / delete on, as IntelliJ lists them per remote.
pub(crate) fn remote_names(refs: &crate::git::RepositoryRefs) -> Vec<String> {
    let mut names: Vec<String> = refs.remote_branches().filter_map(|r| r.remote().map(str::to_owned)).collect();
    names.dedup();
    if names.is_empty() {
        names.push("origin".into());
    }
    names
}

pub fn branch_widget_label(model: &RepoModel) -> String {
    use crate::git::RepositoryState::*;
    let refs = model.refs();
    let branch = refs
        .current_branch
        .clone()
        .or_else(|| refs.head_commit.as_ref().map(|h| h[..8.min(h.len())].to_owned()))
        .unwrap_or_else(|| "No branch".into());
    // Several roots: say which repository the branch belongs to.
    let branch = match model.active_root_label().filter(|_| model.is_multi_root()) {
        Some(root) => format!("{root}: {branch}"),
        None => branch,
    };
    match model.state() {
        Normal => branch,
        Merging => format!("Merging {branch}"),
        Rebasing => format!("Rebasing {branch}"),
        CherryPicking => format!("Cherry-picking in {branch}"),
        Reverting => format!("Reverting in {branch}"),
    }
}

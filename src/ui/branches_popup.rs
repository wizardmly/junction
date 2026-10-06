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

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

struct BranchAction {
    label: String,
    enabled: bool,
    run: Run,
}

pub struct BranchesPopup {
    model: Entity<RepoModel>,
    search: Entity<InputState>,
    expanded: Option<String>,
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
        Self { model, search, expanded: None, _subscriptions: subscriptions }
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

fn action(label: impl Into<String>, enabled: bool, run: Run) -> BranchAction {
    BranchAction { label: label.into(), enabled, run }
}

/// The actions IntelliJ offers for a branch, in its order.
fn branch_actions(model: &Entity<RepoModel>, reference: &RefName, current: Option<&str>) -> Vec<BranchAction> {
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
            checkout_model.update(cx, |model, cx| {
                model.run_operation("Checkout", move |repo| {
                    status::checkout(repo, &reference)?;
                    Ok(format!("Checked out {}", reference.name))
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
            actions.push(action(
                "Delete",
                !is_current,
                git_op(model, "Delete Branch", vec!["branch".into(), "-d".into(), name.clone()], format!("Deleted branch {name}")),
            ));
        }
        RefKind::Tag => {}
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

impl Render for BranchesPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let query = self.search.read(cx).value().to_lowercase();
        let model = self.model.read(cx);
        let refs = model.refs().clone();
        let current = refs.current_branch.clone();
        let head = refs.head_commit.clone().unwrap_or_default();
        let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        let entity = cx.entity();

        let top_action = |id: &'static str, icon: IconName, label: &'static str, run: Run| {
            row(id, &palette)
                .child(Icon::new(icon).small().text_color(palette.text_secondary))
                .child(label)
                .on_click(move |_, window, cx| run(window, cx))
        };

        let section = |title: &'static str| {
            div().px_2().pt_2().pb_0p5().text_xs().text_color(palette.text_secondary).child(title)
        };

        let mut list = v_flex().gap_px();
        if query.is_empty() {
            let new_model = self.model.clone();
            let new_head = head.clone();
            list = list
                .child(top_action(
                    "bp-update",
                    IconName::ArrowDownToLine,
                    "Update Project…",
                    git_op(&self.model, "Update Project", vec!["pull".into(), "--rebase".into(), "--autostash".into()], "Project updated".into()),
                ))
                .child(top_action("bp-push", IconName::ArrowUpFromLine, "Push…", git_op(&self.model, "Push", vec!["push".into()], "Pushed".into())))
                .child(top_action(
                    "bp-new",
                    IconName::Plus,
                    "New Branch…",
                    Rc::new(move |window, cx| dialogs::new_branch(new_model.clone(), new_head.clone(), window, cx)),
                ));
        }

        let add_group = |list: gpui_kit::Div, title: &'static str, refs_in_group: Vec<RefName>| -> gpui_kit::Div {
            if refs_in_group.is_empty() {
                return list;
            }
            let mut list = list.child(section(title));
            for reference in refs_in_group {
                let is_current = reference.kind == RefKind::LocalBranch && Some(&reference.name) == current.as_ref();
                let color: Hsla = match reference.kind {
                    _ if is_current => palette.ref_head,
                    RefKind::RemoteBranch => palette.ref_remote,
                    RefKind::Tag => palette.ref_tag,
                    _ => palette.ref_local,
                };
                let expanded = self.expanded.as_deref() == Some(reference.full_name.as_str());
                let toggle_name = reference.full_name.clone();
                let toggle_entity = entity.clone();
                list = list.child(
                    row(format!("branch-{}", reference.full_name), &palette)
                        .when(expanded, |el| el.bg(palette.selection))
                        .child(Icon::new(if reference.kind == RefKind::Tag { IconName::Tag } else { IconName::GitBranch }).small().text_color(color))
                        .child(div().when(is_current, |el| el.font_weight(FontWeight::SEMIBOLD)).child(reference.name.clone()))
                        .when(reference.behind > 0, |el| el.child(div().text_xs().text_color(palette.link).child(format!("↓{}", reference.behind))))
                        .when(reference.ahead > 0, |el| el.child(div().text_xs().text_color(palette.status_added).child(format!("↑{}", reference.ahead))))
                        .child(div().flex_1())
                        .when_some(reference.upstream.clone(), |el, upstream| {
                            el.child(div().text_xs().text_color(palette.text_secondary).child(upstream))
                        })
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
                    for (ix, branch_action) in branch_actions(&self.model, &reference, current.as_deref()).into_iter().enumerate() {
                        let run = branch_action.run.clone();
                        let close_entity = entity.clone();
                        list = list.child(
                            row(format!("branch-action-{}-{ix}", reference.full_name), &palette)
                                .pl(px(30.))
                                .when(!branch_action.enabled, |el| el.text_color(palette.text_disabled).cursor_default())
                                .child(branch_action.label)
                                .when(branch_action.enabled, |el| {
                                    el.on_click(move |_, window, cx| {
                                        close_entity.update(cx, |this, cx| {
                                            this.expanded = None;
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

        list = add_group(list, "Local", refs.local_branches().filter(|r| matches(&r.name)).cloned().collect());
        list = add_group(list, "Remote", refs.remote_branches().filter(|r| matches(&r.name)).cloned().collect());
        if !query.is_empty() {
            list = add_group(list, "Tags", refs.tags().filter(|r| matches(&r.name)).cloned().collect());
        }

        v_flex()
            .w(px(400.))
            .gap_1()
            .child(Input::new(&self.search).small().prefix(Icon::new(IconName::Search).xsmall()))
            .child(div().id("branches-popup-list").max_h(px(520.)).overflow_y_scrollbar().child(list))
    }
}

pub fn branch_widget_label(model: &RepoModel) -> String {
    use crate::git::RepositoryState::*;
    let refs = model.refs();
    let branch = refs
        .current_branch
        .clone()
        .or_else(|| refs.head_commit.as_ref().map(|h| h[..8.min(h.len())].to_owned()))
        .unwrap_or_else(|| "No branch".into());
    match model.state() {
        Normal => branch,
        Merging => format!("Merging {branch}"),
        Rebasing => format!("Rebasing {branch}"),
        CherryPicking => format!("Cherry-picking in {branch}"),
        Reverting => format!("Reverting in {branch}"),
    }
}

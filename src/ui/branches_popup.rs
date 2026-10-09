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

gpui_kit::actions!(branches_popup, [SelectPrevious, SelectNext, Activate, ExpandSelected, CollapseSelected]);

const CONTEXT: &str = "BranchesPopup";

/// Keyboard navigation as in IntelliJ's popup: ↑↓ move, Enter or → opens a
/// branch's actions (Enter runs an action), ← goes back to the branch. The
/// search field keeps focus, so the keys are bound over it.
pub fn init(cx: &mut App) {
    use gpui_kit::KeyBinding;
    let context = Some("BranchesPopup > Input");
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("enter", Activate, context),
        KeyBinding::new("right", ExpandSelected, context),
        KeyBinding::new("left", CollapseSelected, context),
    ]);
}

/// A row the keyboard can reach.
#[derive(Clone)]
enum Nav {
    /// A top action (Update Project…, New Branch…).
    Run(Run),
    /// A branch row (its key in `expanded`).
    Branch(String),
    /// One of an expanded branch's actions.
    Action { branch: String, run: Run },
}

pub struct BranchesPopup {
    model: Entity<RepoModel>,
    search: Entity<InputState>,
    expanded: Option<String>,
    /// The keyboard-highlighted row, an index into `nav`.
    cursor: Option<usize>,
    /// The reachable rows of the last render, with their list positions.
    nav: Vec<(usize, Nav)>,
    scroll: gpui_kit::ScrollHandle,
    /// Picks the row to highlight once the next render has listed them.
    pending_select: Option<Box<dyn Fn(&Nav) -> bool>>,
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
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    // Typing highlights the first match, as IntelliJ's speed search does.
                    let searching = !this.search.read(cx).value().is_empty();
                    this.cursor = searching.then_some(0);
                    this.expanded = None;
                    this.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                    cx.notify();
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            search,
            expanded: None,
            cursor: None,
            nav: Vec::new(),
            scroll: gpui_kit::ScrollHandle::new(),
            pending_select: None,
            collapsed: ["Tags"].into_iter().collect(),
            on_commit: None,
            _subscriptions: subscriptions,
        }
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
        Rc::new(move |window, cx| checkout_branch(&checkout_model, checkout_ref.clone(), window, cx)),
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
            let delete_model = model.clone();
            let delete_ref = reference.clone();
            actions.push(action(
                "Delete",
                !is_current,
                Rc::new(move |window, cx| delete_branch(&delete_model, &delete_ref, window, cx)),
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

/// Checkout: a remote branch whose local namesake exists asks whether to
/// check that one out or overwrite it; local changes in the way ask for
/// Smart / Force Checkout, as IntelliJ's "Git Checkout Problem" does.
pub(crate) fn checkout_branch(model: &Entity<RepoModel>, reference: RefName, window: &mut Window, cx: &mut App) {
    if reference.kind == RefKind::RemoteBranch {
        let local_name = reference.branch_without_remote().to_owned();
        if let Some(local) = model.read(cx).refs().find(&format!("refs/heads/{local_name}")).cloned() {
            let (overwrite_model, existing_model) = (model.clone(), model.clone());
            let remote = reference.clone();
            dialogs::choose(
                "Checkout",
                format!("A local branch '{local_name}' already exists. Check it out, or overwrite it with '{}'?", reference.name),
                Vec::new(),
                "Cancel",
                vec![
                    ("Overwrite", Rc::new(move |_: &mut Window, cx: &mut App| {
                        let remote = remote.clone();
                        overwrite_model.update(cx, |model, cx| {
                            model.run_operation("Checkout", move |repo| {
                                status::checkout_overwriting(repo, &remote)?;
                                Ok(format!("Checked out {} (reset to {})", remote.branch_without_remote(), remote.name))
                            }, cx)
                        });
                    }) as Run),
                    ("Checkout Existing", Rc::new(move |_: &mut Window, cx: &mut App| run_checkout(&existing_model, local.clone(), status::CheckoutMode::Plain, cx)) as Run),
                ],
                window,
                cx,
            );
            return;
        }
    }
    run_checkout(model, reference, status::CheckoutMode::Plain, cx);
}

fn run_checkout(model: &Entity<RepoModel>, reference: RefName, mode: status::CheckoutMode, cx: &mut App) {
    // Synchronous branch control: the same branch in every root that has it.
    let others = if crate::settings::Settings::get(cx).sync_branches { model.read(cx).other_roots() } else { Vec::new() };
    let task_ref = reference.clone();
    let entity = model.clone();
    model.update(cx, |model, cx| {
        model.run_task(
            "Checkout",
            move |repo| {
                let outcome = status::checkout_with(repo, &task_ref, mode)?;
                let synced = sync_to_roots(repo, &others, |other| {
                    other.run(["rev-parse", "--verify", "-q", &task_ref.full_name]).is_ok() && status::checkout(other, &task_ref).is_ok()
                });
                Ok((outcome, synced))
            },
            move |model, result, cx| match result {
                Ok((outcome, synced)) => {
                    let mut message = match synced {
                        0 => format!("Checked out {}", reference.name),
                        n => format!("Checked out {} in {} repositories", reference.name, n + 1),
                    };
                    if outcome == status::CheckoutOutcome::RestoredWithConflicts {
                        message.push_str(
                            ". Restoring the local changes caused conflicts: resolve them in the Commit tool window (the changes are also kept in the stash)",
                        );
                    }
                    model.notify("Checkout", message, false, cx);
                }
                Err(error) => {
                    let text = error.to_string();
                    match status::overwritten_files(&text).filter(|_| mode == status::CheckoutMode::Plain) {
                        Some(files) => {
                            // The main window, even when another app (or no window manager) has focus.
                            let Some(window) = cx.active_window().or_else(|| cx.windows().into_iter().next()) else { return };
                            let (smart, force) = (entity.clone(), entity.clone());
                            let (smart_ref, force_ref) = (reference.clone(), reference.clone());
                            let name = reference.name.clone();
                            window
                                .update(cx, move |_, window, cx| {
                                    dialogs::choose(
                                        "Git Checkout Problem",
                                        format!(
                                            "Your local changes to the following files would be overwritten by checkout of '{name}'. \
                                             Smart Checkout stashes them, checks out and restores them; Force Checkout discards them."
                                        ),
                                        files,
                                        "Don't Checkout",
                                        vec![
                                            ("Force Checkout", Rc::new(move |_: &mut Window, cx: &mut App| run_checkout(&force, force_ref.clone(), status::CheckoutMode::Force, cx)) as Run),
                                            ("Smart Checkout", Rc::new(move |_: &mut Window, cx: &mut App| run_checkout(&smart, smart_ref.clone(), status::CheckoutMode::Smart, cx)) as Run),
                                        ],
                                        window,
                                        cx,
                                    )
                                })
                                .ok();
                        }
                        None => model.notify("Checkout failed", text, true, cx),
                    }
                }
            },
            cx,
        )
    });
}

/// Delete: a branch that isn't merged into HEAD (or its upstream) asks first,
/// listing the commits that would be lost, and offers Force Delete; the
/// notification then offers Restore.
fn delete_branch(model: &Entity<RepoModel>, reference: &RefName, window: &mut Window, cx: &mut App) {
    let Some(repository) = model.read(cx).repository().cloned() else { return };
    let name = reference.name.clone();
    let tip = reference.target.clone();
    let merged_into = |target: &str| repository.run(["merge-base", "--is-ancestor", &reference.full_name, target]).is_ok();
    let upstream_merged = reference.upstream.as_ref().is_some_and(|u| merged_into(&format!("refs/remotes/{u}")));
    let delete = move |force: bool, model: &Entity<RepoModel>, cx: &mut App| {
        let (name, tip) = (name.clone(), tip.clone());
        model.update(cx, |model, cx| {
            model.run_operation("Delete Branch", move |repo| {
                repo.run(["branch", if force { "-D" } else { "-d" }, &name])?;
                Ok(format!("Deleted branch {name} (was {})", &tip[..tip.len().min(8)]))
            }, cx)
        });
    };
    if merged_into("HEAD") || upstream_merged {
        return delete(false, model, cx);
    }
    let current = model.read(cx).refs().current_branch.clone().unwrap_or_else(|| "HEAD".into());
    let commits: Vec<String> = repository
        .run(["log", "--format=%h %s", "-n", "20", &format!("HEAD..{}", reference.full_name)])
        .map(|out| out.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    let model = model.clone();
    let delete = Rc::new(delete);
    dialogs::choose(
        "Branch Is Not Fully Merged",
        format!("The branch '{}' is not fully merged into '{current}'. These commits would be lost:", reference.name),
        commits,
        "Cancel",
        vec![("Force Delete", Rc::new(move |_: &mut Window, cx: &mut App| delete(true, &model, cx)) as Run)],
        window,
        cx,
    );
}

fn row(id: impl Into<SharedString>, palette: &Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    let hover = palette.hover;
    h_flex()
        .id(gpui_kit::ElementId::Name(id.into()))
        .flex_shrink_0()
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
        self.cursor = None;
        self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
    }
}

impl BranchesPopup {
    fn set_cursor(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.cursor = Some(ix);
        if let Some((row, _)) = self.nav.get(ix) {
            self.scroll.scroll_to_item(*row);
        }
        cx.notify();
    }

    fn move_cursor(&mut self, down: bool, cx: &mut Context<Self>) {
        let n = self.nav.len();
        if n == 0 {
            return;
        }
        let ix = match (self.cursor, down) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(ix), true) => (ix + 1) % n,
            (Some(ix), false) => (ix + n - 1) % n,
        };
        self.set_cursor(ix, cx);
    }

    /// The highlighted row's index in `nav` after the next render, found by its kind.
    fn select_after_render(&mut self, find: impl Fn(&Nav) -> bool + 'static, cx: &mut Context<Self>) {
        self.pending_select = Some(Box::new(find));
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, nav)) = self.cursor.and_then(|ix| self.nav.get(ix)).cloned() else { return };
        match nav {
            Nav::Run(run) | Nav::Action { run, .. } => {
                self.expanded = None;
                cx.emit(gpui_kit::DismissEvent);
                cx.notify();
                run(window, cx);
            }
            Nav::Branch(_) => self.expand_selected(true, cx),
        }
    }

    /// → opens the highlighted branch's actions and moves to the first one;
    /// ← closes them and goes back to the branch.
    fn expand_selected(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some((_, nav)) = self.cursor.and_then(|ix| self.nav.get(ix)).cloned() else { return };
        match (nav, open) {
            (Nav::Branch(key), true) => {
                self.expanded = Some(key.clone());
                self.select_after_render(move |nav| matches!(nav, Nav::Action { branch, .. } if *branch == key), cx);
            }
            (Nav::Branch(key), false) if self.expanded.as_deref() == Some(key.as_str()) => {
                self.expanded = None;
                cx.notify();
            }
            (Nav::Action { branch, .. }, false) => {
                self.expanded = None;
                self.select_after_render(move |nav| matches!(nav, Nav::Branch(key) if *key == branch), cx);
            }
            _ => {}
        }
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
        let searching = !query.is_empty();
        let collapsed = self.collapsed.clone();
        let cursor = self.cursor;

        // Rows of the scrolling list, and the ones the keyboard can reach
        // (with their position among the rows, for scrolling them into view).
        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
        let mut nav: Vec<(usize, Nav)> = Vec::new();
        let highlight = |el: gpui_kit::Stateful<gpui_kit::Div>, nav_ix: usize| {
            el.when(cursor == Some(nav_ix), |el| el.bg(palette.hover))
        };

        let top_action = |id: &'static str, icon: IconName, label: &'static str, run: Run, nav_ix: usize| {
            let row_run = run.clone();
            highlight(row(id, &palette), nav_ix)
                .child(Icon::new(icon).small().text_color(palette.text_secondary))
                .child(label)
                .on_click({
                    let entity = entity.clone();
                    move |_, window, cx| {
                        entity.update(cx, |_, cx| cx.emit(gpui_kit::DismissEvent));
                        row_run(window, cx)
                    }
                })
        };
        let mut push_top = |rows: &mut Vec<gpui_kit::AnyElement>, id: &'static str, icon: IconName, label: &'static str, run: Run| {
            let nav_ix = nav.len();
            nav.push((rows.len(), Nav::Run(run.clone())));
            rows.push(top_action(id, icon, label, run, nav_ix).into_any_element());
        };

        let section = |title: &'static str, count: usize| {
            let entity = entity.clone();
            let is_collapsed = collapsed.contains(title) && !searching;
            h_flex()
                .id(SharedString::from(format!("bp-group-{title}")))
                .flex_shrink_0()
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

        if query.is_empty() {
            // An operation in progress: its Continue / Skip / Abort come first, as in IntelliJ.
            {
                use crate::git::RepositoryState::*;
                use crate::git::merge::{self, OperationStep};
                let state = model.state();
                let labels = match state {
                    Rebasing => Some(("Continue Rebase", "Abort Rebase")),
                    Merging => Some(("", "Abort Merge")),
                    CherryPicking => Some(("Continue Cherry-Pick", "Abort Cherry-Pick")),
                    Reverting => Some(("Continue Revert", "Abort Revert")),
                    Normal => None,
                };
                if let Some((continue_label, abort_label)) = labels {
                    let step = |step: OperationStep| {
                        let model = self.model.clone();
                        Rc::new(move |_: &mut Window, cx: &mut gpui_kit::App| {
                            model.update(cx, |m, cx| m.run_operation("Git", move |repo| merge::step(repo, state, step), cx))
                        }) as Run
                    };
                    if state != Merging {
                        push_top(&mut rows, "bp-continue", IconName::Play, continue_label, step(OperationStep::Continue));
                    }
                    if state == Rebasing {
                        push_top(&mut rows, "bp-skip", IconName::ArrowRight, "Skip Commit", step(OperationStep::Skip));
                    }
                    let abort_model = self.model.clone();
                    push_top(&mut rows, "bp-abort", IconName::X, abort_label, Rc::new(move |window: &mut Window, cx: &mut App| {
                        dialogs::abort_operation(abort_model.clone(), state, window, cx)
                    }));
                    rows.push(div().flex_shrink_0().my_1().h(px(1.)).bg(palette.border).into_any_element());
                }
            }
            let new_model = self.model.clone();
            let new_head = head.clone();
            let update_model = self.model.clone();
            push_top(&mut rows, "bp-update", IconName::ArrowDownToLine, "Update Project…", Rc::new(move |window: &mut Window, cx: &mut App| {
                dialogs::update_project(update_model.clone(), window, cx)
            }));
            if let Some(run) = self.on_commit.clone() {
                push_top(&mut rows, "bp-commit", IconName::Check, "Commit…", run);
            }
            let push_model = self.model.clone();
            push_top(&mut rows, "bp-push", IconName::ArrowUpFromLine, "Push…", Rc::new(move |window: &mut Window, cx: &mut App| {
                dialogs::push(push_model.clone(), window, cx)
            }));
            push_top(&mut rows, "bp-new", IconName::Plus, "New Branch…", Rc::new(move |window: &mut Window, cx: &mut App| {
                dialogs::new_branch(new_model.clone(), new_head.clone(), window, cx)
            }));
            let revision_model = self.model.clone();
            push_top(&mut rows, "bp-checkout-rev", IconName::Tag, "Checkout Tag or Revision…", Rc::new(move |window: &mut Window, cx: &mut App| {
                dialogs::checkout_revision(revision_model.clone(), window, cx)
            }));
            // Multi-root projects: pick the repository whose branches are listed
            // (and that branch operations target), plus synchronous control.
            if model.is_multi_root() {
                let project = model.project_root().map(std::path::Path::to_path_buf).unwrap_or_default();
                let active = model.repository().map(|r| r.root().to_path_buf());
                rows.push(
                    div().flex_shrink_0().px_2().pt_2().pb_0p5().text_xs().text_color(palette.text_secondary).child("Repositories").into_any_element(),
                );
                for (ix, root) in model.roots().iter().enumerate() {
                    let is_active = active.as_ref() == Some(&root.path);
                    let path = root.path.clone();
                    let switch_model = self.model.clone();
                    rows.push(
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
                            })
                            .into_any_element(),
                    );
                }
                let sync = crate::settings::Settings::get(cx).sync_branches;
                rows.push(
                    row("bp-sync", &palette)
                        .child(Icon::new(if sync { IconName::Check } else { IconName::Circle }).xsmall().text_color(if sync { palette.accent } else { gpui_kit::transparent_black() }))
                        .child(div().text_color(palette.text_secondary).child("Execute branch operations on all roots"))
                        .on_click(move |_, _, cx| crate::settings::Settings::update(cx, |s| s.sync_branches = !s.sync_branches))
                        .into_any_element(),
                );
            }
        }
        drop(push_top);

        let favorites = refs.favorites.clone();
        let mut add_group = |rows: &mut Vec<gpui_kit::AnyElement>, nav: &mut Vec<(usize, Nav)>, title: &'static str, mut refs_in_group: Vec<RefName>| {
            if refs_in_group.is_empty() {
                return;
            }
            if title != "Recent" {
                // Favorites first, the rest keeps its order.
                refs_in_group.sort_by_key(|r| !favorites.contains(&r.full_name));
            }
            let count = refs_in_group.len();
            rows.push(section(title, count).into_any_element());
            if collapsed.contains(title) && !searching {
                return;
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
                let toggle_name = key.clone();
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
                let nav_ix = nav.len();
                nav.push((rows.len(), Nav::Branch(key.clone())));
                rows.push(
                    highlight(row(format!("branch-{title}-{}", reference.full_name), &palette), nav_ix)
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
                        })
                        .into_any_element(),
                );
                if expanded {
                    for (ix, branch_action) in branch_actions(&self.model, &reference, current.as_deref(), &remote_names).into_iter().enumerate() {
                        let run = branch_action.run.clone();
                        let close_entity = entity.clone();
                        let nav_ix = nav.len();
                        if branch_action.enabled {
                            nav.push((rows.len(), Nav::Action { branch: key.clone(), run: run.clone() }));
                        }
                        rows.push(
                            row(format!("branch-action-{title}-{}-{ix}", reference.full_name), &palette)
                                .when(branch_action.enabled, |el| highlight(el, nav_ix))
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
                                })
                                .into_any_element(),
                        );
                    }
                }
            }
        };

        if !searching {
            let recent: Vec<RefName> = refs
                .recent
                .iter()
                .filter(|name| Some(*name) != current.as_ref())
                .filter_map(|name| refs.find(&format!("refs/heads/{name}")).cloned())
                .collect();
            add_group(&mut rows, &mut nav, "Recent", recent);
        }
        add_group(&mut rows, &mut nav, "Local", refs.local_branches().filter(|r| matches(&r.name)).cloned().collect());
        add_group(&mut rows, &mut nav, "Remote", refs.remote_branches().filter(|r| matches(&r.name)).cloned().collect());
        add_group(&mut rows, &mut nav, "Tags", refs.tags().filter(|r| matches(&r.name)).cloned().collect());
        self.nav = nav;
        if let Some(find) = self.pending_select.take() {
            if let Some(ix) = self.nav.iter().position(|(_, nav)| find(nav)) {
                self.cursor = Some(ix);
                self.scroll.scroll_to_item(self.nav[ix].0);
                // Paint the new highlight on the next frame.
                cx.notify();
            }
        }

        v_flex()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_cursor(false, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_cursor(true, cx)))
            .on_action(cx.listener(|this, _: &Activate, window, cx| this.activate(window, cx)))
            .on_action(cx.listener(|this, _: &ExpandSelected, _, cx| this.expand_selected(true, cx)))
            .on_action(cx.listener(|this, _: &CollapseSelected, _, cx| this.expand_selected(false, cx)))
            .w(px(400.))
            .gap_1()
            .child(Input::new(&self.search).small().prefix(Icon::new(IconName::Search).xsmall()))
            .child(
                div()
                    .relative()
                    .child(
                        v_flex()
                            .id("branches-popup-list")
                            .max_h(px(520.))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .gap_px()
                            .children(rows),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

/// Remotes tags push to / delete on, as IntelliJ lists them per remote.
/// The remotes a tag can be pushed to or deleted on: the configured ones
/// (none, no "Push to …" items, as in IntelliJ).
pub(crate) fn remote_names(refs: &crate::git::RepositoryRefs) -> Vec<String> {
    if !refs.remotes.is_empty() {
        return refs.remotes.clone();
    }
    let mut names: Vec<String> = refs.remote_branches().filter_map(|r| r.remote().map(str::to_owned)).collect();
    names.dedup();
    names
}

pub fn branch_widget_label(model: &RepoModel) -> String {
    use crate::git::RepositoryState::*;
    let refs = model.refs();
    let rebasing = (model.state() == Rebasing).then(|| model.repository().and_then(|r| r.rebasing_branch())).flatten();
    let branch = refs
        .current_branch
        .clone()
        .or(rebasing)
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

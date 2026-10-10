//! VCS operations: commit, push, update, branches, stash, the VCS
//! Operations popup, conflicts / merge, and the Git tool window's Log tabs.

use super::*;
use crate::ui::as_icons as icons;

impl Workspace {
    pub(super) fn show_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity();
        dialogs::conflicts(
            self.model.clone(),
            Rc::new(move |conflict, window, cx| workspace.update(cx, |this, cx| this.open_merge(conflict, window, cx))),
            window,
            cx,
        );
    }

    /// The merge tool, when it is what the editor area shows (F7 goes to it).
    pub(super) fn front_merge(&self) -> Option<Entity<MergeView>> {
        self.merge.as_ref().filter(|_| self.front == Front::Merge).map(|(m, _)| m.clone())
    }

    pub fn open_merge(&mut self, conflict: Conflict, window: &mut Window, cx: &mut Context<Self>) {
        if !conflict.kind.can_merge() {
            return;
        }
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        if self.merge.as_ref().is_some_and(|(m, _)| m.read(cx).path() == conflict.path) {
            return;
        }
        let model = self.model.clone();
        let view = cx.new(|cx| MergeView::new(model, repository, conflict, cx));
        let subscription = cx.subscribe_in(&view, window, |this, _, event: &MergeEvent, window, cx| match event {
            MergeEvent::Compare(source) => this.open_diff(source.clone(), cx),
            MergeEvent::Closed(applied) => {
                this.merge = None;
                this.fix_front(cx);
                // Back to the Conflicts dialog while files still conflict
                // (the status doesn't know yet of one just resolved).
                let left = merge::conflicts(this.model.read(cx).status()).len();
                if left > usize::from(*applied) {
                    this.show_conflicts(window, cx);
                }
                cx.notify();
            }
        });
        self.merge = Some((view, subscription));
        self.focus_group(0, cx);
        self.front = Front::Merge;
        cx.notify();
    }

    /// "Rebasing main · 2 conflicts · Resolve… Continue Skip Abort", above the editor.
    pub(super) fn render_operation_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let model = self.model.read(cx);
        let state = model.state();
        if state == RepositoryState::Normal {
            return None;
        }
        let palette = cx.palette().clone();
        let conflicts = merge::conflicts(model.status());
        let editing = state == RepositoryState::Rebasing
            && model.repository().is_some_and(|r| r.git_dir().join("rebase-merge").join("amend").exists());
        let label = match state {
            RepositoryState::Rebasing => "Rebase in progress",
            RepositoryState::Merging => "Merge in progress",
            RepositoryState::CherryPicking => "Cherry-pick in progress",
            RepositoryState::Reverting => "Revert in progress",
            RepositoryState::Normal => "",
        };
        let step = |step: OperationStep| {
            let model = self.model.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                model.update(cx, |m, cx| m.run_operation("Git", move |repo| merge::step(repo, state, step), cx));
            }
        };
        let entity = cx.entity();
        Some(
            h_flex()
                .h(px(34.))
                .px_3()
                .gap_2()
                .text_sm()
                .bg(if conflicts.is_empty() { palette.diff_header } else { palette.diff_deleted })
                .border_b_1()
                .border_color(palette.border)
                .child(Icon::new(icons::VCS_MERGE).small())
                .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(label))
                .child(div().text_color(palette.text_secondary).child(match conflicts.len() {
                    // An `edit` step of an interactive rebase, not a conflict.
                    0 if editing => "Stopped for editing".to_owned(),
                    0 => "All conflicts resolved".to_owned(),
                    1 => "1 file with conflicts".to_owned(),
                    n => format!("{n} files with conflicts"),
                }))
                .child(div().flex_1())
                .when(!conflicts.is_empty(), |el| {
                    el.child(Button::new("op-resolve").small().primary().label("Resolve…").on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.show_conflicts(window, cx))
                    }))
                })
                .child(
                    Button::new("op-continue")
                        .small()
                        .outline()
                        .label("Continue")
                        .disabled(!conflicts.is_empty())
                        .on_click(step(OperationStep::Continue)),
                )
                .when(state != RepositoryState::Merging, |el| {
                    el.child(Button::new("op-skip").small().outline().label("Skip").on_click(step(OperationStep::Skip)))
                })
                .child(Button::new("op-abort").small().outline().label("Abort").on_click({
                    let model = self.model.clone();
                    move |_, window, cx| dialogs::abort_operation(model.clone(), state, window, cx)
                })),
        )
    }

    pub fn show_history(&mut self, path: String, cx: &mut Context<Self>) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let filter = crate::git::LogFilter { paths: vec![path], ..Default::default() };
        self.open_log_tab(format!("History: {name}"), filter, cx);
    }

    /// Opens another Log tab with its own filters. Tabs share the repository;
    /// switching tabs swaps the Log's filters.
    pub fn open_log_tab(&mut self, title: String, filter: crate::git::LogFilter, cx: &mut Context<Self>) {
        self.save_log_tab(cx);
        self.log_tabs.push(LogTab { title, filter: filter.clone(), selected: None });
        self.active_log = self.log_tabs.len() - 1;
        self.tools.open(ToolWindow::Git);
        self.bottom_tab = BottomTab::Log;
        self.model.update(cx, |m, cx| m.set_filter(filter, cx));
        cx.notify();
    }

    pub(super) fn save_log_tab(&mut self, cx: &mut Context<Self>) {
        let current = self.model.read(cx).filter().clone();
        let selected = self.model.read(cx).selected_hash().map(str::to_owned);
        if let Some(tab) = self.log_tabs.get_mut(self.active_log) {
            tab.filter = current;
            tab.selected = selected;
        }
    }

    pub(super) fn switch_log_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.bottom_tab == BottomTab::Log && ix == self.active_log {
            return;
        }
        self.save_log_tab(cx);
        self.active_log = ix.min(self.log_tabs.len() - 1);
        self.bottom_tab = BottomTab::Log;
        let filter = self.log_tabs[self.active_log].filter.clone();
        let selected = self.log_tabs[self.active_log].selected.clone();
        self.model.update(cx, |m, cx| {
            m.set_filter(filter, cx);
            m.select_hash(selected, cx);
        });
        cx.notify();
    }

    pub(super) fn close_log_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix == 0 || ix >= self.log_tabs.len() {
            return;
        }
        let was_active = ix == self.active_log;
        self.log_tabs.remove(ix);
        if was_active {
            self.active_log = 0;
            let filter = self.log_tabs[0].filter.clone();
            self.model.update(cx, |m, cx| m.set_filter(filter, cx));
        } else if ix < self.active_log {
            self.active_log -= 1;
        }
        cx.notify();
    }

    pub(super) fn on_commit(&mut self, _: &CommitChanges, window: &mut Window, cx: &mut Context<Self>) {
        if !Settings::get(cx).non_modal_commit {
            return self.open_modal_commit(window, cx);
        }
        self.tools.open(ToolWindow::Commit);
        self.left_tab = LeftTab::Commit;
        self.commit.update(cx, |commit, cx| commit.focus_message(window, cx));
        cx.notify();
    }

    /// Settings › Commit › "Use non-modal commit interface" off: Commit
    /// opens IntelliJ's modal Commit Changes dialog (the changes, the
    /// message and the commit buttons) instead of the tool window.
    pub(super) fn open_modal_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_commit.get() {
            return;
        }
        // The view is shown in one place at a time.
        self.tools.hide(ToolWindow::Commit);
        self.modal_commit.set(true);
        self.commit.update(cx, |commit, cx| commit.set_in_dialog(true, cx));
        let (commit, open) = (self.commit.clone(), self.modal_commit.clone());
        window.open_dialog(cx, move |dialog, _, _| {
            let (open, commit_view) = (open.clone(), commit.clone());
            dialog
                .title("Commit Changes")
                .w(px(760.))
                .child(div().h(px(560.)).child(commit.clone()))
                .on_close(move |_, _, cx| {
                    open.set(false);
                    commit_view.update(cx, |commit, cx| commit.set_in_dialog(false, cx));
                })
        });
        self.commit.update(cx, |commit, cx| commit.focus_message(window, cx));
        cx.notify();
    }

    pub(super) fn on_push(&mut self, _: &PushChanges, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::push(self.model.clone(), window, cx);
    }

    pub(super) fn on_update(&mut self, _: &UpdateProject, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::update_project(self.model.clone(), window, cx);
    }

    pub(super) fn on_show_branches(&mut self, _: &ShowBranches, window: &mut Window, cx: &mut Context<Self>) {
        self.open_branches(window, cx);
    }

    /// Opens the branches popup with its search field focused, so typing
    /// filters right away (IntelliJ's speed search).
    pub(super) fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branches_open = true;
        self.branches_popup.update(cx, |popup, cx| popup.reset_search(window, cx));
        let popup = self.branches_popup.clone();
        window.defer(cx, move |window, cx| popup.update(cx, |popup, cx| popup.focus_search(window, cx)));
        // Opened from the keyboard, the popover focuses itself when it renders;
        // take the focus back for the search field afterwards.
        let popup = self.branches_popup.clone();
        window.on_next_frame(move |window, cx| popup.update(cx, |popup, cx| popup.focus_search(window, cx)));
        cx.notify();
    }

    pub(super) fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.reload(cx));
    }

    pub(super) fn on_stash(&mut self, _: &StashChanges, window: &mut Window, cx: &mut Context<Self>) {
        dialogs::stash(self.model.clone(), window, cx);
    }

    /// IntelliJ's VCS Operations quick list, in order; `None` is a separator.
    pub(super) fn vcs_operation_list() -> Vec<Option<(&'static str, &'static str, VcsRun)>> {
        let op = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| -> VcsRun { Rc::new(f) };
        vec![
            Some(("Commit…", "Ctrl+K", op(|this, window, cx| this.on_commit(&CommitChanges, window, cx)))),
            Some(("Push…", "Ctrl+Shift+K", op(|this, window, cx| dialogs::push(this.model.clone(), window, cx)))),
            Some(("Update Project…", "Ctrl+T", op(|this, window, cx| dialogs::update_project(this.model.clone(), window, cx)))),
            Some(("Pull…", "", op(|this, window, cx| remote_dialogs::pull(this.model.clone(), window, cx)))),
            Some(("Fetch", "", op(|this, _, cx| {
                this.model.update(cx, |m, cx| m.run_operation("Fetch", |repo| {
                    repo.run(["fetch", "--all", "--prune"])?;
                    Ok("Fetched all remotes".into())
                }, cx))
            }))),
            None,
            Some(("Branches…", "Ctrl+Shift+`", op(|this, window, cx| this.open_branches(window, cx)))),
            Some(("New Branch…", "", op(|this, window, cx| dialogs::new_branch(this.model.clone(), "HEAD".into(), window, cx)))),
            Some(("New Tag…", "", op(|this, window, cx| dialogs::new_tag(this.model.clone(), "HEAD".into(), window, cx)))),
            Some(("Merge…", "", op(|this, window, cx| dialogs::merge(this.model.clone(), window, cx)))),
            Some(("Rebase…", "", op(|this, window, cx| dialogs::rebase(this.model.clone(), window, cx)))),
            Some(("Manage Remotes…", "", op(|this, window, cx| remote_dialogs::manage_remotes(this.model.clone(), window, cx)))),
            Some(("New Worktree…", "", op(|this, window, cx| crate::ui::worktree_view::new_worktree(this.model.clone(), window, cx)))),
            Some(("Reset HEAD…", "", op(|this, window, cx| dialogs::reset_to(this.model.clone(), "HEAD".into(), window, cx)))),
            None,
            Some(("Stash Changes…", "", op(|this, window, cx| dialogs::stash(this.model.clone(), window, cx)))),
            Some(("Unstash Changes…", "", op(|this, window, cx| {
                this.show_left_tab(LeftTab::Stash, window, cx);
            }))),
            Some(("Shelve Changes…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.shelve(window, cx))))),
            Some(("Unshelve Changes…", "", op(|this, window, cx| {
                this.show_left_tab(LeftTab::Shelf, window, cx);
            }))),
            None,
            Some(("Create Patch…", "", op(|this, window, cx| this.commit.update(cx, |c, cx| c.create_patch(window, cx))))),
            Some(("Apply Patch…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), false, window, cx)))),
            Some(("Apply Patch from Clipboard…", "", op(|this, window, cx| patch_dialogs::apply_patch(this.model.clone(), true, window, cx)))),
            None,
            Some(("Show Git Log", "Alt+9", op(|this, _, cx| {
                this.tools.open(ToolWindow::Git);
                this.bottom_tab = BottomTab::Log;
                cx.notify();
            }))),
            Some(("Settings…", "Ctrl+Alt+S", op(|this, window, cx| crate::ui::settings_dialog::open(Some(this.model.clone()), window, cx)))),
        ]
    }

    /// IntelliJ's VCS Operations quick list, numbered like the original:
    /// 1–9 run an entry at once, ↑ ↓ and Enter pick one.
    pub(super) fn on_vcs_operations(&mut self, _: &VcsOperations, window: &mut Window, cx: &mut Context<Self>) {
        let items: Rc<Vec<Option<(&'static str, &'static str, VcsRun)>>> = Rc::new(Self::vcs_operation_list());
        let runs: Rc<Vec<VcsRun>> = Rc::new(items.iter().flatten().map(|(_, _, run)| run.clone()).collect());
        let selected = Rc::new(std::cell::Cell::new(0usize));
        let workspace = cx.entity();
        let focus = cx.focus_handle();
        let dialog_focus = focus.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let pick = {
                let (runs, workspace) = (runs.clone(), workspace.clone());
                Rc::new(move |n: usize, window: &mut Window, cx: &mut gpui_kit::App| {
                    let Some(run) = runs.get(n).cloned() else { return };
                    window.close_dialog(cx);
                    workspace.update(cx, |this, cx| run(this, window, cx));
                })
            };
            let mut list = v_flex()
                .id("vcs-operations")
                .track_focus(&dialog_focus)
                .gap_px()
                .on_key_down({
                    let (pick, selected, count) = (pick.clone(), selected.clone(), runs.len());
                    move |e: &gpui_kit::KeyDownEvent, window, cx| {
                        let key = e.keystroke.key.as_str();
                        match key {
                            "up" => selected.set((selected.get() + count - 1) % count.max(1)),
                            "down" => selected.set((selected.get() + 1) % count.max(1)),
                            "enter" => pick(selected.get(), window, cx),
                            _ => match key.parse::<usize>() {
                                Ok(n @ 1..=9) if !e.keystroke.modifiers.modified() => pick(n - 1, window, cx),
                                _ => return,
                            },
                        }
                        cx.stop_propagation();
                        window.refresh();
                    }
                });
            let mut number = 0;
            for (ix, item) in items.iter().enumerate() {
                let Some((label, shortcut, _)) = item else {
                    list = list.child(div().my_1().h(px(1.)).bg(palette.border));
                    continue;
                };
                let n = number;
                number += 1;
                let pick = pick.clone();
                list = list.child(
                    h_flex()
                        .id(("vcs-op", ix))
                        .h(px(26.))
                        .px_2()
                        .gap_2()
                        .rounded(px(4.))
                        .text_sm()
                        .cursor_pointer()
                        .when(n == selected.get(), |el| el.bg(palette.selection))
                        .hover(|s| s.bg(palette.hover))
                        .on_click(move |_, window, cx| pick(n, window, cx))
                        .child(div().w(px(16.)).text_color(palette.text_secondary).child(if n < 9 { (n + 1).to_string() } else { String::new() }))
                        .child(div().flex_1().child(*label))
                        .child(div().text_xs().text_color(palette.text_secondary).child(crate::ui::file_menus::shortcut_label(shortcut))),
                );
            }
            dialog.title("VCS Operations").w(px(340.)).child(list)
        });
        window.focus(&focus, cx);
    }
}

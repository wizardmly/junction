//! The Worktrees tab of the Git tool window: every working tree of the
//! repository, with New Worktree…, Open, Open in New Window and Delete.

use std::path::{Path, PathBuf};
use crate::ui::as_icons as icons;
use std::rc::Rc;
use std::cell::Cell;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::worktree::{self, Worktree};
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{row_height, tool_button};
use crate::ui::dialogs::{self, focus_input, footer};

pub struct WorktreeView {
    model: Entity<RepoModel>,
    worktrees: Vec<Worktree>,
    selected: Option<usize>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl WorktreeView {
    pub fn new(model: Entity<RepoModel>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&model, |this, _, event, cx| {
            if matches!(event, RepoEvent::Reloaded) {
                this.reload(cx);
            }
        })];
        let mut this = Self { model, worktrees: Vec::new(), selected: None, _load: None, _subscriptions: subscriptions };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        self._load = Some(cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { worktree::list(&repository).unwrap_or_default() }).await;
            this.update(cx, |this, cx| {
                let previous = this.selected.and_then(|ix| this.worktrees.get(ix)).map(|w| w.path.clone());
                this.worktrees = list;
                this.selected = previous.and_then(|p| this.worktrees.iter().position(|w| w.path == p));
                cx.notify();
            })
            .ok();
        }));
    }

    fn current_root(&self, cx: &App) -> Option<PathBuf> {
        self.model.read(cx).repository().map(|r| canonical(r.root()))
    }

    fn selected(&self) -> Option<Worktree> {
        self.selected.and_then(|ix| self.worktrees.get(ix)).cloned()
    }

    fn open(&self, new_window: bool, cx: &mut Context<Self>) {
        let Some(worktree) = self.selected() else { return };
        if new_window {
            crate::open_project_window(Some(worktree.path), cx);
        } else {
            self.model.update(cx, |m, cx| m.open(worktree.path, cx));
        }
    }

    fn delete(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(worktree) = self.selected() else { return };
        if worktree.main || Some(canonical(&worktree.path)) == self.current_root(cx) {
            return;
        }
        let model = self.model.clone();
        let force = worktree.locked;
        dialogs::confirm(
            "Delete Worktree",
            format!(
                "Delete the worktree at {}?\nIts folder is removed; the branch{} is kept.",
                worktree.path.display(),
                worktree.branch.as_deref().map(|b| format!(" '{b}'")).unwrap_or_default()
            ),
            "Delete",
            move |cx| {
                let path = worktree.path.clone();
                model.update(cx, |m, cx| {
                    m.run_operation("Delete Worktree", move |repo| {
                        if path.exists() {
                            worktree::remove(repo, &path, force)?;
                        } else {
                            worktree::prune(repo)?;
                        }
                        Ok(format!("Deleted worktree {}", path.display()))
                    }, cx)
                })
            },
            window,
            cx,
        );
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Render for WorktreeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.current_root(cx);
        let selected = self.selected();
        let can_delete = selected.as_ref().is_some_and(|w| !w.main && Some(canonical(&w.path)) != current);
        let entity = cx.entity();

        let mut rows = v_flex();
        for (ix, wt) in self.worktrees.iter().enumerate() {
            let is_selected = self.selected == Some(ix);
            let is_current = Some(canonical(&wt.path)) == current;
            let name = wt.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| wt.path.display().to_string());
            let branch = wt.branch.clone().unwrap_or_else(|| format!("detached at {}", &wt.head[..wt.head.len().min(8)]));
            let menu_entity = entity.clone();
            let deletable = !wt.main && !is_current;
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("worktree-{ix}")))
                    .h(px(row_height() + 4.))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .when(is_selected, |el| el.bg(palette.selection))
                    .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                        this.selected = Some(ix);
                        if event.click_count() == 2 {
                            this.open(false, cx);
                        }
                        cx.notify();
                    }))
                    .context_menu(move |menu, _, _| {
                        let (o, n, d) = (menu_entity.clone(), menu_entity.clone(), menu_entity.clone());
                        menu.item(PopupMenuItem::new("Open").disabled(is_current).on_click(move |_, _, cx| {
                            o.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.open(false, cx)
                            })
                        }))
                        .item(PopupMenuItem::new("Open in New Window").on_click(move |_, _, cx| {
                            n.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.open(true, cx)
                            })
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Delete…").disabled(!deletable).on_click(move |_, window, cx| {
                            d.update(cx, |this, cx| {
                                this.selected = Some(ix);
                                this.delete(window, cx)
                            })
                        }))
                    })
                    .child(Icon::new(if wt.main { icons::MODULE } else { icons::FOLDER }).small().text_color(palette.text_secondary))
                    .child(div().w(px(180.)).overflow_hidden().whitespace_nowrap().text_ellipsis().when(is_current, |el| el.font_weight(gpui_kit::FontWeight::SEMIBOLD)).child(name))
                    .child(
                        h_flex()
                            .w(px(200.))
                            .gap_1()
                            .overflow_hidden()
                            .child(Icon::new(icons::BRANCH).xsmall().text_color(if wt.branch.is_some() { palette.ref_local } else { palette.text_secondary }))
                            .child(div().whitespace_nowrap().text_ellipsis().child(branch)),
                    )
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(palette.text_secondary).child(wt.path.display().to_string()))
                    .when(is_current, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("current")))
                    .when(wt.main, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("main")))
                    .when(wt.locked, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("locked")))
                    .when(wt.prunable, |el| el.child(div().text_xs().text_color(palette.status_deleted).child("missing"))),
            );
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("wt-new", icons::ADD, "New Worktree…").on_click(cx.listener(|this, _, window, cx| {
                        new_worktree(this.model.clone(), window, cx)
                    })))
                    .child(
                        tool_button("wt-open", icons::FOLDER, "Open")
                            .disabled(selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.open(false, cx))),
                    )
                    .child(
                        tool_button("wt-open-new", icons::LAYOUT, "Open in New Window")
                            .disabled(selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.open(true, cx))),
                    )
                    .child(
                        tool_button("wt-delete", icons::DELETE, "Delete…")
                            .disabled(!can_delete)
                            .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                    )
                    .child(tool_button("wt-refresh", icons::REFRESH, "Refresh").on_click(cx.listener(|this, _, _, cx| this.reload(cx)))),
            )
            .child(div().id("worktrees").flex_1().min_h_0().overflow_y_scrollbar().child(rows))
    }
}

/// New Worktree…: a new or existing branch checked out in another folder.
pub fn new_worktree(model: Entity<RepoModel>, window: &mut Window, cx: &mut App) {
    let (root, branches, current) = {
        let m = model.read(cx);
        let Some(repo) = m.repository() else { return };
        let refs = m.refs();
        (
            repo.root().to_path_buf(),
            refs.local_branches().map(|r| r.name.clone()).collect::<Vec<_>>(),
            refs.current_branch.clone().unwrap_or_else(|| "HEAD".into()),
        )
    };
    let repo_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
    let branch = cx.new(|cx| InputState::new(window, cx).placeholder("Branch name"));
    let base = cx.new(|cx| InputState::new(window, cx).default_value(current));
    let location = cx.new(|cx| InputState::new(window, cx).placeholder(format!("{}/{repo_name}-<branch>", parent.display())));
    let create = Rc::new(Cell::new(true));
    let focus_target = branch.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let secondary = cx.palette().text_secondary;
        let is_new = create.get();
        let (create_cell, create_ok) = (create.clone(), create.clone());
        let (branch_ok, base_ok, location_ok, menu_branch) = (branch.clone(), base.clone(), location.clone(), branch.clone());
        let (model, parent, repo_name) = (model.clone(), parent.clone(), repo_name.clone());
        let existing = branches.clone();
        let label = |text: &'static str| div().w(px(80.)).text_sm().text_color(secondary).child(text);
        dialog
            .title("New Worktree")
            .w(px(520.))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("wt-create-branch").label("Create new branch").checked(is_new).on_change(move |v, window, _| {
                            create_cell.set(*v);
                            window.refresh();
                        }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(label("Branch:"))
                            .child(div().flex_1().child(Input::new(&branch)))
                            .when(!is_new, |el| {
                                el.child(Button::new("wt-branches").outline().icon(icons::CHEVRON_DOWN).dropdown_menu(move |mut menu, _, _| {
                                    for name in &existing {
                                        let input = menu_branch.clone();
                                        let name = name.clone();
                                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| {
                                            input.update(cx, |s, cx| s.set_value(name.clone(), window, cx))
                                        }));
                                    }
                                    menu
                                }))
                            }),
                    )
                    .when(is_new, |el| el.child(h_flex().gap_2().child(label("From:")).child(div().flex_1().child(Input::new(&base)))))
                    .child(h_flex().gap_2().child(label("Location:")).child(div().flex_1().child(Input::new(&location)))),
            )
            .on_ok(move |_, _, cx| {
                let branch = branch_ok.read(cx).value().trim().to_owned();
                if branch.is_empty() {
                    return false;
                }
                let base = base_ok.read(cx).value().trim().to_owned();
                let typed = location_ok.read(cx).value().trim().to_owned();
                let path = if typed.is_empty() {
                    parent.join(format!("{repo_name}-{}", branch.replace('/', "-")))
                } else {
                    PathBuf::from(typed)
                };
                let new_branch = create_ok.get();
                model.update(cx, |m, cx| {
                    m.run_operation("New Worktree", move |repo| {
                        worktree::add(repo, &path, &branch, new_branch, Some(&base))?;
                        Ok(format!("Created worktree for {branch} at {}", path.display()))
                    }, cx)
                });
                true
            })
            .footer(footer("Create"))
    });
    focus_input(&focus_target, window, cx);
}

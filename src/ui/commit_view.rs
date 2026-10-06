//! The Commit tool window (non-modal commit): changes with checkboxes,
//! unversioned files, Amend, the message editor, and Commit / Commit and Push.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::{
    Disableable as _,
    Icon, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{InputEvent, Textarea, TextareaState},
    list::ListItem,
    tree::{TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::status::{self, CommitRequest};
use crate::git::StatusKind;
use crate::model::{RepoEvent, RepoModel};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, FILE_PREFIX, ROW_HEIGHT, tool_button};
use crate::ui::diff_view::DiffSource;

pub enum CommitEvent {
    OpenDiff(DiffSource),
}

impl EventEmitter<CommitEvent> for CommitView {}

const CHANGES: &str = "grp:changes";
const UNVERSIONED: &str = "grp:unversioned";
const CHANGES_SCOPE: &str = "c:";
const UNVERSIONED_SCOPE: &str = "u:";

pub struct CommitView {
    model: Entity<RepoModel>,
    tree: Entity<TreeState>,
    message: Entity<TextareaState>,
    included: HashSet<String>,
    /// Every path we have seen, so new changes start included and
    /// unchecked ones stay unchecked across refreshes.
    known: HashSet<String>,
    kinds: HashMap<String, StatusKind>,
    counts: HashMap<SharedString, usize>,
    amend: bool,
    last_selection: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl CommitView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let message = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder("Commit Message"));
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event, cx| {
                if matches!(event, RepoEvent::Reloaded) {
                    this.rebuild(cx);
                }
            }),
            cx.subscribe(&message, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe(&tree, |this, tree, cx| {
                let selected = tree.read(cx).selected_item().map(|i| i.id.clone());
                if selected != this.last_selection {
                    this.last_selection = selected.clone();
                    if let Some(source) = selected.and_then(|id| this.diff_source(&id)) {
                        cx.emit(CommitEvent::OpenDiff(source));
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
            tree,
            message,
            included: HashSet::new(),
            known: HashSet::new(),
            kinds: HashMap::new(),
            counts: HashMap::new(),
            amend: false,
            last_selection: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    fn path_of(id: &str) -> Option<(&str, bool)> {
        if let Some(rest) = id.strip_prefix(CHANGES_SCOPE) {
            return rest.strip_prefix(FILE_PREFIX).map(|p| (p, false));
        }
        id.strip_prefix(UNVERSIONED_SCOPE)?.strip_prefix(FILE_PREFIX).map(|p| (p, true))
    }

    fn diff_source(&self, id: &str) -> Option<DiffSource> {
        let (path, unversioned) = Self::path_of(id)?;
        Some(DiffSource::WorkingTree { path: path.to_owned(), unversioned })
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let status = self.model.read(cx).status().clone();
        self.kinds = status.entries.iter().map(|e| (e.path.clone(), e.kind)).collect();
        // IntelliJ includes tracked changes by default and leaves unversioned files out.
        for entry in status.changes() {
            if self.known.insert(entry.path.clone()) {
                self.included.insert(entry.path.clone());
            }
        }
        for entry in status.unversioned() {
            self.known.insert(entry.path.clone());
        }
        self.included.retain(|path| self.kinds.contains_key(path));

        let changes: Vec<String> = status.changes().map(|e| e.path.clone()).collect();
        let unversioned: Vec<String> = status.unversioned().map(|e| e.path.clone()).collect();
        let mut items = vec![TreeItem::new(CHANGES, "Changes")
            .expanded(true)
            .children(common::file_tree(changes, CHANGES_SCOPE))];
        if !unversioned.is_empty() {
            items.push(
                TreeItem::new(UNVERSIONED, "Unversioned Files")
                    .expanded(true)
                    .children(common::file_tree(unversioned, UNVERSIONED_SCOPE)),
            );
        }
        self.counts.clear();
        common::count_files(&items, &mut self.counts);
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        cx.notify();
    }

    /// Paths under a tree node (a file, a directory, or a whole group).
    fn paths_under(&self, id: &str) -> Vec<String> {
        if let Some((path, _)) = Self::path_of(id) {
            return vec![path.to_owned()];
        }
        let (scope, dir) = if id == CHANGES {
            (CHANGES_SCOPE, None)
        } else if id == UNVERSIONED {
            (UNVERSIONED_SCOPE, None)
        } else if let Some(dir) = id.strip_prefix(CHANGES_SCOPE).and_then(|r| r.strip_prefix(common::DIR_PREFIX)) {
            (CHANGES_SCOPE, Some(dir))
        } else if let Some(dir) = id.strip_prefix(UNVERSIONED_SCOPE).and_then(|r| r.strip_prefix(common::DIR_PREFIX)) {
            (UNVERSIONED_SCOPE, Some(dir))
        } else {
            return Vec::new();
        };
        let want_unversioned = scope == UNVERSIONED_SCOPE;
        self.kinds
            .iter()
            .filter(|(_, kind)| (**kind == StatusKind::Unversioned) == want_unversioned)
            .map(|(path, _)| path)
            .filter(|path| dir.is_none_or(|dir| path.starts_with(&format!("{dir}/"))))
            .cloned()
            .collect()
    }

    fn toggle(&mut self, id: &str, value: bool, cx: &mut Context<Self>) {
        for path in self.paths_under(id) {
            if value {
                self.included.insert(path);
            } else {
                self.included.remove(&path);
            }
        }
        cx.notify();
    }

    fn set_amend(&mut self, amend: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = amend;
        if amend && self.message.read(cx).value().trim().is_empty() {
            if let Some(last) = self.model.read(cx).repository().and_then(status::last_commit_message) {
                self.message.update(cx, |state, cx| state.set_value(last, window, cx));
            }
        }
        cx.notify();
    }

    fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_owned();
        if message.is_empty() {
            return;
        }
        let mut paths: Vec<String> = self.included.iter().cloned().collect();
        paths.sort();
        let unversioned: Vec<String> =
            paths.iter().filter(|p| self.kinds.get(*p) == Some(&StatusKind::Unversioned)).cloned().collect();
        let request = CommitRequest { message, amend: self.amend, paths, unversioned, sign_off: false };
        let count = request.paths.len();
        let branch = self.model.read(cx).refs().current_branch.clone();
        self.model.update(cx, |model, cx| {
            model.run_operation(if push { "Commit and Push" } else { "Commit" }, move |repo| {
                let hash = status::commit(repo, &request)?;
                let mut summary = format!("{count} file{} committed: {}", if count == 1 { "" } else { "s" }, &hash[..8]);
                if push {
                    let has_upstream = repo.run(["rev-parse", "--abbrev-ref", "@{upstream}"]).is_ok();
                    match (&branch, has_upstream) {
                        (_, true) => repo.run(["push"])?,
                        (Some(branch), false) => repo.run(["push", "-u", "origin", branch])?,
                        (None, false) => anyhow::bail!("cannot push a detached HEAD"),
                    };
                    summary.push_str(", pushed");
                }
                Ok(summary)
            }, cx)
        });
        self.amend = false;
        self.message.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }
}

impl Render for CommitView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let included = self.included.clone();
        let kinds = self.kinds.clone();
        let counts = self.counts.clone();
        let entity = cx.entity();
        let can_commit = !self.message.read(cx).value().trim().is_empty() && (!self.included.is_empty() || self.amend);
        let busy = self.model.read(cx).busy().is_some();

        // A group/dir checkbox is checked when every file under it is included.
        let paths_by_node: HashMap<SharedString, Vec<String>> = counts
            .keys()
            .map(|id| (id.clone(), self.paths_under(id)))
            .collect();

        let tree_palette = palette.clone();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(32.))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("commit-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(
                        |this, _, _, cx| this.model.update(cx, |m, cx| m.reload(cx)),
                    )))
                    .child(tool_button("commit-rollback", IconName::Undo2, "Rollback…").on_click(cx.listener(
                        |this, _, _, cx| {
                            let paths: Vec<String> = this
                                .included
                                .iter()
                                .filter(|p| this.kinds.get(*p) != Some(&StatusKind::Unversioned))
                                .cloned()
                                .collect();
                            if paths.is_empty() {
                                return;
                            }
                            this.model.update(cx, |m, cx| {
                                m.run_operation("Rollback", move |repo| {
                                    let mut args = vec!["restore".to_owned(), "--staged".into(), "--worktree".into(), "--source=HEAD".into(), "--".into()];
                                    args.extend(paths.iter().cloned());
                                    repo.run(&args)?;
                                    Ok(format!("Rolled back {} files", paths.len()))
                                }, cx)
                            });
                        },
                    )))
                    .child(tool_button("commit-diff", IconName::FileDiff, "Show Diff").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(source) = this.last_selection.clone().and_then(|id| this.diff_source(&id)) {
                                cx.emit(CommitEvent::OpenDiff(source));
                            }
                        },
                    ))),
            )
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.tree, move |ix, entry, _, _, _| {
                        let palette = &tree_palette;
                        let item = entry.item();
                        let id = item.id.clone();
                        let file = CommitView::path_of(&id).map(|(p, _)| p.to_owned());
                        let checked = match &file {
                            Some(path) => included.contains(path),
                            None => paths_by_node.get(&id).is_some_and(|paths| !paths.is_empty() && paths.iter().all(|p| included.contains(p))),
                        };
                        let color = file
                            .as_ref()
                            .and_then(|p| kinds.get(p))
                            .map_or(palette.text, |k| common::status_color(*k, &palette));
                        let is_group = id.as_ref() == CHANGES || id.as_ref() == UNVERSIONED;
                        let toggle_entity = entity.clone();
                        let toggle_id = id.clone();
                        ListItem::new(ix)
                            .py_0()
                            .px_1()
                            .h(px(ROW_HEIGHT))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .pl(px(entry.depth() as f32 * 14.))
                                    .text_sm()
                                    .child(if entry.is_folder() {
                                        Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                            .xsmall()
                                            .text_color(palette.text_secondary)
                                    } else {
                                        Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                                    })
                                    .child(
                                        Checkbox::new(SharedString::from(format!("check-{id}")))
                                            .checked(checked)
                                            .on_change(move |value, _, cx| {
                                                let id = toggle_id.clone();
                                                toggle_entity.update(cx, |this, cx| this.toggle(&id, *value, cx));
                                            }),
                                    )
                                    .when(!is_group, |el| {
                                        el.child(
                                            Icon::new(match &file {
                                                Some(p) => common::file_icon(p),
                                                None => IconName::Folder,
                                            })
                                            .small()
                                            .text_color(palette.text_secondary),
                                        )
                                    })
                                    .child(div().text_color(color).child(item.label.clone()))
                                    .when(file.is_none(), |el| {
                                        let n = counts.get(&id).copied().unwrap_or(0);
                                        el.child(
                                            div()
                                                .text_xs()
                                                .text_color(palette.text_secondary)
                                                .child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                        )
                                    }),
                            )
                    })
                    .size_full(),
                ),
            )
            .child(
                v_flex()
                    .border_t_1()
                    .border_color(palette.border)
                    .p_2()
                    .gap_2()
                    .child(
                        h_flex().gap_3().child(
                            Checkbox::new("amend")
                                .label("Amend")
                                .checked(self.amend)
                                .on_change(cx.listener(|this, value, window, cx| this.set_amend(*value, window, cx))),
                        ),
                    )
                    .child(Textarea::new(&self.message).h(px(110.)))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("do-commit")
                                    .primary()
                                    .small()
                                    .label(if self.amend { "Amend Commit" } else { "Commit" })
                                    .disabled(!can_commit || busy)
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(false, window, cx))),
                            )
                            .child(
                                Button::new("do-commit-push")
                                    .outline()
                                    .small()
                                    .label(if self.amend { "Amend Commit and Push…" } else { "Commit and Push…" })
                                    .disabled(!can_commit || busy)
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(true, window, cx))),
                            ),
                    ),
            )
    }
}

//! The Commit tool window (non-modal commit): changes with checkboxes,
//! unversioned files, Amend, the message editor, and Commit / Commit and Push.
//! With "Enable staging area" on, it shows Staged / Unstaged trees with
//! Stage / Unstage actions instead of checkboxes, like IntelliJ.

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
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::status::{self, CommitRequest};
use crate::git::StatusKind;
use crate::git::merge::{self, Conflict};
use crate::model::{RepoEvent, RepoModel};
use crate::settings::Settings;
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, FILE_PREFIX, ROW_HEIGHT, tool_button};
use crate::ui::diff_view::DiffSource;

pub enum CommitEvent {
    OpenDiff(DiffSource),
    /// "Commit and Push…" committed; show the Push dialog next.
    OpenPush,
    /// A conflicted file was picked: open the merge tool.
    OpenMerge(Conflict),
}

impl EventEmitter<CommitEvent> for CommitView {}

/// A top-level node of the changes tree. Node ids are `<scope><f:|d:><path>`.
struct Group {
    id: &'static str,
    scope: &'static str,
    label: &'static str,
    files: Vec<(String, StatusKind)>,
}

const CHANGES_SCOPE: &str = "c:";
const UNVERSIONED_SCOPE: &str = "u:";
const STAGED_SCOPE: &str = "s:";
const UNSTAGED_SCOPE: &str = "w:";
const CONFLICTS_SCOPE: &str = "m:";

pub struct CommitView {
    model: Entity<RepoModel>,
    tree: Entity<TreeState>,
    message: Entity<TextareaState>,
    groups: Vec<Group>,
    staging: bool,
    included: HashSet<String>,
    /// Every path we have seen, so new changes start included and
    /// unchecked ones stay unchecked across refreshes.
    known: HashSet<String>,
    kinds: HashMap<String, StatusKind>,
    counts: HashMap<SharedString, usize>,
    amend: bool,
    push_after_commit: bool,
    last_selection: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl CommitView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let message = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder("Commit Message"));
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event, cx| match event {
                RepoEvent::Reloaded => this.rebuild(cx),
                RepoEvent::Notify { title, error, .. } if title == "Commit" => {
                    if std::mem::take(&mut this.push_after_commit) && !error {
                        cx.emit(CommitEvent::OpenPush);
                    }
                }
                _ => {}
            }),
            cx.observe_global::<Settings>(|this, cx| {
                if Settings::get(cx).staging_area != this.staging {
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
                    let path = selected.as_deref().and_then(Self::path_of).map(|(_, p)| p.to_owned());
                    let conflict = path.and_then(|path| {
                        merge::conflicts(this.model.read(cx).status()).into_iter().find(|c| c.path == path)
                    });
                    if let Some(conflict) = conflict.filter(|c| c.kind.can_merge()) {
                        cx.emit(CommitEvent::OpenMerge(conflict));
                    } else if let Some(source) = selected.and_then(|id| this.diff_source(&id)) {
                        cx.emit(CommitEvent::OpenDiff(source));
                    }
                }
            }),
        ];
        let mut this = Self {
            model,
            tree,
            message,
            groups: Vec::new(),
            staging: false,
            included: HashSet::new(),
            known: HashSet::new(),
            kinds: HashMap::new(),
            counts: HashMap::new(),
            amend: false,
            push_after_commit: false,
            last_selection: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    /// The scope and file path of a file node.
    fn path_of(id: &str) -> Option<(&str, &str)> {
        [CHANGES_SCOPE, UNVERSIONED_SCOPE, STAGED_SCOPE, UNSTAGED_SCOPE, CONFLICTS_SCOPE].into_iter().find_map(|scope| {
            id.strip_prefix(scope)?.strip_prefix(FILE_PREFIX).map(|path| (scope, path))
        })
    }

    fn diff_source(&self, id: &str) -> Option<DiffSource> {
        let (scope, path) = Self::path_of(id)?;
        let path = path.to_owned();
        Some(match scope {
            STAGED_SCOPE => DiffSource::Staged { path },
            UNSTAGED_SCOPE => DiffSource::Unstaged { path },
            _ => DiffSource::WorkingTree { path, unversioned: scope == UNVERSIONED_SCOPE },
        })
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let status = self.model.read(cx).status().clone();
        self.staging = Settings::get(cx).staging_area;
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

        let unversioned: Vec<(String, StatusKind)> = status.unversioned().map(|e| (e.path.clone(), e.kind)).collect();
        // Conflicted files get their own "Merge Conflicts" node, as in IntelliJ.
        let conflicted: Vec<(String, StatusKind)> =
            status.entries.iter().filter(|e| e.kind == StatusKind::Conflicted).map(|e| (e.path.clone(), e.kind)).collect();
        let not_conflicted = |files: Vec<(String, StatusKind)>| -> Vec<(String, StatusKind)> {
            files.into_iter().filter(|(_, k)| *k != StatusKind::Conflicted).collect()
        };
        let conflicts_group = Group { id: "grp:conflicts", scope: CONFLICTS_SCOPE, label: "Merge Conflicts", files: conflicted };
        let mut groups = if self.staging {
            vec![
                Group { id: "grp:staged", scope: STAGED_SCOPE, label: "Staged", files: status.staged() },
                Group { id: "grp:unstaged", scope: UNSTAGED_SCOPE, label: "Unstaged", files: not_conflicted(status.unstaged()) },
                Group { id: "grp:unversioned", scope: UNVERSIONED_SCOPE, label: "Unversioned Files", files: unversioned },
            ]
        } else {
            vec![
                Group {
                    id: "grp:changes",
                    scope: CHANGES_SCOPE,
                    label: "Changes",
                    files: not_conflicted(status.changes().map(|e| (e.path.clone(), e.kind)).collect()),
                },
                Group { id: "grp:unversioned", scope: UNVERSIONED_SCOPE, label: "Unversioned Files", files: unversioned },
            ]
        };
        if !conflicts_group.files.is_empty() {
            groups.insert(0, conflicts_group);
        }
        self.groups = groups;
        let items: Vec<TreeItem> = self
            .groups
            .iter()
            // The first group always shows, as IntelliJ's default changelist does.
            .enumerate()
            .filter(|(_, group)| !group.files.is_empty() || matches!(group.id, "grp:changes" | "grp:staged"))
            .map(|(_, group)| {
                TreeItem::new(group.id, group.label)
                    .expanded(true)
                    .children(common::file_tree(group.files.iter().map(|(p, _)| p.clone()), group.scope))
            })
            .collect();
        self.counts.clear();
        common::count_files(&items, &mut self.counts);
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        cx.notify();
    }

    fn group_of(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id || id.starts_with(g.scope))
    }

    /// Paths under a tree node (a file, a directory, or a whole group).
    fn paths_under(&self, id: &str) -> Vec<String> {
        if let Some((_, path)) = Self::path_of(id) {
            return vec![path.to_owned()];
        }
        let Some(group) = self.group_of(id) else { return Vec::new() };
        let dir = id.strip_prefix(group.scope).and_then(|r| r.strip_prefix(common::DIR_PREFIX));
        group
            .files
            .iter()
            .map(|(path, _)| path)
            .filter(|path| dir.is_none_or(|dir| path.starts_with(&format!("{dir}/"))))
            .cloned()
            .collect()
    }

    /// Stage (unstaged / unversioned nodes) or unstage (staged nodes).
    fn stage_node(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(group) = self.group_of(id) else { return };
        let unstage = group.scope == STAGED_SCOPE;
        let paths = self.paths_under(id);
        if paths.is_empty() {
            return;
        }
        let title = if unstage { "Unstage" } else { "Stage" };
        self.model.update(cx, |model, cx| {
            model.run_operation(title, move |repo| {
                if unstage { status::unstage(repo, &paths)? } else { status::stage(repo, &paths)? }
                Ok(String::new())
            }, cx)
        });
    }

    fn stage_selected(&mut self, unstage: bool, cx: &mut Context<Self>) {
        let target = match self.last_selection.clone() {
            Some(id) if (self.group_of(&id).map(|g| g.scope) == Some(STAGED_SCOPE)) == unstage => id.to_string(),
            // Nothing suitable selected: act on the whole group, like "Stage All".
            _ => if unstage { "grp:staged" } else { "grp:unstaged" }.to_owned(),
        };
        self.stage_node(&target, cx);
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

    pub fn focus_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |state, cx| state.focus(window, cx));
    }

    fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_owned();
        if message.is_empty() {
            return;
        }
        let staged_only = self.staging;
        let mut paths: Vec<String> = if staged_only { Vec::new() } else { self.included.iter().cloned().collect() };
        paths.sort();
        let unversioned: Vec<String> =
            paths.iter().filter(|p| self.kinds.get(*p) == Some(&StatusKind::Unversioned)).cloned().collect();
        let count = if staged_only {
            self.groups.iter().find(|g| g.scope == STAGED_SCOPE).map_or(0, |g| g.files.len())
        } else {
            paths.len()
        };
        let request = CommitRequest { message, amend: self.amend, paths, unversioned, sign_off: false, staged_only };
        self.push_after_commit = push;
        self.model.update(cx, |model, cx| {
            model.run_operation("Commit", move |repo| {
                let hash = status::commit(repo, &request)?;
                Ok(format!("{count} file{} committed: {}", if count == 1 { "" } else { "s" }, &hash[..8]))
            }, cx)
        });
        self.amend = false;
        self.message.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }
}

impl CommitView {
    /// Rollback: checked files, or in staging mode the selected node
    /// (unstaged changes are restored from the index, staged ones from HEAD).
    fn rollback(&mut self, cx: &mut Context<Self>) {
        let (paths, from_index) = if self.staging {
            let Some(id) = self.last_selection.clone() else { return };
            let scope = self.group_of(&id).map(|g| g.scope);
            if scope == Some(UNVERSIONED_SCOPE) {
                return;
            }
            (self.paths_under(&id), scope == Some(UNSTAGED_SCOPE))
        } else {
            let paths = self
                .included
                .iter()
                .filter(|p| self.kinds.get(*p) != Some(&StatusKind::Unversioned))
                .cloned()
                .collect();
            (paths, false)
        };
        if paths.is_empty() {
            return;
        }
        self.model.update(cx, |m, cx| {
            m.run_operation("Rollback", move |repo| {
                let mut args: Vec<String> = if from_index {
                    vec!["restore".into(), "--worktree".into(), "--".into()]
                } else {
                    vec!["restore".into(), "--staged".into(), "--worktree".into(), "--source=HEAD".into(), "--".into()]
                };
                args.extend(paths.iter().cloned());
                repo.run(&args)?;
                Ok(format!("Rolled back {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
            }, cx)
        });
    }
}

impl Render for CommitView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let staging = self.staging;
        let included = self.included.clone();
        let counts = self.counts.clone();
        let entity = cx.entity();
        let staged_count = self.groups.iter().find(|g| g.scope == STAGED_SCOPE).map_or(0, |g| g.files.len());
        let has_changes = if staging { staged_count > 0 } else { !self.included.is_empty() };
        let can_commit = !self.message.read(cx).value().trim().is_empty() && (has_changes || self.amend);
        let busy = self.model.read(cx).busy().is_some();

        // Per node: the paths under it (for group/dir checkboxes), its color,
        // and whether it is a group or belongs to the Staged tree.
        let paths_by_node: HashMap<SharedString, Vec<String>> =
            counts.keys().map(|id| (id.clone(), self.paths_under(id))).collect();
        let kinds: HashMap<String, StatusKind> = self
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(move |(p, k)| (format!("{}{}{}", g.scope, FILE_PREFIX, p), *k)))
            .collect();
        let group_ids: Vec<&'static str> = self.groups.iter().map(|g| g.id).collect();

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
                    .child(
                        tool_button("commit-rollback", IconName::Undo2, "Rollback…")
                            .on_click(cx.listener(|this, _, _, cx| this.rollback(cx))),
                    )
                    .child(tool_button("commit-diff", IconName::FileDiff, "Show Diff").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(source) = this.last_selection.clone().and_then(|id| this.diff_source(&id)) {
                                cx.emit(CommitEvent::OpenDiff(source));
                            }
                        },
                    )))
                    .when(staging, |el| {
                        el.child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                            .child(
                                tool_button("commit-stage", IconName::Plus, "Stage")
                                    .on_click(cx.listener(|this, _, _, cx| this.stage_selected(false, cx))),
                            )
                            .child(
                                tool_button("commit-unstage", IconName::Minus, "Unstage")
                                    .on_click(cx.listener(|this, _, _, cx| this.stage_selected(true, cx))),
                            )
                    }),
            )
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.tree, move |ix, entry, _, _, _| {
                        let palette = &tree_palette;
                        let item = entry.item();
                        let id = item.id.clone();
                        let file = CommitView::path_of(&id).map(|(_, p)| p.to_owned());
                        let checked = match &file {
                            Some(path) => included.contains(path),
                            None => paths_by_node.get(&id).is_some_and(|paths| !paths.is_empty() && paths.iter().all(|p| included.contains(p))),
                        };
                        let color = kinds.get(id.as_ref()).map_or(palette.text, |k| common::status_color(*k, palette));
                        let is_group = group_ids.contains(&id.as_ref());
                        let in_staged = id.starts_with(STAGED_SCOPE) || id.as_ref() == "grp:staged";
                        let in_conflicts = id.starts_with(CONFLICTS_SCOPE) || id.as_ref() == "grp:conflicts";
                        let toggle_entity = entity.clone();
                        let toggle_id = id.clone();
                        let stage_entity = entity.clone();
                        let stage_id = id.clone();
                        let n = counts.get(&id).copied().unwrap_or(0);
                        ListItem::new(ix).py_0().px_1().h(px(ROW_HEIGHT)).child(
                            h_flex()
                                .w_full()
                                .gap_1()
                                .group("commit-row")
                                .pl(px(entry.depth() as f32 * 14.))
                                .text_sm()
                                .child(if entry.is_folder() {
                                    Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight })
                                        .xsmall()
                                        .text_color(palette.text_secondary)
                                } else {
                                    Icon::new(IconName::Circle).xsmall().text_color(gpui_kit::transparent_black())
                                })
                                .when(!staging && !id.starts_with(CONFLICTS_SCOPE) && id.as_ref() != "grp:conflicts", |el| {
                                    el.child(
                                        Checkbox::new(SharedString::from(format!("check-{id}")))
                                            .checked(checked)
                                            .on_change(move |value, _, cx| {
                                                let id = toggle_id.clone();
                                                toggle_entity.update(cx, |this, cx| this.toggle(&id, *value, cx));
                                            }),
                                    )
                                })
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
                                    el.child(
                                        div()
                                            .text_xs()
                                            .text_color(palette.text_secondary)
                                            .child(format!("{n} {}", if n == 1 { "file" } else { "files" })),
                                    )
                                })
                                // Staging mode: a +/- button appears on hover, as in IntelliJ.
                                .when(staging && (n > 0 || file.is_some()) && !in_conflicts, |el| {
                                    el.child(div().flex_1()).child(
                                        div().opacity(0.).group_hover("commit-row", |s| s.opacity(1.)).child(
                                            tool_button(
                                                SharedString::from(format!("stage-{id}")),
                                                if in_staged { IconName::Minus } else { IconName::Plus },
                                                if in_staged { "Unstage" } else { "Stage" },
                                            )
                                            .on_click(move |_, _, cx| {
                                                let id = stage_id.clone();
                                                stage_entity.update(cx, |this, cx| this.stage_node(&id, cx));
                                            }),
                                        ),
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

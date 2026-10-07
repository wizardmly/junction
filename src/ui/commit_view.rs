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
    menu::{ContextMenuExt as _, PopupMenuItem},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    popover::Popover,
    menu::DropdownMenu as _,
    WindowExt as _,
    list::ListItem,
    tree::{TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::git::changelists::{self, Changelists};
use crate::git::status::{self, CommitRequest};
use crate::git::StatusKind;
use crate::git::merge::{self, Conflict};
use crate::model::{ExcludedHunks, RepoEvent, RepoModel};
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
    /// Annotate the work tree version of a file.
    Annotate(String),
    ShowHistory(String),
    /// "Compare with Branch / Revision…" for a file.
    CompareWith(String),
}

impl EventEmitter<CommitEvent> for CommitView {}

gpui_kit::actions!(commit_view, [ShowMessageHistory]);

const CONTEXT: &str = "CommitView";

pub fn init(cx: &mut gpui_kit::App) {
    // Commit Message History: Ctrl+M on every platform, as in IntelliJ.
    cx.bind_keys([gpui_kit::KeyBinding::new("ctrl-m", ShowMessageHistory, Some(CONTEXT))]);
}

/// A top-level node of the changes tree. Node ids are `<scope><f:|d:><path>`.
struct Group {
    id: String,
    scope: String,
    label: String,
    files: Vec<(String, StatusKind)>,
    /// The changelist this group shows (changelist mode only).
    changelist: Option<String>,
}

impl Group {
    fn new(id: &str, scope: &str, label: &str, files: Vec<(String, StatusKind)>) -> Self {
        Self { id: id.into(), scope: scope.into(), label: label.into(), files, changelist: None }
    }
}

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
    /// Commit Options popover: "Author" override and "GPG-sign" (defaults to `commit.gpgSign`).
    author: Entity<InputState>,
    gpg_sign: Option<bool>,
    changelists: Changelists,
    changelists_root: Option<std::path::PathBuf>,
    _subscriptions: Vec<Subscription>,
}

impl CommitView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let message = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder("Commit Message"));
        let author = cx.new(|cx| InputState::new(window, cx).placeholder("Name <email>"));
        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, event, window, cx| match event {
                RepoEvent::Reloaded => this.rebuild(cx),
                RepoEvent::PrefillCommitMessage(message) => {
                    let message = format!("{message}\n\n");
                    this.message.update(cx, |state, cx| {
                        state.set_value(message, window, cx);
                        state.focus(window, cx);
                    });
                }
                RepoEvent::Notify { title, error, .. } if title == "Commit" => {
                    if std::mem::take(&mut this.push_after_commit) && !error {
                        cx.emit(CommitEvent::OpenPush);
                    }
                }
                _ => {}
            }),
            cx.observe_global::<ExcludedHunks>(|_, cx| cx.notify()),
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
            author,
            gpg_sign: None,
            changelists: Changelists::default(),
            changelists_root: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    /// The scope and file path of a file node.
    fn path_of(id: &str) -> Option<(&str, &str)> {
        let end = id.find(':')? + 1;
        let (scope, rest) = id.split_at(end);
        if scope == "grp:" {
            return None;
        }
        rest.strip_prefix(FILE_PREFIX).map(|path| (scope, path))
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
        let conflicts_group = Group::new("grp:conflicts", CONFLICTS_SCOPE, "Merge Conflicts", conflicted);
        let mut groups = if self.staging {
            vec![
                Group::new("grp:staged", STAGED_SCOPE, "Staged", status.staged()),
                Group::new("grp:unstaged", UNSTAGED_SCOPE, "Unstaged", not_conflicted(status.unstaged())),
                Group::new("grp:unversioned", UNVERSIONED_SCOPE, "Unversioned Files", unversioned),
            ]
        } else {
            // One group per changelist, the active one's new changes landing in it.
            let changes = not_conflicted(status.changes().map(|e| (e.path.clone(), e.kind)).collect());
            let root = self.model.read(cx).repository().map(|r| r.root().to_path_buf());
            if root != self.changelists_root {
                self.changelists = self.model.read(cx).repository().map(Changelists::load).unwrap_or_default();
                self.changelists_root = root;
            }
            let changed: Vec<String> = changes.iter().map(|(p, _)| p.clone()).collect();
            if self.changelists.sync(&changed) {
                self.save_changelists(cx);
            }
            let mut groups: Vec<Group> = self
                .changelists
                .lists
                .iter()
                .enumerate()
                .map(|(ix, list)| Group {
                    id: format!("grp:cl{ix}"),
                    scope: format!("c{ix}:"),
                    label: list.name.clone(),
                    files: changes.iter().filter(|(p, _)| self.changelists.list_of(p) == list.name).cloned().collect(),
                    changelist: Some(list.name.clone()),
                })
                .collect();
            groups.push(Group::new("grp:unversioned", UNVERSIONED_SCOPE, "Unversioned Files", unversioned));
            groups
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
            .filter(|(_, group)| !group.files.is_empty() || group.changelist.is_some() || group.id == "grp:staged")
            .map(|(_, group)| {
                TreeItem::new(group.id.clone(), group.label.clone())
                    .expanded(true)
                    .children(common::file_tree(group.files.iter().map(|(p, _)| p.clone()), &group.scope))
            })
            .collect();
        self.counts.clear();
        common::count_files(&items, &mut self.counts);
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        cx.notify();
    }

    fn group_of(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id || id.starts_with(g.scope.as_str()))
    }

    /// Paths under a tree node (a file, a directory, or a whole group).
    /// Files for Shelve / Create Patch: the checked files (the selected node
    /// in staging mode), or `file` when it isn't among them.
    fn action_paths(&self, file: Option<&str>) -> Vec<String> {
        if let Some(file) = file {
            if !self.included.contains(file) {
                return vec![file.to_owned()];
            }
        }
        if self.staging {
            return self.last_selection.as_deref().map(|id| self.paths_under(id)).unwrap_or_default();
        }
        let mut paths: Vec<String> = self.included.iter().cloned().collect();
        paths.sort();
        paths
    }

    pub fn shelve(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.action_paths(None);
        let name = self.message.read(cx).value().lines().next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let name = name.unwrap_or_else(|| crate::ui::patch_dialogs::default_shelf_name(&paths));
        crate::ui::patch_dialogs::shelve(self.model.clone(), paths, name, window, cx);
    }

    pub fn create_patch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.action_paths(None);
        if paths.is_empty() {
            self.model.update(cx, |m, cx| m.notify("Create Patch", "Select the files to include", true, cx));
            return;
        }
        let source = crate::ui::patch_dialogs::PatchSource::Local { paths };
        crate::ui::patch_dialogs::create_patch(self.model.clone(), source, window, cx);
    }

    fn save_changelists(&self, cx: &Context<Self>) {
        if let Some(repository) = self.model.read(cx).repository() {
            self.changelists.save(repository);
        }
    }

    fn update_changelists(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Changelists)) {
        f(&mut self.changelists);
        self.save_changelists(cx);
        self.rebuild(cx);
    }

    /// Move to Another Changelist (F6) for the clicked file.
    fn move_to_changelist(&mut self, path: &str, target: &str, cx: &mut Context<Self>) {
        let paths = vec![path.to_owned()];
        self.update_changelists(cx, |lists| lists.move_files(&paths, target));
    }

    /// New Changelist… (optionally moving `path` into it), or Edit Changelist… when `edit` names one.
    fn changelist_dialog(&mut self, edit: Option<String>, move_path: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (name, comment) = match &edit {
            Some(list) => (list.clone(), self.changelists.comment(list).to_owned()),
            None => (String::new(), String::new()),
        };
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name").default_value(name));
        let comment_input =
            cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder("Comment (used as the commit message)").default_value(comment));
        let active = std::rc::Rc::new(std::cell::Cell::new(edit.is_none()));
        let renaming_default = edit.as_deref() == Some(changelists::DEFAULT_NAME);
        let entity = cx.entity();
        let focus = name_input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let (name_input, comment_input) = (name_input.clone(), comment_input.clone());
            let (active_set, active_ok) = (active.clone(), active.clone());
            let entity = entity.clone();
            let edit = edit.clone();
            let move_path = move_path.clone();
            let creating = edit.is_none();
            dialog
                .title(if creating { "New Changelist" } else { "Edit Changelist" })
                .w(px(440.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(Input::new(&name_input).disabled(renaming_default))
                        .child(Textarea::new(&comment_input))
                        .when(creating, |el| {
                            el.child(Checkbox::new("cl-active").label("Set active").checked(active.get()).on_change(
                                move |value, window, _| {
                                    active_set.set(*value);
                                    window.refresh();
                                },
                            ))
                        }),
                )
                .on_ok(move |_, _, cx| {
                    let name = name_input.read(cx).value().trim().to_owned();
                    let comment = comment_input.read(cx).value().trim().to_owned();
                    let make_active = active_ok.get();
                    let edit = edit.clone();
                    let move_path = move_path.clone();
                    entity.update(cx, |this, cx| {
                        let ok = match &edit {
                            Some(old) => this.changelists.rename(old, &name, &comment),
                            None => this.changelists.add(&name, &comment, make_active),
                        };
                        if !ok {
                            return false;
                        }
                        if let Some(path) = move_path {
                            this.changelists.move_files(&[path], &name);
                        }
                        this.save_changelists(cx);
                        this.rebuild(cx);
                        true
                    })
                })
                .footer(crate::ui::dialogs::footer(if creating { "Create" } else { "OK" }))
        });
        crate::ui::dialogs::focus_input(&focus, window, cx);
    }

    fn new_changelist(&mut self, move_path: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.changelist_dialog(None, move_path, window, cx);
    }

    fn set_active_changelist(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        // IntelliJ puts the changelist's comment in an empty commit message.
        let comment = self.changelists.comment(&name).to_owned();
        if !comment.is_empty() && self.message.read(cx).value().trim().is_empty() {
            self.message.update(cx, |state, cx| state.set_value(comment, window, cx));
        }
        self.update_changelists(cx, |lists| lists.active = name);
    }

    fn paths_under(&self, id: &str) -> Vec<String> {
        if let Some((_, path)) = Self::path_of(id) {
            return vec![path.to_owned()];
        }
        let Some(group) = self.group_of(id) else { return Vec::new() };
        let dir = id.strip_prefix(group.scope.as_str()).and_then(|r| r.strip_prefix(common::DIR_PREFIX));
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
            Some(id) if (self.group_of(&id).map(|g| g.scope.as_str()) == Some(STAGED_SCOPE)) == unstage => id.to_string(),
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

    /// Commit, after IntelliJ's pre-commit checks: detached HEAD, CRLF line
    /// separators, and files too large for hosting services.
    fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.message.read(cx).value().trim().is_empty() {
            return;
        }
        let Some(repository) = self.model.read(cx).repository().cloned() else { return };
        let settings = Settings::get(cx).clone();
        let paths = self.commit_paths(cx);
        let mut warnings: Vec<String> = Vec::new();
        if settings.warn_detached_head && self.model.read(cx).refs().current_branch.is_none() {
            warnings.push(match self.model.read(cx).state() {
                crate::git::RepositoryState::Rebasing => "A rebase is in progress. The commit will be part of the rebased history.".into(),
                _ => "HEAD is detached: the commit won't belong to any branch and may be lost after checkout.".into(),
            });
        }
        let crlf = if settings.warn_crlf { status::crlf_files(&repository, &paths) } else { Vec::new() };
        if !crlf.is_empty() {
            warnings.push(format!(
                "{} file{} with CRLF line separators will be committed as is: {}",
                crlf.len(),
                if crlf.len() == 1 { "" } else { "s" },
                crlf.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        for (path, size) in status::large_files(&repository, &paths, 50 * 1024 * 1024) {
            warnings.push(format!("{path} is {} MB; most Git hosts reject files over 50 MB.", size / (1024 * 1024)));
        }
        if warnings.is_empty() {
            return self.do_commit(push, window, cx);
        }
        let entity = cx.entity();
        let has_crlf = !crlf.is_empty();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let (commit_entity, fix_entity) = (entity.clone(), entity.clone());
            let mut footer = gpui_kit::component::dialog::DialogFooter::new()
                .gap_2()
                .child(gpui_kit::component::dialog::DialogClose::new().child(Button::new("warn-cancel").label("Cancel").outline()));
            if has_crlf {
                footer = footer.child(gpui_kit::component::dialog::DialogClose::new().child(
                    Button::new("warn-fix").label("Fix and Commit").outline().on_click(move |_, window, cx| {
                        fix_entity.update(cx, |this, cx| {
                            if let Some(repo) = this.model.read(cx).repository() {
                                // Like IntelliJ: let git convert line separators on commit.
                                let value = if cfg!(windows) { "true" } else { "input" };
                                let _ = repo.run(["config", "core.autocrlf", value]);
                            }
                            this.do_commit(push, window, cx)
                        })
                    }),
                ));
            }
            footer = footer.child(gpui_kit::component::dialog::DialogClose::new().child(
                Button::new("warn-commit").label("Commit Anyway").primary().on_click(move |_, window, cx| {
                    commit_entity.update(cx, |this, cx| this.do_commit(push, window, cx))
                }),
            ));
            dialog
                .title("Commit")
                .w(px(520.))
                .child(v_flex().gap_2().children(warnings.iter().map(|w| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .text_sm()
                        .child(Icon::new(IconName::TriangleAlert).small().text_color(palette.status_conflict))
                        .child(div().flex_1().child(w.clone()))
                })))
                .footer(footer)
        });
    }

    /// The files a commit would include.
    fn commit_paths(&self, cx: &Context<Self>) -> Vec<String> {
        if self.staging {
            let _ = cx;
            self.groups.iter().find(|g| g.scope == STAGED_SCOPE).map(|g| g.files.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default()
        } else {
            let mut paths: Vec<String> = self.included.iter().cloned().collect();
            paths.sort();
            paths
        }
    }

    fn do_commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_owned();
        if message.is_empty() {
            return;
        }
        let staged_only = self.staging;
        let paths: Vec<String> = if staged_only { Vec::new() } else { self.commit_paths(cx) };
        let unversioned: Vec<String> =
            paths.iter().filter(|p| self.kinds.get(*p) == Some(&StatusKind::Unversioned)).cloned().collect();
        let count = if staged_only { self.commit_paths(cx).len() } else { paths.len() };
        let settings = Settings::get(cx).clone();
        let author = self.author.read(cx).value().trim().to_owned();
        let gpg_sign = self.gpg_default(cx);
        let request = CommitRequest {
            message: message.clone(),
            amend: self.amend,
            paths,
            unversioned,
            sign_off: settings.sign_off,
            staged_only,
            author: (!author.is_empty()).then_some(author),
            gpg_sign,
            run_hooks: settings.run_hooks,
            cleanup: settings.cleanup_message,
            excluded_hunks: if staged_only { Default::default() } else { ExcludedHunks::get(cx).clone() },
        };
        let committed = request.paths.clone();
        ExcludedHunks::update(cx, |map| map.retain(|path, _| !committed.contains(path)));
        crate::settings::remember_message(&message);
        self.push_after_commit = push;
        self.model.update(cx, |model, cx| {
            model.run_operation("Commit", move |repo| {
                let hash = status::commit(repo, &request)?;
                Ok(format!("{count} file{} committed: {}", if count == 1 { "" } else { "s" }, &hash[..8]))
            }, cx)
        });
        self.amend = false;
        // IntelliJ keeps the author override only for one commit.
        self.author.update(cx, |state, cx| state.set_value("", window, cx));
        self.message.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn on_message_history(&mut self, _: &ShowMessageHistory, window: &mut Window, cx: &mut Context<Self>) {
        let history = crate::settings::message_history();
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let mut list = v_flex().id("message-history").max_h(px(360.)).overflow_y_scroll().gap_px();
            if history.is_empty() {
                list = list.child(div().p_2().text_sm().text_color(palette.text_secondary).child("No recent commit messages"));
            }
            for (ix, message) in history.iter().enumerate() {
                let (entity, chosen) = (entity.clone(), message.clone());
                list = list.child(
                    v_flex()
                        .id(("history-message", ix))
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.hover))
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let chosen = chosen.clone();
                            entity.update(cx, |this, cx| {
                                this.message.update(cx, |state, cx| {
                                    state.set_value(chosen, window, cx);
                                    state.focus(window, cx);
                                })
                            });
                        })
                        .child(div().child(message.lines().next().unwrap_or_default().to_owned()))
                        .when(message.lines().count() > 1, |el| {
                            el.child(div().text_xs().text_color(palette.text_secondary).child(format!("{} more lines", message.lines().count() - 1)))
                        }),
                );
            }
            dialog.title("Commit Message History").w(px(520.)).child(list)
        });
    }

    fn gpg_default(&self, cx: &gpui_kit::App) -> bool {
        self.gpg_sign.unwrap_or_else(|| self.model.read(cx).repository().is_some_and(status::gpg_sign_default))
    }

    /// The gear next to Commit: IntelliJ's Commit Options.
    fn render_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let author = self.author.clone();
        Popover::new("commit-options")
            .anchor(gpui_kit::Anchor::BottomLeft)
            .trigger(
                Button::new("commit-options-button")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Settings))
                    .tooltip("Commit Options"),
            )
            .content(move |_, _, cx| {
                let palette = cx.palette().clone();
                let settings = Settings::get(cx).clone();
                let gpg = entity.read(cx).gpg_default(cx);
                let gpg_entity = entity.clone();
                let toggle = |id: &'static str, label: &'static str, value: bool, set: fn(&mut Settings, bool)| {
                    Checkbox::new(id).label(label).checked(value).on_change(move |v, _, cx| Settings::update(cx, |s| set(s, *v)))
                };
                v_flex()
                    .w(px(300.))
                    .gap_2()
                    .p_1()
                    .text_sm()
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Git"))
                    .child(h_flex().gap_2().child(div().w(px(50.)).child("Author:")).child(div().flex_1().child(Input::new(&author).small())))
                    .child(toggle("opt-signoff", "Sign-off commit", settings.sign_off, |s, v| s.sign_off = v))
                    .child(Checkbox::new("opt-gpg").label("Sign commit with GPG").checked(gpg).on_change(move |v, _, cx| {
                        gpg_entity.update(cx, |this, cx| {
                            this.gpg_sign = Some(*v);
                            cx.notify();
                        })
                    }))
                    .child(div().pt_1().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Before Commit"))
                    .child(toggle("opt-hooks", "Run Git hooks", settings.run_hooks, |s, v| s.run_hooks = v))
                    .child(toggle("opt-cleanup", "Clean up commit message", settings.cleanup_message, |s, v| s.cleanup_message = v))
                    .child(div().text_xs().text_color(palette.text_secondary).child("Removes # comment lines and surrounding whitespace"))
            })
    }
}

impl CommitView {
    /// Rollback: checked files, or in staging mode the selected node
    /// (unstaged changes are restored from the index, staged ones from HEAD).
    fn rollback(&mut self, cx: &mut Context<Self>) {
        let (paths, from_index) = if self.staging {
            let Some(id) = self.last_selection.clone() else { return };
            let scope = self.group_of(&id).map(|g| g.scope.clone());
            if scope.as_deref() == Some(UNVERSIONED_SCOPE) {
                return;
            }
            (self.paths_under(&id), scope.as_deref() == Some(UNSTAGED_SCOPE))
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

/// The context menu of a changelist node.
fn changelist_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    entity: &Entity<CommitView>,
    list: String,
) -> gpui_kit::component::menu::PopupMenu {
    let is_default = list == changelists::DEFAULT_NAME;
    let (new, active, edit, delete) = (entity.clone(), entity.clone(), entity.clone(), entity.clone());
    let (active_list, edit_list, delete_list) = (list.clone(), list.clone(), list.clone());
    menu.item(PopupMenuItem::new("New Changelist…").on_click(move |_, window, cx| {
        new.update(cx, |this, cx| this.new_changelist(None, window, cx))
    }))
    .item(PopupMenuItem::new("Set Active Changelist").on_click(move |_, window, cx| {
        let name = active_list.clone();
        active.update(cx, |this, cx| this.set_active_changelist(name, window, cx))
    }))
    .item(PopupMenuItem::new("Edit Changelist…").on_click(move |_, window, cx| {
        let name = edit_list.clone();
        edit.update(cx, |this, cx| this.changelist_dialog(Some(name), None, window, cx))
    }))
    .item(PopupMenuItem::new("Delete Changelist").disabled(is_default).on_click(move |_, _, cx| {
        let name = delete_list.clone();
        delete.update(cx, |this, cx| this.update_changelists(cx, |lists| lists.remove(&name)))
    }))
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
        let history_entity = cx.entity();
        // First-line length against Settings › Commit › subject limit.
        let subject_hint = {
            let text = self.message.read(cx).value();
            let len = text.lines().next().unwrap_or_default().chars().count();
            (len > 0).then_some((len, Settings::get(cx).commit_subject_limit))
        };

        // Per node: the paths under it (for group/dir checkboxes), its color,
        // and whether it is a group or belongs to the Staged tree.
        let paths_by_node: HashMap<SharedString, Vec<String>> =
            counts.keys().map(|id| (id.clone(), self.paths_under(id))).collect();
        let kinds: HashMap<String, StatusKind> = self
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(move |(p, k)| (format!("{}{}{}", g.scope, FILE_PREFIX, p), *k)))
            .collect();
        let group_ids: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        let changelist_of: HashMap<String, String> =
            self.groups.iter().filter_map(|g| g.changelist.clone().map(|c| (g.id.clone(), c))).collect();
        let list_names: Vec<String> = self.changelists.lists.iter().map(|l| l.name.clone()).collect();
        let active_list = self.changelists.active.clone();
        let partial_paths: HashSet<String> =
            if staging { HashSet::new() } else { ExcludedHunks::get(cx).keys().cloned().collect() };

        let tree_palette = palette.clone();
        v_flex()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_message_history))
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
                    .child(
                        tool_button("commit-shelve", IconName::Layers, "Shelve Changes…")
                            .on_click(cx.listener(|this, _, window, cx| this.shelve(window, cx))),
                    )
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
                        let is_group = group_ids.iter().any(|g| g.as_str() == id.as_ref());
                        let group_list = changelist_of.get(id.as_ref()).cloned();
                        let is_active = group_list.as_deref() == Some(active_list.as_str()) && list_names.len() > 1;
                        let move_targets: Vec<String> = list_names.clone();
                        let in_staged = id.starts_with(STAGED_SCOPE) || id.as_ref() == "grp:staged";
                        let in_conflicts = id.starts_with(CONFLICTS_SCOPE) || id.as_ref() == "grp:conflicts";
                        let toggle_entity = entity.clone();
                        let toggle_id = id.clone();
                        let stage_entity = entity.clone();
                        let stage_id = id.clone();
                        let n = counts.get(&id).copied().unwrap_or(0);
                        let (menu_entity, menu_id, menu_file) = (entity.clone(), id.clone(), file.clone());
                        let menu_kind = kinds.get(id.as_ref()).copied();
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
                                .child(
                                    div()
                                        .text_color(color)
                                        .when(is_active, |el| el.font_weight(gpui_kit::FontWeight::BOLD))
                                        .child(item.label.clone()),
                                )
                                .when(file.as_ref().is_some_and(|f| partial_paths.contains(f)), |el| {
                                    el.child(div().text_xs().text_color(palette.text_secondary).child("partially included"))
                                })
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
                                })
                                .context_menu(move |menu, _, cx| {
                                    if let Some(list) = group_list.clone() {
                                        return changelist_menu(menu, &menu_entity, list);
                                    }
                                    let Some(path) = menu_file.clone() else { return menu };
                                    let in_changelist = menu_id.starts_with('c');
                                    let menu = if in_changelist && !staging {
                                        let mut menu = menu;
                                        let current = menu_entity.read(cx).changelists.list_of(&path).to_owned();
                                        for target in move_targets.iter().filter(|t| **t != current) {
                                            let (entity, path, target) = (menu_entity.clone(), path.clone(), target.clone());
                                            menu = menu.item(PopupMenuItem::new(format!("Move to \u{201c}{target}\u{201d}")).on_click(move |_, _, cx| {
                                                let (path, target) = (path.clone(), target.clone());
                                                entity.update(cx, |this, cx| this.move_to_changelist(&path, &target, cx))
                                            }));
                                        }
                                        let (entity, path) = (menu_entity.clone(), path.clone());
                                        menu.item(PopupMenuItem::new("Move to New Changelist…").on_click(move |_, window, cx| {
                                            let path = path.clone();
                                            entity.update(cx, |this, cx| this.new_changelist(Some(path), window, cx))
                                        }))
                                        .separator()
                                    } else {
                                        menu
                                    };
                                    let tracked = menu_kind != Some(StatusKind::Unversioned) && menu_kind != Some(StatusKind::Added);
                                    let (e_diff, e_blame, e_history) = (menu_entity.clone(), menu_entity.clone(), menu_entity.clone());
                                    let (i_diff, p_blame, p_history, p_copy) = (menu_id.clone(), path.clone(), path.clone(), path.clone());
                                    let (e_compare, p_compare) = (menu_entity.clone(), path.clone());
                                    menu.item(PopupMenuItem::new("Show Diff").on_click(move |_, _, cx| {
                                        e_diff.update(cx, |this, cx| {
                                            if let Some(source) = this.diff_source(&i_diff) {
                                                cx.emit(CommitEvent::OpenDiff(source));
                                            }
                                        })
                                    }))
                                    .separator()
                                    .item(PopupMenuItem::new("Annotate with Git Blame").disabled(!tracked).on_click(move |_, _, cx| {
                                        e_blame.update(cx, |_, cx| cx.emit(CommitEvent::Annotate(p_blame.clone())))
                                    }))
                                    .item(PopupMenuItem::new("Show History").disabled(!tracked).on_click(move |_, _, cx| {
                                        e_history.update(cx, |_, cx| cx.emit(CommitEvent::ShowHistory(p_history.clone())))
                                    }))
                                    .item(PopupMenuItem::new("Compare with Branch or Revision…").disabled(!tracked).on_click(move |_, _, cx| {
                                        e_compare.update(cx, |_, cx| cx.emit(CommitEvent::CompareWith(p_compare.clone())))
                                    }))
                                    .separator()
                                    .item(PopupMenuItem::new("Shelve Changes…").on_click({
                                        let (entity, path) = (menu_entity.clone(), path.clone());
                                        move |_, window, cx| {
                                            let (model, paths) = entity.read_with(cx, |this, _| (this.model.clone(), this.action_paths(Some(&path))));
                                            let name = crate::ui::patch_dialogs::default_shelf_name(&paths);
                                            crate::ui::patch_dialogs::shelve(model, paths, name, window, cx)
                                        }
                                    }))
                                    .item(PopupMenuItem::new("Shelve Silently").on_click({
                                        let (entity, path) = (menu_entity.clone(), path.clone());
                                        move |_, _, cx| {
                                            let (model, paths) = entity.read_with(cx, |this, _| (this.model.clone(), this.action_paths(Some(&path))));
                                            crate::ui::patch_dialogs::shelve_silently(model, paths, cx)
                                        }
                                    }))
                                    .item(PopupMenuItem::new("Create Patch…").on_click({
                                        let (entity, path) = (menu_entity.clone(), path.clone());
                                        move |_, window, cx| {
                                            let (model, paths) = entity.read_with(cx, |this, _| (this.model.clone(), this.action_paths(Some(&path))));
                                            let source = crate::ui::patch_dialogs::PatchSource::Local { paths };
                                            crate::ui::patch_dialogs::create_patch(model, source, window, cx)
                                        }
                                    }))
                                    .item(PopupMenuItem::new("Copy as Patch to Clipboard").on_click({
                                        let (entity, path) = (menu_entity.clone(), path.clone());
                                        move |_, _, cx| {
                                            let (model, paths) = entity.read_with(cx, |this, _| (this.model.clone(), this.action_paths(Some(&path))));
                                            let source = crate::ui::patch_dialogs::PatchSource::Local { paths };
                                            crate::ui::patch_dialogs::copy_patch(model, source, cx)
                                        }
                                    }))
                                    .separator()
                                    .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(p_copy.clone()))
                                    }))
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
                        h_flex()
                            .gap_3()
                            .child(
                                Checkbox::new("amend")
                                    .label("Amend")
                                    .checked(self.amend)
                                    .on_change(cx.listener(|this, value, window, cx| this.set_amend(*value, window, cx))),
                            )
                            .child(div().flex_1())
                            .when_some(subject_hint, |el, (len, limit)| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(if len > limit { palette.status_conflict } else { palette.text_secondary })
                                        .child(format!("{len}/{limit}")),
                                )
                            })
                            .child(
                                Button::new("commit-history")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Clock))
                                    .tooltip("Commit Message History  Ctrl+M")
                                    .dropdown_menu({
                                        let entity = history_entity.clone();
                                        move |mut menu, _, _| {
                                            let history = crate::settings::message_history();
                                            if history.is_empty() {
                                                return menu.item(PopupMenuItem::new("No recent commit messages").disabled(true));
                                            }
                                            for message in history {
                                                let subject: String = message.lines().next().unwrap_or_default().chars().take(70).collect();
                                                let entity = entity.clone();
                                                menu = menu.item(PopupMenuItem::new(subject).on_click(move |_, window, cx| {
                                                    let message = message.clone();
                                                    entity.update(cx, |this, cx| {
                                                        this.message.update(cx, |state, cx| state.set_value(message, window, cx))
                                                    })
                                                }));
                                            }
                                            menu.max_h(px(360.))
                                        }
                                    }),
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
                            )
                            .child(div().flex_1())
                            .child(self.render_options(cx)),
                    ),
            )
    }
}

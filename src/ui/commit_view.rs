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
use crate::ui::common::{self, FILE_PREFIX, row_height, tool_button};
use crate::ui::diff_view::DiffSource;

mod menu;

pub enum CommitEvent {
    OpenDiff(DiffSource),
    /// "Commit and Push…" committed; show the Push dialog next.
    OpenPush,
    /// A conflicted file was picked: open the merge tool.
    OpenMerge(Conflict),
    /// Edit Source (F4).
    EditSource(String),
}

impl EventEmitter<CommitEvent> for CommitView {}

gpui_kit::actions!(commit_view, [ShowMessageHistory, CommitChanges, CommitAndPush, ShowDiff, RollbackFiles, AddToVcs, DeleteFiles, EditSource, MoveToChangelist]);

const CONTEXT: &str = "CommitView";
/// The changes tree, where IntelliJ's file shortcuts apply (not in the message editor).
const TREE_CONTEXT: &str = "CommitTree";

pub fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    // Commit Message History: Ctrl+M on every platform, as in IntelliJ.
    cx.bind_keys([
        KeyBinding::new("ctrl-m", ShowMessageHistory, Some(CONTEXT)),
        // Commit (Ctrl+Enter) and Commit and Push… (Ctrl+Alt+K) from anywhere
        // in the Commit tool window, the message editor included.
        KeyBinding::new("secondary-enter", CommitChanges, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-k", CommitAndPush, Some(CONTEXT)),
        // The message editor's own Ctrl+Enter would otherwise win.
        KeyBinding::new("secondary-enter", CommitChanges, Some("CommitView > Input")),
        KeyBinding::new("secondary-alt-k", CommitAndPush, Some("CommitView > Input")),
        KeyBinding::new("secondary-d", ShowDiff, Some(TREE_CONTEXT)),
        KeyBinding::new("secondary-alt-z", RollbackFiles, Some(TREE_CONTEXT)),
        KeyBinding::new("secondary-alt-a", AddToVcs, Some(TREE_CONTEXT)),
        KeyBinding::new("delete", DeleteFiles, Some(TREE_CONTEXT)),
        KeyBinding::new("f4", EditSource, Some(TREE_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-backspace", DeleteFiles, Some(TREE_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-down", EditSource, Some(TREE_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-m", MoveToChangelist, Some(TREE_CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-shift-m", MoveToChangelist, Some(TREE_CONTEXT)),
    ]);
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
const IGNORED_SCOPE: &str = "i:";

pub struct CommitView {
    model: Entity<RepoModel>,
    tree: Entity<TreeState>,
    message: Entity<TextareaState>,
    spelling: Entity<crate::ui::spell_overlay::SpellOverlay>,
    groups: Vec<Group>,
    staging: bool,
    /// Expand All / Collapse All: how the next rebuild lays out the tree.
    expand_all: bool,
    included: HashSet<String>,
    /// Every path we have seen, so new changes start included and
    /// unchecked ones stay unchecked across refreshes.
    known: HashSet<String>,
    kinds: HashMap<String, StatusKind>,
    /// The workspace's handler for menu actions (Git submenu, Delete, …).
    file_actions: Option<crate::ui::file_menus::FileActions>,
    counts: HashMap<SharedString, usize>,
    amend: bool,
    push_after_commit: bool,
    last_selection: Option<SharedString>,
    /// Commit Options popover: "Author" override and "GPG-sign" (defaults to `commit.gpgSign`).
    author: Entity<InputState>,
    gpg_sign: Option<bool>,
    changelists: Changelists,
    changelists_root: Option<std::path::PathBuf>,
    /// Group By › Module: each changed file's module folder.
    modules: HashMap<String, String>,
    _subscriptions: Vec<Subscription>,
}

impl CommitView {
    pub fn new(model: Entity<RepoModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let message = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder("Commit Message"));
        let author = cx.new(|cx| InputState::new(window, cx).placeholder("Name <email>"));
        let spelling = cx.new(|cx| crate::ui::spell_overlay::SpellOverlay::new(message.clone(), cx));
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
            spelling,
            groups: Vec::new(),
            staging: false,
            expand_all: true,
            included: HashSet::new(),
            known: HashSet::new(),
            kinds: HashMap::new(),
            file_actions: None,
            counts: HashMap::new(),
            amend: false,
            push_after_commit: false,
            last_selection: None,
            author,
            gpg_sign: None,
            changelists: Changelists::default(),
            changelists_root: None,
            modules: HashMap::new(),
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
        if Settings::get(cx).commit_show_ignored {
            let ignored = self.model.read(cx).repository().map(crate::git::status::ignored).unwrap_or_default();
            groups.push(Group::new("grp:ignored", IGNORED_SCOPE, "Ignored Files", ignored.into_iter().map(|p| (p, StatusKind::Unversioned)).collect()));
        }
        self.groups = groups;
        let settings = Settings::get(cx);
        let (by_directory, by_module, by_repository) =
            (settings.commit_group_by_directory, settings.commit_group_by_module, settings.commit_group_by_repository);
        let repository = self.model.read(cx).repository().map(|r| (r.root().to_path_buf(), r.name()));
        self.modules.clear();
        if let (true, Some((root, _))) = (by_module, &repository) {
            let mut cache = HashMap::new();
            for (path, _) in self.groups.iter().flat_map(|g| g.files.iter()) {
                let module = common::module_of(root, path, &mut cache);
                self.modules.insert(path.clone(), module);
            }
        }
        let repo_name = repository.map(|(_, name)| name).unwrap_or_default();
        let modules = self.modules.clone();
        let module_of = move |path: &str| modules.get(path).cloned().unwrap_or_default();
        let expand = self.expand_all;
        let items: Vec<TreeItem> = self
            .groups
            .iter()
            // The first group always shows, as IntelliJ's default changelist does.
            .enumerate()
            .filter(|(_, group)| !group.files.is_empty() || group.changelist.is_some() || group.id == "grp:staged")
            .map(|(_, group)| {
                let paths = group.files.iter().map(|(p, _)| p.clone());
                TreeItem::new(group.id.clone(), group.label.clone())
                    // Ignored Files starts collapsed, as it can be long.
                    .expanded(expand && group.id != "grp:ignored")
                    .children(common::grouped_file_tree(
                        paths.collect(),
                        &group.scope,
                        expand,
                        by_directory,
                        by_module.then_some((repo_name.as_str(), &module_of as &dyn Fn(&str) -> String)),
                        by_repository.then_some(repo_name.as_str()),
                    ))
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
        let rest = id.strip_prefix(group.scope.as_str());
        if let Some(module) = rest.and_then(|r| r.strip_prefix(common::MODULE_PREFIX)) {
            return group.files.iter().map(|(p, _)| p).filter(|p| self.modules.get(*p).map(String::as_str) == Some(module)).cloned().collect();
        }
        let dir = rest.and_then(|r| r.strip_prefix(common::DIR_PREFIX));
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

    /// A message, and something to commit (or Amend).
    fn can_commit(&self, cx: &gpui_kit::App) -> bool {
        let staged_count = self.groups.iter().find(|g| g.scope == STAGED_SCOPE).map_or(0, |g| g.files.len());
        let has_changes = if self.staging { staged_count > 0 } else { !self.included.is_empty() };
        !self.message.read(cx).value().trim().is_empty() && (has_changes || self.amend)
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
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Checkbox::new("opt-gpg").label("Sign commit with GPG").checked(gpg).on_change({
                                let gpg_entity = gpg_entity.clone();
                                move |v, _, cx| {
                                    gpg_entity.update(cx, |this, cx| {
                                        this.gpg_sign = Some(*v);
                                        cx.notify();
                                    })
                                }
                            }))
                            .child(Button::new("opt-gpg-configure").link().small().label("Configure…").on_click(move |_, window, cx| {
                                let model = gpg_entity.read(cx).model.clone();
                                crate::ui::dialogs::configure_gpg(model, window, cx)
                            })),
                    )
                    .child(div().pt_1().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Before Commit"))
                    .child(toggle("opt-hooks", "Run Git hooks", settings.run_hooks, |s, v| s.run_hooks = v))
                    .child(toggle("opt-cleanup", "Clean up commit message", settings.cleanup_message, |s, v| s.cleanup_message = v))
                    .child(div().text_xs().text_color(palette.text_secondary).child("Removes # comment lines and surrounding whitespace"))
            })
    }
}

impl CommitView {
    /// The selected node's file, for the tree's keyboard shortcuts.
    fn selected_file(&self) -> Option<String> {
        self.last_selection.as_deref().and_then(Self::path_of).map(|(_, p)| p.to_owned())
    }

    fn selected_paths(&self) -> Vec<String> {
        match self.selected_file() {
            Some(file) => self.action_paths(Some(&file)),
            None => self.last_selection.as_deref().map(|id| self.paths_under(id)).unwrap_or_default(),
        }
    }

    fn on_show_diff(&mut self, _: &ShowDiff, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.last_selection.clone().and_then(|id| self.diff_source(&id)) {
            cx.emit(CommitEvent::OpenDiff(source));
        }
    }

    fn on_rollback(&mut self, _: &RollbackFiles, window: &mut Window, cx: &mut Context<Self>) {
        let file = self.selected_file().filter(|_| !self.staging);
        self.rollback(file.as_deref(), window, cx);
    }

    fn on_add_to_vcs(&mut self, _: &AddToVcs, _: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<String> = self.selected_paths().into_iter().filter(|p| self.kinds.get(p) == Some(&StatusKind::Unversioned)).collect();
        if !paths.is_empty() {
            crate::ui::file_menus::add_to_vcs(&self.model, paths, cx);
        }
    }

    fn on_delete(&mut self, _: &DeleteFiles, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.model.read(cx).repository().map(|r| r.root().to_path_buf());
        if let Some(root) = root {
            crate::ui::file_menus::delete_files(root, self.selected_paths(), self.file_actions(), window, cx);
        }
    }

    fn on_edit_source(&mut self, _: &EditSource, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_file() {
            cx.emit(CommitEvent::EditSource(path));
        }
    }

    fn on_move_to_changelist(&mut self, _: &MoveToChangelist, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<String> = self.selected_paths().into_iter().filter(|p| self.kinds.get(p) != Some(&StatusKind::Unversioned)).collect();
        if !self.staging && !paths.is_empty() {
            self.move_dialog(paths, window, cx);
        }
    }

    /// Rollback…: checked files (or the clicked one), or in staging mode the
    /// selected node (unstaged changes come back from the index, staged ones
    /// from HEAD), confirmed in the Rollback Changes dialog.
    pub fn rollback(&mut self, file: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::rollback_dialog::{self, RollbackFrom};
        let (paths, from) = if self.staging && file.is_none() {
            let Some(id) = self.last_selection.clone() else { return };
            let scope = self.group_of(&id).map(|g| g.scope.clone());
            if scope.as_deref() == Some(UNVERSIONED_SCOPE) {
                return;
            }
            let from = if scope.as_deref() == Some(UNSTAGED_SCOPE) { RollbackFrom::Index } else { RollbackFrom::Head };
            (self.paths_under(&id), from)
        } else {
            (self.action_paths(file), RollbackFrom::Head)
        };
        let files = paths.into_iter().map(|p| {
            let kind = self.kinds.get(&p).copied().unwrap_or(StatusKind::Modified);
            (p, kind)
        });
        rollback_dialog::rollback(self.model.clone(), files.collect(), from, window, cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let submodules = self.model.read(cx).submodule_paths().clone();
        let palette = cx.palette().clone();
        let staging = self.staging;
        let by_directory = Settings::get(cx).commit_group_by_directory;
        let included = self.included.clone();
        let counts = self.counts.clone();
        let entity = cx.entity();
        let can_commit = self.can_commit(cx);
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
            .on_action(cx.listener(|this, _: &CommitChanges, window, cx| {
                if this.can_commit(cx) && this.model.read(cx).busy().is_none() {
                    this.commit(false, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &CommitAndPush, window, cx| {
                if this.can_commit(cx) && this.model.read(cx).busy().is_none() {
                    this.commit(true, window, cx)
                }
            }))
            .size_full()
            .child(
                h_flex()
                    .h(px(crate::ui::common::toolbar_height()))
                    .px_1()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(tool_button("commit-refresh", IconName::RefreshCw, "Refresh").on_click(cx.listener(
                        |this, _, _, cx| this.model.update(cx, |m, cx| m.reload(cx)),
                    )))
                    .child(
                        tool_button("commit-rollback", IconName::Undo2, "Rollback…")
                            .on_click(cx.listener(|this, _, window, cx| this.rollback(None, window, cx))),
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
                    .child(
                        tool_button("commit-stash", IconName::Archive, "Stash Changes…")
                            .on_click(cx.listener(|this, _, window, cx| crate::ui::dialogs::stash(this.model.clone(), window, cx))),
                    )
                    .child(
                        tool_button("commit-update", IconName::ArrowDownToLine, "Update Project…  Ctrl+T")
                            .on_click(cx.listener(|this, _, window, cx| crate::ui::dialogs::update_project(this.model.clone(), window, cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                    .child(
                        Button::new("commit-view-options")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Eye))
                            .tooltip("View Options")
                            .dropdown_menu({
                                let entity = cx.entity();
                                let settings = Settings::get(cx);
                                let (by_dir, by_module, by_repo, show_ignored) = (
                                    by_directory,
                                    settings.commit_group_by_module,
                                    settings.commit_group_by_repository,
                                    settings.commit_show_ignored,
                                );
                                move |menu, _, _| {
                                    let (e1, e2, e3, e4) = (entity.clone(), entity.clone(), entity.clone(), entity.clone());
                                    menu.label("Group By")
                                        .item(PopupMenuItem::new("Repository").checked(by_repo).on_click(move |_, _, cx| {
                                            Settings::update(cx, |s| s.commit_group_by_repository = !by_repo);
                                            e3.update(cx, |this, cx| this.rebuild(cx));
                                        }))
                                        .item(PopupMenuItem::new("Module").checked(by_module).on_click(move |_, _, cx| {
                                            Settings::update(cx, |s| s.commit_group_by_module = !by_module);
                                            e4.update(cx, |this, cx| this.rebuild(cx));
                                        }))
                                        .item(PopupMenuItem::new("Directory").checked(by_dir).on_click(move |_, _, cx| {
                                            Settings::update(cx, |s| s.commit_group_by_directory = !by_dir);
                                            e1.update(cx, |this, cx| this.rebuild(cx));
                                        }))
                                        .separator()
                                        .item(PopupMenuItem::new("Show Ignored Files").checked(show_ignored).on_click(move |_, _, cx| {
                                            Settings::update(cx, |s| s.commit_show_ignored = !show_ignored);
                                            e2.update(cx, |this, cx| this.rebuild(cx));
                                        }))
                                }
                            }),
                    )
                    .child(tool_button("commit-expand", IconName::ChevronsUpDown, "Expand All").on_click(cx.listener(|this, _, _, cx| {
                        this.expand_all = true;
                        this.rebuild(cx);
                    })))
                    .child(tool_button("commit-collapse", IconName::ChevronsDownUp, "Collapse All").on_click(cx.listener(|this, _, _, cx| {
                        this.expand_all = false;
                        this.rebuild(cx);
                    })))
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
                div()
                    .flex_1()
                    .min_h_0()
                    .key_context(TREE_CONTEXT)
                    .on_action(cx.listener(Self::on_show_diff))
                    .on_action(cx.listener(Self::on_rollback))
                    .on_action(cx.listener(Self::on_add_to_vcs))
                    .on_action(cx.listener(Self::on_delete))
                    .on_action(cx.listener(Self::on_edit_source))
                    .on_action(cx.listener(Self::on_move_to_changelist))
                    .child(
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
                        let in_staged = id.starts_with(STAGED_SCOPE) || id.as_ref() == "grp:staged";
                        let in_conflicts = id.starts_with(CONFLICTS_SCOPE) || id.as_ref() == "grp:conflicts";
                        let toggle_entity = entity.clone();
                        let toggle_id = id.clone();
                        let stage_entity = entity.clone();
                        let stage_id = id.clone();
                        let n = counts.get(&id).copied().unwrap_or(0);
                        let (menu_entity, menu_id, menu_file) = (entity.clone(), id.clone(), file.clone());
                        ListItem::new(ix).py_0().px_1().h(px(row_height())).child(
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
                                .when(
                                    !staging
                                        && !id.starts_with(CONFLICTS_SCOPE)
                                        && id.as_ref() != "grp:conflicts"
                                        && !id.starts_with(IGNORED_SCOPE)
                                        && id.as_ref() != "grp:ignored",
                                    |el| {
                                    el.child(
                                        Checkbox::new(SharedString::from(format!("check-{id}")))
                                            .checked(checked)
                                            .on_change(move |value, _, cx| {
                                                let id = toggle_id.clone();
                                                toggle_entity.update(cx, |this, cx| this.toggle(&id, *value, cx));
                                            }),
                                    )
                                    },
                                )
                                .when(!is_group, |el| {
                                    el.child(
                                        Icon::new(match &file {
                                            Some(p) if submodules.contains(p.as_str()) => IconName::FolderGit2,
                                            Some(p) => common::file_icon(p),
                                            None if id.split_once(':').is_some_and(|(_, r)| r.starts_with(common::REPO_PREFIX)) => IconName::FolderGit2,
                                            None if id.split_once(':').is_some_and(|(_, r)| r.starts_with(common::MODULE_PREFIX)) => IconName::Layers,
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
                                .when_some(file.as_ref().filter(|_| !by_directory).and_then(|f| f.rsplit_once('/')).map(|(dir, _)| dir.to_owned()), |el, dir| {
                                    el.child(div().text_xs().text_color(palette.text_secondary).child(dir))
                                })
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
                                .context_menu(move |menu, window, cx| {
                                    menu::commit_menu(menu, &menu_entity, &menu_id, menu_file.clone(), group_list.clone(), window, cx)
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
                    .child({
                        // The commit message uses the editor font with a right margin
                        // line at the subject limit, as IntelliJ's commit editor does.
                        let mono = gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone();
                        let font_size = gpui_kit::rems(0.875).to_pixels(window.rem_size());
                        let font = gpui_kit::font(mono.clone());
                        let advance = window
                            .text_system()
                            .advance(window.text_system().resolve_font(&font), font_size, 'm')
                            .map(|size| size.width)
                            .unwrap_or(px(7.));
                        let margin = px(12.) + advance * Settings::get(cx).commit_subject_limit as f32;
                        div()
                            .relative()
                            .overflow_hidden()
                            .font_family(mono)
                            .key_context(crate::ui::spell_overlay::EDITOR_CONTEXT)
                            .on_action(cx.listener(|this, _: &crate::ui::spell_overlay::ShowSpellingFixes, window, cx| {
                                this.spelling.update(cx, |spelling, cx| spelling.show_at_cursor(window, cx))
                            }))
                            .child(Textarea::new(&self.message).h(px(110.)))
                            .child(div().absolute().top(px(4.)).bottom(px(4.)).left(margin).w(px(1.)).bg(palette.border))
                            .child(self.spelling.clone())
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("do-commit")
                                    .primary()
                                    .small()
                                    .label(if self.amend { "Amend Commit" } else { "Commit" })
                                    .disabled(!can_commit || busy)
                                    .tooltip(if cfg!(target_os = "macos") { "Commit (⌘⏎)" } else { "Commit (Ctrl+Enter)" })
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(false, window, cx))),
                            )
                            .child(
                                Button::new("do-commit-push")
                                    .outline()
                                    .small()
                                    .label(if self.amend { "Amend Commit and Push…" } else { "Commit and Push…" })
                                    .disabled(!can_commit || busy)
                                    .tooltip(if cfg!(target_os = "macos") { "Commit and Push (⌥⌘K)" } else { "Commit and Push (Ctrl+Alt+K)" })
                                    .on_click(cx.listener(|this, _, window, cx| this.commit(true, window, cx))),
                            )
                            .child(div().flex_1())
                            .child(self.render_options(cx)),
                    ),
            )
    }
}

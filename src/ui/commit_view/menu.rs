//! The Commit tool window's right-click menu on files, folders and groups,
//! item for item as IntelliJ's.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, WindowExt as _, h_flex,
    input::{Input, InputState},
    menu::PopupMenu,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use super::{CommitEvent, CommitView, IGNORED_SCOPE, changelist_menu};
use crate::git::StatusKind;
use crate::theme::ActivePalette as _;
use crate::ui::common::FILE_PREFIX;
use crate::ui::file_menus::{self, FileActions, GitTarget, entry};

impl CommitView {
    pub fn set_file_actions(&mut self, actions: FileActions) {
        self.file_actions = Some(actions);
    }

    pub(super) fn file_actions(&self) -> FileActions {
        self.file_actions.clone().unwrap_or_else(|| Rc::new(|_, _, _| {}))
    }

    /// Commit File(s)…: only these files stay included, and the message gets focus.
    pub fn commit_only(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let known: Vec<String> = paths.into_iter().filter(|p| self.kinds.contains_key(p)).collect();
        if known.is_empty() {
            return;
        }
        self.included = known.into_iter().collect();
        self.focus_message(window, cx);
        cx.notify();
    }

    /// The scope of a node id ("unv:", "c0:", …).
    fn scope_of(&self, id: &str) -> Option<String> {
        Self::path_of(id).map(|(scope, _)| scope.to_owned()).or_else(|| self.group_of(id).map(|g| g.scope.clone()))
    }

    /// Move to Another Changelist…: pick an existing changelist or name a new one.
    pub(super) fn move_dialog(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let current = paths.first().map(|p| self.changelists.list_of(p).to_owned()).unwrap_or_default();
        let targets: Vec<String> = self.changelists.lists.iter().map(|l| l.name.clone()).filter(|n| *n != current).collect();
        let selected = Rc::new(RefCell::new(targets.first().cloned()));
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("New changelist name"));
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let mut list = v_flex().gap_0p5();
            for (ix, name) in targets.iter().enumerate() {
                let chosen = selected.borrow().as_deref() == Some(name.as_str());
                let (select, name_owned) = (selected.clone(), name.clone());
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("move-target-{ix}")))
                        .px_2()
                        .h(px(26.))
                        .rounded_sm()
                        .text_sm()
                        .cursor_pointer()
                        .when(chosen, |el| el.bg(palette.selection))
                        .child(name.clone())
                        .on_click(move |_, window, _| {
                            *select.borrow_mut() = Some(name_owned.clone());
                            window.refresh();
                        }),
                );
            }
            let (selected, new_name_ok, entity, paths) = (selected.clone(), new_name.clone(), entity.clone(), paths.clone());
            dialog
                .title("Move to Another Changelist")
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .when(!targets.is_empty(), |el| el.child(div().text_sm().text_color(palette.text_secondary).child("Existing changelist:")).child(list))
                        .child(div().text_sm().text_color(palette.text_secondary).child("Or a new changelist:"))
                        .child(Input::new(&new_name)),
                )
                .footer(crate::ui::dialogs::footer("OK"))
                .on_ok(move |_, _, cx| {
                    let new = new_name_ok.read(cx).value().trim().to_owned();
                    let target = if new.is_empty() { selected.borrow().clone() } else { Some(new.clone()) };
                    let Some(target) = target else { return false };
                    let paths = paths.clone();
                    entity.update(cx, |this, cx| {
                        if !new.is_empty() {
                            this.changelists.add(&new, "", false);
                        }
                        this.update_changelists(cx, |lists| lists.move_files(&paths, &target));
                    });
                    true
                })
        });
    }
}

/// The menu of a file, folder, group or changelist node.
pub(super) fn commit_menu(
    menu: PopupMenu,
    entity: &Entity<CommitView>,
    id: &str,
    file: Option<String>,
    changelist: Option<String>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    if id.starts_with(IGNORED_SCOPE) {
        return menu;
    }
    let this = entity.read(cx);
    let paths = match &file {
        Some(f) => this.action_paths(Some(f)),
        None => this.paths_under(id),
    };
    if paths.is_empty() && changelist.is_none() {
        return menu;
    }
    let menu = match changelist {
        Some(list) => changelist_menu(menu, entity, list).separator(),
        None => menu,
    };
    let is_unversioned = |p: &String| this.kinds.get(p) == Some(&StatusKind::Unversioned);
    let unversioned: Vec<String> = paths.iter().filter(|p| is_unversioned(p)).cloned().collect();
    let tracked: Vec<String> = paths.iter().filter(|p| !is_unversioned(p)).cloned().collect();
    let staging = this.staging;
    let model = this.model.clone();
    let actions = this.file_actions();
    let root = model.read(cx).repository().map(|r| r.root().to_path_buf());
    let scope = this.scope_of(id).unwrap_or_default();
    let first = file.clone().or_else(|| paths.first().cloned());
    let diff_source = first.as_ref().and_then(|f| this.diff_source(&format!("{scope}{FILE_PREFIX}{f}")));
    let rollback_file = file.clone().filter(|_| !staging);

    let commit_label = if file.is_some() && paths.len() == 1 { "Commit File…" } else { "Commit Files…" };
    let (e_commit, p_commit) = (entity.clone(), paths.clone());
    let (e_rollback, p_rollback) = (entity.clone(), tracked.clone());
    let (e_move, p_move) = (entity.clone(), tracked.clone());
    let (e_diff, e_tab, d_diff, d_tab) = (entity.clone(), entity.clone(), diff_source.clone(), diff_source);
    let (e_source, p_source) = (entity.clone(), first.clone());
    let (a_delete, p_delete, r_delete) = (actions.clone(), paths.clone(), root);
    let (m_add, p_add) = (model.clone(), unversioned.clone());
    let menu = menu
        .item(entry(commit_label, "").on_click(move |_, window, cx| {
            let paths = p_commit.clone();
            e_commit.update(cx, |this, cx| this.commit_only(paths, window, cx))
        }))
        .item(entry("Rollback…", "Ctrl+Alt+Z").icon(Icon::new(IconName::Undo2)).disabled(tracked.is_empty()).on_click(move |_, window, cx| {
            let (file, paths) = (rollback_file.clone(), p_rollback.clone());
            e_rollback.update(cx, |this, cx| match &file {
                Some(file) => this.rollback(Some(file), window, cx),
                None => {
                    let files = paths.iter().map(|p| (p.clone(), this.kinds.get(p).copied().unwrap_or(StatusKind::Modified))).collect();
                    crate::ui::rollback_dialog::rollback(this.model.clone(), files, crate::ui::rollback_dialog::RollbackFrom::Head, window, cx)
                }
            })
        }))
        .item(entry("Move to Another Changelist…", "Alt+Shift+M").disabled(staging || tracked.is_empty()).on_click(move |_, window, cx| {
            let paths = p_move.clone();
            e_move.update(cx, |this, cx| this.move_dialog(paths, window, cx))
        }))
        .item(entry("Show Diff", "Ctrl+D").icon(Icon::new(IconName::GitCompare)).disabled(d_diff.is_none()).on_click(move |_, _, cx| {
            if let Some(source) = d_diff.clone() {
                e_diff.update(cx, |_, cx| cx.emit(CommitEvent::OpenDiff(source)))
            }
        }))
        .item(entry("Show Diff in a New Tab", "").icon(Icon::new(IconName::GitCompare)).disabled(d_tab.is_none()).on_click(move |_, _, cx| {
            if let Some(source) = d_tab.clone() {
                e_tab.update(cx, |_, cx| cx.emit(CommitEvent::OpenDiff(source)))
            }
        }))
        .item(entry("Jump to Source", "F4").icon(Icon::new(IconName::Pencil)).disabled(first.is_none()).on_click(move |_, _, cx| {
            if let Some(path) = p_source.clone() {
                e_source.update(cx, |_, cx| cx.emit(CommitEvent::EditSource(path)))
            }
        }))
        .separator()
        .item(entry("Delete…", "Delete").on_click(move |_, window, cx| {
            if let Some(root) = r_delete.clone() {
                file_menus::delete_files(root, p_delete.clone(), a_delete.clone(), window, cx)
            }
        }))
        .item(entry("Add to VCS", "Ctrl+Alt+A").disabled(unversioned.is_empty()).on_click(move |_, _, cx| file_menus::add_to_vcs(&m_add, p_add.clone(), cx)));
    let (m_ignore, p_ignore) = (model.clone(), unversioned.clone());
    let none_unversioned = unversioned.is_empty();
    let menu = menu.submenu_with_icon(Some(Icon::new(IconName::Ban)), "Add to .gitignore", window, cx, move |m, _, _| {
        let (m1, p1, m2, p2) = (m_ignore.clone(), p_ignore.clone(), m_ignore.clone(), p_ignore.clone());
        m.item(entry(".gitignore", "").disabled(none_unversioned).on_click(move |_, _, cx| file_menus::ignore(&m1, p1.clone(), false, cx)))
            .item(entry(".git/info/exclude", "").disabled(none_unversioned).on_click(move |_, _, cx| file_menus::ignore(&m2, p2.clone(), true, cx)))
    });
    let (m_patch, p_patch) = (model.clone(), tracked.clone());
    let (m_copy, p_copy) = (model.clone(), paths.clone());
    let (m_shelve, p_shelve) = (model.clone(), tracked.clone());
    let m_refresh = model.clone();
    let menu = menu
        .separator()
        .item(entry("Create Patch from Local Changes…", "").icon(Icon::new(IconName::Plus)).disabled(tracked.is_empty()).on_click(move |_, window, cx| {
            let source = crate::ui::patch_dialogs::PatchSource::Local { paths: p_patch.clone() };
            crate::ui::patch_dialogs::create_patch(m_patch.clone(), source, window, cx)
        }))
        .item(entry("Copy as Patch to Clipboard", "").on_click(move |_, _, cx| {
            let source = crate::ui::patch_dialogs::PatchSource::Local { paths: p_copy.clone() };
            crate::ui::patch_dialogs::copy_patch(m_copy.clone(), source, cx)
        }))
        .item(entry("Shelve Changes…", "").icon(Icon::new(IconName::ArrowDownToLine)).disabled(tracked.is_empty()).on_click(move |_, window, cx| {
            let paths = p_shelve.clone();
            let name = crate::ui::patch_dialogs::default_shelf_name(&paths);
            crate::ui::patch_dialogs::shelve(m_shelve.clone(), paths, name, window, cx)
        }))
        .separator()
        .item(entry("Refresh", "").icon(Icon::new(IconName::RefreshCw)).on_click(move |_, _, cx| m_refresh.update(cx, |m, cx| m.reload(cx))))
        .separator();
    let menu = if file.is_some() { file_menus::local_history_submenu(menu, window, cx) } else { menu };
    let target = GitTarget { model, paths, file, actions };
    file_menus::git_submenu(menu, target, window, cx)
}

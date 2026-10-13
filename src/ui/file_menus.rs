//! Context menus of files and folders in the Project, Commit and Changes
//! tool windows, laid out as IntelliJ's: each item with its shortcut on the
//! right, the same separators, and the shared Git submenu.

use std::cell::RefCell;
use crate::ui::as_icons as icons;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::{
    Icon, WindowExt as _, h_flex,
    input::{Input, InputState},
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::git::StatusKind;
use crate::model::RepoModel;
use crate::theme::ActivePalette as _;
use crate::ui::dialogs;

/// What a menu asks the workspace to do.
pub enum FileAction {
    /// The file's local changes.
    ShowDiff(String),
    OpenFile(String),
    /// Open in Right Split.
    OpenInSplit(String),
    Annotate(String),
    ShowHistory(String),
    /// Compare with Revision: the file's revisions to pick from.
    CompareWithRevision(String),
    /// Compare with Branch: a branch, tag or revision to type or pick.
    CompareWithBranch(String),
    /// Compare With…: another file picked from disk.
    CompareWithFile(String),
    /// Compare With… on two selected files: the first against the second.
    CompareFiles { path: String, other: PathBuf },
    /// Compare with Clipboard: the clipboard's text against the file.
    CompareWithClipboard(String),
    /// Commit File(s)…: the Commit tool window with only these included.
    CommitFiles(Vec<String>),
    Branches,
    Unstash,
    /// Files were created, renamed or deleted on disk.
    FilesChanged,
    /// A file or folder moved (Rename, Cut and Paste), project-relative paths.
    Moved { from: String, to: String },
    /// Find / Replace in Files scoped to a project-relative folder.
    FindInPath { dir: String, replace: bool },
}

pub type FileActions = Rc<dyn Fn(FileAction, &mut Window, &mut App)>;

/// A menu item with its shortcut on the right, as IntelliJ shows them.
pub fn entry(label: impl Into<SharedString>, shortcut: &'static str) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, cx| {
        let palette = cx.palette().clone();
        h_flex()
            .w_full()
            .min_w(px(220.))
            .gap_6()
            .child(div().flex_1().child(label.clone()))
            .when(!shortcut.is_empty(), |el| el.child(div().text_color(palette.text_secondary).child(shortcut_label(shortcut))))
    })
}

/// A Windows / Linux shortcut as written in the menus ("Ctrl+Alt+Z"), in the
/// platform's own form: macOS uses IntelliJ's mac keymap symbols (⌥⌘Z).
pub fn shortcut_label(shortcut: &str) -> String {
    if !cfg!(target_os = "macos") {
        return shortcut.to_owned();
    }
    // IntelliJ's macOS keymap moves a few shortcuts rather than swapping Ctrl for ⌘.
    let moved = match shortcut {
        "Delete" => Some("⌘⌫"),
        "F4" => Some("⌘↓"),
        "Alt+Shift+M" => Some("⇧⌘M"),
        "Ctrl+F4" => Some("⌘W"),
        "Ctrl+G" => Some("⌘L"),
        "Ctrl+Alt+Left" => Some("⌘["),
        "Ctrl+Alt+Right" => Some("⌘]"),
        "Ctrl+Alt+S" => Some("⌘,"),
        "Alt+`" => Some("⌃V"),
        _ => None,
    };
    if let Some(moved) = moved {
        return moved.to_owned();
    }
    let mut modifiers = String::new();
    let mut key = "";
    for part in shortcut.split('+') {
        match part {
            "Ctrl" => modifiers.push('⌘'),
            "Alt" => modifiers.push('⌥'),
            "Shift" => modifiers.push('⇧'),
            "" => key = "+",
            other => key = other,
        }
    }
    // macOS orders modifiers ⌃⌥⇧⌘.
    let mut ordered: Vec<char> = modifiers.chars().collect();
    ordered.sort_by_key(|c| "⌃⌥⇧⌘".chars().position(|m| m == *c));
    ordered.into_iter().collect::<String>() + key
}

/// A submenu whose items this client has no counterpart for (Analyze,
/// Refactor, Bookmarks, …): kept so the menu reads as IntelliJ's.
pub fn stub_submenu(
    menu: PopupMenu,
    label: &'static str,
    items: &'static [(&'static str, &'static str)],
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu.submenu(label, window, cx, move |mut m, _, _| {
        for (label, shortcut) in items {
            m = m.item(entry(*label, shortcut).disabled(true));
        }
        m
    })
}

pub fn local_history_submenu(menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    stub_submenu(
        menu,
        "Local History",
        &[("Show History…", ""), ("Show Project History…", ""), ("Recent Changes", "Alt+Shift+C"), ("Put Label…", "")],
        window,
        cx,
    )
}

/// The files a Git submenu acts on.
#[derive(Clone)]
pub struct GitTarget {
    pub model: Entity<RepoModel>,
    /// Repository-relative paths; a folder's files, or one file.
    pub paths: Vec<String>,
    /// The file itself, when one was clicked (not a folder).
    pub file: Option<String>,
    pub actions: FileActions,
}

impl GitTarget {
    fn kinds(&self, cx: &App) -> HashMap<String, StatusKind> {
        self.model.read(cx).project_status().entries.iter().map(|e| (e.path.clone(), e.kind)).collect()
    }
}

pub fn git_submenu(menu: PopupMenu, target: GitTarget, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    menu.submenu("Git", window, cx, move |m, _, cx| git_items(m, &target, cx))
}

fn git_items(menu: PopupMenu, target: &GitTarget, cx: &mut Context<PopupMenu>) -> PopupMenu {
    // IntelliJ's Git actions here work on the clicked file's repository.
    // A folder acts in the repository that holds the folder itself.
    if let Some(path) = target.file.clone().or_else(|| common_dir(&target.paths)) {
        target.model.update(cx, |m, cx| {
            m.focus_path(&path, cx);
        });
    }
    let kinds = target.kinds(cx);
    let changed: Vec<String> = target.paths.iter().filter(|p| kinds.get(*p).is_some_and(|k| *k != StatusKind::Unversioned)).cloned().collect();
    let unversioned: Vec<String> = target.paths.iter().filter(|p| kinds.get(*p) == Some(&StatusKind::Unversioned)).cloned().collect();
    let file = target.file.clone();
    let file_tracked = file.as_ref().is_some_and(|f| !matches!(kinds.get(f), Some(StatusKind::Unversioned | StatusKind::Added)));
    let file_changed = file.as_ref().is_some_and(|f| kinds.contains_key(f));
    let any_tracked = target.paths.iter().any(|p| kinds.get(p) != Some(&StatusKind::Unversioned));
    let model = target.model.clone();
    let actions = target.actions.clone();
    let act = |action: fn(String) -> FileAction| {
        let (actions, file) = (actions.clone(), file.clone());
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            if let Some(file) = file.clone() {
                actions(action(file), window, cx)
            }
        }
    };
    let dialog = |f: fn(Entity<RepoModel>, &mut Window, &mut App)| {
        let model = model.clone();
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| f(model.clone(), window, cx)
    };
    let commit_label = if target.file.is_some() { "Commit File…" } else { "Commit Files…" };
    menu.item(entry(commit_label, "").disabled(changed.is_empty() && unversioned.is_empty()).on_click({
        let (actions, paths) = (actions.clone(), target.paths.clone());
        move |_, window, cx| actions(FileAction::CommitFiles(paths.clone()), window, cx)
    }))
    // Add also marks conflicted files resolved, as in IntelliJ.
    .item(entry("Add", "Ctrl+Alt+A").disabled(unversioned.is_empty() && !target.paths.iter().any(|p| kinds.get(p) == Some(&StatusKind::Conflicted))).on_click({
        let conflicted = target.paths.iter().filter(|p| kinds.get(*p) == Some(&StatusKind::Conflicted)).cloned();
        let (model, paths) = (model.clone(), unversioned.iter().cloned().chain(conflicted).collect::<Vec<_>>());
        move |_, _, cx| add_to_vcs(&model, paths.clone(), cx)
    }))
    .item(entry(".git/info/exclude", "").disabled(unversioned.is_empty()).on_click({
        let (model, paths) = (model.clone(), unversioned.clone());
        move |_, _, cx| ignore(&model, paths.clone(), true, cx)
    }))
    .separator()
    .item(entry("Annotate with Git Blame", "").disabled(!file_tracked).on_click(act(FileAction::Annotate)))
    .item(entry("Show Diff", "").disabled(!file_changed).on_click(act(FileAction::ShowDiff)))
    .item(entry("Compare with Revision…", "").disabled(!file_tracked).on_click(act(FileAction::CompareWithRevision)))
    .item(entry("Compare with Branch…", "").disabled(!file_tracked).on_click(act(FileAction::CompareWithBranch)))
    .item(entry("Show History", "").disabled(!any_tracked).on_click({
        let (actions, path) = (actions.clone(), file.clone().or_else(|| common_dir(&target.paths)));
        move |_, window, cx| {
            if let Some(path) = path.clone() {
                actions(FileAction::ShowHistory(path), window, cx)
            }
        }
    }))
    .item(entry("Show History for Selection", "").disabled(true))
    .separator()
    .item(entry("Rollback…", "Ctrl+Alt+Z").icon(Icon::new(icons::VCS_REVERT)).disabled(changed.is_empty()).on_click({
        let model = model.clone();
        let files: Vec<(String, StatusKind)> = changed.iter().map(|p| (p.clone(), kinds[p])).collect();
        move |_, window, cx| {
            crate::ui::rollback_dialog::rollback(model.clone(), files.clone(), crate::ui::rollback_dialog::RollbackFrom::Head, window, cx)
        }
    }))
    .separator()
    .item(entry("Push…", "Ctrl+Shift+K").on_click(dialog(dialogs::push)))
    .item(entry("Pull…", "").on_click(dialog(crate::ui::remote_dialogs::pull)))
    .item(entry("Fetch", "").on_click({
        let model = model.clone();
        move |_, _, cx| {
            model.update(cx, |m, cx| m.fetch_all(cx))
        }
    }))
    .separator()
    .item(entry("Merge…", "").on_click(dialog(dialogs::merge)))
    .item(entry("Rebase…", "").on_click(dialog(dialogs::rebase)))
    .separator()
    .item(entry("Branches…", "Ctrl+Shift+`").on_click({
        let actions = actions.clone();
        move |_, window, cx| actions(FileAction::Branches, window, cx)
    }))
    .item(entry("New Branch…", "").on_click(dialog(|m, w, cx| dialogs::new_branch(m, "HEAD".into(), w, cx))))
    .item(entry("New Tag…", "").on_click(dialog(|m, w, cx| dialogs::new_tag(m, "HEAD".into(), w, cx))))
    .item(entry("Reset HEAD…", "").on_click(dialog(|m, w, cx| dialogs::reset_to(m, "HEAD".into(), w, cx))))
    .separator()
    .item(entry("Stash Changes…", "").on_click(dialog(dialogs::stash)))
    .item(entry("Unstash Changes…", "").on_click({
        let actions = actions.clone();
        move |_, window, cx| actions(FileAction::Unstash, window, cx)
    }))
    .separator()
    .item(entry("Manage Remotes…", "").on_click(dialog(crate::ui::remote_dialogs::manage_remotes)))
    .item(entry("Clone…", "").on_click(dialog(crate::ui::clone_dialog::clone)))
}

/// The deepest folder holding all `paths`.
fn common_dir(paths: &[String]) -> Option<String> {
    let first = paths.first()?;
    let mut dir: Vec<&str> = first.split('/').collect();
    dir.pop();
    for path in &paths[1..] {
        let parts: Vec<&str> = path.split('/').collect();
        let same = dir.iter().zip(&parts).take_while(|(a, b)| a == b).count();
        dir.truncate(same.min(parts.len().saturating_sub(1)));
    }
    Some(dir.join("/"))
}

pub fn add_to_vcs(model: &Entity<RepoModel>, paths: Vec<String>, cx: &mut App) {
    model.update(cx, |m, cx| {
        m.run_in_roots("Add to VCS", paths, move |repo, paths| {
            let mut args = vec!["add".to_owned(), "--".into()];
            args.extend(paths.iter().cloned());
            repo.run(&args)?;
            Ok(format!("Added {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
        }, cx)
    });
}

/// Add to .gitignore (the root one) or to .git/info/exclude.
pub fn ignore(model: &Entity<RepoModel>, paths: Vec<String>, exclude: bool, cx: &mut App) {
    // Each root's own .gitignore (or exclude file), with its own paths.
    model.update(cx, |m, cx| {
        m.run_in_roots("Ignore", paths, move |repo, paths| {
            let file = if exclude { repo.git_dir().join("info").join("exclude") } else { repo.root().join(".gitignore") };
            crate::git::status::append_ignore(&file, &paths)?;
            Ok(format!("Ignored {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
        }, cx)
    });
}

/// Delete…: asks first, then removes the files (and folders) from disk.
pub fn delete_files(root: PathBuf, paths: Vec<String>, actions: FileActions, window: &mut Window, cx: &mut App) {
    if paths.is_empty() {
        return;
    }
    let message = match paths.as_slice() {
        [one] => format!("Delete \u{201c}{one}\u{201d}?"),
        many => format!("Delete {} files?", many.len()),
    };
    let window_handle = window.window_handle();
    dialogs::confirm("Delete", message, "Delete", move |cx| {
        for path in &paths {
            let full = root.join(path);
            let _ = if full.is_dir() { std::fs::remove_dir_all(&full) } else { std::fs::remove_file(&full) };
        }
        let actions = actions.clone();
        // The dialog's OK runs while its window is being updated; refresh once that's done.
        cx.defer(move |cx| {
            window_handle.update(cx, |_, window, cx| actions(FileAction::FilesChanged, window, cx)).ok();
        });
    }, window, cx);
}

/// A one-field dialog (New File, New Directory, Rename).
pub fn prompt_name(title: String, initial: String, ok_label: &'static str, on_ok: impl Fn(String, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let name = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    let focus = name.clone();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (name, on_ok) = (name.clone(), on_ok.clone());
        dialog.title(title.clone()).w(px(400.)).child(Input::new(&name)).footer(dialogs::footer(ok_label)).on_ok(move |_, window, cx| {
            let value = name.read(cx).value().trim().to_owned();
            if value.is_empty() {
                return false;
            }
            on_ok(value, window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}

/// Cut / Copy in the Project view: absolute paths, and whether they move.
pub type FileClipboard = Rc<RefCell<Option<(Vec<PathBuf>, bool)>>>;

/// The target of a Project view menu.
#[derive(Clone)]
pub struct ProjectTarget {
    pub model: Entity<RepoModel>,
    pub root: PathBuf,
    /// Repository-relative; empty for the root folder.
    pub path: String,
    pub is_dir: bool,
    /// Files under a folder (for Git, Create Gist); with several items
    /// selected, the files of all of them.
    pub files: Vec<String>,
    /// Every selected item, (path, is folder), the clicked one included:
    /// what Cut, Copy, Delete and Git act on, as in IntelliJ.
    pub items: Vec<(String, bool)>,
    pub actions: FileActions,
    pub clipboard: FileClipboard,
}

impl ProjectTarget {
    fn full(&self) -> PathBuf {
        if self.path.is_empty() { self.root.clone() } else { self.root.join(&self.path) }
    }

    /// The folder New and Paste put things in.
    fn folder(&self) -> PathBuf {
        let full = self.full();
        if self.is_dir { full } else { full.parent().map(Path::to_path_buf).unwrap_or(full) }
    }

    fn name(&self) -> String {
        self.full().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// More than one item is selected.
    fn multi(&self) -> bool {
        self.items.len() > 1
    }

    fn item_full(&self, path: &str) -> PathBuf {
        if path.is_empty() { self.root.clone() } else { self.root.join(path) }
    }

    /// The selected items on disk.
    fn fulls(&self) -> Vec<PathBuf> {
        self.items.iter().map(|(p, _)| self.item_full(p)).collect()
    }

    /// The selected items that can be cut or deleted (not the project root).
    fn removable(&self) -> Vec<String> {
        self.items.iter().map(|(p, _)| p.clone()).filter(|p| !p.is_empty()).collect()
    }

    /// The selected items' absolute paths, one per line (the system clipboard).
    fn fulls_text(&self) -> String {
        self.fulls().iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
    }

    /// Cut / Copy: the selected items (Cut leaves the project root out).
    fn put_on_clipboard(&self, cut: bool, cx: &mut App) {
        let fulls: Vec<PathBuf> = if cut { self.removable().iter().map(|p| self.item_full(p)).collect() } else { self.fulls() };
        if fulls.is_empty() {
            return;
        }
        let text = fulls.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
        *self.clipboard.borrow_mut() = Some((fulls, cut));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Two files selected: Compare With… compares them with each other.
    fn file_pair(&self) -> Option<(String, String)> {
        match self.items.as_slice() {
            [(a, false), (b, false)] => Some((a.clone(), b.clone())),
            _ => None,
        }
    }

    fn compare(&self, window: &mut Window, cx: &mut App) {
        match self.file_pair() {
            Some((a, b)) => {
                // The clicked file on the left, the other one on the right.
                let (path, other) = if b == self.path { (b, a) } else { (a, b) };
                let other = self.item_full(&other);
                (self.actions)(FileAction::CompareFiles { path, other }, window, cx)
            }
            None => (self.actions)(FileAction::CompareWithFile(self.path.clone()), window, cx),
        }
    }
}

#[cfg(target_os = "windows")]
const EXPLORER: &str = "Explorer";
#[cfg(target_os = "macos")]
const EXPLORER: &str = "Finder";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const EXPLORER: &str = "Files";

/// Project view: right-click on a file or a folder.
pub fn project_menu(menu: PopupMenu, target: ProjectTarget, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let is_dir = target.is_dir;
    let t = target.clone();
    let menu = menu.submenu("New", window, cx, move |m, _, _| {
        let (file, dir) = (t.clone(), t.clone());
        m.item(entry("File", "").icon(Icon::new(icons::FILE_TEXT)).on_click(move |_, window, cx| new_entry(&file, false, window, cx)))
            .item(entry("Directory", "").icon(Icon::new(icons::FOLDER)).on_click(move |_, window, cx| new_entry(&dir, true, window, cx)))
    });
    let has_clipboard = target.clipboard.borrow().is_some();
    let (t_cut, t_copy, t_paste) = (target.clone(), target.clone(), target.clone());
    let t = target.clone();
    let menu = menu
        .separator()
        .item(entry("Cut", "Ctrl+X").icon(Icon::new(icons::CUT)).disabled(target.removable().is_empty()).on_click(move |_, _, cx| t_cut.put_on_clipboard(true, cx)))
        .item(entry("Copy", "Ctrl+C").icon(Icon::new(icons::COPY)).on_click(move |_, _, cx| t_copy.put_on_clipboard(false, cx)))
        .item(entry("Copy Path/Reference…", "").on_click(move |_, window, cx| {
            let paths: Vec<String> = t.items.iter().map(|(p, _)| p.clone()).collect();
            let at = window.mouse_position();
            crate::ui::copy_path::open_for(&t.model, paths, None, Some(at), window, cx);
        }))
        .item(entry("Paste", "Ctrl+V").icon(Icon::new(icons::PASTE)).disabled(!has_clipboard).on_click(move |_, window, cx| paste(&t_paste, window, cx)))
        .separator()
        .item(entry("Find Usages", "Alt+F7").disabled(true))
        .when(is_dir, |m| {
            let (t_find, t_replace) = (target.clone(), target.clone());
            m.item(entry("Find in Files…", "Ctrl+Shift+F").on_click(move |_, window, cx| {
                (t_find.actions)(FileAction::FindInPath { dir: t_find.path.clone(), replace: false }, window, cx)
            }))
            .item(entry("Replace in Files…", "Ctrl+Shift+R").on_click(move |_, window, cx| {
                (t_replace.actions)(FileAction::FindInPath { dir: t_replace.path.clone(), replace: true }, window, cx)
            }))
        });
    let menu = stub_submenu(menu, "Analyze", &[("Inspect Code…", ""), ("Code Cleanup…", ""), ("Analyze Dependencies…", "")], window, cx);
    let t_rename = target.clone();
    let menu = menu
        .separator()
        .item(entry("Rename…", "Shift+F6").disabled(target.path.is_empty() || target.multi()).on_click(move |_, window, cx| rename(&t_rename, window, cx)));
    let t = target.clone();
    let menu = menu.submenu("Refactor", window, cx, move |m, _, _| {
        let (t_rename, t_delete) = (t.clone(), t.clone());
        m.item(entry("Rename…", "Shift+F6").disabled(t.path.is_empty() || t.multi()).on_click(move |_, window, cx| rename(&t_rename, window, cx)))
            .item(entry("Move…", "F6").disabled(true))
            .item(entry("Copy…", "F5").disabled(true))
            .item(entry("Safe Delete…", "Alt+Delete").disabled(t.removable().is_empty()).on_click(move |_, window, cx| {
                delete_files(t_delete.root.clone(), t_delete.removable(), t_delete.actions.clone(), window, cx)
            }))
    });
    let menu = if is_dir || target.multi() {
        stub_submenu(menu.separator(), "Bookmarks", &[("Add Bookmark", "F11"), ("Add Mnemonic Bookmark…", "Ctrl+F11"), ("Show All Bookmarks", "Shift+F11")], window, cx)
    } else {
        bookmarks_submenu(menu.separator(), target.full().to_string_lossy().into_owned(), window, cx)
    };
    let t_delete = target.clone();
    let menu = menu
        .separator()
        .item(entry("Reformat Code", "Ctrl+Alt+L").icon(Icon::new(icons::FILE_TEXT)).disabled(true))
        .item(entry("Optimize Imports", "Ctrl+Alt+O").disabled(true))
        .when(!target.removable().is_empty(), |m| {
            m.item(entry("Delete…", "Delete").on_click(move |_, window, cx| {
                delete_files(t_delete.root.clone(), t_delete.removable(), t_delete.actions.clone(), window, cx)
            }))
        })
        .when(!is_dir, |m| m.item(entry("Override File Type", "").disabled(true)))
        .separator();
    let t_split = target.clone();
    let menu = menu.when(!is_dir, |m| {
        m.item(entry("Open in Right Split", "Shift+Enter").icon(Icon::new(icons::SPLIT_VERTICALLY)).on_click(move |_, window, cx| {
            (t_split.actions)(FileAction::OpenInSplit(t_split.path.clone()), window, cx)
        }))
    });
    let menu = open_in_submenu(menu, target.full(), is_dir, window, cx);
    let menu = local_history_submenu(menu.separator(), window, cx);
    let git = GitTarget {
        model: target.model.clone(),
        paths: if is_dir || target.multi() { target.files.clone() } else { vec![target.path.clone()] },
        file: (!is_dir && !target.multi()).then(|| target.path.clone()),
        actions: target.actions.clone(),
    };
    let menu = git_submenu(menu, git, window, cx);
    let (t_reload, t_compare, t_clip, t_gist) = (target.clone(), target.clone(), target.clone(), target.clone());
    let menu = menu.item(entry("Repair IDE on File", "").disabled(true))
        .item(entry("Reload from Disk", "").icon(Icon::new(icons::REFRESH)).on_click(move |_, window, cx| {
            (t_reload.actions)(FileAction::FilesChanged, window, cx)
        }))
        .separator()
        .item(entry("Compare With…", "Ctrl+D").icon(Icon::new(icons::VCS_DIFF)).disabled(if target.multi() { target.file_pair().is_none() } else { is_dir }).on_click(move |_, window, cx| {
            t_compare.compare(window, cx)
        }))
        .when(!is_dir && !target.multi(), |m| {
            m.item(entry("Compare with Clipboard", "").icon(Icon::new(icons::PASTE)).on_click(move |_, window, cx| {
                (t_clip.actions)(FileAction::CompareWithClipboard(t_clip.path.clone()), window, cx)
            }))
        })
        .when(target.path.is_empty(), |m| m.item(entry("Open Module Settings", "F4").disabled(true)))
        .separator();
    let menu = if is_dir {
        stub_submenu(menu, "Mark Directory As", &[("Sources Root", ""), ("Test Sources Root", ""), ("Resources Root", ""), ("Test Resources Root", ""), ("Excluded", "")], window, cx)
    } else {
        menu
    };
    menu.item(entry("Create Gist…", "").on_click(move |_, window, cx| {
            let paths = if t_gist.is_dir || t_gist.multi() { t_gist.files.clone() } else { vec![t_gist.path.clone()] };
            let files: Vec<(String, String)> = paths
                .iter()
                .take(30)
                .filter_map(|p| Some((p.rsplit('/').next().unwrap_or(p).to_owned(), std::fs::read_to_string(t_gist.root.join(p)).ok()?)))
                .collect();
            crate::ui::github_dialogs::create_gist(t_gist.model.clone(), files, window, cx)
        }))
        .when(is_dir, |m| m.item(entry("Analyze Dependencies…", "").disabled(true)))
}

/// Open In: the file manager, a terminal in the folder, the system's app for the file.
pub fn open_in_submenu(menu: PopupMenu, full: PathBuf, is_dir: bool, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    menu.submenu("Open In", window, cx, move |m, _, _| {
        let (explorer, app) = (full.clone(), full.clone());
        let folder = if is_dir { full.clone() } else { full.parent().map(Path::to_path_buf).unwrap_or_else(|| full.clone()) };
        m.item(entry(EXPLORER, "").on_click(move |_, _, cx| reveal(&explorer, cx)))
            .item(entry("Terminal", "").on_click(move |_, _, _| open_terminal(&folder)))
            .when(!is_dir, |m| m.item(entry("Associated Application", "").on_click(move |_, _, cx| open_with_system(&app, cx))))
    })
}

/// Bookmarks on a file itself (the Project view, an editor tab), as IntelliJ's.
pub fn bookmarks_submenu(menu: PopupMenu, file: String, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    menu.submenu("Bookmarks", window, cx, move |m, _, cx| {
        let current = crate::bookmarks::Bookmarks::get(cx).at(&file, None).cloned();
        let (f1, f2) = (file.clone(), file.clone());
        let name = Path::new(&file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        m.item(entry(if current.is_some() { "Remove Bookmark" } else { "Add Bookmark" }, "F11").on_click(move |_, _, cx| {
            crate::bookmarks::Bookmarks::update(cx, |b| b.toggle(&f1, None, ""));
        }))
        .item(entry(if current.as_ref().is_some_and(|b| b.mnemonic.is_some()) { "Edit Mnemonic…" } else { "Add Mnemonic Bookmark…" }, "Ctrl+F11").on_click(
            move |_, window, cx| crate::ui::bookmark_dialog::choose_mnemonic(f2.clone(), None, name.clone(), window, cx),
        ))
        .item(entry("Show All Bookmarks", "Shift+F11").on_click(|_, window, cx| window.dispatch_action(Box::new(crate::ui::workspace::ShowBookmarks), cx)))
    })
}

pub fn new_entry(target: &ProjectTarget, directory: bool, window: &mut Window, cx: &mut App) {
    let folder = target.folder();
    let (root, actions) = (target.root.clone(), target.actions.clone());
    let title = if directory { "New Directory" } else { "New File" };
    prompt_name(title.into(), String::new(), "OK", move |name, window, cx| {
        let path = folder.join(&name);
        if directory {
            let _ = std::fs::create_dir_all(&path);
        } else {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if !path.exists() {
                let _ = std::fs::write(&path, "");
            }
        }
        actions(FileAction::FilesChanged, window, cx);
        if !directory {
            if let Ok(relative) = path.strip_prefix(&root) {
                actions(FileAction::OpenFile(relative.to_string_lossy().replace('\\', "/")), window, cx);
            }
        }
    }, window, cx);
}

pub fn rename(target: &ProjectTarget, window: &mut Window, cx: &mut App) {
    let (root, path, actions, model) = (target.root.clone(), target.path.clone(), target.actions.clone(), target.model.clone());
    let title = if target.is_dir { "Rename Directory" } else { "Rename File" };
    prompt_name(title.into(), target.name(), "Refactor", move |name, window, cx| {
        let from = root.join(&path);
        let Some(to) = from.parent().map(|p| p.join(&name)) else { return };
        if to == from || to.exists() {
            return;
        }
        // The repository holding the path (a nested root), with its own path.
        let routed = model.read(cx).route(&path);
        match &routed {
            Some((repo, own)) => crate::git::ops::rename_path(Some(repo), own, &from, &to),
            None => crate::git::ops::rename_path(model.read(cx).repository(), &path, &from, &to),
        }
        let renamed = std::path::Path::new(&path).with_file_name(&name).to_string_lossy().replace('\\', "/");
        actions(FileAction::Moved { from: path.clone(), to: renamed }, window, cx);
    }, window, cx);
}

/// The Project view's keyboard shortcuts for the menu's items (the ones the
/// menu shows on the right), on the selected row: what to run, if the key is one.
/// (Run it outside the view's own update, as a menu click is.)
pub fn project_shortcut(target: &ProjectTarget, keystroke: &gpui_kit::Keystroke) -> Option<Box<dyn FnOnce(&mut Window, &mut App)>> {
    let m = keystroke.modifiers;
    let ctrl = m.control || m.platform;
    let root = target.path.is_empty();
    let removable = !target.removable().is_empty();
    let t = target.clone();
    let full = target.fulls_text();
    let run: Box<dyn FnOnce(&mut Window, &mut App)> = match (ctrl, m.alt, m.shift, keystroke.key.as_str()) {
        (true, false, false, "x") if removable => Box::new(move |_, cx| t.put_on_clipboard(true, cx)),
        (true, false, false, "c") => Box::new(move |_, cx| t.put_on_clipboard(false, cx)),
        (true, false, false, "v") if target.clipboard.borrow().is_some() => Box::new(move |window, cx| paste(&t, window, cx)),
        (true, false, true, "c") => Box::new(move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(full))),
        (false, false, true, "f6") if !root && !target.multi() => Box::new(move |window, cx| rename(&t, window, cx)),
        (false, _, false, "delete") if removable => Box::new(move |window, cx| delete_files(t.root.clone(), t.removable(), t.actions.clone(), window, cx)),
        (false, false, true, "enter") if !target.is_dir => Box::new(move |window, cx| (t.actions)(FileAction::OpenInSplit(t.path.clone()), window, cx)),
        (true, false, true, "f" | "r") => {
            let replace = keystroke.key == "r";
            let dir = if target.is_dir { target.path.clone() } else { Path::new(&target.path).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default() };
            Box::new(move |window, cx| (t.actions)(FileAction::FindInPath { dir, replace }, window, cx))
        }
        (true, false, false, "d") if (!target.multi() && !target.is_dir) || target.file_pair().is_some() => Box::new(move |window, cx| t.compare(window, cx)),
        _ => return None,
    };
    Some(run)
}

fn paste(target: &ProjectTarget, window: &mut Window, cx: &mut App) {
    let Some((sources, cut)) = target.clipboard.borrow().clone() else { return };
    transfer(target, &sources, cut, window, cx);
    if cut {
        *target.clipboard.borrow_mut() = None;
    }
}

/// Drag and drop in the Project view onto a folder: IntelliJ's Move dialog
/// (Ctrl held: Copy) for the dragged items.
pub fn drop_items(target: &ProjectTarget, sources: Vec<PathBuf>, copy: bool, window: &mut Window, cx: &mut App) {
    let folder = target.folder();
    // Nothing to do when every item is already there (or is the folder itself).
    let sources: Vec<PathBuf> = sources
        .into_iter()
        .filter(|s| copy || (s.parent() != Some(folder.as_path()) && !folder.starts_with(s)))
        .collect();
    // An item inside a dragged folder moves with the folder.
    let sources: Vec<PathBuf> = sources.iter().filter(|s| !sources.iter().any(|o| o != *s && s.starts_with(o))).cloned().collect();
    if sources.is_empty() {
        return;
    }
    let what = match sources.as_slice() {
        [one] => format!("\u{201c}{}\u{201d}", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        many => format!("{} items", many.len()),
    };
    let to = folder.strip_prefix(&target.root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let to = if to.is_empty() { target.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { to };
    let (title, ok) = if copy { ("Copy", "Copy") } else { ("Move", "Refactor") };
    let message = format!("{title} {what} to directory \u{201c}{to}\u{201d}?");
    let (target, window_handle) = (target.clone(), window.window_handle());
    dialogs::confirm(title, message, ok, move |cx| {
        let (target, sources) = (target.clone(), sources.clone());
        // The dialog's OK runs while its window is being updated; move once that's done.
        cx.defer(move |cx| {
            window_handle.update(cx, |_, window, cx| transfer(&target, &sources, !copy, window, cx)).ok();
        });
    }, window, cx);
}

/// Copies (or moves) `sources` into the target's folder.
fn transfer(target: &ProjectTarget, sources: &[PathBuf], cut: bool, window: &mut Window, cx: &mut App) {
    let folder = target.folder();
    for source in sources {
        let Some(name) = source.file_name() else { continue };
        let mut dest = folder.join(name);
        if dest == *source && !cut {
            dest = folder.join(format!("{}_copy", name.to_string_lossy()));
        }
        if dest == *source || dest.starts_with(source) {
            continue;
        }
        if cut {
            let relative = |p: &Path| p.strip_prefix(&target.root).ok().map(|r| r.to_string_lossy().replace('\\', "/"));
            let (Some(from), Some(to)) = (relative(source), relative(&dest)) else {
                let _ = std::fs::rename(source, &dest);
                continue;
            };
            // A tracked file moves with `git mv` (one rename, as after Rename);
            // within its repository only.
            let model = target.model.read(cx);
            match (model.route(&from), model.route(&to)) {
                (Some((repo, own)), Some((to_repo, _))) if repo.root() == to_repo.root() => crate::git::ops::rename_path(Some(&repo), &own, source, &dest),
                _ => {
                    let _ = std::fs::rename(source, &dest);
                }
            }
            // Open tabs follow the moved file.
            if dest.exists() {
                (target.actions)(FileAction::Moved { from, to }, window, cx);
            }
        } else {
            let _ = copy_recursive(source, &dest);
        }
    }
    (target.actions)(FileAction::FilesChanged, window, cx);
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Open In › Explorer / Finder / Files: the folder with the item selected.
fn reveal(path: &Path, cx: &App) {
    cx.reveal_path(path);
}

/// Open In › Associated Application.
fn open_with_system(path: &Path, cx: &App) {
    #[cfg(target_os = "windows")]
    cx.open_with_system(&PathBuf::from(path.to_string_lossy().replace('/', "\\")));
    #[cfg(not(target_os = "windows"))]
    cx.open_with_system(path);
}

fn open_terminal(dir: &Path) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", "start", "cmd"]).current_dir(dir).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("x-terminal-emulator").current_dir(dir).spawn();
    let _ = result;
}

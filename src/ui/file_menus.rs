//! Context menus of files and folders in the Project, Commit and Changes
//! tool windows, laid out as IntelliJ's: each item with its shortcut on the
//! right, the same separators, and the shared Git submenu.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName;
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
    Annotate(String),
    ShowHistory(String),
    /// Compare with Revision: the file's revisions to pick from.
    CompareWithRevision(String),
    /// Compare with Branch: a branch, tag or revision to type or pick.
    CompareWithBranch(String),
    /// Compare With…: another file picked from disk.
    CompareWithFile(String),
    /// Compare with Clipboard: the clipboard's text against the file.
    CompareWithClipboard(String),
    /// Commit File(s)…: the Commit tool window with only these included.
    CommitFiles(Vec<String>),
    Branches,
    Unstash,
    /// Files were created, renamed or deleted on disk.
    FilesChanged,
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
    stub_submenu(menu, "Local History", &[("Show History", ""), ("Put Label…", "")], window, cx)
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
        self.model.read(cx).status().entries.iter().map(|e| (e.path.clone(), e.kind)).collect()
    }
}

pub fn git_submenu(menu: PopupMenu, target: GitTarget, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    menu.submenu("Git", window, cx, move |m, _, cx| git_items(m, &target, cx))
}

fn git_items(menu: PopupMenu, target: &GitTarget, cx: &mut Context<PopupMenu>) -> PopupMenu {
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
    .item(entry("Add", "Ctrl+Alt+A").disabled(unversioned.is_empty()).on_click({
        let (model, paths) = (model.clone(), unversioned.clone());
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
    .item(entry("Rollback…", "Ctrl+Alt+Z").icon(Icon::new(IconName::Undo2)).disabled(changed.is_empty()).on_click({
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
            model.update(cx, |m, cx| {
                m.run_operation("Fetch", |repo| {
                    repo.run(["fetch", "--all", "--prune"])?;
                    Ok("Fetched all remotes".into())
                }, cx)
            })
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
        m.run_operation("Add to VCS", move |repo| {
            let mut args = vec!["add".to_owned(), "--".into()];
            args.extend(paths.iter().cloned());
            repo.run(&args)?;
            Ok(format!("Added {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
        }, cx)
    });
}

/// Add to .gitignore (the root one) or to .git/info/exclude.
pub fn ignore(model: &Entity<RepoModel>, paths: Vec<String>, exclude: bool, cx: &mut App) {
    model.update(cx, |m, cx| {
        m.run_operation("Ignore", move |repo| {
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
    /// Files under a folder (for Git, Create Gist).
    pub files: Vec<String>,
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
        m.item(entry("File", "").icon(Icon::new(IconName::File)).on_click(move |_, window, cx| new_entry(&file, false, window, cx)))
            .item(entry("Directory", "").icon(Icon::new(IconName::Folder)).on_click(move |_, window, cx| new_entry(&dir, true, window, cx)))
    });
    let has_clipboard = target.clipboard.borrow().is_some();
    let (t_cut, t_copy, t_paste) = (target.clone(), target.clone(), target.clone());
    let t = target.clone();
    let menu = menu
        .separator()
        .item(entry("Cut", "Ctrl+X").icon(Icon::new(IconName::Scissors)).disabled(target.path.is_empty()).on_click(move |_, _, cx| {
            *t_cut.clipboard.borrow_mut() = Some((vec![t_cut.full()], true));
            cx.write_to_clipboard(ClipboardItem::new_string(t_cut.full().to_string_lossy().into_owned()));
        }))
        .item(entry("Copy", "Ctrl+C").icon(Icon::new(IconName::Copy)).on_click(move |_, _, cx| {
            *t_copy.clipboard.borrow_mut() = Some((vec![t_copy.full()], false));
            cx.write_to_clipboard(ClipboardItem::new_string(t_copy.full().to_string_lossy().into_owned()));
        }))
        .submenu("Copy Path/Reference…", window, cx, move |m, _, _| {
            let (abs, name, rel) = (t.full().to_string_lossy().into_owned(), t.name(), t.path.clone());
            m.item(entry("Absolute Path", "Ctrl+Shift+C").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(abs.clone()))))
                .item(entry("File Name", "").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(name.clone()))))
                .item(entry("Path From Repository Root", "").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(rel.clone()))))
        })
        .item(entry("Paste", "Ctrl+V").icon(Icon::new(IconName::Clipboard)).disabled(!has_clipboard).on_click(move |_, window, cx| paste(&t_paste, window, cx)))
        .separator()
        .item(entry("Find Usages", "Alt+F7").disabled(true))
        .when(is_dir, |m| m.item(entry("Find in Files…", "Ctrl+Shift+F").disabled(true)).item(entry("Replace in Files…", "Ctrl+Shift+R").disabled(true)));
    let menu = stub_submenu(menu, "Analyze", &[("Inspect Code…", ""), ("Code Cleanup…", ""), ("Analyze Dependencies…", "")], window, cx);
    let t_rename = target.clone();
    let menu = menu
        .separator()
        .item(entry("Rename…", "Shift+F6").disabled(target.path.is_empty()).on_click(move |_, window, cx| rename(&t_rename, window, cx)));
    let t = target.clone();
    let menu = menu.submenu("Refactor", window, cx, move |m, _, _| {
        let (t_rename, t_delete) = (t.clone(), t.clone());
        m.item(entry("Rename…", "Shift+F6").disabled(t.path.is_empty()).on_click(move |_, window, cx| rename(&t_rename, window, cx)))
            .item(entry("Move…", "F6").disabled(true))
            .item(entry("Copy…", "F5").disabled(true))
            .item(entry("Safe Delete…", "Alt+Delete").disabled(t.path.is_empty()).on_click(move |_, window, cx| {
                delete_files(t_delete.root.clone(), vec![t_delete.path.clone()], t_delete.actions.clone(), window, cx)
            }))
    });
    let menu = stub_submenu(menu.separator(), "Bookmarks", &[("Toggle Bookmark", "F11"), ("Toggle Bookmark with Mnemonic", "Ctrl+F11"), ("Show All Bookmarks", "Shift+F11")], window, cx);
    let t_delete = target.clone();
    let menu = menu
        .separator()
        .item(entry("Reformat Code", "Ctrl+Alt+L").icon(Icon::new(IconName::TextAlignStart)).disabled(true))
        .item(entry("Optimize Imports", "Ctrl+Alt+O").disabled(true))
        .item(entry("Delete…", "Delete").disabled(target.path.is_empty()).on_click(move |_, window, cx| {
            delete_files(t_delete.root.clone(), vec![t_delete.path.clone()], t_delete.actions.clone(), window, cx)
        }))
        .when(!is_dir, |m| m.item(entry("Override File Type", "").disabled(true)))
        .separator();
    let t_split = target.clone();
    let menu = menu.when(!is_dir, |m| {
        m.item(entry("Open in Right Split", "Shift+Enter").icon(Icon::new(IconName::Columns2)).on_click(move |_, window, cx| {
            (t_split.actions)(FileAction::OpenFile(t_split.path.clone()), window, cx)
        }))
    });
    let t = target.clone();
    let menu = menu.submenu("Open In", window, cx, move |m, _, _| {
        let (t_explorer, t_terminal, t_app) = (t.clone(), t.clone(), t.clone());
        m.item(entry(EXPLORER, "").on_click(move |_, _, cx| cx.reveal_path(&t_explorer.full())))
            .item(entry("Terminal", "").on_click(move |_, _, _| open_terminal(&t_terminal.folder())))
            .when(!t.is_dir, |m| m.item(entry("Associated Application", "").on_click(move |_, _, cx| cx.open_with_system(&t_app.full()))))
    });
    let menu = local_history_submenu(menu.separator(), window, cx);
    let git = GitTarget {
        model: target.model.clone(),
        paths: if is_dir { target.files.clone() } else { vec![target.path.clone()] },
        file: (!is_dir).then(|| target.path.clone()),
        actions: target.actions.clone(),
    };
    let menu = git_submenu(menu, git, window, cx);
    let (t_reload, t_compare, t_clip, t_gist) = (target.clone(), target.clone(), target.clone(), target.clone());
    let menu = menu.item(entry("Repair IDE on File", "").disabled(true))
        .item(entry("Reload from Disk", "").icon(Icon::new(IconName::RefreshCw)).on_click(move |_, window, cx| {
            (t_reload.actions)(FileAction::FilesChanged, window, cx)
        }))
        .separator()
        .item(entry("Compare With…", "Ctrl+D").icon(Icon::new(IconName::GitCompare)).disabled(is_dir).on_click(move |_, window, cx| {
            (t_compare.actions)(FileAction::CompareWithFile(t_compare.path.clone()), window, cx)
        }))
        .item(entry("Compare with Clipboard", "").icon(Icon::new(IconName::Clipboard)).disabled(is_dir).on_click(move |_, window, cx| {
            (t_clip.actions)(FileAction::CompareWithClipboard(t_clip.path.clone()), window, cx)
        }))
        .separator();
    let menu = if is_dir {
        stub_submenu(menu, "Mark Directory As", &[("Sources Root", ""), ("Test Sources Root", ""), ("Resources Root", ""), ("Test Resources Root", ""), ("Excluded", "")], window, cx)
    } else {
        menu
    };
    menu.item(entry("Create Gist…", "").on_click(move |_, window, cx| {
            let paths = if t_gist.is_dir { t_gist.files.clone() } else { vec![t_gist.path.clone()] };
            let files: Vec<(String, String)> = paths
                .iter()
                .take(30)
                .filter_map(|p| Some((p.rsplit('/').next().unwrap_or(p).to_owned(), std::fs::read_to_string(t_gist.root.join(p)).ok()?)))
                .collect();
            crate::ui::github_dialogs::create_gist(t_gist.model.clone(), files, window, cx)
        }))
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

fn rename(target: &ProjectTarget, window: &mut Window, cx: &mut App) {
    let (root, path, actions, model) = (target.root.clone(), target.path.clone(), target.actions.clone(), target.model.clone());
    let title = if target.is_dir { "Rename Directory" } else { "Rename File" };
    prompt_name(title.into(), target.name(), "Refactor", move |name, window, cx| {
        let from = root.join(&path);
        let Some(to) = from.parent().map(|p| p.join(&name)) else { return };
        if to == from || to.exists() {
            return;
        }
        // Tracked files move with git mv, so the rename shows as one change.
        let tracked = model.read(cx).repository().is_some_and(|r| r.run(["ls-files", "--error-unmatch", "--", path.as_str()]).is_ok());
        let moved = tracked
            && model
                .read(cx)
                .repository()
                .is_some_and(|r| r.run(["mv", "--", from.to_string_lossy().as_ref(), to.to_string_lossy().as_ref()]).is_ok());
        if !moved {
            let _ = std::fs::rename(&from, &to);
        }
        actions(FileAction::FilesChanged, window, cx);
    }, window, cx);
}

fn paste(target: &ProjectTarget, window: &mut Window, cx: &mut App) {
    let Some((sources, cut)) = target.clipboard.borrow().clone() else { return };
    let folder = target.folder();
    for source in &sources {
        let Some(name) = source.file_name() else { continue };
        let mut dest = folder.join(name);
        if dest == *source && !cut {
            dest = folder.join(format!("{}_copy", name.to_string_lossy()));
        }
        if dest == *source || dest.starts_with(source) {
            continue;
        }
        let _ = if cut { std::fs::rename(source, &dest) } else { copy_recursive(source, &dest) };
    }
    if cut {
        *target.clipboard.borrow_mut() = None;
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

fn open_terminal(dir: &Path) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", "start", "cmd"]).current_dir(dir).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = std::process::Command::new("x-terminal-emulator").current_dir(dir).spawn();
    let _ = result;
}

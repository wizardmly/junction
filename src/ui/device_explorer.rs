//! The Device Explorer tool window, as Android Studio's: a device picker
//! over two tabs. Files is the device's file tree (Name, Permissions, Date,
//! Size) with New File / Directory, Open, Save As / Save To, Upload, Delete,
//! Synchronize and Copy Path(s); Processes lists the debuggable processes
//! (Process Name, PID, ABI, VM, User ID, Debugger) with Kill, Force stop,
//! Clear app data and Uninstall app. Both can be narrowed to the project's
//! application IDs with the package filter.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    input::{Input, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, Modifiers, MouseButton, ParentElement as _, PathPromptOptions, Render, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _,
    px, rgb, uniform_list,
};

use crate::android::files::{self, Entry, EntryKind, FileSystem, Totals};
use crate::android::jdwp::{self, Process};
use crate::android::{Device, Kind};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;
use crate::ui::file_menus::entry as menu_entry;

pub enum DeviceExplorerEvent {
    Hide,
    Notify { title: String, message: String, error: bool },
    /// Open: a device file copied to the computer, for the editor.
    Open(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Files,
    Processes,
}

const PROCESS_POLL: Duration = Duration::from_secs(2);
const ROW_H: f32 = 22.;
const BUSY: &str = "Device Explorer is busy, please retry later or cancel current operation";

/// A folder's children as far as they're known.
enum Dir {
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

#[derive(Clone)]
enum Row {
    Entry { path: String, depth: usize },
    Loading { depth: usize },
    Error { depth: usize, text: String },
}

/// The Files tab of the active device.
struct Files {
    serial: String,
    fs: Option<Arc<FileSystem>>,
    error: Option<String>,
    root: String,
    entries: HashMap<String, Entry>,
    dirs: HashMap<String, Dir>,
    /// Symbolic links that lead to folders.
    dir_links: HashSet<String>,
    expanded: HashSet<String>,
    selected: Vec<String>,
    anchor: Option<String>,
    rows: Vec<Row>,
}

impl Files {
    fn new(serial: String, root: String) -> Self {
        Self {
            serial,
            fs: None,
            error: None,
            root,
            entries: HashMap::new(),
            dirs: HashMap::new(),
            dir_links: HashSet::new(),
            expanded: HashSet::new(),
            selected: Vec::new(),
            anchor: None,
            rows: Vec::new(),
        }
    }

    fn is_dir(&self, path: &str) -> bool {
        self.entries.get(path).is_some_and(Entry::is_dir) || self.dir_links.contains(path)
    }

    fn run_as(&self, path: &str) -> Option<String> {
        self.entries.get(path).and_then(|e| e.run_as.clone())
    }

    fn rebuild(&mut self) {
        fn walk(files: &Files, dir: &str, depth: usize, rows: &mut Vec<Row>) {
            match files.dirs.get(dir) {
                Some(Dir::Loaded(children)) => {
                    for child in children {
                        rows.push(Row::Entry { path: child.clone(), depth });
                        if files.expanded.contains(child) && files.is_dir(child) {
                            walk(files, child, depth + 1, rows);
                        }
                    }
                }
                Some(Dir::Loading) | None => rows.push(Row::Loading { depth }),
                Some(Dir::Failed(text)) => rows.push(Row::Error { depth, text: text.clone() }),
            }
        }
        let mut rows = Vec::new();
        walk(self, &self.root.clone(), 0, &mut rows);
        self.rows = rows;
        let visible: HashSet<&str> = self.rows.iter().filter_map(|r| if let Row::Entry { path, .. } = r { Some(path.as_str()) } else { None }).collect();
        self.selected.retain(|p| visible.contains(p.as_str()));
    }

    fn row_of(&self, path: &str) -> Option<usize> {
        self.rows.iter().position(|r| matches!(r, Row::Entry { path: p, .. } if p == path))
    }


    /// The one selected folder New and Upload act on.
    fn selected_dir(&self) -> Option<String> {
        match self.selected.as_slice() {
            [path] if self.is_dir(path) => Some(path.clone()),
            _ => None,
        }
    }
}

struct Processes {
    serial: String,
    list: Vec<Process>,
    known: HashMap<u32, Process>,
    selected: Vec<u32>,
    anchor: Option<u32>,
    loaded: bool,
    error: Option<String>,
}

impl Processes {
    fn new(serial: String) -> Self {
        Self { serial, list: Vec::new(), known: HashMap::new(), selected: Vec::new(), anchor: None, loaded: false, error: None }
    }

    fn selected_processes(&self) -> Vec<Process> {
        self.list.iter().filter(|p| self.selected.contains(&p.pid)).cloned().collect()
    }
}

/// A device in the picker.
#[derive(Clone, PartialEq)]
struct Target {
    serial: String,
    name: String,
    version: String,
    kind: Kind,
}

pub struct DeviceExplorer {
    device_manager: Entity<DeviceManager>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    devices: Vec<Target>,
    active: Option<String>,
    tab: Tab,
    files: Option<Files>,
    processes: Option<Processes>,
    package_filter: bool,
    app_ids: Vec<String>,
    app_ids_for: Option<PathBuf>,
    shown: bool,
    /// What a transfer or delete is doing (the progress line); one at a time.
    busy: Option<String>,
    process_poll: Option<Task<()>>,
    focus: FocusHandle,
    files_scroll: UniformListScrollHandle,
    process_scroll: UniformListScrollHandle,
    _observe: Subscription,
}

impl EventEmitter<DeviceExplorerEvent> for DeviceExplorer {}

impl Focusable for DeviceExplorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeviceExplorer {
    pub fn new(device_manager: Entity<DeviceManager>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&device_manager, |this, dm, cx| {
            let devices: Vec<Target> = dm.read(cx).devices().iter().filter_map(target).collect();
            this.set_devices(devices, cx);
        });
        Self {
            device_manager,
            project,
            devices: Vec::new(),
            active: None,
            tab: Tab::Files,
            files: None,
            processes: None,
            package_filter: false,
            app_ids: Vec::new(),
            app_ids_for: None,
            shown: false,
            busy: None,
            process_poll: None,
            focus: cx.focus_handle(),
            files_scroll: UniformListScrollHandle::new(),
            process_scroll: UniformListScrollHandle::new(),
            _observe: observe,
        }
    }

    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if shown {
            self.load_app_ids(cx);
            if self.files.is_none() {
                self.reset_device(cx);
            }
            self.poll_processes(cx);
        }
    }

    /// Device Manager's Open in Device Explorer.
    pub fn show_device(&mut self, serial: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(serial) {
            self.active = Some(serial.to_owned());
            self.reset_device(cx);
        }
        cx.notify();
    }

    fn set_devices(&mut self, devices: Vec<Target>, cx: &mut Context<Self>) {
        if devices == self.devices {
            return;
        }
        self.devices = devices;
        let active_gone = self.active.as_ref().is_none_or(|s| !self.devices.iter().any(|d| &d.serial == s));
        if active_gone {
            self.active = self.devices.first().map(|d| d.serial.clone());
            self.reset_device(cx);
        }
        cx.notify();
    }

    fn adb(&self, cx: &App) -> Option<crate::android::adb::Adb> {
        self.device_manager.read(cx).adb()
    }

    fn notify(&self, title: &str, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(DeviceExplorerEvent::Notify { title: title.into(), message: message.into(), error });
    }

    /// The project's application IDs, for the package filter.
    fn load_app_ids(&mut self, cx: &mut Context<Self>) {
        let project = (self.project)(cx);
        if project == self.app_ids_for {
            return;
        }
        self.app_ids_for = project.clone();
        let Some(project) = project else { return };
        cx.spawn(async move |this, cx| {
            let ids = cx.background_spawn(async move { crate::android::application_ids(&project) }).await;
            this.update(cx, |this, cx| {
                let changed = ids != this.app_ids;
                this.app_ids = ids;
                if changed && this.package_filter {
                    this.reset_device(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn filtering(&self) -> bool {
        self.package_filter && !self.app_ids.is_empty()
    }

    fn toggle_package_filter(&mut self, cx: &mut Context<Self>) {
        self.package_filter = !self.package_filter;
        self.reset_device(cx);
        cx.notify();
    }

    /// A new device, or the root changed: start over.
    fn reset_device(&mut self, cx: &mut Context<Self>) {
        self.files = None;
        self.processes = None;
        let Some(serial) = self.active.clone() else {
            cx.notify();
            return;
        };
        if !self.shown {
            return;
        }
        let root = if self.filtering() { "/data/data".to_owned() } else { "/".to_owned() };
        self.files = Some(Files::new(serial.clone(), root));
        self.processes = Some(Processes::new(serial.clone()));
        let Some(adb) = self.adb(cx) else { return };
        cx.spawn(async move |this, cx| {
            let fs = cx.background_spawn({
                let serial = serial.clone();
                async move { FileSystem::open(adb, &serial) }
            });
            let fs = Arc::new(fs.await);
            this.update(cx, |this, cx| {
                let Some(files) = this.files.as_mut().filter(|f| f.serial == serial) else { return };
                files.fs = Some(fs);
                let root = files.root.clone();
                this.load_dir(&root, cx);
            })
            .ok();
        })
        .detach();
        self.poll_processes(cx);
        cx.notify();
    }

    // ---------------------------------------------------------------- Files

    fn load_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        let filter = self.filtering().then(|| self.app_ids.clone());
        let Some(files) = self.files.as_mut() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let run_as = files.run_as(path);
        let serial = files.serial.clone();
        if !matches!(files.dirs.get(path), Some(Dir::Loaded(_))) {
            files.dirs.insert(path.to_owned(), Dir::Loading);
        }
        files.rebuild();
        cx.notify();
        let path = path.to_owned();
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn({
                    let (fs, path, run_as) = (fs.clone(), path.clone(), run_as.clone());
                    async move {
                        let mut result = fs.list(&path, run_as.as_deref());
                        // The package filter: only the project's folders under /data/data.
                        if let (Ok(entries), Some(ids)) = (&mut result, &filter) {
                            if path == "/data/data" {
                                entries.retain(|e| ids.contains(&e.name));
                            }
                        }
                        let links: HashSet<String> = match &result {
                            Ok(entries) => {
                                let links: Vec<String> = entries.iter().filter(|e| e.kind == EntryKind::Symlink).map(|e| e.path.clone()).collect();
                                fs.dir_links(&links, run_as.as_deref())
                            }
                            Err(_) => HashSet::new(),
                        };
                        (result, links)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(files) = this.files.as_mut().filter(|f| f.serial == serial) else { return };
                let (result, links) = listed;
                match result {
                    Ok(entries) => {
                        let children: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
                        // Forget what was below entries that are gone.
                        if let Some(Dir::Loaded(old)) = files.dirs.get(&path) {
                            for gone in old.iter().filter(|p| !children.contains(p)) {
                                files.entries.remove(gone);
                                files.expanded.remove(gone);
                            }
                        }
                        for entry in entries {
                            files.dir_links.remove(&entry.path);
                            files.entries.insert(entry.path.clone(), entry);
                        }
                        files.dir_links.extend(links);
                        files.dirs.insert(path.clone(), Dir::Loaded(children));
                    }
                    Err(error) => {
                        files.dirs.insert(path.clone(), Dir::Failed(error));
                    }
                }
                files.rebuild();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_expanded(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_mut() else { return };
        if !files.is_dir(path) {
            return;
        }
        if !files.expanded.remove(path) {
            files.expanded.insert(path.to_owned());
            if !matches!(files.dirs.get(path), Some(Dir::Loaded(_))) {
                self.load_dir(path, cx);
                return;
            }
        }
        files.rebuild();
        cx.notify();
    }

    fn expand(&mut self, path: &str, cx: &mut Context<Self>) {
        let expanded = self.files.as_ref().is_some_and(|f| f.expanded.contains(path));
        if !expanded {
            self.toggle_expanded(path, cx);
        }
    }

    fn click_row(&mut self, path: &str, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if event.click_count() >= 2 {
            let is_dir = self.files.as_ref().is_some_and(|f| f.is_dir(path));
            if is_dir {
                self.toggle_expanded(path, cx);
            } else {
                self.open(vec![path.to_owned()], cx);
            }
            return;
        }
        self.select_file(path, event.modifiers(), cx);
    }

    fn select_file(&mut self, path: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_mut() else { return };
        if modifiers.shift {
            let anchor = files.anchor.clone().unwrap_or_else(|| path.to_owned());
            let (Some(a), Some(b)) = (files.row_of(&anchor), files.row_of(path)) else { return };
            let range = a.min(b)..=a.max(b);
            files.selected = files.rows[range].iter().filter_map(|r| if let Row::Entry { path, .. } = r { Some(path.clone()) } else { None }).collect();
        } else if modifiers.secondary() {
            if let Some(ix) = files.selected.iter().position(|p| p == path) {
                files.selected.remove(ix);
            } else {
                files.selected.push(path.to_owned());
            }
            files.anchor = Some(path.to_owned());
        } else {
            files.selected = vec![path.to_owned()];
            files.anchor = Some(path.to_owned());
        }
        cx.notify();
    }

    fn file_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let ctrl = m.control || m.platform;
        let Some(files) = self.files.as_ref() else { return };
        let selected = files.selected.clone();
        let handled = match k.key.as_str() {
            "s" if ctrl && m.shift && !selected.is_empty() => {
                self.save_as(selected, cx);
                true
            }
            "o" if ctrl && m.shift => {
                if let Some(dir) = files.selected_dir() {
                    self.upload(dir, window, cx);
                }
                true
            }
            "c" if ctrl && m.shift && !selected.is_empty() => {
                self.copy_paths(&selected, cx);
                true
            }
            "f5" if ctrl => {
                self.synchronize(cx);
                true
            }
            "delete" if !selected.is_empty() => {
                self.delete(selected, window, cx);
                true
            }
            "enter" if !selected.is_empty() => {
                let dirs: Vec<String> = selected.iter().filter(|p| files.is_dir(p)).cloned().collect();
                let plain: Vec<String> = selected.iter().filter(|p| !files.is_dir(p)).cloned().collect();
                for dir in dirs {
                    self.toggle_expanded(&dir, cx);
                }
                if !plain.is_empty() {
                    self.open(plain, cx);
                }
                true
            }
            "up" | "down" if !m.alt && !ctrl => {
                let current = selected.last().and_then(|p| files.row_of(p));
                let rows = &files.rows;
                let step = |from: usize, down: bool| -> Option<usize> {
                    let mut ix = from;
                    loop {
                        ix = if down { ix + 1 } else { ix.checked_sub(1)? };
                        match rows.get(ix)? {
                            Row::Entry { .. } => return Some(ix),
                            _ => continue,
                        }
                    }
                };
                let next = match current {
                    Some(ix) => step(ix, k.key == "down"),
                    None => rows.iter().position(|r| matches!(r, Row::Entry { .. })),
                };
                if let Some(Row::Entry { path, .. }) = next.and_then(|ix| rows.get(ix)) {
                    let path = path.clone();
                    let ix = next.unwrap_or(0);
                    let mods = Modifiers { shift: m.shift, ..Default::default() };
                    self.select_file(&path, mods, cx);
                    self.files_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
                true
            }
            "right" => {
                if let [path] = selected.as_slice() {
                    if files.is_dir(path) {
                        if files.expanded.contains(path) {
                            if let Some(Dir::Loaded(children)) = files.dirs.get(path) {
                                if let Some(first) = children.first().cloned() {
                                    self.select_file(&first, Modifiers::default(), cx);
                                }
                            }
                        } else {
                            let path = path.clone();
                            self.expand(&path, cx);
                        }
                    }
                }
                true
            }
            "left" => {
                if let [path] = selected.as_slice() {
                    let path = path.clone();
                    if files.expanded.contains(&path) {
                        self.toggle_expanded(&path, cx);
                    } else {
                        let parent = files::parent(&path).to_owned();
                        if parent != files.root && files.entries.contains_key(&parent) {
                            self.select_file(&parent, Modifiers::default(), cx);
                            if let Some(ix) = self.files.as_ref().and_then(|f| f.row_of(&parent)) {
                                self.files_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn copy_paths(&self, paths: &[String], cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(paths.join("\n")));
    }

    /// Synchronize: every loaded folder that shows is listed again.
    fn synchronize(&mut self, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let mut reload = vec![files.root.clone()];
        reload.extend(files.expanded.iter().filter(|p| matches!(files.dirs.get(*p), Some(Dir::Loaded(_)) | Some(Dir::Failed(_)))).cloned());
        for path in reload {
            self.load_dir(&path, cx);
        }
    }

    fn start_busy(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        if self.busy.is_some() {
            self.notify("Device Explorer", BUSY, true, cx);
            return false;
        }
        self.busy = Some(text);
        cx.notify();
        true
    }

    fn end_busy(&mut self, cx: &mut Context<Self>) {
        self.busy = None;
        cx.notify();
    }

    /// New › File / Directory: a name, created in the selected folder.
    fn new_entry(&mut self, parent: String, directory: bool, initial: String, window: &mut Window, cx: &mut Context<Self>) {
        let (title, prompt, empty, failure) = if directory {
            ("New Folder", "Enter new folder name:", "Folder name cannot be empty", "Could not create folder")
        } else {
            ("New File", "Enter new file name:", "File name cannot be empty", "Could not create file")
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let focus = input.clone();
        let this = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let value = input.read(cx).value().trim().to_owned();
            let error = if value.contains('/') { Some("Path cannot contain \"/\" characters") } else if value.is_empty() { Some(empty) } else { None };
            let (input, this, parent) = (input.clone(), this.clone(), parent.clone());
            dialog
                .title(title)
                .w(px(400.))
                .child(
                    v_flex()
                        .gap_2()
                        .text_sm()
                        .child(prompt)
                        .child(Input::new(&input))
                        .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_deleted).child(e))),
                )
                .footer(crate::ui::dialogs::footer_enabled("OK", error.is_none()))
                .on_ok(move |_, window, cx| {
                    let name = input.read(cx).value().trim().to_owned();
                    if name.is_empty() || name.contains('/') {
                        return false;
                    }
                    this.update(cx, |this, cx| this.create(parent.clone(), name, directory, failure, window, cx));
                    true
                })
        });
        crate::ui::dialogs::focus_input(&focus, window, cx);
    }

    fn create(&mut self, parent: String, name: String, directory: bool, failure: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let run_as = files.run_as(&parent);
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let (parent, name) = (parent.clone(), name.clone());
                    async move {
                        if directory { fs.new_directory(&parent, &name, run_as.as_deref()) } else { fs.new_file(&parent, &name, run_as.as_deref()) }
                    }
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.load_dir(&parent, cx);
                    if let Some(files) = this.files.as_mut() {
                        files.expanded.insert(parent.clone());
                        let created = files::resolve(&parent, &name);
                        files.selected = vec![created.clone()];
                        files.anchor = Some(created);
                    }
                }
                Err(error) => {
                    let message = format!("{failure} '{name}':\n{error}");
                    let retry = !error.ends_with("Permission denied");
                    let this_entity = cx.entity();
                    window_handle
                        .update(cx, |_, window, cx| {
                            message_dialog("Error", message, window, cx);
                            if retry {
                                // Ask again with the name typed, as Android Studio does.
                                this_entity.update(cx, |this, cx| this.new_entry(parent.clone(), directory, name.clone(), window, cx));
                            }
                        })
                        .ok();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open: copy to the computer, then show in the editor.
    fn open(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let entries: Vec<Entry> =
            paths.iter().filter_map(|p| files.entries.get(p)).filter(|e| !e.is_dir() && !files.dir_links.contains(&e.path)).cloned().collect();
        if entries.is_empty() {
            return;
        }
        let names = entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>().join(", ");
        let serial = files.serial.clone();
        if !self.start_busy(format!("Downloading {names}"), cx) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move {
                    entries
                        .into_iter()
                        .map(|entry| {
                            let local = files::local_copy(&serial, &entry.path);
                            let _ = std::fs::remove_file(&local);
                            fs.download(&entry, &local).map(|_| local).map_err(|e| format!("Error opening contents of device file {}: {e}", entry.path))
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                for result in results {
                    match result {
                        Ok(local) => cx.emit(DeviceExplorerEvent::Open(local)),
                        Err(error) => this.notify("Device Explorer", error, true, cx),
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Save As… (one entry) / Save To… (several): pull to a place picked on the computer.
    fn save_as(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let entries: Vec<(Entry, bool)> = paths.iter().filter_map(|p| files.entries.get(p).map(|e| (e.clone(), files.is_dir(p)))).collect();
        if entries.is_empty() {
            return;
        }
        if self.busy.is_some() {
            self.notify("Device Explorer", BUSY, true, cx);
            return;
        }
        let base = crate::ui::running_devices::default_save_dir();
        // One file: a file name; a folder or several entries: a folder to save into.
        let single_file = entries.len() == 1 && !entries[0].1;
        let pick: Task<Option<Vec<(Entry, PathBuf)>>> = if single_file {
            let name = entries[0].0.name.clone();
            let answer = cx.prompt_for_new_path(&base, Some(&name));
            let entry = entries[0].0.clone();
            cx.spawn(async move |_, _| crate::ui::running_devices::chosen_path(answer.await, &name).map(|path| vec![(entry, path)]))
        } else {
            let answer = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some(if entries.len() > 1 { "Save To".into() } else { "Save As".into() }),
            });
            cx.spawn(async move |_, _| {
                let folder = match answer.await {
                    Ok(Ok(Some(mut paths))) => paths.pop()?,
                    Ok(Ok(None)) => return None,
                    _ => base,
                };
                Some(entries.into_iter().map(|(entry, _)| {
                    let target = folder.join(&entry.name);
                    (entry, target)
                }).collect())
            })
        };
        cx.spawn(async move |this, cx| {
            let Some(targets) = pick.await else { return };
            let names = targets.iter().map(|(e, _)| e.name.clone()).collect::<Vec<_>>().join(", ");
            if !this.update(cx, |this, cx| this.start_busy(format!("Downloading {names}"), cx)).unwrap_or(false) {
                return;
            }
            let started = Instant::now();
            let (totals, problems) = cx
                .background_spawn(async move {
                    let mut totals = Totals::default();
                    let mut problems = Vec::new();
                    for (entry, target) in targets {
                        match fs.download(&entry, &target) {
                            Ok(()) => totals.add(Totals::of(&target)),
                            Err(e) => problems.push(format!("Error saving contents of device file {}: {e}", entry.path)),
                        }
                    }
                    (totals, problems)
                })
                .await;
            let (ok, message) = files::transfer_summary(totals, started.elapsed().as_millis(), &problems, true);
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                this.notify("Device Explorer", message, !ok, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Upload…: files from the computer into the selected folder.
    fn upload(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            self.notify("Device Explorer", BUSY, true, cx);
            return;
        }
        let answer = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Upload".into()) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| match answer.await {
            Ok(Ok(Some(paths))) => {
                this.update(cx, |this, cx| this.upload_paths(dir, paths, cx)).ok();
            }
            Ok(Ok(None)) => {}
            // No file dialog on this system: type the path.
            _ => {
                window_handle
                    .update(cx, |_, window, cx| {
                        let this = this.clone();
                        crate::ui::file_menus::prompt_name("Upload".into(), String::new(), "Upload", move |text, _, cx| {
                            let paths: Vec<PathBuf> = text.split(';').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect();
                            this.update(cx, |this, cx| this.upload_paths(dir.clone(), paths, cx)).ok();
                        }, window, cx);
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn upload_paths(&mut self, dir: String, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        if paths.is_empty() {
            return;
        }
        let run_as = files.run_as(&dir);
        let names = paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
        if !self.start_busy(format!("Uploading {names}"), cx) {
            return;
        }
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let (totals, problems) = cx
                .background_spawn({
                    let dir = dir.clone();
                    async move {
                        let mut totals = Totals::default();
                        let mut problems = Vec::new();
                        for path in paths {
                            if !path.exists() {
                                problems.push(format!("{}: No such file or directory", path.display()));
                                continue;
                            }
                            match fs.upload(&path, &dir, run_as.as_deref()) {
                                Ok(()) => totals.add(Totals::of(&path)),
                                Err(e) => problems.push(format!("Error uploading {}: {e}", path.display())),
                            }
                        }
                        (totals, problems)
                    }
                })
                .await;
            let (ok, message) = files::transfer_summary(totals, started.elapsed().as_millis(), &problems, false);
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                this.notify("Device Explorer", message, !ok, cx);
                if let Some(files) = this.files.as_mut() {
                    files.expanded.insert(dir.clone());
                }
                this.load_dir(&dir, cx);
            })
            .ok();
        })
        .detach();
    }

    fn delete(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let entries: Vec<(Entry, bool)> = paths.iter().filter_map(|p| files.entries.get(p).map(|e| (e.clone(), files.is_dir(p)))).collect();
        if entries.is_empty() {
            return;
        }
        let dirs = entries.iter().filter(|(_, d)| *d).count();
        let message = match entries.as_slice() {
            [(entry, true)] => format!("Are you sure you want to delete the selected folder '{}'?", entry.name),
            [(entry, false)] => format!("Are you sure you want to delete the selected file '{}'?", entry.name),
            many if dirs > 0 && dirs < many.len() => format!("Are you sure you want to delete {} selected files and directories?", many.len()),
            many if dirs == many.len() => format!("Are you sure you want to delete {} selected folders?", many.len()),
            many => format!("Are you sure you want to delete {} selected files?", many.len()),
        };
        let this = cx.entity();
        let window_handle = window.window_handle();
        crate::ui::dialogs::confirm("Delete", message, "Delete", move |cx| {
            let entries: Vec<Entry> = entries.iter().map(|(e, _)| e.clone()).collect();
            this.update(cx, |this, cx| this.delete_entries(entries, window_handle, cx));
        }, window, cx);
    }

    fn delete_entries(&mut self, entries: Vec<Entry>, window: gpui_kit::AnyWindowHandle, cx: &mut Context<Self>) {
        let Some(fs) = self.files.as_ref().and_then(|f| f.fs.clone()) else { return };
        if !self.start_busy(format!("Deleting {} entries", entries.len()), cx) {
            return;
        }
        let parents: HashSet<String> = entries.iter().map(|e| files::parent(&e.path).to_owned()).collect();
        cx.spawn(async move |this, cx| {
            let problems: Vec<String> = cx
                .background_spawn(async move { entries.iter().filter_map(|e| fs.delete(e).err().map(|err| format!("{}: {err}", e.path))).collect() })
                .await;
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                for parent in &parents {
                    this.load_dir(parent, cx);
                }
                if !problems.is_empty() {
                    let message = if let [one] = problems.as_slice() {
                        format!("Could not erase file or folder:\n{one}")
                    } else {
                        let shown: Vec<&str> = problems.iter().take(10).map(String::as_str).collect();
                        format!("Could not erase files or folders:\n  {}{}", shown.join(",\n  "), if problems.len() > 10 { "\n  ..." } else { "" })
                    };
                    window.update(cx, |_, window, cx| message_dialog("Error", message, window, cx)).ok();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Files dropped from the computer onto a folder (or the tree: the root).
    fn drop_paths(&mut self, target: Option<String>, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let dir = match target {
            Some(path) if files.is_dir(&path) => path,
            Some(path) => files::parent(&path).to_owned(),
            None => files.root.clone(),
        };
        self.upload_paths(dir, paths.paths().to_vec(), cx);
    }

    fn file_menu(this: Entity<Self>, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let state = this.read(cx);
        let Some(files) = state.files.as_ref() else { return menu };
        let selected = files.selected.clone();
        if selected.is_empty() {
            return menu;
        }
        let multi = selected.len() > 1;
        let dir = files.selected_dir();
        let any_file = selected.iter().any(|p| !files.is_dir(p));
        let act = |f: Rc<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>| {
            let this = this.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| this.update(cx, |this, cx| f(this, window, cx))
        };
        let mut menu = menu;
        // New ›: on one folder; greyed out with several entries, hidden on a file.
        if dir.is_some() || multi {
            let new_dir = dir.clone().unwrap_or_default();
            let (file_this, dir_this) = (this.clone(), this.clone());
            let (file_parent, dir_parent) = (new_dir.clone(), new_dir.clone());
            let sub = PopupMenu::build(window, cx, move |sub, _, _| {
                let (file_this, dir_this, file_parent, dir_parent) = (file_this.clone(), dir_this.clone(), file_parent.clone(), dir_parent.clone());
                sub.item(PopupMenuItem::new("File").icon(Icon::new(icons::FILE_TEXT)).on_click(move |_, window, cx| {
                    file_this.update(cx, |this, cx| this.new_entry(file_parent.clone(), false, "NewTextFile.txt".into(), window, cx))
                }))
                .item(PopupMenuItem::new("Directory").icon(Icon::new(icons::FOLDER)).on_click(move |_, window, cx| {
                    dir_this.update(cx, |this, cx| this.new_entry(dir_parent.clone(), true, "NewFolder".into(), window, cx))
                }))
            });
            menu = menu.item(PopupMenuItem::submenu("New", sub).disabled(dir.is_none())).separator();
        }
        if any_file {
            let paths = selected.clone();
            menu = menu.item(menu_entry("Open", "").icon(Icon::new(icons::MENU_OPEN)).on_click(act(Rc::new(move |this, _, cx| this.open(paths.clone(), cx)))));
        }
        let save = selected.clone();
        menu = menu.item(
            menu_entry(if multi { "Save To..." } else { "Save As..." }, "Ctrl+Shift+S")
                .icon(Icon::new(icons::DOWNLOAD))
                .on_click(act(Rc::new(move |this, _, cx| this.save_as(save.clone(), cx)))),
        );
        if let Some(dir) = dir.clone() {
            menu = menu.item(menu_entry("Upload...", "Ctrl+Shift+O").icon(Icon::new(icons::UPLOAD)).on_click(act(Rc::new(move |this, window, cx| this.upload(dir.clone(), window, cx)))));
        }
        let delete = selected.clone();
        menu = menu
            .item(menu_entry("Delete...", "Delete").icon(Icon::new(icons::DELETE)).on_click(act(Rc::new(move |this, window, cx| this.delete(delete.clone(), window, cx)))))
            .separator()
            .item(menu_entry("Synchronize", "Ctrl+F5").icon(Icon::new(icons::STUDIO_REFRESH)).on_click(act(Rc::new(|this, _, cx| this.synchronize(cx)))));
        let copy = selected.clone();
        menu.item(
            menu_entry(if multi { "Copy Paths" } else { "Copy Path" }, "Ctrl+Shift+C")
                .icon(Icon::new(icons::COPY))
                .on_click(act(Rc::new(move |this, _, cx| this.copy_paths(&copy, cx)))),
        )
    }

    // ------------------------------------------------------------ Processes

    fn poll_processes(&mut self, cx: &mut Context<Self>) {
        if self.process_poll.is_some() || !self.shown || self.tab != Tab::Processes {
            return;
        }
        self.process_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let job = this
                    .update(cx, |this, cx| {
                        if !this.shown || this.tab != Tab::Processes {
                            return None;
                        }
                        let processes = this.processes.as_mut()?;
                        Some((this.device_manager.read(cx).adb()?, processes.serial.clone(), std::mem::take(&mut processes.known)))
                    })
                    .ok()
                    .flatten();
                let Some((adb, serial, mut known)) = job else { break };
                let (result, known) = cx
                    .background_spawn(async move {
                        let result = jdwp::list(&adb, &serial, &mut known);
                        (result, known)
                    })
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        if let Some(processes) = this.processes.as_mut() {
                            processes.known = known;
                            processes.loaded = true;
                            match result {
                                Ok(list) => {
                                    processes.error = None;
                                    processes.selected.retain(|pid| list.iter().any(|p| p.pid == *pid));
                                    if processes.list != list {
                                        processes.list = list;
                                    }
                                }
                                Err(e) => processes.error = Some(e.to_string()),
                            }
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(PROCESS_POLL).await;
            }
            this.update(cx, |this, _| this.process_poll = None).ok();
        }));
    }

    fn refresh_processes(&mut self, cx: &mut Context<Self>) {
        if let Some(processes) = self.processes.as_mut() {
            processes.known.clear();
        }
        self.process_poll = None;
        self.poll_processes(cx);
    }

    fn visible_processes(&self) -> Vec<Process> {
        let Some(processes) = self.processes.as_ref() else { return Vec::new() };
        let filtering = self.filtering();
        processes.list.iter().filter(|p| !filtering || p.package_name().is_some_and(|n| self.app_ids.iter().any(|id| id == n))).cloned().collect()
    }

    fn select_process(&mut self, pid: u32, modifiers: Modifiers, cx: &mut Context<Self>) {
        let visible = self.visible_processes();
        let Some(processes) = self.processes.as_mut() else { return };
        if modifiers.shift {
            let anchor = processes.anchor.unwrap_or(pid);
            let (Some(a), Some(b)) = (visible.iter().position(|p| p.pid == anchor), visible.iter().position(|p| p.pid == pid)) else { return };
            processes.selected = visible[a.min(b)..=a.max(b)].iter().map(|p| p.pid).collect();
        } else if modifiers.secondary() {
            if let Some(ix) = processes.selected.iter().position(|p| *p == pid) {
                processes.selected.remove(ix);
            } else {
                processes.selected.push(pid);
            }
            processes.anchor = Some(pid);
        } else {
            processes.selected = vec![pid];
            processes.anchor = Some(pid);
        }
        cx.notify();
    }

    fn selected_processes(&self) -> Vec<Process> {
        self.processes.as_ref().map(Processes::selected_processes).unwrap_or_default()
    }

    /// Runs an action on each selected process in the background, then refreshes.
    fn process_action(&mut self, what: &'static str, run: fn(&crate::android::adb::Adb, &str, &Process) -> Result<(), String>, cx: &mut Context<Self>) {
        let selected = self.selected_processes();
        let Some(adb) = self.adb(cx) else { return };
        let Some(serial) = self.processes.as_ref().map(|p| p.serial.clone()) else { return };
        if selected.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let errors: Vec<String> = cx.background_spawn(async move { selected.iter().filter_map(|p| run(&adb, &serial, p).err()).collect() }).await;
            this.update(cx, |this, cx| {
                for error in errors {
                    this.notify(&format!("Unable to {what}"), error, true, cx);
                }
                this.refresh_processes(cx);
            })
            .ok();
        })
        .detach();
    }

    fn kill_processes(&mut self, cx: &mut Context<Self>) {
        self.process_action("kill process", |adb, serial, p| jdwp::kill(adb, serial, p.pid).map_err(|e| e.to_string()), cx);
    }

    fn force_stop(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "force stop",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                adb.shell(serial, &format!("am force-stop {package}")).map(|_| ()).map_err(|e| e.to_string())
            },
            cx,
        );
    }

    fn clear_app_data(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "clear app data",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                let out = adb.shell(serial, &format!("pm clear {package}")).map_err(|e| e.to_string())?;
                if out.contains("Success") { Ok(()) } else { Err("Failed to clear app data.".into()) }
            },
            cx,
        );
    }

    fn uninstall_app(&mut self, cx: &mut Context<Self>) {
        self.process_action(
            "uninstall app",
            |adb, serial, p| {
                let package = p.package_name().ok_or("Couldn't find package name for process.")?;
                let out = adb.run(&["-s", serial, "uninstall", package], Duration::from_secs(60)).map_err(|e| e.to_string())?;
                if out.stdout.contains("Success") { Ok(()) } else { Err("Failed to uninstall app.".into()) }
            },
            cx,
        );
    }

    fn process_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let visible = self.visible_processes();
        let handled = match k.key.as_str() {
            "up" | "down" if !m.alt && !m.control && !m.platform => {
                let current = self.processes.as_ref().and_then(|p| p.selected.last().copied()).and_then(|pid| visible.iter().position(|p| p.pid == pid));
                let next = match current {
                    Some(ix) if k.key == "down" => (ix + 1 < visible.len()).then_some(ix + 1),
                    Some(ix) => ix.checked_sub(1),
                    None => (!visible.is_empty()).then_some(0),
                };
                if let Some(ix) = next {
                    self.select_process(visible[ix].pid, Modifiers { shift: m.shift, ..Default::default() }, cx);
                    self.process_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
                true
            }
            "c" if (m.control || m.platform) && !m.shift => {
                // Copy: "name: pid" per selected process, as Android Studio.
                let text: String = self.selected_processes().iter().map(|p| format!("{}: {}\n", p.display_name(), p.pid)).collect();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn process_menu(this: Entity<Self>, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let selected = this.read(cx).selected_processes();
        if selected.is_empty() {
            return menu;
        }
        let many = selected.len() > 1;
        let known = selected.iter().any(|p| !p.pid_only());
        let packages = selected.iter().all(|p| p.package_name().is_some());
        let act = |f: fn(&mut Self, &mut Context<Self>)| {
            let this = this.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| this.update(cx, |this, cx| f(this, cx))
        };
        let mut menu = menu;
        if known {
            menu = menu
                .item(PopupMenuItem::new(if many { "Kill processes" } else { "Kill process" }).icon(Icon::new(icons::KILL_PROCESS)).on_click(act(Self::kill_processes)))
                .item(PopupMenuItem::new(if many { "Force stop processes" } else { "Force stop process" }).icon(Icon::new(icons::STOP)).on_click(act(Self::force_stop)));
        }
        if packages {
            menu = menu
                .item(PopupMenuItem::new(if many { "Clear apps data" } else { "Clear app data" }).icon(Icon::new(icons::CLEAR)).on_click(act(Self::clear_app_data)))
                .item(PopupMenuItem::new(if many { "Uninstall apps" } else { "Uninstall app" }).icon(Icon::new(icons::DELETE)).on_click(act(Self::uninstall_app)));
        }
        menu
    }

    // --------------------------------------------------------------- Render

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (sync, filter) = (cx.entity(), cx.entity());
        let filtering = self.package_filter;
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Device Explorer"))
            .child(div().flex_1())
            .child(tool_button("de-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let (sync, filter) = (sync.clone(), filter.clone());
                menu.item(PopupMenuItem::new("Synchronize").on_click(move |_, _, cx| sync.update(cx, |this, cx| this.synchronize(cx))))
                    .item(PopupMenuItem::new("Package Filter").checked(filtering).on_click(move |_, _, cx| filter.update(cx, |this, cx| this.toggle_package_filter(cx))))
            }))
            .child(tool_button("de-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(DeviceExplorerEvent::Hide))))
            .border_b_1()
            .border_color(palette.border)
    }

    fn render_device_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let active = self.active.as_ref().and_then(|s| self.devices.iter().find(|d| &d.serial == s)).cloned();
        let devices = self.devices.clone();
        let entity = cx.entity();
        let grey = if palette.dark { rgb(0x6f737a) } else { rgb(0x818594) };
        let label: gpui_kit::AnyElement = match &active {
            Some(device) => h_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .child(Icon::new(device_icon(device.kind)).small())
                .child(div().whitespace_nowrap().child(device.name.clone()))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(grey).child(device.version.clone()))
                .into_any_element(),
            None => div().flex_1().text_color(palette.status_deleted).child("No Connected Devices").into_any_element(),
        };
        h_flex().px_2().py_1().flex_shrink_0().child(
            Button::new("de-device")
                .outline()
                .small()
                .w_full()
                .child(h_flex().w_full().gap_1().text_sm().child(label).child(Icon::new(icons::CHEVRON_DOWN).xsmall().text_color(palette.text_secondary)))
                .dropdown_menu(move |mut menu, _, _| {
                    for device in &devices {
                        let (entity, serial) = (entity.clone(), device.serial.clone());
                        let (name, version) = (device.name.clone(), device.version.clone());
                        menu = menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                let palette = cx.palette().clone();
                                let grey = if palette.dark { rgb(0x6f737a) } else { rgb(0x818594) };
                                h_flex().gap_1p5().child(div().child(name.clone())).child(div().text_color(grey).child(version.clone()))
                            })
                            .icon(Icon::new(device_icon(device.kind)))
                            .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.show_device(&serial, cx))),
                        );
                    }
                    menu
                }),
        )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let current = self.tab;
        let tab = |id: &'static str, label: &'static str, value: Tab| {
            div()
                .id(id)
                .px_2()
                .h_full()
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(value == current, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                .when(value != current, |el| el.text_color(palette.text_secondary))
                .child(label)
        };
        let select = |value: Tab| {
            move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.tab = value;
                this.poll_processes(cx);
                cx.notify();
            }
        };
        h_flex()
            .h(px(28.))
            .px_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(tab("de-tab-files", "Files", Tab::Files).on_click(cx.listener(select(Tab::Files))))
            .child(tab("de-tab-processes", "Processes", Tab::Processes).on_click(cx.listener(select(Tab::Processes))))
    }

    fn filter_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        let enabled = !self.app_ids.is_empty();
        let text = format!("Turn {} package filter", if self.package_filter { "off" } else { "on" });
        let tooltip = if enabled { text } else { format!("{text}\nDisabled due to no application IDs found") };
        tool_button(id, icons::LOCATE, tooltip)
            .selected(self.package_filter && enabled)
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_package_filter(cx)))
    }

    fn render_files_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let files = self.files.as_ref();
        let ready = files.is_some_and(|f| f.fs.is_some());
        let selected = files.map(|f| f.selected.clone()).unwrap_or_default();
        let dir = files.and_then(Files::selected_dir);
        let (new_dir, upload_dir) = (dir.clone(), dir.clone());
        let (save, delete) = (selected.clone(), selected.clone());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(
                tool_button("de-new-dir", icons::NEW_FOLDER, "New Directory")
                    .disabled(dir.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(dir) = new_dir.clone() {
                            this.new_entry(dir, true, "NewFolder".into(), window, cx);
                        }
                    })),
            )
            .child(
                tool_button("de-save", icons::DOWNLOAD, "Save As... (Ctrl+Shift+S)")
                    .disabled(selected.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| this.save_as(save.clone(), cx))),
            )
            .child(
                tool_button("de-upload", icons::UPLOAD, "Upload... (Ctrl+Shift+O)")
                    .disabled(dir.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(dir) = upload_dir.clone() {
                            this.upload(dir, window, cx);
                        }
                    })),
            )
            .child(
                tool_button("de-delete", icons::DELETE, "Delete... (Delete)")
                    .disabled(selected.is_empty())
                    .on_click(cx.listener(move |this, _, window, cx| this.delete(delete.clone(), window, cx))),
            )
            .child(tool_button("de-sync", icons::STUDIO_REFRESH, "Synchronize (Ctrl+F5)").disabled(!ready).on_click(cx.listener(|this, _, _, cx| this.synchronize(cx))))
            .child(self.filter_button("de-files-filter", cx))
    }

    fn render_file_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let Some(files) = self.files.as_ref() else { return Vec::new() };
        let grey = palette.text_secondary;
        range
            .filter_map(|ix| {
                let row = files.rows.get(ix)?;
                let indent = |depth: usize| px(4. + depth as f32 * 16.);
                Some(match row {
                    Row::Loading { depth } => h_flex()
                        .h(px(ROW_H))
                        .pl(indent(*depth))
                        .gap_1()
                        .text_sm()
                        .text_color(grey)
                        .child(div().w(px(16.)))
                        .child(div().italic().child("loading..."))
                        .into_any_element(),
                    Row::Error { depth, text } => h_flex()
                        .h(px(ROW_H))
                        .pl(indent(*depth))
                        .gap_1()
                        .text_sm()
                        .text_color(palette.status_deleted)
                        .child(div().w(px(16.)))
                        .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text.clone()))
                        .into_any_element(),
                    Row::Entry { path, depth } => {
                        let entry = files.entries.get(path)?;
                        let is_dir = files.is_dir(path);
                        let expanded = files.expanded.contains(path);
                        let selected = files.selected.contains(path);
                        let base: Icon = if is_dir {
                            icons::FOLDER.into()
                        } else if matches!(entry.kind, EntryKind::File | EntryKind::Symlink) {
                            crate::ui::common::known_file_icon(&entry.name).unwrap_or_else(|| icons::FILE_ANY.into())
                        } else {
                            icons::FILE_ANY.into()
                        };
                        let icon = div()
                            .relative()
                            .size(px(16.))
                            .flex_shrink_0()
                            .child(base.small())
                            .when(entry.kind == EntryKind::Symlink, |el| el.child(div().absolute().top_0().left_0().child(Icon::new(icons::SYMLINK).small())));
                        let chevron = div().w(px(16.)).flex_shrink_0().when(is_dir, |el| {
                            el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall().text_color(grey))
                        });
                        let (click_path, chevron_path, menu_path, right_path, drop_path) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
                        let menu_entity = entity.clone();
                        let tooltip = entry.link_target.clone().map(|t| SharedString::from(format!("Link target: {t}")));
                        let cell = |text: String| div().px_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
                        h_flex()
                            .id(SharedString::from(format!("de-row-{path}")))
                            .h(px(ROW_H))
                            .w_full()
                            .text_sm()
                            .when(selected, |el| el.bg(palette.selection))
                            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| this.click_row(&click_path, e, window, cx)))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus, cx);
                                    if !this.files.as_ref().is_some_and(|f| f.selected.contains(&right_path)) {
                                        this.select_file(&right_path, Modifiers::default(), cx);
                                    }
                                }),
                            )
                            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                                cx.stop_propagation();
                                this.drop_paths(Some(drop_path.clone()), paths, cx)
                            }))
                            .context_menu(move |menu, window, cx| {
                                let _ = &menu_path;
                                Self::file_menu(menu_entity.clone(), menu, window, cx)
                            })
                            .child(
                                h_flex()
                                    .id(SharedString::from(format!("de-name-{path}")))
                                    .flex_1()
                                    .min_w(px(NAME_MIN))
                                    .pl(indent(*depth))
                                    .gap_1()
                                    .child(div().id(SharedString::from(format!("de-chevron-{path}"))).child(chevron).on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_expanded(&chevron_path, cx);
                                        },
                                    )))
                                    .child(icon)
                                    .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(entry.name.clone()))
                                    .when_some(tooltip, |el, t| el.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(t.clone()).build(window, cx))),
                            )
                            .child(cell(entry.permissions.clone()).w(px(PERM_W)).flex_shrink_0())
                            .child(cell(entry.date.clone()).w(px(DATE_W)).flex_shrink_0())
                            .child(
                                cell(entry.size.map(files::humanized_size).unwrap_or_default())
                                    .w(px(SIZE_W))
                                    .flex_shrink_0()
                                    .flex()
                                    .justify_end(),
                            )
                            .into_any_element()
                    }
                })
            })
            .collect()
    }

    fn render_files(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let Some(files) = self.files.as_ref() else { return div().flex_1().into_any_element() };
        let header = h_flex()
            .h(px(24.))
            .flex_shrink_0()
            .text_sm()
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w(px(NAME_MIN)).px_1().child("Name"))
            .child(div().w(px(PERM_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Permissions"))
            .child(div().w(px(DATE_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Date"))
            .child(div().w(px(SIZE_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Size"));
        let body: gpui_kit::AnyElement = if let Some(error) = &files.error {
            message_layer(error.clone(), palette.status_deleted).into_any_element()
        } else if files.fs.is_none() {
            message_layer("Loading...".into(), palette.text_secondary).into_any_element()
        } else {
            uniform_list("de-files", files.rows.len(), cx.processor(Self::render_file_rows)).track_scroll(&self.files_scroll).flex_1().min_h_0().into_any_element()
        };
        v_flex()
            .id("de-files-tab")
            .flex_1()
            .min_h_0()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.drop_paths(None, paths, cx)))
            .child(self.render_files_toolbar(cx))
            .child(table("de-files-scroll", NAME_MIN + PERM_W + DATE_W + SIZE_W, header, body))
            .into_any_element()
    }

    fn render_processes_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let selected = self.selected_processes();
        let many = selected.len() > 1;
        let known = selected.iter().any(|p| !p.pid_only());
        let packages = !selected.is_empty() && selected.iter().all(|p| p.package_name().is_some());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(
                tool_button("de-kill", icons::KILL_PROCESS, if many { "Kill processes\nTerminates the target VM" } else { "Kill process\nTerminates the target VM" })
                    .disabled(!known)
                    .on_click(cx.listener(|this, _, _, cx| this.kill_processes(cx))),
            )
            .child(
                tool_button(
                    "de-force-stop",
                    icons::STOP,
                    if many { "Force stop processes\nExecutes command: am force-stop" } else { "Force stop process\nExecutes command: am force-stop" },
                )
                .disabled(!known)
                .on_click(cx.listener(|this, _, _, cx| this.force_stop(cx))),
            )
            .child(
                tool_button("de-clear", icons::CLEAR, if many { "Clear apps data\nClears application data" } else { "Clear app data\nClears application data" })
                    .disabled(!packages)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_app_data(cx))),
            )
            .child(
                tool_button("de-uninstall", icons::DELETE, if many { "Uninstall apps\nUninstalls application" } else { "Uninstall app\nUninstalls application" })
                    .disabled(!packages)
                    .on_click(cx.listener(|this, _, _, cx| this.uninstall_app(cx))),
            )
            .child(tool_button("de-refresh", icons::STUDIO_REFRESH, "Refresh (Ctrl+F5)").on_click(cx.listener(|this, _, _, cx| this.refresh_processes(cx))))
            .child(self.filter_button("de-process-filter", cx))
    }

    fn render_process_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let visible = self.visible_processes();
        let selected: Vec<u32> = self.processes.as_ref().map(|p| p.selected.clone()).unwrap_or_default();
        range
            .filter_map(|ix| {
                let process = visible.get(ix)?.clone();
                let is_selected = selected.contains(&process.pid);
                let pid = process.pid;
                let dash = |v: Option<String>| if process.pid_only() { "-".to_owned() } else { v.unwrap_or_else(|| "-".into()) };
                let debugger = if process.pid_only() { "-" } else if process.waiting { "Waiting" } else { "No" };
                let user = if process.pid_only() { "-".to_owned() } else { process.user_id.map(|u| u.to_string()).unwrap_or_else(|| "n/a".into()) };
                let cell = |text: String| div().px_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
                let menu_entity = entity.clone();
                Some(
                    h_flex()
                        .id(("de-process", pid as usize))
                        .h(px(ROW_H))
                        .w_full()
                        .text_sm()
                        .when(is_selected, |el| el.bg(palette.selection))
                        .when(!is_selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.select_process(pid, e.modifiers(), cx)
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                window.focus(&this.focus, cx);
                                if !this.processes.as_ref().is_some_and(|p| p.selected.contains(&pid)) {
                                    this.select_process(pid, Modifiers::default(), cx);
                                }
                            }),
                        )
                        .context_menu(move |menu, _, cx| Self::process_menu(menu_entity.clone(), menu, cx))
                        .child(cell(process.display_name()).flex_1().min_w(px(NAME_MIN)))
                        .child(cell(pid.to_string()).w(px(PID_W)).flex_shrink_0())
                        .child(cell(dash(process.abi.clone())).w(px(ABI_W)).flex_shrink_0())
                        .child(cell(dash(process.vm.clone())).w(px(VM_W)).flex_shrink_0())
                        .child(cell(user).w(px(USER_W)).flex_shrink_0())
                        .child(cell(debugger.into()).w(px(DEBUGGER_W)).flex_shrink_0())
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_processes(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let column = |label: &'static str| div().px_1().border_l_1().border_color(palette.border).whitespace_nowrap().overflow_hidden().child(label);
        let header = h_flex()
            .h(px(24.))
            .flex_shrink_0()
            .text_sm()
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w(px(NAME_MIN)).px_1().child("Process Name"))
            .child(column("PID").w(px(PID_W)).flex_shrink_0())
            .child(column("ABI").w(px(ABI_W)).flex_shrink_0())
            .child(column("VM").w(px(VM_W)).flex_shrink_0())
            .child(column("User ID").w(px(USER_W)).flex_shrink_0())
            .child(column("Debugger").w(px(DEBUGGER_W)).flex_shrink_0());
        let processes = self.processes.as_ref();
        let count = self.visible_processes().len();
        let body: gpui_kit::AnyElement = match processes {
            Some(p) if !p.loaded => message_layer("Loading...".into(), palette.text_secondary).into_any_element(),
            Some(Processes { error: Some(error), .. }) if count == 0 => message_layer(error.clone(), palette.status_deleted).into_any_element(),
            _ if count == 0 => message_layer("No debuggable process on device".into(), palette.text_secondary).into_any_element(),
            _ => uniform_list("de-processes", count, cx.processor(Self::render_process_rows)).track_scroll(&self.process_scroll).flex_1().min_h_0().into_any_element(),
        };
        v_flex()
            .id("de-processes-tab")
            .flex_1()
            .min_h_0()
            .child(self.render_processes_toolbar(cx))
            .child(table("de-processes-scroll", NAME_MIN + PID_W + ABI_W + VM_W + USER_W + DEBUGGER_W, header, body))
            .into_any_element()
    }

    fn render_busy(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let text = self.busy.clone()?;
        let palette = cx.palette().clone();
        Some(
            h_flex()
                .h(px(26.))
                .px_2()
                .gap_2()
                .flex_shrink_0()
                .border_t_1()
                .border_color(palette.border)
                .text_sm()
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small().color(palette.text_secondary))
                .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text)),
        )
    }
}

/// The Name column's narrowest; a narrower window scrolls the table sideways.
const NAME_MIN: f32 = 170.;
const PERM_W: f32 = 80.;
const DATE_W: f32 = 108.;
const SIZE_W: f32 = 56.;
const PID_W: f32 = 56.;
const ABI_W: f32 = 64.;
const VM_W: f32 = 96.;
const USER_W: f32 = 56.;
const DEBUGGER_W: f32 = 70.;

fn device_icon(kind: Kind) -> crate::ui::as_icons::AsIcon {
    match kind {
        Kind::Virtual => icons::VIRTUAL_DEVICE_PHONE,
        Kind::Physical => icons::PHYSICAL_DEVICE_PHONE,
    }
}

/// A device Device Explorer can open: online, with a serial.
fn target(device: &Device) -> Option<Target> {
    if !device.online {
        return None;
    }
    let version = match (device.api, &device.codename) {
        (Some(api), _) => crate::android::release_name(api),
        (None, Some(codename)) => format!("Android {codename} Preview"),
        _ => String::new(),
    };
    Some(Target { serial: device.serial.clone()?, name: device.name.clone(), version, kind: device.kind })
}

/// A table that keeps its columns readable: below `min_width` it scrolls sideways.
fn table(id: &'static str, min_width: f32, header: impl IntoElement, body: gpui_kit::AnyElement) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_x_scroll()
        .child(v_flex().size_full().min_w(px(min_width)).child(header).child(body))
}

fn message_layer(text: String, color: gpui_kit::Hsla) -> impl IntoElement {
    v_flex().flex_1().items_center().pt(px(60.)).px_4().child(div().text_sm().text_center().text_color(color).child(text))
}

/// An error message with OK, as IntelliJ's `Messages.showMessageDialog`.
fn message_dialog(title: &'static str, message: String, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title)
            .w(px(460.))
            .child(div().text_sm().whitespace_normal().children(message.lines().map(|l| div().child(l.to_owned())).collect::<Vec<_>>()))
            .footer(DialogFooter::new().child(Button::new("de-ok").label("OK").primary().on_click(|_, window, cx| window.close_dialog(cx))))
    });
}

impl Render for DeviceExplorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let content: gpui_kit::AnyElement = if self.devices.is_empty() || self.active.is_none() {
            v_flex()
                .flex_1()
                .child(message_layer("Connect a device via USB cable or run an Android Virtual Device".into(), palette.text_secondary))
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_tabs(cx))
                .child(match self.tab {
                    Tab::Files => self.render_files(cx),
                    Tab::Processes => self.render_processes(cx),
                })
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(palette.panel)
            .key_context("DeviceExplorer")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| match this.tab {
                Tab::Files => this.file_key(event, window, cx),
                Tab::Processes => this.process_key(event, window, cx),
            }))
            .child(self.render_header(cx))
            .child(self.render_device_picker(cx))
            .child(content)
            .children(self.render_busy(cx))
    }
}

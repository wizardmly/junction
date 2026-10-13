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

mod chrome;
mod files_tab;
mod processes_tab;

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

    // ------------------------------------------------------------ Processes

    // --------------------------------------------------------------- Render
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

//! The Device Manager tool window, as Android Studio's: every AVD and every
//! device adb sees in one table (status, Name with the Android version
//! below it, API, Type), sorted by a column header click and grouped by
//! Form Factor, Status or Type; ▶ / ■ start and stop an emulator; the ⋮
//! menu cold boots, wipes, deletes, shows details or the AVD folder; the
//! toolbar pairs devices over Wi-Fi.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement as _,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, prelude::FluentBuilder as _, px, rgb,
};

use crate::android::{self, Device, Kind, ScanCache, Tools, avd};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;

mod actions;
mod table;

/// How often the list follows adb and the AVD folder while shown.
const POLL: Duration = Duration::from_secs(2);

pub enum DeviceManagerEvent {
    Hide,
    /// Start / Stop Mirroring a device in Running Devices.
    Mirror { serial: String, title: String, start: bool },
    /// Open in Device Explorer.
    Explore(String),
    Notify { title: String, message: String, error: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortColumn {
    Status,
    Name,
    Api,
    Type,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Grouping {
    None,
    FormFactor,
    Status,
    Type,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transition {
    Starting,
    Stopping,
    /// `adb connect` to a device available for Wi-Fi pairing.
    Connecting,
}

impl Transition {
    fn label(self) -> &'static str {
        match self {
            Transition::Starting => "Starting up",
            Transition::Stopping => "Shutting down",
            Transition::Connecting => "Connecting",
        }
    }
}

/// View Details: the panel under the table.
struct Details {
    id: String,
    summary: Option<Vec<(&'static str, String)>>,
    _load: Task<()>,
}

pub struct DeviceManager {
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    tools: Tools,
    devices: Vec<Device>,
    loaded: bool,
    adb_error: Option<String>,
    cache: Arc<Mutex<ScanCache>>,
    shown: bool,
    poll: Option<Task<()>>,
    /// Asks the running scan loop to scan again right away.
    rescan: bool,
    transitions: HashMap<String, (Transition, Instant)>,
    selected: Option<String>,
    sort: (SortColumn, bool),
    grouping: Grouping,
    collapsed: HashSet<&'static str>,
    details: Option<Details>,
    /// The serials Running Devices mirrors.
    mirrored: HashSet<String>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<DeviceManagerEvent> for DeviceManager {}

impl DeviceManager {
    pub fn new(project: Rc<dyn Fn(&App) -> Option<PathBuf>>) -> Self {
        Self {
            project,
            tools: Tools::default(),
            devices: Vec::new(),
            loaded: false,
            adb_error: None,
            cache: Arc::default(),
            shown: false,
            poll: None,
            rescan: false,
            transitions: HashMap::new(),
            selected: None,
            sort: (SortColumn::Name, true),
            grouping: Grouping::None,
            collapsed: HashSet::new(),
            details: None,
            mirrored: HashSet::new(),
            _tasks: Vec::new(),
        }
    }

    /// The tool window was shown or hidden: the list follows adb only
    /// while it shows (the first time starts adb's server, as Android
    /// Studio does when it opens).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown && self.poll.is_none() {
            self.poll = Some(cx.spawn(async move |this, cx| {
                loop {
                    let Ok(Some((project, cache))) = this.update(cx, |this, cx| {
                        this.rescan = false;
                        this.shown.then(|| ((this.project)(cx), this.cache.clone()))
                    }) else {
                        break;
                    };
                    let (tools, devices, error) = cx
                        .background_spawn(async move {
                            let tools = Tools::locate(project.as_deref());
                            let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
                            let (devices, error) = android::scan(&tools, &mut cache);
                            (tools, devices, error)
                        })
                        .await;
                    if this.update(cx, |this, cx| this.apply_scan(tools, devices, error, cx)).is_err() {
                        break;
                    }
                    let start = Instant::now();
                    while start.elapsed() < POLL {
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        if this.read_with(cx, |this, _| this.rescan || !this.shown).unwrap_or(true) {
                            break;
                        }
                    }
                }
                this.update(cx, |this, _| this.poll = None).ok();
            }));
        }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn adb(&self) -> Option<android::adb::Adb> {
        self.tools.adb.clone()
    }

    pub fn set_mirrored(&mut self, mirrored: HashSet<String>, cx: &mut Context<Self>) {
        if self.mirrored != mirrored {
            self.mirrored = mirrored;
            cx.notify();
        }
    }

    /// Starts an AVD (Running Devices' + menu).
    pub fn launch(&mut self, id: &str, cx: &mut Context<Self>) {
        self.start(id, false, cx);
    }

    pub fn open_wifi_pairing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pair_over_wifi(window, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rescan = true;
        self.set_shown(self.shown, cx);
    }

    fn apply_scan(&mut self, tools: Tools, devices: Vec<Device>, error: Option<String>, cx: &mut Context<Self>) {
        self.tools = tools;
        self.adb_error = error;
        self.loaded = true;
        // A start ends once the device is up, a stop once adb lets it go.
        self.transitions.retain(|id, (transition, since)| {
            let device = devices.iter().find(|d| &d.id == id);
            match transition {
                Transition::Starting => device.is_some_and(|d| !d.online) && since.elapsed() < Duration::from_secs(600),
                Transition::Stopping => device.is_some_and(|d| d.serial.is_some()) && since.elapsed() < Duration::from_secs(60),
                Transition::Connecting => device.is_some() && since.elapsed() < Duration::from_secs(60),
            }
        });
        if self.selected.as_ref().is_some_and(|id| !devices.iter().any(|d| &d.id == id)) {
            self.selected = None;
        }
        if self.details.as_ref().is_some_and(|d| !devices.iter().any(|x| x.id == d.id)) {
            self.details = None;
        }
        if self.devices != devices {
            self.devices = devices;
        }
        cx.notify();
    }

    fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    fn notify_error(&self, title: impl Into<String>, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(DeviceManagerEvent::Notify { title: title.into(), message: message.into(), error: true });
    }

    /// A Wi-Fi pairing finished: the balloon, and the new device in the list.
    pub(crate) fn paired(&mut self, name: String, cx: &mut Context<Self>) {
        cx.emit(DeviceManagerEvent::Notify { title: format!("{name} connected over Wi-Fi"), message: "The device is now available to use.".into(), error: false });
        self.refresh(cx);
    }
}

const STATUS_W: f32 = 24.;
const API_W: f32 = 65.;
const TYPE_W: f32 = 80.;
const ACTIONS_W: f32 = 58.;

impl Render for DeviceManager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let body: gpui_kit::AnyElement = if self.devices.is_empty() {
            self.render_empty(cx).into_any_element()
        } else {
            div().id("dm-rows").flex_1().min_h_0().overflow_y_scrollbar().child(self.render_rows(cx)).into_any_element()
        };
        v_flex()
            .size_full()
            .bg(palette.panel)
            .child(self.render_header(cx))
            .child(self.render_toolbar(cx))
            .children(self.render_banner(cx))
            .child(self.render_column_header(cx))
            .child(body)
            .children(self.render_details(cx))
    }
}

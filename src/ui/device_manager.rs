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
}

impl Transition {
    fn label(self) -> &'static str {
        match self {
            Transition::Starting => "Starting up",
            Transition::Stopping => "Shutting down",
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

    /// ▶ Start (Quick Boot), or Cold Boot.
    fn start(&mut self, id: &str, cold_boot: bool, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let Some(avd) = device.avd.clone() else { return };
        self.transitions.insert(device.id.clone(), (Transition::Starting, Instant::now()));
        cx.notify();
        let tools = self.tools.clone();
        let id = device.id.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let launched = cx.background_spawn({
                let name = avd.name.clone();
                async move { android::launch(&tools, &name, cold_boot) }
            });
            let (child, log) = match launched.await {
                Ok(started) => started,
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.transitions.remove(&id);
                        this.notify_error(format!("Failed to start {}", device.name), e.to_string(), cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| this.refresh(cx)).ok();
            // The emulator runs until closed; a thread of its own waits for it.
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let mut child = child;
                let _ = tx.send(child.wait().map(|s| s.success()).unwrap_or(false));
            });
            let success = rx.await.unwrap_or(true);
            this.update(cx, |this, cx| {
                let stopping = this.transitions.get(&id).is_some_and(|(t, _)| *t == Transition::Stopping);
                this.transitions.remove(&id);
                if !success && !stopping {
                    let reason = android::launch_error(&log);
                    this.notify_error(format!("Failed to start {}", device.name), if reason.is_empty() { "The emulator exited".to_owned() } else { reason }, cx);
                }
                this.refresh(cx);
            })
            .ok();
        }));
    }

    /// ■ Stop: the emulator console's `kill`.
    fn stop(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Task<()>> {
        let device = self.device(id)?.clone();
        let (adb, serial) = (self.tools.adb.clone()?, device.serial.clone()?);
        self.transitions.insert(device.id.clone(), (Transition::Stopping, Instant::now()));
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { adb.emu_kill(&serial) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.transitions.remove(&device.id);
                    this.notify_error(format!("Failed to stop {}", device.name), e.to_string(), cx);
                }
                this.refresh(cx);
            })
            .ok();
        });
        Some(task)
    }

    /// Wipe Data and Delete stop a running emulator first and wait for it
    /// to go before touching its files.
    fn stop_then(&mut self, id: &str, title: &'static str, action: fn(&avd::Avd) -> std::io::Result<()>, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let Some(avd) = device.avd.clone() else { return };
        let running = device.serial.is_some();
        if running {
            if let Some(task) = self.stop(id, cx) {
                task.detach();
            }
        }
        let id = id.to_owned();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let start = Instant::now();
            while running && start.elapsed() < Duration::from_secs(60) {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let gone = this.read_with(cx, |this, _| this.device(&id).is_none_or(|d| d.serial.is_none())).unwrap_or(true);
                if gone {
                    // The emulator lets go of its files a moment after adb loses it.
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    break;
                }
            }
            let result = cx.background_spawn(async move { action(&avd) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.notify_error(format!("{title} failed for {}", device.name), e.to_string(), cx);
                }
                this.refresh(cx);
            })
            .ok();
        }));
    }

    fn confirm_wipe(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(device) = self.device(id) else { return };
        let suffix = if device.serial.is_some() { " This will stop the device." } else { "" };
        let message = format!("Do you really want to wipe user files from {}?{suffix}", device.name);
        let (entity, id) = (cx.entity(), id.to_owned());
        crate::ui::dialogs::confirm(
            "Confirm Data Wipe",
            message,
            "Yes",
            move |cx| entity.update(cx, |this, cx| this.stop_then(&id, "Wipe Data", avd::wipe_data, cx)),
            window,
            cx,
        );
    }

    fn confirm_delete(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(device) = self.device(id) else { return };
        let suffix = if device.serial.is_some() { " This will stop the device." } else { "" };
        let message = format!("Do you really want to delete {}?{suffix}", device.name);
        let (entity, id) = (cx.entity(), id.to_owned());
        crate::ui::dialogs::confirm(
            "Confirm Deletion",
            message,
            "Yes",
            move |cx| entity.update(cx, |this, cx| this.stop_then(&id, "Delete", avd::delete, cx)),
            window,
            cx,
        );
    }

    fn view_details(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(device) = self.device(id).cloned() else { return };
        let adb = self.tools.adb.clone();
        let load = cx.spawn(async move |this, cx| {
            let summary = cx.background_spawn(async move { android::summary(&device, adb.as_ref()) }).await;
            this.update(cx, |this, cx| {
                if let Some(details) = &mut this.details {
                    details.summary = Some(summary);
                }
                cx.notify();
            })
            .ok();
        });
        self.details = Some(Details { id: id.to_owned(), summary: None, _load: load });
        cx.notify();
    }

    fn pair_over_wifi(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::wifi_pairing::open(cx.entity().downgrade(), self.tools.adb.clone(), window, cx);
    }

    /// A Wi-Fi pairing finished: the balloon, and the new device in the list.
    pub(crate) fn paired(&mut self, name: String, cx: &mut Context<Self>) {
        cx.emit(DeviceManagerEvent::Notify { title: format!("{name} connected over Wi-Fi"), message: "The device is now available to use.".into(), error: false });
        self.refresh(cx);
    }

    /// The ⋮ menu (also the row's context menu).
    fn device_menu(entity: Entity<Self>, device: &Device, transitioning: bool, menu: PopupMenu) -> PopupMenu {
        let id = device.id.clone();
        let action = |f: fn(&mut Self, &str, &mut Window, &mut Context<Self>)| {
            let (entity, id) = (entity.clone(), id.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| entity.update(cx, |this, cx| f(this, &id, window, cx))
        };
        let mut menu = menu;
        // Open in Device Explorer: any device adb can reach.
        if let Some(serial) = device.serial.clone().filter(|_| device.online) {
            let entity = entity.clone();
            menu = menu
                .item(PopupMenuItem::new("Open in Device Explorer").icon(Icon::new(icons::TW_DEVICE_EXPLORER)).on_click(move |_, _, cx| {
                    entity.update(cx, |_, cx| cx.emit(DeviceManagerEvent::Explore(serial.clone())))
                }))
                .separator();
        }
        if let Some(avd) = &device.avd {
            let offline = device.serial.is_none() && !transitioning;
            let dir = avd.dir.clone();
            menu = menu
                .item(PopupMenuItem::new("Cold Boot").disabled(!offline).on_click(action(|this, id, _, cx| this.start(id, true, cx))))
                .separator()
                .item(PopupMenuItem::new("Wipe Data").icon(Icon::new(icons::CLEAR)).disabled(transitioning).on_click(action(Self::confirm_wipe)))
                .item(PopupMenuItem::new("Delete").icon(Icon::new(icons::DELETE)).disabled(transitioning).on_click(action(Self::confirm_delete)))
                .separator()
                .item(PopupMenuItem::new("View Details").on_click(action(|this, id, _, cx| this.view_details(id, cx))))
                .item(PopupMenuItem::new("Show on Disk").icon(Icon::new(icons::SHOW)).on_click(move |_, _, cx| cx.reveal_path(&dir)));
        } else {
            menu = menu.item(PopupMenuItem::new("View Details").on_click(action(|this, id, _, cx| this.view_details(id, cx))));
        }
        menu
    }

    fn group_key(&self, device: &Device) -> &'static str {
        match self.grouping {
            Grouping::None => "",
            Grouping::FormFactor => device.form.label(),
            Grouping::Status => {
                if device.online { "Online" } else { "Offline" }
            }
            Grouping::Type => device.kind.label(),
        }
    }

    fn sorted(&self) -> Vec<&Device> {
        let mut devices: Vec<&Device> = self.devices.iter().collect();
        let (column, ascending) = self.sort;
        devices.sort_by(|a, b| {
            let order = match column {
                SortColumn::Status => b.online.cmp(&a.online),
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Api => a.api.cmp(&b.api).then_with(|| a.codename.cmp(&b.codename)),
                SortColumn::Type => a.kind.label().cmp(b.kind.label()),
            };
            let order = if ascending { order } else { order.reverse() };
            order.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        devices
    }

    fn toggle_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
        cx.notify();
    }

    fn grouping_menu(entity: Entity<Self>, current: Grouping, menu: PopupMenu) -> PopupMenu {
        let mut menu = menu.label("Group By");
        for (label, grouping) in [("None", Grouping::None), ("Form Factor", Grouping::FormFactor), ("Status", Grouping::Status), ("Type", Grouping::Type)] {
            let entity = entity.clone();
            menu = menu.item(PopupMenuItem::new(label).checked(current == grouping).on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.grouping = grouping;
                    this.collapsed.clear();
                    cx.notify();
                })
            }));
        }
        menu
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let grouping = self.grouping;
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Device Manager"))
            .child(div().flex_1())
            .child(tool_button("dm-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let refresh = entity.clone();
                Self::grouping_menu(entity.clone(), grouping, menu)
                    .separator()
                    .item(PopupMenuItem::new("Refresh").on_click(move |_, _, cx| refresh.update(cx, |this, cx| this.refresh(cx))))
            }))
            .child(tool_button("dm-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(DeviceManagerEvent::Hide))))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (group_entity, add_entity) = (cx.entity(), cx.entity());
        let grouping = self.grouping;
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(tool_button("dm-group", icons::GROUP, "Group").dropdown_menu(move |menu, _, _| Self::grouping_menu(group_entity.clone(), grouping, menu)))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
            .child(tool_button("dm-add", icons::ADD, "Add Device").dropdown_menu(move |menu, _, _| {
                let entity = add_entity.clone();
                menu.item(PopupMenuItem::new("Pair Devices Using Wi-Fi").icon(Icon::new(icons::PAIR_OVER_WIFI)).on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.pair_over_wifi(window, cx))
                }))
            }))
            .child(tool_button("dm-pair", icons::PAIR_OVER_WIFI, "Pair Devices Using Wi-Fi").on_click(cx.listener(|this, _, window, cx| this.pair_over_wifi(window, cx))))
    }

    fn render_column_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (sorted_by, ascending) = self.sort;
        let column = |id: &'static str, label: &'static str, sort: SortColumn, cx: &mut Context<Self>| {
            h_flex()
                .id(id)
                .h_full()
                .px_1()
                .gap_1()
                .border_r_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|el| el.bg(palette.hover))
                .when(sorted_by == sort, |el| {
                    el.child(Icon::new(if ascending { icons::CHEVRON_UP } else { icons::CHEVRON_DOWN }).xsmall().text_color(palette.text_secondary))
                })
                .child(div().whitespace_nowrap().overflow_hidden().child(label))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_sort(sort, cx)))
        };
        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .text_sm()
            .border_b_1()
            .border_color(palette.border)
            .child(column("dm-col-status", "", SortColumn::Status, cx).w(px(STATUS_W)).flex_shrink_0())
            .child(column("dm-col-name", "Name", SortColumn::Name, cx).flex_1().min_w_0())
            .child(column("dm-col-api", "API", SortColumn::Api, cx).w(px(API_W)).flex_shrink_0())
            .child(column("dm-col-type", "Type", SortColumn::Type, cx).w(px(TYPE_W)).flex_shrink_0())
            .child(div().w(px(ACTIONS_W)).flex_shrink_0())
    }

    fn render_row(&self, device: &Device, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let transition = self.transitions.get(&device.id).map(|(t, _)| *t);
        let busy = transition.is_some() || device.booting;
        let selected = self.selected.as_deref() == Some(device.id.as_str());
        let line2_color = if selected { palette.text } else if palette.dark { rgb(0x6f737a).into() } else { rgb(0x818594).into() };
        let line2 = transition
            .map(|t| t.label().to_owned())
            .or_else(|| device.booting.then(|| "Booting".to_owned()))
            .or_else(|| device.error.clone())
            .unwrap_or_else(|| device.version_label());
        let status: gpui_kit::AnyElement = if busy {
            Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small().color(palette.text_secondary).into_any_element()
        } else if device.error.is_some() {
            Icon::new(icons::STATUS_WARNING).small().into_any_element()
        } else if device.online {
            Icon::new(icons::AVD_STATUS_ONLINE).small().into_any_element()
        } else {
            div().into_any_element()
        };
        let icon = match (device.kind, device.form) {
            (Kind::Virtual, android::FormFactor::Wear) => icons::VIRTUAL_DEVICE_WEAR,
            (Kind::Virtual, android::FormFactor::Tv) => icons::VIRTUAL_DEVICE_TV,
            (Kind::Virtual, android::FormFactor::Automotive) => icons::VIRTUAL_DEVICE_CAR,
            (Kind::Virtual, android::FormFactor::Desktop) => icons::VIRTUAL_DEVICE_DESKTOP,
            (Kind::Virtual, _) => icons::VIRTUAL_DEVICE_PHONE,
            (Kind::Physical, android::FormFactor::Wear) => icons::PHYSICAL_DEVICE_WEAR,
            (Kind::Physical, android::FormFactor::Tv) => icons::PHYSICAL_DEVICE_TV,
            (Kind::Physical, android::FormFactor::Automotive) => icons::PHYSICAL_DEVICE_CAR,
            (Kind::Physical, _) => icons::PHYSICAL_DEVICE_PHONE,
        };
        let id = device.id.clone();
        let entity = cx.entity();
        let menu_device = device.clone();
        let overflow_device = device.clone();
        let overflow_entity = entity.clone();
        let start_stop = device.avd.is_some().then(|| {
            let running = device.serial.is_some() && transition != Some(Transition::Starting);
            let (icon, tooltip) = if running || transition == Some(Transition::Starting) { (icons::AVD_STOP, "Stop") } else { (icons::AVD_RUN, "Start") };
            let id = id.clone();
            Button::new(SharedString::from(format!("dm-run-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(icon))
                .tooltip(tooltip)
                .disabled(transition.is_some())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if running {
                        if let Some(task) = this.stop(&id, cx) {
                            task.detach();
                        }
                    } else {
                        this.start(&id, false, cx);
                    }
                }))
        });
        // Physical devices: Start / Stop Mirroring, as Android Studio's row.
        let mirroring = (device.kind == Kind::Physical && device.online).then(|| {
            let serial = device.serial.clone().unwrap_or_default();
            let active = self.mirrored.contains(&serial);
            let title = crate::ui::running_devices::title_for(device);
            Button::new(SharedString::from(format!("dm-mirror-{id}")))
                .ghost()
                .xsmall()
                .icon(Icon::new(if active { icons::AVD_STOP_MIRROR } else { icons::AVD_START_MIRROR }))
                .tooltip(if active { "Stop Mirroring" } else { "Start Mirroring" })
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(DeviceManagerEvent::Mirror { serial: serial.clone(), title: title.clone(), start: !active });
                }))
        });
        h_flex()
            .id(SharedString::from(format!("dm-row-{id}")))
            .h(px(if crate::ui::common::compact() { 34. } else { 40. }))
            .flex_shrink_0()
            .text_sm()
            .when(selected, |el| el.bg(palette.selection))
            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _, _, cx| {
                    this.selected = Some(id.clone());
                    cx.notify();
                }
            }))
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener({
                    let id = id.clone();
                    move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }
                }),
            )
            .context_menu(move |menu, _, _| Self::device_menu(entity.clone(), &menu_device, busy, menu))
            .child(h_flex().w(px(STATUS_W)).flex_shrink_0().justify_center().child(status))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .gap_2()
                    .child(Icon::new(icon).small())
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(device.name.clone()))
                            .child(div().text_xs().text_color(line2_color).whitespace_nowrap().overflow_hidden().text_ellipsis().child(line2)),
                    ),
            )
            .child(div().w(px(API_W)).flex_shrink_0().px_1().child(device.api_label()))
            .child(div().w(px(TYPE_W)).flex_shrink_0().px_1().child(device.kind.label()))
            .child(
                h_flex()
                    .w(px(ACTIONS_W))
                    .flex_shrink_0()
                    .justify_end()
                    .pr_1()
                    .children(start_stop)
                    .children(mirroring)
                    .child(
                        Button::new(SharedString::from(format!("dm-more-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(icons::OVERFLOW))
                            .tooltip("More Actions")
                            .dropdown_menu(move |menu, _, _| Self::device_menu(overflow_entity.clone(), &overflow_device, busy, menu)),
                    ),
            )
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let devices = self.sorted();
        let mut rows = v_flex();
        if self.grouping == Grouping::None {
            for device in devices {
                rows = rows.child(self.render_row(device, cx));
            }
            return rows.into_any_element();
        }
        let mut keys: Vec<&'static str> = devices.iter().map(|d| self.group_key(d)).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let collapsed = self.collapsed.contains(key);
            let members: Vec<&Device> = devices.iter().copied().filter(|d| self.group_key(d) == key).collect();
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("dm-group-{key}")))
                    .h(px(26.))
                    .px_1()
                    .gap_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(|el| el.bg(palette.hover))
                    .child(Icon::new(if collapsed { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN }).xsmall().text_color(palette.text_secondary))
                    .child(format!("{key} ({})", members.len()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(key) {
                            this.collapsed.insert(key);
                        }
                        cx.notify();
                    })),
            );
            if !collapsed {
                for device in members {
                    rows = rows.child(self.render_row(device, cx));
                }
            }
        }
        rows.into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .child(div().text_color(palette.text_secondary).child(if self.loaded { "No devices connected." } else { "Loading devices…" }))
            .when(self.loaded, |el| {
                el.child(
                    div()
                        .id("dm-add-new")
                        .text_color(palette.link)
                        .cursor_pointer()
                        .hover(|el| el.underline())
                        .child("Add a new device...")
                        .on_click(cx.listener(|this, _, window, cx| this.pair_over_wifi(window, cx))),
                )
            })
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let palette = cx.palette().clone();
        let message = if !self.loaded {
            return None;
        } else if self.tools.adb.is_none() {
            "Android SDK not found: set ANDROID_HOME to the SDK folder, or install the SDK Platform-Tools.".to_owned()
        } else {
            self.adb_error.clone()?
        };
        let (bg, border) = if palette.dark { (rgb(0x3d3223), rgb(0x5e4d33)) } else { (rgb(0xfff6de), rgb(0xfed277)) };
        Some(
            h_flex()
                .m_1()
                .p_2()
                .gap_2()
                .rounded(px(4.))
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_sm()
                .child(Icon::new(icons::STATUS_WARNING).small())
                .child(div().flex_1().min_w_0().child(message)),
        )
    }

    fn render_details(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let details = self.details.as_ref()?;
        let device = self.device(&details.id)?;
        let palette = cx.palette().clone();
        let mut text = format!("{}\n", device.name);
        let row = |label: &str, value: &str| {
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(150.)).flex_shrink_0().text_color(palette.text_secondary).child(format!("{label}:")))
                .child(div().min_w_0().child(value.to_owned()))
        };
        let mut body = v_flex().p_2().gap_1();
        body = body.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Summary"));
        match &details.summary {
            None => body = body.child(div().text_sm().text_color(palette.text_secondary).child("Loading…")),
            Some(summary) => {
                for (label, value) in summary {
                    text.push_str(&format!("{label}: {value}\n"));
                    body = body.child(row(label, value));
                }
            }
        }
        if let Some(avd) = &device.avd {
            body = body.child(div().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).child("Properties"));
            text.push_str("\nProperties\n");
            for (key, value) in &avd.config {
                text.push_str(&format!("{key}: {value}\n"));
                body = body.child(row(key, value));
            }
        }
        Some(
            // Half the window, as Android Studio's splitter opens it.
            v_flex()
                .flex_1()
                .min_h(px(120.))
                .border_t_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .h(px(crate::ui::common::header_height()))
                        .px_2()
                        .gap_1()
                        .flex_shrink_0()
                        .child(div().flex_1().min_w_0().text_sm().font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().text_ellipsis().child(device.name.clone()))
                        .child(tool_button("dm-details-copy", icons::COPY, "Copy properties to clipboard").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                        }))
                        .child(tool_button("dm-details-close", icons::CLOSE, "Close").on_click(cx.listener(|this, _, _, cx| {
                            this.details = None;
                            cx.notify();
                        }))),
                )
                .child(
                    div()
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .text_sm()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(div().h_full().flex().items_center().border_b_2().border_color(palette.accent).child("Device Info")),
                )
                .child(div().id("dm-details-body").flex_1().min_h_0().overflow_y_scrollbar().child(body)),
        )
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

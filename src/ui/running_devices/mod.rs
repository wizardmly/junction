//! The Running Devices tool window, as Android Studio's: a tab per mirrored
//! device (+ adds one, × stops mirroring), the device toolbar (Power, Volume,
//! Rotate, Back / Home / Overview, Hardware Input, Take Screenshot, Record
//! Screen) and the live screen, scaled to fit or zoomed. Clicks and drags
//! touch the screen, the wheel scrolls, typing goes to the device, and the
//! clipboard is shared both ways.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui_kit::component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Corners, ElementInputHandler, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Image, ImageFormat, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement as _,
    Pixels, Render, RenderImage, ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task,
    UTF16Selection, Window, canvas, div, img, point, prelude::FluentBuilder as _, px, size,
};

use crate::android::adb::Adb;
use crate::android::mirror::{self, Event, Recording, Session};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;

pub(crate) mod capture;
mod input;
mod view;

pub enum RunningDevicesEvent {
    Hide,
    Notify { title: String, message: String, error: bool },
    /// The serials being mirrored, for the Device Manager's buttons.
    Mirroring(HashSet<String>),
    ShowDeviceManager,
}

enum MirrorEvent {
    /// The tab's device went away or failed: the panel shows why.
    Changed,
    Notify { title: String, message: String, error: bool },
}

#[derive(Clone, PartialEq)]
enum State {
    Connecting,
    Live,
    Failed(String),
}

// Android KeyEvent codes.
/// How long a hidden Running Devices window keeps its connections.
const IDLE_DISCONNECT: Duration = Duration::from_secs(10 * 60);

const KEYCODE_HOME: u32 = 3;
const KEYCODE_BACK: u32 = 4;
const KEYCODE_VOLUME_UP: u32 = 24;
const KEYCODE_VOLUME_DOWN: u32 = 25;
const KEYCODE_POWER: u32 = 26;
const KEYCODE_APP_SWITCH: u32 = 187;

/// One mirrored device: its session and screen.
pub struct MirrorView {
    pub serial: String,
    pub title: String,
    adb: Adb,
    session: Option<Session>,
    state: State,
    image: Option<Arc<RenderImage>>,
    /// Images replaced since the last render, to free from the atlas.
    retired: Vec<Arc<RenderImage>>,
    frame_size: (u32, u32),
    /// Device pixels per physical screen pixel; None fits the window.
    zoom: Option<f32>,
    /// The scale the fit came to at the last paint.
    fit: Rc<Cell<f32>>,
    zoom_label_until: Option<Instant>,
    /// Where the screen was painted last, for mouse positions.
    screen: Rc<Cell<Bounds<Pixels>>>,
    pressed: bool,
    hardware_input: bool,
    /// Whether text goes straight into the device's focused field through
    /// the helper (it holds the device's UI automation connection).
    text_helper: bool,
    /// The helper start last notified, so a retry failing the same way
    /// stays quiet.
    helper_notice: Option<Result<(), String>>,
    recording: Option<Recording>,
    /// The clipboard text both sides last agreed on.
    clipboard: Option<String>,
    marked: Option<String>,
    focus: FocusHandle,
    active: bool,
    zoom_controls: bool,
    _events: Option<Task<()>>,
    _clipboard_poll: Task<()>,
    /// When the tool window was hidden, while it is.
    hidden_since: Option<Instant>,
    _idle: Option<Task<()>>,
    /// The last "can't type" reason shown, to show each once.
    typing_notice: Option<String>,
    _intercept: Subscription,
}

impl EventEmitter<MirrorEvent> for MirrorView {}

impl Focusable for MirrorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The file picked in the save dialog; without a file dialog (no portal on
/// Linux) the default folder.
pub(crate) fn chosen_path(answer: Result<anyhow::Result<Option<PathBuf>>, futures::channel::oneshot::Canceled>, name: &str) -> Option<PathBuf> {
    match answer {
        Ok(Ok(path)) => path,
        _ => Some(default_save_dir().join(name)),
    }
}

pub(crate) fn default_save_dir() -> PathBuf {
    let home = crate::android::sdk::home().unwrap_or_else(std::env::temp_dir);
    let desktop = home.join("Desktop");
    if desktop.is_dir() { desktop } else { home }
}

/// The tool window: the tabs and the selected device.
pub struct RunningDevices {
    devices: Entity<DeviceManager>,
    tabs: Vec<Entity<MirrorView>>,
    active: usize,
    show_zoom_controls: bool,
    /// AVDs started from + : mirrored once they are up.
    pending: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RunningDevicesEvent> for RunningDevices {}

impl RunningDevices {
    pub fn new(devices: Entity<DeviceManager>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&devices, |this: &mut Self, devices, cx| {
            // A device that went away (unplugged, emulator stopped) loses its tab.
            let online: HashSet<String> = devices.read(cx).devices().iter().filter(|d| d.online).filter_map(|d| d.serial.clone()).collect();
            let gone: Vec<usize> = this
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| matches!(t.read(cx).state, State::Failed(_)) && !online.contains(&t.read(cx).serial))
                .map(|(ix, _)| ix)
                .collect();
            for ix in gone.into_iter().rev() {
                this.close(ix, cx);
            }
            if this.pending.is_empty() {
                return;
            }
            let ready: Vec<(String, String, String)> = devices
                .read(cx)
                .devices()
                .iter()
                .filter(|d| d.online && this.pending.contains(&d.id))
                .filter_map(|d| Some((d.id.clone(), d.serial.clone()?, tab_title(d))))
                .collect();
            for (id, serial, title) in ready {
                this.pending.remove(&id);
                this.open_tab(&serial, title, cx);
            }
        });
        Self { devices, tabs: Vec::new(), active: 0, show_zoom_controls: true, pending: HashSet::new(), _subscriptions: vec![observe] }
    }

    fn mirrored(&self, cx: &App) -> HashSet<String> {
        self.tabs.iter().map(|t| t.read(cx).serial.clone()).collect()
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        for (ix, tab) in self.tabs.iter().enumerate() {
            tab.update(cx, |t, _| t.active = ix == active);
        }
        cx.emit(RunningDevicesEvent::Mirroring(self.mirrored(cx)));
        cx.notify();
    }

    /// Start Mirroring: a tab for the device (or its existing one).
    pub fn start(&mut self, serial: &str, title: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(serial, title, cx);
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.read(cx).focus.clone(), cx);
        }
    }

    fn open_tab(&mut self, serial: &str, title: String, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.read(cx).serial == serial) {
            self.active = ix;
        } else {
            let Some(adb) = self.devices.read(cx).adb() else {
                cx.emit(RunningDevicesEvent::Notify { title: "Cannot mirror the device".into(), message: "adb was not found".into(), error: true });
                return;
            };
            let view = cx.new(|cx| MirrorView::new(adb, serial.to_owned(), title, cx));
            self._subscriptions.push(cx.subscribe(&view, |_, _, event: &MirrorEvent, cx| match event {
                MirrorEvent::Changed => cx.notify(),
                MirrorEvent::Notify { title, message, error } => {
                    cx.emit(RunningDevicesEvent::Notify { title: title.clone(), message: message.clone(), error: *error })
                }
            }));
            self.tabs.push(view);
            self.active = self.tabs.len() - 1;
        }
        self.changed(cx);
    }

    /// Stop Mirroring: the device's tab closes.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |t, cx| t.set_paused(!shown, cx));
        }
    }

    pub fn stop(&mut self, serial: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.read(cx).serial == serial) {
            self.close(ix, cx);
        }
    }

    fn close(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        tab.update(cx, |t, _| t.stop());
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if self.active > ix {
            self.active -= 1;
        }
        self.changed(cx);
    }

    /// + : the connected devices not mirrored yet, the AVDs to start, and
    /// Wi-Fi pairing, as Android Studio lists them.
    fn add_menu(this: Entity<Self>, menu: PopupMenu, cx: &App) -> PopupMenu {
        let mirrored = this.read(cx).mirrored(cx);
        let manager = this.read(cx).devices.clone();
        let devices = manager.read(cx).devices().to_vec();
        let mut menu = menu;
        let connected: Vec<_> = devices.iter().filter(|d| d.online && d.serial.as_ref().is_some_and(|s| !mirrored.contains(s))).collect();
        if !connected.is_empty() {
            menu = menu.label("Connected Devices");
            for device in connected {
                let (serial, title) = (device.serial.clone().unwrap_or_default(), tab_title(device));
                let this = this.clone();
                menu = menu.item(PopupMenuItem::new(title.clone()).on_click(move |_, window, cx| {
                    this.update(cx, |this, cx| this.start(&serial, title.clone(), window, cx))
                }));
            }
            menu = menu.separator();
        }
        let avds: Vec<_> = devices.iter().filter(|d| d.avd.is_some() && d.serial.is_none()).collect();
        if !avds.is_empty() {
            menu = menu.label("Virtual Devices");
            for device in avds {
                let (id, manager, this) = (device.id.clone(), manager.clone(), this.clone());
                menu = menu.item(PopupMenuItem::new(tab_title(device)).on_click(move |_, _, cx| {
                    this.update(cx, |this, _| this.pending.insert(id.clone()));
                    manager.update(cx, |m, cx| m.launch(&id, cx))
                }));
            }
            menu = menu.separator();
        }
        let pair = manager.clone();
        menu.item(PopupMenuItem::new("Pair Devices Using Wi-Fi").icon(Icon::new(icons::PAIR_OVER_WIFI)).on_click(move |_, window, cx| {
            pair.update(cx, |m, cx| m.open_wifi_pairing(window, cx))
        }))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut tabs = h_flex().gap_1().min_w_0().overflow_x_hidden();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            let view = tab.read(cx);
            let online = view.state != State::Connecting && !matches!(view.state, State::Failed(_));
            tabs = tabs.child(
                h_flex()
                    .id(("rd-tab", ix))
                    .h(px(24.))
                    .pl_2()
                    .pr_0p5()
                    .gap_1()
                    .min_w_0()
                    .rounded(px(4.))
                    .border_1()
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |el| el.bg(palette.selection).border_color(palette.accent.opacity(0.5)))
                    .when(!active, |el| el.border_color(gpui_kit::transparent_black()).hover(|el| el.bg(palette.hover)))
                    .when(online, |el| el.child(Icon::new(icons::AVD_STATUS_ONLINE).xsmall()))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().max_w(px(220.)).child(view.title.clone()))
                    .child(
                        Button::new(("rd-tab-close", ix)).ghost().xsmall().icon(Icon::new(icons::CLOSE_SMALL)).tooltip("Close").on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close(ix, cx)
                        })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = ix;
                        if let Some(tab) = this.tabs.get(ix) {
                            window.focus(&tab.read(cx).focus.clone(), cx);
                        }
                        this.changed(cx);
                    })),
            );
        }
        let entity = cx.entity();
        let options = entity.clone();
        let zoom = self.show_zoom_controls;
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).mr_2().whitespace_nowrap().child("Running Devices"))
            .child(tabs)
            .child(tool_button("rd-add", icons::ADD, "Add Device").dropdown_menu(move |menu, _, cx| Self::add_menu(entity.clone(), menu, cx)))
            .child(div().flex_1())
            .child(tool_button("rd-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let (toggle, close) = (options.clone(), options.clone());
                menu.item(PopupMenuItem::new("Show Zoom Controls").checked(zoom).on_click(move |_, _, cx| {
                    toggle.update(cx, |this, cx| {
                        this.show_zoom_controls = !this.show_zoom_controls;
                        cx.notify();
                    })
                }))
                .separator()
                .item(PopupMenuItem::new("Close Tab").on_click(move |_, _, cx| close.update(cx, |this, cx| this.close(this.active, cx))))
            }))
            .child(tool_button("rd-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(RunningDevicesEvent::Hide))))
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let link = |id: &'static str, label: &'static str| {
            div().id(id).text_color(palette.link).cursor_pointer().hover(|el| el.underline()).child(label)
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .px_6()
            .text_sm()
            .text_color(palette.text_secondary)
            .child(
                div()
                    .text_center()
                    .child("To mirror a physical device, connect it via USB cable or over WiFi, click ＋ and select the device from the list."),
            )
            .child(
                // Words wrap one by one so the link sits inside the sentence.
                h_flex()
                    .flex_wrap()
                    .justify_center()
                    .children(
                        "To launch a virtual device, click ＋ and select the device from the list, or use the"
                            .split(' ')
                            .map(|word| div().mr(px(3.5)).child(word)),
                    )
                    .child(link("rd-open-dm", "Device Manager").on_click(cx.listener(|_, _, _, cx| cx.emit(RunningDevicesEvent::ShowDeviceManager))))
                    .child("."),
            )
    }
}

/// The device's name with its API level, as Android Studio's tabs show it
/// (AVD names usually carry it already).
fn tab_title(device: &crate::android::Device) -> String {
    match device.api {
        Some(api) if !device.name.contains(&format!("API {api}")) => format!("{} API {api}", device.name),
        _ => device.name.clone(),
    }
}

/// The tab title Start Mirroring gives a device.
pub fn title_for(device: &crate::android::Device) -> String {
    tab_title(device)
}

impl Render for RunningDevices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let zoom = self.show_zoom_controls;
        let body: gpui_kit::AnyElement = match self.tabs.get(self.active) {
            Some(tab) => {
                tab.update(cx, |t, cx| {
                    if t.zoom_controls_visible() != zoom {
                        t.set_zoom_controls(zoom);
                        cx.notify();
                    }
                });
                let view = tab.clone();
                let (shot, record) = (view.clone(), view.clone());
                div()
                    .id("rd-body")
                    .flex_1()
                    .min_h_0()
                    .child(view)
                    .context_menu(move |menu, _, _| {
                        let (shot, record) = (shot.clone(), record.clone());
                        menu.item(PopupMenuItem::new("Take Screenshot").icon(Icon::new(icons::EMULATOR_SCREENSHOT)).on_click(move |_, window, cx| {
                            shot.update(cx, |v, cx| v.take_screenshot(window, cx))
                        }))
                        .item(PopupMenuItem::new("Record Screen").icon(Icon::new(icons::EMULATOR_SCREEN_RECORD)).on_click(move |_, _, cx| {
                            record.update(cx, |v, cx| v.toggle_recording(cx))
                        }))
                    })
                    .into_any_element()
            }
            None => self.render_empty(cx).into_any_element(),
        };
        v_flex().size_full().bg(palette.panel).child(self.render_header(cx)).child(body)
    }
}

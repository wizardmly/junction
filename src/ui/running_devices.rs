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
    UTF16Selection, WeakEntity, Window, canvas, div, img, point, prelude::FluentBuilder as _, px, size,
};

use crate::android::adb::Adb;
use crate::android::mirror::{self, Event, Recording, Session};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;

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

/// The keys typed as key events rather than text: GPUI's key name to
/// Android's KeyEvent code.
fn keycode(key: &str) -> Option<u32> {
    let code = match key {
        "enter" => 66,
        "backspace" => 67,
        "delete" => 112,
        "tab" => 61,
        "escape" => 111,
        "up" => 19,
        "down" => 20,
        "left" => 21,
        "right" => 22,
        "home" => 122,
        "end" => 123,
        "pageup" => 92,
        "pagedown" => 93,
        "insert" => 124,
        "space" => 62,
        "," => 55,
        "." => 56,
        "-" => 69,
        "=" => 70,
        "[" => 71,
        "]" => 72,
        "\\" => 73,
        ";" => 74,
        "'" => 75,
        "/" => 76,
        "`" => 68,
        _ => {
            let mut chars = key.chars();
            let (c, rest) = (chars.next()?, chars.next());
            if rest.is_none() && c.is_ascii_lowercase() {
                29 + (c as u32 - 'a' as u32)
            } else if rest.is_none() && c.is_ascii_digit() {
                7 + (c as u32 - '0' as u32)
            } else if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
                130 + n
            } else {
                return None;
            }
        }
    };
    Some(code)
}

/// The keys that are text unless a modifier makes them a shortcut.
fn is_typed(key: &str) -> bool {
    key == "space" || key.chars().count() == 1
}

fn meta_state(m: &gpui_kit::Modifiers) -> u32 {
    let mut meta = 0;
    if m.shift {
        meta |= mirror::META_SHIFT_ON | 0x40;
    }
    if m.alt {
        meta |= mirror::META_ALT_ON | 0x10;
    }
    if m.control {
        meta |= mirror::META_CTRL_ON | 0x2000;
    }
    if m.platform {
        meta |= mirror::META_META_ON | 0x20000;
    }
    meta
}

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

impl MirrorView {
    fn new(adb: Adb, serial: String, title: String, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        // Keys go to the device before the IDE's shortcuts see them, as far
        // as Android Studio lets them (all of them with Hardware Input).
        let weak = cx.entity().downgrade();
        let handle = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if !handle.is_focused(window) {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            if this.update(cx, |this, cx| this.on_keystroke(&event.keystroke, cx)) {
                cx.stop_propagation();
            }
        });
        // The computer's clipboard follows to the device while the tab shows.
        let clipboard_poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                let Ok(active) = this.read_with(cx, |this, _| this.active && this.state == State::Live) else { break };
                if !active {
                    continue;
                }
                let text = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
                if this.update(cx, |this, _| this.sync_clipboard(text)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            serial,
            title,
            adb,
            session: None,
            state: State::Connecting,
            image: None,
            retired: Vec::new(),
            frame_size: (0, 0),
            zoom: None,
            fit: Rc::new(Cell::new(1.)),
            zoom_label_until: None,
            screen: Rc::new(Cell::new(Bounds::default())),
            pressed: false,
            hardware_input: false,
            recording: None,
            clipboard: None,
            marked: None,
            focus,
            active: false,
            zoom_controls: true,
            _events: None,
            _clipboard_poll: clipboard_poll,
            hidden_since: None,
            _idle: None,
            typing_notice: None,
            _intercept: intercept,
        };
        this.connect(cx);
        this
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.session = Some(Session::start(self.adb.clone(), self.serial.clone(), tx));
        self.state = State::Connecting;
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connected => {
                self.state = State::Live;
                self.zoom_label_until = Some(Instant::now() + Duration::from_secs(2));
            }
            Event::Frame if self.hidden_since.is_some() => return,
            Event::Frame => {
                let Some(frame) = self.session.as_ref().and_then(Session::take_frame) else { return };
                let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra) else { return };
                if self.frame_size != (frame.width, frame.height) {
                    self.frame_size = (frame.width, frame.height);
                    // A rotation: the device's touch area changed too.
                    self.pressed = false;
                }
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                if let Some(old) = self.image.replace(image) {
                    self.retired.push(old);
                }
            }
            Event::Clipboard(text) => {
                if self.clipboard.as_ref() != Some(&text) {
                    self.clipboard = Some(text.clone());
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                return;
            }
            Event::Typing(why) => {
                if self.typing_notice.as_ref() != Some(&why) {
                    self.typing_notice = Some(why.clone());
                    cx.emit(MirrorEvent::Notify { title: format!("Could not type on {}", self.title), message: why, error: true });
                }
                return;
            }
            Event::Closed(reason) => {
                self.session = None;
                self.pressed = false;
                self.state = State::Failed(reason.unwrap_or_else(|| "Lost connection to the device".into()));
                cx.emit(MirrorEvent::Changed);
            }
        }
        cx.notify();
    }

    fn sync_clipboard(&mut self, text: Option<String>) {
        let Some(text) = text.filter(|t| !t.is_empty() && t.len() <= mirror::MAX_CLIPBOARD) else { return };
        if self.clipboard.as_ref() == Some(&text) {
            return;
        }
        if let Some(session) = &self.session {
            session.set_clipboard(&text, false);
        }
        self.clipboard = Some(text);
    }

    fn zoom_controls_visible(&self) -> bool {
        self.zoom_controls
    }

    fn set_zoom_controls(&mut self, shown: bool) {
        self.zoom_controls = shown;
    }

    fn live(&self) -> Option<&Session> {
        self.session.as_ref().filter(|_| self.state == State::Live)
    }

    fn press(&self, keycode: u32) {
        if let Some(session) = self.live() {
            session.press(keycode, 0);
        }
    }

    /// A key typed on the screen: special keys and shortcuts as key events;
    /// plain characters come as text through the input handler (so IMEs
    /// work). With Hardware Input every key is a key event.
    fn on_keystroke(&mut self, keystroke: &gpui_kit::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.live() else { return false };
        let m = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        if !self.hardware_input {
            // The IDE keeps its tool window shortcuts and Find Action.
            let ide_key = (m.alt && !m.control && key.len() == 1 && key.as_bytes()[0].is_ascii_digit())
                || (m.control && m.shift && key == "a")
                || (m.alt && key == "f12")
                || (m.control && key == "tab");
            if ide_key || m.function {
                return false;
            }
            // AltGr (Ctrl+Alt on Windows) types its character.
            if cfg!(windows) && m.control && m.alt && keystroke.key_char.as_deref().is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control)) {
                return false;
            }
            if is_typed(key) && !(m.control || m.alt || m.platform) {
                return false;
            }
        }
        let Some(code) = keycode(key) else { return false };
        session.press(code, meta_state(m));
        self.marked = None;
        cx.notify();
        true
    }

    /// A window position as a pixel of the device's screen.
    fn device_point(&self, position: gpui_kit::Point<Pixels>) -> Option<(i32, i32)> {
        let bounds = self.screen.get();
        let (w, h) = self.frame_size;
        if w == 0 || bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return None;
        }
        let x = f32::from(position.x - bounds.origin.x) / f32::from(bounds.size.width);
        let y = f32::from(position.y - bounds.origin.y) / f32::from(bounds.size.height);
        Some(((x.clamp(0., 1.) * (w - 1) as f32).round() as i32, (y.clamp(0., 1.) * (h - 1) as f32).round() as i32))
    }

    fn inside(&self, position: gpui_kit::Point<Pixels>) -> bool {
        self.screen.get().contains(&position)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if !self.inside(event.position) {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.touch(mirror::ACTION_DOWN, x, y, w, h);
            self.pressed = true;
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {
        if !self.pressed {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.touch(mirror::ACTION_MOVE, x, y, w, h);
        }
    }

    fn on_mouse_up(&mut self, position: gpui_kit::Point<Pixels>) {
        if !std::mem::take(&mut self.pressed) {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(position)) {
            session.touch(mirror::ACTION_UP, x, y, w, h);
        }
    }

    fn on_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, _: &mut Context<Self>) {
        if !self.inside(event.position) {
            return;
        }
        let delta = match event.delta {
            // A wheel notch is three lines (the system's scroll setting); the
            // device scrolls by notches.
            gpui_kit::ScrollDelta::Lines(d) => (d.x / 3., d.y / 3.),
            gpui_kit::ScrollDelta::Pixels(d) => (f32::from(d.x) / 40., f32::from(d.y) / 40.),
        };
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.scroll(x, y, w, h, delta.0, delta.1);
        }
    }

    fn rotate(&self, left: bool) {
        if let Some(session) = self.live() {
            session.rotate(left, self.frame_size.0 > self.frame_size.1);
        }
    }

    /// The zoom now, as device pixels per physical screen pixel.
    fn scale(&self) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit.get())
    }

    fn set_zoom(&mut self, zoom: Option<f32>, cx: &mut Context<Self>) {
        self.zoom = zoom.map(|z| z.clamp(0.05, 4.));
        self.zoom_label_until = Some(Instant::now() + Duration::from_secs(2));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2100)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        // IntelliJ's zoom steps snap to round percentages.
        let next = (self.scale() * factor * 20.).round() / 20.;
        let next = if (next - self.scale()).abs() < 0.01 { self.scale() * factor } else { next };
        self.set_zoom(Some(next), cx);
    }

    fn take_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live().is_none() {
            return;
        }
        let (adb, serial, title) = (self.adb.clone(), self.serial.clone(), self.title.clone());
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            let result = cx.background_spawn({
                let (adb, serial) = (adb.clone(), serial.clone());
                async move { mirror::screenshot(&adb, &serial) }
            });
            let result = result.await;
            cx.update_window(window_handle, |_, window, cx| match result {
                Ok(png) => screenshot_dialog(weak, png, window, cx),
                Err(e) => {
                    weak.update(cx, |_, cx| cx.emit(MirrorEvent::Notify { title: format!("Screenshot of {title} failed"), message: e.to_string(), error: true }))
                        .ok();
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if let Some(recording) = self.recording.take() {
            let (adb, serial, title) = (self.adb.clone(), self.serial.clone(), self.title.clone());
            let name = format!("Screen_recording_{}.mp4", chrono::Local::now().format("%Y%m%d_%H%M%S"));
            let target = cx.prompt_for_new_path(&default_save_dir(), Some(&name));
            cx.notify();
            cx.spawn(async move |this, cx| {
                let target = chosen_path(target.await, &name);
                let result = cx
                    .background_spawn({
                        let target = target.clone();
                        async move {
                            match target {
                                Some(target) => recording.finish(&adb, &serial, &target).map(|_| Some(target)),
                                None => {
                                    recording.cancel(&adb, &serial);
                                    Ok(None)
                                }
                            }
                        }
                    })
                    .await;
                this.update(cx, |_, cx| match result {
                    Ok(Some(path)) => {
                        cx.emit(MirrorEvent::Notify { title: "Screen recording saved".into(), message: path.display().to_string(), error: false })
                    }
                    Ok(None) => {}
                    Err(e) => cx.emit(MirrorEvent::Notify { title: format!("Screen recording of {title} failed"), message: e.to_string(), error: true }),
                })
                .ok();
            })
            .detach();
            return;
        }
        if self.live().is_none() {
            return;
        }
        match Recording::start(&self.adb, &self.serial) {
            Ok(recording) => {
                self.recording = Some(recording);
                // The elapsed time on the button follows the clock.
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let Ok(recording) = this.update(cx, |this, cx| {
                            cx.notify();
                            this.recording.is_some()
                        }) else {
                            break;
                        };
                        if !recording {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(e) => cx.emit(MirrorEvent::Notify { title: "Cannot record the screen".into(), message: e.to_string(), error: true }),
        }
        cx.notify();
    }

    /// While the tool window is hidden the connection stays (showing it
    /// again is instant) but frames aren't drawn; hidden for
    /// [`IDLE_DISCONNECT`] it ends the device-side server, and showing the
    /// window reconnects.
    fn set_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if paused {
            if self.hidden_since.is_none() {
                self.hidden_since = Some(Instant::now());
                self.pressed = false;
                self._idle = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(IDLE_DISCONNECT).await;
                    let _ = this.update(cx, |this, _| {
                        if this.hidden_since.is_some() && this.session.is_some() {
                            this.session = None;
                            this._events = None;
                            this.state = State::Connecting;
                        }
                    });
                }));
            }
            return;
        }
        if self.hidden_since.take().is_none() {
            return;
        }
        self._idle = None;
        if self.session.is_none() && self.state == State::Connecting {
            self.connect(cx);
        } else {
            // The newest frame waited while hidden.
            self.on_event(Event::Frame, cx);
        }
    }

    fn stop(&mut self) {
        if let Some(recording) = self.recording.take() {
            let (adb, serial) = (self.adb.clone(), self.serial.clone());
            std::thread::spawn(move || recording.cancel(&adb, &serial));
        }
        self.session = None;
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let live = self.state == State::Live;
        let key = |id: &'static str, icon: icons::AsIcon, tooltip: &'static str, keycode: u32, cx: &mut Context<Self>| {
            tool_button(id, icon, tooltip).disabled(!live).on_click(cx.listener(move |this, _, _, _| this.press(keycode)))
        };
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border);
        let recording = self.recording.as_ref().map(|r| r.started.elapsed().as_secs());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(key("rd-power", icons::EMULATOR_POWER, "Power", KEYCODE_POWER, cx))
            .child(key("rd-volume-up", icons::EMULATOR_VOLUME_UP, "Volume Up", KEYCODE_VOLUME_UP, cx))
            .child(key("rd-volume-down", icons::EMULATOR_VOLUME_DOWN, "Volume Down", KEYCODE_VOLUME_DOWN, cx))
            .child(separator())
            .child(tool_button("rd-rotate-left", icons::EMULATOR_ROTATE_LEFT, "Rotate Left").disabled(!live).on_click(cx.listener(|this, _, _, _| this.rotate(true))))
            .child(tool_button("rd-rotate-right", icons::EMULATOR_ROTATE_RIGHT, "Rotate Right").disabled(!live).on_click(cx.listener(|this, _, _, _| this.rotate(false))))
            .child(separator())
            .child(key("rd-back", icons::EMULATOR_BACK, "Back", KEYCODE_BACK, cx))
            .child(key("rd-home", icons::EMULATOR_HOME, "Home", KEYCODE_HOME, cx))
            .child(key("rd-overview", icons::EMULATOR_OVERVIEW, "Overview", KEYCODE_APP_SWITCH, cx))
            .child(separator())
            .child(
                tool_button(
                    "rd-hardware-input",
                    icons::EMULATOR_HARDWARE_INPUT,
                    "Hardware Input: Enable transparent forwarding of keyboard and mouse events to the device",
                )
                .disabled(!live)
                .selected(self.hardware_input)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.hardware_input = !this.hardware_input;
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
            )
            .child(separator())
            .child(tool_button("rd-screenshot", icons::EMULATOR_SCREENSHOT, "Take Screenshot").disabled(!live).on_click(cx.listener(|this, _, window, cx| this.take_screenshot(window, cx))))
            .child(
                tool_button("rd-record", icons::EMULATOR_SCREEN_RECORD, if recording.is_some() { "Stop Recording" } else { "Record Screen" })
                    .disabled(!live && recording.is_none())
                    .selected(recording.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_recording(cx))),
            )
            .when_some(recording, |el, secs| {
                el.child(div().text_xs().text_color(palette.status_conflict).child(format!("{:02}:{:02}", secs / 60, secs % 60)))
            })
    }

    fn render_zoom_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        v_flex()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .p_0p5()
            .gap_0p5()
            .rounded(px(6.))
            .border_1()
            .border_color(palette.border)
            .bg(gpui_kit::Hsla { a: 1., ..palette.panel })
            .child(tool_button("rd-zoom-in", icons::ADD, "Zoom In").on_click(cx.listener(|this, _, _, cx| this.zoom_by(1.25, cx))))
            .child(tool_button("rd-zoom-out", icons::REMOVE, "Zoom Out").on_click(cx.listener(|this, _, _, cx| this.zoom_by(0.8, cx))))
            .child(tool_button("rd-zoom-actual", icons::ACTUAL_ZOOM, "Zoom to Actual Size (100%)").on_click(cx.listener(|this, _, _, cx| this.set_zoom(Some(1.), cx))))
            .child(
                tool_button("rd-zoom-fit", icons::FIT_CONTENT, "Zoom to Fit in Window")
                    .selected(self.zoom.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.set_zoom(None, cx))),
            )
    }

    fn render_screen(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(image) = self.image.clone() else {
            return div().into_any_element();
        };
        let (fw, fh) = (self.frame_size.0 as f32, self.frame_size.1 as f32);
        let scale_factor = window.scale_factor();
        let (screen, fit, focus, entity) = (self.screen.clone(), self.fit.clone(), self.focus.clone(), cx.entity());
        let paint = canvas(|_, _, _| {}, move |bounds, _, window, cx| {
            // Fit: the largest that fits, centered; zoomed: the canvas is the screen.
            let rect = if zoomed_size(bounds, fw, fh) {
                bounds
            } else {
                let s = (f32::from(bounds.size.width) / fw).min(f32::from(bounds.size.height) / fh);
                fit.set(s * scale_factor);
                let (w, h) = (fw * s, fh * s);
                Bounds::new(
                    point(bounds.origin.x + (bounds.size.width - px(w)) / 2., bounds.origin.y + (bounds.size.height - px(h)) / 2.),
                    size(px(w), px(h)),
                )
            };
            screen.set(rect);
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            let _ = window.paint_image(rect, rect, Corners::default(), image, 0, false);
        });
        match self.zoom {
            None => paint.size_full().into_any_element(),
            Some(z) => {
                let (w, h) = (fw * z / scale_factor, fh * z / scale_factor);
                div()
                    .id("rd-scroll")
                    .size_full()
                    .overflow_scroll()
                    .child(
                        div()
                            .min_w_full()
                            .min_h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(paint.w(px(w)).h(px(h)).flex_shrink_0()),
                    )
                    .into_any_element()
            }
        }
    }
}

/// A canvas laid out at the screen's zoomed size has exactly its aspect.
fn zoomed_size(bounds: Bounds<Pixels>, fw: f32, fh: f32) -> bool {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    h > 0. && fh > 0. && ((w / h) - (fw / fh)).abs() < 0.002
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

/// Take Screenshot: the image, Copy to Clipboard, and Save.
fn screenshot_dialog(view: WeakEntity<MirrorView>, png: Vec<u8>, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::dialog::DialogFooter;
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png.clone()));
    let png = Arc::new(png);
    window.open_dialog(cx, move |dialog, _, _| {
        let (copy_image, save_png, recapture) = (image.clone(), png.clone(), view.clone());
        dialog
            .title("Screenshot")
            .w(px(560.))
            .child(
                div()
                    .h(px(480.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(img(image.clone()).object_fit(ObjectFit::Contain).max_w_full().max_h_full()),
            )
            .footer(
                DialogFooter::new()
                    .w_full()
                    .gap_2()
                    .child(Button::new("rd-shot-recapture").label("Recapture").outline().on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        recapture.update(cx, |this, cx| this.take_screenshot(window, cx)).ok();
                    }))
                    .child(Button::new("rd-shot-copy").label("Copy to Clipboard").outline().on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_image(&copy_image));
                    }))
                    .child(div().flex_1())
                    .child(Button::new("rd-shot-cancel").label("Cancel").outline().on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("rd-shot-save").label("Save").primary().on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let name = format!("Screenshot_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"));
                        let target = cx.prompt_for_new_path(&default_save_dir(), Some(&name));
                        let png = save_png.clone();
                        cx.spawn(async move |_| {
                            if let Some(path) = chosen_path(target.await, &name) {
                                let _ = std::fs::write(path, png.as_slice());
                            }
                        })
                        .detach();
                    })),
            )
    });
}

impl gpui_kit::EntityInputHandler for MirrorView {
    fn text_for_range(&mut self, range: std::ops::Range<usize>, adjusted: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let marked = self.marked.as_ref()?;
        let utf16: Vec<u16> = marked.encode_utf16().collect();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        *adjusted = Some(range.clone());
        Some(String::from_utf16_lossy(&utf16[range]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked.as_ref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if let Some(session) = self.live() {
            if !text.is_empty() && !self.hardware_input {
                session.type_text(text);
            }
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // The IME window opens under the screen's top left.
        let screen = self.screen.get();
        Some(Bounds::new(screen.origin, size(px(1.), px(20.))))
    }

    fn character_index_for_point(&mut self, _: gpui_kit::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Render for MirrorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        let palette = cx.palette().clone();
        let body: gpui_kit::AnyElement = match &self.state {
            State::Connecting => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).color(palette.text_secondary))
                .child("Connecting to the device")
                .into_any_element(),
            State::Failed(reason) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .px_4()
                .text_sm()
                .child(div().text_color(palette.text_secondary).text_center().child(reason.clone()))
                .child(Button::new("rd-reconnect").label("Reconnect").outline().small().on_click(cx.listener(|this, _, _, cx| this.connect(cx))))
                .into_any_element(),
            State::Live => self.render_screen(window, cx),
        };
        let show_label = self.state == State::Live && self.zoom_label_until.is_some_and(|t| Instant::now() < t);
        let percent = (self.scale() * 100.).round() as u32;
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("rd-screen")
                    .key_context("RunningDevice")
                    .track_focus(&self.focus)
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.focus.is_focused(window), |el| el.border_1().border_color(palette.accent.opacity(0.6)))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.on_mouse_down(e, window, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, window, cx| window.focus(&this.focus, cx)))
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| this.on_mouse_move(e, window, cx)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.on_mouse_up(e.position)))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.on_mouse_up(e.position)))
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| this.on_wheel(e, window, cx)))
                    .child(body)
                    .when(self.state == State::Live && self.zoom_controls, |el| el.child(self.render_zoom_controls(cx)))
                    .when(show_label, |el| {
                        el.child(
                            div()
                                .absolute()
                                .bottom(px(8.))
                                .right(px(8.))
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(gpui_kit::Hsla { a: 1., ..palette.panel })
                                .border_1()
                                .border_color(palette.border)
                                .text_sm()
                                .child(format!("{percent}%")),
                        )
                    }),
            )
    }
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

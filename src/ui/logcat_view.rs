//! The Logcat tool window, as Android Studio's: a device picker and the
//! filter query on top, the actions down the left (Clear, Scroll to the
//! End, Soft-Wrap, Standard / Compact format, Pause, Restart), and the log
//! with Android Studio's level colors, tag colors and PROCESS STARTED /
//! ENDED lines. Stack trace frames link to the source.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::{
    Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FollowMode, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ListAlignment, ListState, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, list,
    prelude::FluentBuilder as _, px,
};

use crate::android::Device;
use crate::domain::logcat::{self, Entry, Filter, Level};
use crate::infra::adb_logcat::AdbLog;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;
use crate::ui::run_service::RunService;

/// Android Studio's default cycle buffer: 1024 KB of log.
const BUFFER_BYTES: usize = 1024 * 1024;

pub enum LogcatEvent {
    /// A stack frame was clicked: find `file` under `package` ("com/example") and open it at the 1-based line.
    OpenFrame { package: String, file: String, line: u32 },
    Hide,
}

pub struct LogcatView {
    devices: Entity<DeviceManager>,
    run: Entity<RunService>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    /// The device picked in the combo (its id); else the run target.
    picked: Option<String>,
    /// The device being followed: id and serial.
    following: Option<(String, String)>,
    /// The project's application IDs, for `package:mine`.
    mine: Vec<String>,
    entries: VecDeque<Entry>,
    /// The id of `entries[0]`; ids count every entry since the start.
    first: u64,
    bytes: usize,
    /// Ids of the entries the filter lets through, in order.
    shown: VecDeque<u64>,
    filter: Filter,
    query: Entity<InputState>,
    list: ListState,
    compact: bool,
    wrap: bool,
    paused: bool,
    /// Selected rows (ids), for Copy.
    selected: Option<(u64, u64)>,
    focus: FocusHandle,
    stop: Arc<AtomicBool>,
    receiver: Option<std::sync::mpsc::Receiver<Entry>>,
    open: bool,
    _pump: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LogcatEvent> for LogcatView {}

impl Focusable for LogcatView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl LogcatView {
    pub fn new(devices: Entity<DeviceManager>, run: Entity<RunService>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).default_value("package:mine").placeholder("Filter: package:mine level:warn tag:MyTag -tag:Noisy is:crash age:5m"));
        let list = ListState::new(0, ListAlignment::Top, px(400.));
        list.set_follow_mode(FollowMode::Tail);
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refilter(cx);
                }
            }),
            cx.observe(&devices, |this, _, cx| this.follow_device(cx)),
            // A launch's application ID joins `package:mine` (a debug suffix
            // is not in the build files).
            cx.observe(&run, |this, run, cx| {
                let id = run.read(cx).console.as_ref().and_then(|c| c.application_id.clone());
                if let Some(id) = id.filter(|id| !this.mine.contains(id)) {
                    this.mine.push(id);
                    this.refilter(cx);
                }
            }),
        ];
        Self {
            devices,
            run,
            project,
            picked: None,
            following: None,
            mine: Vec::new(),
            entries: VecDeque::new(),
            first: 0,
            bytes: 0,
            shown: VecDeque::new(),
            filter: Filter::default(),
            query,
            list,
            compact: false,
            wrap: false,
            paused: false,
            selected: None,
            focus: cx.focus_handle(),
            stop: Arc::default(),
            receiver: None,
            open: false,
            _pump: None,
            _subscriptions: subscriptions,
        }
    }

    /// The tool window showed or hid. Logcat starts reading when first
    /// shown and keeps reading while hidden, as Android Studio's does.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown && !self.open {
            self.open = true;
            if let Some(root) = (self.project)(cx) {
                for id in crate::android::application_ids(&root) {
                    if !self.mine.contains(&id) {
                        self.mine.push(id);
                    }
                }
            }
            self.refilter(cx);
            self.follow_device(cx);
        }
    }

    /// Devices with a log: the online ones.
    fn devices(&self, cx: &App) -> Vec<Device> {
        self.devices.read(cx).devices().iter().filter(|d| d.online && d.serial.is_some() && d.wifi.is_none()).cloned().collect()
    }

    /// The device to show: the picked one, else the run target, else the first.
    fn target(&self, cx: &App) -> Option<Device> {
        let all = self.devices(cx);
        if let Some(id) = &self.picked {
            return all.into_iter().find(|d| d.id == *id);
        }
        let run = self.run.read(cx).device(cx).map(|d| d.id);
        all.iter().find(|d| Some(&d.id) == run.as_ref()).cloned().or_else(|| all.into_iter().next())
    }

    /// Starts following the target device when it changed.
    fn follow_device(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let target = self.target(cx).and_then(|d| Some((d.id.clone(), d.serial.clone()?)));
        let same = match (&target, &self.following) {
            (Some(t), Some(f)) => t == f,
            (None, _) => true,
            _ => false,
        };
        if same {
            return;
        }
        // A device that went away keeps its log on screen, as an [OFFLINE] entry does.
        if let Some(target) = target {
            self.start(target, cx);
        }
    }

    fn start(&mut self, (id, serial): (String, String), cx: &mut Context<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(adb) = self.devices.read(cx).adb() else { return };
        self.following = Some((id, serial.clone()));
        self.entries.clear();
        self.shown.clear();
        self.bytes = 0;
        self.selected = None;
        self.list.reset(0);
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let source = AdbLog { adb };
            let _ = crate::usecase::logcat::follow(&source, &serial, &stop, &mut |entry| {
                let _ = tx.send(entry);
            });
        });
        self._pump = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let Ok(alive) = this.update(cx, |this, cx| this.drain(cx)) else { break };
                if !alive {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Takes what the reader sent; false once it ended.
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(rx) = &self.receiver else { return false };
        let mut batch = Vec::new();
        let alive = loop {
            match rx.try_recv() {
                Ok(entry) => batch.push(entry),
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        };
        if !alive {
            self.receiver = None;
        }
        if batch.is_empty() {
            return alive;
        }
        for entry in batch {
            self.bytes += size_of(&entry);
            let id = self.first + self.entries.len() as u64;
            if !self.paused && self.filter.matches(&entry, entry.millis) {
                self.shown.push_back(id);
            }
            self.entries.push_back(entry);
        }
        let mut dropped = 0;
        while self.bytes > BUFFER_BYTES {
            let Some(old) = self.entries.pop_front() else { break };
            self.bytes -= size_of(&old);
            self.first += 1;
            while self.shown.front().is_some_and(|id| *id < self.first) {
                self.shown.pop_front();
                dropped += 1;
            }
        }
        if dropped > 0 {
            // Dropped rows were counted before the new ones were added.
            self.list.splice(0..dropped.min(self.list.item_count()), 0);
        }
        let count = self.list.item_count();
        let fresh = self.shown.len().saturating_sub(count);
        if fresh > 0 {
            self.list.splice(count..count, fresh);
        }
        cx.notify();
        alive
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.filter = Filter::parse(&query, &self.mine);
        if self.paused {
            cx.notify();
            return;
        }
        let now = self.entries.back().map_or(0, |e| e.millis);
        self.shown = self.entries.iter().enumerate().filter(|(_, e)| self.filter.matches(e, now)).map(|(i, _)| self.first + i as u64).collect();
        self.list.reset(self.shown.len());
        if self.list.is_following_tail() || self.selected.is_none() {
            self.list.set_follow_mode(FollowMode::Tail);
        }
        cx.notify();
    }

    fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.get(id.checked_sub(self.first)? as usize)
    }

    /// Clear Logcat: the device's buffers and the view.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.first += self.entries.len() as u64;
        self.entries.clear();
        self.shown.clear();
        self.bytes = 0;
        self.selected = None;
        self.list.reset(0);
        if let (Some((_, serial)), Some(adb)) = (self.following.clone(), self.devices.read(cx).adb()) {
            cx.background_executor()
                .spawn(async move {
                    use crate::domain::logcat::LogSource as _;
                    let _ = AdbLog { adb }.clear(&serial);
                })
                .detach();
        }
        cx.notify();
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        if let Some(following) = self.following.clone() {
            self.start(following, cx);
        } else {
            self.follow_device(cx);
        }
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        self.paused = !self.paused;
        if !self.paused {
            self.refilter(cx);
        }
        cx.notify();
    }

    fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Tail);
        self.list.scroll_to_end();
        cx.notify();
    }

    fn pick(&mut self, id: String, cx: &mut Context<Self>) {
        self.picked = Some(id);
        self.following = None;
        self.follow_device(cx);
    }

    fn select(&mut self, id: u64, extend: bool, cx: &mut Context<Self>) {
        self.selected = match (extend, self.selected) {
            (true, Some((anchor, _))) => Some((anchor, id)),
            _ => Some((id, id)),
        };
        self.list.pause_following_tail();
        cx.notify();
    }

    fn is_selected(&self, id: u64) -> bool {
        self.selected.is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&id))
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let Some((a, b)) = self.selected else { return };
        let text: Vec<String> = self.shown.iter().filter(|id| (a.min(b)..=a.max(b)).contains(id)).filter_map(|id| self.entry(*id)).map(|e| format_line(e, self.compact)).collect();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.join("\n")));
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.key == "c" && k.modifiers.secondary() {
            self.copy(cx);
            cx.stop_propagation();
        }
    }

    fn render_row(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let Some(id) = self.shown.get(ix).copied() else { return div().into_any_element() };
        let Some(e) = self.entry(id).cloned() else { return div().into_any_element() };
        let selected = self.is_selected(id);
        let mut row = h_flex()
            .id(("logcat-row", ix))
            .w_full()
            .px_2()
            .items_start()
            .when(selected, |el| el.bg(palette.selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select(id, event.modifiers.shift, cx);
                }),
            );
        if e.system {
            return row.child(div().whitespace_nowrap().child(e.message)).into_any_element();
        }
        let dark = palette.dark;
        let (badge_fg, badge_bg) = level_badge(e.level, dark);
        let message_color = level_message(e.level, dark).unwrap_or(palette.text);
        let time = if self.compact { e.time.get(11..).unwrap_or(&e.time).to_owned() } else { e.time.clone() };
        let mut header = h_flex().flex_none().whitespace_nowrap().child(format!("{time} "));
        if !self.compact {
            header = header
                .child(format!("{:>5}-{:<5} ", e.pid, e.tid))
                .child(div().text_color(tag_color(&e.tag, dark)).child(pad(if e.tag.is_empty() { "<no-tag>" } else { &e.tag }, 23)))
                .child(pad(&e.package, 35));
        }
        header = header.child(div().px(px(4.)).text_color(badge_fg).bg(badge_bg).child(e.level.letter().to_string())).child(" ");
        let message = match logcat::stack_frame(&e.message) {
            Some((package, file, line)) if file != "Unknown Source" => {
                // "at com.example.Foo.bar(" + link "Foo.kt:42" + ")"
                let open = e.message.find('(').unwrap_or(0) + 1;
                let close = e.message.rfind(')').unwrap_or(e.message.len()).max(open);
                let (before, link, after) = (e.message[..open].to_owned(), e.message[open..close].to_owned(), e.message[close..].to_owned());
                h_flex()
                    .child(before)
                    .child(
                        div()
                            .id(("logcat-frame", ix))
                            .text_color(palette.link)
                            .underline()
                            .cursor_pointer()
                            .child(link)
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(LogcatEvent::OpenFrame { package: package.clone(), file: file.clone(), line }))),
                    )
                    .child(after)
                    .into_any_element()
            }
            _ => div().child(e.message).into_any_element(),
        };
        row = row.child(header).child(div().text_color(message_color).when(self.wrap, |el| el.flex_1().min_w_0().whitespace_normal()).when(!self.wrap, |el| el.whitespace_nowrap()).child(message));
        row.into_any_element()
    }

    fn device_label(d: &Device) -> String {
        let release = d.props.get("ro.build.version.release").cloned().unwrap_or_default();
        match d.api {
            Some(api) if !release.is_empty() => format!("{} Android {release}, API {api}", d.name),
            Some(api) => format!("{} API {api}", d.name),
            None => d.name.clone(),
        }
    }
}

fn size_of(e: &Entry) -> usize {
    e.message.len() + e.tag.len() + e.package.len() + 48
}

/// Pads or shortens to Android Studio's column width, plus the separating space.
fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count > width {
        let keep = width - 3;
        let head: String = text.chars().take(keep / 2).collect();
        let tail: String = text.chars().skip(count - (keep - keep / 2)).collect();
        format!("{head}...{tail} ")
    } else {
        format!("{text:<width$} ")
    }
}

/// The line as Android Studio copies it.
fn format_line(e: &Entry, compact: bool) -> String {
    if e.system {
        return e.message.clone();
    }
    if compact {
        return format!("{} {} {}", e.time.get(11..).unwrap_or(&e.time), e.level.letter(), e.message);
    }
    format!("{} {:>5}-{:<5} {}{}{}  {}", e.time, e.pid, e.tid, pad(&e.tag, 23), pad(&e.package, 35), e.level.letter(), e.message)
}

fn hex(rgb: u32) -> Hsla {
    gpui_kit::rgb(rgb).into()
}

/// LOGCAT_V2_LEVEL_* of Android Studio's Default and Darcula schemes.
fn level_badge(level: Level, dark: bool) -> (Hsla, Hsla) {
    let (fg, bg) = match (level, dark) {
        (Level::Verbose, _) => (0x000000, 0xd6d6d6),
        (Level::Debug, false) => (0x000000, 0xcfe7ff),
        (Level::Debug, true) => (0xbbbbbb, 0x305d78),
        (Level::Info, false) => (0x414d41, 0xe9f5e6),
        (Level::Info, true) => (0xe9f5e6, 0x6a8759),
        (Level::Warn, false) => (0x000000, 0xf5eac1),
        (Level::Warn, true) => (0x000000, 0xbbb529),
        (Level::Error, false) => (0xffffff, 0xcf5b56),
        (Level::Error, true) => (0x000000, 0xcf5b56),
        (Level::Assert, false) => (0xffffff, 0x7f0000),
        (Level::Assert, true) => (0xffffff, 0x8b3c3c),
    };
    (hex(fg), hex(bg))
}

/// LOGCAT_V2_MESSAGE_*: None keeps the editor's text color.
fn level_message(level: Level, dark: bool) -> Option<Hsla> {
    let rgb = match (level, dark) {
        (Level::Verbose, _) => return None,
        (Level::Debug, false) => 0x389fd6,
        (Level::Debug, true) => 0x299999,
        (Level::Info, false) => 0x59a869,
        (Level::Info, true) => 0xabc023,
        (Level::Warn, false) => 0x645607,
        (Level::Warn, true) => 0xbbb529,
        (Level::Error | Level::Assert, false) => 0xcd0000,
        (Level::Error | Level::Assert, true) => 0xff6b68,
    };
    Some(hex(rgb))
}

/// Each tag keeps a color from a small pool, as in Android Studio.
fn tag_color(tag: &str, dark: bool) -> Hsla {
    const LIGHT: [u32; 8] = [0x8a5a00, 0x00627a, 0x7a3e9d, 0x067d17, 0xb3261e, 0x0033b3, 0x871094, 0x5c6a00];
    const DARK: [u32; 8] = [0xe0a85c, 0x5fb3b3, 0xc58fe0, 0x7ec699, 0xf28b82, 0x8ab4f8, 0xd7a3e6, 0xc3cf6a];
    let hash = tag.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let pool = if dark { &DARK } else { &LIGHT };
    hex(pool[hash as usize % pool.len()])
}

fn side_button(id: &'static str, icon: icons::AsIcon, tooltip: impl Into<SharedString>) -> Button {
    tool_button(id, icon, tooltip)
}

impl Render for LogcatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let devices = self.devices(cx);
        let following = self.following.clone();
        let current = following.as_ref().and_then(|(id, _)| self.devices.read(cx).devices().iter().find(|d| d.id == *id).cloned());
        let label = match (&current, &following) {
            (Some(d), _) if d.online => Self::device_label(d),
            (Some(d), _) => format!("{} [OFFLINE]", Self::device_label(d)),
            (None, Some(_)) => "Device [OFFLINE]".to_owned(),
            (None, None) if devices.is_empty() => "No connected devices".to_owned(),
            (None, None) => "Select a device".to_owned(),
        };
        let this = cx.entity().downgrade();
        let picked = following.as_ref().map(|(id, _)| id.clone());
        let device_combo = Button::new("logcat-device")
            .ghost()
            .small()
            .icon(Icon::new(icons::PHYSICAL_DEVICE_PHONE).small())
            .label(label)
            .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
            .dropdown_menu(move |mut menu, _, _| {
                if devices.is_empty() {
                    return menu.item(PopupMenuItem::new("No connected devices").disabled(true));
                }
                for d in &devices {
                    let (id, this) = (d.id.clone(), this.clone());
                    menu = menu.item(PopupMenuItem::new(Self::device_label(d)).checked(picked.as_deref() == Some(d.id.as_str())).on_click(move |_, _, cx| {
                        let id = id.clone();
                        this.update(cx, |v, cx| v.pick(id, cx)).ok();
                    }));
                }
                menu
            });
        let format = self.compact;
        let this = cx.entity().downgrade();
        let format_button = Button::new("logcat-format")
            .ghost()
            .xsmall()
            .icon(Icon::new(icons::SETTINGS))
            .tooltip("Configure Logcat Formatting Options")
            .dropdown_menu(move |menu, _, _| {
                let (a, b) = (this.clone(), this.clone());
                menu.item(PopupMenuItem::new("Standard").checked(!format).on_click(move |_, _, cx| {
                    a.update(cx, |v, cx| {
                        v.compact = false;
                        cx.notify();
                    })
                    .ok();
                }))
                .item(PopupMenuItem::new("Compact").checked(format).on_click(move |_, _, cx| {
                    b.update(cx, |v, cx| {
                        v.compact = true;
                        cx.notify();
                    })
                    .ok();
                }))
            });
        let toolbar = v_flex()
            .w(px(30.))
            .flex_none()
            .items_center()
            .py_1()
            .gap_0p5()
            .border_r_1()
            .border_color(palette.border)
            .child(side_button("logcat-clear", icons::CLEAR, "Clear Logcat").on_click(cx.listener(|this, _, _, cx| this.clear(cx))))
            .child(
                side_button("logcat-end", icons::DOWN, "Scroll to the End (clicking on a particular line stops scrolling and keeps that line visible)")
                    .selected(self.list.is_following_tail())
                    .on_click(cx.listener(|this, _, _, cx| this.scroll_to_end(cx))),
            )
            .child(side_button("logcat-wrap", icons::SOFT_WRAP, "Soft-Wrap").selected(self.wrap).on_click(cx.listener(|this, _, _, cx| {
                this.wrap = !this.wrap;
                this.list.remeasure();
                cx.notify();
            })))
            .child(format_button)
            .child(
                side_button("logcat-pause", if self.paused { icons::RESUME } else { icons::PAUSE }, if self.paused { "Resume Logcat" } else { "Pause Logcat" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(side_button("logcat-restart", icons::RERUN, "Restart Logcat").on_click(cx.listener(|this, _, _, cx| this.restart(cx))));
        let empty_reason = if self.following.is_none() {
            Some(if self.devices(cx).is_empty() { "No connected devices" } else { "Select a device" }.to_owned())
        } else if self.shown.is_empty() && !self.entries.is_empty() {
            Some("All logs entries are hidden by the filter".to_owned())
        } else {
            None
        };
        let banner = |text: &'static str| h_flex().h(px(26.)).px_2().gap_2().bg(palette.toolbar).border_b_1().border_color(palette.border).text_sm().child(text);
        let needs_ids = self.mine.is_empty() && self.query.read(cx).value().contains("package:mine");
        v_flex()
            .size_full()
            .bg(palette.panel)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                h_flex()
                    .h(px(32.))
                    .px_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().px_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Logcat"))
                    .child(device_combo)
                    .child(div().flex_1().min_w(px(120.)).child(Input::new(&self.query).small().cleanable(true).prefix(Icon::new(icons::FILTER).xsmall().text_color(palette.text_secondary))))
                    .child(tool_button("logcat-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(LogcatEvent::Hide)))),
            )
            .when(self.paused, |el| el.child(banner("Logcat is paused")))
            .when(needs_ids, |el| el.child(banner("Could not detect project package names. Is the project synced?")))
            .child(
                h_flex().flex_1().min_h_0().items_stretch().child(toolbar).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .font_family(mono)
                        .text_xs()
                        .font_weight(FontWeight::NORMAL)
                        .when_some(empty_reason, |el, reason| el.child(div().p_4().text_color(palette.text_secondary).child(reason)))
                        .child(list(self.list.clone(), cx.processor(Self::render_row)).size_full()),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_like_android_studio() {
        assert_eq!(pad("Tag", 6), "Tag    ");
        assert_eq!(pad("VeryLongTagName", 10), "Ver...Name ");
        let e = logcat::parse_threadtime("2026-10-15 02:05:48.123  1234  1250 D OkHttp  : hi").unwrap();
        assert_eq!(format_line(&e, true), "02:05:48.123 D hi");
    }
}

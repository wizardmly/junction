//! The Logcat tool window, as Android Studio's: a device picker and the
//! filter query on top, the actions down the left (Clear, Scroll to the
//! End, Soft-Wrap, Standard / Compact format, Pause, Restart), and the log
//! with Android Studio's level colors, tag colors and PROCESS STARTED /
//! ENDED lines. Stack trace frames link to the source. The filter field
//! completes keys and values and keeps a history (`filter`).

mod filter;
mod tabs;

pub use tabs::LogcatTabs;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::android::Device;
use crate::domain::logcat::{self, Entry, Filter, Level};
use crate::domain::retrace::{self, Retracer};
use crate::infra::adb_logcat::AdbLog;
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::console_view::{ConsoleEvent, ConsoleLine, ConsoleView};
use crate::ui::device_manager::DeviceManager;
use crate::ui::run_service::RunService;

/// Android Studio's default cycle buffer: 1024 KB of log.
const BUFFER_BYTES: usize = 1024 * 1024;

pub enum LogcatEvent {
    /// A stack frame was clicked: find `file` under `package` ("com/example") and open it at the 1-based line.
    OpenFrame { package: String, file: String, line: u32 },
    Hide,
    /// Split Panels › Split Right (`down` false) / Split Down: a new panel
    /// beside this one.
    Split { down: bool },
    /// Split Panels › Close: this panel goes, its neighbour takes its place.
    ClosePanel,
    /// Import Logcat from a File…: asks for a file and opens it in a new tab.
    Import,
    /// Tabs or panels changed: worth keeping for the next start.
    LayoutChanged,
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
    completion: Option<filter::CompletionPopup>,
    /// The next query change was a completed value: no popup for it.
    skip_completion: bool,
    history: crate::domain::logcat_filter::History,
    history_open: bool,
    /// The entries shown, in a read-only editor (Android Studio's Logcat
    /// is an editor too): a drag selects across lines, Ctrl+C copies.
    console: Entity<ConsoleView>,
    /// Bumped when what's shown is rebuilt (filter, format, clear).
    generation: u64,
    compact: bool,
    wrap: bool,
    paused: bool,
    focus: FocusHandle,
    stop: Arc<AtomicBool>,
    receiver: Option<std::sync::mpsc::Receiver<Entry>>,
    open: bool,
    /// In a tab of `LogcatTabs`: the title and Hide are the tab strip's.
    embedded: bool,
    /// Screen Record in progress on the followed device.
    recording: Option<crate::android::mirror::Recording>,
    /// An imported log file's name: the panel shows the file, not a device.
    file: Option<String>,
    /// That file's path, to reopen it after a restart.
    file_path: Option<PathBuf>,
    /// R8 / ProGuard mappings that retrace stack frames.
    mappings: Mappings,
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
        let console = cx.new(|cx| ConsoleView::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&console, |_, _, event: &ConsoleEvent, cx| {
                if let ConsoleEvent::OpenFrame { package, file, line } = event {
                    cx.emit(LogcatEvent::OpenFrame { package: package.clone(), file: file.clone(), line: *line });
                }
            }),
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.query_changed(cx),
                InputEvent::PressEnter { .. } => this.remember_filter(cx),
                InputEvent::Blur => {
                    this.remember_filter(cx);
                    this.completion = None;
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::CompletionCommand { .. } => {}
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
            completion: None,
            skip_completion: false,
            history: crate::infra::logcat_filter_store::load(),
            history_open: false,
            console,
            generation: 1,
            compact: false,
            wrap: false,
            paused: false,
            focus: cx.focus_handle(),
            stop: Arc::default(),
            receiver: None,
            open: false,
            embedded: false,
            recording: None,
            file: None,
            file_path: None,
            mappings: Mappings::default(),
            _pump: None,
            _subscriptions: subscriptions,
        }
    }

    /// Screen Capture: the followed device's screen in the screenshot
    /// dialog (Recapture, Copy to Clipboard, Save).
    fn take_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        let (Some((_, serial)), Some(adb)) = (self.following.clone(), self.devices.read(cx).adb()) else { return };
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        let task = cx.background_spawn(async move { crate::android::mirror::screenshot(&adb, &serial) });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update_window(handle, |_, window, cx| match result {
                Ok(png) => {
                    let recapture: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window, cx| {
                        weak.update(cx, |this, cx| this.take_screenshot(window, cx)).ok();
                    });
                    crate::ui::running_devices::capture::screenshot_dialog(png, recapture, window, cx);
                }
                Err(e) => window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()).title("Screen Capture failed"), cx),
            })
            .ok();
        })
        .detach();
    }

    /// Screen Record: starts recording the followed device; the second
    /// click stops it and asks where to save the video.
    fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        let (Some((_, serial)), Some(adb)) = (self.following.clone(), self.devices.read(cx).adb()) else { return };
        let Some(recording) = self.recording.take() else {
            match crate::android::mirror::Recording::start(&adb, &serial) {
                Ok(r) => self.recording = Some(r),
                Err(e) => window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()).title("Cannot record the screen"), cx),
            }
            cx.notify();
            return;
        };
        let name = format!("Screen_recording_{}.mp4", chrono::Local::now().format("%Y%m%d_%H%M%S"));
        let target = cx.prompt_for_new_path(&crate::ui::running_devices::default_save_dir(), Some(&name));
        let handle = window.window_handle();
        cx.notify();
        cx.spawn(async move |_, cx| {
            let target = crate::ui::running_devices::chosen_path(target.await, &name);
            let result = cx
                .background_spawn(async move {
                    match target {
                        Some(target) => recording.finish(&adb, &serial, &target).map(|_| Some(target)),
                        None => {
                            recording.cancel(&adb, &serial);
                            Ok(None)
                        }
                    }
                })
                .await;
            cx.update_window(handle, |_, window, cx| match result {
                Ok(Some(path)) => window.push_notification(gpui_kit::component::notification::Notification::success(path.display().to_string()).title("Screen recording saved"), cx),
                Ok(None) => {}
                Err(e) => window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()).title("Screen recording failed"), cx),
            })
            .ok();
        })
        .detach();
    }

    /// A closed tab: its log stops being read.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.receiver = None;
        self._pump = None;
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

    /// Shows an imported log (Import Logcat from a File): its entries, all
    /// of them, and the filter it was saved with.
    pub fn load_file(&mut self, path: PathBuf, log: crate::domain::logcat_file::LogFile, window: &mut Window, cx: &mut Context<Self>) {
        self.shutdown();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.file_path = Some(path);
        self.following = None;
        self.file = Some(match &log.device {
            Some(device) => format!("{name} ({device})"),
            None => name,
        });
        if let Some(root) = (self.project)(cx) {
            for id in crate::android::application_ids(&root) {
                if !self.mine.contains(&id) {
                    self.mine.push(id);
                }
            }
        }
        let filter = crate::domain::logcat_file::imported_filter(&log, &self.mine).unwrap_or_default();
        self.query.update(cx, |q, cx| q.set_value(filter, window, cx));
        self.first += self.entries.len() as u64;
        self.bytes = log.entries.iter().map(size_of).sum();
        self.entries = log.entries.into();
        self.refilter(cx);
    }

    /// Export Logcat to a File…: every entry read, as Android Studio's
    /// `.logcat` JSON with the device and the filter.
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        let device = self.following.as_ref().and_then(|(id, _)| self.devices.read(cx).devices().iter().find(|d| d.id == *id).cloned());
        let info = device.as_ref().map(|d| {
            let prop = |k: &str| d.props.get(k).cloned().unwrap_or_default();
            crate::domain::logcat_file::DeviceInfo {
                serial: d.serial.clone().unwrap_or_default(),
                manufacturer: prop("ro.product.manufacturer"),
                model: if prop("ro.product.model").is_empty() { d.name.clone() } else { prop("ro.product.model") },
                release: prop("ro.build.version.release"),
                api: d.api.unwrap_or(0),
                avd: d.avd.as_ref().map(|a| a.display_name.clone()),
            }
        });
        let info = info.unwrap_or_default();
        let entries: Vec<Entry> = self.entries.iter().cloned().collect();
        let query = self.query.read(cx).value().to_string();
        let text = crate::domain::logcat_file::write(&entries, &info, &query, &self.mine);
        let device_name = if info.serial.is_empty() { "logcat".to_owned() } else { info.name().replace([' ', '/', '\\', ':'], "_") };
        let name = format!("{device_name}_{}.logcat", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"));
        let target = cx.prompt_for_new_path(&crate::ui::running_devices::default_save_dir(), Some(&name));
        let handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            // No file dialog (no portal on Linux): the default folder.
            let Some(path) = crate::ui::running_devices::chosen_path(target.await, &name) else { return };
            let result = std::fs::write(&path, text);
            cx.update_window(handle, |_, window, cx| match result {
                Ok(()) => window.push_notification(Notification::success(path.display().to_string()).title("Logcat exported"), cx),
                Err(e) => window.push_notification(Notification::error(e.to_string()).title("Cannot export Logcat"), cx),
            })
            .ok();
        })
        .detach();
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
        if !self.open || self.file.is_some() {
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
        self.generation += 1;
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
        while self.bytes > BUFFER_BYTES {
            let Some(old) = self.entries.pop_front() else { break };
            self.bytes -= size_of(&old);
            self.first += 1;
            while self.shown.front().is_some_and(|id| *id < self.first) {
                self.shown.pop_front();
            }
        }
        // The console catches up when drawn.
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
        self.generation += 1;
        cx.notify();
    }

    /// Clear Logcat: the device's buffers and the view.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.clear_view();
        if let (Some((_, serial)), Some(adb), None) = (self.following.clone(), self.devices.read(cx).adb(), &self.file) {
            cx.background_executor()
                .spawn(async move {
                    use crate::domain::logcat::LogSource as _;
                    let _ = AdbLog { adb }.clear(&serial);
                })
                .detach();
        }
        cx.notify();
    }

    fn clear_view(&mut self) {
        self.first += self.entries.len() as u64;
        self.entries.clear();
        self.shown.clear();
        self.bytes = 0;
        self.generation += 1;
    }

    /// A launch cleared the device's log ("Clear log before launch"): a
    /// panel reading that device empties.
    pub fn clear_device(&mut self, serial: &str, cx: &mut Context<Self>) {
        if self.file.is_none() && self.following.as_ref().is_some_and(|(_, s)| s == serial) {
            self.clear_view();
            cx.notify();
        }
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
        self.console.update(cx, |c, cx| c.scroll_to_end(cx));
        cx.notify();
    }

    fn pick(&mut self, id: String, cx: &mut Context<Self>) {
        self.picked = Some(id);
        self.following = None;
        // A device picked in an imported log's panel: back to the device.
        if self.file.take().is_some() {
            self.file_path = None;
            self.entries.clear();
            self.shown.clear();
            self.bytes = 0;
        }
        self.follow_device(cx);
    }

    /// Load Proguard Mappings…: a mapping file for this panel's stack
    /// traces. Without a file dialog, the project's newest mapping (the
    /// one Android Studio's chooser starts at).
    fn choose_mapping(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Select a Proguard Mapping File".into()) });
        let root = (self.project)(cx);
        cx.spawn_in(window, async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(picked)) => match picked.and_then(|p| p.into_iter().next()) {
                    Some(path) => path,
                    None => return,
                },
                _ => match root.and_then(|root| crate::infra::r8_mapping::project_mappings(&root).into_iter().next()) {
                    Some(path) => path,
                    None => return,
                },
            };
            this.update_in(cx, |this, window, cx| this.load_mapping(path, true, window, cx)).ok();
        })
        .detach();
    }

    /// Loads a mapping for the panel; `report` says when it fails.
    fn load_mapping(&mut self, path: PathBuf, report: bool, window: &mut Window, cx: &mut Context<Self>) {
        let loaded = cx.background_executor().spawn({
            let path = path.clone();
            async move { crate::infra::r8_mapping::load(&path) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = loaded.await;
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(retracer) => {
                    this.mappings.manual = Some((path, Rc::new(retracer)));
                    this.generation += 1;
                    cx.notify();
                }
                Err(e) if report => {
                    use gpui_kit::component::{WindowExt as _, notification::Notification};
                    window.push_notification(Notification::error(e).title("Failed to add mapping file"), cx);
                }
                Err(_) => {}
            })
            .ok();
        })
        .detach();
    }

    /// What the panel keeps across restarts.
    pub fn panel_state(&self, cx: &App) -> crate::domain::logcat_layout::PanelState {
        crate::domain::logcat_layout::PanelState {
            filter: self.query.read(cx).value().to_string(),
            device: self.picked.clone(),
            compact: self.compact,
            wrap: self.wrap,
            mapping: self.mappings.manual.as_ref().map(|(p, _)| p.to_string_lossy().into_owned()),
            file: self.file_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    /// Back as it was kept (an imported file is read by the tabs).
    pub fn restore_state(&mut self, state: crate::domain::logcat_layout::PanelState, window: &mut Window, cx: &mut Context<Self>) {
        self.picked = state.device;
        self.compact = state.compact;
        self.wrap = state.wrap;
        let wrap = self.wrap;
        self.console.update(cx, |c, cx| c.set_soft_wrap(wrap, window, cx));
        self.skip_completion = true;
        self.query.update(cx, |q, cx| q.set_value(state.filter, window, cx));
        self.refilter(cx);
        if let Some(path) = state.mapping {
            self.load_mapping(PathBuf::from(path), false, window, cx);
        }
        if self.open && self.file.is_none() {
            self.follow_device(cx);
        }
    }

    /// A frame named an `r8-map-id` no loaded mapping has: look for the
    /// project's mapping R8 wrote with it (Android Studio's
    /// `AutoProguardMessageRewriter`), once per id.
    fn find_mapping(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.mappings.wanted.take() else { return };
        if !self.mappings.tried.insert(id.clone()) {
            return;
        }
        let Some(root) = (self.project)(cx) else { return };
        let found = cx.background_executor().spawn(async move {
            let path = crate::infra::r8_mapping::find_by_map_id(&root, &id)?;
            crate::infra::r8_mapping::load(&path).ok().map(|r| (path, r))
        });
        cx.spawn(async move |this, cx| {
            let Some((path, retracer)) = found.await else { return };
            this.update(cx, |this, cx| {
                this.mappings.auto = Some((path, Rc::new(retracer)));
                this.generation += 1;
                cx.notify();
            })
            .ok();
        })
        .detach();
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

/// The panel's mappings: the one loaded with Load Proguard Mappings… and
/// the project's one found by a frame's `r8-map-id`.
#[derive(Default)]
struct Mappings {
    manual: Option<(PathBuf, Rc<Retracer>)>,
    auto: Option<(PathBuf, Rc<Retracer>)>,
    /// Map ids already looked for.
    tried: std::collections::HashSet<String>,
    /// A map id seen while drawing, to look for next.
    wanted: std::cell::RefCell<Option<String>>,
}

impl Mappings {
    fn retrace(&self, message: &str) -> Option<Vec<String>> {
        let auto = self.auto.as_ref().map(|(_, r)| r);
        if let Some(id) = retrace::frame_map_id(message) {
            if let Some(r) = auto.filter(|r| r.map_id() == Some(id)) {
                return r.retrace(message);
            }
            if !self.tried.contains(id) {
                self.wanted.replace(Some(id.to_owned()));
            }
        }
        if self.manual.is_none() && auto.is_none() {
            return None;
        }
        self.manual.as_ref().and_then(|(_, r)| r.retrace(message)).or_else(|| auto.and_then(|r| r.retrace(message)))
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

/// An entry as the console shows it: Android Studio's columns (the copy
/// format), the level as a colored badge, the tag in its color, the
/// message in the level's color, a stack frame's `Foo.kt:42` a link.
fn console_line(e: &Entry, compact: bool, dark: bool, secondary: Hsla, mappings: &Mappings) -> ConsoleLine {
    use gpui_kit::HighlightStyle;
    if e.system {
        return ConsoleLine::plain(e.message.clone(), Some(secondary));
    }
    let mut text = String::new();
    let mut spans = Vec::new();
    let time = if compact { e.time.get(11..).unwrap_or(&e.time) } else { &e.time };
    text.push_str(time);
    text.push(' ');
    if !compact {
        text.push_str(&format!("{:>5}-{:<5} ", e.pid, e.tid));
        let start = text.len();
        text.push_str(&pad(if e.tag.is_empty() { "<no-tag>" } else { &e.tag }, 23));
        spans.push((start..text.len() - 1, HighlightStyle { color: Some(tag_color(&e.tag, dark)), ..Default::default() }));
        text.push_str(&pad(&e.package, 35));
    }
    let (fg, bg) = level_badge(e.level, dark);
    let start = text.len();
    text.push_str(&format!(" {} ", e.level.letter()));
    spans.push((start..text.len(), HighlightStyle { color: Some(fg), background_color: Some(bg), ..Default::default() }));
    text.push(' ');
    let message_at = text.len();
    // A retraced frame can be several (inlining): the rest go on lines of
    // their own, under the message column.
    let retraced = mappings.retrace(&e.message);
    let lines = retraced.as_deref().unwrap_or(std::slice::from_ref(&e.message));
    let indent = " ".repeat(text.chars().count());
    let mut links = Vec::new();
    for (i, message) in lines.iter().enumerate() {
        if i > 0 {
            text.push('\n');
            text.push_str(&indent);
        }
        let at = text.len();
        text.push_str(message);
        if let Some((package, file, line)) = logcat::stack_frame(message).filter(|(_, file, _)| file != "Unknown Source") {
            // "at com.example.Foo.bar(" + "Foo.kt:42" + ")"
            let open = message.find('(').map_or(0, |i| i + 1);
            let close = message.rfind(')').unwrap_or(message.len()).max(open);
            links.push((at + open..at + close, crate::domain::console_links::ConsoleLink::Frame { package, file, line }));
        }
    }
    if let Some(color) = level_message(e.level, dark) {
        spans.push((message_at..text.len(), HighlightStyle { color: Some(color), ..Default::default() }));
    }
    ConsoleLine { text: text.into(), color: None, spans, links }
}

/// The line as Android Studio copies it.
#[cfg(test)]
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        // The console catches up with what's shown.
        {
            let (entries, first, compact, dark, secondary, mappings) = (&self.entries, self.first, self.compact, palette.dark, palette.text_secondary, &self.mappings);
            let line = |id: u64| entries.get((id - first) as usize).map(|e| console_line(e, compact, dark, secondary, mappings)).unwrap_or_default();
            let (shown, generation) = (&self.shown, self.generation);
            self.console.update(cx, |c, cx| c.sync(shown, generation, &line, window, cx));
        }
        self.find_mapping(cx);
        let filter_field = self.render_filter(window, cx);
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let devices = self.devices(cx);
        let following = self.following.clone();
        let current = following.as_ref().and_then(|(id, _)| self.devices.read(cx).devices().iter().find(|d| d.id == *id).cloned());
        let label = match (&current, &following) {
            _ if self.file.is_some() => self.file.clone().unwrap_or_default(),
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
                        v.generation += 1;
                        cx.notify();
                    })
                    .ok();
                }))
                .item(PopupMenuItem::new("Compact").checked(format).on_click(move |_, _, cx| {
                    b.update(cx, |v, cx| {
                        v.compact = true;
                        v.generation += 1;
                        cx.notify();
                    })
                    .ok();
                }))
            });
        let split = cx.entity().downgrade();
        // Scrolls when a split panel is too short for all of it.
        let toolbar = v_flex()
            .id("logcat-side-toolbar")
            .overflow_y_scroll()
            .w(px(30.))
            .flex_none()
            .items_center()
            .py_1()
            .gap_0p5()
            .border_r_1()
            .border_color(palette.border)
            .child(side_button("logcat-clear", icons::CLEAR, "Clear Logcat").on_click(cx.listener(|this, _, _, cx| this.clear(cx))))
            .child(side_button("logcat-export", icons::EXPORT, "Export Logcat to a File…").disabled(self.entries.is_empty()).on_click(cx.listener(|this, _, window, cx| this.export(window, cx))))
            .when(self.embedded, |el| el.child(side_button("logcat-import", icons::IMPORT, "Import Logcat from a File…").on_click(cx.listener(|_, _, _, cx| cx.emit(LogcatEvent::Import)))))
            .child(
                side_button("logcat-end", icons::DOWN, "Scroll to the End (clicking on a particular line stops scrolling and keeps that line visible)")
                    .selected(self.console.read(cx).is_following(cx))
                    .on_click(cx.listener(|this, _, _, cx| this.scroll_to_end(cx))),
            )
            .child(side_button("logcat-wrap", icons::SOFT_WRAP, "Soft-Wrap").selected(self.wrap).on_click(cx.listener(|this, _, window, cx| {
                this.wrap = !this.wrap;
                let wrap = this.wrap;
                this.console.update(cx, |c, cx| c.set_soft_wrap(wrap, window, cx));
                cx.notify();
            })))
            .child(format_button)
            .child(
                side_button("logcat-pause", if self.paused { icons::RESUME } else { icons::PAUSE }, if self.paused { "Resume Logcat" } else { "Pause Logcat" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(side_button("logcat-restart", icons::RERUN, "Restart Logcat").on_click(cx.listener(|this, _, _, cx| this.restart(cx))))
            .child(
                side_button("logcat-mappings", icons::ABBREVIATE_PACKAGE_NAMES, match &self.mappings.manual {
                    Some((path, _)) => format!("Load Proguard Mappings… (now {})", path.display()),
                    None => "Load Proguard Mappings…".to_owned(),
                })
                .selected(self.mappings.manual.is_some())
                .on_click(cx.listener(|this, _, window, cx| this.choose_mapping(window, cx))),
            )
            // Android Studio's Screen Capture and Screen Record, for the followed device.
            .child(side_button("logcat-screenshot", icons::EMULATOR_SCREENSHOT, "Screen Capture").disabled(self.following.is_none()).on_click(cx.listener(|this, _, window, cx| this.take_screenshot(window, cx))))
            .child(
                side_button("logcat-record", if self.recording.is_some() { icons::STOP } else { icons::EMULATOR_SCREEN_RECORD }, if self.recording.is_some() { "Stop Recording" } else { "Screen Record" })
                    .disabled(self.following.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_recording(window, cx))),
            )
            // Android Studio's Split Panels (`LogcatSplitterActions`).
            .when(self.embedded, |el| {
                el.child(Button::new("logcat-split").ghost().xsmall().icon(Icon::new(icons::SPLIT_VERTICALLY)).tooltip("Split Panels").dropdown_menu(move |menu, _, _| {
                    let (right, down, close) = (split.clone(), split.clone(), split.clone());
                    menu.item(PopupMenuItem::new("Split Right").icon(icons::SPLIT_VERTICALLY).on_click(move |_, _, cx| {
                        right.update(cx, |_, cx| cx.emit(LogcatEvent::Split { down: false })).ok();
                    }))
                    .item(PopupMenuItem::new("Split Down").icon(icons::SPLIT_HORIZONTALLY).on_click(move |_, _, cx| {
                        down.update(cx, |_, cx| cx.emit(LogcatEvent::Split { down: true })).ok();
                    }))
                    .item(PopupMenuItem::new("Close").icon(icons::CLOSE).on_click(move |_, _, cx| {
                        close.update(cx, |_, cx| cx.emit(LogcatEvent::ClosePanel)).ok();
                    }))
                }))
            });
        let empty_reason = if self.file.is_some() {
            (self.shown.is_empty()).then(|| if self.entries.is_empty() { "The file has no log entries" } else { "All logs entries are hidden by the filter" }.to_owned())
        } else if self.following.is_none() {
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
            .child(
                h_flex()
                    .h(px(32.))
                    .px_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .when(!self.embedded, |el| el.child(div().px_1().text_sm().font_weight(FontWeight::SEMIBOLD).child("Logcat")))
                    .child(device_combo)
                    .child(filter_field)
                    .when(!self.embedded, |el| el.child(tool_button("logcat-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(LogcatEvent::Hide))))),
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
                        .when_some(empty_reason.clone(), |el, reason| el.child(div().p_4().text_color(palette.text_secondary).child(reason)))
                        .when(empty_reason.is_none(), |el| el.child(self.console.clone())),
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

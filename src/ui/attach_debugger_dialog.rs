//! Run › Attach Debugger to Android Process: Android Studio's "Choose
//! Process" dialog (`AndroidProcessChooserDialog`). Each online device
//! with its debuggable processes under it (the project's only, unless
//! "Show all processes"), refreshed while the dialog is open; the last
//! choice is selected again. OK (or a double click) starts a debug
//! session on the process.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use gpui_kit::component::{
    Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::android::jdwp::{self, Process};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::device_manager::DeviceManager;

/// What Android Studio keeps in the project's properties: the last device
/// and process, and "Show all processes".
#[derive(Default)]
struct Remembered {
    device: Option<String>,
    process: Option<String>,
    show_all: bool,
}

static REMEMBERED: LazyLock<Mutex<Remembered>> = LazyLock::new(Mutex::default);

struct DeviceRow {
    serial: String,
    name: String,
    detail: String,
    emulator: bool,
    processes: Vec<Process>,
}

pub struct AttachDebuggerView {
    devices: Entity<DeviceManager>,
    /// The project's application IDs, lower case: its processes start with one.
    app_ids: Vec<String>,
    show_all: bool,
    rows: Vec<DeviceRow>,
    /// (serial, pid)
    selected: Option<(String, u32)>,
    /// What a process told about itself, by (serial, pid).
    known: HashMap<(String, u32), Process>,
    loaded: bool,
    _poll: Task<()>,
}

impl AttachDebuggerView {
    fn new(devices: Entity<DeviceManager>, project: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let app_ids = project.map(|root| crate::android::application_ids(&root)).unwrap_or_default().into_iter().map(|id| id.to_lowercase()).collect();
        let show_all = REMEMBERED.lock().unwrap().show_all;
        // The device list follows adb while the dialog is open.
        devices.update(cx, |d, cx| d.hold(true, cx));
        cx.on_release({
            let devices = devices.clone();
            move |_, cx| devices.update(cx, |d, cx| d.hold(false, cx))
        })
        .detach();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                let Ok(targets) = this.update(cx, |this, cx| this.targets(cx)) else { return };
                let (adb, devices, known) = targets;
                let found = cx
                    .background_spawn(async move {
                        let Some(adb) = adb else { return Vec::new() };
                        devices
                            .into_iter()
                            .map(|(serial, row)| {
                                let pids = jdwp::pids(&adb, &serial).unwrap_or_default();
                                let processes = pids
                                    .into_iter()
                                    .map(|pid| known.get(&(serial.clone(), pid)).cloned().unwrap_or_else(|| jdwp::hello(&adb, &serial, pid).unwrap_or(Process { pid, ..Default::default() })))
                                    .collect();
                                DeviceRow { processes, ..row }
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                if this.update(cx, |this, cx| this.update_rows(found, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        });
        Self { devices, app_ids, show_all, rows: Vec::new(), selected: None, known: HashMap::new(), loaded: false, _poll: poll }
    }

    /// The adb, the online devices and the processes already described.
    #[allow(clippy::type_complexity)]
    fn targets(&self, cx: &App) -> (Option<crate::android::adb::Adb>, Vec<(String, DeviceRow)>, HashMap<(String, u32), Process>) {
        let manager = self.devices.read(cx);
        let devices = manager
            .devices()
            .iter()
            .filter(|d| d.online)
            .filter_map(|d| {
                let serial = d.serial.clone()?;
                let release = d.props.get("ro.build.version.release").cloned().unwrap_or_default();
                let detail = match d.api {
                    Some(_) if !release.is_empty() => format!("Android {release}, API {}", d.api_label()),
                    Some(_) => format!("API {}", d.api_label()),
                    None => String::new(),
                };
                let row = DeviceRow { serial: serial.clone(), name: d.name.clone(), detail, emulator: d.avd.is_some() || serial.starts_with("emulator-"), processes: Vec::new() };
                Some((serial, row))
            })
            .collect();
        (manager.adb(), devices, self.known.clone())
    }

    fn update_rows(&mut self, rows: Vec<DeviceRow>, cx: &mut Context<Self>) {
        self.known = rows.iter().flat_map(|r| r.processes.iter().filter(|p| !p.pid_only()).map(|p| ((r.serial.clone(), p.pid), p.clone()))).collect();
        self.rows = rows;
        self.loaded = true;
        let visible = self.visible();
        // A process that went away leaves nothing selected; with nothing
        // selected, the last choice (else the first process), as Android
        // Studio selects it.
        if self.selected.as_ref().is_some_and(|(s, pid)| !visible.iter().any(|(serial, p)| serial == s && p.pid == *pid)) {
            self.selected = None;
        }
        if self.selected.is_none() {
            let remembered = REMEMBERED.lock().unwrap();
            let last = visible.iter().find(|(serial, p)| p.name.is_some() && p.name == remembered.process && remembered.device.as_ref().is_none_or(|d| d == serial));
            self.selected = last.or(visible.first()).map(|(serial, p)| (serial.clone(), p.pid));
        }
        cx.notify();
    }

    fn related(&self, p: &Process) -> bool {
        let name = p.display_name().to_lowercase();
        self.app_ids.iter().any(|id| name.starts_with(id.as_str()))
    }

    /// (serial, process) in the tree's order: processes by name.
    fn visible(&self) -> Vec<(String, Process)> {
        let mut out = Vec::new();
        for row in &self.rows {
            let mut processes: Vec<&Process> = row.processes.iter().filter(|p| self.show_all || self.related(p)).collect();
            processes.sort_by_key(|p| p.display_name());
            out.extend(processes.into_iter().map(|p| (row.serial.clone(), p.clone())));
        }
        out
    }

    fn chosen(&self) -> Option<(String, Process)> {
        let (serial, pid) = self.selected.clone()?;
        self.rows.iter().find(|r| r.serial == serial)?.processes.iter().find(|p| p.pid == pid).map(|p| (serial, p.clone()))
    }

    /// OK: the debugger attaches; the choice is remembered.
    fn attach(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((serial, process)) = self.chosen() else { return false };
        let Some(adb) = self.devices.read(cx).adb() else { return false };
        {
            let mut remembered = REMEMBERED.lock().unwrap();
            remembered.device = Some(serial.clone());
            remembered.process = process.name.clone();
            remembered.show_all = self.show_all;
        }
        let (name, pid) = (process.display_name(), process.pid);
        crate::ui::debug_service::DebugService::global(cx).update(cx, |d, cx| d.attach_android(name, adb, serial, pid, cx));
        true
    }
}

impl Render for AttachDebuggerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut tree = v_flex().py_1();
        let mut seen = HashSet::new();
        for row in &self.rows {
            seen.insert(row.serial.clone());
            tree = tree.child(
                h_flex()
                    .h(px(24.))
                    .px_2()
                    .gap_1p5()
                    .child(Icon::new(if row.emulator { icons::VIRTUAL_DEVICE_PHONE } else { icons::PHYSICAL_DEVICE_PHONE }).small())
                    .child(row.name.clone())
                    .child(div().text_color(palette.text_secondary).child(row.detail.clone())),
            );
            let mut processes: Vec<&Process> = row.processes.iter().filter(|p| self.show_all || self.related(p)).collect();
            processes.sort_by_key(|p| p.display_name());
            for p in processes {
                let key = (row.serial.clone(), p.pid);
                let selected = self.selected.as_ref() == Some(&key);
                let user = p.user_id.filter(|u| *u != 0);
                tree = tree.child(
                    h_flex()
                        .id(SharedString::from(format!("attach-{}-{}", row.serial, p.pid)))
                        .h(px(24.))
                        .pl(px(30.))
                        .pr_2()
                        .gap_1p5()
                        .when(selected, |el| el.bg(palette.selection))
                        .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                        .child(p.display_name())
                        .when_some(user, |el, u| el.child(div().text_color(palette.text_secondary).child(format!("(user {u})"))))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.selected = Some(key.clone());
                            cx.notify();
                            if e.click_count() == 2 && this.attach(cx) {
                                window.close_dialog(cx);
                            }
                        })),
                );
            }
        }
        let empty = if !self.loaded {
            Some("Loading…")
        } else if self.rows.is_empty() {
            Some("No connected devices")
        } else {
            None
        };
        let combo = |id: &'static str, label: &'static str| {
            Button::new(id).outline().small().label(label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |menu, _, _| menu.item(PopupMenuItem::new(label).checked(true)))
        };
        let _ = window;
        v_flex()
            .gap_2()
            .child("Select a process to attach to:")
            .child(
                div()
                    .id("attach-processes")
                    .h(px(260.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.panel)
                    .child(tree)
                    .when_some(empty, |el, text| el.child(div().p_2().text_color(palette.text_secondary).child(text))),
            )
            .child(Checkbox::new("attach-show-all").label("Show all processes").checked(self.show_all).on_change(cx.listener(|this, value: &bool, _, cx| {
                this.show_all = *value;
                REMEMBERED.lock().unwrap().show_all = *value;
                if this.selected.is_none() {
                    this.selected = this.visible().first().map(|(s, p)| (s.clone(), p.pid));
                }
                cx.notify();
            })))
            // Junction debugs Java / Kotlin only: one choice each, as AS
            // shows them with no run configuration of its own.
            .child(h_flex().gap_2().child(div().w(px(250.)).child("Use Android Debugger Settings from:")).child(combo("attach-config", "[Use default settings]")))
            .child(h_flex().gap_2().child(div().w(px(250.)).child("Debug Type:")).child(combo("attach-debugger", "Java Only")))
    }
}

/// Opens the dialog.
pub fn open(devices: Entity<DeviceManager>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut App) {
    let root = project(cx);
    let view = cx.new(|cx| AttachDebuggerView::new(devices, root, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let enabled = view.read(cx).chosen().is_some();
        let ok = view.clone();
        dialog.title("Choose Process").w(px(520.)).child(view.clone()).footer(crate::ui::dialogs::footer_enabled("OK", enabled)).on_ok(move |_, _, cx| ok.update(cx, |v, cx| v.attach(cx)))
    });
}

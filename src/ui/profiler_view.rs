//! The Profiler tool window, as Android Studio's live session: a device and
//! process picker, Start / Stop, the CPU timeline (the app's share of the
//! CPU and its threads) and the Memory timeline stacked by category (Java,
//! Native, Graphics, Stack, Code, Others). Capture heap dump saves an
//! `.hprof` and lists its classes with their Allocations, Shallow Size and
//! Retained Size; a class's Instance List (Depth, Shallow and Retained Size)
//! and an instance's References, expandable toward the GC roots. Record
//! takes a Java/Kotlin Method Sample or Trace and shows its threads' calls
//! as Call Chart, Flame Chart, Top Down and Bottom Up, or a System Trace
//! (Perfetto) as CPU and thread timelines.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    StatefulInteractiveElement as _, Subscription, Window, canvas, div, point, prelude::FluentBuilder as _, px, relative, size, uniform_list,
};

use crate::domain::hprof::{HeapSummary, Instance};
use crate::domain::method_trace::{CallTree, Clock, MethodTrace};
use crate::usecase::profile_app::TraceMode;
use crate::domain::profiler::{ProfiledProcess, Sample};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;
use crate::ui::system_trace_view::SystemTraceView;

pub enum ProfilerEvent {
    Hide,
    Notify { title: String, message: String, error: bool },
}

const TICK: Duration = Duration::from_secs(1);
/// Points kept on the timelines: two minutes at a point a second.
const POINTS: usize = 120;
const ROW_H: f32 = 22.;

#[derive(Clone)]
struct Target {
    serial: String,
    name: String,
}

/// A heap dump on screen: its file, classes and the heap shown, and the
/// class and instance picked.
struct HeapView {
    file: PathBuf,
    summary: HeapSummary,
    heap: String,
    class: Option<usize>,
    instances: Vec<Instance>,
    instance: Option<u32>,
    refs: Vec<RefRow>,
    /// Expanded reference rows, by their path from the picked instance.
    expanded: HashSet<Vec<u32>>,
}

/// A row of the References tree: `field` of `object` points at the row
/// above it (or at the picked instance).
#[derive(Clone)]
struct RefRow {
    path: Vec<u32>,
    field: String,
    object: u32,
    expandable: bool,
}

/// Referrers listed under one object; the rest are summarized.
const MAX_REFERRERS: usize = 200;

impl HeapView {
    fn new(file: PathBuf, summary: HeapSummary, heap: String) -> Self {
        Self { file, summary, heap, class: None, instances: Vec::new(), instance: None, refs: Vec::new(), expanded: HashSet::new() }
    }

    fn pick_class(&mut self, row: Option<usize>) {
        self.class = row;
        self.instances = row.map(|r| self.summary.graph.instances(r)).unwrap_or_default();
        self.pick_instance(None);
    }

    fn pick_instance(&mut self, object: Option<u32>) {
        self.instance = object;
        self.expanded.clear();
        self.refresh_refs();
    }

    fn refresh_refs(&mut self) {
        fn walk(graph: &crate::domain::hprof::HeapGraph, path: &mut Vec<u32>, expanded: &HashSet<Vec<u32>>, out: &mut Vec<RefRow>) {
            let Some(&object) = path.last() else { return };
            for r in graph.referrers(object).into_iter().take(MAX_REFERRERS) {
                // A referrer already on the path would loop.
                let cycle = path.contains(&r.object);
                path.push(r.object);
                let expandable = !cycle && !graph.referrers(r.object).is_empty();
                out.push(RefRow { path: path[1..].to_vec(), field: r.field, object: r.object, expandable });
                if expandable && expanded.contains(&path[1..]) {
                    walk(graph, path, expanded, out);
                }
                path.pop();
            }
        }
        self.refs.clear();
        if let Some(object) = self.instance {
            walk(&self.summary.graph, &mut vec![object], &self.expanded, &mut self.refs);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraceTab {
    CallChart,
    TopDown,
    FlameChart,
    BottomUp,
}

/// A method trace on screen: the thread, tab and clock picked, and the
/// tree rows shown.
struct TraceView {
    file: PathBuf,
    trace: MethodTrace,
    thread: usize,
    tab: TraceTab,
    clock: Clock,
    /// The picked thread's Bottom Up tree, made when its tab opens.
    bottom_up: Option<CallTree>,
    expanded: HashSet<u32>,
    /// (node, depth) in the tree shown.
    rows: Vec<(u32, u32)>,
    hovered: Option<u32>,
    /// The Call Chart's visible time range; `None` shows the whole thread.
    range: Option<(u64, u64)>,
    /// The Call Chart's hovered call, by its index.
    hovered_call: Option<usize>,
}

impl TraceView {
    fn new(file: PathBuf, trace: MethodTrace) -> Self {
        let clock = if trace.wall { Clock::Wall } else { Clock::Thread };
        let mut view = Self { file, trace, thread: 0, tab: TraceTab::CallChart, clock, bottom_up: None, expanded: HashSet::new(), rows: Vec::new(), hovered: None, range: None, hovered_call: None };
        view.reset();
        view
    }

    fn tree(&self) -> &CallTree {
        match (&self.bottom_up, self.tab) {
            (Some(up), TraceTab::BottomUp) => up,
            _ => &self.trace.threads[self.thread].tree,
        }
    }

    /// After picking a thread, tab or clock: Top Down opens along its
    /// heaviest path, Bottom Up closed.
    fn reset(&mut self) {
        self.hovered = None;
        self.hovered_call = None;
        self.range = None;
        self.bottom_up = None;
        if self.trace.threads.is_empty() {
            self.rows.clear();
            return;
        }
        if self.tab == TraceTab::BottomUp {
            self.bottom_up = Some(self.trace.threads[self.thread].tree.bottom_up());
            self.expanded.clear();
        } else {
            let mut path = self.tree().heaviest_path(self.clock);
            path.pop();
            self.expanded = path.into_iter().collect();
        }
        self.refresh_rows();
    }

    /// The picked thread's whole time span on the picked clock.
    fn span(&self) -> (u64, u64) {
        let t = &self.trace.threads[self.thread];
        let c = self.clock as usize;
        (t.span[0][c], t.span[1][c].max(t.span[0][c] + 1))
    }

    fn visible(&self) -> (u64, u64) {
        self.range.unwrap_or_else(|| self.span())
    }

    /// Narrows (`factor` < 1) or widens the Call Chart around its middle,
    /// within the thread's span.
    fn zoom(&mut self, factor: f64) {
        let (lo, hi) = self.span();
        let (start, end) = self.visible();
        let middle = (start + end) as f64 / 2.;
        let width = ((end - start) as f64 * factor).clamp(10., (hi - lo) as f64);
        self.set_range(middle - width / 2., width);
    }

    /// Moves the Call Chart by `fraction` of its width.
    fn pan(&mut self, fraction: f64) {
        let (start, end) = self.visible();
        let width = (end - start) as f64;
        self.set_range(start as f64 + width * fraction, width);
    }

    fn set_range(&mut self, start: f64, width: f64) {
        let (lo, hi) = self.span();
        let start = start.clamp(lo as f64, (hi as f64 - width).max(lo as f64));
        let range = (start as u64, (start + width) as u64);
        self.range = if range.0 <= lo && range.1 >= hi { None } else { Some(range) };
    }

    fn refresh_rows(&mut self) {
        fn walk(tree: &CallTree, node: u32, depth: u32, clock: Clock, expanded: &HashSet<u32>, out: &mut Vec<(u32, u32)>) {
            for child in tree.sorted_children(node, clock) {
                out.push((child, depth));
                if expanded.contains(&child) {
                    walk(tree, child, depth + 1, clock, expanded, out);
                }
            }
        }
        let mut rows = Vec::new();
        if !self.trace.threads.is_empty() {
            walk(self.tree(), 0, 0, self.clock, &self.expanded, &mut rows);
        }
        self.rows = rows;
    }
}

/// Microseconds as Android Studio writes them: `12,345`.
fn micros(us: u64) -> String {
    let digits = us.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 { String::new() } else { format!("{:.2}", part as f64 * 100. / whole as f64) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Recording {
    Idle,
    Starting,
    Running,
    Stopping,
}

pub struct ProfilerView {
    device_manager: Entity<DeviceManager>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    devices: Vec<Target>,
    active: Option<String>,
    processes: Vec<ProfiledProcess>,
    app_ids: Vec<String>,
    process: Option<ProfiledProcess>,
    /// The running session's samples; `None` when stopped.
    session: Option<Arc<Mutex<VecDeque<Sample>>>>,
    samples: VecDeque<Sample>,
    dumping: bool,
    error: Option<String>,
    heap: Option<HeapView>,
    trace_mode: TraceMode,
    recording: Recording,
    /// The process being recorded.
    recorded: Option<ProfiledProcess>,
    trace: Option<TraceView>,
    /// The System Trace recording on the device.
    system_recording: Option<u32>,
    system_trace: Option<Entity<SystemTraceView>>,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProfilerEvent> for ProfilerView {}

fn color(hex: u32) -> Hsla {
    gpui_kit::rgb(hex).into()
}

/// Android Studio's memory category colors, bottom to top.
const CATEGORY_COLORS: [u32; 6] = [0xec407a, 0x4fc3f7, 0x9ccc65, 0xffca28, 0x8d6e63, 0x9e9e9e];
const CPU_COLOR: u32 = 0x66bb6a;

fn mb(kb: u64) -> String {
    format!("{:.1} MB", kb as f64 / 1024.)
}

impl ProfilerView {
    pub fn new(device_manager: Entity<DeviceManager>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&device_manager, |this, dm, cx| {
            this.devices = dm.read(cx).devices().iter().filter(|d| d.online).filter_map(|d| Some(Target { serial: d.serial.clone()?, name: d.name.clone() })).collect();
            if this.active.as_ref().is_none_or(|s| !this.devices.iter().any(|d| &d.serial == s)) {
                this.active = this.devices.first().map(|d| d.serial.clone());
                this.load_processes(cx);
            }
            cx.notify();
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search classes"));
        let filtered = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            device_manager,
            project,
            devices: Vec::new(),
            active: None,
            processes: Vec::new(),
            app_ids: Vec::new(),
            process: None,
            session: None,
            samples: VecDeque::new(),
            dumping: false,
            error: None,
            heap: None,
            trace_mode: TraceMode::Sample,
            recording: Recording::Idle,
            recorded: None,
            trace: None,
            system_recording: None,
            system_trace: None,
            filter,
            _subscriptions: vec![observe, filtered],
        }
    }

    fn adb(&self, cx: &App) -> Option<crate::android::adb::Adb> {
        self.device_manager.read(cx).adb()
    }

    /// The device's app processes; the project's app picked first.
    pub fn load_processes(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb)) = (self.active.clone(), self.adb(cx)) else { return };
        let project = (self.project)(cx);
        let task = cx.background_spawn(async move {
            let ids = project.map(|p| crate::android::application_ids(&p)).unwrap_or_default();
            let source = crate::infra::adb_profiler::AdbProfiler { adb };
            (crate::domain::profiler::ProfilerSource::processes(&source, &serial), ids)
        });
        cx.spawn(async move |this, cx| {
            let (result, ids) = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        this.processes = list;
                        this.app_ids = ids;
                        // Project apps first.
                        let ids = this.app_ids.clone();
                        this.processes.sort_by_key(|p| !ids.iter().any(|id| p.name == *id || p.name.starts_with(&format!("{id}:"))));
                        if this.process.as_ref().is_none_or(|p| !this.processes.contains(p)) {
                            this.process = this.processes.iter().find(|p| ids.contains(&p.name)).cloned();
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(process)) = (self.active.clone(), self.adb(cx), self.process.clone()) else { return };
        let samples: Arc<Mutex<VecDeque<Sample>>> = Arc::default();
        self.session = Some(samples.clone());
        self.samples.clear();
        self.error = None;
        let started = Instant::now();
        // The sampler runs on its own thread until the session is dropped;
        // the view copies what it has each tick.
        let session = Arc::downgrade(&samples);
        std::thread::spawn(move || {
            let source = crate::infra::adb_profiler::AdbProfiler { adb };
            let mut sampler = crate::usecase::profile_app::Sampler::default();
            loop {
                let tick = Instant::now();
                let sample = sampler.sample(&source, &serial, process.pid, started.elapsed().as_secs_f32());
                let Some(shared) = session.upgrade() else { break };
                {
                    let mut list = shared.lock().unwrap();
                    list.push_back(sample);
                    while list.len() > POINTS {
                        list.pop_front();
                    }
                }
                drop(shared);
                std::thread::sleep(TICK.saturating_sub(tick.elapsed()));
            }
        });
        drop(samples);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let running = this
                    .update(cx, |this, cx| {
                        let Some(session) = &this.session else { return false };
                        this.samples = session.lock().unwrap().clone();
                        // A sample without memory: the process is gone.
                        if this.samples.len() >= 3 && this.samples.iter().rev().take(3).all(|s| s.memory.is_none() && s.cpu.is_none()) {
                            this.error = Some(format!("{} is no longer running", this.process.as_ref().map(|p| p.name.as_str()).unwrap_or("The process")));
                            this.session = None;
                        }
                        cx.notify();
                        this.session.is_some()
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        // The sampler thread stops when it finds the session dropped.
        self.session = None;
        cx.notify();
    }

    fn capture_heap_dump(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(process)) = (self.active.clone(), self.adb(cx), self.process.clone()) else { return };
        if self.dumping {
            return;
        }
        self.dumping = true;
        let name = process.name.replace([':', '/'], "_");
        let file = crate::index::libraries::cache_dir().unwrap_or_else(std::env::temp_dir).join("captures").join(format!("{name}_{}.hprof", chrono::Local::now().format("%Y-%m-%d_%H.%M.%S")));
        let to = file.clone();
        let task = cx.background_spawn(async move { crate::usecase::profile_app::capture_heap_dump(&crate::infra::adb_profiler::AdbProfiler { adb }, &serial, process.pid, &to) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.dumping = false;
                match result {
                    Ok(summary) => {
                        let heap = if summary.heaps.iter().any(|h| h == "app") { "app".to_owned() } else { summary.heaps.first().cloned().unwrap_or_default() };
                        cx.emit(ProfilerEvent::Notify { title: "Heap Dump".into(), message: format!("Saved to {}", file.display()), error: false });
                        this.heap = Some(HeapView::new(file, summary, heap));
                        this.trace = None;
                        this.system_trace = None;
                    }
                    Err(e) => cx.emit(ProfilerEvent::Notify { title: "Heap Dump".into(), message: format!("Cannot capture a heap dump: {e} (the app must be debuggable)"), error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(process)) = (self.active.clone(), self.adb(cx), self.process.clone()) else { return };
        if self.recording != Recording::Idle {
            return;
        }
        self.recording = Recording::Starting;
        self.recorded = Some(process.clone());
        let mode = self.trace_mode;
        let task = cx.background_spawn(async move {
            let source = crate::infra::adb_profiler::AdbProfiler { adb };
            match mode {
                TraceMode::System => crate::usecase::profile_app::start_system_trace(&source, &serial, &process.name).map(Some),
                _ => crate::usecase::profile_app::start_method_trace(&source, &serial, process.pid, mode).map(|()| None),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(id) => {
                        this.system_recording = id;
                        this.recording = Recording::Running;
                    }
                    Err(e) => {
                        this.recording = Recording::Idle;
                        let hint = if mode == TraceMode::System { "System Trace needs perfetto: Android 9 or later" } else { "the app must be debuggable or profileable" };
                        cx.emit(ProfilerEvent::Notify { title: "CPU Recording".into(), message: format!("Cannot start recording: {e} ({hint})"), error: true });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(process)) = (self.active.clone(), self.adb(cx), self.recorded.clone()) else { return };
        if self.recording != Recording::Running {
            return;
        }
        self.recording = Recording::Stopping;
        let name = process.name.replace([':', '/'], "_");
        let system = self.system_recording.take();
        let extension = if system.is_some() { "pftrace" } else { "trace" };
        let file = crate::index::libraries::cache_dir().unwrap_or_else(std::env::temp_dir).join("captures").join(format!("{name}_{}.{extension}", chrono::Local::now().format("%Y-%m-%d_%H.%M.%S")));
        if let Some(id) = system {
            let to = file.clone();
            let task = cx.background_spawn(async move { crate::usecase::profile_app::stop_system_trace(&crate::infra::adb_profiler::AdbProfiler { adb }, &serial, id, process.pid, &to) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    this.recording = Recording::Idle;
                    match result {
                        Ok(trace) => {
                            cx.emit(ProfilerEvent::Notify { title: "System Trace".into(), message: format!("Saved to {}", file.display()), error: false });
                            this.system_trace = Some(cx.new(|_| SystemTraceView::new(file, trace)));
                            this.trace = None;
                            this.heap = None;
                        }
                        Err(e) => cx.emit(ProfilerEvent::Notify { title: "System Trace".into(), message: format!("Cannot record the system trace: {e}"), error: true }),
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
            cx.notify();
            return;
        }
        let to = file.clone();
        let task = cx.background_spawn(async move { crate::usecase::profile_app::stop_method_trace(&crate::infra::adb_profiler::AdbProfiler { adb }, &serial, process.pid, &to) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.recording = Recording::Idle;
                match result {
                    Ok(trace) => {
                        cx.emit(ProfilerEvent::Notify { title: "CPU Recording".into(), message: format!("Saved to {}", file.display()), error: false });
                        this.trace = Some(TraceView::new(file, trace));
                        this.heap = None;
                        this.system_trace = None;
                    }
                    Err(e) => cx.emit(ProfilerEvent::Notify { title: "CPU Recording".into(), message: format!("Cannot record the method trace: {e}"), error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn render_trace_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let Some(view) = &self.trace else { return Vec::new() };
        let tree = view.tree();
        let (clock, c) = (view.clock, view.clock as usize);
        let whole = view.trace.threads[view.thread].tree.total(clock);
        view.rows[range.start.min(view.rows.len())..range.end.min(view.rows.len())]
            .iter()
            .map(|&(node, depth)| {
                let n = &tree.nodes[node as usize];
                let expandable = !n.children.is_empty();
                let expanded = view.expanded.contains(&node);
                let (total, own) = (n.total[c], n.own[c]);
                let children = total.saturating_sub(own);
                let cell = |w: f32, text: String| div().w(px(w)).flex_none().text_right().child(text);
                h_flex()
                    .id(("pf-trace", node as usize))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .text_sm()
                    .hover(|el| el.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(view) = &mut this.trace {
                            if expandable && !view.expanded.remove(&node) {
                                view.expanded.insert(node);
                            }
                            view.refresh_rows();
                        }
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .pl(px(depth as f32 * 16.))
                            .child(div().w(px(14.)).flex_none().when(expandable, |el| el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(view.trace.label(n.method))),
                    )
                    .child(cell(100., micros(total)))
                    .child(cell(60., percent(total, whole)))
                    .child(cell(100., micros(own)))
                    .child(cell(60., percent(own, whole)))
                    .child(cell(100., micros(children)))
                    .child(cell(60., percent(children, whole)))
                    .into_any_element()
            })
            .collect()
    }

    /// Green for the app's code, orange for the platform's, blue for the
    /// rest, as Android Studio colors its charts.
    fn method_color(&self, trace: &MethodTrace, method: u32) -> Hsla {
        const SYSTEM: [&str; 11] = ["android.", "androidx.", "com.android.", "dalvik.", "libcore.", "java.", "javax.", "sun.", "kotlin.", "kotlinx.", "org.apache.harmony."];
        let class = trace.methods.get(method as usize).map(|m| m.class.as_str()).unwrap_or_default();
        if self.app_ids.iter().any(|id| class.starts_with(&format!("{id}."))) {
            color(0x81c784)
        } else if SYSTEM.iter().any(|p| class.starts_with(p)) {
            color(0xffb74d)
        } else {
            color(0x64b5f6)
        }
    }

    /// The Call Chart: each call where and as long as it ran, callers above
    /// their callees. Ctrl + wheel zooms, Shift + wheel (or a sideways
    /// wheel) pans, and clicking a call zooms to it.
    fn render_call_chart(&self, view: &TraceView, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        const BAR: f32 = 18.;
        const MAX_BARS: usize = 5000;
        let thread = &view.trace.threads[view.thread];
        let c = view.clock as usize;
        let (start, end) = view.visible();
        let width = (end - start) as f64;
        let mut depth_max = 0;
        let mut bars = Vec::new();
        for (i, call) in thread.calls.iter().enumerate() {
            if call.end[c] < start || call.start[c] > end {
                continue;
            }
            let (s, e) = (call.start[c].max(start), call.end[c].min(end));
            let w = e.saturating_sub(s) as f64 / width;
            depth_max = depth_max.max(call.depth + 1);
            if w < 0.0005 || bars.len() >= MAX_BARS {
                continue;
            }
            bars.push((i, call.method, call.depth, (s - start) as f64 / width, w));
        }
        let hovered = view.hovered_call.and_then(|i| thread.calls.get(i)).map(|call| {
            let d = call.end[c] - call.start[c];
            format!("{}   {} μs   {} – {} μs", view.trace.label(call.method), micros(d), micros(call.start[c] - thread.span[0][c]), micros(call.end[c] - thread.span[0][c]))
        });
        let (lo, _) = view.span();
        let caption = format!("{} – {} μs", micros(start - lo), micros(end - lo));
        let chart = div().relative().w_full().h(px(depth_max as f32 * BAR)).children(bars.into_iter().map(|(i, method, depth, x, w)| {
            div()
                .id(("pf-call", i))
                .absolute()
                .left(relative(x as f32))
                .w(relative(w as f32))
                .top(px(depth as f32 * BAR))
                .h(px(BAR - 1.))
                .border_r_1()
                .border_color(palette.panel)
                .bg(self.method_color(&view.trace, method))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_xs()
                .text_color(gpui_kit::black())
                // Padding inside the label, so a sliver of a call stays a sliver.
                .child(div().pl_1().child(view.trace.label(method)))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    if let Some(view) = &mut this.trace {
                        if *over {
                            view.hovered_call = Some(i);
                        } else if view.hovered_call == Some(i) {
                            view.hovered_call = None;
                        }
                    }
                    cx.notify();
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(view) = &mut this.trace {
                        let call = view.trace.threads[view.thread].calls[i];
                        let c = view.clock as usize;
                        let width = (call.end[c] - call.start[c]).max(10) as f64;
                        // A little room either side, as Android Studio leaves.
                        view.set_range(call.start[c] as f64 - width * 0.05, width * 1.1);
                    }
                    cx.notify();
                }))
        }));
        let zoomed = view.range.is_some();
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(ROW_H))
                    .flex_none()
                    .px_2()
                    .gap_2()
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .child(Button::new("pf-zoom-in").ghost().xsmall().label("Zoom In").on_click(cx.listener(|this, _, _, cx| {
                        if let Some(v) = &mut this.trace {
                            v.zoom(0.5);
                        }
                        cx.notify();
                    })))
                    .child(Button::new("pf-zoom-out").ghost().xsmall().label("Zoom Out").disabled(!zoomed).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(v) = &mut this.trace {
                            v.zoom(2.);
                        }
                        cx.notify();
                    })))
                    .child(Button::new("pf-zoom-reset").ghost().xsmall().label("Reset Zoom").disabled(!zoomed).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(v) = &mut this.trace {
                            v.range = None;
                        }
                        cx.notify();
                    })))
                    .child(div().flex_none().child(caption))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(hovered.unwrap_or_else(|| "Hover over a call to see its time; click it to zoom in".into()))),
            )
            .child(
                v_flex()
                    .id("pf-call-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .on_scroll_wheel(cx.listener(|this, e: &gpui_kit::ScrollWheelEvent, _, cx| {
                        let delta = e.delta.pixel_delta(px(ROW_H));
                        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                        let Some(v) = &mut this.trace else { return };
                        if e.modifiers.control || e.modifiers.platform {
                            v.zoom(if dy > 0. { 0.8 } else { 1.25 });
                        } else if e.modifiers.shift || dx.abs() > dy.abs() {
                            let along = if dx.abs() > dy.abs() { dx } else { dy };
                            v.pan(-along as f64 / 1000.);
                        } else {
                            return;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(chart),
            )
    }

    /// The Flame Chart: the Top Down tree from the bottom up, each call
    /// path as wide as its share of the thread's time, longest first.
    /// Green is the app's code, orange the platform's, blue the rest.
    fn render_flame_chart(&self, view: &TraceView, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        const BAR: f32 = 18.;
        const MAX_BARS: usize = 4000;
        let tree = view.tree();
        let whole = tree.total(view.clock).max(1) as f64;
        let mut bars = Vec::new();
        let mut depth_max = 0;
        let mut stack = vec![(0u32, 0u32, 0f64)];
        while let Some((node, depth, x)) = stack.pop() {
            let mut at = x;
            for child in tree.sorted_children(node, view.clock) {
                let w = tree.nodes[child as usize].total[view.clock as usize] as f64 / whole;
                // Narrower than a pixel or so on a wide window.
                if w >= 0.0005 && bars.len() < MAX_BARS {
                    bars.push((child, depth, at, w));
                    depth_max = depth_max.max(depth + 1);
                    stack.push((child, depth + 1, at));
                }
                at += w;
            }
        }
        let hovered = view.hovered.map(|node| {
            let n = &tree.nodes[node as usize];
            let total = n.total[view.clock as usize];
            format!("{}   {} μs ({}%)", view.trace.label(n.method), micros(total), percent(total, tree.total(view.clock)))
        });
        let height = depth_max as f32 * BAR;
        let chart = div().relative().w_full().h(px(height)).children(bars.into_iter().map(|(node, depth, x, w)| {
            let method = tree.nodes[node as usize].method;
            div()
                .id(("pf-flame", node as usize))
                .absolute()
                .left(relative(x as f32))
                .w(relative(w as f32))
                .bottom(px(depth as f32 * BAR))
                .h(px(BAR - 1.))
                .border_r_1()
                .border_color(palette.panel)
                .bg(self.method_color(&view.trace, method))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_xs()
                .text_color(gpui_kit::black())
                // Padding inside the label, so a sliver of a call stays a sliver.
                .child(div().pl_1().child(view.trace.label(method)))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    if let Some(view) = &mut this.trace {
                        if *over {
                            view.hovered = Some(node);
                        } else if view.hovered == Some(node) {
                            view.hovered = None;
                        }
                    }
                    cx.notify();
                }))
        }));
        v_flex()
            .flex_1()
            .min_h_0()
            .child(div().h(px(ROW_H)).flex_none().px_2().text_xs().text_color(palette.text_secondary).overflow_hidden().whitespace_nowrap().child(hovered.unwrap_or_else(|| "Hover over a method to see its time".into())))
            .child(v_flex().id("pf-flame-scroll").flex_1().min_h_0().overflow_y_scroll().justify_end().px_2().child(chart))
    }

    fn render_trace(&self, view: &TraceView, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let threads: Vec<(usize, String)> = view.trace.threads.iter().enumerate().map(|(i, t)| (i, format!("{} ({})", t.name, t.id))).collect();
        let thread_label = threads.get(view.thread).map(|t| t.1.clone()).unwrap_or_else(|| "No threads".into());
        let thread_picker = Button::new("pf-trace-thread").outline().xsmall().label(thread_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for (i, name) in &threads {
                    let (entity, i) = (entity.clone(), *i);
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(v) = &mut this.trace {
                                v.thread = i;
                                v.reset();
                            }
                            cx.notify();
                        })
                    }));
                }
                menu
            }
        });
        let (wall, thread_time) = (view.trace.wall, view.trace.thread_time);
        let clock_label = if view.clock == Clock::Wall { "Wall Clock Time" } else { "Thread Time" };
        let clock_picker = Button::new("pf-trace-clock").outline().xsmall().label(clock_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for (clock, label, on) in [(Clock::Wall, "Wall Clock Time", wall), (Clock::Thread, "Thread Time", thread_time)] {
                    let entity = entity.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(!on).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(v) = &mut this.trace {
                                v.clock = clock;
                                v.reset();
                            }
                            cx.notify();
                        })
                    }));
                }
                menu
            }
        });
        let tab = |id: &'static str, label: &'static str, which: TraceTab| {
            Button::new(id).ghost().xsmall().label(label).selected(view.tab == which).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(v) = &mut this.trace {
                    if v.tab != which {
                        v.tab = which;
                        v.reset();
                    }
                }
                cx.notify();
            }))
        };
        let total = view.trace.threads.get(view.thread).map(|t| t.tree.total(view.clock)).unwrap_or(0);
        let head = |w: f32, text: &'static str| div().w(px(w)).flex_none().text_right().child(text);
        let columns = h_flex()
            .h(px(ROW_H))
            .flex_none()
            .px_2()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w_0().child("Name"))
            .child(head(100., "Total (μs)"))
            .child(head(60., "%"))
            .child(head(100., "Self (μs)"))
            .child(head(60., "%"))
            .child(head(100., "Children (μs)"))
            .child(head(60., "%"));
        v_flex()
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_sm().child("Method Trace"))
                    .child(thread_picker)
                    .child(clock_picker)
                    .child(tab("pf-tab-call", "Call Chart", TraceTab::CallChart))
                    .child(tab("pf-tab-flame", "Flame Chart", TraceTab::FlameChart))
                    .child(tab("pf-tab-top", "Top Down", TraceTab::TopDown))
                    .child(tab("pf-tab-bottom", "Bottom Up", TraceTab::BottomUp))
                    .child(div().text_color(palette.text_secondary).child(format!("Thread {} μs of {} μs recorded, {} threads", micros(total), micros(view.trace.elapsed_us), view.trace.threads.len())))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(palette.text_secondary).child(view.file.display().to_string())),
            )
            .when(view.trace.threads.is_empty(), |el| el.child(div().p_2().text_sm().text_color(palette.text_secondary).child("The trace has no method calls")))
            .when(!view.trace.threads.is_empty() && view.tab == TraceTab::FlameChart, |el| el.child(self.render_flame_chart(view, palette, cx)))
            .when(!view.trace.threads.is_empty() && view.tab == TraceTab::CallChart, |el| el.child(self.render_call_chart(view, palette, cx)))
            .when(!view.trace.threads.is_empty() && matches!(view.tab, TraceTab::TopDown | TraceTab::BottomUp), |el| el.child(columns).child(uniform_list("pf-trace-rows", view.rows.len(), cx.processor(Self::render_trace_rows)).flex_1().min_h_0()))
    }

    /// The classes shown: the picked heap's, matching the search.
    fn classes(&self, cx: &App) -> Vec<usize> {
        let Some(heap) = &self.heap else { return Vec::new() };
        let query = self.filter.read(cx).value().trim().to_lowercase();
        heap.summary.classes.iter().enumerate().filter(|(_, c)| c.heap == heap.heap && (query.is_empty() || c.name.to_lowercase().contains(&query))).map(|(i, _)| i).collect()
    }

    fn render_class_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let shown = self.classes(cx);
        let Some(heap) = &self.heap else { return Vec::new() };
        shown[range.start.min(shown.len())..range.end.min(shown.len())]
            .iter()
            .map(|&i| {
                let c = &heap.summary.classes[i];
                let selected = heap.class == Some(i);
                h_flex()
                    .id(("pf-class", i))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .text_sm()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(h) = &mut this.heap {
                            h.pick_class(Some(i));
                        }
                        cx.notify();
                    }))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(c.name.clone()))
                    .child(div().w(px(110.)).flex_none().text_right().child(c.count.to_string()))
                    .child(div().w(px(130.)).flex_none().text_right().child(c.shallow.to_string()))
                    .child(div().w(px(130.)).flex_none().text_right().child(c.retained.to_string()))
                    .into_any_element()
            })
            .collect()
    }

    fn render_instance_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let Some(heap) = &self.heap else { return Vec::new() };
        let graph = &heap.summary.graph;
        heap.instances[range.start.min(heap.instances.len())..range.end.min(heap.instances.len())]
            .iter()
            .map(|inst| {
                let object = inst.object;
                let selected = heap.instance == Some(object);
                h_flex()
                    .id(("pf-instance", object as usize))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .text_sm()
                    .when(inst.depth.is_none(), |el| el.text_color(palette.text_secondary))
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(h) = &mut this.heap {
                            h.pick_instance(Some(object));
                        }
                        cx.notify();
                    }))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(graph.name(object)))
                    .child(div().w(px(60.)).flex_none().text_right().child(inst.depth.map_or_else(String::new, |d| d.to_string())))
                    .child(div().w(px(100.)).flex_none().text_right().child(inst.shallow.to_string()))
                    .child(div().w(px(110.)).flex_none().text_right().child(inst.retained.to_string()))
                    .into_any_element()
            })
            .collect()
    }

    fn render_ref_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let Some(heap) = &self.heap else { return Vec::new() };
        let graph = &heap.summary.graph;
        heap.refs[range.start.min(heap.refs.len())..range.end.min(heap.refs.len())]
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let inst = graph.instance(row.object);
                let expanded = heap.expanded.contains(&row.path);
                let path = row.path.clone();
                let expandable = row.expandable;
                h_flex()
                    .id(("pf-ref", range.start + i))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .text_sm()
                    .hover(|el| el.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(h) = &mut this.heap {
                            if expandable && !h.expanded.remove(&path) {
                                h.expanded.insert(path.clone());
                            }
                            h.refresh_refs();
                        }
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .pl(px((row.path.len() - 1) as f32 * 16.))
                            .child(div().w(px(14.)).flex_none().when(expandable, |el| el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(format!("{} in {}", row.field, graph.name(row.object))))
                            .when(graph.is_root(row.object), |el| el.child(div().flex_none().text_xs().text_color(palette.text_secondary).child("GC root"))),
                    )
                    .child(div().w(px(60.)).flex_none().text_right().child(inst.depth.map_or_else(String::new, |d| d.to_string())))
                    .child(div().w(px(100.)).flex_none().text_right().child(inst.shallow.to_string()))
                    .child(div().w(px(110.)).flex_none().text_right().child(inst.retained.to_string()))
                    .into_any_element()
            })
            .collect()
    }

    /// Instance List of the picked class, and the picked instance's References.
    fn render_instance_panel(&self, heap: &HeapView, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = |first: &'static str| {
            h_flex()
                .h(px(ROW_H))
                .flex_none()
                .px_2()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text_secondary)
                .border_b_1()
                .border_color(palette.border)
                .child(div().flex_1().min_w_0().child(first))
                .child(div().w(px(60.)).flex_none().text_right().child("Depth"))
                .child(div().w(px(100.)).flex_none().text_right().child("Shallow Size"))
                .child(div().w(px(110.)).flex_none().text_right().child("Retained Size"))
        };
        let title = |text: String| div().px_2().py_1().flex_none().text_xs().font_weight(FontWeight::SEMIBOLD).child(text);
        let class_name = heap.class.and_then(|c| heap.summary.classes.get(c)).map(|c| c.name.clone()).unwrap_or_default();
        v_flex()
            .w(relative(0.5))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(palette.border)
            .child(title(format!("Instance List: {class_name} ({})", heap.instances.len())))
            .child(columns("Instance"))
            .child(uniform_list("pf-instances", heap.instances.len(), cx.processor(Self::render_instance_rows)).flex_1().min_h_0())
            .when_some(heap.instance, |el, object| {
                let empty = heap.refs.is_empty();
                el.child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .border_t_1()
                        .border_color(palette.border)
                        .child(title(format!("References: {}", heap.summary.graph.name(object))))
                        .child(columns("Reference"))
                        .when(empty, |el| {
                            let why = if heap.summary.graph.is_root(object) { "A GC root; nothing else references it" } else { "Nothing reachable references it" };
                            el.child(div().p_2().text_sm().text_color(palette.text_secondary).child(why))
                        })
                        .when(!empty, |el| el.child(uniform_list("pf-refs", heap.refs.len(), cx.processor(Self::render_ref_rows)).flex_1().min_h_0())),
                )
            })
    }

    /// Shorter charts while a heap dump shares the window.
    fn render_charts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = self.heap.is_some() || self.trace.is_some() || self.system_trace.is_some();
        let palette = cx.palette().clone();
        let last = self.samples.back().copied();
        let cpu_label = match last {
            Some(Sample { cpu: Some(cpu), threads, .. }) => format!("CPU  {cpu:.0} %{}", threads.map(|t| format!("   Threads {t}")).unwrap_or_default()),
            _ => "CPU".to_owned(),
        };
        let memory = last.and_then(|s| s.memory);
        let samples: Vec<Sample> = self.samples.iter().copied().collect();
        let cpu_samples = samples.clone();
        let (grid, text) = (palette.border, palette.text_secondary);
        let cpu_chart = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let w = f32::from(bounds.size.width) / POINTS as f32;
                let h = f32::from(bounds.size.height);
                window.paint_quad(gpui_kit::quad(bounds, px(0.), gpui_kit::transparent_black(), px(1.), grid, Default::default()));
                let start = POINTS.saturating_sub(cpu_samples.len());
                for (i, s) in cpu_samples.iter().enumerate() {
                    let Some(cpu) = s.cpu else { continue };
                    let bar = (cpu / 100. * h).max(1.);
                    let x = bounds.origin.x + px((start + i) as f32 * w);
                    window.paint_quad(gpui_kit::fill(Bounds::new(point(x, bounds.origin.y + px(h - bar)), size(px(w.max(1.)), px(bar))), color(CPU_COLOR)));
                }
            },
        )
        .size_full();
        let peak = samples.iter().filter_map(|s| s.memory.map(|m| m.total)).max().unwrap_or(0).max(1);
        let memory_chart = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let w = f32::from(bounds.size.width) / POINTS as f32;
                let h = f32::from(bounds.size.height);
                window.paint_quad(gpui_kit::quad(bounds, px(0.), gpui_kit::transparent_black(), px(1.), grid, Default::default()));
                let start = POINTS.saturating_sub(samples.len());
                // The scale keeps the peak at 80% of the height.
                let scale = h * 0.8 / peak as f32;
                for (i, s) in samples.iter().enumerate() {
                    let Some(m) = s.memory else { continue };
                    let x = bounds.origin.x + px((start + i) as f32 * w);
                    let mut y = h;
                    for ((_, kb), c) in m.categories().iter().zip(CATEGORY_COLORS) {
                        let bar = *kb as f32 * scale;
                        if bar <= 0. {
                            continue;
                        }
                        y -= bar;
                        window.paint_quad(gpui_kit::fill(Bounds::new(point(x, bounds.origin.y + px(y)), size(px(w.max(1.)), px(bar))), color(c)));
                    }
                }
            },
        )
        .size_full();
        let legend = h_flex().gap_3().text_xs().text_color(text).child(div().font_weight(FontWeight::SEMIBOLD).text_color(palette.text).child(match memory {
            Some(m) => format!("MEMORY  Total {}", mb(m.total)),
            None => "MEMORY".to_owned(),
        }));
        let legend = match memory {
            Some(m) => legend.children(m.categories().iter().zip(CATEGORY_COLORS).map(|((name, kb), c)| {
                h_flex().gap_1().child(div().size(px(8.)).bg(color(c))).child(format!("{name} {}", mb(*kb)))
            })),
            None => legend,
        };
        v_flex()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py_1()
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(cpu_label))
            .child(div().h(px(if compact { 44. } else { 70. })).w_full().child(cpu_chart))
            .child(legend)
            .child(div().h(px(if compact { 70. } else { 120. })).w_full().child(memory_chart))
    }
}

impl Render for ProfilerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let devices = self.devices.clone();
        let device_label = self.active.as_ref().and_then(|s| self.devices.iter().find(|d| &d.serial == s)).map(|d| d.name.clone()).unwrap_or_else(|| "No Connected Devices".into());
        let device_picker = Button::new("pf-device").outline().xsmall().label(device_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for d in &devices {
                    let (entity, serial) = (entity.clone(), d.serial.clone());
                    menu = menu.item(PopupMenuItem::new(d.name.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.stop(cx);
                            this.active = Some(serial.clone());
                            this.process = None;
                            this.load_processes(cx);
                        })
                    }));
                }
                menu
            }
        });
        let processes = self.processes.clone();
        let app_ids = self.app_ids.clone();
        let process_label = self.process.as_ref().map(|p| format!("{} ({})", p.name, p.pid)).unwrap_or_else(|| "Select a process".into());
        let process_picker = Button::new("pf-process").outline().xsmall().label(process_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                let mut separated = false;
                for p in &processes {
                    let ours = app_ids.iter().any(|id| p.name == *id || p.name.starts_with(&format!("{id}:")));
                    if !ours && !separated && !app_ids.is_empty() {
                        menu = menu.separator();
                        separated = true;
                    }
                    let (entity, process) = (entity.clone(), p.clone());
                    menu = menu.item(PopupMenuItem::new(format!("{} ({})", p.name, p.pid)).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.stop(cx);
                            this.process = Some(process.clone());
                            this.start(cx);
                        })
                    }));
                }
                menu.separator().item(PopupMenuItem::new("Refresh").on_click({
                    let entity = entity.clone();
                    move |_, _, cx| entity.update(cx, |this, cx| this.load_processes(cx))
                }))
            }
        });
        let running = self.session.is_some();
        let can = self.process.is_some();
        let mode_picker = Button::new("pf-trace-mode").outline().xsmall().label(self.trace_mode.label()).disabled(self.recording != Recording::Idle).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for mode in [TraceMode::Sample, TraceMode::Trace, TraceMode::System] {
                    let entity = entity.clone();
                    menu = menu.item(PopupMenuItem::new(mode.label()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.trace_mode = mode;
                            cx.notify();
                        })
                    }));
                }
                menu
            }
        });
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Profiler"))
            .child(device_picker)
            .child(process_picker)
            .child(if running {
                Button::new("pf-stop").xsmall().outline().label("Stop").on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
            } else {
                Button::new("pf-start").xsmall().primary().label("Start").disabled(!can).on_click(cx.listener(|this, _, _, cx| this.start(cx)))
            })
            .child(Button::new("pf-heap").xsmall().outline().label(if self.dumping { "Dumping heap…" } else { "Capture heap dump" }).disabled(!can || self.dumping).on_click(cx.listener(|this, _, _, cx| this.capture_heap_dump(cx))))
            .child(mode_picker)
            .child(match self.recording {
                Recording::Idle => Button::new("pf-record").xsmall().outline().label("Record").disabled(!can).on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                Recording::Running => Button::new("pf-record").xsmall().primary().label("Stop recording").on_click(cx.listener(|this, _, _, cx| this.stop_recording(cx))),
                Recording::Starting => Button::new("pf-record").xsmall().outline().label("Starting…").disabled(true),
                Recording::Stopping => Button::new("pf-record").xsmall().outline().label("Saving trace…").disabled(true),
            })
            .child(div().flex_1())
            .child(tool_button("pf-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(ProfilerEvent::Hide))));
        let heap = self.heap.as_ref().map(|heap| {
            let heaps = heap.summary.heaps.clone();
            let entity = entity.clone();
            let count = self.classes(cx).len();
            let total: u64 = heap.summary.classes.iter().filter(|c| c.heap == heap.heap).map(|c| c.shallow).sum();
            let heap_picker = Button::new("pf-heap-name").outline().xsmall().label(format!("{} heap", heap.heap)).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
                for name in &heaps {
                    let (entity, name) = (entity.clone(), name.clone());
                    menu = menu.item(PopupMenuItem::new(format!("{name} heap")).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(h) = &mut this.heap {
                                h.heap = name.clone();
                                h.pick_class(None);
                            }
                            cx.notify();
                        })
                    }));
                }
                menu
            });
            v_flex()
                .flex_1()
                .min_h_0()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_sm().child("Heap Dump"))
                        .child(heap_picker)
                        .child(div().w(px(220.)).child(Input::new(&self.filter).xsmall().cleanable(true)))
                        .child(div().text_color(palette.text_secondary).child(format!("{count} classes, {} shallow", mb(total / 1024))))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(palette.text_secondary).child(heap.file.display().to_string())),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .items_start()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(
                                    h_flex()
                                        .h(px(ROW_H))
                                        .px_2()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.text_secondary)
                                        .border_b_1()
                                        .border_color(palette.border)
                                        .child(div().flex_1().min_w_0().child("Class Name"))
                                        .child(div().w(px(110.)).flex_none().text_right().child("Allocations"))
                                        .child(div().w(px(130.)).flex_none().text_right().child("Shallow Size"))
                                        .child(div().w(px(130.)).flex_none().text_right().child("Retained Size")),
                                )
                                .child(uniform_list("pf-classes", count, cx.processor(Self::render_class_rows)).flex_1().min_h_0()),
                        )
                        .when(heap.class.is_some(), |el| el.child(self.render_instance_panel(heap, &palette, cx))),
                )
        });
        let trace = self.trace.as_ref().map(|view| self.render_trace(view, &palette, cx).into_any_element());
        let empty = self.session.is_none() && self.samples.is_empty();
        let charts = (!empty).then(|| self.render_charts(cx));
        v_flex()
            .size_full()
            .child(header)
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .when(empty && self.heap.is_none() && self.trace.is_none() && self.system_trace.is_none(), |el| {
                el.child(div().p_4().text_sm().text_color(palette.text_secondary).child("Pick a process to watch its CPU and memory; Capture heap dump lists what its Java heap holds, and Record shows which methods it spends its time in."))
            })
            .children(charts)
            .children(heap)
            .children(trace)
            .children(self.system_trace.clone())
    }
}

//! The Profiler tool window, as Android Studio's live session: a device and
//! process picker, Start / Stop, the CPU timeline (the app's share of the
//! CPU and its threads) and the Memory timeline stacked by category (Java,
//! Native, Graphics, Stack, Code, Others). Capture heap dump saves an
//! `.hprof` and lists its classes with their Allocations, Shallow Size and
//! Retained Size; a class's Instance List (Depth, Shallow and Retained Size)
//! and an instance's References, expandable toward the GC roots.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
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
use crate::domain::profiler::{ProfiledProcess, Sample};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;

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
        let compact = self.heap.is_some();
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
        let empty = self.session.is_none() && self.samples.is_empty();
        v_flex()
            .size_full()
            .child(header)
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .when(empty && self.heap.is_none(), |el| {
                el.child(div().p_4().text_sm().text_color(palette.text_secondary).child("Pick a process to watch its CPU and memory; Capture heap dump lists what its Java heap holds."))
            })
            .when(!empty, |el| el.child(self.render_charts(cx)))
            .children(heap)
    }
}

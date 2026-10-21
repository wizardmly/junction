//! The Profiler's System Trace, as Android Studio shows a Perfetto
//! recording: a lane per CPU core with the threads it ran (the app's in
//! green) and its clock frequency below, then each of the app's threads
//! with its state (Running, Runnable, Sleeping, Waiting) and its
//! `android.os.Trace` sections. Ctrl + wheel zooms, Shift + wheel pans,
//! clicking a section zooms to it; hovering shows what ran when.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{Disableable as _, Sizable as _, button::{Button, ButtonVariants as _}, h_flex, v_flex};
use gpui_kit::{
    Bounds, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseMoveEvent, ParentElement as _, Pixels, Render, ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point,
    prelude::FluentBuilder as _, px, relative, size,
};

use crate::domain::system_trace::{SystemTrace, ThreadState};
use crate::theme::ActivePalette as _;

const LABEL_W: f32 = 210.;
const CPU_H: f32 = 22.;
const FREQ_H: f32 = 12.;
const STATE_H: f32 = 12.;
const SECTION_H: f32 = 17.;
/// Section bars drawn at once; narrower ones are skipped first.
const MAX_SECTIONS: usize = 4000;

const APP_COLOR: u32 = 0x81c784;
const OTHER_COLOR: u32 = 0x90a4ae;
const FREQ_COLOR: u32 = 0xffb74d;

fn color(hex: u32) -> Hsla {
    gpui_kit::rgb(hex).into()
}

fn state_color(state: ThreadState) -> Option<Hsla> {
    match state {
        ThreadState::Running => Some(color(0x66bb6a)),
        ThreadState::Runnable => Some(color(0x64b5f6)),
        ThreadState::Waiting => Some(color(0xffa726)),
        ThreadState::Sleeping | ThreadState::Dead => None,
    }
}

/// Nanoseconds as a short duration: `850 μs`, `12.40 ms`, `3.25 s`.
pub fn duration(ns: u64) -> String {
    if ns < 1_000_000 {
        format!("{} μs", ns / 1000)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1e6)
    } else {
        format!("{:.2} s", ns as f64 / 1e9)
    }
}

pub struct SystemTraceView {
    file: PathBuf,
    trace: SystemTrace,
    /// The visible time range; `None` shows the whole trace.
    range: Option<(u64, u64)>,
    hovered: Option<String>,
    /// The thread whose state times are summed below.
    thread: usize,
    /// Where the lanes were painted, for mouse positions: (left, width).
    track: Rc<Cell<(f32, f32)>>,
}

impl SystemTraceView {
    pub fn new(file: PathBuf, trace: SystemTrace) -> Self {
        Self { file, trace, range: None, hovered: None, thread: 0, track: Rc::new(Cell::new((0., 1.))) }
    }

    fn span(&self) -> (u64, u64) {
        (self.trace.start, self.trace.end.max(self.trace.start + 1))
    }

    fn visible(&self) -> (u64, u64) {
        self.range.unwrap_or_else(|| self.span())
    }

    fn zoom(&mut self, factor: f64) {
        let (lo, hi) = self.span();
        let (start, end) = self.visible();
        let middle = (start + end) as f64 / 2.;
        let width = ((end - start) as f64 * factor).clamp(10_000., (hi - lo) as f64);
        self.set_range(middle - width / 2., width);
    }

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

    /// The time under a window x position.
    fn time_at(&self, x: f32) -> u64 {
        let (left, width) = self.track.get();
        let (start, end) = self.visible();
        let fraction = ((x - left) / width.max(1.)).clamp(0., 1.) as f64;
        start + ((end - start) as f64 * fraction) as u64
    }

    fn hover_cpu(&mut self, cpu: usize, x: f32) {
        let at = self.time_at(x);
        let Some(lane) = self.trace.cpus.get(cpu) else { return };
        let i = lane.slices.partition_point(|s| s.end <= at);
        let ran = lane.slices.get(i).filter(|s| s.start <= at).map(|s| format!("{}   {}", self.trace.thread_label(s.tid), duration(s.end - s.start)));
        let frequency = lane.frequency_at(at).map(|khz| format!("{:.2} GHz", khz as f64 / 1e6));
        let at = duration(at - self.trace.start);
        self.hovered = Some(match (ran, frequency) {
            (Some(r), Some(f)) => format!("CPU {cpu} at {at}: {r}   ({f})"),
            (Some(r), None) => format!("CPU {cpu} at {at}: {r}"),
            (None, Some(f)) => format!("CPU {cpu} at {at}: idle   ({f})"),
            (None, None) => format!("CPU {cpu} at {at}: idle"),
        });
    }

    fn hover_thread(&mut self, thread: usize, x: f32) {
        let at = self.time_at(x);
        let Some(t) = self.trace.threads.get(thread) else { return };
        let i = t.states.partition_point(|s| s.end <= at);
        let state = t.states.get(i).filter(|s| s.start <= at);
        self.hovered = Some(match state {
            Some(s) => {
                let cpu = s.cpu.map(|c| format!(" on CPU {c}")).unwrap_or_default();
                format!("{} ({}) at {}: {}{cpu} for {}", t.name, t.tid, duration(at - self.trace.start), s.state.label(), duration(s.end - s.start))
            }
            None => format!("{} ({}) at {}: no data", t.name, t.tid, duration(at - self.trace.start)),
        });
    }

    fn label(&self, text: String, selected: bool, palette: &crate::theme::Palette) -> gpui_kit::Div {
        div()
            .w(px(LABEL_W))
            .flex_none()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_xs()
            .when(selected, |el| el.font_weight(FontWeight::SEMIBOLD))
            .text_color(if selected { palette.text } else { palette.text_secondary })
            .child(text)
    }

    fn cpu_lane(&self, cpu: usize, max_khz: u32, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let (start, end) = self.visible();
        let width = (end - start) as f64;
        let lane = &self.trace.cpus[cpu];
        // Only what is visible, merged below a pixel's worth by the painter.
        let first = lane.slices.partition_point(|s| s.end < start);
        let slices: Vec<(f64, f64, bool)> = lane.slices[first..]
            .iter()
            .take_while(|s| s.start <= end)
            .map(|s| ((s.start.max(start) - start) as f64 / width, (s.end.min(end) - start) as f64 / width, self.trace.is_app_thread(s.tid)))
            .collect();
        let f0 = lane.frequency.partition_point(|&(t, _)| t <= start).saturating_sub(1);
        let mut frequency: Vec<(f64, u32)> = lane.frequency[f0..].iter().take_while(|&&(t, _)| t <= end).map(|&(t, khz)| ((t.max(start) - start) as f64 / width, khz)).collect();
        frequency.push((1., frequency.last().map(|f| f.1).unwrap_or(0)));
        let (track, grid) = (self.track.clone(), palette.border);
        let lanes = canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window, _| {
                track.set((f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                let w = f32::from(bounds.size.width);
                let lane_h = px(CPU_H - 4.);
                let mut last_px = (-1, false);
                for (x0, x1, ours) in &slices {
                    let (a, b) = (*x0 as f32 * w, (*x1 as f32 * w).max(*x0 as f32 * w + 1.));
                    // Many slices inside one pixel: paint it once.
                    if (a as i32, *ours) == last_px && b - a < 1. {
                        continue;
                    }
                    last_px = (a as i32, *ours);
                    window.paint_quad(gpui_kit::fill(Bounds::new(point(bounds.origin.x + px(a), bounds.origin.y + px(2.)), size(px(b - a), lane_h)), color(if *ours { APP_COLOR } else { OTHER_COLOR })));
                }
                let top = bounds.origin.y + px(CPU_H);
                window.paint_quad(gpui_kit::fill(Bounds::new(point(bounds.origin.x, top + px(FREQ_H - 1.)), size(bounds.size.width, px(1.))), grid));
                for pair in frequency.windows(2) {
                    let ((x0, khz), (x1, _)) = (pair[0], pair[1]);
                    let h = khz as f32 / max_khz.max(1) as f32 * (FREQ_H - 2.);
                    if h <= 0. {
                        continue;
                    }
                    let (a, b) = (x0 as f32 * w, x1 as f32 * w);
                    window.paint_quad(gpui_kit::fill(Bounds::new(point(bounds.origin.x + px(a), top + px(FREQ_H - 1. - h)), size(px((b - a).max(1.)), px(h))), color(FREQ_COLOR).opacity(0.6)));
                }
            },
        )
        .size_full();
        h_flex()
            .h(px(CPU_H + FREQ_H))
            .flex_none()
            .child(self.label(format!("CPU {cpu}"), false, palette))
            .child(
                div()
                    .id(("st-cpu", cpu))
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                        this.hover_cpu(cpu, f32::from(e.position.x));
                        cx.notify();
                    }))
                    .child(lanes),
            )
    }

    fn thread_lane(&self, i: usize, budget: &mut usize, palette: &crate::theme::Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let (start, end) = self.visible();
        let width = (end - start) as f64;
        let thread = &self.trace.threads[i];
        let first = thread.states.partition_point(|s| s.end < start);
        let states: Vec<(f64, f64, Hsla)> = thread.states[first..]
            .iter()
            .take_while(|s| s.start <= end)
            .filter_map(|s| Some(((s.start.max(start) - start) as f64 / width, (s.end.min(end) - start) as f64 / width, state_color(s.state)?)))
            .collect();
        let (sleeping, track) = (palette.border, self.track.clone());
        let state_bar = canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window, _| {
                track.set((f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                let w = f32::from(bounds.size.width);
                window.paint_quad(gpui_kit::fill(Bounds::new(point(bounds.origin.x, bounds.origin.y + px(STATE_H / 2.)), size(bounds.size.width, px(1.))), sleeping));
                let mut last = (-1, Hsla::default());
                for (x0, x1, c) in &states {
                    let (a, b) = (*x0 as f32 * w, (*x1 as f32 * w).max(*x0 as f32 * w + 1.));
                    if (a as i32, *c) == last && b - a < 1. {
                        continue;
                    }
                    last = (a as i32, *c);
                    window.paint_quad(gpui_kit::fill(Bounds::new(point(bounds.origin.x + px(a), bounds.origin.y + px(1.)), size(px(b - a), px(STATE_H - 2.))), *c));
                }
            },
        )
        .size_full();
        let mut depth_max = 0;
        let mut bars = Vec::new();
        for (s_i, s) in thread.sections.iter().enumerate() {
            if s.end < start || s.start > end {
                continue;
            }
            depth_max = depth_max.max(s.depth + 1);
            let w = (s.end.min(end) - s.start.max(start)) as f64 / width;
            if w < 0.0005 || *budget == 0 {
                continue;
            }
            *budget -= 1;
            bars.push((s_i, s.depth, (s.start.max(start) - start) as f64 / width, w, s.name.clone()));
        }
        let sections = div().relative().w_full().h(px(depth_max as f32 * SECTION_H)).children(bars.into_iter().map(|(s_i, depth, x, w, name)| {
            div()
                .id(("st-section", i * 1_000_000 + s_i))
                .absolute()
                .left(relative(x as f32))
                .w(relative(w as f32))
                .top(px(depth as f32 * SECTION_H))
                .h(px(SECTION_H - 1.))
                .border_r_1()
                .border_color(palette.panel)
                .bg(color(0x64b5f6))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_xs()
                .text_color(gpui_kit::black())
                .child(div().pl_1().child(name))
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    if *over {
                        let t = &this.trace.threads[i];
                        let s = &t.sections[s_i];
                        this.hovered = Some(format!("{}   {}   on {} ({}), at {}", s.name, duration(s.end - s.start), t.name, t.tid, duration(s.start - this.trace.start)));
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let s = &this.trace.threads[i].sections[s_i];
                    let width = (s.end - s.start).max(10_000) as f64;
                    this.set_range(s.start as f64 - width * 0.05, width * 1.1);
                    this.thread = i;
                    cx.notify();
                }))
        }));
        let selected = self.thread == i;
        h_flex()
            .items_start()
            .flex_none()
            .py(px(2.))
            .border_b_1()
            .border_color(palette.border)
            .child(
                div().id(("st-thread", i)).cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                    this.thread = i;
                    cx.notify();
                })).child(self.label(format!("{} ({})", if thread.name.is_empty() { "<unknown>" } else { &thread.name }, thread.tid), selected, palette)),
            )
            .child(
                v_flex()
                    .id(("st-thread-track", i))
                    .flex_1()
                    .min_w_0()
                    .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                        this.hover_thread(i, f32::from(e.position.x));
                        cx.notify();
                    }))
                    .child(div().h(px(STATE_H)).w_full().child(state_bar))
                    .child(sections),
            )
    }
}

impl Render for SystemTraceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let (start, end) = self.visible();
        let zoomed = self.range.is_some();
        let caption = format!("{} – {}", duration(start - self.trace.start), duration(end - self.trace.start));
        let toolbar = h_flex()
            .h(px(22.))
            .flex_none()
            .px_2()
            .gap_2()
            .text_xs()
            .text_color(palette.text_secondary)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(palette.text).child("System Trace"))
            .child(div().child(format!("{} ({})", self.trace.process, self.trace.pid)))
            .child(Button::new("st-zoom-in").ghost().xsmall().label("Zoom In").on_click(cx.listener(|this, _, _, cx| {
                this.zoom(0.5);
                cx.notify();
            })))
            .child(Button::new("st-zoom-out").ghost().xsmall().label("Zoom Out").disabled(!zoomed).on_click(cx.listener(|this, _, _, cx| {
                this.zoom(2.);
                cx.notify();
            })))
            .child(Button::new("st-zoom-reset").ghost().xsmall().label("Reset Zoom").disabled(!zoomed).on_click(cx.listener(|this, _, _, cx| {
                this.range = None;
                cx.notify();
            })))
            .child(div().flex_none().child(caption))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(self.file.display().to_string()));
        let hovered = h_flex()
            .h(px(20.))
            .flex_none()
            .px_2()
            .text_xs()
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(self.hovered.clone().unwrap_or_else(|| "Hover over a lane to see what ran; click a section to zoom in".into())));
        let max_khz = self.trace.cpus.iter().flat_map(|c| c.frequency.iter().map(|f| f.1)).max().unwrap_or(0);
        let heading = |text: String| div().px_2().py_1().flex_none().text_xs().font_weight(FontWeight::SEMIBOLD).border_b_1().border_color(palette.border).child(text);
        let cpus: Vec<_> = (0..self.trace.cpus.len()).map(|c| self.cpu_lane(c, max_khz, &palette, cx).into_any_element()).collect();
        let mut budget = MAX_SECTIONS;
        let threads: Vec<_> = (0..self.trace.threads.len()).map(|i| self.thread_lane(i, &mut budget, &palette, cx).into_any_element()).collect();
        let summary = self.trace.threads.get(self.thread).map(|t| {
            let times = t.time_in(start, end);
            h_flex()
                .flex_none()
                .px_2()
                .py_1()
                .gap_3()
                .text_xs()
                .border_t_1()
                .border_color(palette.border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{} ({})", t.name, t.tid)))
                .children(times.into_iter().map(|(state, ns)| {
                    h_flex().gap_1().child(div().size(px(8.)).when_some(state_color(state), |el, c| el.bg(c)).when(state_color(state).is_none(), |el| el.border_1().border_color(palette.border))).child(format!("{} {}", state.label(), duration(ns)))
                }))
                .child(div().flex_1())
                .child(h_flex().gap_1().child(div().size(px(8.)).bg(color(APP_COLOR))).child("The app on a CPU"))
                .child(h_flex().gap_1().child(div().size(px(8.)).bg(color(OTHER_COLOR))).child("Other processes"))
                .child(h_flex().gap_1().child(div().size(px(8.)).bg(color(FREQ_COLOR))).child("CPU frequency"))
        });
        let thread_count = self.trace.threads.len();
        v_flex()
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(palette.border)
            .child(toolbar)
            .child(hovered)
            .child(
                v_flex()
                    .id("st-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                        let delta = e.delta.pixel_delta(px(22.));
                        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                        if e.modifiers.control || e.modifiers.platform {
                            this.zoom(if dy > 0. { 0.8 } else { 1.25 });
                        } else if e.modifiers.shift || dx.abs() > dy.abs() {
                            let along = if dx.abs() > dy.abs() { dx } else { dy };
                            this.pan(-along as f64 / 1000.);
                        } else {
                            return;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(heading(format!("CPU cores ({})", self.trace.cpus.len())))
                    .children(cpus)
                    .child(heading(format!("Threads ({thread_count})")))
                    .when(thread_count == 0, |el| el.child(div().p_2().text_xs().text_color(palette.text_secondary).child("The app's threads didn't run during the recording.")))
                    .children(threads),
            )
            .children(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(850_000), "850 μs");
        assert_eq!(duration(12_400_000), "12.40 ms");
        assert_eq!(duration(3_250_000_000), "3.25 s");
    }
}

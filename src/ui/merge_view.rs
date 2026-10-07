//! IntelliJ's three-way merge tool: Yours | Result | Theirs, with per-chunk
//! accept (>> / <<) and ignore (×) buttons, "Apply non-conflicting changes",
//! Accept Left / Right, and a free-text edit mode for the result.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Textarea, TextareaState},
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::git::merge::{self, Conflict, MergeChunk, Resolution};
use crate::git::Repository;
use crate::model::RepoModel;
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};

const LINE_HEIGHT: f32 = 20.;

pub enum MergeEvent {
    /// The tool closed; `true` when the result was applied.
    Closed(bool),
}

impl EventEmitter<MergeEvent> for MergeView {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideState {
    /// The side didn't change this chunk.
    Unchanged,
    Pending,
    Applied,
    Ignored,
}

#[derive(Clone, Debug)]
struct ChunkState {
    ours: SideState,
    theirs: SideState,
    /// Applied sides in order (`true` = ours), so "both" keeps click order.
    order: Vec<bool>,
}

impl ChunkState {
    fn new(chunk: &MergeChunk) -> Self {
        let state = |changed| if changed { SideState::Pending } else { SideState::Unchanged };
        match chunk {
            MergeChunk::Change { ours_changed, theirs_changed, .. } => {
                Self { ours: state(*ours_changed), theirs: state(*theirs_changed), order: Vec::new() }
            }
            MergeChunk::Equal { .. } => Self { ours: SideState::Unchanged, theirs: SideState::Unchanged, order: Vec::new() },
        }
    }

    fn resolved(&self) -> bool {
        self.ours != SideState::Pending && self.theirs != SideState::Pending
    }

    fn resolution(&self, chunk: &MergeChunk) -> Resolution {
        let identical = matches!(chunk, MergeChunk::Change { conflict: false, ours_changed: true, theirs_changed: true, .. });
        match self.order.as_slice() {
            [] if self.resolved() => Resolution::Base,
            [] => Resolution::Unresolved,
            // The same change on both sides is applied once.
            [_, _] if identical => Resolution::Ours,
            [true] => Resolution::Ours,
            [false] => Resolution::Theirs,
            [true, false] => Resolution::OursThenTheirs,
            _ => Resolution::TheirsThenOurs,
        }
    }
}

struct Cell {
    line: usize,
    text: SharedString,
}

/// One display row; a chunk spans as many rows as its tallest column.
struct Row {
    chunk: usize,
    first: bool,
    left: Option<Cell>,
    center: Option<Cell>,
    right: Option<Cell>,
}

pub struct MergeView {
    model: Entity<RepoModel>,
    conflict: Conflict,
    titles: (&'static str, &'static str),
    base: Vec<String>,
    ours: Vec<String>,
    theirs: Vec<String>,
    chunks: Vec<MergeChunk>,
    states: Vec<ChunkState>,
    rows: Rc<Vec<Row>>,
    edit: Option<Entity<TextareaState>>,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    /// The row "Next Unresolved Change" last went to.
    cursor: usize,
    _load: Option<Task<()>>,
}

fn strs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

fn line_vec(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

impl MergeView {
    pub fn new(model: Entity<RepoModel>, repository: Repository, conflict: Conflict, cx: &mut Context<Self>) -> Self {
        let titles = merge::side_titles(model.read(cx).state());
        let mut this = Self {
            model,
            conflict: conflict.clone(),
            titles,
            base: Vec::new(),
            ours: Vec::new(),
            theirs: Vec::new(),
            chunks: Vec::new(),
            states: Vec::new(),
            rows: Rc::new(Vec::new()),
            edit: None,
            error: None,
            scroll: UniformListScrollHandle::new(),
            cursor: 0,
            _load: None,
        };
        this._load = Some(cx.spawn(async move |this, cx| {
            let path = conflict.path.clone();
            let versions = cx.background_spawn(async move { merge::load_versions(&repository, &path) }).await;
            this.update(cx, |this, cx| {
                match versions {
                    Ok(v) => {
                        this.base = line_vec(&v.base);
                        this.ours = line_vec(&v.ours);
                        this.theirs = line_vec(&v.theirs);
                        let (b, o, t) = this.slices();
                        this.chunks = merge::chunks(&b, &o, &t);
                        this.states = this.chunks.iter().map(ChunkState::new).collect();
                        this.rebuild();
                        this.go_to_unresolved(true);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        this
    }

    pub fn path(&self) -> &str {
        &self.conflict.path
    }

    fn slices(&self) -> (Vec<&str>, Vec<&str>, Vec<&str>) {
        (strs(&self.base), strs(&self.ours), strs(&self.theirs))
    }

    fn resolutions(&self) -> Vec<Resolution> {
        self.chunks.iter().zip(&self.states).map(|(c, s)| s.resolution(c)).collect()
    }

    fn result_text(&self) -> String {
        let (b, o, t) = self.slices();
        let mut text = merge::result_lines(&self.chunks, &self.resolutions(), &b, &o, &t).join("\n");
        text.push('\n');
        text
    }

    fn rebuild(&mut self) {
        let (b, o, t) = self.slices();
        let mut rows = Vec::new();
        let (mut left_line, mut right_line, mut result_line) = (0usize, 0usize, 0usize);
        for (ix, (chunk, state)) in self.chunks.iter().zip(&self.states).enumerate() {
            let (left, right, base_range): (&[&str], &[&str], Range<usize>) = match chunk {
                MergeChunk::Equal { base } => (&b[base.clone()], &b[base.clone()], base.clone()),
                MergeChunk::Change { ours, theirs, base, .. } => (&o[ours.clone()], &t[theirs.clone()], base.clone()),
            };
            let result = merge::result_lines(
                std::slice::from_ref(chunk),
                &[state.resolution(chunk)],
                &b,
                &o,
                &t,
            );
            let _ = base_range;
            let height = left.len().max(right.len()).max(result.len()).max(1);
            for k in 0..height {
                let cell = |lines: &[&str], start: usize| {
                    lines.get(k).map(|text| Cell { line: start + k + 1, text: SharedString::from(text.to_string()) })
                };
                rows.push(Row {
                    chunk: ix,
                    first: k == 0,
                    left: cell(left, left_line),
                    center: cell(&result, result_line),
                    right: cell(right, right_line),
                });
            }
            left_line += left.len();
            right_line += right.len();
            result_line += result.len();
        }
        self.rows = Rc::new(rows);
    }

    fn update_chunk(&mut self, ix: usize, cx: &mut Context<Self>, f: impl FnOnce(&mut ChunkState)) {
        if let Some(state) = self.states.get_mut(ix) {
            f(state);
            self.rebuild();
            cx.notify();
        }
    }

    fn apply_side(&mut self, ix: usize, ours: bool, cx: &mut Context<Self>) {
        self.update_chunk(ix, cx, |s| {
            let side = if ours { &mut s.ours } else { &mut s.theirs };
            if *side == SideState::Pending {
                *side = SideState::Applied;
                s.order.push(ours);
            }
        });
    }

    fn ignore_side(&mut self, ix: usize, ours: bool, cx: &mut Context<Self>) {
        self.update_chunk(ix, cx, |s| {
            let side = if ours { &mut s.ours } else { &mut s.theirs };
            if *side == SideState::Pending {
                *side = SideState::Ignored;
            }
        });
    }

    /// The magic wand: apply every change that doesn't conflict.
    fn apply_non_conflicting(&mut self, cx: &mut Context<Self>) {
        for (chunk, state) in self.chunks.iter().zip(self.states.iter_mut()) {
            if state.resolved() {
                continue;
            }
            match merge::automatic(chunk) {
                Some(Resolution::Ours) => {
                    state.ours = SideState::Applied;
                    state.order = vec![true];
                    if state.theirs == SideState::Pending {
                        state.theirs = SideState::Applied;
                    }
                }
                Some(Resolution::Theirs) => {
                    state.theirs = SideState::Applied;
                    state.order = vec![false];
                }
                _ => {}
            }
        }
        self.rebuild();
        self.go_to_unresolved(true);
        cx.notify();
    }

    /// Accept Left / Accept Right for every unresolved chunk.
    fn accept_all(&mut self, ours: bool, cx: &mut Context<Self>) {
        for state in &mut self.states {
            if state.resolved() {
                continue;
            }
            let (mine, other) = if ours { (&mut state.ours, &mut state.theirs) } else { (&mut state.theirs, &mut state.ours) };
            if *mine == SideState::Pending {
                *mine = SideState::Applied;
                state.order.push(ours);
            }
            if *other == SideState::Pending {
                *other = SideState::Ignored;
            }
        }
        self.rebuild();
        cx.notify();
    }

    fn counts(&self) -> (usize, usize) {
        let mut changes = 0;
        let mut conflicts = 0;
        for (chunk, state) in self.chunks.iter().zip(&self.states) {
            if let MergeChunk::Change { conflict, .. } = chunk {
                if !state.resolved() {
                    if *conflict {
                        conflicts += 1;
                    } else {
                        changes += 1;
                    }
                }
            }
        }
        (changes, conflicts)
    }

    fn go_to_unresolved(&mut self, from_start: bool) {
        let current = if from_start { 0 } else { self.cursor + 1 };
        let target = self
            .rows
            .iter()
            .enumerate()
            .skip(current)
            .find(|(_, r)| r.first && !self.states[r.chunk].resolved())
            .map(|(ix, _)| ix);
        // Wrap around to the first unresolved change.
        let target = target.or_else(|| {
            self.rows.iter().position(|r| r.first && !self.states[r.chunk].resolved())
        });
        if let Some(ix) = target {
            self.cursor = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    fn toggle_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.take().is_none() {
            let text = self.result_text();
            let state = cx.new(|cx| TextareaState::new(window, cx));
            state.update(cx, |s, cx| s.set_value(text, window, cx));
            self.edit = Some(state);
        }
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let content = match &self.edit {
            Some(edit) => {
                let mut text = edit.read(cx).value().to_string();
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text
            }
            None => self.result_text(),
        };
        let path = self.conflict.path.clone();
        self.model.update(cx, |model, cx| {
            model.run_operation("Resolve Conflict", move |repo| {
                merge::resolve_with(repo, &path, &content)?;
                Ok(format!("{path} merged"))
            }, cx)
        });
        cx.emit(MergeEvent::Closed(true));
    }

    fn render_row(&self, row: &Row, palette: &Palette, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let chunk = &self.chunks[row.chunk];
        let state = &self.states[row.chunk];
        let ix = row.chunk;
        let (conflict, is_change) = match chunk {
            MergeChunk::Change { conflict, .. } => (*conflict, true),
            MergeChunk::Equal { .. } => (false, false),
        };
        let resolved = state.resolved();
        let change_bg = |side: SideState| -> Option<Hsla> {
            if !is_change {
                return None;
            }
            let base = if conflict { palette.diff_deleted } else { palette.diff_modified };
            match side {
                SideState::Unchanged => None,
                SideState::Pending => Some(base),
                _ => Some(base.opacity(0.35)),
            }
        };
        let center_bg = if !is_change {
            None
        } else if resolved {
            Some(palette.diff_inserted.opacity(0.5))
        } else if conflict {
            Some(palette.diff_deleted)
        } else {
            Some(palette.diff_modified)
        };

        let text_cell = |cell: &Option<Cell>, bg: Option<Hsla>| {
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .when_some(bg, |el, bg| el.bg(bg))
                .child(
                    div()
                        .w(px(38.))
                        .flex_shrink_0()
                        .text_right()
                        .pr_2()
                        .text_color(palette.text_disabled)
                        .child(cell.as_ref().map(|c| c.line.to_string()).unwrap_or_default()),
                )
                .when_some(cell.as_ref(), |el, c| el.child(div().whitespace_nowrap().child(c.text.clone())))
        };

        // The gutters between columns hold the chunk's action buttons.
        let gutter = |ours: bool, cx: &mut Context<Self>| {
            let side = if ours { state.ours } else { state.theirs };
            let show = row.first && side == SideState::Pending;
            h_flex()
                .w(px(44.))
                .flex_shrink_0()
                .h_full()
                .justify_center()
                .gap_0p5()
                .when(show, |el| {
                    let accept = div()
                        .id(SharedString::from(format!("accept-{}-{ix}", if ours { "l" } else { "r" })))
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(palette.accent)
                        .hover(|s| s.bg(palette.hover))
                        .child(if ours { "»" } else { "«" })
                        .on_click(cx.listener(move |this, _, _, cx| this.apply_side(ix, ours, cx)));
                    let ignore = div()
                        .id(SharedString::from(format!("ignore-{}-{ix}", if ours { "l" } else { "r" })))
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(palette.text_secondary)
                        .hover(|s| s.bg(palette.hover))
                        .child("×")
                        .on_click(cx.listener(move |this, _, _, cx| this.ignore_side(ix, ours, cx)));
                    if ours { el.child(ignore).child(accept) } else { el.child(accept).child(ignore) }
                })
        };

        h_flex()
            .h(px(LINE_HEIGHT))
            .w_full()
            .child(text_cell(&row.left, change_bg(state.ours)))
            .child(gutter(true, cx))
            .child(text_cell(&row.center, center_bg))
            .child(gutter(false, cx))
            .child(text_cell(&row.right, change_bg(state.theirs)))
            .into_any_element()
    }
}

impl Render for MergeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mono = cx.theme().mono_font_family.clone();
        let (changes, conflicts) = self.counts();
        let all_resolved = changes == 0 && conflicts == 0;
        let rows = self.rows.clone();
        let editing = self.edit.is_some();
        let summary = match (changes, conflicts) {
            (0, 0) => "All changes have been processed".to_owned(),
            (c, 0) => format!("{c} change{} left", if c == 1 { "" } else { "s" }),
            (c, k) => format!("{c} change{}, {k} conflict{} left", if c == 1 { "" } else { "s" }, if k == 1 { "" } else { "s" }),
        };

        let header = |title: &str| div().flex_1().pl(px(46.)).child(title.to_owned());
        let body = if let Some(edit) = &self.edit {
            div().flex_1().min_h_0().p_2().child(Textarea::new(edit).h_full()).into_any_element()
        } else {
            uniform_list(
                "merge-rows",
                rows.len(),
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    let palette = cx.palette().clone();
                    range.map(|ix| this.render_row(&rows[ix], &palette, cx)).collect()
                }),
            )
            .track_scroll(&self.scroll)
            .font_family(mono)
            .text_size(px(12.5))
            .flex_1()
            .w_full()
            .into_any_element()
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(palette.toolbar)
                    .child(
                        tool_button("merge-wand", IconName::CheckCheck, "Apply All Non-Conflicting Changes")
                            .disabled(editing)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_non_conflicting(cx))),
                    )
                    .child(
                        tool_button("merge-next", IconName::ChevronDown, "Next Unresolved Change")
                            .disabled(editing)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.go_to_unresolved(false);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("merge-edit")
                            .ghost()
                            .xsmall()
                            .label(if editing { "Back to Chunks" } else { "Edit Result" })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_edit(window, cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border))
                    .child(common::icon(common::file_icon(&self.conflict.path)).text_color(palette.text_secondary))
                    .child(div().text_sm().child(format!("Merge Revisions for {}", self.conflict.path)))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(palette.text_secondary).child(summary)),
            )
            .when(!editing, |el| {
                el.child(
                    h_flex()
                        .h(px(22.))
                        .text_xs()
                        .text_color(palette.text_secondary)
                        .border_b_1()
                        .border_color(palette.border)
                        .child(header(self.titles.0))
                        .child(div().w(px(44.)))
                        .child(header("Result"))
                        .child(div().w(px(44.)))
                        .child(header(self.titles.1)),
                )
            })
            .when_some(self.error.clone(), |el, e| el.child(div().p_3().text_color(palette.status_conflict).child(e)))
            .child(body)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("merge-accept-left")
                            .outline()
                            .small()
                            .label("Accept Left")
                            .disabled(editing)
                            .on_click(cx.listener(|this, _, _, cx| this.accept_all(true, cx))),
                    )
                    .child(
                        Button::new("merge-accept-right")
                            .outline()
                            .small()
                            .label("Accept Right")
                            .disabled(editing)
                            .on_click(cx.listener(|this, _, _, cx| this.accept_all(false, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("merge-cancel")
                            .outline()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::Closed(false)))),
                    )
                    .child(
                        Button::new("merge-apply")
                            .primary()
                            .small()
                            .label("Apply")
                            .disabled(!(all_resolved || editing))
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
    }
}

//! IntelliJ's three-way merge tool: three editors, "Changes from" one side
//! (read-only), the Result (editable, starting from the base) and the other
//! side (read-only), joined by two dividers that connect each change. Per
//! change: `>>` / `<<` apply a side (the second one appends), `×` ignores it,
//! and the magic wand merges a simple conflict word by word. The toolbar
//! applies all non-conflicting changes from the left, both or the right side.

use std::ops::Range;
use crate::ui::as_icons as icons;

use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Render, Styled as _, Task, Window, canvas, div, prelude::FluentBuilder as _, px,
};

use crate::git::Repository;
use crate::git::diff::{self, HighlightMode, IgnoreWhitespace};
use crate::git::merge::{self, Conflict, MergeChunk};
use crate::model::RepoModel;
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};
use crate::ui::diff_panes::{line_height, BlockColors, Connector, DIVIDER_WIDTH, Segment, paint_divider};
use crate::ui::diff_view::edit::replacement;
use crate::ui::text_buffer::EditKind;
use crate::ui::text_panes::{BUTTON_WIDTH, PaneContent, PaneHost, PaneLayout, RowLook, RowTarget, STRIPE_WIDTH, TextPanes, pane_area};

pub enum MergeEvent {
    /// The tool closed; `true` when the result was applied.
    Closed(bool),
    /// Compare Contents: two of the versions in the diff viewer.
    Compare(crate::ui::diff_view::DiffSource),
}

impl EventEmitter<MergeEvent> for MergeView {}

const OURS: usize = 0;
const RESULT: usize = 1;
const THEIRS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideState {
    /// The side didn't change this chunk.
    Unchanged,
    Pending,
    Applied,
    Ignored,
}

/// One change of the merge, as it stands (undoes with the result text).
#[derive(Clone, Debug)]
pub struct ChangeState {
    ours: SideState,
    theirs: SideState,
    /// Its lines in the result.
    result: Range<usize>,
}

impl ChangeState {
    fn side(&self, ours: bool) -> SideState {
        if ours { self.ours } else { self.theirs }
    }

    fn side_mut(&mut self, ours: bool) -> &mut SideState {
        if ours { &mut self.ours } else { &mut self.theirs }
    }

    fn resolved(&self) -> bool {
        self.ours != SideState::Pending && self.theirs != SideState::Pending
    }
}

/// A change chunk of the merge, fixed once loaded.
#[derive(Clone, Debug)]
struct Change {
    base: Range<usize>,
    ours: Range<usize>,
    theirs: Range<usize>,
    ours_changed: bool,
    theirs_changed: bool,
    conflict: bool,
    /// The magic wand's word-by-word result, for a simple conflict.
    simple: Option<String>,
    /// Changed words per line of each side against the base, and of the
    /// base against either side.
    words_ours: Vec<Vec<Range<usize>>>,
    words_theirs: Vec<Vec<Range<usize>>>,
    words_base: Vec<Vec<Range<usize>>>,
}

impl Change {
    fn range(&self, pane: usize) -> Range<usize> {
        if pane == OURS { self.ours.clone() } else { self.theirs.clone() }
    }

    /// The color family of one side: red for a conflict, else by what the
    /// side did to the base.
    fn kind(&self, ours: bool) -> Kind {
        if self.conflict {
            return Kind::Conflict;
        }
        let side = if ours { &self.ours } else { &self.theirs };
        let changed = if ours { self.ours_changed } else { self.theirs_changed };
        match (changed, self.base.is_empty(), side.is_empty()) {
            (false, ..) => Kind::None,
            (true, true, _) => Kind::Inserted,
            (true, _, true) => Kind::Deleted,
            _ => Kind::Modified,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    None,
    Inserted,
    Deleted,
    Modified,
    Conflict,
}

impl Kind {
    fn colors(self, p: &Palette) -> Option<BlockColors> {
        Some(match self {
            Kind::None => return None,
            Kind::Inserted => BlockColors { fill: p.diff_inserted, border: p.diff_inserted_border },
            Kind::Deleted => BlockColors { fill: p.diff_deleted, border: p.diff_deleted_border },
            Kind::Modified => BlockColors { fill: p.diff_modified, border: p.diff_modified_border },
            Kind::Conflict => BlockColors { fill: p.diff_conflict, border: p.diff_conflict_border },
        })
    }
}

pub struct MergeView {
    model: Entity<RepoModel>,
    conflict: Conflict,
    /// The side panes' titles, naming the branches once known.
    titles: (String, String),
    /// The common ancestor's text, for Compare Contents.
    base: String,
    changes: Vec<Change>,
    panes: TextPanes<Vec<ChangeState>>,
    error: Option<String>,
    /// The change Next / Previous went to last.
    current: Option<usize>,
    /// Collapse Unchanged Fragments, and the fragments opened since.
    collapse: bool,
    expanded: std::collections::HashSet<usize>,
    context_lines: usize,
    /// The three versions as loaded, to re-compare with other options.
    versions: Option<merge::MergeVersions>,
    ignore: IgnoreWhitespace,
    highlight: HighlightMode,
    /// Gear › Align Changes: each change as tall on all three panes.
    align: bool,
    /// The rows each change takes on each pane (with Align Changes, its
    /// padding too).
    blocks: Vec<[Range<usize>; 3]>,
    _load: Option<Task<()>>,
}

impl MergeView {
    pub fn new(model: Entity<RepoModel>, repository: Repository, conflict: Conflict, cx: &mut Context<Self>) -> Self {
        let state = model.read(cx).state();
        let (left, right) = merge::side_titles(state);
        let titles = (left.to_owned(), right.to_owned());
        let mut panes = TextPanes::new(3, cx);
        panes.editable = Some(RESULT);
        let mut this = Self {
            model,
            conflict: conflict.clone(),
            titles,
            base: String::new(),
            changes: Vec::new(),
            panes,
            error: None,
            current: None,
            collapse: false,
            expanded: Default::default(),
            context_lines: crate::settings::Settings::get(cx).diff.context_lines,
            versions: None,
            ignore: IgnoreWhitespace::None,
            highlight: HighlightMode::Words,
            align: crate::settings::Settings::get(cx).diff.align_changes,
            blocks: Vec::new(),
            _load: None,
        };
        this._load = Some(cx.spawn(async move |this, cx| {
            let path = conflict.path.clone();
            let (versions, titles) =
                cx.background_spawn(async move { (merge::load_versions(&repository, &path), merge::branch_titles(&repository, state)) }).await;
            this.update(cx, |this, cx| {
                this.titles = titles;
                match versions {
                    Ok(v) => this.load(v),
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

    fn load(&mut self, v: merge::MergeVersions) {
        self.versions = Some(v.clone());
        let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
        let (b, o, t) = (lines(&v.base), lines(&v.ours), lines(&v.theirs));
        fn refs(l: &[String]) -> Vec<&str> {
            l.iter().map(String::as_str).collect()
        }
        let (br, or, tr) = (refs(&b), refs(&o), refs(&t));
        // Lines are compared as the whitespace option has them.
        let keys = |l: &[&str]| l.iter().map(|l| diff::normalize(l, self.ignore)).collect::<Vec<_>>();
        let (bk, ok, tk) = (keys(&br), keys(&or), keys(&tr));
        let chunks = merge::chunks(&refs(&bk), &refs(&ok), &refs(&tk));
        let join = |l: &[&str]| l.iter().map(|s| format!("{s}\n")).collect::<String>();
        self.changes = chunks
            .iter()
            .filter_map(|c| match c {
                MergeChunk::Change { base, ours, theirs, ours_changed, theirs_changed, conflict } => Some({
                    Change {
                    words_ours: Vec::new(),
                    words_theirs: Vec::new(),
                    words_base: Vec::new(),
                    base: base.clone(),
                    ours: ours.clone(),
                    theirs: theirs.clone(),
                    ours_changed: *ours_changed,
                    theirs_changed: *theirs_changed,
                    conflict: *conflict,
                    simple: conflict
                        .then(|| merge::resolve_simple(&join(&br[base.clone()]), &join(&or[ours.clone()]), &join(&tr[theirs.clone()])))
                        .flatten(),
                    }
                }),
                MergeChunk::Equal { .. } => None,
            })
            .collect();
        self.compute_words();
        // The result starts as the base.
        let state = |changed| if changed { SideState::Pending } else { SideState::Unchanged };
        self.panes.extra = self
            .changes
            .iter()
            .map(|c| ChangeState { ours: state(c.ours_changed), theirs: state(c.theirs_changed), result: c.base.clone() })
            .collect();
        let language = crate::ui::file_editor::language_for(&self.conflict.path);
        self.base = v.base.clone();
        self.panes.set_texts(vec![v.ours, v.base, v.theirs], language);
        self.panes.layouts = vec![
            PaneLayout { mirrored: true, buttons: BUTTON_WIDTH * 2. + 2., ..Default::default() },
            PaneLayout { mirrored: false, buttons: BUTTON_WIDTH + 2., ..Default::default() },
            PaneLayout { mirrored: false, buttons: BUTTON_WIDTH * 2. + 2., ..Default::default() },
        ];
        self.refresh();
        self.go_to_first_unresolved();
    }

    /// Compare Contents: Left / Base / Right / Result against each other,
    /// as IntelliJ's merge tool offers (the result as it stands now).
    fn compare(&mut self, old: usize, new: usize, cx: &mut Context<Self>) {
        // 0 left, 1 base, 2 right, 3 result.
        let text = |which: usize| match which {
            0 => self.panes.buffers[OURS].text().to_owned(),
            1 => self.base.clone(),
            2 => self.panes.buffers[THEIRS].text().to_owned(),
            _ => self.result_text(),
        };
        let title = |which: usize| match which {
            0 => self.titles.0.clone(),
            1 => "Base".to_owned(),
            2 => self.titles.1.clone(),
            _ => "Result".to_owned(),
        };
        cx.emit(MergeEvent::Compare(crate::ui::diff_view::DiffSource::Texts {
            path: self.conflict.path.clone(),
            old: text(old),
            new: text(new),
            old_title: title(old),
            new_title: title(new),
        }));
    }

    /// The changed words (or characters) of each change, as the highlight
    /// option asks.
    fn compute_words(&mut self) {
        let Some(v) = &self.versions else { return };
        let inner = self.highlight.inner();
        let chars = self.highlight == HighlightMode::Characters;
        let (b, o, t): (Vec<&str>, Vec<&str>, Vec<&str>) = (v.base.lines().collect(), v.ours.lines().collect(), v.theirs.lines().collect());
        for c in &mut self.changes {
            if !inner {
                (c.words_ours, c.words_theirs, c.words_base) = Default::default();
                continue;
            }
            let (base_o, words_ours) = diff::line_fragments(&b[c.base.clone()], &o[c.ours.clone()], chars);
            let (base_t, words_theirs) = diff::line_fragments(&b[c.base.clone()], &t[c.theirs.clone()], chars);
            c.words_base = base_o
                .into_iter()
                .zip(base_t)
                .map(|(mut a, b)| {
                    a.extend(b);
                    a.sort_by_key(|r| r.start);
                    a
                })
                .collect();
            c.words_ours = if c.ours_changed { words_ours } else { Vec::new() };
            c.words_theirs = if c.theirs_changed { words_theirs } else { Vec::new() };
        }
    }

    /// Nothing is resolved or edited yet (the result is still the base), so
    /// the versions can be compared again.
    fn untouched(&self) -> bool {
        self.panes.buffers[RESULT].text() == self.base
            && self.states().iter().all(|s| !matches!(s.ours, SideState::Applied | SideState::Ignored) && !matches!(s.theirs, SideState::Applied | SideState::Ignored))
    }

    /// The whitespace option re-compares the versions, before any change
    /// is taken.
    fn set_ignore(&mut self, ignore: IgnoreWhitespace, cx: &mut Context<Self>) {
        if ignore == self.ignore || !self.untouched() {
            return;
        }
        self.ignore = ignore;
        if let Some(v) = self.versions.clone() {
            self.load(v);
        }
        cx.notify();
    }

    fn set_highlight(&mut self, highlight: HighlightMode, cx: &mut Context<Self>) {
        self.highlight = highlight;
        self.compute_words();
        cx.notify();
    }

    fn states(&self) -> &[ChangeState] {
        &self.panes.extra
    }

    /// After the result or the change states changed: rows, scroll links.
    fn refresh(&mut self) {
        let folds = self.folds();
        let ranges: Vec<[Range<usize>; 3]> =
            self.changes.iter().zip(self.states()).map(|(c, s)| [c.ours.clone(), s.result.clone(), c.theirs.clone()]).collect();
        self.blocks = vec![[0..0, 0..0, 0..0]; ranges.len()];
        for pane in 0..3 {
            let line_count = self.panes.buffers[pane].line_count();
            let mut rows = Vec::new();
            let mut first_row = vec![0; line_count + 1];
            let (mut line, mut fold, mut change) = (0, 0, 0);
            loop {
                // A change's block ends before the line after it: pad it to
                // the tallest side's height when aligning.
                while let Some(r) = ranges.get(change).filter(|r| r[pane].end <= line) {
                    let start = if r[pane].is_empty() { rows.len() } else { first_row[r[pane].start] };
                    if self.align {
                        let tallest = r.iter().map(|r| r.len()).max().unwrap_or(0);
                        rows.extend(std::iter::repeat_n(RowTarget::Filler, tallest - r[pane].len()));
                    }
                    self.blocks[change][pane] = start..rows.len();
                    change += 1;
                }
                if line >= line_count {
                    break;
                }
                if let Some((id, lines)) = folds.get(fold).filter(|(_, l)| l[pane].start == line) {
                    first_row[line] = rows.len();
                    rows.push(RowTarget::Fold { id: *id, count: lines[pane].len() });
                    line = lines[pane].end.max(line + 1);
                    fold += 1;
                    continue;
                }
                first_row[line] = rows.len();
                rows.push(RowTarget::Line(line));
                line += 1;
            }
            self.panes.set_rows(pane, rows);
        }
        self.panes.links = vec![(OURS, RESULT, self.segments(OURS)), (RESULT, THEIRS, self.segments(THEIRS))];
    }

    /// Collapse Unchanged Fragments: the unchanged stretches between
    /// changes (the same lines on all three panes), less their context
    /// lines, unless opened. Stretch `k` comes before change `k`.
    fn folds(&self) -> Vec<(usize, [Range<usize>; 3])> {
        if !self.collapse {
            return Vec::new();
        }
        let context = self.context_lines;
        let mut out = Vec::new();
        let mut starts = [0; 3];
        let ends = |ix: usize| -> [usize; 3] {
            match (self.changes.get(ix), self.states().get(ix)) {
                (Some(c), Some(s)) => [c.ours.start, s.result.start, c.theirs.start],
                _ => [0, 1, 2].map(|p| self.panes.buffers[p].line_count()),
            }
        };
        for k in 0..=self.changes.len() {
            let end = ends(k);
            let len = end[0].saturating_sub(starts[0]);
            if (0..3).all(|p| end[p].saturating_sub(starts[p]) == len) {
                let head = if k == 0 { 0 } else { context };
                let tail = if k == self.changes.len() { 0 } else { context };
                if len > head + tail + 1 && !self.expanded.contains(&k) {
                    out.push((k, [0, 1, 2].map(|p| starts[p] + head..end[p] - tail)));
                }
            }
            if let (Some(c), Some(s)) = (self.changes.get(k), self.states().get(k)) {
                starts = [c.ours.end, s.result.end, c.theirs.end];
            }
        }
        out
    }

    /// The rows a change takes on a pane (changes are never folded).
    fn block(&self, change: usize, pane: usize) -> Range<usize> {
        self.blocks.get(change).map_or(0..0, |b| b[pane].clone())
    }

    /// The blocks pairing a side's lines with the result's, for scrolling
    /// together: each change, and the unchanged stretches between them.
    fn segments(&self, side: usize) -> Vec<Segment> {
        let mut out = Vec::new();
        let (mut s, mut r) = (0, 0);
        for ix in 0..self.changes.len() {
            let (range, result) = (self.block(ix, side), self.block(ix, RESULT));
            out.push(Segment { left: s..range.start, right: r..result.start, change: None, kind: crate::git::diff::RowKind::Equal });
            out.push(Segment { left: range.clone(), right: result.clone(), change: Some(0), kind: crate::git::diff::RowKind::Modified });
            s = range.end;
            r = result.end;
        }
        let (side_end, result_end) = (self.panes.row_of(side, usize::MAX), self.panes.row_of(RESULT, usize::MAX));
        out.push(Segment { left: s..side_end.max(s), right: r..result_end.max(r), change: None, kind: crate::git::diff::RowKind::Equal });
        // Each side lists its pairs left to right as (side, result).
        if side == THEIRS {
            for seg in &mut out {
                std::mem::swap(&mut seg.left, &mut seg.right);
            }
        }
        out
    }

    /// `>>` / `<<`: puts a side's lines into the result, in place of what
    /// is there, or after the other side's when that was applied already.
    fn apply_side(&mut self, ix: usize, ours: bool, cx: &mut Context<Self>) {
        let Some(state) = self.states().get(ix).cloned() else { return };
        if state.side(ours) != SideState::Pending {
            return;
        }
        let change = self.changes[ix].clone();
        let pane = if ours { OURS } else { THEIRS };
        let append = state.side(!ours) == SideState::Applied;
        let (text, range) = replacement(&self.panes.buffers[pane], change.range(pane), &self.panes.buffers[RESULT], state.result.clone(), append);
        self.replace_result(ix, range, &text);
        let s = &mut self.panes.extra[ix];
        *s.side_mut(ours) = SideState::Applied;
        // The same change on both sides is applied once.
        if !change.conflict && s.side(!ours) == SideState::Pending {
            *s.side_mut(!ours) = SideState::Applied;
        }
        self.refresh();
        cx.notify();
    }

    /// Replaces result text belonging to change `ix`, growing or shrinking
    /// its range and moving the changes below.
    fn replace_result(&mut self, ix: usize, range: Range<usize>, text: &str) {
        let before = self.panes.buffers[RESULT].line_count() as isize;
        let caret = range.start;
        self.panes.edit(range, text, EditKind::Other, Some(caret));
        self.panes.line_edits.clear();
        let delta = self.panes.buffers[RESULT].line_count() as isize - before;
        let shift = |n: usize| (n as isize + delta).max(0) as usize;
        let states = &mut self.panes.extra;
        states[ix].result.end = shift(states[ix].result.end).max(states[ix].result.start);
        for s in &mut states[ix + 1..] {
            s.result = shift(s.result.start)..shift(s.result.end);
        }
        let language = crate::ui::file_editor::language_for(&self.conflict.path);
        self.panes.highlight(RESULT, language);
    }

    /// `×`: leaves the side's change out.
    fn ignore_side(&mut self, ix: usize, ours: bool, cx: &mut Context<Self>) {
        if self.states().get(ix).is_none_or(|s| s.side(ours) != SideState::Pending) {
            return;
        }
        self.panes.record_extra();
        *self.panes.extra[ix].side_mut(ours) = SideState::Ignored;
        cx.notify();
    }

    /// The magic wand on one conflict.
    fn resolve_simple(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(text) = self.changes.get(ix).and_then(|c| c.simple.clone()) else { return };
        let state = self.states()[ix].clone();
        if state.resolved() {
            return;
        }
        let result = &self.panes.buffers[RESULT];
        let range = result.lines_span(state.result.clone());
        // Keep the result's own ending at the end of the file.
        let text = if range.end == result.text().len() && !result.text().ends_with('\n') && !result.text().is_empty() {
            text.trim_end_matches('\n').to_owned()
        } else {
            text
        };
        self.replace_result(ix, range, &text);
        let s = &mut self.panes.extra[ix];
        s.ours = SideState::Applied;
        s.theirs = SideState::Applied;
        self.refresh();
        cx.notify();
    }

    /// Apply Non-Conflicting Changes from the left (`Some(true)`), the right
    /// (`Some(false)`) or both sides (`None`).
    fn apply_non_conflicting(&mut self, from: Option<bool>, cx: &mut Context<Self>) {
        for ix in 0..self.changes.len() {
            let change = &self.changes[ix];
            if change.conflict {
                continue;
            }
            for ours in [true, false] {
                if from.is_none_or(|f| f == ours) && self.states()[ix].side(ours) == SideState::Pending {
                    self.apply_side(ix, ours, cx);
                }
            }
        }
        self.go_to_first_unresolved();
    }

    /// Resolve Simple Conflicts: the magic wand on every conflict it can do.
    fn resolve_all_simple(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.changes.len() {
            if self.changes[ix].simple.is_some() {
                self.resolve_simple(ix, cx);
            }
        }
        self.go_to_first_unresolved();
    }

    fn counts(&self) -> (usize, usize) {
        let (mut changes, mut conflicts) = (0, 0);
        for (change, state) in self.changes.iter().zip(self.states()) {
            if !state.resolved() {
                if change.conflict { conflicts += 1 } else { changes += 1 }
            }
        }
        (changes, conflicts)
    }

    /// Shows a change on all three panes and puts the caret on its first
    /// line, in the pane that has the caret (the result at first).
    fn show_change(&mut self, ix: usize) {
        let Some(change) = self.changes.get(ix) else { return };
        let result = self.states()[ix].result.start;
        self.current = Some(ix);
        let rows = vec![(OURS, self.panes.row_of(OURS, change.ours.start)), (RESULT, self.panes.row_of(RESULT, result)), (THEIRS, self.panes.row_of(THEIRS, change.theirs.start))];
        self.panes.show_rows(rows);
        let pane = self.caret_pane();
        let line = self.change_lines(ix, pane).start;
        let buffer = &self.panes.buffers[pane];
        let offset = buffer.line_range(line.min(buffer.line_count().saturating_sub(1))).start;
        self.panes.caret = Some((pane, crate::ui::text_buffer::Selection::caret(offset)));
    }

    fn caret_pane(&self) -> usize {
        self.panes.caret.map_or(RESULT, |c| c.0)
    }

    /// A change's lines on a pane.
    fn change_lines(&self, ix: usize, pane: usize) -> Range<usize> {
        match pane {
            RESULT => self.states()[ix].result.clone(),
            _ => self.changes[ix].range(pane),
        }
    }

    fn unresolved(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.changes.len()).filter(|ix| !self.states()[*ix].resolved())
    }

    /// The first unresolved change (on opening, after applying changes).
    fn go_to_first_unresolved(&mut self) {
        let first = self.unresolved().next();
        if let Some(ix) = first {
            self.show_change(ix);
        }
    }

    /// Next Change: the first unresolved change below the caret's line.
    fn next_change(&self) -> Option<usize> {
        let pane = self.caret_pane();
        let Some((_, sel)) = self.panes.caret else { return self.unresolved().next() };
        let line = self.panes.buffers[pane].line_of(sel.head);
        self.unresolved().find(|ix| self.change_lines(*ix, pane).start > line)
    }

    /// Previous Change: the last unresolved change above the caret's line.
    fn previous_change(&self) -> Option<usize> {
        let pane = self.caret_pane();
        let (_, sel) = self.panes.caret?;
        let line = self.panes.buffers[pane].line_of(sel.head);
        self.unresolved().filter(|ix| {
            let lines = self.change_lines(*ix, pane);
            lines.start < line && lines.end <= line
        })
        .last()
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.next_change() {
            self.show_change(ix);
            cx.notify();
        }
    }

    pub fn previous_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.previous_change() {
            self.show_change(ix);
            cx.notify();
        }
    }

    fn result_text(&self) -> String {
        self.panes.buffers[RESULT].text().to_owned()
    }

    /// Apply: saves the result and marks the file resolved; asks first
    /// while changes are left unresolved.
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (changes, conflicts) = self.counts();
        if changes + conflicts > 0 {
            let entity = cx.entity();
            let left = changes + conflicts;
            crate::ui::dialogs::confirm(
                "Apply Changes",
                format!("There {} {left} unresolved change{} left. Save the result and mark the file as resolved anyway?", if left == 1 { "is" } else { "are" }, if left == 1 { "" } else { "s" }),
                "Apply",
                move |cx| entity.update(cx, |this, cx| this.finish(cx)),
                window,
                cx,
            );
            return;
        }
        self.finish(cx);
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        let content = self.result_text();
        let path = self.conflict.path.clone();
        self.model.update(cx, |model, cx| {
            model.run_operation(
                "Resolve Conflict",
                move |repo| {
                    merge::resolve_with(repo, &path, &content)?;
                    Ok(format!("{path} merged"))
                },
                cx,
            )
        });
        cx.emit(MergeEvent::Closed(true));
    }

    /// Cancel: asks before throwing away what was merged so far.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.untouched() {
            cx.emit(MergeEvent::Closed(false));
            return;
        }
        let entity = cx.entity();
        crate::ui::dialogs::confirm(
            "Cancel Visual Merge",
            "Are you sure you want to exit without applying changes?",
            "Discard Changes",
            move |cx| entity.update(cx, |_, cx| cx.emit(MergeEvent::Closed(false))),
            window,
            cx,
        );
    }

    /// Accept Left / Accept Right: the whole file from one side.
    fn accept(&mut self, ours: bool, cx: &mut Context<Self>) {
        let conflict = self.conflict.clone();
        self.model.update(cx, |model, cx| {
            model.run_operation(
                "Resolve Conflict",
                move |repo| {
                    merge::accept(repo, &conflict, ours)?;
                    Ok(format!("{} resolved", conflict.path))
                },
                cx,
            )
        });
        cx.emit(MergeEvent::Closed(true));
    }

    /// The colors of each visible row of a pane.
    fn row_looks(&self, pane: usize, palette: &Palette) -> Vec<RowLook> {
        let rows = self.panes.visible_rows(pane);
        let mut looks: Vec<RowLook> = rows.clone().map(|_| RowLook::default()).collect();
        for (change, state) in self.changes.iter().zip(self.states()) {
            if state.resolved() {
                continue;
            }
            let (range, kind) = match pane {
                RESULT => {
                    let kind = if change.conflict {
                        Kind::Conflict
                    } else if state.ours == SideState::Pending {
                        change.kind(true)
                    } else {
                        change.kind(false)
                    };
                    (state.result.clone(), kind)
                }
                _ => {
                    let ours = pane == OURS;
                    if state.side(ours) != SideState::Pending {
                        continue;
                    }
                    (change.range(pane), change.kind(ours))
                }
            };
            let Some(colors) = kind.colors(palette) else { continue };
            let word = match kind {
                Kind::Conflict => palette.diff_conflict_word,
                Kind::Inserted => palette.diff_inserted_word,
                Kind::Deleted => palette.diff_deleted_word,
                _ => palette.diff_modified_word,
            };
            // The result shows the base's changed words while it still is the base.
            let words = match pane {
                OURS => &change.words_ours,
                THEIRS => &change.words_theirs,
                _ if state.ours != SideState::Applied && state.theirs != SideState::Applied && range.len() == change.base.len() => &change.words_base,
                _ => &Vec::new(),
            };
            let first = self.panes.row_of(pane, range.start);
            for (k, row) in (first..first + range.len()).enumerate() {
                if rows.contains(&row) {
                    let look = &mut looks[row - rows.start];
                    // Do not highlight: only the gutter marker.
                    if self.highlight == HighlightMode::None {
                        look.marker = Some(colors.border);
                        continue;
                    }
                    look.background = Some(colors.fill);
                    look.gutter = Some(colors.fill);
                    look.top = (k == 0).then_some(colors.border);
                    look.bottom = (k + 1 == range.len()).then_some(colors.border);
                    look.words = words.get(k).map(|w| w.iter().map(|r| (r.clone(), word)).collect()).unwrap_or_default();
                }
            }
        }
        looks
    }

    /// Insertion lines on empty ranges and the gutter buttons of each change.
    fn overlays(&self, pane: usize, palette: &Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let height = self.panes.view_height.get().max(1600.);
        let layout = self.panes.layouts[pane];
        for (ix, (change, state)) in self.changes.iter().zip(self.states()).enumerate() {
            if state.resolved() {
                continue;
            }
            let ours = pane == OURS;
            let (range, kind) = match pane {
                RESULT => (state.result.clone(), if change.conflict { Kind::Conflict } else { change.kind(state.ours == SideState::Pending) }),
                _ if state.side(ours) == SideState::Pending => (change.range(pane), change.kind(ours)),
                _ => continue,
            };
            let block = self.block(ix, pane);
            let y = self.panes.row_top(pane, block.start);
            if self.panes.row_top(pane, block.end) < -line_height() || y > height {
                continue;
            }
            let Some(colors) = kind.colors(palette) else { continue };
            if range.is_empty() {
                out.push(div().absolute().left_0().right_0().top(px(y)).h(px(1.)).bg(colors.border).into_any_element());
            }
            // On an empty side the buttons sit on the row below the insertion line.
            let top = y;
            let column = |width: f32| {
                h_flex()
                    .absolute()
                    .top(px(top))
                    .h(px(line_height()))
                    .w(px(width))
                    .justify_center()
                    .items_center()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            };
            match pane {
                RESULT => {
                    if change.simple.is_some() {
                        out.push(
                            column(layout.buttons)
                                .left(px(layout.buttons_offset()))
                                .child(
                                    tool_button(("merge-wand", ix), icons::MAGIC_RESOLVE, "Resolve simple conflict")
                                        .on_click(cx.listener(move |this, _, _, cx| this.resolve_simple(ix, cx))),
                                )
                                .into_any_element(),
                        );
                    }
                }
                _ => {
                    let apply = tool_button(
                        (if ours { "merge-apply-left" } else { "merge-apply-right" }, ix),
                        if ours { icons::DIFF_ARROW_RIGHT } else { icons::DIFF_ARROW },
                        if state.side(!ours) == SideState::Applied { "Append" } else { "Accept" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.apply_side(ix, ours, cx)));
                    let ignore = tool_button((if ours { "merge-ignore-left" } else { "merge-ignore-right" }, ix), icons::CLOSE, "Ignore")
                        .on_click(cx.listener(move |this, _, _, cx| this.ignore_side(ix, ours, cx)));
                    let el = column(layout.buttons);
                    out.push(if ours {
                        el.right(px(layout.buttons_offset())).child(ignore).child(apply).into_any_element()
                    } else {
                        el.left(px(layout.buttons_offset())).child(apply).child(ignore).into_any_element()
                    });
                }
            }
        }
        out
    }

    /// A divider between a side and the result: each pending change of that
    /// side, joined to its lines in the result.
    fn divider(&self, side: usize, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let ours = side == OURS;
        let connectors: Vec<Connector> = self
            .changes
            .iter()
            .zip(self.states())
            .enumerate()
            .filter(|(_, (_, s))| s.side(ours) == SideState::Pending)
            .filter_map(|(ix, (c, _))| {
                let colors = c.kind(ours).colors(palette)?;
                let (side, result) = (self.block(ix, if ours { OURS } else { THEIRS }), self.block(ix, RESULT));
                let (left, right) = if ours { (side, result) } else { (result, side) };
                Some(Connector { left, right, colors })
            })
            .collect();
        let (l, r) = if ours { (OURS, RESULT) } else { (RESULT, THEIRS) };
        let scroll = (self.panes.scroll[l].1, self.panes.scroll[r].1);
        let tops = (self.panes.tops(l).to_vec(), self.panes.tops(r).to_vec());
        self.panes.divider_area(if ours { OURS } else { RESULT }, cx).child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| paint_divider(bounds, scroll, (&tops.0, &tops.1), &connectors, &[], gpui_kit::transparent_black(), window))
                },
            )
            .size_full(),
        )
    }

    fn stripe_marks(&self, pane: usize, palette: &Palette) -> Vec<(Range<usize>, Hsla)> {
        let ours = pane == OURS;
        self.changes
            .iter()
            .zip(self.states())
            .enumerate()
            .filter(|(_, (_, s))| s.side(ours) == SideState::Pending)
            .filter_map(|(ix, (c, _))| Some((self.block(ix, pane), c.kind(ours).colors(palette)?.border)))
            .collect()
    }
}

impl PaneHost for MergeView {
    type Extra = Vec<ChangeState>;

    fn panes(&mut self) -> &mut TextPanes<Vec<ChangeState>> {
        &mut self.panes
    }

    fn text_panes(&self) -> &TextPanes<Vec<ChangeState>> {
        &self.panes
    }

    /// Typing in the result moves the changes below the edit and stretches
    /// a change that the edit touches. Undo restores them as they were.
    fn edited(&mut self, _: &mut Window, _: &mut Context<Self>) {
        let edits = std::mem::take(&mut self.panes.line_edits);
        if !self.panes.restored {
            for edit in edits {
                let delta = edit.new_last as isize - edit.old_last as isize;
                let shift = |n: usize| (n as isize + delta).max(0) as usize;
                for state in &mut self.panes.extra {
                    let r = state.result.clone();
                    state.result = if edit.old_last < r.start {
                        shift(r.start)..shift(r.end)
                    } else if edit.first >= r.end {
                        r
                    } else {
                        r.start.min(edit.first)..shift(r.end.max(edit.old_last + 1))
                    };
                }
            }
        }
        let language = crate::ui::file_editor::language_for(&self.conflict.path);
        self.panes.highlight(RESULT, language);
        self.refresh();
    }

    fn open_fold(&mut self, id: usize, cx: &mut Context<Self>) {
        self.expanded.insert(id);
        self.refresh();
        cx.notify();
    }
}

crate::impl_pane_input!(MergeView);

fn ignore_label(value: IgnoreWhitespace) -> &'static str {
    match value {
        IgnoreWhitespace::None => "Do not ignore",
        IgnoreWhitespace::Trim => "Trim whitespaces",
        IgnoreWhitespace::All => "Ignore whitespaces",
        IgnoreWhitespace::AllAndEmptyLines => "Ignore whitespaces and empty lines",
    }
}

fn highlight_label(value: HighlightMode) -> &'static str {
    match value {
        HighlightMode::Words => "Highlight words",
        HighlightMode::Lines => "Highlight lines",
        HighlightMode::Split => "Highlight split changes",
        HighlightMode::Characters => "Highlight characters",
        HighlightMode::None => "Do not highlight",
    }
}

impl Render for MergeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.panes.before_render();
        self.panes.apply_settings(cx);
        let settings = crate::settings::Settings::get(cx).diff.clone();
        if settings.context_lines != self.context_lines || settings.align_changes != self.align {
            self.context_lines = settings.context_lines;
            self.align = settings.align_changes;
            self.refresh();
        }
        let palette = cx.palette().clone();
        let (changes, conflicts) = self.counts();
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let summary = match (changes, conflicts) {
            (0, 0) => "All changes have been processed".to_owned(),
            (c, 0) => plural(c, "change"),
            (0, k) => plural(k, "conflict"),
            (c, k) => format!("{}. {}", plural(c, "change"), plural(k, "conflict")),
        };
        let all_done = changes == 0 && conflicts == 0 && !self.changes.is_empty();
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border);
        let has_simple = self.changes.iter().zip(self.states()).any(|(c, s)| c.simple.is_some() && !s.resolved());
        let has_non_conflicting = |ours: Option<bool>| {
            self.changes.iter().zip(self.states()).any(|(c, s)| {
                !c.conflict && [true, false].iter().any(|o| ours.is_none_or(|x| x == *o) && s.side(*o) == SideState::Pending)
            })
        };

        let toolbar = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_0p5()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(
                tool_button("merge-prev", icons::UP, "Previous Change (Shift+F7)")
                    .disabled(self.previous_change().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.previous_difference(cx))),
            )
            .child(
                tool_button("merge-next", icons::DOWN, "Next Change (F7)")
                    .disabled(self.next_change().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.next_difference(cx))),
            )
            .child(separator())
            .child(
                tool_button("merge-apply-left-all", icons::DIFF_ARROW_RIGHT, "Apply Non-Conflicting Changes from the Left Side")
                    .disabled(!has_non_conflicting(Some(true)))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_non_conflicting(Some(true), cx))),
            )
            .child(
                tool_button("merge-apply-all", icons::CHECKED, "Apply All Non-Conflicting Changes")
                    .disabled(!has_non_conflicting(None))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_non_conflicting(None, cx))),
            )
            .child(
                tool_button("merge-apply-right-all", icons::DIFF_ARROW, "Apply Non-Conflicting Changes from the Right Side")
                    .disabled(!has_non_conflicting(Some(false)))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_non_conflicting(Some(false), cx))),
            )
            .child(
                tool_button("merge-wand-all", icons::MAGIC_RESOLVE, "Resolve Simple Conflicts")
                    .disabled(!has_simple)
                    .on_click(cx.listener(|this, _, _, cx| this.resolve_all_simple(cx))),
            )
            .child(separator())
            .child({
                use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
                let entity = cx.entity();
                let (ignore, untouched) = (self.ignore, self.untouched());
                gpui_kit::component::button::Button::new("merge-whitespace").ghost().xsmall().label(ignore_label(ignore)).dropdown_menu(move |mut menu, _, _| {
                    for value in [IgnoreWhitespace::None, IgnoreWhitespace::Trim, IgnoreWhitespace::All, IgnoreWhitespace::AllAndEmptyLines] {
                        let entity = entity.clone();
                        // Comparing again would lose what was resolved.
                        menu = menu.item(
                            PopupMenuItem::new(ignore_label(value))
                                .checked(ignore == value)
                                .disabled(!untouched && ignore != value)
                                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.set_ignore(value, cx))),
                        );
                    }
                    menu
                })
            })
            .child({
                use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
                let entity = cx.entity();
                let highlight = self.highlight;
                gpui_kit::component::button::Button::new("merge-highlight").ghost().xsmall().label(highlight_label(highlight)).dropdown_menu(move |mut menu, _, _| {
                    for value in [HighlightMode::Words, HighlightMode::Lines, HighlightMode::Characters, HighlightMode::None] {
                        let entity = entity.clone();
                        menu = menu.item(
                            PopupMenuItem::new(highlight_label(value))
                                .checked(highlight == value)
                                .on_click(move |_, _, cx| entity.update(cx, |this, cx| this.set_highlight(value, cx))),
                        );
                    }
                    menu
                })
            })
            .child(separator())
            .child(tool_button("merge-collapse", icons::COLLAPSE_ALL, "Collapse Unchanged Fragments").selected(self.collapse).on_click(cx.listener(
                |this, _, _, cx| {
                    this.collapse = !this.collapse;
                    this.expanded.clear();
                    this.refresh();
                    cx.notify();
                },
            )))
            .child(tool_button("merge-sync", icons::SYNCHRONIZE_SCROLLING, "Synchronize Scrolling").selected(self.panes.sync).on_click(cx.listener(|this, _, _, cx| {
                this.panes.sync = !this.panes.sync;
                cx.notify();
            })))
            .child({
                use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
                let entity = cx.entity();
                gpui_kit::component::button::Button::new("merge-compare")
                    .ghost()
                    .xsmall()
                    .icon(icons::VCS_DIFF)
                    .tooltip("Compare Contents")
                    .dropdown_menu(move |menu, _, _| {
                        let item = |label: &'static str, old: usize, new: usize| {
                            let entity = entity.clone();
                            PopupMenuItem::new(label).on_click(move |_, _, cx| entity.update(cx, |this, cx| this.compare(old, new, cx)))
                        };
                        menu.item(item("Compare Left with Base", 1, 0))
                            .item(item("Compare Right with Base", 1, 2))
                            .item(item("Compare Result with Base", 1, 3))
                            .separator()
                            .item(item("Compare Left with Right", 0, 2))
                            .item(item("Compare Left with Result", 0, 3))
                            .item(item("Compare Right with Result", 2, 3))
                    })
            })
            .child({
                use gpui_kit::component::menu::DropdownMenu as _;
                gpui_kit::component::button::Button::new("merge-gear")
                    .ghost()
                    .xsmall()
                    .icon(icons::SETTINGS)
                    .tooltip("Settings")
                    .dropdown_menu(|menu, window, cx| crate::ui::text_panes::gear_menu(menu, true, window, cx))
            })
            .child(
                tool_button("merge-help", icons::QUESTION_MARK, "Help")
                    .on_click(|_, _, cx| cx.open_url("https://www.jetbrains.com/help/idea/resolve-conflicts.html")),
            )
            .child(separator())
            .child(common::icon(common::file_icon(&self.conflict.path)).text_color(palette.text_secondary))
            .child(div().ml_1().text_sm().child(format!("Merge Revisions for {}", self.conflict.path)))
            .child(div().flex_1())
            .child(div().text_xs().text_color(palette.text_secondary).child(summary));

        let title = |text: &str, lock: bool, pane: usize| {
            let weight = self.panes.weight(pane);
            h_flex()
                .flex_basis(px(0.))
                .map(move |mut el| {
                    el.style().flex_grow = Some(weight);
                    el
                })
                .min_w_0()
                .px(px(6.))
                .gap_1p5()
                .when(lock, |el| el.child(common::icon(icons::LOCKED).text_color(palette.text_secondary)))
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.to_owned()))
        };
        let header = h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(STRIPE_WIDTH)))
            .child(title(&self.titles.0, true, OURS))
            .child(div().w(px(DIVIDER_WIDTH)))
            .child(title("Result", false, RESULT))
            .child(div().w(px(DIVIDER_WIDTH)))
            .child(title(&self.titles.1, true, THEIRS))
            .child(div().w(px(STRIPE_WIDTH)));

        let mut panes = Vec::new();
        for pane in 0..3 {
            let looks = self.row_looks(pane, &palette);
            let overlays = self.overlays(pane, &palette, cx);
            panes.push(self.panes.render_pane(pane, PaneContent { looks, overlays }, &palette, window, cx));
        }
        let mut panes = panes.into_iter();
        let (left, center, right) = (panes.next().unwrap(), panes.next().unwrap(), panes.next().unwrap());
        let thumb = palette.text_disabled.opacity(0.25);
        let body = pane_area("merge-panes", &self.panes.focus, cx)
            .child(self.panes.render_stripe(OURS, self.stripe_marks(OURS, &palette), thumb, cx))
            .child(left)
            .child(self.divider(OURS, &palette, cx))
            .child(center)
            .child(self.divider(THEIRS, &palette, cx))
            .child(right)
            .child(self.panes.render_stripe(THEIRS, self.stripe_marks(THEIRS, &palette), thumb, cx))
            .context_menu({
                let entity = cx.entity();
                move |menu, _, cx| crate::ui::text_panes::edit_menu(menu, &entity, cx)
            });

        v_flex()
            .size_full()
            .child(toolbar)
            .child(header)
            .when(all_done, |el| {
                el.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .text_sm()
                        .bg(palette.diff_inserted)
                        .border_b_1()
                        .border_color(palette.border)
                        .child("All changes have been processed.")
                        .child(
                            div()
                                .id("merge-finish")
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Save changes and finish merging")
                                .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
                        ),
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
                    .child(Button::new("merge-accept-left").outline().small().label("Accept Left").on_click(cx.listener(|this, _, _, cx| this.accept(true, cx))))
                    .child(Button::new("merge-accept-right").outline().small().label("Accept Right").on_click(cx.listener(|this, _, _, cx| this.accept(false, cx))))
                    .child(div().flex_1())
                    .child(Button::new("merge-cancel").outline().small().label("Cancel").on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))))
                    .child(Button::new("merge-apply").primary().small().label("Apply").on_click(cx.listener(|this, _, window, cx| this.apply(window, cx)))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_what_each_side_did() {
        let change = |base: Range<usize>, ours: Range<usize>, conflict| Change {
            base,
            ours,
            theirs: 0..0,
            ours_changed: true,
            theirs_changed: false,
            conflict,
            simple: None,
            words_ours: Vec::new(),
            words_theirs: Vec::new(),
            words_base: Vec::new(),
        };
        assert_eq!(change(2..2, 2..4, false).kind(true), Kind::Inserted);
        assert_eq!(change(2..4, 2..2, false).kind(true), Kind::Deleted);
        assert_eq!(change(2..4, 2..5, false).kind(true), Kind::Modified);
        assert_eq!(change(2..4, 2..5, false).kind(false), Kind::None);
        assert_eq!(change(2..4, 2..5, true).kind(false), Kind::Conflict);
    }
}

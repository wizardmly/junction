//! IntelliJ's console (`ConsoleViewImpl`): output in a read-only editor,
//! so a drag selects across lines as in any editor, Ctrl+C / Ctrl+A copy,
//! Ctrl+F finds; lines can be colored (errors red, commands gray) and carry
//! styled spans (Logcat's level badge, tag colors), and the hyperlinks its
//! filters add (web addresses, `File.kt:12:5`, a stack frame) open with a
//! click. It follows new output while scrolled to the end, as IntelliJ's
//! sticks to the end: scrolled up, it stays put; lines can be added
//! at the end and dropped from the start (a log's buffer) without redoing
//! the rest.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString, Styled as _, UnderlineStyle, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::ansi::{self, AnsiColor, AnsiStyle};
use crate::domain::console_links::{self, ConsoleLink};
use crate::theme::ActivePalette as _;

pub enum ConsoleEvent {
    /// A file link: an absolute path, 0-based line and column.
    OpenFile { path: std::path::PathBuf, line: u32, col: u32 },
    /// A stack frame link (`at com.example.Foo.bar(Foo.kt:42)`): the
    /// class's package, the file and the 1-based line.
    OpenFrame { package: String, file: String, line: u32 },
    /// The toolbar's Clear All: the owner empties what the console shows.
    ClearAll,
}

/// A line to show, with its color, styled spans and links (in the line).
#[derive(Clone, Default)]
pub struct ConsoleLine {
    pub text: SharedString,
    pub color: Option<Hsla>,
    pub spans: Vec<(Range<usize>, HighlightStyle)>,
    /// Links besides the ones found in the text (web addresses, paths).
    pub links: Vec<(Range<usize>, ConsoleLink)>,
}

impl ConsoleLine {
    pub fn plain(text: impl Into<SharedString>, color: Option<Hsla>) -> Self {
        Self { text: text.into(), color, ..Default::default() }
    }

    /// A line of program output: its ANSI escape codes become styles
    /// (`state` carries them from line to line).
    pub fn ansi(text: &SharedString, color: Option<Hsla>, state: &mut AnsiStyle, dark: bool) -> Self {
        if !ansi::has_escapes(text) && state.is_plain() {
            return Self::plain(text.clone(), color);
        }
        let (plain, styled, hyperlinks) = ansi::decode_links(text, state);
        let spans = styled.into_iter().map(|(range, style)| (range, ansi_highlight(&style, dark))).collect();
        let links = hyperlinks.into_iter().map(|(range, uri)| (range, console_links::hyperlink(&uri))).collect();
        Self { text: plain.into(), color, spans, links }
    }
}

/// The console's ANSI colors: Darcula's and IntelliJ Light's.
fn ansi_rgb(color: AnsiColor, dark: bool) -> Hsla {
    const DARK: [u32; 16] = [0x7f7f7f, 0xff6b68, 0xa8c023, 0xd6bf55, 0x5394ec, 0xae8abe, 0x299999, 0xbbbbbb, 0x999999, 0xff8785, 0xc2d64f, 0xf2d55e, 0x7eaef1, 0xc794d2, 0x6cc4c4, 0xffffff];
    const LIGHT: [u32; 16] = [0x000000, 0xc91b00, 0x00a600, 0x8e7700, 0x0225c7, 0xb22fb2, 0x00838f, 0x808080, 0x686868, 0xff3b30, 0x1f9e1f, 0xa68a0d, 0x3b6ef5, 0xd22fd2, 0x00a6b2, 0x1e1e1e];
    let value = match color {
        AnsiColor::Named(n) => (if dark { DARK } else { LIGHT })[n as usize % 16],
        AnsiColor::Indexed(n) => {
            let (r, g, b) = ansi::indexed_rgb(n);
            (r as u32) << 16 | (g as u32) << 8 | b as u32
        }
        AnsiColor::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
    };
    gpui_kit::Rgba::from(gpui_kit::rgb(value)).into()
}

fn ansi_highlight(style: &AnsiStyle, dark: bool) -> HighlightStyle {
    let hex = |value: u32| -> Hsla { gpui_kit::Rgba::from(gpui_kit::rgb(value)).into() };
    // The console's own text and background, for inverse and dim.
    let (text, back) = if dark { (hex(0xbbbbbb), hex(0x1e1f22)) } else { (hex(0x1e1e1e), hex(0xffffff)) };
    let (mut fg, mut bg) = (style.fg.map(|c| ansi_rgb(c, dark)), style.bg.map(|c| ansi_rgb(c, dark)));
    if style.inverse {
        (fg, bg) = (Some(bg.unwrap_or(back)), Some(fg.unwrap_or(text)));
    }
    if style.dim {
        // Halfway to the background (the editor paints text opaque).
        let c = fg.unwrap_or(text);
        fg = Some(Hsla { l: c.l + (back.l - c.l) * 0.45, ..c });
    }
    HighlightStyle {
        color: fg,
        background_color: bg,
        font_weight: style.bold.then_some(gpui_kit::FontWeight::BOLD),
        font_style: style.italic.then_some(gpui_kit::FontStyle::Italic),
        underline: style.underline.then(|| UnderlineStyle { thickness: px(1.), color: fg, wavy: false }),
        ..Default::default()
    }
}

/// What the highlighter paints, in "virtual" offsets: the text's offset
/// plus `origin`, the length dropped from the start so far.
#[derive(Default)]
struct Painted {
    origin: usize,
    /// Each line's start and color.
    lines: VecDeque<(usize, Option<Hsla>)>,
    spans: VecDeque<(Range<usize>, HighlightStyle)>,
    links: VecDeque<(Range<usize>, ConsoleLink)>,
    link_color: Hsla,
}

struct ConsoleHighlighter(Rc<RefCell<Painted>>);

impl InputHighlighter for ConsoleHighlighter {
    fn language(&self) -> SharedString {
        "console".into()
    }

    fn update(&mut self, _: Option<InputEdit>, _: &Rope, _: bool, _: &mut Window, _: &mut Context<EditorState>) {}

    fn styles(&self, range: &Range<usize>, _: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        let p = self.0.borrow();
        let (start, end) = (range.start + p.origin, range.end + p.origin);
        // Boundaries: line starts, span and link edges inside the range.
        let mut cuts: Vec<usize> = vec![start, end];
        let first = p.lines.partition_point(|(s, _)| *s <= start).saturating_sub(1);
        cuts.extend(p.lines.range(first..).map(|(s, _)| *s).take_while(|s| *s < end).filter(|s| *s > start));
        let first_span = p.spans.partition_point(|(r, _)| r.end <= start);
        let spans: Vec<&(Range<usize>, HighlightStyle)> = p.spans.range(first_span..).take_while(|(r, _)| r.start < end).collect();
        let first_link = p.links.partition_point(|(r, _)| r.end <= start);
        let links: Vec<Range<usize>> = p.links.range(first_link..).take_while(|(r, _)| r.start < end).map(|(r, _)| r.clone()).collect();
        for r in spans.iter().map(|(r, _)| r).chain(links.iter()) {
            cuts.extend([r.start.max(start), r.end.min(end)]);
        }
        cuts.sort_unstable();
        cuts.dedup();
        cuts.windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let line = p.lines.partition_point(|(s, _)| *s <= a).saturating_sub(1);
                let mut style = HighlightStyle { color: p.lines.get(line).and_then(|(_, c)| *c), ..Default::default() };
                for (_, s) in spans.iter().filter(|(r, _)| r.start <= a && b <= r.end) {
                    style.color = s.color.or(style.color);
                    style.background_color = s.background_color.or(style.background_color);
                    style.font_weight = s.font_weight.or(style.font_weight);
                    style.font_style = s.font_style.or(style.font_style);
                    style.underline = s.underline.or(style.underline);
                }
                if links.iter().any(|r| r.start <= a && b <= r.end) {
                    style.color = Some(p.link_color);
                    style.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(p.link_color), wavy: false });
                }
                (a - p.origin..b - p.origin, style)
            })
            .collect()
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// How much text a console keeps: IntelliJ's cycle buffer, 1024 KB unless
/// Settings › Editor › General › Console overrides it.
fn cycle_buffer(cx: &gpui_kit::App) -> usize {
    match crate::settings::Settings::get(cx).console_buffer_kb {
        0 => 1024 * 1024,
        kb => kb as usize * 1024,
    }
}

pub struct ConsoleView {
    state: Entity<EditorState>,
    painted: Rc<RefCell<Painted>>,
    /// The shown lines' keys (for `sync`), and the generation they're of.
    ids: VecDeque<u64>,
    generation: u64,
    /// `set_lines`' lines, to append only what's new.
    texts: Vec<(SharedString, Option<Hsla>)>,
    /// Virtual end of the text, and where its last line starts: the caret
    /// while following the end (at a line's start, so long lines don't
    /// scroll the view sideways).
    end: usize,
    tail: usize,
    /// Android Studio's console toolbar on the left (Soft-Wrap, Scroll to
    /// the End, and Clear All when `clearable`); Logcat has its own.
    toolbar: Option<bool>,
    wrap: bool,
    /// The ANSI style the next appended line starts with.
    ansi: AnsiStyle,
    _observe: gpui_kit::Subscription,
}

impl EventEmitter<ConsoleEvent> for ConsoleView {}

impl ConsoleView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let painted = Rc::new(RefCell::new(Painted { link_color: cx.palette().link, ..Default::default() }));
        let factory_painted = painted.clone();
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).line_number(false).soft_wrap(false).indent_guides(false).folding(false).searchable(true);
            state.set_readonly(true, cx);
            let p = factory_painted.clone();
            state.set_highlighter_factory(Rc::new(move |_| Some(Box::new(ConsoleHighlighter(p.clone())) as Box<dyn InputHighlighter>)), cx);
            state
        });
        let _observe = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, painted, ids: VecDeque::new(), generation: 0, texts: Vec::new(), end: 0, tail: 0, toolbar: None, wrap: false, ansi: AnsiStyle::default(), _observe }
    }

    /// With Android Studio's console toolbar; Clear All when `clearable`
    /// (the owner handles [`ConsoleEvent::ClearAll`]).
    pub fn with_toolbar(mut self, clearable: bool) -> Self {
        self.toolbar = Some(clearable);
        self
    }

    pub fn set_soft_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
    }

    /// Whether the view follows new output: it shows the end
    /// (`ConsoleViewImpl.isStickingToEnd`).
    pub fn is_following(&self, cx: &gpui_kit::App) -> bool {
        self.state.read(cx).scrolled_to_end()
    }

    /// Scroll to End: the caret to the last line, following it.
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        let tail = self.tail - self.painted.borrow().origin;
        self.state.update(cx, |state, cx| {
            state.set_selected_range(tail..tail, cx);
            state.set_scroll_offset(gpui_kit::point(px(0.), px(-1.0e7)), cx);
        });
    }

    /// Appends lines' text and paint at virtual offset `self.end`; the
    /// text to insert (with the separating line breaks).
    fn add(&mut self, lines: &[ConsoleLine]) -> String {
        let mut p = self.painted.borrow_mut();
        let mut text = String::new();
        let mut at = self.end;
        for line in lines {
            if !p.lines.is_empty() {
                text.push('\n');
                at += 1;
            }
            p.lines.push_back((at, line.color));
            p.spans.extend(line.spans.iter().map(|(r, s)| (r.start + at..r.end + at, *s)));
            // Links found in the text give way to the line's own (OSC 8).
            let found_in_text = console_links::links(&line.text).into_iter().filter(|(r, _)| !line.links.iter().any(|(own, _)| own.start < r.end && r.start < own.end));
            let mut found: Vec<(Range<usize>, ConsoleLink)> = found_in_text.chain(line.links.iter().cloned()).collect();
            found.sort_by_key(|(r, _)| r.start);
            p.links.extend(found.into_iter().map(|(r, l)| (r.start + at..r.end + at, l)));
            self.tail = at;
            text.push_str(&line.text);
            at += line.text.len();
        }
        self.end = at;
        text
    }

    /// Shows these lines instead of what's shown.
    pub fn reset(&mut self, lines: &[ConsoleLine], window: &mut Window, cx: &mut Context<Self>) {
        {
            let mut p = self.painted.borrow_mut();
            p.origin = 0;
            p.lines.clear();
            p.spans.clear();
            p.links.clear();
        }
        self.end = 0;
        self.tail = 0;
        let text = self.add(lines);
        let tail = self.tail;
        self.state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.set_selected_range(tail..tail, cx);
            state.set_scroll_offset(gpui_kit::point(px(0.), px(-1.0e7)), cx);
        });
        cx.notify();
    }

    /// Adds lines at the end, following them unless the user moved away.
    pub fn push(&mut self, lines: &[ConsoleLine], window: &mut Window, cx: &mut Context<Self>) {
        if lines.is_empty() {
            return;
        }
        let follow = self.is_following(cx);
        let origin = self.painted.borrow().origin;
        let len = self.end - origin;
        let text = self.add(lines);
        let tail = self.tail - origin;
        self.state.update(cx, |state, cx| {
            let (selection, offset) = (state.selected_range(), state.scroll_offset());
            state.set_selected_range(len..len, cx);
            state.insert(text, window, cx);
            // Following: to the new end. Scrolled up: where it was, the
            // insert's scroll to the caret undone.
            state.set_selected_range(if follow { tail..tail } else { selection }, cx);
            state.set_scroll_offset(if follow { gpui_kit::point(px(0.), px(-1.0e7)) } else { offset }, cx);
        });
        cx.notify();
    }

    /// Drops the first `n` lines (a log's buffer is full).
    pub fn drop_front(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let removed = {
            let mut p = self.painted.borrow_mut();
            if n == 0 || p.lines.is_empty() {
                return;
            }
            if n >= p.lines.len() {
                drop(p);
                return self.reset(&[], window, cx);
            }
            let new_origin = p.lines[n].0;
            let removed = new_origin - p.origin;
            p.lines.drain(..n);
            while p.spans.front().is_some_and(|(r, _)| r.end <= new_origin) {
                p.spans.pop_front();
            }
            while p.links.front().is_some_and(|(r, _)| r.end <= new_origin) {
                p.links.pop_front();
            }
            p.origin = new_origin;
            removed
        };
        let follow = self.is_following(cx);
        self.state.update(cx, |state, cx| {
            let (selection, offset) = (state.selected_range(), state.scroll_offset());
            state.set_selected_range(0..removed, cx);
            state.replace("", window, cx);
            let shift = |o: usize| o.saturating_sub(removed);
            state.set_selected_range(shift(selection.start)..shift(selection.end), cx);
            // The lines left above: the view keeps showing the same lines
            // (the end, when following).
            let gone = state.line_height().unwrap_or_default() * n as f32;
            let y = if follow { px(-1.0e7) } else { (offset.y + gone).min(px(0.)) };
            state.set_scroll_offset(gpui_kit::point(offset.x, y), cx);
        });
        cx.notify();
    }

    /// Shows the lines of `ids` (in order, new ones at the end): drops what
    /// left the start, appends what's new, else starts over. A new
    /// `generation` (a filter or format change) always starts over.
    pub fn sync(&mut self, ids: &VecDeque<u64>, generation: u64, line: &dyn Fn(u64) -> ConsoleLine, window: &mut Window, cx: &mut Context<Self>) {
        if generation != self.generation {
            self.generation = generation;
            self.ids = ids.clone();
            let lines: Vec<ConsoleLine> = ids.iter().map(|id| line(*id)).collect();
            return self.reset(&lines, window, cx);
        }
        if self.ids.len() == ids.len() && self.ids.front() == ids.front() && self.ids.back() == ids.back() {
            return;
        }
        let gone = match ids.front() {
            Some(first) => self.ids.partition_point(|id| id < first),
            None => self.ids.len(),
        };
        if gone > 0 {
            self.ids.drain(..gone);
            self.drop_front(gone, window, cx);
        }
        let kept = self.ids.len();
        if kept <= ids.len() && (kept == 0 || (self.ids.front() == ids.front() && self.ids.back() == ids.get(kept - 1))) {
            let fresh: Vec<u64> = ids.range(kept..).copied().collect();
            let lines: Vec<ConsoleLine> = fresh.iter().map(|id| line(*id)).collect();
            self.ids.extend(fresh);
            if kept == 0 {
                self.reset(&lines, window, cx);
            } else {
                self.push(&lines, window, cx);
            }
        } else {
            self.ids = ids.clone();
            let lines: Vec<ConsoleLine> = ids.iter().map(|id| line(*id)).collect();
            self.reset(&lines, window, cx);
        }
    }

    /// Shows these lines: appends when they extend what's shown, else
    /// starts over. Past IntelliJ's cycle buffer (`idea.cycle.buffer.size`,
    /// 1024 KB, or Settings' override) the oldest lines go.
    pub fn set_lines(&mut self, lines: &[(SharedString, Option<Hsla>)], window: &mut Window, cx: &mut Context<Self>) {
        let extends = lines.len() >= self.texts.len() && self.texts.iter().zip(lines).all(|(a, b)| a.0 == b.0);
        if extends && lines.len() == self.texts.len() {
            return;
        }
        let dark = cx.palette().dark;
        let as_lines = |l: &[(SharedString, Option<Hsla>)], state: &mut AnsiStyle| l.iter().map(|(t, c)| ConsoleLine::ansi(t, *c, state, dark)).collect::<Vec<_>>();
        if extends && !self.texts.is_empty() {
            let mut state = self.ansi;
            let fresh = as_lines(&lines[self.texts.len()..], &mut state);
            self.ansi = state;
            self.push(&fresh, window, cx);
        } else {
            let mut state = AnsiStyle::default();
            let all = as_lines(lines, &mut state);
            self.ansi = state;
            self.reset(&all, window, cx);
        }
        self.texts = lines.to_vec();
        let limit = cycle_buffer(cx);
        if self.end - self.painted.borrow().origin > limit {
            let n = {
                let p = self.painted.borrow();
                let cut = self.end - limit;
                // The first line starting at or past the cut stays.
                p.lines.partition_point(|(start, _)| *start < cut).max(1)
            };
            self.drop_front(n, window, cx);
        }
    }

    /// A click (not a drag) on a link follows it.
    fn click(&mut self, cx: &mut Context<Self>) {
        let range = self.state.read(cx).selected_range();
        if !range.is_empty() {
            return;
        }
        let link = {
            let p = self.painted.borrow();
            let at = range.start + p.origin;
            p.links.iter().find(|(r, _)| r.start <= at && at < r.end).map(|(_, l)| l.clone())
        };
        match link {
            Some(ConsoleLink::Url(url)) => cx.open_url(&url),
            Some(ConsoleLink::File { path, line, col }) => {
                cx.emit(ConsoleEvent::OpenFile { path: path.into(), line: line.unwrap_or(1).saturating_sub(1), col: col.unwrap_or(1).saturating_sub(1) })
            }
            Some(ConsoleLink::Frame { package, file, line }) => cx.emit(ConsoleEvent::OpenFrame { package, file, line }),
            None => {}
        }
    }
}

impl Render for ConsoleView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let editor = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .font_family(mono)
            .text_size(px(13.))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.click(cx)))
            .child(Editor::new(&self.state).bordered(false).h_full());
        let Some(clearable) = self.toolbar else { return gpui_kit::component::h_flex().size_full().child(editor) };
        use crate::ui::as_icons as icons;
        use crate::ui::common::tool_button;
        use gpui_kit::component::Selectable as _;
        let palette = cx.palette().clone();
        let strip = gpui_kit::component::v_flex()
            .flex_none()
            .w(px(28.))
            .h_full()
            .py_1()
            .gap_1()
            .items_center()
            .border_r_1()
            .border_color(palette.border)
            .child(tool_button("console-wrap", icons::SOFT_WRAP, "Soft-Wrap").selected(self.wrap).on_click(cx.listener(|this, _, window, cx| {
                this.wrap = !this.wrap;
                let wrap = this.wrap;
                this.set_soft_wrap(wrap, window, cx);
                cx.notify();
            })))
            .child(tool_button("console-end", icons::DOWN, "Scroll to the End").on_click(cx.listener(|this, _, _, cx| this.scroll_to_end(cx))))
            .when(clearable, |el| el.child(tool_button("console-clear", icons::CLEAR, "Clear All").on_click(cx.listener(|_, _, _, cx| cx.emit(ConsoleEvent::ClearAll)))));
        gpui_kit::component::h_flex().size_full().child(strip).child(editor)
    }
}

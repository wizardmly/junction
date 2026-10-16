//! IntelliJ's console (`ConsoleViewImpl`): output in a read-only editor,
//! so a drag selects across lines as in any editor, Ctrl+C / Ctrl+A copy,
//! Ctrl+F finds; lines can be colored (errors red, commands gray) and carry
//! styled spans (Logcat's level badge, tag colors), and the hyperlinks its
//! filters add (web addresses, `File.kt:12:5`, a stack frame) open with a
//! click. It follows the end while the caret is there; lines can be added
//! at the end and dropped from the start (a log's buffer) without redoing
//! the rest.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString, Styled as _, UnderlineStyle, Window, div, px};

use crate::domain::console_links::{self, ConsoleLink};
use crate::theme::ActivePalette as _;

pub enum ConsoleEvent {
    /// A file link: an absolute path, 0-based line and column.
    OpenFile { path: std::path::PathBuf, line: u32, col: u32 },
    /// A stack frame link (`at com.example.Foo.bar(Foo.kt:42)`): the
    /// class's package, the file and the 1-based line.
    OpenFrame { package: String, file: String, line: u32 },
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
        Self { state, painted, ids: VecDeque::new(), generation: 0, texts: Vec::new(), end: 0, tail: 0, _observe }
    }

    pub fn set_soft_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
    }

    /// Whether the view follows the end (the caret is on the last line).
    pub fn is_following(&self, cx: &gpui_kit::App) -> bool {
        let state = self.state.read(cx);
        let origin = self.painted.borrow().origin;
        let range = state.selected_range();
        range.is_empty() && range.start + origin >= self.tail
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
            let mut found: Vec<(Range<usize>, ConsoleLink)> = console_links::links(&line.text).into_iter().chain(line.links.iter().cloned()).collect();
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
            let selection = state.selected_range();
            state.set_selected_range(len..len, cx);
            state.insert(text, window, cx);
            state.set_selected_range(if follow { tail..tail } else { selection }, cx);
            if follow {
                state.set_scroll_offset(gpui_kit::point(px(0.), px(-1.0e7)), cx);
            }
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
        self.state.update(cx, |state, cx| {
            let selection = state.selected_range();
            state.set_selected_range(0..removed, cx);
            state.replace("", window, cx);
            let shift = |o: usize| o.saturating_sub(removed);
            state.set_selected_range(shift(selection.start)..shift(selection.end), cx);
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
    /// starts over.
    pub fn set_lines(&mut self, lines: &[(SharedString, Option<Hsla>)], window: &mut Window, cx: &mut Context<Self>) {
        let extends = lines.len() >= self.texts.len() && self.texts.iter().zip(lines).all(|(a, b)| a.0 == b.0);
        if extends && lines.len() == self.texts.len() {
            return;
        }
        let as_lines = |l: &[(SharedString, Option<Hsla>)]| l.iter().map(|(t, c)| ConsoleLine::plain(t.clone(), *c)).collect::<Vec<_>>();
        if extends && !self.texts.is_empty() {
            self.push(&as_lines(&lines[self.texts.len()..]), window, cx);
        } else {
            self.reset(&as_lines(lines), window, cx);
        }
        self.texts = lines.to_vec();
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
        div()
            .size_full()
            .font_family(mono)
            .text_size(px(13.))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.click(cx)))
            .child(Editor::new(&self.state).bordered(false).h_full())
    }
}

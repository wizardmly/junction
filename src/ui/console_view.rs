//! IntelliJ's console (`ConsoleViewImpl`): output in a read-only editor,
//! so a drag selects across lines as in any editor, Ctrl+C / Ctrl+A copy,
//! Ctrl+F finds; lines can be colored (errors red, commands gray), and the
//! hyperlinks its filters add (web addresses, `File.kt:12:5`) open with a
//! click. It follows the end while the caret is there.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString, Styled as _,
    UnderlineStyle, Window, div, px,
};

use crate::domain::console_links::{self, ConsoleLink};
use crate::theme::ActivePalette as _;

pub enum ConsoleEvent {
    /// A file link: an absolute path, 0-based line and column.
    OpenFile { path: std::path::PathBuf, line: u32, col: u32 },
}

/// What the highlighter paints: each line's color and the links.
#[derive(Default)]
struct Painted {
    /// Line start offsets and colors.
    lines: Vec<(usize, Option<Hsla>)>,
    links: Vec<(Range<usize>, ConsoleLink)>,
    link_color: Hsla,
}

struct ConsoleHighlighter(Rc<RefCell<Painted>>);

impl InputHighlighter for ConsoleHighlighter {
    fn language(&self) -> SharedString {
        "console".into()
    }

    fn update(&mut self, _: Option<InputEdit>, _: &Rope, _: bool, _: &mut Window, _: &mut Context<EditorState>) {}

    fn styles(&self, range: &Range<usize>, _: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        let painted = self.0.borrow();
        // Boundaries: line starts and link edges inside the range.
        let mut cuts: Vec<usize> = vec![range.start, range.end];
        let first = painted.lines.partition_point(|(s, _)| *s <= range.start).saturating_sub(1);
        cuts.extend(painted.lines[first..].iter().map(|(s, _)| *s).take_while(|s| *s < range.end).filter(|s| *s > range.start));
        let first_link = painted.links.partition_point(|(r, _)| r.end <= range.start);
        for (r, _) in painted.links[first_link..].iter().take_while(|(r, _)| r.start < range.end) {
            cuts.extend([r.start.max(range.start), r.end.min(range.end)]);
        }
        cuts.sort_unstable();
        cuts.dedup();
        cuts.windows(2)
            .map(|w| {
                let (start, end) = (w[0], w[1]);
                let line = painted.lines.partition_point(|(s, _)| *s <= start).saturating_sub(1);
                let mut style = HighlightStyle { color: painted.lines.get(line).and_then(|(_, c)| *c), ..Default::default() };
                if painted.links.iter().any(|(r, _)| r.start <= start && end <= r.end) {
                    style.color = Some(painted.link_color);
                    style.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(painted.link_color), wavy: false });
                }
                (start..end, style)
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
    /// The lines shown, to append only what's new.
    shown: Vec<(SharedString, Option<Hsla>)>,
    len: usize,
    /// Where the last line starts: the caret while following the end (at
    /// a line's start, so long lines don't scroll the view sideways).
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
        Self { state, painted, shown: Vec::new(), len: 0, tail: 0, _observe }
    }

    /// Shows these lines: appends when they extend what's shown, else
    /// starts over.
    pub fn set_lines(&mut self, lines: &[(SharedString, Option<Hsla>)], window: &mut Window, cx: &mut Context<Self>) {
        let extends = lines.len() >= self.shown.len() && self.shown.iter().zip(lines).all(|(a, b)| a.0 == b.0);
        if extends && lines.len() == self.shown.len() {
            return;
        }
        let new = if extends { &lines[self.shown.len()..] } else { lines };
        let mut text = String::new();
        let mut at = if extends { self.len } else { 0 };
        let mut tail = if extends { self.tail } else { 0 };
        {
            let mut painted = self.painted.borrow_mut();
            if !extends {
                painted.lines.clear();
                painted.links.clear();
            }
            for (i, (line, color)) in new.iter().enumerate() {
                if at > 0 || i > 0 || (extends && !self.shown.is_empty()) {
                    text.push('\n');
                    at += 1;
                }
                painted.lines.push((at, *color));
                tail = at;
                painted.links.extend(console_links::links(line).into_iter().map(|(r, l)| (r.start + at..r.end + at, l)));
                text.push_str(line);
                at += line.len();
            }
        }
        let (len, old_tail) = (self.len, self.tail);
        self.state.update(cx, |state, cx| {
            // The end in view (the offset is clamped to the text).
            let to_end = |state: &mut EditorState, cx: &mut Context<EditorState>| state.set_scroll_offset(gpui_kit::point(px(0.), px(-1.0e7)), cx);
            if !extends {
                state.set_value(text, window, cx);
                state.set_selected_range(tail..tail, cx);
                to_end(state, cx);
                return;
            }
            // Following the end unless the user put the caret or a selection elsewhere.
            let selection = state.selected_range();
            let follow = selection.is_empty() && selection.start == old_tail;
            state.set_selected_range(len..len, cx);
            state.insert(text, window, cx);
            state.set_selected_range(if follow { tail..tail } else { selection }, cx);
            if follow {
                to_end(state, cx);
            }
        });
        self.len = at;
        self.tail = tail;
        self.shown = lines.to_vec();
        cx.notify();
    }

    /// A click (not a drag) on a link follows it.
    fn click(&mut self, cx: &mut Context<Self>) {
        let range = self.state.read(cx).selected_range();
        if !range.is_empty() {
            return;
        }
        let link = self.painted.borrow().links.iter().find(|(r, _)| r.start <= range.start && range.start < r.end).map(|(_, l)| l.clone());
        match link {
            Some(ConsoleLink::Url(url)) => cx.open_url(&url),
            Some(ConsoleLink::File { path, line, col }) => {
                cx.emit(ConsoleEvent::OpenFile { path: path.into(), line: line.unwrap_or(1).saturating_sub(1), col: col.unwrap_or(1).saturating_sub(1) })
            }
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

//! The Terminal tool window (Alt+F12), as Android Studio's: a "Local" tab
//! per shell session, opened in the project's directory; + opens another,
//! ⌄ another shell, Ctrl+C copies a selection, Ctrl+V pastes.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Point as GridPoint, Side as CellSide};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use futures::StreamExt as _;
use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, ClipboardItem, Context, ElementInputHandler, Entity, EventEmitter, FocusHandle, Focusable, Font, FontStyle,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels,
    Render, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, StrikethroughStyle, Styled as _, Subscription, Task, TextAlign,
    TextRun, UTF16Selection, UnderlineStyle, Window, canvas, div, fill, point, prelude::FluentBuilder as _, px, size,
};

use crate::settings::Settings;
use crate::terminal::{self, GridSize, Session, ShellChoice, TermEvent};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;

/// Space around the text, as JediTerm leaves it.
const PAD_X: f32 = 8.;
const PAD_Y: f32 = 4.;

pub enum TerminalViewEvent {
    /// The shell exited: the tab closes.
    Exited,
    NewTab,
}

/// One terminal tab: the shell's screen, drawn cell by cell.
pub struct TerminalView {
    session: Session,
    focus: FocusHandle,
    /// Cell width and height, measured from the font.
    cell: Rc<Cell<(f32, f32)>>,
    /// Where the grid was painted last, for mouse positions.
    origin: Rc<Cell<gpui_kit::Point<Pixels>>>,
    /// IME composition shown at the cursor before it's committed.
    marked: Option<String>,
    selecting: bool,
    /// Wheel movement not yet a whole line.
    wheel: f32,
    _events: Task<()>,
    _intercept: Subscription,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    pub fn new(session: Session, mut events: futures::channel::mpsc::UnboundedReceiver<TermEvent>, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                // A burst of output wakes the view once.
                let mut batch = vec![event];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |this, cx| this.on_events(batch, cx)).is_err() {
                    break;
                }
            }
        });
        // Keys go to the shell before the IDE's shortcuts (Ctrl+E, Ctrl+N
        // and the like belong to the shell here), as in Android Studio.
        let weak = cx.entity().downgrade();
        let handle = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if !handle.is_focused(window) {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            if this.update(cx, |this, cx| this.on_keystroke(&event.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        Self {
            session,
            focus,
            cell: Rc::new(Cell::new((8., 16.))),
            origin: Rc::new(Cell::new(point(px(0.), px(0.)))),
            marked: None,
            selecting: false,
            wheel: 0.,
            _events: task,
            _intercept: intercept,
        }
    }

    fn on_events(&mut self, events: Vec<TermEvent>, cx: &mut Context<Self>) {
        for event in events {
            match event {
                TermEvent::PtyWrite(text) => self.session.write(text.into_bytes()),
                TermEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                TermEvent::ClipboardLoad(_, format) => {
                    let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                    self.session.write(format(&text).into_bytes());
                }
                TermEvent::ColorRequest(index, format) => {
                    let color = self.colors(cx).resolve_index(index);
                    let rgba = color.to_rgb();
                    let rgb = Rgb { r: (rgba.r * 255.) as u8, g: (rgba.g * 255.) as u8, b: (rgba.b * 255.) as u8 };
                    self.session.write(format(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    let (w, h) = self.cell.get();
                    let size = self.session.size();
                    let window = alacritty_terminal::event::WindowSize {
                        num_lines: size.lines as u16,
                        num_cols: size.columns as u16,
                        cell_width: w as u16,
                        cell_height: h as u16,
                    };
                    self.session.write(format(window).into_bytes());
                }
                TermEvent::ChildExit(_) | TermEvent::Exit => cx.emit(TerminalViewEvent::Exited),
                _ => {}
            }
        }
        cx.notify();
    }

    fn colors(&self, cx: &App) -> TermColors {
        let palette = cx.palette();
        TermColors::new(palette.dark, palette.text, palette.panel)
    }

    /// Sends typed bytes: the view follows the prompt again, as a key
    /// press does in a terminal.
    fn input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        {
            let mut term = self.session.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.session.write(bytes);
        cx.notify();
    }

    fn has_selection(&self) -> bool {
        self.session.term.lock().selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self.session.term.lock().selection_to_string();
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.session.term.lock().scroll_display(Scroll::Bottom);
            self.session.paste(&text);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let mut term = self.session.term.lock();
        let top = GridPoint::new(term.grid().topmost_line(), Column(0));
        let bottom = GridPoint::new(term.grid().bottommost_line(), term.grid().last_column());
        let mut selection = Selection::new(SelectionType::Simple, top, CellSide::Left);
        selection.update(bottom, CellSide::Right);
        term.selection = Some(selection);
        drop(term);
        cx.notify();
    }

    /// Clear Buffer: the scrollback and the screen; the shell redraws its
    /// prompt (Ctrl+L).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        {
            let mut term = self.session.term.lock();
            term.grid_mut().clear_history();
            term.selection = None;
        }
        self.session.write(vec![0x0c]);
        cx.notify();
    }

    /// A key press: copy / paste / new tab, keys the shell takes, or false
    /// to let the IDE (or text input) have it.
    fn on_keystroke(&mut self, keystroke: &gpui_kit::Keystroke, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        let mac = cfg!(target_os = "macos");
        // The IDE keeps: Alt+F12 (hide), Shift+Esc, tool window Alt+digits,
        // Find Action, Hide All, Switcher.
        let ide_key = (m.alt && !m.control && key == "f12")
            || (m.shift && !m.control && !m.alt && key == "escape")
            || (m.alt && !m.control && !m.shift && key.len() == 1 && key.as_bytes()[0].is_ascii_digit())
            || (m.control && m.shift && (key == "a" || key == "f12"))
            || (m.control && key == "tab");
        if ide_key {
            return false;
        }
        let copy = if mac { m.platform && key == "c" } else { (m.control && m.shift && key == "c") || (m.control && key == "insert") };
        if copy || (!mac && m.control && !m.shift && !m.alt && key == "c" && self.has_selection()) {
            self.copy(cx);
            self.session.term.lock().selection = None;
            cx.notify();
            return true;
        }
        let paste = if mac { m.platform && key == "v" } else { (m.control && !m.alt && key == "v") || (m.shift && key == "insert") };
        if paste {
            self.paste(cx);
            return true;
        }
        let new_tab = if mac { m.platform && key == "t" } else { m.control && m.shift && key == "t" };
        if new_tab {
            cx.emit(TerminalViewEvent::NewTab);
            return true;
        }
        if m.platform || m.function {
            return false;
        }
        // AltGr (Ctrl+Alt on Windows) types its character.
        if cfg!(windows) && m.control && m.alt && keystroke.key_char.as_deref().is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control)) {
            return false;
        }
        let mode = *self.session.term.lock().mode();
        match terminal::key_bytes(key, m.control, m.alt, m.shift, keystroke.key_char.as_deref(), mode) {
            Some(bytes) => {
                self.marked = None;
                self.input(bytes, cx);
                true
            }
            None => false,
        }
    }

    /// The cell under a window position, and which half of it.
    fn grid_point(&self, position: gpui_kit::Point<Pixels>) -> (GridPoint, CellSide) {
        let (w, h) = self.cell.get();
        let origin = self.origin.get();
        let x = f32::from(position.x - origin.x).max(0.);
        let y = f32::from(position.y - origin.y).max(0.);
        let term = self.session.term.lock();
        let col = ((x / w) as usize).min(term.columns().saturating_sub(1));
        let row = ((y / h) as usize).min(term.screen_lines().saturating_sub(1));
        let side = if x - col as f32 * w < w / 2. { CellSide::Left } else { CellSide::Right };
        let offset = term.grid().display_offset();
        (alacritty_terminal::term::viewport_to_point(offset, alacritty_terminal::index::Point::new(row, Column(col))), side)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (at, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            n if n >= 3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let mut term = self.session.term.lock();
        if event.modifiers.shift && let Some(selection) = term.selection.as_mut() {
            selection.update(at, side);
        } else {
            term.selection = Some(Selection::new(kind, at, side));
        }
        drop(term);
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (at, side) = self.grid_point(event.position);
        if let Some(selection) = self.session.term.lock().selection.as_mut() {
            selection.update(at, side);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<Self>) {
        self.selecting = false;
        let mut term = self.session.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
        drop(term);
        cx.notify();
    }

    fn on_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let (_, h) = self.cell.get();
        self.wheel += f32::from(event.delta.pixel_delta(px(h)).y) / h;
        let lines = self.wheel.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.wheel -= lines as f32;
        let mode = *self.session.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs (less, vim) scroll by arrow keys.
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.session.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            self.session.term.lock().scroll_display(Scroll::Delta(lines));
        }
        cx.notify();
    }

    /// What the grid shows now, resolved to colors.
    fn snapshot(&self, colors: &TermColors) -> Screen {
        let term = self.session.term.lock();
        let content = term.renderable_content();
        let (columns, lines) = (term.columns(), term.screen_lines());
        let offset = content.display_offset;
        let mut rows: Vec<Vec<Option<Glyph>>> = vec![vec![None; columns]; lines];
        let overrides = content.colors;
        for indexed in content.display_iter {
            let Some(view) = alacritty_terminal::term::point_to_viewport(offset, indexed.point) else { continue };
            let (row, col) = (view.line, view.column.0);
            if row >= lines || col >= columns {
                continue;
            }
            let cell = indexed.cell;
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let mut fg = colors.resolve(cell.fg, overrides);
            let mut bg = colors.resolve(cell.bg, overrides);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.intersects(Flags::DIM) {
                fg = fg.opacity(0.66);
            }
            let selected = content.selection.is_some_and(|s| s.contains(indexed.point));
            rows[row][col] = Some(Glyph {
                c: if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c },
                fg,
                bg: (bg != colors.background).then_some(bg),
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strike: cell.flags.contains(Flags::STRIKEOUT),
                wide: cell.flags.contains(Flags::WIDE_CHAR),
                selected,
            });
        }
        let cursor = content.mode.contains(TermMode::SHOW_CURSOR).then_some(content.cursor).and_then(|c| {
            let view = alacritty_terminal::term::point_to_viewport(offset, c.point)?;
            (view.line < lines && c.shape != CursorShape::Hidden).then_some((view.line, view.column.0, c.shape))
        });
        Screen { rows, cursor }
    }
}

#[derive(Clone)]
struct Glyph {
    c: char,
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    wide: bool,
    selected: bool,
}

struct Screen {
    rows: Vec<Vec<Option<Glyph>>>,
    cursor: Option<(usize, usize, CursorShape)>,
}

/// IntelliJ's console colors (ANSI 0–15), the theme's text and background.
struct TermColors {
    ansi: [Hsla; 16],
    foreground: Hsla,
    background: Hsla,
}

fn hex(rgb: u32) -> Hsla {
    gpui_kit::rgb(rgb).into()
}

impl TermColors {
    fn new(dark: bool, foreground: Hsla, background: Hsla) -> Self {
        let ansi: [u32; 16] = if dark {
            // Darcula / Dark console colors.
            [0x000000, 0xF0524F, 0x5C962C, 0xA68A0D, 0x3993D4, 0xA771BF, 0x00A3A3, 0x808080, 0x595959, 0xFF4050, 0x4FC414, 0xE5BF00, 0x1FB0FF, 0xED7EED, 0x00E5E5, 0xFFFFFF]
        } else {
            // IntelliJ Light console colors.
            [0x000000, 0xC7222D, 0x067D17, 0x9E880D, 0x0033B3, 0x871094, 0x00627A, 0x8C8C8C, 0x595959, 0xE55765, 0x369650, 0xC29E4A, 0x3574F0, 0xA571E6, 0x24A394, 0xFFFFFF]
        };
        Self { ansi: ansi.map(hex), foreground, background }
    }

    /// xterm's 256 colors: 16 ANSI, a 6×6×6 cube, 24 grays.
    fn resolve_index(&self, index: usize) -> Hsla {
        match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let i = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { 55 + v as u32 * 40 };
                hex(level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6))
            }
            232..=255 => {
                let v = 8 + (index as u32 - 232) * 10;
                hex(v << 16 | v << 8 | v)
            }
            256 => self.foreground,
            257 => self.background,
            _ => self.foreground,
        }
    }

    fn resolve(&self, color: Color, overrides: &alacritty_terminal::term::color::Colors) -> Hsla {
        let rgb = |c: Rgb| hex((c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32);
        match color {
            Color::Spec(c) => rgb(c),
            Color::Indexed(i) => overrides[i as usize].map(rgb).unwrap_or_else(|| self.resolve_index(i as usize)),
            Color::Named(name) => {
                if let Some(c) = overrides[name] {
                    return rgb(c);
                }
                match name {
                    NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => self.foreground,
                    NamedColor::DimForeground => self.foreground.opacity(0.66),
                    NamedColor::Background => self.background,
                    n if (n as usize) < 16 => self.ansi[n as usize],
                    // Dim black … dim white.
                    n => self.ansi[(n as usize).saturating_sub(NamedColor::DimBlack as usize).min(7)].opacity(0.66),
                }
            }
        }
    }
}

impl gpui_kit::EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, range: std::ops::Range<usize>, adjusted: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let marked = self.marked.as_ref()?;
        let utf16: Vec<u16> = marked.encode_utf16().collect();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        *adjusted = Some(range.clone());
        Some(String::from_utf16_lossy(&utf16[range]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked.as_ref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.input(text.as_bytes().to_vec(), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // The IME window opens at the cursor.
        let (w, h) = self.cell.get();
        let origin = self.origin.get();
        let term = self.session.term.lock();
        let cursor = term.grid().cursor.point;
        let row = (cursor.line.0 + term.grid().display_offset() as i32).max(0) as f32;
        Some(Bounds::new(point(origin.x + px(cursor.column.0 as f32 * w), origin.y + px(row * h)), size(px(w), px(h))))
    }

    fn character_index_for_point(&mut self, _: gpui_kit::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let colors = self.colors(cx);
        let screen = self.snapshot(&colors);
        let focused = self.focus.is_focused(window);
        let font_size = px(Settings::get(cx).editor_font_size());
        let family = cx.theme_mono_family();
        let font = gpui_kit::font(family);
        let font_id = window.text_system().resolve_font(&font);
        let cell_w = window.text_system().advance(font_id, font_size, 'm').map(|s| f32::from(s.width)).unwrap_or(8.);
        let cell_h = (f32::from(font_size) * 1.3).round();
        self.cell.set((cell_w, cell_h));
        let marked = self.marked.clone();
        let entity = cx.entity();
        let (origin, focus) = (self.origin.clone(), self.focus.clone());
        let selection = palette.selection;
        let background = colors.background;
        let cursor_color = colors.foreground;
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .bg(background)
            .cursor(gpui_kit::CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.on_mouse_down(e, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.on_mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, cx| this.on_mouse_up(e, cx)))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.on_wheel(e, cx)))
            .child(
                canvas(
                    {
                        let entity = entity.clone();
                        move |bounds, _, cx| {
                            // The grid follows the panel's size.
                            let columns = ((f32::from(bounds.size.width) - 2. * PAD_X) / cell_w).floor().max(2.) as usize;
                            let lines = ((f32::from(bounds.size.height) - 2. * PAD_Y) / cell_h).floor().max(1.) as usize;
                            let size = GridSize { columns, lines };
                            if entity.read(cx).session.size() != size {
                                entity.update(cx, |this, cx| {
                                    this.session.resize(size);
                                    cx.notify();
                                });
                            }
                        }
                    },
                    move |bounds, _, window, cx| {
                        let top_left = point(bounds.origin.x + px(PAD_X), bounds.origin.y + px(PAD_Y));
                        origin.set(top_left);
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        paint_screen(&screen, top_left, (cell_w, cell_h), &font, font_size, background, selection, window, cx);
                        if let Some((row, col, shape)) = screen.cursor {
                            let at = point(top_left.x + px(col as f32 * cell_w), top_left.y + px(row as f32 * cell_h));
                            let glyph = screen.rows.get(row).and_then(|r| r.get(col)).cloned().flatten();
                            let width = px(cell_w * if glyph.as_ref().is_some_and(|g| g.wide) { 2. } else { 1. });
                            let cell = Bounds::new(at, size(width, px(cell_h)));
                            if let Some(text) = marked.as_ref() {
                                // The IME's composition, underlined at the cursor.
                                let run = TextRun {
                                    len: text.len(),
                                    font: font.clone(),
                                    color: cursor_color,
                                    background_color: Some(background),
                                    underline: Some(UnderlineStyle { thickness: px(1.), color: Some(cursor_color), wavy: false }),
                                    strikethrough: None,
                                };
                                let shaped = window.text_system().shape_line(text.clone().into(), font_size, &[run], None);
                                let _ = shaped.paint(at, px(cell_h), TextAlign::Left, None, window, cx);
                            } else if !focused {
                                // Unfocused: a hollow block, as JediTerm draws it.
                                for edge in [
                                    Bounds::new(at, size(width, px(1.))),
                                    Bounds::new(point(at.x, at.y + px(cell_h - 1.)), size(width, px(1.))),
                                    Bounds::new(at, size(px(1.), px(cell_h))),
                                    Bounds::new(point(at.x + width - px(1.), at.y), size(px(1.), px(cell_h))),
                                ] {
                                    window.paint_quad(fill(edge, cursor_color));
                                }
                            } else {
                                match shape {
                                    CursorShape::Beam => window.paint_quad(fill(Bounds::new(at, size(px(2.), px(cell_h))), cursor_color)),
                                    CursorShape::Underline => window.paint_quad(fill(Bounds::new(point(at.x, at.y + px(cell_h - 2.)), size(width, px(2.))), cursor_color)),
                                    _ => {
                                        window.paint_quad(fill(cell, cursor_color));
                                        if let Some(g) = glyph.filter(|g| g.c != ' ') {
                                            let run = TextRun { len: g.c.len_utf8(), font: styled_font(&font, &g), color: background, background_color: None, underline: None, strikethrough: None };
                                            let shaped = window.text_system().shape_line(g.c.to_string().into(), font_size, &[run], None);
                                            let _ = shaped.paint(at, px(cell_h), TextAlign::Left, None, window, cx);
                                        }
                                    }
                                }
                            }
                        }
                    },
                )
                .size_full(),
            )
    }
}

fn styled_font(font: &Font, glyph: &Glyph) -> Font {
    let mut font = font.clone();
    if glyph.bold {
        font.weight = FontWeight::BOLD;
    }
    if glyph.italic {
        font.style = FontStyle::Italic;
    }
    font
}

/// Paints the grid: backgrounds and selection, then text in runs of one
/// style, each glyph forced onto its cell (wide ones over two cells).
#[allow(clippy::too_many_arguments)]
fn paint_screen(screen: &Screen, origin: gpui_kit::Point<Pixels>, (w, h): (f32, f32), font: &Font, font_size: Pixels, background: Hsla, selection: Hsla, window: &mut Window, cx: &mut App) {
    for (row, cells) in screen.rows.iter().enumerate() {
        let y = origin.y + px(row as f32 * h);
        let mut col = 0;
        while col < cells.len() {
            let fill_of = |g: &Option<Glyph>| g.as_ref().and_then(|g| if g.selected { Some(selection) } else { g.bg });
            let color = fill_of(&cells[col]);
            let start = col;
            while col < cells.len() && fill_of(&cells[col]) == color {
                col += if cells[col].as_ref().is_some_and(|g| g.wide) { 2 } else { 1 };
            }
            if let Some(color) = color {
                let end = col.min(cells.len());
                window.paint_quad(fill(Bounds::new(point(origin.x + px(start as f32 * w), y), size(px((end - start) as f32 * w), px(h))), color));
            }
        }
        // Text: runs of narrow glyphs, wide glyphs one by one.
        let mut col = 0;
        while col < cells.len() {
            let Some(glyph) = cells[col].as_ref() else {
                col += 1;
                continue;
            };
            let x = origin.x + px(col as f32 * w);
            if glyph.wide {
                let run = text_run(glyph, glyph.c.len_utf8(), font);
                let shaped = window.text_system().shape_line(glyph.c.to_string().into(), font_size, &[run], None);
                let _ = shaped.paint(point(x, y), px(h), TextAlign::Left, None, window, cx);
                col += 2;
                continue;
            }
            let mut text = String::new();
            let mut runs: Vec<TextRun> = Vec::new();
            while let Some(g) = cells.get(col).and_then(|c| c.as_ref()).filter(|g| !g.wide) {
                text.push(g.c);
                match runs.last_mut() {
                    Some(last) if last.color == g.fg && last.font == styled_font(font, g) && last.underline.is_some() == g.underline && last.strikethrough.is_some() == g.strike => {
                        last.len += g.c.len_utf8();
                    }
                    _ => runs.push(text_run(g, g.c.len_utf8(), font)),
                }
                col += 1;
            }
            if text.trim().is_empty() && runs.iter().all(|r| r.underline.is_none() && r.strikethrough.is_none()) {
                continue;
            }
            let shaped = window.text_system().shape_line(text.into(), font_size, &runs, Some(px(w)));
            let _ = shaped.paint(point(x, y), px(h), TextAlign::Left, None, window, cx);
        }
    }
    let _ = background;
}

fn text_run(glyph: &Glyph, len: usize, font: &Font) -> TextRun {
    TextRun {
        len,
        font: styled_font(font, glyph),
        color: glyph.fg,
        background_color: None,
        underline: glyph.underline.then_some(UnderlineStyle { thickness: px(1.), color: Some(glyph.fg), wavy: false }),
        strikethrough: glyph.strike.then_some(StrikethroughStyle { thickness: px(1.), color: Some(glyph.fg) }),
    }
}

trait MonoFamily {
    fn theme_mono_family(&self) -> SharedString;
}

impl<V> MonoFamily for Context<'_, V> {
    fn theme_mono_family(&self) -> SharedString {
        use gpui_kit::component::ActiveTheme as _;
        self.theme().mono_font_family.clone()
    }
}

pub enum TerminalPanelEvent {
    Hide,
}

struct TerminalTab {
    title: SharedString,
    view: Entity<TerminalView>,
    _subscription: Subscription,
}

/// The Terminal tool window: its tabs and their header.
pub struct TerminalPanel {
    tabs: Vec<TerminalTab>,
    active: usize,
    /// The project's directory, where new sessions start.
    cwd: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    error: Option<String>,
    /// A tab closed: the one shown next takes the focus.
    refocus: bool,
}

impl EventEmitter<TerminalPanelEvent> for TerminalPanel {}

impl TerminalPanel {
    pub fn new(cwd: Rc<dyn Fn(&App) -> Option<PathBuf>>) -> Self {
        Self { tabs: Vec::new(), active: 0, cwd, error: None, refocus: false }
    }

    /// The active session's focus, starting one if there is none.
    pub fn focus_handle(&mut self, cx: &mut Context<Self>) -> Option<FocusHandle> {
        if self.tabs.is_empty() && self.error.is_none() {
            self.new_session(terminal::default_shell(), cx);
        }
        self.tabs.get(self.active).map(|t| t.view.read(cx).focus.clone())
    }

    /// A new tab running `shell` in the project's directory: "Local",
    /// "Local (2)", … as Android Studio names them.
    pub fn new_session(&mut self, shell: ShellChoice, cx: &mut Context<Self>) {
        let cwd = (self.cwd)(cx);
        let (session, events) = match Session::spawn(&shell, cwd.as_deref(), GridSize { columns: 80, lines: 24 }) {
            Ok(spawned) => spawned,
            Err(e) => {
                self.error = Some(format!("{}: {e}", shell.program));
                cx.notify();
                return;
            }
        };
        self.error = None;
        let view = cx.new(|cx| TerminalView::new(session, events, cx));
        let subscription = cx.subscribe(&view, |this, view, event: &TerminalViewEvent, cx| match event {
            TerminalViewEvent::Exited => {
                if let Some(ix) = this.tabs.iter().position(|t| t.view == view) {
                    this.close(ix, cx);
                }
            }
            TerminalViewEvent::NewTab => this.new_session(terminal::default_shell(), cx),
        });
        let taken = |title: &str| self.tabs.iter().any(|t| t.title.as_ref() == title);
        let mut title = shell.name.clone();
        let mut n = 2;
        while taken(&title) {
            title = format!("{} ({n})", shell.name);
            n += 1;
        }
        self.tabs.push(TerminalTab { title: title.into(), view, _subscription: subscription });
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    fn close(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        if self.tabs.is_empty() {
            cx.emit(TerminalPanelEvent::Hide);
        } else {
            self.refocus = true;
        }
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut tabs = h_flex().gap_1().min_w_0().overflow_x_hidden();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            tabs = tabs.child(
                h_flex()
                    .id(("terminal-tab", ix))
                    .h(px(24.))
                    .pl_2()
                    .pr_0p5()
                    .gap_1()
                    .rounded(px(4.))
                    .border_1()
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |el| el.bg(palette.selection).border_color(palette.accent.opacity(0.5)))
                    .when(!active, |el| el.border_color(gpui_kit::transparent_black()).hover(|el| el.bg(palette.hover)))
                    .child(div().whitespace_nowrap().child(tab.title.clone()))
                    .child(
                        Button::new(("terminal-tab-close", ix)).ghost().xsmall().icon(Icon::new(icons::CLOSE_SMALL)).on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close(ix, cx)
                        })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = ix;
                        if let Some(tab) = this.tabs.get(ix) {
                            window.focus(&tab.view.read(cx).focus.clone(), cx);
                        }
                        cx.notify();
                    })),
            );
        }
        let entity = cx.entity();
        let shells_entity = entity.clone();
        h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_1()
            .flex_shrink_0()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).mr_2().child("Terminal"))
            .child(tabs)
            .child(tool_button("terminal-new", icons::ADD, "New Tab (Ctrl+Shift+T)").on_click(cx.listener(|this, _, window, cx| {
                this.new_session(terminal::default_shell(), cx);
                this.focus_active(window, cx);
            })))
            .child(tool_button("terminal-shells", icons::CHEVRON_DOWN, "New Session").dropdown_menu(move |mut menu, _, _| {
                for shell in terminal::available_shells() {
                    let entity = shells_entity.clone();
                    menu = menu.item(PopupMenuItem::new(shell.name.clone()).on_click(move |_, window, cx| {
                        let shell = shell.clone();
                        entity.update(cx, |this, cx| {
                            this.new_session(shell, cx);
                            this.focus_active(window, cx);
                        })
                    }));
                }
                menu
            }))
            .child(div().flex_1())
            .child(tool_button("terminal-options", icons::MORE_VERTICAL, "Options").dropdown_menu(move |menu, _, _| {
                let (e1, e2, e3) = (entity.clone(), entity.clone(), entity.clone());
                menu.item(PopupMenuItem::new("New Tab").on_click(move |_, window, cx| {
                    e1.update(cx, |this, cx| {
                        this.new_session(terminal::default_shell(), cx);
                        this.focus_active(window, cx);
                    })
                }))
                .item(PopupMenuItem::new("Close Tab").on_click(move |_, _, cx| e2.update(cx, |this, cx| this.close(this.active, cx))))
                .separator()
                .item(PopupMenuItem::new("Clear Buffer").on_click(move |_, _, cx| {
                    e3.update(cx, |this, cx| {
                        if let Some(tab) = this.tabs.get(this.active) {
                            tab.view.update(cx, |view, cx| view.clear(cx));
                        }
                    })
                }))
            }))
            .child(tool_button("terminal-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(TerminalPanelEvent::Hide))))
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.view.read(cx).focus.clone(), cx);
        }
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if self.tabs.is_empty() && self.error.is_none() {
            self.new_session(terminal::default_shell(), cx);
        }
        if std::mem::take(&mut self.refocus) {
            self.focus_active(window, cx);
        }
        let body = match self.tabs.get(self.active) {
            Some(tab) => {
                let view = tab.view.clone();
                div()
                    .id("terminal-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(view.clone())
                    .context_menu(move |menu, _, _| {
                        let (copy, paste, all, clear) = (view.clone(), view.clone(), view.clone(), view.clone());
                        menu.item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| copy.update(cx, |v, cx| v.copy(cx))))
                            .item(PopupMenuItem::new("Paste").on_click(move |_, _, cx| paste.update(cx, |v, cx| v.paste(cx))))
                            .item(PopupMenuItem::new("Select All").on_click(move |_, _, cx| all.update(cx, |v, cx| v.select_all(cx))))
                            .separator()
                            .item(PopupMenuItem::new("Clear Buffer").on_click(move |_, _, cx| clear.update(cx, |v, cx| v.clear(cx))))
                    })
                    .into_any_element()
            }
            None => div()
                .flex_1()
                .p_2()
                .text_sm()
                .text_color(palette.status_conflict)
                .child(format!("Cannot open a terminal: {}", self.error.clone().unwrap_or_default()))
                .into_any_element(),
        };
        v_flex().size_full().bg(palette.panel).child(self.render_header(cx)).child(body)
    }
}

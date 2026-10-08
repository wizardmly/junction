//! Typo highlighting for a plain-text editor (the commit message), as
//! IntelliJ's spelling inspection does: misspelled words get a wavy
//! underline, and Alt+Enter on one (or a right-click) lists the fixes:
//! "Change to …" suggestions and Save to Dictionary.

use std::ops::Range;

use gpui_kit::component::{input::{InputEvent, TextareaState}, v_flex};
use gpui_kit::{
    Bounds, Context, DispatchPhase, Entity, FocusHandle, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, PathBuilder, Pixels, Point, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder as _, px,
};

use crate::theme::ActivePalette as _;

gpui_kit::actions!(spell_overlay, [ShowSpellingFixes, CloseSpellingFixes, SelectPrevFix, SelectNextFix, ApplyFix]);

/// Key context to put on the element wrapping the checked editor.
pub const EDITOR_CONTEXT: &str = "SpellChecked";
const POPUP_CONTEXT: &str = "SpellingFixes";

pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("alt-enter", ShowSpellingFixes, Some(EDITOR_CONTEXT)),
        gpui_kit::KeyBinding::new("escape", CloseSpellingFixes, Some(POPUP_CONTEXT)),
        gpui_kit::KeyBinding::new("up", SelectPrevFix, Some(POPUP_CONTEXT)),
        gpui_kit::KeyBinding::new("down", SelectNextFix, Some(POPUP_CONTEXT)),
        gpui_kit::KeyBinding::new("enter", ApplyFix, Some(POPUP_CONTEXT)),
    ]);
}

struct Fixes {
    range: Range<usize>,
    word: String,
    suggestions: Vec<String>,
    /// Row under the keyboard; the last row is Save to Dictionary.
    selected: usize,
    at: Point<Pixels>,
}

pub struct SpellOverlay {
    input: Entity<TextareaState>,
    typos: Vec<Range<usize>>,
    fixes: Option<Fixes>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl SpellOverlay {
    pub fn new(input: Entity<TextareaState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.fixes = None;
                this.recheck(cx);
            }
        });
        let mut this = Self { input, typos: Vec::new(), fixes: None, focus: cx.focus_handle(), _subscription: subscription };
        this.recheck(cx);
        this
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        let state = self.input.read(cx);
        let text = state.value();
        let cursor = state.cursor();
        // The word being typed is left alone until the cursor moves on.
        self.typos = crate::spell::misspelled(&text).into_iter().filter(|r| r.end != cursor || cursor < text.len()).collect();
        cx.notify();
    }

    /// Alt+Enter: the fixes for the typo at the cursor.
    pub fn show_at_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.input.read(cx).cursor();
        let text = self.input.read(cx).value();
        let typos = crate::spell::misspelled(&text);
        if let Some(range) = typos.into_iter().find(|r| r.start <= cursor && cursor <= r.end) {
            self.open(range, window, cx);
        }
    }

    fn open(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.input.read(cx);
        let Some(bounds) = state.range_to_bounds(&range) else { return };
        let word = state.value()[range.clone()].to_owned();
        let suggestions = crate::spell::suggestions(&word, 5);
        self.fixes = Some(Fixes { range, word, suggestions, selected: 0, at: point(bounds.left(), bounds.bottom() + px(2.)) });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fixes = None;
        self.input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn apply(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fixes) = self.fixes.take() else { return };
        if let Some(text) = fixes.suggestions.get(row) {
            let (range, text) = (fixes.range.clone(), text.clone());
            self.input.update(cx, |state, cx| {
                // Only replace when the word is still there.
                if state.value().get(range.clone()) == Some(fixes.word.as_str()) {
                    state.set_selected_range(range, cx);
                    state.replace(text, window, cx);
                }
            });
        } else {
            crate::spell::add_word(&fixes.word);
            self.recheck(cx);
        }
        self.close(window, cx);
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some(fixes) = &mut self.fixes {
            let rows = fixes.suggestions.len() + 1;
            fixes.selected = if down { (fixes.selected + 1) % rows } else { (fixes.selected + rows - 1) % rows };
            cx.notify();
        }
    }

    fn underlines(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let input = self.input.clone();
        let typos = self.typos.clone();
        let this = cx.entity().downgrade();
        let color = cx.palette().status_added;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = input.read(cx);
                let mut rects: Vec<(Bounds<Pixels>, Range<usize>)> = Vec::new();
                for range in &typos {
                    let Some(word) = state.range_to_bounds(range) else { continue };
                    // A word the soft wrap split is not underlined.
                    if word.size.width <= px(0.) || word.size.height > px(40.) {
                        continue;
                    }
                    let y = word.bottom() - px(2.);
                    if y < bounds.top() || y > bounds.bottom() {
                        continue;
                    }
                    paint_wave(window, word.left(), word.right(), y, color);
                    rects.push((word, range.clone()));
                }
                let this = this.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    // Captured, so a typo's fixes replace the editor's Cut/Copy/Paste menu.
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Right {
                        return;
                    }
                    if let Some((_, range)) = rects.iter().find(|(r, _)| r.contains(&e.position)) {
                        let range = range.clone();
                        cx.stop_propagation();
                        this.update(cx, |this, cx| this.open(range, window, cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }

    fn popup(&self, fixes: &Fixes, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let row = |ix: usize, label: String, cx: &mut Context<Self>| {
            div()
                .id(("spelling-fix", ix))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .when(ix == fixes.selected, |el| el.bg(palette.selection))
                .hover(|s| s.bg(palette.hover))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.apply(ix, window, cx)))
        };
        let mut list = v_flex().p_1().text_sm();
        if fixes.suggestions.is_empty() {
            list = list.child(div().px_2().py_0p5().text_color(palette.text_secondary).child("No suggestions"));
        }
        for (ix, word) in fixes.suggestions.iter().enumerate() {
            list = list.child(row(ix, format!("Change to '{word}'"), cx));
        }
        let save = fixes.suggestions.len();
        list = list
            .child(div().my_0p5().h(px(1.)).bg(palette.border))
            .child(row(save, format!("Save '{}' to dictionary", fixes.word.to_lowercase()), cx));
        deferred(
            anchored().position(fixes.at).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("spelling-fixes")
                    .key_context(POPUP_CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &CloseSpellingFixes, window, cx| this.close(window, cx)))
                    .on_action(cx.listener(|this, _: &SelectPrevFix, _, cx| this.step(false, cx)))
                    .on_action(cx.listener(|this, _: &SelectNextFix, _, cx| this.step(true, cx)))
                    .on_action(cx.listener(|this, _: &ApplyFix, window, cx| {
                        let row = this.fixes.as_ref().map_or(0, |f| f.selected);
                        this.apply(row, window, cx)
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.fixes = None;
                        cx.notify();
                    }))
                    .min_w(px(200.))
                    .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                    .text_color(palette.text)
                    .border_1()
                    .border_color(palette.border)
                    .rounded_md()
                    .shadow_lg()
                    .child(list),
            ),
        )
        .with_priority(1)
    }
}

/// IntelliJ's typo underline: a thin zigzag under the word.
fn paint_wave(window: &mut Window, left: Pixels, right: Pixels, y: Pixels, color: gpui_kit::Hsla) {
    let (step, height) = (px(2.), px(1.5));
    let mut path = PathBuilder::stroke(px(1.));
    path.move_to(point(left, y));
    let mut x = left;
    let mut up = true;
    while x < right {
        x = (x + step).min(right);
        path.line_to(point(x, if up { y - height } else { y }));
        up = !up;
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

impl Render for SpellOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let underlines = self.underlines(cx);
        let popup = self.fixes.as_ref().map(|fixes| self.popup(fixes, cx));
        div().absolute().inset_0().child(underlines).children(popup)
    }
}

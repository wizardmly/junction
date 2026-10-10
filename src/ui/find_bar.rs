//! IntelliJ's editor Find / Replace bar (Ctrl+F / Ctrl+R): Match Case,
//! Words and Regex toggles, "3/12" results, ↑ ↓ (also Enter / Shift+Enter,
//! F3 / Shift+F3), and in replace mode Replace, Replace All and Exclude.
//! Matches are highlighted in the editor, the current one selected.

use std::ops::Range;
use crate::ui::as_icons as icons;

use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{EditorState, Input, InputEvent, InputState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    Styled as _, Subscription, Window, actions, div, prelude::FluentBuilder as _, px,
};

use crate::index::text_search::{SearchContext, TextQuery, compile};
use crate::theme::ActivePalette as _;
use crate::ui::common::tool_button;

actions!(find_bar, [CloseFind, NextOccurrence, PreviousOccurrence, ReplaceOne, ToggleMatchCase, ToggleWords, ToggleRegex]);

const CONTEXT: &str = "FindBar";

pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseFind, Some(CONTEXT)),
        KeyBinding::new("enter", NextOccurrence, Some("FindBar > Input")),
        KeyBinding::new("shift-enter", PreviousOccurrence, Some("FindBar > Input")),
        KeyBinding::new("f3", NextOccurrence, Some(CONTEXT)),
        KeyBinding::new("shift-f3", PreviousOccurrence, Some(CONTEXT)),
        // IntelliJ's mnemonics: Alt+C Match Case, Alt+W Words, Alt+X Regex.
        KeyBinding::new("alt-c", ToggleMatchCase, Some(CONTEXT)),
        KeyBinding::new("alt-w", ToggleWords, Some(CONTEXT)),
        KeyBinding::new("alt-x", ToggleRegex, Some(CONTEXT)),
    ]);
}

pub enum FindBarEvent {
    Closed,
}

impl EventEmitter<FindBarEvent> for FindBar {}

pub struct FindBar {
    editor: Entity<EditorState>,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    pub open: bool,
    read_only: bool,
    replace_mode: bool,
    case_sensitive: bool,
    whole_words: bool,
    regex: bool,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    error: Option<String>,
    highlights: Option<RangeDecorationCollection>,
    _subscriptions: Vec<Subscription>,
}

impl FindBar {
    pub fn new(editor: Entity<EditorState>, read_only: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search(true, cx);
                }
            }),
            // Typing in the editor moves the matches with it.
            cx.subscribe(&editor, |this, _, event: &InputEvent, cx| {
                if this.open && matches!(event, InputEvent::Change) {
                    this.search(false, cx);
                }
            }),
        ];
        Self {
            editor,
            query,
            replacement,
            open: false,
            read_only,
            replace_mode: false,
            case_sensitive: false,
            whole_words: false,
            regex: false,
            matches: Vec::new(),
            current: None,
            error: None,
            highlights: None,
            _subscriptions: subscriptions,
        }
    }

    /// Ctrl+F / Ctrl+R: opens with the selection (or keeps the last query)
    /// and focuses the search field.
    pub fn show(&mut self, replace: bool, initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.replace_mode = replace && !self.read_only;
        if let Some(text) = initial.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.query.update(cx, |q, cx| q.set_value(text, window, cx));
        }
        self.query.update(cx, |q, cx| {
            q.focus(window, cx);
            q.select_all(window, cx);
        });
        self.search(true, cx);
    }

    pub fn query_text(&self, cx: &gpui_kit::App) -> String {
        self.query.read(cx).value().to_string()
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        if let Some(highlights) = &self.highlights {
            highlights.clear(cx);
        }
        self.editor.update(cx, |e, cx| e.focus(window, cx));
        cx.emit(FindBarEvent::Closed);
        cx.notify();
    }

    /// Recomputes the matches; `jump` selects the first one at or after the caret.
    fn search(&mut self, jump: bool, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        let query = TextQuery {
            text: self.query.read(cx).value().to_string(),
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            regex: self.regex,
            mask: None,
            context: SearchContext::Anywhere,
        };
        self.matches.clear();
        self.error = None;
        match compile(&query) {
            Ok(re) => self.matches = re.find_iter(&text).filter(|m| !m.is_empty()).map(|m| m.range()).collect(),
            Err(error) if !error.is_empty() => self.error = Some(error),
            Err(_) => {}
        }
        if jump {
            let caret = self.editor.read(cx).selected_range().start;
            self.current = self.matches.iter().position(|m| m.start >= caret).or((!self.matches.is_empty()).then_some(0));
        } else {
            self.current = self.current.filter(|&ix| ix < self.matches.len()).or((!self.matches.is_empty()).then_some(0));
        }
        self.paint(cx);
        if jump {
            self.select_current(cx);
        }
        cx.notify();
    }

    fn paint(&mut self, cx: &mut Context<Self>) {
        let palette = cx.palette().clone();
        let decorations: Vec<RangeDecoration> = self
            .matches
            .iter()
            .enumerate()
            .map(|(ix, range)| {
                let style = if Some(ix) == self.current { RangeDecorationStyle::Frame } else { RangeDecorationStyle::Fill };
                RangeDecoration::new(range.clone()).with_style(style).with_color(palette.search_match)
            })
            .collect();
        match &self.highlights {
            Some(highlights) => highlights.set(decorations, cx),
            None => {
                let collection = self.editor.update(cx, |e, cx| e.create_range_decorations_collection(decorations, cx));
                self.highlights = Some(collection);
            }
        }
    }

    fn select_current(&mut self, cx: &mut Context<Self>) {
        if let Some(range) = self.current.and_then(|ix| self.matches.get(ix).cloned()) {
            self.editor.update(cx, |e, cx| e.set_selected_range(range, cx));
        }
    }

    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.current = Some(match self.current {
            Some(ix) if forward => (ix + 1) % n,
            Some(ix) => (ix + n - 1) % n,
            None => 0,
        });
        self.paint(cx);
        self.select_current(cx);
        cx.notify();
    }

    /// The replacement for one match: `$1` groups expand in regex mode.
    fn replacement_for(&self, text: &str, range: &Range<usize>, cx: &gpui_kit::App) -> String {
        let replacement = self.replacement.read(cx).value().to_string();
        if !self.regex {
            return replacement;
        }
        let query = TextQuery {
            text: self.query.read(cx).value().to_string(),
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            regex: true,
            mask: None,
            context: SearchContext::Anywhere,
        };
        let Ok(re) = compile(&query) else { return replacement };
        match re.captures_at(text, range.start) {
            Some(caps) if caps.get(0).is_some_and(|m| m.start() == range.start) => {
                let mut out = String::new();
                caps.expand(&replacement, &mut out);
                out
            }
            _ => replacement,
        }
    }

    fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.current.and_then(|ix| self.matches.get(ix).cloned()) else { return };
        let text = self.editor.read(cx).value().to_string();
        let with = self.replacement_for(&text, &range, cx);
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(range.clone(), cx);
            e.replace(with.clone(), window, cx);
        });
        // The edit re-ran the search; carry on from the replaced spot.
        let next = self.matches.iter().position(|m| m.start >= range.start + with.len());
        self.current = next.or((!self.matches.is_empty()).then_some(0));
        self.paint(cx);
        self.select_current(cx);
        cx.notify();
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for range in &self.matches {
            out.push_str(&text[last..range.start]);
            out.push_str(&self.replacement_for(&text, range, cx));
            last = range.end;
        }
        out.push_str(&text[last..]);
        // One undoable edit over the whole text.
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(0..text.len(), cx);
            e.replace(out, window, cx);
        });
    }

    /// Exclude: skips the current match (it stays unreplaced) and moves on.
    fn exclude_current(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.current {
            self.matches.remove(ix);
            self.current = (!self.matches.is_empty()).then(|| ix % self.matches.len());
            self.paint(cx);
            self.select_current(cx);
            cx.notify();
        }
    }
}

impl Render for FindBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if !self.open {
            return v_flex();
        }
        let toggle = |id: &'static str, icon: Icon, tip: &'static str, on: bool| tool_button(id, icon, tip).when(on, |b| b.selected(true));
        let status = match (&self.error, self.current) {
            (Some(error), _) => error.clone(),
            (None, _) if self.query.read(cx).value().is_empty() => String::new(),
            (None, Some(ix)) => format!("{}/{}", ix + 1, self.matches.len()),
            (None, None) => "0 results".to_owned(),
        };
        let no_match = self.error.is_some() || (!self.query.read(cx).value().is_empty() && self.matches.is_empty());
        let search_row = h_flex()
            .gap_1()
            .child(
                div().w(px(300.)).child(
                    Input::new(&self.query)
                        .small()
                        .prefix(Icon::new(icons::SEARCH).xsmall().text_color(palette.text_secondary))
                        .suffix(
                            h_flex()
                                .gap_0p5()
                                .child(toggle("find-case", Icon::from(icons::MATCH_CASE), "Match Case  Alt+C", self.case_sensitive).on_click(cx.listener(|this, _, _, cx| {
                                    this.case_sensitive = !this.case_sensitive;
                                    this.search(false, cx);
                                })))
                                .child(toggle("find-words", Icon::from(icons::EXACT_WORDS), "Words  Alt+W", self.whole_words).on_click(cx.listener(|this, _, _, cx| {
                                    this.whole_words = !this.whole_words;
                                    this.search(false, cx);
                                })))
                                .child(toggle("find-regex", Icon::from(icons::REGEX), "Regex  Alt+X", self.regex).on_click(cx.listener(|this, _, _, cx| {
                                    this.regex = !this.regex;
                                    this.search(false, cx);
                                }))),
                        ),
                ),
            )
            .child(
                div()
                    .min_w(px(70.))
                    .text_xs()
                    .text_color(if no_match { palette.status_conflict } else { palette.text_secondary })
                    .child(status),
            )
            .child(tool_button("find-prev", icons::UP, "Previous Occurrence  Shift+Enter").on_click(cx.listener(|this, _, _, cx| this.step(false, cx))))
            .child(tool_button("find-next", icons::DOWN, "Next Occurrence  Enter").on_click(cx.listener(|this, _, _, cx| this.step(true, cx))))
            .child(div().flex_1())
            .child(tool_button("find-close", icons::CLOSE, "Close  Escape").on_click(cx.listener(|this, _, window, cx| this.close(window, cx))));
        let replace_row = self.replace_mode.then(|| {
            h_flex()
                .gap_1()
                .child(div().w(px(300.)).child(Input::new(&self.replacement).small()))
                .child(Button::new("find-replace").xsmall().outline().label("Replace").disabled(self.current.is_none()).on_click(
                    cx.listener(|this, _, window, cx| this.replace_current(window, cx)),
                ))
                .child(Button::new("find-replace-all").xsmall().outline().label("Replace All").disabled(self.matches.is_empty()).on_click(
                    cx.listener(|this, _, window, cx| this.replace_all(window, cx)),
                ))
                .child(Button::new("find-exclude").xsmall().ghost().label("Exclude").disabled(self.current.is_none()).on_click(
                    cx.listener(|this, _, _, cx| this.exclude_current(cx)),
                ))
        });
        v_flex()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &NextOccurrence, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousOccurrence, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &ReplaceOne, window, cx| this.replace_current(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleMatchCase, _, cx| {
                this.case_sensitive = !this.case_sensitive;
                this.search(false, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleWords, _, cx| {
                this.whole_words = !this.whole_words;
                this.search(false, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| {
                this.regex = !this.regex;
                this.search(false, cx);
            }))
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(search_row)
            .children(replace_row)
    }
}

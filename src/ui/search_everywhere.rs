//! Search Everywhere (Shift twice), as IntelliJ's: All, Classes, Files,
//! Symbols, Actions and Text tabs in one popup. Ctrl+N, Ctrl+Shift+N,
//! Ctrl+Alt+Shift+N and Ctrl+Shift+A open it on their tab.

use std::collections::HashSet;
use crate::ui::as_icons as icons;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::index::lang::Lang;
use crate::index::nav::{Target, fuzzy_score};
use crate::index::service::CodeIndex;
use crate::index::text_search::{self, FileScope, TextMatch, TextQuery};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, tool_button};
use crate::ui::find_popup::{FindRequest, highlighted, target_of};
use crate::ui::find_view::FoundItem;
use crate::ui::navigate::symbol_icon;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SeTab {
    All,
    Classes,
    Files,
    Symbols,
    Actions,
    Text,
}

impl SeTab {
    pub const ALL: [SeTab; 6] = [SeTab::All, SeTab::Classes, SeTab::Files, SeTab::Symbols, SeTab::Actions, SeTab::Text];

    fn label(self) -> &'static str {
        match self {
            SeTab::All => "All",
            SeTab::Classes => "Classes",
            SeTab::Files => "Files",
            SeTab::Symbols => "Symbols",
            SeTab::Actions => "Actions",
            SeTab::Text => "Text",
        }
    }

    fn command(self) -> &'static str {
        match self {
            SeTab::All => "/all",
            SeTab::Classes => "/classes",
            SeTab::Files => "/files",
            SeTab::Symbols => "/symbols",
            SeTab::Actions => "/actions",
            SeTab::Text => "/text",
        }
    }
}

/// A command for the Actions tab and Find Action.
#[derive(Clone)]
pub struct ActionEntry {
    pub name: String,
    pub shortcut: String,
    pub group: String,
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
}

pub enum SeEvent {
    Open(Target),
    Run(Rc<dyn Fn(&mut Window, &mut App)>),
    Close,
    FindWindowText(FindRequest),
    FindWindowItems(String, Vec<FoundItem>),
}

#[derive(Clone)]
enum Kind {
    Target { title: String, detail: String, right: String, icon: Icon, target: Target },
    Action(usize),
    Text(TextMatch),
    Command(SeTab),
    More(SeTab),
}

#[derive(Clone)]
struct Row {
    group: SeTab,
    kind: Kind,
}

/// Per group in the All tab.
const ALL_LIMIT: usize = 6;
const TAB_LIMIT: usize = 300;

pub struct SearchEverywhere {
    index: Entity<CodeIndex>,
    input: Entity<InputState>,
    tab: SeTab,
    include_non_project: bool,
    show_preview: bool,
    /// Contributors left out of the All tab, and languages filtered out.
    hidden_groups: HashSet<SeTab>,
    hidden_langs: HashSet<&'static str>,
    actions: Rc<Vec<ActionEntry>>,
    rows: Vec<Row>,
    selected: usize,
    searching: bool,
    ignored: Option<Arc<Vec<String>>>,
    preview: Option<(String, Entity<EditorState>)>,
    scroll: gpui_kit::UniformListScrollHandle,
    focus: gpui_kit::FocusHandle,
    /// Where its tab row dragged it (IntelliJ's popups move).
    drag: gpui_kit::component::dialog::DialogDrag,
    cancel: Arc<AtomicBool>,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SeEvent> for SearchEverywhere {}

/// "Foo.kt:12:3" → ("Foo.kt", line 11, column 2).
fn split_position(query: &str) -> (&str, Option<(u32, u32)>) {
    let mut parts = query.rsplitn(3, ':').collect::<Vec<_>>();
    parts.reverse();
    let num = |s: &str| s.trim().parse::<u32>().ok().filter(|n| *n > 0);
    match parts.as_slice() {
        [name, line, col] if num(line).is_some() && num(col).is_some() => (name, Some((num(line).unwrap() - 1, num(col).unwrap() - 1))),
        [a, b, line] if num(line).is_some() => (&query[..a.len() + 1 + b.len()], Some((num(line).unwrap() - 1, 0))),
        [name, line] if num(line).is_some() => (name, Some((num(line).unwrap() - 1, 0))),
        _ => (query, None),
    }
}

/// Find Action matching: every word of the query in the name, or fuzzy.
fn action_score(query: &str, name: &str) -> Option<i32> {
    let lower = name.to_lowercase();
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if !words.is_empty() && words.iter().all(|w| lower.contains(w.as_str())) {
        let prefix = if lower.starts_with(&words[0]) { 30 } else { 0 };
        return Some(100 + prefix - name.len() as i32 / 4);
    }
    fuzzy_score(&query.replace(' ', ""), name)
}

impl SearchEverywhere {
    pub fn new(index: Entity<CodeIndex>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type / to see commands"));
        let mut subscriptions = vec![cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.search(cx);
            }
        })];
        let weak = cx.entity().downgrade();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let Some(this) = weak.upgrade() else { return };
            let (input, root, preview) = {
                let p = this.read(cx);
                (p.input.clone(), p.focus.clone(), p.preview.as_ref().map(|(_, e)| e.clone()))
            };
            if !gpui_kit::Focusable::focus_handle(input.read(cx), cx).contains_focused(window, cx) {
                // Esc closes the popup from its result list and preview too.
                let in_preview = preview.is_some_and(|p| gpui_kit::Focusable::focus_handle(p.read(cx), cx).contains_focused(window, cx));
                if crate::ui::common::is_plain_escape(&event.keystroke) && (root.is_focused(window) || in_preview) {
                    this.update(cx, |_, cx| cx.emit(SeEvent::Close));
                    cx.stop_propagation();
                }
                return;
            }
            if this.update(cx, |this, cx| this.on_key(&event.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        }));
        Self {
            index,
            input,
            tab: SeTab::All,
            include_non_project: false,
            show_preview: false,
            hidden_groups: HashSet::from([SeTab::Text]),
            hidden_langs: HashSet::new(),
            actions: Rc::new(Vec::new()),
            rows: Vec::new(),
            selected: 0,
            searching: false,
            ignored: None,
            preview: None,
            scroll: gpui_kit::UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            drag: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _search: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn tab(&self) -> SeTab {
        self.tab
    }

    /// Opens on a tab. `text` (the editor selection) replaces the query.
    pub fn show(&mut self, tab: SeTab, text: Option<String>, actions: Vec<ActionEntry>, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        // Each opening starts with the project's items only.
        self.include_non_project = false;
        self.actions = Rc::new(actions);
        if let Some(text) = text.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        self.search(cx);
    }

    /// Pressing the popup's shortcut again: Include non-project items.
    pub fn toggle_non_project(&mut self, cx: &mut Context<Self>) {
        self.include_non_project = !self.include_non_project;
        self.search(cx);
    }

    fn set_tab(&mut self, tab: SeTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.preview = None;
        self.search(cx);
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let raw = self.input.read(cx).value().to_string();
        let query = raw.trim().to_owned();
        self.selected = 0;
        self.preview = None;
        // "/" lists the tab commands.
        if query.starts_with('/') && !query.contains(' ') {
            self.rows = SeTab::ALL
                .into_iter()
                .filter(|t| t.command().starts_with(&query))
                .map(|t| Row { group: SeTab::All, kind: Kind::Command(t) })
                .collect();
            self._search = None;
            cx.notify();
            return;
        }
        if query.is_empty() {
            self.rows.clear();
            self._search = None;
            cx.notify();
            return;
        }
        let tab = self.tab;
        let wants = |g: SeTab| tab == g || (tab == SeTab::All && !self.hidden_groups.contains(&g));
        let limit = if tab == SeTab::All { ALL_LIMIT + 1 } else { TAB_LIMIT };
        let (name, position) = split_position(&query);
        let name = name.to_owned();
        let index = self.index.read(cx);
        // Library items show their library, as IntelliJ's "< JDK 21 >".
        let libraries: Vec<(std::path::PathBuf, String)> =
            index.index.read().map(|i| i.external.libraries.iter().map(|l| (l.root.clone(), l.name.clone())).collect()).unwrap_or_default();
        let shown = move |path: &str| -> String {
            if !crate::index::store::ProjectIndex::is_external(path) {
                return path.to_owned();
            }
            libraries
                .iter()
                .find_map(|(root, name)| {
                    let rel = std::path::Path::new(path).strip_prefix(root).ok()?;
                    Some(format!("{name} › {}", rel.to_string_lossy().replace('\\', "/")))
                })
                .unwrap_or_else(|| path.to_owned())
        };
        let libraries = self.include_non_project;
        let classes = wants(SeTab::Classes).then(|| index.search_symbols(name.clone(), true, libraries, cx));
        let symbols = wants(SeTab::Symbols).then(|| index.search_symbols(name.clone(), false, libraries, cx));
        let files = wants(SeTab::Files).then(|| index.search_files(name.clone(), libraries, cx));
        let text = wants(SeTab::Text).then(|| {
            let q = TextQuery { text: raw.clone(), ..Default::default() };
            index.find_text(q, FileScope::Project, if tab == SeTab::All { ALL_LIMIT + 1 } else { TAB_LIMIT }, cancel.clone(), cx)
        });
        let ignored = (wants(SeTab::Files) && self.include_non_project).then(|| match &self.ignored {
            Some(list) => Task::ready(list.clone()),
            None => {
                let task = index.ignored_files(cx);
                cx.spawn(async move |_, _| Arc::new(task.await))
            }
        });
        let actions: Vec<(usize, i32)> = if wants(SeTab::Actions) {
            let mut found: Vec<(usize, i32)> = self.actions.iter().enumerate().filter_map(|(i, a)| action_score(&query, &a.name).map(|s| (i, s))).collect();
            found.sort_by(|a, b| b.1.cmp(&a.1));
            found
        } else {
            Vec::new()
        };
        self.searching = true;
        cx.notify();
        self._search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(60)).await;
            let mut rows: Vec<Row> = Vec::new();
            let at = |t: Target| match position {
                Some((line, col)) => Target { line, col, ..t },
                None => t,
            };
            let push_group = |rows: &mut Vec<Row>, group: SeTab, items: Vec<Kind>| {
                let more = tab == SeTab::All && items.len() > ALL_LIMIT;
                rows.extend(items.into_iter().take(if tab == SeTab::All { ALL_LIMIT } else { TAB_LIMIT }).map(|kind| Row { group, kind }));
                if more {
                    rows.push(Row { group, kind: Kind::More(group) });
                }
            };
            let lang_filter = this.update(cx, |this, _| this.hidden_langs.clone()).unwrap_or_default();
            let lang_ok = |path: &str| lang_filter.is_empty() || !Lang::from_path(path).is_some_and(|l| lang_filter.contains(l.name()));
            if let Some(task) = classes {
                let found = task.await;
                let items = found
                    .into_iter()
                    .filter(|m| lang_ok(&m.target.path))
                    .take(limit)
                    .map(|m| Kind::Target {
                        title: m.target.name.clone(),
                        detail: m.target.container.clone().unwrap_or_default(),
                        right: shown(&m.target.path),
                        icon: symbol_icon(m.kind),
                        target: at(m.target),
                    })
                    .collect();
                push_group(&mut rows, SeTab::Classes, items);
            }
            if let Some(task) = files {
                let mut found = task.await;
                if let Some(ignored) = ignored {
                    let list = ignored.await;
                    this.update(cx, |this, _| this.ignored = Some(list.clone())).ok();
                    let name2 = name.clone();
                    let extra: Vec<(String, i32)> = cx
                        .background_spawn(async move {
                            list.iter()
                                .filter_map(|p| {
                                    let file = p.rsplit('/').next().unwrap_or(p);
                                    fuzzy_score(&name2, file).map(|s| (p.clone(), s + 20))
                                })
                                .collect()
                        })
                        .await;
                    found.extend(extra);
                    found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.len().cmp(&b.0.len())));
                }
                let items = found
                    .into_iter()
                    .filter(|(p, _)| lang_ok(p))
                    .take(limit)
                    .map(|(path, _)| {
                        let file = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
                        let full = shown(&path);
                        let dir = full.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
                        Kind::Target {
                            title: file.clone(),
                            detail: dir,
                            right: String::new(),
                            icon: common::file_icon(&path),
                            target: at(Target { path, line: 0, col: 0, name: file, label: String::new(), container: None }),
                        }
                    })
                    .collect();
                push_group(&mut rows, SeTab::Files, items);
            }
            if let Some(task) = symbols {
                let found = task.await;
                let items = found
                    .into_iter()
                    .filter(|m| lang_ok(&m.target.path))
                    .take(limit)
                    .map(|m| {
                        let file = m.target.path.rsplit(['/', '\\']).next().unwrap_or(&m.target.path).to_owned();
                        Kind::Target {
                            title: m.target.name.clone(),
                            detail: match &m.target.container {
                                Some(c) => format!("{c} · {}", m.target.label),
                                None => m.target.label.clone(),
                            },
                            right: format!("{file}:{}", m.target.line + 1),
                            icon: symbol_icon(m.kind),
                            target: m.target,
                        }
                    })
                    .collect();
                push_group(&mut rows, SeTab::Symbols, items);
            }
            if !actions.is_empty() {
                let items = actions.into_iter().take(limit).map(|(i, _)| Kind::Action(i)).collect();
                push_group(&mut rows, SeTab::Actions, items);
            }
            if let Some(task) = text {
                if let Ok(found) = task.await {
                    let items = found.matches.into_iter().filter(|m| lang_ok(&m.path)).take(limit).map(Kind::Text).collect();
                    push_group(&mut rows, SeTab::Text, items);
                }
            }
            this.update(cx, |this, cx| {
                this.rows = rows;
                this.searching = false;
                this.selected = 0;
                cx.notify();
            })
            .ok();
        }));
    }

    fn on_key(&mut self, k: &gpui_kit::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &k.modifiers;
        let ctrl = m.control || m.platform;
        match (k.key.as_str(), m.alt, ctrl, m.shift) {
            ("escape", false, false, false) => cx.emit(SeEvent::Close),
            ("down", false, false, false) => self.move_selection(1, window, cx),
            ("up", false, false, false) => self.move_selection(-1, window, cx),
            ("pagedown", false, false, false) => self.move_selection(10, window, cx),
            ("pageup", false, false, false) => self.move_selection(-10, window, cx),
            ("enter", false, _, _) => self.pick(self.selected, window, cx),
            ("tab", false, false, shift) => {
                let i = SeTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0) as isize;
                let next = (i + if shift { -1 } else { 1 }).rem_euclid(SeTab::ALL.len() as isize) as usize;
                self.set_tab(SeTab::ALL[next], cx);
            }
            ("p", true, false, false) => {
                self.show_preview = !self.show_preview;
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.scroll.scroll_to_item(self.selected, gpui_kit::ScrollStrategy::Nearest);
        self.update_preview(window, cx);
        cx.notify();
    }

    fn row_target(&self, row: &Row) -> Option<Target> {
        match &row.kind {
            Kind::Target { target, .. } => Some(target.clone()),
            Kind::Text(m) => Some(target_of(m)),
            _ => None,
        }
    }

    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).cloned() else { return };
        match row.kind {
            Kind::Command(tab) => {
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                self.set_tab(tab, cx);
            }
            Kind::More(tab) => self.set_tab(tab, cx),
            Kind::Action(i) => {
                if let Some(action) = self.actions.get(i) {
                    let run = action.run.clone();
                    cx.emit(SeEvent::Close);
                    cx.emit(SeEvent::Run(run));
                }
            }
            _ => {
                if let Some(target) = self.row_target(&row) {
                    cx.emit(SeEvent::Close);
                    cx.emit(SeEvent::Open(target));
                }
            }
        }
    }

    fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_preview {
            return;
        }
        let Some(target) = self.rows.get(self.selected).and_then(|r| self.row_target(r)) else {
            self.preview = None;
            return;
        };
        let Some(root) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        if self.preview.as_ref().is_none_or(|(p, _)| *p != target.path) {
            let text = text_search::read_text(&root.join(&target.path)).unwrap_or_default();
            let language = crate::ui::file_editor::language_for(&target.path);
            let state = cx.new(|cx| {
                let mut state = EditorState::new(window, cx).language(language).line_number(true).soft_wrap(false).default_value(text);
                state.set_readonly(true, cx);
                state
            });
            self.preview = Some((target.path.clone(), state));
        }
        let typing = gpui_kit::Focusable::focus_handle(self.input.read(cx), cx).contains_focused(window, cx);
        if let Some((_, state)) = &self.preview {
            state.update(cx, |s, cx| s.set_cursor_position(lsp_types::Position::new(target.line, target.col), window, cx));
        }
        if typing {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    fn open_in_find_window(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        if query.trim().is_empty() {
            return;
        }
        if self.tab == SeTab::Text {
            cx.emit(SeEvent::FindWindowText(FindRequest {
                query: TextQuery { text: query, ..Default::default() },
                replacement: None,
                scope_label: "Project Files".into(),
                scope: FileScope::Project,
                new_tab: true,
            }));
        } else {
            let items = self
                .rows
                .iter()
                .filter_map(|r| match &r.kind {
                    Kind::Target { title, detail, icon, target, .. } => Some(FoundItem { title: title.clone(), detail: detail.clone(), icon: icon.clone(), target: target.clone() }),
                    Kind::Text(m) => Some(FoundItem { title: m.text.clone(), detail: format!("{}:{}", m.path, m.line + 1), icon: common::file_icon(&m.path), target: target_of(m) }),
                    _ => None,
                })
                .collect();
            cx.emit(SeEvent::FindWindowItems(format!("{} '{}'", self.tab.label(), query.trim()), items));
        }
        cx.emit(SeEvent::Close);
    }

    fn render_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let tab = self.tab;
        let hidden_groups = self.hidden_groups.clone();
        let hidden_langs = self.hidden_langs.clone();
        let active = if tab == SeTab::All { hidden_groups.len() != 1 || !hidden_groups.contains(&SeTab::Text) } else { !hidden_langs.is_empty() };
        Button::new("se-filter")
            .ghost()
            .xsmall()
            .icon(Icon::new(icons::FILTER))
            .tooltip("Filter")
            .selected(active)
            .disabled(tab == SeTab::Actions)
            .dropdown_menu(move |mut menu, _, _| {
                if tab == SeTab::All {
                    for g in [SeTab::Classes, SeTab::Files, SeTab::Symbols, SeTab::Actions, SeTab::Text] {
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(g.label()).checked(!hidden_groups.contains(&g)).on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                if !this.hidden_groups.remove(&g) {
                                    this.hidden_groups.insert(g);
                                }
                                this.search(cx);
                            })
                        }));
                    }
                } else {
                    for lang in Lang::ALL {
                        let name = lang.name();
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(name).checked(!hidden_langs.contains(name)).on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                if !this.hidden_langs.remove(name) {
                                    this.hidden_langs.insert(name);
                                }
                                this.search(cx);
                            })
                        }));
                    }
                }
                menu
            })
    }
}

impl Render for SearchEverywhere {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if self.show_preview && self.preview.is_none() && !self.rows.is_empty() {
            self.update_preview(window, cx);
        }
        let mut tabs = h_flex().gap_0p5();
        for tab in SeTab::ALL {
            tabs = tabs.child(
                Button::new(("se-tab", tab as usize))
                    .ghost()
                    .small()
                    .label(tab.label())
                    .selected(self.tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx))),
            );
        }
        let non_project = !matches!(self.tab, SeTab::Actions | SeTab::Text);
        let header = h_flex()
            .h(px(40.))
            .px_2()
            .gap_2()
            .child(tabs)
            // The empty space moves the popup.
            .child(div().h_full().flex_1().on_mouse_down(gpui_kit::MouseButton::Left, common::drag_start(&self.drag)))
            .when(non_project, |el| {
                el.child(Checkbox::new("se-non-project").label("Include non-project items").checked(self.include_non_project).on_click(cx.listener(
                    |this, checked: &bool, _, cx| {
                        this.include_non_project = *checked;
                        this.search(cx);
                    },
                )))
            })
            .child(
                tool_button("se-preview", icons::PREVIEW, "Preview (Alt+P)")
                    .selected(self.show_preview)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_preview = !this.show_preview;
                        this.preview = None;
                        this.update_preview(window, cx);
                        cx.notify();
                    })),
            )
            .child(self.render_filter(cx))
            .child(tool_button("se-find-window", icons::OPEN_NEW_TAB, "Open in Find Tool Window").on_click(cx.listener(|this, _, _, cx| this.open_in_find_window(cx))));

        let rows = Rc::new(self.rows.clone());
        let actions = self.actions.clone();
        let selected = self.selected;
        let all = self.tab == SeTab::All;
        let query_empty = self.input.read(cx).value().trim().is_empty();
        let body = if rows.is_empty() {
            let text = if query_empty {
                ""
            } else if self.searching {
                "Searching…"
            } else {
                "Nothing found"
            };
            div().h(px(if query_empty { 0. } else { 60. })).flex().items_center().justify_center().text_sm().text_color(palette.text_secondary).child(text).into_any_element()
        } else {
            let list = uniform_list(
                "se-results",
                rows.len(),
                cx.processor(move |_, range: Range<usize>, _, cx| {
                    let palette = cx.palette().clone();
                    range
                        .map(|ix| {
                            let row = &rows[ix];
                            let first = all && (ix == 0 || rows[ix - 1].group != row.group);
                            let base = h_flex()
                                .id(ix)
                                .w_full()
                                .h(px(26.))
                                .px_3()
                                .gap_2()
                                .text_sm()
                                .cursor_pointer()
                                .when(first && ix > 0, |el| el.border_t_1().border_color(palette.border))
                                .when(ix == selected, |el| el.bg(palette.selection))
                                .when(ix != selected, |el| el.hover(|el| el.bg(palette.hover)))
                                .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)));
                            let right = |text: String| div().flex_shrink_0().text_xs().text_color(palette.text_secondary).child(text);
                            let group_label = div().w(px(64.)).flex_shrink_0().text_xs().text_right().text_color(palette.text_secondary).child(if first { row.group.label() } else { "" });
                            match &row.kind {
                                Kind::Target { title, detail, right: r, icon, .. } => base
                                    .child(common::icon(icon.clone()).text_color(palette.text_secondary))
                                    .child(div().whitespace_nowrap().child(title.clone()))
                                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_xs().text_color(palette.text_secondary).child(detail.clone()))
                                    .child(right(r.clone()))
                                    .when(all, |el| el.child(group_label)),
                                Kind::Action(i) => {
                                    let a = &actions[*i];
                                    base.child(common::icon(icons::LIGHTNING).text_color(palette.text_secondary))
                                        .child(div().whitespace_nowrap().child(a.name.clone()))
                                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_xs().text_color(palette.text_secondary).child(a.group.clone()))
                                        .child(right(a.shortcut.clone()))
                                        .when(all, |el| el.child(group_label))
                                }
                                Kind::Text(m) => {
                                    let file = m.path.rsplit('/').next().unwrap_or(&m.path).to_owned();
                                    base.child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(highlighted(&m.text, &m.ranges, &palette)))
                                        .child(right(format!("{file} {}", m.line + 1)))
                                        .when(all, |el| el.child(group_label))
                                }
                                Kind::Command(tab) => base
                                    .child(div().w(px(80.)).child(tab.command()))
                                    .child(div().text_xs().text_color(palette.text_secondary).child(format!("Switch to {}", tab.label()))),
                                Kind::More(_) => base.child(div().pl(px(24.)).text_xs().text_color(palette.link).child("… more")),
                            }
                            .into_any_element()
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full();
            let height = (self.rows.len() as f32 * 26.).min(26. * 15.);
            let preview = self.preview.as_ref().filter(|_| self.show_preview).map(|(path, state)| {
                v_flex()
                    .h(px(260.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(h_flex().h(px(22.)).px_3().gap_1().text_xs().text_color(palette.text_secondary).child(common::icon(common::file_icon(path))).child(path.clone()))
                    .child(div().flex_1().min_h_0().child(Editor::new(state).bordered(false).h_full()))
            });
            v_flex().child(div().h(px(height)).border_t_1().border_color(palette.border).child(list)).children(preview).into_any_element()
        };

        let drag = self.drag.clone();
        v_flex()
            .id("search-everywhere")
            .key_context("SearchEverywhere")
            .track_focus(&self.focus)
            .relative()
            .left(drag.offset().x)
            .top(drag.offset().y)
            .child(drag.tracker())
            .w(px(720.))
            .bg(gpui_kit::Hsla { a: 1.0, ..palette.panel })
            .border_1()
            .border_color(palette.border)
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .child(header)
            .child(
                div().px_2().pb_2().child(
                    Input::new(&self.input)
                        .prefix(Icon::new(icons::SEARCH).small().text_color(palette.text_secondary)),
                ),
            )
            .child(body)
            .child(div().when(!self.rows.is_empty(), |el| {
                el.h(px(24.))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(palette.border)
                    .text_xs()
                    .text_color(palette.text_secondary)
                    .font_weight(FontWeight::NORMAL)
                    .child("Press Tab to switch tabs · Alt+P to preview · Enter to open")
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_and_actions() {
        assert_eq!(split_position("Main.kt:12"), ("Main.kt", Some((11, 0))));
        assert_eq!(split_position("Main.kt:12:5"), ("Main.kt", Some((11, 4))));
        assert_eq!(split_position("Main.kt"), ("Main.kt", None));
        assert_eq!(split_position("a:b"), ("a:b", None));
        assert!(action_score("push", "Push…").is_some());
        assert!(action_score("new br", "New Branch…").is_some());
        assert!(action_score("xyz", "Push…").is_none());
    }
}

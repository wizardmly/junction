//! Find in Files and Replace in Files (Ctrl+Shift+F / Ctrl+Shift+R), as
//! IntelliJ's popup: query with Match Case / Words / Regex, file mask,
//! context filter, In Project / Module / Directory / Scope, a result list
//! with a preview, and Open in Find Window.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::index::nav::Target;
use crate::index::service::CodeIndex;
use crate::index::text_search::{self, FileScope, SearchContext, SearchResult, TextMatch, TextQuery};
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::common::{self, tool_button};

/// What the workspace knows about files, for the Module and Scope tabs.
#[derive(Clone, Default)]
pub struct ScopeData {
    pub current_file: Option<String>,
    pub open_files: Vec<String>,
    pub recent_files: Vec<String>,
    pub recently_changed: Vec<String>,
    pub local_changes: Vec<String>,
    /// Folders holding a build file (Gradle, Cargo, Go, Dart, CMake, …), "" for the root.
    pub modules: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScopeTab {
    Project,
    Module,
    Directory,
    Scope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NamedScope {
    ProjectFiles,
    ProductionFiles,
    TestFiles,
    OpenFiles,
    CurrentFile,
    RecentlyViewed,
    RecentlyChanged,
    LocalChanges,
}

impl NamedScope {
    const ALL: [NamedScope; 8] = [
        NamedScope::ProjectFiles,
        NamedScope::ProductionFiles,
        NamedScope::TestFiles,
        NamedScope::OpenFiles,
        NamedScope::CurrentFile,
        NamedScope::RecentlyViewed,
        NamedScope::RecentlyChanged,
        NamedScope::LocalChanges,
    ];

    fn label(self) -> &'static str {
        match self {
            NamedScope::ProjectFiles => "Project Files",
            NamedScope::ProductionFiles => "Project Production Files",
            NamedScope::TestFiles => "Project Test Files",
            NamedScope::OpenFiles => "Open Files",
            NamedScope::CurrentFile => "Current File",
            NamedScope::RecentlyViewed => "Recently Viewed Files",
            NamedScope::RecentlyChanged => "Recently Changed Files",
            NamedScope::LocalChanges => "Local Changes",
        }
    }
}

/// Everything needed to run a search again from the Find tool window.
#[derive(Clone)]
pub struct FindRequest {
    pub query: TextQuery,
    /// `Some` for Replace in Files.
    pub replacement: Option<String>,
    /// "Project Files", "Directory app/src", …
    pub scope_label: String,
    pub scope: FileScope,
    pub new_tab: bool,
}

pub enum FindEvent {
    Open(Target),
    Close,
    ShowInFindWindow(FindRequest),
    /// Replace changed these files on disk.
    FilesChanged(Vec<String>),
}

const POPUP_LIMIT: usize = 1000;

pub struct FindPopup {
    index: Entity<CodeIndex>,
    replace_mode: bool,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    case_sensitive: bool,
    whole_words: bool,
    regex: bool,
    mask_on: bool,
    mask: Entity<InputState>,
    context: SearchContext,
    tab: ScopeTab,
    module: String,
    directory: Entity<InputState>,
    recursive: bool,
    named: NamedScope,
    data: ScopeData,
    pinned: bool,
    new_tab: bool,
    result: SearchResult,
    error: Option<String>,
    searching: bool,
    selected: usize,
    preview: Option<(String, Entity<EditorState>)>,
    /// New results came in: the preview moves to the selected match on the
    /// next render.
    preview_stale: bool,
    cancel: Arc<AtomicBool>,
    scroll: gpui_kit::UniformListScrollHandle,
    focus: gpui_kit::FocusHandle,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FindEvent> for FindPopup {}

impl FindPopup {
    pub fn new(index: Entity<CodeIndex>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx));
        let replacement = cx.new(|cx| InputState::new(window, cx));
        let mask = cx.new(|cx| InputState::new(window, cx).default_value("*.java"));
        let directory = cx.new(|cx| InputState::new(window, cx));
        let mut subscriptions = vec![cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.schedule_search(cx),
            InputEvent::PressEnter { secondary, .. } => {
                if *secondary {
                    this.open_in_find_window(cx)
                } else {
                    this.open_selected(window, cx)
                }
            }
            _ => {}
        })];
        for input in [&mask, &directory] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_search(cx);
                }
            }));
        }
        // Single-line inputs swallow Up/Down and Enter variants, so the
        // popup's keys are taken before the inputs see them.
        let weak = cx.entity().downgrade();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let Some(this) = weak.upgrade() else { return };
            // Only while typing in the popup's own fields: an open menu keeps its keys.
            let fields = {
                let p = this.read(cx);
                [p.query.clone(), p.replacement.clone(), p.mask.clone(), p.directory.clone()]
            };
            if !fields.iter().any(|f| gpui_kit::Focusable::focus_handle(f.read(cx), cx).contains_focused(window, cx)) {
                return;
            }
            let handled = this.update(cx, |this, cx| this.on_key(&event.keystroke, window, cx));
            if handled {
                cx.stop_propagation();
            }
        }));
        subscriptions.push(cx.subscribe_in(&replacement, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary, .. } = event {
                if *secondary { this.open_in_find_window(cx) } else { this.replace_selected(window, cx) }
            }
        }));
        Self {
            index,
            replace_mode: false,
            query,
            replacement,
            case_sensitive: false,
            whole_words: false,
            regex: false,
            mask_on: false,
            mask,
            context: SearchContext::Anywhere,
            tab: ScopeTab::Project,
            module: String::new(),
            directory,
            recursive: true,
            named: NamedScope::ProjectFiles,
            data: ScopeData::default(),
            pinned: false,
            new_tab: false,
            result: SearchResult::default(),
            error: None,
            searching: false,
            selected: 0,
            preview: None,
            preview_stale: false,
            cancel: Arc::new(AtomicBool::new(false)),
            scroll: gpui_kit::UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _search: None,
            _subscriptions: subscriptions,
        }
    }

    /// Shows the popup: `text` (the editor selection or the word at the
    /// caret) replaces the query when given.
    pub fn show(&mut self, replace: bool, text: Option<String>, data: ScopeData, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_mode = replace;
        if self.module.is_empty() || !data.modules.contains(&self.module) {
            self.module = data.modules.first().cloned().unwrap_or_default();
        }
        if self.directory.read(cx).value().is_empty() {
            let dir = data.current_file.as_deref().and_then(|f| f.rsplit_once('/')).map(|(d, _)| d.to_owned()).unwrap_or_default();
            self.directory.update(cx, |s, cx| s.set_value(dir, window, cx));
        }
        self.data = data;
        if let Some(text) = text.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.query.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.query.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        self.schedule_search(cx);
        cx.notify();
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    fn text_query(&self, cx: &App) -> TextQuery {
        TextQuery {
            text: self.query.read(cx).value().to_string(),
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            regex: self.regex,
            mask: self.mask_on.then(|| self.mask.read(cx).value().to_string()),
            context: self.context,
        }
    }

    fn scope(&self, cx: &App) -> (String, FileScope) {
        match self.tab {
            ScopeTab::Project => ("Project Files".into(), FileScope::Project),
            ScopeTab::Module => {
                let name = module_name(&self.module);
                (format!("Module '{name}'"), FileScope::Under { dir: self.module.clone(), recursive: true })
            }
            ScopeTab::Directory => {
                let dir = self.directory.read(cx).value().trim().trim_matches('/').to_owned();
                (format!("Directory {}", if dir.is_empty() { "<project root>" } else { &dir }), FileScope::Under { dir, recursive: self.recursive })
            }
            ScopeTab::Scope => {
                let files = |v: &Vec<String>| FileScope::Files(v.clone());
                let scope = match self.named {
                    NamedScope::ProjectFiles => FileScope::Project,
                    NamedScope::ProductionFiles => FileScope::Production,
                    NamedScope::TestFiles => FileScope::Tests,
                    NamedScope::OpenFiles => files(&self.data.open_files),
                    NamedScope::CurrentFile => FileScope::Files(self.data.current_file.iter().cloned().collect()),
                    NamedScope::RecentlyViewed => files(&self.data.recent_files),
                    NamedScope::RecentlyChanged => files(&self.data.recently_changed),
                    NamedScope::LocalChanges => files(&self.data.local_changes),
                };
                (self.named.label().into(), scope)
            }
        }
    }

    fn request(&self, cx: &App) -> FindRequest {
        let (scope_label, scope) = self.scope(cx);
        FindRequest {
            query: self.text_query(cx),
            replacement: self.replace_mode.then(|| self.replacement.read(cx).value().to_string()),
            scope_label,
            scope,
            new_tab: self.new_tab,
        }
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let query = self.text_query(cx);
        if query.text.is_empty() {
            self.result = SearchResult::default();
            self.error = None;
            self.searching = false;
            self.preview = None;
            self._search = None;
            cx.notify();
            return;
        }
        let (_, scope) = self.scope(cx);
        self.searching = true;
        cx.notify();
        let index = self.index.clone();
        self._search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let task = cx.update(|cx| index.read(cx).find_text(query, scope, POPUP_LIMIT, cancel, cx));
            let found = task.await;
            this.update(cx, |this, cx| {
                this.searching = false;
                match found {
                    Ok(result) => {
                        this.result = result;
                        this.error = None;
                    }
                    Err(error) => {
                        this.result = SearchResult::default();
                        this.error = Some(error).filter(|e| !e.is_empty());
                    }
                }
                this.selected = 0;
                this.preview_dirty(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The preview follows the selection on the next render (it needs the window).
    fn preview_dirty(&mut self, _: &mut Context<Self>) {
        if self.result.matches.is_empty() {
            self.preview = None;
        }
        self.preview_stale = true;
    }

    fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.result.matches.get(self.selected).cloned() else { return };
        let Some(root) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let fresh = self.preview.as_ref().is_none_or(|(path, _)| *path != m.path);
        if fresh {
            let text = text_search::read_text(&root.join(&m.path)).unwrap_or_default();
            let language = crate::ui::file_editor::language_for(&m.path);
            let state = cx.new(|cx| {
                let mut state = EditorState::new(window, cx).language(language).line_number(true).soft_wrap(false).default_value(text);
                state.set_readonly(true, cx);
                state
            });
            self.preview = Some((m.path.clone(), state));
        }
        let typing = gpui_kit::Focusable::focus_handle(self.query.read(cx), cx).contains_focused(window, cx);
        if let Some((_, state)) = &self.preview {
            let range = m.offset.clone();
            let found = m.ranges.first().and_then(|r| m.text.get(r.clone())).unwrap_or_default().to_owned();
            let insensitive = !self.case_sensitive;
            state.update(cx, |state, cx| {
                state.set_search_query(found, insensitive, cx);
                state.set_cursor_position(lsp_types::Position::new(m.line, m.col), window, cx);
                state.set_selected_range(range, cx);
            });
        }
        // Moving the preview's caret focuses it; keep typing in the query.
        if typing {
            self.query.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.result.matches.len() {
            self.selected = ix;
            self.scroll.scroll_to_item(ix, gpui_kit::ScrollStrategy::Nearest);
            self.update_preview(window, cx);
            cx.notify();
        }
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.result.matches.len();
        if n == 0 {
            return;
        }
        let ix = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        self.select(ix, window, cx);
    }

    fn open_selected(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.result.matches.get(self.selected) else { return };
        let target = target_of(m);
        cx.emit(FindEvent::Open(target));
        if !self.pinned {
            cx.emit(FindEvent::Close);
        }
    }

    fn open_in_find_window(&mut self, cx: &mut Context<Self>) {
        if self.query.read(cx).value().is_empty() {
            return;
        }
        let request = self.request(cx);
        cx.emit(FindEvent::ShowInFindWindow(request));
        cx.emit(FindEvent::Close);
    }

    /// Replace: the selected line's occurrences, then the next line.
    fn replace_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.result.matches.get(self.selected).cloned() else { return };
        let query = self.text_query(cx);
        let replacement = self.replacement.read(cx).value().to_string();
        let Some(root) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let Ok(re) = text_search::compile(&query) else { return };
        let path = root.join(&m.path);
        let Some(text) = text_search::read_text(&path) else { return };
        let starts: Vec<usize> = text_search::occurrences(&m.path, &text, &re, query.context)
            .into_iter()
            .filter(|r| line_of(&text, r.start) == m.line as usize)
            .map(|r| r.start)
            .collect();
        let (new_text, count) = text_search::replace(&m.path, &text, &query, &re, &replacement, Some(&starts));
        if count > 0 && std::fs::write(&path, new_text).is_ok() {
            self.preview = None;
            cx.emit(FindEvent::FilesChanged(vec![m.path.clone()]));
            self.schedule_search(cx);
        }
        let _ = window;
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self.request(cx);
        let Some(replacement) = request.replacement.clone() else { return };
        let (occurrences, files) = (self.result.occurrences, self.result.files);
        if occurrences == 0 {
            return;
        }
        let more = if self.result.truncated { "+" } else { "" };
        let entity = cx.entity();
        let message = format!(
            "Replace {occurrences}{more} occurrences of '{}' with '{}' in {files}{more} files?",
            request.query.text, replacement
        );
        window.open_dialog(cx, move |dialog, _, _| {
            let entity = entity.clone();
            let request = request.clone();
            dialog
                .title("Replace All")
                .w(px(480.))
                .child(div().text_sm().child(message.clone()))
                .footer(crate::ui::dialogs::footer("Replace"))
                .on_ok(move |_, _, cx| {
                    let (entity, request) = (entity.clone(), request.clone());
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| this.run_replace_all(request, cx));
                    });
                    true
                })
        });
    }

    fn run_replace_all(&mut self, request: FindRequest, cx: &mut Context<Self>) {
        let index = self.index.clone();
        let root = index.read(cx).root().map(|r| r.to_path_buf());
        let task = cx.background_spawn(async move {
            let root = root?;
            replace_everywhere(&root, &request)
        });
        cx.spawn(async move |this, cx| {
            let changed = task.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                this.preview = None;
                if !changed.is_empty() {
                    cx.emit(FindEvent::FilesChanged(changed));
                }
                this.schedule_search(cx);
            })
            .ok();
        })
        .detach();
    }

    fn toggle(&mut self, f: impl FnOnce(&mut Self), cx: &mut Context<Self>) {
        f(self);
        self.schedule_search(cx);
    }
}

/// Replaces every occurrence of a request in its scope; the changed files.
pub fn replace_everywhere(root: &std::path::Path, request: &FindRequest) -> Option<Vec<String>> {
    let replacement = request.replacement.clone()?;
    let re = text_search::compile(&request.query).ok()?;
    let cancel = AtomicBool::new(false);
    let files = request.scope.select(|| crate::index::store::list_files(root));
    let result = text_search::search(root, &files, &request.query, &re, usize::MAX, &cancel);
    let mut paths: Vec<String> = result.matches.iter().map(|m| m.path.clone()).collect();
    paths.dedup();
    let mut changed = Vec::new();
    for rel in paths {
        let path = root.join(&rel);
        let Some(text) = text_search::read_text(&path) else { continue };
        let (new_text, count) = text_search::replace(&rel, &text, &request.query, &re, &replacement, None);
        if count > 0 && std::fs::write(&path, new_text).is_ok() {
            changed.push(rel);
        }
    }
    Some(changed)
}

pub fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|b| **b == b'\n').count()
}

pub fn target_of(m: &TextMatch) -> Target {
    Target { path: m.path.clone(), line: m.line, col: m.col, name: String::new(), label: String::new(), container: None }
}

pub fn module_name(dir: &str) -> String {
    if dir.is_empty() { "<root>".into() } else { dir.rsplit('/').next().unwrap_or(dir).to_owned() }
}

/// A result line with its occurrences highlighted, as IntelliJ shows them.
pub fn highlighted(text: &str, ranges: &[Range<usize>], palette: &Palette) -> StyledText {
    let style = HighlightStyle {
        background_color: Some(if palette.dark { gpui_kit::hsla(0.13, 0.5, 0.35, 0.8) } else { gpui_kit::hsla(0.15, 1.0, 0.75, 0.9) }),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.start < r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
        .map(|r| (r.clone(), style))
        .collect();
    StyledText::new(SharedString::from(text.to_owned())).with_highlights(ranges)
}

/// A text toggle on the search field: "Cc", "W", ".*".
fn toggle_button(id: &'static str, label: &'static str, tooltip: &'static str, on: bool) -> Button {
    Button::new(id).ghost().xsmall().label(label).tooltip(tooltip).selected(on)
}

impl Render for FindPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        if (self.preview.is_none() || self.preview_stale) && !self.result.matches.is_empty() {
            self.preview_stale = false;
            self.update_preview(window, cx);
        }
        let entity = cx.entity();
        let title = if self.replace_mode { "Replace in Files" } else { "Find in Files" };

        let context = self.context;
        let header = h_flex()
            .h(px(40.))
            .px_3()
            .gap_2()
            .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
            .child(div().flex_1())
            .child(Checkbox::new("find-mask-on").label("File mask:").checked(self.mask_on).on_click(cx.listener(|this, checked: &bool, _, cx| {
                let checked = *checked;
                this.toggle(|this| this.mask_on = checked, cx)
            })))
            .child(
                h_flex()
                    .w(px(130.))
                    .child(div().flex_1().child(Input::new(&self.mask).xsmall().disabled(!self.mask_on)))
                    .child(Button::new("find-mask-menu").ghost().xsmall().icon(Icon::new(IconName::ChevronDown)).disabled(!self.mask_on).dropdown_menu({
                        let entity = entity.clone();
                        move |mut menu, _, _| {
                            for mask in ["*.java", "*.kt", "*.kts", "*.xml", "*.gradle", "*.c", "*.cpp", "*.h", "*.rs", "*.swift", "*.dart", "*.go", "*.m", "*.v", "*.ts", "*.py", "*.md"] {
                                let entity = entity.clone();
                                menu = menu.item(PopupMenuItem::new(mask).on_click(move |_, window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.mask.update(cx, |s, cx| s.set_value(mask, window, cx));
                                        this.schedule_search(cx);
                                    })
                                }));
                            }
                            menu
                        }
                    })),
            )
            .child(
                Button::new("find-filter")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Funnel))
                    .tooltip("Filter Search Results")
                    .selected(context != SearchContext::Anywhere)
                    .dropdown_menu({
                        let entity = entity.clone();
                        move |mut menu, _, _| {
                            menu = menu.label("Context:");
                            for c in SearchContext::ALL {
                                let entity = entity.clone();
                                menu = menu.item(PopupMenuItem::new(c.label()).checked(c == context).on_click(move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.toggle(|this| this.context = c, cx))
                                }));
                            }
                            menu
                        }
                    }),
            )
            .child(
                tool_button("find-pin", IconName::Pin, if self.pinned { "Unpin Window" } else { "Pin Window" })
                    .selected(self.pinned)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pinned = !this.pinned;
                        cx.notify();
                    })),
            );

        let toggles = h_flex()
            .gap_0p5()
            .child(toggle_button("find-case", "Cc", "Match Case (Alt+C)", self.case_sensitive).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.case_sensitive = !t.case_sensitive, cx))))
            .child(toggle_button("find-words", "W", "Words (Alt+W)", self.whole_words).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.whole_words = !t.whole_words, cx))))
            .child(toggle_button("find-regex", ".*", "Regex (Alt+X)", self.regex).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.regex = !t.regex, cx))));
        let search_row = div().px_3().child(
            Input::new(&self.query)
                .prefix(Icon::new(IconName::Search).small().text_color(palette.text_secondary))
                .suffix(toggles),
        );
        let replace_row = self.replace_mode.then(|| {
            div().px_3().pt_1().child(Input::new(&self.replacement).prefix(Icon::new(IconName::Replace).small().text_color(palette.text_secondary)))
        });
        let error = self.error.clone().map(|e| div().px_3().pt_1().text_xs().text_color(palette.status_conflict).child(e));

        let tab_button = |id: &'static str, label: &'static str, tab: ScopeTab, current: ScopeTab| {
            Button::new(id).ghost().small().label(label).selected(tab == current)
        };
        let current = self.tab;
        let mut scope_row = h_flex()
            .h(px(36.))
            .px_3()
            .gap_1()
            .child(tab_button("find-tab-project", "In Project", ScopeTab::Project, current).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.tab = ScopeTab::Project, cx))))
            .child(tab_button("find-tab-module", "Module", ScopeTab::Module, current).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.tab = ScopeTab::Module, cx))))
            .child(tab_button("find-tab-directory", "Directory", ScopeTab::Directory, current).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.tab = ScopeTab::Directory, cx))))
            .child(tab_button("find-tab-scope", "Scope", ScopeTab::Scope, current).on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.tab = ScopeTab::Scope, cx))));
        match self.tab {
            ScopeTab::Project => {}
            ScopeTab::Module => {
                let modules = self.data.modules.clone();
                scope_row = scope_row.child(
                    Button::new("find-module")
                        .outline()
                        .small()
                        .label(module_name(&self.module))
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |mut menu, _, _| {
                                for m in modules.iter() {
                                    let (entity, m2) = (entity.clone(), m.clone());
                                    let label = if m.is_empty() { "<root>".to_owned() } else { format!("{}   {}", module_name(m), m) };
                                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                                        let m2 = m2.clone();
                                        entity.update(cx, |this, cx| this.toggle(|t| t.module = m2, cx))
                                    }));
                                }
                                menu
                            }
                        }),
                );
            }
            ScopeTab::Directory => {
                scope_row = scope_row
                    .child(div().w(px(240.)).child(Input::new(&self.directory).small()))
                    .child(
                        tool_button("find-recursive", IconName::FolderTree, "Recursive")
                            .selected(self.recursive)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle(|t| t.recursive = !t.recursive, cx))),
                    )
                    .child(tool_button("find-browse", IconName::FolderOpen, "Browse…").on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))));
            }
            ScopeTab::Scope => {
                let named = self.named;
                scope_row = scope_row.child(
                    Button::new("find-scope")
                        .outline()
                        .small()
                        .label(named.label())
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let entity = entity.clone();
                            move |mut menu, _, _| {
                                for s in NamedScope::ALL {
                                    let entity = entity.clone();
                                    menu = menu.item(PopupMenuItem::new(s.label()).checked(s == named).on_click(move |_, _, cx| {
                                        entity.update(cx, |this, cx| this.toggle(|t| t.named = s, cx))
                                    }));
                                }
                                menu
                            }
                        }),
                );
            }
        }
        let count = if self.result.matches.is_empty() {
            String::new()
        } else {
            let plus = if self.result.truncated { "+" } else { "" };
            let s = |n: usize| if n == 1 { "" } else { "es" };
            let f = |n: usize| if n == 1 { "" } else { "s" };
            format!(
                "{}{plus} match{} in {}{plus} file{}",
                self.result.occurrences,
                s(self.result.occurrences),
                self.result.files,
                f(self.result.files)
            )
        };
        scope_row = scope_row.child(div().flex_1()).child(div().text_xs().text_color(palette.text_secondary).child(count));

        let has_query = !self.query.read(cx).value().is_empty();
        let matches = Rc::new(self.result.matches.clone());
        let selected = self.selected;
        let body = if !has_query || matches.is_empty() {
            let text = if !has_query {
                "Type search query to find in files"
            } else if self.searching {
                "Searching…"
            } else {
                "Nothing found"
            };
            div().flex_1().min_h_0().flex().items_center().justify_center().text_sm().text_color(palette.text_secondary).child(text).into_any_element()
        } else {
            let list = uniform_list(
                "find-results",
                matches.len(),
                cx.processor(move |_, range: Range<usize>, _, cx| {
                    let palette = cx.palette().clone();
                    range
                        .map(|ix| {
                            let m = &matches[ix];
                            let name = m.path.rsplit('/').next().unwrap_or(&m.path).to_owned();
                            h_flex()
                                .id(ix)
                                .w_full()
                                .h(px(22.))
                                .px_3()
                                .gap_3()
                                .text_sm()
                                .cursor_pointer()
                                .when(ix == selected, |el| el.bg(palette.selection))
                                .when(ix != selected, |el| el.hover(|el| el.bg(palette.hover)))
                                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(highlighted(&m.text, &m.ranges, &palette)))
                                .child(div().flex_shrink_0().text_xs().text_color(palette.text_secondary).child(format!("{name} {}", m.line + 1)))
                                .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                                    this.select(ix, window, cx);
                                    if event.click_count() > 1 {
                                        this.open_selected(window, cx);
                                    }
                                }))
                                .into_any_element()
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full();
            let preview = self.preview.as_ref().map(|(path, state)| {
                v_flex()
                    .h(px(250.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        h_flex()
                            .h(px(24.))
                            .px_3()
                            .gap_1()
                            .text_xs()
                            .text_color(palette.text_secondary)
                            .child(common::icon(common::file_icon(path)))
                            .child(path.clone()),
                    )
                    .child(div().flex_1().min_h_0().child(Editor::new(state).bordered(false).h_full()))
            });
            v_flex()
                .flex_1()
                .min_h_0()
                .child(div().h(px(200.)).flex_shrink_0().border_t_1().border_color(palette.border).child(list))
                .children(preview)
                .into_any_element()
        };

        let footer = h_flex()
            .h(px(44.))
            .px_3()
            .gap_2()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.toolbar)
            .child(Checkbox::new("find-new-tab").label("Open results in new tab").checked(self.new_tab).on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.new_tab = *checked;
                cx.notify();
            })))
            .child(div().flex_1())
            .child(div().text_xs().text_color(palette.text_secondary).child("Ctrl+Enter"))
            .child(
                Button::new("find-open-window")
                    .outline()
                    .small()
                    .label(if self.replace_mode { "Open in Find Window" } else { "Open in Find Window" })
                    .disabled(!has_query)
                    .on_click(cx.listener(|this, _, _, cx| this.open_in_find_window(cx))),
            )
            .when(self.replace_mode, |el| {
                el.child(
                    Button::new("find-replace")
                        .outline()
                        .small()
                        .label("Replace")
                        .disabled(self.result.matches.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.replace_selected(window, cx))),
                )
                .child(
                    Button::new("find-replace-all")
                        .primary()
                        .small()
                        .label("Replace All")
                        .disabled(self.result.matches.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.replace_all(window, cx))),
                )
            });

        v_flex()
            .id("find-popup")
            .key_context("FindPopup")
            .w(px(780.))
            .h(px(if self.replace_mode { 680. } else { 640. }))
            .bg(gpui_kit::Hsla { a: 1.0, ..palette.panel })
            .border_1()
            .border_color(palette.border)
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .track_focus(&self.focus)
            .child(header)
            .child(search_row)
            .children(replace_row)
            .children(error)
            .child(scope_row)
            .child(body)
            .child(footer)
    }
}

impl FindPopup {
    fn on_key(&mut self, k: &gpui_kit::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &k.modifiers;
        let ctrl = m.control || m.platform;
        match (k.key.as_str(), m.alt, ctrl, m.shift) {
            ("escape", false, false, false) => cx.emit(FindEvent::Close),
            ("down", false, false, false) => self.move_selection(1, window, cx),
            ("up", false, false, false) => self.move_selection(-1, window, cx),
            ("pagedown", false, false, false) => self.move_selection(8, window, cx),
            ("pageup", false, false, false) => self.move_selection(-8, window, cx),
            ("enter", false, true, false) => self.open_in_find_window(cx),
            ("enter", false, false, false) => {
                if self.replace_mode && gpui_kit::Focusable::focus_handle(self.replacement.read(cx), cx).is_focused(window) {
                    self.replace_selected(window, cx)
                } else {
                    self.open_selected(window, cx)
                }
            }
            ("c", true, false, false) => self.toggle(|t| t.case_sensitive = !t.case_sensitive, cx),
            ("w", true, false, false) => self.toggle(|t| t.whole_words = !t.whole_words, cx),
            ("x", true, false, false) => self.toggle(|t| t.regex = !t.regex, cx),
            ("p", true, false, false) => self.toggle(|t| t.tab = ScopeTab::Project, cx),
            ("m", true, false, false) => self.toggle(|t| t.tab = ScopeTab::Module, cx),
            ("d", true, false, false) => self.toggle(|t| t.tab = ScopeTab::Directory, cx),
            ("s", true, false, false) => self.toggle(|t| t.tab = ScopeTab::Scope, cx),
            ("k", true, false, false) => self.toggle(|t| t.mask_on = !t.mask_on, cx),
            ("b", true, false, false) => {
                self.new_tab = !self.new_tab;
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.index.read(cx).root().map(|r| r.to_path_buf()) else { return };
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        let directory = self.directory.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let rel = path.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            this.update_in(cx, |this, window, cx| {
                directory.update(cx, |s, cx| s.set_value(rel, window, cx));
                this.schedule_search(cx);
            })
            .ok();
        })
        .detach();
    }
}

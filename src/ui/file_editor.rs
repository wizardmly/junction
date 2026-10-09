//! The file editor: a working-tree file (editable, saved with Ctrl+S) or a
//! file at a revision (read-only), with syntax highlighting and IntelliJ's
//! change markers against HEAD. Its context menu carries the editor's Git
//! actions: Show History for Selection, Annotate, Show Current Revision,
//! Rollback Lines.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{
    h_flex,
    input::{
        Copy, Cut, DefinitionProvider, Editor, EditorState, HoverProvider, InputEvent, Paste, Rope, RopeExt as _, SelectAll,
        ShowDocumentHandler,
    },
    native_menu::NativeMenu,
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, EventEmitter, InteractiveElement as _, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, actions, anchored, canvas, deferred, div, fill, point, prelude::FluentBuilder as _, px, size,
};

use crate::git::Repository;
use crate::index::nav::{self, Target};
use crate::index::service::CodeIndex;
use crate::git::diff::{self, Hunk};
use crate::theme::ActivePalette as _;
use crate::ui::annotation_gutter::{Annotation, AnnotationAction};
use crate::ui::common::{self, tool_button};
use crate::ui::diff_view::DiffSource;

actions!(
    file_editor,
    [
        SaveFile,
        ShowSelectionHistory,
        AnnotateFile,
        ShowFileHistory,
        ShowCurrentRevision,
        RollbackLines,
        ShowFileDiff,
        OpenOnHosting,
        CreateGist,
        GotoDeclaration,
        FindUsages,
        FindNext,
        FindPrevious,
        CloseChangePopup,
        NextChange,
        PreviousChange,
        OpenFind,
        OpenReplace,
        CompareWithClipboard
    ]
);

const CONTEXT: &str = "FileEditor";
const POPUP_CONTEXT: &str = "ChangePopup";
/// The change marker strip: its width, and the gap to the text.
const MARKER_WIDTH: f32 = 3.;
const MARKER_GAP: f32 = 4.;

pub fn init(cx: &mut gpui_kit::App) {
    register_grammars();
    cx.bind_keys([
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" }, SaveFile, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-z", RollbackLines, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-b" } else { "ctrl-b" }, GotoDeclaration, Some(CONTEXT)),
        // Next / Previous Change: Ctrl+Alt+Shift+Down / Up (⌃⌥⇧↓ / ↑ on macOS).
        KeyBinding::new("ctrl-alt-shift-down", NextChange, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-shift-up", PreviousChange, Some(CONTEXT)),
        KeyBinding::new("alt-f7", FindUsages, Some(CONTEXT)),
        // IntelliJ: Ctrl+R replaces (Ctrl+F finds, from the editor itself), F3 / Shift+F3 step.
        KeyBinding::new("secondary-f", OpenFind, Some(CONTEXT)),
        KeyBinding::new("secondary-r", OpenReplace, Some(CONTEXT)),
        // Over the text field's own Ctrl+F / Ctrl+H panel.
        KeyBinding::new("secondary-f", OpenFind, Some("FileEditor > Input")),
        KeyBinding::new("secondary-r", OpenReplace, Some("FileEditor > Input")),
        KeyBinding::new("f3", FindNext, Some(CONTEXT)),
        KeyBinding::new("shift-f3", FindPrevious, Some(CONTEXT)),
        KeyBinding::new("escape", CloseChangePopup, Some(POPUP_CONTEXT)),
    ]);
}

pub enum FileEditorEvent {
    Annotate { path: String, revision: Option<String> },
    ShowHistory(String),
    SelectionHistory { path: String, lines: (usize, usize) },
    SelectCommit(String),
    OpenDiff(DiffSource),
    FilesChanged,
    CreateGist { name: String, content: String },
    /// Go to Declaration found these (one jumps, several ask).
    Navigate(Vec<crate::index::nav::Target>),
    /// Go to Declaration found nothing for the identifier at this offset
    /// (on a declaration itself, IntelliJ shows its usages instead).
    NoDeclaration { text: String, offset: usize },
    /// Find Usages of the identifier at a byte offset of this text.
    FindUsages { text: String, offset: usize },
    /// Saved: re-index this file.
    Saved(String),
    /// The text was edited (a preview tab becomes a normal one).
    Edited,
}

impl EventEmitter<FileEditorEvent> for FileEditor {}

/// Highlighting the toolkit lacks: its C++ query is only the C++ additions
/// (tree-sitter-cpp's queries inherit C's), Swift has none, Dart isn't
/// built in.
fn register_grammars() {
    use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
    let registry = LanguageRegistry::singleton();
    for (name, language, highlights) in grammars() {
        registry.register(name, &LanguageConfig::new(name, language, Vec::new(), &highlights, "", ""));
    }
}

fn grammars() -> [(&'static str, tree_sitter::Language, String); 3] {
    [
        ("cpp", tree_sitter_cpp::LANGUAGE.into(), format!("{}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY)),
        ("swift", tree_sitter_swift::LANGUAGE.into(), tree_sitter_swift::HIGHLIGHTS_QUERY.to_owned()),
        ("dart", tree_sitter_dart::LANGUAGE.into(), tree_sitter_dart::HIGHLIGHTS_QUERY.to_owned()),
    ]
}

#[cfg(test)]
#[test]
fn registered_grammars_compile() {
    for (name, language, highlights) in grammars() {
        if let Err(error) = tree_sitter::Query::new(&language, &highlights) {
            panic!("{name}: {error}");
        }
    }
}

/// The tree-sitter language for a file name (plain text when unknown).
pub fn language_for(path: &str) -> &'static str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    // C++ standard headers have no extension (External Libraries).
    if ext.is_empty() && crate::index::store::ProjectIndex::is_external(path) && crate::index::libraries::library_lang(path).is_some() {
        return "cpp";
    }
    match (name.as_str(), ext) {
        (_, "swiftinterface") => "swift",
        ("cmakelists.txt", _) | (_, "cmake") => "cmake",
        ("makefile" | "gnumakefile", _) | (_, "mk") => "make",
        (_, "rs") => "rust",
        (_, "kt" | "kts") => "kotlin",
        (_, "java") => "java",
        (_, "swift") => "swift",
        (_, "dart") => "dart",
        (_, "go") => "go",
        (_, "c" | "h") => "c",
        (_, "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "mm" | "m") => "cpp",
        (_, "js" | "mjs" | "cjs" | "jsx") => "javascript",
        (_, "ts" | "tsx") => "typescript",
        (_, "py") => "python",
        (_, "json") => "json",
        (_, "toml") => "toml",
        (_, "yml" | "yaml") => "yaml",
        (_, "md" | "markdown") => "markdown",
        (_, "sh" | "bash" | "zsh") => "bash",
        (_, "proto") => "proto",
        _ => "plaintext",
    }
}

pub struct FileEditor {
    repository: Repository,
    path: String,
    /// `None` for the working tree.
    revision: Option<String>,
    state: Entity<EditorState>,
    /// Ctrl+F / Ctrl+R.
    find: Entity<crate::ui::find_bar::FindBar>,
    /// Line separator and indent, for the status bar.
    format: TextFormat,
    /// HEAD's version, for change markers.
    base: String,
    saved: String,
    hunks: Vec<Hunk>,
    /// The change block whose popup is open (a click on its gutter marker).
    popup: Option<usize>,
    /// Where the popup goes: under its block, as the last paint laid it out.
    popup_anchor: Rc<std::cell::Cell<Option<Point<Pixels>>>>,
    popup_focus: gpui_kit::FocusHandle,
    error: Option<String>,
    code_index: Option<Entity<CodeIndex>>,
    /// Annotate with Git Blame, shown in the gutter.
    annotation: Option<Annotation>,
    annotation_task: Option<gpui_kit::Task<()>>,
    /// The annotation hover card: its line and where the mouse rested.
    hover_card: Option<(usize, Point<Pixels>)>,
    hover_task: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Ctrl+click / Ctrl+hover and Quick Documentation through the code index.
struct IndexProvider {
    index: Entity<CodeIndex>,
    path: String,
    /// All targets of the last lookup: the editor follows only the first,
    /// the show-document hook hands the whole list on.
    last: Rc<RefCell<Vec<Target>>>,
}

fn lsp_position(line: u32, col: u32) -> lsp_types::Position {
    lsp_types::Position { line, character: col }
}

impl DefinitionProvider for IndexProvider {
    fn definitions(&self, text: &Rope, offset: usize, _: &mut Window, cx: &mut gpui_kit::App) -> gpui_kit::Task<anyhow::Result<Vec<lsp_types::LocationLink>>> {
        let text = text.to_string();
        let origin = nav::word_at(&text, offset).map(|(_, r)| {
            let (sl, sc) = nav::position(&text, r.start);
            let (el, ec) = nav::position(&text, r.end);
            lsp_types::Range { start: lsp_position(sl, sc), end: lsp_position(el, ec) }
        });
        let root = self.index.read(cx).root().map(|r| r.to_path_buf()).unwrap_or_default();
        let task = self.index.read(cx).definitions(self.path.clone(), text, offset, cx);
        let last = self.last.clone();
        cx.spawn(async move |_| {
            let targets = task.await;
            *last.borrow_mut() = targets.clone();
            Ok(targets
                .iter()
                .filter_map(|t| {
                    let uri = crate::index::lsp::path_to_uri(&root.join(&t.path)).parse::<lsp_types::Uri>().ok()?;
                    let range = lsp_types::Range { start: lsp_position(t.line, t.col), end: lsp_position(t.line, t.col) };
                    Some(lsp_types::LocationLink { origin_selection_range: origin, target_uri: uri, target_range: range, target_selection_range: range })
                })
                .collect())
        })
    }
}

impl HoverProvider for IndexProvider {
    fn hover(&self, text: &Rope, offset: usize, _: &mut Window, cx: &mut gpui_kit::App) -> gpui_kit::Task<anyhow::Result<Option<lsp_types::Hover>>> {
        let task = self.index.read(cx).hover(self.path.clone(), text.to_string(), offset, cx);
        cx.spawn(async move |_| {
            Ok(task.await.map(|value| lsp_types::Hover {
                contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent { kind: lsp_types::MarkupKind::Markdown, value }),
                range: None,
            }))
        })
    }
}

fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(ix, _)| ix + 1)
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count()
}

/// What the status bar shows about a file besides the caret.
#[derive(Clone, Debug, PartialEq)]
pub struct TextFormat {
    pub crlf: bool,
    /// "4 spaces", "2 spaces" or "Tab".
    pub indent: &'static str,
}

impl TextFormat {
    pub fn of(text: &str) -> Self {
        let crlf = text.find('\n').is_some_and(|ix| ix > 0 && text.as_bytes()[ix - 1] == b'\r');
        let mut tabs = 0;
        let mut widths = [0usize; 9];
        for line in text.lines().take(2000) {
            if line.starts_with('\t') {
                tabs += 1;
            } else {
                let n = line.len() - line.trim_start_matches(' ').len();
                if n > 0 && n < line.len() {
                    // The unit that divides the indent: 2, 4 or 8.
                    for unit in [8, 4, 2] {
                        if n % unit == 0 {
                            widths[unit] += 1;
                            break;
                        }
                    }
                }
            }
        }
        let spaces: usize = widths.iter().sum();
        let indent = if tabs > spaces {
            "Tab"
        } else if widths[2] > 0 && widths[2] * 5 >= spaces {
            "2 spaces"
        } else if widths[8] > 0 && widths[4] == 0 && widths[2] == 0 {
            "8 spaces"
        } else {
            "4 spaces"
        };
        Self { crlf, indent }
    }
}

impl FileEditor {
    /// The status bar's caret position (1-based line and column).
    pub fn caret(&self, cx: &gpui_kit::App) -> (u32, u32) {
        let position = self.state.read(cx).cursor_position();
        (position.line + 1, position.character + 1)
    }

    pub fn format(&self) -> &TextFormat {
        &self.format
    }

    pub fn input(&self) -> &Entity<EditorState> {
        &self.state
    }

    pub fn new(repository: Repository, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (content, error) = match &revision {
            Some(rev) => match repository.run(["show", &format!("{rev}:{path}")]) {
                Ok(text) => (text, None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
            None => match std::fs::read(repository.root().join(&path)) {
                Ok(bytes) if bytes.contains(&0) => (String::new(), Some("Binary file".to_owned())),
                Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
        };
        // A library's source (External Libraries) isn't in the repository.
        let external = crate::index::store::ProjectIndex::is_external(&path);
        let base = if revision.is_none() && !external {
            repository.run(["show", &format!("HEAD:{path}")]).unwrap_or_default()
        } else {
            content.clone()
        };
        let language = language_for(&path);
        let initial = content.clone();
        let state = cx.new(|cx| EditorState::new(window, cx).language(language).line_number(true).searchable(true).default_value(initial));
        let subscriptions = vec![cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                // IntelliJ closes the change popup on typing.
                this.popup = None;
                this.update_markers(cx);
                // The annotations follow the edit (new lines: Not Committed Yet).
                if this.annotation.is_some() {
                    this.load_annotations(true, cx);
                }
                if this.is_dirty(cx) {
                    cx.emit(FileEditorEvent::Edited);
                }
                cx.notify();
            }
        })];
        let read_only = revision.is_some() || external;
        let find = cx.new(|cx| crate::ui::find_bar::FindBar::new(state.clone(), read_only, window, cx));
        let mut this = Self {
            find,
            repository,
            path,
            revision,
            state,
            base,
            format: TextFormat::of(&content),
            saved: content,
            hunks: Vec::new(),
            popup: None,
            popup_anchor: Rc::default(),
            popup_focus: cx.focus_handle(),
            error,
            code_index: None,
            annotation: None,
            annotation_task: None,
            hover_card: None,
            hover_task: None,
            _subscriptions: subscriptions,
        };
        this.update_markers(cx);
        this
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Navigation through the project's code index: Ctrl+click, hover,
    /// Go to Declaration and Find Usages.
    pub fn attach_index(&mut self, index: Entity<CodeIndex>, cx: &mut Context<Self>) {
        let last: Rc<RefCell<Vec<Target>>> = Rc::default();
        let provider = Rc::new(IndexProvider { index: index.clone(), path: self.path.clone(), last: last.clone() });
        let editor = cx.entity().downgrade();
        let show: ShowDocumentHandler = Rc::new(move |_, _, cx| {
            let targets = last.borrow().clone();
            editor.update(cx, |_, cx| cx.emit(FileEditorEvent::Navigate(targets))).ok();
            true
        });
        self.state.update(cx, |state, cx| {
            let lsp = state.lsp_mut();
            lsp.definition_provider = Some(provider.clone());
            lsp.hover_provider = Some(provider);
            lsp.show_document = Some(show);
            cx.notify();
        });
        if !self.read_only() {
            let text = self.state.read(cx).value().to_string();
            index.read(cx).warm_up(&self.path, text, cx);
        }
        self.code_index = Some(index);
    }

    /// Moves the cursor to a 0-based line and UTF-16 column, scrolled into view.
    pub fn go_to(&mut self, line: u32, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_cursor_position(lsp_position(line, col), window, cx));
    }

    /// (line, UTF-16 column) of the cursor, for navigation history.
    pub fn cursor(&self, cx: &gpui_kit::App) -> (u32, u32) {
        let p = self.state.read(cx).cursor_position();
        (p.line, p.character)
    }

    fn goto_declaration(&mut self, _: &GotoDeclaration, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        let task = index.read(cx).definitions(self.path.clone(), text.clone(), offset, cx);
        cx.spawn(async move |this, cx| {
            let targets = task.await;
            let event = if targets.is_empty() { FileEditorEvent::NoDeclaration { text, offset } } else { FileEditorEvent::Navigate(targets) };
            this.update(cx, |_, cx| cx.emit(event)).ok();
        })
        .detach();
    }

    fn find_usages(&mut self, _: &FindUsages, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        cx.emit(FileEditorEvent::FindUsages { text, offset });
    }

    pub fn selected_text(&self, cx: &gpui_kit::App) -> Option<String> {
        Some(self.state.read(cx).selected_value().to_string()).filter(|s| !s.is_empty())
    }

    /// What Find starts with: the selection, else the word at the caret.
    pub fn search_text(&self, cx: &gpui_kit::App) -> Option<String> {
        let state = self.state.read(cx);
        let selected = state.selected_value().to_string();
        if !selected.is_empty() {
            return Some(selected);
        }
        let text = state.value().to_string();
        crate::index::nav::word_at(&text, state.cursor()).map(|(w, _)| w)
    }

    fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        self.step_search(true, window, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.step_search(false, window, cx);
    }

    /// F3 / Shift+F3: the next match of the last search, or of the word at the caret.
    fn step_search(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.read(cx).query_text(cx).is_empty() || !self.find.read(cx).open {
            let word = if self.find.read(cx).query_text(cx).is_empty() { self.search_text(cx) } else { None };
            self.find.update(cx, |f, cx| f.show(false, word, window, cx));
            self.focus(window, cx);
        }
        self.find.update(cx, |f, cx| f.step(forward, cx));
    }

    fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.state.read(cx).selected_value().to_string();
        self.find.update(cx, |f, cx| f.show(replace, Some(selected), window, cx));
    }

    /// Go to Line:Column (Ctrl+G), 1-based.
    pub fn line_count(&self, cx: &gpui_kit::App) -> usize {
        // The line after a final newline counts, as the editor shows it.
        self.state.read(cx).value().split('\n').count()
    }

    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).value().to_string()
    }

    /// Focuses the text, as switching to the tab does.
    pub fn focus(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        let handle = gpui_kit::Focusable::focus_handle(self.state.read(cx), cx);
        window.focus(&handle, cx);
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        !self.read_only() && self.text(cx) != self.saved
    }

    /// A revision, or a library's source: neither is edited.
    pub fn read_only(&self) -> bool {
        self.revision.is_some() || crate::index::store::ProjectIndex::is_external(&self.path)
    }

    /// "< JDK 21 > › java/util/List.java" for a library file.
    fn library_title(&self, cx: &gpui_kit::App) -> Option<String> {
        if !crate::index::store::ProjectIndex::is_external(&self.path) {
            return None;
        }
        let index = self.code_index.as_ref()?.read(cx).index.read().ok()?;
        let library = index.external.library_of(&self.path)?;
        let rel = std::path::Path::new(&self.path).strip_prefix(&library.root).ok()?.to_string_lossy().replace('\\', "/");
        Some(format!("{} › {rel}", library.name))
    }

    /// IntelliJ's change markers (painted in the gutter by `marker_strip`).
    fn update_markers(&mut self, cx: &mut Context<Self>) {
        if self.read_only() || self.error.is_some() {
            return;
        }
        let text = self.text(cx);
        self.hunks = diff::commit_hunks(&self.base, &text);
        if self.popup.is_some_and(|ix| ix >= self.hunks.len()) {
            self.popup = None;
        }
    }

    /// The gutter's change markers: added lines a green bar, modified a blue
    /// one, deleted lines a grey wedge between the lines around them. A click
    /// opens the change popup. Painted after the editor, from its layout.
    fn marker_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.state.clone();
        let hunks = self.hunks.clone();
        let this = cx.entity().downgrade();
        let anchor = self.popup_anchor.clone();
        let popup = self.popup;
        let palette = cx.palette().clone();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = state.read(cx);
                let Some(line_height) = state.line_height() else { return };
                let Some(visible) = state.visible_row_range() else { return };
                let rope = state.text();
                let lines = rope.lines_len().max(1);
                let line_top = |line: usize| {
                    let at = rope.line_start_offset(line.min(lines - 1));
                    state.range_to_bounds(&(at..at)).map(|b| (b.left(), b.top()))
                };
                let mut marks: Vec<(Bounds<Pixels>, usize)> = Vec::new();
                for (ix, hunk) in hunks.iter().enumerate() {
                    let (color, rect) = if hunk.new.is_empty() {
                        // The wedge sits on the boundary above line `start`.
                        let line = hunk.new.start;
                        if line + 1 < visible.start || line > visible.end + 1 {
                            continue;
                        }
                        let Some((left, top)) = (if line >= lines { line_top(lines - 1).map(|(l, t)| (l, t + line_height)) } else { line_top(line) })
                        else {
                            continue;
                        };
                        let x = left - px(MARKER_GAP + MARKER_WIDTH + 2.);
                        (palette.text_secondary, Bounds::new(point(x, top - px(3.)), size(px(MARKER_WIDTH + 4.), px(6.))))
                    } else {
                        let first = hunk.new.start.max(visible.start);
                        let last = (hunk.new.end - 1).min(visible.end.saturating_sub(1)).min(lines - 1);
                        if first > last {
                            continue;
                        }
                        let (Some((left, top)), Some((_, bottom))) = (line_top(first), line_top(last)) else { continue };
                        let x = left - px(MARKER_GAP + MARKER_WIDTH);
                        let color = if hunk.old.is_empty() { palette.status_added } else { palette.status_modified };
                        (color, Bounds::new(point(x, top), size(px(MARKER_WIDTH), bottom + line_height - top)))
                    };
                    let rect = rect.intersect(&bounds);
                    if rect.size.height <= px(0.) {
                        continue;
                    }
                    window.paint_quad(fill(rect, color));
                    marks.push((rect, ix));
                }
                // The popup hangs under its block's last visible line.
                let placed = popup.and_then(|ix| marks.iter().find(|(_, i)| *i == ix)).map(|(r, _)| point(r.left(), r.bottom() + px(2.)));
                if placed != anchor.get() {
                    anchor.set(placed);
                    window.refresh();
                }
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                        return;
                    }
                    // A little slack each side: the bar itself is thin.
                    let hit = marks.iter().find(|(r, _)| {
                        let wide = Bounds::new(point(r.left() - px(4.), r.top()), size(r.size.width + px(8.), r.size.height.max(px(6.))));
                        wide.contains(&e.position)
                    });
                    if let Some(&(_, ix)) = hit {
                        cx.stop_propagation();
                        this.update(cx, |this, cx| this.toggle_popup(ix, window, cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }

    fn toggle_popup(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = if self.popup == Some(ix) { None } else { Some(ix) };
        if self.popup.is_some() {
            window.focus(&self.popup_focus, cx);
        }
        cx.notify();
    }

    fn close_popup(&mut self, _: &CloseChangePopup, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = None;
        self.focus(window, cx);
        cx.notify();
    }

    /// Previous / Next Change from the popup: moves the caret there and
    /// shows that block's popup, wrapping around as IntelliJ does.
    /// Next / Previous Change: the caret goes to the next changed block,
    /// wrapping around, as IntelliJ's gutter navigation does.
    fn go_to_change(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.hunks.is_empty() {
            return;
        }
        let line = self.state.read(cx).cursor_position().line as usize;
        let starts: Vec<usize> = self.hunks.iter().map(|h| h.new.start).collect();
        let target = if forward {
            starts.iter().copied().find(|&s| s > line).unwrap_or(starts[0])
        } else {
            starts.iter().rev().copied().find(|&s| s < line).unwrap_or(*starts.last().unwrap())
        };
        self.state.update(cx, |state, cx| {
            state.set_cursor_position(gpui_kit::component::input::Position::new(target as u32, 0), window, cx)
        });
        cx.notify();
    }

    fn step_popup(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ix), count) = (self.popup, self.hunks.len()) else { return };
        if count == 0 {
            return;
        }
        let next = if forward { (ix + 1) % count } else { (ix + count - 1) % count };
        let line = self.hunks[next].new.start as u32;
        self.state.update(cx, |state, cx| {
            state.set_cursor_position(gpui_kit::component::input::Position::new(line, 0), window, cx)
        });
        self.popup = Some(next);
        window.focus(&self.popup_focus, cx);
        cx.notify();
    }

    /// Rollback from the popup: the block goes back to HEAD, as one undoable edit.
    fn rollback_hunk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk) = self.hunks.get(ix).cloned() else { return };
        let text = self.text(cx);
        let content = diff::splice_hunk(&self.base, &text, &hunk, false);
        // Only the block's own bytes change; the rest of the text keeps its
        // offsets, so the edit is local and undo restores it.
        let start = line_start(&text, hunk.new.start);
        let tail = text.len() - line_start(&text, hunk.new.end).max(start);
        let replacement = content[start.min(content.len())..content.len().saturating_sub(tail).max(start.min(content.len()))].to_owned();
        let end = text.len() - tail;
        self.state.update(cx, |state, cx| {
            state.set_selected_range(start..end, cx);
            state.replace(replacement, window, cx);
        });
        self.popup = None;
        self.update_markers(cx);
        // As Rollback Lines does: the Commit window sees it at once.
        self.save_now(cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// Stage from the popup: the block's lines go into the index.
    fn stage_hunk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk) = self.hunks.get(ix).cloned() else { return };
        // Git stages what's on disk.
        self.save_now(cx);
        let text = self.text(cx);
        match diff::stage_lines(&self.repository, &self.path, &text, hunk.new.clone()) {
            Ok(()) => {
                self.popup = None;
                cx.emit(FileEditorEvent::FilesChanged);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        self.focus(window, cx);
        cx.notify();
    }

    /// The change popup: IntelliJ's toolbar over HEAD's version of the block.
    fn render_popup(&self, ix: usize, at: Point<Pixels>, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let hunk = self.hunks[ix].clone();
        let old: Vec<String> = self.base.lines().skip(hunk.old.start).take(hunk.old.len()).map(str::to_owned).collect();
        let count = self.hunks.len();
        let shown = old.len().min(30);
        let more = old.len() - shown;
        let toolbar = h_flex()
            .gap_0p5()
            .px_1()
            .py_0p5()
            .child(tool_button("change-prev", IconName::ArrowUp, "Previous Change").on_click(cx.listener(|this, _, window, cx| this.step_popup(false, window, cx))))
            .child(tool_button("change-next", IconName::ArrowDown, "Next Change").on_click(cx.listener(|this, _, window, cx| this.step_popup(true, window, cx))))
            .child(tool_button("change-rollback", IconName::Undo2, "Rollback  Ctrl+Alt+Z").on_click(cx.listener(move |this, _, window, cx| this.rollback_hunk(ix, window, cx))))
            .child(tool_button("change-diff", IconName::FileDiff, "Show Diff").on_click(cx.listener(|this, _, window, cx| {
                this.popup = None;
                this.show_diff(&ShowFileDiff, window, cx);
            })))
            .child(tool_button("change-copy", IconName::Copy, "Copy").on_click({
                let old = old.join("\n");
                cx.listener(move |this, _, window, cx| {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(old.clone()));
                    this.popup = None;
                    this.focus(window, cx);
                    cx.notify();
                })
            }))
            .child(tool_button("change-stage", IconName::Plus, "Stage").on_click(cx.listener(move |this, _, window, cx| this.stage_hunk(ix, window, cx))))
            .child(div().px_1().text_xs().text_color(palette.text_secondary).child(format!("{} of {count}", ix + 1)));
        let body = (!old.is_empty()).then(|| {
            v_flex()
                .id("change-popup-old")
                .max_h(px(360.))
                .overflow_y_scroll()
                .border_t_1()
                .border_color(palette.border)
                .bg(palette.diff_deleted)
                .px_2()
                .py_1()
                .font_family(gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone())
                .text_xs()
                .children(old.into_iter().take(shown).map(|line| div().whitespace_nowrap().child(if line.is_empty() { " ".to_owned() } else { line })))
                .when(more > 0, |el| el.child(div().text_color(palette.text_secondary).child(format!("… {more} more lines"))))
        });
        deferred(
            anchored().position(at).snap_to_window_with_margin(px(8.)).child(
                v_flex()
                    .id("change-popup")
                    .key_context(POPUP_CONTEXT)
                    .track_focus(&self.popup_focus)
                    .on_action(cx.listener(Self::close_popup))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.popup = None;
                        cx.notify();
                    }))
                    .min_w(px(220.))
                    .max_w(px(720.))
                    .bg(gpui_kit::component::ActiveTheme::theme(&**cx).popover)
                    .border_1()
                    .border_color(palette.border)
                    .rounded_md()
                    .shadow_lg()
                    .overflow_hidden()
                    .child(toolbar)
                    .children(body),
            ),
        )
        .with_priority(1)
    }

    /// The selected lines (1-based, inclusive), or the cursor's line.
    fn selected_lines(&self, cx: &gpui_kit::App) -> (usize, usize) {
        let text = self.text(cx);
        let range: Range<usize> = self.state.read(cx).selected_range();
        let start = line_of(&text, range.start) + 1;
        let mut end = line_of(&text, range.end) + 1;
        // A selection ending at a line start doesn't include that line.
        if range.end > range.start && line_start(&text, end - 1) == range.end {
            end -= 1;
        }
        (start, end.max(start))
    }

    fn save(&mut self, _: &SaveFile, _: &mut Window, cx: &mut Context<Self>) {
        self.save_now(cx);
    }

    /// Saves unsaved text, as IntelliJ does when a tab closes.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        match std::fs::write(self.repository.root().join(&self.path), &text) {
            Ok(()) => {
                self.format = TextFormat::of(&text);
                self.saved = text;
                cx.emit(FileEditorEvent::FilesChanged);
                cx.emit(FileEditorEvent::Saved(self.path.clone()));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// Git › Open on GitHub: the file at this revision (HEAD's commit for the
    /// working tree), with the selected lines highlighted.
    fn open_on_hosting(&mut self, _: &OpenOnHosting, _: &mut Window, cx: &mut Context<Self>) {
        let Some(web) = crate::git::hosting::web_repo(&self.repository) else {
            self.error = Some("No GitHub, GitLab or Bitbucket remote found".into());
            cx.notify();
            return;
        };
        let revision = match &self.revision {
            Some(revision) => revision.clone(),
            None => self.repository.run(["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_else(|_| "HEAD".into()),
        };
        cx.open_url(&web.file_url(&revision, &self.path, Some(self.selected_lines(cx))));
    }

    /// Create Gist: the selection, or the whole file when nothing is selected.
    fn create_gist(&mut self, _: &CreateGist, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let range: Range<usize> = self.state.read(cx).selected_range();
        let content = if range.is_empty() { text } else { text.get(range).unwrap_or_default().to_owned() };
        let name = self.path.rsplit('/').next().unwrap_or(&self.path).to_owned();
        cx.emit(FileEditorEvent::CreateGist { name, content });
    }

    fn rollback_lines(&mut self, _: &RollbackLines, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        let (start, end) = self.selected_lines(cx);
        let (start, end) = (start - 1, end);
        // Every change block the selection touches goes back to HEAD, bottom-up.
        let touched: Vec<Hunk> = self
            .hunks
            .iter()
            .filter(|h| if h.new.is_empty() { h.new.start >= start && h.new.start <= end } else { h.new.start < end && h.new.end > start })
            .cloned()
            .collect();
        if touched.is_empty() {
            return;
        }
        let mut content = text;
        for hunk in touched.iter().rev() {
            content = diff::splice_hunk(&self.base, &content, hunk, false);
        }
        self.state.update(cx, |state, cx| state.set_value(content, window, cx));
        // set_value doesn't report a change; refresh the markers ourselves.
        self.update_markers(cx);
        self.save(&SaveFile, window, cx);
    }

    fn show_selection_history(&mut self, _: &ShowSelectionHistory, _: &mut Window, cx: &mut Context<Self>) {
        let lines = self.selected_lines(cx);
        cx.emit(FileEditorEvent::SelectionHistory { path: self.path.clone(), lines });
    }

    /// Annotate with Git Blame toggles the gutter annotations, as in IntelliJ.
    fn annotate(&mut self, _: &AnnotateFile, _: &mut Window, cx: &mut Context<Self>) {
        if self.annotation.is_some() {
            self.annotation_action(AnnotationAction::Close, cx);
        } else {
            self.show_annotations(cx);
        }
    }

    pub fn annotation(&self) -> Option<&Annotation> {
        self.annotation.as_ref()
    }

    /// Shows the blame annotations in the gutter (no-op when shown).
    pub fn show_annotations(&mut self, cx: &mut Context<Self>) {
        if self.annotation.is_none() && self.error.is_none() {
            self.annotation = Some(Annotation::loading());
            self.load_annotations(false, cx);
        }
    }

    /// Blames the text shown: the revision, or the editor's own (possibly
    /// unsaved) text of a working-tree file. `debounce` waits for typing to pause.
    fn load_annotations(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let repository = self.repository.clone();
        let path = self.path.clone();
        let revision = self.revision.clone();
        let text = self.text(cx);
        self.annotation_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            }
            let result = cx
                .background_spawn(async move {
                    match &revision {
                        Some(rev) => crate::git::blame::blame(&repository, &path, Some(rev)),
                        None => crate::git::blame::blame_contents(&repository, &path, &text),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(annotation) = this.annotation.as_mut() else { return };
                match result {
                    Ok(blame) => annotation.set_blame(blame),
                    Err(error) => {
                        annotation.loading = false;
                        annotation.error = Some(error.to_string().lines().last().unwrap_or_default().to_owned());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Runs what the annotation gutter asked for.
    pub fn annotation_action(&mut self, action: AnnotationAction, cx: &mut Context<Self>) {
        match action {
            AnnotationAction::Annotate { hash, path } => cx.emit(FileEditorEvent::Annotate { path, revision: Some(hash) }),
            AnnotationAction::SelectInLog(hash) => cx.emit(FileEditorEvent::SelectCommit(hash)),
            AnnotationAction::ShowDiff { hash, path } => cx.emit(FileEditorEvent::OpenDiff(DiffSource::Commit { hash, path, old_path: None })),
            AnnotationAction::SetView(view) => {
                if let Some(annotation) = self.annotation.as_mut() {
                    annotation.view = view;
                }
            }
            AnnotationAction::Close => {
                self.annotation = None;
                self.annotation_task = None;
                self.hover_card = None;
                self.hover_task = None;
            }
            AnnotationAction::HideCard => {
                self.hover_card = None;
                self.hover_task = None;
            }
            AnnotationAction::Hover => {
                // IntelliJ's tooltip waits for the mouse to rest.
                self.hover_card = None;
                let hovered = self.annotation.as_ref().and_then(|a| a.hovered.get());
                self.hover_task = hovered.map(|(line, at)| {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
                        this.update(cx, |this, cx| {
                            let still = this.annotation.as_ref().and_then(|a| a.hovered.get()).map(|(l, _)| l);
                            if still == Some(line) {
                                this.hover_card = Some((line, at));
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                });
            }
        }
        cx.notify();
    }

    fn show_history(&mut self, _: &ShowFileHistory, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FileEditorEvent::ShowHistory(self.path.clone()));
    }

    /// Show Current Revision: selects the last commit that changed the file.
    fn show_current_revision(&mut self, _: &ShowCurrentRevision, _: &mut Window, cx: &mut Context<Self>) {
        let rev = self.revision.clone().unwrap_or_else(|| "HEAD".into());
        match self.repository.run(["log", "-1", "--format=%H", &rev, "--", &self.path]) {
            Ok(hash) if !hash.trim().is_empty() => cx.emit(FileEditorEvent::SelectCommit(hash.trim().to_owned())),
            _ => self.error = Some("The file isn't in any commit yet".into()),
        }
        cx.notify();
    }

    /// Compare with Clipboard: the clipboard against the file (saved first),
    /// which stays editable in the diff.
    fn compare_with_clipboard(&mut self, _: &CompareWithClipboard, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        self.save(&SaveFile, window, cx);
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        cx.emit(FileEditorEvent::OpenDiff(DiffSource::Clipboard { path: self.path.clone(), text }));
    }

    fn show_diff(&mut self, _: &ShowFileDiff, _: &mut Window, cx: &mut Context<Self>) {
        let source = match &self.revision {
            Some(rev) => DiffSource::Commit { hash: rev.clone(), path: self.path.clone(), old_path: None },
            None => DiffSource::WorkingTree { path: self.path.clone(), unversioned: false },
        };
        cx.emit(FileEditorEvent::OpenDiff(source));
    }
}

impl Render for FileEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let dirty = self.is_dirty(cx);
        let read_only = self.read_only();
        let title = match &self.revision {
            Some(rev) => format!("{} ({})", self.path, &rev[..rev.len().min(8)]),
            None => self.library_title(cx).unwrap_or_else(|| self.path.clone()),
        };
        let changes = self.hunks.len();
        let annotated = self.annotation.is_some();
        let annotation_gutter = self
            .annotation
            .as_ref()
            .filter(|a| !a.loading || !a.blame.lines.is_empty())
            .map(|a| a.gutter(self.state.clone(), cx.entity().downgrade(), window, cx).into_any_element());
        let hover_card = self
            .hover_card
            .and_then(|(line, at)| Some((at, self.annotation.as_ref()?.hover_card(line, cx)?.into_any_element())));
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(|this, _: &NextChange, window, cx| this.go_to_change(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousChange, window, cx| this.go_to_change(false, window, cx)))
            .on_action(cx.listener(Self::rollback_lines))
            .on_action(cx.listener(Self::open_on_hosting))
            .on_action(cx.listener(Self::create_gist))
            .on_action(cx.listener(Self::goto_declaration))
            .on_action(cx.listener(Self::find_usages))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(|this, _: &OpenFind, window, cx| this.open_find(false, window, cx)))
            .on_action(cx.listener(|this, _: &OpenReplace, window, cx| this.open_find(true, window, cx)))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::show_selection_history))
            .on_action(cx.listener(Self::annotate))
            .on_action(cx.listener(Self::show_history))
            .on_action(cx.listener(Self::show_current_revision))
            .on_action(cx.listener(Self::show_diff))
            .on_action(cx.listener(Self::compare_with_clipboard))
            .child(
                h_flex()
                    .h(px(crate::ui::common::header_height()))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_sm()
                    .child(common::icon(common::file_icon(&self.path)))
                    .child(div().child(title))
                    .when(dirty, |el| el.child(div().text_color(palette.text_secondary).child("•")))
                    .when(read_only, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("read-only")))
                    .when(!read_only && changes > 0, |el| {
                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!(
                            "{changes} change{}",
                            if changes == 1 { "" } else { "s" }
                        )))
                    })
                    .child(div().flex_1())
                    .when(!read_only, |el| {
                        el.child(tool_button("editor-save", IconName::Check, "Save  Ctrl+S").on_click(cx.listener(
                            |this, _, window, cx| this.save(&SaveFile, window, cx),
                        )))
                    })
                    .child(tool_button("editor-diff", IconName::FileDiff, "Show Diff").on_click(cx.listener(
                        |this, _, window, cx| this.show_diff(&ShowFileDiff, window, cx),
                    )))
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(error))
            })
            .child(self.find.clone())
            .when_some(self.annotation.as_ref().and_then(|a| a.error.clone()), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(format!("Cannot annotate: {error}")))
            })
            .child(
                div().relative().flex_1().min_h_0().child(
                    h_flex().size_full().children(annotation_gutter).child(div().flex_1().min_w_0().h_full().child(
                    Editor::new(&self.state)
                        .h_full()
                        .bordered(false)
                        .readonly(read_only)
                        .context_menu(move |menu: NativeMenu, _, _| {
                            let menu = menu
                                .menu("Copy", Box::new(Copy))
                                .menu_with_disabled("Cut", read_only, Box::new(Cut))
                                .menu_with_disabled("Paste", read_only, Box::new(Paste))
                                .menu("Select All", Box::new(SelectAll))
                                .separator()
                                .menu("Go to Declaration", Box::new(GotoDeclaration))
                                .menu("Find Usages", Box::new(FindUsages))
                                .menu("Select in Project View", Box::new(crate::ui::workspace::SelectInProject))
                                .separator()
                                .menu_with_disabled("Compare with Clipboard", read_only, Box::new(CompareWithClipboard))
                                .separator();
                            let git = NativeMenu::new()
                                .menu("Show History for Selection", Box::new(ShowSelectionHistory))
                                .menu(if annotated { "Close Annotations" } else { "Annotate with Git Blame" }, Box::new(AnnotateFile))
                                .menu("Show History", Box::new(ShowFileHistory))
                                .menu("Show Current Revision", Box::new(ShowCurrentRevision))
                                .menu("Show Diff", Box::new(ShowFileDiff))
                                .menu_with_disabled("Rollback Lines", read_only, Box::new(RollbackLines))
                                .separator()
                                .menu("Open on GitHub / GitLab", Box::new(OpenOnHosting))
                                .menu("Create Gist…", Box::new(CreateGist));
                            menu.submenu("Git", git)
                        }),
                )))
                .when(!read_only, |el| el.child(self.marker_strip(cx)))
                .when_some(hover_card, |el, (at, card)| {
                    el.child(deferred(anchored().position(point(at.x + px(12.), at.y + px(16.))).child(card)).with_priority(1))
                })
                .when_some(self.popup.zip(self.popup_anchor.get()), |el, (ix, at)| el.child(self.render_popup(ix, at, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_languages_and_lines() {
        assert_eq!(language_for("app/src/main/java/A.kt"), "kotlin");
        assert_eq!(language_for("CMakeLists.txt"), "cmake");
        assert_eq!(language_for("native/jni.cpp"), "cpp");
        assert_eq!(language_for("README"), "plaintext");
        let text = "a\nbb\nccc\n";
        assert_eq!(line_start(text, 2), 5);
        assert_eq!(line_of(text, 5), 2);
    }
}

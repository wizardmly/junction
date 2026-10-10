//! The file editor: a working-tree file (editable, saved with Ctrl+S) or a
//! file at a revision (read-only), with syntax highlighting and IntelliJ's
//! change markers against HEAD. Its context menu carries the editor's Git
//! actions: Show History for Selection, Annotate, Show Current Revision,
//! Rollback Lines.

use std::cell::RefCell;
use crate::ui::as_icons as icons;
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
        CompareWithClipboard,
        DeleteLine,
        CloseFindBar,
        ToggleLineNumbers
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
        // Esc in the text closes the find bar too, once the text field has
        // passed on it (no menu, extra cursors or completion to dismiss).
        KeyBinding::new("escape", CloseFindBar, Some(CONTEXT)),
        // IntelliJ: Next / Previous Occurrence of the Find window's results.
        KeyBinding::new("secondary-alt-down", crate::ui::workspace::NextOccurrence, Some("FileEditor > Input")),
        KeyBinding::new("secondary-alt-up", crate::ui::workspace::PreviousOccurrence, Some("FileEditor > Input")),
        // IntelliJ's keymap: Ctrl+Shift+Z redoes, Ctrl+Y deletes the line
        // (over the text field's Windows-style Ctrl+Y redo).
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-z", gpui_kit::component::input::Redo, Some("FileEditor > Input")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", DeleteLine, Some("FileEditor > Input")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-backspace", DeleteLine, Some("FileEditor > Input")),
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
    /// The file is gone from disk (a checkout, say) and had no unsaved edits.
    Deleted,
}

impl EventEmitter<FileEditorEvent> for FileEditor {}

/// Highlighting the toolkit lacks or Android Studio colors differently:
/// C, C++, ObjC, Java and Rust built-in types are keywords; C++'s and TSX's
/// queries are only additions to C's and TypeScript's; Swift has none; Dart,
/// ObjC, XML, Groovy (Gradle), .properties and V aren't built in; CMake,
/// proto and C# come without queries. Ours (Groovy, proto, V) are in
/// assets/highlights.
fn register_grammars() {
    use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
    let registry = LanguageRegistry::singleton();
    for (name, language, highlights) in grammars() {
        // Rust's macro bodies are Rust again.
        let (injection_languages, injections) = match name {
            "rust" => (vec!["rust".into()], include_str!("../../vendor/gpui-component/src/highlighter/languages/rust/injections.scm")),
            _ => (Vec::new(), ""),
        };
        registry.register(name, &LanguageConfig::new(name, language, injection_languages, &highlights, injections, ""));
    }
}

/// Android Studio colors built-in types (`int`, `i32`) as keywords; the
/// first pattern matching a node wins, so these go first.
const C_BUILTINS: &str = "(primitive_type) @keyword\n(sized_type_specifier) @keyword\n";

fn grammars() -> Vec<(&'static str, tree_sitter::Language, String)> {
    vec![
        ("c", tree_sitter_c::LANGUAGE.into(), format!("{C_BUILTINS}{}", tree_sitter_c::HIGHLIGHT_QUERY)),
        ("cpp", tree_sitter_cpp::LANGUAGE.into(), format!("{C_BUILTINS}{}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY)),
        (
            "java",
            tree_sitter_java::LANGUAGE.into(),
            format!("[(integral_type) (floating_point_type) (boolean_type) (void_type)] @keyword\n{}", tree_sitter_java::HIGHLIGHTS_QUERY),
        ),
        (
            "rust",
            tree_sitter_rust::LANGUAGE.into(),
            format!("(primitive_type) @keyword\n[(integer_literal) (float_literal)] @number\n{}", include_str!("../../vendor/gpui-component/src/highlighter/languages/rust/highlights.scm")),
        ),
        ("swift", tree_sitter_swift::LANGUAGE.into(), tree_sitter_swift::HIGHLIGHTS_QUERY.to_owned()),
        ("dart", tree_sitter_dart::LANGUAGE.into(), tree_sitter_dart::HIGHLIGHTS_QUERY.to_owned()),
        // TSX's query is only TypeScript's additions, like C++'s.
        (
            "tsx",
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            format!("{}\n{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY),
        ),
        ("objc", tree_sitter_objc::LANGUAGE.into(), format!("{C_BUILTINS}{}\n{}", tree_sitter_objc::HIGHLIGHTS_QUERY, tree_sitter_c::HIGHLIGHT_QUERY)),
        ("xml", tree_sitter_xml::LANGUAGE_XML.into(), tree_sitter_xml::XML_HIGHLIGHT_QUERY.to_owned()),
        ("dtd", tree_sitter_xml::LANGUAGE_DTD.into(), tree_sitter_xml::DTD_HIGHLIGHT_QUERY.to_owned()),
        ("properties", tree_sitter_properties::LANGUAGE.into(), tree_sitter_properties::HIGHLIGHTS_QUERY.to_owned()),
        ("groovy", tree_sitter_groovy::LANGUAGE.into(), include_str!("../../assets/highlights/groovy.scm").to_owned()),
        ("cmake", tree_sitter_cmake::LANGUAGE.into(), tree_sitter_cmake::HIGHLIGHTS_QUERY.to_owned()),
        ("proto", tree_sitter_proto::LANGUAGE.into(), include_str!("../../assets/highlights/proto.scm").to_owned()),
        ("csharp", tree_sitter_c_sharp::LANGUAGE.into(), tree_sitter_c_sharp::HIGHLIGHTS_QUERY.to_owned()),
        ("v", tree_sitter_vlang::LANGUAGE.into(), include_str!("../../assets/highlights/v.scm").to_owned()),
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
        (_, "v" | "vsh") => "v",
        (_, "c" | "h") => "c",
        (_, "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "inl" | "tpp") => "cpp",
        (_, "m" | "mm") => "objc",
        (_, "js" | "mjs" | "cjs" | "jsx") => "javascript",
        (_, "ts" | "mts" | "cts") => "typescript",
        (_, "tsx") => "tsx",
        (_, "py" | "pyi") => "python",
        (_, "json" | "jsonc") => "json",
        (_, "toml") => "toml",
        (_, "yml" | "yaml") => "yaml",
        (_, "md" | "markdown") => "markdown",
        (_, "sh" | "bash" | "zsh") | ("gradlew", _) => "bash",
        (_, "proto") => "proto",
        (_, "xml" | "xsd" | "xsl" | "xslt" | "svg" | "plist" | "iml" | "xib" | "storyboard" | "pom" | "kml") => "xml",
        (_, "dtd") => "dtd",
        (_, "html" | "htm" | "xhtml") => "html",
        (_, "css") => "css",
        (_, "gradle" | "groovy") => "groovy",
        (_, "properties") => "properties",
        (_, "sql") => "sql",
        (_, "lua") => "lua",
        (_, "rb") | ("gemfile" | "rakefile" | "podfile", _) => "ruby",
        (_, "php") => "php",
        (_, "cs") => "csharp",
        (_, "scala" | "sc") => "scala",
        (_, "diff" | "patch") => "diff",
        _ => "plaintext",
    }
}

/// An image opened in the editor.
struct ImageFile {
    image: std::sync::Arc<gpui_kit::Image>,
    format: gpui_kit::ImageFormat,
    width: u32,
    height: u32,
    size: usize,
    /// The viewer's size at the last paint, for Fit Zoom to Window.
    viewport: Rc<std::cell::Cell<gpui_kit::Size<Pixels>>>,
}

impl ImageFile {
    fn new(format: gpui_kit::ImageFormat, bytes: Vec<u8>) -> Self {
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .unwrap_or((0, 0));
        let size = bytes.len();
        Self { image: std::sync::Arc::new(gpui_kit::Image::from_bytes(format, bytes)), format, width, height, size, viewport: Rc::default() }
    }

    /// "64x64 PNG 1.2 kB", as IntelliJ's image editor shows it.
    fn info(&self) -> String {
        let name = format!("{:?}", self.format).to_uppercase();
        let size = if self.size < 1024 { format!("{} B", self.size) } else if self.size < 1024 * 1024 { format!("{:.1} kB", self.size as f64 / 1024.) } else { format!("{:.1} MB", self.size as f64 / (1024. * 1024.)) };
        format!("{}x{} {name} {size}", self.width, self.height)
    }
}

/// The image formats the editor shows as pictures. SVG stays text (XML).
fn image_format(path: &str) -> Option<gpui_kit::ImageFormat> {
    use gpui_kit::ImageFormat::*;
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => Png,
        "jpg" | "jpeg" => Jpeg,
        "gif" => Gif,
        "webp" => Webp,
        "bmp" => Bmp,
        "ico" => Ico,
        "tif" | "tiff" => Tiff,
        _ => return None,
    })
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
    /// Not under version control (neither in HEAD nor in the index): no
    /// change markers, as IntelliJ shows none for unversioned files.
    unversioned: bool,
    saved: String,
    hunks: Vec<Hunk>,
    /// The change block whose popup is open (a click on its gutter marker).
    popup: Option<usize>,
    /// Where the popup goes: under its block, as the last paint laid it out.
    popup_anchor: Rc<std::cell::Cell<Option<Point<Pixels>>>>,
    /// Whether the last right-click landed on the line-number gutter, which
    /// has its own menu (Annotate with Git Blame), as in IntelliJ.
    gutter_click: Rc<std::cell::Cell<bool>>,
    popup_focus: gpui_kit::FocusHandle,
    error: Option<String>,
    code_index: Option<Entity<CodeIndex>>,
    /// Annotate with Git Blame, shown in the gutter.
    annotation: Option<Annotation>,
    annotation_task: Option<gpui_kit::Task<()>>,
    /// The annotation hover card: its line and where the mouse rested.
    hover_card: Option<(usize, Point<Pixels>)>,
    hover_task: Option<gpui_kit::Task<()>>,
    markers_task: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
    /// An image file, shown instead of the text (IntelliJ's image viewer).
    image: Option<ImageFile>,
    /// The image's zoom; `None` fits it to the window (shrinking only).
    zoom: Option<f32>,
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
        let image = image_format(&path).map(|format| {
            let bytes = match &revision {
                Some(rev) => repository.run_bytes(["show", &format!("{rev}:{path}")]).map_err(|e| e.to_string()),
                None => std::fs::read(repository.root().join(&path)).map_err(|e| e.to_string()),
            };
            bytes.map(|bytes| ImageFile::new(format, bytes))
        });
        let (image, image_error) = match image {
            Some(Ok(image)) => (Some(image), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let (content, error) = if image_error.is_some() || image.is_some() { (String::new(), image_error) } else { match &revision {
            Some(rev) => match repository.run(["show", &format!("{rev}:{path}")]) {
                Ok(text) => (text, None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
            None => match std::fs::read(repository.root().join(&path)) {
                Ok(bytes) if bytes.contains(&0) => (String::new(), Some("Binary file".to_owned())),
                Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
        } };
        // A library's source (External Libraries) isn't in the repository.
        let external = crate::index::store::ProjectIndex::is_external(&path);
        let (base, unversioned) = if revision.is_none() && !external && image.is_none() {
            match repository.run(["show", &format!("HEAD:{path}")]) {
                Ok(base) => (base, false),
                Err(_) => (String::new(), repository.run(["ls-files", "--error-unmatch", "--", &path]).is_err()),
            }
        } else {
            (content.clone(), false)
        };
        let language = language_for(&path);
        let initial = content.clone();
        // Settings › Editor › General: line numbers, whitespaces, indent guides, soft wrap.
        let view = crate::settings::Settings::get(cx).diff.clone();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(view.show_line_numbers)
                .show_whitespaces(view.show_whitespaces)
                .indent_guides(view.show_indent_guides)
                .soft_wrap(view.soft_wrap)
                .searchable(true)
                .default_value(initial)
        });
        let settings_state = state.clone();
        let mut shown = view;
        let settings = cx.observe_global_in::<crate::settings::Settings>(window, move |_, window, cx| {
            let view = crate::settings::Settings::get(cx).diff.clone();
            if view == shown {
                return;
            }
            settings_state.update(cx, |state, cx| {
                state.set_line_number(view.show_line_numbers, window, cx);
                state.set_show_whitespaces(view.show_whitespaces, window, cx);
                state.set_indent_guides(view.show_indent_guides, window, cx);
                state.set_soft_wrap(view.soft_wrap, window, cx);
            });
            shown = view;
        });
        let subscriptions = vec![settings, cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
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
            unversioned,
            format: TextFormat::of(&content),
            saved: content,
            hunks: Vec::new(),
            popup: None,
            popup_anchor: Rc::default(),
            gutter_click: Rc::default(),
            popup_focus: cx.focus_handle(),
            error,
            code_index: None,
            annotation: None,
            annotation_task: None,
            hover_card: None,
            hover_task: None,
            markers_task: None,
            image,
            zoom: None,
            _subscriptions: subscriptions,
        };
        this.update_markers(cx);
        this
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The zoom that fits the image in the last laid-out viewer, 1 at most.
    fn fit_zoom(&self) -> f32 {
        let Some(image) = &self.image else { return 1. };
        let bounds = image.viewport.get();
        if image.width == 0 || image.height == 0 || bounds.width <= px(0.) {
            return 1.;
        }
        let margin = 32.;
        let fit = ((f32::from(bounds.width) - margin) / image.width as f32).min((f32::from(bounds.height) - margin) / image.height as f32);
        fit.clamp(1. / 32., 1.)
    }

    /// The image, centered at its zoom, scrollable when larger than the view.
    fn render_image(&self, image: &ImageFile, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let zoom = self.zoom.unwrap_or_else(|| self.fit_zoom());
        let (w, h) = (image.width as f32 * zoom, image.height as f32 * zoom);
        let viewport = image.viewport.clone();
        let entity = cx.entity().downgrade();
        div()
            .id("image-viewer")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .bg(gpui_kit::component::ActiveTheme::theme(&**cx).highlight_theme.style.editor_background.unwrap_or(palette.panel))
            .child(
                gpui_kit::canvas(
                    move |bounds, _, cx| {
                        // A resize changes the fit: draw again with the new size.
                        if viewport.replace(bounds.size) != bounds.size {
                            entity.update(cx, |_, cx| cx.notify()).ok();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                h_flex().min_w_full().min_h_full().justify_center().items_center().p_4().child(
                    gpui_kit::img(image.image.clone())
                        .w(px(w))
                        .h(px(h))
                        .flex_none()
                        .border_1()
                        .border_color(palette.border),
                ),
            )
            .into_any_element()
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

    /// Moves the cursor to a 0-based line and UTF-16 column. A line out of
    /// view scrolls to the middle of the editor, as IntelliJ's navigation.
    pub fn go_to(&mut self, line: u32, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_cursor_position(lsp_position(line, col), window, cx));
        self.center_line(line as usize, 0, window, cx);
    }

    fn center_line(&mut self, line: usize, attempt: usize, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let (Some(range), Some(height)) = (state.visible_row_range(), state.line_height()) else {
            // A file just opened isn't laid out yet: after its first frame.
            if attempt < 4 {
                cx.on_next_frame(window, move |this, window, cx| this.center_line(line, attempt + 1, window, cx));
            }
            return;
        };
        // The range has a row of overscan past the bottom edge.
        let rows = range.len().saturating_sub(2).max(1);
        if line >= range.start + 1 && line + 3 <= range.end || line < rows / 2 && range.start == 0 {
            return;
        }
        let top = line.saturating_sub(rows / 2);
        let x = state.scroll_offset().x;
        self.state.update(cx, |state, cx| state.set_scroll_offset(gpui_kit::point(x, -(height * top as f32)), cx));
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

    /// After git changed the working tree (checkout, pull, rollback…):
    /// takes the file's new text from disk unless it has unsaved edits, as
    /// IntelliJ reloads its editors, and the new HEAD version for the
    /// change markers.
    pub fn sync_with_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() || self.error.is_some() {
            return;
        }
        let (repository, path) = (self.repository.clone(), self.path.clone());
        let task = cx.background_spawn(async move {
            let disk = std::fs::read(repository.root().join(&path)).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let base = repository.run(["show", &format!("HEAD:{path}")]).ok();
            (disk, base)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (disk, base) = task.await;
            this.update_in(cx, |this, window, cx| {
                if this.is_dirty(cx) {
                    return;
                }
                let Some(disk) = disk else {
                    cx.emit(FileEditorEvent::Deleted);
                    return;
                };
                if disk != this.saved {
                    this.format = TextFormat::of(&disk);
                    this.saved = disk.clone();
                    this.state.update(cx, |state, cx| state.set_value(disk, window, cx));
                }
                this.unversioned = base.is_none();
                this.base = base.unwrap_or_default();
                this.update_markers(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A revision, or a library's source: neither is edited.
    pub fn read_only(&self) -> bool {
        self.revision.is_some() || self.image.is_some() || crate::index::store::ProjectIndex::is_external(&self.path)
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
        if self.read_only() || self.error.is_some() || self.unversioned {
            return;
        }
        // Diffing a long file takes a while: off the main thread, the
        // markers following a moment after the typing.
        let (base, text) = (self.base.clone(), self.text(cx));
        self.markers_task = Some(cx.spawn(async move |this, cx| {
            let hunks = cx.background_spawn(async move { diff::commit_hunks(&base, &text) }).await;
            this.update(cx, |this, cx| {
                this.hunks = hunks;
                if this.popup.is_some_and(|ix| ix >= this.hunks.len()) {
                    this.popup = None;
                }
                cx.notify();
            })
            .ok();
        }));
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
            .child(tool_button("change-prev", icons::UP, "Previous Change").on_click(cx.listener(|this, _, window, cx| this.step_popup(false, window, cx))))
            .child(tool_button("change-next", icons::DOWN, "Next Change").on_click(cx.listener(|this, _, window, cx| this.step_popup(true, window, cx))))
            .child(tool_button("change-rollback", icons::VCS_REVERT, "Rollback  Ctrl+Alt+Z").on_click(cx.listener(move |this, _, window, cx| this.rollback_hunk(ix, window, cx))))
            .child(tool_button("change-diff", icons::VCS_DIFF, "Show Diff").on_click(cx.listener(|this, _, window, cx| {
                this.popup = None;
                this.show_diff(&ShowFileDiff, window, cx);
            })))
            .child(tool_button("change-copy", icons::COPY, "Copy").on_click({
                let old = old.join("\n");
                cx.listener(move |this, _, window, cx| {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(old.clone()));
                    this.popup = None;
                    this.focus(window, cx);
                    cx.notify();
                })
            }))
            .child(tool_button("change-stage", icons::ADD, "Stage").on_click(cx.listener(move |this, _, window, cx| this.stage_hunk(ix, window, cx))))
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
    pub(crate) fn open_on_hosting(&mut self, _: &OpenOnHosting, _: &mut Window, cx: &mut Context<Self>) {
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
    pub(crate) fn create_gist(&mut self, _: &CreateGist, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let range: Range<usize> = self.state.read(cx).selected_range();
        let content = if range.is_empty() { text } else { text.get(range).unwrap_or_default().to_owned() };
        let name = self.path.rsplit('/').next().unwrap_or(&self.path).to_owned();
        cx.emit(FileEditorEvent::CreateGist { name, content });
    }

    /// Delete Line (Ctrl+Y): the lines the selection touches.
    fn delete_line(&mut self, _: &DeleteLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        let (start, end) = self.selected_lines(cx);
        let from = line_start(&text, start - 1);
        let to = line_start(&text, end);
        // The last line takes the newline before it instead.
        let from = if to == text.len() && !text[from..].contains('\n') && from > 0 { from - 1 } else { from };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(from..to, cx);
            state.replace("", window, cx);
        });
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

    /// The gutter menu's Show Line Numbers (Settings › Editor › General).
    fn toggle_line_numbers(&mut self, _: &ToggleLineNumbers, _: &mut Window, cx: &mut Context<Self>) {
        crate::settings::Settings::update(cx, |settings| settings.diff.show_line_numbers = !settings.diff.show_line_numbers);
    }

    /// Tells a right-click on the line-number gutter (left of the text)
    /// from one on the text, for the context menu.
    fn gutter_probe(&self) -> impl IntoElement + use<> {
        let (state, hit) = (self.state.clone(), self.gutter_click.clone());
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (state, hit) = (state.clone(), hit.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Right {
                        return;
                    }
                    let state = state.read(cx);
                    let input = state.input_bounds();
                    let text_left = state.visible_row_range().and_then(|rows| {
                        let rope = state.text();
                        let at = rope.line_start_offset(rows.start.min(rope.lines_len().saturating_sub(1)));
                        state.range_to_bounds(&(at..at)).map(|b| b.left())
                    });
                    hit.set(text_left.is_some_and(|left| {
                        input.contains(&e.position) && e.position.x >= input.left() && e.position.x < left
                    }));
                });
            },
        )
        .absolute()
        .inset_0()
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
        let gutter_click = self.gutter_click.clone();
        let annotation_gutter = self
            .annotation
            .as_ref()
            .filter(|a| !a.loading || !a.blame.lines.is_empty())
            .map(|a| a.gutter(self.state.clone(), cx.entity().downgrade(), window, cx).into_any_element());
        let hover_card = self
            .hover_card
            .and_then(|(line, at)| Some((at, self.annotation.as_ref()?.hover_card(line, cx)?.into_any_element())));
        let has_image = self.image.is_some();
        let image_info = self.image.as_ref().map(ImageFile::info);
        let image_body = self.image.as_ref().map(|image| self.render_image(image, cx));
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(|this, _: &NextChange, window, cx| this.go_to_change(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousChange, window, cx| this.go_to_change(false, window, cx)))
            .on_action(cx.listener(Self::rollback_lines))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(|this, _: &CloseFindBar, window, cx| {
                if this.find.read(cx).open {
                    this.find.update(cx, |f, cx| f.close(window, cx));
                } else {
                    cx.propagate();
                }
            }))
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
            .on_action(cx.listener(Self::toggle_line_numbers))
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
                    .when_some(image_info, |el, info| el.child(div().text_xs().text_color(palette.text_secondary).child(info)))
                    .when(dirty, |el| el.child(div().text_color(palette.text_secondary).child("•")))
                    .when(read_only && !has_image, |el| el.child(div().text_xs().text_color(palette.text_secondary).child("read-only")))
                    .when(!read_only && changes > 0, |el| {
                        el.child(div().text_xs().text_color(palette.text_secondary).child(format!(
                            "{changes} change{}",
                            if changes == 1 { "" } else { "s" }
                        )))
                    })
                    .child(div().flex_1())
                    .when(!read_only, |el| {
                        el.child(tool_button("editor-save", icons::CHECKED, "Save  Ctrl+S").on_click(cx.listener(
                            |this, _, window, cx| this.save(&SaveFile, window, cx),
                        )))
                    })
                    .when(has_image, |el| {
                        // IntelliJ's image editor toolbar.
                        let zoom = |id: &'static str, icon: icons::AsIcon, tooltip: &'static str, to: fn(Option<f32>) -> Option<f32>| {
                            tool_button(id, icon, tooltip).on_click(cx.listener(move |this, _, _, cx| {
                                this.zoom = to(this.zoom.or(Some(this.fit_zoom())));
                                cx.notify();
                            }))
                        };
                        el.child(zoom("image-zoom-in", icons::ZOOM_IN, "Zoom In", |z| z.map(|z| (z * 1.5).min(32.))))
                            .child(zoom("image-zoom-out", icons::ZOOM_OUT, "Zoom Out", |z| z.map(|z| (z / 1.5).max(1. / 32.))))
                            .child(zoom("image-actual", icons::ACTUAL_ZOOM, "Actual Size", |_| Some(1.)))
                            .child(zoom("image-fit", icons::FIT_CONTENT, "Fit Zoom to Window", |_| None).on_click(cx.listener(|this, _, _, cx| {
                                this.zoom = None;
                                cx.notify();
                            })))
                    })
                    .when(!has_image, |el| el.child(tool_button("editor-diff", icons::VCS_DIFF, "Show Diff").on_click(cx.listener(
                        |this, _, window, cx| this.show_diff(&ShowFileDiff, window, cx),
                    ))))
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(error))
            })
            .child(self.find.clone())
            .when_some(self.annotation.as_ref().and_then(|a| a.error.clone()), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(format!("Cannot annotate: {error}")))
            })
            .when_some(image_body, |el, body| el.child(body))
            .when(!has_image, |el| el.child(
                div().relative().flex_1().min_h_0().child(
                    h_flex().size_full().children(annotation_gutter).child(div().flex_1().min_w_0().h_full().text_size(px(crate::ui::diff_panes::font_size())).child(
                    Editor::new(&self.state)
                        .h_full()
                        .bordered(false)
                        .readonly(read_only)
                        .context_menu(move |menu: NativeMenu, _, cx| {
                            // The gutter has IntelliJ's gutter menu instead.
                            if gutter_click.replace(false) {
                                let numbers = crate::settings::Settings::get(cx).diff.show_line_numbers;
                                return menu
                                    .menu(if annotated { "Close Annotations" } else { "Annotate with Git Blame" }, Box::new(AnnotateFile))
                                    .separator()
                                    .menu_with_check("Show Line Numbers", numbers, Box::new(ToggleLineNumbers));
                            }
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
                .child(self.gutter_probe())
                .when(!read_only, |el| el.child(self.marker_strip(cx)))
                .when_some(hover_card, |el, (at, card)| {
                    el.child(deferred(anchored().position(point(at.x + px(12.), at.y + px(16.))).child(card)).with_priority(1))
                })
                .when_some(self.popup.zip(self.popup_anchor.get()), |el, (ix, at)| el.child(self.render_popup(ix, at, cx))),
            ))
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

/// Every language Junction indexes, and the other files it opens, gets
/// keywords (or tags) in the editor's keyword color.
#[cfg(test)]
#[test]
fn every_language_highlights() {
    use gpui_kit::component::highlighter::SyntaxHighlighter;
    register_grammars();
    let theme = crate::theme::editor_colors(true);
    let keyword = theme.style.syntax.keyword.map(gpui_kit::HighlightStyle::from).and_then(|s| s.color);
    let tag = theme.style.syntax.tag.map(gpui_kit::HighlightStyle::from).and_then(|s| s.color);
    let samples = [
        ("a.c", "static int main(void) { return 0; }", "return"),
        ("b.c", "unsigned long x = 0;", "unsigned long"),
        ("c.c", "int main(int argc) { char c; }", "char"),
        ("d.c", "int main(int argc) { char c; }", "int"),
        ("b.cpp", "int x = 0;", "int"),
        ("B.java", "class B { int x; }", "int"),
        ("b.rs", "fn f(x: i32) {}", "i32"),
        ("a.h", "typedef struct point { int x; } point;", "struct"),
        ("a.cpp", "namespace demo { class Foo {}; }", "namespace"),
        ("a.m", "@interface Foo : NSObject\n@end\n", "@interface"),
        ("a.mm", "@implementation Foo\n@end\n", "@implementation"),
        ("a.rs", "pub fn main() { let x = 1; }", "let"),
        ("a.go", "package main\nfunc main() { return }", "func"),
        ("A.java", "public class A { }", "class"),
        ("A.kt", "class A { fun f() = 1 }", "fun"),
        ("a.swift", "func f() -> Int { return 1 }", "func"),
        ("a.dart", "class A { void f() { return; } }", "class"),
        ("a.v", "module main\nfn main() {\n\tmut x := 1\n\treturn\n}\n", "fn"),
        ("a.js", "const x = 1; function f() { return x; }", "function"),
        ("a.ts", "interface A { x: number }", "interface"),
        ("a.tsx", "const a = <div/>; export default a;", "const"),
        ("a.py", "def f():\n    return 1\n", "def"),
        ("build.gradle", "plugins { id 'java' }\nif (true) { def x = 1 }\n", "def"),
        ("a.sh", "if true; then echo hi; fi", "if"),
        ("a.sql", "SELECT a FROM b;", "SELECT"),
        ("a.lua", "local x = 1", "local"),
        ("a.rb", "def f\nend\n", "def"),
        ("a.cs", "class A { }", "class"),
        ("CMakeLists.txt", "if(WIN32)\nendif()\n", "if"),
        ("a.proto", "syntax = \"proto3\";\nmessage A { int32 x = 1; }", "message"),
        ("a.scala", "object A { val x = 1 }", "object"),
        ("a.php", "<?php function f() { return 1; }", "function"),
    ];
    for (name, text, word) in samples {
        let language = language_for(name);
        let rope = gpui_kit::component::Rope::from_str(text);
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.update(None, &rope, None);
        let at = text.find(word).unwrap();
        let styles = highlighter.styles(&(0..text.len()), &theme);
        let color = styles.iter().find(|(r, _)| r.start <= at && at < r.end).and_then(|(_, s)| s.color);
        assert_eq!(color, keyword, "{name} ({language}): {word:?} in {styles:?}");
    }
    for (name, text, word) in [("a.xml", "<manifest package=\"a\"><app/></manifest>", "manifest"), ("a.html", "<div class=\"a\"></div>", "div")] {
        let rope = gpui_kit::component::Rope::from_str(text);
        let mut highlighter = SyntaxHighlighter::new(language_for(name));
        highlighter.update(None, &rope, None);
        let at = text.find(word).unwrap();
        let styles = highlighter.styles(&(0..text.len()), &theme);
        let color = styles.iter().find(|(r, _)| r.start <= at && at < r.end).and_then(|(_, s)| s.color);
        assert!(color.is_some() && (color == tag || color == keyword), "{name}: {styles:?}");
    }
}

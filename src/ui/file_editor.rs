//! The file editor: a working-tree file (editable, saved with Ctrl+S) or a
//! file at a revision (read-only), with syntax highlighting and IntelliJ's
//! change markers against HEAD. Its context menu carries the editor's Git
//! actions: Show History for Selection, Annotate, Show Current Revision,
//! Rollback Lines.

mod breakpoints;
mod coverage;
mod editing;
mod fixes;
mod generate;
mod templates;
pub use breakpoints::RunToCursor;
mod previews;
mod vector_preview;
mod merged_manifest;
mod parameter_info;
mod overrides;
mod problems;
mod test_marks;

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
        ShowLocalSelectionHistory,
        AnnotateFile,
        ShowFileHistory,
        ShowCurrentRevision,
        RollbackLines,
        ShowFileDiff,
        OpenOnHosting,
        CreateGist,
        GotoDeclaration,
        FindUsages,
        RenameElement,
        OptimizeImports,
        ReformatCode,
        ReformatFile,
        ParameterInfo,
        IntroduceVariable,
        IntroduceConstant,
        ExtractMethod,
        IntroduceParameter,
        ChangeSignature,
        CallHierarchy,
        TypeHierarchy,
        MethodHierarchy,
        SurroundWith,
        SurroundWithLiveTemplate,
        Generate,
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
        ToggleLineNumbers,
        ToggleBookmark,
        ToggleBookmarkWithMnemonic,
        ToggleSoftWrap,
        ToggleGutterIcons,
        ToggleIndentGuides,
        ToggleWhitespaces,
        ConfigureSoftWraps,
        ConfigureGutterIcons,
        CopyAbsolutePath,
        CopyPathReference,
        TriggerCompletion
    ]
);

const CONTEXT: &str = "FileEditor";
const POPUP_CONTEXT: &str = "ChangePopup";
/// The change marker strip: its width, and the gap to the text.
const MARKER_WIDTH: f32 = 3.;
const MARKER_GAP: f32 = 4.;
/// IntelliJ's gutter icons are 14px, one per 16px slot.
const GUTTER_ICON: f32 = 14.;
const GUTTER_ICON_SLOT: f32 = 16.;
/// Space between the line numbers and the icons.
const GUTTER_ICON_GAP: f32 = 4.;

pub fn init(cx: &mut gpui_kit::App) {
    register_grammars();
    fixes::init(cx);
    overrides::init(cx);
    test_marks::init(cx);
    breakpoints::init(cx);
    editing::init(cx);
    cx.bind_keys([
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" }, SaveFile, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-z", RollbackLines, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-b" } else { "ctrl-b" }, GotoDeclaration, Some(CONTEXT)),
        // Next / Previous Change: Ctrl+Alt+Shift+Down / Up (⌃⌥⇧↓ / ↑ on macOS).
        KeyBinding::new("ctrl-alt-shift-down", NextChange, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-shift-up", PreviousChange, Some(CONTEXT)),
        KeyBinding::new("alt-f7", FindUsages, Some(CONTEXT)),
        KeyBinding::new("shift-f6", RenameElement, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-o", OptimizeImports, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-l" }, ReformatCode, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-shift-l" } else { "ctrl-alt-shift-l" }, ReformatFile, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-p" } else { "ctrl-p" }, ParameterInfo, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-v" } else { "ctrl-alt-v" }, IntroduceVariable, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-c" } else { "ctrl-alt-c" }, IntroduceConstant, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-m" } else { "ctrl-alt-m" }, ExtractMethod, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-p" } else { "ctrl-alt-p" }, IntroduceParameter, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-f6" } else { "ctrl-f6" }, ChangeSignature, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-h", CallHierarchy, Some(CONTEXT)),
        KeyBinding::new("ctrl-h", TypeHierarchy, Some(CONTEXT)),
        // Over the input's own Ctrl+H (Replace, which is Ctrl+R here).
        KeyBinding::new("ctrl-h", TypeHierarchy, Some("FileEditor > Input")),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-shift-h" } else { "ctrl-shift-h" }, MethodHierarchy, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-t" } else { "ctrl-alt-t" }, SurroundWith, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-alt-j" } else { "ctrl-alt-j" }, SurroundWithLiveTemplate, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-n" } else { "alt-insert" }, Generate, Some(CONTEXT)),
        // Bookmarks: F11, Ctrl+F11 (F3 / ⌥F3 in the macOS keymap).
        KeyBinding::new(if cfg!(target_os = "macos") { "f3" } else { "f11" }, ToggleBookmark, Some(CONTEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "alt-f3" } else { "ctrl-f11" }, ToggleBookmarkWithMnemonic, Some(CONTEXT)),
        // Basic completion: Ctrl+Space (⌃Space).
        KeyBinding::new("ctrl-space", TriggerCompletion, Some("FileEditor > Input")),
        // Copy Absolute Path: Ctrl+Shift+C (⇧⌘C).
        KeyBinding::new("secondary-shift-c", CopyAbsolutePath, Some(CONTEXT)),
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
    /// Local History › Show History for Selection: 1-based lines, inclusive.
    LocalSelectionHistory { path: String, lines: (usize, usize) },
    SelectCommit(String),
    OpenDiff(DiffSource),
    FilesChanged,
    CreateGist { name: String, content: String },
    /// Go to Declaration found these (one jumps, several ask in a popup at
    /// `at`: the mouse for Ctrl+click, the caret for Ctrl+B).
    Navigate { targets: Vec<crate::index::nav::Target>, at: Option<Point<Pixels>> },
    /// Go to Declaration found nothing for the identifier at this offset
    /// (on a declaration itself, IntelliJ shows its usages instead).
    NoDeclaration { text: String, offset: usize, at: Option<Point<Pixels>>, declaration: bool },
    /// A gutter implementation marker: its targets, chosen in a popup at `at`.
    ChooseTargets { title: String, targets: Vec<crate::index::nav::Target>, at: Point<Pixels> },
    /// Find Usages of the identifier at a byte offset of this text.
    FindUsages { text: String, offset: usize },
    /// Refactor › Rename (Shift+F6) of the identifier at this offset.
    Rename { text: String, offset: usize },
    /// Copy Path/Reference…: the Copy popup for this file.
    CopyPathReference,
    /// Saved: re-index this file.
    Saved(String),
    /// Other files written (a language server's code action): reload them.
    FilesWritten(Vec<String>),
    /// Files a language server's code action renamed (from, to): their
    /// tabs follow them.
    FilesMoved(Vec<(String, String)>),
    /// The text was edited (a preview tab becomes a normal one).
    Edited,
    /// The text changed, saved or not: the same file in the other split
    /// follows it.
    TextChanged,
    /// The file is gone from disk (a checkout, say) and had no unsaved edits.
    Deleted,
    /// Call Hierarchy (Ctrl+Alt+H) of the function at this offset.
    CallHierarchy { text: String, offset: usize },
    /// Type Hierarchy (Ctrl+H) of the type at this offset.
    TypeHierarchy { text: String, offset: usize },
    /// Method Hierarchy (Ctrl+Shift+H) of the method at this offset.
    MethodHierarchy { text: String, offset: usize },
    /// The strings banner's Open editor: the Translations Editor of this
    /// values file's res folder.
    OpenTranslations(String),
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
    /// The file inside `repository` (git's path for it).
    own: String,
    path: String,
    /// `None` for the working tree.
    revision: Option<String>,
    state: Entity<EditorState>,
    /// Ctrl+F / Ctrl+R.
    find: Entity<crate::ui::find_bar::FindBar>,
    /// Line separator and indent, for the status bar.
    format: TextFormat,
    /// HEAD's version, for change markers.
    base: std::sync::Arc<str>,
    /// Not under version control (neither in HEAD nor in the index): no
    /// change markers, as IntelliJ shows none for unversioned files.
    unversioned: bool,
    saved: String,
    hunks: Vec<Hunk>,
    /// The last Run with Coverage's lines of this file (0-based).
    coverage: Vec<(u32, crate::domain::coverage::LineCoverage)>,
    /// The change block whose popup is open (a click on its gutter marker).
    popup: Option<usize>,
    /// Where the popup goes: under its block, as the last paint laid it out.
    popup_anchor: Rc<std::cell::Cell<Option<Point<Pixels>>>>,
    /// Whether the last right-click landed on the line-number gutter, which
    /// has its own menu (Annotate with Git Blame), as in IntelliJ.
    gutter_click: Rc<std::cell::Cell<bool>>,
    /// The line the gutter was right-clicked on, for the menu's bookmark
    /// items (F11 from the keyboard takes the caret's line).
    gutter_line: Rc<std::cell::Cell<Option<usize>>>,
    /// Icon slots the implementation markers take.
    hierarchy_slots: usize,
    /// Resource previews by line (color swatches, drawable thumbnails).
    previews: Vec<(u32, crate::usecase::android_resources::Preview)>,
    /// The strings banner's Hide notification.
    strings_banner_hidden: bool,
    /// Download Sources on a decompiled class: in progress, or why it failed.
    sources_status: Option<(bool, String)>,
    /// Extend Selection's steps, for Shrink Selection.
    selection_steps: Vec<std::ops::Range<usize>>,
    /// The language server's code actions being asked for (Alt+Enter).
    server_actions_task: Option<gpui_kit::Task<()>>,
    /// Add names to call arguments, waiting on the callee's declaration.
    argument_names_task: Option<gpui_kit::Task<()>>,
    /// A vector drawable's preview, and Code / Split / Design.
    vector: Option<vector_preview::VectorPreview>,
    vector_mode: vector_preview::VectorMode,
    /// The preview's zoom, px per dp; none: fit.
    vector_zoom: Option<f32>,
    /// The drawable preview's transparency chessboard (off, as IntelliJ's image viewer starts).
    vector_chessboard: bool,
    /// The manifest editor's Merged Manifest tab is selected.
    manifest_merged: bool,
    merged: Option<merged_manifest::MergedManifest>,
    /// The gutter's vector drawable icons, by file.
    vector_icons: Rc<std::collections::HashMap<String, std::sync::Arc<gpui_kit::Image>>>,
    previews_task: Option<gpui_kit::Task<()>>,
    /// Parameter Info (Ctrl+P) shown, and its lookup or autopopup.
    parameter_info: Option<parameter_info::ParameterInfo>,
    parameter_task: Option<gpui_kit::Task<()>>,
    /// The test classes and methods of a test file, for the gutter's run
    /// icons, and the popup a click on one opens.
    test_marks: Vec<crate::domain::testing::TestMark>,
    test_marks_task: Option<gpui_kit::Task<()>>,
    test_menu: Option<(crate::domain::testing::TestMark, Point<Pixels>)>,
    /// The color picker a click on a color literal's gutter swatch opens.
    color_edit: Option<previews::ColorEdit>,
    /// A breakpoint's properties popup (a right-click on it).
    breakpoint_popup: Option<breakpoints::BreakpointPopup>,
    /// Lines before the last edit: breakpoints below it move along.
    line_count: usize,
    /// The text before the last edit, for breakpoints to follow it.
    breakpoint_text: Option<String>,
    /// Inspections of the text (a newer dependency version in a Gradle file).
    inspection_problems: Vec<crate::domain::diagnostics::Problem>,
    inspections_task: Option<gpui_kit::Task<()>>,
    /// Alt+Enter's popup.
    context_actions: Option<fixes::ContextActions>,
    /// A live template being filled in.
    template: Option<templates::TemplateState>,
    context_focus: gpui_kit::FocusHandle,
    popup_focus: gpui_kit::FocusHandle,
    /// The file couldn't be read: no editing, saving or reloading.
    error: Option<String>,
    /// A failed action (Open on GitHub, Stage Lines…): shown until the next
    /// edit, without blocking the editor.
    notice: Option<String>,
    code_index: Option<Entity<CodeIndex>>,
    /// Syntax errors of the current text, and the compiler's problems from
    /// the last build (see `problems.rs`).
    syntax_problems: Vec<crate::domain::diagnostics::Problem>,
    compiler_problems: Vec<crate::domain::diagnostics::Problem>,
    /// The language server's problems, once it has published for the
    /// file; whether it has yet to see the latest text (edits are counted
    /// so one made while the text was on its way isn't missed), and the
    /// server's publish count last read (see `watch_server`).
    server_problems: Option<Vec<crate::domain::diagnostics::Problem>>,
    server_dirty: bool,
    server_edits: u64,
    server_revision: u64,
    server_task: Option<gpui_kit::Task<()>>,
    problems_task: Option<gpui_kit::Task<()>>,
    /// Annotate with Git Blame, shown in the gutter.
    annotation: Option<Annotation>,
    annotation_task: Option<gpui_kit::Task<()>>,
    /// The annotation hover card: its line and where the mouse rested.
    hover_card: Option<(usize, Point<Pixels>)>,
    hover_task: Option<gpui_kit::Task<()>>,
    markers_task: Option<gpui_kit::Task<()>>,
    /// The gutter's implementation markers (see `hierarchy_strip`).
    /// Sorted by line, each with its slot (two marks on one line keep the
    /// gutter order: up, then down), shared with the paint callback.
    hierarchy: Rc<Vec<(u32, usize, crate::index::hierarchy::GutterMark)>>,
    hierarchy_task: Option<gpui_kit::Task<()>>,
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
    /// The last lookup was on a declaration itself, at this offset of this
    /// text: Ctrl+click shows its usages, as IntelliJ does.
    declaration: Rc<RefCell<Option<(String, usize)>>>,
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
        let task = self.index.read(cx).go_to(self.path.clone(), text.clone(), offset, cx);
        let (last, declaration, path) = (self.last.clone(), self.declaration.clone(), self.path.clone());
        cx.spawn(async move |_| {
            let nav = task.await;
            *declaration.borrow_mut() = None;
            let on_declaration = matches!(nav, crate::index::service::Nav::Declaration);
            let targets = match nav {
                crate::index::service::Nav::Targets(targets) => targets,
                crate::index::service::Nav::Declaration => Vec::new(),
            };
            *last.borrow_mut() = targets.clone();
            // On a declaration: a link to itself, so Ctrl+hover underlines it
            // and Ctrl+click reaches the show-document hook (usages).
            if on_declaration {
                *declaration.borrow_mut() = Some((text.clone(), offset));
                let uri = crate::index::lsp::path_to_uri(&root.join(&path)).parse::<lsp_types::Uri>()?;
                let origin = origin.unwrap_or(lsp_types::Range::default());
                return Ok(vec![lsp_types::LocationLink { origin_selection_range: Some(origin), target_uri: uri, target_range: origin, target_selection_range: origin }]);
            }
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

    /// `path` is the project path the workspace knows the file by; `own`
    /// the same file inside `repository` (they differ for a nested root).
    pub fn new(repository: Repository, own: String, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let image = image_format(&path).map(|format| {
            let bytes = match &revision {
                Some(rev) => repository.run_bytes(["show", &format!("{rev}:{own}")]).map_err(|e| e.to_string()),
                None => std::fs::read(repository.root().join(&own)).map_err(|e| e.to_string()),
            };
            bytes.map(|bytes| ImageFile::new(format, bytes))
        });
        let (image, image_error) = match image {
            Some(Ok(image)) => (Some(image), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let (content, error) = if image_error.is_some() || image.is_some() { (String::new(), image_error) } else { match &revision {
            Some(rev) => match repository.run(["show", &format!("{rev}:{own}")]) {
                Ok(text) => (text, None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
            None => match std::fs::read(repository.root().join(&own)) {
                Ok(bytes) if bytes.contains(&0) => (String::new(), Some("Binary file".to_owned())),
                Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), None),
                Err(error) => (String::new(), Some(error.to_string())),
            },
        } };
        // A library's source (External Libraries) isn't in the repository.
        let external = crate::index::store::ProjectIndex::is_external(&path);
        let (base, unversioned) = if revision.is_none() && !external && image.is_none() {
            match repository.run(["show", &format!("HEAD:{own}")]) {
                Ok(base) => (base, false),
                Err(_) => (String::new(), repository.run(["ls-files", "--error-unmatch", "--", &own]).is_err()),
            }
        } else {
            (content.clone(), false)
        };
        let language = crate::infra::file_type_store::language_of(&repository.root().join(&own)).unwrap_or_else(|| language_for(&path).to_owned());
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
        let settings = cx.observe_global_in::<crate::settings::Settings>(window, move |this, window, cx| {
            let view = crate::settings::Settings::get(cx).diff.clone();
            if view == shown {
                return;
            }
            if view.show_gutter_icons != shown.show_gutter_icons {
                this.apply_gutter_extra(cx);
            }
            settings_state.update(cx, |state, cx| {
                state.set_line_number(view.show_line_numbers, window, cx);
                state.set_show_whitespaces(view.show_whitespaces, window, cx);
                state.set_indent_guides(view.show_indent_guides, window, cx);
                state.set_soft_wrap(view.soft_wrap, window, cx);
            });
            shown = view;
        });
        let bookmarks = cx.observe_global::<crate::bookmarks::Bookmarks>(|this, cx| {
            this.apply_gutter_extra(cx);
            cx.notify();
        });
        let subscriptions = vec![settings, bookmarks, cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                // IntelliJ closes the change popup on typing.
                this.popup = None;
                this.notice = None;
                this.update_markers(cx);
                this.update_hierarchy(true, cx);
                this.update_problems(true, cx);
                this.update_inspections(true, cx);
                this.server_edits += 1;
                this.server_dirty = true;
                this.update_previews(true, cx);
                this.update_test_marks(true, cx);
                this.follow_breakpoints(cx);
                this.follow_bookmarks(cx);
                // The annotations follow the edit (new lines: Not Committed Yet).
                if this.annotation.is_some() {
                    this.load_annotations(true, cx);
                }
                if this.is_dirty(cx) {
                    cx.emit(FileEditorEvent::Edited);
                }
                cx.emit(FileEditorEvent::TextChanged);
                cx.notify();
            }
        }), cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Change => {
                    this.template_edited(window, cx);
                    this.parameter_info_autopopup(window, cx);
                }
                // A live template picked in the completion popup expands.
                InputEvent::CompletionCommand { command, .. } if command == crate::domain::live_templates::EXPAND_COMMAND => {
                    this.expand_template(window, cx);
                }
                // A server completion's auto-import, computed once picked.
                InputEvent::CompletionCommand { command, argument } if command == crate::ui::completion::RESOLVE_COMMAND => {
                    if let Some(item) = argument.as_deref().and_then(|a| serde_json::from_str(a).ok()) {
                        this.resolve_completion(item, window, cx);
                    }
                }
                _ => {}
            }
        })];
        let read_only = revision.is_some() || external;
        let find = cx.new(|cx| crate::ui::find_bar::FindBar::new(state.clone(), read_only, window, cx));
        let mut this = Self {
            find,
            repository,
            own,
            path,
            revision,
            state,
            base: base.into(),
            unversioned,
            format: TextFormat::of(&content),
            saved: content,
            hunks: Vec::new(),
            coverage: Vec::new(),
            popup: None,
            popup_anchor: Rc::default(),
            gutter_click: Rc::default(),
            gutter_line: Rc::default(),
            hierarchy_slots: 0,
            previews: Vec::new(),
            strings_banner_hidden: false,
            sources_status: None,
            selection_steps: Vec::new(),
            server_actions_task: None,
            argument_names_task: None,
            vector: None,
            vector_mode: Default::default(),
            vector_zoom: None,
            vector_chessboard: false,
            manifest_merged: false,
            merged: None,
            vector_icons: Rc::default(),
            previews_task: None,
            parameter_info: None,
            parameter_task: None,
            test_marks: Vec::new(),
            test_marks_task: None,
            test_menu: None,
            color_edit: None,
            line_count: 0,
            breakpoint_text: None,
            breakpoint_popup: None,
            inspection_problems: Vec::new(),
            inspections_task: None,
            context_actions: None,
            template: None,
            context_focus: cx.focus_handle(),
            popup_focus: cx.focus_handle(),
            error,
            notice: None,
            code_index: None,
            syntax_problems: Vec::new(),
            compiler_problems: Vec::new(),
            server_problems: None,
            server_dirty: false,
            server_edits: 0,
            server_revision: 0,
            server_task: None,
            problems_task: None,
            annotation: None,
            annotation_task: None,
            hover_card: None,
            hover_task: None,
            markers_task: None,
            hierarchy: Rc::default(),
            hierarchy_task: None,
            image,
            zoom: None,
            _subscriptions: subscriptions,
        };
        this.update_markers(cx);
        this.line_count = this.state.read(cx).text().lines_len();
        if this.breakpoint_file().is_some() {
            this.breakpoint_text = Some(this.text(cx));
        }
        this.update_test_marks(false, cx);
        if this.breakpoint_file().is_some() {
            // Breakpoints and the execution point.
            let debug = crate::ui::debug_service::DebugService::global(cx);
            this._subscriptions.push(cx.observe(&debug, |_, _, cx| cx.notify()));
        }
        this.watch_coverage(cx);
        if this.bookmark_file().is_some_and(|f| crate::domain::testing::test_kind(&f).is_some()) {
            // The run icons show the last run's outcomes.
            let tests = crate::ui::test_service::TestService::global(cx);
            this._subscriptions.push(cx.observe(&tests, |_, _, cx| cx.notify()));
        }
        this.apply_gutter_extra(cx);
        this
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Override File Type: highlight as `language` from now on.
    pub fn set_language(&mut self, language: String, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_highlighter(language, cx));
        cx.notify();
    }

    /// The repository the file is read from, and its path there.
    pub fn source(&self) -> (Repository, String) {
        (self.repository.clone(), self.own.clone())
    }

    /// The caret's 0-based line.
    pub fn caret_line(&self, cx: &gpui_kit::App) -> usize {
        self.state.read(cx).cursor_position().line as usize
    }

    /// The selected lines (1-based) for a web URL.
    pub fn url_lines(&self, cx: &gpui_kit::App) -> (usize, usize) {
        self.selected_lines(cx)
    }

    /// The text as edited and as last saved, for the same file's editor in
    /// the other split.
    pub fn buffer(&self, cx: &gpui_kit::App) -> (String, String) {
        (self.text(cx), self.saved.clone())
    }

    /// Takes the other split's text and saved state for the same file, as
    /// IntelliJ's splits share one document: only the changed span is
    /// replaced, and this editor's caret stays where it was in the text.
    pub fn mirror(&mut self, text: &str, saved: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        self.saved = saved.to_owned();
        let own = self.text(cx);
        if own != text {
            let caret = self.state.read(cx).cursor();
            let prefix = own.bytes().zip(text.bytes()).take_while(|(a, b)| a == b).count();
            let suffix = own.bytes().rev().zip(text.bytes().rev()).take_while(|(a, b)| a == b).count().min(own.len().min(text.len()) - prefix);
            // A caret after the edit moves with the text after it.
            let caret = if caret >= own.len() - suffix { caret + text.len() - own.len() } else { caret.min(prefix) };
            self.replace_text(&own, text, caret, window, cx);
        }
        cx.notify();
    }

    /// Replaces the text (unsaved), e.g. a moved file's edits.
    pub fn set_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_value(text, window, cx));
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
        let declaration: Rc<RefCell<Option<(String, usize)>>> = Rc::default();
        let provider = Rc::new(IndexProvider { index: index.clone(), path: self.path.clone(), last: last.clone(), declaration: declaration.clone() });
        let editor = cx.entity().downgrade();
        // Ctrl+click: several targets, or a declaration's usages, list in a
        // popup where the mouse is (IntelliJ on Windows).
        let show: ShowDocumentHandler = Rc::new(move |_, window, cx| {
            // The popup's corner just right of and below the mouse.
            let at = Some(window.mouse_position() + point(px(2.), px(12.)));
            let event = match declaration.borrow().clone() {
                Some((text, offset)) => FileEditorEvent::NoDeclaration { text, offset, at, declaration: true },
                None => FileEditorEvent::Navigate { targets: last.borrow().clone(), at },
            };
            editor.update(cx, |_, cx| cx.emit(event)).ok();
            true
        });
        let editable = !self.read_only();
        let completion = crate::ui::completion::IndexCompletion::new(index.clone(), self.path.clone());
        self.state.update(cx, |state, cx| {
            let lsp = state.lsp_mut();
            lsp.definition_provider = Some(provider.clone());
            lsp.hover_provider = Some(provider);
            lsp.show_document = Some(show);
            if editable {
                lsp.completion_provider = Some(completion);
                // Room for the package at the right, as IntelliJ shows it.
                lsp.completion_menu.max_width = px(560.);
            }
            cx.notify();
        });
        if !self.read_only() {
            let text = self.state.read(cx).value().to_string();
            index.read(cx).warm_up(&self.path, text, cx);
        }
        self._subscriptions.push(cx.subscribe(&index, |this, _, event: &crate::index::service::IndexEvent, cx| {
            if matches!(event, crate::index::service::IndexEvent::Changed) {
                this.update_hierarchy(false, cx);
                this.update_previews(false, cx);
            }
        }));
        self.code_index = Some(index);
        self.update_hierarchy(false, cx);
        self.update_problems(false, cx);
        self.update_inspections(false, cx);
        self.update_previews(false, cx);
        self.attach_compiler(cx);
        self.watch_server(cx);
    }

    /// The gutter's implementation markers, recomputed from the text after
    /// an edit (a moment later) or when the index changes.
    fn update_hierarchy(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let Some(index) = self.code_index.clone() else { return };
        if self.revision.is_some() || self.image.is_some() || crate::index::service::CodeIndex::lang_of(&self.path, "").is_none() {
            return;
        }
        let path = self.path.clone();
        self.hierarchy_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            }
            let Ok(task) = this.update(cx, |this, cx| index.read(cx).gutter_marks(path, this.text(cx), cx)) else { return };
            let marks = task.await;
            this.update(cx, |this, cx| {
                // Room for two icons where a member both overrides and is overridden.
                let mut per_line: std::collections::HashMap<u32, usize> = Default::default();
                for m in &marks {
                    *per_line.entry(m.line).or_default() += 1;
                }
                this.hierarchy_slots = per_line.values().copied().max().unwrap_or(1).clamp(1, 2);
                this.hierarchy = Rc::new(slotted(marks));
                this.apply_gutter_extra(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// IntelliJ's implementation markers (I↓ O↓ I↑ O↑) between the line
    /// numbers and the fold icons; a click lists where they lead.
    fn hierarchy_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.state.clone();
        let this = cx.entity().downgrade();
        let dark = cx.palette().dark;
        let marks = self.hierarchy.clone();
        let kotlin = self.path.ends_with(".kt") || self.path.ends_with(".kts");
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = state.read(cx);
                let (Some(line_height), Some(visible), Some(left)) = (state.line_height(), state.visible_row_range(), state.gutter_extra_left()) else { return };
                let rope = state.text();
                let lines = rope.lines_len().max(1);
                let mut hits: Vec<(Bounds<Pixels>, usize)> = Vec::new();
                let first = marks.partition_point(|(line, _, _)| (*line as usize) < visible.start);
                for (i, (line, slot, mark)) in marks.iter().enumerate().skip(first) {
                    let line = *line as usize;
                    if line > visible.end || line >= lines {
                        break;
                    }
                    let at = rope.line_start_offset(line);
                    let Some(top) = state.range_to_bounds(&(at..at)).map(|b| b.top()) else { continue };
                    let icon = px(GUTTER_ICON);
                    let rect = Bounds::new(point(left + px(GUTTER_ICON_GAP + 1. + GUTTER_ICON_SLOT * *slot as f32), top + (line_height - icon) / 2.), size(icon, icon));
                    if !bounds.contains(&rect.center()) {
                        continue;
                    }
                    let icon = match mark.kind {
                        crate::index::hierarchy::MarkKind::Implemented => icons::GUTTER_IMPLEMENTED_METHOD,
                        crate::index::hierarchy::MarkKind::Overridden => icons::GUTTER_OVERRIDEN_METHOD,
                        crate::index::hierarchy::MarkKind::Implementing => icons::GUTTER_IMPLEMENTING_METHOD,
                        crate::index::hierarchy::MarkKind::Overriding => icons::GUTTER_OVERRIDING_METHOD,
                    };
                    for (path, rgba) in if dark { icon.dark } else { icon.light } {
                        let color: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                        window.paint_svg(rect, (*path).into(), None, gpui_kit::TransformationMatrix::unit(), color, cx).ok();
                    }
                    hits.push((rect, i));
                }
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                        return;
                    }
                    let Some(&(rect, i)) = hits.iter().find(|(r, _)| r.dilate(px(2.)).contains(&e.position)) else { return };
                    let mark = &marks[i].2;
                    cx.stop_propagation();
                    // The popup's corner at the icon's bottom right, as IntelliJ's.
                    let event = FileEditorEvent::ChooseTargets { title: mark.title(kotlin), targets: mark.targets.clone(), at: rect.bottom_right() };
                    this.update(cx, |_, cx| cx.emit(event)).ok();
                    window.refresh();
                });
            },
        )
        .absolute()
        .inset_0()
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
        let task = index.read(cx).go_to(self.path.clone(), text.clone(), offset, cx);
        cx.spawn(async move |this, cx| {
            let nav = task.await;
            this.update(cx, |this, cx| {
                // Ctrl+B's popups open under the caret.
                let at = this.state.read(cx).range_to_bounds(&(offset..offset)).map(|b| b.bottom_left() + point(px(0.), px(4.)));
                let event = match nav {
                    crate::index::service::Nav::Declaration => FileEditorEvent::NoDeclaration { text, offset, at, declaration: true },
                    crate::index::service::Nav::Targets(targets) if targets.is_empty() => FileEditorEvent::NoDeclaration { text, offset, at, declaration: false },
                    crate::index::service::Nav::Targets(targets) => FileEditorEvent::Navigate { targets, at },
                };
                cx.emit(event)
            })
            .ok();
        })
        .detach();
    }

    fn find_usages(&mut self, _: &FindUsages, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        cx.emit(FileEditorEvent::FindUsages { text, offset });
    }

    fn rename_element(&mut self, _: &RenameElement, _: &mut Window, cx: &mut Context<Self>) {
        self.rename(cx);
    }

    /// Refactor › Rename at the caret (Shift+F6).
    pub fn rename(&mut self, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        cx.emit(FileEditorEvent::Rename { text, offset });
    }

    fn optimize_imports_action(&mut self, _: &OptimizeImports, window: &mut Window, cx: &mut Context<Self>) {
        self.optimize_imports(window, cx);
    }

    /// Code › Optimize Imports (Ctrl+Alt+O) of a Java or Kotlin file, an
    /// undoable edit, its result in a hint as Android Studio shows it; of
    /// another language's file, by its language server.
    pub fn optimize_imports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let style = crate::index::lang::Lang::from_path(&self.path).and_then(crate::infra::index_code_model::IndexCodeModel::import_style);
        let Some(style) = style else {
            // Other languages: their language server's Organize Imports.
            self.server_optimize_imports(window, cx);
            return;
        };
        let text = self.text(cx);
        let hint = |message: String, window: &mut Window, cx: &mut Context<Self>| Self::hint("Optimize Imports", message, window, cx);
        let Some(edit) = crate::domain::imports::optimize(style, &text) else {
            hint("Unused imports not found".into(), window, cx);
            return;
        };
        let cursor = self.state.read(cx).cursor();
        // The caret stays on its text: after the block it moves with it.
        let cursor = if cursor >= edit.range.end {
            cursor + edit.text.len() - edit.range.len()
        } else {
            cursor.min(edit.range.start)
        };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(edit.range.clone(), cx);
            state.replace(&edit.text, window, cx);
            state.set_selected_range(cursor..cursor, cx);
        });
        let notice = match edit.removed {
            0 => "Rearranged imports".to_owned(),
            1 => "Removed 1 unused import".to_owned(),
            n => format!("Removed {n} unused imports"),
        };
        hint(notice, window, cx);
    }

    /// Code › Reformat Code (Ctrl+Alt+L), as IntelliJ's: the selected
    /// lines, else the file, in IntelliJ's default code style, as one
    /// undoable edit; the caret stays on its text. Imports are optimized
    /// when the last Reformat File run asked for it, as IntelliJ does.
    pub fn reformat_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::reformat_dialog::{ReformatOptions, ReformatScope, last_options};
        let selected = !self.state.read(cx).selected_range().is_empty();
        let (optimize_imports, code_cleanup) = last_options();
        let scope = if selected { ReformatScope::Selected } else { ReformatScope::WholeFile };
        self.run_reformat(ReformatOptions { scope, optimize_imports, code_cleanup }, window, cx);
    }

    /// Code › Reformat File… (Ctrl+Alt+Shift+L): the scope and options in
    /// IntelliJ's dialog, then the reformat.
    pub fn reformat_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let style = self.jvm_style();
        if self.read_only() || (style.is_none() && !self.path.ends_with(".xml")) {
            return;
        }
        let name = std::path::Path::new(&self.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let selected = !self.state.read(cx).selected_range().is_empty();
        let changed = self.hunks.iter().any(|h| !h.new.is_empty());
        let this = cx.entity().downgrade();
        crate::ui::reformat_dialog::open(
            &name,
            selected,
            changed,
            style.is_some(),
            move |options, window, cx| {
                let _ = this.update(cx, |editor, cx| editor.run_reformat(options, window, cx));
            },
            window,
            cx,
        );
    }

    fn run_reformat(&mut self, options: crate::ui::reformat_dialog::ReformatOptions, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::reformat_dialog::ReformatScope;
        let xml = self.path.ends_with(".xml");
        let style = self.jvm_style().map(|(_, s)| s);
        if self.read_only() || (style.is_none() && !xml) {
            return;
        }
        let text = self.text(cx);
        let (selection, cursor) = {
            let state = self.state.read(cx);
            (state.selected_range(), state.cursor())
        };
        let line_of = |offset: usize| text[..offset.min(text.len())].matches('\n').count();
        // The lines to reformat, last first so earlier ones keep their place; none: the file.
        let mut ranges: Vec<std::ops::Range<usize>> = match options.scope {
            ReformatScope::Selected if !selection.is_empty() => {
                // A selection ending at a line's start doesn't take that line.
                let end = if selection.end > 0 && text[..selection.end].ends_with('\n') { selection.end - 1 } else { selection.end };
                vec![line_of(selection.start)..line_of(end) + 1]
            }
            ReformatScope::VcsChanged => self.hunks.iter().map(|h| h.new.clone()).filter(|r| !r.is_empty()).collect(),
            _ => Vec::new(),
        };
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        let whole = options.scope == ReformatScope::WholeFile || (options.scope == ReformatScope::Selected && ranges.is_empty());
        // `.editorconfig`'s indent, over the file's own.
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(std::path::Path::to_path_buf)).unwrap_or_else(|| self.repository.root().to_path_buf());
        let indent = crate::infra::editorconfig::indent_for(&root, &self.absolute_path());
        let indent = indent.as_deref();
        let mut new = match style {
            Some(style) if whole => crate::domain::reformat::reformat_indented(style, &text, None, indent),
            Some(style) => ranges.iter().fold(text.clone(), |t, r| crate::domain::reformat::reformat_indented(style, &t, Some(r.clone()), indent)),
            // XML: the whole file (its tags and attributes don't follow lines).
            None => crate::domain::xml_reformat::reformat_xml_indented(crate::domain::xml_reformat::XmlKind::of(&self.path), &text, indent),
        };
        // The lines the reformat itself changed, for the notice.
        let formatted = new.clone();
        // Optimize imports / Code cleanup (Junction's clean-up removes unused imports).
        let mut removed = 0;
        if let Some(style) = style.filter(|_| options.optimize_imports || options.code_cleanup) {
            if let Some(edit) = crate::domain::imports::optimize(style, &new) {
                removed = edit.removed;
                new = format!("{}{}{}", &new[..edit.range.start], edit.text, &new[edit.range.end..]);
            }
        }
        let hint = |message: String, window: &mut Window, cx: &mut Context<Self>| Self::hint("Reformat Code", message, window, cx);
        if new == text {
            return hint("No lines changed: content is already properly formatted".into(), window, cx);
        }
        // Only blank lines go away: the n-th non-blank line is the same one.
        let old_lines: Vec<&str> = text.split('\n').collect();
        let new_lines: Vec<&str> = new.split('\n').collect();
        let caret_line = line_of(cursor);
        let line_start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let old_line = old_lines.get(caret_line).copied().unwrap_or("");
        let into_text = (cursor - line_start).saturating_sub(old_line.len() - old_line.trim_start().len());
        // Removed imports above the caret are lines fewer before it.
        let below_imports = old_lines[..caret_line].iter().any(|l| l.trim_start().starts_with("import "));
        let rank = old_lines[..caret_line].iter().filter(|l| !l.trim().is_empty()).count().saturating_sub(if below_imports { removed } else { 0 });
        let target = new_lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()).nth(rank).map(|(i, _)| i).unwrap_or(new_lines.len().saturating_sub(1));
        let new_start: usize = new_lines[..target].iter().map(|l| l.len() + 1).sum();
        let target_line = new_lines.get(target).copied().unwrap_or("");
        let indent = target_line.len() - target_line.trim_start().len();
        let mut caret = (new_start + indent + into_text).min(new_start + target_line.len());
        while !new.is_char_boundary(caret) {
            caret -= 1;
        }
        let changed = {
            let formatted_lines: Vec<&str> = formatted.split('\n').collect();
            let mut count = 0;
            let (mut a, mut b) = (old_lines.iter().filter(|l| !l.trim().is_empty()), formatted_lines.iter().filter(|l| !l.trim().is_empty()));
            while let (Some(x), Some(y)) = (a.next(), b.next()) {
                count += usize::from(x != y);
            }
            count + old_lines.len().abs_diff(formatted_lines.len())
        };
        // One edit: the changed span.
        let prefix = text.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
        let suffix = text.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(text.len().min(new.len()) - prefix);
        let (mut start, mut old_end) = (prefix, text.len() - suffix);
        while !text.is_char_boundary(start) || !new.is_char_boundary(start) {
            start -= 1;
        }
        while !text.is_char_boundary(old_end) {
            old_end += 1;
        }
        let new_end = new.len() - (text.len() - old_end);
        let replacement = new[start..new_end].to_owned();
        self.state.update(cx, |state, cx| {
            state.set_selected_range(start..old_end, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(caret..caret, cx);
        });
        cx.notify();
        let imports = match removed {
            0 => String::new(),
            1 => ", removed 1 import".to_owned(),
            n => format!(", removed {n} imports"),
        };
        if changed == 0 {
            return hint(format!("Removed {removed} unused {}", if removed == 1 { "import" } else { "imports" }), window, cx);
        }
        hint(format!("Formatted {changed} {}{imports}", if changed == 1 { "line" } else { "lines" }), window, cx);
    }

    /// A refactoring's result or why it can't run, as Android Studio's
    /// hint shows it.
    fn hint(title: &str, message: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{notification::Notification, WindowExt as _};
        window.push_notification(Notification::info(message.into()).title(title.to_owned()), cx);
    }

    /// The Java / Kotlin import style of this file, for refactorings.
    fn jvm_style(&self) -> Option<(crate::index::lang::Lang, crate::domain::imports::ImportStyle)> {
        let lang = crate::index::lang::Lang::from_path(&self.path)?;
        Some((lang, crate::infra::index_code_model::IndexCodeModel::import_style(lang)?))
    }

    fn introduce_variable_action(&mut self, _: &IntroduceVariable, window: &mut Window, cx: &mut Context<Self>) {
        self.introduce(crate::ui::introduce_dialog::Introduce::Variable, window, cx);
    }

    /// Extract Method / Function (Ctrl+Alt+M): the selected statements or
    /// expression into a new private method after this one, through the
    /// dialog; the new method's name selected after.
    fn extract_method_action(&mut self, _: &ExtractMethod, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::imports::ImportStyle;
        let Some((lang, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let title = if style == ImportStyle::Java { "Extract Method" } else { "Extract Function" };
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        let plan = match crate::index::extract_method::plan(lang, &text, selection) {
            Ok(plan) => plan,
            Err(message) => return Self::hint(title, format!("Cannot perform refactoring.\n{message}"), window, cx),
        };
        let editor = cx.entity().downgrade();
        let dialog_plan = plan.clone();
        crate::ui::extract_method_dialog::open(style, dialog_plan, move |request, window, cx| {
            editor
                .update(cx, |this, cx| {
                    let text = this.text(cx);
                    let unit = crate::domain::surround::indent_unit(&text);
                    match crate::domain::extract_method::extract(style, &text, &plan, &request, &unit) {
                        Err(message) => Self::hint(title, message, window, cx),
                        Ok(done) => this.state.update(cx, |state, cx| {
                            state.set_selected_range(done.range.clone(), cx);
                            state.replace(&done.text, window, cx);
                            state.set_selected_range(done.select.clone(), cx);
                        }),
                    }
                })
                .ok();
        }, window, cx);
    }

    /// Introduce Parameter (Ctrl+Alt+P): the selected expression becomes
    /// the function's new last parameter; its calls in the project pass
    /// the expression (or, in Kotlin, it's the parameter's default value).
    fn introduce_parameter_action(&mut self, _: &IntroduceParameter, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::introduce_parameter::{apply, call_edit, edits};
        use crate::index::introduce_parameter::{call_sites, plan};
        use crate::index::lang::Lang;
        use crate::infra::project_files::DiskFiles;
        use crate::usecase::update_calls;
        const TITLE: &str = "Introduce Parameter";
        let Some((lang, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        let (plan, source) = match plan(lang, &text, selection) {
            Ok(found) => found,
            Err(message) => return Self::hint(TITLE, format!("Cannot perform refactoring.\n{message}"), window, cx),
        };
        let suggestions = crate::domain::introduce_variable::suggest_names(style, &source, &|n| plan.taken.iter().any(|t| t == n) || plan.params.iter().any(|p| p == n));
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(|r| r.to_path_buf()));
        let own = self.path.clone();
        // The calls here (not inside what's replaced) and in the project's other Kotlin / Java files.
        let in_occurrence = |c: &crate::domain::introduce_parameter::CallSite| plan.occurrences.iter().any(|o| o.start <= c.args_close && c.args_close < o.end);
        let here: Vec<_> = call_sites(lang, &text, &plan.function, plan.params.len()).into_iter().filter(|c| !in_occurrence(c)).collect();
        let (function, params) = (plan.function.clone(), plan.params.len());
        let files = root.clone().map(|root| DiskFiles { root, repository: None });
        let task = cx.background_spawn(async move {
            let Some(root) = root else { return Vec::new() };
            let files = DiskFiles { root, repository: None };
            update_calls::find(&files, &own, &function, &|path, text| Lang::from_path(path).map(|lang| call_sites(lang, text, &function, params)).unwrap_or_default())
        });
        cx.spawn_in(window, async move |this, cx| {
            let elsewhere = task.await;
            this.update_in(cx, |_, window, cx| {
                let editor = cx.entity().downgrade();
                let calls = here.len() + elsewhere.iter().map(|f| f.calls.len()).sum::<usize>();
                let dialog_plan = plan.clone();
                crate::ui::introduce_parameter_dialog::open(style, dialog_plan, suggestions, calls, move |request, window, cx| {
                    editor
                        .update(cx, |this, cx| {
                            if this.text(cx) != text {
                                return Self::hint(TITLE, "The file changed; try again", window, cx);
                            }
                            let changes = match edits(style, &plan, &here, &request) {
                                Ok(changes) => changes,
                                Err(message) => return Self::hint(TITLE, message, window, cx),
                            };
                            // The caret where the selected occurrence was.
                            let at = plan.occurrences[plan.selected].start;
                            let caret = changes.iter().filter(|(r, _)| r.start < at).fold(at as isize, |c, (r, s)| c + s.len() as isize - r.len() as isize) as usize;
                            let new = apply(&text, changes);
                            this.replace_text(&text, &new, caret, window, cx);
                            // The other files' calls, unless the value is a default.
                            let written = match (&files, request.default_value && style == crate::domain::imports::ImportStyle::Kotlin) {
                                (Some(files), false) => update_calls::rewrite(files, &elsewhere, &|f| f.calls.iter().filter_map(|c| call_edit(&plan, c, &request.name)).collect()),
                                _ => Vec::new(),
                            };
                            if !written.is_empty() {
                                cx.emit(FileEditorEvent::FilesWritten(written));
                            }
                        })
                        .ok();
                }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Change Signature (Ctrl+F6): the function at the caret's name, return
    /// type and parameters through the dialog; the declaration, its body's
    /// uses of renamed parameters and the calls in the project follow.
    fn change_signature_action(&mut self, _: &ChangeSignature, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::change_signature::{call_edits, edits};
        use crate::domain::imports::ImportStyle;
        use crate::domain::introduce_parameter::apply;
        use crate::index::introduce_parameter::call_sites;
        use crate::index::lang::Lang;
        use crate::infra::project_files::DiskFiles;
        use crate::usecase::update_calls;
        const TITLE: &str = "Change Signature";
        let Some((lang, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        let signature = match crate::index::change_signature::signature_at(lang, &text, offset) {
            Ok(s) => s,
            Err(message) => return Self::hint(TITLE, message, window, cx),
        };
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(|r| r.to_path_buf()));
        let own = self.path.clone();
        let name = signature.name.clone();
        let here = call_sites(lang, &text, &name, signature.params.len());
        let params = signature.params.len();
        let files = root.clone().map(|root| DiskFiles { root, repository: None });
        let task = cx.background_spawn({
            let name = name.clone();
            async move {
                let Some(root) = root else { return Vec::new() };
                let files = DiskFiles { root, repository: None };
                update_calls::find(&files, &own, &name, &|path, text| Lang::from_path(path).map(|lang| call_sites(lang, text, &name, params)).unwrap_or_default())
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let elsewhere = task.await;
            this.update_in(cx, |_, window, cx| {
                let editor = cx.entity().downgrade();
                let calls = here.len() + elsewhere.iter().map(|f| f.calls.len()).sum::<usize>();
                crate::ui::change_signature_dialog::open(style, signature.clone(), calls, move |request, window, cx| {
                    editor
                        .update(cx, |this, cx| {
                            if this.text(cx) != text {
                                return Self::hint(TITLE, "The file changed; try again", window, cx);
                            }
                            let changes = match edits(style, &text, &signature, &here, &request) {
                                Ok(changes) => changes,
                                Err(message) => return Self::hint(TITLE, message, window, cx),
                            };
                            let caret = signature.name_range.start;
                            let new = apply(&text, changes);
                            this.replace_text(&text, &new, caret.min(new.len()), window, cx);
                            let written = files.as_ref().map_or_else(Vec::new, |files| {
                                update_calls::rewrite(files, &elsewhere, &|f| {
                                    let file_style = if f.path.ends_with(".java") { ImportStyle::Java } else { ImportStyle::Kotlin };
                                    f.calls.iter().flat_map(|c| call_edits(file_style, &signature, &request, c)).collect()
                                })
                            });
                            if !written.is_empty() {
                                cx.emit(FileEditorEvent::FilesWritten(written));
                            }
                        })
                        .ok();
                }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Inline Function, as IntelliJ's dialog: Inline All and Remove (every
    /// call in the project becomes the function's body, the declaration
    /// goes) or, invoked on a call, Inline This Only.
    pub(super) fn inline_function(&mut self, lang: crate::index::lang::Lang, text: String, offset: usize, plan: crate::domain::introduce_parameter::ParameterPlan, declaration: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::inline_function::{call_edit, edits, inline_calls};
        use crate::domain::introduce_parameter::apply;
        use crate::index::introduce_parameter::call_sites;
        use crate::index::lang::Lang;
        use crate::infra::project_files::DiskFiles;
        use crate::usecase::update_calls;
        const TITLE: &str = "Inline Function";
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(|r| r.to_path_buf()));
        let own = self.path.clone();
        let (function, params) = (plan.function.clone(), plan.params.len());
        let here = call_sites(lang, &text, &function, params);
        // Invoked on a call (not the declaration): that one can go alone.
        let this_call = here.iter().find(|c| c.name_range.start <= offset && offset <= c.name_range.end && !declaration.contains(&offset)).cloned();
        let files = root.clone().map(|root| DiskFiles { root, repository: None });
        let task = cx.background_spawn(async move {
            let Some(root) = root else { return Vec::new() };
            let files = DiskFiles { root, repository: None };
            update_calls::find(&files, &own, &function, &|path, text| Lang::from_path(path).map(|lang| call_sites(lang, text, &function, params)).unwrap_or_default())
        });
        cx.spawn_in(window, async move |this, cx| {
            let elsewhere = task.await;
            this.update_in(cx, |_, window, cx| {
                let calls = here.iter().filter(|c| !(declaration.start <= c.name_range.start && c.name_range.end <= declaration.end)).count() + elsewhere.iter().map(|f| f.calls.len()).sum::<usize>();
                let editor = cx.entity().downgrade();
                let message = format!("Function {}() has {calls} usages.", plan.function);
                let mut options: Vec<(&'static str, std::rc::Rc<dyn Fn(&mut Window, &mut gpui_kit::App)>)> = Vec::new();
                if let Some(call) = this_call {
                    let (editor, plan, text) = (editor.clone(), plan.clone(), text.clone());
                    options.push(("Inline This Only", std::rc::Rc::new(move |window, cx| {
                        editor
                            .update(cx, |this, cx| {
                                if this.text(cx) != text {
                                    return Self::hint(TITLE, "The file changed; try again", window, cx);
                                }
                                match call_edit(&plan, &text, &call) {
                                    Ok((range, body)) => {
                                        let new = apply(&text, vec![(range.clone(), body)]);
                                        this.replace_text(&text, &new, range.start, window, cx);
                                    }
                                    Err(message) => Self::hint(TITLE, message, window, cx),
                                }
                            })
                            .ok();
                    })));
                }
                options.push(("Inline All and Remove", std::rc::Rc::new(move |window, cx| {
                    editor
                        .update(cx, |this, cx| {
                            if this.text(cx) != text {
                                return Self::hint(TITLE, "The file changed; try again", window, cx);
                            }
                            // Every call must inline before anything is written.
                            let changes = match edits(&plan, &text, declaration.clone(), &here) {
                                Ok(changes) => changes,
                                Err(message) => return Self::hint(TITLE, message, window, cx),
                            };
                            if let Some(message) = elsewhere.iter().find_map(|f| inline_calls(&plan, &f.text, &f.calls).err()) {
                                return Self::hint(TITLE, message, window, cx);
                            }
                            let new = apply(&text, changes);
                            this.replace_text(&text, &new, declaration.start.min(new.len()), window, cx);
                            let written = files.as_ref().map_or_else(Vec::new, |files| {
                                update_calls::rewrite(files, &elsewhere, &|f| f.calls.iter().filter_map(|c| call_edit(&plan, &f.text, c).ok()).collect())
                            });
                            if !written.is_empty() {
                                cx.emit(FileEditorEvent::FilesWritten(written));
                            }
                        })
                        .ok();
                })));
                crate::ui::dialogs::choose(TITLE, message, Vec::new(), "Cancel", options, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Inline Property / Field: the references (this one only, when on
    /// one) become the initializer; with all of them, the declaration goes.
    /// The project's other files are searched first; a reference that can't
    /// be told apart stops it.
    pub(super) fn inline_field(&mut self, lang: crate::index::lang::Lang, text: String, field: crate::index::inline_field::Field, on: Option<crate::index::inline_field::Reference>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::introduce_parameter::apply;
        use crate::index::inline_field::{References, edits, references};
        use crate::index::lang::Lang;
        use crate::infra::project_files::DiskFiles;
        use crate::usecase::project_edits;
        let title = format!("Inline {}", field.kind);
        let here = references(lang, &text, &field, true);
        if let Some(&at) = here.unresolved.first() {
            let line = text[..at].matches('\n').count() + 1;
            return Self::hint(&title, format!("Cannot inline {} {}: the reference on line {line} can't be followed.", field.kind.to_lowercase(), field.name), window, cx);
        }
        let root = self.code_index.as_ref().and_then(|i| i.read(cx).root().map(|r| r.to_path_buf()));
        let own = self.path.clone();
        let files = root.clone().map(|root| DiskFiles { root, repository: None });
        let task = cx.background_spawn({
            let field = field.clone();
            async move {
                let Some(root) = root else { return Vec::new() };
                let files = DiskFiles { root, repository: None };
                project_edits::scan(&files, &own, &field.name, &|path, text| {
                    let found = references(Lang::from_path(path)?, text, &field, false);
                    (!found.refs.is_empty() || !found.imports.is_empty() || !found.unresolved.is_empty()).then_some(found)
                })
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let elsewhere: Vec<project_edits::FileChange<References>> = task.await;
            this.update_in(cx, |_, window, cx| {
                if let Some(file) = elsewhere.iter().find(|f| !f.found.unresolved.is_empty()) {
                    let line = file.text[..file.found.unresolved[0]].matches('\n').count() + 1;
                    return Self::hint(&title, format!("Cannot inline {} {}: the reference at {}:{line} can't be followed.", field.kind.to_lowercase(), field.name, file.path), window, cx);
                }
                let others: usize = elsewhere.iter().map(|f| f.found.refs.len()).sum();
                if others > 0 && !field.portable {
                    let file = &elsewhere.iter().find(|f| !f.found.refs.is_empty()).unwrap().path;
                    return Self::hint(&title, format!("Cannot inline {} {} into {file}: its initializer names things that may mean something else there.", field.kind.to_lowercase(), field.name), window, cx);
                }
                let usages = here.refs.len() + others;
                let editor = cx.entity().downgrade();
                let mut options: Vec<(&'static str, std::rc::Rc<dyn Fn(&mut Window, &mut gpui_kit::App)>)> = Vec::new();
                if let Some(reference) = on.clone() {
                    let (editor, field, text, title) = (editor.clone(), field.clone(), text.clone(), title.clone());
                    options.push(("Inline This Only", std::rc::Rc::new(move |window, cx| {
                        editor
                            .update(cx, |this, cx| {
                                if this.text(cx) != text {
                                    return Self::hint(&title, "The file changed; try again", window, cx);
                                }
                                let one = References { refs: vec![reference.clone()], ..Default::default() };
                                let new = apply(&text, edits(&field, &one));
                                this.replace_text(&text, &new, reference.range.start, window, cx);
                            })
                            .ok();
                    })));
                }
                let message = format!("{} {} has {usages} usages.", field.kind, field.name);
                let dialog_title = title.clone();
                options.push(("Inline All and Remove", std::rc::Rc::new(move |window, cx| {
                    editor
                        .update(cx, |this, cx| {
                            if this.text(cx) != text {
                                return Self::hint(&title, "The file changed; try again", window, cx);
                            }
                            let mut changes = edits(&field, &here);
                            changes.retain(|(r, _)| r.end <= field.declaration.start || r.start >= field.declaration.end);
                            changes.push((field.declaration.clone(), String::new()));
                            let new = apply(&text, changes);
                            this.replace_text(&text, &new, field.declaration.start.min(new.len()), window, cx);
                            let written = files.as_ref().map_or_else(Vec::new, |files| project_edits::write(files, &elsewhere, &|f| edits(&field, &f.found)));
                            if !written.is_empty() {
                                cx.emit(FileEditorEvent::FilesWritten(written));
                            }
                        })
                        .ok();
                })));
                crate::ui::dialogs::choose(dialog_title, message, Vec::new(), "Cancel", options, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Safe Delete (Alt+Delete): the declaration at the caret goes when
    /// nothing uses it; with usages, IntelliJ's "Usages Detected" offers to
    /// view them or delete anyway.
    fn safe_delete(&mut self, _: &editing::SafeDelete, window: &mut Window, cx: &mut Context<Self>) {
        const TITLE: &str = "Safe Delete";
        let Some(lang) = crate::index::lang::Lang::from_path(&self.path).filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let offset = self.state.read(cx).cursor();
        let Some(decl) = crate::index::safe_delete::declaration_at(lang, &text, offset) else {
            return Self::hint(TITLE, "Cannot perform refactoring. Caret should be positioned at the name of element to be deleted.", window, cx);
        };
        let Some(index) = self.code_index.clone() else { return };
        let task = index.read(cx).usages(self.path.clone(), text.clone(), decl.name_offset, cx);
        let path = self.path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let (_, usages) = task.await;
            // Uses outside the declaration itself (a recursive call isn't one).
            let first = text[..decl.lines.start].matches('\n').count() as u32;
            let last = first + text[decl.lines.clone()].trim_end_matches('\n').matches('\n').count() as u32;
            let uses: Vec<String> = usages
                .into_iter()
                .filter(|u| u.group != "Declarations" && !(u.path == path && (first..=last).contains(&u.line)))
                .map(|u| format!("{}:{}", u.path, u.line + 1))
                .collect();
            this.update_in(cx, |_, window, cx| {
                let editor = cx.entity().downgrade();
                let delete: std::rc::Rc<dyn Fn(&mut Window, &mut gpui_kit::App)> = {
                    let (editor, text, lines) = (editor.clone(), text.clone(), decl.lines.clone());
                    std::rc::Rc::new(move |window, cx| {
                        editor
                            .update(cx, |this, cx| {
                                if this.text(cx) != text {
                                    return Self::hint(TITLE, "The file changed; try again", window, cx);
                                }
                                let mut new = text.clone();
                                new.replace_range(lines.clone(), "");
                                this.replace_text(&text, &new, lines.start.min(new.len()), window, cx);
                            })
                            .ok();
                    })
                };
                let what = format!("{} '{}'", decl.kind.to_lowercase(), decl.name);
                if uses.is_empty() {
                    crate::ui::dialogs::choose(TITLE, format!("Delete {what}?"), Vec::new(), "Cancel", vec![("Delete", delete)], window, cx);
                    return;
                }
                let n = uses.len();
                let view: std::rc::Rc<dyn Fn(&mut Window, &mut gpui_kit::App)> = {
                    let (text, offset) = (text.clone(), decl.name_offset);
                    std::rc::Rc::new(move |_, cx| {
                        editor.update(cx, |_, cx| cx.emit(FileEditorEvent::FindUsages { text: text.clone(), offset })).ok();
                    })
                };
                let message = format!("{} '{}' has {n} usage{} that {} not safe to delete.", decl.kind, decl.name, if n == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" });
                crate::ui::dialogs::choose("Usages Detected", message, uses, "Cancel", vec![("Delete Anyway", delete), ("View Usages", view)], window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn introduce_constant_action(&mut self, _: &IntroduceConstant, window: &mut Window, cx: &mut Context<Self>) {
        self.introduce(crate::ui::introduce_dialog::Introduce::Constant, window, cx);
    }

    /// Refactor › Introduce Variable (Ctrl+Alt+V) / Constant (Ctrl+Alt+C):
    /// the selected expression, or with none the one chosen from those at
    /// the caret.
    pub fn introduce(&mut self, kind: crate::ui::introduce_dialog::Introduce, window: &mut Window, cx: &mut Context<Self>) {
        let Some((lang, _)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        match crate::index::introduce::expressions_at(lang, &text, selection) {
            Err(message) => Self::hint(Self::introduce_title(kind), format!("Cannot perform refactoring.\n{message}"), window, cx),
            Ok(ranges) if ranges.len() == 1 => self.introduce_for(kind, ranges[0].clone(), window, cx),
            Ok(ranges) => self.choose_expression(kind, &text, ranges, window, cx),
        }
    }

    fn introduce_title(kind: crate::ui::introduce_dialog::Introduce) -> &'static str {
        match kind {
            crate::ui::introduce_dialog::Introduce::Variable => "Introduce Variable",
            crate::ui::introduce_dialog::Introduce::Constant => "Introduce Constant",
        }
    }

    /// Introduce Variable or Constant for the expression at `range`: the
    /// dialog, then the edit, the caret after the new name.
    pub(super) fn introduce_for(&mut self, kind: crate::ui::introduce_dialog::Introduce, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::imports::ImportStyle;
        use crate::domain::introduce_variable::{introduce, introduce_constant, suggest_constant_names, suggest_names};
        use crate::ui::introduce_dialog::Introduce;
        let Some((lang, style)) = self.jvm_style() else { return };
        let title = Self::introduce_title(kind);
        let text = self.text(cx);
        let fail = |message: String, window: &mut Window, cx: &mut Context<Self>| Self::hint(title, format!("Cannot perform refactoring.\n{message}"), window, cx);
        // The plan, its names and occurrences, and the edit it makes.
        type Edit = Box<dyn Fn(&str, &crate::ui::introduce_dialog::IntroduceRequest) -> Result<crate::domain::introduce_variable::Introduced, String>>;
        let (mut typed, mut visibility) = (false, false);
        let (suggestions, occurrences, has_type, edit): (Vec<String>, usize, bool, Edit) = match kind {
            Introduce::Variable => match crate::index::introduce::extraction(lang, &text, range) {
                Err(message) => return fail(message, window, cx),
                Ok((extraction, taken)) => {
                    typed = extraction.kotlin_type.is_some();
                    (
                        suggest_names(style, &extraction.source, &|n| taken.iter().any(|t| t == n)),
                        extraction.occurrences.len(),
                        extraction.java_type.is_some(),
                        Box::new(move |text, r| introduce(style, text, &extraction, r.all, &r.name, r.modifier, r.explicit_type)),
                    )
                }
            },
            Introduce::Constant => match crate::index::introduce::constant(lang, &text, range) {
                Err(message) => return fail(message, window, cx),
                Ok((plan, taken)) => {
                    visibility = style == ImportStyle::Java && !plan.interface;
                    (
                        suggest_constant_names(style, &plan.source, plan.string_content.as_deref(), &|n| taken.iter().any(|t| t == n)),
                        plan.occurrences.len(),
                        plan.java_type.is_some(),
                        Box::new(move |text, r| introduce_constant(style, text, &plan, r.all, &r.name, r.visibility)),
                    )
                }
            },
        };
        if style == ImportStyle::Java && !has_type {
            return fail("Cannot perform refactoring without a type".into(), window, cx);
        }
        let editor = cx.entity().downgrade();
        crate::ui::introduce_dialog::open(kind, style, suggestions, occurrences, typed, visibility, move |request, window, cx| {
            editor
                .update(cx, |this, cx| {
                    let text = this.text(cx);
                    match edit(&text, &request) {
                        Err(message) => Self::hint(title, message, window, cx),
                        Ok(done) => this.state.update(cx, |state, cx| {
                            state.set_selected_range(done.range.clone(), cx);
                            state.replace(&done.text, window, cx);
                            // The caret after the new name, nothing selected: the
                            // dialog already named it (typing over a selected name
                            // would rename the declaration alone).
                            state.set_selected_range(done.name_at.end..done.name_at.end, cx);
                        }),
                    }
                })
                .ok();
        }, window, cx);
    }

    fn surround_with_action(&mut self, _: &SurroundWith, window: &mut Window, cx: &mut Context<Self>) {
        self.surround_with(window, cx);
    }

    /// Code › Surround With (Ctrl+Alt+T): the popup of surrounders for the
    /// selected lines (or the caret's), and an expression's when the
    /// selection is part of a line.
    pub fn surround_with(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        let lines = crate::domain::surround::lines_of(&text, selection.clone());
        let line_text = &text[lines.clone()];
        let selected = text[selection.clone()].trim();
        // Part of one line selected: an expression too.
        let expression = (!selected.is_empty() && !selected.contains('\n') && selected != line_text.trim()).then(|| {
            let s = &text[selection.clone()];
            let start = selection.start + (s.len() - s.trim_start().len());
            start..start + selected.len()
        });
        if lines.is_empty() || line_text.trim().is_empty() && expression.is_none() {
            return Self::hint("Surround With", "Cannot perform refactoring.\nNothing to surround", window, cx);
        }
        self.choose_surrounder(style, lines, expression, window, cx);
    }

    /// Code › Surround with Live Template (Ctrl+Alt+J): the popup of the
    /// templates with `$SELECTION$`, for the selection (whole lines when
    /// it spans them, else the selected text) or the caret's line.
    pub fn surround_with_live_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, style)) = self.jvm_style().filter(|_| !self.read_only()) else { return };
        let text = self.text(cx);
        let selection = self.state.read(cx).selected_range();
        let lines = crate::domain::surround::lines_of(&text, selection.clone());
        let partial = !selection.is_empty() && !text[selection.clone()].contains('\n') && text[selection.clone()].trim() != text[lines.clone()].trim();
        let range = if partial { selection } else { lines };
        let templates = crate::domain::live_templates::surrounding(style == crate::domain::imports::ImportStyle::Kotlin);
        if templates.is_empty() || text[range.clone()].trim().is_empty() {
            return Self::hint("Surround with Live Template", "No templates", window, cx);
        }
        self.choose_live_template(templates, range, window, cx);
    }

    /// Applies a surrounder to the lines or the expression at `range`.
    pub(super) fn surround(&mut self, surrounder: crate::domain::surround::Surrounder, range: Range<usize>, expression: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::surround::{indent_unit, surround_expression, surround_lines};
        let Some((_, style)) = self.jvm_style() else { return };
        let text = self.text(cx);
        let done = if expression { surround_expression(surrounder, &text, range) } else { surround_lines(style, surrounder, &text, range, &indent_unit(&text)) };
        let select = done.stops.first().cloned().unwrap_or(done.select.clone());
        self.state.update(cx, |state, cx| {
            state.set_selected_range(done.range.clone(), cx);
            state.replace(&done.text, window, cx);
            state.set_selected_range(select, cx);
        });
        if done.stops.len() > 1 {
            let stops = done.stops.into_iter().map(|r| vec![r]).collect();
            self.start_template(crate::domain::live_templates::Session { stops, current: 0, end: done.end }, cx);
        }
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

    pub fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).text().to_string()
    }

    /// Focuses the text, as switching to the tab does.
    pub fn focus(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        let handle = gpui_kit::Focusable::focus_handle(self.state.read(cx), cx);
        window.focus(&handle, cx);
    }

    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        // Compared piece by piece: tabs ask on every frame, and copying a
        // long file out of the editor each time showed up while typing.
        !self.read_only() && *self.state.read(cx).text() != self.saved.as_str()
    }

    /// After git changed the working tree (checkout, pull, rollback…):
    /// takes the file's new text from disk unless it has unsaved edits, as
    /// IntelliJ reloads its editors, and the new HEAD version for the
    /// change markers.
    pub fn sync_with_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() || self.error.is_some() {
            return;
        }
        let (repository, path) = (self.repository.clone(), self.own.clone());
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
                this.base = base.unwrap_or_default().into();
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
        let (base, text) = (self.base.clone(), self.state.read(cx).text().clone());
        self.markers_task = Some(cx.spawn(async move |this, cx| {
            let hunks = cx.background_spawn(async move { diff::commit_hunks(&base, &text.to_string()) }).await;
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
        match diff::stage_lines(&self.repository, &self.own, &text, hunk.new.clone()) {
            Ok(()) => {
                self.popup = None;
                cx.emit(FileEditorEvent::FilesChanged);
            }
            Err(error) => self.set_notice(error.to_string(), cx),
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

    fn save(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        self.optimize_imports_on_the_fly(window, cx);
        self.save_now(cx);
    }

    /// Settings › Editor › Auto Import › Optimize imports on the fly: a
    /// save drops unused imports and sorts the rest, silently, unless the
    /// caret is in the import list (someone is typing one).
    fn optimize_imports_on_the_fly(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::index::lang::Lang;
        let lang = Lang::from_path(&self.path);
        let diff = &crate::settings::Settings::get(cx).diff;
        let on = match lang {
            Some(Lang::Java) => diff.optimize_imports_java,
            Some(Lang::Kotlin) => diff.optimize_imports_kotlin,
            _ => false,
        };
        let style = lang.filter(|_| on && !self.read_only()).and_then(crate::infra::index_code_model::IndexCodeModel::import_style);
        let Some(style) = style else { return };
        let text = self.text(cx);
        let Some(edit) = crate::domain::imports::optimize(style, &text) else { return };
        let cursor = self.state.read(cx).cursor();
        if edit.range.contains(&cursor) {
            return;
        }
        let cursor = if cursor >= edit.range.end { cursor + edit.text.len() - edit.range.len() } else { cursor };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(edit.range.clone(), cx);
            state.replace(&edit.text, window, cx);
            state.set_selected_range(cursor..cursor, cx);
        });
    }

    /// Shows a failed action's message for a few seconds.
    fn set_notice(&mut self, notice: impl Into<String>, cx: &mut Context<Self>) {
        let notice = notice.into();
        self.notice = Some(notice.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(6)).await;
            this.update(cx, |this, cx| {
                if this.notice.as_deref() == Some(notice.as_str()) {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Saves unsaved text, as IntelliJ does when a tab closes.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let text = self.text(cx);
        // Nothing typed since the last save or load: writing would put a
        // stale buffer over a file changed outside meanwhile.
        if text == self.saved {
            return;
        }
        match std::fs::write(self.repository.root().join(&self.own), &text) {
            Ok(()) => {
                self.format = TextFormat::of(&text);
                self.saved = text;
                cx.emit(FileEditorEvent::FilesChanged);
                cx.emit(FileEditorEvent::Saved(self.path.clone()));
            }
            Err(error) => self.set_notice(error.to_string(), cx),
        }
        cx.notify();
    }

    /// Git › Open on GitHub: the file at this revision (HEAD's commit for the
    /// working tree), with the selected lines highlighted.
    pub(crate) fn open_on_hosting(&mut self, _: &OpenOnHosting, _: &mut Window, cx: &mut Context<Self>) {
        let Some(web) = crate::git::hosting::web_repo(&self.repository) else {
            self.set_notice("No GitHub, GitLab or Bitbucket remote found", cx);
            cx.notify();
            return;
        };
        let revision = match &self.revision {
            Some(revision) => revision.clone(),
            None => self.repository.run(["rev-parse", "HEAD"]).map(|h| h.trim().to_owned()).unwrap_or_else(|_| "HEAD".into()),
        };
        cx.open_url(&web.file_url(&revision, &self.own, Some(self.selected_lines(cx))));
    }

    /// Create Gist: the selection, or the whole file when nothing is selected.
    pub(crate) fn create_gist(&mut self, _: &CreateGist, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let range: Range<usize> = self.state.read(cx).selected_range();
        let content = if range.is_empty() { text } else { text.get(range).unwrap_or_default().to_owned() };
        let name = self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path).to_owned();
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
        if self.annotation.is_some() {
            self.load_annotations(true, cx);
        }
        self.save(&SaveFile, window, cx);
    }

    fn show_selection_history(&mut self, _: &ShowSelectionHistory, _: &mut Window, cx: &mut Context<Self>) {
        let lines = self.selected_lines(cx);
        cx.emit(FileEditorEvent::SelectionHistory { path: self.path.clone(), lines });
    }

    /// Local History › Show History for Selection: saved first, so the
    /// history's newest version is what the editor shows.
    fn show_local_selection_history(&mut self, _: &ShowLocalSelectionHistory, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        self.save_now(cx);
        let lines = self.selected_lines(cx);
        cx.emit(FileEditorEvent::LocalSelectionHistory { path: self.path.clone(), lines });
    }

    /// The editor's whole text.
    pub fn current_text(&self, cx: &gpui_kit::App) -> String {
        self.text(cx)
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
        let (state, hit, line) = (self.state.clone(), self.gutter_click.clone(), self.gutter_line.clone());
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (state, hit, line) = (state.clone(), hit.clone(), line.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    if e.button != MouseButton::Right {
                        line.set(None);
                        return;
                    }
                    let state = state.read(cx);
                    let input = state.input_bounds();
                    let text_left = state.visible_row_range().and_then(|rows| {
                        let rope = state.text();
                        let at = rope.line_start_offset(rows.start.min(rope.lines_len().saturating_sub(1)));
                        state.range_to_bounds(&(at..at)).map(|b| b.left())
                    });
                    let on_gutter = text_left.is_some_and(|left| input.contains(&e.position) && e.position.x >= input.left() && e.position.x < left);
                    hit.set(on_gutter);
                    // The line under the click: the last one starting above it.
                    let clicked = on_gutter.then(|| state.visible_row_range()).flatten().and_then(|rows| {
                        let rope = state.text();
                        let lines = rope.lines_len().max(1);
                        (rows.start..rows.end.min(lines))
                            .take_while(|&l| {
                                let at = rope.line_start_offset(l);
                                state.range_to_bounds(&(at..at)).is_some_and(|b| b.top() <= e.position.y)
                            })
                            .last()
                    });
                    line.set(clicked);
                });
            },
        )
        .absolute()
        .inset_0()
    }

    pub fn annotation(&self) -> Option<&Annotation> {
        self.annotation.as_ref()
    }

    /// The file bookmarks are kept under: its absolute path (none for a
    /// revision or an image).
    pub fn bookmark_file(&self) -> Option<String> {
        if self.revision.is_some() || self.image.is_some() {
            return None;
        }
        let path = std::path::Path::new(&self.path);
        Some(if path.is_absolute() { self.path.clone() } else { self.repository.root().join(&self.own).to_string_lossy().into_owned() })
    }

    /// The line a bookmark action is for: the right-clicked gutter line,
    /// else the caret's.
    fn bookmark_line(&self, cx: &gpui_kit::App) -> usize {
        self.gutter_line.take().unwrap_or_else(|| self.state.read(cx).cursor_position().line as usize)
    }

    fn line_text(&self, line: usize, cx: &gpui_kit::App) -> String {
        self.text(cx).lines().nth(line).unwrap_or_default().to_owned()
    }

    /// Toggle Bookmark (F11).
    fn toggle_bookmark(&mut self, _: &ToggleBookmark, _: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        let line = self.bookmark_line(cx);
        let text = self.line_text(line, cx);
        crate::bookmarks::Bookmarks::update(cx, |b| b.toggle(&file, Some(line as u32), &text));
    }

    /// Toggle Bookmark with Mnemonic (Ctrl+F11): the mnemonic chooser.
    fn toggle_bookmark_with_mnemonic(&mut self, _: &ToggleBookmarkWithMnemonic, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        let line = self.bookmark_line(cx);
        let text = self.line_text(line, cx);
        crate::ui::bookmark_dialog::choose_mnemonic(file, Some(line as u32), text, window, cx);
    }

    /// Room in the gutter for the implementation markers and bookmarks.
    fn apply_gutter_extra(&mut self, cx: &mut Context<Self>) {
        let shown = crate::settings::Settings::get(cx).diff.show_gutter_icons;
        let bookmarked = self.bookmark_file().is_some_and(|f| crate::bookmarks::Bookmarks::get(cx).of_file(&f).next().is_some());
        let slots = if shown { self.hierarchy_slots + bookmarked as usize + !self.previews.is_empty() as usize + !self.test_marks.is_empty() as usize } else { 0 };
        let extra = if slots == 0 { px(0.) } else { px(GUTTER_ICON_GAP + GUTTER_ICON_SLOT * slots as f32) };
        self.state.update(cx, |state, cx| state.set_gutter_extra(extra, cx));
    }

    /// The file's bookmarks in the gutter, after the implementation markers:
    /// a bookmark icon, or a mnemonic's letter in a box, as IntelliJ's.
    fn bookmark_strip(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let file = self.bookmark_file()?;
        let marks: Vec<(u32, Option<char>)> = crate::bookmarks::Bookmarks::get(cx).of_file(&file).map(|b| (b.line, b.mnemonic)).collect();
        if marks.is_empty() || !crate::settings::Settings::get(cx).diff.show_gutter_icons {
            return None;
        }
        let state = self.state.clone();
        let slot = self.hierarchy_slots;
        let palette = cx.palette().clone();
        let mono = gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone();
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let placed: Vec<(Bounds<Pixels>, Option<char>)> = {
                        let state = state.read(cx);
                        let (Some(line_height), Some(visible), Some(left)) = (state.line_height(), state.visible_row_range(), state.gutter_extra_left()) else { return };
                        let rope = state.text();
                        let lines = rope.lines_len().max(1);
                        marks
                            .iter()
                            .filter(|(line, _)| (*line as usize) >= visible.start && (*line as usize) <= visible.end && (*line as usize) < lines)
                            .filter_map(|(line, mnemonic)| {
                                let at = rope.line_start_offset(*line as usize);
                                let top = state.range_to_bounds(&(at..at))?.top();
                                let icon = px(GUTTER_ICON);
                                let rect = Bounds::new(point(left + px(GUTTER_ICON_GAP + 1. + GUTTER_ICON_SLOT * slot as f32), top + (line_height - icon) / 2.), size(icon, icon));
                                bounds.contains(&rect.center()).then_some((rect, *mnemonic))
                            })
                            .collect()
                    };
                    for (rect, mnemonic) in placed {
                        match mnemonic {
                            Some(c) => {
                                window.paint_quad(gpui_kit::quad(rect, px(3.), palette.panel, px(1.), palette.text_secondary, Default::default()));
                                let label: gpui_kit::SharedString = c.to_string().into();
                                let run = gpui_kit::TextRun { len: label.len(), font: gpui_kit::font(mono.clone()), color: palette.text, background_color: None, underline: None, strikethrough: None };
                                let shaped = window.text_system().shape_line(label, px(10.), &[run], None);
                                let x = rect.left() + (rect.size.width - shaped.width) / 2.;
                                let _ = shaped.paint(point(x, rect.top()), rect.size.height, gpui_kit::TextAlign::Left, None, window, cx);
                            }
                            None => {
                                let bookmark = icons::TW_BOOKMARKS;
                                for (path, rgba) in if palette.dark { bookmark.dark } else { bookmark.light } {
                                    let color: gpui_kit::Hsla = gpui_kit::rgba(*rgba).into();
                                    window.paint_svg(rect, (*path).into(), None, gpui_kit::TransformationMatrix::unit(), color, cx).ok();
                                }
                            }
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
    }

    /// Moves the file's bookmarks along with their lines after an edit.
    fn follow_bookmarks(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.bookmark_file() else { return };
        if crate::bookmarks::Bookmarks::get(cx).of_file(&file).next().is_none() {
            return;
        }
        let text = self.text(cx);
        let lines: Vec<&str> = text.lines().collect();
        let moves = {
            let mut probe = crate::bookmarks::Bookmarks { list: crate::bookmarks::Bookmarks::get(cx).list.clone() };
            probe.follow(&file, &lines)
        };
        if moves {
            crate::bookmarks::Bookmarks::update(cx, |b| {
                b.follow(&file, &lines);
            });
        }
    }

    /// The gutter menu's view toggles (Settings › Editor › General).
    fn toggle_view(&mut self, f: fn(&mut crate::settings::DiffSettings), cx: &mut Context<Self>) {
        crate::settings::Settings::update(cx, |settings| f(&mut settings.diff));
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
        let path = self.own.clone();
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
        match self.repository.run(["log", "-1", "--format=%H", &rev, "--", &self.own]) {
            Ok(hash) if !hash.trim().is_empty() => cx.emit(FileEditorEvent::SelectCommit(hash.trim().to_owned())),
            _ => self.set_notice("The file isn't in any commit yet", cx),
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
        cx.emit(FileEditorEvent::OpenDiff(DiffSource::Clipboard { path: self.path.clone(), text, title: "Clipboard".to_owned() }));
    }

    fn show_diff(&mut self, _: &ShowFileDiff, _: &mut Window, cx: &mut Context<Self>) {
        let source = match &self.revision {
            Some(rev) => DiffSource::Commit { hash: rev.clone(), path: self.path.clone(), old_path: None },
            None => DiffSource::WorkingTree { path: self.path.clone(), unversioned: false },
        };
        cx.emit(FileEditorEvent::OpenDiff(source));
    }
}

impl FileEditor {
    /// Android Studio's banner over a values file with strings: "Edit
    /// translations for all locales in the translations editor."
    /// IntelliJ's banner over a decompiled class, with Download Sources.
    fn decompiled_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if !self.path.replace('\\', "/").contains("/decompiled/gradle/") {
            return None;
        }
        let coordinate = crate::index::libraries::decompiled_coordinate(std::path::Path::new(&self.path))?;
        let palette = cx.palette().clone();
        let status = self.sources_status.clone();
        let busy = status.as_ref().is_some_and(|(busy, _)| *busy);
        Some(
            h_flex()
                .px_2()
                .h(px(30.))
                .gap_4()
                .text_sm()
                .bg(palette.toolbar)
                .border_b_1()
                .border_color(palette.border)
                .child(div().flex_1().child(format!("Decompiled .class file: no sources were found for {coordinate}")))
                .when_some(status.map(|(_, text)| text), |el, text| el.child(div().text_color(palette.text_secondary).child(text)))
                .when(!busy, |el| el.child(div().id("download-sources").text_color(palette.link).cursor_pointer().child("Download Sources").on_click(cx.listener(move |this, _, _, cx| this.download_sources(coordinate.clone(), cx))))),
        )
    }

    /// Downloads the library's sources jar, rescans the libraries, then
    /// opens this class's source in place of the stub.
    fn download_sources(&mut self, coordinate: String, cx: &mut Context<Self>) {
        let mut parts = coordinate.splitn(3, ':');
        let (Some(group), Some(artifact), Some(version)) = (parts.next().map(str::to_owned), parts.next().map(str::to_owned), parts.next().map(str::to_owned)) else { return };
        self.sources_status = Some((true, "Downloading sources…".into()));
        cx.notify();
        let task = cx.background_spawn(async move { crate::infra::sources_download::download(&group, &artifact, &version) });
        // The source file the stub stands for, once the scan unpacks it.
        let stub = self.path.replace('\\', "/");
        let wanted: Vec<String> = ["kt", "java"]
            .iter()
            .filter_map(|ext| stub.rsplit_once('.').map(|(base, _)| format!("{}.{ext}", base.replacen("/decompiled/gradle/", "/sources/gradle/", 1))))
            .collect();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let ok = this
                .update(cx, |this, cx| match result {
                    Err(e) => {
                        this.sources_status = Some((false, e));
                        cx.notify();
                        false
                    }
                    Ok(_) => {
                        this.sources_status = Some((true, "Indexing the sources…".into()));
                        if let Some(index) = this.code_index.clone() {
                            index.update(cx, |index, cx| index.rescan_libraries(cx));
                        }
                        cx.notify();
                        true
                    }
                })
                .unwrap_or(false);
            if !ok {
                return;
            }
            for _ in 0..240 {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                if let Some(found) = wanted.iter().find(|p| std::path::Path::new(p).is_file()).cloned() {
                    this.update(cx, |this, cx| {
                        this.sources_status = None;
                        let name = found.rsplit('/').next().unwrap_or_default().to_owned();
                        let target = crate::index::nav::Target { path: found, line: 0, col: 0, name, label: String::new(), container: None };
                        cx.emit(FileEditorEvent::Navigate { targets: vec![target], at: None });
                        // The source replaces the stub, as IntelliJ's.
                        cx.emit(FileEditorEvent::Deleted);
                    })
                    .ok();
                    return;
                }
            }
            this.update(cx, |this, cx| {
                this.sources_status = Some((false, "Sources downloaded; this class isn't in them".into()));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn strings_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        // Not for read-only files (the SDK's framework strings).
        if self.strings_banner_hidden || self.revision.is_some() || self.read_only() || !self.path.ends_with(".xml") {
            return None;
        }
        let folder = std::path::Path::new(&self.path).parent()?.file_name()?.to_string_lossy().into_owned();
        if !(folder == "values" || folder.starts_with("values-")) || !self.state.read(cx).value().contains("<string ") {
            return None;
        }
        let palette = cx.palette().clone();
        let link = |id: &'static str, label: &'static str| div().id(id).text_color(palette.link).cursor_pointer().child(label);
        let path = self.path.clone();
        Some(
            h_flex()
                .px_2()
                .h(px(30.))
                .gap_4()
                .text_sm()
                .bg(palette.toolbar)
                .border_b_1()
                .border_color(palette.border)
                .child(div().flex_1().child("Edit translations for all locales in the translations editor."))
                .child(link("strings-open-editor", "Open editor").on_click(cx.listener(move |_, _, _, cx| cx.emit(FileEditorEvent::OpenTranslations(path.clone())))))
                .child(link("strings-hide", "Hide notification").on_click(cx.listener(|this, _, _, cx| {
                    this.strings_banner_hidden = true;
                    cx.notify();
                }))),
        )
    }
}

impl Render for FileEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let java_file = self.path.ends_with(".java");
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
        let gutter_line = self.gutter_line.clone();
        let bookmark_file = self.bookmark_file();
        let bookmark_strip = self.bookmark_strip(cx);
        let breakpoint_strip = self.breakpoint_strip(cx);
        let (preview_strip, test_strip) = if crate::settings::Settings::get(cx).diff.show_gutter_icons {
            let bookmarked = self.bookmark_file().is_some_and(|f| crate::bookmarks::Bookmarks::get(cx).of_file(&f).next().is_some());
            let slot = self.hierarchy_slots + bookmarked as usize;
            (self.preview_strip(slot, cx), self.test_strip(slot + !self.previews.is_empty() as usize, cx))
        } else {
            (None, None)
        };
        let icons_shown = crate::settings::Settings::get(cx).diff.show_gutter_icons;
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
        let vector_buttons = self.vector_mode_buttons(cx);
        let vector_pane = self.vector_pane(cx);
        let manifest_pane = self.manifest_pane(cx);
        let manifest_tabs = self.manifest_tabs(cx);
        // Design or the merged manifest shows instead of the text.
        let vector_design = self.vector_design_only() || manifest_pane.is_some();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            // Live templates take Tab, Shift+Tab, Enter and Escape first.
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::IndentInline, window, cx| {
                if this.template_tab(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::OutdentInline, window, cx| {
                if this.template.is_some() && this.template_tab(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Enter, window, cx| {
                if !this.state.read(cx).is_completion_menu_open(cx) && this.template_enter(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| {
                if this.template_escape(cx) {
                    cx.stop_propagation();
                } else if !this.state.read(cx).is_completion_menu_open(cx) && this.close_parameter_info(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ParameterInfo, window, cx| this.show_parameter_info(true, window, cx)))
            .on_action(cx.listener(|this, _: &overrides::OverrideMethods, window, cx| this.override_methods(false, window, cx)))
            .on_action(cx.listener(|this, _: &overrides::ImplementMethods, window, cx| this.override_methods(true, window, cx)))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::show_context_actions))
            .on_action(cx.listener(Self::run_test_at_caret))
            .on_action(cx.listener(Self::debug_test_at_caret))
            .on_action(cx.listener(Self::toggle_line_breakpoint))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::comment_line))
            .on_action(cx.listener(Self::comment_block))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::move_statement_up))
            .on_action(cx.listener(Self::inline))
            .on_action(cx.listener(Self::unwrap_remove))
            .on_action(cx.listener(Self::safe_delete))
            .on_action(cx.listener(Self::collapse_region))
            .on_action(cx.listener(Self::expand_region))
            .on_action(cx.listener(Self::collapse_all_regions))
            .on_action(cx.listener(Self::expand_all_regions))
            .on_action(cx.listener(Self::move_element_left))
            .on_action(cx.listener(Self::move_element_right))
            .on_action(cx.listener(Self::move_statement_down))
            .on_action(cx.listener(Self::extend_selection))
            .on_action(cx.listener(Self::shrink_selection))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::start_new_line))
            .on_action(cx.listener(Self::toggle_case))
            .on_action(cx.listener(Self::run_to_cursor))
            .on_action(cx.listener(Self::evaluate_expression))
            .on_action(cx.listener(|this, _: &NextChange, window, cx| this.go_to_change(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousChange, window, cx| this.go_to_change(false, window, cx)))
            .on_action(cx.listener(Self::rollback_lines))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(|this, _: &TriggerCompletion, window, cx| {
                if !this.read_only() {
                    this.state.update(cx, |state, cx| state.trigger_completion(window, cx));
                }
            }))
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
            .on_action(cx.listener(Self::rename_element))
            .on_action(cx.listener(Self::optimize_imports_action))
            .on_action(cx.listener(|this, _: &ReformatCode, window, cx| this.reformat_code(window, cx)))
            .on_action(cx.listener(|this, _: &ReformatFile, window, cx| this.reformat_file(window, cx)))
            .on_action(cx.listener(Self::introduce_variable_action))
            .on_action(cx.listener(Self::introduce_constant_action))
            .on_action(cx.listener(Self::extract_method_action))
            .on_action(cx.listener(Self::introduce_parameter_action))
            .on_action(cx.listener(Self::change_signature_action))
            .on_action(cx.listener(|this, _: &CallHierarchy, _, cx| {
                let (text, offset) = (this.text(cx), this.state.read(cx).cursor());
                cx.emit(FileEditorEvent::CallHierarchy { text, offset });
            }))
            .on_action(cx.listener(|this, _: &TypeHierarchy, _, cx| {
                let (text, offset) = (this.text(cx), this.state.read(cx).cursor());
                cx.emit(FileEditorEvent::TypeHierarchy { text, offset });
            }))
            .on_action(cx.listener(|this, _: &MethodHierarchy, _, cx| {
                let (text, offset) = (this.text(cx), this.state.read(cx).cursor());
                cx.emit(FileEditorEvent::MethodHierarchy { text, offset });
            }))
            .on_action(cx.listener(Self::surround_with_action))
            .on_action(cx.listener(|this, _: &SurroundWithLiveTemplate, window, cx| this.surround_with_live_template(window, cx)))
            .on_action(cx.listener(|this, _: &Generate, window, cx| this.generate(window, cx)))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(|this, _: &OpenFind, window, cx| this.open_find(false, window, cx)))
            .on_action(cx.listener(|this, _: &OpenReplace, window, cx| this.open_find(true, window, cx)))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::show_selection_history))
            .on_action(cx.listener(Self::show_local_selection_history))
            .on_action(cx.listener(Self::annotate))
            .on_action(cx.listener(Self::toggle_line_numbers))
            .on_action(cx.listener(Self::toggle_bookmark))
            .on_action(cx.listener(Self::toggle_bookmark_with_mnemonic))
            .on_action(cx.listener(|this, _: &CopyAbsolutePath, _, cx| {
                let path = std::path::Path::new(&this.path);
                let full = if path.is_absolute() { path.to_path_buf() } else { this.repository.root().join(&this.own) };
                let text = full.to_string_lossy().into_owned();
                let text = if cfg!(windows) { text.replace('/', "\\") } else { text };
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
            }))
            .on_action(cx.listener(|_, _: &CopyPathReference, _, cx| cx.emit(FileEditorEvent::CopyPathReference)))
            .on_action(cx.listener(|this, _: &ToggleSoftWrap, _, cx| this.toggle_view(|d| d.soft_wrap = !d.soft_wrap, cx)))
            .on_action(cx.listener(|this, _: &ToggleGutterIcons, _, cx| this.toggle_view(|d| d.show_gutter_icons = !d.show_gutter_icons, cx)))
            .on_action(cx.listener(|this, _: &ToggleIndentGuides, _, cx| this.toggle_view(|d| d.show_indent_guides = !d.show_indent_guides, cx)))
            .on_action(cx.listener(|this, _: &ToggleWhitespaces, _, cx| this.toggle_view(|d| d.show_whitespaces = !d.show_whitespaces, cx)))
            .on_action(cx.listener(|_, _: &ConfigureSoftWraps, window, cx| crate::ui::settings_dialog::open_page(None, "General", window, cx)))
            .on_action(cx.listener(|_, _: &ConfigureGutterIcons, window, cx| crate::ui::settings_dialog::open_page(None, "Gutter Icons", window, cx)))
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
                    .children(vector_buttons)
                    .when(!has_image, |el| el.child(tool_button("editor-diff", icons::VCS_DIFF, "Show Diff").on_click(cx.listener(
                        |this, _, window, cx| this.show_diff(&ShowFileDiff, window, cx),
                    ))))
            )
            .when_some(self.error.clone().or_else(|| self.notice.clone()), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(error))
            })
            .when_some(self.strings_banner(cx), |el, banner| el.child(banner))
            .when_some(self.decompiled_banner(cx), |el, banner| el.child(banner))
            .child(self.find.clone())
            .when_some(self.annotation.as_ref().and_then(|a| a.error.clone()), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(format!("Cannot annotate: {error}")))
            })
            .when_some(image_body, |el, body| el.child(body))
            .when(!has_image, |el| el.child(
                div().relative().flex_1().min_h_0().child(
                    h_flex().size_full().children(annotation_gutter).child(div().flex_1().min_w_0().h_full().when(vector_design, |el| el.flex_none().w(px(0.)).overflow_hidden()).text_size(px(crate::ui::diff_panes::font_size())).child(
                    Editor::new(&self.state)
                        .h_full()
                        .bordered(false)
                        .readonly(read_only)
                        .context_menu(move |menu: NativeMenu, _, cx| {
                            let java_file = java_file;
                            // The gutter has IntelliJ's gutter menu instead.
                            if gutter_click.replace(false) {
                                let view = crate::settings::Settings::get(cx).diff.clone();
                                let appearance = NativeMenu::new()
                                    .menu_with_check("Show Line Numbers", view.show_line_numbers, Box::new(ToggleLineNumbers))
                                    .menu_with_check("Show Gutter Icons", view.show_gutter_icons, Box::new(ToggleGutterIcons))
                                    .menu_with_check("Show Indent Guides", view.show_indent_guides, Box::new(ToggleIndentGuides))
                                    .menu_with_check("Show Whitespaces", view.show_whitespaces, Box::new(ToggleWhitespaces));
                                let bookmarked = bookmark_file
                                    .as_ref()
                                    .zip(gutter_line.get())
                                    .and_then(|(f, l)| crate::bookmarks::Bookmarks::get(cx).at(f, Some(l as u32)).map(|b| b.mnemonic.is_some()));
                                return menu
                                    .menu(if annotated { "Close Annotations" } else { "Annotate with Git Blame" }, Box::new(AnnotateFile))
                                    .separator()
                                    .menu_with_disabled(if bookmarked.is_some() { "Remove Bookmark" } else { "Add Bookmark" }, bookmark_file.is_none(), Box::new(ToggleBookmark))
                                    .menu_with_disabled(
                                        if bookmarked == Some(true) { "Edit Mnemonic…" } else { "Add Mnemonic Bookmark…" },
                                        bookmark_file.is_none(),
                                        Box::new(ToggleBookmarkWithMnemonic),
                                    )
                                    .separator()
                                    .menu_with_check("Soft-Wrap", view.soft_wrap, Box::new(ToggleSoftWrap))
                                    .menu("Configure Soft Wraps…", Box::new(ConfigureSoftWraps))
                                    .separator()
                                    .submenu("Appearance", appearance)
                                    .menu("Configure Gutter Icons…", Box::new(ConfigureGutterIcons));
                            }
                            let menu = menu
                                .menu("Copy", Box::new(Copy))
                                .menu_with_disabled("Cut", read_only, Box::new(Cut))
                                .menu_with_disabled("Paste", read_only, Box::new(Paste))
                                .menu("Select All", Box::new(SelectAll))
                                .submenu("Copy / Paste Special", NativeMenu::new().menu("Copy Path/Reference…", Box::new(CopyPathReference)))
                                .separator()
                                .menu("Go to Declaration", Box::new(GotoDeclaration))
                                .menu("Find Usages", Box::new(FindUsages))
                                .menu("Type Hierarchy", Box::new(TypeHierarchy))
                                .menu("Method Hierarchy", Box::new(MethodHierarchy))
                                .menu("Call Hierarchy", Box::new(CallHierarchy))
                                .menu_with_disabled("Generate…", read_only, Box::new(Generate))
                                .menu_with_disabled("Surround With…", read_only, Box::new(SurroundWith))
                                .menu_with_disabled("Unwrap/Remove…", read_only, Box::new(editing::UnwrapRemove))
                                .menu_with_disabled("Reformat Code", read_only, Box::new(ReformatCode))
                                .menu_with_disabled("Reformat File…", read_only, Box::new(ReformatFile))
                                .submenu(
                                    "Refactor",
                                    NativeMenu::new()
                                        .menu_with_disabled("Rename…", read_only, Box::new(RenameElement))
                                        .menu_with_disabled("Introduce Variable…", read_only, Box::new(IntroduceVariable))
                                        .menu_with_disabled("Introduce Constant…", read_only, Box::new(IntroduceConstant))
                                        .menu_with_disabled("Change Signature…", read_only, Box::new(ChangeSignature))
                                        .menu_with_disabled("Introduce Parameter…", read_only, Box::new(IntroduceParameter))
                                        .menu_with_disabled("Inline…", read_only, Box::new(editing::Inline))
                                        .menu_with_disabled("Safe Delete…", read_only, Box::new(editing::SafeDelete))
                                        .menu_with_disabled(if java_file { "Extract Method…" } else { "Extract Function…" }, read_only, Box::new(ExtractMethod)),
                                )
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
                            menu.submenu("Git", git).submenu(
                                "Local History",
                                NativeMenu::new()
                                    .menu("Show History", Box::new(crate::ui::workspace::ShowLocalHistory))
                                    .menu_with_disabled("Show History for Selection…", read_only, Box::new(ShowLocalSelectionHistory))
                                    .separator()
                                    .menu("Put Label…", Box::new(crate::ui::workspace::PutLocalHistoryLabel)),
                            )
                        }),
                )).children(vector_pane).children(manifest_pane))
                .child(self.gutter_probe())
                // Design shows the drawable alone: no gutter.
                .when(!self.coverage.is_empty() && !vector_design, |el| el.child(self.coverage_strip(cx)))
                .when(!read_only && !vector_design, |el| el.child(self.marker_strip(cx)))
                .when(!self.hierarchy.is_empty() && icons_shown && !vector_design, |el| el.child(self.hierarchy_strip(cx)))
                .children(bookmark_strip.filter(|_| !vector_design))
                .children(preview_strip.filter(|_| !vector_design))
                .children(test_strip.filter(|_| !vector_design))
                .children(breakpoint_strip.filter(|_| !vector_design))
                .when_some(hover_card, |el, (at, card)| {
                    el.child(deferred(anchored().position(point(at.x + px(12.), at.y + px(16.))).child(card)).with_priority(1))
                })
                .when_some(self.popup.zip(self.popup_anchor.get()), |el, (ix, at)| el.child(self.render_popup(ix, at, cx)))
                .children(self.render_expression_highlight(cx))
                .children(self.render_template_stops(cx))
                .children(self.render_parameter_info(cx))
                .children(self.render_context_actions(cx))
                .children(self.render_test_menu(cx))
                .children(self.render_color_edit(cx))
                .children(self.render_breakpoint_popup(cx))
                .children(self.render_hot_swap(cx)),
            ))
            .children(manifest_tabs)
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

/// The gutter's marks sorted by line with their slots, at most two a line:
/// up, then down, as IntelliJ orders them.
fn slotted(mut marks: Vec<crate::index::hierarchy::GutterMark>) -> Vec<(u32, usize, crate::index::hierarchy::GutterMark)> {
    marks.sort_by_key(|m| (m.line, m.kind.goes_down()));
    let mut out: Vec<(u32, usize, crate::index::hierarchy::GutterMark)> = Vec::with_capacity(marks.len());
    for m in marks {
        let slot = out.iter().rev().take_while(|(l, _, _)| *l == m.line).count();
        if slot < 2 {
            out.push((m.line, slot, m));
        }
    }
    out
}

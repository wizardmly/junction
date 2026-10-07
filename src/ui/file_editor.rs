//! The file editor: a working-tree file (editable, saved with Ctrl+S) or a
//! file at a revision (read-only), with syntax highlighting and IntelliJ's
//! change markers against HEAD. Its context menu carries the editor's Git
//! actions: Show History for Selection, Annotate, Show Current Revision,
//! Rollback Lines.

use std::ops::Range;

use gpui_kit::component::{
    h_flex,
    input::{Copy, Cut, Editor, EditorState, InputEvent, Paste, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle, SelectAll},
    native_menu::NativeMenu,
    v_flex,
};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    Render, Styled as _, Subscription, Window, actions, div, prelude::FluentBuilder as _, px,
};

use crate::git::Repository;
use crate::git::diff::{self, Hunk};
use crate::theme::ActivePalette as _;
use crate::ui::common::{self, tool_button};
use crate::ui::diff_view::DiffSource;

actions!(
    file_editor,
    [SaveFile, ShowSelectionHistory, AnnotateFile, ShowFileHistory, ShowCurrentRevision, RollbackLines, ShowFileDiff, OpenOnHosting, CreateGist]
);

const CONTEXT: &str = "FileEditor";

pub fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" }, SaveFile, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-z", RollbackLines, Some(CONTEXT)),
    ]);
}

pub enum FileEditorEvent {
    Closed,
    Annotate { path: String, revision: Option<String> },
    ShowHistory(String),
    SelectionHistory { path: String, lines: (usize, usize) },
    SelectCommit(String),
    OpenDiff(DiffSource),
    FilesChanged,
    CreateGist { name: String, content: String },
}

impl EventEmitter<FileEditorEvent> for FileEditor {}

/// The tree-sitter language for a file name (plain text when unknown).
pub fn language_for(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match (name.as_str(), ext) {
        ("cmakelists.txt", _) | (_, "cmake") => "cmake",
        ("makefile" | "gnumakefile", _) | (_, "mk") => "make",
        (_, "rs") => "rust",
        (_, "kt" | "kts") => "kotlin",
        (_, "java") => "java",
        (_, "swift") => "swift",
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
    /// HEAD's version, for change markers.
    base: String,
    saved: String,
    hunks: Vec<Hunk>,
    markers: Option<RangeDecorationCollection>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
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

impl FileEditor {
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
        let base = if revision.is_none() {
            repository.run(["show", &format!("HEAD:{path}")]).unwrap_or_default()
        } else {
            content.clone()
        };
        let language = language_for(&path);
        let initial = content.clone();
        let state = cx.new(|cx| EditorState::new(window, cx).language(language).line_number(true).default_value(initial));
        let subscriptions = vec![cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_markers(cx);
                cx.notify();
            }
        })];
        let mut this = Self {
            repository,
            path,
            revision,
            state,
            base,
            saved: content,
            hunks: Vec::new(),
            markers: None,
            error,
            _subscriptions: subscriptions,
        };
        this.update_markers(cx);
        this
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).value().to_string()
    }

    fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.revision.is_none() && self.text(cx) != self.saved
    }

    /// IntelliJ's change markers: added lines green, modified blue, deletions a thin frame.
    fn update_markers(&mut self, cx: &mut Context<Self>) {
        if self.revision.is_some() || self.error.is_some() {
            return;
        }
        let text = self.text(cx);
        self.hunks = diff::commit_hunks(&self.base, &text);
        let palette = cx.palette().clone();
        let decorations: Vec<RangeDecoration> = self
            .hunks
            .iter()
            .map(|hunk| {
                if hunk.new.is_empty() {
                    let at = line_start(&text, hunk.new.start);
                    let end = line_start(&text, hunk.new.start + 1).max(at);
                    RangeDecoration::new(at..end).with_style(RangeDecorationStyle::Frame).with_color(palette.status_deleted)
                } else {
                    let range = line_start(&text, hunk.new.start)..line_start(&text, hunk.new.end);
                    let color = if hunk.old.is_empty() { palette.diff_inserted } else { palette.status_modified.opacity(0.18) };
                    RangeDecoration::new(range).with_style(RangeDecorationStyle::Fill).with_color(color)
                }
            })
            .collect();
        match &self.markers {
            Some(markers) => markers.set(decorations, cx),
            None => {
                let markers = self.state.update(cx, |state, cx| state.create_range_decorations_collection(decorations, cx));
                self.markers = Some(markers);
            }
        }
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
        if self.revision.is_some() {
            return;
        }
        let text = self.text(cx);
        match std::fs::write(self.repository.root().join(&self.path), &text) {
            Ok(()) => {
                self.saved = text;
                cx.emit(FileEditorEvent::FilesChanged);
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
        if self.revision.is_some() {
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

    fn annotate(&mut self, _: &AnnotateFile, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FileEditorEvent::Annotate { path: self.path.clone(), revision: self.revision.clone() });
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

    fn show_diff(&mut self, _: &ShowFileDiff, _: &mut Window, cx: &mut Context<Self>) {
        let source = match &self.revision {
            Some(rev) => DiffSource::Commit { hash: rev.clone(), path: self.path.clone(), old_path: None },
            None => DiffSource::WorkingTree { path: self.path.clone(), unversioned: false },
        };
        cx.emit(FileEditorEvent::OpenDiff(source));
    }
}

impl Render for FileEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let dirty = self.is_dirty(cx);
        let read_only = self.revision.is_some();
        let title = match &self.revision {
            Some(rev) => format!("{} ({})", self.path, &rev[..rev.len().min(8)]),
            None => self.path.clone(),
        };
        let changes = self.hunks.len();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::rollback_lines))
            .on_action(cx.listener(Self::open_on_hosting))
            .on_action(cx.listener(Self::create_gist))
            .on_action(cx.listener(Self::show_selection_history))
            .on_action(cx.listener(Self::annotate))
            .on_action(cx.listener(Self::show_history))
            .on_action(cx.listener(Self::show_current_revision))
            .on_action(cx.listener(Self::show_diff))
            .child(
                h_flex()
                    .h(px(30.))
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
                    .child(tool_button("editor-close", IconName::X, "Close").on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(FileEditorEvent::Closed)
                    }))),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(div().px_2().py_1().text_sm().text_color(palette.status_conflict).child(error))
            })
            .child(
                div().flex_1().min_h_0().child(
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
                                .separator();
                            let git = NativeMenu::new()
                                .menu("Show History for Selection", Box::new(ShowSelectionHistory))
                                .menu("Annotate with Git Blame", Box::new(AnnotateFile))
                                .menu("Show History", Box::new(ShowFileHistory))
                                .menu("Show Current Revision", Box::new(ShowCurrentRevision))
                                .menu("Show Diff", Box::new(ShowFileDiff))
                                .menu_with_disabled("Rollback Lines", read_only, Box::new(RollbackLines))
                                .separator()
                                .menu("Open on GitHub / GitLab", Box::new(OpenOnHosting))
                                .menu("Create Gist…", Box::new(CreateGist));
                            menu.submenu("Git", git)
                        }),
                ),
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

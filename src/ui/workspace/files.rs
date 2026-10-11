//! Opening files, diffs and annotations, file actions from the Project and
//! Commit tool windows, and opening repositories.

use super::*;

impl Workspace {
    /// Annotate with Git Blame: opens the file (at `revision`, read-only)
    /// with the annotations in its gutter, as IntelliJ does.
    pub fn annotate(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file(path, revision, window, cx);
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| editor.show_annotations(cx));
        }
    }

    /// Show History: a Log tab for one file, following renames, as IntelliJ's "History: name" tab.
    /// Opens a file in the editor: the working tree (`None`) or a revision, read-only.
    pub fn open_file(&mut self, path: String, revision: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if revision.is_none() {
            self.recent_files.touch(&path);
        }
        self.open_tab(path, revision, window, cx);
    }

    pub(super) fn on_file_editor_event(&mut self, view: &Entity<FileEditor>, event: &FileEditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FileEditorEvent::Edited => self.keep_tab(view, cx),
            FileEditorEvent::Deleted => {
                // Closed without saving: saving would bring the file back.
                if let Some(ix) = self.editors.iter().position(|t| t.view == *view) {
                    self.remove_tab(ix, cx);
                }
            }
            FileEditorEvent::Navigate { targets, at } => self.navigate(targets.clone(), *at, window, cx),
            FileEditorEvent::ChooseTargets { title, targets, at } => {
                if let [target] = targets.as_slice() {
                    self.go_to_target(target.clone(), window, cx);
                } else {
                    let locations = targets.iter().map(|t| self.location_label(t, cx)).collect();
                    crate::ui::navigate::choose_target(title.clone(), targets.clone(), locations, Some(*at), self.picker_callback(cx), window, cx);
                }
            }
            FileEditorEvent::NoDeclaration { text, offset, at } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                if self.code_index.read(cx).declared_at(&path, text, *offset) {
                    self.show_usages_popup(path, text.clone(), *offset, *at, window, cx);
                } else {
                    Self::nav_hint("Cannot find declaration to go to", window, cx);
                }
            }
            FileEditorEvent::FindUsages { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                let (index, text, offset) = (self.code_index.clone(), text.clone(), *offset);
                let _ = index;
                self.find.update(cx, |view, cx| view.find_usages(path, text, offset, cx));
                self.tools.open(ToolWindow::Git);
                self.bottom_tab = BottomTab::Find;
                cx.notify();
            }
            FileEditorEvent::Saved(path) => {
                let path = path.clone();
                self.recently_changed.touch(&path);
                self.code_index.update(cx, |index, cx| index.refresh_file(&path, cx));
            }
            FileEditorEvent::CreateGist { name, content } => {
                crate::ui::github_dialogs::create_gist(self.model.clone(), vec![(name.clone(), content.clone())], window, cx)
            }
            FileEditorEvent::Annotate { path, revision } => self.annotate(path.clone(), revision.clone(), window, cx),
            FileEditorEvent::ShowHistory(path) => self.show_history(path.clone(), cx),
            FileEditorEvent::SelectionHistory { path, lines } => {
                let name = path.rsplit('/').next().unwrap_or(path);
                let filter = crate::git::LogFilter {
                    paths: vec![path.clone()],
                    lines: Some(*lines),
                    branches: vec!["HEAD".into()],
                    ..Default::default()
                };
                self.open_log_tab(format!("History for Selection: {name}:{}-{}", lines.0, lines.1), filter, cx);
            }
            FileEditorEvent::SelectCommit(hash) => {
                self.tools.open(ToolWindow::Git);
                self.bottom_tab = BottomTab::Log;
                let hash = hash.clone();
                self.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                cx.notify();
            }
            FileEditorEvent::OpenDiff(source) => self.open_diff(source.clone(), cx),
            FileEditorEvent::FilesChanged => self.model.update(cx, |m, cx| m.reload(cx)),
        }
    }

    /// Files changed on disk by Replace: re-index, refresh VCS, reload an unmodified editor.
    pub(super) fn files_changed(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        for path in &paths {
            self.recently_changed.touch(path);
            let path = path.clone();
            self.code_index.update(cx, |index, cx| index.refresh_file(&path, cx));
        }
        self.model.update(cx, |m, cx| m.reload(cx));
        // Unmodified tabs of changed files reload from disk.
        let stale: Vec<usize> = (0..self.editors.len())
            .filter(|&i| {
                let e = self.editors[i].view.read(cx);
                e.revision().is_none() && paths.iter().any(|p| p == e.path()) && !e.is_dirty(cx)
            })
            .collect();
        for ix in stale {
            let Some(repository) = self.model.read(cx).project_repository().cloned() else { break };
            let old = self.editors[ix].view.clone();
            let (path, (line, col)) = (old.read(cx).path().to_owned(), old.read(cx).cursor(cx));
            let view = cx.new(|cx| FileEditor::new(repository, path, None, window, cx));
            let subscription = cx.subscribe_in(&view, window, Self::on_file_editor_event);
            let index = self.code_index.clone();
            view.update(cx, |editor, cx| {
                editor.attach_index(index, cx);
                editor.go_to(line, col, window, cx);
            });
            let tab = &mut self.editors[ix];
            tab.view = view;
            tab._subscription = subscription;
        }
        cx.notify();
    }

    pub(super) fn toggle_project(&mut self, _: &ToggleProjectWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_tool(ToolWindow::Project, window, cx);
    }

    /// Select In › Project View: shows the current editor's file in the tree.
    pub(super) fn select_in_project(&mut self, _: &SelectInProject, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let path = editor.read(cx).path().to_owned();
        self.tools.open(ToolWindow::Project);
        self.project.update(cx, |project, cx| project.reveal(&path, cx));
        cx.notify();
    }

    pub(super) fn toggle_find(&mut self, _: &ToggleFindWindow, _: &mut Window, cx: &mut Context<Self>) {
        if self.tools.is_open(ToolWindow::Git) && self.bottom_tab == BottomTab::Find {
            self.tools.hide(ToolWindow::Git);
        } else {
            self.tools.open(ToolWindow::Git);
            self.bottom_tab = BottomTab::Find;
        }
        cx.notify();
    }

    pub(super) fn file_action(&mut self, action: crate::ui::file_menus::FileAction, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffSource;
        use crate::ui::file_menus::FileAction;
        match action {
            FileAction::ShowDiff(path) => {
                let unversioned = self.model.read(cx).project_status().entries.iter().any(|e| e.path == path && e.kind == crate::git::StatusKind::Unversioned);
                self.open_diff(DiffSource::WorkingTree { path, unversioned }, cx)
            }
            FileAction::OpenFile(path) => self.open_file(path, None, window, cx),
            FileAction::Annotate(path) => self.annotate(path, None, window, cx),
            FileAction::ShowHistory(path) => self.show_history(path, cx),
            FileAction::CompareWithRevision(path) => {
                let workspace = cx.entity();
                dialogs::compare_file_with_revision(
                    self.model.clone(),
                    path,
                    Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                    window,
                    cx,
                );
            }
            FileAction::CompareWithBranch(path) => {
                let workspace = cx.entity();
                dialogs::compare_file_with(
                    self.model.clone(),
                    path,
                    Rc::new(move |source, _, cx| workspace.update(cx, |this, cx| this.open_diff(source, cx))),
                    window,
                    cx,
                );
            }
            FileAction::CompareWithClipboard(path) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.open_diff(DiffSource::Clipboard { path, text }, cx)
            }
            FileAction::CompareWithFile(path) => {
                let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Compare With".into()) });
                cx.spawn(async move |this, cx| {
                    let Ok(Ok(Some(picked))) = paths.await else { return };
                    let Some(other) = picked.into_iter().next() else { return };
                    this.update(cx, |this, cx| this.open_diff(DiffSource::Files { path, other }, cx)).ok();
                })
                .detach();
            }
            FileAction::CommitFiles(paths) => {
                self.show_left_tab(LeftTab::Commit, window, cx);
                self.commit.update(cx, |commit, cx| commit.commit_only(paths, window, cx));
                cx.notify();
            }
            FileAction::Branches => self.open_branches(window, cx),
            FileAction::Unstash => {
                self.show_left_tab(LeftTab::Stash, window, cx);
            }
            FileAction::FilesChanged => {
                self.code_index.update(cx, |index, cx| index.refresh(cx));
                self.model.update(cx, |m, cx| m.reload(cx));
            }
        }
    }

    pub(super) fn open_diff(&mut self, source: crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffSource;
        let Some(mut repository) = self.model.read(cx).repository().cloned() else { return };
        // Local changes have paths relative to the project: in a multi-root
        // project the diff opens in the root that holds the file.
        let mut source = source;
        if self.model.read(cx).root_states().len() > 1 {
            if let DiffSource::WorkingTree { path, .. } | DiffSource::Staged { path } | DiffSource::Unstaged { path } = &mut source {
                if let Some((root, own)) = self.model.read(cx).route(path) {
                    repository = root;
                    *path = own;
                }
            }
        }
        // A diff replaces the annotations and the file editor in the editor area.
        self.focus_group(0, cx);
        self.front = Front::Diff;
        self.diff.update(cx, |diff, cx| diff.show(repository, source, cx));
        cx.notify();
    }

    pub(super) fn open_repository(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git Repository".into()),
        });
        let model = self.model.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    model.update(cx, |model, cx| model.open(path, cx));
                }
            }
        })
        .detach();
    }
}

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
        // An APK or app bundle opens in the APK Analyzer, as in Android Studio.
        if revision.is_none() && (path.ends_with(".apk") || path.ends_with(".aab")) {
            if let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) {
                return self.open_apk(root.join(&path), cx);
            }
        }
        // A saved Logcat opens in a Logcat tab, as Android Studio imports it.
        if revision.is_none() && path.ends_with(".logcat") {
            if let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) {
                self.tools.open(ToolWindow::Logcat);
                self.logcat_view.update(cx, |l, cx| l.open_file(root.join(&path), window, cx));
                cx.notify();
                return;
            }
        }
        if revision.is_none() {
            self.recent_files.touch(&path);
            // Leaving a file: where we were is a recent location.
            if let Some(here) = self.current_position(cx).filter(|(p, _, _)| *p != path) {
                self.recent_places.visit(here);
            }
        }
        self.open_tab(path, revision, window, cx);
    }

    pub(crate) fn on_file_editor_event(&mut self, view: &Entity<FileEditor>, event: &FileEditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FileEditorEvent::TextChanged => self.mirror_to_twin(view, window, cx),
            FileEditorEvent::Edited => {
                self.keep_tab(view, cx);
                let editor = view.read(cx);
                if editor.revision().is_none() {
                    let (line, col) = editor.cursor(cx);
                    let path = editor.path().to_owned();
                    // Lines added or removed: the places below move with them.
                    let count = editor.line_count(cx) as i64;
                    if let Some(before) = self.line_counts.insert(path.clone(), count) {
                        let delta = count - before;
                        if delta != 0 {
                            let from = if delta > 0 { line as i64 - delta } else { line as i64 };
                            self.recent_places.shift(&path, from.max(0) as u32, delta);
                        }
                    }
                    self.recent_places.change((path, line, col));
                }
                self.schedule_session_save(cx);
            }
            FileEditorEvent::CopyPathReference => {
                let editor = view.read(cx);
                let (path, lines) = (editor.path().to_owned(), editor.url_lines(cx));
                crate::ui::copy_path::open_for(&self.model, vec![path], Some(lines), None, window, cx);
            }
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
                    let rows = self.symbol_rows(targets, cx);
                    // "Open in Find Tool Window": the same list there.
                    let items: Vec<crate::ui::find_view::FoundItem> = targets
                        .iter()
                        .zip(&rows)
                        .map(|(t, r)| crate::ui::find_view::FoundItem {
                            title: r.name.clone(),
                            detail: [r.package.clone().map(|p| format!("({p})")), Some(r.location.clone())].into_iter().flatten().collect::<Vec<_>>().join("  "),
                            icon: r.icon.clone(),
                            target: t.clone(),
                        })
                        .collect();
                    let workspace = cx.entity().downgrade();
                    let find_title = title.split(" (").next().unwrap_or(title).trim_start_matches("Choose ").to_owned();
                    let on_find: std::rc::Rc<dyn Fn(&mut Window, &mut gpui_kit::App)> = std::rc::Rc::new(move |_, cx| {
                        let items = items.clone();
                        let find_title = find_title.clone();
                        workspace
                            .update(cx, |this, cx| {
                                this.find.update(cx, |view, cx| view.show_items(find_title, items, cx));
                                this.show_find_window(cx);
                                cx.notify();
                            })
                            .ok();
                    });
                    crate::ui::navigate::choose_symbol(title.clone(), targets.clone(), rows, Some(*at), self.picker_callback(cx), on_find, window, cx);
                }
            }
            FileEditorEvent::NoDeclaration { text, offset, at, declaration } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                if *declaration {
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
                self.show_find_window(cx);
                cx.notify();
            }
            FileEditorEvent::Rename { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                self.rename_symbol(path, text.clone(), *offset, window, cx);
            }
            FileEditorEvent::FilesWritten(paths) => {
                let paths = paths.clone();
                self.files_changed(paths, window, cx);
            }
            FileEditorEvent::FilesMoved(moves) => {
                for (from, to) in moves.clone() {
                    self.retarget_tabs(&from, &to, window, cx);
                    self.files_changed(vec![from, to], window, cx);
                }
            }
            FileEditorEvent::CallHierarchy { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                let (text, offset) = (text.clone(), *offset);
                match self.hierarchy.update(cx, |view, cx| view.call_hierarchy(path, text, offset, cx)) {
                    Ok(()) => self.tools.open(ToolWindow::Hierarchy),
                    Err(message) => {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(gpui_kit::component::notification::Notification::info(message).title("Call Hierarchy"), cx);
                    }
                }
                cx.notify();
            }
            FileEditorEvent::MethodHierarchy { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                match self.hierarchy.update(cx, |view, cx| view.method_hierarchy(&path, text, *offset, cx)) {
                    Ok(()) => self.tools.open(ToolWindow::Hierarchy),
                    Err(message) => {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(gpui_kit::component::notification::Notification::info(message).title("Method Hierarchy"), cx);
                    }
                }
                cx.notify();
            }
            FileEditorEvent::TypeHierarchy { text, offset } => {
                let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
                match self.code_index.read(cx).type_at(&path, text, *offset) {
                    Some(name) => {
                        self.hierarchy.update(cx, |view, cx| view.type_hierarchy(name, cx));
                        self.tools.open(ToolWindow::Hierarchy);
                    }
                    None => {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(gpui_kit::component::notification::Notification::info("No type at the caret").title("Type Hierarchy"), cx);
                    }
                }
                cx.notify();
            }
            FileEditorEvent::OpenTranslations(path) => {
                let path = path.clone();
                self.open_translations(&path, window, cx);
            }
            FileEditorEvent::Saved(path) => {
                self.mirror_to_twin(view, window, cx);
                let path = path.clone();
                self.reload_translations(&path, cx);
                self.recently_changed.touch(&path);
                self.code_index.update(cx, |index, cx| index.refresh_file(&path, cx));
                if let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) {
                    if self.history_pruned.as_ref() != Some(&root) {
                        self.history_pruned = Some(root.clone());
                        crate::ui::local_history_window::prune(root.clone());
                    }
                    crate::ui::local_history_window::record_file(root.clone(), path.clone(), false);
                    let file = if std::path::Path::new(&path).is_absolute() { std::path::PathBuf::from(&path) } else { root.join(&path) };
                    crate::ui::debug_service::DebugService::global(cx).update(cx, |debug, cx| debug.source_saved(file.clone(), cx));
                    crate::ui::compile_service::CompileService::global(cx).update(cx, |service, cx| service.file_saved(file, cx));
                }
            }
            FileEditorEvent::CreateGist { name, content } => {
                crate::ui::github_dialogs::create_gist(self.model.clone(), vec![(name.clone(), content.clone())], window, cx)
            }
            FileEditorEvent::Annotate { path, revision } => self.annotate(path.clone(), revision.clone(), window, cx),
            FileEditorEvent::ShowHistory(path) => self.show_history(path.clone(), cx),
            FileEditorEvent::LocalSelectionHistory { path, lines } => {
                use crate::ui::local_history_window::{self as lh, Scope};
                let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
                let current = view.read(cx).current_text(cx);
                let scope = Scope::Selection { path: path.clone(), start: lines.0.saturating_sub(1), end: lines.1, current };
                lh::open(root, scope, None, self.model.clone(), cx.entity().downgrade(), window.window_handle(), cx);
            }
            FileEditorEvent::SelectionHistory { path, lines } => {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
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
    /// Files written outside the editors (Local History's Revert).
    pub fn files_changed_on_disk(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.files_changed(paths, window, cx);
    }

    /// Android Studio's Translations Editor of the res folder holding a
    /// values file (a project path), else of the project's first module:
    /// every open file saved first, as its edits go to disk.
    pub(super) fn open_translations(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::translations_editor::{TranslationsEditor, TranslationsEvent, res_of};
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        let res = res_of(&root.join(path)).or_else(|| first_res(&root));
        let Some(res) = res else {
            window.push_notification(Notification::info("No string resources in this project").title("Translations Editor"), cx);
            return;
        };
        let editors: Vec<_> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).collect();
        for editor in editors {
            editor.update(cx, |e, cx| e.save_now(cx));
        }
        match &self.translations {
            Some((view, _)) if view.read(cx).covers(&res) => view.update(cx, |v, cx| v.reload(cx)),
            _ => {
                // Every res folder with values, this one first.
                let mut others: Vec<std::path::PathBuf> = crate::infra::resource_scanner::ResFolders { root: root.clone() }
                    .res_dirs()
                    .into_iter()
                    .filter(|r| *r != res && std::fs::read_dir(r).is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("values"))))
                    .collect();
                others.sort();
                let label = |r: &std::path::Path| r.strip_prefix(&root).unwrap_or(r).to_string_lossy().replace('\\', "/");
                let folders = std::iter::once(res).chain(others).map(|r| {
                    let l = label(&r);
                    (r, l)
                }).collect();
                let view = cx.new(|cx| TranslationsEditor::new(folders, window, cx));
                let subscription = cx.subscribe_in(&view, window, move |this, _, event: &TranslationsEvent, window, cx| match event {
                    TranslationsEvent::FilesChanged(paths) => {
                        let Some(root) = this.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
                        let paths = paths.iter().filter_map(|p| p.strip_prefix(&root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).collect();
                        this.files_changed(paths, window, cx);
                    }
                });
                self.translations = Some((view, subscription));
            }
        }
        self.focus_group(0, cx);
        self.front = Front::Translations;
        cx.notify();
    }

    /// A values file saved in an editor: the Translations Editor follows.
    fn reload_translations(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some((view, _)) = &self.translations else { return };
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        if crate::ui::translations_editor::res_of(&root.join(path)).is_some_and(|res| view.read(cx).covers(&res)) {
            view.update(cx, |v, cx| v.reload(cx));
        }
    }

    /// Local History › Show History for the open file.
    pub(super) fn show_local_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.editor().map(|e| e.read(cx).path().to_owned()) else { return };
        self.show_local_history_of(path, window, cx);
    }

    /// Reformat Code / Optimize Imports on files: an open file's editor
    /// takes the new text as an undoable edit, the others are written.
    pub(super) fn clean_up_files(&mut self, items: &[(String, bool)], clean_up: crate::usecase::clean_up_files::CleanUp, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::project_files::ProjectFiles as _;
        use crate::usecase::clean_up_files as usecase;
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        let files = crate::infra::project_files::DiskFiles { root, repository: None };
        let open: Vec<Entity<FileEditor>> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).filter(|v| v.read(cx).revision().is_none()).collect();
        let mut changed = 0;
        for path in usecase::targets(&files, items, clean_up) {
            let indent = crate::infra::editorconfig::indent_for(&files.root, &files.root.join(&path));
            let indent = indent.as_deref();
            if let Some(editor) = open.iter().find(|v| v.read(cx).path() == path) {
                let (text, saved) = editor.read(cx).buffer(cx);
                if let Some(new) = usecase::cleaned(&path, &text, clean_up, indent) {
                    editor.update(cx, |e, cx| e.mirror(&new, &saved, window, cx));
                    changed += 1;
                }
            } else if let Some(new) = files.read(&path).ok().and_then(|text| usecase::cleaned(&path, &text, clean_up, indent)) {
                if files.write(&path, &new).is_ok() {
                    changed += 1;
                }
            }
        }
        window.push_notification(Notification::info(usecase::summary(clean_up, changed)).title(clean_up.title()), cx);
        self.file_action(crate::ui::file_menus::FileAction::FilesChanged, window, cx);
    }

    /// Analyze › Inspect Code… on a scope: every open file saved first, the
    /// inspections run in the background, the findings in the Problems
    /// window's Inspection tab (a notification when there are none).
    pub(super) fn inspect_code(&mut self, items: Vec<(String, bool)>, scope: String, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::project_files::ProjectFiles as _;
        use crate::usecase::inspect_code as usecase;
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        let editors: Vec<_> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).collect();
        for editor in editors {
            editor.update(cx, |e, cx| e.save_now(cx));
        }
        let lint = lint_plan(&root, &items, cx);
        let lint_items = items.clone();
        let lint_root = root.clone();
        let task = cx.background_spawn(async move {
            let files = crate::infra::project_files::DiskFiles { root: root.clone(), repository: None };
            let targets = usecase::targets(&files, &items, &|p| crate::index::lang::Lang::from_path(p).is_some());
            let count = targets.len();
            let findings: Vec<(std::path::PathBuf, Vec<crate::domain::diagnostics::Problem>)> = targets
                .into_iter()
                .filter_map(|path| {
                    let text = files.read(&path).ok()?;
                    let syntax = crate::index::lang::Lang::from_path(&path).map(|lang| crate::infra::tree_sitter_syntax::TreeSitterSyntax { lang });
                    let found = usecase::inspect(&path, &text, syntax.as_ref().map(|s| s as &dyn crate::domain::diagnostics::SyntaxChecker));
                    (!found.is_empty()).then(|| (root.join(&path), found.into_iter().map(|f| f.problem).collect()))
                })
                .collect();
            (count, findings)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (count, findings) = task.await;
            let linting = !lint.is_empty();
            this.update_in(cx, |this, window, cx| {
                if findings.is_empty() && !linting {
                    let files = if count == 1 { "1 file".to_owned() } else { format!("{count} files") };
                    window.push_notification(Notification::info(format!("No suspicious code found. {files} processed in '{scope}'.")).title("Code Analysis"), cx);
                }
                if linting {
                    window.push_notification(Notification::info("Running Android Lint…").title("Code Analysis"), cx);
                }
                this.problems_view.update(cx, |p, cx| p.show_inspection(scope, findings, cx));
                this.tools.open(crate::ui::tool_windows::ToolWindow::Problems);
                cx.notify();
            })
            .ok();
            if !linting {
                return;
            }
            // Android Lint runs Gradle: its findings join the results when it ends.
            let found = cx
                .background_spawn(async move {
                    let mut found = Vec::new();
                    let mut errors = Vec::new();
                    let read = |p: &std::path::Path| std::fs::read_to_string(p).ok();
                    for (build_root, modules) in lint {
                        match crate::usecase::inspect_code::run_lint(&crate::infra::gradle::GradleBuild, &build_root, &modules, &read, &|| false) {
                            Ok(more) => found.extend(more),
                            Err(error) => errors.push(error),
                        }
                    }
                    let in_scope = |file: &std::path::Path| {
                        let rel = file.strip_prefix(&lint_root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                        lint_items.iter().any(|(item, dir)| item.is_empty() || rel == *item || (*dir && rel.starts_with(&format!("{item}/"))))
                    };
                    let mut by_file: Vec<(std::path::PathBuf, Vec<crate::domain::diagnostics::Problem>)> = Vec::new();
                    for (file, finding) in found.into_iter().filter(|(f, _)| in_scope(f)) {
                        match by_file.iter_mut().find(|(p, _)| *p == file) {
                            Some((_, problems)) => problems.push(finding.problem),
                            None => by_file.push((file, vec![finding.problem])),
                        }
                    }
                    (by_file, errors)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let (findings, errors) = found;
                let count: usize = findings.iter().map(|(_, p)| p.len()).sum();
                let message = match (count, errors.first()) {
                    (_, Some(error)) => format!("Android Lint failed: {}", error.lines().next().unwrap_or_default()),
                    (0, None) => "Android Lint found no problems".to_owned(),
                    (1, None) => "Android Lint found 1 problem".to_owned(),
                    (n, None) => format!("Android Lint found {n} problems"),
                };
                let notification = if errors.is_empty() { Notification::info(message) } else { Notification::error(message) };
                window.push_notification(notification.title("Code Analysis"), cx);
                this.problems_view.update(cx, |p, cx| p.add_inspection(findings, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Build › Analyze APK…: the analyzer of `apk` in the editor area.
    pub(super) fn open_apk(&mut self, apk: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.apk.as_ref().is_none_or(|a| *a.read(cx).path() != apk) {
            self.apk = Some(cx.new(|cx| crate::ui::apk_analyzer::ApkAnalyzer::new(apk, cx)));
        }
        self.front = Front::Apk;
        cx.notify();
    }

    /// Build › Analyze APK…: asks for the APK first.
    pub(super) fn analyze_apk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Select an APK or app bundle".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(picked))) = paths.await else { return };
            let Some(apk) = picked.into_iter().next() else { return };
            this.update(cx, |this, cx| this.open_apk(apk, cx)).ok();
        })
        .detach();
    }

    /// Analyze › Analyze Dependencies… on a scope: its Java and Kotlin files
    /// in the Dependency Viewer.
    pub(super) fn analyze_dependencies(&mut self, items: Vec<(String, bool)>, scope: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        let files = crate::infra::project_files::DiskFiles { root, repository: None };
        let jvm = |p: &str| p.ends_with(".kt") || p.ends_with(".kts") || p.ends_with(".java");
        let targets = crate::usecase::inspect_code::targets(&files, &items, &jvm);
        let task = self.code_index.read(cx).dependencies(targets, cx);
        cx.spawn_in(window, async move |this, cx| {
            let deps = task.await;
            this.update_in(cx, |_, window, cx| {
                let workspace = cx.entity().downgrade();
                let open_file: crate::ui::dependency_viewer::OpenFile = Rc::new(move |path, window, cx| {
                    workspace.update(cx, |w, cx| w.file_action(crate::ui::file_menus::FileAction::OpenFile(path), window, cx)).ok();
                });
                crate::ui::dependency_viewer::open(scope, deps, open_file, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Local History › Show History… of a project file or folder from a
    /// menu (empty: Show Project History).
    pub(super) fn show_local_history_of(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::local_history_window::{self as lh, Scope};
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        let scope = if root.join(&path).is_dir() { Scope::Folder(path) } else { Scope::File(path) };
        lh::open(root, scope, None, self.model.clone(), cx.entity().downgrade(), window.window_handle(), cx);
    }

    /// Recent Changes (Alt+Shift+C).
    pub(super) fn recent_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        crate::ui::local_history_window::recent_changes(root, self.model.clone(), cx.entity().downgrade(), window, cx);
    }

    pub(super) fn put_local_history_label(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
        crate::ui::local_history_window::put_label(root, window, cx);
    }

    /// Refactor › Rename (Shift+F6), as Android Studio's: every open file
    /// saved first, the dialog, then the occurrences renamed on disk and a
    /// class's file renamed along with it, its open tabs following.
    /// "Renamed 'a' to 'b'" with an Undo button: Android Studio undoes a
    /// refactoring with Ctrl+Z; here the notification carries it.
    fn offer_rename_undo(&mut self, files: crate::infra::project_files::DiskFiles, renamed: crate::usecase::rename_symbol::Renamed, old: &str, new: &str, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity().downgrade();
        let undo = std::rc::Rc::new((files, renamed));
        let notification = Notification::success(format!("Renamed \u{2018}{old}\u{2019} to \u{2018}{new}\u{2019}")).title("Rename").action(move |_, _, _| {
            let entity = entity.clone();
            let undo = undo.clone();
            Button::new("notify-undo-rename").label("Undo").small().outline().on_click(move |_, window, cx| {
                let (files, renamed) = &*undo;
                let result = crate::usecase::rename_symbol::undo(files, renamed);
                entity
                    .update(cx, |this, cx| match result {
                        Err(message) => window.push_notification(Notification::error(message).title("Undo Rename"), cx),
                        Ok(()) => {
                            let mut changed = renamed.changed.clone();
                            if let Some((from, to)) = &renamed.file {
                                this.retarget_tabs(to, from, window, cx);
                                changed.retain(|p| p != from);
                                changed.push(from.clone());
                            }
                            this.files_changed(changed, window, cx);
                            window.push_notification(Notification::info("Rename undone").title("Rename"), cx);
                        }
                    })
                    .ok();
            })
        });
        window.push_notification(notification, cx);
    }

    fn rename_symbol(&mut self, path: String, text: String, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let editors: Vec<_> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).collect();
        for editor in editors {
            editor.update(cx, |e, cx| e.save_now(cx));
        }
        let task = self.code_index.read(cx).rename_target(path, text, offset, cx);
        cx.spawn_in(window, async move |this, cx| {
            let target = task.await;
            this.update_in(cx, |this, window, cx| match target {
                Err(message) => window.push_notification(Notification::info(message).title("Rename"), cx),
                Ok(target) => {
                    let workspace = cx.entity().downgrade();
                    let model = this.model.clone();
                    crate::ui::rename_dialog::open(target, move |request, window, cx| {
                        let model = model.read(cx);
                        let Some(root) = model.project_root().map(|r| r.to_path_buf()) else { return };
                        let files = crate::infra::project_files::DiskFiles { root, repository: model.project_repository().cloned() };
                        let result = crate::usecase::rename_symbol::rename(&files, &request.occurrences, &request.old, &request.new, &request.declaration, request.rename_file);
                        workspace
                            .update(cx, |this, cx| match result {
                                Err(message) => window.push_notification(Notification::error(message).title("Rename"), cx),
                                Ok(renamed) => {
                                    if let Some((from, to)) = &renamed.file {
                                        this.retarget_tabs(from, to, window, cx);
                                    }
                                    let mut changed = renamed.changed.clone();
                                    if let Some((from, to)) = renamed.file.clone() {
                                        changed.retain(|p| *p != from);
                                        changed.push(to);
                                    }
                                    this.files_changed(changed, window, cx);
                                    if renamed.replaced > 0 || renamed.file.is_some() {
                                        this.offer_rename_undo(files, renamed, &request.old, &request.new, window, cx);
                                    }
                                    // The reloaded tab keeps the keyboard, as Android Studio's editor
                                    // does: once the dialog has closed and given its focus back (to the
                                    // replaced editor).
                                    if let Some(editor) = this.editor().cloned() {
                                        window
                                            .spawn(cx, async move |cx| {
                                                let closed = *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(50);
                                                cx.background_executor().timer(closed).await;
                                                cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(window, cx))).ok();
                                            })
                                            .detach();
                                    }
                                }
                            })
                            .ok();
                    }, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

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
            let old = self.editors[ix].view.clone();
            let (path, (line, col)) = (old.read(cx).path().to_owned(), old.read(cx).cursor(cx));
            let model = self.model.read(cx);
            let Some((repository, own)) = model.route(&path).or_else(|| model.project_repository().cloned().map(|r| (r, path.clone()))) else { break };
            let view = cx.new(|cx| FileEditor::new(repository, own, path, None, window, cx));
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

    /// A file or folder moved on disk (Rename, Cut and Paste): its open
    /// tabs follow it, keeping their unsaved text and caret, as IntelliJ's do.
    pub(super) fn retarget_tabs(&mut self, from: &str, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        let moved = |path: &str| -> Option<String> {
            if path == from {
                Some(to.to_owned())
            } else {
                path.strip_prefix(from).filter(|rest| rest.starts_with('/')).map(|rest| format!("{to}{rest}"))
            }
        };
        let groups = std::iter::once(&mut self.editors).chain(self.split.as_mut().map(|s| &mut s.editors));
        let mut retargeted = Vec::new();
        for (g, editors) in groups.enumerate() {
            for (ix, tab) in editors.iter().enumerate() {
                let e = tab.view.read(cx);
                if let Some(path) = e.revision().is_none().then(|| moved(e.path())).flatten() {
                    retargeted.push((g, ix, path));
                }
            }
        }
        for (g, ix, path) in retargeted {
            let old = if g == 0 { self.editors[ix].view.clone() } else { self.split.as_ref().map(|s| s.editors[ix].view.clone()).expect("split") };
            let (line, col) = old.read(cx).cursor(cx);
            let unsaved = old.read(cx).is_dirty(cx).then(|| old.read(cx).text(cx));
            let model = self.model.read(cx);
            let Some((repository, own)) = model.route(&path).or_else(|| model.project_repository().cloned().map(|r| (r, path.clone()))) else { continue };
            let view = cx.new(|cx| FileEditor::new(repository, own, path, None, window, cx));
            let subscription = cx.subscribe_in(&view, window, Self::on_file_editor_event);
            let index = self.code_index.clone();
            view.update(cx, |editor, cx| {
                editor.attach_index(index, cx);
                if let Some(text) = unsaved {
                    editor.set_text(text, window, cx);
                }
                editor.go_to(line, col, window, cx);
            });
            let tab = if g == 0 { &mut self.editors[ix] } else { &mut self.split.as_mut().expect("split").editors[ix] };
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

    /// Alt+3: the Find tool window, once a search has run (as in IntelliJ,
    /// where it appears with the first search).
    pub(super) fn toggle_find(&mut self, _: &ToggleFindWindow, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.read(cx).is_empty() {
            return;
        }
        self.shortcut_tool(ToolWindow::Find, window, cx);
    }

    /// Shows search results: the Find tool window opens on its side.
    pub(super) fn show_find_window(&mut self, cx: &mut Context<Self>) {
        self.tools.open(ToolWindow::Find);
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
            FileAction::OpenInSplit(path) => self.open_in_split(path, window, cx),
            FileAction::ShowLocalHistory(path) => self.show_local_history_of(path, window, cx),
            FileAction::PutLocalHistoryLabel => self.put_local_history_label(window, cx),
            FileAction::RecentChanges => self.recent_changes(window, cx),
            FileAction::FindUsages(path) => {
                let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
                let text = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
                let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match crate::domain::project_files::class_declaration(&text, &stem) {
                    Some(offset) => {
                        self.find.update(cx, |view, cx| view.find_usages(path, text, offset, cx));
                        self.show_find_window(cx);
                        cx.notify();
                    }
                    None => window.push_notification(Notification::info(format!("No class {stem} declared in the file")).title("Find Usages"), cx),
                }
            }
            FileAction::CleanUp { items, clean_up } => self.clean_up_files(&items, clean_up, window, cx),
            FileAction::Annotate(path) => self.annotate(path, None, window, cx),
            FileAction::ShowHistory(path) => self.show_history(path, cx),
            FileAction::CompareWithRevision(path) => {
                // The file's root becomes the active one; revisions are its.
                let path = self.model.update(cx, |m, cx| m.focus_path(&path, cx)).map(|(_, own)| own).unwrap_or(path);
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
                let path = self.model.update(cx, |m, cx| m.focus_path(&path, cx)).map(|(_, own)| own).unwrap_or(path);
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
                self.open_diff(DiffSource::Clipboard { path, text, title: "Clipboard".to_owned() }, cx)
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
            FileAction::CompareFiles { path, other } => self.open_diff(DiffSource::Files { path, other }, cx),
            FileAction::CommitFiles(paths) => {
                self.show_left_tab(LeftTab::Commit, window, cx);
                self.commit.update(cx, |commit, cx| commit.commit_only(paths, window, cx));
                cx.notify();
            }
            FileAction::Branches => self.open_branches(window, cx),
            FileAction::Unstash => {
                self.show_left_tab(LeftTab::Stash, window, cx);
            }
            FileAction::Moved { from, to } => {
                if let (Some(root), Some(store)) = (self.model.read(cx).project_root().map(|r| r.to_path_buf()), crate::infra::file_type_store::DiskFileTypes::new()) {
                    let abs = |p: &str| root.join(p).to_string_lossy().replace('\\', "/");
                    crate::usecase::override_file_type::file_moved(&store, &abs(&from), &abs(&to)).ok();
                }
                self.retarget_tabs(&from, &to, window, cx);
                self.code_index.update(cx, |index, cx| index.refresh(cx));
                self.model.update(cx, |m, cx| m.reload(cx));
            }
            FileAction::SetExcluded { dir, excluded } => {
                let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
                let Some(store) = crate::infra::excluded_store::DiskExcluded::for_project(&root) else { return };
                match crate::usecase::mark_directory::set_excluded(&store, &dir, excluded) {
                    Ok(true) => self.code_index.update(cx, |index, cx| index.refresh(cx)),
                    Ok(false) => {}
                    Err(error) => window.push_notification(Notification::error(error).title("Mark Directory As"), cx),
                }
            }
            FileAction::SetFileType { path, language } => {
                let Some(root) = self.model.read(cx).project_root().map(|r| r.to_path_buf()) else { return };
                let Some(store) = crate::infra::file_type_store::DiskFileTypes::new() else { return };
                let file = root.join(&path).to_string_lossy().replace('\\', "/");
                match crate::usecase::override_file_type::set_file_type(&store, &file, language.as_deref()) {
                    Ok(true) => {
                        let language = language.unwrap_or_else(|| crate::ui::file_editor::language_for(&path).to_owned());
                        let editors: Vec<_> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).filter(|v| v.read(cx).path() == path).collect();
                        for editor in editors {
                            editor.update(cx, |e, cx| e.set_language(language.clone(), cx));
                        }
                    }
                    Ok(false) => {}
                    Err(error) => window.push_notification(Notification::error(error).title("Override File Type"), cx),
                }
            }
            FileAction::InspectCode { items, scope } => self.inspect_code(items, scope, window, cx),
            FileAction::AnalyzeDependencies { items, scope } => self.analyze_dependencies(items, scope, window, cx),
            FileAction::RepairFiles(files) => {
                self.code_index.update(cx, |index, cx| {
                    for file in &files {
                        index.refresh_file(file, cx);
                    }
                });
                self.files_changed_on_disk(files, window, cx);
            }
            FileAction::FilesChanged => {
                self.code_index.update(cx, |index, cx| index.refresh(cx));
                self.model.update(cx, |m, cx| m.reload(cx));
            }
            FileAction::FindInPath { dir, replace } => {
                self.find_in = Some(dir);
                self.open_find_popup(replace, window, cx);
            }
        }
    }

    /// A diff's gutter and annotation actions.
    pub fn on_diff_event(&mut self, diff: &Entity<DiffView>, event: &crate::ui::diff_view::DiffEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffEvent;
        // The diff's paths are its repository's own (a nested root's).
        let root = diff.read(cx).repository().map(|r| r.root().to_path_buf());
        let project_path = |this: &Self, path: &str, cx: &App| match &root {
            Some(root) => this.model.read(cx).project_path(root, path),
            None => path.to_owned(),
        };
        match event {
            DiffEvent::SelectInLog(hash) => {
                window.activate_window();
                self.tools.open(ToolWindow::Git);
                self.bottom_tab = BottomTab::Log;
                let hash = hash.clone();
                self.model.update(cx, |m, cx| m.select_hash(Some(hash), cx));
                cx.notify();
            }
            DiffEvent::OpenDiff(source) => {
                window.activate_window();
                self.open_diff(source.clone(), cx);
            }
            DiffEvent::Annotate { path, revision } => {
                window.activate_window();
                let path = project_path(self, path, cx);
                self.annotate(path, revision.clone(), window, cx);
            }
            DiffEvent::OpenInWindow => {
                let view = diff.read(cx);
                let (Some(repository), Some(source)) = (view.repository().cloned(), view.source().cloned()) else { return };
                crate::ui::diff_window::open(repository, source, cx.entity().downgrade(), window.window_handle(), cx);
            }
        }
    }

    /// Jump to Source from a diff in another window.
    pub fn go_to_diff_target(&mut self, root: Option<std::path::PathBuf>, mut target: crate::index::nav::Target, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = root {
            target.path = self.model.read(cx).project_path(&root, &target.path);
        }
        self.go_to_target(target, window, cx)
    }

    /// Files changed outside the project window (a separate diff window saved).
    pub fn reload_model(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.reload(cx));
    }

    /// Show Diff in a New Tab: the diff in a tab of its own, which later
    /// previews don't replace.
    pub(super) fn open_diff_in_new_tab(&mut self, source: crate::ui::diff_view::DiffSource, window: &mut Window, cx: &mut Context<Self>) {
        self.save_before_diff(&source, cx);
        let Some((repository, source)) = self.route_diff(source, cx) else { return };
        let view = cx.new(DiffView::new);
        let subscriptions = vec![
            cx.subscribe_in(&view, window, |this, diff, event: &crate::ui::navigate::OpenTarget, window, cx| {
                let mut target = event.0.clone();
                if let Some(root) = diff.read(cx).repository().map(|r| r.root().to_path_buf()) {
                    target.path = this.model.read(cx).project_path(&root, &target.path);
                }
                this.go_to_target(target, window, cx)
            }),
            cx.subscribe(&view, |this, _, _: &crate::ui::diff_view::FilesChanged, cx| {
                this.model.update(cx, |m, cx| m.reload(cx));
            }),
            cx.subscribe_in(&view, window, |this, diff, event: &crate::ui::diff_view::DiffEvent, window, cx| this.on_diff_event(diff, event, window, cx)),
        ];
        view.update(cx, |diff, cx| diff.show(repository, source, cx));
        self.diff_tabs.push((view, subscriptions));
        self.focus_group(0, cx);
        self.front = Front::DiffTab(self.diff_tabs.len() - 1);
        cx.notify();
    }

    /// A diff's repository: local changes have project paths, and in a
    /// multi-root project open in the root that holds the file.
    fn route_diff(&self, source: crate::ui::diff_view::DiffSource, cx: &gpui_kit::App) -> Option<(crate::git::Repository, crate::ui::diff_view::DiffSource)> {
        use crate::ui::diff_view::DiffSource;
        let mut repository = self.model.read(cx).repository().cloned()?;
        let mut source = source;
        if !self.model.read(cx).root_states().is_empty() {
            if let DiffSource::WorkingTree { path, .. }
            | DiffSource::Staged { path }
            | DiffSource::Unstaged { path }
            | DiffSource::Files { path, .. }
            | DiffSource::Clipboard { path, .. } = &mut source
            {
                if let Some((root, own)) = self.model.read(cx).route(path) {
                    repository = root;
                    *path = own;
                }
            }
        }
        Some((repository, source))
    }

    /// The local side of a diff is what the editor holds: unsaved text is
    /// saved first (IntelliJ diffs the document itself).
    fn save_before_diff(&mut self, source: &crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffSource;
        let (DiffSource::WorkingTree { path, .. } | DiffSource::Unstaged { path } | DiffSource::Clipboard { path, .. }) = source else { return };
        let views: Vec<_> = self.editors.iter().chain(self.split.iter().flat_map(|s| s.editors.iter())).map(|t| t.view.clone()).collect();
        for view in views {
            if view.read(cx).revision().is_none() && view.read(cx).path() == path {
                view.update(cx, |editor, cx| editor.save_now(cx));
            }
        }
    }

    pub(super) fn open_diff(&mut self, source: crate::ui::diff_view::DiffSource, cx: &mut Context<Self>) {
        use crate::ui::diff_view::DiffSource;
        self.save_before_diff(&source, cx);
        let Some(mut repository) = self.model.read(cx).repository().cloned() else { return };
        // Local changes have paths relative to the project: in a multi-root
        // project the diff opens in the root that holds the file.
        let mut source = source;
        if !self.model.read(cx).root_states().is_empty() {
            if let DiffSource::WorkingTree { path, .. }
            | DiffSource::Staged { path }
            | DiffSource::Unstaged { path }
            | DiffSource::Files { path, .. }
            | DiffSource::Clipboard { path, .. } = &mut source
            {
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

/// The project's first module `res` folder with strings: `app/src/main/res`.
fn first_res(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let modules = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir());
    let mut found: Vec<std::path::PathBuf> = modules.map(|m| m.join("src/main/res")).filter(|r| r.join("values/strings.xml").is_file()).collect();
    found.sort();
    found.into_iter().next()
}

/// The Android modules Inspect Code runs Lint on for a scope, by build:
/// those inside it and those holding a file or folder of it, each with the
/// variant picked in Build Variants.
fn lint_plan(root: &std::path::Path, items: &[(String, bool)], cx: &mut gpui_kit::App) -> Vec<(std::path::PathBuf, Vec<crate::usecase::inspect_code::LintModule>)> {
    let sync = crate::ui::sync_service::SyncService::global(cx);
    let cx: &gpui_kit::App = cx;
    let mut out = Vec::new();
    for (build_root, modules) in sync.read(cx).builds() {
        let picked: Vec<crate::usecase::inspect_code::LintModule> = modules
            .iter()
            .filter(|m| m.kind.is_android())
            .filter(|m| {
                let Ok(rel) = m.dir.strip_prefix(root) else { return false };
                let rel = rel.to_string_lossy().replace('\\', "/");
                items.iter().any(|(item, _)| item.is_empty() || rel == *item || rel.starts_with(&format!("{item}/")) || item.starts_with(&format!("{rel}/")) || rel.is_empty())
            })
            .map(|m| crate::usecase::inspect_code::LintModule { path: m.path.clone(), dir: m.dir.clone(), variant: crate::ui::sync_service::variant_of(build_root, m, cx) })
            .collect();
        if !picked.is_empty() {
            out.push((build_root.clone(), picked));
        }
    }
    out
}

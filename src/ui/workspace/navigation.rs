//! Code navigation: go to declaration / usages, Back / Forward, Recent
//! Files, File Structure, Go to Line.

use super::*;

impl Workspace {
    /// Go to Declaration's result: one target opens, several ask.
    pub(super) fn navigate(&mut self, targets: Vec<crate::index::nav::Target>, at: Option<gpui_kit::Point<gpui_kit::Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        match targets.len() {
            0 => Self::nav_hint("Cannot find declaration to go to", window, cx),
            1 => self.go_to_target(targets.into_iter().next().unwrap(), window, cx),
            _ => {
                let workspace = cx.entity().downgrade();
                let locations = targets.iter().map(|t| self.location_label(t, cx)).collect();
                crate::ui::navigate::choose_target(
                    "Choose Declaration",
                    targets,
                    locations,
                    at,
                    Rc::new(move |target, window, cx| {
                        workspace.update(cx, |this, cx| this.go_to_target(target, window, cx)).ok();
                    }),
                    window,
                    cx,
                );
            }
        }
    }

    /// A navigation miss, as IntelliJ's hint: information, not success.
    pub(super) fn nav_hint(message: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(Notification::info(message.to_owned()).title("Go to Declaration"), cx);
    }

    /// Where a target is, for a chooser row: "dir/File.kt:12", or for a
    /// library file "JDK 17 › java/lang/String.java:120".
    pub(super) fn location_label(&self, target: &crate::index::nav::Target, cx: &gpui_kit::App) -> String {
        let line = target.line + 1;
        if crate::index::store::ProjectIndex::is_external(&target.path) {
            let index = self.code_index.read(cx).index.clone();
            let library = index.read().ok().and_then(|index| {
                let library = index.external.library_of(&target.path)?;
                let rel = std::path::Path::new(&target.path).strip_prefix(&library.root).ok()?.to_string_lossy().replace('\\', "/");
                Some(format!("{} › {rel}:{line}", library.name))
            });
            if let Some(label) = library {
                return label;
            }
        }
        format!("{}:{line}", target.path)
    }

    /// The implementation chooser's rows, as IntelliJ's: the class (a
    /// member's owner), its package, visibility, and module or library.
    pub(super) fn symbol_rows(&self, targets: &[crate::index::nav::Target], cx: &gpui_kit::App) -> Vec<crate::ui::navigate::SymbolRow> {
        use crate::index::store::ProjectIndex;
        let index = self.code_index.read(cx).index.clone();
        let Ok(index) = index.read() else { return Vec::new() };
        let root = index.root.clone();
        let project = gradle_project_name(&root);
        targets
            .iter()
            .map(|t| {
                let member = t.container.is_some() && matches!(t.label.split(' ').next(), Some("method" | "function" | "property" | "field" | "constructor"));
                let (name, outer) = match (&t.container, member) {
                    (Some(c), true) => match c.rsplit_once('.') {
                        Some((outer, owner)) => (owner.to_owned(), Some(outer.to_owned())),
                        None => (c.clone(), None),
                    },
                    (c, _) => (t.name.clone(), c.clone()),
                };
                let file = if ProjectIndex::is_external(&t.path) { std::path::PathBuf::from(&t.path) } else { root.join(&t.path) };
                let text = std::fs::read_to_string(&file).unwrap_or_default();
                let kotlin = t.path.ends_with(".kt") || t.path.ends_with(".kts");
                let jvm = kotlin || t.path.ends_with(".java");
                let package = text.lines().take(200).map(str::trim).find_map(|l| l.strip_prefix("package ")).map(|p| p.trim().trim_end_matches(';').trim().to_owned());
                let package = match (package, outer) {
                    (Some(p), Some(o)) => Some(format!("{p}.{o}")),
                    (p, o) => p.or(o),
                };
                let symbol = index.file(&t.path).and_then(|e| e.symbols.iter().find(|s| s.kind.is_type() && s.name == name && (member || s.line == t.line)).cloned());
                let icon: gpui_kit::component::Icon = match &symbol {
                    Some(s) => crate::ui::project_view::type_icon(s, kotlin).into(),
                    None => crate::ui::navigate::symbol_icon(crate::index::symbols::SymbolKind::Class),
                };
                let visibility = jvm.then(|| {
                    let line = text.lines().nth(t.line as usize).unwrap_or("");
                    let words: Vec<&str> = line.split(|c: char| !c.is_alphanumeric()).collect();
                    let has = |w: &str| words.contains(&w);
                    let icon = if has("private") {
                        crate::ui::as_icons::VISIBILITY_PRIVATE
                    } else if has("protected") {
                        crate::ui::as_icons::VISIBILITY_PROTECTED
                    } else if has("internal") || !kotlin && !has("public") && !line.contains("interface") {
                        crate::ui::as_icons::VISIBILITY_PACKAGE
                    } else {
                        crate::ui::as_icons::VISIBILITY_PUBLIC
                    };
                    gpui_kit::component::Icon::from(icon)
                });
                let library = ProjectIndex::is_external(&t.path);
                let location = if library {
                    index
                        .external
                        .library_of(&t.path)
                        .map(|l| match &l.root_label {
                            Some(label) => format!("{} ({label})", l.name),
                            None => l.name.clone(),
                        })
                        .unwrap_or_else(|| t.path.clone())
                } else {
                    module_label(&root, &project, &t.path)
                };
                crate::ui::navigate::SymbolRow { name, package, icon, visibility, location, library }
            })
            .collect()
    }

    /// Go to Declaration on a declaration: its usages, as IntelliJ's Show
    /// Usages popup (one usage jumps straight there).
    pub(super) fn show_usages_popup(&mut self, path: String, text: String, offset: usize, at: Option<gpui_kit::Point<gpui_kit::Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.code_index.read(cx).usages(path, text, offset, cx);
        cx.spawn_in(window, async move |this, cx| {
            let (word, usages) = task.await;
            this.update_in(cx, |this, window, cx| {
                let targets: Vec<crate::index::nav::Target> = usages
                    .into_iter()
                    .filter(|u| u.group != "Declarations")
                    .map(|u| crate::index::nav::Target { path: u.path, line: u.line, col: u.col, name: u.text, label: String::new(), container: None })
                    .collect();
                match targets.len() {
                    0 => window.push_notification(Notification::info(format!("No usages of {word} found")).title("Show Usages"), cx),
                    1 => this.go_to_target(targets.into_iter().next().unwrap(), window, cx),
                    _ => {
                        let locations = targets.iter().map(|t| this.location_label(t, cx)).collect();
                        let n = targets.len();
                        crate::ui::navigate::choose_target(format!("Usages of {word}   {n} usages"), targets, locations, at, this.picker_callback(cx), window, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens a file at a position, remembering where we were for Back.
    pub(super) fn go_to_target(&mut self, target: crate::index::nav::Target, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        self.recent_places.extend_visit(here.clone());
        self.nav.jumped_from(here);
        self.open_at(target.path, target.line, target.col, window, cx);
    }

    pub(super) fn current_position(&self, cx: &gpui_kit::App) -> Option<(String, u32, u32)> {
        let editor = self.editor()?.read(cx);
        if editor.revision().is_some() {
            return None;
        }
        let (line, col) = editor.cursor(cx);
        Some((editor.path().to_owned(), line, col))
    }

    pub(super) fn open_at(&mut self, path: String, line: u32, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file(path, None, window, cx);
        if let Some(editor) = self.editor().cloned() {
            editor.update(cx, |editor, cx| editor.go_to(line, col, window, cx));
        }
    }

    /// Ctrl+Alt+Down / Up: the Find window's next / previous result, opened.
    /// Without results the keys go on to the editor (add a caret).
    pub(super) fn step_occurrence(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.find.read(cx).has_occurrences() {
            cx.propagate();
            return;
        }
        self.find.update(cx, |find, cx| find.step(delta, cx));
    }

    pub(super) fn navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        self.recent_places.extend_visit(here.clone());
        let Some((path, line, col)) = self.nav.back(here) else { return };
        self.open_at(path, line, col, window, cx);
    }

    pub(super) fn navigate_forward(&mut self, _: &NavigateForward, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        self.recent_places.extend_visit(here.clone());
        let Some((path, line, col)) = self.nav.forward(here) else { return };
        self.open_at(path, line, col, window, cx);
    }

    pub(super) fn picker_callback(&self, cx: &mut Context<Self>) -> Rc<dyn Fn(crate::index::nav::Target, &mut Window, &mut gpui_kit::App)> {
        let workspace = cx.entity().downgrade();
        Rc::new(move |target, window, cx| {
            workspace.update(cx, |this, cx| this.go_to_target(target, window, cx)).ok();
        })
    }

    /// A bookmark's place: a project path for files in the project.
    fn bookmark_target(&self, bookmark: &crate::bookmarks::Bookmark, cx: &gpui_kit::App) -> crate::index::nav::Target {
        let file = std::path::Path::new(&bookmark.file);
        let path = match self.model.read(cx).project_root().and_then(|root| file.strip_prefix(root).ok()) {
            Some(own) => own.to_string_lossy().replace('\\', "/"),
            None => bookmark.file.clone(),
        };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        crate::index::nav::Target { path, line: bookmark.line, col: 0, name, label: String::new(), container: None }
    }

    /// A bookmark of this project's files or of library sources (which sit
    /// in no other repository), not another project's.
    fn in_project(bookmark: &crate::bookmarks::Bookmark, root: Option<&std::path::Path>) -> bool {
        let file = std::path::Path::new(&bookmark.file);
        root.is_none_or(|r| file.starts_with(r) || !file.ancestors().skip(1).any(|d| d.join(".git").exists()))
    }

    /// Show All Bookmarks (Shift+F11): the project's bookmarks, mnemonic
    /// ones first, as IntelliJ's list.
    pub(super) fn show_bookmarks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.model.read(cx).project_root().map(|r| r.to_path_buf());
        let mut list: Vec<crate::bookmarks::Bookmark> = crate::bookmarks::Bookmarks::get(cx)
            .list
            .iter()
            .filter(|b| Self::in_project(b, root.as_deref()))
            .cloned()
            .collect();
        list.sort_by(|a, b| (a.mnemonic.is_none(), a.mnemonic, &a.file, a.place()).cmp(&(b.mnemonic.is_none(), b.mnemonic, &b.file, b.place())));
        if list.is_empty() {
            Self::nav_hint("No bookmarks. Add one with F11 or the gutter menu", window, cx);
            return;
        }
        let items: Vec<crate::ui::find_view::FoundItem> = list
            .iter()
            .map(|b| {
                let target = self.bookmark_target(b, cx);
                let prefix = b.mnemonic.map(|m| format!("{m}  ")).unwrap_or_default();
                crate::ui::find_view::FoundItem {
                    title: match b.place() {
                        Some(line) => format!("{prefix}{}:{}", target.name, line + 1),
                        None => format!("{prefix}{}", target.name),
                    },
                    detail: b.text.clone(),
                    icon: gpui_kit::component::Icon::new(crate::ui::as_icons::TW_BOOKMARKS),
                    target,
                }
            })
            .collect();
        let n = items.len();
        crate::ui::navigate::pick_from_list("Bookmarks", items, vec![0; n], self.picker_callback(cx), window, cx);
    }

    /// Go to Bookmark 0–9 (Ctrl+digit).
    pub(super) fn go_to_bookmark(&mut self, mnemonic: char, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.model.read(cx).project_root().map(|r| r.to_path_buf());
        let Some(bookmark) = crate::bookmarks::Bookmarks::get(cx).with_mnemonic(mnemonic).filter(|b| Self::in_project(b, root.as_deref())).cloned() else { return };
        let target = self.bookmark_target(&bookmark, cx);
        self.go_to_target(target, window, cx);
    }

    /// Recent Locations (Ctrl+Shift+E): the places visited and changed,
    /// the current one first.
    pub(super) fn recent_locations_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        self.recent_places.extend_visit(here);
        let root = self.model.read(cx).project_root().map(|r| r.to_path_buf());
        let dark = cx.palette().dark;
        let mut texts: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
        let mut highlighters = std::collections::HashMap::new();
        let mut locations = Vec::new();
        for place in self.recent_places.iter() {
            let text = texts.entry(place.path.clone()).or_insert_with(|| {
                let open = self.editors.iter().find(|t| t.view.read(cx).path() == place.path && t.view.read(cx).revision().is_none());
                match open {
                    Some(tab) => Some(tab.view.read(cx).text(cx)),
                    None => {
                        let file = std::path::Path::new(&place.path);
                        let file = if file.is_absolute() { file.to_path_buf() } else { root.as_ref().map(|r| r.join(file)).unwrap_or_default() };
                        std::fs::read_to_string(file).ok()
                    }
                }
            });
            if let Some(text) = text {
                locations.push(crate::ui::recent_locations::Location::new(place.path.clone(), place.line, place.col, place.changed, text, dark, &mut highlighters));
            }
        }
        if locations.is_empty() {
            window.push_notification(Notification::info("No recent locations").title("Recent Locations"), cx);
            return;
        }
        crate::ui::recent_locations::RecentLocations::open(locations, self.picker_callback(cx), window, cx);
    }

    /// Navigate › Last Edit Location (Ctrl+Shift+Backspace): the latest
    /// place edited; pressed again from there, the one before, and so on.
    pub(super) fn last_edit_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        let near = |p: &history::Place| here.as_ref().is_some_and(|(path, line, _)| *path == p.path && line.abs_diff(p.line) <= 2);
        let (places, mut next) = match self.edit_walk.take() {
            Some((places, i, at)) if Some(&at) == here.as_ref() => (places, i + 1),
            _ => {
                let places: Vec<history::Place> = self.recent_places.iter().filter(|p| p.changed).cloned().collect();
                // Already at the latest edit: the one before it.
                let skip = places.first().is_some_and(near) as usize;
                (places, skip)
            }
        };
        while places.get(next).is_some_and(|p| near(p) && next > 0) {
            next += 1;
        }
        let Some(place) = places.get(next).cloned() else {
            window.push_notification(Notification::info("No more edit locations").title("Last Edit Location"), cx);
            return;
        };
        self.nav.jumped_from(here);
        self.open_at(place.path.clone(), place.line, place.col, window, cx);
        let at = self.current_position(cx).unwrap_or((place.path.clone(), place.line, place.col));
        self.edit_walk = Some((places, next, at));
    }

    /// Recent Files (Ctrl+E).
    pub(super) fn recent_files_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.editor().map(|e| e.read(cx).path().to_owned());
        // IntelliJ preselects the previous file, so Ctrl+E Enter switches back.
        let mut files: Vec<String> = self.recent_files.iter().filter(|p| Some(*p) != current.as_ref()).cloned().collect();
        files.extend(current);
        let items = files
            .into_iter()
            .map(|path| {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
                // A library file shows its library and folder, not a cache path.
                let target = crate::index::nav::Target { path: path.clone(), line: 0, col: 0, name: String::new(), label: String::new(), container: None };
                let location = self.location_label(&target, cx);
                let location = location.rsplit_once(':').map_or(location.as_str(), |(l, _)| l);
                let dir = match location.rsplit_once(['/', '\\']) {
                    Some((d, _)) => d.to_owned(),
                    None => location.split_once(" › ").map(|(library, _)| library.to_owned()).unwrap_or_default(),
                };
                crate::ui::find_view::FoundItem {
                    title: name.clone(),
                    detail: dir,
                    icon: crate::ui::common::file_icon(&path),
                    target: crate::index::nav::Target { path, line: 0, col: 0, name, label: String::new(), container: None },
                }
            })
            .collect::<Vec<_>>();
        let n = items.len();
        let on_pick = self.picker_callback(cx);
        let recent: Vec<String> = self.recent_files.to_vec();
        let workspace = cx.entity().downgrade();
        // Re-opening a recent file keeps its last caret: open without a position.
        let open_plain: Rc<dyn Fn(crate::index::nav::Target, &mut Window, &mut gpui_kit::App)> = Rc::new(move |target, window, cx| {
            let _ = &recent;
            let path = target.path.clone();
            workspace.update(cx, |this, cx| this.open_file(path, None, window, cx)).ok();
        });
        let _ = on_pick;
        crate::ui::navigate::pick_from_list("Recent Files", items, vec![0; n], open_plain, window, cx);
    }

    /// File Structure (Ctrl+F12): the current file's declarations.
    pub(super) fn file_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let path = editor.read(cx).path().to_owned();
        let symbols = self.code_index.read(cx).file_symbols(&path);
        let mut indents = Vec::new();
        let items = symbols
            .into_iter()
            .map(|m| {
                indents.push(m.target.container.as_deref().map(|c| c.split('.').count()).unwrap_or(0));
                crate::ui::find_view::FoundItem {
                    title: m.target.name.clone(),
                    detail: m.target.label.split(" · ").next().unwrap_or_default().to_owned(),
                    icon: crate::ui::navigate::symbol_icon(m.kind),
                    target: m.target,
                }
            })
            .collect();
        let on_pick = self.picker_callback(cx);
        crate::ui::navigate::pick_from_list("File Structure", items, indents, on_pick, window, cx);
    }

    /// Go to Line:Column (Ctrl+G).
    pub(super) fn goto_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor() else { return };
        let editor = editor.clone();
        let (line, col) = editor.read(cx).cursor(cx);
        let lines = editor.read(cx).line_count(cx);
        let input = cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx).default_value(format!("{}:{}", line + 1, col + 1)));
        let workspace = cx.entity().downgrade();
        let go = {
            let (input, editor, workspace) = (input.clone(), editor.clone(), workspace.clone());
            move |window: &mut Window, cx: &mut gpui_kit::App| {
                let text = input.read(cx).value().to_string();
                let mut parts = text.split([':', ',']).map(|p| p.trim().parse::<u32>().ok());
                let Some(Some(line)) = parts.next() else { return };
                let col = parts.next().flatten().unwrap_or(1);
                let target_line = line.saturating_sub(1);
                let (path, here) = { let e = editor.read(cx); (e.path().to_owned(), e.cursor(cx)) };
                workspace
                    .update(cx, |this, cx| {
                        this.nav.remember((path.clone(), here.0, here.1));
                        this.recent_places.visit((path.clone(), here.0, here.1));
                        editor.update(cx, |e, cx| e.go_to(target_line, col.saturating_sub(1), window, cx));
                    })
                    .ok();
            }
        };
        let go = Rc::new(go);
        let sub_go = go.clone();
        let subscription = cx.subscribe_in(&input, window, move |_, _, event: &gpui_kit::component::input::InputEvent, window, cx| {
            if let gpui_kit::component::input::InputEvent::PressEnter { .. } = event {
                sub_go(window, cx);
                window.close_dialog(cx);
            }
        });
        let input2 = input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let _ = &subscription;
            let go = go.clone();
            dialog
                .title("Go to Line:Column")
                .w(px(360.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(format!("[Line] [:column]   (1–{lines})")))
                        .child(gpui_kit::component::input::Input::new(&input2)),
                )
                .footer(dialogs::footer("OK"))
                .on_ok(move |_, window, cx| {
                    go(window, cx);
                    true
                })
        });
        // IntelliJ selects the prefilled position, so typing replaces it.
        let select = input.clone();
        window.defer(cx, move |window, cx| {
            select.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            })
        });
    }
}

/// The root project's name as Gradle (and Android Studio) has it:
/// settings.gradle's `rootProject.name`, else the folder's, spaces as `_`.
fn gradle_project_name(root: &std::path::Path) -> String {
    let declared = ["settings.gradle.kts", "settings.gradle"].iter().find_map(|f| {
        let text = std::fs::read_to_string(root.join(f)).ok()?;
        let line = text.lines().find(|l| l.trim_start().starts_with("rootProject.name"))?;
        let value = line.split_once('=')?.1.trim();
        Some(value.trim_matches(|c| c == '"' || c == '\'' || c == ';').to_owned())
    });
    let name = declared.unwrap_or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    name.replace(' ', "_")
}

/// "My_TV.app.main": the project, the Gradle module holding `path`, and its
/// source set, as Android Studio names modules.
fn module_label(root: &std::path::Path, project: &str, path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let mut module: Vec<&str> = Vec::new();
    let mut rest = &parts[..parts.len().saturating_sub(1)];
    // The innermost folder with a build script.
    for n in (1..parts.len()).rev() {
        let dir = root.join(parts[..n].join("/"));
        if dir.join("build.gradle.kts").is_file() || dir.join("build.gradle").is_file() {
            module = parts[..n].to_vec();
            rest = &parts[n..parts.len() - 1];
            break;
        }
    }
    let mut out = vec![project.to_owned()];
    out.extend(module.iter().map(|m| m.to_string()));
    if let Some(i) = rest.iter().position(|p| *p == "src") {
        if let Some(set) = rest.get(i + 1) {
            out.push(set.to_string());
        }
    }
    out.join(".")
}

//! Code navigation: go to declaration / usages, Back / Forward, Recent
//! Files, File Structure, Go to Line.

use super::*;

impl Workspace {
    /// Go to Declaration's result: one target opens, several ask.
    pub(super) fn navigate(&mut self, targets: Vec<crate::index::nav::Target>, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Go to Declaration on a declaration: its usages, as IntelliJ's Show
    /// Usages popup (one usage jumps straight there).
    pub(super) fn show_usages_popup(&mut self, path: String, text: String, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
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
                        crate::ui::navigate::choose_target("Usages", targets, locations, this.picker_callback(cx), window, cx);
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
        let Some((path, line, col)) = self.nav.back(here) else { return };
        self.open_at(path, line, col, window, cx);
    }

    pub(super) fn navigate_forward(&mut self, _: &NavigateForward, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.current_position(cx);
        let Some((path, line, col)) = self.nav.forward(here) else { return };
        self.open_at(path, line, col, window, cx);
    }

    pub(super) fn picker_callback(&self, cx: &mut Context<Self>) -> Rc<dyn Fn(crate::index::nav::Target, &mut Window, &mut gpui_kit::App)> {
        let workspace = cx.entity().downgrade();
        Rc::new(move |target, window, cx| {
            workspace.update(cx, |this, cx| this.go_to_target(target, window, cx)).ok();
        })
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

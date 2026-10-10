//! Find / Replace in Files and Search Everywhere popups.

use super::*;

impl Workspace {
    /// The Module / Scope tabs' view of the project.
    pub(super) fn scope_data(&self, cx: &gpui_kit::App) -> crate::ui::find_popup::ScopeData {
        let current_file = self.editor().filter(|e| e.read(cx).revision().is_none()).map(|e| e.read(cx).path().to_owned());
        let open_files: Vec<String> = self.editors.iter().filter(|t| t.view.read(cx).revision().is_none()).map(|t| t.view.read(cx).path().to_owned()).collect();
        let local_changes: Vec<String> = self.model.read(cx).status().entries.iter().map(|e| e.path.clone()).collect();
        let mut recently_changed = self.recently_changed.clone();
        recently_changed.extend(local_changes.iter().filter(|p| !self.recently_changed.contains(p)).cloned());
        let modules = self.code_index.read(cx).root().map(|root| modules_of(&crate::index::store::list_files(root))).unwrap_or_default();
        crate::ui::find_popup::ScopeData {
            open_files,
            current_file,
            recent_files: self.recent_files.clone(),
            recently_changed,
            local_changes,
            modules,
        }
    }

    /// Find in Files (Ctrl+Shift+F) / Replace in Files (Ctrl+Shift+R).
    pub(super) fn open_find_popup(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if self.find_popup.is_none() {
            let index = self.code_index.clone();
            let popup = cx.new(|cx| crate::ui::find_popup::FindPopup::new(index, window, cx));
            let subscription = cx.subscribe_in(&popup, window, |this, _, event: &crate::ui::find_popup::FindEvent, window, cx| {
                use crate::ui::find_popup::FindEvent;
                match event {
                    FindEvent::Open(target) => this.go_to_target(target.clone(), window, cx),
                    FindEvent::Close => this.close_popups(window, cx),
                    FindEvent::ShowInFindWindow(request) => {
                        let request = request.clone();
                        this.find.update(cx, |view, cx| view.find_text(request, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                    FindEvent::FilesChanged(paths) => this.files_changed(paths.clone(), window, cx),
                }
            });
            self.find_popup = Some((popup, subscription));
        }
        let text = self.editor().and_then(|e| e.read(cx).search_text(cx));
        let data = self.scope_data(cx);
        if let Some((popup, _)) = &self.find_popup {
            popup.update(cx, |p, cx| p.show(replace, text, data, window, cx));
        }
        self.show_find_popup = true;
        cx.notify();
    }

    pub(super) fn close_popups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find_popup = false;
        self.show_search_everywhere = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn render_popups(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let mut out = Vec::new();
        let popup = self.find_popup.as_ref().filter(|_| self.show_find_popup).map(|(p, _)| (p.clone(), p.read(cx).is_pinned()));
        if let Some((popup, pinned)) = popup {
            if !pinned {
                out.push(
                    div()
                        .id("popup-backdrop")
                        .absolute()
                        .inset_0()
                        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_popups(window, cx)))
                        .into_any_element(),
                );
            }
            out.push(div().absolute().top(px(72.)).left_0().right_0().flex().justify_center().child(popup).into_any_element());
        }
        if let Some((se, _)) = self.search_everywhere.as_ref().filter(|_| self.show_search_everywhere) {
            out.push(
                div()
                    .id("se-backdrop")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_popups(window, cx)))
                    .into_any_element(),
            );
            out.push(div().absolute().top(px(96.)).left_0().right_0().flex().justify_center().child(se.clone()).into_any_element());
        }
        out
    }

    /// Every command Find Action (Ctrl+Shift+A) and the Actions tab know.
    pub(super) fn action_entries(&self, cx: &mut Context<Self>) -> Vec<crate::ui::search_everywhere::ActionEntry> {
        use crate::ui::search_everywhere::ActionEntry;
        let weak = cx.entity().downgrade();
        let mut out = Vec::new();
        let mut add = |name: &str, shortcut: &str, group: &str, run: VcsRun| {
            let weak = weak.clone();
            out.push(ActionEntry {
                name: name.to_owned(),
                shortcut: crate::ui::file_menus::shortcut_label(shortcut),
                group: group.to_owned(),
                run: Rc::new(move |window, cx| {
                    weak.update(cx, |this, cx| run(this, window, cx)).ok();
                }),
            });
        };
        let op = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| -> VcsRun { Rc::new(f) };
        for (name, shortcut, run) in Self::vcs_operation_list().into_iter().flatten() {
            add(name, shortcut, "Git", run);
        }
        let navigate: Vec<(&str, &str, VcsRun)> = vec![
            ("Search Everywhere", "Double Shift", op(|this, window, cx| this.open_search_everywhere(SeTab::All, window, cx))),
            ("Find in Files…", "Ctrl+Shift+F", op(|this, window, cx| this.open_find_popup(false, window, cx))),
            ("Replace in Files…", "Ctrl+Shift+R", op(|this, window, cx| this.open_find_popup(true, window, cx))),
            ("Go to Class…", "Ctrl+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Classes, window, cx))),
            ("Go to File…", "Ctrl+Shift+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Files, window, cx))),
            ("Go to Symbol…", "Ctrl+Alt+Shift+N", op(|this, window, cx| this.open_search_everywhere(SeTab::Symbols, window, cx))),
            ("Go to Text…", "", op(|this, window, cx| this.open_search_everywhere(SeTab::Text, window, cx))),
            ("Recent Files", "Ctrl+E", op(|this, window, cx| this.recent_files_popup(window, cx))),
            ("File Structure", "Ctrl+F12", op(|this, window, cx| this.file_structure(window, cx))),
            ("Go to Line:Column…", "Ctrl+G", op(|this, window, cx| this.goto_line(window, cx))),
            ("Back", "Ctrl+Alt+Left", op(|this, window, cx| this.navigate_back(&NavigateBack, window, cx))),
            ("Forward", "Ctrl+Alt+Right", op(|this, window, cx| this.navigate_forward(&NavigateForward, window, cx))),
            ("Select in Project View", "Alt+F1", op(|this, window, cx| this.select_in_project(&SelectInProject, window, cx))),
        ];
        for (name, shortcut, run) in navigate {
            add(name, shortcut, "Navigate", run);
        }
        let windows: Vec<(&str, &str, VcsRun)> = vec![
            ("Project", "Alt+1", op(|this, window, cx| this.toggle_project(&ToggleProjectWindow, window, cx))),
            ("Find", "Alt+3", op(|this, window, cx| this.toggle_find(&ToggleFindWindow, window, cx))),
            ("Git", "Alt+9", op(|this, window, cx| this.on_toggle_git(&ToggleGitWindow, window, cx))),
            ("Commit", "Alt+0", op(|this, window, cx| this.show_left_tab(LeftTab::Commit, window, cx))),
            ("Refresh", "Ctrl+Alt+Y", op(|this, window, cx| this.on_refresh(&Refresh, window, cx))),
            ("VCS Operations Popup…", "Alt+`", op(|this, window, cx| this.on_vcs_operations(&VcsOperations, window, cx))),
            ("Close Tab", "Ctrl+F4", op(|this, _, cx| this.close_active_tab(cx))),
            ("Select Next Tab", "Alt+Right", op(|this, window, cx| this.step_tab(1, window, cx))),
            ("Select Previous Tab", "Alt+Left", op(|this, window, cx| this.step_tab(-1, window, cx))),
            ("Reopen Closed Tab", "", op(|this, window, cx| this.reopen_closed_tab(window, cx))),
            ("Close All Tabs", "", op(|this, _, cx| {
                let all = (0..this.editors.len()).collect();
                this.close_tabs(all, cx);
            })),
        ];
        for (name, shortcut, run) in windows {
            add(name, shortcut, "Window", run);
        }
        out
    }

    /// Search Everywhere on a tab; its own shortcut again toggles non-project items.
    pub(super) fn open_search_everywhere(&mut self, tab: SeTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        if let Some((se, _)) = &self.search_everywhere {
            if self.show_search_everywhere && se.read(cx).tab() == tab {
                se.update(cx, |se, cx| se.toggle_non_project(cx));
                return;
            }
        }
        if self.search_everywhere.is_none() {
            let index = self.code_index.clone();
            let se = cx.new(|cx| crate::ui::search_everywhere::SearchEverywhere::new(index, window, cx));
            let subscription = cx.subscribe_in(&se, window, |this, _, event: &crate::ui::search_everywhere::SeEvent, window, cx| {
                use crate::ui::search_everywhere::SeEvent;
                match event {
                    SeEvent::Open(target) => this.go_to_target(target.clone(), window, cx),
                    SeEvent::Close => this.close_popups(window, cx),
                    SeEvent::Run(run) => {
                        let run = run.clone();
                        window.defer(cx, move |window, cx| run(window, cx));
                    }
                    SeEvent::FindWindowText(request) => {
                        let request = request.clone();
                        this.find.update(cx, |view, cx| view.find_text(request, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                    SeEvent::FindWindowItems(title, items) => {
                        let (title, items) = (title.clone(), items.clone());
                        this.find.update(cx, |view, cx| view.show_items(title, items, cx));
                        this.tools.open(ToolWindow::Git);
                        this.bottom_tab = BottomTab::Find;
                        cx.notify();
                    }
                }
            });
            self.search_everywhere = Some((se, subscription));
        }
        self.show_find_popup = false;
        let text = self.editor().and_then(|e| e.read(cx).selected_text(cx));
        let actions = self.action_entries(cx);
        if let Some((se, _)) = &self.search_everywhere {
            se.update(cx, |se, cx| se.show(tab, text, actions, window, cx));
        }
        self.show_search_everywhere = true;
        cx.notify();
    }
}

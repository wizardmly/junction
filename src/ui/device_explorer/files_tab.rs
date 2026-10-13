//! The Files tab: the device's tree, selection and keys, New / Open /
//! Save As / Upload / Delete / Synchronize, its menu and its table.

use super::*;

impl DeviceExplorer {
    pub(super) fn load_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        let filter = self.filtering().then(|| self.app_ids.clone());
        let Some(files) = self.files.as_mut() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let run_as = files.run_as(path);
        let serial = files.serial.clone();
        if !matches!(files.dirs.get(path), Some(Dir::Loaded(_))) {
            files.dirs.insert(path.to_owned(), Dir::Loading);
        }
        files.rebuild();
        cx.notify();
        let path = path.to_owned();
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn({
                    let (fs, path, run_as) = (fs.clone(), path.clone(), run_as.clone());
                    async move {
                        let mut result = fs.list(&path, run_as.as_deref());
                        // The package filter: only the project's folders under /data/data.
                        if let (Ok(entries), Some(ids)) = (&mut result, &filter) {
                            if path == "/data/data" {
                                entries.retain(|e| ids.contains(&e.name));
                            }
                        }
                        let links: HashSet<String> = match &result {
                            Ok(entries) => {
                                let links: Vec<String> = entries.iter().filter(|e| e.kind == EntryKind::Symlink).map(|e| e.path.clone()).collect();
                                fs.dir_links(&links, run_as.as_deref())
                            }
                            Err(_) => HashSet::new(),
                        };
                        (result, links)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(files) = this.files.as_mut().filter(|f| f.serial == serial) else { return };
                let (result, links) = listed;
                match result {
                    Ok(entries) => {
                        let children: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
                        // Forget what was below entries that are gone.
                        if let Some(Dir::Loaded(old)) = files.dirs.get(&path) {
                            for gone in old.iter().filter(|p| !children.contains(p)) {
                                files.entries.remove(gone);
                                files.expanded.remove(gone);
                            }
                        }
                        for entry in entries {
                            files.dir_links.remove(&entry.path);
                            files.entries.insert(entry.path.clone(), entry);
                        }
                        files.dir_links.extend(links);
                        files.dirs.insert(path.clone(), Dir::Loaded(children));
                    }
                    Err(error) => {
                        files.dirs.insert(path.clone(), Dir::Failed(error));
                    }
                }
                files.rebuild();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_expanded(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_mut() else { return };
        if !files.is_dir(path) {
            return;
        }
        if !files.expanded.remove(path) {
            files.expanded.insert(path.to_owned());
            if !matches!(files.dirs.get(path), Some(Dir::Loaded(_))) {
                self.load_dir(path, cx);
                return;
            }
        }
        files.rebuild();
        cx.notify();
    }

    pub(super) fn expand(&mut self, path: &str, cx: &mut Context<Self>) {
        let expanded = self.files.as_ref().is_some_and(|f| f.expanded.contains(path));
        if !expanded {
            self.toggle_expanded(path, cx);
        }
    }

    pub(super) fn click_row(&mut self, path: &str, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if event.click_count() >= 2 {
            let is_dir = self.files.as_ref().is_some_and(|f| f.is_dir(path));
            if is_dir {
                self.toggle_expanded(path, cx);
            } else {
                self.open(vec![path.to_owned()], cx);
            }
            return;
        }
        self.select_file(path, event.modifiers(), cx);
    }

    pub(super) fn select_file(&mut self, path: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_mut() else { return };
        if modifiers.shift {
            let anchor = files.anchor.clone().unwrap_or_else(|| path.to_owned());
            let (Some(a), Some(b)) = (files.row_of(&anchor), files.row_of(path)) else { return };
            let range = a.min(b)..=a.max(b);
            files.selected = files.rows[range].iter().filter_map(|r| if let Row::Entry { path, .. } = r { Some(path.clone()) } else { None }).collect();
        } else if modifiers.secondary() {
            if let Some(ix) = files.selected.iter().position(|p| p == path) {
                files.selected.remove(ix);
            } else {
                files.selected.push(path.to_owned());
            }
            files.anchor = Some(path.to_owned());
        } else {
            files.selected = vec![path.to_owned()];
            files.anchor = Some(path.to_owned());
        }
        cx.notify();
    }

    pub(super) fn file_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let ctrl = m.control || m.platform;
        let Some(files) = self.files.as_ref() else { return };
        let selected = files.selected.clone();
        let handled = match k.key.as_str() {
            "s" if ctrl && m.shift && !selected.is_empty() => {
                self.save_as(selected, cx);
                true
            }
            "o" if ctrl && m.shift => {
                if let Some(dir) = files.selected_dir() {
                    self.upload(dir, window, cx);
                }
                true
            }
            "c" if ctrl && m.shift && !selected.is_empty() => {
                self.copy_paths(&selected, cx);
                true
            }
            "f5" if ctrl => {
                self.synchronize(cx);
                true
            }
            "delete" if !selected.is_empty() => {
                self.delete(selected, window, cx);
                true
            }
            "enter" if !selected.is_empty() => {
                let dirs: Vec<String> = selected.iter().filter(|p| files.is_dir(p)).cloned().collect();
                let plain: Vec<String> = selected.iter().filter(|p| !files.is_dir(p)).cloned().collect();
                for dir in dirs {
                    self.toggle_expanded(&dir, cx);
                }
                if !plain.is_empty() {
                    self.open(plain, cx);
                }
                true
            }
            "up" | "down" if !m.alt && !ctrl => {
                let current = selected.last().and_then(|p| files.row_of(p));
                let rows = &files.rows;
                let step = |from: usize, down: bool| -> Option<usize> {
                    let mut ix = from;
                    loop {
                        ix = if down { ix + 1 } else { ix.checked_sub(1)? };
                        match rows.get(ix)? {
                            Row::Entry { .. } => return Some(ix),
                            _ => continue,
                        }
                    }
                };
                let next = match current {
                    Some(ix) => step(ix, k.key == "down"),
                    None => rows.iter().position(|r| matches!(r, Row::Entry { .. })),
                };
                if let Some(Row::Entry { path, .. }) = next.and_then(|ix| rows.get(ix)) {
                    let path = path.clone();
                    let ix = next.unwrap_or(0);
                    let mods = Modifiers { shift: m.shift, ..Default::default() };
                    self.select_file(&path, mods, cx);
                    self.files_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
                true
            }
            "right" => {
                if let [path] = selected.as_slice() {
                    if files.is_dir(path) {
                        if files.expanded.contains(path) {
                            if let Some(Dir::Loaded(children)) = files.dirs.get(path) {
                                if let Some(first) = children.first().cloned() {
                                    self.select_file(&first, Modifiers::default(), cx);
                                }
                            }
                        } else {
                            let path = path.clone();
                            self.expand(&path, cx);
                        }
                    }
                }
                true
            }
            "left" => {
                if let [path] = selected.as_slice() {
                    let path = path.clone();
                    if files.expanded.contains(&path) {
                        self.toggle_expanded(&path, cx);
                    } else {
                        let parent = files::parent(&path).to_owned();
                        if parent != files.root && files.entries.contains_key(&parent) {
                            self.select_file(&parent, Modifiers::default(), cx);
                            if let Some(ix) = self.files.as_ref().and_then(|f| f.row_of(&parent)) {
                                self.files_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    pub(super) fn copy_paths(&self, paths: &[String], cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(paths.join("\n")));
    }

    /// Synchronize: every loaded folder that shows is listed again.
    pub(super) fn synchronize(&mut self, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let mut reload = vec![files.root.clone()];
        reload.extend(files.expanded.iter().filter(|p| matches!(files.dirs.get(*p), Some(Dir::Loaded(_)) | Some(Dir::Failed(_)))).cloned());
        for path in reload {
            self.load_dir(&path, cx);
        }
    }

    /// New › File / Directory: a name, created in the selected folder.
    pub(super) fn new_entry(&mut self, parent: String, directory: bool, initial: String, window: &mut Window, cx: &mut Context<Self>) {
        let (title, prompt, empty, failure) = if directory {
            ("New Folder", "Enter new folder name:", "Folder name cannot be empty", "Could not create folder")
        } else {
            ("New File", "Enter new file name:", "File name cannot be empty", "Could not create file")
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let focus = input.clone();
        let this = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = cx.palette().clone();
            let value = input.read(cx).value().trim().to_owned();
            let error = if value.contains('/') { Some("Path cannot contain \"/\" characters") } else if value.is_empty() { Some(empty) } else { None };
            let (input, this, parent) = (input.clone(), this.clone(), parent.clone());
            dialog
                .title(title)
                .w(px(400.))
                .child(
                    v_flex()
                        .gap_2()
                        .text_sm()
                        .child(prompt)
                        .child(Input::new(&input))
                        .when_some(error, |el, e| el.child(div().text_xs().text_color(palette.status_deleted).child(e))),
                )
                .footer(crate::ui::dialogs::footer_enabled("OK", error.is_none()))
                .on_ok(move |_, window, cx| {
                    let name = input.read(cx).value().trim().to_owned();
                    if name.is_empty() || name.contains('/') {
                        return false;
                    }
                    this.update(cx, |this, cx| this.create(parent.clone(), name, directory, failure, window, cx));
                    true
                })
        });
        crate::ui::dialogs::focus_input(&focus, window, cx);
    }

    pub(super) fn create(&mut self, parent: String, name: String, directory: bool, failure: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let run_as = files.run_as(&parent);
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let (parent, name) = (parent.clone(), name.clone());
                    async move {
                        if directory { fs.new_directory(&parent, &name, run_as.as_deref()) } else { fs.new_file(&parent, &name, run_as.as_deref()) }
                    }
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.load_dir(&parent, cx);
                    if let Some(files) = this.files.as_mut() {
                        files.expanded.insert(parent.clone());
                        let created = files::resolve(&parent, &name);
                        files.selected = vec![created.clone()];
                        files.anchor = Some(created);
                    }
                }
                Err(error) => {
                    let message = format!("{failure} '{name}':\n{error}");
                    let retry = !error.ends_with("Permission denied");
                    let this_entity = cx.entity();
                    window_handle
                        .update(cx, |_, window, cx| {
                            message_dialog("Error", message, window, cx);
                            if retry {
                                // Ask again with the name typed, as Android Studio does.
                                this_entity.update(cx, |this, cx| this.new_entry(parent.clone(), directory, name.clone(), window, cx));
                            }
                        })
                        .ok();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open: copy to the computer, then show in the editor.
    pub(super) fn open(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let entries: Vec<Entry> =
            paths.iter().filter_map(|p| files.entries.get(p)).filter(|e| !e.is_dir() && !files.dir_links.contains(&e.path)).cloned().collect();
        if entries.is_empty() {
            return;
        }
        let names = entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>().join(", ");
        let serial = files.serial.clone();
        if !self.start_busy(format!("Downloading {names}"), cx) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move {
                    entries
                        .into_iter()
                        .map(|entry| {
                            let local = files::local_copy(&serial, &entry.path);
                            let _ = std::fs::remove_file(&local);
                            fs.download(&entry, &local).map(|_| local).map_err(|e| format!("Error opening contents of device file {}: {e}", entry.path))
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                for result in results {
                    match result {
                        Ok(local) => cx.emit(DeviceExplorerEvent::Open(local)),
                        Err(error) => this.notify("Device Explorer", error, true, cx),
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Save As… (one entry) / Save To… (several): pull to a place picked on the computer.
    pub(super) fn save_as(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        let entries: Vec<(Entry, bool)> = paths.iter().filter_map(|p| files.entries.get(p).map(|e| (e.clone(), files.is_dir(p)))).collect();
        if entries.is_empty() {
            return;
        }
        if self.busy.is_some() {
            self.notify("Device Explorer", BUSY, true, cx);
            return;
        }
        let base = crate::ui::running_devices::default_save_dir();
        // One file: a file name; a folder or several entries: a folder to save into.
        let single_file = entries.len() == 1 && !entries[0].1;
        let pick: Task<Option<Vec<(Entry, PathBuf)>>> = if single_file {
            let name = entries[0].0.name.clone();
            let answer = cx.prompt_for_new_path(&base, Some(&name));
            let entry = entries[0].0.clone();
            cx.spawn(async move |_, _| crate::ui::running_devices::chosen_path(answer.await, &name).map(|path| vec![(entry, path)]))
        } else {
            let answer = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some(if entries.len() > 1 { "Save To".into() } else { "Save As".into() }),
            });
            cx.spawn(async move |_, _| {
                let folder = match answer.await {
                    Ok(Ok(Some(mut paths))) => paths.pop()?,
                    Ok(Ok(None)) => return None,
                    _ => base,
                };
                Some(entries.into_iter().map(|(entry, _)| {
                    let target = folder.join(&entry.name);
                    (entry, target)
                }).collect())
            })
        };
        cx.spawn(async move |this, cx| {
            let Some(targets) = pick.await else { return };
            let names = targets.iter().map(|(e, _)| e.name.clone()).collect::<Vec<_>>().join(", ");
            if !this.update(cx, |this, cx| this.start_busy(format!("Downloading {names}"), cx)).unwrap_or(false) {
                return;
            }
            let started = Instant::now();
            let (totals, problems) = cx
                .background_spawn(async move {
                    let mut totals = Totals::default();
                    let mut problems = Vec::new();
                    for (entry, target) in targets {
                        match fs.download(&entry, &target) {
                            Ok(()) => totals.add(Totals::of(&target)),
                            Err(e) => problems.push(format!("Error saving contents of device file {}: {e}", entry.path)),
                        }
                    }
                    (totals, problems)
                })
                .await;
            let (ok, message) = files::transfer_summary(totals, started.elapsed().as_millis(), &problems, true);
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                this.notify("Device Explorer", message, !ok, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Upload…: files from the computer into the selected folder.
    pub(super) fn upload(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            self.notify("Device Explorer", BUSY, true, cx);
            return;
        }
        let answer = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Upload".into()) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| match answer.await {
            Ok(Ok(Some(paths))) => {
                this.update(cx, |this, cx| this.upload_paths(dir, paths, cx)).ok();
            }
            Ok(Ok(None)) => {}
            // No file dialog on this system: type the path.
            _ => {
                window_handle
                    .update(cx, |_, window, cx| {
                        let this = this.clone();
                        crate::ui::file_menus::prompt_name("Upload".into(), String::new(), "Upload", move |text, _, cx| {
                            let paths: Vec<PathBuf> = text.split(';').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect();
                            this.update(cx, |this, cx| this.upload_paths(dir.clone(), paths, cx)).ok();
                        }, window, cx);
                    })
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn upload_paths(&mut self, dir: String, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let Some(fs) = files.fs.clone() else { return };
        if paths.is_empty() {
            return;
        }
        let run_as = files.run_as(&dir);
        let names = paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
        if !self.start_busy(format!("Uploading {names}"), cx) {
            return;
        }
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let (totals, problems) = cx
                .background_spawn({
                    let dir = dir.clone();
                    async move {
                        let mut totals = Totals::default();
                        let mut problems = Vec::new();
                        for path in paths {
                            if !path.exists() {
                                problems.push(format!("{}: No such file or directory", path.display()));
                                continue;
                            }
                            match fs.upload(&path, &dir, run_as.as_deref()) {
                                Ok(()) => totals.add(Totals::of(&path)),
                                Err(e) => problems.push(format!("Error uploading {}: {e}", path.display())),
                            }
                        }
                        (totals, problems)
                    }
                })
                .await;
            let (ok, message) = files::transfer_summary(totals, started.elapsed().as_millis(), &problems, false);
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                this.notify("Device Explorer", message, !ok, cx);
                if let Some(files) = this.files.as_mut() {
                    files.expanded.insert(dir.clone());
                }
                this.load_dir(&dir, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn delete(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let entries: Vec<(Entry, bool)> = paths.iter().filter_map(|p| files.entries.get(p).map(|e| (e.clone(), files.is_dir(p)))).collect();
        if entries.is_empty() {
            return;
        }
        let dirs = entries.iter().filter(|(_, d)| *d).count();
        let message = match entries.as_slice() {
            [(entry, true)] => format!("Are you sure you want to delete the selected folder '{}'?", entry.name),
            [(entry, false)] => format!("Are you sure you want to delete the selected file '{}'?", entry.name),
            many if dirs > 0 && dirs < many.len() => format!("Are you sure you want to delete {} selected files and directories?", many.len()),
            many if dirs == many.len() => format!("Are you sure you want to delete {} selected folders?", many.len()),
            many => format!("Are you sure you want to delete {} selected files?", many.len()),
        };
        let this = cx.entity();
        let window_handle = window.window_handle();
        crate::ui::dialogs::confirm("Delete", message, "Delete", move |cx| {
            let entries: Vec<Entry> = entries.iter().map(|(e, _)| e.clone()).collect();
            this.update(cx, |this, cx| this.delete_entries(entries, window_handle, cx));
        }, window, cx);
    }

    pub(super) fn delete_entries(&mut self, entries: Vec<Entry>, window: gpui_kit::AnyWindowHandle, cx: &mut Context<Self>) {
        let Some(fs) = self.files.as_ref().and_then(|f| f.fs.clone()) else { return };
        if !self.start_busy(format!("Deleting {} entries", entries.len()), cx) {
            return;
        }
        let parents: HashSet<String> = entries.iter().map(|e| files::parent(&e.path).to_owned()).collect();
        cx.spawn(async move |this, cx| {
            let problems: Vec<String> = cx
                .background_spawn(async move { entries.iter().filter_map(|e| fs.delete(e).err().map(|err| format!("{}: {err}", e.path))).collect() })
                .await;
            this.update(cx, |this, cx| {
                this.end_busy(cx);
                for parent in &parents {
                    this.load_dir(parent, cx);
                }
                if !problems.is_empty() {
                    let message = if let [one] = problems.as_slice() {
                        format!("Could not erase file or folder:\n{one}")
                    } else {
                        let shown: Vec<&str> = problems.iter().take(10).map(String::as_str).collect();
                        format!("Could not erase files or folders:\n  {}{}", shown.join(",\n  "), if problems.len() > 10 { "\n  ..." } else { "" })
                    };
                    window.update(cx, |_, window, cx| message_dialog("Error", message, window, cx)).ok();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Files dropped from the computer onto a folder (or the tree: the root).
    pub(super) fn drop_paths(&mut self, target: Option<String>, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let Some(files) = self.files.as_ref() else { return };
        let dir = match target {
            Some(path) if files.is_dir(&path) => path,
            Some(path) => files::parent(&path).to_owned(),
            None => files.root.clone(),
        };
        self.upload_paths(dir, paths.paths().to_vec(), cx);
    }

    pub(super) fn file_menu(this: Entity<Self>, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let state = this.read(cx);
        let Some(files) = state.files.as_ref() else { return menu };
        let selected = files.selected.clone();
        if selected.is_empty() {
            return menu;
        }
        let multi = selected.len() > 1;
        let dir = files.selected_dir();
        let any_file = selected.iter().any(|p| !files.is_dir(p));
        let act = |f: Rc<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>| {
            let this = this.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| this.update(cx, |this, cx| f(this, window, cx))
        };
        let mut menu = menu;
        // New ›: on one folder; greyed out with several entries, hidden on a file.
        if dir.is_some() || multi {
            let new_dir = dir.clone().unwrap_or_default();
            let (file_this, dir_this) = (this.clone(), this.clone());
            let (file_parent, dir_parent) = (new_dir.clone(), new_dir.clone());
            let sub = PopupMenu::build(window, cx, move |sub, _, _| {
                let (file_this, dir_this, file_parent, dir_parent) = (file_this.clone(), dir_this.clone(), file_parent.clone(), dir_parent.clone());
                sub.item(PopupMenuItem::new("File").icon(Icon::new(icons::FILE_TEXT)).on_click(move |_, window, cx| {
                    file_this.update(cx, |this, cx| this.new_entry(file_parent.clone(), false, "NewTextFile.txt".into(), window, cx))
                }))
                .item(PopupMenuItem::new("Directory").icon(Icon::new(icons::FOLDER)).on_click(move |_, window, cx| {
                    dir_this.update(cx, |this, cx| this.new_entry(dir_parent.clone(), true, "NewFolder".into(), window, cx))
                }))
            });
            menu = menu.item(PopupMenuItem::submenu("New", sub).disabled(dir.is_none())).separator();
        }
        if any_file {
            let paths = selected.clone();
            menu = menu.item(menu_entry("Open", "").icon(Icon::new(icons::MENU_OPEN)).on_click(act(Rc::new(move |this, _, cx| this.open(paths.clone(), cx)))));
        }
        let save = selected.clone();
        menu = menu.item(
            menu_entry(if multi { "Save To..." } else { "Save As..." }, "Ctrl+Shift+S")
                .icon(Icon::new(icons::DOWNLOAD))
                .on_click(act(Rc::new(move |this, _, cx| this.save_as(save.clone(), cx)))),
        );
        if let Some(dir) = dir.clone() {
            menu = menu.item(menu_entry("Upload...", "Ctrl+Shift+O").icon(Icon::new(icons::UPLOAD)).on_click(act(Rc::new(move |this, window, cx| this.upload(dir.clone(), window, cx)))));
        }
        let delete = selected.clone();
        menu = menu
            .item(menu_entry("Delete...", "Delete").icon(Icon::new(icons::DELETE)).on_click(act(Rc::new(move |this, window, cx| this.delete(delete.clone(), window, cx)))))
            .separator()
            .item(menu_entry("Synchronize", "Ctrl+F5").icon(Icon::new(icons::STUDIO_REFRESH)).on_click(act(Rc::new(|this, _, cx| this.synchronize(cx)))));
        let copy = selected.clone();
        menu.item(
            menu_entry(if multi { "Copy Paths" } else { "Copy Path" }, "Ctrl+Shift+C")
                .icon(Icon::new(icons::COPY))
                .on_click(act(Rc::new(move |this, _, cx| this.copy_paths(&copy, cx)))),
        )
    }

    pub(super) fn render_files_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let files = self.files.as_ref();
        let ready = files.is_some_and(|f| f.fs.is_some());
        let selected = files.map(|f| f.selected.clone()).unwrap_or_default();
        let dir = files.and_then(Files::selected_dir);
        let (new_dir, upload_dir) = (dir.clone(), dir.clone());
        let (save, delete) = (selected.clone(), selected.clone());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(
                tool_button("de-new-dir", icons::NEW_FOLDER, "New Directory")
                    .disabled(dir.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(dir) = new_dir.clone() {
                            this.new_entry(dir, true, "NewFolder".into(), window, cx);
                        }
                    })),
            )
            .child(
                tool_button("de-save", icons::DOWNLOAD, "Save As... (Ctrl+Shift+S)")
                    .disabled(selected.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| this.save_as(save.clone(), cx))),
            )
            .child(
                tool_button("de-upload", icons::UPLOAD, "Upload... (Ctrl+Shift+O)")
                    .disabled(dir.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(dir) = upload_dir.clone() {
                            this.upload(dir, window, cx);
                        }
                    })),
            )
            .child(
                tool_button("de-delete", icons::DELETE, "Delete... (Delete)")
                    .disabled(selected.is_empty())
                    .on_click(cx.listener(move |this, _, window, cx| this.delete(delete.clone(), window, cx))),
            )
            .child(tool_button("de-sync", icons::STUDIO_REFRESH, "Synchronize (Ctrl+F5)").disabled(!ready).on_click(cx.listener(|this, _, _, cx| this.synchronize(cx))))
            .child(self.filter_button("de-files-filter", cx))
    }

    pub(super) fn render_file_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let Some(files) = self.files.as_ref() else { return Vec::new() };
        let grey = palette.text_secondary;
        range
            .filter_map(|ix| {
                let row = files.rows.get(ix)?;
                let indent = |depth: usize| px(4. + depth as f32 * 16.);
                Some(match row {
                    Row::Loading { depth } => h_flex()
                        .h(px(ROW_H))
                        .pl(indent(*depth))
                        .gap_1()
                        .text_sm()
                        .text_color(grey)
                        .child(div().w(px(16.)))
                        .child(div().italic().child("loading..."))
                        .into_any_element(),
                    Row::Error { depth, text } => h_flex()
                        .h(px(ROW_H))
                        .pl(indent(*depth))
                        .gap_1()
                        .text_sm()
                        .text_color(palette.status_deleted)
                        .child(div().w(px(16.)))
                        .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text.clone()))
                        .into_any_element(),
                    Row::Entry { path, depth } => {
                        let entry = files.entries.get(path)?;
                        let is_dir = files.is_dir(path);
                        let expanded = files.expanded.contains(path);
                        let selected = files.selected.contains(path);
                        let base: Icon = if is_dir {
                            icons::FOLDER.into()
                        } else if matches!(entry.kind, EntryKind::File | EntryKind::Symlink) {
                            crate::ui::common::known_file_icon(&entry.name).unwrap_or_else(|| icons::FILE_ANY.into())
                        } else {
                            icons::FILE_ANY.into()
                        };
                        let icon = div()
                            .relative()
                            .size(px(16.))
                            .flex_shrink_0()
                            .child(base.small())
                            .when(entry.kind == EntryKind::Symlink, |el| el.child(div().absolute().top_0().left_0().child(Icon::new(icons::SYMLINK).small())));
                        let chevron = div().w(px(16.)).flex_shrink_0().when(is_dir, |el| {
                            el.child(Icon::new(if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall().text_color(grey))
                        });
                        let (click_path, chevron_path, menu_path, right_path, drop_path) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
                        let menu_entity = entity.clone();
                        let tooltip = entry.link_target.clone().map(|t| SharedString::from(format!("Link target: {t}")));
                        let cell = |text: String| div().px_1().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
                        h_flex()
                            .id(SharedString::from(format!("de-row-{path}")))
                            .h(px(ROW_H))
                            .w_full()
                            .text_sm()
                            .when(selected, |el| el.bg(palette.selection))
                            .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| this.click_row(&click_path, e, window, cx)))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus, cx);
                                    if !this.files.as_ref().is_some_and(|f| f.selected.contains(&right_path)) {
                                        this.select_file(&right_path, Modifiers::default(), cx);
                                    }
                                }),
                            )
                            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                                cx.stop_propagation();
                                this.drop_paths(Some(drop_path.clone()), paths, cx)
                            }))
                            .context_menu(move |menu, window, cx| {
                                let _ = &menu_path;
                                Self::file_menu(menu_entity.clone(), menu, window, cx)
                            })
                            .child(
                                h_flex()
                                    .id(SharedString::from(format!("de-name-{path}")))
                                    .flex_1()
                                    .min_w(px(NAME_MIN))
                                    .pl(indent(*depth))
                                    .gap_1()
                                    .child(div().id(SharedString::from(format!("de-chevron-{path}"))).child(chevron).on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_expanded(&chevron_path, cx);
                                        },
                                    )))
                                    .child(icon)
                                    .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(entry.name.clone()))
                                    .when_some(tooltip, |el, t| el.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(t.clone()).build(window, cx))),
                            )
                            .child(cell(entry.permissions.clone()).w(px(PERM_W)).flex_shrink_0())
                            .child(cell(entry.date.clone()).w(px(DATE_W)).flex_shrink_0())
                            .child(
                                cell(entry.size.map(files::humanized_size).unwrap_or_default())
                                    .w(px(SIZE_W))
                                    .flex_shrink_0()
                                    .flex()
                                    .justify_end(),
                            )
                            .into_any_element()
                    }
                })
            })
            .collect()
    }

    pub(super) fn render_files(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let Some(files) = self.files.as_ref() else { return div().flex_1().into_any_element() };
        let header = h_flex()
            .h(px(24.))
            .flex_shrink_0()
            .text_sm()
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().flex_1().min_w(px(NAME_MIN)).px_1().child("Name"))
            .child(div().w(px(PERM_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Permissions"))
            .child(div().w(px(DATE_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Date"))
            .child(div().w(px(SIZE_W)).flex_shrink_0().px_1().border_l_1().border_color(palette.border).child("Size"));
        let body: gpui_kit::AnyElement = if let Some(error) = &files.error {
            message_layer(error.clone(), palette.status_deleted).into_any_element()
        } else if files.fs.is_none() {
            message_layer("Loading...".into(), palette.text_secondary).into_any_element()
        } else {
            uniform_list("de-files", files.rows.len(), cx.processor(Self::render_file_rows)).track_scroll(&self.files_scroll).flex_1().min_h_0().into_any_element()
        };
        v_flex()
            .id("de-files-tab")
            .flex_1()
            .min_h_0()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.drop_paths(None, paths, cx)))
            .child(self.render_files_toolbar(cx))
            .child(table("de-files-scroll", NAME_MIN + PERM_W + DATE_W + SIZE_W, header, body))
            .into_any_element()
    }
}

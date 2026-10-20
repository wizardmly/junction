//! Reacting to the repository model: reloads, notifications and their
//! follow-up links, conflicts.

use super::*;

impl Workspace {
    /// Everything the workspace does when the repository model reports an
    /// event. Takes `this` like the subscription it came from.
    pub(super) fn on_repo_event(this: &mut Self, event: &RepoEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let RepoEvent::Reloaded = event {
            // Open files follow what git did to the working tree.
            for tab in &this.editors {
                tab.view.update(cx, |editor, cx| editor.sync_with_disk(window, cx));
            }
            // Text reloaded from disk has other lines: counted again from here.
            for tab in &this.editors {
                let editor = tab.view.read(cx);
                if editor.revision().is_none() {
                    this.line_counts.insert(editor.path().to_owned(), editor.line_count(cx) as i64);
                }
            }
            // Index the opened project; re-index what changed on disk.
            let root = this.model.read(cx).project_root().map(|p| p.to_path_buf());
            this.code_index.update(cx, |index, cx| {
                if index.root() != root.as_deref() {
                    index.set_root(root.clone(), cx);
                } else {
                    index.refresh(cx);
                }
            });
            // The project's tabs as they were when it was last open.
            if let Some(root) = root {
                this.restore_session(root, window, cx);
            }
        }
        if let RepoEvent::OpenLogTab { title, filter } = event {
            this.open_log_tab(title.clone(), filter.clone(), cx);
        }
        if let RepoEvent::Cloned(dir) = event {
            clone_dialog::open_cloned(this.model.clone(), dir.clone(), window, cx);
        }
        if let RepoEvent::ShowConflicts = event {
            // Not over the merge tool or a dialog the user has open.
            if this.merge.is_none() && !window.has_active_dialog(cx) {
                this.show_conflicts(window, cx);
            }
        }
        if let RepoEvent::PrefillCommitMessage(_) = event {
            this.show_left_tab(LeftTab::Commit, window, cx);
        }
        if let RepoEvent::Compare { old, new } = event {
            this.changes.update(cx, |changes, cx| changes.compare(old.clone(), new.clone(), cx));
            this.tools.open(ToolWindow::Changes);
            cx.notify();
        }
        if let RepoEvent::UntrustedRoots(roots) = event {
            this.offer_trust_roots(roots.clone(), window, cx);
        }
        if let RepoEvent::InvalidRoots(roots) = event {
            this.report_invalid_roots(roots.clone(), window, cx);
        }
        if let RepoEvent::UnregisteredRoots(roots) = event {
            this.offer_unregistered_roots(roots.clone(), window, cx);
        }
        if let RepoEvent::Notify { title, message, error } = event {
            // Quiet operations (Stage / Unstage) report only failures.
            if message.is_empty() && !*error {
                return;
            }
            // Update Project: "N files updated in M commits" with View Commits.
            let (warning, message) = match message.strip_prefix(dialogs::PARTIAL_FAILURE) {
                Some(rest) => (true, rest.to_owned()),
                None => (false, message.clone()),
            };
            // "text\u{1f}files-range commits-range", one such line per root
            // after a multi-root update: the ranges of all of them.
            let (message, updated_range) = if message.contains('\u{1f}') {
                let mut text = Vec::new();
                let mut ranges = Vec::new();
                for line in message.lines() {
                    match line.split_once('\u{1f}') {
                        Some((t, r)) => {
                            text.push(t.to_owned());
                            ranges.push(r.to_owned());
                        }
                        None => text.push(line.to_owned()),
                    }
                }
                (text.join("\n"), Some(ranges.join(" ")))
            } else {
                (message.clone(), None)
            };
            let message = &message;
            let title = &if warning { format!("{title} finished with errors") } else { title.clone() };
            let entity = cx.entity();
            let mut notification = if *error {
                // A short summary; the full output is in the Console tab.
                let detail = message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest);
                Notification::error(error_summary(detail)).title(title.clone())
            } else if warning {
                Notification::warning(message.clone()).title(title.clone())
            } else {
                Notification::success(message.clone()).title(title.clone())
            };
            // A rejected push asks how to update, in IntelliJ's
            // Push Rejected dialog rather than a balloon.
            if message.split_once(" failed: ").map_or(message.as_str(), |(_, rest)| rest).starts_with(dialogs::PUSH_REJECTED)
                || message.starts_with(dialogs::PUSH_REJECTED)
            {
                dialogs::push_rejected(this.model.clone(), window, cx);
                return;
            }
            // A multi-root push: "libs/core: Push rejected: …" among the
            // other roots' results, which still show.
            if message.lines().any(|line| line.contains(dialogs::PUSH_REJECTED)) {
                dialogs::push_rejected(this.model.clone(), window, cx);
            }
            // IntelliJ's balloon actions: the obvious next step.
            if *error {
                notification = notification.action(move |_, _, _| {
                    let entity = entity.clone();
                    Button::new("notify-details").label("Show Details").small().outline().on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.tools.open(ToolWindow::Git);
                            this.bottom_tab = BottomTab::Console;
                            cx.notify();
                        })
                    })
                });
            } else if let Some(range) = updated_range {
                // "View Files": the Updated Files tree for the pulled range.
                let files_entity = entity.clone();
                let files_range = range.clone();
                notification = notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (entity, range) = (files_entity.clone(), files_range.clone());
                    div()
                        .id("notify-view-files")
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child("View Files")
                        .on_click(move |_, _, cx| {
                            // Every root's updated files (the even tokens).
                            for files in range.split(' ').step_by(2) {
                                let Some((old, new)) = files.split_once("..") else { continue };
                                let (old, new) = (old.to_owned(), new.to_owned());
                                entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.compare(old, Some(new), cx)));
                            }
                        })
                        .into_any_element()
                });
                notification = notification.action(move |_, _, _| {
                    let entity = entity.clone();
                    let range = range.clone();
                    Button::new("notify-view-commits").label("View Commits").small().outline().on_click(move |_, _, cx| {
                        // Every root's incoming commits (the odd tokens); each root keeps its own.
                        let commits: Vec<String> = range.split(' ').skip(1).step_by(2).map(str::to_owned).collect();
                        entity.update(cx, |this, cx| {
                            let filter = crate::git::LogFilter { branches: commits, ..Default::default() };
                            this.open_log_tab("Update Info".into(), filter, cx);
                        })
                    })
                })
                .autohide(true);
            } else if let Some(deleted) = (title == "Delete Branch").then(|| {
                // "Deleted branch X (was T)", or one such line per root ("lib: Deleted …").
                let model = this.model.read(cx);
                let roots: Vec<(std::path::PathBuf, String)> = model.roots().iter().map(|r| (r.path.clone(), model.root_label(&r.path))).collect();
                let parsed: Vec<(crate::git::Repository, String, String)> = message
                    .lines()
                    .filter_map(|line| {
                        let (repo, rest) = match line.split_once(": Deleted branch ") {
                            Some((label, rest)) => (model.repository_at(&roots.iter().find(|(_, l)| l == label)?.0)?, rest),
                            None => (model.repository()?.clone(), line.strip_prefix("Deleted branch ")?),
                        };
                        let (name, tip) = rest.strip_suffix(')')?.split_once(" (was ")?;
                        (!tip.is_empty()).then(|| (repo, name.to_owned(), tip.to_owned()))
                    })
                    .collect();
                parsed
            }).filter(|d| !d.is_empty())
            {
                // IntelliJ's "Restore" link brings the deleted branch back (in every root).
                notification = notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (note, entity, deleted) = (cx.entity(), entity.clone(), deleted.clone());
                    div()
                        .id("notify-restore")
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child("Restore")
                        .on_click(move |_, window, cx| {
                            note.update(cx, |n, cx| n.dismiss(window, cx));
                            let model = entity.read(cx).model.clone();
                            // One operation (and balloon) for every root.
                            let groups: Vec<_> = deleted.iter().map(|(repo, name, tip)| (repo.clone(), (name.clone(), tip.clone()))).collect();
                            model.update(cx, |m, cx| {
                                m.run_in_groups_keep_going("Restore Branch", groups, move |repo, (name, tip)| {
                                    crate::git::refs::restore_branch(repo, &name, &tip)?;
                                    Ok(format!("Restored branch {name}"))
                                }, cx)
                            });
                        })
                        .into_any_element()
                });
            } else if title == crate::ui::rebase_dialog::DROP_TITLE && message.starts_with(crate::ui::rebase_dialog::DROPPED) {
                // "Undo" puts the dropped commits back while the branch
                // is still where the drop left it (rebase saved the old
                // tip in ORIG_HEAD).
                let heads = this.model.read(cx).repository().and_then(|r| {
                    let head = |name: &str| r.run(["rev-parse", "--verify", "-q", name]).ok().map(|h| h.trim().to_owned());
                    Some((head("ORIG_HEAD")?, head("HEAD")?))
                });
                notification = notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (entity, heads) = (entity.clone(), heads.clone());
                    div()
                        .id("notify-undo-drop")
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child("Undo")
                        .on_click(move |_, _, cx| {
                            let Some((old, new)) = heads.clone() else { return };
                            let model = entity.read(cx).model.clone();
                            model.update(cx, |m, cx| {
                                m.run_operation("Undo Drop", move |repo| {
                                    crate::git::rebase::undo_drop(repo, &old, &new)?;
                                    Ok("The dropped commits are back".to_owned())
                                }, cx)
                            });
                        })
                        .into_any_element()
                });
            } else if warning && title.starts_with("Checkout") && this.model.read(cx).can_rollback_checkout() {
                // A synchronous checkout that failed in some roots: IntelliJ's Rollback.
                let rollback = entity.clone();
                notification = notification.autohide(false).content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (note, entity) = (cx.entity(), rollback.clone());
                    div()
                        .id("notify-rollback")
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child("Rollback")
                        .on_click(move |_, window, cx| {
                            note.update(cx, |n, cx| n.dismiss(window, cx));
                            entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.rollback_checkout(cx)));
                        })
                        .into_any_element()
                });
            } else if title == "Commit" {
                // The balloon's "Undo" link undoes exactly this commit (each
                // root's one, after a multi-root commit), while still HEAD.
                let committed: Vec<(std::path::PathBuf, String)> = {
                    let model = this.model.read(cx);
                    let roots: Vec<(std::path::PathBuf, String)> = model.roots().iter().map(|r| (r.path.clone(), model.root_label(&r.path))).collect();
                    message
                        .lines()
                        .filter_map(|line| {
                            let short = line.rsplit(": ").next()?.trim();
                            let (label, _) = line.split_once(": ")?;
                            let repo = if label.ends_with("committed") {
                                model.repository()?.clone()
                            } else {
                                let root = roots.iter().find(|(_, l)| l == label)?.0.clone();
                                model.repository_at(&root)?
                            };
                            let head = repo.run(["rev-parse", "HEAD"]).ok()?.trim().to_owned();
                            (!short.is_empty() && head.starts_with(short)).then(|| (repo.root().to_path_buf(), head))
                        })
                        .collect()
                };
                let committed = (!committed.is_empty() && committed.len() == message.lines().count()).then_some(committed);
                let undo_entity = entity.clone();
                notification = if committed.is_none() { notification } else { notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (entity, hash, note) = (undo_entity.clone(), committed.clone(), cx.entity());
                    v_flex()
                        .child(
                            div()
                                .id("notify-undo")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Undo")
                                .on_click(move |_, window, cx| {
                                    let Some(commits) = hash.clone() else { return };
                                    note.update(cx, |n, cx| n.dismiss(window, cx));
                                    entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.undo_commits(commits, cx)));
                                }),
                        )
                        .into_any_element()
                }) };
                notification = notification.action(move |_, _, _| {
                    let entity = entity.clone();
                    Button::new("notify-view").label("View Commit").small().outline().on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.tools.open(ToolWindow::Git);
                            this.bottom_tab = BottomTab::Log;
                            let head = this.model.read(cx).refs().head_commit.clone();
                            this.model.update(cx, |m, cx| m.select_hash(head, cx));
                            cx.notify();
                        })
                    })
                })
                .autohide(true);
            }
            window.push_notification(notification, cx);
            // Kept in the Notifications tool window, newest first.
            this.notifications.insert(0, crate::ui::status_bar::NotificationRecord {
                title: title.clone(),
                message: message.clone(),
                error: *error,
                warning,
                time: chrono::Local::now(),
            });
            this.notifications.truncate(200);
            if !this.tools.is_open(ToolWindow::Notifications) {
                this.unread_notifications += 1;
            }
        }
    }

    /// Build and Run: the Build window shows a build, the Run window a
    /// launch, errors become balloons.
    pub(super) fn on_run_event(&mut self, event: &crate::ui::run_service::RunEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::run_service::RunEvent;
        match event {
            RunEvent::ShowBuild => self.tools.open(ToolWindow::Build),
            RunEvent::ShowRun => self.tools.open(ToolWindow::Run),
            RunEvent::Error { title, message } => window.push_notification(Notification::error(message.clone()).title(title.clone()), cx),
            RunEvent::Generated { title, message, folder } => {
                let folder = folder.clone();
                let notification = Notification::success(message.clone()).title(title.clone()).action(move |_, _, _| {
                    let folder = folder.clone();
                    gpui_kit::component::button::Button::new("generated-locate").label("Locate").small().outline().on_click(move |_, _, cx| cx.reveal_path(&folder))
                });
                window.push_notification(notification, cx);
            }
            RunEvent::Attach { name, serial, pid, adb } => {
                let (name, serial, adb) = (name.clone(), serial.clone(), adb.clone());
                let pid = *pid;
                crate::ui::debug_service::DebugService::global(cx).update(cx, |d, cx| d.attach_android(name, adb, serial, pid, cx));
            }
        }
        cx.notify();
    }

    /// Debug: a session shows the Debug window, a pause the execution
    /// point in the editor, errors become balloons.
    pub(super) fn on_debug_event(&mut self, event: &crate::ui::debug_service::DebugServiceEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::debug_service::DebugServiceEvent;
        match event {
            DebugServiceEvent::Show => self.tools.open(ToolWindow::Debug),
            DebugServiceEvent::Navigate { path, line } => {
                let root = self.model.read(cx).project_root().map(std::path::Path::to_path_buf);
                let path = match root.as_deref().and_then(|r| path.strip_prefix(r).ok()) {
                    Some(rel) => rel.to_string_lossy().replace('\\', "/"),
                    None => path.to_string_lossy().into_owned(),
                };
                self.open_at(path, *line, 0, window, cx);
            }
            DebugServiceEvent::Error { title, message } => window.push_notification(Notification::error(message.clone()).title(title.clone()), cx),
        }
        cx.notify();
    }

    /// Tests: a test run shows the Run window, a failure to start one
    /// becomes a balloon.
    pub(super) fn on_test_event(&mut self, event: &crate::ui::test_service::TestServiceEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::test_service::TestServiceEvent;
        match event {
            TestServiceEvent::Show => self.tools.open(ToolWindow::Run),
            TestServiceEvent::Error { title, message } => window.push_notification(Notification::error(message.clone()).title(title.clone()), cx),
        }
        cx.notify();
    }

    /// Logcat: a stack frame opens its source (the project's first, then a
    /// library's), Hide hides the window.
    /// Problems and TODO: an item opens its file at its place.
    pub(super) fn on_problems_event(&mut self, event: &crate::ui::problems_view::ProblemsEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::problems_view::ProblemsEvent;
        match event {
            ProblemsEvent::OpenFile { path, line, col } => self.open_absolute(path, *line, *col, window, cx),
            ProblemsEvent::Hide => {
                self.tools.hide(ToolWindow::Problems);
                cx.notify();
            }
        }
    }

    pub(super) fn on_todo_event(&mut self, event: &crate::ui::todo_view::TodoEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::todo_view::TodoEvent;
        match event {
            TodoEvent::OpenFile { path, line, col } => self.open_absolute(path, *line, *col, window, cx),
            TodoEvent::Hide => {
                self.tools.hide(ToolWindow::Todo);
                cx.notify();
            }
        }
    }

    /// Opens a file given on disk at a 0-based line and column.
    fn open_absolute(&mut self, path: &std::path::Path, line: u32, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.model.read(cx).project_root().map(std::path::Path::to_path_buf);
        let path = match root.as_deref().and_then(|r| path.strip_prefix(r).ok()) {
            Some(rel) => rel.to_string_lossy().replace('\\', "/"),
            None => path.to_string_lossy().into_owned(),
        };
        self.open_at(path, line, col, window, cx);
    }

    pub(super) fn on_logcat_event(&mut self, event: &crate::ui::logcat_view::LogcatEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::logcat_view::LogcatEvent;
        match event {
            LogcatEvent::OpenFrame { package, file, line } => {
                let suffix = if package.is_empty() { file.clone() } else { format!("{package}/{file}") };
                let task = self.code_index.read(cx).search_files(file.clone(), true, cx);
                let line = line.saturating_sub(1);
                cx.spawn_in(window, async move |this, cx| {
                    let found = task.await;
                    let path = found.iter().map(|(p, _)| p).find(|p| p.replace('\\', "/").ends_with(&format!("/{suffix}")) || **p == suffix).cloned();
                    if let Some(path) = path {
                        this.update_in(cx, |this, window, cx| this.open_at(path, line, 0, window, cx)).ok();
                    }
                })
                .detach();
            }
            LogcatEvent::Hide => {
                self.tools.hide(ToolWindow::Logcat);
                cx.notify();
            }
        }
    }

    pub(super) fn on_run_view_event(&mut self, tool: ToolWindow, event: &crate::ui::run_views::RunViewEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::run_views::RunViewEvent;
        match event {
            RunViewEvent::OpenFile { path, line, col } => {
                let root = self.model.read(cx).project_root().map(std::path::Path::to_path_buf);
                let path = match root.as_deref().and_then(|r| path.strip_prefix(r).ok()) {
                    Some(rel) => rel.to_string_lossy().replace('\\', "/"),
                    None => path.to_string_lossy().into_owned(),
                };
                self.open_at(path, *line, *col, window, cx);
            }
            RunViewEvent::Hide => {
                self.tools.hide(tool);
                cx.notify();
            }
        }
    }

    /// Sync Project with Gradle Files: Android Studio's "Gradle files have
    /// changed" offer, its failure and unresolved dependencies, and the
    /// libraries re-indexed from a finished sync.
    pub(super) fn on_sync_event(&mut self, event: &crate::ui::sync_service::SyncEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::sync_service::{SyncEvent, SyncService};
        match event {
            SyncEvent::Offer { root } => {
                let root = root.clone();
                let notification = Notification::warning("Gradle files have changed since last project sync. A project sync may be necessary for the IDE to work properly.")
                    .title("Gradle Sync")
                    .autohide(false)
                    .content(move |_, _, cx| {
                        let palette = cx.palette().clone();
                        let note = cx.entity();
                        let (sync_note, ignore_note, root) = (note.clone(), note, root.clone());
                        let link = |id: &'static str, label: &'static str| div().id(id).text_sm().text_color(palette.link).cursor_pointer().child(label);
                        h_flex()
                            .gap_4()
                            .child(link("sync-now", "Sync Now").on_click(move |_, window, cx| {
                                let root = root.clone();
                                SyncService::global(cx).update(cx, |s, cx| s.sync(root, cx));
                                sync_note.update(cx, |n, cx| n.dismiss(window, cx));
                            }))
                            .child(link("sync-ignore", "Ignore these changes").on_click(move |_, window, cx| {
                                ignore_note.update(cx, |n, cx| n.dismiss(window, cx));
                            }))
                            .into_any_element()
                    });
                window.push_notification(notification, cx);
            }
            SyncEvent::Status | SyncEvent::Modules | SyncEvent::Started => cx.notify(),
            SyncEvent::Finished { build, seconds, unresolved } => {
                self.code_index.update(cx, |index, cx| index.refresh(cx));
                if !unresolved.is_empty() {
                    let mut message = unresolved.iter().take(8).cloned().collect::<Vec<_>>().join("\n");
                    if unresolved.len() > 8 {
                        message.push_str(&format!("\n… and {} more", unresolved.len() - 8));
                    }
                    window.push_notification(Notification::warning(message).title(format!("Gradle sync of {build} finished in {seconds} s with warnings")), cx);
                }
            }
            SyncEvent::Failed { build, output } => {
                if crate::domain::jdk::is_missing_jdk(output) {
                    self.report_gradle_jdk(output, window, cx);
                    return;
                }
                // Android Studio shows the Build window's Sync tab on a failure.
                self.tools.open(ToolWindow::Build);
                let message = format!("Gradle project sync failed. Basic functionality (e.g. editing, debugging) will not work properly.\n{output}");
                window.push_notification(Notification::error(message).title(format!("Gradle sync of {build} failed")).autohide(false), cx);
            }
        }
    }

    /// Android Studio's "Invalid Gradle JDK configuration found", with
    /// Open Gradle Settings (Build Tools › Gradle › Gradle JDK).
    pub(super) fn report_gradle_jdk(&mut self, output: &str, window: &mut Window, cx: &mut Context<Self>) {
        let reason = output.lines().map(str::trim).find(|l| crate::domain::jdk::is_missing_jdk(l)).unwrap_or("Gradle could not find a JDK to run on.").trim_start_matches("ERROR: ").to_owned();
        let model = self.model.clone();
        let notification = Notification::error(reason)
            .title("Invalid Gradle JDK configuration found")
            .autohide(false)
            .content(move |_, _, cx| {
                let palette = cx.palette().clone();
                let (note, model) = (cx.entity(), model.clone());
                div()
                    .id("gradle-jdk-settings")
                    .text_sm()
                    .text_color(palette.link)
                    .cursor_pointer()
                    .child("Open Gradle Settings")
                    .on_click(move |_, window, cx| {
                        note.update(cx, |n, cx| n.dismiss(window, cx));
                        crate::ui::settings_dialog::open_page(Some(model.clone()), "Gradle", window, cx);
                    })
                    .into_any_element()
            });
        window.push_notification(notification, cx);
    }

    /// IntelliJ's "Unregistered VCS roots detected" balloon: Add Roots,
    /// Configure (Directory Mappings) or Ignore.
    fn offer_unregistered_roots(&mut self, roots: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let Some(project) = model.read(cx).project_root().map(|p| p.to_path_buf()) else { return };
        let names: Vec<String> = roots.iter().map(|r| crate::git::roots::label(&project, r)).collect();
        let message = if roots.len() == 1 {
            format!("The directory {} is under Git, but is not registered in the Settings.", names[0])
        } else {
            format!("The following directories are roots of Git repositories, but they are not registered in the Settings: {}", names.join(", "))
        };
        let notification = Notification::info(message)
            .title(if roots.len() == 1 { "Unregistered VCS root detected" } else { "Unregistered VCS roots detected" })
            .autohide(false)
            .content(move |_, _, cx| {
                let palette = cx.palette().clone();
                let note = cx.entity();
                let link = |id: &'static str, label: &'static str| {
                    div().id(id).text_sm().text_color(palette.link).cursor_pointer().child(label)
                };
                let (add_model, add_roots, add_note) = (model.clone(), roots.clone(), note.clone());
                let (ignore_model, ignore_roots, ignore_note) = (model.clone(), roots.clone(), note.clone());
                let (configure_model, configure_note) = (model.clone(), note.clone());
                h_flex()
                    .gap_4()
                    .child(link("roots-add", if roots.len() == 1 { "Add Root" } else { "Add Roots" }).on_click(move |_, window, cx| {
                        add_model.update(cx, |m, cx| m.register_roots(add_roots.clone(), true, cx));
                        add_note.update(cx, |n, cx| n.dismiss(window, cx));
                    }))
                    .child(link("roots-configure", "Configure").on_click(move |_, window, cx| {
                        configure_note.update(cx, |n, cx| n.dismiss(window, cx));
                        crate::ui::mappings_dialog::directory_mappings(configure_model.clone(), window, cx);
                    }))
                    .child(link("roots-ignore", "Ignore").on_click(move |_, window, cx| {
                        ignore_model.update(cx, |m, cx| m.register_roots(ignore_roots.clone(), false, cx));
                        ignore_note.update(cx, |n, cx| n.dismiss(window, cx));
                    }))
                    .into_any_element()
            });
        window.push_notification(notification, cx);
    }

    /// IntelliJ's "VCS root configuration problems": mapped folders that
    /// aren't repositories, with Configure… (Directory Mappings).
    fn report_invalid_roots(&mut self, roots: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let Some(project) = model.read(cx).project_root().map(|p| p.to_path_buf()) else { return };
        let names: Vec<String> = roots.iter().map(|r| crate::git::roots::label(&project, r)).collect();
        let message = if roots.len() == 1 {
            format!("The directory {} is registered as a Git root, but no Git repositories were found there.", names[0])
        } else {
            format!("The following directories are registered as VCS roots, but they are not: {}", names.join(", "))
        };
        let notification = Notification::error(message).title("VCS root configuration problems").autohide(false).content(move |_, _, cx| {
            let palette = cx.palette().clone();
            let (note, model) = (cx.entity(), model.clone());
            div()
                .id("roots-problems-configure")
                .text_sm()
                .text_color(palette.link)
                .cursor_pointer()
                .child("Configure…")
                .on_click(move |_, window, cx| {
                    note.update(cx, |n, cx| n.dismiss(window, cx));
                    crate::ui::mappings_dialog::directory_mappings(model.clone(), window, cx);
                })
                .into_any_element()
        });
        window.push_notification(notification, cx);
    }

    /// Roots git refuses because another user owns them (common on Windows
    /// for repositories on another drive): Trust adds them to safe.directory.
    fn offer_trust_roots(&mut self, roots: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let project = model.read(cx).project_root().map(|p| p.to_path_buf()).unwrap_or_default();
        let names: Vec<String> = roots.iter().map(|r| crate::git::roots::label(&project, r)).collect();
        let message = format!(
            "Git refuses to work with {} because the folder{} owned by another user. Trusting adds {} to safe.directory in your global Git config.",
            names.join(", "),
            if roots.len() == 1 { " is" } else { "s are" },
            if roots.len() == 1 { "it" } else { "them" },
        );
        let notification = Notification::warning(message).title("Untrusted Git roots").autohide(false).content(move |_, _, cx| {
            let palette = cx.palette().clone();
            let (note, model, roots) = (cx.entity(), model.clone(), roots.clone());
            div()
                .id("roots-trust")
                .text_sm()
                .text_color(palette.link)
                .cursor_pointer()
                .child("Trust")
                .on_click(move |_, window, cx| {
                    note.update(cx, |n, cx| n.dismiss(window, cx));
                    model.update(cx, |m, cx| m.trust_roots(roots.clone(), cx));
                })
                .into_any_element()
        });
        window.push_notification(notification, cx);
    }
}

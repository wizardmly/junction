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
            // Index the opened project; re-index what changed on disk.
            let root = this.model.read(cx).project_root().map(|p| p.to_path_buf());
            this.code_index.update(cx, |index, cx| {
                if index.root() != root.as_deref() {
                    index.set_root(root, cx);
                } else {
                    index.refresh(cx);
                }
            });
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
            let (message, updated_range) = match message.split_once('\u{1f}') {
                Some((text, range)) => (text.to_owned(), Some(range.to_owned())),
                None => (message.clone(), None),
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
                            let files = range.split(' ').next().unwrap_or_default();
                            let Some((old, new)) = files.split_once("..") else { return };
                            let (old, new) = (old.to_owned(), new.to_owned());
                            entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.compare(old, Some(new), cx)));
                        })
                        .into_any_element()
                });
                notification = notification.action(move |_, _, _| {
                    let entity = entity.clone();
                    let range = range.clone();
                    Button::new("notify-view-commits").label("View Commits").small().outline().on_click(move |_, _, cx| {
                        let commits = range.split(' ').last().unwrap_or_default().to_owned();
                        entity.update(cx, |this, cx| {
                            let filter = crate::git::LogFilter { branches: vec![commits], ..Default::default() };
                            this.open_log_tab("Update Info".into(), filter, cx);
                        })
                    })
                })
                .autohide(true);
            } else if let Some((name, tip)) = (title == "Delete Branch")
                .then(|| message.strip_prefix("Deleted branch ")?.strip_suffix(')')?.split_once(" (was "))
                .flatten()
            {
                // IntelliJ's "Restore" link brings the deleted branch back.
                let (name, tip) = (name.to_owned(), tip.to_owned());
                notification = notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (entity, name, tip) = (entity.clone(), name.clone(), tip.clone());
                    div()
                        .id("notify-restore")
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child("Restore")
                        .on_click(move |_, _, cx| {
                            let (name, tip) = (name.clone(), tip.clone());
                            let model = entity.read(cx).model.clone();
                            model.update(cx, |m, cx| {
                                m.run_operation("Restore Branch", move |repo| {
                                    repo.run(["branch", &name, &tip])?;
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
            } else if title == "Commit" {
                // The balloon's "Undo" link undoes exactly this commit.
                // (Only for a commit to the active repository: a multi-root
                // commit, one per root, has nothing single to undo.)
                let short = message.rsplit(": ").next().unwrap_or_default().trim().to_owned();
                let committed = this
                    .model
                    .read(cx)
                    .repository()
                    .and_then(|r| r.run(["rev-parse", "HEAD"]).ok())
                    .map(|h| h.trim().to_owned())
                    .filter(|h| !message.contains('\n') && !short.is_empty() && h.starts_with(&short));
                let undo_entity = entity.clone();
                notification = if committed.is_none() { notification } else { notification.content(move |_, _, cx| {
                    let palette = cx.palette().clone();
                    let (entity, hash) = (undo_entity.clone(), committed.clone());
                    v_flex()
                        .child(
                            div()
                                .id("notify-undo")
                                .text_sm()
                                .text_color(palette.link)
                                .cursor_pointer()
                                .child("Undo")
                                .on_click(move |_, _, cx| {
                                    let Some(hash) = hash.clone() else { return };
                                    entity.update(cx, |this, cx| this.model.update(cx, |m, cx| m.undo_commit(hash, cx)));
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
}

//! The editor session kept across restarts, as IntelliJ reopens a project:
//! its tabs with their carets, the active one, and Recent Locations. Saved
//! a moment after tabs or text change, and when the app quits or the
//! window closes; restored when the project opens.

use super::*;
use crate::domain::editor_session::{EditorSession, SessionTab};

impl Workspace {
    fn session(&self, cx: &gpui_kit::App) -> EditorSession {
        let tabs = self
            .editors
            .iter()
            .filter(|t| t.view.read(cx).revision().is_none())
            .map(|t| {
                let e = t.view.read(cx);
                let (line, col) = e.cursor(cx);
                SessionTab { path: e.path().to_owned(), line, col, pinned: t.pinned }
            })
            .collect();
        let active = self.editor().map(|e| e.read(cx)).filter(|e| e.revision().is_none()).map(|e| e.path().to_owned());
        EditorSession { tabs, active, places: self.recent_places.to_vec() }
    }

    /// Saves the session of the project whose session is showing.
    pub(super) fn save_session(&mut self, cx: &mut Context<Self>) {
        self.session_save = None;
        let root = self.model.read(cx).project_root().map(|r| r.to_path_buf());
        // Not before it was restored: that would save it empty.
        let (Some(root), Some(shown)) = (root, self.session_root.as_ref()) else { return };
        if root != *shown {
            return;
        }
        self.sessions.save(&root, &self.session(cx));
    }

    /// Saves the session in a moment (once for a burst of changes).
    pub(super) fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
        if self.session_save.is_some() || self.session_root.is_none() {
            return;
        }
        self.session_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            this.update(cx, |this, cx| this.save_session(cx)).ok();
        }));
    }

    /// Closes the editor tabs and forgets the session's state (its places,
    /// the edit walk, the closed tabs), before another session or none.
    fn close_session(&mut self, cx: &mut Context<Self>) {
        let all: Vec<usize> = (0..self.editors.len()).collect();
        self.close_tabs(all, cx);
        self.closed_tabs.clear();
        self.edit_walk = None;
        self.session_save = None;
        self.recent_places = history::RecentPlaces::default();
    }

    /// File › Close Project: its session saved and its tabs closed, then
    /// the Welcome screen.
    pub(super) fn close_project(&mut self, cx: &mut Context<Self>) {
        if self.model.read(cx).repository().is_none() {
            return;
        }
        self.save_session(cx);
        self.close_session(cx);
        self.session_root = None;
        self.model.update(cx, |model, cx| model.close_project(cx));
        cx.notify();
    }

    /// Reopens a project's tabs and carets and its Recent Locations, once
    /// per opening.
    pub(super) fn restore_session(&mut self, root: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_root.as_ref() == Some(&root) {
            return;
        }
        // Another project in this window: its tabs give way to this one's,
        // as IntelliJ's project replaces the one before.
        if let Some(old) = self.session_root.clone() {
            self.sessions.save(&old, &self.session(cx));
            self.close_session(cx);
        }
        let session = self.sessions.load(&root);
        self.session_root = Some(root.clone());
        let places = history::RecentPlaces::restore(session.places.clone());
        if self.editors.is_empty() {
            let (tabs, active) = session.reopenable(|path| root.join(path).is_file());
            for tab in &tabs {
                self.open_at(tab.path.clone(), tab.line, tab.col, window, cx);
                if tab.pinned {
                    if let Some(open) = self.editors.iter_mut().find(|t| t.view.read(cx).path() == tab.path) {
                        open.pinned = true;
                    }
                }
            }
            if let Some(active) = active {
                self.open_at(active.path, active.line, active.col, window, cx);
            }
        }
        // Reopening isn't a visit: the places stay as they were.
        self.recent_places = places;
        cx.notify();
    }
}

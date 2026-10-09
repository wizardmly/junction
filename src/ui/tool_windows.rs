//! Which tool windows sit on which side, in which order, and which one is
//! open on each side, as IntelliJ's new UI keeps it: one open window per
//! side, stripe buttons dragged to reorder or to move a window to another
//! side (also Move To in the button's menu), and the layout remembered.

use crate::settings::Settings;

/// The narrowest a side panel opens when Git moves there.
const GIT_SIDE_WIDTH: u32 = 640;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ToolWindow {
    Project,
    Commit,
    PullRequests,
    Changes,
    Git,
    Notifications,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    Bottom,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::Left, Side::Right, Side::Bottom];

    fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
            Side::Bottom => 2,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Bottom => "bottom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Left => "Left",
            Side::Right => "Right",
            Side::Bottom => "Bottom",
        }
    }
}

impl ToolWindow {
    pub const ALL: [ToolWindow; 6] =
        [ToolWindow::Project, ToolWindow::Commit, ToolWindow::PullRequests, ToolWindow::Changes, ToolWindow::Git, ToolWindow::Notifications];

    fn key(self) -> &'static str {
        match self {
            ToolWindow::Project => "project",
            ToolWindow::Commit => "commit",
            ToolWindow::PullRequests => "prs",
            ToolWindow::Changes => "changes",
            ToolWindow::Git => "git",
            ToolWindow::Notifications => "notifications",
        }
    }

    fn default_side(self) -> Side {
        match self {
            ToolWindow::Git => Side::Bottom,
            // The new UI keeps Notifications on the right stripe.
            ToolWindow::Notifications => Side::Right,
            _ => Side::Left,
        }
    }
}

/// A stripe button being dragged, drawn as its icon.
#[derive(Clone, Copy)]
pub struct DraggedToolWindow(pub ToolWindow, pub gpui_kit::assets::IconName);

impl gpui_kit::Render for DraggedToolWindow {
    fn render(&mut self, _: &mut gpui_kit::Window, cx: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        use crate::theme::ActivePalette as _;
        use gpui_kit::{ParentElement as _, Styled as _};
        let palette = cx.palette().clone();
        gpui_kit::div()
            .size(gpui_kit::px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(palette.selection)
            .child(gpui_kit::component::Icon::new(self.1))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolWindows {
    /// Every tool window with its side, in stripe order.
    order: Vec<(ToolWindow, Side)>,
    /// The open window of each side.
    open: [Option<ToolWindow>; 3],
    /// Hide All Tool Windows (Ctrl+Shift+F12) remembers what was open.
    hidden_all: Option<[Option<ToolWindow>; 3]>,
}

impl ToolWindows {
    pub fn load(cx: &gpui_kit::App) -> Self {
        let mut order: Vec<(ToolWindow, Side)> = Settings::get(cx)
            .tool_windows
            .split(',')
            .filter_map(|entry| {
                let (name, side) = entry.trim().split_once(':')?;
                let window = ToolWindow::ALL.into_iter().find(|w| w.key() == name)?;
                let side = Side::ALL.into_iter().find(|s| s.key() == side)?;
                Some((window, side))
            })
            .collect();
        for window in ToolWindow::ALL {
            if !order.iter().any(|(w, _)| *w == window) {
                order.push((window, window.default_side()));
            }
        }
        order.dedup_by_key(|(w, _)| *w);
        let mut this = Self { order, open: [None; 3], hidden_all: None };
        this.open(ToolWindow::Commit);
        this.open(ToolWindow::Git);
        this
    }

    fn save(&self, cx: &mut gpui_kit::App) {
        let text = self.order.iter().map(|(w, s)| format!("{}:{}", w.key(), s.key())).collect::<Vec<_>>().join(",");
        Settings::update(cx, |s| s.tool_windows = text);
    }

    pub fn side(&self, window: ToolWindow) -> Side {
        self.order.iter().find(|(w, _)| *w == window).map_or(window.default_side(), |(_, s)| *s)
    }

    pub fn on_side(&self, side: Side) -> Vec<ToolWindow> {
        self.order.iter().filter(|(_, s)| *s == side).map(|(w, _)| *w).collect()
    }

    pub fn active(&self, side: Side) -> Option<ToolWindow> {
        self.open[side.index()]
    }

    pub fn is_open(&self, window: ToolWindow) -> bool {
        self.active(self.side(window)) == Some(window)
    }

    /// Shows a window, closing the one that shared its side.
    pub fn open(&mut self, window: ToolWindow) {
        self.hidden_all = None;
        self.open[self.side(window).index()] = Some(window);
    }

    pub fn hide(&mut self, window: ToolWindow) {
        let side = self.side(window).index();
        if self.open[side] == Some(window) {
            self.open[side] = None;
        }
    }

    pub fn toggle(&mut self, window: ToolWindow) {
        if self.is_open(window) { self.hide(window) } else { self.open(window) }
    }

    /// Hide All Tool Windows, or bring them back when all are hidden.
    pub fn toggle_all(&mut self) {
        if let Some(open) = self.hidden_all.take() {
            self.open = open;
        } else if self.open.iter().any(Option::is_some) {
            self.hidden_all = Some(self.open);
            self.open = [None; 3];
        }
    }

    /// Moves a window to `side`, before `before` (or last), keeping it open
    /// if it was.
    pub fn move_to(&mut self, window: ToolWindow, side: Side, before: Option<ToolWindow>, cx: &mut gpui_kit::App) {
        if before == Some(window) {
            return;
        }
        let was_open = self.is_open(window);
        self.hide(window);
        self.order.retain(|(w, _)| *w != window);
        let at = before.and_then(|b| self.order.iter().position(|(w, _)| *w == b)).unwrap_or(self.order.len());
        self.order.insert(at, (window, side));
        if was_open {
            self.open(window);
        }
        self.save(cx);
        // The Log needs room beside the editor: a side that was sized for
        // the Commit window gets wider for Git.
        if window == ToolWindow::Git && side != Side::Bottom && Settings::get(cx).tool_window_sizes[side.index()] < GIT_SIDE_WIDTH {
            Settings::update(cx, |s| s.tool_window_sizes[side.index()] = GIT_SIDE_WIDTH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> ToolWindows {
        let order = ToolWindow::ALL.into_iter().map(|w| (w, w.default_side())).collect();
        let mut this = ToolWindows { order, open: [None; 3], hidden_all: None };
        this.open(ToolWindow::Commit);
        this.open(ToolWindow::Git);
        this
    }

    #[test]
    fn one_window_per_side_and_hide_all() {
        let mut t = layout();
        t.open(ToolWindow::Project);
        assert!(t.is_open(ToolWindow::Project) && !t.is_open(ToolWindow::Commit) && t.is_open(ToolWindow::Git));
        t.toggle_all();
        assert!(!t.is_open(ToolWindow::Project) && !t.is_open(ToolWindow::Git));
        t.toggle_all();
        assert!(t.is_open(ToolWindow::Project) && t.is_open(ToolWindow::Git));
    }
}

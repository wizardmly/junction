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
    Find,
    Terminal,
    Git,
    Notifications,
    DeviceManager,
    RunningDevices,
    DeviceExplorer,
    LayoutInspector,
    Profiler,
    AppInspection,
    Build,
    Run,
    Debug,
    Logcat,
    Problems,
    Todo,
    BuildVariants,
    /// Call Hierarchy (Ctrl+Alt+H).
    Hierarchy,
    /// Run with Coverage's results.
    Coverage,
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
    pub const ALL: [ToolWindow; 23] = [
        ToolWindow::Project,
        ToolWindow::Commit,
        ToolWindow::PullRequests,
        ToolWindow::Changes,
        ToolWindow::BuildVariants,
        ToolWindow::Find,
        ToolWindow::Build,
        ToolWindow::Run,
        ToolWindow::Debug,
        ToolWindow::Logcat,
        ToolWindow::Profiler,
        ToolWindow::AppInspection,
        ToolWindow::Problems,
        ToolWindow::Todo,
        ToolWindow::Terminal,
        ToolWindow::Git,
        ToolWindow::Notifications,
        ToolWindow::DeviceManager,
        ToolWindow::RunningDevices,
        ToolWindow::DeviceExplorer,
        ToolWindow::LayoutInspector,
        ToolWindow::Hierarchy,
        ToolWindow::Coverage,
    ];

    fn key(self) -> &'static str {
        match self {
            ToolWindow::Project => "project",
            ToolWindow::Commit => "commit",
            ToolWindow::PullRequests => "prs",
            ToolWindow::Changes => "changes",
            ToolWindow::Find => "find",
            ToolWindow::Terminal => "terminal",
            ToolWindow::Git => "git",
            ToolWindow::Notifications => "notifications",
            ToolWindow::DeviceManager => "devices",
            ToolWindow::RunningDevices => "running",
            ToolWindow::DeviceExplorer => "explorer",
            ToolWindow::LayoutInspector => "layout-inspector",
            ToolWindow::Profiler => "profiler",
            ToolWindow::AppInspection => "app_inspection",
            ToolWindow::Build => "build",
            ToolWindow::Run => "run",
            ToolWindow::Debug => "debug",
            ToolWindow::Logcat => "logcat",
            ToolWindow::Problems => "problems",
            ToolWindow::Todo => "todo",
            ToolWindow::BuildVariants => "variants",
            ToolWindow::Hierarchy => "hierarchy",
            ToolWindow::Coverage => "coverage",
        }
    }

    /// A window showing results that don't outlive the session.
    fn transient(self) -> bool {
        matches!(self, ToolWindow::Find | ToolWindow::Changes | ToolWindow::Run | ToolWindow::Debug | ToolWindow::Hierarchy | ToolWindow::Coverage)
    }

    fn default_side(self) -> Side {
        match self {
            ToolWindow::Git | ToolWindow::Find | ToolWindow::Terminal | ToolWindow::Build | ToolWindow::Run | ToolWindow::Debug | ToolWindow::Logcat | ToolWindow::Profiler | ToolWindow::AppInspection | ToolWindow::Problems | ToolWindow::Todo => Side::Bottom,
            // The new UI keeps Notifications on the right stripe, and
            // Android Studio its Device Manager, Running Devices and Device
            // Explorer below it.
            ToolWindow::Notifications | ToolWindow::DeviceManager | ToolWindow::RunningDevices | ToolWindow::DeviceExplorer | ToolWindow::LayoutInspector | ToolWindow::Hierarchy | ToolWindow::Coverage => Side::Right,
            _ => Side::Left,
        }
    }
}

/// A stripe button being dragged, drawn as its icon.
#[derive(Clone)]
pub struct DraggedToolWindow(pub ToolWindow, pub gpui_kit::component::Icon);

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
            .child(self.1.clone())
    }
}

/// A window the saved layout doesn't know yet (one added in a later
/// version) goes before the first window that follows it by default, so Find
/// and Terminal sit above Git as in IntelliJ.
fn add_missing(order: &mut Vec<(ToolWindow, Side)>) {
    for (ix, window) in ToolWindow::ALL.into_iter().enumerate() {
        if !order.iter().any(|(w, _)| *w == window) {
            let at = ToolWindow::ALL[ix + 1..].iter().find_map(|next| order.iter().position(|(w, _)| w == next)).unwrap_or(order.len());
            order.insert(at, (window, window.default_side()));
        }
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
        add_missing(&mut order);
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

    /// The open windows to reopen with the project (IntelliJ keeps them in
    /// the workspace), by key; windows that show one search's or one run's
    /// results aren't among them, as they'd come back empty.
    pub fn open_keys(&self) -> Vec<String> {
        let open = self.hidden_all.unwrap_or(self.open);
        open.into_iter().flatten().filter(|w| !w.transient()).map(|w| w.key().to_owned()).collect()
    }

    /// Reopens the windows `open_keys` gave; the other sides close.
    pub fn restore_open(&mut self, keys: &[String]) {
        self.open = [None; 3];
        self.hidden_all = None;
        for window in ToolWindow::ALL.into_iter().filter(|w| !w.transient() && keys.iter().any(|k| k == w.key())) {
            self.open(window);
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
        if window == ToolWindow::Git {
            self.make_room(window, GIT_SIDE_WIDTH, cx);
        }
    }

    /// Widens a window's side panel to at least `width` (not the bottom's).
    pub fn make_room(&self, window: ToolWindow, width: u32, cx: &mut gpui_kit::App) {
        let side = self.side(window);
        if side != Side::Bottom && Settings::get(cx).tool_window_sizes[side.index()] < width {
            Settings::update(cx, |s| s.tool_window_sizes[side.index()] = width);
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

    #[test]
    fn find_and_terminal_share_the_bottom_with_git() {
        let mut t = layout();
        assert_eq!(t.on_side(Side::Bottom), vec![ToolWindow::Find, ToolWindow::Build, ToolWindow::Run, ToolWindow::Debug, ToolWindow::Logcat, ToolWindow::Profiler, ToolWindow::AppInspection, ToolWindow::Problems, ToolWindow::Todo, ToolWindow::Terminal, ToolWindow::Git]);
        t.open(ToolWindow::Terminal);
        assert!(t.is_open(ToolWindow::Terminal) && !t.is_open(ToolWindow::Git));
        t.open(ToolWindow::Find);
        assert!(t.is_open(ToolWindow::Find) && !t.is_open(ToolWindow::Terminal));
    }

    #[test]
    fn open_windows_come_back_without_transient_ones() {
        let mut t = layout();
        t.open(ToolWindow::Project);
        t.open(ToolWindow::Find);
        t.open(ToolWindow::Notifications);
        let keys = t.open_keys();
        assert_eq!(keys, ["project", "notifications"]);
        let mut fresh = layout();
        fresh.restore_open(&keys);
        assert!(fresh.is_open(ToolWindow::Project) && fresh.is_open(ToolWindow::Notifications));
        assert_eq!(fresh.active(Side::Bottom), None);
        // Hidden by Hide All Tool Windows: what was open still counts.
        fresh.toggle_all();
        assert_eq!(fresh.open_keys(), keys);
    }

    #[test]
    fn new_windows_join_an_old_layout_before_git() {
        // A layout saved before Find and Terminal existed, Git moved left.
        let mut order = vec![(ToolWindow::Project, Side::Left), (ToolWindow::Git, Side::Left), (ToolWindow::Notifications, Side::Right)];
        add_missing(&mut order);
        let names: Vec<ToolWindow> = order.iter().map(|(w, _)| *w).filter(|w| matches!(w, ToolWindow::Project | ToolWindow::Find | ToolWindow::Terminal | ToolWindow::Git)).collect();
        assert_eq!(names, [ToolWindow::Project, ToolWindow::Find, ToolWindow::Terminal, ToolWindow::Git]);
        assert!(order.iter().any(|(w, s)| *w == ToolWindow::Terminal && *s == Side::Bottom));
    }
}

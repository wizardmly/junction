//! The App Inspection tool window, as Android Studio shows it: a device and
//! app picker over two inspectors. The Database Inspector shows the app's
//! databases with their tables, and the picked table's rows in a grid. The databases are read
//! from the device (`run-as`, so the app must be debuggable) with their
//! WAL applied; Refresh reads them again, and Live updates every few
//! seconds. The query bar runs a SELECT on the picked database. The
//! Background Task Inspector is in `background_task_view`, the Network
//! Inspector in `network_view`.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::background_task_view::BackgroundTaskView;
use crate::ui::common::tool_button;
use crate::ui::network_view::NetworkView;
use crate::ui::device_manager::DeviceManager;
use crate::usecase::inspect_database::{self, OpenedDatabase};

pub enum AppInspectionEvent {
    Hide,
}

const ROW_H: f32 = 22.;
const COLUMN_W: f32 = 160.;
/// Rows read from a table; Android Studio pages through 50 at a time.
const ROW_LIMIT: usize = 5000;
const LIVE_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inspector {
    Database,
    BackgroundTasks,
    Network,
}

#[derive(Clone)]
struct Target {
    serial: String,
    name: String,
}

/// The table on screen: (database, table) names and its rows.
struct Grid {
    database: String,
    table: String,
    /// The query that made the grid, run again on refresh.
    query: Option<String>,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

pub struct AppInspectionView {
    device_manager: Entity<DeviceManager>,
    project: Rc<dyn Fn(&App) -> Option<PathBuf>>,
    devices: Vec<Target>,
    active: Option<String>,
    apps: Vec<String>,
    app_ids: Vec<String>,
    app: Option<String>,
    databases: Vec<OpenedDatabase>,
    collapsed: Vec<String>,
    grid: Option<Grid>,
    loading: bool,
    error: Option<String>,
    live: bool,
    query: Entity<InputState>,
    /// The database queries run on.
    query_database: Option<String>,
    inspector: Inspector,
    background: Entity<BackgroundTaskView>,
    network: Entity<NetworkView>,
    /// The tool window shows; the Network Inspector reads only then.
    shown: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AppInspectionEvent> for AppInspectionView {}

impl AppInspectionView {
    pub fn new(device_manager: Entity<DeviceManager>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&device_manager, |this, dm, cx| {
            this.devices = dm.read(cx).devices().iter().filter(|d| d.online).filter_map(|d| Some(Target { serial: d.serial.clone()?, name: d.name.clone() })).collect();
            if this.active.as_ref().is_none_or(|s| !this.devices.iter().any(|d| &d.serial == s)) {
                this.active = this.devices.first().map(|d| d.serial.clone());
                this.load_apps(cx);
            }
            cx.notify();
        });
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("SELECT * FROM table WHERE … ORDER BY … LIMIT …"));
        let run = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.run_query(cx);
            }
        });
        Self {
            device_manager,
            project,
            devices: Vec::new(),
            active: None,
            apps: Vec::new(),
            app_ids: Vec::new(),
            app: None,
            databases: Vec::new(),
            collapsed: Vec::new(),
            grid: None,
            loading: false,
            error: None,
            live: false,
            query,
            query_database: None,
            inspector: Inspector::Database,
            background: cx.new(|_| BackgroundTaskView::new()),
            network: cx.new(|_| NetworkView::new()),
            shown: false,
            _subscriptions: vec![observe, run],
        }
    }

    fn adb(&self, cx: &App) -> Option<crate::android::adb::Adb> {
        self.device_manager.read(cx).adb()
    }

    /// The device's running apps, the project's first; it is picked when running.
    pub fn load_apps(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb)) = (self.active.clone(), self.adb(cx)) else { return };
        let project = (self.project)(cx);
        let task = cx.background_spawn(async move {
            let ids = project.map(|p| crate::android::application_ids(&p)).unwrap_or_default();
            let source = crate::infra::adb_profiler::AdbProfiler { adb };
            (crate::domain::profiler::ProfilerSource::processes(&source, &serial), ids)
        });
        cx.spawn(async move |this, cx| {
            let (result, ids) = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(processes) => {
                        this.apps = inspect_database::apps(&processes, &ids);
                        this.app_ids = ids;
                        if this.app.as_ref().is_none_or(|a| !this.apps.contains(a)) {
                            this.app = this.apps.iter().find(|a| this.app_ids.contains(a)).cloned();
                            this.app_changed(cx);
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A new app or device: the shown inspector reads it, the other forgets the old one.
    fn app_changed(&mut self, cx: &mut Context<Self>) {
        self.grid = None;
        self.background.update(cx, |b, cx| b.clear(cx));
        self.network.update(cx, |n, cx| n.forget(cx));
        if self.inspector != Inspector::Database {
            self.databases.clear();
        }
        self.refresh(cx);
    }

    /// The tool window showed or hid.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown == self.shown {
            return;
        }
        self.shown = shown;
        if self.inspector == Inspector::Network {
            if shown {
                self.watch_network(cx);
            } else {
                self.network.update(cx, |n, _| n.stop());
            }
        }
    }

    fn watch_network(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(app)) = (self.active.clone(), self.adb(cx), self.app.clone()) else { return };
        if self.shown {
            self.network.update(cx, |n, cx| n.watch(adb, serial, app, cx));
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.inspector {
            Inspector::Database => self.load_databases(cx),
            Inspector::BackgroundTasks => self.load_background(cx),
            Inspector::Network => self.watch_network(cx),
        }
    }

    fn busy(&self, cx: &App) -> bool {
        match self.inspector {
            Inspector::Database => self.loading,
            Inspector::BackgroundTasks => self.background.read(cx).loading,
            Inspector::Network => false,
        }
    }

    fn load_background(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(app)) = (self.active.clone(), self.adb(cx), self.app.clone()) else { return };
        self.background.update(cx, |b, cx| b.load(adb, serial, app, cx));
    }

    fn show_inspector(&mut self, inspector: Inspector, cx: &mut Context<Self>) {
        if self.inspector == inspector {
            return;
        }
        if self.inspector == Inspector::Network {
            self.network.update(cx, |n, _| n.stop());
        }
        self.inspector = inspector;
        self.error = None;
        self.refresh(cx);
        cx.notify();
    }

    /// Reads the app's databases again, keeping the table shown.
    fn load_databases(&mut self, cx: &mut Context<Self>) {
        let (Some(serial), Some(adb), Some(app)) = (self.active.clone(), self.adb(cx), self.app.clone()) else {
            self.databases.clear();
            self.grid = None;
            return;
        };
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let task = cx.background_spawn(async move { inspect_database::open_all(&crate::infra::adb_databases::AdbDatabases { adb }, &serial, &app) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((databases, errors)) => {
                        this.databases = databases;
                        this.error = (!errors.is_empty()).then(|| errors.join("\n"));
                        let shown = this.grid.take();
                        match shown {
                            Some(Grid { database, query: Some(sql), .. }) => this.show_query(&database, &sql),
                            Some(Grid { database, table, .. }) => this.show_table(&database, &table),
                            None => {}
                        }
                    }
                    Err(e) => {
                        this.databases.clear();
                        this.grid = None;
                        this.error = Some(format!("{e} (the app must be debuggable)"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn show_table(&mut self, database: &str, table: &str) {
        let Some(db) = self.databases.iter().find(|d| d.name == database) else { return };
        let Some(t) = db.tables.iter().find(|t| t.name == table) else { return };
        match inspect_database::rows(db, t, ROW_LIMIT) {
            Ok(rows) => self.grid = Some(Grid { database: database.to_owned(), table: table.to_owned(), query: None, columns: t.columns.clone(), rows }),
            Err(e) => {
                self.grid = None;
                self.error = Some(format!("{table}: {e}"));
            }
        }
    }

    /// The query bar's database: the one picked, else the shown table's, else the first.
    fn query_target(&self) -> Option<String> {
        let known = |name: &String| self.databases.iter().any(|d| &d.name == name);
        self.query_database.clone().filter(known).or_else(|| self.grid.as_ref().map(|g| g.database.clone()).filter(known)).or_else(|| self.databases.first().map(|d| d.name.clone()))
    }

    fn run_query(&mut self, cx: &mut Context<Self>) {
        let sql = self.query.read(cx).value().trim().to_owned();
        if sql.is_empty() {
            return;
        }
        let Some(database) = self.query_target() else { return };
        self.error = None;
        self.show_query(&database, &sql);
        cx.notify();
    }

    fn show_query(&mut self, database: &str, sql: &str) {
        let Some(db) = self.databases.iter().find(|d| d.name == database) else { return };
        match inspect_database::query(db, sql, ROW_LIMIT) {
            Ok((table, columns, rows)) => self.grid = Some(Grid { database: database.to_owned(), table, query: Some(sql.to_owned()), columns, rows }),
            Err(e) => self.error = Some(e),
        }
    }

    fn set_live(&mut self, live: bool, cx: &mut Context<Self>) {
        self.live = live;
        if live {
            self.refresh(cx);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LIVE_INTERVAL).await;
                    let go = this
                        .update(cx, |this, cx| {
                            if this.live && !this.busy(cx) {
                                this.refresh(cx);
                            }
                            this.live
                        })
                        .unwrap_or(false);
                    if !go {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn render_rows(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let palette = cx.palette().clone();
        let Some(grid) = &self.grid else { return Vec::new() };
        let width = grid.columns.len().max(1);
        grid.rows[range.start.min(grid.rows.len())..range.end.min(grid.rows.len())]
            .iter()
            .enumerate()
            .map(|(i, row)| {
                h_flex()
                    .h(px(ROW_H))
                    .text_sm()
                    .hover(|el| el.bg(palette.hover))
                    .child(div().w(px(56.)).flex_none().px_2().text_right().text_color(palette.text_secondary).child((range.start + i + 1).to_string()))
                    .children((0..width).map(|c| {
                        let value = row.get(c).cloned().unwrap_or_default();
                        let null = value == "NULL";
                        div()
                            .w(px(COLUMN_W))
                            .flex_none()
                            .px_2()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .border_l_1()
                            .border_color(palette.border)
                            .when(null, |el| el.italic().text_color(palette.text_secondary))
                            .child(value)
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let shown = self.grid.as_ref().filter(|g| g.query.is_none()).map(|g| (g.database.clone(), g.table.clone()));
        let mut list = v_flex().id("ai-databases").size_full().overflow_y_scroll().py_1();
        for db in &self.databases {
            let open = !self.collapsed.contains(&db.name);
            let name = db.name.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("ai-db-{}", db.name)))
                    .h(px(ROW_H))
                    .px_1()
                    .gap_1()
                    .text_sm()
                    .hover(|el| el.bg(palette.hover))
                    .child(Icon::new(if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())
                    .child(Icon::new(icons::DB_OBJECT).small())
                    .child(div().overflow_hidden().whitespace_nowrap().child(db.name.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(i) = this.collapsed.iter().position(|c| *c == name) {
                            this.collapsed.remove(i);
                        } else {
                            this.collapsed.push(name.clone());
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for table in &db.tables {
                let selected = shown.as_ref().is_some_and(|(d, t)| *d == db.name && *t == table.name);
                let (database, name) = (db.name.clone(), table.name.clone());
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("ai-t-{}-{}", db.name, table.name)))
                        .h(px(ROW_H))
                        .pl(px(30.))
                        .pr_1()
                        .gap_1()
                        .text_sm()
                        .when(selected, |el| el.bg(palette.selection))
                        .when(!selected, |el| el.hover(|el| el.bg(palette.hover)))
                        .child(Icon::new(icons::GRID).small())
                        .child(div().overflow_hidden().whitespace_nowrap().child(table.name.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.error = None;
                            this.show_table(&database, &name);
                            cx.notify();
                        })),
                );
            }
        }
        list
    }
}

impl Render for AppInspectionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        let devices = self.devices.clone();
        let device_label = self.active.as_ref().and_then(|s| self.devices.iter().find(|d| &d.serial == s)).map(|d| d.name.clone()).unwrap_or_else(|| "No Connected Devices".into());
        let device_picker = Button::new("ai-device").outline().xsmall().label(device_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                for d in &devices {
                    let (entity, serial) = (entity.clone(), d.serial.clone());
                    menu = menu.item(PopupMenuItem::new(d.name.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.active = Some(serial.clone());
                            this.app = None;
                            this.databases.clear();
                            this.grid = None;
                            this.background.update(cx, |b, cx| b.clear(cx));
                            this.network.update(cx, |n, cx| n.forget(cx));
                            this.load_apps(cx);
                        })
                    }));
                }
                menu
            }
        });
        let apps = self.apps.clone();
        let app_ids = self.app_ids.clone();
        let app_label = self.app.clone().unwrap_or_else(|| "Select a process".into());
        let app_picker = Button::new("ai-app").outline().xsmall().label(app_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, _| {
                let mut separated = false;
                for app in &apps {
                    if !app_ids.contains(app) && !separated && !app_ids.is_empty() {
                        menu = menu.separator();
                        separated = true;
                    }
                    let (entity, app) = (entity.clone(), app.clone());
                    menu = menu.item(PopupMenuItem::new(app.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.app = Some(app.clone());
                            this.app_changed(cx);
                        })
                    }));
                }
                menu.separator().item(PopupMenuItem::new("Refresh").on_click({
                    let entity = entity.clone();
                    move |_, _, cx| entity.update(cx, |this, cx| this.load_apps(cx))
                }))
            }
        });
        let busy = self.busy(cx);
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("App Inspection"))
            .child(
                h_flex()
                    .gap_0p5()
                    .child(Button::new("ai-tab-db").xsmall().ghost().label("Database Inspector").selected(self.inspector == Inspector::Database).on_click(cx.listener(|this, _, _, cx| this.show_inspector(Inspector::Database, cx))))
                    .child(
                        Button::new("ai-tab-bt")
                            .xsmall()
                            .ghost()
                            .label("Background Task Inspector")
                            .selected(self.inspector == Inspector::BackgroundTasks)
                            .on_click(cx.listener(|this, _, _, cx| this.show_inspector(Inspector::BackgroundTasks, cx))),
                    )
                    .child(Button::new("ai-tab-net").xsmall().ghost().label("Network Inspector").selected(self.inspector == Inspector::Network).on_click(cx.listener(|this, _, _, cx| this.show_inspector(Inspector::Network, cx)))),
            )
            .child(device_picker)
            .child(app_picker)
            .when(self.inspector != Inspector::Network, |el| el.child(
                Button::new("ai-refresh")
                    .xsmall()
                    .outline()
                    .icon(Icon::new(icons::REFRESH))
                    .label(if busy { "Reading…" } else { "Refresh" })
                    .disabled(self.app.is_none() || busy)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            ))
            .when(self.inspector == Inspector::BackgroundTasks, |el| el.child(self.background.update(cx, |b, cx| b.tag_filter(cx).into_any_element())))
            .when(self.inspector != Inspector::Network, |el| el.child(Checkbox::new("ai-live").label("Live updates").checked(self.live).on_change({
                let entity = entity.clone();
                move |v, _, cx| {
                    let v = *v;
                    entity.update(cx, |this, cx| this.set_live(v, cx))
                }
            })))
            .child(div().flex_1())
            .child(tool_button("ai-hide", icons::HIDE, "Hide").on_click(cx.listener(|_, _, _, cx| cx.emit(AppInspectionEvent::Hide))));
        let grid = self.grid.as_ref().map(|grid| {
            let count = grid.rows.len();
            let width = 56. + COLUMN_W * grid.columns.len().max(1) as f32;
            let note = if count >= ROW_LIMIT { format!("First {ROW_LIMIT} rows") } else { format!("{count} rows") };
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .flex_shrink_0()
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(if grid.query.is_some() { "Query result".to_owned() } else { grid.table.clone() }))
                        .child(div().text_color(palette.text_secondary).child(format!("{} · {note}", grid.database)))
                        .when_some(grid.query.clone(), |el, q| el.child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(palette.text_secondary).child(q))),
                )
                .child(
                    div().id("ai-grid-scroll").flex_1().min_h_0().overflow_x_scroll().child(
                        v_flex()
                            .w(px(width))
                            .h_full()
                            .child(
                                h_flex()
                                    .h(px(ROW_H))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.text_secondary)
                                    .border_b_1()
                                    .border_color(palette.border)
                                    .child(div().w(px(56.)).flex_none())
                                    .children(grid.columns.iter().map(|c| div().w(px(COLUMN_W)).flex_none().px_2().overflow_hidden().whitespace_nowrap().border_l_1().border_color(palette.border).child(c.clone()))),
                            )
                            .child(uniform_list("ai-rows", count, cx.processor(Self::render_rows)).flex_1().min_h_0()),
                    ),
                )
        });
        let hint = if self.app.is_none() {
            "Pick a running debuggable app to inspect its databases."
        } else if self.loading && self.databases.is_empty() {
            "Reading the app's databases…"
        } else if self.databases.is_empty() {
            "The app has no databases."
        } else {
            "Pick a table to see its rows."
        };
        if self.inspector != Inspector::Database {
            let body = if self.inspector == Inspector::Network { self.network.clone().into_any_element() } else { self.background.clone().into_any_element() };
            return v_flex()
                .size_full()
                .child(header)
                .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
                .child(div().flex_1().min_h_0().child(body));
        }
        let query_bar = (!self.databases.is_empty()).then(|| {
            let names: Vec<String> = self.databases.iter().map(|d| d.name.clone()).collect();
            let target = self.query_target().unwrap_or_default();
            h_flex()
                .px_2()
                .py_1()
                .gap_2()
                .flex_shrink_0()
                .border_b_1()
                .border_color(palette.border)
                .child(Button::new("ai-query-db").outline().xsmall().label(target).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
                    let entity = entity.clone();
                    move |mut menu, _, _| {
                        for name in &names {
                            let (entity, name) = (entity.clone(), name.clone());
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.query_database = Some(name.clone());
                                    cx.notify();
                                })
                            }));
                        }
                        menu
                    }
                }))
                .child(div().flex_1().min_w_0().child(Input::new(&self.query).xsmall().cleanable(true)))
                .child(Button::new("ai-run").xsmall().outline().label("Run").on_click(cx.listener(|this, _, _, cx| this.run_query(cx))))
        });
        v_flex()
            .size_full()
            .child(header)
            .children(query_bar)
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().w(px(240.)).flex_none().h_full().border_r_1().border_color(palette.border).child(self.render_tree(cx)))
                    .when_some(grid, |el, grid| el.child(grid))
                    .when(self.grid.is_none(), |el| el.child(div().flex_1().p_4().text_sm().text_color(palette.text_secondary).child(hint))),
            )
    }
}

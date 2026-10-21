//! The App Inspection tool window's Database Inspector, as Android Studio
//! shows it: a device and app picker, the app's databases with their
//! tables, and the picked table's rows in a grid. The databases are read
//! from the device (`run-as`, so the app must be debuggable) with their
//! WAL applied; Refresh reads them again.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
    button::Button,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px, uniform_list,
};

use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::device_manager::DeviceManager;
use crate::usecase::inspect_database::{self, OpenedDatabase};

pub enum AppInspectionEvent {
    Hide,
}

const ROW_H: f32 = 22.;
const COLUMN_W: f32 = 160.;
/// Rows read from a table; Android Studio pages through 50 at a time.
const ROW_LIMIT: usize = 5000;

#[derive(Clone)]
struct Target {
    serial: String,
    name: String,
}

/// The table on screen: (database, table) names and its rows.
struct Grid {
    database: String,
    table: String,
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
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AppInspectionEvent> for AppInspectionView {}

impl AppInspectionView {
    pub fn new(device_manager: Entity<DeviceManager>, project: Rc<dyn Fn(&App) -> Option<PathBuf>>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&device_manager, |this, dm, cx| {
            this.devices = dm.read(cx).devices().iter().filter(|d| d.online).filter_map(|d| Some(Target { serial: d.serial.clone()?, name: d.name.clone() })).collect();
            if this.active.as_ref().is_none_or(|s| !this.devices.iter().any(|d| &d.serial == s)) {
                this.active = this.devices.first().map(|d| d.serial.clone());
                this.load_apps(cx);
            }
            cx.notify();
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
            _subscriptions: vec![observe],
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
                            this.load_databases(cx);
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
                        let shown = this.grid.as_ref().map(|g| (g.database.clone(), g.table.clone()));
                        this.grid = None;
                        if let Some((database, table)) = shown {
                            this.show_table(&database, &table);
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
            Ok(rows) => self.grid = Some(Grid { database: database.to_owned(), table: table.to_owned(), columns: t.columns.clone(), rows }),
            Err(e) => {
                self.grid = None;
                self.error = Some(format!("{table}: {e}"));
            }
        }
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
        let shown = self.grid.as_ref().map(|g| (g.database.clone(), g.table.clone()));
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
                            this.grid = None;
                            this.load_databases(cx);
                        })
                    }));
                }
                menu.separator().item(PopupMenuItem::new("Refresh").on_click({
                    let entity = entity.clone();
                    move |_, _, cx| entity.update(cx, |this, cx| this.load_apps(cx))
                }))
            }
        });
        let header = h_flex()
            .h(px(crate::ui::common::header_height()))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("App Inspection"))
            .child(div().text_xs().px_2().py(px(2.)).rounded_sm().bg(palette.selection).child("Database Inspector"))
            .child(device_picker)
            .child(app_picker)
            .child(
                Button::new("ai-refresh")
                    .xsmall()
                    .outline()
                    .icon(Icon::new(icons::REFRESH))
                    .label(if self.loading { "Reading…" } else { "Refresh" })
                    .disabled(self.app.is_none() || self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load_databases(cx))),
            )
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
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(grid.table.clone()))
                        .child(div().text_color(palette.text_secondary).child(format!("{} · {note}", grid.database))),
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
        v_flex()
            .size_full()
            .child(header)
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

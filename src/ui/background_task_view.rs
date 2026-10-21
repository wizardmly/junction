//! App Inspection's Background Task Inspector, as Android Studio shows it:
//! the app's workers, jobs, alarms and wake locks in one table with Class
//! Name, Status, Start and Result columns, and the picked task's details
//! beside it (a worker's tags, constraints, frequency, input and output,
//! and the work before and after it, which can be followed). Workers come
//! from WorkManager's database, the rest from `dumpsys`; Android Studio's
//! in-app agent isn't used, so tasks can't be cancelled from here.

use gpui_kit::component::{Icon, Sizable as _, button::Button, h_flex, menu::{DropdownMenu as _, PopupMenuItem}, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::domain::background_tasks::{JobState, WorkState, duration_ms, relative};
use crate::theme::{ActivePalette as _, Palette};
use crate::ui::as_icons as icons;
use crate::usecase::inspect_background_tasks::{self, BackgroundTasks};

const ROW_H: f32 = 22.;
const NAME_W: f32 = 300.;
const STATUS_W: f32 = 110.;
const START_W: f32 = 120.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Workers,
    Jobs,
    Alarms,
    WakeLocks,
}

impl Category {
    const ALL: [Category; 4] = [Category::Workers, Category::Jobs, Category::Alarms, Category::WakeLocks];

    fn label(self) -> &'static str {
        match self {
            Self::Workers => "Workers",
            Self::Jobs => "Jobs",
            Self::Alarms => "Alarms",
            Self::WakeLocks => "WakeLocks",
        }
    }
}

/// A task in the table: its category and index there.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Picked(Category, usize);

pub struct BackgroundTaskView {
    tasks: BackgroundTasks,
    loaded: bool,
    pub loading: bool,
    error: Option<String>,
    collapsed: Vec<Category>,
    picked: Option<Picked>,
    /// The worker tag the table is filtered to.
    tag: Option<String>,
}

impl BackgroundTaskView {
    pub fn new() -> Self {
        Self { tasks: BackgroundTasks::default(), loaded: false, loading: false, error: None, collapsed: Vec::new(), picked: None, tag: None }
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.tasks = BackgroundTasks::default();
        self.loaded = false;
        self.picked = None;
        self.tag = None;
        cx.notify();
    }

    /// Reads the app's tasks again, keeping the picked one.
    pub fn load(&mut self, adb: crate::android::adb::Adb, serial: String, app: String, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let task = cx.background_spawn(async move {
            let databases = crate::infra::adb_databases::AdbDatabases { adb: adb.clone() };
            let tasks = crate::infra::adb_background_tasks::AdbBackgroundTasks { adb };
            inspect_background_tasks::load(&databases, &tasks, &serial, &app)
        });
        cx.spawn(async move |this, cx| {
            let tasks = task.await;
            this.update(cx, |this, cx| {
                let picked_worker = this.picked_worker_id();
                this.loading = false;
                this.loaded = true;
                this.error = (!tasks.errors.is_empty()).then(|| tasks.errors.join("\n"));
                this.tasks = tasks;
                // Workers keep their pick by id; the others by position.
                this.picked = match (this.picked, picked_worker) {
                    (Some(Picked(Category::Workers, _)), Some(id)) => this.tasks.workers.iter().position(|w| w.id == id).map(|i| Picked(Category::Workers, i)),
                    (Some(p), _) if this.count(p.0) > p.1 => Some(p),
                    _ => None,
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn picked_worker_id(&self) -> Option<String> {
        match self.picked {
            Some(Picked(Category::Workers, i)) => self.tasks.workers.get(i).map(|w| w.id.clone()),
            _ => None,
        }
    }

    fn count(&self, category: Category) -> usize {
        match category {
            Category::Workers => self.tasks.workers.len(),
            Category::Jobs => self.tasks.jobs.len(),
            Category::Alarms => self.tasks.alarms.len(),
            Category::WakeLocks => self.tasks.wake_locks.len(),
        }
    }

    fn pick_worker(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.tasks.workers.iter().position(|w| w.id == id) {
            self.picked = Some(Picked(Category::Workers, i));
            cx.notify();
        }
    }

    /// The worker tags, for the filter.
    fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.tasks.workers.iter().flat_map(|w| w.tags.iter().cloned()).collect();
        tags.sort();
        tags.dedup();
        tags
    }

    /// The table's rows of a category: index, name, status (and its color), start, result.
    fn rows(&self, category: Category, palette: &Palette) -> Vec<(usize, String, String, Hsla, String, String)> {
        match category {
            Category::Workers => self
                .tasks
                .workers
                .iter()
                .enumerate()
                .filter(|(_, w)| self.tag.as_ref().is_none_or(|t| w.tags.contains(t)))
                .map(|(i, w)| {
                    let result = match w.state {
                        WorkState::Succeeded if w.output.is_empty() => "Success".to_owned(),
                        WorkState::Succeeded => format!("Success · {} output", w.output.len()),
                        WorkState::Failed => "Failure".to_owned(),
                        WorkState::Cancelled => "Cancelled".to_owned(),
                        _ => String::new(),
                    };
                    (i, w.simple_class().to_owned(), w.state.label().to_owned(), work_color(w.state, palette), w.enqueued_at.map(clock).unwrap_or_default(), result)
                })
                .collect(),
            Category::Jobs => self
                .tasks
                .jobs
                .iter()
                .enumerate()
                .map(|(i, j)| {
                    let color = if j.state == JobState::Running { palette.link } else { palette.text };
                    let result = if j.state == JobState::Waiting && !j.unsatisfied.is_empty() { format!("Waiting for {}", j.unsatisfied.join(", ")) } else { String::new() };
                    (i, format!("{} #{}", j.simple_service(), j.id), j.state.label().to_owned(), color, j.enqueued.as_deref().map(relative).unwrap_or_default(), result)
                })
                .collect(),
            Category::Alarms => self
                .tasks
                .alarms
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let name = if a.tag.is_empty() { a.kind.clone() } else { a.tag.clone() };
                    let result = a.repeat.as_deref().and_then(|r| r.parse::<i64>().ok()).map(|r| format!("Repeats every {}", duration_ms(r))).unwrap_or_default();
                    (i, name, "Scheduled".to_owned(), palette.text, a.when.as_deref().map(relative).unwrap_or_default(), result)
                })
                .collect(),
            Category::WakeLocks => self
                .tasks
                .wake_locks
                .iter()
                .enumerate()
                .map(|(i, l)| (i, l.tag.clone(), "Acquired".to_owned(), palette.link, l.acquired.as_deref().map(relative).unwrap_or_default(), l.level.clone()))
                .collect(),
        }
    }

    fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette().clone();
        let cell = |w: f32| div().w(px(w)).flex_none().px_2().overflow_hidden().whitespace_nowrap();
        let mut list = v_flex().id("bt-table").flex_1().min_h_0().overflow_y_scroll();
        for category in Category::ALL {
            let rows = self.rows(category, &palette);
            let open = !self.collapsed.contains(&category);
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("bt-cat-{}", category.label())))
                    .h(px(ROW_H))
                    .px_1()
                    .gap_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|el| el.bg(palette.hover))
                    .child(Icon::new(if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).xsmall())
                    .child(format!("{} ({})", category.label(), rows.len()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(i) = this.collapsed.iter().position(|c| *c == category) {
                            this.collapsed.remove(i);
                        } else {
                            this.collapsed.push(category);
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for (i, name, status, color, start, result) in rows {
                let picked = self.picked == Some(Picked(category, i));
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("bt-{}-{i}", category.label())))
                        .h(px(ROW_H))
                        .pl(px(18.))
                        .text_sm()
                        .when(picked, |el| el.bg(palette.selection))
                        .when(!picked, |el| el.hover(|el| el.bg(palette.hover)))
                        .child(cell(NAME_W).child(name))
                        .child(cell(STATUS_W).text_color(color).child(status))
                        .child(cell(START_W).text_color(palette.text_secondary).child(start))
                        .child(div().flex_1().min_w_0().px_2().overflow_hidden().whitespace_nowrap().text_color(palette.text_secondary).child(result))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.picked = Some(Picked(category, i));
                            cx.notify();
                        })),
                );
            }
        }
        let header = h_flex()
            .h(px(ROW_H))
            .pl(px(18.))
            .flex_shrink_0()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(cell(NAME_W).child("Class Name"))
            .child(cell(STATUS_W).child("Status"))
            .child(cell(START_W).child("Start"))
            .child(div().px_2().child("Result"));
        v_flex().flex_1().min_w_0().h_full().child(header).child(list).into_any_element()
    }

    fn render_details(&self, picked: Picked, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = cx.palette().clone();
        let section = |title: &str| div().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.text_secondary).child(title.to_owned());
        let line = |label: &str, value: String| {
            h_flex()
                .gap_2()
                .text_sm()
                .items_start()
                .child(div().w(px(110.)).flex_none().text_color(palette.text_secondary).child(label.to_owned()))
                .child(div().flex_1().min_w_0().child(if value.is_empty() { "–".to_owned() } else { value }))
        };
        let data = |values: &[(String, String)]| -> AnyElement {
            if values.is_empty() {
                return line("", "None".into()).into_any_element();
            }
            v_flex().children(values.iter().map(|(k, v)| line(k, v.clone()))).into_any_element()
        };
        let mut body = v_flex().gap_0p5();
        let title: String;
        match picked {
            Picked(Category::Workers, i) => {
                let w = self.tasks.workers.get(i)?;
                title = w.simple_class().to_owned();
                let frequency = if w.periodic() {
                    let flex = if w.flex_ms > 0 && w.flex_ms < w.interval_ms { format!(" (flex {})", duration_ms(w.flex_ms)) } else { String::new() };
                    format!("Periodic, every {}{flex}", duration_ms(w.interval_ms))
                } else {
                    "One Time".to_owned()
                };
                let link = |id: &String, key: &str| -> AnyElement {
                    let name = self.tasks.workers.iter().find(|o| &o.id == id).map(|o| o.simple_class().to_owned()).unwrap_or_else(|| id.clone());
                    let id = id.clone();
                    div()
                        .id(SharedString::from(format!("bt-link-{key}-{id}")))
                        .text_sm()
                        .text_color(palette.link)
                        .cursor_pointer()
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| this.pick_worker(&id, cx)))
                        .into_any_element()
                };
                body = body
                    .child(section("Description"))
                    .child(line("Class", w.class.clone()))
                    .child(line("Tags", w.tags.join(", ")))
                    .when(!w.names.is_empty(), |el| el.child(line("Unique name", w.names.join(", "))))
                    .child(line("UUID", w.id.clone()))
                    .child(section("Execution"))
                    .child(line("Constraints", if w.constraints.is_empty() { "None".into() } else { w.constraints.join("\n") }))
                    .child(line("Frequency", frequency))
                    .child(line("State", w.state.label().to_owned()))
                    .when(w.initial_delay_ms > 0, |el| el.child(line("Initial delay", duration_ms(w.initial_delay_ms))))
                    .when(w.expedited, |el| el.child(line("Expedited", "Yes".into())))
                    .child(section("WorkContinuation"))
                    .child(h_flex().gap_2().text_sm().child(div().w(px(110.)).flex_none().text_color(palette.text_secondary).child("Before")).child(v_flex().children(w.prerequisites.iter().map(|p| link(p, "pre")))).when(w.prerequisites.is_empty(), |el| el.child("–")))
                    .child(h_flex().gap_2().text_sm().child(div().w(px(110.)).flex_none().text_color(palette.text_secondary).child("After")).child(v_flex().children(w.dependents.iter().map(|p| link(p, "dep")))).when(w.dependents.is_empty(), |el| el.child("–")))
                    .child(section("Results"))
                    .child(line("Time started", w.enqueued_at.map(clock).unwrap_or_default()))
                    .child(line("Retries", w.attempts.to_string()))
                    .child(section("Input Data"))
                    .child(data(&w.input))
                    .child(section("Output Data"))
                    .child(if w.state.finished() { data(&w.output) } else { line("", "Awaiting data…".into()).into_any_element() });
            }
            Picked(Category::Jobs, i) => {
                let j = self.tasks.jobs.get(i)?;
                title = format!("Job #{}", j.id);
                body = body
                    .child(section("Description"))
                    .child(line("Service", j.service.clone()))
                    .child(line("Job id", j.id.to_string()))
                    .when(j.work_manager(), |el| el.child(line("Scheduled by", "WorkManager".into())))
                    .child(section("Execution"))
                    .child(line("Constraints", j.required.join(", ")))
                    .child(line("Unsatisfied", j.unsatisfied.join(", ")))
                    .child(line("Frequency", j.periodic.clone().unwrap_or_else(|| "One Time".into())))
                    .child(line("Persisted", if j.persisted { "Yes".into() } else { "No".into() }))
                    .child(section("Results"))
                    .child(line("State", j.state.label().to_owned()))
                    .child(line("Enqueued", j.enqueued.as_deref().map(relative).unwrap_or_default()))
                    .child(line("Run time", j.run_time.clone().unwrap_or_default()));
            }
            Picked(Category::Alarms, i) => {
                let a = self.tasks.alarms.get(i)?;
                title = "Alarm".to_owned();
                body = body
                    .child(section("Description"))
                    .child(line("Tag", a.tag.clone()))
                    .child(line("Type", a.kind.clone()))
                    .child(line("Wakes device", if a.wakeup() { "Yes".into() } else { "No".into() }))
                    .child(section("Execution"))
                    .child(line("Fires", a.when.as_deref().map(relative).unwrap_or_default()))
                    .child(line("Window", a.window.as_deref().map(relative).map(|w| w.trim_start_matches("in ").to_owned()).unwrap_or_else(|| "Exact".into())))
                    .child(line("Frequency", a.repeat.as_deref().and_then(|r| r.parse::<i64>().ok()).map(|r| format!("Every {}", duration_ms(r))).unwrap_or_else(|| "One Time".into())))
                    .child(line("Operation", a.operation.clone().unwrap_or_default()));
            }
            Picked(Category::WakeLocks, i) => {
                let l = self.tasks.wake_locks.get(i)?;
                title = "Wake Lock".to_owned();
                body = body.child(section("Description")).child(line("Tag", l.tag.clone())).child(line("Level", l.level.clone())).child(section("Execution")).child(line("Acquired", l.acquired.as_deref().map(relative).unwrap_or_default()));
            }
        }
        Some(
            v_flex()
                .w(px(380.))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .h(px(ROW_H + 4.))
                        .px_2()
                        .gap_2()
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(div().flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().child(title))
                        .child(crate::ui::common::tool_button("bt-close", icons::CLOSE, "Close").on_click(cx.listener(|this, _, _, cx| {
                            this.picked = None;
                            cx.notify();
                        }))),
                )
                .child(div().id("bt-details").flex_1().min_h_0().overflow_y_scroll().px_2().pb_2().child(body))
                .into_any_element(),
        )
    }

    /// The tag filter, shown in the App Inspection header.
    pub fn tag_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tags = self.tags();
        let entity = cx.entity();
        Button::new("bt-tag").outline().xsmall().label(self.tag.clone().map(|t| format!("Tag: {t}")).unwrap_or_else(|| "All tags".into())).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(
            move |mut menu, _, _| {
                let all = entity.clone();
                menu = menu.item(PopupMenuItem::new("All tags").on_click(move |_, _, cx| {
                    all.update(cx, |this, cx| {
                        this.tag = None;
                        cx.notify();
                    })
                }));
                if !tags.is_empty() {
                    menu = menu.separator();
                }
                for tag in &tags {
                    let (entity, tag) = (entity.clone(), tag.clone());
                    menu = menu.item(PopupMenuItem::new(tag.clone()).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.tag = Some(tag.clone());
                            cx.notify();
                        })
                    }));
                }
                menu
            },
        )
    }
}

fn work_color(state: WorkState, palette: &Palette) -> Hsla {
    match state {
        WorkState::Running => palette.link,
        WorkState::Succeeded => palette.status_added,
        WorkState::Failed => palette.status_conflict,
        WorkState::Cancelled => palette.text_disabled,
        WorkState::Enqueued | WorkState::Blocked => palette.text,
    }
}

/// Milliseconds since the epoch as the local time of day.
fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).unwrap_or_default()
}

impl Render for BackgroundTaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let empty = Category::ALL.iter().all(|c| self.count(*c) == 0);
        let details = self.picked.and_then(|p| self.render_details(p, cx));
        let hint = if !self.loaded { if self.loading { "Reading the app's background tasks…" } else { "Pick a running app to see its background tasks." } } else { "The app has no workers, jobs, alarms or wake locks." };
        v_flex()
            .size_full()
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(empty, |el| el.child(div().flex_1().p_4().text_sm().text_color(palette.text_secondary).child(hint)))
                    .when(!empty, |el| el.child(self.render_table(cx)))
                    .children(details),
            )
    }
}

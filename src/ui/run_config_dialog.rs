//! Android Studio's Run / Debug Configurations dialog (Run › Edit
//! Configurations…) for Android App configurations: the list on the left
//! (+ adds one, − removes, Copy duplicates), the selected one's Name,
//! Module, Installation Options (Deploy) and Launch Options (Launch,
//! Activity, Launch Flags) and Miscellaneous › Logcat (Show logcat
//! automatically, Clear log before launch) and the Before launch list
//! (Gradle-aware Make, Run Gradle task, Run External tool; + − ✎ ↑ ↓) on
//! the right. Gradle configurations (+ › Gradle) run a line of tasks and
//! arguments in a build instead. OK saves them for the project.

use std::path::PathBuf;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::run::{BeforeLaunch, LaunchOptions, LaunchTarget, ModuleInfo, SavedRunConfiguration};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::common::tool_button;
use crate::ui::dialogs;

/// A configuration being edited, and the build it belongs to.
#[derive(Clone)]
struct Draft {
    root: PathBuf,
    saved: SavedRunConfiguration,
}

pub struct RunConfigView {
    drafts: Vec<Draft>,
    selected: Option<usize>,
    /// The project's Gradle builds.
    roots: Vec<PathBuf>,
    /// The application modules, by build.
    modules: Vec<(PathBuf, ModuleInfo)>,
    name: Entity<InputState>,
    activity: Entity<InputState>,
    flags: Entity<InputState>,
    /// A Gradle configuration's Run line.
    gradle_run: Entity<InputState>,
    /// The selected row of the Before launch list.
    before_selected: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

/// A build in the Gradle project picker: its folder's name.
fn root_label(root: &std::path::Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// "Unnamed", "Unnamed (1)"… as IntelliJ names a new configuration.
fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.into();
    }
    (1..).map(|i| format!("{base} ({i})")).find(|n| !taken.contains(n)).unwrap_or_default()
}

impl RunConfigView {
    fn names(&self) -> Vec<String> {
        self.drafts.iter().map(|d| d.saved.name.clone()).collect()
    }

    /// The fields into the selected configuration.
    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (name, activity, flags, run) = self
            .selected
            .and_then(|i| self.drafts.get(i))
            .map(|d| {
                let activity = match &d.saved.options.launch {
                    LaunchTarget::Activity(a) => a.clone(),
                    _ => String::new(),
                };
                (d.saved.name.clone(), activity, d.saved.options.flags.clone(), d.saved.gradle.clone().unwrap_or_default())
            })
            .unwrap_or_default();
        self.name.update(cx, |s, cx| s.set_value(name, window, cx));
        self.gradle_run.update(cx, |s, cx| s.set_value(run, window, cx));
        self.activity.update(cx, |s, cx| s.set_value(activity, window, cx));
        self.flags.update(cx, |s, cx| s.set_value(flags, window, cx));
    }

    /// The fields' text into the selected configuration.
    fn store_fields(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.selected.and_then(|i| self.drafts.get_mut(i)) else { return };
        d.saved.name = self.name.read(cx).value().trim().to_owned();
        d.saved.options.flags = self.flags.read(cx).value().trim().to_owned();
        if let Some(run) = &mut d.saved.gradle {
            *run = self.gradle_run.read(cx).value().trim().to_owned();
        }
        if let LaunchTarget::Activity(a) = &mut d.saved.options.launch {
            *a = self.activity.read(cx).value().trim().to_owned();
        }
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.store_fields(cx);
        self.selected = Some(ix);
        self.before_selected = None;
        self.load_fields(window, cx);
        cx.notify();
    }

    /// + (`gradle`: + › Gradle) or Copy.
    fn add(&mut self, copy: bool, gradle: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.store_fields(cx);
        let draft = match (copy, self.selected.and_then(|i| self.drafts.get(i))) {
            (true, Some(d)) => Draft { root: d.root.clone(), saved: SavedRunConfiguration { name: unique_name(&d.saved.name, &self.names()), ..d.saved.clone() } },
            _ if gradle => {
                let Some(root) = self.roots.first() else { return };
                // Android Studio's Gradle configurations have no Before launch tasks.
                let options = LaunchOptions { before_launch: Vec::new(), ..Default::default() };
                Draft { root: root.clone(), saved: SavedRunConfiguration { name: unique_name("Unnamed", &self.names()), module: String::new(), gradle: Some(String::new()), options } }
            }
            _ => {
                let Some((root, module)) = self.modules.first() else { return };
                Draft { root: root.clone(), saved: SavedRunConfiguration { name: unique_name("Unnamed", &self.names()), module: module.path.clone(), gradle: None, options: LaunchOptions::default() } }
            }
        };
        self.drafts.push(draft);
        self.selected = Some(self.drafts.len() - 1);
        self.load_fields(window, cx);
        cx.notify();
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        self.drafts.remove(ix);
        self.selected = (!self.drafts.is_empty()).then(|| ix.min(self.drafts.len() - 1));
        self.load_fields(window, cx);
        cx.notify();
    }

    fn change(&mut self, f: impl FnOnce(&mut Draft), cx: &mut Context<Self>) {
        self.store_fields(cx);
        if let Some(d) = self.selected.and_then(|i| self.drafts.get_mut(i)) {
            f(d);
        }
        cx.notify();
    }

    /// Run Gradle task / Run External tool: asks for the tasks or the
    /// command line, then adds the task (or changes row `edit`).
    fn prompt_task(&mut self, edit: Option<usize>, gradle: bool, window: &mut Window, cx: &mut Context<Self>) {
        let initial = edit
            .and_then(|i| self.selected.and_then(|s| self.drafts.get(s)).and_then(|d| d.saved.options.before_launch.get(i).cloned()))
            .map(|task| match task {
                BeforeLaunch::Gradle(t) | BeforeLaunch::External(t) => t,
                BeforeLaunch::Make => String::new(),
            })
            .unwrap_or_default();
        let this = cx.entity().downgrade();
        let title = if gradle { "Run Gradle Task (e.g. clean :app:lint)" } else { "Run External Tool (program and arguments)" };
        crate::ui::file_menus::prompt_name(
            title.into(),
            initial,
            "OK",
            move |value, _, cx| {
                this.update(cx, |v, cx| {
                    let task = if gradle { BeforeLaunch::Gradle(value) } else { BeforeLaunch::External(value) };
                    let mut at = None;
                    v.change(
                        |d| {
                            let list = &mut d.saved.options.before_launch;
                            match edit {
                                Some(i) if i < list.len() => {
                                    list[i] = task;
                                    at = Some(i);
                                }
                                _ => {
                                    list.push(task);
                                    at = Some(list.len() - 1);
                                }
                            }
                        },
                        cx,
                    );
                    v.before_selected = at;
                })
                .ok();
            },
            window,
            cx,
        );
    }

    /// Edit (✎ or a double click): Gradle and external tool rows.
    fn edit_task(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.selected.and_then(|s| self.drafts.get(s)).and_then(|d| d.saved.options.before_launch.get(ix).cloned());
        match task {
            Some(BeforeLaunch::Gradle(_)) => self.prompt_task(Some(ix), true, window, cx),
            Some(BeforeLaunch::External(_)) => self.prompt_task(Some(ix), false, window, cx),
            _ => {}
        }
    }

    /// − / ↑ / ↓ on the selected Before launch row.
    fn move_task(&mut self, by: Option<isize>, cx: &mut Context<Self>) {
        let Some(ix) = self.before_selected else { return };
        let mut at = None;
        self.change(
            |d| {
                let list = &mut d.saved.options.before_launch;
                if ix >= list.len() {
                    return;
                }
                match by {
                    None => {
                        list.remove(ix);
                        at = (!list.is_empty()).then(|| ix.min(list.len() - 1));
                    }
                    Some(by) => {
                        let to = ix as isize + by;
                        if (0..list.len() as isize).contains(&to) {
                            list.swap(ix, to as usize);
                            at = Some(to as usize);
                        } else {
                            at = Some(ix);
                        }
                    }
                }
            },
            cx,
        );
        self.before_selected = at;
    }

    fn render_before_launch(&self, d: &Draft, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let tasks = d.saved.options.before_launch.clone();
        let selected = self.before_selected.filter(|i| *i < tasks.len());
        let has_make = tasks.contains(&BeforeLaunch::Make);
        let this = cx.entity().downgrade();
        let add = tool_button("rc-bl-add", icons::ADD, "Add").dropdown_menu(move |menu, _, _| {
            let (make, gradle, tool) = (this.clone(), this.clone(), this.clone());
            menu.item(PopupMenuItem::new("Gradle-aware Make").icon(Icon::new(icons::GRADLE)).disabled(has_make).on_click(move |_, _, cx| {
                make.update(cx, |v, cx| {
                    let mut at = None;
                    v.change(
                        |d| {
                            d.saved.options.before_launch.push(BeforeLaunch::Make);
                            at = Some(d.saved.options.before_launch.len() - 1);
                        },
                        cx,
                    );
                    v.before_selected = at;
                })
                .ok();
            }))
            .item(PopupMenuItem::new("Run Gradle task").icon(Icon::new(icons::GRADLE)).on_click(move |_, window, cx| {
                gradle.update(cx, |v, cx| v.prompt_task(None, true, window, cx)).ok();
            }))
            .item(PopupMenuItem::new("Run External tool").icon(Icon::new(icons::TW_TERMINAL)).on_click(move |_, window, cx| {
                tool.update(cx, |v, cx| v.prompt_task(None, false, window, cx)).ok();
            }))
        });
        let editable = selected.is_some_and(|i| tasks[i] != BeforeLaunch::Make);
        let mut rows = v_flex().id("rc-bl-list").h(px(84.)).overflow_y_scroll();
        if tasks.is_empty() {
            rows = rows.child(div().p_2().text_sm().text_color(palette.text_secondary).child("There are no tasks to run before launch"));
        }
        for (ix, task) in tasks.iter().enumerate() {
            let chosen = selected == Some(ix);
            let icon = if matches!(task, BeforeLaunch::External(_)) { icons::TW_TERMINAL } else { icons::GRADLE };
            rows = rows.child(
                h_flex()
                    .id(("rc-bl-row", ix))
                    .h(px(22.))
                    .px_2()
                    .gap_1()
                    .text_sm()
                    .when(chosen, |el| el.bg(palette.selection))
                    .when(!chosen, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Icon::new(icon).xsmall())
                    .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(task.label()))
                    .on_click(cx.listener(move |this, e: &gpui_kit::ClickEvent, window, cx| {
                        this.before_selected = Some(ix);
                        if e.click_count() == 2 {
                            this.edit_task(ix, window, cx);
                        }
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .border_1()
            .border_color(palette.border)
            .rounded_sm()
            .child(
                h_flex()
                    .h(px(26.))
                    .px_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(add)
                    .child(tool_button("rc-bl-remove", icons::REMOVE, "Remove").disabled(selected.is_none()).on_click(cx.listener(|this, _, _, cx| this.move_task(None, cx))))
                    .child(tool_button("rc-bl-edit", icons::EDIT, "Edit").disabled(!editable).on_click(cx.listener(|this, _, window, cx| {
                        if let Some(ix) = this.before_selected {
                            this.edit_task(ix, window, cx);
                        }
                    })))
                    .child(tool_button("rc-bl-up", icons::MOVE_UP, "Move Up").disabled(selected.is_none_or(|i| i == 0)).on_click(cx.listener(|this, _, _, cx| this.move_task(Some(-1), cx))))
                    .child(tool_button("rc-bl-down", icons::MOVE_DOWN, "Move Down").disabled(selected.is_none_or(|i| i + 1 >= tasks.len())).on_click(cx.listener(|this, _, _, cx| this.move_task(Some(1), cx)))),
            )
            .child(rows)
            .into_any_element()
    }

    /// The problem with the configurations, as the dialog shows it.
    fn problem(&self, cx: &App) -> Option<String> {
        let mut names = self.names();
        if let Some(i) = self.selected {
            names[i] = self.name.read(cx).value().trim().to_owned();
        }
        for (i, n) in names.iter().enumerate() {
            if n.is_empty() {
                return Some("Configuration name is empty".into());
            }
            if names[..i].contains(n) {
                return Some(format!("Configuration '{n}' already exists"));
            }
        }
        let selected = self.selected.and_then(|i| self.drafts.get(i));
        if selected.is_some_and(|d| d.saved.gradle.is_some()) && self.gradle_run.read(cx).value().trim().is_empty() {
            return Some("No tasks specified".into());
        }
        let activity_missing = self.selected.and_then(|i| self.drafts.get(i)).is_some_and(|d| matches!(d.saved.options.launch, LaunchTarget::Activity(_))) && self.activity.read(cx).value().trim().is_empty();
        activity_missing.then(|| "Activity not specified".into())
    }

    /// OK: the configurations by build, saved.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.problem(cx).is_some() {
            return false;
        }
        self.store_fields(cx);
        let mut roots: Vec<PathBuf> = self.roots.iter().cloned().chain(self.modules.iter().map(|(r, _)| r.clone())).collect();
        roots.sort();
        roots.dedup();
        for root in roots {
            let list: Vec<SavedRunConfiguration> = self.drafts.iter().filter(|d| d.root == root).map(|d| d.saved.clone()).collect();
            crate::infra::run_config_store::save(&root, &list);
        }
        true
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = cx.palette().clone();
        let Some(d) = self.selected.and_then(|i| self.drafts.get(i)).cloned() else {
            return div().p_4().text_sm().text_color(palette.text_secondary).child("Press + to add an Android App or Gradle configuration.").into_any_element();
        };
        let label = |text: &'static str| div().w(px(110.)).flex_none().text_sm().text_color(palette.text_secondary).child(text);
        let heading = |text: &'static str| div().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).child(text);
        let this = cx.entity().downgrade();
        if d.saved.gradle.is_some() {
            let roots = self.roots.clone();
            let project = Button::new("rc-project").outline().small().icon(Icon::new(icons::GRADLE).small()).label(root_label(&d.root)).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
                let this = this.clone();
                move |mut menu, _, _| {
                    for root in &roots {
                        let (root, this) = (root.clone(), this.clone());
                        menu = menu.item(PopupMenuItem::new(root_label(&root)).on_click(move |_, _, cx| {
                            let root = root.clone();
                            this.update(cx, |v, cx| v.change(|d| d.root = root, cx)).ok();
                        }));
                    }
                    menu
                }
            });
            return v_flex()
                .id("rc-settings")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p_3()
                .gap_2()
                .child(h_flex().gap_2().child(label("Name:")).child(div().flex_1().child(Input::new(&self.name).small())))
                .child(h_flex().gap_2().child(label("Gradle project:")).child(project))
                .child(h_flex().gap_2().child(label("Run:")).child(div().flex_1().child(Input::new(&self.gradle_run).small())))
                .child(div().text_xs().text_color(palette.text_secondary).child("Tasks and arguments, e.g. clean :app:lint --info"))
                .child(heading("Before launch"))
                .child(self.render_before_launch(&d, cx))
                .into_any_element();
        }
        let modules = self.modules.clone();
        let module_label = modules.iter().find(|(r, m)| *r == d.root && m.path == d.saved.module).map_or_else(|| format!("{} (missing)", d.saved.module), |(_, m)| m.name());
        let module = Button::new("rc-module").outline().small().icon(Icon::new(icons::MODULE_ANDROID).small()).label(module_label).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let this = this.clone();
            move |mut menu, _, _| {
                for (root, m) in &modules {
                    let (root, path, this) = (root.clone(), m.path.clone(), this.clone());
                    menu = menu.item(PopupMenuItem::new(m.name()).on_click(move |_, _, cx| {
                        let (root, path) = (root.clone(), path.clone());
                        this.update(cx, |v, cx| v.change(|d| {
                            d.root = root;
                            d.saved.module = path;
                        }, cx)).ok();
                    }));
                }
                menu
            }
        });
        let choice = |id: &'static str, current: &'static str, options: Vec<(&'static str, fn(&mut Draft))>| {
            let this = this.clone();
            Button::new(id).outline().small().label(current).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
                for (label, apply) in &options {
                    let (this, apply) = (this.clone(), *apply);
                    menu = menu.item(PopupMenuItem::new(*label).on_click(move |_, _, cx| {
                        this.update(cx, |v, cx| v.change(apply, cx)).ok();
                    }));
                }
                menu
            })
        };
        let deploy = choice(
            "rc-deploy",
            if d.saved.options.deploy { "Default APK" } else { "Nothing" },
            vec![("Default APK", |d: &mut Draft| d.saved.options.deploy = true), ("Nothing", |d: &mut Draft| d.saved.options.deploy = false)],
        );
        let specified = matches!(d.saved.options.launch, LaunchTarget::Activity(_));
        let launch = choice(
            "rc-launch",
            match d.saved.options.launch {
                LaunchTarget::DefaultActivity => "Default Activity",
                LaunchTarget::Activity(_) => "Specified Activity",
                LaunchTarget::Nothing => "Nothing",
            },
            vec![
                ("Default Activity", |d: &mut Draft| d.saved.options.launch = LaunchTarget::DefaultActivity),
                ("Specified Activity", |d: &mut Draft| {
                    if !matches!(d.saved.options.launch, LaunchTarget::Activity(_)) {
                        d.saved.options.launch = LaunchTarget::Activity(String::new());
                    }
                }),
                ("Nothing", |d: &mut Draft| d.saved.options.launch = LaunchTarget::Nothing),
            ],
        );
        v_flex()
            .id("rc-settings")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .child(h_flex().gap_2().child(label("Name:")).child(div().flex_1().child(Input::new(&self.name).small())))
            .child(h_flex().gap_2().child(label("Module:")).child(module))
            .child(heading("Installation Options"))
            .child(h_flex().gap_2().child(label("Deploy:")).child(deploy))
            .child(heading("Launch Options"))
            .child(h_flex().gap_2().child(label("Launch:")).child(launch))
            .when(specified, |el| {
                el.child(h_flex().gap_2().child(label("Activity:")).child(div().flex_1().child(Input::new(&self.activity).small())))
            })
            .child(h_flex().gap_2().child(label("Launch Flags:")).child(div().flex_1().child(Input::new(&self.flags).small())))
            .child(heading("Logcat"))
            .child(Checkbox::new("rc-show-logcat").label("Show logcat automatically").checked(d.saved.options.show_logcat).on_change({
                let this = this.clone();
                move |value: &bool, _, cx| {
                    let value = *value;
                    this.update(cx, |v, cx| v.change(|d| d.saved.options.show_logcat = value, cx)).ok();
                }
            }))
            .child(Checkbox::new("rc-clear-logcat").label("Clear log before launch").checked(d.saved.options.clear_logcat).on_change({
                let this = this.clone();
                move |value: &bool, _, cx| {
                    let value = *value;
                    this.update(cx, |v, cx| v.change(|d| d.saved.options.clear_logcat = value, cx)).ok();
                }
            }))
            .child(heading("Before launch"))
            .child(self.render_before_launch(&d, cx))
            .into_any_element()
    }
}

impl Render for RunConfigView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let names_now = {
            let mut n = self.names();
            if let Some(i) = self.selected {
                n[i] = self.name.read(cx).value().trim().to_owned();
            }
            n
        };
        let mut list = v_flex().id("rc-list").flex_1().min_h_0().overflow_y_scroll().py_1();
        let group = |icon, title: &'static str| h_flex().h(px(24.)).px_2().gap_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).child(Icon::new(icon).xsmall()).child(title);
        let apps: Vec<usize> = (0..self.drafts.len()).filter(|i| self.drafts[*i].saved.gradle.is_none()).collect();
        let gradle: Vec<usize> = (0..self.drafts.len()).filter(|i| self.drafts[*i].saved.gradle.is_some()).collect();
        let mut rows: Vec<(usize, bool)> = Vec::new();
        if !apps.is_empty() || gradle.is_empty() {
            list = list.child(group(icons::MODULE_ANDROID, "Android App"));
            rows.extend(apps.iter().map(|i| (*i, false)));
        }
        let first_gradle = rows.len();
        rows.extend(gradle.iter().map(|i| (*i, true)));
        for (n, &(ix, is_gradle)) in rows.iter().enumerate() {
            if n == first_gradle && is_gradle {
                list = list.child(group(icons::GRADLE, "Gradle"));
            }
            let name = &names_now[ix];
            let selected = self.selected == Some(ix);
            list = list.child(
                h_flex()
                    .id(("rc-row", ix))
                    .h(px(24.))
                    .pl(px(26.))
                    .pr_2()
                    .gap_1()
                    .text_sm()
                    .when(selected, |el| el.bg(palette.selection))
                    .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
                    .child(Icon::new(if is_gradle { icons::GRADLE } else { icons::MODULE_ANDROID }).xsmall())
                    .child(SharedString::from(name.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.select(ix, window, cx))),
            );
        }
        let has = self.selected.is_some();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .items_start()
                    .h(px(470.))
                    .border_1()
                    .border_color(palette.border)
                    .rounded_md()
                    .child(
                        v_flex()
                            .w(px(240.))
                            .flex_none()
                            .h_full()
                            .border_r_1()
                            .border_color(palette.border)
                            .child(
                                h_flex()
                                    .h(px(28.))
                                    .px_1()
                                    .flex_none()
                                    .border_b_1()
                                    .border_color(palette.border)
                                    .child(tool_button("rc-add", icons::ADD, "Add New Configuration").dropdown_menu({
                                        let (app, gradle) = (cx.entity().downgrade(), cx.entity().downgrade());
                                        let no_apps = self.modules.is_empty();
                                        move |menu, _, _| {
                                            let (app, gradle) = (app.clone(), gradle.clone());
                                            menu.label("Add New Configuration")
                                                .item(PopupMenuItem::new("Android App").icon(Icon::new(icons::MODULE_ANDROID)).disabled(no_apps).on_click(move |_, window, cx| {
                                                    app.update(cx, |v, cx| v.add(false, false, window, cx)).ok();
                                                }))
                                                .item(PopupMenuItem::new("Gradle").icon(Icon::new(icons::GRADLE)).on_click(move |_, window, cx| {
                                                    gradle.update(cx, |v, cx| v.add(false, true, window, cx)).ok();
                                                }))
                                        }
                                    }))
                                    .child(tool_button("rc-remove", icons::REMOVE, "Remove Configuration").disabled(!has).on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))))
                                    .child(tool_button("rc-copy", icons::COPY, "Copy Configuration").disabled(!has).on_click(cx.listener(|this, _, window, cx| this.add(true, false, window, cx)))),
                            )
                            .child(list),
                    )
                    .child(self.render_settings(cx)),
            )
            // The line is kept when empty: the dialog doesn't jump.
            .child(div().h(px(16.)).text_xs().text_color(palette.status_conflict).children(self.problem(cx)))
    }
}

/// Opens Edit Configurations: the saved configurations, else the default
/// one per application module; `on_saved` after OK (to pick one again).
pub fn open(roots: Vec<PathBuf>, modules: Vec<(PathBuf, ModuleInfo)>, current: Vec<(PathBuf, SavedRunConfiguration)>, selected: Option<String>, on_saved: impl Fn(&mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let activity = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. .SettingsActivity"));
        let flags = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. --ez demo true"));
        let gradle_run = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. clean build"));
        let notify = |_: &mut RunConfigView, _: Entity<InputState>, e: &InputEvent, cx: &mut Context<RunConfigView>| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        };
        let _subscriptions = vec![cx.subscribe(&name, notify), cx.subscribe(&activity, notify), cx.subscribe(&flags, notify), cx.subscribe(&gradle_run, notify)];
        let drafts: Vec<Draft> = current.into_iter().map(|(root, saved)| Draft { root, saved }).collect();
        let selected_ix = selected.and_then(|n| drafts.iter().position(|d| d.saved.name == n)).or((!drafts.is_empty()).then_some(0));
        let mut view = RunConfigView { drafts, selected: selected_ix, roots, modules, name, activity, flags, gradle_run, before_selected: None, _subscriptions };
        view.load_fields(window, cx);
        view
    });
    let focus = view.read(cx).name.clone();
    let on_saved = std::rc::Rc::new(on_saved);
    window.open_dialog(cx, move |dialog, _, _| {
        let (view_ok, on_saved) = (view.clone(), on_saved.clone());
        dialog.title("Run/Debug Configurations").w(px(760.)).child(view.clone()).footer(dialogs::footer("OK")).on_ok(move |_, window, cx| {
            if !view_ok.update(cx, |v, cx| v.save(cx)) {
                return false;
            }
            on_saved(window, cx);
            true
        })
    });
    dialogs::focus_input(&focus, window, cx);
}

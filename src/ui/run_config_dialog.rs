//! Android Studio's Run / Debug Configurations dialog (Run › Edit
//! Configurations…) for Android App configurations: the list on the left
//! (+ adds one, − removes, Copy duplicates), the selected one's Name,
//! Module, Installation Options (Deploy) and Launch Options (Launch,
//! Activity, Launch Flags) and Miscellaneous › Logcat (Show logcat
//! automatically, Clear log before launch) on the right. OK saves them for
//! the project.

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

use crate::domain::run::{LaunchOptions, LaunchTarget, ModuleInfo, SavedRunConfiguration};
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
    /// The application modules, by build.
    modules: Vec<(PathBuf, ModuleInfo)>,
    name: Entity<InputState>,
    activity: Entity<InputState>,
    flags: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
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
        let (name, activity, flags) = self
            .selected
            .and_then(|i| self.drafts.get(i))
            .map(|d| {
                let activity = match &d.saved.options.launch {
                    LaunchTarget::Activity(a) => a.clone(),
                    _ => String::new(),
                };
                (d.saved.name.clone(), activity, d.saved.options.flags.clone())
            })
            .unwrap_or_default();
        self.name.update(cx, |s, cx| s.set_value(name, window, cx));
        self.activity.update(cx, |s, cx| s.set_value(activity, window, cx));
        self.flags.update(cx, |s, cx| s.set_value(flags, window, cx));
    }

    /// The fields' text into the selected configuration.
    fn store_fields(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.selected.and_then(|i| self.drafts.get_mut(i)) else { return };
        d.saved.name = self.name.read(cx).value().trim().to_owned();
        d.saved.options.flags = self.flags.read(cx).value().trim().to_owned();
        if let LaunchTarget::Activity(a) = &mut d.saved.options.launch {
            *a = self.activity.read(cx).value().trim().to_owned();
        }
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.store_fields(cx);
        self.selected = Some(ix);
        self.load_fields(window, cx);
        cx.notify();
    }

    fn add(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.store_fields(cx);
        let draft = match (copy, self.selected.and_then(|i| self.drafts.get(i))) {
            (true, Some(d)) => Draft { root: d.root.clone(), saved: SavedRunConfiguration { name: unique_name(&d.saved.name, &self.names()), ..d.saved.clone() } },
            _ => {
                let Some((root, module)) = self.modules.first() else { return };
                Draft { root: root.clone(), saved: SavedRunConfiguration { name: unique_name("Unnamed", &self.names()), module: module.path.clone(), options: LaunchOptions::default() } }
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
        let activity_missing = self.selected.and_then(|i| self.drafts.get(i)).is_some_and(|d| matches!(d.saved.options.launch, LaunchTarget::Activity(_))) && self.activity.read(cx).value().trim().is_empty();
        activity_missing.then(|| "Activity not specified".into())
    }

    /// OK: the configurations by build, saved.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.problem(cx).is_some() {
            return false;
        }
        self.store_fields(cx);
        let mut roots: Vec<PathBuf> = self.modules.iter().map(|(r, _)| r.clone()).collect();
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
            return div().p_4().text_sm().text_color(palette.text_secondary).child("Press + to add an Android App configuration.").into_any_element();
        };
        let label = |text: &'static str| div().w(px(110.)).flex_none().text_sm().text_color(palette.text_secondary).child(text);
        let heading = |text: &'static str| div().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).child(text);
        let this = cx.entity().downgrade();
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
            .flex_1()
            .min_w_0()
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
        let mut list = v_flex().id("rc-list").flex_1().min_h_0().overflow_y_scroll().py_1().child(
            h_flex().h(px(24.)).px_2().gap_1().text_sm().font_weight(FontWeight::SEMIBOLD).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).child(Icon::new(icons::MODULE_ANDROID).xsmall()).child("Android App"),
        );
        for (ix, name) in names_now.iter().enumerate() {
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
                    .child(Icon::new(icons::MODULE_ANDROID).xsmall())
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
                    .h(px(380.))
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
                                    .child(tool_button("rc-add", icons::ADD, "Add New Configuration").on_click(cx.listener(|this, _, window, cx| this.add(false, window, cx))))
                                    .child(tool_button("rc-remove", icons::REMOVE, "Remove Configuration").disabled(!has).on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))))
                                    .child(tool_button("rc-copy", icons::COPY, "Copy Configuration").disabled(!has).on_click(cx.listener(|this, _, window, cx| this.add(true, window, cx)))),
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
pub fn open(modules: Vec<(PathBuf, ModuleInfo)>, current: Vec<(PathBuf, SavedRunConfiguration)>, selected: Option<String>, on_saved: impl Fn(&mut Window, &mut App) + 'static, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let activity = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. .SettingsActivity"));
        let flags = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. --ez demo true"));
        let notify = |_: &mut RunConfigView, _: Entity<InputState>, e: &InputEvent, cx: &mut Context<RunConfigView>| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        };
        let _subscriptions = vec![cx.subscribe(&name, notify), cx.subscribe(&activity, notify), cx.subscribe(&flags, notify)];
        let drafts: Vec<Draft> = current.into_iter().map(|(root, saved)| Draft { root, saved }).collect();
        let selected_ix = selected.and_then(|n| drafts.iter().position(|d| d.saved.name == n)).or((!drafts.is_empty()).then_some(0));
        let mut view = RunConfigView { drafts, selected: selected_ix, modules, name, activity, flags, _subscriptions };
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

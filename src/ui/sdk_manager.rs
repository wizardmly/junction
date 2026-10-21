//! Settings › Languages & Frameworks › Android SDK (Tools › SDK Manager),
//! as Android Studio's: the SDK location, the SDK Platforms and SDK Tools
//! tabs with Hide Obsolete Packages and Show Package Details, and Apply's
//! Confirm Change, License Agreement and SDK Component Installer.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui_kit::component::{
    Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState},
    notification::Notification,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::domain::sdk_packages::{self, Catalog, Package, Plan, Repository, Row, Status};
use crate::infra::sdk_repository;
use crate::theme::ActivePalette as _;
use crate::usecase::manage_sdk::{self, Step};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Platforms,
    Tools,
}

enum Remote {
    Loading,
    Failed(String),
    /// The list; whether it is the copy kept from last time.
    Ready(Arc<Repository>, bool),
}

pub struct SdkManagerView {
    location: Entity<InputState>,
    sdk: PathBuf,
    tab: Tab,
    details: bool,
    hide_obsolete: bool,
    remote: Remote,
    local: Vec<Package>,
    catalog: Catalog,
    /// The checked packages, and the installed ones to update.
    wanted: BTreeSet<String>,
    updates: BTreeSet<String>,
    _subscription: Subscription,
}

impl SdkManagerView {
    pub fn new(project: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sdk = crate::android::sdk::locate(project.as_deref()).or_else(crate::android::sdk::default_sdk).unwrap_or_default();
        let location = cx.new(|cx| InputState::new(window, cx).default_value(sdk.to_string_lossy().into_owned()));
        let subscription = cx.subscribe(&location, |this: &mut Self, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let path = PathBuf::from(input.read(cx).value().trim());
                if path != this.sdk {
                    this.sdk = path;
                    this.rescan(cx);
                }
            }
        });
        let task = cx.background_spawn(async { sdk_repository::fetch_repository() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.remote = match result {
                    Ok((repo, cached)) => Remote::Ready(Arc::new(repo), cached),
                    Err(e) => Remote::Failed(e),
                };
                this.rebuild();
                cx.notify();
            })
            .ok();
        })
        .detach();
        let mut this = Self {
            location,
            sdk,
            tab: Tab::Platforms,
            details: false,
            hide_obsolete: true,
            remote: Remote::Loading,
            local: Vec::new(),
            catalog: Catalog::new(&[], &[]),
            wanted: BTreeSet::new(),
            updates: BTreeSet::new(),
            _subscription: subscription,
        };
        this.local = sdk_repository::installed(&this.sdk);
        this.reset();
        this
    }

    fn repository(&self) -> Option<&Arc<Repository>> {
        match &self.remote {
            Remote::Ready(repo, _) => Some(repo),
            _ => None,
        }
    }

    fn rebuild(&mut self) {
        let remote = self.repository().map(|r| r.packages.clone()).unwrap_or_default();
        self.catalog = Catalog::new(&remote, &self.local);
    }

    /// The checkboxes back to what is installed.
    fn reset(&mut self) {
        self.wanted = self.local.iter().map(|p| p.path.clone()).collect();
        self.updates.clear();
        self.rebuild();
    }

    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        self.local = sdk_repository::installed(&self.sdk);
        self.reset();
        cx.notify();
    }

    pub fn plan(&self) -> Plan {
        sdk_packages::plan(&self.catalog, &self.wanted, &self.updates)
    }

    /// A row's checkbox: checks its first package; an installed one with an
    /// update goes to updating, then to removing.
    fn click(&mut self, packages: &[String], cx: &mut Context<Self>) {
        let Some(first) = packages.first() else { return };
        let any = packages.iter().any(|p| self.wanted.contains(p));
        let updatable = self.catalog.local.contains_key(first) && matches!(self.catalog.status(first), Status::UpdateAvailable(_));
        if !any {
            self.wanted.insert(first.clone());
        } else if updatable && self.wanted.contains(first) && !self.updates.contains(first) {
            self.updates.insert(first.clone());
        } else {
            for p in packages {
                self.wanted.remove(p);
                self.updates.remove(p);
            }
        }
        cx.notify();
    }

    /// What Apply will do to the row's packages, if anything.
    fn pending(&self, row: &Row) -> Option<&'static str> {
        let installed = |p: &String| self.catalog.local.contains_key(p);
        if row.packages.iter().any(|p| installed(p) && !self.wanted.contains(p)) {
            Some("Will be removed")
        } else if row.packages.iter().any(|p| self.updates.contains(p)) {
            Some("Will be updated")
        } else if row.packages.iter().any(|p| !installed(p) && self.wanted.contains(p)) {
            Some("Will be installed")
        } else {
            None
        }
    }

    /// Apply: Confirm Change, the licenses to accept, then the installer.
    pub fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let plan = self.plan();
        let Some(repository) = self.repository().cloned().or_else(|| plan.install.is_empty().then(Arc::default)) else { return };
        if plan.is_empty() {
            return;
        }
        let sdk = self.sdk.clone();
        let this = cx.entity().downgrade();
        // After the Settings dialog OK closes.
        window.defer(cx, move |window, cx| confirm_change(plan, repository, sdk, this, window, cx));
    }
}

fn package_list(title: &str, packages: &[Package], palette: &crate::theme::Palette) -> impl IntoElement {
    v_flex()
        .gap_1()
        .when(!packages.is_empty(), |el| {
            el.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title.to_owned()))
                .children(packages.iter().map(|p| div().pl_4().text_sm().text_color(palette.text).child(format!("{} ({})", p.display_name, p.revision))))
        })
}

fn confirm_change(plan: Plan, repository: Arc<Repository>, sdk: PathBuf, view: WeakEntity<SdkManagerView>, window: &mut Window, cx: &mut App) {
    let size: u64 = plan.install.iter().filter_map(|p| sdk_packages::archive_for(p, std::env::consts::OS, std::env::consts::ARCH)).map(|a| a.size).sum();
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let (plan, repository, sdk, view) = (plan.clone(), repository.clone(), sdk.clone(), view.clone());
        dialog
            .title("Confirm Change")
            .w(px(520.))
            .child(
                v_flex()
                    .gap_3()
                    .child(package_list("The following components will be installed:", &plan.install, &palette))
                    .child(package_list("The following components will be deleted:", &plan.uninstall, &palette))
                    .when(size > 0, |el| el.child(div().text_xs().text_color(palette.text_secondary).child(format!("Download size: {}", crate::domain::apk::size(size)))))
                    .child(div().text_xs().text_color(palette.text_secondary).child(format!("SDK location: {}", sdk.display()))),
            )
            .footer(crate::ui::dialogs::footer("OK"))
            .on_ok(move |_, window, cx| {
                let (plan, repository, sdk, view) = (plan.clone(), repository.clone(), sdk.clone(), view.clone());
                window.defer(cx, move |window, cx| licenses(plan, repository, sdk, view, window, cx));
                true
            })
    });
}

/// License Agreement: the licenses of the downloads not accepted yet.
fn licenses(plan: Plan, repository: Arc<Repository>, sdk: PathBuf, view: WeakEntity<SdkManagerView>, window: &mut Window, cx: &mut App) {
    let needed: Vec<(String, String)> = plan.licenses_to_accept(&repository, &|id, text| sdk_repository::license_accepted(&sdk, id, text)).into_iter().map(|(i, t)| (i.to_owned(), t.to_owned())).collect();
    if needed.is_empty() {
        install(plan, repository, sdk, view, window, cx);
        return;
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        let palette = cx.palette().clone();
        let (plan, repository, sdk, view, needed) = (plan.clone(), repository.clone(), sdk.clone(), view.clone(), needed.clone());
        let texts = needed.iter().map(|(id, text)| {
            v_flex().gap_1().child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(id.clone())).child(div().text_xs().text_color(palette.text).child(text.trim().to_owned()))
        });
        dialog
            .title("License Agreement")
            .w(px(720.))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child("Accept the licenses of the components to install:"))
                    .child(div().id("sdk-licenses").h(px(380.)).p_2().border_1().border_color(palette.border).rounded(px(4.)).overflow_y_scrollbar().child(v_flex().gap_3().children(texts))),
            )
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("sdk-license-decline").label("Decline").outline()))
                    .child(DialogAction::new().child(Button::new("sdk-license-accept").label("Accept").primary())),
            )
            .on_ok(move |_, window, cx| {
                for (id, text) in &needed {
                    if let Err(e) = sdk_repository::accept_license(&sdk, id, text) {
                        window.push_notification(Notification::error(e).title("SDK Manager"), cx);
                        return true;
                    }
                }
                let (plan, repository, sdk, view) = (plan.clone(), repository.clone(), sdk.clone(), view.clone());
                window.defer(cx, move |window, cx| install(plan, repository, sdk, view, window, cx));
                true
            })
    });
}

/// SDK Component Installer: its log and the current download.
struct InstallerView {
    log: Vec<(String, bool)>,
    current: Option<(String, u64, u64)>,
    done: Option<bool>,
    cancel: Arc<AtomicBool>,
}

impl Render for InstallerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let progress = self.current.as_ref().map(|(name, done, total)| {
            let fraction = if *total > 0 { (*done as f32 / *total as f32).clamp(0., 1.) } else { 0. };
            v_flex()
                .gap_1()
                .child(div().text_sm().child(match total {
                    0 => format!("Downloading {name}… {}", crate::domain::apk::size(*done)),
                    _ => format!("Downloading {name}… {} of {}", crate::domain::apk::size(*done), crate::domain::apk::size(*total)),
                }))
                .child(div().h(px(6.)).w_full().rounded(px(3.)).bg(palette.hover).child(div().h_full().rounded(px(3.)).bg(palette.accent).w(gpui_kit::relative(fraction))))
        });
        let mono = {
            use gpui_kit::component::ActiveTheme as _;
            cx.theme().mono_font_family.clone()
        };
        let finished = self.done.is_some();
        let cancel = self.cancel.clone();
        v_flex()
            .gap_2()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(match self.done {
                None => "Installing requested components",
                Some(true) => "Done",
                Some(false) => "Finished with errors",
            }))
            .children(progress)
            .child(
                div()
                    .id("sdk-installer-log")
                    .h(px(240.))
                    .p_2()
                    .border_1()
                    .border_color(palette.border)
                    .rounded(px(4.))
                    .overflow_y_scrollbar()
                    .child(v_flex().children(self.log.iter().map(|(line, error)| div().text_xs().font_family(mono.clone()).text_color(if *error { palette.status_conflict } else { palette.text }).child(line.clone())))),
            )
            .child(
                h_flex().justify_end().gap_2().child(if finished {
                    Button::new("sdk-installer-finish").label("Finish").primary().on_click(|_, window, cx| window.close_dialog(cx))
                } else {
                    Button::new("sdk-installer-cancel").label("Cancel").outline().on_click(move |_, _, _| cancel.store(true, Ordering::Relaxed))
                }),
            )
    }
}

fn install(plan: Plan, repository: Arc<Repository>, sdk: PathBuf, view: WeakEntity<SdkManagerView>, window: &mut Window, cx: &mut App) {
    let cancel = Arc::new(AtomicBool::new(false));
    let installer = cx.new(|_| InstallerView { log: Vec::new(), current: None, done: None, cancel: cancel.clone() });
    let shown = installer.clone();
    window.open_dialog(cx, move |dialog, _, _| dialog.title("SDK Component Installer").w(px(640.)).child(shown.clone()));
    let steps: Arc<Mutex<Vec<Step>>> = Arc::default();
    let sink = steps.clone();
    let folder = sdk_repository::SdkFolder { root: sdk, repository };
    let task = cx.background_spawn(async move { manage_sdk::apply(&folder, &plan, &mut |step| sink.lock().unwrap().push(step), &|| cancel.load(Ordering::Relaxed)) });
    window
        .spawn(cx, async move |cx| {
            let mut task = std::pin::pin!(task);
            let drain = |installer: &Entity<InstallerView>, cx: &mut gpui_kit::AsyncWindowContext| {
                let new: Vec<Step> = std::mem::take(&mut *steps.lock().unwrap());
                installer
                    .update(cx, |v, cx| {
                        for step in new {
                            match step {
                                Step::Uninstalling(name) => v.log.push((format!("Deleting \"{name}\""), false)),
                                Step::Uninstalled(name) => v.log.push((format!("\"{name}\" deleted."), false)),
                                Step::Downloading(name, done, total) => {
                                    if v.current.as_ref().is_none_or(|(n, ..)| *n != name) {
                                        v.log.push((format!("Downloading \"{name}\""), false));
                                    }
                                    v.current = Some((name, done, total));
                                }
                                Step::Installed(name) => {
                                    v.current = None;
                                    v.log.push((format!("\"{name}\" ready."), false));
                                }
                                Step::Failed(name, e) => {
                                    v.current = None;
                                    v.log.push((format!("\"{name}\" failed: {e}"), true));
                                }
                            }
                        }
                        cx.notify();
                    });
            };
            let ok = loop {
                let timer = cx.background_executor().timer(std::time::Duration::from_millis(200));
                match futures::future::select(task.as_mut(), timer).await {
                    futures::future::Either::Left((ok, _)) => break ok,
                    futures::future::Either::Right(_) => drain(&installer, cx),
                }
            };
            drain(&installer, cx);
            installer
                .update(cx, |v, cx| {
                    v.done = Some(ok);
                    v.current = None;
                    cx.notify();
                });
            cx.update(|window, cx| {
                view.update(cx, |v, cx| v.rescan(cx)).ok();
                let notification = if ok { Notification::info("SDK components installed") } else { Notification::error("Some SDK components failed to install") };
                window.push_notification(notification.title("SDK Manager"), cx);
            })
            .ok();
        })
        .detach();
}

impl Render for SdkManagerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, label: &'static str, which: Tab, active: bool| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .text_sm()
                .cursor_pointer()
                .border_b_2()
                .border_color(if active { palette.accent } else { gpui_kit::transparent_black() })
                .when(!active, |el| el.hover(|el| el.bg(palette.hover)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    cx.notify();
                }))
        };
        let rows = match self.tab {
            Tab::Platforms => self.catalog.platform_rows(self.details, self.hide_obsolete),
            Tab::Tools => self.catalog.tool_rows(self.details, self.hide_obsolete),
        };
        let platforms = self.tab == Tab::Platforms;
        let note = match (&self.remote, self.tab) {
            (Remote::Loading, _) => Some(("Loading the list of packages…".to_owned(), false)),
            (Remote::Failed(e), _) => Some((e.clone(), true)),
            (Remote::Ready(_, true), _) => Some(("The repository can't be reached; showing the list from last time.".to_owned(), false)),
            (Remote::Ready(..), Tab::Platforms) => Some((
                "Each Android SDK Platform package includes the Android platform and sources pertaining to an API level by default. Check \"show package details\" to display individual SDK components.".to_owned(),
                false,
            )),
            (Remote::Ready(..), Tab::Tools) => Some(("Below are the available SDK developer tools. Check \"show package details\" to display available versions of an SDK Tool.".to_owned(), false)),
        };
        let cell = |w: f32| div().w(px(w)).flex_none().overflow_hidden().whitespace_nowrap();
        let header = h_flex()
            .h(px(24.))
            .px_1()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(24.)).flex_none())
            .child(div().flex_1().min_w_0().child("Name"))
            .when(platforms, |el| el.child(cell(80.).child("API Level")))
            .child(cell(120.).child(if platforms { "Revision" } else { "Version" }))
            .child(cell(190.).child("Status"));
        let mut table = v_flex();
        for (ix, row) in rows.iter().enumerate() {
            let checkable = row.status.is_some();
            let checked = row.packages.iter().any(|p| self.wanted.contains(p));
            let pending = self.pending(row);
            let packages = row.packages.clone();
            let status = match (pending, &row.status) {
                (Some(p), _) => p.to_owned(),
                (None, Some(s)) => s.to_string(),
                (None, None) => String::new(),
            };
            table = table.child(
                h_flex()
                    .id(("sdk-row", ix))
                    .h(px(24.))
                    .px_1()
                    .text_sm()
                    .hover(|el| el.bg(palette.hover))
                    .child(div().w(px(24.)).flex_none().when(checkable, |el| {
                        el.child(Checkbox::new(("sdk-check", ix)).checked(checked).on_change(cx.listener(move |this, _: &bool, _, cx| this.click(&packages, cx))))
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .pl(px(row.depth as f32 * 18.))
                            .when(!checkable, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(row.name.clone()),
                    )
                    .when(platforms, |el| el.child(cell(80.).child(row.api.clone())))
                    .child(cell(120.).child(row.revision.clone()))
                    .child(cell(190.).text_color(if pending.is_some() { palette.accent } else { palette.text_secondary }).child(status)),
            );
        }
        let entity = cx.entity();
        let (details_entity, obsolete_entity) = (entity.clone(), entity.clone());
        v_flex()
            .gap_2()
            .child(div().text_xs().text_color(palette.text_secondary).child("Manager for the Android SDK and Tools used by the IDE"))
            .child(h_flex().gap_2().text_sm().child("Android SDK Location:").child(div().flex_1().child(Input::new(&self.location).small())))
            .when(!self.sdk.join("platform-tools").is_dir() && self.local.is_empty(), |el| {
                el.child(div().text_xs().text_color(palette.text_secondary).child("No packages are installed here yet; Apply installs the checked ones into this folder."))
            })
            .child(h_flex().border_b_1().border_color(palette.border).child(tab("sdk-tab-platforms", "SDK Platforms", Tab::Platforms, platforms)).child(tab("sdk-tab-tools", "SDK Tools", Tab::Tools, !platforms)))
            .when_some(note, |el, (text, error)| el.child(div().text_xs().text_color(if error { palette.status_conflict } else { palette.text_secondary }).child(SharedString::from(text))))
            .child(v_flex().border_1().border_color(palette.border).rounded(px(4.)).child(header).child(div().id("sdk-rows").max_h(px(330.)).overflow_y_scrollbar().child(table)))
            .child(
                h_flex()
                    .gap_4()
                    .justify_end()
                    .child(Checkbox::new("sdk-hide-obsolete").label("Hide Obsolete Packages").checked(self.hide_obsolete).on_change(move |v, _, cx| {
                        let v = *v;
                        obsolete_entity.update(cx, |this, cx| {
                            this.hide_obsolete = v;
                            cx.notify();
                        })
                    }))
                    .child(Checkbox::new("sdk-details").label("Show Package Details").checked(self.details).on_change(move |v, _, cx| {
                        let v = *v;
                        details_entity.update(cx, |this, cx| {
                            this.details = v;
                            cx.notify();
                        })
                    })),
            )
    }
}

//! IntelliJ's Download JDK (the Gradle JDK dropdown's "Download JDK…"):
//! Version and Vendor from JetBrains' JDK list for this machine, the
//! Location (`~/.jdks/<name>`), then the download with its progress; the
//! JDK it installed is picked as the Gradle JDK.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, h_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::jdk_feed::{self, JdkItem};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;

pub enum JdkDownloadEvent {
    /// Installed: the JDK's home.
    Installed(std::path::PathBuf),
    Closed,
}

enum State {
    Loading,
    Failed(String),
    Ready,
    /// Bytes so far and in all.
    Downloading(u64, u64),
}

pub struct JdkDownloadPanel {
    items: Vec<JdkItem>,
    major: u32,
    /// The picked vendor's item.
    item: usize,
    location: Entity<InputState>,
    state: State,
    cancel: Arc<AtomicBool>,
}

impl EventEmitter<JdkDownloadEvent> for JdkDownloadPanel {}

impl JdkDownloadPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let location = cx.new(|cx| InputState::new(window, cx));
        let task = cx.background_spawn(async { crate::infra::jdk_download::fetch_list() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    // Early-access builds aren't offered.
                    Ok(items) => {
                        this.items = items.into_iter().filter(|i| !i.version.contains("-ea") && !i.product.as_deref().is_some_and(|p| p.contains("Early-Access"))).collect();
                        this.state = State::Ready;
                        // IntelliJ's suggestion: the newest version's default product.
                        if let Some(first) = this.items.first() {
                            this.major = first.major;
                            this.pick(0, window, cx);
                        }
                    }
                    Err(e) => this.state = State::Failed(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { items: Vec::new(), major: 0, item: 0, location, state: State::Loading, cancel: Arc::default() }
    }

    fn pick(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.item = item;
        if let Some(i) = self.items.get(item) {
            let path = crate::infra::jdk_download::default_location(i).to_string_lossy().into_owned();
            self.location.update(cx, |s, cx| s.set_value(path, window, cx));
        }
        cx.notify();
    }

    fn pick_major(&mut self, major: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.major = major;
        if let Some(ix) = self.items.iter().position(|i| i.major == major) {
            self.pick(ix, window, cx);
        }
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(self.item).cloned() else { return };
        let target = std::path::PathBuf::from(self.location.read(cx).value().trim());
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.state = State::Downloading(0, item.size);
        let (tx, rx) = std::sync::mpsc::channel::<(u64, u64)>();
        let task = cx.background_spawn(async move { crate::infra::jdk_download::install(&item, &target, &mut |done, total| { let _ = tx.send((done, total)); }, &|| cancel.load(Ordering::Relaxed)) });
        cx.spawn(async move |this, cx| {
            let mut task = std::pin::pin!(task);
            // Progress while it downloads.
            let result = loop {
                let timer = cx.background_executor().timer(std::time::Duration::from_millis(200));
                match futures::future::select(task.as_mut(), timer).await {
                    futures::future::Either::Left((result, _)) => break result,
                    futures::future::Either::Right(_) => {
                        if let Some(last) = rx.try_iter().last() {
                            this.update(cx, |this, cx| {
                                this.state = State::Downloading(last.0, last.1);
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(home) => {
                        this.state = State::Ready;
                        cx.emit(JdkDownloadEvent::Installed(home));
                    }
                    Err(e) => this.state = State::Failed(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for JdkDownloadPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let label = |text: &'static str| div().w(px(80.)).flex_none().text_color(palette.text_secondary).child(text);
        let this = cx.entity().downgrade();
        let majors = jdk_feed::majors(&self.items);
        let versions = Button::new("jdk-version").outline().small().label(if self.major == 0 { "…".to_owned() } else { self.major.to_string() }).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu({
            let this = this.clone();
            move |mut menu, _, _| {
                for m in &majors {
                    let (m, this) = (*m, this.clone());
                    menu = menu.item(PopupMenuItem::new(m.to_string()).on_click(move |_, window, cx| {
                        this.update(cx, |v, cx| v.pick_major(m, window, cx)).ok();
                    }));
                }
                menu
            }
        });
        let vendors: Vec<(usize, String)> = self.items.iter().enumerate().filter(|(_, i)| i.major == self.major).map(|(ix, i)| (ix, i.label())).collect();
        let current = self.items.get(self.item).map(JdkItem::label).unwrap_or_default();
        let vendor = Button::new("jdk-vendor").outline().small().label(current).child(Icon::new(icons::CHEVRON_DOWN).xsmall()).dropdown_menu(move |mut menu, _, _| {
            for (ix, l) in &vendors {
                let (ix, this) = (*ix, this.clone());
                menu = menu.item(PopupMenuItem::new(l.clone()).on_click(move |_, window, cx| {
                    this.update(cx, |v, cx| v.pick(ix, window, cx)).ok();
                }));
            }
            menu
        });
        let busy = matches!(self.state, State::Downloading(..) | State::Loading);
        let status = match &self.state {
            State::Loading => Some(("Loading the list of JDKs…".to_owned(), false)),
            State::Failed(e) => Some((e.clone(), true)),
            State::Downloading(done, total) if *total > 0 => Some((format!("Downloading… {} of {} MB ({}%)", done >> 20, total >> 20, done * 100 / total), false)),
            State::Downloading(done, _) => Some((format!("Downloading… {} MB", done >> 20), false)),
            State::Ready => None,
        };
        v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(palette.border)
            .rounded_md()
            .bg(palette.panel)
            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Download JDK"))
            .child(h_flex().gap_2().child(label("Version:")).child(versions))
            .child(h_flex().gap_2().child(label("Vendor:")).child(vendor))
            .child(h_flex().gap_2().child(label("Location:")).child(div().flex_1().child(Input::new(&self.location).small())))
            .when_some(status, |el, (text, error)| el.child(div().text_xs().text_color(if error { palette.status_conflict } else { palette.text_secondary }).child(text)))
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("jdk-download").primary().small().label("Download").disabled(busy || self.items.is_empty()).on_click(cx.listener(|this, _, _, cx| this.download(cx))))
                    .child(Button::new("jdk-cancel").ghost().small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                        this.cancel.store(true, Ordering::Relaxed);
                        cx.emit(JdkDownloadEvent::Closed);
                    }))),
            )
    }
}

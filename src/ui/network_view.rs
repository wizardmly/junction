//! App Inspection's Network Inspector. Android Studio shows each HTTP
//! request through an agent inside the app; without one, this shows what
//! the device does: the app's TCP and UDP connections, read from the
//! kernel's socket tables every second while the tab is open, with when
//! each was opened and how long it lasted. Closed connections stay listed
//! until Clear.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::{Sizable as _, button::Button, checkbox::Checkbox, h_flex, v_flex};
use gpui_kit::{AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px};

use crate::domain::background_tasks::duration_ms;
use crate::domain::network_connections::{Connection, ConnectionLog};
use crate::infra::adb_network::AdbNetwork;
use crate::theme::ActivePalette as _;
use crate::usecase::inspect_network;

const ROW_H: f32 = 22.;
const POLL: Duration = Duration::from_secs(1);
const COLUMNS: [(&str, f32); 6] = [("Protocol", 70.), ("Remote Address", 260.), ("Local Address", 220.), ("State", 110.), ("Opened", 90.), ("Duration", 90.)];

/// The app being watched.
#[derive(Clone)]
struct Watched {
    adb: crate::android::adb::Adb,
    serial: String,
    app: String,
}

pub struct NetworkView {
    watched: Option<Watched>,
    uid: Option<u32>,
    log: ConnectionLog,
    error: Option<String>,
    /// Polling: on while the tab is shown. Each start bumps the generation
    /// so an older loop stops.
    generation: u64,
    reading: bool,
    show_listening: bool,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl NetworkView {
    pub fn new() -> Self {
        Self { watched: None, uid: None, log: ConnectionLog::default(), error: None, generation: 0, reading: false, show_listening: false }
    }

    /// Starts watching the app (a different one starts a new log), reading
    /// its sockets every second until [`Self::stop`].
    pub fn watch(&mut self, adb: crate::android::adb::Adb, serial: String, app: String, cx: &mut Context<Self>) {
        let same = self.watched.as_ref().is_some_and(|w| w.serial == serial && w.app == app);
        if !same {
            self.log = ConnectionLog::default();
            self.uid = None;
            self.error = None;
        }
        self.watched = Some(Watched { adb, serial, app });
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            loop {
                let go = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        if !this.reading {
                            this.read(cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
        cx.notify();
    }

    pub fn stop(&mut self) {
        self.generation += 1;
    }

    pub fn forget(&mut self, cx: &mut Context<Self>) {
        self.stop();
        self.watched = None;
        self.uid = None;
        self.log = ConnectionLog::default();
        self.error = None;
        cx.notify();
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(w) = self.watched.clone() else { return };
        self.reading = true;
        let known = self.uid;
        let task = cx.background_spawn(async move {
            let source = AdbNetwork { adb: w.adb };
            let uid = match known {
                Some(uid) => uid,
                None => inspect_network::uid(&source, &w.serial, &w.app)?,
            };
            Ok::<_, String>((uid, inspect_network::sockets(&source, &w.serial, uid)?))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.reading = false;
                match result {
                    Ok((uid, sockets)) => {
                        this.uid = Some(uid);
                        this.error = None;
                        this.log.update(sockets, now_ms());
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn row(&self, i: usize, c: &Connection, now: u64, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let s = &c.socket;
        let end = |(ip, port): (std::net::IpAddr, u16)| if ip.is_ipv6() { format!("[{ip}]:{port}") } else { format!("{ip}:{port}") };
        let remote = if s.connected() { end(s.remote) } else { "–".to_owned() };
        let state = if c.closed { "Closed".to_owned() } else { s.state.to_owned() };
        let opened = chrono::DateTime::from_timestamp_millis(c.first_seen as i64).map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).unwrap_or_default();
        let lasted = duration_ms((if c.closed { c.last_seen } else { now }).saturating_sub(c.first_seen) as i64);
        let values = [s.protocol.label().to_owned(), remote, end(s.local), state, opened, lasted];
        let color = if c.closed { palette.text_secondary } else if s.state == "ESTABLISHED" { palette.text } else { palette.text_secondary };
        h_flex()
            .id(("net-row", i))
            .h(px(ROW_H))
            .text_sm()
            .text_color(color)
            .hover(|el| el.bg(palette.hover))
            .children(COLUMNS.iter().zip(values).map(|((_, w), v)| div().w(px(*w)).flex_none().px_2().overflow_hidden().whitespace_nowrap().child(v)))
    }
}

impl Render for NetworkView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let now = now_ms();
        let shown: Vec<Connection> = self.log.connections.iter().filter(|c| self.show_listening || c.socket.connected()).rev().cloned().collect();
        let open = shown.iter().filter(|c| !c.closed).count();
        let entity = cx.entity();
        let toolbar = h_flex()
            .px_2()
            .py_1()
            .gap_3()
            .flex_shrink_0()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_color(palette.text_secondary).child(format!("{open} open · {} seen", shown.len())))
            .child(Checkbox::new("net-listening").label("Show listening and unconnected sockets").checked(self.show_listening).on_change(move |v, _, cx| {
                let v = *v;
                entity.update(cx, |this, cx| {
                    this.show_listening = v;
                    cx.notify();
                })
            }))
            .child(div().flex_1())
            .child(Button::new("net-clear").xsmall().outline().label("Clear").on_click(cx.listener(|this, _, _, cx| {
                this.log.connections.retain(|c| !c.closed);
                cx.notify();
            })));
        let header = h_flex()
            .h(px(ROW_H))
            .flex_shrink_0()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.text_secondary)
            .border_b_1()
            .border_color(palette.border)
            .children(COLUMNS.iter().map(|(name, w)| div().w(px(*w)).flex_none().px_2().child(*name)));
        let hint = if self.watched.is_none() {
            "Pick a running app to see its network connections."
        } else if self.uid.is_none() && self.error.is_none() {
            "Reading the app's connections…"
        } else {
            "No connections yet. They appear here as the app opens them."
        };
        let rows: Vec<_> = shown.iter().enumerate().map(|(i, c)| self.row(i, c, now, cx).into_any_element()).collect();
        v_flex()
            .size_full()
            .child(toolbar)
            .when_some(self.error.clone(), |el, e| el.child(div().px_2().py_1().text_xs().text_color(palette.status_conflict).child(SharedString::from(e))))
            .child(div().px_2().py_1().text_xs().text_color(palette.text_secondary).child(
                "Android Studio shows HTTP requests through an agent in the app; Junction shows the app's sockets as the device lists them (remote addresses, not host names or request contents).",
            ))
            .child(header)
            .when(rows.is_empty(), |el| el.child(div().p_4().text_sm().text_color(palette.text_secondary).child(hint)))
            .child(v_flex().id("net-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
    }
}

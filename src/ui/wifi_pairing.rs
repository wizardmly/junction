//! Pair devices over Wi-Fi, as Android Studio's dialog: a QR code an
//! Android 11+ device scans from Developer options › Wireless debugging,
//! or the devices waiting for a pairing code, each paired with the six
//! digits it shows. Both go through `adb mdns services` and `adb pair`,
//! then wait for the device to connect.

use std::time::{Duration, Instant};

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, canvas, div, fill, point, prelude::FluentBuilder as _,
    px, rgb, size,
};

use crate::android::adb::{Adb, MdnsService, MdnsSupport, ServiceKind};
use crate::android::pairing::{self, QR_SERVICE_PREFIX, QrPairing};
use crate::theme::ActivePalette as _;
use crate::ui::as_icons as icons;
use crate::ui::device_manager::DeviceManager;

const LEARN_MORE: &str = "https://d.android.com/r/studio-ui/wifi-pairing";
/// How long a paired device has to show up in adb.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, PartialEq)]
enum Phase {
    Loading,
    Error(String),
    Ready,
}

#[derive(Clone, PartialEq)]
enum QrState {
    Waiting,
    Connecting,
    Connected(String),
    Failed,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    QrCode,
    PairingCode,
}

pub struct WifiPairing {
    adb: Option<Adb>,
    manager: WeakEntity<DeviceManager>,
    phase: Phase,
    tab: Tab,
    qr: Option<QrPairing>,
    qr_state: QrState,
    /// Devices waiting for a pairing code.
    services: Vec<MdnsService>,
    closed: bool,
    _tasks: Vec<Task<()>>,
}

/// Pair Devices Using Wi-Fi.
pub fn open(manager: WeakEntity<DeviceManager>, adb: Option<Adb>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| WifiPairing::new(manager, adb, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let closing = view.clone();
        dialog
            .title("Pair devices over Wi-Fi")
            .w(px(600.))
            .child(view.clone())
            .footer(DialogFooter::new().child(DialogClose::new().child(Button::new("wifi-close").label("Close").outline())))
            .on_close(move |_, _, cx| closing.update(cx, |this, _| this.closed = true))
    });
}

/// Blocking: waits for a paired device to come online in adb (connecting
/// it if adb doesn't on its own). The device's name.
fn wait_connected(adb: &Adb, host: &str, guid: &str) -> Result<String, String> {
    let start = Instant::now();
    let mut connect_tried = Instant::now() - Duration::from_secs(60);
    while start.elapsed() < CONNECT_TIMEOUT {
        if let Ok(devices) = adb.devices() {
            let found = devices.into_iter().find(|d| {
                d.state == "device"
                    && ((!guid.is_empty() && d.serial.starts_with(guid))
                        || (d.serial.starts_with(host) && d.serial[host.len()..].chars().next().is_none_or(|c| c == ':')))
            });
            if let Some(device) = found {
                let props = adb.getprop(&device.serial).unwrap_or_default();
                return Ok(crate::android::physical_name(&props, device.model.as_deref().unwrap_or(&device.serial)));
            }
        }
        // adb connects a paired device by itself when it sees its connect
        // service; when it hasn't yet, connect to it directly.
        if connect_tried.elapsed() > Duration::from_secs(5) {
            connect_tried = Instant::now();
            let target = adb
                .mdns_services()
                .unwrap_or_default()
                .into_iter()
                .find(|s| s.kind == ServiceKind::Connect && (s.name == guid || s.host == host));
            if let Some(target) = target {
                let _ = adb.connect(&target.address());
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Device did not connect within specified timeout".into())
}

impl WifiPairing {
    fn new(manager: WeakEntity<DeviceManager>, adb: Option<Adb>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            adb,
            manager,
            phase: Phase::Loading,
            tab: Tab::QrCode,
            qr: None,
            qr_state: QrState::Waiting,
            services: Vec::new(),
            closed: false,
            _tasks: Vec::new(),
        };
        this.start(cx);
        this
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(adb) = self.adb.clone() else {
            self.phase = Phase::Error("Android SDK not found: set ANDROID_HOME to the SDK folder, or install the SDK Platform-Tools.".into());
            return;
        };
        self._tasks.push(cx.spawn(async move |this, cx| {
            let check = cx.background_spawn({
                let adb = adb.clone();
                async move { adb.mdns_check() }
            });
            let update_hint = "Please update to the latest version of \"platform-tools\" using the SDK manager.";
            let phase = match check.await {
                MdnsSupport::Supported => Phase::Ready,
                MdnsSupport::AdbTooOld => {
                    Phase::Error(format!("The currently installed version of the \"Android Debug Bridge\" (adb) does not support Wi-Fi pairing.\n{update_hint}"))
                }
                MdnsSupport::NotSupported | MdnsSupport::Disabled => {
                    Phase::Error(format!("This system does not meet the requirements to support Wi-Fi pairing.\n{update_hint}"))
                }
                MdnsSupport::Error => Phase::Error("There was an unexpected error during Wi-Fi pairing initialization.".into()),
            };
            let ready = phase == Phase::Ready;
            if this
                .update(cx, |this, cx| {
                    this.phase = phase;
                    if ready {
                        this.new_qr_code();
                    }
                    cx.notify();
                })
                .is_err()
                || !ready
            {
                return;
            }
            // Follow the advertised services until the dialog closes.
            loop {
                let services = cx.background_spawn({
                    let adb = adb.clone();
                    async move { adb.mdns_services() }
                });
                let services = services.await.unwrap_or_default();
                let keep_going = this
                    .update(cx, |this, cx| {
                        this.on_services(services, cx);
                        !this.closed
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }));
    }

    fn new_qr_code(&mut self) {
        self.qr = Some(pairing::new_qr());
        self.qr_state = QrState::Waiting;
    }

    fn on_services(&mut self, services: Vec<MdnsService>, cx: &mut Context<Self>) {
        let pairing: Vec<MdnsService> = services.into_iter().filter(|s| s.kind == ServiceKind::Pairing).collect();
        let codes: Vec<MdnsService> = pairing.iter().filter(|s| !s.name.starts_with(QR_SERVICE_PREFIX)).cloned().collect();
        if self.services != codes {
            self.services = codes;
            cx.notify();
        }
        // The device that scanned our code advertises our service name.
        if self.qr_state == QrState::Waiting {
            let scanned = self.qr.as_ref().and_then(|qr| pairing.iter().find(|s| s.name == qr.service).map(|s| (s.clone(), qr.password.clone())));
            if let Some((service, password)) = scanned {
                self.pair_qr(service, password, cx);
            }
        }
    }

    fn pair_qr(&mut self, service: MdnsService, password: String, cx: &mut Context<Self>) {
        let Some(adb) = self.adb.clone() else { return };
        self.qr_state = QrState::Connecting;
        cx.notify();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let (host, guid) = adb.pair(&service.address(), &password)?;
                    wait_connected(&adb, &host, &guid)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(name) => {
                        this.qr_state = QrState::Connected(name.clone());
                        this.manager.update(cx, |m, cx| m.paired(name, cx)).ok();
                    }
                    Err(_) => this.qr_state = QrState::Failed,
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn open_pairing_code(&mut self, service: MdnsService, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(adb), manager) = (self.adb.clone(), self.manager.clone()) else { return };
        let entry = cx.new(|cx| PairingCode::new(adb, manager, service, window, cx));
        let first = entry.read(cx).digits[0].clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (pair, done) = (entry.clone(), entry.read(cx).state.clone());
            let finished = matches!(done, CodeState::Connected(_));
            let busy = matches!(done, CodeState::Pairing | CodeState::Connecting);
            dialog
                .title("Enter pairing code")
                .w(px(460.))
                .child(entry.clone())
                .footer(
                    DialogFooter::new()
                        .gap_2()
                        .when(!finished, |el| el.child(DialogClose::new().child(Button::new("code-cancel").label("Cancel").outline())))
                        .child(DialogAction::new().child(Button::new("code-pair").label(if finished { "Done" } else { "Pair" }).primary().disabled(busy))),
                )
                .on_ok(move |_, _, cx| pair.update(cx, |this, cx| this.submit(cx)))
        });
        crate::ui::dialogs::focus_input(&first, window, cx);
    }

    fn render_qr(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let active = self.qr_state == QrState::Waiting;
        let image: gpui_kit::AnyElement = match (&self.qr_state, &self.qr) {
            (QrState::Connected(_), _) => h_flex().size(px(220.)).justify_center().child(Icon::new(icons::SUCCESS).size(px(96.))).into_any_element(),
            (_, Some(qr)) => {
                let modules = qr.modules.clone();
                canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<gpui_kit::Pixels>, _, window, _| {
                        // A white quiet zone four modules wide, as a QR code needs.
                        let n = modules.len() as f32 + 8.;
                        let cell = bounds.size.width / n;
                        window.paint_quad(fill(bounds, rgb(0xffffff)));
                        for (y, row) in modules.iter().enumerate() {
                            for (x, dark) in row.iter().enumerate() {
                                if *dark {
                                    let origin = point(bounds.origin.x + cell * (x as f32 + 4.), bounds.origin.y + cell * (y as f32 + 4.));
                                    window.paint_quad(fill(Bounds::new(origin, size(cell, cell)), rgb(0x000000)));
                                }
                            }
                        }
                    },
                )
                .size(px(220.))
                .when(!active, |el| el.opacity(0.25))
                .into_any_element()
            }
            _ => div().size(px(220.)).into_any_element(),
        };
        let status: gpui_kit::AnyElement = match &self.qr_state {
            QrState::Waiting => div().child(" ").into_any_element(),
            QrState::Connecting => h_flex()
                .gap_2()
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small())
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Connecting to device. This takes up to 2 minutes."))
                .into_any_element(),
            QrState::Connected(name) => div().font_weight(FontWeight::SEMIBOLD).child(format!("{name} connected")).into_any_element(),
            QrState::Failed => h_flex()
                .gap_2()
                .child(Icon::new(icons::STATUS_ERROR).small())
                .child("An error occurred connecting device. Scan to try again.")
                .into_any_element(),
        };
        let show_link = matches!(self.qr_state, QrState::Connected(_) | QrState::Failed);
        v_flex()
            .items_center()
            .gap_2()
            .py_3()
            .text_sm()
            .when(active, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .child("To pair an")
                        .child(div().font_weight(FontWeight::BOLD).child("Android 11+"))
                        .child("device"),
                )
                .child(div().child("scan the QR code from your device"))
            })
            .child(image)
            .child(status)
            .when(show_link, |el| {
                el.child(
                    div()
                        .id("wifi-scan-new")
                        .text_color(palette.link)
                        .cursor_pointer()
                        .hover(|el| el.underline())
                        .child("Scan new device")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_qr_code();
                            cx.notify();
                        })),
                )
            })
            .child(div().pt_4().font_weight(FontWeight::BOLD).child("QR scanner available at:"))
            .child(div().child("Developer options > Wireless debugging > Pair using QR code"))
    }

    fn render_pairing_code(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let mut list = v_flex().gap_1().w_full();
        for service in &self.services {
            let target = service.clone();
            list = list.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette.border)
                    .child(Icon::new(icons::PHYSICAL_DEVICE_PHONE).size(px(24.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(format!("Device at {}", service.host))
                            .child(div().text_xs().text_color(palette.text_secondary).child("Available to pair")),
                    )
                    .child(
                        Button::new(SharedString::from(format!("wifi-pair-{}", service.name)))
                            .label("Pair")
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| this.open_pairing_code(target.clone(), window, cx))),
                    ),
            );
        }
        v_flex()
            .gap_2()
            .py_3()
            .text_sm()
            .child(div().font_weight(FontWeight::BOLD).child("Set your Android 11+ device to pairing mode"))
            .child(div().child("Go to Developer options > Wireless debugging > Pair device with pairing code"))
            .child(
                h_flex()
                    .pt_3()
                    .gap_2()
                    .text_color(palette.text_secondary)
                    .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small())
                    .child(if self.services.is_empty() { "Searching for devices..." } else { "Available Wi-Fi devices" }),
            )
            .child(list)
    }
}

impl Render for WifiPairing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let tab = |id: &'static str, label: &'static str, value: Tab, current: Tab| {
            div()
                .id(id)
                .px_3()
                .h_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .when(value == current, |el| el.border_b_2().border_color(palette.accent).text_color(palette.text))
                .when(value != current, |el| el.text_color(palette.text_secondary))
                .child(label)
        };
        let header = v_flex()
            .gap_2()
            .pb_3()
            .child(div().text_xl().child("Pair new devices over Wi-Fi"))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .text_sm()
                    .text_color(palette.text_secondary)
                    .child("Pair devices to enable wireless debugging. Other devices can be paired using a pairing code.")
                    .child(div().id("wifi-learn-more").text_color(palette.link).cursor_pointer().child("Learn more.").on_click(|_, _, cx| cx.open_url(LEARN_MORE))),
            );
        let body: gpui_kit::AnyElement = match &self.phase {
            Phase::Loading => h_flex()
                .h(px(420.))
                .justify_center()
                .gap_2()
                .text_sm()
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small())
                .child("Preparing Wi-Fi pairing...")
                .into_any_element(),
            Phase::Error(message) => v_flex()
                .h(px(420.))
                .justify_center()
                .items_center()
                .gap_1()
                .text_sm()
                .text_color(palette.text_secondary)
                .children(message.lines().map(|l| div().child(l.to_owned())))
                .into_any_element(),
            Phase::Ready => {
                let current = self.tab;
                v_flex()
                    .h(px(420.))
                    .child(
                        h_flex()
                            .h(px(30.))
                            .text_sm()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(tab("wifi-tab-qr", "Pair using QR code", Tab::QrCode, current).on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::QrCode;
                                cx.notify();
                            })))
                            .child(tab("wifi-tab-code", "Pair using pairing code", Tab::PairingCode, current).on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::PairingCode;
                                cx.notify();
                            }))),
                    )
                    .child(
                        div().id("wifi-tab-body").flex_1().min_h_0().overflow_y_scroll().map(|el| match current {
                            Tab::QrCode => el.child(self.render_qr(cx)),
                            Tab::PairingCode => el.child(self.render_pairing_code(cx)),
                        }),
                    )
                    .into_any_element()
            }
        };
        v_flex().child(header).child(body)
    }
}

#[derive(Clone, PartialEq)]
enum CodeState {
    Entering,
    Invalid,
    Pairing,
    Connecting,
    Connected(String),
    Failed,
}

/// Enter pairing code: six one-digit boxes.
struct PairingCode {
    adb: Adb,
    manager: WeakEntity<DeviceManager>,
    service: MdnsService,
    digits: Vec<Entity<InputState>>,
    state: CodeState,
    /// Set while the boxes are rewritten, so their change events are ignored.
    updating: bool,
    _subscriptions: Vec<Subscription>,
    _task: Option<Task<()>>,
}

impl PairingCode {
    fn new(adb: Adb, manager: WeakEntity<DeviceManager>, service: MdnsService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let digits: Vec<Entity<InputState>> = (0..6).map(|_| cx.new(|cx| InputState::new(window, cx))).collect();
        let subscriptions = digits
            .iter()
            .enumerate()
            .map(|(ix, input)| {
                cx.subscribe_in(input, window, move |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.on_digit(ix, window, cx),
                    InputEvent::PressEnter { .. } => {
                        this.submit(cx);
                    }
                    _ => {}
                })
            })
            .collect();
        Self { adb, manager, service, digits, state: CodeState::Entering, updating: false, _subscriptions: subscriptions, _task: None }
    }

    fn code(&self, cx: &App) -> String {
        self.digits.iter().map(|d| d.read(cx).value().to_string()).collect()
    }

    /// One digit per box: a typed digit moves to the next box, a pasted
    /// code fills them all.
    fn on_digit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.updating {
            return;
        }
        let text: String = self.digits[ix].read(cx).value().chars().filter(char::is_ascii_digit).collect();
        self.updating = true;
        if text.len() > 1 && text.len() <= 6 && ix == 0 || text.len() == 6 {
            for (i, c) in text.chars().take(6).enumerate() {
                self.digits[i].update(cx, |s, cx| s.set_value(c.to_string(), window, cx));
            }
            let last = text.len().min(6) - 1;
            self.digits[last].update(cx, |s, cx| s.focus(window, cx));
        } else {
            let keep: String = text.chars().last().map(String::from).unwrap_or_default();
            self.digits[ix].update(cx, |s, cx| s.set_value(keep.clone(), window, cx));
            if !keep.is_empty() && ix + 1 < self.digits.len() {
                self.digits[ix + 1].update(cx, |s, cx| s.focus(window, cx));
            }
        }
        self.updating = false;
        if self.state == CodeState::Invalid {
            self.state = CodeState::Entering;
        }
        cx.notify();
    }

    /// Pair (or Done once connected): true closes the dialog.
    fn submit(&mut self, cx: &mut Context<Self>) -> bool {
        match self.state {
            CodeState::Connected(_) => return true,
            CodeState::Pairing | CodeState::Connecting => return false,
            _ => {}
        }
        let code = self.code(cx);
        if code.len() != 6 {
            self.state = CodeState::Invalid;
            cx.notify();
            return false;
        }
        self.state = CodeState::Pairing;
        cx.notify();
        let (adb, service) = (self.adb.clone(), self.service.clone());
        self._task = Some(cx.spawn(async move |this, cx| {
            let paired = cx.background_spawn({
                let adb = adb.clone();
                async move { adb.pair(&service.address(), &code) }
            });
            let result = match paired.await {
                Ok((host, guid)) => {
                    this.update(cx, |this, cx| {
                        this.state = CodeState::Connecting;
                        cx.notify();
                    })
                    .ok();
                    cx.background_spawn(async move { wait_connected(&adb, &host, &guid) }).await
                }
                Err(e) => Err(e),
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(name) => {
                        this.state = CodeState::Connected(name.clone());
                        this.manager.update(cx, |m, cx| m.paired(name, cx)).ok();
                    }
                    Err(_) => this.state = CodeState::Failed,
                }
                cx.notify();
            })
            .ok();
        }));
        false
    }
}

impl Render for PairingCode {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let editable = matches!(self.state, CodeState::Entering | CodeState::Invalid | CodeState::Failed);
        let mut boxes = h_flex().gap_2().justify_center();
        for (ix, digit) in self.digits.iter().enumerate() {
            boxes = boxes.child(div().id(("code-digit", ix)).w(px(40.)).text_xl().child(Input::new(digit).disabled(!editable)));
        }
        let status: Option<gpui_kit::AnyElement> = match &self.state {
            CodeState::Entering => None,
            CodeState::Invalid => Some(div().text_color(palette.status_conflict).child("Pairing code must be exactly 6 digits").into_any_element()),
            CodeState::Pairing | CodeState::Connecting => Some(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).small())
                    .child(if self.state == CodeState::Pairing { "Pairing with device..." } else { "Connecting to device. This takes up to 2 minutes." })
                    .into_any_element(),
            ),
            CodeState::Connected(name) => {
                Some(h_flex().gap_2().child(Icon::new(icons::SUCCESS).small()).child(format!("{name} connected")).into_any_element())
            }
            CodeState::Failed => {
                Some(h_flex().gap_2().child(Icon::new(icons::STATUS_ERROR).small()).child("There was an error pairing the device").into_any_element())
            }
        };
        v_flex()
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child("Enter the 6 digit code shown on the device at")
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(self.service.address()))
                    .child("to pair."),
            )
            .child(boxes)
            .children(status)
    }
}

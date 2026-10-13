//! One mirrored device (a Running Devices tab): its connection, frames,
//! zoom, the toolbar and the screen.

use super::*;

impl MirrorView {
    pub(super) fn new(adb: Adb, serial: String, title: String, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        // Keys go to the device before the IDE's shortcuts see them, as far
        // as Android Studio lets them (all of them with Hardware Input).
        let weak = cx.entity().downgrade();
        let handle = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if !handle.is_focused(window) {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            if this.update(cx, |this, cx| this.on_keystroke(&event.keystroke, cx)) {
                cx.stop_propagation();
            }
        });
        // The computer's clipboard follows to the device while the tab shows.
        let clipboard_poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                let Ok(active) = this.read_with(cx, |this, _| this.active && this.state == State::Live) else { break };
                if !active {
                    continue;
                }
                let text = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
                if this.update(cx, |this, _| this.sync_clipboard(text)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            serial,
            title,
            adb,
            session: None,
            state: State::Connecting,
            image: None,
            retired: Vec::new(),
            frame_size: (0, 0),
            zoom: None,
            fit: Rc::new(Cell::new(1.)),
            zoom_label_until: None,
            screen: Rc::new(Cell::new(Bounds::default())),
            pressed: false,
            hardware_input: false,
            text_helper: true,
            recording: None,
            clipboard: None,
            marked: None,
            focus,
            active: false,
            zoom_controls: true,
            _events: None,
            _clipboard_poll: clipboard_poll,
            hidden_since: None,
            _idle: None,
            typing_notice: None,
            helper_notice: None,
            _intercept: intercept,
        };
        this.connect(cx);
        this
    }

    pub(super) fn connect(&mut self, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.helper_notice = None;
        self.session = Some(Session::start(self.adb.clone(), self.serial.clone(), tx, self.text_helper));
        self.state = State::Connecting;
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(super) fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connected => {
                self.state = State::Live;
                self.zoom_label_until = Some(Instant::now() + Duration::from_secs(2));
            }
            Event::Frame if self.hidden_since.is_some() => return,
            Event::Frame => {
                let Some(frame) = self.session.as_ref().and_then(Session::take_frame) else { return };
                let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra) else { return };
                if self.frame_size != (frame.width, frame.height) {
                    self.frame_size = (frame.width, frame.height);
                    // A rotation: the device's touch area changed too.
                    self.pressed = false;
                }
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                if let Some(old) = self.image.replace(image) {
                    self.retired.push(old);
                }
            }
            Event::Clipboard(text) => {
                if self.clipboard.as_ref() != Some(&text) {
                    self.clipboard = Some(text.clone());
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                return;
            }
            Event::Typing(why) => {
                if self.typing_notice.as_ref() != Some(&why) {
                    self.typing_notice = Some(why.clone());
                    cx.emit(MirrorEvent::Notify { title: format!("Could not type on {}", self.title), message: why, error: true });
                }
                return;
            }
            Event::Helper(result) => {
                // Once per outcome: a retry that fails the same way stays quiet.
                if self.helper_notice.as_ref() != Some(&result) {
                    self.helper_notice = Some(result.clone());
                    let (title, message, error) = match result {
                        Ok(()) => (
                            format!("Direct text input is on for {}", self.title),
                            "Typed text goes straight into the device's focused field, with any keyboard.".to_owned(),
                            false,
                        ),
                        Err(why) => (
                            format!("Direct text input could not start on {}", self.title),
                            format!("{why}\nNon-ASCII text can only be typed with Gboard as the device's keyboard until it starts."),
                            true,
                        ),
                    };
                    cx.emit(MirrorEvent::Notify { title, message, error });
                }
                return;
            }
            Event::Closed(reason) => {
                self.session = None;
                self.pressed = false;
                self.state = State::Failed(reason.unwrap_or_else(|| "Lost connection to the device".into()));
                cx.emit(MirrorEvent::Changed);
            }
        }
        cx.notify();
    }

    pub(super) fn sync_clipboard(&mut self, text: Option<String>) {
        let Some(text) = text.filter(|t| !t.is_empty() && t.len() <= mirror::MAX_CLIPBOARD) else { return };
        if self.clipboard.as_ref() == Some(&text) {
            return;
        }
        if let Some(session) = &self.session {
            session.set_clipboard(&text, false);
        }
        self.clipboard = Some(text);
    }

    pub(super) fn zoom_controls_visible(&self) -> bool {
        self.zoom_controls
    }

    pub(super) fn set_zoom_controls(&mut self, shown: bool) {
        self.zoom_controls = shown;
    }

    pub(super) fn live(&self) -> Option<&Session> {
        self.session.as_ref().filter(|_| self.state == State::Live)
    }

    pub(super) fn rotate(&self, left: bool) {
        if let Some(session) = self.live() {
            session.rotate(left, self.frame_size.0 > self.frame_size.1);
        }
    }

    /// The zoom now, as device pixels per physical screen pixel.
    pub(super) fn scale(&self) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit.get())
    }

    pub(super) fn set_zoom(&mut self, zoom: Option<f32>, cx: &mut Context<Self>) {
        self.zoom = zoom.map(|z| z.clamp(0.05, 4.));
        self.zoom_label_until = Some(Instant::now() + Duration::from_secs(2));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2100)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    pub(super) fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        // IntelliJ's zoom steps snap to round percentages.
        let next = (self.scale() * factor * 20.).round() / 20.;
        let next = if (next - self.scale()).abs() < 0.01 { self.scale() * factor } else { next };
        self.set_zoom(Some(next), cx);
    }

    /// While the tool window is hidden the connection stays (showing it
    /// again is instant) but frames aren't drawn; hidden for
    /// [`IDLE_DISCONNECT`] it ends the device-side server, and showing the
    /// window reconnects.
    pub(super) fn set_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if paused {
            if self.hidden_since.is_none() {
                self.hidden_since = Some(Instant::now());
                self.pressed = false;
                self._idle = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(IDLE_DISCONNECT).await;
                    let _ = this.update(cx, |this, _| {
                        if this.hidden_since.is_some() && this.session.is_some() {
                            this.session = None;
                            this._events = None;
                            this.state = State::Connecting;
                        }
                    });
                }));
            }
            return;
        }
        if self.hidden_since.take().is_none() {
            return;
        }
        self._idle = None;
        if self.session.is_none() && self.state == State::Connecting {
            self.connect(cx);
        } else {
            // The newest frame waited while hidden.
            self.on_event(Event::Frame, cx);
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(recording) = self.recording.take() {
            let (adb, serial) = (self.adb.clone(), self.serial.clone());
            std::thread::spawn(move || recording.cancel(&adb, &serial));
        }
        self.session = None;
    }

    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let live = self.state == State::Live;
        let key = |id: &'static str, icon: icons::AsIcon, tooltip: &'static str, keycode: u32, cx: &mut Context<Self>| {
            tool_button(id, icon, tooltip).disabled(!live).on_click(cx.listener(move |this, _, _, _| this.press(keycode)))
        };
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(palette.border);
        let recording = self.recording.as_ref().map(|r| r.started.elapsed().as_secs());
        h_flex()
            .h(px(crate::ui::common::toolbar_height()))
            .px_1()
            .gap_0p5()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .child(key("rd-power", icons::EMULATOR_POWER, "Power", KEYCODE_POWER, cx))
            .child(key("rd-volume-up", icons::EMULATOR_VOLUME_UP, "Volume Up", KEYCODE_VOLUME_UP, cx))
            .child(key("rd-volume-down", icons::EMULATOR_VOLUME_DOWN, "Volume Down", KEYCODE_VOLUME_DOWN, cx))
            .child(separator())
            .child(tool_button("rd-rotate-left", icons::EMULATOR_ROTATE_LEFT, "Rotate Left").disabled(!live).on_click(cx.listener(|this, _, _, _| this.rotate(true))))
            .child(tool_button("rd-rotate-right", icons::EMULATOR_ROTATE_RIGHT, "Rotate Right").disabled(!live).on_click(cx.listener(|this, _, _, _| this.rotate(false))))
            .child(separator())
            .child(key("rd-back", icons::EMULATOR_BACK, "Back", KEYCODE_BACK, cx))
            .child(key("rd-home", icons::EMULATOR_HOME, "Home", KEYCODE_HOME, cx))
            .child(key("rd-overview", icons::EMULATOR_OVERVIEW, "Overview", KEYCODE_APP_SWITCH, cx))
            .child(separator())
            .child(
                tool_button(
                    "rd-hardware-input",
                    icons::EMULATOR_HARDWARE_INPUT,
                    "Hardware Input: Enable transparent forwarding of keyboard and mouse events to the device",
                )
                .disabled(!live)
                .selected(self.hardware_input)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.hardware_input = !this.hardware_input;
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
            )
            .child(
                tool_button(
                    "rd-text-helper",
                    icons::EDIT,
                    if self.text_helper {
                        "Direct Text Input: On. Typed text goes straight into the device's focused field. Turn off to free the device's UI automation connection for UI tests"
                    } else {
                        "Direct Text Input: Off. Turn on to type non-ASCII text straight into the device's focused field"
                    },
                )
                .disabled(!live)
                .selected(self.text_helper)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.text_helper = !this.text_helper;
                    this.helper_notice = None;
                    this.typing_notice = None;
                    if let Some(session) = this.session.as_ref() {
                        session.set_text_helper(this.text_helper);
                    }
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
            )
            .child(separator())
            .child(tool_button("rd-screenshot", icons::EMULATOR_SCREENSHOT, "Take Screenshot").disabled(!live).on_click(cx.listener(|this, _, window, cx| this.take_screenshot(window, cx))))
            .child(
                tool_button("rd-record", icons::EMULATOR_SCREEN_RECORD, if recording.is_some() { "Stop Recording" } else { "Record Screen" })
                    .disabled(!live && recording.is_none())
                    .selected(recording.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_recording(cx))),
            )
            .when_some(recording, |el, secs| {
                el.child(div().text_xs().text_color(palette.status_conflict).child(format!("{:02}:{:02}", secs / 60, secs % 60)))
            })
    }

    pub(super) fn render_zoom_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        v_flex()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .p_0p5()
            .gap_0p5()
            .rounded(px(6.))
            .border_1()
            .border_color(palette.border)
            .bg(gpui_kit::Hsla { a: 1., ..palette.panel })
            .child(tool_button("rd-zoom-in", icons::ADD, "Zoom In").on_click(cx.listener(|this, _, _, cx| this.zoom_by(1.25, cx))))
            .child(tool_button("rd-zoom-out", icons::REMOVE, "Zoom Out").on_click(cx.listener(|this, _, _, cx| this.zoom_by(0.8, cx))))
            .child(tool_button("rd-zoom-actual", icons::ACTUAL_ZOOM, "Zoom to Actual Size (100%)").on_click(cx.listener(|this, _, _, cx| this.set_zoom(Some(1.), cx))))
            .child(
                tool_button("rd-zoom-fit", icons::FIT_CONTENT, "Zoom to Fit in Window")
                    .selected(self.zoom.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.set_zoom(None, cx))),
            )
    }

    pub(super) fn render_screen(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(image) = self.image.clone() else {
            return div().into_any_element();
        };
        let (fw, fh) = (self.frame_size.0 as f32, self.frame_size.1 as f32);
        let scale_factor = window.scale_factor();
        let (screen, fit, focus, entity) = (self.screen.clone(), self.fit.clone(), self.focus.clone(), cx.entity());
        let paint = canvas(|_, _, _| {}, move |bounds, _, window, cx| {
            // Fit: the largest that fits, centered; zoomed: the canvas is the screen.
            let rect = if zoomed_size(bounds, fw, fh) {
                bounds
            } else {
                let s = (f32::from(bounds.size.width) / fw).min(f32::from(bounds.size.height) / fh);
                fit.set(s * scale_factor);
                let (w, h) = (fw * s, fh * s);
                Bounds::new(
                    point(bounds.origin.x + (bounds.size.width - px(w)) / 2., bounds.origin.y + (bounds.size.height - px(h)) / 2.),
                    size(px(w), px(h)),
                )
            };
            screen.set(rect);
            window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            let _ = window.paint_image(rect, rect, Corners::default(), image, 0, false);
        });
        match self.zoom {
            None => paint.size_full().into_any_element(),
            Some(z) => {
                let (w, h) = (fw * z / scale_factor, fh * z / scale_factor);
                div()
                    .id("rd-scroll")
                    .size_full()
                    .overflow_scroll()
                    .child(
                        div()
                            .min_w_full()
                            .min_h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(paint.w(px(w)).h(px(h)).flex_shrink_0()),
                    )
                    .into_any_element()
            }
        }
    }
}

/// A canvas laid out at the screen's zoomed size has exactly its aspect.
pub(super) fn zoomed_size(bounds: Bounds<Pixels>, fw: f32, fh: f32) -> bool {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    h > 0. && fh > 0. && ((w / h) - (fw / fh)).abs() < 0.002
}

impl Render for MirrorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        let palette = cx.palette().clone();
        let body: gpui_kit::AnyElement = match &self.state {
            State::Connecting => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(palette.text_secondary)
                .child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).color(palette.text_secondary))
                .child("Connecting to the device")
                .into_any_element(),
            State::Failed(reason) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .px_4()
                .text_sm()
                .child(div().text_color(palette.text_secondary).text_center().child(reason.clone()))
                .child(Button::new("rd-reconnect").label("Reconnect").outline().small().on_click(cx.listener(|this, _, _, cx| this.connect(cx))))
                .into_any_element(),
            State::Live => self.render_screen(window, cx),
        };
        let show_label = self.state == State::Live && self.zoom_label_until.is_some_and(|t| Instant::now() < t);
        let percent = (self.scale() * 100.).round() as u32;
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("rd-screen")
                    .key_context("RunningDevice")
                    .track_focus(&self.focus)
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.focus.is_focused(window), |el| el.border_1().border_color(palette.accent.opacity(0.6)))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.on_mouse_down(e, window, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, window, cx| window.focus(&this.focus, cx)))
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| this.on_mouse_move(e, window, cx)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.on_mouse_up(e.position)))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.on_mouse_up(e.position)))
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| this.on_wheel(e, window, cx)))
                    .child(body)
                    .when(self.state == State::Live && self.zoom_controls, |el| el.child(self.render_zoom_controls(cx)))
                    .when(show_label, |el| {
                        el.child(
                            div()
                                .absolute()
                                .bottom(px(8.))
                                .right(px(8.))
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(gpui_kit::Hsla { a: 1., ..palette.panel })
                                .border_1()
                                .border_color(palette.border)
                                .text_sm()
                                .child(format!("{percent}%")),
                        )
                    }),
            )
    }
}

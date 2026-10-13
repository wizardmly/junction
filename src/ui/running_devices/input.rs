//! Input on a mirrored screen: keys and typed text (with the IME), touch,
//! drag and the wheel.

use super::*;

impl MirrorView {
    pub(super) fn press(&self, keycode: u32) {
        if let Some(session) = self.live() {
            session.press(keycode, 0);
        }
    }

    /// A key typed on the screen: special keys and shortcuts as key events;
    /// plain characters come as text through the input handler (so IMEs
    /// work). With Hardware Input every key is a key event.
    pub(super) fn on_keystroke(&mut self, keystroke: &gpui_kit::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.live() else { return false };
        let m = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        if !self.hardware_input {
            // The IDE keeps its tool window shortcuts and Find Action.
            let ide_key = (m.alt && !m.control && key.len() == 1 && key.as_bytes()[0].is_ascii_digit())
                || (m.control && m.shift && key == "a")
                || (m.alt && key == "f12")
                || (m.control && key == "tab");
            if ide_key || m.function {
                return false;
            }
            // AltGr (Ctrl+Alt on Windows) types its character.
            if cfg!(windows) && m.control && m.alt && keystroke.key_char.as_deref().is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control)) {
                return false;
            }
            if is_typed(key) && !(m.control || m.alt || m.platform) {
                return false;
            }
        }
        let Some(code) = keycode(key) else { return false };
        session.press(code, meta_state(m));
        self.marked = None;
        cx.notify();
        true
    }

    /// A window position as a pixel of the device's screen.
    pub(super) fn device_point(&self, position: gpui_kit::Point<Pixels>) -> Option<(i32, i32)> {
        let bounds = self.screen.get();
        let (w, h) = self.frame_size;
        if w == 0 || bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return None;
        }
        let x = f32::from(position.x - bounds.origin.x) / f32::from(bounds.size.width);
        let y = f32::from(position.y - bounds.origin.y) / f32::from(bounds.size.height);
        Some(((x.clamp(0., 1.) * (w - 1) as f32).round() as i32, (y.clamp(0., 1.) * (h - 1) as f32).round() as i32))
    }

    pub(super) fn inside(&self, position: gpui_kit::Point<Pixels>) -> bool {
        self.screen.get().contains(&position)
    }

    pub(super) fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if !self.inside(event.position) {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.touch(mirror::ACTION_DOWN, x, y, w, h);
            self.pressed = true;
        }
    }

    pub(super) fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {
        if !self.pressed {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.touch(mirror::ACTION_MOVE, x, y, w, h);
        }
    }

    pub(super) fn on_mouse_up(&mut self, position: gpui_kit::Point<Pixels>) {
        if !std::mem::take(&mut self.pressed) {
            return;
        }
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(position)) {
            session.touch(mirror::ACTION_UP, x, y, w, h);
        }
    }

    pub(super) fn on_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, _: &mut Context<Self>) {
        if !self.inside(event.position) {
            return;
        }
        let delta = match event.delta {
            // A wheel notch is three lines (the system's scroll setting); the
            // device scrolls by notches.
            gpui_kit::ScrollDelta::Lines(d) => (d.x / 3., d.y / 3.),
            gpui_kit::ScrollDelta::Pixels(d) => (f32::from(d.x) / 40., f32::from(d.y) / 40.),
        };
        let (w, h) = self.frame_size;
        if let (Some(session), Some((x, y))) = (self.live(), self.device_point(event.position)) {
            session.scroll(x, y, w, h, delta.0, delta.1);
        }
    }
}

/// The keys typed as key events rather than text: GPUI's key name to
/// Android's KeyEvent code.
pub(super) fn keycode(key: &str) -> Option<u32> {
    let code = match key {
        "enter" => 66,
        "backspace" => 67,
        "delete" => 112,
        "tab" => 61,
        "escape" => 111,
        "up" => 19,
        "down" => 20,
        "left" => 21,
        "right" => 22,
        "home" => 122,
        "end" => 123,
        "pageup" => 92,
        "pagedown" => 93,
        "insert" => 124,
        "space" => 62,
        "," => 55,
        "." => 56,
        "-" => 69,
        "=" => 70,
        "[" => 71,
        "]" => 72,
        "\\" => 73,
        ";" => 74,
        "'" => 75,
        "/" => 76,
        "`" => 68,
        _ => {
            let mut chars = key.chars();
            let (c, rest) = (chars.next()?, chars.next());
            if rest.is_none() && c.is_ascii_lowercase() {
                29 + (c as u32 - 'a' as u32)
            } else if rest.is_none() && c.is_ascii_digit() {
                7 + (c as u32 - '0' as u32)
            } else if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
                130 + n
            } else {
                return None;
            }
        }
    };
    Some(code)
}

/// The keys that are text unless a modifier makes them a shortcut.
pub(super) fn is_typed(key: &str) -> bool {
    key == "space" || key.chars().count() == 1
}

pub(super) fn meta_state(m: &gpui_kit::Modifiers) -> u32 {
    let mut meta = 0;
    if m.shift {
        meta |= mirror::META_SHIFT_ON | 0x40;
    }
    if m.alt {
        meta |= mirror::META_ALT_ON | 0x10;
    }
    if m.control {
        meta |= mirror::META_CTRL_ON | 0x2000;
    }
    if m.platform {
        meta |= mirror::META_META_ON | 0x20000;
    }
    meta
}

impl gpui_kit::EntityInputHandler for MirrorView {
    fn text_for_range(&mut self, range: std::ops::Range<usize>, adjusted: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let marked = self.marked.as_ref()?;
        let utf16: Vec<u16> = marked.encode_utf16().collect();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        *adjusted = Some(range.clone());
        Some(String::from_utf16_lossy(&utf16[range]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked.as_ref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if let Some(session) = self.live() {
            if !text.is_empty() && !self.hardware_input {
                session.type_text(text);
            }
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<std::ops::Range<usize>>,
        text: &str,
        _: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // The IME window opens under the screen's top left.
        let screen = self.screen.get();
        Some(Bounds::new(screen.origin, size(px(1.), px(20.))))
    }

    fn character_index_for_point(&mut self, _: gpui_kit::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

//! Device mirroring (Running Devices): scrcpy's server (Apache 2.0, bundled
//! in assets/scrcpy) runs on the device as `shell`, streams the screen as
//! H.264 over an adb forward and takes input and clipboard messages on a
//! second socket. The frames are decoded here with OpenH264.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;

use super::adb::Adb;
use super::process;

/// The server's version: the client must pass the exact same one.
const SERVER_VERSION: &str = "3.3.3";
static SERVER: &[u8] = include_bytes!("../../assets/scrcpy/scrcpy-server");
const DEVICE_PATH: &str = "/data/local/tmp/junction-scrcpy-server.jar";
/// Longest clipboard text sent either way (Android Studio's default limit).
pub const MAX_CLIPBOARD: usize = 5000;

/// A decoded screen, BGRA as GPUI draws it.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub enum Event {
    /// The stream is up; the device's name.
    Connected(String),
    /// A new frame waits in [`Session::take_frame`].
    Frame,
    /// The device's clipboard changed.
    Clipboard(String),
    /// The session ended: why, unless it was stopped.
    Closed(Option<String>),
}

// Android's MotionEvent and KeyEvent constants used by the panel.
pub const ACTION_DOWN: u8 = 0;
pub const ACTION_UP: u8 = 1;
pub const ACTION_MOVE: u8 = 2;
pub const META_SHIFT_ON: u32 = 0x1;
pub const META_ALT_ON: u32 = 0x2;
pub const META_CTRL_ON: u32 = 0x1000;
pub const META_META_ON: u32 = 0x10000;

/// The pointer id scrcpy injects as a finger on the touch screen.
const POINTER_ID_GENERIC_FINGER: i64 = -2;

struct Shared {
    frame: Mutex<Option<Frame>>,
    stop: AtomicBool,
    control: Mutex<Option<TcpStream>>,
    video: Mutex<Option<TcpStream>>,
    server: Mutex<Option<Child>>,
    /// What the server printed: shown when it fails.
    log: Mutex<String>,
    sequence: AtomicU64,
}

/// One mirrored device. Dropping it stops the server.
pub struct Session {
    pub serial: String,
    adb: Adb,
    shared: Arc<Shared>,
    port: Arc<Mutex<Option<u16>>>,
}

impl Session {
    /// Starts mirroring in the background; `events` reports how it goes.
    pub fn start(adb: Adb, serial: String, events: UnboundedSender<Event>) -> Self {
        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            stop: AtomicBool::new(false),
            control: Mutex::new(None),
            video: Mutex::new(None),
            server: Mutex::new(None),
            log: Mutex::new(String::new()),
            sequence: AtomicU64::new(1),
        });
        let port = Arc::new(Mutex::new(None));
        {
            let (adb, serial, shared, port) = (adb.clone(), serial.clone(), shared.clone(), port.clone());
            std::thread::spawn(move || {
                let result = run(&adb, &serial, &shared, &port, &events);
                if !shared.stop.load(Ordering::SeqCst) {
                    let message = match result {
                        Ok(()) => "Lost connection to the device".to_owned(),
                        Err(error) => error.to_string(),
                    };
                    let log = shared.log.lock().unwrap().trim().to_owned();
                    let message = server_error(&log).map_or(message, |e| e.to_owned());
                    let _ = events.unbounded_send(Event::Closed(Some(message)));
                }
                cleanup(&adb, &serial, &shared, &port);
            });
        }
        Session { serial, adb, shared, port }
    }

    /// The newest frame, once: frames that came in between are skipped.
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.frame.lock().unwrap().take()
    }

    fn send(&self, message: &[u8]) {
        if let Some(stream) = self.shared.control.lock().unwrap().as_mut() {
            let _ = stream.write_all(message);
        }
    }

    pub fn key(&self, action: u8, keycode: u32, repeat: u32, meta: u32) {
        let mut m = vec![0u8, action];
        m.extend(keycode.to_be_bytes());
        m.extend(repeat.to_be_bytes());
        m.extend(meta.to_be_bytes());
        self.send(&m);
    }

    /// A key press and release.
    pub fn press(&self, keycode: u32, meta: u32) {
        self.key(ACTION_DOWN, keycode, 0, meta);
        self.key(ACTION_UP, keycode, 0, meta);
    }

    /// Text the device types character by character (ASCII; see [`Self::paste`]).
    pub fn text(&self, text: &str) {
        let bytes = &text.as_bytes()[..text.len().min(300)];
        let mut m = vec![1u8];
        m.extend((bytes.len() as u32).to_be_bytes());
        m.extend(bytes);
        self.send(&m);
    }

    /// A finger at (`x`, `y`) of a `w`×`h` frame.
    pub fn touch(&self, action: u8, x: i32, y: i32, w: u32, h: u32) {
        let mut m = vec![2u8, action];
        m.extend(POINTER_ID_GENERIC_FINGER.to_be_bytes());
        m.extend(position(x, y, w, h));
        let pressure: u16 = if action == ACTION_UP { 0 } else { 0xffff };
        m.extend(pressure.to_be_bytes());
        m.extend(0u32.to_be_bytes()); // action button
        m.extend(0u32.to_be_bytes()); // buttons
        self.send(&m);
    }

    /// A wheel turn; `dx`/`dy` in notches, positive right / up.
    pub fn scroll(&self, x: i32, y: i32, w: u32, h: u32, dx: f32, dy: f32) {
        let fixed = |v: f32| -> i16 { ((v / 16.0).clamp(-1.0, 1.0) * 32768.0).clamp(-32768.0, 32767.0) as i16 };
        let mut m = vec![3u8];
        m.extend(position(x, y, w, h));
        m.extend(fixed(dx).to_be_bytes());
        m.extend(fixed(dy).to_be_bytes());
        m.extend(0u32.to_be_bytes());
        self.send(&m);
    }

    /// Sets the device clipboard; `paste` also pastes it where the focus is.
    pub fn set_clipboard(&self, text: &str, paste: bool) {
        let text = truncate(text, MAX_CLIPBOARD);
        let mut m = vec![9u8];
        m.extend(self.shared.sequence.fetch_add(1, Ordering::SeqCst).to_be_bytes());
        m.push(paste as u8);
        m.extend((text.len() as u32).to_be_bytes());
        m.extend(text.as_bytes());
        self.send(&m);
    }

    /// Types any text: ASCII as key events, the rest through the clipboard,
    /// since the device's key map has no keys for it.
    pub fn type_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
            self.text(text);
        } else {
            self.set_clipboard(text, true);
        }
    }

    /// Rotates the screen a quarter turn (counterclockwise when `left`), as
    /// the user rotation with auto-rotate off. `landscape` is what the
    /// frames show now, for when auto-rotate decided the rotation.
    pub fn rotate(&self, left: bool, landscape: bool) {
        let (adb, serial) = (self.adb.clone(), self.serial.clone());
        std::thread::spawn(move || {
            let auto = adb.shell(&serial, "settings get system accelerometer_rotation").map(|s| s.trim() == "1").unwrap_or(true);
            let current = if auto {
                landscape as u32
            } else {
                adb.shell(&serial, "settings get system user_rotation").ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0)
            };
            let next = if left { (current + 1) % 4 } else { (current + 3) % 4 };
            let _ = adb.shell(&serial, &format!("settings put system accelerometer_rotation 0; settings put system user_rotation {next}"));
        });
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for stream in [&self.shared.control, &self.shared.video] {
            if let Some(stream) = stream.lock().unwrap().as_ref() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        if let Some(mut child) = self.shared.server.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let (adb, serial, port) = (self.adb.clone(), self.serial.clone(), self.port.lock().unwrap().take());
        if let Some(port) = port {
            std::thread::spawn(move || {
                let _ = adb.run(&["-s", &serial, "forward", "--remove", &format!("tcp:{port}")], Duration::from_secs(5));
            });
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn position(x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    let mut m = Vec::with_capacity(12);
    m.extend(x.to_be_bytes());
    m.extend(y.to_be_bytes());
    m.extend((w.min(0xffff) as u16).to_be_bytes());
    m.extend((h.min(0xffff) as u16).to_be_bytes());
    m
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The server's own error line, when it printed one.
fn server_error(log: &str) -> Option<&str> {
    log.lines().rev().find_map(|l| l.trim().strip_prefix("ERROR:").or_else(|| l.trim().strip_prefix("[server] ERROR:"))).map(str::trim)
}

/// The server jar on this computer, written once per version.
fn server_file() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("junction-scrcpy-server-{SERVER_VERSION}.jar"));
    if std::fs::metadata(&path).map(|m| m.len() as usize).ok() != Some(SERVER.len()) {
        std::fs::write(&path, SERVER)?;
    }
    Ok(path)
}

fn run(adb: &Adb, serial: &str, shared: &Arc<Shared>, port_slot: &Mutex<Option<u16>>, events: &UnboundedSender<Event>) -> io::Result<()> {
    let jar = server_file()?;
    let push = adb.run(&["-s", serial, "push", &jar.to_string_lossy(), DEVICE_PATH], Duration::from_secs(60))?;
    if !push.success {
        return Err(io::Error::other(format!("Could not install the mirroring agent: {}", push.message())));
    }
    let scid = rand::random::<u32>() & 0x7fff_ffff;
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let forward = adb.run(&["-s", serial, "forward", &format!("tcp:{port}"), &format!("localabstract:scrcpy_{scid:08x}")], Duration::from_secs(10))?;
    if !forward.success {
        return Err(io::Error::other(forward.message()));
    }
    *port_slot.lock().unwrap() = Some(port);
    if shared.stop.load(Ordering::SeqCst) {
        return Ok(());
    }

    let command = format!(
        "CLASSPATH={DEVICE_PATH} app_process / com.genymobile.scrcpy.Server {SERVER_VERSION} scid={scid:08x} log_level=info \
         audio=false tunnel_forward=true video_codec=h264 max_fps=60 clipboard_autosync=true power_on=true cleanup=true"
    );
    let mut child = process::command(&adb.path)
        .args(["-s", serial, "shell", &command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut log = shared.log.lock().unwrap();
                log.push_str(&String::from_utf8_lossy(&buf[..n]));
                if log.len() > 64 * 1024 {
                    let cut = log.len() - 32 * 1024;
                    let cut = (cut..log.len()).find(|&i| log.is_char_boundary(i)).unwrap_or(0);
                    log.drain(..cut);
                }
            }
        });
    }
    *shared.server.lock().unwrap() = Some(child);

    // The forward accepts at once even before the server listens; the
    // server's first byte says it is there.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut video = loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(child) = shared.server.lock().unwrap().as_mut() {
            if let Ok(Some(_)) = child.try_wait() {
                std::thread::sleep(Duration::from_millis(200));
                let log = shared.log.lock().unwrap().trim().to_owned();
                return Err(io::Error::other(server_error(&log).map_or_else(
                    || if log.is_empty() { "The mirroring agent stopped".to_owned() } else { log.lines().last().unwrap_or("").to_owned() },
                    |e| e.to_owned(),
                )));
            }
        }
        if Instant::now() > deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Timed out connecting to the device"));
        }
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut byte = [0u8; 1];
            if stream.read_exact(&mut byte).is_ok() {
                stream.set_read_timeout(None)?;
                break stream;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let control = TcpStream::connect(("127.0.0.1", port))?;
    control.set_nodelay(true)?;
    *shared.video.lock().unwrap() = Some(video.try_clone()?);
    *shared.control.lock().unwrap() = Some(control.try_clone()?);

    let mut name = [0u8; 64];
    video.read_exact(&mut name)?;
    let name = String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(64)]).into_owned();
    let mut codec = [0u8; 12];
    video.read_exact(&mut codec)?;
    let _ = events.unbounded_send(Event::Connected(name));

    {
        let events = events.clone();
        let mut control = control;
        std::thread::spawn(move || read_device_messages(&mut control, &events));
    }

    let mut decoder = openh264::decoder::Decoder::new().map_err(|e| io::Error::other(e.to_string()))?;
    let mut header = [0u8; 12];
    let mut packet = Vec::new();
    let mut rgba = Vec::new();
    let mut failures = 0;
    loop {
        video.read_exact(&mut header)?;
        let size = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
        if size > 64 << 20 {
            return Err(io::Error::other("Unexpected video packet"));
        }
        packet.resize(size, 0);
        video.read_exact(&mut packet)?;
        match decoder.decode(&packet) {
            Ok(Some(yuv)) => {
                failures = 0;
                use openh264::formats::YUVSource as _;
                let (w, h) = yuv.dimensions();
                rgba.resize(w * h * 4, 0);
                yuv.write_rgba8(&mut rgba);
                let mut bgra = std::mem::take(&mut rgba);
                for px in bgra.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                let fresh = shared.frame.lock().unwrap().replace(Frame { width: w as u32, height: h as u32, bgra }).is_none();
                if fresh && events.unbounded_send(Event::Frame).is_err() {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(_) => {
                failures += 1;
                if failures > 100 {
                    return Err(io::Error::other("Too many invalid video frames"));
                }
            }
        }
    }
}

fn read_device_messages(control: &mut TcpStream, events: &UnboundedSender<Event>) -> io::Result<()> {
    loop {
        let mut kind = [0u8; 1];
        control.read_exact(&mut kind)?;
        match kind[0] {
            0 => {
                let mut len = [0u8; 4];
                control.read_exact(&mut len)?;
                let mut text = vec![0u8; u32::from_be_bytes(len) as usize];
                control.read_exact(&mut text)?;
                let _ = events.unbounded_send(Event::Clipboard(String::from_utf8_lossy(&text).into_owned()));
            }
            1 => {
                let mut sequence = [0u8; 8];
                control.read_exact(&mut sequence)?;
            }
            2 => {
                let mut head = [0u8; 4];
                control.read_exact(&mut head)?;
                let mut data = vec![0u8; u16::from_be_bytes([head[2], head[3]]) as usize];
                control.read_exact(&mut data)?;
            }
            _ => return Err(io::Error::other("Unexpected device message")),
        }
    }
}

fn cleanup(adb: &Adb, serial: &str, shared: &Shared, port: &Mutex<Option<u16>>) {
    if let Some(mut child) = shared.server.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(port) = port.lock().unwrap().take() {
        let _ = adb.run(&["-s", serial, "forward", "--remove", &format!("tcp:{port}")], Duration::from_secs(5));
    }
}

/// `adb exec-out screencap -p`: the screen as PNG.
pub fn screenshot(adb: &Adb, serial: &str) -> io::Result<Vec<u8>> {
    let (ok, png, err) = process::run_bytes(&adb.path, &["-s", serial, "exec-out", "screencap", "-p"], Duration::from_secs(20))?;
    if !ok || !png.starts_with(b"\x89PNG") {
        return Err(io::Error::other(if err.trim().is_empty() { "The device returned no screenshot".to_owned() } else { err.trim().to_owned() }));
    }
    Ok(png)
}

/// A `screenrecord` running on the device until [`Recording::finish`].
pub struct Recording {
    child: Child,
    pub started: Instant,
}

pub const RECORDING_PATH: &str = "/sdcard/junction-screen-record.mp4";

impl Recording {
    pub fn start(adb: &Adb, serial: &str) -> io::Result<Self> {
        let child = process::command(&adb.path)
            .args(["-s", serial, "shell", "screenrecord", "--bit-rate", "8000000", RECORDING_PATH])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Recording { child, started: Instant::now() })
    }

    /// Stops the recording (screenrecord finishes the file on SIGINT) and
    /// copies it to `target`.
    pub fn finish(mut self, adb: &Adb, serial: &str, target: &std::path::Path) -> io::Result<()> {
        let _ = adb.shell(serial, "pkill -INT screenrecord || killall -INT screenrecord");
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait()?.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The file is complete a moment after the process exits.
        std::thread::sleep(Duration::from_millis(500));
        let out = adb.run(&["-s", serial, "pull", RECORDING_PATH, &target.to_string_lossy()], Duration::from_secs(120))?;
        let _ = adb.shell(serial, &format!("rm -f {RECORDING_PATH}"));
        if !out.success {
            return Err(io::Error::other(out.message()));
        }
        Ok(())
    }

    pub fn cancel(mut self, adb: &Adb, serial: &str) {
        let _ = adb.shell(serial, "pkill -INT screenrecord || killall -INT screenrecord");
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = adb.shell(serial, &format!("rm -f {RECORDING_PATH}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_errors() {
        assert_eq!(position(10, 20, 1080, 2400), vec![0, 0, 0, 10, 0, 0, 0, 20, 0x04, 0x38, 0x09, 0x60]);
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(server_error("INFO: Device: x\nERROR: Could not open video stream\n"), Some("Could not open video stream"));
        assert_eq!(server_error("[server] ERROR: Encoding error\n"), Some("Encoding error"));
        assert_eq!(server_error("INFO: ok"), None);
    }
}

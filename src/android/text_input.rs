//! Types text into the device's focused text field without its keyboard or
//! clipboard (a paste shows Android's "pasted from clipboard" toast and the
//! keyboard's clipboard chip). A small dex (assets/text-input, source in
//! tools/text-input) runs as the shell user through `app_process`, connects
//! a UiAutomation like `uiautomator` does and sets the field's text; it takes
//! one base64 line per text on stdin and answers `ok` or `no <why>`.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use super::adb::Adb;
use super::process;

static HELPER: &[u8] = include_bytes!("../../assets/text-input/junction-text-input.jar");
const DEVICE_PATH: &str = "/data/local/tmp/junction-text-input.jar";

/// Matches the helper's command line but not the pkill's own.
const KILL_HELPER: &str = "pkill -f dev.junction.Text[I]nput; pkill -x junction-input";

pub struct TextInput {
    adb: Adb,
    serial: String,
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl TextInput {
    /// Installs and starts the helper; Err says why it can't run.
    pub fn start(adb: &Adb, serial: &str) -> Result<Self, String> {
        let jar = helper_file().map_err(|e| e.to_string())?;
        let push = adb.run(&["-s", serial, "push", &jar.to_string_lossy(), DEVICE_PATH], Duration::from_secs(30)).map_err(|e| e.to_string())?;
        if !push.success {
            return Err(push.message());
        }
        // app_process takes its options after the class directory: before
        // it they go to the VM, which then won't start.
        // Only one UiAutomation may connect: end a helper an earlier session
        // left behind (its adb went away without closing it).
        let _ = adb.shell(serial, KILL_HELPER);
        let mut child = process::command(&adb.path)
            .args(["-s", serial, "shell", &format!("CLASSPATH={DEVICE_PATH} app_process / --nice-name=junction-input dev.junction.TextInput")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            return Err("No pipe to the helper".to_owned());
        };
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line.trim().to_owned()).is_err() {
                    break;
                }
            }
        });
        // The last lines the helper (or the VM) wrote on stderr, to say why
        // it stopped.
        let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        {
            let errors = errors.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { break };
                    let mut errors = errors.lock().unwrap();
                    errors.push(line.trim().to_owned());
                    if errors.len() > 20 {
                        errors.remove(0);
                    }
                }
            });
        }
        let input = TextInput { adb: adb.clone(), serial: serial.to_owned(), child, stdin, lines };
        // app_process plus the UiAutomation connection take a moment.
        let answer = input.answer(Duration::from_secs(20)).map_err(|e| {
            // stderr may lag stdout's end a little.
            std::thread::sleep(Duration::from_millis(300));
            let errors = errors.lock().unwrap();
            let errors: Vec<&str> = errors.iter().map(String::as_str).filter(|l| !l.is_empty()).collect();
            if errors.is_empty() { e } else { format!("{e} {}", errors[errors.len().saturating_sub(5)..].join(" | ")) }
        })?;
        match answer.as_str() {
            "ready" => Ok(input),
            other => {
                let mut why = other.strip_prefix("fail ").unwrap_or(other).to_owned();
                if why.starts_with("Another UI automation") {
                    // Name what holds it, for the notification's details.
                    let holders = adb
                        .shell(serial, "ps -A -o PID,USER,ARGS 2>/dev/null | grep -i -E 'uiautomator|instrument|app_process|junction' | grep -v grep")
                        .unwrap_or_default();
                    let holders: Vec<&str> = holders.lines().map(str::trim).filter(|l| !l.is_empty()).take(5).collect();
                    if !holders.is_empty() {
                        why.push_str(&format!(" Running on the device: {}", holders.join("; ")));
                    }
                }
                Err(why)
            }
        }
    }

    /// Replaces the focused field's selection with `text`.
    pub fn insert(&mut self, text: &str) -> Result<(), String> {
        writeln!(self.stdin, "{}", base64(text.as_bytes())).and_then(|_| self.stdin.flush()).map_err(|_| STOPPED.to_owned())?;
        let answer = self.answer(Duration::from_secs(5))?;
        match answer.as_str() {
            "ok" => Ok(()),
            other => Err(other.strip_prefix("no ").unwrap_or(other).to_owned()),
        }
    }

    /// Whether the helper still runs (a failed insert may have ended it).
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn answer(&self, timeout: Duration) -> Result<String, String> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => Err("The device did not answer in time.".to_owned()),
            Err(RecvTimeoutError::Disconnected) => Err(STOPPED.to_owned()),
        }
    }
}

const STOPPED: &str = "The text input helper on the device stopped.";

impl Drop for TextInput {
    fn drop(&mut self) {
        // Closing stdin ends the helper on the device; the kill ends adb,
        // and the pkill the helper in case adb didn't pass the end on.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let (adb, serial) = (self.adb.clone(), self.serial.clone());
        std::thread::spawn(move || {
            let _ = adb.shell(&serial, KILL_HELPER);
        });
    }
}

/// The helper as a file adb can push, written once per build.
fn helper_file() -> io::Result<PathBuf> {
    // Named by content, so a new build never reuses an old copy.
    let hash = HELPER.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3));
    let path = std::env::temp_dir().join(format!("junction-text-input-{hash:016x}.jar"));
    if std::fs::metadata(&path).map(|m| m.len() as usize) .ok() != Some(HELPER.len()) {
        let partial = path.with_extension("part");
        std::fs::write(&partial, HELPER)?;
        std::fs::rename(&partial, &path)?;
    }
    Ok(path)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("中文".as_bytes()), "5Lit5paH");
    }
}

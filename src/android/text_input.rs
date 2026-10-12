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

pub struct TextInput {
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
        let mut child = process::command(&adb.path)
            .args(["-s", serial, "shell", &format!("CLASSPATH={DEVICE_PATH} app_process / dev.junction.TextInput")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
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
        let input = TextInput { child, stdin, lines };
        // app_process plus the UiAutomation connection take a moment.
        match input.answer(Duration::from_secs(15))?.as_str() {
            "ready" => Ok(input),
            other => Err(other.strip_prefix("fail ").unwrap_or(other).to_owned()),
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
        // Closing stdin ends the helper on the device; the kill ends adb.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The helper as a file adb can push, written once per build.
fn helper_file() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("junction-text-input-{}.jar", HELPER.len()));
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

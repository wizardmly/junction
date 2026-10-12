//! Running the SDK's command-line tools (adb, the emulator) without a
//! console window on Windows and without hanging on a stuck device.

use std::io::{self, Read as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a finished command printed.
#[derive(Debug, Default, Clone)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// stderr, else stdout: what to show when the command failed.
    pub fn message(&self) -> String {
        let text = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        text.trim().to_owned()
    }
}

/// A process that doesn't flash a console window on Windows.
pub fn command(program: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Runs `program args`, killing it when it outlives `timeout`.
pub fn run(program: &Path, args: &[&str], timeout: Duration) -> io::Result<Output> {
    let mut child = command(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    // A daemon the command starts (adb's server) may keep the pipes open
    // after the command exits, so the output is collected with a deadline.
    let reader = |pipe: Option<Box<dyn io::Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut buf = [0u8; 8192];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    text.extend_from_slice(&buf[..n]);
                    // Sent as it arrives, so a pipe held open still yields what was printed.
                    let _ = tx.send(std::mem::take(&mut text));
                }
            }
        });
        rx
    };
    let stdout = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn io::Read + Send>));
    let stderr = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn io::Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the command did not finish in time"));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let collect = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        let mut text = Vec::new();
        let until = Instant::now() + Duration::from_millis(500);
        while let Ok(chunk) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            text.extend(chunk);
        }
        String::from_utf8_lossy(&text).into_owned()
    };
    Ok(Output { success: status.success(), stdout: collect(stdout), stderr: collect(stderr) })
}

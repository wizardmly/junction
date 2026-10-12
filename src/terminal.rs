//! Terminal sessions for the Terminal tool window: a shell on a pseudo
//! terminal (ConPTY on Windows), emulated by alacritty_terminal, and the
//! bytes keys and pastes send to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;

pub use alacritty_terminal::event::Event as TermEvent;

/// Forwards the emulator's events (output arrived, title, bell, exit) to
/// the UI thread.
#[derive(Clone)]
pub struct Listener(futures::channel::mpsc::UnboundedSender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0.unbounded_send(event).ok();
    }
}

/// The emulator's screen size, in cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// A shell the "New Session" dropdown offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellChoice {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

/// The shell a new tab runs, as Android Studio picks it: PowerShell on
/// Windows, the login shell ($SHELL) elsewhere.
pub fn default_shell() -> ShellChoice {
    if cfg!(windows) {
        ShellChoice { name: "Local".into(), program: "powershell.exe".into(), args: Vec::new() }
    } else {
        let program = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
        // macOS terminals start login shells, so the profile runs as in Terminal.app.
        let args = if cfg!(target_os = "macos") { vec!["-l".into()] } else { Vec::new() };
        ShellChoice { name: "Local".into(), program, args }
    }
}

/// The shells installed here, for the New Session dropdown.
pub fn available_shells() -> Vec<ShellChoice> {
    let mut shells = Vec::new();
    let shell = |name: &str, program: &str| ShellChoice { name: name.into(), program: program.into(), args: Vec::new() };
    if cfg!(windows) {
        shells.push(shell("Windows PowerShell", "powershell.exe"));
        shells.push(shell("Command Prompt", "cmd.exe"));
        if which("pwsh.exe").is_some() {
            shells.push(shell("PowerShell", "pwsh.exe"));
        }
        let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let git_bash = Path::new(&program_files).join(r"Git\bin\bash.exe");
        if git_bash.exists() {
            shells.push(ShellChoice { name: "Git Bash".into(), program: git_bash.to_string_lossy().into_owned(), args: vec!["--login".into(), "-i".into()] });
        }
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        if Path::new(&root).join(r"System32\wsl.exe").exists() {
            shells.push(shell("WSL", "wsl.exe"));
        }
    } else {
        let listed = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for line in listed.lines().map(str::trim).filter(|l| l.starts_with('/')) {
            let name = line.rsplit('/').next().unwrap_or(line).to_owned();
            if Path::new(line).exists() && seen.insert(name.clone()) {
                shells.push(shell(&name, line));
            }
        }
        if shells.is_empty() {
            let default = default_shell();
            shells.push(ShellChoice { name: default.program.rsplit('/').next().unwrap_or("sh").into(), ..default });
        }
    }
    shells
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(program)).find(|p| p.is_file())
}

/// A running shell and its emulated screen.
pub struct Session {
    pub term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    size: GridSize,
}

impl Session {
    /// Starts `shell` in `cwd` on a terminal of `size`; the emulator's
    /// events arrive on the returned receiver.
    pub fn spawn(shell: &ShellChoice, cwd: Option<&Path>, size: GridSize) -> anyhow::Result<(Self, futures::channel::mpsc::UnboundedReceiver<Event>)> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let listener = Listener(tx);
        let mut env = HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        env.insert("TERMINAL_EMULATOR".to_owned(), "JetBrains-JediTerm".to_owned());
        env.insert("TERM_PROGRAM".to_owned(), "Junction".to_owned());
        // A UTF-8 locale when none is set (an app started from a desktop
        // entry may have none), so non-ASCII output isn't escaped.
        if cfg!(unix) && ["LC_ALL", "LC_CTYPE", "LANG"].iter().all(|v| std::env::var(v).map_or(true, |s| s.is_empty())) {
            env.insert("LANG".to_owned(), if cfg!(target_os = "macos") { "en_US.UTF-8" } else { "C.UTF-8" }.to_owned());
        }
        let options = tty::Options {
            shell: Some(tty::Shell::new(shell.program.clone(), shell.args.clone())),
            working_directory: cwd.filter(|d| d.is_dir()).map(Path::to_path_buf),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, window_size(size), 0)?;
        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok((Self { term, sender, size }, rx))
    }

    pub fn write(&self, bytes: impl Into<std::borrow::Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            self.sender.send(Msg::Input(bytes)).ok();
        }
    }

    /// Pastes text as a terminal does: bracketed when the program asked for
    /// it, line breaks sent as Enter.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.write(paste_bytes(text, bracketed));
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size || size.columns == 0 || size.lines == 0 {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        self.sender.send(Msg::Resize(window_size(size))).ok();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.sender.send(Msg::Shutdown).ok();
    }
}

fn window_size(size: GridSize) -> WindowSize {
    WindowSize { num_lines: size.lines as u16, num_cols: size.columns as u16, cell_width: 8, cell_height: 16 }
}

pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // The end marker inside the text would end the paste early.
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

/// A key press as the bytes a terminal sends for it, or `None` for keys
/// that type text (they arrive through the input handler, so IMEs work).
/// On macOS Option+key types its character, as Option isn't Meta there.
pub fn key_bytes(key: &str, ctrl: bool, alt: bool, shift: bool, key_char: Option<&str>, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modifier = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let csi_mod = |tail: char| -> Vec<u8> {
        if modifier > 1 { format!("\x1b[1;{modifier}{tail}").into_bytes() } else if app_cursor { format!("\x1bO{tail}").into_bytes() } else { format!("\x1b[{tail}").into_bytes() }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier > 1 { format!("\x1b[{code};{modifier}~").into_bytes() } else { format!("\x1b[{code}~").into_bytes() }
    };
    let esc_if_alt = |bytes: &[u8]| -> Vec<u8> {
        let mut out = if alt { vec![0x1b] } else { Vec::new() };
        out.extend_from_slice(bytes);
        out
    };
    let bytes = match key {
        "up" => csi_mod('A'),
        "down" => csi_mod('B'),
        "right" => csi_mod('C'),
        "left" => csi_mod('D'),
        "home" => csi_mod('H'),
        "end" => csi_mod('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => csi_f(modifier, 'P'),
        "f2" => csi_f(modifier, 'Q'),
        "f3" => csi_f(modifier, 'R'),
        "f4" => csi_f(modifier, 'S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" => esc_if_alt(b"\r"),
        "tab" if shift => b"\x1b[Z".to_vec(),
        "tab" => esc_if_alt(b"\t"),
        "escape" => esc_if_alt(b"\x1b"),
        "backspace" if ctrl => esc_if_alt(b"\x08"),
        "backspace" => esc_if_alt(b"\x7f"),
        "space" if ctrl => vec![0],
        _ if ctrl => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else { return None };
            let code = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '8' | '?' => 0x7f,
                _ => return None,
            };
            esc_if_alt(&[code])
        }
        // Alt+key: ESC then the key, as xterm's "meta sends escape".
        _ if alt && !cfg!(target_os = "macos") => {
            let text = key_char.filter(|t| !t.is_empty()).map(str::to_owned).unwrap_or_else(|| if shift { key.to_uppercase() } else { key.to_owned() });
            if text.chars().count() != 1 {
                return None;
            }
            esc_if_alt(text.as_bytes())
        }
        _ => return None,
    };
    Some(bytes)
}

fn csi_f(modifier: u8, tail: char) -> Vec<u8> {
    if modifier > 1 { format!("\x1b[1;{modifier}{tail}").into_bytes() } else { format!("\x1bO{tail}").into_bytes() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
        key_bytes(key, ctrl, alt, shift, None, TermMode::empty())
    }

    #[test]
    fn control_and_special_keys() {
        assert_eq!(keys("c", true, false, false), Some(vec![3]));
        assert_eq!(keys("e", true, false, false), Some(vec![5]));
        assert_eq!(keys("enter", false, false, false), Some(b"\r".to_vec()));
        assert_eq!(keys("backspace", false, false, false), Some(vec![0x7f]));
        assert_eq!(keys("up", false, false, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(keys("left", true, false, false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(keys("delete", false, false, false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(keys("tab", false, false, true), Some(b"\x1b[Z".to_vec()));
        assert_eq!(keys("f5", false, false, false), Some(b"\x1b[15~".to_vec()));
        // Plain typing goes through the input handler.
        assert_eq!(keys("a", false, false, false), None);
        assert_eq!(key_bytes("up", false, false, false, None, TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn pastes_send_enter_and_respect_bracketed_mode() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(paste_bytes("ls\r\n", true), b"\x1b[200~ls\r\x1b[201~".to_vec());
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_runs_in_the_given_directory() {
        use alacritty_terminal::grid::Dimensions as _;
        let dir = std::env::temp_dir();
        let shell = ShellChoice { name: "sh".into(), program: "/bin/sh".into(), args: Vec::new() };
        let (session, _rx) = Session::spawn(&shell, Some(&dir), GridSize { columns: 80, lines: 24 }).unwrap();
        session.write(b"pwd\r".to_vec());
        let want = dir.canonicalize().unwrap().to_string_lossy().into_owned();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let screen: String = {
                let term = session.term.lock();
                let grid = term.grid();
                (0..grid.screen_lines())
                    .map(|l| (0..grid.columns()).map(|c| grid[alacritty_terminal::index::Line(l as i32)][alacritty_terminal::index::Column(c)].c).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if screen.lines().any(|l| l.trim_end().ends_with(want.as_str()) && !l.contains("pwd")) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "pwd output not seen:\n{screen}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

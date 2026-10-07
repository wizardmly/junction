use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use super::RepositoryState;

/// One executed command, as listed in the Git tool window's Console tab.
#[derive(Clone, Debug)]
pub struct ConsoleEntry {
    pub command_line: String,
    pub output: String,
    pub success: bool,
    pub duration: Duration,
}

/// Shared record of every git command the application ran.
#[derive(Clone, Default)]
pub struct GitConsole {
    entries: Arc<Mutex<Vec<ConsoleEntry>>>,
}

impl GitConsole {
    pub fn entries(&self) -> Vec<ConsoleEntry> {
        self.entries.lock().unwrap().clone()
    }

    fn push(&self, entry: ConsoleEntry) {
        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);
        // Keep the console bounded the way IntelliJ trims its VCS console.
        let overflow = entries.len().saturating_sub(1000);
        entries.drain(..overflow);
    }
}

/// A working tree plus the git executable used to operate on it.
#[derive(Clone)]
pub struct Repository {
    root: PathBuf,
    git_dir: PathBuf,
    executable: PathBuf,
    console: GitConsole,
}

/// A git process that, on Windows, doesn't flash a console window.
fn git_command(executable: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

impl Repository {
    /// Finds the repository containing `path`.
    pub fn discover(path: &Path, console: GitConsole) -> Result<Self> {
        let executable = PathBuf::from("git");
        let output = git_command(&executable)
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--show-toplevel", "--absolute-git-dir"])
            .output()
            .context("failed to run git; is it installed and on PATH?")?;
        if !output.status.success() {
            bail!(
                "{} is not inside a git repository: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8(output.stdout)?;
        let mut lines = stdout.lines();
        let root = PathBuf::from(lines.next().context("missing toplevel")?);
        let git_dir = PathBuf::from(lines.next().context("missing git dir")?);
        Ok(Self { root, git_dir, executable, console })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Runs git and returns stdout, failing with stderr when git fails.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.run_with_input(args, None)
    }

    /// Runs git with `input` written to its stdin.
    pub fn run_with_input<I, S>(&self, args: I, input: Option<&str>) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.run_full(args, input, &[])
    }

    /// Runs git with extra environment variables (e.g. a sequence editor).
    pub fn run_with_env<I, S>(&self, args: I, env: &[(&str, &str)]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.run_full(args, None, env)
    }

    fn run_full<I, S>(&self, args: I, input: Option<&str>, env: &[(&str, &str)]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        run_in(&self.executable, &self.root, &self.console, args, input, env)
    }

    pub fn state(&self) -> RepositoryState {
        let exists = |name: &str| self.git_dir.join(name).exists();
        if exists("rebase-merge") || exists("rebase-apply") {
            RepositoryState::Rebasing
        } else if exists("MERGE_HEAD") {
            RepositoryState::Merging
        } else if exists("CHERRY_PICK_HEAD") {
            RepositoryState::CherryPicking
        } else if exists("REVERT_HEAD") {
            RepositoryState::Reverting
        } else {
            RepositoryState::Normal
        }
    }

    /// `user.name` and `user.email`, used to highlight "my" commits.
    pub fn current_user(&self) -> (Option<String>, Option<String>) {
        let get = |key: &str| {
            self.run(["config", "--get", key])
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        (get("user.name"), get("user.email"))
    }
}


/// Runs git in `cwd` (a repository, or the parent folder for `clone` / `init`),
/// recording it in the console.
pub fn run_in<I, S>(executable: &Path, cwd: &Path, console: &GitConsole, args: I, input: Option<&str>, env: &[(&str, &str)]) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|arg| arg.as_ref().to_owned()).collect();
    let started = Instant::now();
    let mut child = git_command(executable)
        .current_dir(cwd)
        .envs(crate::askpass::git_env())
        // Stable, parseable output regardless of the user's config.
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false", "-c", "log.showSignature=false"])
        .args(&args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // Never block on an editor (rebase/cherry-pick --continue, merge commits).
        .env("GIT_EDITOR", "true")
        .envs(env.iter().copied())
        .env("LC_ALL", "C")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let success = output.status.success();

    // The console shows what a user would have typed, not our -c overrides.
    let command_line = format!("git {}", args.iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(" "));
    let mut console_output = String::new();
    if !stdout.is_empty() && stdout.len() < 4096 && !stdout.contains('\0') {
        console_output.push_str(&stdout);
    }
    console_output.push_str(&stderr);
    console.push(ConsoleEntry {
        command_line: command_line.clone(),
        output: console_output.trim_end().to_owned(),
        success,
        duration: started.elapsed(),
    });

    if !success {
        bail!("{command_line} failed: {}", stderr.trim());
    }
    Ok(stdout)
}

fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"') {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

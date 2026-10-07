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

/// Settings › Git: path to the git executable ("git" = found on PATH) and
/// "Use credential helper". Both apply to every command run afterwards.
static EXECUTABLE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
static NO_CREDENTIAL_HELPER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_executable(path: &str) {
    let path = path.trim();
    *EXECUTABLE.write().unwrap() = (!path.is_empty()).then(|| PathBuf::from(path));
}

/// The git to run: the one set in Settings, else `git` on PATH, else a
/// git found in a usual install location (Git for Windows, Scoop, GitHub
/// Desktop, Visual Studio, Homebrew, …).
pub fn executable() -> PathBuf {
    if let Some(path) = EXECUTABLE.read().unwrap().clone() {
        return path;
    }
    detected_executable().unwrap_or_else(|| PathBuf::from("git"))
}

static DETECTED: std::sync::RwLock<Option<Option<PathBuf>>> = std::sync::RwLock::new(None);

/// The git an empty "Path to Git executable" uses, if one is installed.
pub fn detected_executable() -> Option<PathBuf> {
    if let Some(found) = DETECTED.read().unwrap().clone() {
        return found;
    }
    let found = detect_executable();
    *DETECTED.write().unwrap() = Some(found.clone());
    found
}

/// Looks for git again (after the user installed it). PATH comes from the
/// environment this process started with, so the install locations matter.
pub fn redetect_executable() {
    *DETECTED.write().unwrap() = None;
}

fn detect_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    install_locations().into_iter().find(|p| p.is_file())
}

fn install_locations() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    if cfg!(windows) {
        let mut roots: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|k| env(k)).collect();
        for drive in ['C', 'D', 'E', 'F'] {
            roots.push(PathBuf::from(format!("{drive}:\\Program Files")));
            out.push(PathBuf::from(format!("{drive}:\\Git\\cmd\\git.exe")));
        }
        for root in &roots {
            out.push(root.join("Git").join("cmd").join("git.exe"));
        }
        if let Some(local) = env("LOCALAPPDATA") {
            out.push(local.join("Programs").join("Git").join("cmd").join("git.exe"));
            // GitHub Desktop bundles a git per version: app-3.4.1\resources\app\git.
            if let Ok(dirs) = std::fs::read_dir(local.join("GitHubDesktop")) {
                let mut versions: Vec<PathBuf> = dirs.flatten().map(|d| d.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("app-"))).collect();
                versions.sort();
                for v in versions.into_iter().rev() {
                    out.push(v.join("resources").join("app").join("git").join("cmd").join("git.exe"));
                }
            }
        }
        if let Some(home) = env("USERPROFILE") {
            out.push(home.join("scoop").join("apps").join("git").join("current").join("cmd").join("git.exe"));
        }
        // Visual Studio's Team Explorer git.
        for root in &roots {
            let Ok(years) = std::fs::read_dir(root.join("Microsoft Visual Studio")) else { continue };
            for year in years.flatten() {
                let Ok(editions) = std::fs::read_dir(year.path()) else { continue };
                for edition in editions.flatten() {
                    out.push(edition.path().join(r"Common7\IDE\CommonExtensions\Microsoft\TeamFoundation\Team Explorer\Git\cmd\git.exe"));
                }
            }
        }
    } else {
        for dir in ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin", "/opt/local/bin"] {
            out.push(Path::new(dir).join("git"));
        }
    }
    out
}

/// Why a folder couldn't be opened, when it is something the user can fix.
#[derive(Debug)]
pub enum OpenError {
    /// git isn't installed (or not where Settings says).
    GitMissing(PathBuf),
    /// git refuses a repository owned by another user (`safe.directory`).
    Unsafe(PathBuf),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::GitMissing(git) => write!(f, "Git is not installed, or cannot be run from {}.", git.display()),
            OpenError::Unsafe(path) => write!(f, "Git refuses to open {} because the folder is owned by another user.", path.display()),
        }
    }
}

impl std::error::Error for OpenError {}

/// Adds a folder to git's global `safe.directory` list, as IntelliJ's
/// "Trust directory" does.
pub fn trust_directory(path: &Path) -> Result<()> {
    let value = path.to_string_lossy().replace('\\', "/");
    let output = git_command(&executable()).args(["config", "--global", "--add", "safe.directory", &value]).output()?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

/// Windows folder pickers can hand back `\\?\` paths, which git rejects.
fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

pub fn set_use_credential_helper(enabled: bool) {
    NO_CREDENTIAL_HELPER.store(!enabled, std::sync::atomic::Ordering::Relaxed);
}

/// The Test button: `git --version` with `path` (empty = PATH lookup).
pub fn executable_version(path: &str) -> Result<String> {
    let path = if path.trim().is_empty() { detected_executable().unwrap_or_else(|| PathBuf::from("git")) } else { PathBuf::from(path.trim()) };
    let output = git_command(&path).arg("--version").output().with_context(|| format!("cannot run {}", path.display()))?;
    if !output.status.success() {
        bail!("{} --version failed", path.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// A process of the configured git that doesn't flash a console window
/// on Windows.
pub fn git_process() -> Command {
    git_command(&executable())
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
        let executable = executable();
        let path = &plain_path(path);
        let output = match git_command(&executable).arg("-C").arg(path).args(["rev-parse", "--show-toplevel", "--absolute-git-dir"]).output() {
            Ok(output) => output,
            Err(_) => return Err(OpenError::GitMissing(executable).into()),
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("dubious ownership") || stderr.contains("safe.directory") {
                // git names the repository root in its message: 'D:/x' is owned by …
                let root = stderr.split('\'').nth(1).map(PathBuf::from).unwrap_or_else(|| path.to_path_buf());
                return Err(OpenError::Unsafe(root).into());
            }
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

    /// A repository nested in this one (a submodule), sharing the console.
    pub fn nested(&self, path: &str) -> Result<Repository> {
        Repository::discover(&self.root.join(path), self.console.clone())
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

    pub(crate) fn run_full<I, S>(&self, args: I, input: Option<&str>, env: &[(&str, &str)]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        run_in(&self.executable, &self.root, &self.console, args, input, env)
    }

    /// Runs git and returns raw stdout.
    pub fn run_bytes<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        run_in_bytes(&self.executable, &self.root, &self.console, args, None, &[])
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
    run_in_bytes(executable, cwd, console, args, input, env).map(|stdout| String::from_utf8_lossy(&stdout).into_owned())
}

/// [`run_in`] for binary output, e.g. `cat-file blob` of an image.
pub fn run_in_bytes<I, S>(executable: &Path, cwd: &Path, console: &GitConsole, args: I, input: Option<&str>, env: &[(&str, &str)]) -> Result<Vec<u8>>
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
        // With the credential helper off, credentials come from our askpass prompt only.
        .args(if NO_CREDENTIAL_HELPER.load(std::sync::atomic::Ordering::Relaxed) { &["-c", "credential.helper="][..] } else { &[][..] })
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
    let stdout = output.stdout;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let success = output.status.success();

    // The console shows what a user would have typed, not our -c overrides.
    let command_line = format!("git {}", args.iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(" "));
    let mut console_output = String::new();
    if !stdout.is_empty() && stdout.len() < 4096 && !stdout.contains(&0) {
        console_output.push_str(&String::from_utf8_lossy(&stdout));
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

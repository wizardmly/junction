//! User settings (Settings › Version Control › Git), saved as `key=value`
//! lines in the platform config directory.

use std::path::PathBuf;

use gpui_kit::{App, Global};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateMethod {
    #[default]
    Merge,
    Rebase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub dark: bool,
    /// "Enable staging area": the Commit tool window shows Staged and
    /// Unstaged trees instead of changelists with checkboxes.
    pub staging_area: bool,
    pub update_method: UpdateMethod,
    /// "Auto-update if push of the current branch was rejected".
    pub auto_update_on_push_rejected: bool,
    /// "Warn if CRLF line separators are about to be committed".
    pub warn_crlf: bool,
    /// Commit message right margin, also used for the first-line hint.
    pub commit_subject_limit: usize,
    /// "Warn when committing in detached HEAD or during rebase".
    pub warn_detached_head: bool,
    /// "Protected branches": force push is refused for these (comma-separated).
    pub protected_branches: String,
    /// Commit options remembered between commits.
    pub sign_off: bool,
    pub run_hooks: bool,
    pub cleanup_message: bool,
    /// "Update branch info": fetch every N minutes so the branches popup and
    /// the Log show incoming commits; 0 turns it off.
    pub fetch_interval_minutes: u32,
    /// "Path to Git executable"; empty means `git` on PATH.
    pub git_executable: String,
    /// "Use credential helper".
    pub use_credential_helper: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            staging_area: false,
            update_method: UpdateMethod::Merge,
            auto_update_on_push_rejected: false,
            warn_crlf: true,
            commit_subject_limit: 72,
            warn_detached_head: true,
            protected_branches: "master, main".into(),
            sign_off: false,
            run_hooks: true,
            cleanup_message: false,
            fetch_interval_minutes: 10,
            git_executable: String::new(),
            use_credential_helper: true,
        }
    }
}

impl Global for Settings {}

pub fn config_dir() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(dir.join("GitGlass"))
}

fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("settings.conf"))
}

const HISTORY_SEPARATOR: &str = "\n\u{1e}\n";
const HISTORY_LIMIT: usize = 20;

/// Recent commit messages, newest first (Commit Message History, Ctrl+M).
pub fn message_history() -> Vec<String> {
    let Some(path) = config_dir().map(|d| d.join("commit-messages.txt")) else { return Vec::new() };
    std::fs::read_to_string(path)
        .map(|text| text.split(HISTORY_SEPARATOR).filter(|m| !m.trim().is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn remember_message(message: &str) {
    let Some(dir) = config_dir() else { return };
    let message = message.trim();
    if message.is_empty() {
        return;
    }
    let mut history = message_history();
    history.retain(|m| m != message);
    history.insert(0, message.to_owned());
    history.truncate(HISTORY_LIMIT);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("commit-messages.txt"), history.join(HISTORY_SEPARATOR));
}

/// Recently opened repositories, newest first (the Welcome screen and project widget).
pub fn recent_projects() -> Vec<PathBuf> {
    let Some(path) = config_dir().map(|d| d.join("recent-projects.txt")) else { return Vec::new() };
    std::fs::read_to_string(path)
        .map(|text| text.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

pub fn remember_project(path: &std::path::Path) {
    let Some(dir) = config_dir() else { return };
    let mut recent = recent_projects();
    recent.retain(|p| p != path);
    recent.insert(0, path.to_path_buf());
    recent.truncate(HISTORY_LIMIT);
    let text: Vec<String> = recent.iter().map(|p| p.display().to_string()).collect();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("recent-projects.txt"), text.join("\n"));
}

pub fn forget_project(path: &std::path::Path) {
    let Some(dir) = config_dir() else { return };
    let mut recent = recent_projects();
    recent.retain(|p| p != path);
    let text: Vec<String> = recent.iter().map(|p| p.display().to_string()).collect();
    let _ = std::fs::write(dir.join("recent-projects.txt"), text.join("\n"));
}

impl Settings {
    pub fn is_protected(&self, branch: &str) -> bool {
        self.protected_branches.split(',').map(str::trim).any(|p| !p.is_empty() && (p == branch || glob(p, branch)))
    }
}

/// `*` wildcards, as IntelliJ's protected-branch patterns allow.
fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, rest)) => {
            text.starts_with(prefix) && (0..=text.len() - prefix.len()).any(|i| glob(rest, &text[prefix.len() + i..]))
        }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let flag = value == "true";
            match key {
                "theme" => settings.dark = value != "light",
                "staging_area" => settings.staging_area = flag,
                "update_method" => {
                    settings.update_method = if value == "rebase" { UpdateMethod::Rebase } else { UpdateMethod::Merge }
                }
                "auto_update_on_push_rejected" => settings.auto_update_on_push_rejected = flag,
                "warn_crlf" => settings.warn_crlf = flag,
                "warn_detached_head" => settings.warn_detached_head = flag,
                "protected_branches" => settings.protected_branches = value.to_owned(),
                "sign_off" => settings.sign_off = flag,
                "run_hooks" => settings.run_hooks = flag,
                "cleanup_message" => settings.cleanup_message = flag,
                "git_executable" => settings.git_executable = value.to_owned(),
                "use_credential_helper" => settings.use_credential_helper = flag,
                "fetch_interval_minutes" => {
                    if let Ok(n) = value.parse() {
                        settings.fetch_interval_minutes = n;
                    }
                }
                "commit_subject_limit" => {
                    if let Ok(n) = value.parse() {
                        settings.commit_subject_limit = n;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn serialize(&self) -> String {
        format!(
            "theme={}\nstaging_area={}\nupdate_method={}\nauto_update_on_push_rejected={}\nwarn_crlf={}\ncommit_subject_limit={}\n\
             warn_detached_head={}\nprotected_branches={}\nsign_off={}\nrun_hooks={}\ncleanup_message={}\nfetch_interval_minutes={}\ngit_executable={}\nuse_credential_helper={}\n",
            if self.dark { "dark" } else { "light" },
            self.staging_area,
            match self.update_method {
                UpdateMethod::Merge => "merge",
                UpdateMethod::Rebase => "rebase",
            },
            self.auto_update_on_push_rejected,
            self.warn_crlf,
            self.commit_subject_limit,
            self.warn_detached_head,
            self.protected_branches,
            self.sign_off,
            self.run_hooks,
            self.cleanup_message,
            self.fetch_interval_minutes,
            self.git_executable,
            self.use_credential_helper,
        )
    }

    pub fn load() -> Self {
        config_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.serialize());
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Changes settings, saves them, and re-renders every window.
    /// Hands the Git settings to the command runner.
    pub fn apply_git(&self) {
        crate::git::set_executable(&self.git_executable);
        crate::git::set_use_credential_helper(self.use_credential_helper);
    }

    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let mut settings = cx.global::<Self>().clone();
        f(&mut settings);
        settings.save();
        settings.apply_git();
        cx.set_global(settings);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let settings = Settings { dark: false, staging_area: true, update_method: UpdateMethod::Rebase, ..Default::default() };
        assert_eq!(Settings::parse(&settings.serialize()), settings);
        let custom = Settings { protected_branches: "release/*, main".into(), sign_off: true, run_hooks: false, ..Default::default() };
        assert_eq!(Settings::parse(&custom.serialize()), custom);
    }

    #[test]
    fn protected_branch_patterns() {
        let settings = Settings { protected_branches: "main, release/*".into(), ..Default::default() };
        assert!(settings.is_protected("main"));
        assert!(settings.is_protected("release/1.2"));
        assert!(!settings.is_protected("feature/x"));
    }
}

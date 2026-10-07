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
        }
    }
}

impl Global for Settings {}

fn config_path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(dir.join("GitGlass").join("settings.conf"))
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
            "theme={}\nstaging_area={}\nupdate_method={}\nauto_update_on_push_rejected={}\nwarn_crlf={}\ncommit_subject_limit={}\n",
            if self.dark { "dark" } else { "light" },
            self.staging_area,
            match self.update_method {
                UpdateMethod::Merge => "merge",
                UpdateMethod::Rebase => "rebase",
            },
            self.auto_update_on_push_rejected,
            self.warn_crlf,
            self.commit_subject_limit,
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
    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let mut settings = cx.global::<Self>().clone();
        f(&mut settings);
        settings.save();
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
    }
}

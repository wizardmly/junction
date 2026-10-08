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
    /// Appearance › Theme: "Sync with OS" (then `dark` follows the system).
    pub theme_follows_system: bool,
    /// Appearance › "Compact mode": smaller toolbars, stripes and rows.
    pub compact: bool,
    /// Tool window layout, `name:side` in stripe order (see ui::tool_windows).
    pub tool_windows: String,
    /// Left, right and bottom tool window sizes.
    pub tool_window_sizes: [u32; 3],
    /// "Enable staging area": the Commit tool window shows Staged and
    /// Unstaged trees instead of changelists with checkboxes.
    pub staging_area: bool,
    pub update_method: UpdateMethod,
    /// "Clean working tree using": stash (default) or shelve.
    pub update_shelve: bool,
    /// Multi-root projects: "Execute branch operations on all roots".
    pub sync_branches: bool,
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
    /// The Log's View Options, remembered between sessions.
    pub log: LogSettings,
    /// The diff and merge viewers' gear menu.
    pub diff: DiffSettings,
    /// The Project tool window's ⋮ options.
    pub project: ProjectSettings,
    /// Commit tool window › View Options.
    pub commit_group_by_directory: bool,
    pub commit_group_by_module: bool,
    pub commit_group_by_repository: bool,
    pub commit_show_ignored: bool,
    /// Languages & Frameworks: ask language servers first for navigation.
    pub use_language_servers: bool,
    /// Per-language server command overrides (`rust` → `rust-analyzer`);
    /// "off" disables that language's server.
    pub language_servers: std::collections::BTreeMap<String, String>,
}

/// The Log's View Options: columns, references, highlighting and sorting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSettings {
    pub show_author: bool,
    pub show_date: bool,
    pub show_hash: bool,
    /// "5 minutes ago" instead of "Today 10:15".
    pub relative_dates: bool,
    /// One reference label per row plus a count.
    pub compact_refs: bool,
    /// Reference labels before the subject (else after it, right-aligned).
    pub refs_on_left: bool,
    pub highlight_mine: bool,
    pub highlight_merges: bool,
    pub highlight_current_branch: bool,
    pub highlight_not_merged: bool,
    /// `--date-order` instead of IntelliSort's topological order.
    pub sort_by_date: bool,
    /// The changed files of the selected commit: Group By › Directory / Module.
    pub changes_by_directory: bool,
    pub changes_by_module: bool,
    /// Author, date and hash column widths (dragged at their left edge).
    pub columns: [u32; 3],
}

pub const TOOL_WINDOW_SIZES: [u32; 3] = [340, 340, 380];

pub const LOG_COLUMNS: [u32; 3] = [150, 140, 76];

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            show_author: true,
            show_date: true,
            show_hash: false,
            relative_dates: false,
            compact_refs: false,
            refs_on_left: true,
            highlight_mine: true,
            highlight_merges: true,
            highlight_current_branch: false,
            highlight_not_merged: false,
            columns: LOG_COLUMNS,
            sort_by_date: false,
            changes_by_directory: true,
            changes_by_module: false,
        }
    }
}

/// How the Project tool window orders siblings (⋮ › Sort By).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProjectSort {
    #[default]
    Name,
    Type,
    Modified,
}

/// The Project tool window's ⋮ menu: Behavior, Appearance and Sort By.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Behavior › Always Select Opened File.
    pub autoscroll_from_source: bool,
    /// Behavior › Open Files with Single Click.
    pub single_click: bool,
    /// Behavior › Enable Preview Tab: a single click shows the file in one
    /// reused, italic tab until it is edited or opened for real.
    pub preview_tab: bool,
    /// Appearance › Compact Middle Packages.
    pub compact_middle: bool,
    /// Appearance › Show Excluded Files.
    pub show_excluded: bool,
    /// Sort By › Folders Always on Top.
    pub folders_on_top: bool,
    pub sort: ProjectSort,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { autoscroll_from_source: false, single_click: false, preview_tab: false, compact_middle: true, show_excluded: true, folders_on_top: true, sort: ProjectSort::Name }
    }
}

impl ProjectSettings {
    fn fields(&mut self) -> [(&'static str, &mut bool); 6] {
        [
            ("project_autoscroll_from_source", &mut self.autoscroll_from_source),
            ("project_single_click", &mut self.single_click),
            ("project_preview_tab", &mut self.preview_tab),
            ("project_compact_middle", &mut self.compact_middle),
            ("project_show_excluded", &mut self.show_excluded),
            ("project_folders_on_top", &mut self.folders_on_top),
        ]
    }
}

/// The diff and merge viewers' gear menu, shared by both as in IntelliJ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSettings {
    pub show_line_numbers: bool,
    pub show_whitespaces: bool,
    pub show_indent_guides: bool,
    /// Side-by-side: pad change blocks so both sides stay level.
    pub align_changes: bool,
    /// Long lines wrap at the pane's edge instead of scrolling sideways.
    pub soft_wrap: bool,
    /// Lines kept around changes when unchanged fragments are collapsed.
    pub context_lines: usize,
}

impl Default for DiffSettings {
    fn default() -> Self {
        Self { show_line_numbers: true, show_whitespaces: false, show_indent_guides: true, align_changes: false, soft_wrap: false, context_lines: 4 }
    }
}

impl DiffSettings {
    fn fields(&mut self) -> [(&'static str, &mut bool); 5] {
        [
            ("diff_soft_wrap", &mut self.soft_wrap),
            ("diff_show_line_numbers", &mut self.show_line_numbers),
            ("diff_show_whitespaces", &mut self.show_whitespaces),
            ("diff_show_indent_guides", &mut self.show_indent_guides),
            ("diff_align_changes", &mut self.align_changes),
        ]
    }
}

impl LogSettings {
    fn fields(&mut self) -> [(&'static str, &mut bool); 13] {
        [
            ("log_changes_by_directory", &mut self.changes_by_directory),
            ("log_changes_by_module", &mut self.changes_by_module),
            ("log_show_author", &mut self.show_author),
            ("log_show_date", &mut self.show_date),
            ("log_show_hash", &mut self.show_hash),
            ("log_relative_dates", &mut self.relative_dates),
            ("log_compact_refs", &mut self.compact_refs),
            ("log_refs_on_left", &mut self.refs_on_left),
            ("log_highlight_mine", &mut self.highlight_mine),
            ("log_highlight_merges", &mut self.highlight_merges),
            ("log_highlight_current_branch", &mut self.highlight_current_branch),
            ("log_highlight_not_merged", &mut self.highlight_not_merged),
            ("log_sort_by_date", &mut self.sort_by_date),
        ]
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            theme_follows_system: false,
            compact: false,
            tool_windows: String::new(),
            tool_window_sizes: TOOL_WINDOW_SIZES,
            staging_area: false,
            update_method: UpdateMethod::Merge,
            update_shelve: false,
            sync_branches: true,
            use_language_servers: true,
            language_servers: Default::default(),
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
            log: LogSettings::default(),
            diff: DiffSettings::default(),
            project: ProjectSettings::default(),
            commit_group_by_directory: true,
            commit_group_by_module: false,
            commit_group_by_repository: false,
            commit_show_ignored: false,
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
    let current = dir.join("Junction");
    // The app was called GitGlass before: carry its settings and accounts over once.
    let legacy = dir.join("GitGlass");
    if !current.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &current);
    }
    Some(current)
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
                "theme" => {
                    settings.dark = value != "light";
                    settings.theme_follows_system = value == "system";
                }
                "theme_dark" => settings.dark = flag,
                "compact_mode" => settings.compact = flag,
                "tool_windows" => settings.tool_windows = value.to_owned(),
                "tool_window_sizes" => {
                    let sizes: Vec<u32> = value.split(',').filter_map(|w| w.trim().parse().ok()).collect();
                    if let [l, r, b] = sizes[..] {
                        settings.tool_window_sizes = [l, r, b].map(|w| w.clamp(120, 2000));
                    }
                }
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
                "log_columns" => {
                    let widths: Vec<u32> = value.split(',').filter_map(|w| w.trim().parse().ok()).collect();
                    if let [a, d, h] = widths[..] {
                        settings.log.columns = [a, d, h].map(|w| w.clamp(30, 800));
                    }
                }
                key if key.starts_with("log_") => {
                    if let Some((_, field)) = settings.log.fields().into_iter().find(|(k, _)| *k == key) {
                        *field = flag;
                    }
                }
                "diff_context_lines" => {
                    if let Ok(n) = value.parse() {
                        settings.diff.context_lines = n;
                    }
                }
                key if key.starts_with("diff_") => {
                    if let Some((_, field)) = settings.diff.fields().into_iter().find(|(k, _)| *k == key) {
                        *field = flag;
                    }
                }
                "project_sort" => {
                    settings.project.sort = match value {
                        "type" => ProjectSort::Type,
                        "modified" => ProjectSort::Modified,
                        _ => ProjectSort::Name,
                    }
                }
                key if key.starts_with("project_") => {
                    if let Some((_, field)) = settings.project.fields().into_iter().find(|(k, _)| *k == key) {
                        *field = flag;
                    }
                }
                "commit_group_by_directory" => settings.commit_group_by_directory = flag,
                "commit_group_by_module" => settings.commit_group_by_module = flag,
                "commit_group_by_repository" => settings.commit_group_by_repository = flag,
                "commit_show_ignored" => settings.commit_show_ignored = flag,
                "update_clean" => settings.update_shelve = value == "shelve",
                "sync_branches" => settings.sync_branches = flag,
                "use_language_servers" => settings.use_language_servers = flag,
                key if key.starts_with("lsp_") => {
                    if !value.is_empty() {
                        settings.language_servers.insert(key["lsp_".len()..].to_owned(), value.to_owned());
                    }
                }
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
        let mut log = self.log.clone();
        let log_lines: String = log.fields().into_iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        let mut text = format!(
            "theme={}\nstaging_area={}\nupdate_method={}\nauto_update_on_push_rejected={}\nwarn_crlf={}\ncommit_subject_limit={}\n\
             warn_detached_head={}\nprotected_branches={}\nsign_off={}\nrun_hooks={}\ncleanup_message={}\nfetch_interval_minutes={}\ngit_executable={}\nuse_credential_helper={}\n",
            if self.theme_follows_system { "system" } else if self.dark { "dark" } else { "light" },
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
        );
        text.push_str(&log_lines);
        let mut diff = self.diff.clone();
        for (k, v) in diff.fields() {
            text.push_str(&format!("{k}={v}\n"));
        }
        text.push_str(&format!("diff_context_lines={}\n", self.diff.context_lines));
        let [a, d, h] = self.log.columns;
        text.push_str(&format!("log_columns={a},{d},{h}\n"));
        let mut project = self.project.clone();
        for (k, v) in project.fields() {
            text.push_str(&format!("{k}={v}\n"));
        }
        text.push_str(&format!(
            "project_sort={}\n",
            match self.project.sort {
                ProjectSort::Name => "name",
                ProjectSort::Type => "type",
                ProjectSort::Modified => "modified",
            }
        ));
        text.push_str(&format!(
            "commit_group_by_directory={}\ncommit_show_ignored={}\nupdate_clean={}\nsync_branches={}\n",
            self.commit_group_by_directory,
            self.commit_show_ignored,
            if self.update_shelve { "shelve" } else { "stash" },
            self.sync_branches
        ));
        text.push_str(&format!("use_language_servers={}\n", self.use_language_servers));
        let [l, r, b] = self.tool_window_sizes;
        text.push_str(&format!(
            "theme_dark={}\ncompact_mode={}\ntool_windows={}\ntool_window_sizes={l},{r},{b}\ncommit_group_by_module={}\ncommit_group_by_repository={}\n",
            self.dark, self.compact, self.tool_windows, self.commit_group_by_module, self.commit_group_by_repository
        ));
        for (lang, command) in &self.language_servers {
            text.push_str(&format!("lsp_{lang}={command}\n"));
        }
        text
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
        crate::index::lsp::configure(self.use_language_servers, self.language_servers.clone());
    }

    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let mut settings = cx.global::<Self>().clone();
        f(&mut settings);
        settings.save();
        settings.apply_git();
        crate::ui::common::set_compact(settings.compact);
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
        let mut diff = Settings::default();
        diff.diff.show_whitespaces = true;
        diff.diff.context_lines = 8;
        diff.project.single_click = true;
        diff.project.sort = ProjectSort::Modified;
        assert_eq!(Settings::parse(&diff.serialize()), diff);
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

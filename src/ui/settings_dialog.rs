//! Settings (Ctrl+Alt+S), laid out as IntelliJ's: a search field over the
//! page tree on the left, the page on the right, and Cancel / Apply / OK.
//! Options are changed on a draft that Apply or OK puts into effect; the
//! account and directory mapping pages save as they go, as their dialogs did.

use std::collections::BTreeMap;

use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::hosting::account::Service;
use crate::domain::live_templates::CustomTemplates;
use crate::index::lang::Lang;
use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;
use crate::ui::accounts_dialog::AccountsView;
use crate::ui::as_icons as icons;
use crate::domain::code_style::{LangStyle, StyleLang};
use crate::ui::keymap;
use crate::ui::mappings_dialog::MappingsView;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Appearance,
    System,
    Keymap,
    EditorGeneral,
    CodeEditing,
    AutoImport,
    EditorFont,
    GutterIcons,
    EditorTabs,
    CodeStyle,
    LiveTemplates,
    Intentions,
    Commit,
    Mappings,
    Git,
    GitHub,
    GitLab,
    Languages,
    Compiler,
    Gradle,
    AndroidSdk,
}

/// The page tree: a group and its pages; a group with no pages is a page itself.
const TREE: &[(&str, Option<Page>, &[Page])] = &[
    ("Appearance & Behavior", None, &[Page::Appearance, Page::System]),
    ("Keymap", Some(Page::Keymap), &[]),
    ("Editor", None, &[Page::EditorGeneral, Page::CodeEditing, Page::AutoImport, Page::EditorFont, Page::GutterIcons, Page::EditorTabs, Page::CodeStyle, Page::LiveTemplates, Page::Intentions]),
    ("Version Control", None, &[Page::Commit, Page::Mappings, Page::Git, Page::GitHub, Page::GitLab]),
    ("Build, Execution, Deployment", None, &[Page::Gradle, Page::Compiler]),
    ("Languages & Frameworks", Some(Page::Languages), &[Page::AndroidSdk]),
];

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::System => "System Settings",
            Page::Keymap => "Keymap",
            Page::EditorGeneral => "General",
            Page::CodeEditing => "Code Editing",
            Page::AutoImport => "Auto Import",
            Page::EditorFont => "Font",
            Page::GutterIcons => "Gutter Icons",
            Page::EditorTabs => "Editor Tabs",
            Page::CodeStyle => "Code Style",
            Page::LiveTemplates => "Live Templates",
            Page::Intentions => "Intentions",
            Page::Commit => "Commit",
            Page::Mappings => "Directory Mappings",
            Page::Git => "Git",
            Page::GitHub => "GitHub",
            Page::GitLab => "GitLab",
            Page::Languages => "Languages & Frameworks",
            Page::Compiler => "Compiler",
            Page::Gradle => "Gradle",
            Page::AndroidSdk => "Android SDK",
        }
    }

    /// The breadcrumb over the page.
    fn path(self) -> String {
        TREE.iter()
            .find(|(_, _, pages)| pages.contains(&self))
            .map_or_else(|| self.name().to_owned(), |(group, _, _)| format!("{group} › {}", self.name()))
    }

    /// What the search finds the page by besides its name: its options.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Page::Appearance => &["Theme", "Dark", "Light", "Sync with OS", "Compact mode"],
            Page::System => &["System", "Startup", "Reopen projects on startup", "Confirm before exiting", "Exit"],
            Page::Keymap => &["Shortcut", "Keyboard"],
            Page::EditorGeneral => &["Soft-wrap", "Show line numbers", "Show whitespaces", "Show indent guides", "Appearance", "Diff", "Merge"],
            Page::CodeEditing => &["Code Editing", "Refactorings", "Specify refactoring options", "In modal dialogs", "In the editor"],
            Page::AutoImport => &["Auto Import", "Java", "Kotlin", "Optimize imports on the fly", "Imports"],
            Page::EditorFont => &["Font", "Size", "Line height"],
            Page::GutterIcons => &["Gutter", "Icons", "Bookmarks", "Implemented", "Overridden"],
            Page::EditorTabs => &["Tabs", "Close button", "Mark modified", "Tab limit", "Preview tab"],
            Page::CodeStyle => &["Code Style", "Tabs and Indents", "Use tab character", "Tab size", "Indent", "Continuation indent", "Detect and use existing file indents", "Java", "Kotlin", "Groovy", "XML", "Reformat"],
            Page::LiveTemplates => &["Live Templates", "Templates", "Abbreviation", "Template text", "User", "sout", "fori", "iter", "main", "psvm"],
            Page::Intentions => &["Intentions", "Alt+Enter", "Convert to block body", "Add braces", "Specify type explicitly", "Put arguments on separate lines", "string template"],
            Page::Commit => &[
                "Use non-modal commit interface",
                "Limit subject line length",
                "Run Git hooks",
                "Sign-off commit",
                "Clean up commit message",
                "Warn about files larger than",
            ],
            Page::Mappings => &["Directory", "Root", "Mappings", "VCS"],
            Page::Git => &[
                "Path to Git executable",
                "Test",
                "Enable staging area",
                "Use credential helper",
                "Add the 'cherry picked from <hash>' suffix when picking commits pushed to protected branches",
                "Warn if CRLF line separators are about to be committed",
                "Warn when committing in detached HEAD or during rebase",
                "Auto-update if push of the current branch was rejected",
                "Show Push dialog for Commit and Push",
                "Show only for commits to protected branches",
                "Protected branches",
                "Update method",
                "Merge",
                "Rebase",
                "Clean working tree using",
                "Stash",
                "Shelve",
                "Show the Update Project dialog",
                "Execute branch operations on all roots",
                "Update branch info",
                "Fetch",
            ],
            Page::GitHub => &["Account", "Token", "Log In", "Pull Request"],
            Page::GitLab => &["Account", "Token", "Log In", "Merge Request"],
            Page::Languages => &["Language server", "LSP", "Code Navigation", "Go to Declaration"],
            Page::Compiler => &["Compile on save", "Gradle", "Build", "Compiler errors"],
            Page::AndroidSdk => &["SDK Manager", "Android SDK Location", "SDK Platforms", "SDK Tools", "Build-Tools", "Platform-Tools", "NDK", "CMake", "Emulator"],
            Page::Gradle => &["Gradle JDK", "JDK", "Java", "JAVA_HOME", "GRADLE_LOCAL_JAVA_HOME", "Build Tools", "Gradle user home", "Offline work", "Distribution", "Wrapper", "Specified location", "Gradle version"],
        }
    }

    fn matches(self, query: &str) -> bool {
        query.is_empty()
            || matches(self.path().as_str(), query)
            || self.keywords().iter().any(|k| matches(k, query))
            // The keymap is searched by its actions too.
            || (self == Page::Keymap && keymap::ENTRIES.iter().any(|e| matches(e.name, query)))
    }
}

fn matches(text: &str, query: &str) -> bool {
    !query.is_empty() && text.to_lowercase().contains(query)
}

pub struct SettingsView {
    draft: Settings,
    page: Page,
    search: Entity<InputState>,
    protected: Entity<InputState>,
    margin: Entity<InputState>,
    fetch_interval: Entity<InputState>,
    git_path: Entity<InputState>,
    font_size: Entity<InputState>,
    tab_limit: Entity<InputState>,
    /// Code Style: each language's indent and continuation indent.
    indents: Vec<(StyleLang, Entity<InputState>, Entity<InputState>)>,
    large_file: Entity<InputState>,
    /// Languages & Frameworks: a server command per language, and what
    /// runs when it is left empty.
    servers: Vec<(Lang, Entity<InputState>, Option<String>)>,
    /// The Test button's result.
    git_test: Option<Result<String, String>>,
    github: Entity<AccountsView>,
    gitlab: Entity<AccountsView>,
    mappings: Option<Entity<MappingsView>>,
    /// Keymap changes not applied yet: action → shortcuts (`None`: the defaults).
    keymap: BTreeMap<&'static str, Option<Vec<String>>>,
    /// The action waiting for its new shortcut to be pressed.
    recording: Option<&'static str>,
    intercept: Option<Subscription>,
    /// Build Tools › Gradle: the JDKs on this machine, and the project's
    /// Gradle builds.
    jdks: Vec<crate::domain::jdk::Jdk>,
    gradle_builds: Vec<std::path::PathBuf>,
    /// Gradle user home, and each build's Gradle installation folder
    /// ("Specified location").
    gradle_user_home: Entity<InputState>,
    gradle_homes: Vec<Entity<InputState>>,
    /// "Download JDK…" open for a build (its key).
    jdk_download: Option<(String, Entity<crate::ui::jdk_download_panel::JdkDownloadPanel>, Subscription)>,
    /// Editor › Live Templates: the draft, and the person's template being
    /// edited with its fields.
    templates: CustomTemplates,
    template: Option<(usize, TemplateFields)>,
    /// Languages & Frameworks › Android SDK, made when first shown.
    sdk_manager: Option<Entity<crate::ui::sdk_manager::SdkManagerView>>,
    project_root: Option<std::path::PathBuf>,
    _subscriptions: Vec<Subscription>,
}

/// The fields of the person's template being edited.
struct TemplateFields {
    abbreviation: Entity<InputState>,
    description: Entity<InputState>,
    text: Entity<TextareaState>,
    /// An expression and a default value per variable, in the text's order.
    defaults: Vec<(String, Entity<InputState>, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(model: Option<Entity<RepoModel>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = Settings::get(cx).clone();
        let input = |value: String, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).default_value(value));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let protected = input(initial.protected_branches.clone(), window, cx);
        let margin = input(initial.commit_subject_limit.to_string(), window, cx);
        let fetch_interval = input(initial.fetch_interval_minutes.max(1).to_string(), window, cx);
        let font_size = input(format!("{}", initial.editor_font_size()), window, cx);
        let tab_limit = input(initial.diff.tab_limit.to_string(), window, cx);
        let indents = StyleLang::ALL
            .into_iter()
            .map(|lang| {
                let style = initial.code_style.get(lang);
                (lang, input(style.indent.to_string(), window, cx), input(style.continuation.to_string(), window, cx))
            })
            .collect();
        let large_file = input(if initial.large_file_mb > 0 { initial.large_file_mb } else { 50 }.to_string(), window, cx);
        let git_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(match crate::git::detected_executable() {
                    Some(git) => format!("Auto-detected: {}", git.display()),
                    None => "Git not found: install Git or enter its path".to_owned(),
                })
                .default_value(initial.git_executable.clone())
        });
        let servers = Lang::ALL
            .into_iter()
            .filter(|l| *l != Lang::Tsx)
            .map(|lang| {
                let detected = crate::index::lsp::detected(lang);
                let placeholder = match &detected {
                    Some(command) => format!("Auto: {command}"),
                    None => format!("Not found: {}", lang.default_servers().join(" / ")),
                };
                let value = initial.language_servers.get(lang.key()).cloned().unwrap_or_default();
                (lang, cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value)), detected)
            })
            .collect::<Vec<_>>();
        let github = cx.new(|cx| AccountsView::new_for(Some(Service::GitHub), window, cx));
        let gitlab = cx.new(|cx| AccountsView::new_for(Some(Service::GitLab), window, cx));
        let project_root = model.as_ref().and_then(|m| m.read(cx).project_root().map(std::path::Path::to_path_buf));
        let sdk_project = project_root.clone();
        let mut gradle_builds: Vec<std::path::PathBuf> = crate::ui::sync_service::SyncService::try_global(cx).map(|s| s.read(cx).builds().iter().map(|(b, _)| b.clone()).collect()).unwrap_or_default();
        if gradle_builds.is_empty() {
            let gradle_file = |root: &std::path::Path| ["settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts"].iter().any(|f| root.join(f).is_file());
            gradle_builds.extend(project_root.filter(|r| gradle_file(r)));
        }
        let jdks = crate::infra::jdk_discovery::discover(&crate::infra::jdk_discovery::Env::current(), None);
        let gradle_user_home = cx.new(|cx| {
            let default = std::env::var("GRADLE_USER_HOME").ok().filter(|v| !v.is_empty()).map(std::path::PathBuf::from).or_else(|| crate::android::sdk::home().map(|h| h.join(".gradle")));
            let mut state = InputState::new(window, cx).default_value(initial.gradle_user_home.clone());
            if let Some(default) = default {
                state = state.placeholder(default.display().to_string());
            }
            state
        });
        let gradle_homes: Vec<Entity<InputState>> = gradle_builds
            .iter()
            .map(|build| {
                let value = initial.gradle_distribution.get(&crate::domain::diagnostics::file_key(build)).cloned().unwrap_or_default();
                cx.new(|cx| InputState::new(window, cx).placeholder("Gradle installation folder (has bin/gradle)").default_value(value))
            })
            .collect();
        let mappings = model.and_then(|model| MappingsView::new(model, cx)).map(|view| cx.new(|_| view));
        let mut subscriptions = vec![cx.subscribe(&search, |this: &mut Self, search, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                // Keep a page the search still finds, else show the first one it does.
                let query = search.read(cx).value().trim().to_lowercase();
                if !this.page.matches(&query) {
                    if let Some(page) = all_pages().find(|p| p.matches(&query)) {
                        this.page = page;
                    }
                }
                cx.notify();
            }
        })];
        for input in &gradle_homes {
            subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        for (_, input, _) in &servers {
            subscriptions.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        Self {
            draft: initial,
            page: Page::Appearance,
            search,
            protected,
            margin,
            fetch_interval,
            git_path,
            font_size,
            tab_limit,
            indents,
            large_file,
            servers,
            git_test: None,
            github,
            gitlab,
            mappings,
            keymap: BTreeMap::new(),
            recording: None,
            intercept: None,
            jdks,
            gradle_builds,
            gradle_user_home,
            gradle_homes,
            jdk_download: None,
            templates: crate::domain::live_templates::custom(),
            template: None,
            sdk_manager: None,
            project_root: sdk_project,
            _subscriptions: subscriptions,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// Apply / OK: the draft and the fields into effect.
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.draft.clone();
        next.protected_branches = self.protected.read(cx).value().trim().to_owned();
        next.git_executable = self.git_path.read(cx).value().trim().to_owned();
        next.gradle_user_home = self.gradle_user_home.read(cx).value().trim().to_owned();
        for (build, input) in self.gradle_builds.iter().zip(&self.gradle_homes) {
            let key = crate::domain::diagnostics::file_key(build);
            let home = input.read(cx).value().trim().to_owned();
            // A specified location left empty is the wrapper.
            if next.gradle_distribution.contains_key(&key) && !home.is_empty() {
                next.gradle_distribution.insert(key, home);
            } else {
                next.gradle_distribution.remove(&key);
            }
        }
        next.language_servers = self
            .servers
            .iter()
            .filter_map(|(lang, input, _)| {
                let command = input.read(cx).value().trim().to_owned();
                (!command.is_empty()).then(|| (lang.key().to_owned(), command))
            })
            .collect();
        if let Ok(limit) = self.margin.read(cx).value().trim().parse::<usize>() {
            next.commit_subject_limit = limit.clamp(20, 200);
        }
        if next.fetch_interval_minutes > 0 {
            next.fetch_interval_minutes = self.fetch_interval.read(cx).value().trim().parse::<u32>().unwrap_or(10).clamp(1, 1440);
        }
        if next.large_file_mb > 0 {
            next.large_file_mb = self.large_file.read(cx).value().trim().parse::<u32>().unwrap_or(50).clamp(1, 100_000);
        }
        if let Ok(limit) = self.tab_limit.read(cx).value().trim().parse::<usize>() {
            next.diff.tab_limit = limit.clamp(1, 100);
        }
        for (lang, indent, continuation) in &self.indents {
            let style = next.code_style.get(*lang);
            let number = |input: &Entity<InputState>, or: usize| input.read(cx).value().trim().parse::<usize>().unwrap_or(or);
            next.code_style.set(*lang, LangStyle { indent: number(indent, style.indent), continuation: number(continuation, style.continuation), ..style });
        }
        if let Ok(size) = self.font_size.read(cx).value().trim().parse::<f32>() {
            next.editor_font_tenths = (size.clamp(8., 32.) * 10.).round() as u32;
        }
        // Keep what changed elsewhere meanwhile (tool window sizes, View Options).
        let mut current = Settings::get(cx).clone();
        copy_options(&next, &mut current);
        Settings::update(cx, |s| *s = current.clone());
        self.draft = current;
        crate::theme::refresh(cx);
        self.sync_template(window, cx);
        crate::infra::template_store::save(self.templates.clone());
        for (action, keys) in std::mem::take(&mut self.keymap) {
            keymap::set(action, keys, cx);
        }
        if let Some(sdk) = &self.sdk_manager {
            sdk.update(cx, |sdk, cx| sdk.apply(window, cx));
        }
        window.refresh();
        cx.notify();
    }

    fn set(&mut self, f: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        f(&mut self.draft);
        cx.notify();
    }
}

/// The options this dialog edits, from the draft onto the live settings.
fn copy_options(from: &Settings, to: &mut Settings) {
    to.dark = from.dark;
    to.theme_follows_system = from.theme_follows_system;
    to.compact = from.compact;
    to.reopen_projects = from.reopen_projects;
    to.confirm_exit = from.confirm_exit;
    to.staging_area = from.staging_area;
    to.update_method = from.update_method;
    to.update_shelve = from.update_shelve;
    to.sync_branches = from.sync_branches;
    to.update_dialog = from.update_dialog;
    to.auto_update_on_push_rejected = from.auto_update_on_push_rejected;
    to.warn_crlf = from.warn_crlf;
    to.commit_subject_limit = from.commit_subject_limit;
    to.warn_detached_head = from.warn_detached_head;
    to.non_modal_commit = from.non_modal_commit;
    to.large_file_mb = from.large_file_mb;
    to.cherry_pick_suffix = from.cherry_pick_suffix;
    to.commit_push_dialog = from.commit_push_dialog;
    to.commit_push_dialog_protected_only = from.commit_push_dialog_protected_only;
    to.editor_font_tenths = from.editor_font_tenths;
    to.protected_branches = from.protected_branches.clone();
    to.sign_off = from.sign_off;
    to.run_hooks = from.run_hooks;
    to.cleanup_message = from.cleanup_message;
    to.fetch_interval_minutes = from.fetch_interval_minutes;
    to.git_executable = from.git_executable.clone();
    to.use_credential_helper = from.use_credential_helper;
    to.use_language_servers = from.use_language_servers;
    to.compile_on_save = from.compile_on_save;
    to.gradle_jdk = from.gradle_jdk.clone();
    to.gradle_distribution = from.gradle_distribution.clone();
    to.gradle_user_home = from.gradle_user_home.clone();
    to.gradle_offline = from.gradle_offline;
    to.language_servers = from.language_servers.clone();
    to.diff.soft_wrap = from.diff.soft_wrap;
    to.diff.show_line_numbers = from.diff.show_line_numbers;
    to.diff.show_whitespaces = from.diff.show_whitespaces;
    to.diff.show_indent_guides = from.diff.show_indent_guides;
    to.diff.show_gutter_icons = from.diff.show_gutter_icons;
    to.diff.tab_close_button = from.diff.tab_close_button;
    to.diff.tab_mark_modified = from.diff.tab_mark_modified;
    to.diff.tab_multiple_rows = from.diff.tab_multiple_rows;
    to.diff.tab_limit = from.diff.tab_limit;
    to.code_style = from.code_style.clone();
    to.diff.optimize_imports_java = from.diff.optimize_imports_java;
    to.diff.optimize_imports_kotlin = from.diff.optimize_imports_kotlin;
    to.diff.refactor_in_editor = from.diff.refactor_in_editor;
    to.disabled_intentions = from.disabled_intentions.clone();
    to.project.preview_tab = from.project.preview_tab;
}

/// A row of the page tree: a group (a page itself or not) or an indented page.
fn tree_row(id: SharedString, label: &str, indent: f32, selected: bool, page: bool, palette: &crate::theme::Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .h(px(24.))
        .flex()
        .items_center()
        .pl(px(6. + indent))
        .rounded(px(3.))
        .text_sm()
        .text_color(if page { palette.text } else { palette.text_secondary })
        .cursor_pointer()
        .when(selected, |el| el.bg(palette.selection))
        .when(!selected, |el| el.hover(|s| s.bg(palette.hover)))
        .child(label.to_owned())
}

fn all_pages() -> impl Iterator<Item = Page> {
    TREE.iter().flat_map(|(_, page, pages)| page.iter().copied().chain(pages.iter().copied()))
}

// Keymap recording.
impl SettingsView {
    fn record(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.recording = Some(action);
        let entity = cx.entity().downgrade();
        self.intercept = Some(cx.intercept_keystrokes(move |event, _, cx| {
            let keystroke = event.keystroke.clone();
            // Modifiers alone wait for the key.
            if matches!(keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") {
                return;
            }
            cx.stop_propagation();
            entity.update(cx, |this, cx| this.recorded(keystroke, cx)).ok();
        }));
        cx.notify();
    }

    fn recorded(&mut self, keystroke: Keystroke, cx: &mut Context<Self>) {
        let Some(action) = self.recording.take() else { return };
        let plain = !keystroke.modifiers.control && !keystroke.modifiers.alt && !keystroke.modifiers.shift && !keystroke.modifiers.platform;
        if !(plain && keystroke.key == "escape") {
            self.keymap.insert(action, Some(vec![keystroke.unparse()]));
        }
        // Dropped after this keystroke is handled.
        if let Some(intercept) = self.intercept.take() {
            cx.defer(move |_| drop(intercept));
        }
        cx.notify();
    }

    fn shortcuts(&self, action: &'static str, cx: &App) -> Vec<String> {
        match self.keymap.get(action) {
            Some(Some(keys)) => keys.clone(),
            Some(None) => keymap::defaults(action, cx),
            None => keymap::shortcuts(action, cx),
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let query = self.query(cx);
        let entity = cx.entity();
        let mut tree = v_flex().gap_px();
        for (ix, (group, page, pages)) in TREE.iter().enumerate() {
            let shown: Vec<Page> = pages.iter().copied().filter(|p| p.matches(&query)).collect();
            let own = page.filter(|p| p.matches(&query));
            if own.is_none() && shown.is_empty() {
                continue;
            }
            let target = own.or(shown.first().copied());
            let selected = own.is_some_and(|p| p == self.page);
            let entity_group = entity.clone();
            tree = tree.child(
                tree_row(SharedString::from(format!("settings-group-{ix}")), group, 0., selected, own.is_some(), &palette).on_click(move |_, _, cx| {
                    if let Some(page) = target {
                        entity_group.update(cx, |this, cx| {
                            this.page = page;
                            cx.notify();
                        })
                    }
                }),
            );
            for page in shown {
                let entity = entity.clone();
                tree = tree.child(
                    tree_row(SharedString::from(format!("settings-page-{page:?}")), page.name(), 16., page == self.page, true, &palette).on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.page = page;
                            cx.notify();
                        })
                    }),
                );
            }
        }
        let nothing = !all_pages().any(|p| p.matches(&query));
        let content: AnyElement = if nothing {
            div().text_sm().text_color(palette.text_secondary).child("Nothing found").into_any_element()
        } else {
            match self.page {
                Page::Appearance => self.appearance(&query, cx).into_any_element(),
                Page::System => self.system(&query, cx).into_any_element(),
                Page::Keymap => self.keymap_page(&query, cx).into_any_element(),
                Page::EditorGeneral => self.editor_general(&query, cx).into_any_element(),
                Page::CodeEditing => self.code_editing(&query, cx).into_any_element(),
                Page::AutoImport => self.auto_import(&query, cx).into_any_element(),
                Page::EditorFont => self.editor_font(&query, cx).into_any_element(),
                Page::GutterIcons => self.gutter_icons(&query, cx).into_any_element(),
                Page::EditorTabs => self.editor_tabs(&query, cx).into_any_element(),
                Page::CodeStyle => self.code_style_page(&query, cx).into_any_element(),
                Page::Commit => self.commit_page(&query, cx).into_any_element(),
                Page::Mappings => match &self.mappings {
                    Some(view) => view.clone().into_any_element(),
                    None => div().text_sm().text_color(palette.text_secondary).child("Open a project to map its directories").into_any_element(),
                },
                Page::Git => self.git_page(&query, window, cx).into_any_element(),
                Page::GitHub => self.github.clone().into_any_element(),
                Page::GitLab => self.gitlab.clone().into_any_element(),
                Page::Languages => self.languages(&query, cx).into_any_element(),
                Page::Gradle => self.gradle_page(&query, cx).into_any_element(),
                Page::AndroidSdk => {
                    let project = self.project_root.clone();
                    self.sdk_manager.get_or_insert_with(|| cx.new(|cx| crate::ui::sdk_manager::SdkManagerView::new(project, window, cx))).clone().into_any_element()
                }
                Page::LiveTemplates => self.live_templates_page(&query, cx).into_any_element(),
                Page::Intentions => self.intentions_page(&query, cx).into_any_element(),
                Page::Compiler => v_flex()
                    .gap_2()
                    .child(self.check(
                        "settings-compile-on-save",
                        "Compile on save (Kotlin and Java files: the Gradle module compiles in the background and its errors show in the editor)",
                        self.draft.compile_on_save,
                        &query,
                        |s, v| s.compile_on_save = v,
                        cx,
                    ))
                    .into_any_element(),
            }
        };
        h_flex()
            .h(px(560.))
            .items_start()
            .gap_3()
            .child(
                v_flex()
                    .w(px(210.))
                    .h_full()
                    .gap_2()
                    .pr_2()
                    .border_r_1()
                    .border_color(palette.border)
                    .child(Input::new(&self.search).small().cleanable(true))
                    .child(div().id("settings-tree").flex_1().min_h_0().overflow_y_scrollbar().child(tree)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(palette.text).child(if nothing { String::new() } else { self.page.path() }))
                    .child(div().id("settings-content").flex_1().min_h_0().overflow_y_scrollbar().pr_2().child(content)),
            )
    }
}

// The pages.
impl SettingsView {
    /// The Gradle JDK dropdown's "Download JDK…": the panel under the
    /// build's row; the JDK it installs becomes the build's Gradle JDK.
    fn open_jdk_download(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ui::jdk_download_panel::{JdkDownloadEvent, JdkDownloadPanel};
        let panel = cx.new(|cx| JdkDownloadPanel::new(window, cx));
        let build_key = key.clone();
        let subscription = cx.subscribe(&panel, move |this, _, event: &JdkDownloadEvent, cx| {
            if let JdkDownloadEvent::Installed(home) = event {
                if let Ok(jdk) = crate::infra::jdk_discovery::check(home) {
                    this.draft.gradle_jdk.insert(build_key.clone(), jdk.home.to_string_lossy().into_owned());
                    if !this.jdks.iter().any(|j| j.home == jdk.home) {
                        this.jdks.push(jdk);
                    }
                }
            }
            this.jdk_download = None;
            cx.notify();
        });
        self.jdk_download = Some((key, panel, subscription));
        cx.notify();
    }

    /// Build Tools › Gradle: each build's Gradle JDK, as Android Studio's
    /// dropdown offers them.
    fn gradle_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::domain::jdk::{pick, GradleJdk, Jdk, JdkSource};
        let palette = cx.palette().clone();
        let mut page = v_flex().gap_3().text_sm().child(
            v_flex()
                .gap_1()
                .child(self.section("General Settings", cx))
                .child(self.row("Gradle user home:", query, div().w(px(420.)).child(Input::new(&self.gradle_user_home).small()), cx))
                .child(self.check("settings-gradle-offline", "Offline work (builds and syncs run with --offline: only what is in the Gradle caches)", self.draft.gradle_offline, query, |s, v| s.gradle_offline = v, cx)),
        );
        if self.gradle_builds.is_empty() {
            return page.child(div().text_color(palette.text_secondary).child("The project has no Gradle build."));
        }
        let env = crate::infra::jdk_discovery::Env::current();
        for (ix, build) in self.gradle_builds.iter().enumerate() {
            let key = crate::domain::diagnostics::file_key(build);
            let setting = self.draft.gradle_jdk.get(&key).map(|k| GradleJdk::from_key(k)).unwrap_or_default();
            // This build's choices: its own GRADLE_LOCAL_JAVA_HOME, then the machine's.
            let found: Vec<Jdk> = crate::infra::jdk_discovery::discover(&env, Some(build));
            let mut choices: Vec<Jdk> = found.clone();
            for jdk in &self.jdks {
                if !choices.iter().any(|c| c.home == jdk.home) {
                    choices.push(jdk.clone());
                }
            }
            let picked = pick(&setting, &choices).cloned();
            let current = match (&setting, &picked) {
                (GradleJdk::Auto, Some(jdk)) => format!("Detect automatically: {}", jdk.short_label()),
                (GradleJdk::Auto, None) => "Detect automatically".into(),
                (_, Some(jdk)) => jdk.short_label(),
                (GradleJdk::Path(p), None) => format!("{}  (not found)", p.display()),
                (other, None) => format!("{}  (not set)", other.key().trim_start_matches('#')),
            };
            let view = cx.entity();
            let (menu_choices, menu_key, menu_setting) = (choices.clone(), key.clone(), setting.clone());
            let picker = Button::new(("gradle-jdk", ix))
                .outline()
                .small()
                .label(current)
                .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
                .dropdown_menu(move |mut menu, _, _| {
                    let set = |value: GradleJdk| {
                        let (view, key) = (view.clone(), menu_key.clone());
                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| {
                                match &value {
                                    GradleJdk::Auto => this.draft.gradle_jdk.remove(&key),
                                    other => this.draft.gradle_jdk.insert(key.clone(), other.key()),
                                };
                                cx.notify();
                            })
                        }
                    };
                    menu = menu.item(PopupMenuItem::new("Detect automatically").checked(menu_setting == GradleJdk::Auto).on_click(set(GradleJdk::Auto)));
                    for jdk in &menu_choices {
                        let value = match jdk.source {
                            JdkSource::ProjectLocal => GradleJdk::ProjectLocal,
                            JdkSource::JavaHome => GradleJdk::JavaHome,
                            _ => GradleJdk::Path(jdk.home.clone()),
                        };
                        let checked = value == menu_setting;
                        menu = menu.item(PopupMenuItem::new(jdk.label()).checked(checked).on_click(set(value)));
                    }
                    let (download_view, download_key) = (view.clone(), menu_key.clone());
                    menu = menu.separator().item(PopupMenuItem::new("Download JDK…").on_click(move |_, window, cx| {
                        let key = download_key.clone();
                        download_view.update(cx, |this, cx| this.open_jdk_download(key, window, cx));
                    }));
                    let (view, key) = (view.clone(), menu_key.clone());
                    menu.item(PopupMenuItem::new("Add JDK…").on_click(move |_, window, cx| {
                        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Select JDK Home".into()) });
                        let (view, key) = (view.clone(), key.clone());
                        window
                            .spawn(cx, async move |cx| {
                                let Ok(Ok(Some(paths))) = paths.await else { return };
                                let Some(home) = paths.into_iter().next() else { return };
                                let checked = crate::infra::jdk_discovery::check(&home);
                                let _ = view.update(cx, |this, cx| {
                                    match checked {
                                        Ok(jdk) => {
                                            this.draft.gradle_jdk.insert(key, jdk.home.to_string_lossy().into_owned());
                                            if !this.jdks.iter().any(|j| j.home == jdk.home) {
                                                this.jdks.push(jdk);
                                            }
                                        }
                                        Err(e) => this.git_test = Some(Err(e)),
                                    }
                                    cx.notify();
                                });
                            })
                            .detach();
                    }))
                });
            let name = build.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| build.display().to_string());
            page = page.child(
                v_flex()
                    .gap_1()
                    .child(self.section("Gradle Projects", cx).when(ix > 0, |el| el.invisible()))
                    .child(h_flex().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(name)).child(div().text_color(palette.text_secondary).child(build.display().to_string())))
                    .child(self.distribution_row(ix, build, query, cx))
                    .child(self.row("Gradle JDK:", query, picker, cx))
                    .children(self.jdk_download.as_ref().filter(|(k, ..)| *k == key).map(|(_, panel, _)| panel.clone()))
                    .children(picked.as_ref().map(|jdk| div().text_xs().text_color(palette.text_secondary).child(jdk.home.display().to_string())))
                    .when(picked.is_none(), |el| {
                        el.child(div().text_color(palette.status_conflict).child("No JDK found. Install Android Studio or a JDK 17 or newer, set JAVA_HOME, or choose Add JDK…"))
                    }),
            );
        }
        if let Some(Err(e)) = &self.git_test {
            page = page.child(div().text_color(palette.status_conflict).child(e.clone()));
        }
        page
    }

    /// A build's "Distribution": its wrapper (with the Gradle version it
    /// downloads) or a Gradle installed in a folder.
    fn distribution_row(&self, ix: usize, build: &std::path::Path, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let key = crate::domain::diagnostics::file_key(build);
        let local = self.draft.gradle_distribution.contains_key(&key);
        let wrapper = match crate::infra::gradle::wrapper_version(build) {
            Some(version) => format!("Wrapper (Gradle {version})"),
            None => "Wrapper (no gradle-wrapper.properties: gradle on the PATH)".to_owned(),
        };
        let view = cx.entity();
        let (menu_key, menu_wrapper) = (key.clone(), wrapper.clone());
        let picker = Button::new(("gradle-distribution", ix))
            .outline()
            .small()
            .label(if local { "Specified location".to_owned() } else { wrapper })
            .child(Icon::new(icons::CHEVRON_DOWN).xsmall())
            .dropdown_menu(move |menu, _, _| {
                let (a, b) = ((view.clone(), menu_key.clone()), (view.clone(), menu_key.clone()));
                menu.item(PopupMenuItem::new(menu_wrapper.clone()).checked(!local).on_click(move |_, _, cx| {
                    a.0.update(cx, |this, cx| this.set(|s| drop(s.gradle_distribution.remove(&a.1)), cx));
                }))
                .item(PopupMenuItem::new("Specified location").checked(local).on_click(move |_, _, cx| {
                    b.0.update(cx, |this, cx| this.set(|s| { s.gradle_distribution.entry(b.1.clone()).or_default(); }, cx));
                }))
            });
        let input = self.gradle_homes.get(ix).cloned();
        let home = input.as_ref().map(|i| i.read(cx).value().trim().to_owned()).unwrap_or_default();
        let missing = local && !home.is_empty() && !crate::infra::gradle::local_gradle(std::path::Path::new(&home)).is_file();
        v_flex()
            .gap_1()
            .child(self.row("Distribution:", query, picker, cx))
            .when_some(input.filter(|_| local), |el, input| el.child(div().pl_6().w(px(480.)).child(Input::new(&input).small())))
            .when(missing, |el| el.child(div().pl_6().text_color(cx.palette().status_conflict).child("No bin/gradle in this folder.")))
    }

    /// Settings › Editor › Live Templates, as Android Studio's page: the
    /// person's templates ("User", + and −) and the built-in ones by group
    /// (Kotlin, Java), each with a checkbox that turns it on or off; the
    /// selected template of the person's is edited below the list.
    fn live_templates_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::domain::live_templates::builtin;
        let palette = cx.palette().clone();
        let view = cx.entity();
        let selected = self.template.as_ref().map(|(ix, _)| *ix);
        let mut page = v_flex().gap_1().text_sm().child(
            div().text_color(palette.text_secondary).child("Type an abbreviation in a Kotlin or Java file and press Tab; Tab and Enter then move through the template's variables."),
        );
        let add = view.clone();
        let remove = view.clone();
        page = page.child(
            h_flex()
                .gap_2()
                .child(self.section("User", cx))
                .child(div().flex_1())
                .child(Button::new("live-template-add").icon(Icon::new(icons::ADD).xsmall()).ghost().xsmall().tooltip("Add").on_click(move |_, window, cx| {
                    add.update(cx, |this, cx| this.add_template(window, cx))
                }))
                .child(Button::new("live-template-remove").icon(Icon::new(icons::REMOVE).xsmall()).ghost().xsmall().tooltip("Remove").disabled(selected.is_none()).on_click(
                    move |_, window, cx| remove.update(cx, |this, cx| this.remove_template(window, cx)),
                )),
        );
        if self.templates.user.is_empty() {
            page = page.child(div().pl_6().text_xs().text_color(palette.text_secondary).child("No templates of your own: + adds one."));
        }
        for (ix, t) in self.templates.user.iter().enumerate() {
            let (toggle, select) = (view.clone(), view.clone());
            page = page.child(
                h_flex()
                    .id(("live-template-user", ix))
                    .gap_3()
                    .px_1()
                    .rounded(px(3.))
                    .when(selected == Some(ix), |el| el.bg(palette.selection))
                    .on_click(move |_, window, cx| select.update(cx, |this, cx| this.select_template(Some(ix), window, cx)))
                    .child(Checkbox::new(("live-template-on", ix)).checked(t.enabled).on_change(move |v, _, cx| {
                        let v = *v;
                        toggle.update(cx, |this, cx| {
                            if let Some(t) = this.templates.user.get_mut(ix) {
                                t.enabled = v;
                            }
                            cx.notify();
                        })
                    }))
                    .child(self.found(&t.abbreviation, query, cx).w(px(90.)).font_family("monospace").child(t.abbreviation.clone()))
                    .child(self.found(&t.description, query, cx).text_color(palette.text_secondary).child(t.description.clone())),
            );
        }
        if let Some((_, fields)) = &self.template {
            page = page.child(self.template_editor(fields, cx));
        }
        for (group, kotlin) in [("Kotlin", true), ("Java", false)] {
            page = page.child(self.section(group, cx));
            for (ix, t) in builtin(kotlin).iter().enumerate() {
                let on = self.templates.is_enabled(kotlin, &t.name);
                let (toggle, name) = (view.clone(), t.name.to_string());
                page = page.child(
                    h_flex()
                        .gap_3()
                        .px_1()
                        .child(Checkbox::new((if kotlin { "live-template-kotlin" } else { "live-template-java" }, ix)).checked(on).on_change(move |v, _, cx| {
                            let (v, key) = (*v, CustomTemplates::key(kotlin, &name));
                            toggle.update(cx, |this, cx| {
                                this.templates.disabled.retain(|k| *k != key);
                                if !v {
                                    this.templates.disabled.push(key);
                                }
                                cx.notify();
                            })
                        }))
                        .child(self.found(&t.name, query, cx).w(px(90.)).font_family("monospace").child(t.name.to_string()))
                        .child(self.found(&t.description, query, cx).text_color(palette.text_secondary).child(t.description.to_string())),
                );
            }
        }
        page
    }

    /// The selected template's Abbreviation, Description, Template text,
    /// its variables' default values, and where it applies.
    fn template_editor(&self, fields: &TemplateFields, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::domain::live_templates::Context as Where;
        let palette = cx.palette().clone();
        let Some((ix, _)) = &self.template else { return div().into_any_element() };
        let Some(t) = self.templates.user.get(*ix) else { return div().into_any_element() };
        let label = |text: &'static str| div().w(px(110.)).child(text);
        let mut contexts = v_flex().gap_1().child(div().text_color(palette.text_secondary).child("Applicable in:"));
        let groups: [(&'static str, bool, &[(Where, &'static str)]); 2] = [
            ("Kotlin", true, &[(Where::Statement, "Statement"), (Where::Expression, "Expression"), (Where::Class, "Class"), (Where::TopLevel, "Top-level"), (Where::Object, "Object declaration")]),
            ("Java", false, &[(Where::Statement, "Statement"), (Where::Expression, "Expression"), (Where::Class, "Declaration")]),
        ];
        for (group, kotlin, places) in groups {
            let ticked = if kotlin { &t.kotlin } else { &t.java };
            let mut row = h_flex().gap_3().child(div().w(px(60.)).child(group));
            for (n, (place, name)) in places.iter().enumerate() {
                let (view, place) = (cx.entity(), *place);
                let id = (if kotlin { "live-template-ctx-kotlin" } else { "live-template-ctx-java" }, n);
                row = row.child(Checkbox::new(id).label(*name).checked(ticked.contains(&place)).on_change(move |v, _, cx| {
                    let v = *v;
                    view.update(cx, |this, cx| {
                        let ix = this.template.as_ref().map(|(ix, _)| *ix);
                        if let Some(t) = ix.and_then(|ix| this.templates.user.get_mut(ix)) {
                            let list = if kotlin { &mut t.kotlin } else { &mut t.java };
                            list.retain(|c| *c != place);
                            if v {
                                list.push(place);
                            }
                        }
                        cx.notify();
                    })
                }));
            }
            contexts = contexts.child(row);
        }
        let mut variables = v_flex().gap_1();
        if !fields.defaults.is_empty() {
            let head = |w: f32, t: &'static str| div().w(px(w)).text_color(palette.text_secondary).child(t);
            variables = variables.child(h_flex().gap_2().child(head(110., "Name")).child(head(260., "Expression")).child(head(200., "Default value")));
        }
        for (name, expression, input) in &fields.defaults {
            // An expression the editor can't work out is marked.
            let text = expression.read(cx).value().to_string();
            let unknown = !text.trim().is_empty() && crate::domain::live_templates::parse_expression(&text).is_none();
            variables = variables.child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(110.)).font_family("monospace").child(format!("${name}$")))
                    .child(div().w(px(260.)).child(Input::new(expression).small()))
                    .child(div().w(px(200.)).child(Input::new(input).small()))
                    .when(unknown, |el| el.child(div().text_xs().text_color(palette.status_deleted).child("Not supported"))),
            );
        }
        if !fields.defaults.is_empty() {
            variables = variables.child(div().text_xs().text_color(palette.text_secondary).child(
                "Expressions: className(), methodName(), methodParameters(), suggestIndexName(), variableOfType(\"Type\"), arrayVariable(), componentTypeOf(V), typeParameterOf(V, n), subtypes(typeOfVariable(V)), suggestVariableName(T), \"text\", or another variable's NAME. The default value is used when the expression gives nothing.",
            ));
        }
        v_flex()
            .gap_2()
            .my_2()
            .p_2()
            .border_1()
            .border_color(palette.border)
            .rounded(px(4.))
            .child(h_flex().gap_2().child(label("Abbreviation:")).child(div().w(px(160.)).child(Input::new(&fields.abbreviation).small())))
            .child(h_flex().gap_2().child(label("Description:")).child(div().flex_1().child(Input::new(&fields.description).small())))
            .child(v_flex().gap_1().child("Template text:").child(Textarea::new(&fields.text).font_family("monospace")))
            .child(div().text_xs().text_color(palette.text_secondary).child("$NAME$ is a variable Tab moves through; $END$ is where the caret ends; $SELECTION$ is the selected text (Surround With)."))
            .child(variables)
            .child(contexts)
            .into_any_element()
    }

    fn add_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::domain::live_templates::{Context as Where, UserTemplate};
        self.sync_template(window, cx);
        let n = (1..).find(|n| !self.templates.user.iter().any(|t| t.abbreviation == format!("abbr{n}"))).unwrap_or(1);
        self.templates.user.push(UserTemplate {
            abbreviation: format!("abbr{n}"),
            text: "$END$".into(),
            kotlin: vec![Where::Statement],
            java: vec![Where::Statement],
            enabled: true,
            ..Default::default()
        });
        self.select_template(Some(self.templates.user.len() - 1), window, cx);
    }

    fn remove_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ix, _)) = self.template.take() else { return };
        if ix < self.templates.user.len() {
            self.templates.user.remove(ix);
        }
        let next = (!self.templates.user.is_empty()).then(|| ix.min(self.templates.user.len() - 1));
        self.select_template(next, window, cx);
    }

    /// Edits one of the person's templates below the list.
    fn select_template(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_template(window, cx);
        self.template = ix.and_then(|ix| {
            let t = self.templates.user.get(ix)?.clone();
            let abbreviation = cx.new(|cx| InputState::new(window, cx).default_value(t.abbreviation.clone()));
            let description = cx.new(|cx| InputState::new(window, cx).default_value(t.description.clone()));
            let text = cx.new(|cx| TextareaState::new(window, cx).rows(4).default_value(t.text.clone()));
            let subscriptions = vec![
                cx.subscribe_in(&abbreviation, window, Self::template_changed),
                cx.subscribe_in(&description, window, Self::template_changed),
                cx.subscribe_in(&text, window, Self::template_changed),
            ];
            let mut fields = TemplateFields { abbreviation, description, text, defaults: Vec::new(), _subscriptions: subscriptions };
            self.default_fields(&mut fields, &t, window, cx);
            Some((ix, fields))
        });
        cx.notify();
    }

    fn template_changed<E>(&mut self, _: &Entity<E>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::Change) {
            self.sync_template(window, cx);
            cx.notify();
        }
    }

    /// A default value field per variable of the text, keeping the ones
    /// already there.
    fn default_fields(&self, fields: &mut TemplateFields, t: &crate::domain::live_templates::UserTemplate, window: &mut Window, cx: &mut Context<Self>) {
        let names = crate::domain::live_templates::variables(&t.text);
        if fields.defaults.iter().map(|(n, _, _)| n).eq(names.iter()) {
            return;
        }
        let mut old = std::mem::take(&mut fields.defaults);
        for name in names {
            let (expression, input) = match old.iter().position(|(n, _, _)| *n == name) {
                Some(at) => {
                    let (_, e, i) = old.remove(at);
                    (e, i)
                }
                None => {
                    let value = |list: &[(String, String)]| list.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone()).unwrap_or_default();
                    let (expression, default) = (value(&t.expressions), value(&t.defaults));
                    let expression = cx.new(|cx| InputState::new(window, cx).placeholder("None").default_value(expression));
                    let input = cx.new(|cx| InputState::new(window, cx).placeholder("None").default_value(default));
                    fields._subscriptions.push(cx.subscribe_in(&expression, window, Self::template_changed));
                    fields._subscriptions.push(cx.subscribe_in(&input, window, Self::template_changed));
                    (expression, input)
                }
            };
            fields.defaults.push((name, expression, input));
        }
    }

    /// The edited template's fields into the draft.
    fn sync_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ix, mut fields)) = self.template.take() else { return };
        if let Some(t) = self.templates.user.get_mut(ix) {
            t.abbreviation = fields.abbreviation.read(cx).value().trim().to_owned();
            t.description = fields.description.read(cx).value().trim().to_owned();
            t.text = fields.text.read(cx).value().to_string();
            t.defaults = fields
                .defaults
                .iter()
                .map(|(n, _, i)| (n.clone(), i.read(cx).value().to_string()))
                .filter(|(_, v)| !v.is_empty())
                .collect();
            t.expressions = fields
                .defaults
                .iter()
                .map(|(n, e, _)| (n.clone(), e.read(cx).value().trim().to_owned()))
                .filter(|(_, v)| !v.is_empty())
                .collect();
            let t = t.clone();
            self.default_fields(&mut fields, &t, window, cx);
        }
        self.template = Some((ix, fields));
    }

    fn section(&self, title: &'static str, cx: &App) -> gpui_kit::Div {
        div().pt_2().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(cx.palette().text).child(title)
    }

    /// A label the search found gets the find highlight, as IntelliJ marks it.
    fn found(&self, label: &str, query: &str, cx: &App) -> gpui_kit::Div {
        div().rounded(px(3.)).when(matches(label, query), |el| el.bg(cx.palette().search_match))
    }

    fn check(&self, id: &'static str, label: &'static str, value: bool, query: &str, set: fn(&mut Settings, bool), cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        self.found(label, query, cx).child(Checkbox::new(id).label(label).checked(value).on_change(move |v, _, cx| {
            let v = *v;
            entity.update(cx, |this, cx| this.set(|s| set(s, v), cx))
        }))
    }

    fn row(&self, label: &'static str, query: &str, field: impl IntoElement, cx: &App) -> impl IntoElement {
        h_flex().gap_2().text_sm().child(self.found(label, query, cx).child(label)).child(field)
    }

    fn note(&self, text: &'static str, cx: &App) -> impl IntoElement {
        div().pl_6().text_xs().text_color(cx.palette().text_secondary).child(text)
    }

    fn appearance(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft;
        let entity = cx.entity();
        v_flex()
            .gap_2()
            .child(self.row(
                "Theme:",
                query,
                RadioGroup::horizontal("settings-theme")
                    .children(["Dark", "Light", "Sync with OS"])
                    .selected_index(Some(if d.theme_follows_system { 2 } else if d.dark { 0 } else { 1 }))
                    .on_change(move |ix, _, cx| {
                        let ix = *ix;
                        entity.update(cx, |this, cx| {
                            this.set(
                                |s| {
                                    s.theme_follows_system = ix == 2;
                                    if ix < 2 {
                                        s.dark = ix == 0;
                                    }
                                },
                                cx,
                            )
                        })
                    }),
                cx,
            ))
            .child(self.check("settings-compact", "Compact mode", d.compact, query, |s, v| s.compact = v, cx))
            .child(self.note("Smaller toolbars, tool window stripes and rows", cx))
    }

    /// Appearance & Behavior › System Settings: Startup.
    fn system(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft;
        v_flex()
            .gap_2()
            .child(self.check("settings-reopen-projects", "Reopen projects on startup", d.reopen_projects, query, |s, v| s.reopen_projects = v, cx))
            .child(self.note("Started without a folder, opens the last project and its editor tabs", cx))
            .child(self.check("settings-confirm-exit", "Confirm before exiting the IDE", d.confirm_exit, query, |s, v| s.confirm_exit = v, cx))
    }

    fn editor_general(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.diff;
        v_flex()
            .gap_2()
            .child(self.section("Appearance", cx))
            .child(self.note("The editor, and the diff and merge viewers", cx))
            .child(self.check("settings-soft-wrap", "Soft-wrap", d.soft_wrap, query, |s, v| s.diff.soft_wrap = v, cx))
            .child(self.check("settings-line-numbers", "Show line numbers", d.show_line_numbers, query, |s, v| s.diff.show_line_numbers = v, cx))
            .child(self.check("settings-whitespaces", "Show whitespaces", d.show_whitespaces, query, |s, v| s.diff.show_whitespaces = v, cx))
            .child(self.check("settings-indent-guides", "Show indent guides", d.show_indent_guides, query, |s, v| s.diff.show_indent_guides = v, cx))
    }

    fn code_editing(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        v_flex()
            .gap_2()
            .child(self.section("Refactorings", cx))
            .child(self.row(
                "Specify refactoring options:",
                query,
                RadioGroup::horizontal("settings-refactor-options")
                    .children(["In modal dialogs", "In the editor"])
                    .selected_index(Some(usize::from(self.draft.diff.refactor_in_editor)))
                    .on_change(move |ix, _, cx| {
                        let in_editor = *ix == 1;
                        entity.update(cx, |this, cx| this.set(|s| s.diff.refactor_in_editor = in_editor, cx))
                    }),
                cx,
            ))
            .child(self.note("In the editor, Introduce Variable names the variable where it is declared, every use following", cx))
    }

    /// Editor › Intentions: each intention Alt+Enter offers, by language,
    /// with a checkbox to turn it off.
    fn intentions_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let palette = cx.palette().clone();
        let mut page = v_flex().gap_2().child(self.note("Intentions Alt+Enter offers; unchecked ones aren't listed", cx));
        for language in ["Kotlin", "Java"] {
            page = page.child(self.section(if language == "Kotlin" { "Kotlin" } else { "Java" }, cx));
            for (ix, (_, family, description)) in crate::domain::intentions::FAMILIES.iter().enumerate().filter(|(_, (l, _, _))| *l == language) {
                let key = crate::domain::intentions::family_key(language == "Kotlin", family);
                let on = !self.draft.disabled_intentions.contains(&key);
                let entity = entity.clone();
                page = page.child(
                    h_flex()
                        .gap_3()
                        .child(self.found(family, query, cx).child(Checkbox::new(("settings-intention", ix)).label(*family).checked(on).on_change(move |v, _, cx| {
                            let (v, key) = (*v, key.clone());
                            entity.update(cx, |this, cx| {
                                this.set(
                                    |s| {
                                        s.disabled_intentions.retain(|k| *k != key);
                                        if !v {
                                            s.disabled_intentions.push(key);
                                        }
                                    },
                                    cx,
                                )
                            })
                        })))
                        .child(div().text_xs().text_color(palette.text_secondary).child(*description)),
                );
            }
        }
        page
    }

    fn auto_import(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.diff;
        v_flex()
            .gap_2()
            .child(self.section("Java", cx))
            .child(self.check("settings-optimize-java", "Optimize imports on the fly", d.optimize_imports_java, query, |s, v| s.diff.optimize_imports_java = v, cx))
            .child(self.section("Kotlin", cx))
            .child(self.check("settings-optimize-kotlin", "Optimize imports on the fly", d.optimize_imports_kotlin, query, |s, v| s.diff.optimize_imports_kotlin = v, cx))
            .child(self.note("Unused imports are removed and the rest sorted when the file is saved, unless the caret is in the import list", cx))
    }

    fn gutter_icons(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.diff;
        v_flex()
            .gap_2()
            .child(self.check("settings-gutter-icons", "Show gutter icons", d.show_gutter_icons, query, |s, v| s.diff.show_gutter_icons = v, cx))
            .child(self.note("Implementing / implemented and overriding / overridden markers, and bookmarks", cx))
    }

    fn editor_tabs(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.diff;
        v_flex()
            .gap_2()
            .child(self.section("Appearance", cx))
            .child(self.check("settings-tab-rows", "Multiple rows", d.tab_multiple_rows, query, |s, v| s.diff.tab_multiple_rows = v, cx))
            .child(self.note("Otherwise one row that scrolls, as by default", cx))
            .child(self.check("settings-tab-close", "Show close button", d.tab_close_button, query, |s, v| s.diff.tab_close_button = v, cx))
            .child(self.check("settings-tab-modified", "Mark modified (*)", d.tab_mark_modified, query, |s, v| s.diff.tab_mark_modified = v, cx))
            .child(self.section("Opening Policy", cx))
            .child(self.check("settings-preview-tab", "Enable preview tab", self.draft.project.preview_tab, query, |s, v| s.project.preview_tab = v, cx))
            .child(self.section("Closing Policy", cx))
            .child(self.row("Tab limit:", query, div().w(px(70.)).child(Input::new(&self.tab_limit).small()), cx))
            .child(self.note("Past the limit the least recently used unpinned, unmodified tab closes", cx))
    }

    /// Editor › Code Style: General's indent detection, and each
    /// language's Tabs and Indents, what Reformat Code indents with.
    fn code_style_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.code_style;
        let mut page = v_flex()
            .gap_2()
            .child(self.section("General", cx))
            .child(self.check("settings-detect-indents", "Detect and use existing file indents for editing", d.detect_indents, query, |s, v| s.code_style.detect_indents = v, cx))
            .child(self.note("Reformat keeps a file's own indent unit; .editorconfig's indent wins over both", cx));
        for (lang, indent, continuation) in &self.indents {
            let lang = *lang;
            let entity = cx.entity();
            page = page
                .child(self.section(lang.name(), cx))
                .child(self.found("Use tab character", query, cx).child(
                    Checkbox::new(SharedString::from(format!("settings-tab-{}", lang.name()))).label("Use tab character").checked(d.get(lang).use_tab).on_change(move |v, _, cx| {
                        let v = *v;
                        entity.update(cx, |this, cx| {
                            this.set(
                                |s| {
                                    let style = s.code_style.get(lang);
                                    s.code_style.set(lang, LangStyle { use_tab: v, ..style })
                                },
                                cx,
                            )
                        })
                    }),
                ))
                .child(self.row("Indent:", query, div().w(px(70.)).child(Input::new(indent).small()), cx))
                .child(self.row("Continuation indent:", query, div().w(px(70.)).child(Input::new(continuation).small()), cx));
        }
        page
    }

    fn editor_font(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let size = self.font_size.read(cx).value().trim().parse::<f32>().unwrap_or(self.draft.editor_font_size()).clamp(8., 32.);
        let mono = gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone();
        v_flex()
            .gap_2()
            .child(self.row("Size:", query, div().w(px(70.)).child(Input::new(&self.font_size).small()), cx))
            .child(self.note("The editor, diff and merge viewers; lines are 1.6 times as tall", cx))
            .child(
                div()
                    .mt_2()
                    .p_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette.border)
                    .font_family(mono)
                    .text_size(px(size))
                    .line_height(px((size * 1.6).round()))
                    .children(["fn main() {", "    println!(\"Junction Studio\");", "}"].map(|l| div().whitespace_nowrap().child(l))),
            )
    }

    fn commit_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft;
        v_flex()
            .gap_2()
            .child(self.check(
                "settings-non-modal",
                "Use non-modal commit interface",
                d.non_modal_commit,
                query,
                |s, v| s.non_modal_commit = v,
                cx,
            ))
            .child(self.note("Commit from the Commit tool window; off, Commit (Ctrl+K) opens the Commit Changes dialog", cx))
            .child(self.section("Commit Message Inspections", cx))
            .child(self.row("Limit subject line length:", query, div().w(px(70.)).child(Input::new(&self.margin).small()), cx))
            .child(self.section("Before Commit", cx))
            .child(self.check("settings-hooks", "Run Git hooks", d.run_hooks, query, |s, v| s.run_hooks = v, cx))
            .child(self.check("settings-signoff", "Sign-off commit", d.sign_off, query, |s, v| s.sign_off = v, cx))
            .child(self.check("settings-cleanup", "Clean up commit message", d.cleanup_message, query, |s, v| s.cleanup_message = v, cx))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(self.found("Warn about files larger than", query, cx).child(
                        Checkbox::new("settings-large-files").label("Warn about files larger than").checked(d.large_file_mb > 0).on_change({
                            let entity = cx.entity();
                            // The size comes from the field on Apply; 1 marks "on".
                            move |v, _, cx| {
                                let on = *v as u32;
                                entity.update(cx, |this, cx| this.set(|s| s.large_file_mb = on, cx))
                            }
                        }),
                    ))
                    .child(div().w(px(60.)).child(Input::new(&self.large_file).small().disabled(d.large_file_mb == 0)))
                    .child("MB"),
            )
    }

    fn git_page(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let palette = cx.palette().clone();
        let d = self.draft.clone();
        let entity = cx.entity();
        let test_path = self.git_path.clone();
        let test = {
            let entity = entity.clone();
            Button::new("settings-git-test").small().label("Test").on_click(move |_, _, cx| {
                let path = test_path.read(cx).value().to_string();
                let result = crate::git::executable_version(&path).map_err(|e| e.to_string());
                entity.update(cx, |this, cx| {
                    this.git_test = Some(result);
                    cx.notify();
                })
            })
        };
        let (update_entity, clean_entity, fetch_entity) = (entity.clone(), entity.clone(), entity.clone());
        v_flex()
            .gap_2()
            .child(self.row("Path to Git executable:", query, h_flex().flex_1().gap_2().child(div().flex_1().child(Input::new(&self.git_path).small())).child(test), cx))
            .children(self.git_test.clone().map(|result| {
                let (text, color) = match result {
                    Ok(version) => (version, palette.status_added),
                    Err(error) => (error, palette.status_conflict),
                };
                div().pl_6().text_xs().text_color(color).child(text)
            }))
            .child(self.check("settings-staging", "Enable staging area", d.staging_area, query, |s, v| s.staging_area = v, cx))
            .child(self.note("Show Staged and Unstaged changes in the Commit tool window instead of changelists", cx))
            .child(self.check("settings-credential-helper", "Use credential helper", d.use_credential_helper, query, |s, v| s.use_credential_helper = v, cx))
            .child(self.section("Commit", cx))
            .child(self.check(
                "settings-cherry-pick",
                "Add the 'cherry picked from <hash>' suffix when picking commits pushed to protected branches",
                d.cherry_pick_suffix,
                query,
                |s, v| s.cherry_pick_suffix = v,
                cx,
            ))
            .child(self.check("settings-crlf", "Warn if CRLF line separators are about to be committed", d.warn_crlf, query, |s, v| s.warn_crlf = v, cx))
            .child(self.check(
                "settings-detached",
                "Warn when committing in detached HEAD or during rebase",
                d.warn_detached_head,
                query,
                |s, v| s.warn_detached_head = v,
                cx,
            ))
            .child(self.section("Push", cx))
            .child(self.check(
                "settings-auto-update",
                "Auto-update if push of the current branch was rejected",
                d.auto_update_on_push_rejected,
                query,
                |s, v| s.auto_update_on_push_rejected = v,
                cx,
            ))
            .child(self.check("settings-push-dialog", "Show Push dialog for Commit and Push", d.commit_push_dialog, query, |s, v| s.commit_push_dialog = v, cx))
            .child(
                div().pl_6().child(
                    self.found("Show only for commits to protected branches", query, cx).child(
                        Checkbox::new("settings-push-protected")
                            .label("Show only for commits to protected branches")
                            .checked(d.commit_push_dialog_protected_only)
                            .disabled(!d.commit_push_dialog)
                            .on_change({
                                let entity = entity.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    entity.update(cx, |this, cx| this.set(|s| s.commit_push_dialog_protected_only = v, cx))
                                }
                            }),
                    ),
                ),
            )
            .child(self.row("Protected branches:", query, div().flex_1().child(Input::new(&self.protected).small()), cx))
            .child(self.section("Update", cx))
            .child(self.row(
                "Update method:",
                query,
                RadioGroup::horizontal("settings-update-method")
                    .children(["Merge", "Rebase"])
                    .selected_index(Some(if d.update_method == UpdateMethod::Rebase { 1 } else { 0 }))
                    .on_change(move |ix, _, cx| {
                        let rebase = *ix == 1;
                        update_entity.update(cx, |this, cx| this.set(|s| s.update_method = if rebase { UpdateMethod::Rebase } else { UpdateMethod::Merge }, cx))
                    }),
                cx,
            ))
            .child(self.row(
                "Clean working tree using:",
                query,
                RadioGroup::horizontal("settings-update-clean")
                    .children(["Stash", "Shelve"])
                    .selected_index(Some(if d.update_shelve { 1 } else { 0 }))
                    .on_change(move |ix, _, cx| {
                        let shelve = *ix == 1;
                        clean_entity.update(cx, |this, cx| this.set(|s| s.update_shelve = shelve, cx))
                    }),
                cx,
            ))
            .child(self.check("settings-update-dialog", "Show the Update Project dialog (Ctrl+T)", d.update_dialog, query, |s, v| s.update_dialog = v, cx))
            .child(self.section("Branches", cx))
            .child(self.check(
                "settings-sync-branches",
                "Execute branch operations on all roots",
                d.sync_branches,
                query,
                |s, v| s.sync_branches = v,
                cx,
            ))
            .child(self.note("Projects with several Git roots: checkout, new branch and the other branch actions act on every root", cx))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(self.found("Update branch info", query, cx).child(
                        Checkbox::new("settings-fetch").label("Update branch info: fetch every").checked(d.fetch_interval_minutes > 0).on_change(move |v, _, cx| {
                            // The minutes come from the field on Apply; 1 marks "on".
                            let on = *v as u32;
                            fetch_entity.update(cx, |this, cx| this.set(|s| s.fetch_interval_minutes = on, cx))
                        }),
                    ))
                    .child(div().w(px(50.)).child(Input::new(&self.fetch_interval).small().disabled(d.fetch_interval_minutes == 0)))
                    .child("minutes"),
            )
    }

    fn languages(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let enabled = self.draft.use_language_servers;
        let rows: Vec<_> = self
            .servers
            .iter()
            .map(|(lang, input, detected)| {
                let status = match (input.read(cx).value().trim(), detected) {
                    ("off", _) => ("Off", palette.text_secondary),
                    (custom, _) if !custom.is_empty() => {
                        let found = custom.split_whitespace().next().and_then(crate::index::lsp::find_program).is_some();
                        if found { ("Custom", palette.status_added) } else { ("Not found", palette.status_conflict) }
                    }
                    (_, Some(_)) => ("Installed", palette.status_added),
                    (_, None) => ("Index only", palette.text_secondary),
                };
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(div().w(px(90.)).child(lang.name()))
                    .child(div().flex_1().child(Input::new(input).small().disabled(!enabled)))
                    .child(div().w(px(70.)).text_xs().text_color(status.1).child(status.0))
            })
            .collect();
        v_flex()
            .gap_2()
            .child(self.section("Code Navigation", cx))
            .child(self.check(
                "settings-lsp",
                "Use language servers for Go to Declaration, Quick Documentation and Find Usages",
                enabled,
                query,
                |s, v| s.use_language_servers = v,
                cx,
            ))
            .child(div().pl_6().text_xs().text_color(palette.text_secondary).child(
                "The built-in index always answers, and resolves calls across JNI, Dart FFI, extern \"C\" and Swift/Objective-C bridges. \
                 Leave a command empty to use the detected server, or type off to disable one.",
            ))
            .children(rows)
    }

    fn keymap_page(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette().clone();
        let entity = cx.entity();
        // Who else uses each shortcut, for IntelliJ's conflict note.
        let mut users: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
        for entry in keymap::ENTRIES {
            for keys in self.shortcuts(entry.action, cx) {
                users.entry(keys).or_default().push(entry.name);
            }
        }
        let mut list = v_flex().gap_px();
        let mut group = "";
        for (ix, entry) in keymap::ENTRIES.iter().enumerate() {
            let keys = self.shortcuts(entry.action, cx);
            // X11 reports Ctrl+Shift+` as Ctrl+~ too: those twins aren't listed.
            let shown = keys
                .iter()
                .filter(|k| !(k.contains('~') && keys.iter().any(|o| o.contains('`'))))
                .map(|k| keymap::display(k))
                .collect::<Vec<_>>()
                .join(", ");
            if !query.is_empty() && !matches(entry.name, query) && !matches(&shown, query) && !matches(entry.group, query) {
                continue;
            }
            if entry.group != group {
                group = entry.group;
                list = list.child(div().pt_2().pb_0p5().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.text_secondary).child(group));
            }
            let recording = self.recording == Some(entry.action);
            let changed = match self.keymap.get(entry.action) {
                Some(Some(_)) => true,
                Some(None) => false,
                None => !keymap::is_default(entry.action, cx),
            };
            let conflicts: Vec<&str> = keys
                .iter()
                .flat_map(|k| users.get(k).into_iter().flatten().copied())
                .filter(|name| *name != entry.name)
                .collect();
            let action = entry.action;
            let (change, remove, reset) = (entity.clone(), entity.clone(), entity.clone());
            list = list.child(
                h_flex()
                    .h(px(28.))
                    .px_1()
                    .gap_2()
                    .text_sm()
                    .rounded(px(3.))
                    .hover(|s| s.bg(palette.hover))
                    .child(self.found(entry.name, query, cx).flex_1().min_w_0().child(entry.name))
                    .when(!conflicts.is_empty(), |el| {
                        el.child(div().text_xs().text_color(palette.status_conflict).child(format!("Also: {}", conflicts.join(", "))))
                    })
                    .child(
                        div()
                            .w(px(170.))
                            .text_color(if recording { palette.link } else if changed { palette.status_modified } else { palette.text_secondary })
                            .child(if recording { "Press keys… (Esc cancels)".to_owned() } else if shown.is_empty() { "—".to_owned() } else { shown }),
                    )
                    .child(Button::new(("keymap-change", ix)).xsmall().outline().label("Change").on_click(move |_, _, cx| {
                        change.update(cx, |this, cx| this.record(action, cx))
                    }))
                    .child(Button::new(("keymap-remove", ix)).xsmall().ghost().label("Remove").disabled(keys.is_empty()).on_click(move |_, _, cx| {
                        remove.update(cx, |this, cx| {
                            this.keymap.insert(action, Some(Vec::new()));
                            cx.notify();
                        })
                    }))
                    .child(Button::new(("keymap-reset", ix)).xsmall().ghost().label("Reset").disabled(!changed).on_click(move |_, _, cx| {
                        reset.update(cx, |this, cx| {
                            this.keymap.insert(action, None);
                            cx.notify();
                        })
                    })),
            );
        }
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(palette.text_secondary).child("IntelliJ's default keymap; Change records the next shortcut pressed."))
            .child(list)
    }
}

/// Opens Settings; `model` gives the Directory Mappings page its project.
pub fn open(model: Option<Entity<RepoModel>>, window: &mut Window, cx: &mut App) {
    open_page(model, "", window, cx);
}

/// Opens Settings at the page named `page` (the editor's Configure Soft
/// Wraps… / Configure Gutter Icons…).
pub fn open_page(model: Option<Entity<RepoModel>>, page: &str, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| {
        let mut view = SettingsView::new(model, window, cx);
        if let Some(found) = all_pages().find(|p| p.name() == page) {
            view.page = found;
        }
        view
    });
    let focus = view.read(cx).search.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let (apply_view, ok_view) = (view.clone(), view.clone());
        dialog
            .title("Settings")
            .w(px(980.))
            .child(view.clone())
            .footer(
                DialogFooter::new()
                    .gap_2()
                    .child(DialogClose::new().child(Button::new("settings-cancel").label("Cancel").outline()))
                    .child(Button::new("settings-apply").label("Apply").outline().flex_1().on_click(move |_, window, cx| {
                        apply_view.update(cx, |this, cx| this.apply(window, cx))
                    }))
                    .child(DialogAction::new().child(Button::new("settings-ok").label("OK").primary())),
            )
            .on_ok(move |_, window, cx| {
                ok_view.update(cx, |this, cx| this.apply(window, cx));
                true
            })
    });
    crate::ui::dialogs::focus_input(&focus, window, cx);
}

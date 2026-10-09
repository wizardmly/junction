//! Settings (Ctrl+Alt+S), laid out as IntelliJ's: a search field over the
//! page tree on the left, the page on the right, and Cancel / Apply / OK.
//! Options are changed on a draft that Apply or OK puts into effect; the
//! account and directory mapping pages save as they go, as their dialogs did.

use std::collections::BTreeMap;

use gpui_kit::component::{
    Disableable as _, Sizable as _, WindowExt as _, h_flex,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState},
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::hosting::account::Service;
use crate::index::lang::Lang;
use crate::model::RepoModel;
use crate::settings::{Settings, UpdateMethod};
use crate::theme::ActivePalette as _;
use crate::ui::accounts_dialog::AccountsView;
use crate::ui::keymap;
use crate::ui::mappings_dialog::MappingsView;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Appearance,
    Keymap,
    EditorGeneral,
    EditorFont,
    Commit,
    Mappings,
    Git,
    GitHub,
    GitLab,
    Languages,
}

/// The page tree: a group and its pages; a group with no pages is a page itself.
const TREE: &[(&str, Option<Page>, &[Page])] = &[
    ("Appearance & Behavior", None, &[Page::Appearance]),
    ("Keymap", Some(Page::Keymap), &[]),
    ("Editor", None, &[Page::EditorGeneral, Page::EditorFont]),
    ("Version Control", None, &[Page::Commit, Page::Mappings, Page::Git, Page::GitHub, Page::GitLab]),
    ("Languages & Frameworks", Some(Page::Languages), &[]),
];

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Keymap => "Keymap",
            Page::EditorGeneral => "General",
            Page::EditorFont => "Font",
            Page::Commit => "Commit",
            Page::Mappings => "Directory Mappings",
            Page::Git => "Git",
            Page::GitHub => "GitHub",
            Page::GitLab => "GitLab",
            Page::Languages => "Languages & Frameworks",
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
            Page::Keymap => &["Shortcut", "Keyboard"],
            Page::EditorGeneral => &["Soft-wrap", "Show line numbers", "Show whitespaces", "Show indent guides", "Diff", "Merge"],
            Page::EditorFont => &["Font", "Size", "Line height"],
            Page::Commit => &[
                "Use non-modal commit interface",
                "Limit subject line length",
                "Run Git hooks",
                "Sign-off commit",
                "Clean up commit message",
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
            servers,
            git_test: None,
            github,
            gitlab,
            mappings,
            keymap: BTreeMap::new(),
            recording: None,
            intercept: None,
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
        if let Ok(size) = self.font_size.read(cx).value().trim().parse::<f32>() {
            next.editor_font_tenths = (size.clamp(8., 32.) * 10.).round() as u32;
        }
        // Keep what changed elsewhere meanwhile (tool window sizes, View Options).
        let mut current = Settings::get(cx).clone();
        copy_options(&next, &mut current);
        Settings::update(cx, |s| *s = current.clone());
        self.draft = current;
        crate::theme::refresh(cx);
        for (action, keys) in std::mem::take(&mut self.keymap) {
            keymap::set(action, keys, cx);
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
    to.language_servers = from.language_servers.clone();
    to.diff.soft_wrap = from.diff.soft_wrap;
    to.diff.show_line_numbers = from.diff.show_line_numbers;
    to.diff.show_whitespaces = from.diff.show_whitespaces;
    to.diff.show_indent_guides = from.diff.show_indent_guides;
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
                Page::Keymap => self.keymap_page(&query, cx).into_any_element(),
                Page::EditorGeneral => self.editor_general(&query, cx).into_any_element(),
                Page::EditorFont => self.editor_font(&query, cx).into_any_element(),
                Page::Commit => self.commit_page(&query, cx).into_any_element(),
                Page::Mappings => match &self.mappings {
                    Some(view) => view.clone().into_any_element(),
                    None => div().text_sm().text_color(palette.text_secondary).child("Open a project to map its directories").into_any_element(),
                },
                Page::Git => self.git_page(&query, window, cx).into_any_element(),
                Page::GitHub => self.github.clone().into_any_element(),
                Page::GitLab => self.gitlab.clone().into_any_element(),
                Page::Languages => self.languages(&query, cx).into_any_element(),
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

    fn editor_general(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft.diff;
        v_flex()
            .gap_2()
            .child(self.section("Diff and Merge Viewers", cx))
            .child(self.check("settings-soft-wrap", "Soft-wrap", d.soft_wrap, query, |s, v| s.diff.soft_wrap = v, cx))
            .child(self.check("settings-line-numbers", "Show line numbers", d.show_line_numbers, query, |s, v| s.diff.show_line_numbers = v, cx))
            .child(self.check("settings-whitespaces", "Show whitespaces", d.show_whitespaces, query, |s, v| s.diff.show_whitespaces = v, cx))
            .child(self.check("settings-indent-guides", "Show indent guides", d.show_indent_guides, query, |s, v| s.diff.show_indent_guides = v, cx))
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
    let view = cx.new(|cx| SettingsView::new(model, window, cx));
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

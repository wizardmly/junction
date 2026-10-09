//! Settings › Keymap: the main actions and their shortcuts, as IntelliJ's
//! keymap lists them. A shortcut can be changed, removed or reset; changes
//! are saved in `keymap.conf` (`action=keys|keys`, empty for none) and bound
//! over the defaults, which stay in the code.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gpui_kit::{App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, Keystroke, Unbind};

/// An action the Keymap page lists.
pub struct Entry {
    pub group: &'static str,
    pub name: &'static str,
    /// The registered action name (`namespace::Action`).
    pub action: &'static str,
}

const fn entry(group: &'static str, name: &'static str, action: &'static str) -> Entry {
    Entry { group, name, action }
}

pub const ENTRIES: &[Entry] = &[
    entry("Version Control", "Commit…", "workspace::CommitChanges"),
    entry("Version Control", "Push…", "workspace::PushChanges"),
    entry("Version Control", "Update Project…", "workspace::UpdateProject"),
    entry("Version Control", "Branches…", "workspace::ShowBranches"),
    entry("Version Control", "VCS Operations Popup", "workspace::VcsOperations"),
    entry("Version Control", "Stash Changes…", "workspace::StashChanges"),
    entry("Version Control", "Refresh", "workspace::Refresh"),
    entry("Navigate", "Go to File…", "workspace::GotoFile"),
    entry("Navigate", "Go to Class…", "workspace::GotoClass"),
    entry("Navigate", "Go to Symbol…", "workspace::GotoSymbol"),
    entry("Navigate", "Go to Line/Column…", "workspace::GotoLine"),
    entry("Navigate", "Back", "workspace::NavigateBack"),
    entry("Navigate", "Forward", "workspace::NavigateForward"),
    entry("Navigate", "Recent Files", "workspace::RecentFiles"),
    entry("Navigate", "File Structure", "workspace::FileStructure"),
    entry("Navigate", "Select in Project View", "workspace::SelectInProject"),
    entry("Navigate", "Search Everywhere", "workspace::SearchEverywhere"),
    entry("Navigate", "Find Action…", "workspace::FindAction"),
    entry("Find", "Find in Files…", "workspace::FindInPath"),
    entry("Find", "Replace in Files…", "workspace::ReplaceInPath"),
    entry("Find", "Next Occurrence", "workspace::NextOccurrence"),
    entry("Find", "Previous Occurrence", "workspace::PreviousOccurrence"),
    entry("Tool Windows", "Project", "workspace::ToggleProjectWindow"),
    entry("Tool Windows", "Commit", "workspace::ToggleCommitWindow"),
    entry("Tool Windows", "Git", "workspace::ToggleGitWindow"),
    entry("Tool Windows", "Find", "workspace::ToggleFindWindow"),
    entry("Tool Windows", "Hide All Tool Windows", "workspace::HideAllToolWindows"),
    entry("Tool Windows", "Hide Active Tool Window", "workspace::HideActiveToolWindow"),
    entry("Window", "Close Tab", "workspace::CloseTab"),
    entry("Window", "Select Next Tab", "workspace::NextTab"),
    entry("Window", "Select Previous Tab", "workspace::PreviousTab"),
    entry("Window", "Settings…", "workspace::OpenSettings"),
];

/// The default shortcuts (as bound at startup) and the user's changes.
#[derive(Default)]
pub struct Keymap {
    defaults: HashMap<&'static str, Vec<String>>,
    contexts: HashMap<&'static str, Option<Rc<KeyBindingContextPredicate>>>,
    overrides: BTreeMap<String, Vec<String>>,
}

impl Global for Keymap {}

fn path() -> Option<std::path::PathBuf> {
    Some(crate::settings::config_dir()?.join("keymap.conf"))
}

fn parse(text: &str) -> BTreeMap<String, Vec<String>> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.split('|').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()))
        .collect()
}

fn serialize(overrides: &BTreeMap<String, Vec<String>>) -> String {
    overrides.iter().map(|(k, v)| format!("{k}={}\n", v.join("|"))).collect()
}

/// Records the defaults and binds the saved changes; after every module
/// has bound its keys.
pub fn init(cx: &mut App) {
    let mut keymap = Keymap::default();
    {
        let bindings = cx.key_bindings();
        let bindings = bindings.borrow();
        for entry in ENTRIES {
            let Ok(action) = cx.build_action(entry.action, None) else { continue };
            let mut keys = Vec::new();
            for binding in bindings.bindings_for_action(&*action) {
                keys.push(binding.keystrokes().iter().map(|k| k.inner().unparse()).collect::<Vec<_>>().join(" "));
                keymap.contexts.entry(entry.action).or_insert_with(|| binding.predicate());
            }
            keys.dedup();
            keymap.defaults.insert(entry.action, keys);
        }
    }
    let overrides = path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default();
    cx.set_global(keymap);
    for (action, keys) in overrides {
        if let Some(entry) = ENTRIES.iter().find(|e| e.action == action) {
            set(entry.action, Some(keys), cx);
        }
    }
}

/// The action's shortcuts now.
pub fn shortcuts(action: &str, cx: &App) -> Vec<String> {
    let keymap = cx.global::<Keymap>();
    keymap.overrides.get(action).or_else(|| keymap.defaults.get(action)).cloned().unwrap_or_default()
}

pub fn is_default(action: &str, cx: &App) -> bool {
    !cx.global::<Keymap>().overrides.contains_key(action)
}

pub fn defaults(action: &str, cx: &App) -> Vec<String> {
    cx.global::<Keymap>().defaults.get(action).cloned().unwrap_or_default()
}

/// Gives `action` these shortcuts (`None`: back to the defaults): the old
/// ones are unbound for it, the new ones bound, and the change saved.
pub fn set(action: &'static str, keys: Option<Vec<String>>, cx: &mut App) {
    if !cx.has_global::<Keymap>() {
        return;
    }
    let Ok(built) = cx.build_action(action, None) else { return };
    let old = shortcuts(action, cx);
    let keymap = cx.global::<Keymap>();
    let context = keymap.contexts.get(action).cloned().flatten();
    let defaults = keymap.defaults.get(action).cloned().unwrap_or_default();
    let new = keys.clone().unwrap_or_else(|| defaults.clone());
    let mut bindings = Vec::new();
    for keys in old.iter().filter(|k| !new.contains(k)) {
        if let Ok(binding) = KeyBinding::load(keys, Box::new(Unbind(action.into())), context.clone(), false, None, &DummyKeyboardMapper) {
            bindings.push(binding);
        }
    }
    for keys in &new {
        if let Ok(binding) = KeyBinding::load(keys, built.boxed_clone(), context.clone(), false, None, &DummyKeyboardMapper) {
            bindings.push(binding);
        }
    }
    cx.bind_keys(bindings);
    let keymap = cx.global_mut::<Keymap>();
    match keys.filter(|k| *k != defaults) {
        Some(keys) => keymap.overrides.insert(action.to_owned(), keys),
        None => keymap.overrides.remove(action),
    };
    if let Some(path) = path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, serialize(&cx.global::<Keymap>().overrides));
    }
}

/// `ctrl-shift-k` as IntelliJ writes it: Ctrl+Shift+K (⇧⌘K on macOS);
/// a two-stroke shortcut joins its parts with ", ".
pub fn display(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let Ok(k) = Keystroke::parse(stroke) else { return stroke.to_owned() };
            let key = match k.key.as_str() {
                "left" => "Left".to_owned(),
                "right" => "Right".to_owned(),
                "up" => "Up".to_owned(),
                "down" => "Down".to_owned(),
                "escape" => "Escape".to_owned(),
                "enter" => "Enter".to_owned(),
                "space" => "Space".to_owned(),
                "backspace" => "Backspace".to_owned(),
                "delete" => "Delete".to_owned(),
                "tab" => "Tab".to_owned(),
                "pageup" => "Page Up".to_owned(),
                "pagedown" => "Page Down".to_owned(),
                "home" => "Home".to_owned(),
                "end" => "End".to_owned(),
                "insert" => "Insert".to_owned(),
                "shift" => "Shift".to_owned(),
                key => key.to_uppercase(),
            };
            let m = k.modifiers;
            if cfg!(target_os = "macos") {
                let mut text = String::new();
                for (on, sign) in [(m.control, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.platform, "⌘")] {
                    if on {
                        text.push_str(sign);
                    }
                }
                text + &key
            } else {
                let mut parts: Vec<&str> = Vec::new();
                for (on, name) in [(m.control, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.platform, "Meta")] {
                    if on {
                        parts.push(name);
                    }
                }
                parts.push(&key);
                parts.join("+")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips_and_displays() {
        let text = "workspace::CommitChanges=ctrl-alt-k|ctrl-k\nworkspace::PushChanges=\n";
        let parsed = super::parse(text);
        assert_eq!(parsed["workspace::CommitChanges"], vec!["ctrl-alt-k", "ctrl-k"]);
        assert!(parsed["workspace::PushChanges"].is_empty());
        assert_eq!(super::serialize(&parsed), text);
        if !cfg!(target_os = "macos") {
            assert_eq!(super::display("ctrl-shift-k"), "Ctrl+Shift+K");
            assert_eq!(super::display("shift shift"), "Shift, Shift");
            assert_eq!(super::display("alt-f1"), "Alt+F1");
        }
    }
}

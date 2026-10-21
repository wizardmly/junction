//! Logcat's filter field, as Android Studio's: completion of the query's
//! keys (`tag:`, `-tag:`, `tag~:`, `level:`…) and of their values from what
//! the log has shown (tags, packages, processes), and the filter history
//! with its favorites.

use std::ops::Range;

/// What the log has shown so far, for value completion.
#[derive(Default)]
pub struct Known<'a> {
    pub tags: &'a [String],
    pub packages: &'a [String],
    pub processes: &'a [String],
}

/// The suggestions for the term at the caret, and the text they replace.
#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    pub replace: Range<usize>,
    pub items: Vec<String>,
}

/// Keys whose value is text: each with Android Studio's variants.
const STRING_KEYS: &[&str] = &["line", "message", "package", "process", "tag"];
const LEVELS: &[&str] = &["assert", "debug", "error", "info", "verbose", "warn"];
const IS_VALUES: &[&str] = &["crash", "stacktrace"];

/// Every key the query understands, in the popup's order.
pub fn keys() -> Vec<String> {
    let mut out = Vec::new();
    for key in ["age", "is", "level", "line", "message", "name", "package", "process", "tag"] {
        if STRING_KEYS.contains(&key) {
            out.extend([format!("{key}:"), format!("-{key}:"), format!("{key}~:"), format!("-{key}~:"), format!("{key}=:"), format!("-{key}=:")]);
        } else {
            out.push(format!("{key}:"));
        }
    }
    out
}

/// The completion at `caret`: keys while a term without `:` is typed,
/// else the key's values. `invoked` (Ctrl+Space) also offers every key on
/// an empty term.
pub fn complete(query: &str, caret: usize, known: &Known, invoked: bool) -> Option<Completion> {
    let caret = caret.min(query.len());
    let start = query[..caret].rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let end = query[caret..].find(char::is_whitespace).map_or(query.len(), |i| caret + i);
    let word = &query[start..caret];
    let items: Vec<String>;
    let replace;
    if let Some(colon) = word.find(':') {
        let key = word[..colon].trim_start_matches('-').trim_end_matches(['~', '=']);
        let values: Vec<String> = match key {
            "tag" => known.tags.to_vec(),
            "package" => std::iter::once("mine".to_owned()).chain(known.packages.iter().cloned()).collect(),
            "process" => known.processes.to_vec(),
            "level" => LEVELS.iter().map(|s| s.to_string()).collect(),
            "is" => IS_VALUES.iter().map(|s| s.to_string()).collect(),
            _ => return None,
        };
        let typed = word[colon + 1..].trim_start_matches('"');
        items = ranked(values, typed);
        replace = start + colon + 1..end;
    } else {
        if word.is_empty() && !invoked {
            return None;
        }
        let typed = word.to_lowercase();
        items = keys().into_iter().filter(|k| k.starts_with(&typed)).collect();
        replace = start..caret;
    }
    // Nothing left to complete: the term is what the one item says.
    if items.is_empty() || items.len() == 1 && query[replace.clone()].trim_matches('"') == items[0].trim_matches('"') {
        return None;
    }
    Some(Completion { replace, items })
}

/// The values matching what's typed: those starting with it first, then
/// those containing it, ignoring case; quoted when they have spaces.
fn ranked(mut values: Vec<String>, typed: &str) -> Vec<String> {
    values.sort_by_key(|v| v.to_lowercase());
    values.dedup();
    let typed = typed.to_lowercase();
    let (mut first, mut rest) = (Vec::new(), Vec::new());
    for v in values {
        let lower = v.to_lowercase();
        if lower.starts_with(&typed) {
            first.push(v);
        } else if !typed.is_empty() && lower.contains(&typed) {
            rest.push(v);
        }
    }
    first.extend(rest);
    first.into_iter().map(|v| if v.contains(char::is_whitespace) { format!("\"{v}\"") } else { v }).collect()
}

/// Android Studio keeps the 20 latest filters besides the favorites.
pub const HISTORY_SIZE: usize = 20;

/// The filter history: favorites (starred), then the latest filters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
}

impl History {
    /// A filter that was applied: first in the history, unless starred.
    pub fn remember(&mut self, filter: &str) {
        let filter = filter.trim();
        if filter.is_empty() || self.favorites.iter().any(|f| f == filter) {
            return;
        }
        self.recent.retain(|f| f != filter);
        self.recent.insert(0, filter.to_owned());
        self.recent.truncate(HISTORY_SIZE);
    }

    pub fn is_favorite(&self, filter: &str) -> bool {
        self.favorites.iter().any(|f| f == filter.trim())
    }

    /// The field's star: a favorite stops being one and goes back to the
    /// history.
    pub fn toggle_favorite(&mut self, filter: &str) {
        let filter = filter.trim();
        if filter.is_empty() {
            return;
        }
        if self.is_favorite(filter) {
            self.favorites.retain(|f| f != filter);
            self.remember(filter);
        } else {
            self.recent.retain(|f| f != filter);
            self.favorites.push(filter.to_owned());
        }
    }

    pub fn remove(&mut self, filter: &str) {
        self.favorites.retain(|f| f != filter);
        self.recent.retain(|f| f != filter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> (Vec<String>, Vec<String>, Vec<String>) {
        (vec!["OkHttp".into(), "ActivityManager".into(), "MainActivity".into()], vec!["com.example.app".into(), "com.android.systemui".into()], vec!["system_server".into()])
    }

    #[test]
    fn completes_keys() {
        let (tags, packages, processes) = known();
        let known = Known { tags: &tags, packages: &packages, processes: &processes };
        let c = complete("level:warn ta", 13, &known, false).unwrap();
        assert_eq!(c.replace, 11..13);
        assert_eq!(c.items, ["tag:", "tag~:", "tag=:"]);
        let c = complete("-p", 2, &known, false).unwrap();
        assert_eq!(c.items, ["-package:", "-package~:", "-package=:", "-process:", "-process~:", "-process=:"]);
        // Nothing typed: only when asked.
        assert_eq!(complete("level:warn ", 11, &known, false), None);
        assert_eq!(complete("", 0, &known, true).unwrap().items.len(), keys().len());
        assert_eq!(keys()[..4], ["age:", "is:", "level:", "line:"]);
    }

    #[test]
    fn completes_values() {
        let (tags, packages, processes) = known();
        let known = Known { tags: &tags, packages: &packages, processes: &processes };
        // Right after the colon: every value.
        let c = complete("tag:", 4, &known, false).unwrap();
        assert_eq!((c.replace.clone(), c.items.clone()), (4..4, vec!["ActivityManager".to_owned(), "MainActivity".into(), "OkHttp".into()]));
        // Starting with it first, then containing it.
        let c = complete("-tag~:act level:warn", 9, &known, false).unwrap();
        assert_eq!(c.replace, 6..9);
        assert_eq!(c.items, ["ActivityManager", "MainActivity"]);
        assert_eq!(complete("package:", 8, &known, false).unwrap().items, ["com.android.systemui", "com.example.app", "mine"]);
        assert_eq!(complete("level:w", 7, &known, false).unwrap().items, ["warn"]);
        assert_eq!(complete("is:", 3, &known, false).unwrap().items, ["crash", "stacktrace"]);
        // Complete already, or a key without values.
        assert_eq!(complete("level:warn", 10, &known, false), None);
        assert_eq!(complete("age:5", 5, &known, false), None);
        let spaced = vec!["My Tag".to_owned()];
        assert_eq!(complete("tag:my", 6, &Known { tags: &spaced, ..Known::default() }, false).unwrap().items, ["\"My Tag\""]);
    }

    #[test]
    fn keeps_a_history_with_favorites() {
        let mut h = History::default();
        for f in ["package:mine", "level:warn", "package:mine ", " "] {
            h.remember(f);
        }
        assert_eq!(h.recent, ["package:mine", "level:warn"]);
        h.toggle_favorite("level:warn");
        assert_eq!((h.favorites.clone(), h.recent.clone()), (vec!["level:warn".to_owned()], vec!["package:mine".to_owned()]));
        // A favorite isn't repeated in the history.
        h.remember("level:warn");
        assert_eq!(h.recent, ["package:mine"]);
        h.toggle_favorite("level:warn");
        assert_eq!((h.favorites.len(), h.recent[0].as_str()), (0, "level:warn"));
        for i in 0..30 {
            h.remember(&format!("tag:{i}"));
        }
        assert_eq!((h.recent.len(), h.recent[0].as_str()), (HISTORY_SIZE, "tag:29"));
        h.remove("tag:29");
        assert_eq!(h.recent[0], "tag:28");
    }
}

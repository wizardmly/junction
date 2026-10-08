//! Spell checking for commit messages, as IntelliJ's Typo inspection does:
//! a bundled English word list (SCOWL, see assets/dict), common development
//! terms, and the user's own dictionary (Save to Dictionary).

use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Mutex, OnceLock};

const WORDS: &str = include_str!("../assets/dict/en_US.txt");

/// Words of the trade the general list lacks.
const TECH: &str = "api apis app apps async auth authn authz backend backport backports blob blobs bool boolean bugfix bugfixes \
    changelog changelogs checkbox checkboxes checkout checkouts cli config configs const css csv dataset datasets debounce \
    dedupe deduplicate deps dev devs dialogs dir dirs dockerfile dropdown dropdowns enum enums env failover fixup fixups \
    frontend func gitignore gradle hardcode hardcoded hotfix hotfixes html http https impl init inline inlined io ios iter \
    json jvm kotlin linter localhost lockfile lookup lookups macos metadata middleware mutex namespace namespaces navbar \
    nullable oauth param params parsable plugin plugins popup popups prepend prepended presubmit proto readme readonly \
    rebase rebased rebases rebasing refactor refactored refactoring refactors regex regexes repo repos runtime rustfmt \
    sdk serde serializer setter sidebar sql src stderr stdin stdout struct structs subcommand submodule submodules \
    subtree sudo symlink symlinks sync timestamp timestamps todo todos toml tooltip tooltips typo typos ui unescape \
    unmerged unpushed unstage unstaged unstash untracked uri url urls usb utf uuid validator vcs vendored viewport \
    webhook webhooks whitespace widget widgets wip workflow workflows workspace workspaces xml yaml yml";

/// The word list leaves out words with apostrophes.
const CONTRACTIONS: &str = "don't doesn't didn't isn't aren't wasn't weren't can't couldn't won't wouldn't shouldn't haven't \
    hasn't hadn't mustn't needn't it's that's there's here's what's who's let's i'm you're we're they're i've we've you've \
    they've i'll we'll you'll it'll they'll i'd we'd you'd they'd";

fn dictionary() -> &'static Mutex<HashSet<String>> {
    static DICT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut set: HashSet<String> = WORDS.lines().chain(TECH.split_whitespace()).chain(CONTRACTIONS.split_whitespace()).map(str::to_owned).collect();
        set.extend(user_words());
        Mutex::new(set)
    })
}

fn user_path() -> Option<std::path::PathBuf> {
    Some(crate::settings::config_dir()?.join("dictionary.txt"))
}

fn user_words() -> Vec<String> {
    user_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

pub fn is_known(word: &str) -> bool {
    let lower = word.to_lowercase();
    let set = dictionary().lock().unwrap();
    set.contains(&lower) || lower.strip_suffix("'s").is_some_and(|w| set.contains(w))
}

/// Save to Dictionary: the word is known from now on, here and next time.
pub fn add_word(word: &str) {
    let word = word.to_lowercase();
    dictionary().lock().unwrap().insert(word.clone());
    if let Some(path) = user_path() {
        let mut words = user_words();
        if !words.contains(&word) {
            words.push(word);
            words.sort();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, words.join("\n") + "\n");
        }
    }
}

/// Byte ranges of the misspelled words. Code-like tokens are skipped:
/// `quoted` spans, URLs and paths, identifiers with digits, underscores or
/// inner capitals, ALL-CAPS abbreviations and words of fewer than 3 letters.
pub fn misspelled(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut chunk_start = 0;
    // Whitespace-separated chunks first: a chunk that looks like a URL or path is skipped whole.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i <= bytes.len() {
        let at_end = i == bytes.len();
        if at_end || (bytes[i] as char).is_whitespace() {
            if i > chunk_start {
                let chunk = &text[chunk_start..i];
                let ticks = chunk.matches('`').count();
                let code_like = in_code || chunk.contains("://") || chunk.contains('/') || chunk.contains('\\') || chunk.contains('_')
                    || chunk.contains('@') || chunk.contains('#') || chunk.contains("::")
                    || chunk.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']).contains('.');
                if !code_like && ticks == 0 {
                    words_in(chunk, chunk_start, &mut out);
                }
                if ticks % 2 == 1 {
                    in_code = !in_code;
                }
            }
            chunk_start = i + 1;
        }
        i += 1;
    }
    out
}

fn words_in(chunk: &str, offset: usize, out: &mut Vec<Range<usize>>) {
    let mut start: Option<usize> = None;
    let mut chars = chunk.char_indices().peekable();
    while let Some((ix, c)) = chars.next() {
        // An apostrophe between letters is part of the word ("don't").
        let inner_apostrophe = c == '\'' && start.is_some() && chars.peek().is_some_and(|(_, n)| n.is_alphabetic());
        if c.is_alphanumeric() || inner_apostrophe {
            start.get_or_insert(ix);
        } else if let Some(s) = start.take() {
            check(&chunk[s..ix], offset + s, out);
        }
    }
    if let Some(s) = start {
        check(&chunk[s..], offset + s, out);
    }
}

fn check(word: &str, at: usize, out: &mut Vec<Range<usize>>) {
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 3 || word.chars().any(|c| c.is_ascii_digit()) || !word.is_ascii() {
        return;
    }
    // camelCase / PascalCase identifiers and ABBREVIATIONS.
    if word.chars().skip(1).any(|c| c.is_uppercase()) {
        return;
    }
    if !is_known(word) {
        out.push(at..at + word.len());
    }
}

/// Up to `limit` known words one or two edits away, closest first: swapped
/// letters (the commonest typo), then the same first letter and length.
pub fn suggestions(word: &str, limit: usize) -> Vec<String> {
    let lower = word.to_lowercase();
    let set = dictionary().lock().unwrap();
    let mut found: Vec<(u8, String)> = edits(&lower).into_iter().filter(|(_, w)| set.contains(w)).collect();
    if found.is_empty() && lower.len() <= 12 {
        found = edits(&lower)
            .iter()
            .flat_map(|(_, e)| edits(e))
            .filter(|(_, w)| set.contains(w))
            .map(|(_, w)| (2, w))
            .collect();
    }
    let first = lower.chars().next();
    found.sort_by(|(ka, a), (kb, b)| {
        let key = |k: &u8, w: &String| (*k, w.chars().next() != first, w.len().abs_diff(lower.len()));
        key(ka, a).cmp(&key(kb, b)).then_with(|| a.cmp(b))
    });
    let mut seen = HashSet::new();
    found.retain(|(_, w)| seen.insert(w.clone()));
    found.truncate(limit);
    // Keep the word's capitalization.
    let capital = word.chars().next().is_some_and(char::is_uppercase);
    found
        .into_iter()
        .map(|(_, w)| if capital { w[..1].to_uppercase() + &w[1..] } else { w })
        .collect()
}

/// Every string one edit away, with its kind: 0 a swap of neighbours, 1 the rest.
fn edits(word: &str) -> Vec<(u8, String)> {
    let letters = "abcdefghijklmnopqrstuvwxyz";
    let chars: Vec<char> = word.chars().collect();
    let mut out = Vec::new();
    for i in 0..=chars.len() {
        let (a, b) = chars.split_at(i);
        let a: String = a.iter().collect();
        if !b.is_empty() {
            out.push((1, format!("{a}{}", b[1..].iter().collect::<String>())));
        }
        if b.len() > 1 {
            out.push((0, format!("{a}{}{}{}", b[1], b[0], b[2..].iter().collect::<String>())));
        }
        for l in letters.chars() {
            if !b.is_empty() {
                out.push((1, format!("{a}{l}{}", b[1..].iter().collect::<String>())));
            }
            out.push((1, format!("{a}{l}{}", b.iter().collect::<String>())));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_typos_but_not_code() {
        let text = "Fix teh login flow in `parseConfig` for https://x.io/a_b and README.md (refactor, don't retry)";
        let found: Vec<&str> = misspelled(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(found, vec!["teh"]);
        assert!(misspelled("Add HTTP2 support to fooBar and snake_case").is_empty());
        assert!(suggestions("teh", 5).contains(&"the".to_owned()));
        assert_eq!(suggestions("Recieve", 3).first().map(String::as_str), Some("Receive"));
    }
}

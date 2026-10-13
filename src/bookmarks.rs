//! Bookmarks, as IntelliJ's: a line of a file, optionally with a mnemonic
//! (0–9, A–Z). Kept across sessions in the settings folder, keyed by the
//! file's absolute path; each remembers its line's text so it follows the
//! line when lines above it are added or removed.

use std::path::PathBuf;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    /// The file's absolute path.
    pub file: String,
    /// 0-based line.
    pub line: u32,
    #[serde(default)]
    pub mnemonic: Option<char>,
    /// The line's text when it was last placed, to find it again.
    #[serde(default)]
    pub text: String,
}

/// The mnemonics IntelliJ offers, in its chooser's order.
pub const MNEMONICS: &str = "1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ";

#[derive(Default)]
pub struct Bookmarks {
    pub list: Vec<Bookmark>,
}

impl Global for Bookmarks {}

fn store_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("bookmarks.json"))
}

impl Bookmarks {
    pub fn init(cx: &mut App) {
        let list = store_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        cx.set_global(Bookmarks { list });
    }

    pub fn get(cx: &App) -> &Bookmarks {
        cx.try_global::<Bookmarks>().unwrap_or_else(|| {
            static EMPTY: Bookmarks = Bookmarks { list: Vec::new() };
            &EMPTY
        })
    }

    /// Changes the bookmarks and saves them.
    pub fn update(cx: &mut App, f: impl FnOnce(&mut Bookmarks)) {
        if cx.try_global::<Bookmarks>().is_none() {
            cx.set_global(Bookmarks::default());
        }
        let bookmarks = cx.global_mut::<Bookmarks>();
        f(bookmarks);
        if let (Some(path), Ok(json)) = (store_path(), serde_json::to_vec_pretty(&bookmarks.list)) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, json);
        }
    }

    pub fn of_file<'a>(&'a self, file: &'a str) -> impl Iterator<Item = &'a Bookmark> + 'a {
        self.list.iter().filter(move |b| b.file == file)
    }

    pub fn at(&self, file: &str, line: u32) -> Option<&Bookmark> {
        self.list.iter().find(|b| b.file == file && b.line == line)
    }

    pub fn with_mnemonic(&self, mnemonic: char) -> Option<&Bookmark> {
        self.list.iter().find(|b| b.mnemonic == Some(mnemonic))
    }

    /// Toggle Bookmark (F11): adds a plain bookmark, or removes the line's.
    pub fn toggle(&mut self, file: &str, line: u32, text: &str) {
        if let Some(i) = self.list.iter().position(|b| b.file == file && b.line == line) {
            self.list.remove(i);
        } else {
            self.list.push(Bookmark { file: file.to_owned(), line, mnemonic: None, text: text.trim().to_owned() });
        }
    }

    /// Puts a bookmark with `mnemonic` on a line (`None`: a plain one);
    /// another bookmark with the same mnemonic gives it up, as in IntelliJ.
    pub fn set(&mut self, file: &str, line: u32, text: &str, mnemonic: Option<char>) {
        if let Some(m) = mnemonic {
            self.list.retain(|b| b.mnemonic != Some(m) || (b.file == file && b.line == line));
        }
        match self.list.iter_mut().find(|b| b.file == file && b.line == line) {
            Some(b) => b.mnemonic = mnemonic,
            None => self.list.push(Bookmark { file: file.to_owned(), line, mnemonic, text: text.trim().to_owned() }),
        }
    }

    pub fn remove(&mut self, file: &str, line: u32) {
        self.list.retain(|b| !(b.file == file && b.line == line));
    }

    /// After an edit: each of the file's bookmarks whose line no longer
    /// holds its text moves to the nearest line that does. Returns whether
    /// anything moved.
    pub fn follow(&mut self, file: &str, lines: &[&str]) -> bool {
        let mut moved = false;
        for b in self.list.iter_mut().filter(|b| b.file == file && !b.text.is_empty()) {
            let at = b.line as usize;
            if lines.get(at).is_some_and(|l| l.trim() == b.text) {
                continue;
            }
            let found = (1..=lines.len()).find_map(|d| {
                [at.checked_sub(d), Some(at + d)].into_iter().flatten().find(|&l| lines.get(l).is_some_and(|t| t.trim() == b.text))
            });
            match found {
                Some(line) => {
                    b.line = line as u32;
                    moved = true;
                }
                // The line is gone or changed: the bookmark stays, clamped.
                None if at >= lines.len() && !lines.is_empty() => {
                    b.line = lines.len() as u32 - 1;
                    moved = true;
                }
                None => {}
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_mnemonics_and_follows_lines() {
        let mut b = Bookmarks::default();
        b.toggle("/a.kt", 2, "  val x = 1 ");
        assert_eq!(b.at("/a.kt", 2).unwrap().text, "val x = 1");
        b.toggle("/a.kt", 2, "");
        assert!(b.at("/a.kt", 2).is_none());
        b.set("/a.kt", 1, "fun a()", Some('1'));
        b.set("/b.kt", 4, "fun b()", Some('1'));
        assert_eq!(b.with_mnemonic('1').unwrap().file, "/b.kt");
        assert_eq!(b.list.len(), 1);
        // Two lines inserted above: the bookmark moves down with its line.
        assert!(b.follow("/b.kt", &["x", "y", "", "", "", "", "fun b()"]));
        assert_eq!(b.at("/b.kt", 6).unwrap().mnemonic, Some('1'));
    }
}

//! Navigation history, as IntelliJ's IdeDocumentHistory keeps it: Back /
//! Forward, recent files, and Recent Locations' visited and changed places.

/// A place in a file: (path, line, column).
pub type Position = (String, u32, u32);

/// Navigate › Back / Forward, as IntelliJ keeps it.
#[derive(Default)]
pub struct NavHistory {
    back: Vec<Position>,
    forward: Vec<Position>,
}

impl NavHistory {
    /// Remembers `here` for Back, unless it is already the last entry.
    pub fn remember(&mut self, here: Position) {
        if self.back.last() != Some(&here) {
            self.back.push(here);
        }
    }

    /// A jump to a new place: remembers where we were and forgets Forward.
    pub fn jumped_from(&mut self, here: Option<Position>) {
        if let Some(here) = here {
            self.remember(here);
        }
        self.forward.clear();
    }

    /// The place to go Back to, if any; `here` becomes the next Forward.
    pub fn back(&mut self, here: Option<Position>) -> Option<Position> {
        let target = self.back.pop()?;
        self.forward.extend(here);
        Some(target)
    }

    /// The place to go Forward to, if any; `here` goes back on the Back list.
    pub fn forward(&mut self, here: Option<Position>) -> Option<Position> {
        let target = self.forward.pop()?;
        self.back.extend(here);
        Some(target)
    }
}

/// Paths used recently, newest first, without repeats.
#[derive(Default)]
pub struct RecentPaths {
    paths: Vec<String>,
    limit: Option<usize>,
}

impl RecentPaths {
    pub fn with_limit(limit: usize) -> Self {
        Self { paths: Vec::new(), limit: Some(limit) }
    }

    /// Moves `path` to the front.
    pub fn touch(&mut self, path: &str) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_owned());
        if let Some(limit) = self.limit {
            self.paths.truncate(limit);
        }
    }
}

impl std::ops::Deref for RecentPaths {
    type Target = [String];

    fn deref(&self) -> &[String] {
        &self.paths
    }
}

/// A place for Recent Locations (Ctrl+Shift+E): visited, or also changed.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Place {
    pub path: String,
    pub line: u32,
    pub col: u32,
    #[serde(default)]
    pub changed: bool,
}

/// Recent Locations' places, newest first: a place near one already listed
/// (a few lines apart in the same file) replaces it, as IntelliJ merges them.
#[derive(Default)]
pub struct RecentPlaces {
    places: Vec<Place>,
}

impl RecentPlaces {
    const LIMIT: usize = 50;
    const NEAR: u32 = 2;

    /// Places read back from a saved session, newest first.
    pub fn restore(mut places: Vec<Place>) -> Self {
        places.truncate(Self::LIMIT);
        Self { places }
    }

    pub fn visit(&mut self, (path, line, col): Position) {
        self.add(Place { path, line, col, changed: false });
    }

    /// A visit of `here`, if there is one.
    pub fn extend_visit(&mut self, here: Option<Position>) {
        if let Some(here) = here {
            self.visit(here);
        }
    }

    /// Lines were added (`delta` > 0) or removed in `path` from `from` on:
    /// the places below follow, as IntelliJ's markers move with the text;
    /// places in removed lines go to where the removal was.
    pub fn shift(&mut self, path: &str, from: u32, delta: i64) {
        for p in self.places.iter_mut().filter(|p| p.path == path && p.line > from) {
            p.line = (p.line as i64 + delta).max(from as i64) as u32;
        }
    }

    /// Recent Locations' Delete: the place at `line` of `path` is forgotten.
    pub fn remove(&mut self, path: &str, line: u32) {
        self.places.retain(|p| !(p.path == path && p.line == line));
    }

    pub fn change(&mut self, (path, line, col): Position) {
        self.add(Place { path, line, col, changed: true });
    }

    fn add(&mut self, mut place: Place) {
        self.places.retain(|p| {
            let near = p.path == place.path && p.line.abs_diff(place.line) <= Self::NEAR;
            place.changed |= near && p.changed;
            !near
        });
        self.places.insert(0, place);
        self.places.truncate(Self::LIMIT);
    }
}

impl std::ops::Deref for RecentPlaces {
    type Target = [Place];

    fn deref(&self) -> &[Place] {
        &self.places
    }
}

/// Recent Locations' code around a place: the line with one before and
/// one after, their common indent removed, and the first one's 0-based
/// number.
pub fn snippet(text: &str, line: u32) -> (u32, String) {
    let lines: Vec<&str> = text.lines().collect();
    let at = (line as usize).min(lines.len().saturating_sub(1));
    let (from, to) = (at.saturating_sub(1), (at + 2).min(lines.len()));
    let chunk = lines.get(from..to).unwrap_or_default();
    let indent = chunk.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let lines: Vec<&str> = chunk.iter().map(|l| l.get(indent..).unwrap_or(l.trim_start())).collect();
    (from as u32, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, line: u32) -> Position {
        (path.to_owned(), line, 0)
    }

    #[test]
    fn removes_a_place() {
        let mut places = RecentPlaces::default();
        places.visit(at("a", 1));
        places.change(at("b", 9));
        places.remove("a", 1);
        assert_eq!(places.iter().map(|p| (p.path.as_str(), p.line, p.changed)).collect::<Vec<_>>(), [("b", 9, true)]);
    }

    #[test]
    fn back_and_forward() {
        let mut nav = NavHistory::default();
        assert_eq!(nav.back(Some(at("a", 1))), None);
        nav.jumped_from(Some(at("a", 1)));
        nav.jumped_from(Some(at("b", 2)));
        assert_eq!(nav.back(Some(at("c", 3))), Some(at("b", 2)));
        assert_eq!(nav.back(Some(at("b", 2))), Some(at("a", 1)));
        assert_eq!(nav.back(Some(at("a", 1))), None);
        assert_eq!(nav.forward(Some(at("a", 1))), Some(at("b", 2)));
        assert_eq!(nav.forward(Some(at("b", 2))), Some(at("c", 3)));
        assert_eq!(nav.forward(None), None);
        // A new jump forgets Forward.
        assert_eq!(nav.back(Some(at("c", 3))), Some(at("b", 2)));
        nav.jumped_from(Some(at("b", 2)));
        assert_eq!(nav.forward(Some(at("d", 4))), None);
    }

    #[test]
    fn remember_skips_repeats() {
        let mut nav = NavHistory::default();
        nav.remember(at("a", 1));
        nav.remember(at("a", 1));
        nav.jumped_from(Some(at("a", 1)));
        assert_eq!(nav.back(None), Some(at("a", 1)));
        assert_eq!(nav.back(None), None);
    }

    #[test]
    fn recent_paths_move_to_front_and_cap() {
        let mut recent = RecentPaths::with_limit(3);
        for p in ["a", "b", "c", "a", "d"] {
            recent.touch(p);
        }
        assert_eq!(&*recent, ["d", "a", "c"]);
        let mut unlimited = RecentPaths::default();
        for i in 0..60 {
            unlimited.touch(&i.to_string());
        }
        assert_eq!(unlimited.len(), 60);
    }

    #[test]
    fn recent_places_merge_nearby() {
        let mut places = RecentPlaces::default();
        places.visit(at("a", 10));
        places.change(at("b", 3));
        places.visit(at("a", 11));
        places.visit(at("b", 4));
        places.visit(at("b", 20));
        let got: Vec<(&str, u32, bool)> = places.iter().map(|p| (p.path.as_str(), p.line, p.changed)).collect();
        assert_eq!(got, [("b", 20, false), ("b", 4, true), ("a", 11, false)]);
    }

    #[test]
    fn snippets_around_a_line() {
        let text = "fun f() {\n    val a = 1\n    val b = 2\n\n    g()\n}\n";
        assert_eq!(snippet(text, 2), (1, "val a = 1\nval b = 2\n".to_owned()));
        assert_eq!(snippet(text, 0), (0, "fun f() {\n    val a = 1".to_owned()));
        assert_eq!(snippet(text, 99), (4, "    g()\n}".to_owned()));
    }

    #[test]
    fn places_follow_lines_added_and_removed() {
        let mut places = RecentPlaces::default();
        places.visit(at("a", 10));
        places.visit(at("a", 2));
        places.visit(at("b", 10));
        places.shift("a", 5, 3);
        let lines = |p: &RecentPlaces| p.iter().map(|p| (p.path.clone(), p.line)).collect::<Vec<_>>();
        assert_eq!(lines(&places), [("b".into(), 10), ("a".into(), 2), ("a".into(), 13)]);
        // Removing lines 6..=14 from line 5: the place inside goes to 5.
        places.shift("a", 5, -9);
        assert_eq!(lines(&places), [("b".into(), 10), ("a".into(), 2), ("a".into(), 5)]);
    }
}

//! Navigation history and recent-file lists, kept free of UI types so they
//! can be tested on their own.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, line: u32) -> Position {
        (path.to_owned(), line, 0)
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
}

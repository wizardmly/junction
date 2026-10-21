//! Code coverage, as Android Studio's Coverage tool window and editor show
//! it after Run with Coverage: per package and class the share of classes,
//! methods, lines and branches the tests reached ("Class, %", "Method, %",
//! "Line, %", "Branch, %"), and per source line whether it was fully,
//! partly or not covered (the green, yellow and red gutter stripes).

use std::collections::BTreeMap;

/// Covered against all of something (lines, branches…).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counter {
    pub missed: u32,
    pub covered: u32,
}

impl Counter {
    pub fn new(missed: u32, covered: u32) -> Self {
        Self { missed, covered }
    }

    pub fn total(&self) -> u32 {
        self.missed + self.covered
    }

    /// Rounded down, as IntelliJ's `CoverageViewManager` shows it.
    pub fn percent(&self) -> u32 {
        if self.total() == 0 {
            100
        } else {
            (self.covered as u64 * 100 / self.total() as u64) as u32
        }
    }

    /// `50% (1/2)`; empty when there is nothing to count.
    pub fn label(&self) -> String {
        if self.total() == 0 {
            String::new()
        } else {
            format!("{}% ({}/{})", self.percent(), self.covered, self.total())
        }
    }

    fn add(&mut self, other: Counter) {
        self.missed += other.missed;
        self.covered += other.covered;
    }
}

/// How much of a line ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineCoverage {
    Full,
    Partial,
    None,
}

impl LineCoverage {
    /// From JaCoCo's missed and covered instructions and branches of a line.
    pub fn of(missed_instructions: u32, covered_instructions: u32, missed_branches: u32) -> Self {
        if covered_instructions == 0 {
            Self::None
        } else if missed_instructions > 0 || missed_branches > 0 {
            Self::Partial
        } else {
            Self::Full
        }
    }
}

/// A class's counters. `name` is the JVM name with dots (`demo.Calc$Inner`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassCoverage {
    pub name: String,
    /// The source file's name (`Calc.java`), when compiled with one.
    pub source: Option<String>,
    pub methods: Counter,
    pub lines: Counter,
    pub branches: Counter,
}

impl ClassCoverage {
    pub fn covered(&self) -> bool {
        self.methods.covered > 0
    }

    pub fn package(&self) -> &str {
        self.name.rsplit_once('.').map(|(p, _)| p).unwrap_or("")
    }

    pub fn simple_name(&self) -> &str {
        self.name.rsplit_once('.').map(|(_, n)| n).unwrap_or(&self.name)
    }
}

/// The lines of a source file, by 1-based line number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileCoverage {
    /// Package path and name, as JaCoCo knows it: `demo/Calc.java`.
    pub path: String,
    pub lines: BTreeMap<u32, LineCoverage>,
}

/// A package's row: its classes' counters summed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageCoverage {
    pub name: String,
    pub classes: Counter,
    pub methods: Counter,
    pub lines: Counter,
    pub branches: Counter,
}

/// One coverage run's results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageReport {
    /// Shown as the tab's title: the run configuration's name.
    pub name: String,
    pub classes: Vec<ClassCoverage>,
    pub files: Vec<FileCoverage>,
}

impl CoverageReport {
    /// The packages, by name, with their classes' counters summed; a class
    /// counts as covered when any of its methods ran.
    pub fn packages(&self) -> Vec<PackageCoverage> {
        let mut by_name: BTreeMap<&str, PackageCoverage> = BTreeMap::new();
        for class in &self.classes {
            let package = by_name.entry(class.package()).or_insert_with(|| PackageCoverage { name: class.package().to_owned(), ..Default::default() });
            package.classes.add(if class.covered() { Counter::new(0, 1) } else { Counter::new(1, 0) });
            package.methods.add(class.methods);
            package.lines.add(class.lines);
            package.branches.add(class.branches);
        }
        by_name.into_values().collect()
    }

    /// The classes of a package, outer classes first.
    pub fn classes_in<'a>(&'a self, package: &'a str) -> impl Iterator<Item = &'a ClassCoverage> + 'a {
        self.classes.iter().filter(move |c| c.package() == package)
    }

    /// The covered lines of a file, found by the end of its path
    /// (`…/src/main/java/demo/Calc.java` matches `demo/Calc.java`).
    pub fn file(&self, path: &str) -> Option<&FileCoverage> {
        let path = path.replace('\\', "/");
        self.files.iter().filter(|f| path.ends_with(&f.path) && (path.len() == f.path.len() || path[..path.len() - f.path.len()].ends_with('/'))).max_by_key(|f| f.path.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, methods: (u32, u32), lines: (u32, u32)) -> ClassCoverage {
        ClassCoverage { name: name.into(), source: None, methods: Counter::new(methods.0, methods.1), lines: Counter::new(lines.0, lines.1), branches: Counter::default() }
    }

    #[test]
    fn labels_round_down() {
        assert_eq!(Counter::new(2, 1).label(), "33% (1/3)");
        assert_eq!(Counter::new(0, 4).label(), "100% (4/4)");
        assert_eq!(Counter::default().label(), "");
        assert_eq!(LineCoverage::of(0, 3, 0), LineCoverage::Full);
        assert_eq!(LineCoverage::of(0, 3, 1), LineCoverage::Partial);
        assert_eq!(LineCoverage::of(3, 0, 2), LineCoverage::None);
    }

    #[test]
    fn sums_packages_and_finds_files() {
        let report = CoverageReport {
            name: "CalcTest".into(),
            classes: vec![class("demo.Calc", (1, 2), (2, 4)), class("demo.Unused", (1, 0), (3, 0)), class("demo.util.Strings", (0, 1), (0, 1))],
            files: vec![FileCoverage { path: "demo/Calc.java".into(), lines: BTreeMap::from([(3, LineCoverage::Full)]) }],
        };
        let packages = report.packages();
        assert_eq!(packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["demo", "demo.util"]);
        assert_eq!(packages[0].classes.label(), "50% (1/2)");
        assert_eq!(packages[0].methods.label(), "50% (2/4)");
        assert_eq!(packages[0].lines.label(), "44% (4/9)");
        assert_eq!(report.classes_in("demo").count(), 2);
        assert!(report.file("/p/app/src/main/java/demo/Calc.java").is_some());
        assert!(report.file("C:\\p\\src\\main\\java\\demo\\Calc.java").is_some());
        assert!(report.file("/p/src/main/java/notdemo/Calc.java").is_none());
    }
}

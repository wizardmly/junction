//! JaCoCo's XML report, read into the domain's coverage: the counters of
//! each class and the coverage of each source line.

use crate::domain::coverage::{ClassCoverage, Counter, CoverageReport, FileCoverage, LineCoverage};

/// None for something that isn't a JaCoCo report.
pub fn parse(xml: &str) -> Option<CoverageReport> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, options).ok()?;
    let root = doc.root_element();
    if !root.has_tag_name("report") {
        return None;
    }
    let mut report = CoverageReport { name: root.attribute("name").unwrap_or_default().to_owned(), ..Default::default() };
    for package in root.descendants().filter(|n| n.has_tag_name("package")) {
        let package_path = package.attribute("name").unwrap_or_default();
        for node in package.children().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "class" => {
                    let counter = |kind: &str| counter(node, kind);
                    report.classes.push(ClassCoverage {
                        name: node.attribute("name").unwrap_or_default().replace('/', "."),
                        source: node.attribute("sourcefilename").map(str::to_owned),
                        methods: counter("METHOD"),
                        lines: counter("LINE"),
                        branches: counter("BRANCH"),
                    });
                }
                "sourcefile" => {
                    let name = node.attribute("name").unwrap_or_default();
                    let path = if package_path.is_empty() { name.to_owned() } else { format!("{package_path}/{name}") };
                    let number = |line: roxmltree::Node, attr: &str| line.attribute(attr).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                    let lines = node
                        .children()
                        .filter(|n| n.has_tag_name("line"))
                        .filter_map(|line| {
                            let nr = number(line, "nr");
                            (nr > 0).then(|| (nr, LineCoverage::of(number(line, "mi"), number(line, "ci"), number(line, "mb"))))
                        })
                        .collect();
                    report.files.push(FileCoverage { path, lines });
                }
                _ => {}
            }
        }
    }
    report.classes.sort_by(|a, b| a.name.cmp(&b.name));
    Some(report)
}

/// A node's own `<counter type=…>` (not its children's).
fn counter(node: roxmltree::Node, kind: &str) -> Counter {
    node.children()
        .find(|n| n.has_tag_name("counter") && n.attribute("type") == Some(kind))
        .map(|n| {
            let number = |attr| n.attribute(attr).and_then(|v| v.parse().ok()).unwrap_or(0);
            Counter::new(number("missed"), number("covered"))
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_classes_and_lines() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><!DOCTYPE report PUBLIC "-//JACOCO//DTD Report 1.1//EN" "report.dtd"><report name="hs"><sessioninfo id="h" start="1" dump="2"/><package name="demo"><class name="demo/Calc" sourcefilename="Calc.java"><method name="add" desc="(II)I" line="3"><counter type="INSTRUCTION" missed="0" covered="4"/></method><counter type="INSTRUCTION" missed="5" covered="7"/><counter type="BRANCH" missed="1" covered="1"/><counter type="LINE" missed="1" covered="3"/><counter type="METHOD" missed="1" covered="2"/></class><class name="demo/Calc$1" sourcefilename="Calc.java"><counter type="METHOD" missed="2" covered="0"/></class><sourcefile name="Calc.java"><line nr="3" mi="0" ci="4" mb="0" cb="0"/><line nr="5" mi="1" ci="2" mb="1" cb="1"/><line nr="9" mi="3" ci="0" mb="0" cb="0"/><counter type="LINE" missed="1" covered="3"/></sourcefile><counter type="METHOD" missed="3" covered="2"/></package><counter type="METHOD" missed="3" covered="2"/></report>"#;
        let report = parse(xml).unwrap();
        assert_eq!(report.name, "hs");
        assert_eq!(report.classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["demo.Calc", "demo.Calc$1"]);
        let calc = &report.classes[0];
        assert_eq!((calc.methods.label(), calc.lines.label(), calc.branches.label()), ("66% (2/3)".into(), "75% (3/4)".into(), "50% (1/2)".into()));
        assert_eq!(calc.source.as_deref(), Some("Calc.java"));
        assert!(!report.classes[1].covered());
        let file = report.file("/p/src/main/java/demo/Calc.java").unwrap();
        assert_eq!(file.lines.iter().map(|(n, c)| (*n, *c)).collect::<Vec<_>>(), [(3, LineCoverage::Full), (5, LineCoverage::Partial), (9, LineCoverage::None)]);
        assert!(parse("<testsuite/>").is_none());
    }
}

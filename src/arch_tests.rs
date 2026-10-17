//! The Clean architecture's dependency rule, checked on the sources:
//! `ui` → `usecase` → `domain` ← `infra`. Test modules may use whatever
//! they need (a use case's test can run against a real store).

use std::path::Path;

/// The code without `#[cfg(test)]` items (their lines blanked, so line
/// numbers stay): the item after the attribute, through its closing brace.
fn without_tests(text: &str) -> String {
    let mut out = Vec::new();
    let (mut skipping, mut depth, mut opened) = (false, 0i32, false);
    for line in text.lines() {
        if !skipping && line.trim_start().starts_with("#[cfg(test)]") {
            (skipping, depth, opened) = (true, 0, false);
        }
        if skipping {
            for c in line.chars() {
                match c {
                    '{' => (depth, opened) = (depth + 1, true),
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            // A braceless item (`use …;`) ends at its semicolon.
            if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                skipping = false;
            }
            out.push("");
        } else {
            out.push(line);
        }
    }
    out.join("\n")
}

/// Each file's code without its tests.
fn sources(dir: &str) -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path.display().to_string(), without_tests(&text)));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(dir), &mut out);
    out
}

fn violations(dir: &str, forbidden: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, code) in sources(dir) {
        for (n, line) in code.lines().enumerate() {
            let line = line.split("//").next().unwrap_or_default();
            if let Some(f) = forbidden.iter().find(|f| line.contains(*f)) {
                out.push(format!("{path}:{}: {f}", n + 1));
            }
        }
    }
    out
}

#[test]
fn test_items_are_left_out() {
    let text = "use a;\n#[cfg(test)]\nuse b;\n#[cfg(test)]\nmod tests {\n    fn f() { x }\n}\nfn after() {}\n";
    assert_eq!(without_tests(text), "use a;\n\n\n\n\n\n\nfn after() {}");
}

#[test]
fn domain_is_plain_data_and_rules() {
    let outer = ["crate::ui", "crate::usecase", "crate::infra", "crate::index", "crate::model", "crate::git", "crate::android", "crate::hosting", "crate::settings"];
    let io = ["gpui", "std::fs", "std::process", "std::net"];
    let found = violations("domain", &[&outer[..], &io[..]].concat());
    assert!(found.is_empty(), "domain depends on outer layers or I/O:\n{}", found.join("\n"));
}

#[test]
fn use_cases_know_only_the_domain() {
    let found = violations("usecase", &["gpui", "crate::ui", "crate::infra"]);
    assert!(found.is_empty(), "use cases depend on ui / infra:\n{}", found.join("\n"));
}

#[test]
fn infra_never_reaches_the_ui() {
    let found = violations("infra", &["gpui", "crate::ui"]);
    assert!(found.is_empty(), "infra depends on the UI:\n{}", found.join("\n"));
}

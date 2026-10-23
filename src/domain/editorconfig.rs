//! `.editorconfig` (editorconfig.org), which IntelliJ applies over its code
//! style: the indent (`indent_style`, `indent_size`, `tab_width`) of the
//! sections whose glob matches a file, the nearer file's over the farther.

/// One `.editorconfig`: whether it is the root, and its sections' globs
/// and properties (keys lower-cased), in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorConfig {
    pub root: bool,
    pub sections: Vec<(String, Vec<(String, String)>)>,
}

pub fn parse(text: &str) -> EditorConfig {
    let mut config = EditorConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            config.sections.push((glob.to_owned(), Vec::new()));
            continue;
        }
        let Some((key, value)) = line.split_once(['=', ':']) else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim().to_owned());
        match config.sections.last_mut() {
            Some((_, props)) => props.push((key, value)),
            None if key == "root" => config.root = value.eq_ignore_ascii_case("true"),
            None => {}
        }
    }
    config
}

/// Whether a section's glob matches `path` (relative to the config's
/// folder, `/`-separated): `*` within a folder, `**` across, `?`, `{a,b}`,
/// `[abc]`; a glob without `/` matches the file name in any folder.
pub fn matches(glob: &str, path: &str) -> bool {
    let anchored = glob.contains('/');
    let glob = glob.strip_prefix('/').unwrap_or(glob);
    let mut re = String::from("^");
    if !anchored {
        re.push_str("(?:.*/)?");
    }
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    let mut braces = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => {
                braces += 1;
                re.push_str("(?:");
            }
            '}' if braces > 0 => {
                braces -= 1;
                re.push(')');
            }
            ',' if braces > 0 => re.push('|'),
            '[' => match chars[i..].iter().position(|&ch| ch == ']') {
                Some(end) => {
                    let class: String = chars[i + 1..i + end].iter().collect();
                    re.push('[');
                    re.push_str(&class.replacen('!', "^", usize::from(class.starts_with('!'))).replace('\\', "\\\\"));
                    re.push(']');
                    i += end;
                }
                None => re.push_str("\\["),
            },
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(path))
}

/// The indent unit the configs give `path`: `configs` from the farthest
/// to the nearest, each with the path relative to its folder. None when
/// none sets an indent.
pub fn indent(configs: &[(EditorConfig, String)]) -> Option<String> {
    let (mut style, mut size, mut tab_width): (Option<String>, Option<String>, Option<usize>) = (None, None, None);
    for (config, path) in configs {
        for (glob, props) in &config.sections {
            if !matches(glob, path) {
                continue;
            }
            for (key, value) in props {
                let value = value.to_ascii_lowercase();
                match key.as_str() {
                    "indent_style" => style = Some(value),
                    "indent_size" => size = Some(value),
                    "tab_width" => tab_width = value.parse().ok(),
                    _ => {}
                }
            }
        }
    }
    let size = match size.as_deref() {
        Some("tab") => tab_width,
        Some(n) => n.parse::<usize>().ok(),
        None => None,
    };
    match style.as_deref() {
        Some("tab") => Some("\t".into()),
        Some("space") => Some(" ".repeat(size.unwrap_or(4).clamp(1, 16))),
        _ => size.map(|n| " ".repeat(n.clamp(1, 16))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(matches("*", "app/src/A.kt"));
        assert!(matches("*.{kt,kts}", "app/build.gradle.kts"));
        assert!(!matches("*.{kt,kts}", "app/A.java"));
        assert!(matches("src/**/*.java", "src/a/b/A.java"));
        assert!(!matches("src/*.java", "src/a/A.java"));
        assert!(matches("/lib/*.xml", "lib/a.xml"));
        assert!(matches("[Mm]akefile", "Makefile"));
    }

    #[test]
    fn indents() {
        let outer = parse("root = true\n[*]\nindent_style = space\nindent_size = 4\n[*.xml]\nindent_size = 2\n");
        assert!(outer.root);
        let inner = parse("# app\n[*.kt]\nindent_style=tab\n");
        assert_eq!(indent(&[(outer.clone(), "app/A.kt".into())]).as_deref(), Some("    "));
        assert_eq!(indent(&[(outer.clone(), "app/res/a.xml".into())]).as_deref(), Some("  "));
        assert_eq!(indent(&[(outer.clone(), "app/A.kt".into()), (inner, "A.kt".into())]).as_deref(), Some("\t"));
        assert_eq!(indent(&[(parse("[*]\nend_of_line = lf\n"), "A.kt".into())]), None);
        assert_eq!(indent(&[(parse("[*]\nindent_size = tab\ntab_width = 3\n"), "A.kt".into())]).as_deref(), Some("   "));
    }
}

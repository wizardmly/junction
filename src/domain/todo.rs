//! TODO items, as IntelliJ finds them: comments matching the TODO patterns
//! (by default `\btodo\b.*` and `\bfixme\b.*`, case-insensitive).

use std::ops::Range;

/// IntelliJ's default patterns, as one search: the keyword starts an item
/// that runs to the end of the line.
pub const DEFAULT_PATTERN: &str = r"\b(?:todo|fixme)\b";

/// The highlighted part of a found line: from the keyword to the end of
/// the comment (a closing `*/` or `-->` stays out).
pub fn item_range(line: &str, keyword: &Range<usize>) -> Range<usize> {
    let rest = &line[keyword.start..];
    let end = ["*/", "-->"].iter().filter_map(|close| rest.find(close)).min().unwrap_or(rest.len());
    keyword.start..keyword.start + rest[..end].trim_end().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_runs_to_the_comment_end() {
        let line = "val x = 1 // TODO: remove";
        let k = line.find("TODO").unwrap();
        assert_eq!(&line[item_range(line, &(k..k + 4))], "TODO: remove");
        let line = "/* FIXME later */ call()";
        let k = line.find("FIXME").unwrap();
        assert_eq!(&line[item_range(line, &(k..k + 5))], "FIXME later");
        let re = regex::RegexBuilder::new(DEFAULT_PATTERN).case_insensitive(true).build().unwrap();
        assert!(re.is_match("// todo fix") && re.is_match("# FIXME") && !re.is_match("// todos"));
    }
}

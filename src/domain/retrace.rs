//! R8 / ProGuard retrace: a `mapping.txt` turns obfuscated stack frames
//! and exception class names back into the original ones (Android Studio's
//! Logcat "Load Proguard Mappings…", AOSP `ProguardMessageRewriter` over
//! R8's `Retrace`).
//!
//! A mapping can be tens of megabytes, so only an index of class sections
//! is kept; a class's members are read and parsed when a frame needs them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Where a mapping's text lives: a class section by byte range.
pub trait MappingSource: Send {
    fn read(&self, at: u64, len: u32) -> Option<String>;
}

/// The header comment R8 writes: `# pg_map_id: 1a2b3c4`.
const MAP_ID: &str = "# pg_map_id: ";

/// A class line: `com.example.Foo -> a.b:`.
fn class_line(line: &str) -> Option<(&str, &str)> {
    if line.starts_with([' ', '\t', '#']) {
        return None;
    }
    let (original, obfuscated) = line.trim_end().strip_suffix(':')?.split_once(" -> ")?;
    Some((original.trim(), obfuscated.trim()))
}

struct ClassAt {
    original: String,
    at: u64,
    len: u32,
}

/// Builds an index from a mapping's lines, each with its byte offset.
#[derive(Default)]
pub struct IndexBuilder {
    map_id: Option<String>,
    classes: Vec<(String, ClassAt)>,
}

impl IndexBuilder {
    pub fn line(&mut self, line: &str, offset: u64) {
        if let Some(id) = line.strip_prefix(MAP_ID) {
            if self.map_id.is_none() && self.classes.is_empty() {
                self.map_id = Some(id.trim().to_owned());
            }
            return;
        }
        if let Some((original, obfuscated)) = class_line(line) {
            self.close(offset);
            self.classes.push((obfuscated.to_owned(), ClassAt { original: original.to_owned(), at: offset, len: 0 }));
        }
    }

    fn close(&mut self, end: u64) {
        if let Some((_, last)) = self.classes.last_mut().filter(|(_, c)| c.len == 0) {
            last.len = u32::try_from(end - last.at).unwrap_or(u32::MAX);
        }
    }

    pub fn finish(mut self, end: u64, source: Box<dyn MappingSource>) -> Retracer {
        self.close(end);
        let originals = self.classes.iter().enumerate().map(|(i, (_, c))| (c.original.clone(), i)).collect();
        let obfuscated = self.classes.iter().enumerate().map(|(i, (o, _))| (o.clone(), i)).collect();
        Retracer { map_id: self.map_id, classes: self.classes.into_iter().map(|(_, c)| c).collect(), obfuscated, originals, source, parsed: RefCell::new(HashMap::new()) }
    }
}

/// Parses a whole mapping held in memory.
#[cfg(test)]
pub fn from_text(text: &str) -> Retracer {
    struct Text(String);
    impl MappingSource for Text {
        fn read(&self, at: u64, len: u32) -> Option<String> {
            self.0.get(at as usize..at as usize + len as usize).map(str::to_owned)
        }
    }
    let mut builder = IndexBuilder::default();
    let mut offset = 0u64;
    for line in text.split_inclusive('\n') {
        builder.line(line.trim_end_matches(['\r', '\n']), offset);
        offset += line.len() as u64;
    }
    builder.finish(offset, Box::new(Text(text.to_owned())))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Original {
    /// No original lines: the obfuscated ones are the original ones.
    Same,
    /// `:15`, every obfuscated line maps here (an inlining caller).
    Line(u32),
    Range(u32, u32),
}

#[derive(Debug)]
struct Member {
    obfuscated: String,
    lines: Option<(u32, u32)>,
    /// `void com.other.Bar.m()`: a method inlined from another class.
    class: Option<String>,
    name: String,
    original: Original,
}

impl Member {
    fn original_line(&self, line: Option<u32>) -> Option<u32> {
        let line = line?;
        match (self.original, self.lines) {
            (Original::Line(l), _) => Some(l),
            (Original::Range(a, b), Some((s, _))) => Some(a.saturating_add(line.saturating_sub(s)).min(b.max(a))),
            (Original::Range(a, _), None) => Some(a),
            (Original::Same, _) => Some(line),
        }
    }
}

struct Class {
    original: String,
    source_file: Option<String>,
    members: Vec<Member>,
}

/// `1:3:void com.x.Bar.m(int):10:12 -> a`; fields (no parentheses) are skipped.
fn member_line(line: &str) -> Option<Member> {
    let (left, obfuscated) = line.trim().split_once(" -> ")?;
    let mut rest = left;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((n, tail)) = rest.split_once(':') else { break };
        let Ok(n) = n.parse::<u32>() else { break };
        numbers.push(n);
        rest = tail;
    }
    let lines = match numbers[..] {
        [a, b] => Some((a, b)),
        [a] => Some((a, a)),
        _ => None,
    };
    let close = rest.rfind(')')?;
    let open = rest[..close].find('(')?;
    let original = match rest[close + 1..].strip_prefix(':') {
        Some(tail) => match tail.split_once(':') {
            Some((a, b)) => Original::Range(a.parse().ok()?, b.parse().ok()?),
            None => Original::Line(tail.parse().ok()?),
        },
        None => Original::Same,
    };
    let qualified = rest[..open].rsplit_once(' ').map_or(&rest[..open], |(_, q)| q);
    let (class, name) = match qualified.rsplit_once('.') {
        Some((c, n)) => (Some(c.to_owned()), n.to_owned()),
        None => (None, qualified.to_owned()),
    };
    Some(Member { obfuscated: obfuscated.trim().to_owned(), lines, class, name, original })
}

fn parse_class(section: &str) -> Option<Class> {
    let mut lines = section.lines();
    let (original, _) = class_line(lines.next()?)?;
    let mut class = Class { original: original.to_owned(), source_file: None, members: Vec::new() };
    for line in lines {
        let trimmed = line.trim_start();
        if let Some(comment) = trimmed.strip_prefix('#') {
            // # {"id":"sourceFile","fileName":"Foo.kt"}
            if comment.contains("\"sourceFile\"") {
                class.source_file = json_string(comment, "fileName");
            }
        } else if let Some(m) = member_line(trimmed) {
            class.members.push(m);
        }
    }
    Some(class)
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let value = text[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(value[..value.find('"')?].to_owned())
}

/// A loaded mapping.
pub struct Retracer {
    map_id: Option<String>,
    classes: Vec<ClassAt>,
    obfuscated: HashMap<String, usize>,
    originals: HashMap<String, usize>,
    source: Box<dyn MappingSource>,
    parsed: RefCell<HashMap<usize, Option<Arc<Class>>>>,
}

/// A frame: `<indent>at a.b.c(SourceFile:12)<rest>`.
struct Frame<'a> {
    indent: &'a str,
    class: &'a str,
    method: &'a str,
    file: &'a str,
    line: Option<u32>,
    rest: &'a str,
}

fn frame(line: &str) -> Option<Frame<'_>> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let at = body.strip_prefix("at ")?;
    let open = at.find('(')?;
    let close = at[open..].find(')')? + open;
    let (class, method) = at[..open].rsplit_once('.')?;
    let source = &at[open + 1..close];
    let (file, line) = match source.rsplit_once(':') {
        Some((f, l)) if l.parse::<u32>().is_ok() => (f, l.parse().ok()),
        _ => (source, None),
    };
    Some(Frame { indent, class, method, file, line, rest: &at[close + 1..] })
}

/// The `r8-map-id-<id>` R8 puts in the source file of frames when the
/// build asks for it, so the matching mapping can be found.
pub fn frame_map_id(message: &str) -> Option<&str> {
    let f = frame(message)?;
    f.file.strip_prefix("r8-map-id-")
}

impl Retracer {
    pub fn map_id(&self) -> Option<&str> {
        self.map_id.as_deref()
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    fn class(&self, i: usize) -> Option<Arc<Class>> {
        if let Some(c) = self.parsed.borrow().get(&i) {
            return c.clone();
        }
        let at = &self.classes[i];
        let class = self.source.read(at.at, at.len).and_then(|text| parse_class(&text)).map(Arc::new);
        self.parsed.borrow_mut().insert(i, class.clone());
        class
    }

    /// The original name of an obfuscated class.
    pub fn class_name(&self, obfuscated: &str) -> Option<&str> {
        self.obfuscated.get(obfuscated).map(|i| self.classes[*i].original.as_str())
    }

    fn source_file(&self, original: &str, frame_file: &str) -> String {
        if let Some(file) = self.originals.get(original).and_then(|i| self.class(*i)).and_then(|c| c.source_file.clone()) {
            return file;
        }
        let outer = original.rsplit('.').next().unwrap_or(original);
        let outer = outer.split('$').next().unwrap_or(outer);
        let ext = match frame_file.rsplit_once('.') {
            Some((_, e @ ("kt" | "java"))) => e,
            _ => "java",
        };
        format!("{outer}.{ext}")
    }

    /// The original lines for one message line, or `None` when the mapping
    /// knows nothing of it. An inlined frame becomes several frames; frames
    /// the mapping can't tell apart follow as `<OR> at …`, as R8 prints them.
    pub fn retrace(&self, line: &str) -> Option<Vec<String>> {
        match frame(line) {
            Some(f) => self.retrace_frame(&f),
            None => self.retrace_class_names(line).map(|l| vec![l]),
        }
    }

    fn retrace_frame(&self, f: &Frame) -> Option<Vec<String>> {
        let class = self.class(*self.obfuscated.get(f.class)?)?;
        let mut chains: Vec<Vec<&Member>> = Vec::new();
        let covers = |m: &Member| matches!((f.line, m.lines), (Some(l), Some((s, e))) if s <= l && l <= e);
        if class.members.iter().any(|m| m.obfuscated == f.method && covers(m)) {
            // Consecutive members sharing the range: an inlining chain,
            // innermost first.
            let mut previous: Option<&Member> = None;
            for m in &class.members {
                let hit = m.obfuscated == f.method && covers(m);
                match (hit, previous, chains.last_mut()) {
                    (true, Some(p), Some(chain)) if p.lines == m.lines => chain.push(m),
                    (true, _, _) => chains.push(vec![m]),
                    _ => {}
                }
                previous = hit.then_some(m);
            }
        } else {
            chains.extend(class.members.iter().filter(|m| m.obfuscated == f.method && (f.line.is_none() || m.lines.is_none())).map(|m| vec![m]));
        }
        if chains.is_empty() {
            // The class is known but not the method: just the class.
            let file = self.source_file(&class.original, f.file);
            let line = f.line.map(|l| format!(":{l}")).unwrap_or_default();
            return Some(vec![format!("{}at {}.{}({file}{line}){}", f.indent, class.original, f.method, f.rest)]);
        }
        let mut out: Vec<String> = Vec::new();
        let mut seen: Vec<Vec<String>> = Vec::new();
        for chain in chains {
            let frames: Vec<String> = chain
                .iter()
                .map(|m| {
                    let owner = m.class.as_deref().unwrap_or(&class.original);
                    let file = self.source_file(owner, f.file);
                    let line = m.original_line(f.line).map(|l| format!(":{l}")).unwrap_or_default();
                    format!("{owner}.{}({file}{line})", m.name)
                })
                .collect();
            if seen.contains(&frames) {
                continue;
            }
            for (i, frame) in frames.iter().enumerate() {
                let or = if !seen.is_empty() && i == 0 { "<OR> " } else { "" };
                let rest = if i + 1 == frames.len() { f.rest } else { "" };
                out.push(format!("{}{or}at {frame}{rest}", f.indent));
            }
            seen.push(frames);
        }
        Some(out)
    }

    /// `Caused by: a.b: message`, `a.b: message`, `Exception in thread "main" a.b`.
    fn retrace_class_names(&self, line: &str) -> Option<String> {
        let body = line.trim_start();
        let indent = &line[..line.len() - body.len()];
        let mut prefix_len = 0;
        for prefix in ["Caused by: ", "Suppressed: "] {
            if body.starts_with(prefix) {
                prefix_len = prefix.len();
            }
        }
        if body.starts_with("Exception in thread \"") {
            prefix_len = body[21..].find("\" ").map(|i| 21 + i + 2)?;
        }
        let rest = &body[prefix_len..];
        let end = rest.find([':', ' ']).unwrap_or(rest.len());
        let original = self.class_name(&rest[..end])?;
        Some(format!("{indent}{}{original}{}", &body[..prefix_len], &rest[end..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str = "# compiler: R8
# pg_map_id: 4f1c2e9
com.example.app.MainActivity -> a.a:
# {\"id\":\"sourceFile\",\"fileName\":\"MainActivity.kt\"}
    int counter -> b
    1:4:void onCreate(android.os.Bundle):20:23 -> onCreate
    5:5:void com.example.app.Util.check(int):7:7 -> a
    5:5:void crash():31 -> a
    5:5:void onClick(android.view.View):40 -> a
    6:6:void crash():32:32 -> a
    void helper() -> c
    void other() -> c
com.example.app.Util -> a.b:
# {\"id\":\"sourceFile\",\"fileName\":\"Util.kt\"}
    1:2:void check(int):5:6 -> a
com.example.app.BadState -> a.c:
";

    #[test]
    fn frames_and_inlining() {
        let r = from_text(MAPPING);
        assert_eq!(r.map_id(), Some("4f1c2e9"));
        assert_eq!(r.class_count(), 3);
        assert_eq!(r.retrace("\tat a.a.onCreate(SourceFile:3)").unwrap(), vec!["\tat com.example.app.MainActivity.onCreate(MainActivity.kt:22)"]);
        assert_eq!(
            r.retrace("    at a.a.a(SourceFile:5)").unwrap(),
            vec![
                "    at com.example.app.Util.check(Util.kt:7)",
                "    at com.example.app.MainActivity.crash(MainActivity.kt:31)",
                "    at com.example.app.MainActivity.onClick(MainActivity.kt:40)",
            ]
        );
        assert_eq!(r.retrace("at a.a.a(r8-map-id-4f1c2e9:6)").unwrap(), vec!["at com.example.app.MainActivity.crash(MainActivity.kt:32)"]);
        assert_eq!(r.retrace("at a.a.c(Unknown Source)").unwrap(), vec!["at com.example.app.MainActivity.helper(MainActivity.kt)", "<OR> at com.example.app.MainActivity.other(MainActivity.kt)"]);
        assert_eq!(r.retrace("at android.app.Activity.performCreate(Activity.java:8595)"), None);
        assert_eq!(frame_map_id("\tat a.a.a(r8-map-id-4f1c2e9:6)"), Some("4f1c2e9"));
    }

    #[test]
    fn class_names() {
        let r = from_text(MAPPING);
        assert_eq!(r.retrace("Caused by: a.c: bad").unwrap(), vec!["Caused by: com.example.app.BadState: bad"]);
        assert_eq!(r.retrace("a.c").unwrap(), vec!["com.example.app.BadState"]);
        assert_eq!(r.retrace("Exception in thread \"main\" a.c: x").unwrap(), vec!["Exception in thread \"main\" com.example.app.BadState: x"]);
        assert_eq!(r.retrace("java.lang.IllegalStateException: a.c"), None);
        assert_eq!(r.retrace("FATAL EXCEPTION: main"), None);
    }
}

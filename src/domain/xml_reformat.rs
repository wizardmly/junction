//! Reformat Code for XML, as Android Studio's predefined "Android" XML code
//! style does it (AOSP `AndroidXmlPredefinedCodeStyle`,
//! `AndroidXmlCodeStyleSettings`): 4-space indent and continuation, a space
//! in empty tags (`<View />`), attributes rearranged (namespaces, `id`,
//! `name`, `style`, then `AttributeComparator`'s layout order); per file
//! kind, one attribute per line with the first on its own line (layouts,
//! manifests, other resources) or all on the tag's line (values), and the
//! blank lines each kind inserts. Plain XML elsewhere keeps its attribute
//! order and line breaks and is only re-indented. Text, comments and CDATA
//! are kept as written.

/// Which of Android Studio's XML settings apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlKind {
    Layout,
    Manifest,
    Values,
    /// Other Android resources (drawables, menus, navigation…).
    Resource,
    /// Any other XML.
    Plain,
}

impl XmlKind {
    /// The kind for a file path.
    pub fn of(path: &str) -> Self {
        let path = path.replace('\\', "/");
        if path.ends_with("AndroidManifest.xml") {
            return Self::Manifest;
        }
        let mut parts = path.rsplit('/');
        let _file = parts.next();
        let folder = parts.next().unwrap_or("");
        if parts.next() != Some("res") {
            return Self::Plain;
        }
        match folder.split('-').next().unwrap_or("") {
            "layout" => Self::Layout,
            "values" => Self::Values,
            _ => Self::Resource,
        }
    }
}

const INDENT: &str = "    ";
/// IntelliJ's KEEP_BLANK_LINES for XML.
const KEEP_BLANK: usize = 2;

#[derive(Debug)]
enum Node {
    Element(Element),
    /// Text (`raw` as written), with whether it is all whitespace.
    Text(String),
    /// A comment, CDATA, the prolog or a doctype, as written.
    Raw(String),
}

#[derive(Debug)]
struct Element {
    name: String,
    /// `(name, value with its quotes)`, and whether it started a new line.
    attrs: Vec<(String, String, bool)>,
    children: Vec<Node>,
    self_closing: bool,
    /// The content between the tags, as written.
    inner: String,
}

/// The reformatted text, or the text unchanged when it isn't well-formed.
pub fn reformat_xml(kind: XmlKind, text: &str) -> String {
    let Some(nodes) = parse(text) else { return text.to_owned() };
    let mut out = String::new();
    let mut first = true;
    for node in &nodes {
        match node {
            Node::Text(t) => {
                if !first && t.trim().is_empty() {
                    let blanks = t.matches('\n').count().saturating_sub(1).min(KEEP_BLANK);
                    out.push_str(&"\n".repeat(blanks));
                }
            }
            node => {
                if !first {
                    out.push('\n');
                }
                first = false;
                write_node(kind, node, 0, true, &mut out);
            }
        }
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn write_node(kind: XmlKind, node: &Node, depth: usize, root: bool, out: &mut String) {
    match node {
        Node::Element(e) => write_element(kind, e, depth, root, out),
        Node::Raw(raw) => {
            out.push_str(&INDENT.repeat(depth));
            out.push_str(raw.trim());
        }
        Node::Text(t) => {
            out.push_str(&INDENT.repeat(depth));
            out.push_str(t.trim());
        }
    }
}

fn write_element(kind: XmlKind, e: &Element, depth: usize, root: bool, out: &mut String) {
    let indent = INDENT.repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&e.name);
    let column = out.len() - out.rfind('\n').map_or(0, |i| i + 1) + 1;
    write_attributes(kind, e, depth, column, out);
    if e.self_closing {
        out.push_str(" />");
        return;
    }
    out.push('>');
    let mixed = e.children.iter().any(|c| matches!(c, Node::Text(t) if !t.trim().is_empty()) || matches!(c, Node::Raw(r) if r.starts_with("<![CDATA[")));
    let nested = e.children.iter().any(|c| !matches!(c, Node::Text(_)));
    if mixed {
        // Text (a string's value, an item's): as written.
        out.push_str(&e.inner);
    } else if nested {
        // The children other than whitespace: an element's name, None for a comment.
        let siblings: Vec<Option<&str>> = e
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Element(c) => Some(Some(c.name.as_str())),
                Node::Raw(_) => Some(None),
                Node::Text(_) => None,
            })
            .collect();
        let first_tag = e.children.iter().find_map(|c| if let Node::Element(c) = c { Some(c.name.as_str()) } else { None });
        let mut index = 0;
        let mut kept = 0;
        for child in &e.children {
            if let Node::Text(t) = child {
                kept = t.matches('\n').count().saturating_sub(1).min(KEEP_BLANK);
                continue;
            }
            let wanted = (index == 0 && blank_after_begin(kind, first_tag, root)) || (matches!(child, Node::Element(_)) && blank_before(kind, &siblings, index));
            index += 1;
            out.push('\n');
            out.push_str(&"\n".repeat(kept.max(usize::from(wanted))));
            kept = 0;
            write_node(kind, child, depth + 1, false, out);
        }
        out.push('\n');
        out.push_str(&indent);
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

/// Whether the kind's settings put a blank line after a tag's beginning
/// (`insertLineBreakAfterTagBegin`).
fn blank_after_begin(kind: XmlKind, first_tag: Option<&str>, root: bool) -> bool {
    match kind {
        XmlKind::Layout => true,
        // GROUP_TAGS_WITH_SAME_NAME: the root's.
        XmlKind::Manifest => root,
        // INSERT_LINE_BREAKS_AROUND_STYLE: before a first style.
        XmlKind::Values => first_tag == Some("style"),
        XmlKind::Resource | XmlKind::Plain => false,
    }
}

/// Whether the kind's settings put a blank line before the tag at `i` among
/// its siblings (`insertLineBreakBeforeTag`; a comment is a sibling that
/// isn't a tag).
fn blank_before(kind: XmlKind, siblings: &[Option<&str>], i: usize) -> bool {
    let name = |j: usize| siblings[j];
    match kind {
        // INSERT_BLANK_LINE_BEFORE_TAG.
        XmlKind::Layout => true,
        // GROUP_TAGS_WITH_SAME_NAME: between a run of same-named tags and
        // the next tag.
        XmlKind::Manifest => i >= 2 && name(i - 1).is_some() && name(i - 1) != name(i) && name(i - 2) == name(i - 1),
        // INSERT_LINE_BREAKS_AROUND_STYLE.
        XmlKind::Values => name(i) == Some("style") || (i > 0 && name(i - 1) == Some("style")),
        XmlKind::Resource | XmlKind::Plain => false,
    }
}

fn write_attributes(kind: XmlKind, e: &Element, depth: usize, column: usize, out: &mut String) {
    if e.attrs.is_empty() {
        return;
    }
    if kind == XmlKind::Plain {
        // Kept in order, on the lines they were on, aligned with the first.
        let first_wrapped = e.attrs[0].2;
        let continuation = if first_wrapped { " ".repeat((depth + 2) * INDENT.len()) } else { " ".repeat(column) };
        for (i, (name, value, wrapped)) in e.attrs.iter().enumerate() {
            if *wrapped {
                out.push('\n');
                out.push_str(&continuation);
            } else if i > 0 || !first_wrapped {
                out.push(' ');
            }
            out.push_str(name);
            out.push('=');
            out.push_str(value);
        }
        return;
    }
    let mut attrs: Vec<&(String, String, bool)> = e.attrs.iter().collect();
    attrs.sort_by(|a, b| arrangement(&a.0).cmp(&arrangement(&b.0)));
    let continuation = INDENT.repeat(depth + 1);
    // AndroidXmlPolicy: WRAP_ALWAYS, and a break before the first attribute
    // when there are several and none declares a namespace; values: DO_NOT_WRAP.
    let namespaces = attrs.iter().any(|a| a.0 == "xmlns" || a.0.starts_with("xmlns:"));
    let break_first = !namespaces && attrs.len() > 1;
    for (i, (name, value, _)) in attrs.iter().enumerate() {
        let wrap = kind != XmlKind::Values && (i > 0 || break_first);
        if wrap {
            out.push('\n');
            out.push_str(&continuation);
        } else {
            out.push(' ');
        }
        out.push_str(name);
        out.push('=');
        out.push_str(value);
    }
}

/// The predefined Android arrangement rules: the rule an attribute matches
/// and its place within the rule (kept order sorts equal).
fn arrangement(name: &str) -> (u8, (u8, String, u8)) {
    let (prefix, local) = name.split_once(':').map_or(("", name), |(p, l)| (p, l));
    let by_name = |rule: u8| (rule, (0, name.to_owned(), 0));
    let keep = |rule: u8| (rule, (0, String::new(), 0));
    match (prefix, local) {
        ("xmlns", "android") => keep(0),
        ("xmlns", _) => by_name(1),
        ("", "xmlns") => keep(0),
        ("android", "id") => keep(2),
        ("android", "name") => keep(3),
        ("", "name") => keep(4),
        ("", "style") => keep(5),
        ("", _) => by_name(6),
        ("android", local) => (7, android_order(local)),
        _ => by_name(8),
    }
}

/// `AttributeComparator`: layout_width, layout_height, layout_row…,
/// layout_column…, layout_alignWithParentIfMissing, layout_above,
/// layout_below, other layout_…, width, height, then the rest; within a
/// type by the name without its specifier (Width, Start, Top…), then by the
/// specifier's order.
fn android_order(local: &str) -> (u8, String, u8) {
    let kind = if local == "layout_width" {
        0
    } else if local == "layout_height" {
        1
    } else if local.starts_with("layout_row") {
        2
    } else if local.starts_with("layout_column") {
        3
    } else if local == "layout_alignWithParentIfMissing" {
        4
    } else if local == "layout_above" {
        5
    } else if local == "layout_below" {
        6
    } else if local.starts_with("layout_") {
        7
    } else if local == "width" {
        8
    } else if local == "height" {
        9
    } else {
        10
    };
    const SPECIFIERS: [&str; 14] = ["Width", "Height", "InParent", "Horizontal", "Vertical", "Baseline", "Start", "Left", "Top", "End", "Right", "Bottom", "Up", "Down"];
    match SPECIFIERS.iter().position(|s| local.contains(s)) {
        Some(i) => (kind, local.replacen(SPECIFIERS[i], "", 1), i as u8 + 1),
        None => (kind, local.to_owned(), 0),
    }
}

/// The document's nodes, or None when tags don't match.
fn parse(text: &str) -> Option<Vec<Node>> {
    // Open elements: the element and where its content starts.
    let mut stack: Vec<(Element, usize)> = Vec::new();
    let mut top: Vec<Node> = Vec::new();
    let push = |stack: &mut Vec<(Element, usize)>, top: &mut Vec<Node>, node: Node| match stack.last_mut() {
        Some((e, _)) => e.children.push(node),
        None => top.push(node),
    };
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').map_or(text.len(), |n| i + n);
            push(&mut stack, &mut top, Node::Text(text[i..end].to_owned()));
            i = end;
            continue;
        }
        let raw_until = |close: &str| rest.find(close).map(|n| n + close.len());
        if rest.starts_with("<!--") || rest.starts_with("<![CDATA[") || rest.starts_with("<?") {
            let close = if rest.starts_with("<!--") {
                "-->"
            } else if rest.starts_with("<?") {
                "?>"
            } else {
                "]]>"
            };
            let len = raw_until(close)?;
            push(&mut stack, &mut top, Node::Raw(rest[..len].to_owned()));
            i += len;
        } else if rest.starts_with("<!") {
            // A doctype, maybe with an internal subset in brackets.
            let mut depth = 0;
            let len = rest.char_indices().find(|&(_, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    '>' if depth == 0 => return true,
                    _ => {}
                }
                false
            })?.0 + 1;
            push(&mut stack, &mut top, Node::Raw(rest[..len].to_owned()));
            i += len;
        } else if let Some(after) = rest.strip_prefix("</") {
            let len = after.find('>')?;
            let name = after[..len].trim();
            let (mut e, start) = stack.pop()?;
            if e.name != name {
                return None;
            }
            e.inner = text[start..i].to_owned();
            i += 2 + len + 1;
            push(&mut stack, &mut top, Node::Element(e));
        } else {
            let (e, len) = start_tag(rest)?;
            i += len;
            if e.self_closing {
                push(&mut stack, &mut top, Node::Element(e));
            } else {
                stack.push((e, i));
            }
        }
    }
    stack.is_empty().then_some(top)
}

/// A start tag at the beginning of `s`, and its length.
fn start_tag(s: &str) -> Option<(Element, usize)> {
    let bytes = s.as_bytes();
    let mut i = 1;
    let name_end = s[1..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').map(|n| n + 1)?;
    let name = s[1..name_end].to_owned();
    if name.is_empty() {
        return None;
    }
    i = i.max(name_end);
    let mut attrs = Vec::new();
    let mut newline = false;
    loop {
        while i < s.len() && bytes[i].is_ascii_whitespace() {
            newline |= bytes[i] == b'\n';
            i += 1;
        }
        if i >= s.len() {
            return None;
        }
        if s[i..].starts_with("/>") {
            return Some((Element { name, attrs, children: Vec::new(), self_closing: true, inner: String::new() }, i + 2));
        }
        if bytes[i] == b'>' {
            return Some((Element { name, attrs, children: Vec::new(), self_closing: false, inner: String::new() }, i + 1));
        }
        let eq = s[i..].find('=').map(|n| i + n)?;
        let attr = s[i..eq].trim().to_owned();
        if attr.is_empty() || attr.contains(|c: char| c.is_whitespace() || c == '>' || c == '<') {
            return None;
        }
        let mut j = eq + 1;
        while j < s.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = *bytes.get(j)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let close = s[j + 1..].find(quote as char).map(|n| j + 1 + n)?;
        attrs.push((attr, s[j..=close].to_owned(), newline));
        newline = false;
        i = close + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(XmlKind::of("app/src/main/res/layout-land/main.xml"), XmlKind::Layout);
        assert_eq!(XmlKind::of("app/src/main/res/values-fr/strings.xml"), XmlKind::Values);
        assert_eq!(XmlKind::of("app/src/main/res/drawable/a.xml"), XmlKind::Resource);
        assert_eq!(XmlKind::of("app/src/main/AndroidManifest.xml"), XmlKind::Manifest);
        assert_eq!(XmlKind::of("pom.xml"), XmlKind::Plain);
    }

    #[test]
    fn formats_layouts() {
        let text = r#"<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android" android:orientation="vertical"
  android:layout_height="match_parent" xmlns:tools="http://schemas.android.com/tools" android:layout_width="match_parent">
<TextView android:text="@string/hi" android:layout_marginTop="8dp" android:id="@+id/title" android:layout_width="wrap_content" android:layout_height="wrap_content" tools:text="Hi" android:layout_marginStart="4dp"/>
        <!-- buttons -->
  <Button android:layout_width="wrap_content" android:layout_height="wrap_content"></Button>
<requestFocus/>
</LinearLayout>
"#;
        let want = r#"<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools"
    android:layout_width="match_parent"
    android:layout_height="match_parent"
    android:orientation="vertical">

    <TextView
        android:id="@+id/title"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:layout_marginStart="4dp"
        android:layout_marginTop="8dp"
        android:text="@string/hi"
        tools:text="Hi" />
    <!-- buttons -->

    <Button
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"></Button>

    <requestFocus />
</LinearLayout>
"#;
        let got = reformat_xml(XmlKind::Layout, text);
        assert_eq!(got, want, "\n{got}");
        // Formatting again changes nothing.
        assert_eq!(reformat_xml(XmlKind::Layout, &got), got);
    }

    #[test]
    fn formats_values_and_manifests() {
        let text = "<resources>\n<string name=\"a\"   translatable=\"false\">Hello  <b>you</b></string>\n<style parent=\"Base\" name=\"Theme\">\n<item name=\"x\">1</item></style>\n    <color name=\"c\">#fff</color>\n</resources>";
        let want = "<resources>\n    <string name=\"a\" translatable=\"false\">Hello  <b>you</b></string>\n\n    <style name=\"Theme\" parent=\"Base\">\n        <item name=\"x\">1</item>\n    </style>\n\n    <color name=\"c\">#fff</color>\n</resources>";
        assert_eq!(reformat_xml(XmlKind::Values, text), want);
        let manifest = "<manifest xmlns:android=\"a\" package=\"p\"><uses-permission android:name=\"A\"/><uses-permission android:name=\"B\"/><application android:label=\"L\" android:icon=\"I\"><activity android:name=\".Main\"/></application></manifest>";
        let want = "<manifest xmlns:android=\"a\"\n    package=\"p\">\n\n    <uses-permission android:name=\"A\" />\n    <uses-permission android:name=\"B\" />\n\n    <application\n        android:icon=\"I\"\n        android:label=\"L\">\n        <activity android:name=\".Main\" />\n    </application>\n</manifest>";
        let got = reformat_xml(XmlKind::Manifest, manifest);
        assert_eq!(got, want, "\n{got}");
    }

    #[test]
    fn keeps_plain_xml_and_broken_files() {
        let text = "<project a=\"1\"\n         b=\"2\">\n<x/>\n</project>";
        assert_eq!(reformat_xml(XmlKind::Plain, text), "<project a=\"1\"\n         b=\"2\">\n    <x />\n</project>");
        let broken = "<a><b></a>";
        assert_eq!(reformat_xml(XmlKind::Layout, broken), broken);
        assert_eq!(android_order("layout_marginStart").0, 7);
        assert!(android_order("paddingStart") < android_order("paddingTop"));
    }
}

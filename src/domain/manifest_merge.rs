//! Android Studio's Merged Manifest, as the Gradle plugin's manifest
//! merger (ManifestMerger2) makes it: the module's main manifest with its
//! build type's overlay above it and its library modules' manifests below,
//! elements matched by tag and `android:name`, the higher priority's
//! attributes winning (a different value is a conflict unless it says
//! `tools:replace`), `tools:node="remove" | "removeAll" | "replace"` and
//! `tools:remove`, `${placeholders}`, and the build file's values injected
//! (`package`, `versionCode` / `versionName`, `<uses-sdk>`). Each line of
//! the result knows the file it came from, for the view's colors.

use std::fmt::Write as _;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
const TOOLS: &str = "http://schemas.android.com/tools";

/// A manifest to merge: what the view calls it, and its text.
#[derive(Clone, Debug)]
pub struct ManifestSource {
    pub label: String,
    pub text: String,
}

/// What the build file says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildValues {
    pub application_id: Option<String>,
    pub namespace: Option<String>,
    pub min_sdk: Option<String>,
    pub target_sdk: Option<String>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    /// `manifestPlaceholders`.
    pub placeholders: Vec<(String, String)>,
}

/// A line of the merged manifest and the source it came from (an index
/// into `Merged::sources`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedLine {
    pub text: String,
    pub source: usize,
    /// The 0-based line in the source it came from (none for what the
    /// build file injected).
    pub line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    pub lines: Vec<MergedLine>,
    /// The manifests by priority, then the build file when it injected
    /// anything.
    pub sources: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
struct Attr {
    /// `android:name`, `package`, `tools:node`.
    name: String,
    value: String,
    source: usize,
    line: Option<u32>,
}

#[derive(Clone, Debug)]
struct Element {
    tag: String,
    attrs: Vec<Attr>,
    children: Vec<Element>,
    source: usize,
    line: Option<u32>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }

    fn set(&mut self, name: &str, value: &str, source: usize) {
        match self.attrs.iter_mut().find(|a| a.name == name) {
            Some(a) => {
                a.value = value.to_owned();
                a.source = source;
                a.line = None;
            }
            None => self.attrs.push(Attr { name: name.to_owned(), value: value.to_owned(), source, line: None }),
        }
    }

    /// What matches it in another manifest: the tag and `android:name`;
    /// an intent filter by its content.
    fn key(&self) -> String {
        if self.tag == "intent-filter" {
            let mut inner = String::new();
            for c in &self.children {
                let _ = write!(inner, "{}={};", c.tag, c.attr("android:name").unwrap_or_default());
            }
            return format!("intent-filter[{inner}]");
        }
        match self.attr("android:name") {
            Some(name) => format!("{}#{name}", self.tag),
            None => self.tag.clone(),
        }
    }

    /// `activity#com.example.Main`, as the merger's messages name it.
    fn describe(&self) -> String {
        self.attr("android:name").map_or_else(|| self.tag.clone(), |n| format!("{}#{n}", self.tag))
    }
}

fn qualified(node: roxmltree::Node, namespace: Option<&str>, local: &str) -> String {
    match namespace {
        Some(ANDROID) => format!("android:{local}"),
        Some(TOOLS) => format!("tools:{local}"),
        Some(ns) => match node.lookup_prefix(ns) {
            Some(prefix) => format!("{prefix}:{local}"),
            None => local.to_owned(),
        },
        None => local.to_owned(),
    }
}

fn element(doc: &roxmltree::Document, node: roxmltree::Node, source: usize) -> Element {
    let line = |pos: usize| Some(doc.text_pos_at(pos).row.saturating_sub(1));
    Element {
        tag: node.tag_name().name().to_owned(),
        attrs: node.attributes().map(|a| Attr { name: qualified(node, a.namespace(), a.name()), value: a.value().to_owned(), source, line: line(a.range().start) }).collect(),
        children: node.children().filter(|c| c.is_element()).map(|c| element(doc, c, source)).collect(),
        source,
        line: line(node.range().start),
    }
}

/// Merges `lower` (a lower priority manifest's element) into `higher`.
fn merge_into(higher: &mut Element, lower: &Element, labels: &[String], errors: &mut Vec<String>) {
    // `tools:node="replace"`: the higher one alone.
    if higher.attr("tools:node") == Some("replace") {
        return;
    }
    let replaced: Vec<String> = higher.attr("tools:replace").map(|r| r.split(',').map(|s| s.trim().to_owned()).collect()).unwrap_or_default();
    let removed: Vec<String> = higher.attr("tools:remove").map(|r| r.split(',').map(|s| s.trim().to_owned()).collect()).unwrap_or_default();
    for attr in &lower.attrs {
        if attr.name.starts_with("tools:") || attr.name.starts_with("xmlns") || removed.contains(&attr.name) {
            continue;
        }
        match higher.attrs.iter().find(|a| a.name == attr.name) {
            None => higher.attrs.push(attr.clone()),
            Some(mine) if mine.value != attr.value && !replaced.contains(&attr.name) => errors.push(format!(
                "Attribute {}@{} value=({}) from {} is also present at {} value=({}). Suggestion: add 'tools:replace=\"{}\"' to <{}> element at {} to override.",
                higher.describe(),
                attr.name,
                mine.value,
                labels[mine.source],
                labels[attr.source],
                attr.value,
                attr.name,
                higher.tag,
                labels[mine.source]
            )),
            Some(_) => {}
        }
    }
    for child in &lower.children {
        let key = child.key();
        // `tools:node="removeAll"` takes every lower element of its tag.
        if higher.children.iter().any(|c| c.tag == child.tag && c.attr("tools:node") == Some("removeAll")) {
            continue;
        }
        match higher.children.iter_mut().find(|c| c.key() == key) {
            Some(mine) if mine.attr("tools:node") == Some("remove") => {}
            Some(mine) => merge_into(mine, child, labels, errors),
            None => higher.children.push(child.clone()),
        }
    }
}

/// Merges an overlay (a higher priority manifest) into `base`: its
/// attributes win (a different value needs its `tools:replace`), its
/// elements are added or merged, `tools:node` acts on the base's.
fn merge_higher(base: &mut Element, higher: &Element, labels: &[String], errors: &mut Vec<String>) {
    let replaced: Vec<&str> = higher.attr("tools:replace").map(|r| r.split(',').map(str::trim).collect()).unwrap_or_default();
    for attr in &higher.attrs {
        if attr.name.starts_with("xmlns") {
            continue;
        }
        if let Some(mine) = base.attrs.iter().find(|a| a.name == attr.name) {
            if mine.value != attr.value && !attr.name.starts_with("tools:") && !replaced.contains(&attr.name.as_str()) {
                errors.push(format!(
                    "Attribute {}@{} value=({}) from {} is also present at {} value=({}). Suggestion: add 'tools:replace=\"{}\"' to <{}> element at {} to override.",
                    base.describe(),
                    attr.name,
                    attr.value,
                    labels[attr.source],
                    labels[mine.source],
                    mine.value,
                    attr.name,
                    base.tag,
                    labels[attr.source]
                ));
            }
        }
        base.set(&attr.name, &attr.value, attr.source);
    }
    if let Some(r) = higher.attr("tools:remove") {
        let removed: Vec<&str> = r.split(',').map(str::trim).collect();
        base.attrs.retain(|a| !removed.contains(&a.name.as_str()));
    }
    for child in &higher.children {
        let key = child.key();
        match base.children.iter().position(|c| c.key() == key) {
            Some(ix) if matches!(child.attr("tools:node"), Some("replace" | "remove")) => base.children[ix] = child.clone(),
            Some(ix) => merge_higher(&mut base.children[ix], child, labels, errors),
            None => base.children.push(child.clone()),
        }
    }
}

/// Drops what only told the merger something: `tools:` attributes and
/// elements marked removed.
fn strip(element: &mut Element) {
    element.attrs.retain(|a| !a.name.starts_with("tools:") && a.name != "xmlns:tools");
    element.children.retain(|c| !matches!(c.attr("tools:node"), Some("remove" | "removeAll")));
    for c in &mut element.children {
        strip(c);
    }
}

fn substitute(element: &mut Element, placeholders: &[(String, String)]) {
    for a in &mut element.attrs {
        for (key, value) in placeholders {
            a.value = a.value.replace(&format!("${{{key}}}"), value);
        }
    }
    for c in &mut element.children {
        substitute(c, placeholders);
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The merged tree as the view prints it: four-space indents, each
/// attribute on its own line after the first, as Android Studio shows it.
fn print(element: &Element, depth: usize, out: &mut Vec<MergedLine>) {
    let indent = "    ".repeat(depth);
    let attr_indent = format!("{indent}    ");
    let close = if element.children.is_empty() { " />" } else { " >" };
    if element.attrs.is_empty() {
        out.push(MergedLine { text: format!("{indent}<{}{}", element.tag, close.trim_start()), source: element.source, line: element.line });
    } else {
        out.push(MergedLine { text: format!("{indent}<{}", element.tag), source: element.source, line: element.line });
        let last = element.attrs.len() - 1;
        for (i, a) in element.attrs.iter().enumerate() {
            let end = if i == last { close } else { "" };
            out.push(MergedLine { text: format!("{attr_indent}{}=\"{}\"{end}", a.name, escape(&a.value)), source: a.source, line: a.line });
        }
    }
    if !element.children.is_empty() {
        for c in &element.children {
            print(c, depth + 1, out);
        }
        out.push(MergedLine { text: format!("{indent}</{}>", element.tag), source: element.source, line: element.line });
    }
}

/// Merges the manifests, highest priority first (the build type's
/// overlay, the main manifest, then the libraries').
pub fn merge(sources: &[ManifestSource], main: usize, build: &BuildValues, build_label: &str) -> Result<Merged, String> {
    let mut labels: Vec<String> = sources.iter().map(|s| s.label.clone()).collect();
    let build_source = labels.len();
    labels.push(build_label.to_owned());
    let mut errors = Vec::new();
    let mut trees = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        let doc = roxmltree::Document::parse(&s.text).map_err(|e| format!("{}: {e}", s.label))?;
        let root = doc.root_element();
        if root.tag_name().name() != "manifest" {
            return Err(format!("{}: the root element is not <manifest>", s.label));
        }
        trees.push(element(&doc, root, i));
    }
    if main >= trees.len() {
        return Err("No main manifest".into());
    }
    // The main manifest's structure; the overlays above it merged in,
    // nearest first, then the libraries below it.
    let mut root = trees[main].clone();
    for overlay in trees[..main].iter().rev() {
        merge_higher(&mut root, overlay, &labels, &mut errors);
    }
    for library in &trees[main + 1..] {
        // A library's own package is its namespace, not the app's.
        let mut lower = library.clone();
        lower.attrs.retain(|a| a.name != "package");
        merge_into(&mut root, &lower, &labels, &mut errors);
    }
    // The build file's values.
    let mut injected = false;
    let mut inject = |root: &mut Element, name: &str, value: &Option<String>| {
        if let Some(v) = value {
            root.set(name, v, build_source);
            injected = true;
        }
    };
    inject(&mut root, "package", &build.application_id.clone().or(build.namespace.clone()));
    inject(&mut root, "android:versionCode", &build.version_code);
    inject(&mut root, "android:versionName", &build.version_name);
    if build.min_sdk.is_some() || build.target_sdk.is_some() {
        let ix = match root.children.iter().position(|c| c.tag == "uses-sdk") {
            Some(ix) => ix,
            None => {
                // The merger puts it first.
                root.children.insert(0, Element { tag: "uses-sdk".into(), attrs: Vec::new(), children: Vec::new(), source: build_source, line: None });
                0
            }
        };
        let sdk = &mut root.children[ix];
        inject(sdk, "android:minSdkVersion", &build.min_sdk);
        inject(sdk, "android:targetSdkVersion", &build.target_sdk.clone().or(build.min_sdk.clone()));
    }
    let mut placeholders = build.placeholders.clone();
    if let Some(id) = build.application_id.clone().or(build.namespace.clone()) {
        placeholders.push(("applicationId".into(), id));
    }
    substitute(&mut root, &placeholders);
    strip(&mut root);
    root.attrs.insert(0, Attr { name: "xmlns:android".into(), value: ANDROID.into(), source: main, line: root.line });
    let mut lines = Vec::new();
    lines.push(MergedLine { text: "<?xml version=\"1.0\" encoding=\"utf-8\"?>".into(), source: main, line: Some(0) });
    print(&root, 0, &mut lines);
    if !injected {
        labels.pop();
    }
    Ok(Merged { lines, sources: labels, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = r#"xmlns:android="http://schemas.android.com/apk/res/android" xmlns:tools="http://schemas.android.com/tools""#;

    fn source(label: &str, body: &str) -> ManifestSource {
        ManifestSource { label: label.into(), text: format!("<manifest {NS}>{body}</manifest>") }
    }

    #[test]
    fn merges_like_the_gradle_plugin() {
        let main = source(
            "app/src/main/AndroidManifest.xml",
            r#"<uses-permission android:name="android.permission.INTERNET"/>
               <application android:label="App" android:icon="@mipmap/ic">
                 <activity android:name=".Main" android:exported="true">
                   <intent-filter><action android:name="android.intent.action.MAIN"/></intent-filter>
                 </activity>
                 <meta-data android:name="key" android:value="${apiKey}"/>
                 <provider android:name="lib.Init" tools:node="remove"/>
               </application>"#,
        );
        let debug = source("app/src/debug/AndroidManifest.xml", r#"<application android:label="App (debug)" tools:replace="android:label"/>"#);
        let lib = source(
            "lib/src/main/AndroidManifest.xml",
            r#"<uses-permission android:name="android.permission.INTERNET"/>
               <uses-permission android:name="android.permission.CAMERA"/>
               <application android:icon="@mipmap/lib" android:allowBackup="true">
                 <activity android:name="lib.Scan"/>
                 <provider android:name="lib.Init"/>
               </application>"#,
        );
        let build = BuildValues {
            application_id: Some("com.example.app".into()),
            min_sdk: Some("24".into()),
            target_sdk: Some("35".into()),
            version_code: Some("3".into()),
            placeholders: vec![("apiKey".into(), "secret".into())],
            ..Default::default()
        };
        let merged = merge(&[debug, main, lib], 1, &build, "app/build.gradle.kts").unwrap();
        let text: Vec<&str> = merged.lines.iter().map(|l| l.text.as_str()).collect();
        let joined = text.join("\n");
        // The overlay's label (tools:replace), the main icon over the library's (a conflict).
        assert!(joined.contains("android:label=\"App (debug)\""), "{joined}");
        assert_eq!(merged.errors.len(), 1, "{:?}", merged.errors);
        assert!(merged.errors[0].starts_with("Attribute application@android:icon value=(@mipmap/ic) from app/src/main/AndroidManifest.xml is also present at lib/src/main/AndroidManifest.xml value=(@mipmap/lib)"), "{}", merged.errors[0]);
        // The library's permission and activity added once; its provider removed.
        assert_eq!(joined.matches("android.permission.INTERNET").count(), 1);
        assert!(joined.contains("android.permission.CAMERA") && joined.contains("lib.Scan") && !joined.contains("lib.Init"), "{joined}");
        // Build values, placeholders; no tools attributes left.
        assert!(joined.contains("package=\"com.example.app\"") && joined.contains("android:versionCode=\"3\"") && joined.contains("android:minSdkVersion=\"24\""), "{joined}");
        assert!(joined.contains("android:value=\"secret\"") && !joined.contains("tools:"), "{joined}");
        // Line sources: the CAMERA permission from the library, minSdk from the build file.
        // The main manifest's order: its permission before its application.
        let at = |needle: &str| text.iter().position(|l| l.contains(needle)).unwrap();
        assert!(at("android.permission.INTERNET") < at("<application") && at("<application") < at("android.permission.CAMERA"), "{joined}");
        let source_of = |needle: &str| merged.lines.iter().find(|l| l.text.contains(needle)).map(|l| merged.sources[l.source].clone()).unwrap();
        assert_eq!(source_of("CAMERA"), "lib/src/main/AndroidManifest.xml");
        assert_eq!(source_of("minSdkVersion"), "app/build.gradle.kts");
        assert_eq!(source_of("App (debug)"), "app/src/debug/AndroidManifest.xml");
        // Each line knows its line in the source; injected ones don't.
        let line_of = |needle: &str| merged.lines.iter().find(|l| l.text.contains(needle)).unwrap().line;
        assert_eq!((line_of("CAMERA"), line_of("minSdkVersion")), (Some(1), None));
        assert!(roxmltree::Document::parse(&joined).is_ok(), "{joined}");
    }

    #[test]
    fn prints_attributes_one_per_line() {
        let merged = merge(&[source("m", r#"<application android:label="A"><activity android:name=".B"/></application>"#)], 0, &BuildValues::default(), "b").unwrap();
        let text: Vec<&str> = merged.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text[1], "<manifest");
        assert!(text.contains(&"    <application") && text.contains(&"        android:label=\"A\" >") && text.contains(&"            android:name=\".B\" />"), "{text:?}");
        assert_eq!(merged.sources, ["m"]);
    }
}

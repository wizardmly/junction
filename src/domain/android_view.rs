//! The Project window's Android view, as Android Studio draws it: each
//! Gradle module with its `manifests`, `kotlin+java` (packages, with
//! "(test)" / "(androidTest)" for the test source sets) and `res` (by
//! type, a resource's qualified copies under one node: "strings.xml (3)"),
//! every source set merged in; then "Gradle Scripts" with each build
//! file named for what it does ("(Module :app)", "(Project Settings)").

use std::collections::BTreeMap;

/// What a node is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Module,
    /// manifests, assets, a res type folder, a resource's copies.
    Folder,
    /// kotlin+java / java.
    Sources,
    Res,
    Package,
    Scripts,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNode {
    pub label: String,
    /// Greyed after the label: "(debug)", "(Module :app)", "(test)".
    pub note: Option<String>,
    /// Unique among the nodes; a file's is its path.
    pub key: String,
    /// The project-relative file a file node opens, the folder a module is.
    pub path: Option<String>,
    pub kind: ViewKind,
    pub children: Vec<ViewNode>,
}

impl ViewNode {
    fn group(label: &str, key: String, kind: ViewKind) -> Self {
        ViewNode { label: label.to_owned(), note: None, key, path: None, kind, children: Vec::new() }
    }

    fn file(label: String, note: Option<String>, path: &str) -> Self {
        ViewNode { label, note, key: path.to_owned(), path: Some(path.to_owned()), kind: ViewKind::File, children: Vec::new() }
    }

    /// The keys of the nodes above `path`'s file node, outermost first.
    pub fn ancestors_of(nodes: &[ViewNode], path: &str) -> Option<Vec<String>> {
        for node in nodes {
            if node.path.as_deref() == Some(path) && node.kind == ViewKind::File {
                return Some(Vec::new());
            }
            if let Some(mut keys) = Self::ancestors_of(&node.children, path) {
                keys.insert(0, node.key.clone());
                return Some(keys);
            }
        }
        None
    }
}

/// The prefix of the keys of nodes that aren't files or modules.
pub const VIRTUAL: char = '\u{1}';

const BUILD_FILES: [&str; 2] = ["build.gradle", "build.gradle.kts"];

fn split_dir(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The module's name as Gradle writes it: "app", "core:data".
fn module_name(dir: &str) -> String {
    dir.replace('/', ":")
}

/// A test source set's note: "(test)", "(androidTest)"; none for the
/// sets merged into the module's own code (main, debug, a flavor).
fn set_note(set: &str) -> Option<&'static str> {
    if set.starts_with("androidTest") {
        Some("androidTest")
    } else if set.starts_with("test") {
        Some("test")
    } else {
        None
    }
}

/// The view of the project's `files` (project-relative, `/`-separated),
/// named `project`.
pub fn android_tree(project: &str, files: &[String]) -> Vec<ViewNode> {
    let mut modules: Vec<&str> = files
        .iter()
        .filter_map(|f| {
            let (dir, name) = split_dir(f);
            (BUILD_FILES.contains(&name) && !dir.is_empty()).then_some(dir)
        })
        .collect();
    modules.sort();
    modules.dedup();
    let module_of = |f: &str| modules.iter().filter(|m| f.starts_with(&format!("{m}/"))).max_by_key(|m| m.len()).copied();
    let mut out: Vec<ViewNode> = modules
        .iter()
        .map(|m| ViewNode { label: module_name(m), note: None, key: m.to_string(), path: Some(m.to_string()), kind: ViewKind::Module, children: Vec::new() })
        .collect();

    // Per module: manifests, sources by (note, package), res by type then name, assets.
    #[derive(Default)]
    struct Parts<'f> {
        manifests: Vec<(&'f str, Option<String>)>,
        kotlin: bool,
        sources: BTreeMap<(Option<&'static str>, String), Vec<&'f str>>,
        res: BTreeMap<String, BTreeMap<String, Vec<(&'f str, Option<String>)>>>,
        assets: Vec<(&'f str, String)>,
    }
    let mut parts: Vec<Parts> = modules.iter().map(|_| Parts::default()).collect();
    for f in files {
        let Some(m) = module_of(f) else { continue };
        let ix = modules.iter().position(|x| *x == m).unwrap();
        let Some(rest) = f[m.len() + 1..].strip_prefix("src/") else { continue };
        let Some((set, inner)) = rest.split_once('/') else { continue };
        let p = &mut parts[ix];
        let variant_note = (set != "main" && set_note(set).is_none()).then(|| set.to_owned());
        if inner == "AndroidManifest.xml" {
            p.manifests.push((f, variant_note));
        } else if let Some(src) = inner.strip_prefix("java/").or_else(|| inner.strip_prefix("kotlin/")) {
            p.kotlin |= src.ends_with(".kt") || inner.starts_with("kotlin/");
            let (dir, _) = split_dir(src);
            p.sources.entry((set_note(set), dir.replace('/', "."))).or_default().push(f);
        } else if let Some(res) = inner.strip_prefix("res/") {
            let Some((folder, name)) = res.split_once('/') else { continue };
            if name.contains('/') {
                continue;
            }
            let (ty, qualifiers) = folder.split_once('-').unwrap_or((folder, ""));
            let mut note: Vec<&str> = qualifiers.split('-').filter(|q| !q.is_empty()).collect();
            if let Some(v) = &variant_note {
                note.push(v);
            }
            let note = (!note.is_empty()).then(|| note.join(", "));
            p.res.entry(ty.to_owned()).or_default().entry(name.to_owned()).or_default().push((f, note));
        } else if let Some(asset) = inner.strip_prefix("assets/") {
            p.assets.push((f, asset.to_owned()));
        }
    }

    for ((node, m), p) in out.iter_mut().zip(&modules).zip(parts) {
        let key = |what: &str| format!("{VIRTUAL}{m}/{what}");
        if !p.manifests.is_empty() {
            let mut group = ViewNode::group("manifests", key("manifests"), ViewKind::Folder);
            for (f, note) in &p.manifests {
                group.children.push(ViewNode::file("AndroidManifest.xml".into(), note.as_ref().map(|n| format!("({n})")), f));
            }
            node.children.push(group);
        }
        if !p.sources.is_empty() {
            let mut group = ViewNode::group(if p.kotlin { "kotlin+java" } else { "java" }, key("java"), ViewKind::Sources);
            // Main code's packages first, then the tests', as Android Studio lists them.
            let mut packages: Vec<_> = p.sources.into_iter().collect();
            packages.sort_by_key(|((note, package), _)| (note.is_some(), *note == Some("test"), package.clone()));
            for ((note, package), mut sources) in packages {
                sources.sort_by_key(|f| split_dir(f).1.to_lowercase());
                let label = if package.is_empty() { "<default>".to_owned() } else { package.clone() };
                let mut pkg = ViewNode { note: note.map(|n| format!("({n})")), ..ViewNode::group(&label, key(&format!("java/{}/{package}", note.unwrap_or("main"))), ViewKind::Package) };
                pkg.children = sources.iter().map(|f| ViewNode::file(split_dir(f).1.to_owned(), None, f)).collect();
                group.children.push(pkg);
            }
            node.children.push(group);
        }
        if !p.res.is_empty() {
            let mut group = ViewNode::group("res", key("res"), ViewKind::Res);
            for (ty, names) in p.res {
                let mut folder = ViewNode::group(&ty, key(&format!("res/{ty}")), ViewKind::Folder);
                for (name, mut copies) in names {
                    copies.sort_by_key(|(_, note)| note.clone());
                    let note = |n: &Option<String>| n.as_ref().map(|n| format!("({n})"));
                    if let [(f, n)] = copies.as_slice() {
                        folder.children.push(ViewNode::file(name.clone(), note(n), f));
                    } else {
                        let mut copies_node = ViewNode { note: Some(format!("({})", copies.len())), ..ViewNode::group(&name, key(&format!("res/{ty}/{name}")), ViewKind::Folder) };
                        copies_node.children = copies.iter().map(|(f, n)| ViewNode::file(name.clone(), note(n), f)).collect();
                        folder.children.push(copies_node);
                    }
                }
                group.children.push(folder);
            }
            node.children.push(group);
        }
        if !p.assets.is_empty() {
            let mut group = ViewNode::group("assets", key("assets"), ViewKind::Folder);
            let mut assets = p.assets;
            assets.sort_by(|a, b| a.1.cmp(&b.1));
            group.children = assets.into_iter().map(|(f, name)| ViewNode::file(name, None, f)).collect();
            node.children.push(group);
        }
    }
    // A module without sources (an empty folder with a build file) still shows.
    out.push(gradle_scripts(project, files, &modules));
    out
}

/// "Gradle Scripts": the build files, settings, properties and catalogs,
/// each with what it is.
fn gradle_scripts(project: &str, files: &[String], modules: &[&str]) -> ViewNode {
    let mut scripts: Vec<(u8, String, ViewNode)> = Vec::new();
    for f in files {
        let (dir, name) = split_dir(f);
        let note = if BUILD_FILES.contains(&name) {
            if dir.is_empty() { Some((0, format!("Project: {project}"))) } else { Some((1, format!("Module :{}", module_name(dir)))) }
        } else if name.starts_with("settings.gradle") && dir.is_empty() {
            Some((2, "Project Settings".into()))
        } else if name == "gradle.properties" && dir.is_empty() {
            Some((3, "Project Properties".into()))
        } else if name == "gradle-wrapper.properties" && dir == "gradle/wrapper" {
            Some((4, "Gradle Version".into()))
        } else if name.ends_with(".versions.toml") && dir == "gradle" {
            Some((5, "Version Catalog".into()))
        } else if name == "local.properties" && dir.is_empty() {
            Some((6, "SDK Location".into()))
        } else if name.starts_with("proguard-rules") && modules.contains(&dir) {
            Some((7, format!("ProGuard Rules for \":{}\"", module_name(dir))))
        } else {
            None
        };
        if let Some((order, note)) = note {
            scripts.push((order, f.clone(), ViewNode::file(name.to_owned(), Some(format!("({note})")), f)));
        }
    }
    scripts.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let mut group = ViewNode::group("Gradle Scripts", format!("{VIRTUAL}gradle-scripts"), ViewKind::Scripts);
    group.children = scripts.into_iter().map(|(_, _, n)| n).collect();
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(nodes: &[ViewNode], depth: usize, out: &mut Vec<String>) {
        for n in nodes {
            let note = n.note.as_ref().map(|n| format!(" {n}")).unwrap_or_default();
            out.push(format!("{}{}{note}", "  ".repeat(depth), n.label));
            show(&n.children, depth + 1, out);
        }
    }

    #[test]
    fn modules_with_their_sources_and_resources() {
        let files: Vec<String> = [
            "build.gradle.kts",
            "settings.gradle.kts",
            "gradle.properties",
            "gradle/libs.versions.toml",
            "gradle/wrapper/gradle-wrapper.properties",
            "README.md",
            "app/build.gradle.kts",
            "app/proguard-rules.pro",
            "app/src/main/AndroidManifest.xml",
            "app/src/debug/AndroidManifest.xml",
            "app/src/main/java/com/example/MainActivity.kt",
            "app/src/main/java/com/example/ui/Theme.kt",
            "app/src/free/java/com/example/Ads.kt",
            "app/src/test/java/com/example/MainTest.kt",
            "app/src/androidTest/java/com/example/UiTest.kt",
            "app/src/main/res/values/strings.xml",
            "app/src/main/res/values-fr/strings.xml",
            "app/src/main/res/values/colors.xml",
            "app/src/main/res/mipmap-hdpi/ic_launcher.webp",
            "app/src/main/res/layout/main.xml",
            "app/src/debug/res/values/strings.xml",
            "app/src/main/assets/data/a.json",
            "core/data/build.gradle",
            "core/data/src/main/java/d/Repo.java",
        ]
        .map(String::from)
        .to_vec();
        let tree = android_tree("demo", &files);
        let mut out = Vec::new();
        show(&tree, 0, &mut out);
        assert_eq!(
            out.join("\n"),
            "app
  manifests
    AndroidManifest.xml
    AndroidManifest.xml (debug)
  kotlin+java
    com.example
      Ads.kt
      MainActivity.kt
    com.example.ui
      Theme.kt
    com.example (androidTest)
      UiTest.kt
    com.example (test)
      MainTest.kt
  res
    layout
      main.xml
    mipmap
      ic_launcher.webp (hdpi)
    values
      colors.xml
      strings.xml (3)
        strings.xml
        strings.xml (debug)
        strings.xml (fr)
  assets
    data/a.json
core:data
  java
    d
      Repo.java
Gradle Scripts
  build.gradle.kts (Project: demo)
  build.gradle.kts (Module :app)
  build.gradle (Module :core:data)
  settings.gradle.kts (Project Settings)
  gradle.properties (Project Properties)
  gradle-wrapper.properties (Gradle Version)
  libs.versions.toml (Version Catalog)
  proguard-rules.pro (ProGuard Rules for \":app\")"
        );
        assert_eq!(
            ViewNode::ancestors_of(&tree, "app/src/debug/res/values/strings.xml").unwrap(),
            ["app", "\u{1}app/res", "\u{1}app/res/values", "\u{1}app/res/values/strings.xml"]
        );
        assert_eq!(ViewNode::ancestors_of(&tree, "app/build.gradle.kts").unwrap(), ["\u{1}gradle-scripts"]);
    }
}

//! Android resources for navigation and previews: the project's resource
//! table, where a reference leads, and the value a color or string has.

use std::collections::HashMap;

use crate::domain::android_resources::{self as res, Declaration, ResourceRef, ResourceSource};

/// All declarations in one block, grouped by resource (one allocation, so
/// a big app's few hundred thousand don't fragment the heap), and where
/// each resource's group is.
#[derive(Default)]
pub struct ResourceTable {
    all: Vec<Declaration>,
    by_ref: HashMap<ResourceRef, (u32, u32)>,
}

impl ResourceTable {
    pub fn build(source: &dyn ResourceSource) -> Self {
        let mut all = source.scan();
        // By resource, then the default folder's first.
        all.sort_unstable_by(|a, b| {
            (&a.resource, res::folder_order(&a.folder), &a.path, a.line).cmp(&(&b.resource, res::folder_order(&b.folder), &b.path, b.line))
        });
        // An id declared in several places of one file counts once per file.
        all.dedup_by(|a, b| a.resource == b.resource && a.path == b.path && &*a.resource.kind == "id");
        all.shrink_to_fit();
        let mut by_ref: HashMap<ResourceRef, (u32, u32)> = HashMap::new();
        let mut start = 0;
        for i in 1..=all.len() {
            if i == all.len() || all[i].resource != all[start].resource {
                // One copy of each name for all its declarations.
                let key = all[start].resource.clone();
                for d in &mut all[start + 1..i] {
                    d.resource = key.clone();
                }
                by_ref.insert(key, (start as u32, i as u32));
                start = i;
            }
        }
        Self { all, by_ref }
    }

    /// (resources, declarations, values) for benchmarks.
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.by_ref.len(), self.all.len(), self.all.iter().filter(|d| d.value.is_some()).count())
    }

    /// Go to Declaration: every declaration, the default folder's first.
    pub fn declarations(&self, r: &ResourceRef) -> &[Declaration] {
        self.by_ref.get(r).map_or(&[], |&(a, b)| &self.all[a as usize..b as usize])
    }

    /// The default value (the unqualified folder's), following references
    /// (`@color/primary` → `#FF6200EE`).
    pub fn value(&self, r: &ResourceRef) -> Option<String> {
        let mut r = r.clone();
        for _ in 0..8 {
            let value = self.declarations(&r).iter().find_map(|d| d.value.clone())?;
            match value.strip_prefix('@').and_then(|v| v.split_once('/')) {
                Some((kind, name)) if !kind.starts_with("android") => r = ResourceRef::new(kind, name),
                _ => return Some(value),
            }
        }
        None
    }

    /// A color resource's RGBA.
    pub fn color(&self, r: &ResourceRef) -> Option<u32> {
        self.value(r).and_then(|v| res::parse_color(&v))
    }

    /// A drawable or mipmap's bitmap to preview (the densest PNG / WebP / JPG).
    pub fn image(&self, r: &ResourceRef) -> Option<&Declaration> {
        if !matches!(&*r.kind, "drawable" | "mipmap") {
            return None;
        }
        let density = |folder: &str| ["xxxhdpi", "xxhdpi", "xhdpi", "hdpi", "mdpi"].iter().position(|d| folder.contains(d)).unwrap_or(5);
        self.declarations(r).iter().filter(|d| [".png", ".webp", ".jpg", ".jpeg", ".gif"].iter().any(|e| d.path.ends_with(e)) && !d.path.ends_with(".9.png")).min_by_key(|d| density(&d.folder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<Declaration>);
    impl ResourceSource for Fake {
        fn scan(&self) -> Vec<Declaration> {
            self.0.clone()
        }
    }

    fn decl(kind: &str, name: &str, folder: &str, value: Option<&str>) -> Declaration {
        Declaration { resource: ResourceRef::new(kind, name), path: format!("app/src/main/res/{folder}/x.xml").into(), folder: folder.into(), line: 1, col: 1, value: value.map(Into::into) }
    }

    #[test]
    fn resolves_values() {
        let t = ResourceTable::build(&Fake(vec![
            decl("string", "app_name", "values-zh", Some("应用")),
            decl("string", "app_name", "values", Some("App")),
            decl("color", "primary", "values", Some("@color/purple")),
            decl("color", "purple", "values", Some("#6200EE")),
        ]));
        let name = ResourceRef::new("string", "app_name");
        assert_eq!(&*t.declarations(&name)[0].folder, "values");
        assert_eq!(t.value(&name).as_deref(), Some("App"));
        assert_eq!(t.color(&ResourceRef::new("color", "primary")), Some(0x6200EEFF));
        let marks = previews(&t, "a/Main.kt", "x\nval c = R.color.primary\nR.color.none");
        assert_eq!(marks, vec![(1, Preview::Color(0x6200EEFF))]);
        let marks = previews(&t, "res/layout/a.xml", "<View android:background=\"#F00\"\n android:tint=\"@color/purple\"/>");
        assert_eq!(marks, vec![(0, Preview::Color(0xFF0000FF)), (1, Preview::Color(0x6200EEFF))]);
    }
}

/// What the editor gutter previews on a line, as Android Studio does: a
/// color's swatch or a bitmap drawable's thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// RGBA.
    Color(u32),
    /// The image, relative to the project.
    Image(String),
}

/// The first previewable resource of each line of a Kotlin, Java or XML file.
pub fn previews(table: &ResourceTable, path: &str, text: &str) -> Vec<(u32, Preview)> {
    use std::sync::LazyLock;
    static CODE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?:^|[^\w.])R\.(color|drawable|mipmap)\.(\w+)").unwrap());
    static XML: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r#"@(color|drawable|mipmap)/([\w.]+)|[">](#[0-9a-fA-F]{3,8})["<]"#).unwrap());
    let code = path.ends_with(".kt") || path.ends_with(".java");
    if !code && !path.ends_with(".xml") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let caps = if code { CODE.captures(content) } else { XML.captures(content) };
        let Some(c) = caps else { continue };
        let preview = match (c.get(1), c.get(2), c.get(3)) {
            (_, _, Some(hex)) => res::parse_color(hex.as_str()).map(Preview::Color),
            (Some(kind), Some(name), _) => {
                let r = ResourceRef::new(kind.as_str(), name.as_str());
                match kind.as_str() {
                    "color" => table.color(&r).map(Preview::Color),
                    _ => table.image(&r).map(|d| Preview::Image(d.path.to_string())),
                }
            }
            _ => None,
        };
        if let Some(p) = preview {
            out.push((line as u32, p));
        }
    }
    out
}

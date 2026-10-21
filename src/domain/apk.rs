//! APK Analyzer: an APK's (or AAB's) files as a tree with their raw and
//! download sizes, and the share each has of the download.

/// One file of the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApkEntry {
    pub path: String,
    pub raw: u64,
    /// Its size compressed for download, as Play estimates it.
    pub download: u64,
}

/// A folder or file of the tree; folders sum their files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApkNode {
    pub name: String,
    /// The archive path ("res/drawable"), without a trailing slash.
    pub path: String,
    pub raw: u64,
    pub download: u64,
    pub children: Vec<ApkNode>,
}

impl ApkNode {
    pub fn is_dir(&self) -> bool {
        !self.children.is_empty()
    }
}

/// The tree of `entries`, each level largest first as the analyzer sorts.
pub fn tree(entries: &[ApkEntry]) -> ApkNode {
    let mut root = ApkNode::default();
    for e in entries.iter().filter(|e| !e.path.ends_with('/')) {
        let mut node = &mut root;
        node.raw += e.raw;
        node.download += e.download;
        let parts: Vec<&str> = e.path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let path = parts[..=i].join("/");
            let ix = match node.children.iter().position(|c| c.name == *part) {
                Some(ix) => ix,
                None => {
                    node.children.push(ApkNode { name: part.to_string(), path, ..Default::default() });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[ix];
            node.raw += e.raw;
            node.download += e.download;
        }
    }
    sort(&mut root);
    root
}

fn sort(node: &mut ApkNode) {
    node.children.sort_by(|a, b| b.raw.cmp(&a.raw).then_with(|| a.name.cmp(&b.name)));
    for c in &mut node.children {
        sort(c);
    }
}

/// A folder or file of two archives compared, as the APK Size Differ
/// shows it: its raw size in the old and the new one (0 where missing).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApkDiffNode {
    pub name: String,
    pub path: String,
    pub old: u64,
    pub new: u64,
    pub children: Vec<ApkDiffNode>,
}

impl ApkDiffNode {
    pub fn is_dir(&self) -> bool {
        !self.children.is_empty()
    }

    pub fn diff(&self) -> i64 {
        self.new as i64 - self.old as i64
    }
}

/// `old` and `new` merged by path, each level ordered by how much it
/// grew (shrinking last), then by name.
pub fn diff(old: &ApkNode, new: &ApkNode) -> ApkDiffNode {
    fn merge(name: &str, path: &str, old: Option<&ApkNode>, new: Option<&ApkNode>) -> ApkDiffNode {
        let mut children: Vec<ApkDiffNode> = Vec::new();
        for o in old.map_or(&[][..], |n| &n.children) {
            let n = new.and_then(|n| n.children.iter().find(|c| c.name == o.name));
            children.push(merge(&o.name, &o.path, Some(o), n));
        }
        for n in new.map_or(&[][..], |n| &n.children) {
            if !old.is_some_and(|o| o.children.iter().any(|c| c.name == n.name)) {
                children.push(merge(&n.name, &n.path, None, Some(n)));
            }
        }
        children.sort_by(|a, b| b.diff().cmp(&a.diff()).then_with(|| a.name.cmp(&b.name)));
        ApkDiffNode { name: name.to_owned(), path: path.to_owned(), old: old.map_or(0, |n| n.raw), new: new.map_or(0, |n| n.raw), children }
    }
    merge("", "", Some(old), Some(new))
}

/// A size change: "+1.2 KB", "-512 B", "0 B".
pub fn signed_size(delta: i64) -> String {
    match delta {
        0 => "0 B".to_owned(),
        d if d > 0 => format!("+{}", size(d as u64)),
        d => format!("-{}", size(d.unsigned_abs())),
    }
}

/// A size as the analyzer writes it: "512 B", "12.3 KB", "4.5 MB".
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", b / 1024.0 / 1024.0 / 1024.0)
    }
}

/// `part`'s share of `whole`: "12.3%".
pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 { "0%".to_owned() } else { format!("{:.1}%", part as f64 * 100.0 / whole as f64) }
}

/// An APK or AAB opened for analysis.
pub trait ApkArchive {
    fn entries(&self) -> Vec<ApkEntry>;
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_size_tree() {
        let e = |path: &str, raw, download| ApkEntry { path: path.into(), raw, download };
        let root = tree(&[e("classes.dex", 100, 40), e("res/a.png", 30, 30), e("res/b/c.xml", 50, 10), e("res/", 0, 0)]);
        assert_eq!((root.raw, root.download), (180, 80));
        assert_eq!(root.children[0].name, "classes.dex");
        let res = &root.children[1];
        assert_eq!((res.path.as_str(), res.raw, res.download), ("res", 80, 40));
        assert_eq!(res.children[0].path, "res/b");
        assert_eq!(res.children[0].children[0].path, "res/b/c.xml");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(percent(1, 3), "33.3%");
    }

    #[test]
    fn compares_two_trees() {
        let e = |path: &str, raw| ApkEntry { path: path.into(), raw, download: raw };
        let old = tree(&[e("classes.dex", 100), e("res/a.png", 30), e("res/gone.xml", 20)]);
        let new = tree(&[e("classes.dex", 160), e("res/a.png", 30), e("lib/x.so", 5)]);
        let d = diff(&old, &new);
        assert_eq!((d.old, d.new, d.diff()), (150, 195, 45));
        let names: Vec<&str> = d.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["classes.dex", "lib", "res"]);
        let res = &d.children[2];
        assert_eq!((res.old, res.new), (50, 30));
        assert_eq!(res.children[1].name, "gone.xml");
        assert_eq!((res.children[1].old, res.children[1].new), (20, 0));
        assert_eq!(d.children[1].children[0].path, "lib/x.so");
        assert_eq!(signed_size(-512), "-512 B");
        assert_eq!(signed_size(1536), "+1.5 KB");
        assert_eq!(signed_size(0), "0 B");
    }
}

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
}

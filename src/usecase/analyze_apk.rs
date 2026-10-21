//! Build › Analyze APK…: the archive's size tree, and what the analyzer
//! shows for a selected file (the decoded manifest or a binary XML, a dex
//! file's classes, a text file).

use crate::domain::apk::{self, ApkArchive, ApkNode};
use crate::domain::{axml, dex};

pub fn sizes(archive: &dyn ApkArchive) -> ApkNode {
    apk::tree(&archive.entries())
}

/// The pane under the file tree for a selected file.
#[derive(Clone, Debug, PartialEq)]
pub enum Detail {
    /// Text with the editor language to show it in.
    Text { text: String, language: &'static str },
    Dex(dex::DexSummary),
    /// Nothing to show: why.
    None(String),
}

pub fn detail(archive: &dyn ApkArchive, path: &str) -> Detail {
    let bytes = match archive.read(path) {
        Ok(bytes) => bytes,
        Err(error) => return Detail::None(error),
    };
    if path.ends_with(".dex") {
        return dex::summarize(&bytes).map(Detail::Dex).unwrap_or_else(|| Detail::None("Not a dex file".into()));
    }
    if path.ends_with(".xml") {
        if let Some(text) = axml::decode(&bytes) {
            return Detail::Text { text, language: "xml" };
        }
    }
    let texty = [".xml", ".properties", ".txt", ".json", ".version", ".kotlin_builtins", ".pro", ".MF", ".SF"];
    if texty.iter().any(|e| path.ends_with(e)) && !bytes.contains(&0) {
        let language = if path.ends_with(".xml") { "xml" } else if path.ends_with(".json") { "json" } else if path.ends_with(".properties") { "properties" } else { "plaintext" };
        return Detail::Text { text: String::from_utf8_lossy(&bytes).into_owned(), language };
    }
    Detail::None(format!("No preview for {}", path.rsplit('/').next().unwrap_or(path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apk::ApkEntry;

    struct Fake;
    impl ApkArchive for Fake {
        fn entries(&self) -> Vec<ApkEntry> {
            vec![ApkEntry { path: "a.properties".into(), raw: 4, download: 4 }]
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            match path {
                "a.properties" => Ok(b"a=b\n".to_vec()),
                "classes.dex" => Ok(b"nope".to_vec()),
                _ => Err("missing".into()),
            }
        }
    }

    #[test]
    fn picks_the_detail() {
        assert_eq!(sizes(&Fake).raw, 4);
        assert_eq!(detail(&Fake, "a.properties"), Detail::Text { text: "a=b\n".into(), language: "properties" });
        assert_eq!(detail(&Fake, "classes.dex"), Detail::None("Not a dex file".into()));
        assert_eq!(detail(&Fake, "lib/x.so"), Detail::None("missing".into()));
    }
}

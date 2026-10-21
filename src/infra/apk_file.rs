//! An APK or AAB on disk, read with the zip crate. Stored (uncompressed)
//! files are deflated here to estimate their download size, as the
//! download is compressed even where the archive isn't.

use std::io::{Read as _, Write as _};
use std::path::PathBuf;

use crate::domain::apk::{ApkArchive, ApkEntry};

pub struct ApkFile {
    pub path: PathBuf,
}

fn deflated_size(bytes: &[u8]) -> u64 {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).ok();
    encoder.finish().map(|v| v.len() as u64).unwrap_or(bytes.len() as u64)
}

impl ApkArchive for ApkFile {
    fn entries(&self) -> Vec<ApkEntry> {
        let Ok(file) = std::fs::File::open(&self.path) else { return Vec::new() };
        let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(file)) else { return Vec::new() };
        let mut out = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else { continue };
            if entry.is_dir() {
                continue;
            }
            let (path, raw) = (entry.name().to_owned(), entry.size());
            let download = if entry.compression() == zip::CompressionMethod::Stored {
                let mut bytes = Vec::with_capacity(raw as usize);
                entry.read_to_end(&mut bytes).ok();
                deflated_size(&bytes)
            } else {
                entry.compressed_size()
            };
            out.push(ApkEntry { path, raw, download });
        }
        out
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let file = std::fs::File::open(&self.path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
        let mut entry = zip.by_name(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn probe_apk() {
    let Ok(path) = std::env::var("JUNCTION_APK") else { return };
    let apk = ApkFile { path: path.into() };
    let start = std::time::Instant::now();
    let entries = apk.entries();
    let root = crate::domain::apk::tree(&entries);
    eprintln!("{} entries, raw {} download {} in {:?}", entries.len(), root.raw, root.download, start.elapsed());
    let manifest = crate::domain::axml::decode(&apk.read("AndroidManifest.xml").unwrap()).unwrap();
    eprintln!("{}", &manifest[..manifest.len().min(1500)]);
    let start = std::time::Instant::now();
    let dex = crate::domain::dex::summarize(&apk.read("classes.dex").unwrap()).unwrap();
    eprintln!("{} ({} top nodes) in {:?}", dex.headline(), dex.tree.len(), start.elapsed());
}

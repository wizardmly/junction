//! Raw file contents for the binary and image diff viewers.

use super::Repository;
use super::diff::Revisions;

/// One side of a binary diff; `None` when the file doesn't exist there.
#[derive(Clone, Debug, Default)]
pub struct BinarySides {
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
}

/// Loads both versions as bytes, mirroring [`super::diff::load_versions`].
pub fn load(repository: &Repository, revisions: &Revisions) -> BinarySides {
    let blob = |spec: String| repository.run_bytes(["cat-file", "blob", &spec]).ok();
    let work_tree = |path: &str| std::fs::read(repository.root().join(path)).ok();
    let (old, new) = match revisions {
        Revisions::Commit { hash, path, old_path } => {
            let old_path = old_path.as_ref().unwrap_or(path);
            (blob(format!("{hash}^:{old_path}")), blob(format!("{hash}:{path}")))
        }
        Revisions::WorkingTree { path } => (blob(format!("HEAD:{path}")), work_tree(path)),
        Revisions::Staged { path } => (blob(format!("HEAD:{path}")), blob(format!(":{path}"))),
        Revisions::Unstaged { path } => (blob(format!(":{path}")), work_tree(path)),
        Revisions::Between { old, new, path, old_path } => (
            blob(format!("{old}:{}", old_path.as_ref().unwrap_or(path))),
            match new {
                Some(new) => blob(format!("{new}:{path}")),
                None => work_tree(path),
            },
        ),
    };
    BinarySides { old, new }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Webp,
    Ico,
}

impl ImageKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::Bmp => "BMP",
            Self::Webp => "WebP",
            Self::Ico => "ICO",
        }
    }
}

/// Format and pixel size read from the file header, as IntelliJ's image
/// viewer shows them ("64x64 PNG (32-bit color) 1.2 kB").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub kind: ImageKind,
    pub width: u32,
    pub height: u32,
}

pub fn image_info(bytes: &[u8]) -> Option<ImageInfo> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let le16 = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le32 = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let info = |kind, width, height| Some(ImageInfo { kind, width, height });
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return info(ImageKind::Png, be32(16)?, be32(20)?);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return info(ImageKind::Gif, le16(6)?, le16(8)?);
    }
    if bytes.starts_with(b"BM") {
        return info(ImageKind::Bmp, le32(18)?, (le32(22)? as i32).unsigned_abs());
    }
    if bytes.starts_with(b"\0\0\x01\0") {
        let size = |b: u8| if b == 0 { 256 } else { b as u32 };
        return info(ImageKind::Ico, size(*bytes.get(6)?), size(*bytes.get(7)?));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16)? {
            b"VP8 " => info(ImageKind::Webp, le16(26)? & 0x3fff, le16(28)? & 0x3fff),
            b"VP8L" => {
                let b = le32(21)?;
                info(ImageKind::Webp, (b & 0x3fff) + 1, ((b >> 14) & 0x3fff) + 1)
            }
            b"VP8X" => {
                let w = le32(24)? & 0xff_ffff;
                let h = le32(27)? & 0xff_ffff;
                info(ImageKind::Webp, w + 1, h + 1)
            }
            _ => None,
        };
    }
    if bytes.starts_with(b"\xff\xd8") {
        // Walk the JPEG segments to the start-of-frame marker.
        let mut at = 2;
        while at + 9 < bytes.len() {
            if bytes[at] != 0xff {
                return None;
            }
            let marker = bytes[at + 1];
            let length = be16(at + 2)? as usize;
            if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                return info(ImageKind::Jpeg, be16(at + 7)?, be16(at + 5)?);
            }
            at += 2 + length;
        }
    }
    None
}

/// "1.2 kB", the way IntelliJ formats file sizes.
pub fn format_size(bytes: usize) -> String {
    match bytes {
        0..1000 => format!("{bytes} B"),
        1000..1_000_000 => format!("{:.1} kB", bytes as f64 / 1000.),
        _ => format!("{:.1} MB", bytes as f64 / 1_000_000.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_image_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(64u32.to_be_bytes());
        png.extend(32u32.to_be_bytes());
        assert_eq!(image_info(&png), Some(ImageInfo { kind: ImageKind::Png, width: 64, height: 32 }));

        let gif = b"GIF89a\x10\0\x20\0";
        assert_eq!(image_info(gif), Some(ImageInfo { kind: ImageKind::Gif, width: 16, height: 32 }));

        // SOI, APP0 (length 4), SOF0 with height 0x0102 and width 0x0304.
        let jpeg = b"\xff\xd8\xff\xe0\0\x04xx\xff\xc0\0\x11\x08\x01\x02\x03\x04\x03";
        assert_eq!(image_info(jpeg), Some(ImageInfo { kind: ImageKind::Jpeg, width: 0x304, height: 0x102 }));

        assert_eq!(image_info(b"plain text"), None);
        assert_eq!(format_size(1234), "1.2 kB");
    }
}

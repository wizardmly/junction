//! Android's binary XML (a compiled AndroidManifest.xml or layout), back to
//! text, as the APK Analyzer shows it.

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

const NONE: u32 = u32::MAX;

struct Pool {
    strings: Vec<String>,
}

impl Pool {
    fn parse(b: &[u8], start: usize) -> Option<Pool> {
        let header = u16_at(b, start + 2)? as usize;
        let count = u32_at(b, start + 8)? as usize;
        let utf8 = u32_at(b, start + 16)? & 0x100 != 0;
        let strings_start = start + u32_at(b, start + 20)? as usize;
        let mut strings = Vec::with_capacity(count.min(1 << 20));
        for i in 0..count {
            let at = strings_start + u32_at(b, start + header + i * 4)? as usize;
            strings.push(if utf8 { utf8_string(b, at) } else { utf16_string(b, at) }.unwrap_or_default());
        }
        Some(Pool { strings })
    }

    fn get(&self, ix: u32) -> &str {
        if ix == NONE { "" } else { self.strings.get(ix as usize).map_or("", String::as_str) }
    }
}

fn utf8_string(b: &[u8], mut at: usize) -> Option<String> {
    // The UTF-16 length, then the UTF-8 length, each one or two bytes.
    for _ in 0..2 {
        let first = *b.get(at)?;
        at += if first & 0x80 != 0 { 2 } else { 1 };
    }
    let end = at + b.get(at..)?.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&b[at..end]).into_owned())
}

fn utf16_string(b: &[u8], at: usize) -> Option<String> {
    let first = u16_at(b, at)? as usize;
    let (len, at) = if first & 0x8000 != 0 { (((first & 0x7fff) << 16) | u16_at(b, at + 2)? as usize, at + 4) } else { (first, at + 2) };
    let units: Vec<u16> = (0..len).map(|i| u16_at(b, at + i * 2)).collect::<Option<_>>()?;
    Some(String::from_utf16_lossy(&units))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A typed attribute value as aapt2's `dump xmltree` and the analyzer
/// write it.
fn value(pool: &Pool, raw: u32, kind: u8, data: u32) -> String {
    match kind {
        0x03 => pool.get(if raw != NONE { raw } else { data }).to_owned(),
        0x01 => format!("@ref/0x{data:08x}"),
        0x02 => format!("?ref/0x{data:08x}"),
        0x04 => f32::from_bits(data).to_string(),
        0x05 | 0x06 => {
            let units = if kind == 0x05 { ["px", "dp", "sp", "pt", "in", "mm"].get((data & 0xf) as usize).copied().unwrap_or("") } else if data & 0xf == 0 { "%" } else { "%p" };
            let mantissa = (data & 0xffffff00) as i32 as f64;
            let radix = [1.0 / 256.0, 1.0 / 32768.0, 1.0 / 8388608.0, 1.0 / 2147483648.0][((data >> 4) & 3) as usize];
            let mut n = mantissa * radix;
            if kind == 0x06 {
                n *= 100.0;
            }
            format!("{}{units}", (n * 1000.0).round() / 1000.0)
        }
        0x10 => (data as i32).to_string(),
        0x11 => format!("0x{data:08x}"),
        0x12 => (data != 0).to_string(),
        0x1c..=0x1f => format!("#{data:08x}"),
        _ => format!("0x{data:08x}"),
    }
}

/// The document as indented XML text; `None` when it isn't binary XML.
pub fn decode(b: &[u8]) -> Option<String> {
    if u16_at(b, 0)? != 0x0003 {
        return None;
    }
    let mut pool = None;
    let mut namespaces: Vec<(String, String)> = Vec::new();
    let mut pending_ns: Vec<(String, String)> = Vec::new();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    let mut depth = 0usize;
    // Whether the last start tag is still open (`<a …` without its `>`).
    let mut open = false;
    let mut at = u16_at(b, 2)? as usize;
    while at + 8 <= b.len() {
        let kind = u16_at(b, at)?;
        let size = u32_at(b, at + 4)? as usize;
        if size < 8 {
            break;
        }
        match kind {
            0x0001 => pool = Pool::parse(b, at),
            0x0100 => {
                let p = pool.as_ref()?;
                let entry = (p.get(u32_at(b, at + 16)?).to_owned(), p.get(u32_at(b, at + 20)?).to_owned());
                namespaces.push(entry.clone());
                pending_ns.push(entry);
            }
            0x0101 => {
                namespaces.pop();
            }
            0x0102 => {
                let p = pool.as_ref()?;
                let name = p.get(u32_at(b, at + 20)?);
                let attr_start = u16_at(b, at + 24)? as usize;
                let attr_size = u16_at(b, at + 26)? as usize;
                let count = u16_at(b, at + 28)? as usize;
                if open {
                    out.push_str(">\n");
                }
                let indent = "    ".repeat(depth);
                out.push_str(&format!("{indent}<{name}"));
                for (prefix, uri) in pending_ns.drain(..) {
                    out.push_str(&format!("\n{indent}    xmlns:{prefix}=\"{}\"", escape(&uri)));
                }
                for i in 0..count {
                    let a = at + 16 + attr_start + i * attr_size.max(20);
                    let ns = u32_at(b, a)?;
                    let attr = p.get(u32_at(b, a + 4)?);
                    let raw = u32_at(b, a + 8)?;
                    let kind = *b.get(a + 15)?;
                    let data = u32_at(b, a + 16)?;
                    let prefix = if ns == NONE {
                        String::new()
                    } else {
                        let uri = p.get(ns);
                        namespaces.iter().rev().find(|(_, u)| u == uri).map(|(prefix, _)| format!("{prefix}:")).unwrap_or_default()
                    };
                    out.push_str(&format!("\n{indent}    {prefix}{attr}=\"{}\"", escape(&value(p, raw, kind, data))));
                }
                open = true;
                depth += 1;
            }
            0x0103 => {
                let p = pool.as_ref()?;
                depth = depth.saturating_sub(1);
                if open {
                    out.push_str(" />\n");
                } else {
                    out.push_str(&format!("{}</{}>\n", "    ".repeat(depth), p.get(u32_at(b, at + 20)?)));
                }
                open = false;
            }
            0x0104 => {
                let p = pool.as_ref()?;
                if open {
                    out.push_str(">\n");
                    open = false;
                }
                out.push_str(&format!("{}{}\n", "    ".repeat(depth), escape(p.get(u32_at(b, at + 16)?))));
            }
            _ => {}
        }
        at += size;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny binary XML: `<manifest xmlns:android=… package="p" android:versionCode="7"/>`.
    pub fn sample() -> Vec<u8> {
        let strings = ["android", "http://schemas.android.com/apk/res/android", "manifest", "package", "p", "versionCode"];
        let mut pool = Vec::new();
        let mut offsets = Vec::new();
        for s in strings {
            offsets.extend((pool.len() as u32).to_le_bytes());
            pool.extend([s.len() as u8, s.len() as u8]);
            pool.extend(s.as_bytes());
            pool.push(0);
        }
        while pool.len() % 4 != 0 {
            pool.push(0);
        }
        let header = 28u16;
        let mut sp = Vec::new();
        sp.extend(1u16.to_le_bytes());
        sp.extend(header.to_le_bytes());
        sp.extend(((header as usize + offsets.len() + pool.len()) as u32).to_le_bytes());
        sp.extend((strings.len() as u32).to_le_bytes());
        sp.extend(0u32.to_le_bytes());
        sp.extend(0x100u32.to_le_bytes());
        sp.extend(((header as usize + offsets.len()) as u32).to_le_bytes());
        sp.extend(0u32.to_le_bytes());
        sp.extend(&offsets);
        sp.extend(&pool);
        let node = |kind: u16, body: &[u8]| {
            let mut c = Vec::new();
            c.extend(kind.to_le_bytes());
            c.extend(16u16.to_le_bytes());
            c.extend(((16 + body.len()) as u32).to_le_bytes());
            c.extend(1u32.to_le_bytes());
            c.extend(NONE.to_le_bytes());
            c.extend(body);
            c
        };
        let ns: Vec<u8> = [0u32, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut start = Vec::new();
        for v in [NONE, 2] {
            start.extend(v.to_le_bytes());
        }
        for v in [20u16, 20, 2, 0, 0, 0] {
            start.extend(v.to_le_bytes());
        }
        let attr = |ns: u32, name: u32, raw: u32, kind: u8, data: u32| {
            let mut a = Vec::new();
            for v in [ns, name, raw] {
                a.extend(v.to_le_bytes());
            }
            a.extend(8u16.to_le_bytes());
            a.extend([0, kind]);
            a.extend(data.to_le_bytes());
            a
        };
        start.extend(attr(NONE, 3, 4, 0x03, 4));
        start.extend(attr(1, 5, NONE, 0x10, 7));
        let end: Vec<u8> = [NONE, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut body = sp;
        body.extend(node(0x0100, &ns));
        body.extend(node(0x0102, &start));
        body.extend(node(0x0103, &end));
        body.extend(node(0x0101, &ns));
        let mut doc = Vec::new();
        doc.extend(3u16.to_le_bytes());
        doc.extend(8u16.to_le_bytes());
        doc.extend(((8 + body.len()) as u32).to_le_bytes());
        doc.extend(body);
        doc
    }

    #[test]
    fn decodes_binary_xml() {
        let text = decode(&sample()).unwrap();
        assert_eq!(
            text,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest\n    xmlns:android=\"http://schemas.android.com/apk/res/android\"\n    package=\"p\"\n    android:versionCode=\"7\" />\n"
        );
        assert_eq!(decode(b"<manifest/>"), None);
    }
}

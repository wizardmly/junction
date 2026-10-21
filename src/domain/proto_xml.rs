//! An app bundle's XML (base/manifest/AndroidManifest.xml and the res
//! files), which aapt2 writes as an `aapt.pb.XmlNode` protocol buffer
//! instead of binary XML, back to text as the APK Analyzer shows it.

/// One field of a message: its number and either a varint or bytes.
enum Field<'a> {
    Varint(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// The fields of a message in order; `None` when it isn't well formed.
fn fields(b: &[u8]) -> Option<Vec<(u32, Field<'_>)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let key = varint(b, &mut at)?;
        let number = (key >> 3) as u32;
        if number == 0 {
            return None;
        }
        let field = match key & 7 {
            0 => Field::Varint(varint(b, &mut at)?),
            1 => {
                at = at.checked_add(8).filter(|&e| e <= b.len())?;
                continue;
            }
            2 => {
                let len = varint(b, &mut at)? as usize;
                let end = at.checked_add(len).filter(|&e| e <= b.len())?;
                let bytes = &b[at..end];
                at = end;
                Field::Bytes(bytes)
            }
            5 => {
                let bytes = b.get(at..at + 4)?;
                at += 4;
                Field::Fixed32(u32::from_le_bytes(bytes.try_into().ok()?))
            }
            _ => return None,
        };
        out.push((number, field));
    }
    Some(out)
}

fn text(field: &Field) -> String {
    match field {
        Field::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    }
}

fn string_of(b: &[u8], number: u32) -> String {
    fields(b).and_then(|f| f.iter().find(|(n, _)| *n == number).map(|(_, v)| text(v))).unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A reference's resource name without the app's own package
/// (`com.example:string/x` → `string/x`); framework names keep `android:`.
fn short_name(name: &str) -> &str {
    match name.split_once(':') {
        Some((package, rest)) if package != "android" => rest,
        _ => name,
    }
}

/// An `aapt.pb.Item` written as the analyzer writes compiled values.
fn item(b: &[u8]) -> Option<String> {
    let (number, value) = fields(b)?.into_iter().next()?;
    let Field::Bytes(inner) = value else { return None };
    Some(match number {
        // Reference { type, id, name }.
        1 => {
            let f = fields(inner)?;
            let attribute = f.iter().any(|(n, v)| *n == 1 && matches!(v, Field::Varint(1)));
            let id = f.iter().find_map(|(n, v)| match (n, v) {
                (2, Field::Varint(id)) => Some(*id as u32),
                _ => None,
            });
            let name = f.iter().find(|(n, _)| *n == 3).map(|(_, v)| text(v)).unwrap_or_default();
            let sigil = if attribute { '?' } else { '@' };
            if !name.is_empty() {
                format!("{sigil}{}", short_name(&name))
            } else {
                format!("{sigil}ref/0x{:08x}", id.unwrap_or(0))
            }
        }
        // String, RawString, StyledString and FileReference keep their text first.
        2..=5 => string_of(inner, 1),
        6 => String::new(),
        7 => primitive(inner)?,
        _ => return None,
    })
}

fn primitive(b: &[u8]) -> Option<String> {
    let (number, value) = fields(b)?.into_iter().next()?;
    let raw = match value {
        Field::Varint(v) => v,
        Field::Fixed32(v) => v as u64,
        Field::Bytes(_) => 0,
    };
    let data = raw as u32;
    Some(match number {
        1 => "@null".into(),
        2 => "@empty".into(),
        3 | 4 | 5 => f32::from_bits(data).to_string(),
        6 => (data as i32).to_string(),
        7 => format!("0x{data:08x}"),
        8 => (raw != 0).to_string(),
        9..=12 => format!("#{data:08x}"),
        13 | 14 => {
            let units = if number == 13 { ["px", "dp", "sp", "pt", "in", "mm"].get((data & 0xf) as usize).copied().unwrap_or("") } else if data & 0xf == 0 { "%" } else { "%p" };
            let mantissa = (data & 0xffffff00) as i32 as f64;
            let radix = [1.0 / 256.0, 1.0 / 32768.0, 1.0 / 8388608.0, 1.0 / 2147483648.0][((data >> 4) & 3) as usize];
            let mut n = mantissa * radix;
            if number == 14 {
                n *= 100.0;
            }
            format!("{}{units}", (n * 1000.0).round() / 1000.0)
        }
        _ => return None,
    })
}

fn element(b: &[u8], depth: usize, namespaces: &mut Vec<(String, String)>, out: &mut String) -> Option<()> {
    let f = fields(b)?;
    let name = f.iter().find(|(n, _)| *n == 3).map(|(_, v)| text(v)).filter(|n| !n.is_empty())?;
    let indent = "    ".repeat(depth);
    out.push_str(&format!("{indent}<{name}"));
    let declared = namespaces.len();
    for (_, v) in f.iter().filter(|(n, _)| *n == 1) {
        let Field::Bytes(ns) = v else { continue };
        let (prefix, uri) = (string_of(ns, 1), string_of(ns, 2));
        out.push_str(&format!("\n{indent}    xmlns:{prefix}=\"{}\"", escape(&uri)));
        namespaces.push((prefix, uri));
    }
    for (_, v) in f.iter().filter(|(n, _)| *n == 4) {
        let Field::Bytes(attr) = v else { continue };
        let a = fields(attr)?;
        let get = |number: u32| a.iter().find(|(n, _)| *n == number).map(|(_, v)| text(v)).unwrap_or_default();
        let uri = get(1);
        let prefix = if uri.is_empty() { String::new() } else { namespaces.iter().rev().find(|(_, u)| *u == uri).map(|(p, _)| format!("{p}:")).unwrap_or_default() };
        let compiled = a.iter().find_map(|(n, v)| match (n, v) {
            (6, Field::Bytes(item_bytes)) => item(item_bytes),
            _ => None,
        });
        // The compiled value, unless the raw value says more (plain text, or
        // a reference whose name the compiled one dropped).
        let raw = get(3);
        let value = match compiled {
            Some(c) if (c.starts_with("@ref/") || c.starts_with("?ref/")) && !raw.is_empty() => raw,
            Some(c) if !c.is_empty() => c,
            _ => raw,
        };
        out.push_str(&format!("\n{indent}    {prefix}{}=\"{}\"", get(2), escape(&value)));
    }
    let children: Vec<&[u8]> = f
        .iter()
        .filter_map(|(n, v)| match (n, v) {
            (5, Field::Bytes(c)) => Some(*c),
            _ => None,
        })
        .collect();
    if children.is_empty() {
        out.push_str(" />\n");
    } else {
        out.push_str(">\n");
        for child in children {
            node(child, depth + 1, namespaces, out)?;
        }
        out.push_str(&format!("{indent}</{name}>\n"));
    }
    namespaces.truncate(declared);
    Some(())
}

fn node(b: &[u8], depth: usize, namespaces: &mut Vec<(String, String)>, out: &mut String) -> Option<()> {
    for (number, value) in fields(b)? {
        match (number, value) {
            (1, Field::Bytes(e)) => return element(e, depth, namespaces, out),
            (2, Field::Bytes(t)) => {
                let t = String::from_utf8_lossy(t);
                if !t.trim().is_empty() {
                    out.push_str(&format!("{}{}\n", "    ".repeat(depth), escape(t.trim())));
                }
                return Some(());
            }
            _ => {}
        }
    }
    Some(())
}

/// The document as indented XML text; `None` when it isn't an XmlNode.
pub fn decode(b: &[u8]) -> Option<String> {
    let root = fields(b)?.into_iter().find_map(|(n, v)| match (n, v) {
        (1, Field::Bytes(e)) => Some(e),
        _ => None,
    })?;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    element(root, 0, &mut Vec::new(), &mut out)?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(number: u32, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut key = ((number << 3) | 2) as u64;
        let mut len = body.len() as u64;
        for v in [&mut key, &mut len] {
            loop {
                let byte = (*v & 0x7f) as u8;
                *v >>= 7;
                if *v == 0 {
                    out.push(byte);
                    break;
                }
                out.push(byte | 0x80);
            }
        }
        out.extend(body);
        out
    }

    fn varint_field(number: u32, value: u8) -> Vec<u8> {
        vec![(number << 3) as u8, value]
    }

    const ANDROID: &str = "http://schemas.android.com/apk/res/android";

    fn attr(name: &str, value: &str, item: Option<Vec<u8>>) -> Vec<u8> {
        let mut a = [bytes(1, ANDROID.as_bytes()), bytes(2, name.as_bytes()), bytes(3, value.as_bytes())].concat();
        if let Some(item) = item {
            a.extend(bytes(6, &item));
        }
        bytes(4, &a)
    }

    #[test]
    fn decodes_a_bundle_manifest() {
        let reference = bytes(1, &[varint_field(2, 0x7f), bytes(3, b"com.example:string/app_name")].concat());
        let theme_attr = bytes(1, &[varint_field(1, 1), bytes(3, b"android:attr/colorPrimary")].concat());
        let int = bytes(7, &varint_field(6, 24));
        let boolean = bytes(7, &varint_field(8, 1));
        let uses_sdk = [bytes(3, b"uses-sdk"), attr("minSdkVersion", "", Some(int))].concat();
        let application = [bytes(3, b"application"), attr("label", "", Some(reference)), attr("allowBackup", "", Some(boolean)), attr("theme", "", Some(theme_attr))].concat();
        let manifest = [
            bytes(1, &[bytes(1, b"android"), bytes(2, ANDROID.as_bytes())].concat()),
            bytes(3, b"manifest"),
            bytes(4, &[bytes(2, b"package"), bytes(3, b"com.example")].concat()),
            bytes(5, &bytes(1, &uses_sdk)),
            bytes(5, &bytes(1, &application)),
        ]
        .concat();
        let text = decode(&bytes(1, &manifest)).unwrap();
        assert_eq!(
            text,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest\n    xmlns:android=\"http://schemas.android.com/apk/res/android\"\n    package=\"com.example\">\n    <uses-sdk\n        android:minSdkVersion=\"24\" />\n    <application\n        android:label=\"@string/app_name\"\n        android:allowBackup=\"true\"\n        android:theme=\"?android:attr/colorPrimary\" />\n</manifest>\n"
        );
    }

    #[test]
    fn rejects_other_bytes() {
        assert_eq!(decode(b"\x03\x00\x08\x00"), None);
        assert_eq!(decode(b"<manifest/>"), None);
        assert_eq!(decode(&[]), None);
    }
}

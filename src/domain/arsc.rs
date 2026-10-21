//! An APK's compiled resource table (resources.arsc): which name each
//! resource id has, so the analyzer can write `@string/app_name` where the
//! binary XML only holds `0x7f0f001c`.

use std::collections::HashMap;

use super::axml::{u16_at, u32_at, Pool};

#[derive(Debug, Default)]
pub struct ResourceTable {
    names: HashMap<u32, String>,
}

impl ResourceTable {
    /// `None` when `b` isn't a resource table.
    pub fn parse(b: &[u8]) -> Option<ResourceTable> {
        if u16_at(b, 0)? != 0x0002 {
            return None;
        }
        let mut names = HashMap::new();
        let mut at = u16_at(b, 2)? as usize;
        while at + 8 <= b.len() {
            let size = u32_at(b, at + 4)? as usize;
            if size < 8 {
                break;
            }
            if u16_at(b, at)? == 0x0200 {
                package(b, at, size, &mut names);
            }
            at += size;
        }
        Some(ResourceTable { names })
    }

    /// `type/name`, with the package in front for ids outside the app's
    /// own 0x7f package (`com.lib:string/x`).
    pub fn name(&self, id: u32) -> Option<String> {
        self.names.get(&id).cloned()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.names.len()
    }
}

fn package(b: &[u8], start: usize, size: usize, names: &mut HashMap<u32, String>) -> Option<()> {
    let end = (start + size).min(b.len());
    let id = u32_at(b, start + 8)?;
    let units: Vec<u16> = (0..128).map_while(|i| u16_at(b, start + 12 + i * 2).filter(|&c| c != 0)).collect();
    let prefix = if id == 0x7f || units.is_empty() { String::new() } else { format!("{}:", String::from_utf16_lossy(&units)) };
    let types = Pool::parse(b, start + u32_at(b, start + 268)? as usize)?;
    let keys = Pool::parse(b, start + u32_at(b, start + 276)? as usize)?;
    let mut at = start + u16_at(b, start + 2)? as usize;
    while at + 8 <= end {
        let chunk_size = u32_at(b, at + 4)? as usize;
        if chunk_size < 8 {
            break;
        }
        if u16_at(b, at)? == 0x0201 {
            let type_id = *b.get(at + 8)? as u32;
            entries(b, at, |index, key| {
                let rid = (id << 24) | (type_id << 16) | index;
                names.entry(rid).or_insert_with(|| format!("{prefix}{}/{}", types.get(type_id.wrapping_sub(1)), keys.get(key)));
            });
        }
        at += chunk_size;
    }
    Some(())
}

/// Calls `found(entry index, key string index)` for each entry of one
/// configuration's type chunk.
fn entries(b: &[u8], chunk: usize, mut found: impl FnMut(u32, u32)) -> Option<()> {
    let header = u16_at(b, chunk + 2)? as usize;
    let flags = *b.get(chunk + 9)?;
    let count = u32_at(b, chunk + 12)? as usize;
    let entries_start = chunk + u32_at(b, chunk + 16)? as usize;
    let offsets = chunk + header;
    for i in 0..count {
        let (index, offset) = if flags & 0x01 != 0 {
            // Sparse: (entry index, offset / 4) pairs.
            (u16_at(b, offsets + i * 4)? as u32, u16_at(b, offsets + i * 4 + 2)? as usize * 4)
        } else if flags & 0x02 != 0 {
            match u16_at(b, offsets + i * 2)? {
                0xffff => continue,
                offset => (i as u32, offset as usize * 4),
            }
        } else {
            match u32_at(b, offsets + i * 4)? {
                u32::MAX => continue,
                offset => (i as u32, offset as usize),
            }
        };
        let entry = entries_start + offset;
        // A compact entry keeps its key in the first two bytes.
        let key = if u16_at(b, entry + 2)? & 0x0008 != 0 { u16_at(b, entry)? as u32 } else { u32_at(b, entry + 4)? };
        found(index, key);
    }
    Some(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::axml::tests::pool;

    fn chunk(kind: u16, header: &[u8], body: &[u8]) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend(kind.to_le_bytes());
        c.extend(((8 + header.len()) as u16).to_le_bytes());
        c.extend(((8 + header.len() + body.len()) as u32).to_le_bytes());
        c.extend(header);
        c.extend(body);
        c
    }

    /// One package `p` (0x7f) with `string/app_name` and `string/title`
    /// (the second entry missing in a dense chunk, then a sparse one), and
    /// one library package `lib` (0x02) with a compact `color/accent`.
    pub fn sample() -> Vec<u8> {
        let type_chunk = |id: u8, flags: u8, offsets: &[u8], entries: &[u8]| {
            let mut h = vec![id, flags, 0, 0];
            let count = if flags == 0x02 { offsets.len() / 2 } else { offsets.len() / 4 };
            h.extend((count as u32).to_le_bytes());
            h.extend(((8 + 12 + 4 + offsets.len()) as u32).to_le_bytes());
            h.extend(4u32.to_le_bytes()); // config size
            let mut body = offsets.to_vec();
            body.extend(entries);
            chunk(0x0201, &h, &body)
        };
        let full = |key: u32| {
            let mut e = Vec::new();
            e.extend(8u16.to_le_bytes());
            e.extend(0u16.to_le_bytes());
            e.extend(key.to_le_bytes());
            e
        };
        let package = |id: u32, name: &str, types: &[&str], keys: &[&str], chunks: Vec<Vec<u8>>| {
            let mut h = Vec::new();
            h.extend(id.to_le_bytes());
            let mut n: Vec<u16> = name.encode_utf16().collect();
            n.resize(128, 0);
            h.extend(n.iter().flat_map(|c| c.to_le_bytes()));
            let type_pool = pool(types);
            let header_len = 8 + h.len() + 20;
            h.extend((header_len as u32).to_le_bytes());
            h.extend(0u32.to_le_bytes());
            h.extend(((header_len + type_pool.len()) as u32).to_le_bytes());
            h.extend(0u32.to_le_bytes());
            h.extend(0u32.to_le_bytes());
            let mut body = type_pool;
            body.extend(pool(keys));
            for c in chunks {
                body.extend(c);
            }
            chunk(0x0200, &h, &body)
        };
        let dense = {
            let mut offsets = Vec::new();
            offsets.extend(0u32.to_le_bytes());
            offsets.extend(u32::MAX.to_le_bytes());
            type_chunk(1, 0, &offsets, &full(0))
        };
        let sparse = {
            let mut offsets = Vec::new();
            offsets.extend(1u16.to_le_bytes());
            offsets.extend(0u16.to_le_bytes());
            type_chunk(1, 0x01, &offsets, &full(1))
        };
        let compact = {
            let mut e = Vec::new();
            e.extend(0u16.to_le_bytes());
            e.extend(0x0008u16.to_le_bytes());
            e.extend(0xff00ff00u32.to_le_bytes());
            let mut offsets = Vec::new();
            offsets.extend(0u16.to_le_bytes());
            type_chunk(1, 0x02, &[offsets, vec![0xff, 0xff]].concat(), &e)
        };
        let mut body = pool(&[]);
        body.extend(package(0x7f, "p", &["string"], &["app_name", "title"], vec![dense, sparse]));
        body.extend(package(0x02, "lib", &["color"], &["accent"], vec![compact]));
        chunk(0x0002, &2u32.to_le_bytes(), &body)
    }

    #[test]
    fn names_resource_ids() {
        let table = ResourceTable::parse(&sample()).unwrap();
        assert_eq!(table.name(0x7f01_0000).as_deref(), Some("string/app_name"));
        assert_eq!(table.name(0x7f01_0001).as_deref(), Some("string/title"));
        assert_eq!(table.name(0x0201_0000).as_deref(), Some("lib:color/accent"));
        assert_eq!(table.name(0x0201_0001), None);
        assert_eq!(table.len(), 3);
        assert!(ResourceTable::parse(b"PK\x03\x04").is_none());
    }
}

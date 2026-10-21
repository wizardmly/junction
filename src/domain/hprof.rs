//! A heap dump (`.hprof`, Android's flavour or the standard one) read into
//! Android Studio's class list: each class's instance count (Allocations)
//! and Shallow Size, per heap (app, image, zygote).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassStat {
    pub name: String,
    pub heap: String,
    pub count: u64,
    pub shallow: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeapSummary {
    /// Largest shallow size first.
    pub classes: Vec<ClassStat>,
    pub heaps: Vec<String>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    id_size: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len())?;
        let out = &self.data[self.at..end];
        self.at = end;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn id(&mut self) -> Option<u64> {
        let b = self.take(self.id_size)?;
        Some(b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64))
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
}

/// The size of a value of the basic type.
fn type_size(ty: u8, id_size: usize) -> Option<usize> {
    Some(match ty {
        2 => id_size,
        4 | 8 => 1,
        5 | 9 => 2,
        6 | 10 => 4,
        7 | 11 => 8,
        _ => return None,
    })
}

fn primitive_array_name(ty: u8) -> &'static str {
    match ty {
        4 => "boolean[]",
        5 => "char[]",
        6 => "float[]",
        7 => "double[]",
        8 => "byte[]",
        9 => "short[]",
        10 => "int[]",
        11 => "long[]",
        _ => "?[]",
    }
}

/// `Ljava/lang/String;` and `java/lang/String` → `java.lang.String`.
fn class_name(raw: &str) -> String {
    let dims = raw.chars().take_while(|&c| c == '[').count();
    let base = &raw[dims..];
    let base = base.strip_prefix('L').and_then(|b| b.strip_suffix(';')).unwrap_or(base);
    format!("{}{}", base.replace('/', "."), "[]".repeat(dims))
}

/// Reads the dump; `None` when it isn't one.
pub fn summarize(data: &[u8]) -> Option<HeapSummary> {
    let header_end = data.iter().position(|&b| b == 0)?;
    if !data[..header_end].starts_with(b"JAVA PROFILE") {
        return None;
    }
    let mut r = Reader { data, at: header_end + 1, id_size: 4 };
    r.id_size = r.u32()? as usize;
    if !matches!(r.id_size, 4 | 8) {
        return None;
    }
    r.skip(8)?;
    let mut strings: HashMap<u64, String> = HashMap::new();
    let mut class_names: HashMap<u64, u64> = HashMap::new();
    let mut instance_sizes: HashMap<u64, u32> = HashMap::new();
    // (class or array name, heap) → (count, shallow).
    let mut stats: HashMap<(Key, usize), (u64, u64)> = HashMap::new();
    let mut heaps: Vec<String> = vec!["default".into()];
    let mut heap_names: HashMap<u32, usize> = HashMap::new();
    let mut heap = 0usize;
    while r.at < data.len() {
        let tag = r.u8()?;
        r.skip(4)?;
        let length = r.u32()? as usize;
        let end = r.at.checked_add(length)?.min(data.len());
        match tag {
            0x01 => {
                let id = r.id()?;
                let text = r.take(end.checked_sub(r.at)?)?;
                strings.insert(id, String::from_utf8_lossy(text).into_owned());
            }
            0x02 => {
                r.skip(4)?;
                let class = r.id()?;
                r.skip(4)?;
                let name = r.id()?;
                class_names.insert(class, name);
            }
            0x0C | 0x1C => {
                while r.at < end {
                    let sub = r.u8()?;
                    let id = r.id_size;
                    match sub {
                        0xFF | 0x05 | 0x07 | 0x89 | 0x8A | 0x8B | 0x8D | 0x90 => r.skip(id)?,
                        0x01 => r.skip(id * 2)?,
                        0x02 | 0x03 | 0x08 | 0x8E => r.skip(id + 8)?,
                        0x04 | 0x06 | 0x8C => r.skip(id + 4)?,
                        0xFE => {
                            let heap_id = r.u32()?;
                            let name = r.id()?;
                            let name = strings.get(&name).cloned().unwrap_or_else(|| format!("heap {heap_id}"));
                            heap = *heap_names.entry(heap_id).or_insert_with(|| {
                                heaps.push(name);
                                heaps.len() - 1
                            });
                        }
                        0x20 => {
                            let class = r.id()?;
                            r.skip(4 + id * 6)?;
                            let size = r.u32()?;
                            instance_sizes.insert(class, size);
                            for _ in 0..r.u16()? {
                                r.skip(2)?;
                                let ty = r.u8()?;
                                r.skip(type_size(ty, id)?)?;
                            }
                            for _ in 0..r.u16()? {
                                r.skip(id)?;
                                let ty = r.u8()?;
                                r.skip(type_size(ty, id)?)?;
                            }
                            let fields = r.u16()? as usize;
                            r.skip(fields * (id + 1))?;
                        }
                        0x21 => {
                            r.skip(id + 4)?;
                            let class = r.id()?;
                            let len = r.u32()? as usize;
                            r.skip(len)?;
                            let entry = stats.entry((Key::Class(class), heap)).or_default();
                            entry.0 += 1;
                            entry.1 += len as u64;
                        }
                        0x22 => {
                            r.skip(id + 4)?;
                            let count = r.u32()? as usize;
                            let class = r.id()?;
                            r.skip(count * id)?;
                            let entry = stats.entry((Key::Class(class), heap)).or_default();
                            entry.0 += 1;
                            entry.1 += (count * id) as u64;
                        }
                        0x23 | 0xC3 => {
                            r.skip(id + 4)?;
                            let count = r.u32()? as usize;
                            let ty = r.u8()?;
                            let bytes = count * type_size(ty, id)?;
                            if sub == 0x23 {
                                r.skip(bytes)?;
                            }
                            let entry = stats.entry((Key::Primitive(ty), heap)).or_default();
                            entry.0 += 1;
                            entry.1 += bytes as u64;
                        }
                        _ => return None,
                    }
                }
            }
            _ => {}
        }
        r.at = end;
    }
    let mut classes: Vec<ClassStat> = stats
        .into_iter()
        .map(|((key, heap), (count, data_bytes))| {
            let (name, shallow) = match key {
                Key::Class(class) => {
                    let name = class_names.get(&class).and_then(|n| strings.get(n)).map(|n| class_name(n)).unwrap_or_else(|| format!("0x{class:x}"));
                    // An instance's shallow size is its class's; an array's its elements'.
                    let shallow = match instance_sizes.get(&class) {
                        Some(&size) if !name.ends_with("[]") => size as u64 * count,
                        _ => data_bytes,
                    };
                    (name, shallow)
                }
                Key::Primitive(ty) => (primitive_array_name(ty).to_owned(), data_bytes),
            };
            ClassStat { name, heap: heaps[heap].clone(), count, shallow }
        })
        .collect();
    classes.sort_by(|a, b| b.shallow.cmp(&a.shallow).then_with(|| a.name.cmp(&b.name)));
    let mut used: Vec<String> = classes.iter().map(|c| c.heap.clone()).collect();
    used.sort();
    used.dedup();
    Some(HeapSummary { classes, heaps: used })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Class(u64),
    Primitive(u8),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small Android-style dump: an `app` heap with two strings, a byte
    /// array and an object array.
    pub(crate) fn sample() -> Vec<u8> {
        let mut out = b"JAVA PROFILE 1.0.3\0".to_vec();
        out.extend(4u32.to_be_bytes());
        out.extend(0u64.to_be_bytes());
        let record = |out: &mut Vec<u8>, tag: u8, body: &[u8]| {
            out.push(tag);
            out.extend(0u32.to_be_bytes());
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(body);
        };
        let id = |v: u32| v.to_be_bytes().to_vec();
        let string = |sid: u32, text: &str| [id(sid), text.as_bytes().to_vec()].concat();
        record(&mut out, 0x01, &string(1, "java.lang.String"));
        record(&mut out, 0x01, &string(2, "app"));
        record(&mut out, 0x01, &string(3, "java.lang.Object[]"));
        record(&mut out, 0x02, &[1u32.to_be_bytes().to_vec(), id(100), 0u32.to_be_bytes().to_vec(), id(1)].concat());
        record(&mut out, 0x02, &[2u32.to_be_bytes().to_vec(), id(200), 0u32.to_be_bytes().to_vec(), id(3)].concat());
        let mut heap = Vec::new();
        heap.push(0xFE);
        heap.extend(1u32.to_be_bytes());
        heap.extend(id(2));
        // CLASS_DUMP of String: instance size 16, no constants, statics or fields.
        heap.push(0x20);
        heap.extend(id(100));
        heap.extend(0u32.to_be_bytes());
        for _ in 0..6 {
            heap.extend(id(0));
        }
        heap.extend(16u32.to_be_bytes());
        heap.extend([0, 0, 0, 0, 0, 0]);
        for obj in [10, 11] {
            heap.push(0x21);
            heap.extend(id(obj));
            heap.extend(0u32.to_be_bytes());
            heap.extend(id(100));
            heap.extend(8u32.to_be_bytes());
            heap.extend([0u8; 8]);
        }
        heap.push(0x23);
        heap.extend(id(12));
        heap.extend(0u32.to_be_bytes());
        heap.extend(5u32.to_be_bytes());
        heap.push(8);
        heap.extend([1u8; 5]);
        heap.push(0x22);
        heap.extend(id(13));
        heap.extend(0u32.to_be_bytes());
        heap.extend(2u32.to_be_bytes());
        heap.extend(id(200));
        heap.extend(id(10));
        heap.extend(id(11));
        heap.push(0x8D);
        heap.extend(id(10));
        record(&mut out, 0x1C, &heap);
        record(&mut out, 0x2C, &[]);
        out
    }

    #[test]
    fn classes_counted_per_heap() {
        let s = summarize(&sample()).unwrap();
        let rows: Vec<(&str, &str, u64, u64)> = s.classes.iter().map(|c| (c.name.as_str(), c.heap.as_str(), c.count, c.shallow)).collect();
        assert_eq!(rows, [("java.lang.String", "app", 2, 32), ("java.lang.Object[]", "app", 1, 8), ("byte[]", "app", 1, 5)]);
        assert_eq!(s.heaps, ["app"]);
    }

    #[test]
    fn not_a_dump() {
        assert!(summarize(b"PK\x03\x04").is_none());
        assert_eq!(class_name("[Ljava/lang/String;"), "java.lang.String[]");
    }
}

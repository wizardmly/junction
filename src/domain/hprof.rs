//! A heap dump (`.hprof`, Android's flavour or the standard one) read into
//! Android Studio's class list: each class's instance count (Allocations),
//! Shallow Size and Retained Size, per heap (app, image, zygote).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassStat {
    pub name: String,
    pub heap: String,
    pub count: u64,
    pub shallow: u64,
    /// The sum of its reachable instances' retained sizes.
    pub retained: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeapSummary {
    /// Largest retained size first, as Android Studio sorts them.
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

/// What an object points at, read once the classes' field layouts are known.
enum Refs {
    None,
    /// An instance's field values at `off..off + len`.
    Instance { class: u64, off: usize, len: usize },
    /// An object array's `count` ids at `off`.
    Array { off: usize, count: usize },
    /// A class object's static fields holding objects.
    Statics(Vec<u64>),
}

struct Obj {
    id: u64,
    /// Its row in the class list (`u32::MAX` for class objects).
    stat: u32,
    shallow: u64,
    refs: Refs,
}

/// A class's own instance fields (types in dump order) and its superclass.
struct Layout {
    super_id: u64,
    fields: Vec<u8>,
    /// The index of `java.lang.ref.Reference.referent`, which a weak, soft
    /// or phantom reference doesn't retain.
    referent: Option<usize>,
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
    let mut layouts: HashMap<u64, Layout> = HashMap::new();
    // (class or array name, heap) → its row: (count, data bytes).
    let mut stat_ix: HashMap<(Key, usize), u32> = HashMap::new();
    let mut stats: Vec<(Key, usize, u64, u64)> = Vec::new();
    let mut objects: Vec<Obj> = Vec::new();
    let mut roots: Vec<u64> = Vec::new();
    let mut heaps: Vec<String> = vec!["default".into()];
    let mut heap_names: HashMap<u32, usize> = HashMap::new();
    let mut heap = 0usize;
    let mut row = |key: Key, heap: usize, bytes: u64| -> u32 {
        let ix = *stat_ix.entry((key, heap)).or_insert_with(|| {
            stats.push((key, heap, 0, 0));
            stats.len() as u32 - 1
        });
        let entry = &mut stats[ix as usize];
        entry.2 += 1;
        entry.3 += bytes;
        ix
    };
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
                        // Unreachable "roots" don't keep anything alive.
                        0x90 => r.skip(id)?,
                        0xFF | 0x05 | 0x07 | 0x89 | 0x8A | 0x8B | 0x8D => roots.push(r.id()?),
                        0x01 => {
                            roots.push(r.id()?);
                            r.skip(id)?;
                        }
                        0x02 | 0x03 | 0x08 | 0x8E => {
                            roots.push(r.id()?);
                            r.skip(8)?;
                        }
                        0x04 | 0x06 | 0x8C => {
                            roots.push(r.id()?);
                            r.skip(4)?;
                        }
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
                            r.skip(4)?;
                            let super_id = r.id()?;
                            r.skip(id * 5)?;
                            let size = r.u32()?;
                            instance_sizes.insert(class, size);
                            for _ in 0..r.u16()? {
                                r.skip(2)?;
                                let ty = r.u8()?;
                                r.skip(type_size(ty, id)?)?;
                            }
                            let mut statics = Vec::new();
                            for _ in 0..r.u16()? {
                                r.skip(id)?;
                                let ty = r.u8()?;
                                if ty == 2 {
                                    statics.push(r.id()?);
                                } else {
                                    r.skip(type_size(ty, id)?)?;
                                }
                            }
                            let mut fields = Vec::new();
                            let mut names = Vec::new();
                            for _ in 0..r.u16()? {
                                names.push(r.id()?);
                                fields.push(r.u8()?);
                            }
                            let is_reference = class_names.get(&class).and_then(|n| strings.get(n)).is_some_and(|n| matches!(n.as_str(), "java.lang.ref.Reference" | "java/lang/ref/Reference"));
                            let referent = if is_reference { names.iter().position(|n| strings.get(n).is_some_and(|n| n == "referent")) } else { None };
                            layouts.insert(class, Layout { super_id, fields, referent });
                            objects.push(Obj { id: class, stat: u32::MAX, shallow: 0, refs: Refs::Statics(statics) });
                        }
                        0x21 => {
                            let object = r.id()?;
                            r.skip(4)?;
                            let class = r.id()?;
                            let len = r.u32()? as usize;
                            let off = r.at;
                            r.skip(len)?;
                            let stat = row(Key::Class(class), heap, len as u64);
                            objects.push(Obj { id: object, stat, shallow: len as u64, refs: Refs::Instance { class, off, len } });
                        }
                        0x22 => {
                            let object = r.id()?;
                            r.skip(4)?;
                            let count = r.u32()? as usize;
                            let class = r.id()?;
                            let off = r.at;
                            r.skip(count * id)?;
                            let stat = row(Key::Class(class), heap, (count * id) as u64);
                            objects.push(Obj { id: object, stat, shallow: (count * id) as u64, refs: Refs::Array { off, count } });
                        }
                        0x23 | 0xC3 => {
                            let object = r.id()?;
                            r.skip(4)?;
                            let count = r.u32()? as usize;
                            let ty = r.u8()?;
                            let bytes = count * type_size(ty, id)?;
                            if sub == 0x23 {
                                r.skip(bytes)?;
                            }
                            let stat = row(Key::Primitive(ty), heap, bytes as u64);
                            objects.push(Obj { id: object, stat, shallow: bytes as u64, refs: Refs::None });
                        }
                        _ => return None,
                    }
                }
            }
            _ => {}
        }
        r.at = end;
    }
    let name_of = |class: u64| class_names.get(&class).and_then(|n| strings.get(n)).map(|n| class_name(n)).unwrap_or_else(|| format!("0x{class:x}"));
    // An instance's shallow size is its class's; an array's its elements'.
    for o in &mut objects {
        if let Refs::Instance { class, .. } = o.refs {
            if let Some(&size) = instance_sizes.get(&class) {
                o.shallow = size as u64;
            }
        }
    }
    let retained = retained_sizes(data, r.id_size, &objects, &roots, &layouts);
    let mut retained_rows = vec![0u64; stats.len()];
    for (o, size) in objects.iter().zip(&retained) {
        if o.stat != u32::MAX {
            retained_rows[o.stat as usize] += size;
        }
    }
    let mut classes: Vec<ClassStat> = stats
        .into_iter()
        .zip(retained_rows)
        .map(|((key, heap, count, data_bytes), retained)| {
            let (name, shallow) = match key {
                Key::Class(class) => {
                    let name = name_of(class);
                    let shallow = match instance_sizes.get(&class) {
                        Some(&size) if !name.ends_with("[]") => size as u64 * count,
                        _ => data_bytes,
                    };
                    (name, shallow)
                }
                Key::Primitive(ty) => (primitive_array_name(ty).to_owned(), data_bytes),
            };
            ClassStat { name, heap: heaps[heap].clone(), count, shallow, retained }
        })
        .collect();
    classes.sort_by(|a, b| b.retained.cmp(&a.retained).then_with(|| b.shallow.cmp(&a.shallow)).then_with(|| a.name.cmp(&b.name)));
    let mut used: Vec<String> = classes.iter().map(|c| c.heap.clone()).collect();
    used.sort();
    used.dedup();
    Some(HeapSummary { classes, heaps: used })
}

fn id_at(data: &[u8], at: usize, id_size: usize) -> u64 {
    data.get(at..at + id_size).map_or(0, |b| b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64))
}

/// Each object's retained size: its shallow size plus everything only it
/// keeps alive, from the dominator tree of the graph under the GC roots
/// (Cooper, Harvey and Kennedy's iterative algorithm). Objects no root
/// reaches retain nothing.
fn retained_sizes(data: &[u8], id_size: usize, objects: &[Obj], roots: &[u64], layouts: &HashMap<u64, Layout>) -> Vec<u64> {
    const NONE: u32 = u32::MAX;
    let index: HashMap<u64, u32> = objects.iter().enumerate().map(|(i, o)| (o.id, i as u32 + 1)).collect();
    let node = |id: u64| if id == 0 { None } else { index.get(&id).copied() };
    // Node 0 is a virtual root pointing at every GC root.
    let n = objects.len() + 1;
    let mut starts: Vec<u32> = Vec::with_capacity(n + 1);
    let mut edges: Vec<u32> = Vec::new();
    starts.push(0);
    edges.extend(roots.iter().filter_map(|&id| node(id)));
    starts.push(edges.len() as u32);
    for o in objects {
        match &o.refs {
            Refs::None => {}
            Refs::Statics(ids) => edges.extend(ids.iter().filter_map(|&id| node(id))),
            Refs::Array { off, count } => edges.extend((0..*count).filter_map(|i| node(id_at(data, off + i * id_size, id_size)))),
            Refs::Instance { class, off, len } => {
                let (mut at, end) = (*off, off + len);
                let mut class = *class;
                // The class's own fields come first, then each superclass's.
                while let Some(layout) = layouts.get(&class) {
                    for (i, &ty) in layout.fields.iter().enumerate() {
                        let Some(size) = type_size(ty, id_size) else { break };
                        if at + size > end {
                            break;
                        }
                        if ty == 2 && layout.referent != Some(i) {
                            if let Some(target) = node(id_at(data, at, id_size)) {
                                edges.push(target);
                            }
                        }
                        at += size;
                    }
                    if layout.super_id == 0 || layout.super_id == class {
                        break;
                    }
                    class = layout.super_id;
                }
            }
        }
        starts.push(edges.len() as u32);
    }
    // Depth-first postorder from the virtual root.
    let mut order: Vec<u32> = Vec::new();
    let mut post = vec![NONE; n];
    let mut seen = vec![false; n];
    let mut stack: Vec<(u32, u32)> = vec![(0, starts[0])];
    seen[0] = true;
    while let Some(&mut (v, ref mut next)) = stack.last_mut() {
        if *next < starts[v as usize + 1] {
            let w = edges[*next as usize];
            *next += 1;
            if !seen[w as usize] {
                seen[w as usize] = true;
                stack.push((w, starts[w as usize]));
            }
        } else {
            post[v as usize] = order.len() as u32;
            order.push(v);
            stack.pop();
        }
    }
    // Predecessors among reachable nodes.
    let mut pred_count = vec![0u32; n + 1];
    for &v in &order {
        for &w in &edges[starts[v as usize] as usize..starts[v as usize + 1] as usize] {
            pred_count[w as usize + 1] += 1;
        }
    }
    for i in 1..=n {
        pred_count[i] += pred_count[i - 1];
    }
    let mut preds = vec![0u32; pred_count[n] as usize];
    let mut fill = pred_count.clone();
    for &v in &order {
        for &w in &edges[starts[v as usize] as usize..starts[v as usize + 1] as usize] {
            preds[fill[w as usize] as usize] = v;
            fill[w as usize] += 1;
        }
    }
    let mut idom = vec![NONE; n];
    idom[0] = 0;
    let intersect = |idom: &[u32], mut a: u32, mut b: u32| {
        while a != b {
            while post[a as usize] < post[b as usize] {
                a = idom[a as usize];
            }
            while post[b as usize] < post[a as usize] {
                b = idom[b as usize];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &v in order.iter().rev().skip(1) {
            let mut new = NONE;
            for &p in &preds[pred_count[v as usize] as usize..pred_count[v as usize + 1] as usize] {
                if idom[p as usize] == NONE {
                    continue;
                }
                new = if new == NONE { p } else { intersect(&idom, p, new) };
            }
            if new != NONE && idom[v as usize] != new {
                idom[v as usize] = new;
                changed = true;
            }
        }
    }
    let mut retained = vec![0u64; n];
    for &v in &order {
        if v != 0 {
            retained[v as usize] += objects[v as usize - 1].shallow;
            let parent = idom[v as usize];
            if parent != NONE {
                retained[parent as usize] += retained[v as usize];
            }
        }
    }
    retained.remove(0);
    retained
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
        for root in [10, 13] {
            heap.push(0x8D);
            heap.extend(id(root));
        }
        record(&mut out, 0x1C, &heap);
        record(&mut out, 0x2C, &[]);
        out
    }

    #[test]
    fn classes_counted_per_heap() {
        let s = summarize(&sample()).unwrap();
        let rows: Vec<(&str, &str, u64, u64, u64)> = s.classes.iter().map(|c| (c.name.as_str(), c.heap.as_str(), c.count, c.shallow, c.retained)).collect();
        // The array alone keeps the second string; the first is also a
        // root of its own. Nothing reaches the byte array.
        assert_eq!(rows, [("java.lang.String", "app", 2, 32, 32), ("java.lang.Object[]", "app", 1, 8, 24), ("byte[]", "app", 1, 5, 0)]);
        assert_eq!(s.heaps, ["app"]);
    }

    /// `Holder extends java.lang.ref.Reference` with its own `next` field
    /// and the inherited `referent`; a rooted holder whose `next` is one
    /// leaf and whose referent is another.
    #[test]
    fn fields_follow_the_class_chain_and_skip_referents() {
        let mut out = b"JAVA PROFILE 1.0.2\0".to_vec();
        out.extend(4u32.to_be_bytes());
        out.extend(0u64.to_be_bytes());
        let record = |out: &mut Vec<u8>, tag: u8, body: &[u8]| {
            out.push(tag);
            out.extend(0u32.to_be_bytes());
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(body);
        };
        let id = |v: u32| v.to_be_bytes().to_vec();
        for (sid, text) in [(1, "java/lang/ref/Reference"), (2, "Holder"), (3, "Leaf"), (4, "referent"), (5, "next")] {
            record(&mut out, 0x01, &[id(sid), text.as_bytes().to_vec()].concat());
        }
        for (class, name) in [(100, 1), (101, 2), (102, 3)] {
            record(&mut out, 0x02, &[0u32.to_be_bytes().to_vec(), id(class), 0u32.to_be_bytes().to_vec(), id(name)].concat());
        }
        let mut heap = Vec::new();
        let class_dump = |heap: &mut Vec<u8>, class: u32, super_id: u32, size: u32, fields: &[(u32, u8)]| {
            heap.push(0x20);
            heap.extend(id(class));
            heap.extend(0u32.to_be_bytes());
            heap.extend(id(super_id));
            for _ in 0..5 {
                heap.extend(id(0));
            }
            heap.extend(size.to_be_bytes());
            heap.extend([0, 0, 0, 0]);
            heap.extend((fields.len() as u16).to_be_bytes());
            for &(name, ty) in fields {
                heap.extend(id(name));
                heap.push(ty);
            }
        };
        class_dump(&mut heap, 100, 0, 4, &[(4, 2)]);
        class_dump(&mut heap, 101, 100, 12, &[(5, 2), (5, 10)]);
        class_dump(&mut heap, 102, 0, 10, &[]);
        let instance = |heap: &mut Vec<u8>, object: u32, class: u32, values: &[u8]| {
            heap.push(0x21);
            heap.extend(id(object));
            heap.extend(0u32.to_be_bytes());
            heap.extend(id(class));
            heap.extend((values.len() as u32).to_be_bytes());
            heap.extend(values);
        };
        // next = 20, an int, then the inherited referent = 21.
        instance(&mut heap, 10, 101, &[id(20), 7u32.to_be_bytes().to_vec(), id(21)].concat());
        instance(&mut heap, 20, 102, &[]);
        instance(&mut heap, 21, 102, &[]);
        heap.push(0x05);
        heap.extend(id(10));
        record(&mut out, 0x0C, &heap);
        let s = summarize(&out).unwrap();
        let get = |name: &str| s.classes.iter().find(|c| c.name == name).map(|c| (c.count, c.shallow, c.retained)).unwrap();
        assert_eq!(get("Holder"), (1, 12, 22));
        assert_eq!(get("Leaf"), (2, 20, 10));
    }

    #[test]
    #[ignore]
    fn probe_hprof() {
        let Ok(path) = std::env::var("JUNCTION_HPROF") else { return };
        let data = std::fs::read(path).unwrap();
        let start = std::time::Instant::now();
        let s = summarize(&data).unwrap();
        eprintln!("{} classes in {:?}", s.classes.len(), start.elapsed());
        for c in s.classes.iter().take(12) {
            eprintln!("{:50} {:>8} {:>12} {:>12}", c.name, c.count, c.shallow, c.retained);
        }
    }

    #[test]
    fn not_a_dump() {
        assert!(summarize(b"PK\x03\x04").is_none());
        assert_eq!(class_name("[Ljava/lang/String;"), "java.lang.String[]");
    }
}

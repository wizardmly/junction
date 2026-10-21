//! A dex file's classes and method counts, as the APK Analyzer's dex view
//! shows them: packages, classes with their defined and referenced
//! methods, and the classes only referenced (from other dex files or the
//! platform).

use std::collections::BTreeMap;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn uleb(b: &[u8], at: &mut usize) -> Option<u32> {
    let mut out = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        out |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}

/// A package or class of the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DexNode {
    pub name: String,
    pub class: bool,
    /// A class only referenced here, defined elsewhere (shown greyed).
    pub defined_here: bool,
    pub defined_methods: u32,
    pub referenced_methods: u32,
    pub children: Vec<DexNode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DexSummary {
    pub classes: u32,
    pub defined_methods: u32,
    pub referenced_methods: u32,
    /// The top-level packages (and default-package classes).
    pub tree: Vec<DexNode>,
}

impl DexSummary {
    /// The analyzer's line above the tree.
    pub fn headline(&self) -> String {
        format!(
            "This dex file defines {} classes with {} methods, and references {} methods.",
            group(self.classes),
            group(self.defined_methods),
            group(self.referenced_methods)
        )
    }
}

/// 12345 as "12,345".
pub fn group(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn string(b: &[u8], string_ids: usize, ix: u32) -> Option<String> {
    let mut at = u32_at(b, string_ids + ix as usize * 4)? as usize;
    uleb(b, &mut at)?;
    let end = at + b.get(at..)?.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&b[at..end]).into_owned())
}

/// `Lcom/a/B;` as `com.a.B`; `None` for arrays and primitives.
fn class_name(descriptor: &str) -> Option<String> {
    let inner = descriptor.strip_prefix('L')?.strip_suffix(';')?;
    Some(inner.replace('/', "."))
}

#[derive(Default)]
struct Builder {
    packages: BTreeMap<String, Builder>,
    classes: BTreeMap<String, (bool, u32, u32)>,
}

impl Builder {
    fn add(&mut self, name: &str, defined_here: bool, defined: u32, referenced: u32) {
        match name.split_once('.') {
            Some((package, rest)) => self.packages.entry(package.to_owned()).or_default().add(rest, defined_here, defined, referenced),
            None => {
                let e = self.classes.entry(name.to_owned()).or_default();
                e.0 |= defined_here;
                e.1 += defined;
                e.2 += referenced;
            }
        }
    }

    fn build(self) -> Vec<DexNode> {
        let mut out: Vec<DexNode> = self
            .packages
            .into_iter()
            .map(|(name, b)| {
                let children = b.build();
                DexNode {
                    defined_here: children.iter().any(|c| c.defined_here),
                    defined_methods: children.iter().map(|c| c.defined_methods).sum(),
                    referenced_methods: children.iter().map(|c| c.referenced_methods).sum(),
                    name,
                    class: false,
                    children,
                }
            })
            .collect();
        out.extend(self.classes.into_iter().map(|(name, (defined_here, defined, referenced))| DexNode {
            name,
            class: true,
            defined_here,
            defined_methods: defined,
            referenced_methods: referenced,
            children: Vec::new(),
        }));
        out
    }
}

/// The dex file's summary; `None` when it isn't a dex file.
pub fn summarize(b: &[u8]) -> Option<DexSummary> {
    if !b.starts_with(b"dex\n") {
        return None;
    }
    let string_ids = u32_at(b, 0x3c)? as usize;
    let (types, type_ids) = (u32_at(b, 0x40)? as usize, u32_at(b, 0x44)? as usize);
    let (methods, method_ids) = (u32_at(b, 0x58)? as usize, u32_at(b, 0x5c)? as usize);
    let (class_count, class_defs) = (u32_at(b, 0x60)? as usize, u32_at(b, 0x64)? as usize);
    let type_name = |ix: usize| -> Option<String> { string(b, string_ids, u32_at(b, type_ids + ix * 4)?) };
    let mut referenced = vec![0u32; types];
    for m in 0..methods {
        let class = u16_at(b, method_ids + m * 8)? as usize;
        if let Some(r) = referenced.get_mut(class) {
            *r += 1;
        }
    }
    let mut defined = vec![None::<u32>; types];
    let mut total_defined = 0;
    for c in 0..class_count {
        let at = class_defs + c * 32;
        let class = u32_at(b, at)? as usize;
        let data = u32_at(b, at + 24)? as usize;
        let mut count = 0;
        if data != 0 {
            let mut p = data;
            let sizes: Vec<u32> = (0..4).map(|_| uleb(b, &mut p)).collect::<Option<_>>()?;
            count = sizes[2] + sizes[3];
        }
        total_defined += count;
        if let Some(d) = defined.get_mut(class) {
            *d = Some(count);
        }
    }
    let mut root = Builder::default();
    for ix in 0..types {
        if defined[ix].is_none() && referenced[ix] == 0 {
            continue;
        }
        let Some(name) = type_name(ix).as_deref().and_then(class_name) else { continue };
        root.add(&name, defined[ix].is_some(), defined[ix].unwrap_or(0), referenced[ix]);
    }
    Some(DexSummary { classes: class_count as u32, defined_methods: total_defined, referenced_methods: methods as u32, tree: root.build() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_digits() {
        assert_eq!(group(0), "0");
        assert_eq!(group(1234567), "1,234,567");
        assert_eq!(class_name("Lcom/a/B$C;").as_deref(), Some("com.a.B$C"));
        assert_eq!(class_name("[I"), None);
    }
}

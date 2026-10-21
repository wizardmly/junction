//! A read-only SQLite database file, as the Database Inspector shows it:
//! its tables (from `sqlite_master`), their columns (from the CREATE
//! statements) and rows, with the pages a `-wal` file holds applied on top
//! (Room keeps recent writes there).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => f.write_str("NULL"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Real(r) => write!(f, "{r}"),
            Value::Text(t) => f.write_str(t),
            Value::Blob(b) => {
                write!(f, "X'")?;
                for byte in b.iter().take(32) {
                    write!(f, "{byte:02X}")?;
                }
                if b.len() > 32 {
                    write!(f, "…")?;
                }
                write!(f, "'")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub root: u32,
    pub columns: Vec<String>,
    /// The column that is the rowid (`INTEGER PRIMARY KEY`), stored as NULL.
    pub rowid_column: Option<usize>,
    /// `WITHOUT ROWID` tables are index b-trees, which aren't read.
    pub without_rowid: bool,
    pub sql: String,
}

pub struct Database {
    data: Vec<u8>,
    page_size: usize,
    usable: usize,
    /// Pages the WAL holds newer copies of: page number → offset in `wal`.
    wal_pages: HashMap<u32, usize>,
    wal: Vec<u8>,
    utf16: Option<bool>,
}

fn varint(data: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for i in 0..9 {
        let byte = *data.get(at + i)?;
        if i == 8 {
            return Some(((value << 8) | byte as u64, 9));
        }
        value = (value << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

fn be(data: &[u8], at: usize, n: usize) -> Option<u64> {
    Some(data.get(at..at + n)?.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64))
}

impl Database {
    /// Opens the file's bytes, with its `-wal` file's if any.
    pub fn open(data: Vec<u8>, wal: Option<Vec<u8>>) -> Result<Self, String> {
        if !data.starts_with(b"SQLite format 3\0") || data.len() < 100 {
            return Err("Not an SQLite database".into());
        }
        let page_size = match be(&data, 16, 2).unwrap_or(0) {
            1 => 65536,
            n => n as usize,
        };
        if page_size < 512 {
            return Err("Bad page size".into());
        }
        let usable = page_size - data[20] as usize;
        let utf16 = match be(&data, 56, 4).unwrap_or(1) {
            2 => Some(false),
            3 => Some(true),
            _ => None,
        };
        let mut db = Self { data, page_size, usable, wal_pages: HashMap::new(), wal: Vec::new(), utf16 };
        if let Some(wal) = wal {
            db.apply_wal(wal);
        }
        Ok(db)
    }

    /// The committed frames of the WAL: each page's last copy before the
    /// last commit whose salts match the header.
    fn apply_wal(&mut self, wal: Vec<u8>) {
        let magic = be(&wal, 0, 4).unwrap_or(0);
        if (magic != 0x377f0682 && magic != 0x377f0683) || be(&wal, 8, 4) != Some(self.page_size as u64) {
            return;
        }
        let (salt1, salt2) = (be(&wal, 16, 4), be(&wal, 20, 4));
        let frame = 24 + self.page_size;
        let mut pending: HashMap<u32, usize> = HashMap::new();
        let mut at = 32;
        while at + frame <= wal.len() {
            if be(&wal, at + 8, 4) != salt1 || be(&wal, at + 12, 4) != salt2 {
                break;
            }
            let page = be(&wal, at, 4).unwrap_or(0) as u32;
            pending.insert(page, at + 24);
            if be(&wal, at + 4, 4).unwrap_or(0) != 0 {
                self.wal_pages.extend(pending.drain());
            }
            at += frame;
        }
        self.wal = wal;
    }

    fn page(&self, number: u32) -> Option<&[u8]> {
        if let Some(&at) = self.wal_pages.get(&number) {
            return self.wal.get(at..at + self.page_size);
        }
        let start = (number as usize).checked_sub(1)? * self.page_size;
        self.data.get(start..start + self.page_size)
    }

    /// A table b-tree's rows: rowid and record bytes, in rowid order.
    fn walk(&self, root: u32, limit: usize, out: &mut Vec<(i64, Vec<u8>)>, depth: usize) -> Option<()> {
        if depth > 64 || out.len() >= limit {
            return Some(());
        }
        let page = self.page(root)?;
        let header = if root == 1 { 100 } else { 0 };
        let kind = page[header];
        let cells = be(page, header + 3, 2)? as usize;
        let pointers = header + if kind == 5 { 12 } else { 8 };
        match kind {
            5 => {
                for i in 0..cells {
                    let cell = be(page, pointers + i * 2, 2)? as usize;
                    let child = be(page, cell, 4)? as u32;
                    self.walk(child, limit, out, depth + 1)?;
                }
                let right = be(page, header + 8, 4)? as u32;
                self.walk(right, limit, out, depth + 1)
            }
            13 => {
                for i in 0..cells {
                    if out.len() >= limit {
                        break;
                    }
                    let cell = be(page, pointers + i * 2, 2)? as usize;
                    let (size, n) = varint(page, cell)?;
                    let (rowid, m) = varint(page, cell + n)?;
                    let payload = self.payload(page, cell + n + m, size as usize)?;
                    out.push((rowid as i64, payload));
                }
                Some(())
            }
            _ => None,
        }
    }

    /// A cell's payload, following its overflow pages.
    fn payload(&self, page: &[u8], at: usize, size: usize) -> Option<Vec<u8>> {
        let u = self.usable;
        let x = u - 35;
        if size <= x {
            return Some(page.get(at..at + size)?.to_vec());
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + ((size - m) % (u - 4));
        let local = if k <= x { k } else { m };
        let mut out = page.get(at..at + local)?.to_vec();
        let mut next = be(page, at + local, 4)? as u32;
        let mut guard = 0;
        while out.len() < size && next != 0 && guard < 1_000_000 {
            let overflow = self.page(next)?;
            let take = (size - out.len()).min(u - 4);
            out.extend_from_slice(overflow.get(4..4 + take)?);
            next = be(overflow, 0, 4)? as u32;
            guard += 1;
        }
        Some(out)
    }

    fn text(&self, bytes: &[u8]) -> String {
        match self.utf16 {
            None => String::from_utf8_lossy(bytes).into_owned(),
            Some(big) => {
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
                String::from_utf16_lossy(&units)
            }
        }
    }

    /// A record's values.
    fn record(&self, payload: &[u8]) -> Vec<Value> {
        let Some((header_size, n)) = varint(payload, 0) else { return Vec::new() };
        let mut types = Vec::new();
        let mut at = n;
        while at < header_size as usize {
            let Some((t, m)) = varint(payload, at) else { break };
            types.push(t);
            at += m;
        }
        let mut body = header_size as usize;
        let mut out = Vec::with_capacity(types.len());
        for t in types {
            let int = |len: usize| -> i64 {
                let Some(bytes) = payload.get(body..body + len) else { return 0 };
                let mut v = bytes.iter().fold(0i64, |acc, &b| (acc << 8) | b as i64);
                // Sign-extend.
                if len < 8 && bytes.first().is_some_and(|b| b & 0x80 != 0) {
                    v -= 1i64 << (len * 8);
                }
                v
            };
            let (value, len) = match t {
                0 => (Value::Null, 0),
                1 => (Value::Integer(int(1)), 1),
                2 => (Value::Integer(int(2)), 2),
                3 => (Value::Integer(int(3)), 3),
                4 => (Value::Integer(int(4)), 4),
                5 => (Value::Integer(int(6)), 6),
                6 => (Value::Integer(int(8)), 8),
                7 => (Value::Real(f64::from_bits(int(8) as u64)), 8),
                8 => (Value::Integer(0), 0),
                9 => (Value::Integer(1), 0),
                t if t >= 12 && t % 2 == 0 => {
                    let len = ((t - 12) / 2) as usize;
                    (Value::Blob(payload.get(body..body + len).unwrap_or_default().to_vec()), len)
                }
                t if t >= 13 => {
                    let len = ((t - 13) / 2) as usize;
                    (Value::Text(self.text(payload.get(body..body + len).unwrap_or_default())), len)
                }
                _ => (Value::Null, 0),
            };
            out.push(value);
            body += len;
        }
        out
    }

    /// The tables, `sqlite_master`'s own first-level entries, by name.
    pub fn tables(&self) -> Vec<Table> {
        let mut rows = Vec::new();
        self.walk(1, usize::MAX, &mut rows, 0);
        let mut tables: Vec<Table> = rows
            .iter()
            .filter_map(|(_, payload)| {
                let values = self.record(payload);
                let text = |i: usize| match values.get(i) {
                    Some(Value::Text(t)) => t.clone(),
                    _ => String::new(),
                };
                if text(0) != "table" {
                    return None;
                }
                let root = match values.get(3) {
                    Some(Value::Integer(r)) => *r as u32,
                    _ => 0,
                };
                let sql = text(4);
                let (columns, rowid_column) = columns(&sql);
                Some(Table { name: text(1), root, columns, rowid_column, without_rowid: sql.to_uppercase().contains("WITHOUT ROWID"), sql })
            })
            .filter(|t| t.root > 0 && !t.name.starts_with("sqlite_"))
            .collect();
        tables.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        tables
    }

    /// Up to `limit` rows of the table, the rowid column filled in.
    pub fn rows(&self, table: &Table, limit: usize) -> Result<Vec<Vec<Value>>, String> {
        if table.without_rowid {
            return Err("WITHOUT ROWID tables can't be shown".into());
        }
        let mut raw = Vec::new();
        self.walk(table.root, limit, &mut raw, 0).ok_or("The table's pages can't be read")?;
        Ok(raw
            .into_iter()
            .map(|(rowid, payload)| {
                let mut values = self.record(&payload);
                // Columns added later (ALTER TABLE) are missing from old rows.
                values.resize(table.columns.len().max(values.len()), Value::Null);
                if let Some(i) = table.rowid_column {
                    if values.get(i) == Some(&Value::Null) {
                        values[i] = Value::Integer(rowid);
                    }
                }
                values
            })
            .collect())
    }
}

fn unquote(name: &str) -> String {
    let n = name.trim();
    let pairs = [('"', '"'), ('`', '`'), ('[', ']'), ('\'', '\'')];
    for (open, close) in pairs {
        if let Some(inner) = n.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
            return inner.to_owned();
        }
    }
    n.to_owned()
}

/// A column name at the start of a definition, quoted or not.
fn leading_name(def: &str) -> String {
    let def = def.trim();
    let close = match def.chars().next() {
        Some('"') => Some('"'),
        Some('`') => Some('`'),
        Some('[') => Some(']'),
        _ => None,
    };
    match close {
        Some(c) => def[1..].find(c).map_or_else(|| unquote(def), |end| def[1..1 + end].to_owned()),
        None => def.split_whitespace().next().map(unquote).unwrap_or_default(),
    }
}

/// The columns of a CREATE TABLE, and which one is the rowid.
pub fn columns(sql: &str) -> (Vec<String>, Option<usize>) {
    let Some(open) = sql.find('(') else { return (Vec::new(), None) };
    let mut defs = Vec::new();
    let (mut depth, mut start, mut quote) = (0i32, open + 1, None::<char>);
    for (i, c) in sql.char_indices().skip_while(|(i, _)| *i <= open) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '`' | '\'') => quote = Some(c),
            (None, '[') => quote = Some(']'),
            (None, '(') => depth += 1,
            (None, ')') if depth == 0 => {
                defs.push(&sql[start..i]);
                break;
            }
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                defs.push(&sql[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    let mut names = Vec::new();
    let mut rowid = None;
    for def in defs {
        let upper = def.trim().to_uppercase();
        if ["PRIMARY ", "UNIQUE", "CHECK", "FOREIGN ", "CONSTRAINT "].iter().any(|k| upper.starts_with(k)) {
            continue;
        }
        let rest = upper.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        if rest.starts_with("INTEGER PRIMARY KEY") && !rest.contains("DESC") {
            rowid = Some(names.len());
        }
        names.push(leading_name(def));
    }
    (names, rowid)
}

/// Where the Database Inspector reads an app's databases.
pub trait DatabaseSource {
    /// The database files in the app's `databases` folder.
    fn databases(&self, serial: &str, package: &str) -> Result<Vec<String>, String>;
    /// A database's bytes, and its `-wal` file's when there is one.
    fn read(&self, serial: &str, package: &str, name: &str) -> Result<(Vec<u8>, Option<Vec<u8>>), String>;
}

/// `ls -1 databases`: the databases, without their `-wal`, `-shm` and
/// `-journal` companions.
pub fn database_files(ls: &str) -> Vec<String> {
    let mut out: Vec<String> = ls
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !["-wal", "-shm", "-journal"].iter().any(|s| l.ends_with(s)))
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_listing() {
        assert_eq!(database_files("app.db\napp.db-wal\napp.db-shm\ncache.db-journal\ncache.db\n"), ["app.db", "cache.db"]);
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/testdata/sqlite").join(name)).unwrap()
    }

    #[test]
    fn create_table_columns() {
        let (cols, rowid) = columns(r#"CREATE TABLE "users" (id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, `name` TEXT NOT NULL, age INTEGER DEFAULT (1), [the score] REAL, CONSTRAINT x UNIQUE(name))"#);
        assert_eq!(cols, ["id", "name", "age", "the score"]);
        assert_eq!(rowid, Some(0));
        assert_eq!(columns("CREATE TABLE t(a, b)").0, ["a", "b"]);
    }

    #[test]
    fn tables_and_rows() {
        let db = Database::open(fixture("base.db"), None).unwrap();
        let tables = db.tables();
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["notes", "users"]);
        let users = &tables[1];
        assert_eq!(users.columns, ["id", "name", "age", "score", "avatar"]);
        let rows = db.rows(users, usize::MAX).unwrap();
        assert_eq!(rows.len(), 300);
        assert_eq!(rows[0], vec![Value::Integer(1), Value::Text("user1".into()), Value::Integer(21), Value::Real(1.5), Value::Blob(vec![1, 1, 2])]);
        assert_eq!(rows[299][0], Value::Integer(300));
        // The 5000-character note spills onto overflow pages.
        let notes = db.rows(&tables[0], 10).unwrap();
        assert_eq!(notes[0][1], Value::Text("x".repeat(5000)));
        assert_eq!(db.rows(users, 7).unwrap().len(), 7);
    }

    #[test]
    fn wal_pages_win() {
        let db = Database::open(fixture("wal.db"), Some(fixture("wal.db-wal"))).unwrap();
        let tables = db.tables();
        let notes = db.rows(&tables[0], 10).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1][1], Value::Text("from wal".into()));
        assert_eq!(db.rows(&tables[1], 1).unwrap()[0][1], Value::Text("renamed".into()));
        // Without its WAL the file is as before.
        let old = Database::open(fixture("wal.db"), None).unwrap();
        assert_eq!(old.rows(&old.tables()[0], 10).unwrap().len(), 1);
    }

    #[test]
    fn values_print_like_the_inspector() {
        assert_eq!(Value::Blob(vec![0xAB, 1]).to_string(), "X'AB01'");
        assert_eq!(Value::Null.to_string(), "NULL");
    }
}

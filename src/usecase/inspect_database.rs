//! The Database Inspector: an app's databases opened with their tables,
//! and a table's rows.

use crate::domain::profiler::ProfiledProcess;
use crate::domain::sqlite_file::{Database, DatabaseSource, Table, Value};

pub struct OpenedDatabase {
    pub name: String,
    pub database: Database,
    pub tables: Vec<Table>,
}

/// The apps of the running processes (`com.example.app:remote` is
/// `com.example.app`'s), the project's first.
pub fn apps(processes: &[ProfiledProcess], app_ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = processes.iter().map(|p| p.name.split(':').next().unwrap_or(&p.name).to_owned()).collect();
    out.sort();
    out.dedup();
    out.sort_by_key(|a| !app_ids.contains(a));
    out
}

/// Every database of the app, read from the device; one that can't be
/// read is left out and its error returned beside the others.
pub fn open_all(source: &dyn DatabaseSource, serial: &str, package: &str) -> Result<(Vec<OpenedDatabase>, Vec<String>), String> {
    let mut opened = Vec::new();
    let mut errors = Vec::new();
    for name in source.databases(serial, package)? {
        match open(source, serial, package, &name) {
            Ok(db) => opened.push(db),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Ok((opened, errors))
}

pub fn open(source: &dyn DatabaseSource, serial: &str, package: &str, name: &str) -> Result<OpenedDatabase, String> {
    let (data, wal) = source.read(serial, package, name)?;
    let database = Database::open(data, wal)?;
    let tables = database.tables();
    Ok(OpenedDatabase { name: name.to_owned(), database, tables })
}

/// Up to `limit` rows, as the grid shows them.
pub fn rows(db: &OpenedDatabase, table: &Table, limit: usize) -> Result<Vec<Vec<String>>, String> {
    Ok(db
        .database
        .rows(table, limit)?
        .into_iter()
        .map(|row| {
            row.iter()
                .map(|v| match v {
                    // One line per cell.
                    Value::Text(t) => t.replace(['\n', '\r'], " "),
                    v => v.to_string(),
                })
                .collect()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl DatabaseSource for Fake {
        fn databases(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
            Ok(vec!["app.db".into(), "broken.db".into()])
        }

        fn read(&self, _: &str, _: &str, name: &str) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
            match name {
                "app.db" => Ok((include_bytes!("../domain/testdata/sqlite/base.db").to_vec(), None)),
                _ => Err("run-as: package not debuggable".into()),
            }
        }
    }

    #[test]
    fn opens_databases_and_rows() {
        let (dbs, errors) = open_all(&Fake, "s", "com.example").unwrap();
        assert_eq!(dbs.len(), 1);
        assert_eq!(errors, ["broken.db: run-as: package not debuggable"]);
        let names: Vec<&str> = dbs[0].tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["notes", "users"]);
        let users = &dbs[0].tables[1];
        assert_eq!(rows(&dbs[0], users, 10).unwrap().len(), 10);
    }

    #[test]
    fn apps_project_first() {
        let p = |pid, name: &str| ProfiledProcess { pid, name: name.into() };
        let list = apps(&[p(1, "com.b"), p(2, "com.a"), p(3, "com.a:remote"), p(4, "com.mine")], &["com.mine".into()]);
        assert_eq!(list, ["com.mine", "com.a", "com.b"]);
    }
}

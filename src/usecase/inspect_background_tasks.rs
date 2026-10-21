//! The Background Task Inspector: an app's WorkManager workers, jobs,
//! alarms and wake locks, read from the device together.

use crate::domain::background_tasks::{self, Alarm, BackgroundTaskSource, Job, WakeLock, Worker};
use crate::domain::sqlite_file::{Database, DatabaseSource};

#[derive(Clone, Debug, Default)]
pub struct BackgroundTasks {
    pub workers: Vec<Worker>,
    pub jobs: Vec<Job>,
    pub alarms: Vec<Alarm>,
    pub wake_locks: Vec<WakeLock>,
    /// What couldn't be read, with the rest still shown.
    pub errors: Vec<String>,
}

pub fn load(databases: &dyn DatabaseSource, tasks: &dyn BackgroundTaskSource, serial: &str, package: &str) -> BackgroundTasks {
    let mut out = BackgroundTasks::default();
    match workers(databases, serial, package) {
        Ok(w) => out.workers = w,
        Err(e) => out.errors.push(format!("Workers: {e}")),
    }
    match tasks.jobs(serial, package) {
        Ok(text) => out.jobs = background_tasks::parse_jobs(&text, package),
        Err(e) => out.errors.push(format!("Jobs: {e}")),
    }
    match tasks.alarms(serial) {
        Ok(text) => out.alarms = background_tasks::parse_alarms(&text, package),
        Err(e) => out.errors.push(format!("Alarms: {e}")),
    }
    let uid = tasks.packages(serial, package).map(|text| background_tasks::parse_uid(&text, package));
    match (uid, tasks.power(serial)) {
        (Ok(Some(uid)), Ok(text)) => out.wake_locks = background_tasks::parse_wake_locks(&text, uid),
        (Ok(None), _) => out.errors.push(format!("Wake Locks: {package} isn't installed")),
        (Err(e), _) | (_, Err(e)) => out.errors.push(format!("Wake Locks: {e}")),
    }
    out
}

/// The app's workers; none when it doesn't use WorkManager.
fn workers(source: &dyn DatabaseSource, serial: &str, package: &str) -> Result<Vec<Worker>, String> {
    if !source.databases(serial, package)?.iter().any(|d| d == background_tasks::WORK_DATABASE) {
        return Ok(Vec::new());
    }
    let (data, wal) = source.read(serial, package, background_tasks::WORK_DATABASE)?;
    let db = Database::open(data, wal)?;
    background_tasks::workers(&db, &db.tables())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl DatabaseSource for Fake {
        fn databases(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
            Ok(vec!["app.db".into(), background_tasks::WORK_DATABASE.into()])
        }

        fn read(&self, _: &str, _: &str, _: &str) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
            Ok((include_bytes!("../domain/testdata/workmanager/androidx.work.workdb").to_vec(), None))
        }
    }

    impl BackgroundTaskSource for Fake {
        fn jobs(&self, _: &str, _: &str) -> Result<String, String> {
            Ok("Registered 1 jobs:\n  JOB #u0a190/7: 1 com.example/.Sync\n    Ready: true\n".into())
        }

        fn alarms(&self, _: &str) -> Result<String, String> {
            Err("dumpsys: permission denied".into())
        }

        fn power(&self, _: &str) -> Result<String, String> {
            Ok("Wake Locks: size=1\n  PARTIAL_WAKE_LOCK 'sync' ACQ=-1s (uid=10190 pid=1)\n".into())
        }

        fn packages(&self, _: &str, _: &str) -> Result<String, String> {
            Ok("package:com.example uid:10190\n".into())
        }
    }

    #[test]
    fn loads_everything_it_can() {
        let tasks = load(&Fake, &Fake, "s", "com.example");
        assert_eq!((tasks.workers.len(), tasks.jobs.len(), tasks.alarms.len(), tasks.wake_locks.len()), (4, 1, 0, 1));
        assert_eq!(tasks.errors, ["Alarms: dumpsys: permission denied"]);
    }
}

//! The Background Task Inspector's tasks, as Android Studio lists them: the
//! app's WorkManager workers (read from its `androidx.work.workdb`
//! database, with their input and output `Data` decoded), and its jobs,
//! alarms and wake locks (read from `dumpsys jobscheduler`, `alarm` and
//! `power`). Android Studio gets the same from an agent inside the app;
//! these are what the device shows without one.

use std::collections::HashMap;

use crate::domain::sqlite_file::{Database, Table, Value};

/// The database WorkManager keeps its work in.
pub const WORK_DATABASE: &str = "androidx.work.workdb";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    Enqueued,
    Running,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
}

impl WorkState {
    /// `WorkTypeConverters.StateIds`.
    fn from_id(id: i64) -> Option<Self> {
        Some(match id {
            0 => Self::Enqueued,
            1 => Self::Running,
            2 => Self::Succeeded,
            3 => Self::Failed,
            4 => Self::Blocked,
            5 => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Enqueued => "Enqueued",
            Self::Running => "Running",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Blocked => "Blocked",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Worker {
    pub id: String,
    pub class: String,
    pub state: WorkState,
    /// Tags other than the worker's class name, which WorkManager adds itself.
    pub tags: Vec<String>,
    /// Unique work names.
    pub names: Vec<String>,
    pub input: Vec<(String, String)>,
    pub output: Vec<(String, String)>,
    pub attempts: i64,
    /// When it was last enqueued, in milliseconds since the epoch.
    pub enqueued_at: Option<i64>,
    pub initial_delay_ms: i64,
    /// Periodic work's interval and flex; zero for one-time work.
    pub interval_ms: i64,
    pub flex_ms: i64,
    pub expedited: bool,
    pub constraints: Vec<String>,
    pub prerequisites: Vec<String>,
    pub dependents: Vec<String>,
}

impl Worker {
    /// `com.example.SyncWorker` is `SyncWorker`.
    pub fn simple_class(&self) -> &str {
        self.class.rsplit('.').next().unwrap_or(&self.class)
    }

    pub fn periodic(&self) -> bool {
        self.interval_ms > 0
    }
}

fn column(table: &Table, name: &str) -> Option<usize> {
    table.columns.iter().position(|c| c == name)
}

fn text(row: &[Value], at: Option<usize>) -> String {
    match at.and_then(|i| row.get(i)) {
        Some(Value::Text(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn integer(row: &[Value], at: Option<usize>) -> i64 {
    match at.and_then(|i| row.get(i)) {
        Some(Value::Integer(i)) => *i,
        Some(Value::Real(r)) => *r as i64,
        _ => 0,
    }
}

fn data(row: &[Value], at: Option<usize>) -> Vec<(String, String)> {
    match at.and_then(|i| row.get(i)) {
        Some(Value::Blob(b)) => decode_data(b).unwrap_or_else(|e| vec![("(unreadable)".into(), e)]),
        _ => Vec::new(),
    }
}

/// `(key, value)` rows of a two-column link table, by its column names.
fn pairs(db: &Database, tables: &[Table], name: &str, key: &str, value: &str) -> Result<Vec<(String, String)>, String> {
    let Some(table) = tables.iter().find(|t| t.name == name) else { return Ok(Vec::new()) };
    let (k, v) = (column(table, key), column(table, value));
    Ok(db.rows(table, usize::MAX)?.iter().map(|row| (text(row, k), text(row, v))).collect())
}

/// The workers in WorkManager's database, oldest enqueued first.
pub fn workers(db: &Database, tables: &[Table]) -> Result<Vec<Worker>, String> {
    let spec = tables.iter().find(|t| t.name == "WorkSpec").ok_or("This isn't a WorkManager database: it has no WorkSpec table")?;
    let col = |name| column(spec, name);
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (tag, id) in pairs(db, tables, "WorkTag", "tag", "work_spec_id")? {
        tags.entry(id).or_default().push(tag);
    }
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    for (name, id) in pairs(db, tables, "WorkName", "name", "work_spec_id")? {
        names.entry(id).or_default().push(name);
    }
    let dependencies = pairs(db, tables, "Dependency", "work_spec_id", "prerequisite_id")?;
    // Older versions kept the enqueue time as `period_start_time`.
    let enqueued = col("last_enqueue_time").or_else(|| col("period_start_time"));
    let mut out = Vec::new();
    for row in db.rows(spec, usize::MAX)? {
        let id = text(&row, col("id"));
        let class = text(&row, col("worker_class_name"));
        let Some(state) = WorkState::from_id(integer(&row, col("state"))) else { continue };
        let mut constraints = Vec::new();
        match integer(&row, col("required_network_type")) {
            1 => constraints.push("Network must be connected"),
            2 => constraints.push("Network must be unmetered"),
            3 => constraints.push("Network must not be roaming"),
            4 => constraints.push("Network must be metered"),
            5 => constraints.push("Network must be temporarily unmetered"),
            _ => {}
        }
        for (name, label) in [
            ("requires_charging", "Device must be charging"),
            ("requires_device_idle", "Device must be idle"),
            ("requires_battery_not_low", "Battery must not be low"),
            ("requires_storage_not_low", "Storage must not be low"),
        ] {
            if integer(&row, col(name)) != 0 {
                constraints.push(label);
            }
        }
        let mut worker_tags = tags.remove(&id).unwrap_or_default();
        worker_tags.retain(|t| t != &class);
        worker_tags.sort();
        let mut worker_names = names.remove(&id).unwrap_or_default();
        worker_names.sort();
        out.push(Worker {
            prerequisites: dependencies.iter().filter(|(w, _)| *w == id).map(|(_, p)| p.clone()).collect(),
            dependents: dependencies.iter().filter(|(_, p)| *p == id).map(|(w, _)| w.clone()).collect(),
            class,
            state,
            tags: worker_tags,
            names: worker_names,
            input: data(&row, col("input")),
            output: data(&row, col("output")),
            attempts: integer(&row, col("run_attempt_count")),
            enqueued_at: Some(integer(&row, enqueued)).filter(|t| *t > 0),
            initial_delay_ms: integer(&row, col("initial_delay")),
            interval_ms: integer(&row, col("interval_duration")),
            flex_ms: integer(&row, col("flex_duration")),
            expedited: integer(&row, col("run_in_foreground")) != 0,
            constraints: constraints.into_iter().map(str::to_owned).collect(),
            id,
        });
    }
    out.sort_by_key(|w| w.enqueued_at.unwrap_or(i64::MAX));
    Ok(out)
}

/// A big-endian reader over a byte slice.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let out = self.b.get(self.at..self.at + n).ok_or("Data ends early")?;
        self.at += n;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }

    fn uint(&mut self, n: usize) -> Result<u64, String> {
        Ok(self.take(n)?.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.uint(4)? as u32 as i32)
    }

    fn utf(&mut self) -> Result<String, String> {
        let n = self.uint(2)? as usize;
        Ok(modified_utf8(self.take(n)?))
    }
}

/// Java's modified UTF-8: NUL is `C0 80`; surrogate pairs are read lossily.
fn modified_utf8(b: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0xc0 && b.get(i + 1) == Some(&0x80) {
            bytes.push(0);
            i += 2;
        } else {
            bytes.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `androidx.work.Data` as WorkManager stores it, its entries as text:
/// version 1 (`DataOutputStream`, WorkManager 2.9 on) or version 0 (a
/// Java `ObjectOutputStream`).
pub fn decode_data(b: &[u8]) -> Result<Vec<(String, String)>, String> {
    if b.is_empty() {
        return Ok(Vec::new());
    }
    if b.starts_with(&[0xac, 0xed]) {
        return JavaStream::new(b).data();
    }
    let mut r = Reader { b, at: 0 };
    if r.uint(2)? != 0xabef {
        return Err("Unknown Data format".into());
    }
    if r.uint(2)? != 1 {
        return Err("Unknown Data version".into());
    }
    let count = r.i32()?;
    let mut out = Vec::new();
    for _ in 0..count {
        let kind = r.u8()?;
        let one = |r: &mut Reader, kind: u8| -> Result<String, String> {
            Ok(match kind {
                1 => (r.u8()? != 0).to_string(),
                2 => (r.u8()? as i8).to_string(),
                3 => r.i32()?.to_string(),
                4 => (r.uint(8)? as i64).to_string(),
                5 => f32::from_bits(r.uint(4)? as u32).to_string(),
                6 => f64::from_bits(r.uint(8)?).to_string(),
                7 => quoted(&r.utf()?),
                _ => return Err(format!("Unknown Data type {kind}")),
            })
        };
        let value = match kind {
            0 => "null".to_owned(),
            1..=7 => one(&mut r, kind)?,
            8..=14 => {
                let n = r.i32()?.max(0);
                let mut items = Vec::new();
                for _ in 0..n {
                    let item = one(&mut r, kind - 7)?;
                    items.push(if item == "\"androidx.work.Data-95ed6082-b8e9-46e8-a73f-ff56f00f5d9d\"" { "null".to_owned() } else { item });
                }
                format!("[{}]", items.join(", "))
            }
            _ => return Err(format!("Unknown Data type {kind}")),
        };
        out.push((r.utf()?, value));
    }
    Ok(out)
}

fn quoted(s: &str) -> String {
    format!("\"{s}\"")
}

/// A value read from a Java object stream.
#[derive(Clone, Debug)]
enum Java {
    Null,
    Text(String),
    Primitive(String),
    Array(Vec<Java>),
    Object { class: String, fields: Vec<(String, Java)> },
    Enum(String),
    Class(String),
}

impl std::fmt::Display for Java {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Java::Null => f.write_str("null"),
            Java::Text(t) => write!(f, "\"{t}\""),
            Java::Primitive(p) | Java::Enum(p) => f.write_str(p),
            Java::Class(c) => write!(f, "class {c}"),
            Java::Array(items) => write!(f, "[{}]", items.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")),
            // Boxed primitives (`Integer`, `Long`, …) show their value.
            Java::Object { fields, .. } if fields.len() == 1 && fields[0].0 == "value" => write!(f, "{}", fields[0].1),
            Java::Object { class, fields } => write!(f, "{class}{{{}}}", fields.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(", ")),
        }
    }
}

#[derive(Debug)]
struct ClassDesc {
    name: String,
    flags: u8,
    fields: Vec<(u8, String)>,
    parent: Option<std::rc::Rc<ClassDesc>>,
}

#[derive(Clone, Debug)]
enum Handle {
    Desc(std::rc::Rc<ClassDesc>),
    Value(Java),
}

/// The parts of the Java serialization protocol `Data` uses: block data
/// for its count and keys, and strings, boxed primitives and their arrays
/// (other objects are read field by field).
struct JavaStream<'a> {
    r: Reader<'a>,
    handles: Vec<Handle>,
    /// Bytes left in the current block data record.
    block: usize,
}

const TC_NULL: u8 = 0x70;
const TC_REFERENCE: u8 = 0x71;
const TC_CLASSDESC: u8 = 0x72;
const TC_OBJECT: u8 = 0x73;
const TC_STRING: u8 = 0x74;
const TC_ARRAY: u8 = 0x75;
const TC_CLASS: u8 = 0x76;
const TC_BLOCKDATA: u8 = 0x77;
const TC_ENDBLOCKDATA: u8 = 0x78;
const TC_LONGSTRING: u8 = 0x7c;
const TC_ENUM: u8 = 0x7e;
const BASE_HANDLE: u32 = 0x7e0000;

impl<'a> JavaStream<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { r: Reader { b, at: 4 }, handles: Vec::new(), block: 0 }
    }

    fn data(mut self) -> Result<Vec<(String, String)>, String> {
        let count = i32::from_be_bytes(self.block_bytes(4)?.try_into().unwrap_or_default());
        let mut out = Vec::new();
        for _ in 0..count {
            let n = u16::from_be_bytes(self.block_bytes(2)?.try_into().unwrap_or_default()) as usize;
            let key = modified_utf8(&self.block_bytes(n)?);
            let value = self.content()?;
            out.push((key, value.to_string()));
        }
        Ok(out)
    }

    /// `n` bytes of primitive data, across block data records.
    fn block_bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if self.block == 0 {
                self.block = match self.r.u8()? {
                    TC_BLOCKDATA => self.r.u8()? as usize,
                    0x7a => self.r.uint(4)? as usize,
                    other => return Err(format!("Expected block data, found 0x{other:02x}")),
                };
                continue;
            }
            let take = self.block.min(n - out.len());
            out.extend_from_slice(self.r.take(take)?);
            self.block -= take;
        }
        Ok(out)
    }

    fn new_handle(&mut self, h: Handle) -> usize {
        self.handles.push(h);
        self.handles.len() - 1
    }

    fn reference(&mut self) -> Result<Handle, String> {
        let h = (self.r.uint(4)? as u32).wrapping_sub(BASE_HANDLE) as usize;
        self.handles.get(h).cloned().ok_or_else(|| "Bad object reference".to_owned())
    }

    fn class_desc(&mut self) -> Result<Option<std::rc::Rc<ClassDesc>>, String> {
        match self.r.u8()? {
            TC_NULL => Ok(None),
            TC_REFERENCE => match self.reference()? {
                Handle::Desc(d) => Ok(Some(d)),
                Handle::Value(_) => Err("Expected a class description".into()),
            },
            TC_CLASSDESC => {
                let name = self.r.utf()?;
                self.r.take(8)?;
                let slot = self.new_handle(Handle::Value(Java::Null));
                let flags = self.r.u8()?;
                let count = self.r.uint(2)?;
                let mut fields = Vec::new();
                for _ in 0..count {
                    let code = self.r.u8()?;
                    let field = self.r.utf()?;
                    if matches!(code, b'L' | b'[') {
                        self.content()?;
                    }
                    fields.push((code, field));
                }
                self.skip_annotation()?;
                let parent = self.class_desc()?;
                let desc = std::rc::Rc::new(ClassDesc { name, flags, fields, parent });
                self.handles[slot] = Handle::Desc(desc.clone());
                Ok(Some(desc))
            }
            other => Err(format!("Unsupported class description 0x{other:02x}")),
        }
    }

    /// Contents up to `TC_ENDBLOCKDATA`: a class's annotation, or what a
    /// `writeObject` method wrote after the fields.
    fn skip_annotation(&mut self) -> Result<(), String> {
        loop {
            match self.r.peek().ok_or("Data ends early")? {
                TC_ENDBLOCKDATA => {
                    self.r.u8()?;
                    return Ok(());
                }
                TC_BLOCKDATA => {
                    self.r.u8()?;
                    let n = self.r.u8()? as usize;
                    self.r.take(n)?;
                }
                0x7a => {
                    self.r.u8()?;
                    let n = self.r.uint(4)? as usize;
                    self.r.take(n)?;
                }
                _ => {
                    self.content()?;
                }
            }
        }
    }

    fn primitive(&mut self, code: u8) -> Result<Java, String> {
        let r = &mut self.r;
        Ok(Java::Primitive(match code {
            b'B' => (r.u8()? as i8).to_string(),
            b'C' => char::from_u32(r.uint(2)? as u32).unwrap_or('?').to_string(),
            b'D' => f64::from_bits(r.uint(8)?).to_string(),
            b'F' => f32::from_bits(r.uint(4)? as u32).to_string(),
            b'I' => r.i32()?.to_string(),
            b'J' => (r.uint(8)? as i64).to_string(),
            b'S' => (r.uint(2)? as u16 as i16).to_string(),
            b'Z' => (r.u8()? != 0).to_string(),
            b'L' | b'[' => return self.content(),
            other => return Err(format!("Unknown field type {}", other as char)),
        }))
    }

    fn content(&mut self) -> Result<Java, String> {
        match self.r.u8()? {
            TC_NULL => Ok(Java::Null),
            TC_REFERENCE => match self.reference()? {
                Handle::Value(v) => Ok(v),
                Handle::Desc(d) => Ok(Java::Class(d.name.clone())),
            },
            TC_STRING => {
                let s = Java::Text(self.r.utf()?);
                self.new_handle(Handle::Value(s.clone()));
                Ok(s)
            }
            TC_LONGSTRING => {
                let n = self.r.uint(8)? as usize;
                let s = Java::Text(modified_utf8(self.r.take(n)?));
                self.new_handle(Handle::Value(s.clone()));
                Ok(s)
            }
            TC_CLASS => {
                let desc = self.class_desc()?.ok_or("Null class")?;
                let v = Java::Class(desc.name.clone());
                self.new_handle(Handle::Value(v.clone()));
                Ok(v)
            }
            TC_ENUM => {
                let desc = self.class_desc()?.ok_or("Null enum class")?;
                let slot = self.new_handle(Handle::Value(Java::Null));
                let name = match self.content()? {
                    Java::Text(t) => t,
                    other => other.to_string(),
                };
                let v = Java::Enum(format!("{}.{name}", desc.name.rsplit('.').next().unwrap_or(&desc.name)));
                self.handles[slot] = Handle::Value(v.clone());
                Ok(v)
            }
            TC_ARRAY => {
                let desc = self.class_desc()?.ok_or("Null array class")?;
                let slot = self.new_handle(Handle::Value(Java::Null));
                let n = self.r.i32()?.max(0);
                let code = desc.name.as_bytes().get(1).copied().unwrap_or(b'L');
                let mut items = Vec::new();
                for _ in 0..n {
                    items.push(self.primitive(code)?);
                }
                let v = Java::Array(items);
                self.handles[slot] = Handle::Value(v.clone());
                Ok(v)
            }
            TC_OBJECT => {
                let desc = self.class_desc()?.ok_or("Null object class")?;
                let slot = self.new_handle(Handle::Value(Java::Null));
                let mut chain = Vec::new();
                let mut at = Some(desc.clone());
                while let Some(d) = at {
                    at = d.parent.clone();
                    chain.push(d);
                }
                let mut fields = Vec::new();
                for d in chain.iter().rev() {
                    // Externalizable objects write their own data.
                    if d.flags & 0x04 != 0 {
                        self.skip_annotation()?;
                        continue;
                    }
                    for (code, name) in &d.fields {
                        fields.push((name.clone(), self.primitive(*code)?));
                    }
                    if d.flags & 0x01 != 0 {
                        self.skip_annotation()?;
                    }
                }
                let v = Java::Object { class: desc.name.clone(), fields };
                self.handles[slot] = Handle::Value(v.clone());
                Ok(v)
            }
            other => Err(format!("Unsupported object type 0x{other:02x}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    /// All constraints are met; it waits for the scheduler.
    Ready,
    Waiting,
}

impl JobState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Ready => "Ready",
            Self::Waiting => "Waiting",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: i64,
    pub service: String,
    pub state: JobState,
    pub required: Vec<String>,
    pub unsatisfied: Vec<String>,
    /// `-3m12s345ms`: how long ago it was enqueued.
    pub enqueued: Option<String>,
    /// `earliest=+11m47s, latest=none`.
    pub run_time: Option<String>,
    pub periodic: Option<String>,
    pub persisted: bool,
}

impl Job {
    /// WorkManager schedules its workers as jobs of its `SystemJobService`.
    pub fn work_manager(&self) -> bool {
        self.service.ends_with("androidx.work.impl.background.systemjob.SystemJobService")
    }

    pub fn simple_service(&self) -> &str {
        let class = self.service.rsplit('/').next().unwrap_or(&self.service);
        class.rsplit('.').next().unwrap_or(class)
    }
}

/// `JOB #u0a190/12: 6a3f2d1 com.example/.SyncService`: the job id and the
/// `package/service` it runs.
fn job_header(line: &str) -> Option<(i64, &str)> {
    let rest = &line[line.find("JOB #")? + 5..];
    let (_, rest) = rest.split_once('/')?;
    let (id, rest) = rest.split_once(':')?;
    let service = rest.split_whitespace().find(|w| w.contains('/'))?;
    Some((id.trim().parse().ok()?, service))
}

fn constraint_list(value: &str) -> Vec<String> {
    value.split_whitespace().filter(|w| !w.starts_with('[')).map(str::to_owned).collect()
}

/// The package's jobs in `dumpsys jobscheduler <package>`: its registered
/// jobs, with the ones in "Active jobs" running.
pub fn parse_jobs(text: &str, package: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut active = Vec::new();
    let mut section = "";
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if indent == 0 && !t.is_empty() {
            section = if t.starts_with("Registered") {
                "registered"
            } else if t.starts_with("Active jobs") {
                "active"
            } else {
                ""
            };
            current = None;
            continue;
        }
        if section == "active" {
            if let Some((id, service)) = job_header(t)
                && service.starts_with(&format!("{package}/"))
            {
                active.push(id);
            }
            continue;
        }
        if section != "registered" {
            continue;
        }
        if t.starts_with("JOB #") {
            current = None;
            if let Some((id, service)) = job_header(t)
                && service.starts_with(&format!("{package}/"))
                && !jobs.iter().any(|j| j.id == id)
            {
                jobs.push(Job { id, service: service.to_owned(), state: JobState::Waiting, required: Vec::new(), unsatisfied: Vec::new(), enqueued: None, run_time: None, periodic: None, persisted: false });
                current = Some(jobs.len() - 1);
            }
            continue;
        }
        let Some(job) = current.map(|i| &mut jobs[i]) else { continue };
        if let Some(v) = t.strip_prefix("Required constraints:") {
            job.required = constraint_list(v);
        } else if let Some(v) = t.strip_prefix("Unsatisfied constraints:") {
            job.unsatisfied = constraint_list(v);
        } else if let Some(v) = t.strip_prefix("Enqueue time:") {
            job.enqueued = Some(v.trim().to_owned());
        } else if let Some(v) = t.strip_prefix("Run time:") {
            job.run_time = Some(v.trim().to_owned());
        } else if let Some(v) = t.strip_prefix("Periodic:") {
            job.periodic = Some(v.trim().to_owned());
        } else if t == "PERSISTED" {
            job.persisted = true;
        } else if let Some(v) = t.strip_prefix("Ready:") {
            if v.trim_start().starts_with("true") {
                job.state = JobState::Ready;
            }
        }
    }
    for job in &mut jobs {
        if active.contains(&job.id) {
            job.state = JobState::Running;
        }
    }
    jobs
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarm {
    /// `RTC_WAKEUP`, `ELAPSED`, …
    pub kind: String,
    pub tag: String,
    /// `+2m3s`: how long until it fires.
    pub when: Option<String>,
    pub repeat: Option<String>,
    pub window: Option<String>,
    /// The `PendingIntent` or listener it fires.
    pub operation: Option<String>,
}

impl Alarm {
    pub fn wakeup(&self) -> bool {
        self.kind.ends_with("_WAKEUP")
    }
}

/// `key=value` in a line of `key=value` pairs.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
}

/// The package's pending alarms in `dumpsys alarm`.
pub fn parse_alarms(text: &str, package: &str) -> Vec<Alarm> {
    let mut out: Vec<Alarm> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if let Some((at, owner_indent)) = current {
            if indent > owner_indent {
                let alarm = &mut out[at];
                if let Some(tag) = t.strip_prefix("tag=") {
                    alarm.tag = tag.to_owned();
                } else if let Some(op) = t.strip_prefix("operation=").or_else(|| t.strip_prefix("listener=")) {
                    alarm.operation = Some(op.to_owned());
                } else {
                    if let Some(w) = field(t, "whenElapsed").or_else(|| field(t, "when")).filter(|w| w.starts_with(['+', '-'])) {
                        alarm.when = Some(w.to_owned());
                    }
                    if let Some(r) = field(t, "repeatInterval").filter(|r| *r != "0") {
                        alarm.repeat = Some(r.to_owned());
                    }
                    if let Some(w) = field(t, "window").filter(|w| *w != "0") {
                        alarm.window = Some(w.to_owned());
                    }
                }
                continue;
            }
            current = None;
        }
        // `RTC_WAKEUP #3: Alarm{4a1b2c3 type 0 origWhen 1760000600000 com.example}`
        let Some((kind, rest)) = t.split_once(" #") else { continue };
        let Some(alarm) = rest.split_once(": Alarm{").map(|(_, a)| a) else { continue };
        if !alarm.trim_end_matches('}').ends_with(&format!(" {package}")) || !kind.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            continue;
        }
        // Each alarm is listed under its batch and again in other summaries.
        if !seen.insert(alarm.split_whitespace().next().unwrap_or_default().to_owned()) {
            continue;
        }
        out.push(Alarm { kind: kind.to_owned(), tag: String::new(), when: None, repeat: None, window: None, operation: None });
        current = Some((out.len() - 1, indent));
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeLock {
    /// `PARTIAL_WAKE_LOCK`, …
    pub level: String,
    pub tag: String,
    /// `-5s123ms`: how long ago it was acquired.
    pub acquired: Option<String>,
}

/// The wake locks held by `uid` in `dumpsys power`.
pub fn parse_wake_locks(text: &str, uid: u32) -> Vec<WakeLock> {
    let Some(start) = text.find("Wake Locks: size=") else { return Vec::new() };
    let owner = format!("uid={uid}");
    text[start..]
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with(' ') && !l.trim().is_empty())
        .filter(|l| l.split(['(', ' ', ')']).any(|w| w == owner))
        .filter_map(|l| {
            let t = l.trim();
            let level = t.split_whitespace().next()?.to_owned();
            let (_, rest) = t.split_once('\'')?;
            let (tag, rest) = rest.split_once('\'')?;
            Some(WakeLock { level, tag: tag.to_owned(), acquired: field(rest, "ACQ").map(str::to_owned) })
        })
        .collect()
}

/// The app's uid in `pm list packages -U <package>`: `package:com.example uid:10190`.
pub fn parse_uid(text: &str, package: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let mut words = l.split_whitespace();
        (words.next()?.strip_prefix("package:")? == package).then_some(())?;
        words.find_map(|w| w.strip_prefix("uid:"))?.split(',').next()?.parse().ok()
    })
}

/// `+2m3s` is `in 2m 3s`; `-3m12s345ms` is `3m 12s ago`.
pub fn relative(duration: &str) -> String {
    let (sign, rest) = match duration.as_bytes().first() {
        Some(b'+') => (1, &duration[1..]),
        Some(b'-') => (-1, &duration[1..]),
        _ => return duration.to_owned(),
    };
    let mut parts = Vec::new();
    let mut number = String::new();
    let mut unit = String::new();
    for c in rest.chars().chain(std::iter::once('0')) {
        if c.is_ascii_digit() {
            if !unit.is_empty() {
                let zero = number.trim_start_matches('0').is_empty();
                if !zero && (unit != "ms" || parts.is_empty()) {
                    parts.push(format!("{number}{unit}"));
                }
                number.clear();
                unit.clear();
            }
            number.push(c);
        } else {
            unit.push(c);
        }
    }
    if parts.is_empty() {
        return duration.to_owned();
    }
    let text = parts.join(" ");
    if sign > 0 { format!("in {text}") } else { format!("{text} ago") }
}

/// Milliseconds as `15m`, `1h 30m`, `45s`.
pub fn duration_ms(ms: i64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, s / 60 % 60, s % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if sec > 0 || parts.is_empty() {
        parts.push(if s == 0 { format!("{ms}ms") } else { format!("{sec}s") });
    }
    parts.join(" ")
}

/// Where the Background Task Inspector reads the device's view of an app.
pub trait BackgroundTaskSource {
    /// `dumpsys jobscheduler <package>`.
    fn jobs(&self, serial: &str, package: &str) -> Result<String, String>;
    /// `dumpsys alarm`.
    fn alarms(&self, serial: &str) -> Result<String, String>;
    /// `dumpsys power`.
    fn power(&self, serial: &str) -> Result<String, String>;
    /// `pm list packages -U <package>`.
    fn packages(&self, serial: &str, package: &str) -> Result<String, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by `java.io.ObjectOutputStream` as WorkManager 2.8 does:
    /// a String, an Integer, a Long, an Integer[] and a back-referenced Integer.
    const V0: &[u8] = &[
        0xac, 0xed, 0x00, 0x05, 0x77, 0x09, 0x00, 0x00, 0x00, 0x05, 0x00, 0x03, 0x75, 0x72, 0x6c, 0x74, 0x00, 0x15, 0x68, 0x74, 0x74, 0x70, 0x73, 0x3a, 0x2f, 0x2f, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c,
        0x65, 0x2e, 0x63, 0x6f, 0x6d, 0x2f, 0x61, 0x77, 0x07, 0x00, 0x05, 0x63, 0x6f, 0x75, 0x6e, 0x74, 0x73, 0x72, 0x00, 0x11, 0x6a, 0x61, 0x76, 0x61, 0x2e, 0x6c, 0x61, 0x6e, 0x67, 0x2e, 0x49, 0x6e,
        0x74, 0x65, 0x67, 0x65, 0x72, 0x12, 0xe2, 0xa0, 0xa4, 0xf7, 0x81, 0x87, 0x38, 0x02, 0x00, 0x01, 0x49, 0x00, 0x05, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x78, 0x72, 0x00, 0x10, 0x6a, 0x61, 0x76, 0x61,
        0x2e, 0x6c, 0x61, 0x6e, 0x67, 0x2e, 0x4e, 0x75, 0x6d, 0x62, 0x65, 0x72, 0x86, 0xac, 0x95, 0x1d, 0x0b, 0x94, 0xe0, 0x8b, 0x02, 0x00, 0x00, 0x78, 0x70, 0x00, 0x00, 0x00, 0x03, 0x77, 0x05, 0x00,
        0x03, 0x62, 0x69, 0x67, 0x73, 0x72, 0x00, 0x0e, 0x6a, 0x61, 0x76, 0x61, 0x2e, 0x6c, 0x61, 0x6e, 0x67, 0x2e, 0x4c, 0x6f, 0x6e, 0x67, 0x3b, 0x8b, 0xe4, 0x90, 0xcc, 0x8f, 0x23, 0xdf, 0x02, 0x00,
        0x01, 0x4a, 0x00, 0x05, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x78, 0x71, 0x00, 0x7e, 0x00, 0x02, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x05, 0x00, 0x03, 0x69, 0x64, 0x73, 0x75, 0x72,
        0x00, 0x14, 0x5b, 0x4c, 0x6a, 0x61, 0x76, 0x61, 0x2e, 0x6c, 0x61, 0x6e, 0x67, 0x2e, 0x49, 0x6e, 0x74, 0x65, 0x67, 0x65, 0x72, 0x3b, 0xfe, 0x97, 0xad, 0xa0, 0x01, 0x83, 0xe2, 0x1b, 0x02, 0x00,
        0x00, 0x78, 0x70, 0x00, 0x00, 0x00, 0x02, 0x73, 0x71, 0x00, 0x7e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x73, 0x71, 0x00, 0x7e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x77, 0x07, 0x00, 0x05, 0x61,
        0x67, 0x61, 0x69, 0x6e, 0x73, 0x71, 0x00, 0x7e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07,
    ];

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn decodes_java_serialized_data() {
        assert_eq!(decode_data(V0).unwrap(), kv(&[("url", "\"https://example.com/a\""), ("count", "3"), ("big", "1099511627776"), ("ids", "[1, 2]"), ("again", "7")]));
    }

    #[test]
    fn reads_work_manager_database() {
        let db = Database::open(include_bytes!("testdata/workmanager/androidx.work.workdb").to_vec(), None).unwrap();
        let workers = workers(&db, &db.tables()).unwrap();
        let ids: Vec<&str> = workers.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["1b2c-sync", "3d4e-compress", "4e5f-clean", "2c3d-upload"]);
        let sync = &workers[0];
        assert_eq!((sync.simple_class(), sync.state, sync.periodic()), ("SyncWorker", WorkState::Running, true));
        assert_eq!((sync.tags.clone(), sync.names.clone()), (vec!["sync".to_owned()], vec!["periodic-sync".to_owned()]));
        assert_eq!(sync.constraints, ["Network must be connected", "Device must be charging"]);
        // Version 1 Data, with a null in a String array.
        assert_eq!(sync.input, kv(&[("name", "\"photo.jpg\""), ("done", "true"), ("ratio", "1.5"), ("list", "[\"a\", null]")]));
        let upload = &workers[3];
        assert_eq!((upload.state, upload.expedited, upload.prerequisites.clone()), (WorkState::Blocked, true, vec!["3d4e-compress".to_owned()]));
        assert_eq!(upload.input.len(), 5);
        assert_eq!(workers[1].dependents, ["2c3d-upload"]);
        assert_eq!((workers[2].state, workers[2].attempts), (WorkState::Failed, 3));
    }

    #[test]
    fn parses_jobs() {
        let text = "\
JOB SCHEDULER (dumpsys jobscheduler)
Registered 3 jobs:
  JOB #u0a190/12: 6a3f2d1 com.example/androidx.work.impl.background.systemjob.SystemJobService
    u0a190 tag=*job*/com.example/androidx.work.impl.background.systemjob.SystemJobService
    Source: uid=u0a190 user=0 pkg=com.example
    JobInfo:
      Service: com.example/androidx.work.impl.background.systemjob.SystemJobService
      PERSISTED
      Minimum latency: +14m59s999ms
    Required constraints: TIMING_DELAY CONNECTIVITY [0x90000000]
    Satisfied constraints: CONNECTIVITY DEVICE_NOT_DOZING [0x12400000]
    Unsatisfied constraints: TIMING_DELAY [0x80000000]
    Enqueue time: -3m12s345ms
    Run time: earliest=+11m47s654ms, latest=none, original latency=+14m59s999ms
    Ready: false (job=false user=true)
  JOB #u0a190/13: 8c1e2f3 com.example/com.example.SyncService
    Periodic: interval=+15m0s0ms flex=+5m0s0ms
    Required constraints:
    Enqueue time: -10s
    Ready: true (job=true user=true)
  JOB #u0a77/1: 1111111 com.other/com.other.Job
    Ready: true
Active jobs:
  Slot #0(ID=2): JOB #u0a190/13: 8c1e2f3 com.example/com.example.SyncService
    Running for: +2s100ms, timeout at: +9m57s
";
        let jobs = parse_jobs(text, "com.example");
        assert_eq!(jobs.len(), 2);
        assert_eq!((jobs[0].id, jobs[0].state, jobs[0].work_manager(), jobs[0].persisted), (12, JobState::Waiting, true, true));
        assert_eq!((jobs[0].required.clone(), jobs[0].unsatisfied.clone()), (vec!["TIMING_DELAY".to_owned(), "CONNECTIVITY".to_owned()], vec!["TIMING_DELAY".to_owned()]));
        assert_eq!(jobs[0].enqueued.as_deref(), Some("-3m12s345ms"));
        assert_eq!((jobs[1].simple_service(), jobs[1].state, jobs[1].periodic.as_deref()), ("SyncService", JobState::Running, Some("interval=+15m0s0ms flex=+5m0s0ms")));
    }

    #[test]
    fn parses_alarms() {
        let text = "\
Pending alarm batches: 2
Batch{1a2b3c num=1 start=123 end=456 flgs=0x1}:
    RTC_WAKEUP #0: Alarm{4a1b2c3 type 0 origWhen 1760000600000 com.example}
      tag=*walarm*:com.example.action.REMIND
      type=RTC_WAKEUP origWhen=2025-10-09 10:03:20.000 window=+10m0s0ms repeatInterval=0 count=0 flags=0x0
      whenElapsed=+2m3s maxWhenElapsed=+12m3s
      operation=PendingIntent{2b3c4d5: PendingIntentRecord{6e7f8a9 com.example broadcastIntent}}
    ELAPSED #1: Alarm{5b5b5b5 type 3 origWhen 100 com.other}
      tag=*alarm*:other
Batch{2b3c4d num=1 start=789 end=789 flgs=0x0}:
    ELAPSED #0: Alarm{7c7c7c7 type 3 when +1h0m0s com.example}
      tag=*alarm*:com.example/androidx.work.impl.background.systemalarm.RescheduleReceiver
      type=3 whenElapsed=+1h0m0s when=+1h0m0s window=0 repeatInterval=3600000 count=0 flags=0x0
      listener=null
Top Alarms:
    +1s ... RTC_WAKEUP #0: Alarm{4a1b2c3 type 0 origWhen 1760000600000 com.example}
";
        let alarms = parse_alarms(text, "com.example");
        assert_eq!(alarms.len(), 2);
        assert_eq!((alarms[0].kind.as_str(), alarms[0].tag.as_str(), alarms[0].when.as_deref(), alarms[0].wakeup()), ("RTC_WAKEUP", "*walarm*:com.example.action.REMIND", Some("+2m3s"), true));
        assert_eq!(alarms[0].window.as_deref(), Some("+10m0s0ms"));
        assert!(alarms[0].operation.as_deref().unwrap().starts_with("PendingIntent{"));
        assert_eq!((alarms[1].repeat.as_deref(), alarms[1].when.as_deref()), (Some("3600000"), Some("+1h0m0s")));
    }

    #[test]
    fn parses_wake_locks_and_uid() {
        let power = "\
Wake Locks: size=3
  PARTIAL_WAKE_LOCK              'WorkManager: SyncWorker' ACQ=-5s123ms (uid=10190 pid=4321)
  PARTIAL_WAKE_LOCK              'other' ACQ=-1s (uid=10191 pid=1)
  SCREEN_BRIGHT_WAKE_LOCK        'WindowManager' ON_AFTER_RELEASE ACQ=-1m (uid=1000 pid=2 ws=WorkSource{10190})

Suspend Blockers: size=1
";
        let locks = parse_wake_locks(power, 10190);
        assert_eq!(locks, [WakeLock { level: "PARTIAL_WAKE_LOCK".into(), tag: "WorkManager: SyncWorker".into(), acquired: Some("-5s123ms".into()) }]);
        assert_eq!(parse_uid("package:com.example.debug uid:1\npackage:com.example uid:10190\n", "com.example"), Some(10190));
    }

    #[test]
    fn humanizes_durations() {
        assert_eq!(relative("+2m3s"), "in 2m 3s");
        assert_eq!(relative("-3m12s345ms"), "3m 12s ago");
        assert_eq!(relative("-345ms"), "345ms ago");
        assert_eq!(relative("none"), "none");
        assert_eq!(relative("+1h0m0s"), "in 1h");
        assert_eq!(duration_ms(900_000), "15m");
        assert_eq!(duration_ms(5_400_000), "1h 30m");
        assert_eq!(duration_ms(250), "250ms");
    }
}

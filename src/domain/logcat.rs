//! Logcat, as Android Studio's: the device log's entries and the filter
//! query language (`package:mine level:warn tag:Foo -tag:Bar age:5m
//! is:crash`, `key~:` for a regex, `key=:` for an exact match, plain words
//! anywhere in the line). Terms with the same key are ORed, the rest ANDed,
//! as Android Studio combines them.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Assert,
}

impl Level {
    pub fn from_letter(c: char) -> Option<Self> {
        Some(match c {
            'V' => Level::Verbose,
            'D' => Level::Debug,
            'I' => Level::Info,
            'W' => Level::Warn,
            'E' => Level::Error,
            'A' | 'F' => Level::Assert,
            _ => return None,
        })
    }

    pub fn letter(self) -> char {
        match self {
            Level::Verbose => 'V',
            Level::Debug => 'D',
            Level::Info => 'I',
            Level::Warn => 'W',
            Level::Error => 'E',
            Level::Assert => 'A',
        }
    }

    /// `level:` values: a letter or the name, any case.
    fn parse(text: &str) -> Option<Self> {
        let t = text.to_ascii_uppercase();
        match t.as_str() {
            "VERBOSE" => Some(Level::Verbose),
            "DEBUG" => Some(Level::Debug),
            "INFO" => Some(Level::Info),
            "WARN" | "WARNING" => Some(Level::Warn),
            "ERROR" => Some(Level::Error),
            "ASSERT" | "FATAL" => Some(Level::Assert),
            _ if t.chars().count() == 1 => Level::from_letter(t.chars().next()?),
            _ => None,
        }
    }
}

/// One log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// "2026-10-15 02:05:48.123"
    pub time: String,
    /// Milliseconds since the epoch on the device's clock, for `age:`.
    pub millis: i64,
    pub pid: u32,
    pub tid: u32,
    pub level: Level,
    pub tag: String,
    /// The app's package, once its process is known; else the process name.
    pub package: String,
    pub message: String,
    /// Android Studio's PROCESS STARTED / ENDED separator, not a log line.
    pub system: bool,
}

/// `logcat -v threadtime -v year`:
/// `2026-10-15 02:05:48.123  1234  1250 D Tag     : message`.
pub fn parse_threadtime(line: &str) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut rest = line;
    let mut field = || {
        rest = rest.trim_start();
        let end = rest.find(' ').unwrap_or(rest.len());
        let (f, r) = rest.split_at(end);
        rest = r;
        f
    };
    let (date, time) = (field(), field());
    if !date.contains('-') || !time.contains(':') {
        return None;
    }
    let pid = field().parse().ok()?;
    let tid = field().parse().ok()?;
    let level = Level::from_letter(field().chars().next()?)?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let (tag, message) = match rest.find(": ") {
        Some(i) => (rest[..i].trim(), &rest[i + 2..]),
        None => (rest.trim_end_matches(':').trim(), ""),
    };
    let date = if date.len() == 5 { format!("{}-{date}", chrono::Local::now().format("%Y")) } else { date.to_owned() };
    let full = format!("{date} {time}");
    let millis = chrono::NaiveDateTime::parse_from_str(&full, "%Y-%m-%d %H:%M:%S%.3f")
        .ok()
        .and_then(|t| t.and_local_timezone(chrono::Local).single())
        .map_or(0, |t| t.timestamp_millis());
    Some(Entry { time: full, millis, pid, tid, level, tag: tag.to_owned(), package: String::new(), message: message.to_owned(), system: false })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Tag,
    Package,
    Process,
    Message,
    Line,
    Level,
    Age,
    Is,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Contains,
    Exact,
    Regex,
}

#[derive(Clone, Debug)]
struct Term {
    key: Key,
    negated: bool,
    op: Op,
    value: String,
    regex: Option<regex::Regex>,
}

/// A parsed query. `mine` are the project's application IDs.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    terms: Vec<Term>,
    mine: Vec<String>,
}

impl Filter {
    pub fn parse(query: &str, mine: &[String]) -> Self {
        let mut terms = Vec::new();
        for token in tokens(query) {
            let (negated, token) = match token.strip_prefix('-') {
                Some(t) if t.contains(':') => (true, t.to_owned()),
                _ => (false, token),
            };
            let parsed = token.split_once(':').and_then(|(k, v)| {
                let (k, op) = if let Some(k) = k.strip_suffix('~') {
                    (k, Op::Regex)
                } else if let Some(k) = k.strip_suffix('=') {
                    (k, Op::Exact)
                } else {
                    (k, Op::Contains)
                };
                let key = match k {
                    "tag" => Key::Tag,
                    "package" => Key::Package,
                    "process" => Key::Process,
                    "message" => Key::Message,
                    "line" => Key::Line,
                    "level" => Key::Level,
                    "age" => Key::Age,
                    "is" => Key::Is,
                    _ => return None,
                };
                Some((key, op, unquote(v)))
            });
            let (key, op, value) = parsed.unwrap_or((Key::Line, Op::Contains, unquote(&token)));
            if value.is_empty() {
                continue;
            }
            let regex = (op == Op::Regex).then(|| regex::Regex::new(&value).ok()).flatten();
            terms.push(Term { key, negated, op, value, regex });
        }
        Self { terms, mine: mine.to_vec() }
    }

    /// `now`: the newest entry's time, for `age:`.
    pub fn matches(&self, e: &Entry, now: i64) -> bool {
        if e.system {
            // Separators follow the package terms only.
            return self.terms.iter().filter(|t| t.key == Key::Package).all(|t| self.term(t, e, now) != t.negated)
                || !self.terms.iter().any(|t| t.key == Key::Package);
        }
        let mut keys: Vec<Key> = Vec::new();
        for t in &self.terms {
            if !keys.contains(&t.key) {
                keys.push(t.key);
            }
        }
        keys.into_iter().all(|key| {
            let (neg, pos): (Vec<&Term>, Vec<&Term>) = self.terms.iter().filter(|t| t.key == key).partition(|t| t.negated);
            neg.iter().all(|t| !self.term(t, e, now)) && (pos.is_empty() || pos.iter().any(|t| self.term(t, e, now)))
        })
    }

    fn term(&self, t: &Term, e: &Entry, now: i64) -> bool {
        let text = |field: &str| match t.op {
            Op::Contains => field.to_lowercase().contains(&t.value.to_lowercase()),
            Op::Exact => field == t.value,
            Op::Regex => t.regex.as_ref().is_some_and(|r| r.is_match(field)),
        };
        match t.key {
            Key::Tag => text(&e.tag),
            Key::Package if t.value == "mine" && t.op == Op::Contains => self.mine.iter().any(|m| e.package == *m || e.package.starts_with(&format!("{m}:"))),
            Key::Package => text(&e.package),
            Key::Process => text(&e.package),
            Key::Message => text(&e.message),
            Key::Line => text(&format!("{} {} {} {} {}", e.tag, e.package, e.level.letter(), e.message, e.pid)),
            Key::Level => Level::parse(&t.value).is_some_and(|l| e.level >= l),
            Key::Age => parse_age(&t.value).is_some_and(|age| now - e.millis <= age.as_millis() as i64),
            Key::Is => match t.value.as_str() {
                "crash" => e.level >= Level::Error && (e.tag == "AndroidRuntime" && e.message.starts_with("FATAL EXCEPTION") || e.tag == "DEBUG" || e.tag == "libc"),
                "stacktrace" => e.message.trim_start().starts_with("at ") || e.message.starts_with("Caused by:"),
                _ => false,
            },
        }
    }
}

/// `30s`, `5m`, `2h`, `1d`.
fn parse_age(text: &str) -> Option<Duration> {
    let (n, unit) = text.split_at(text.len().checked_sub(1)?);
    let n: u64 = n.parse().ok()?;
    Some(Duration::from_secs(match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => return None,
    }))
}

/// Space-separated, with `"quoted values"` kept whole.
fn tokens(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in query.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(v: &str) -> String {
    v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v).to_owned()
}

/// A stack trace frame in a message: `at com.example.Foo.bar(Foo.kt:42)`
/// gives the package path ("com/example"), the file and the 1-based line.
pub fn stack_frame(message: &str) -> Option<(String, String, u32)> {
    let at = message.trim_start().strip_prefix("at ")?;
    let open = at.find('(')?;
    let close = at[open..].find(')')? + open;
    let (file, line) = at[open + 1..close].split_once(':')?;
    let line = line.parse().ok()?;
    let qualified = &at[..open];
    // com.example.Foo$Inner.bar → com/example
    let class_end = qualified.rfind('.')?;
    let class = &qualified[..class_end];
    let package = class.rsplit_once('.').map_or("", |(p, _)| p);
    Some((package.replace('.', "/"), file.to_owned(), line))
}

/// Android Studio's separator line when an app's process starts or ends.
pub fn separator(started: bool, pid: u32, package: &str, millis: i64) -> Entry {
    let what = if started { "STARTED" } else { "ENDED" };
    Entry {
        time: String::new(),
        millis,
        pid,
        tid: pid,
        level: Level::Info,
        tag: String::new(),
        package: package.to_owned(),
        message: format!("---------------------------- PROCESS {what} ({pid}) for package {package} ----------------------------"),
        system: true,
    }
}

/// Whether a process name is an app's (`com.example` or
/// `com.example:remote`), not a native daemon (`/system/bin/surfaceflinger`,
/// `[kworker/0:1]`, `zygote64`).
pub fn is_app_process(name: &str) -> bool {
    name.contains('.') && !name.starts_with('/') && !name.starts_with('[') && !name.contains(' ')
}

/// `ps -A -o PID,NAME` (or old `ps`, whose first column is the user):
/// the pid and the last column.
pub fn parse_processes(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let name = cols.last()?;
            let pid = cols.iter().take(2).find_map(|c| c.parse::<u32>().ok())?;
            (cols.len() >= 2).then(|| (pid, (*name).to_owned()))
        })
        .collect()
}

/// The device log over adb: the port the Logcat use case reads through.
pub trait LogSource: Send + Sync {
    /// Streams the device's log, one raw line at a time, until it ends or
    /// `stop` is set.
    fn stream(&self, serial: &str, on_line: &mut dyn FnMut(&str), stop: &std::sync::atomic::AtomicBool) -> Result<(), String>;
    /// The running processes: pid and name.
    fn processes(&self, serial: &str) -> Vec<(u32, String)>;
    /// `logcat -c`: clears the device's buffers.
    fn clear(&self, serial: &str) -> Result<(), String>;
}

/// A new Logcat tab's name, as Android Studio's `UniqueNameGenerator`
/// makes it: "Logcat", then "Logcat (2)", "Logcat (3)"… the first unused.
pub fn tab_name(existing: &[String]) -> String {
    if !existing.iter().any(|n| n == "Logcat") {
        return "Logcat".into();
    }
    (2..).map(|n| format!("Logcat ({n})")).find(|name| !existing.contains(name)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_tabs() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(super::tab_name(&[]), "Logcat");
        assert_eq!(super::tab_name(&names(&["Logcat"])), "Logcat (2)");
        assert_eq!(super::tab_name(&names(&["Logcat", "Logcat (3)"])), "Logcat (2)");
        assert_eq!(super::tab_name(&names(&["Logcat (2)"])), "Logcat");
    }

    use super::*;

    fn entry(level: Level, tag: &str, package: &str, message: &str) -> Entry {
        Entry { time: String::new(), millis: 1_000_000, pid: 1, tid: 1, level, tag: tag.into(), package: package.into(), message: message.into(), system: false }
    }

    #[test]
    fn parses_threadtime() {
        let e = parse_threadtime("2026-10-15 02:05:48.123  1234  1250 D OkHttp  : --> GET https://x").unwrap();
        assert_eq!((e.pid, e.tid, e.level, e.tag.as_str(), e.message.as_str()), (1234, 1250, Level::Debug, "OkHttp", "--> GET https://x"));
        assert_eq!(e.time, "2026-10-15 02:05:48.123");
        let e = parse_threadtime("10-15 02:05:48.123  1234  1250 E AndroidRuntime: FATAL EXCEPTION: main").unwrap();
        assert_eq!((e.tag.as_str(), e.message.as_str()), ("AndroidRuntime", "FATAL EXCEPTION: main"));
        assert!(parse_threadtime("--------- beginning of main").is_none());
    }

    #[test]
    fn filters_like_android_studio() {
        let mine = vec!["com.example".to_owned()];
        let app = entry(Level::Debug, "Net", "com.example", "loaded 3 items");
        let other = entry(Level::Error, "Net", "com.android.systemui", "boom");
        let f = |q: &str| Filter::parse(q, &mine);
        assert!(f("package:mine").matches(&app, 0) && !f("package:mine").matches(&other, 0));
        assert!(f("level:warn").matches(&other, 0) && !f("level:W").matches(&app, 0));
        assert!(f("tag:net").matches(&app, 0));
        assert!(!f("-tag:Net").matches(&app, 0));
        // Same key ORed, different keys ANDed.
        assert!(f("tag:Foo tag:Net").matches(&app, 0));
        assert!(!f("tag:Net level:error").matches(&app, 0));
        assert!(f("tag~:N.t message:\"3 items\"").matches(&app, 0));
        assert!(f("items").matches(&app, 0) && !f("items").matches(&other, 0));
        assert!(f("age:5m").matches(&app, 1_000_000 + 60_000) && !f("age:30s").matches(&app, 1_000_000 + 60_000));
        let crash = entry(Level::Error, "AndroidRuntime", "com.example", "FATAL EXCEPTION: main");
        assert!(f("is:crash").matches(&crash, 0) && !f("is:crash").matches(&other, 0));
    }

    #[test]
    fn reads_processes() {
        let ps = "  PID NAME\n    1 init\n  812 /system/bin/surfaceflinger\n 4321 com.example\n 4400 com.example:remote\n";
        let all = parse_processes(ps);
        assert_eq!(all.len(), 4);
        assert_eq!(all[2], (4321, "com.example".to_owned()));
        let apps: Vec<_> = all.iter().filter(|(_, n)| is_app_process(n)).map(|(p, _)| *p).collect();
        assert_eq!(apps, vec![4321, 4400]);
        // Old ps: USER PID PPID ... NAME
        assert_eq!(parse_processes("u0_a12 4321 600 1000 200 0 0 S com.example")[0], (4321, "com.example".to_owned()));
    }

    #[test]
    fn finds_stack_frames() {
        assert_eq!(stack_frame("\tat com.example.ui.MainActivity$1.run(MainActivity.kt:42)"), Some(("com/example/ui".into(), "MainActivity.kt".into(), 42)));
        assert_eq!(stack_frame("at android.os.Handler.dispatchMessage(Unknown Source:0)").map(|f| f.1), Some("Unknown Source".into()));
        assert!(stack_frame("Caused by: java.lang.NullPointerException").is_none());
    }
}

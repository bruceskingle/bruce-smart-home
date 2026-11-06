//! The log of important events, e.g. data fetched or requests which failed, as ERROR, WARNING or INFO entries, kept in a
//! file for each profile in the cache directory and shown in the log panel, see views/log_panel.rs. Verbose output (-v)
//! still goes to stdout and is not logged.
//!
//! More than one process can write to the same log at once, e.g. the app and a scheduled fetch (--fetch). Each entry
//! is written while holding a lock on <profile>.log.lock, by opening the file, writing the whole line and closing it
//! again, so that lines are never mixed up and every process writes to the current file. Entries from other than the
//! app are marked with where they came from, e.g. [fetch].
//!
//! Old entries are never removed from the file, instead the file is rolled, renamed to <profile>.log.old replacing the
//! previous one, as the profile's log settings say, see LogRetention: by default when its first entry is more than 24
//! hours old.
//!
//! The log panel reads the file as it grows, see LogReader, so it shows the entries of all the processes.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Info,
}

impl Level {
    fn as_str(&self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warning => "WARNING",
            Level::Info => "INFO",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "ERROR" => Some(Level::Error),
            "WARNING" => Some(Level::Warning),
            "INFO" => Some(Level::Info),
            _ => None,
        }
    }
}

/// The kind of process writing to the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Process {
    /// The app with its window, whose entries are not marked.
    App,
    /// A fetch of the latest data without a window, --fetch.
    Fetch,
}

impl Process {
    /// How its entries are marked in the log, e.g. [fetch].
    fn source(&self) -> Option<&'static str> {
        match self {
            Process::App => None,
            Process::Fetch => Some("fetch"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub time: OffsetDateTime,
    pub level: Level,
    /// Where the entry came from, e.g. "fetch", or None for the app.
    pub source: Option<String>,
    pub message: String,
}

impl LogEntry {
    /// A line of the file, e.g. 2026-10-26T06:40:20Z INFO [fetch] Fetched meterTelemetry
    fn line(&self) -> String {
        let time = self.time.format(&Rfc3339).unwrap_or_default();
        match &self.source {
            Some(source) => format!("{time} {} [{source}] {}", self.level.as_str(), self.message),
            None => format!("{time} {} {}", self.level.as_str(), self.message),
        }
    }

    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.splitn(3, ' ');
        let time = OffsetDateTime::parse(parts.next()?, &Rfc3339).ok()?;
        let level = Level::parse(parts.next()?)?;
        let rest = parts.next().unwrap_or_default();
        let (source, message) = match rest.strip_prefix('[').and_then(|rest| rest.split_once("] ")) {
            Some((source, message)) if !source.contains(' ') => (Some(source.to_string()), message),
            _ => (None, rest),
        };
        Some(LogEntry { time, level, source, message: message.to_string() })
    }
}

/// When the log file is rolled, renamed to <profile>.log.old, so that the log holds what it says and a little more.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "keep", rename_all = "camelCase")]
pub enum LogRetention {
    /// Each time the app is opened, so that the log has this run of the app, and the previous one in the .old file.
    /// Not when a fetch runs, so that the app's entries are kept.
    EachRun,
    /// When the first entry is more than so many hours old, so that the log and the .old file have at least the last
    /// so many hours, and at most twice that. lastRun was the default before there was more than one process.
    #[serde(alias = "lastRun")]
    Hours {
        #[serde(default = "default_hours")]
        hours: u32,
    },
    /// When it reaches half this many megabytes, so that the log and the .old file are at most this size.
    Size { megabytes: u32 },
}

pub const DEFAULT_HOURS: u32 = 24;

fn default_hours() -> u32 {
    DEFAULT_HOURS
}

impl Default for LogRetention {
    fn default() -> Self {
        LogRetention::Hours { hours: DEFAULT_HOURS }
    }
}

impl LogRetention {
    /// Whether a log whose first entry is at first, of size bytes before line is added, should be rolled first.
    fn roll_before(&self, first: Option<OffsetDateTime>, size: u64, line: &str, now: OffsetDateTime) -> bool {
        match *self {
            LogRetention::EachRun => false,
            LogRetention::Hours { hours } => first.is_some_and(|first| now - first > time::Duration::hours(hours as i64)),
            LogRetention::Size { megabytes } => size > 0 && size + line.len() as u64 + 1 > megabytes as u64 * 1024 * 1024 / 2,
        }
    }
}

/// The log settings of a profile, see Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSettings {
    #[serde(flatten)]
    pub retention: LogRetention,
    /// Whether the log panel is shown.
    #[serde(default = "shown")]
    pub show: bool,
}

fn shown() -> bool {
    true
}

impl Default for LogSettings {
    fn default() -> Self {
        LogSettings { retention: LogRetention::default(), show: true }
    }
}

struct Logger {
    profile_name: String,
    path: PathBuf,
    process: Process,
    retention: LogRetention,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);
/// The profiles whose log has been rolled by this process for LogRetention::EachRun, so that changing back to a
/// profile does not roll it again.
static ROLLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Entries which could not be written to the file, which the log panel shows after those in the file.
static UNWRITTEN: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());

/// The log file of a profile, in the cache directory.
pub fn log_path(profile_name: &str) -> anyhow::Result<PathBuf> {
    Ok(crate::data_home::logs_dir()?.join(format!("{profile_name}.log")))
}

/// The files of a profile's log, the log, the previous log it was rolled to and the lock file, which may not exist.
pub fn log_files(profile_name: &str) -> anyhow::Result<[PathBuf; 3]> {
    let path = log_path(profile_name)?;
    Ok([old_path(&path), lock_path(&path), path])
}

/// The file the log is renamed to when it is rolled.
fn old_path(path: &Path) -> PathBuf {
    let mut old = path.as_os_str().to_owned();
    old.push(".old");
    PathBuf::from(old)
}

fn lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// Locks the log against other processes until the returned file is dropped.
fn lock(path: &Path) -> std::io::Result<File> {
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(lock_path(path))?;
    lock.lock()?;
    Ok(lock)
}

/// The time of the first entry in the file, if it has one.
fn first_entry_time(path: &Path) -> Option<OffsetDateTime> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?).read_line(&mut line).ok()?;
    LogEntry::parse(line.trim_end()).map(|entry| entry.time)
}

/// Renames the log to the .old file, replacing it, while holding the lock.
fn roll(path: &Path) -> std::io::Result<()> {
    match fs::rename(path, old_path(path)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Adds the line to the file, first rolling it if retention says so.
fn append(path: &Path, retention: LogRetention, line: &str, now: OffsetDateTime) -> std::io::Result<()> {
    let _lock = lock(path)?;
    let size = fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0);
    if retention.roll_before(first_entry_time(path), size, line, now) {
        roll(path)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // All at once, so that it is one write to the file.
    file.write_all(format!("{line}\n").as_bytes())
}

/// Starts the log of the profile for this process, then logs that it started. With LogRetention::EachRun the app rolls
/// the log the first time. Called when the profile is chosen, so a change of profile is logged in the previous one.
pub fn start(profile_name: &str, settings: &LogSettings, process: Process) {
    let path = match log_path(profile_name) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Unable to find the log file: {error}");
            return;
        },
    };
    let previous = LOGGER.lock().unwrap().as_ref().map(|logger| logger.profile_name.clone());
    if previous.is_some_and(|previous| previous != profile_name) {
        crate::log_info!("Changed to profile {profile_name}");
    }

    let mut rolled = ROLLED.lock().unwrap();
    if process == Process::App && settings.retention == LogRetention::EachRun && !rolled.iter().any(|name| name == profile_name) {
        rolled.push(profile_name.to_string());
        if let Err(error) = lock(&path).and_then(|_lock| roll(&path)) {
            eprintln!("Unable to roll the log file {}: {error}", path.display());
        }
    }
    drop(rolled);

    *LOGGER.lock().unwrap() = Some(Logger {
        profile_name: profile_name.to_string(),
        path,
        process,
        retention: settings.retention,
    });
    UNWRITTEN.lock().unwrap().clear();
    log(Level::Info, format!("Started, Marco Sparko {}, profile {profile_name}", env!("CARGO_PKG_VERSION")));
}

/// Changes when the log is rolled, from the next entry.
pub fn set_retention(retention: LogRetention) {
    if let Some(logger) = LOGGER.lock().unwrap().as_mut() {
        logger.retention = retention;
    }
}

/// Adds an entry to the log, see the log_info, log_warning and log_error macros. Newlines in the message are replaced
/// so that each entry is one line of the file.
pub fn log(level: Level, message: String) {
    // To the second, which is all that is useful to read.
    let time = OffsetDateTime::now_utc();
    let time = time.replace_nanosecond(0).unwrap_or(time);
    let logger = LOGGER.lock().unwrap();
    let source = logger.as_ref().and_then(|logger| logger.process.source()).map(String::from);
    let entry = LogEntry { time, level, source, message: message.replace(['\r', '\n'], " ") };
    // Before the log is started, e.g. while the profile is read, entries are only printed.
    let Some(logger) = logger.as_ref() else {
        println!("{}", entry.line());
        return;
    };
    if let Err(error) = append(&logger.path, logger.retention, &entry.line(), time) {
        eprintln!("Unable to write to the log file {}: {error}", logger.path.display());
        UNWRITTEN.lock().unwrap().push(entry);
    }
}

/// The current log file, which changes when the profile does.
pub fn current_path() -> Option<PathBuf> {
    LOGGER.lock().unwrap().as_ref().map(|logger| logger.path.clone())
}

/// The entries of this process which could not be written to the file.
pub fn unwritten() -> Vec<LogEntry> {
    UNWRITTEN.lock().unwrap().clone()
}

/// Reads the entries of a log file as they are added to it, by any process. Whether it has changed is found from its
/// size and when it was created, which also works on Windows as each entry is written by opening and closing the file.
pub struct LogReader {
    path: PathBuf,
    /// How much has been read, up to the end of the last whole line.
    offset: u64,
    /// When the file read so far was created, to tell that it has been rolled, where that is known.
    created: Option<SystemTime>,
}

impl LogReader {
    /// A reader which starts at the beginning of the file.
    pub fn new(path: PathBuf) -> Self {
        LogReader { path, offset: 0, created: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The entries added since the last call, or None if there are none.
    pub fn read_new(&mut self) -> Option<Vec<LogEntry>> {
        let metadata = fs::metadata(&self.path).ok()?;
        let created = metadata.created().ok();
        let mut entries = Vec::new();
        let rolled = metadata.len() < self.offset || (self.offset > 0 && created.is_some() && self.created.is_some() && created != self.created);
        if rolled {
            // The rest of the file which was being read is now the .old file.
            if let Some((old_entries, _)) = read_lines(&old_path(&self.path), self.offset) {
                entries = old_entries;
            }
            self.offset = 0;
        }
        self.created = created;
        if metadata.len() > self.offset
            && let Some((new_entries, offset)) = read_lines(&self.path, self.offset)
        {
            entries.extend(new_entries);
            self.offset = offset;
        }
        (!entries.is_empty()).then_some(entries)
    }
}

/// The entries in the whole lines of the file from offset, and the offset after them.
fn read_lines(path: &Path, offset: u64) -> Option<(Vec<LogEntry>, u64)> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    // A line which is still being written is read next time.
    let whole = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1);
    let entries = String::from_utf8_lossy(&bytes[..whole]).lines().filter_map(LogEntry::parse).collect();
    Some((entries, offset + whole as u64))
}

/// Logs an INFO entry, e.g. log_info!("Fetched {count} readings").
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Info, format!($($arg)*)) };
}

/// Logs a WARNING entry, e.g. a request which is being retried.
#[macro_export]
macro_rules! log_warning {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Warning, format!($($arg)*)) };
}

/// Logs an ERROR entry, e.g. a request which failed.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::log($crate::logging::Level::Error, format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use time::macros::datetime;

    use super::{Level, LogEntry, LogReader, LogRetention, LogSettings, append, old_path};

    fn entry(time: time::OffsetDateTime, source: Option<&str>, message: &str) -> LogEntry {
        LogEntry { time, level: Level::Info, source: source.map(String::from), message: message.to_string() }
    }

    #[test]
    fn the_log_is_rolled_when_its_first_entry_is_too_old_or_it_is_too_big() {
        let now = datetime!(2026-10-27 12:00 UTC);
        let hours = LogRetention::Hours { hours: 24 };
        assert!(!hours.roll_before(None, 0, "line", now));
        assert!(!hours.roll_before(Some(datetime!(2026-10-26 13:00 UTC)), 100, "line", now));
        assert!(hours.roll_before(Some(datetime!(2026-10-26 11:00 UTC)), 100, "line", now));
        let size = LogRetention::Size { megabytes: 1 };
        assert!(!size.roll_before(None, 1000, "line", now));
        assert!(size.roll_before(None, 512 * 1024, "line", now));
        assert!(!LogRetention::EachRun.roll_before(Some(datetime!(2020-01-01 0:00 UTC)), 1 << 30, "line", now));
    }

    #[test]
    fn a_line_is_read_back_as_it_was_written() {
        let app = LogEntry { level: Level::Warning, ..entry(datetime!(2026-10-26 06:40:20 UTC), None, "Too many requests") };
        assert_eq!(app.line(), "2026-10-26T06:40:20Z WARNING Too many requests");
        assert_eq!(LogEntry::parse(&app.line()), Some(app));
        let fetch = entry(datetime!(2026-10-26 06:40:20 UTC), Some("fetch"), "[not a source] Started");
        assert_eq!(fetch.line(), "2026-10-26T06:40:20Z INFO [fetch] [not a source] Started");
        assert_eq!(LogEntry::parse(&fetch.line()), Some(fetch));
    }

    #[test]
    fn settings_are_stored_with_the_retention_flattened() {
        let settings = LogSettings { retention: LogRetention::Hours { hours: 48 }, show: false };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json, serde_json::json!({ "keep": "hours", "hours": 48, "show": false }));
        assert_eq!(serde_json::from_value::<LogSettings>(serde_json::json!({})).ok(), None);
        // What the first versions saved as the default is the default now.
        assert_eq!(serde_json::from_value::<LogSettings>(serde_json::json!({ "keep": "lastRun", "show": true })).unwrap(), LogSettings::default());
    }

    #[test]
    fn the_reader_follows_the_file_as_it_grows_and_is_rolled() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(old_path(&path));
        let size = LogRetention::Size { megabytes: 1 };
        let now = datetime!(2026-10-27 12:00 UTC);
        let line = |message: &str| entry(now, None, message).line();
        let messages = |entries: Option<Vec<LogEntry>>| entries.unwrap_or_default().into_iter().map(|entry| entry.message).collect::<Vec<_>>();

        let mut reader = LogReader::new(path.clone());
        assert!(reader.read_new().is_none());
        append(&path, size, &line("one"), now).unwrap();
        append(&path, size, &line("two"), now).unwrap();
        assert_eq!(messages(reader.read_new()), ["one", "two"]);
        assert!(reader.read_new().is_none());

        // Half a line is left for next time.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{}", line("three")).unwrap();
        assert!(reader.read_new().is_none());
        writeln!(file).unwrap();
        assert_eq!(messages(reader.read_new()), ["three"]);

        // Rolled by another process, after it added an entry the reader has not seen.
        append(&path, size, &line("four"), now).unwrap();
        super::roll(&path).unwrap();
        append(&path, size, &line("five"), now).unwrap();
        assert_eq!(messages(reader.read_new()), ["four", "five"]);

        fs::remove_dir_all(&dir).unwrap();
    }
}

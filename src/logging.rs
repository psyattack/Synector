//! Process-wide file logging for every Synector process (GUI, daemon, CLI, overlay windows).
//!
//! Events are written to `%LOCALAPPDATA%\Synector\logs\synector-YYYY-MM-DD.log`: one file per
//! local day, the newest [`RETENTION_DAYS`] days kept, and a level threshold taken from the
//! [`ENV_VAR`] environment variable (`off`, `error`, `warn`, `info` (default), `debug`, `trace`).
//! The file works like a ring: when a day reaches [`MAX_FILE_BYTES`] the oldest half is discarded
//! in place, a notice line explains the restart and the newest events stay at the end, so the name
//! never changes, no extra files appear and one chatty rule cannot produce an unbounded log.
//!
//! Disk logging can also be turned off entirely from Settings (`log_to_disk` in `settings.json`):
//! the logger then drops every event and stops opening the daily file, while the files already on
//! disk stay untouched — re-enabling resumes appending to the same day's file. [`init`] seeds the
//! flag, so no process creates a log file for a run that started with logging disabled; the
//! daemon and the GUI pass a runtime change to [`set_disk_logging_enabled`]. The environment
//! variable stays an independent override: `SYNECTOR_LOG=off` disables logging even when the
//! setting is on.
//!
//! The logger is dependency-free and must never take a process down: an I/O failure drops that
//! event, the first failure of a day is reported once on stderr and the open is not retried
//! until the next day (so a permanently unwritable directory cannot turn every event into a
//! failed `create_dir_all` + open), and a panic is recorded by [`install_panic_hook`]. [`init`]
//! returns an [`InitReport`] so the caller can surface a failure that happened before the first
//! event. All file access is serialized across Synector processes with a named mutex, so
//! compaction cannot discard an event another process appended in the meantime.
//!
//! Call sites log identifiers and outcomes only - script bodies, webhook payloads, rule JSON
//! and other user secrets never belong in a log line; the daemon routes action errors that may
//! embed script output through its `SensitiveLog` event instead.

use std::any::Any;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Local, NaiveDate};

/// Environment variable that selects the minimum level written to the log file.
pub const ENV_VAR: &str = "SYNECTOR_LOG";
/// Prefix of every daily log file.
pub const FILE_PREFIX: &str = "synector-";
/// Suffix of every daily log file.
pub const FILE_SUFFIX: &str = ".log";
/// Number of days of logs kept on disk.
pub const RETENTION_DAYS: i64 = 7;
/// Size at which the oldest half of the daily file is discarded to keep the log bounded.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Messages longer than this many characters are truncated before they are written.
const MAX_MESSAGE_CHARS: usize = 4096;

/// Severity of a log event; also the filtering threshold configured through [`ENV_VAR`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    /// Parses a level name, accepting common aliases; case and surrounding spaces are ignored.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "silent" => Some(Self::Off),
            "error" | "err" | "fatal" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" | "verbose" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    /// Name written into a log line and into `SYNECTOR_LOG` documentation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    /// Maps the levels used by the daemon event stream (`INFO`, `WARN`, `ERROR`, `TRIGGER`,
    /// `ACTION`, ...) onto file levels. Unknown names are treated as `INFO`.
    pub fn from_engine(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Self::Off,
            "error" | "err" | "fatal" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Info,
        }
    }

    /// Whether an event of `level` passes a logger whose threshold is `self`.
    fn allows(self, level: Self) -> bool {
        self != Self::Off && level != Self::Off && level <= self
    }
}

/// A logger bound to a directory. [`init`] installs the process-wide instance.
pub struct Logger {
    dir: PathBuf,
    max_level: Level,
    max_file_bytes: u64,
    /// Mirrors the `log_to_disk` setting: when off, events are dropped before they reach the file.
    disk_enabled: AtomicBool,
    open: Mutex<Option<OpenLog>>,
    /// First I/O failure of the day. Until the date changes the open is not attempted again.
    failure: Mutex<Option<LogFailure>>,
}

#[derive(Debug)]
struct OpenLog {
    date: String,
    size: u64,
    file: File,
}

#[derive(Debug)]
struct LogFailure {
    date: String,
    message: String,
}

/// Outcome of [`init`]: the first logging failure of the current day, if any. The message is
/// also emitted once on stderr; callers such as `main` can surface it (for example in the CLI
/// or a GUI toast) because a release build has no visible console.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitReport {
    pub error: Option<String>,
}

/// One process-wide named-mutex guard per log write. Every process that logs takes it, so an
/// append and a ring-buffer compaction can never interleave across the daemon, the GUI and the
/// CLI. If the mutex cannot be opened (for example an elevated daemon created it with a DACL
/// that a lower-integrity process cannot open) logging stays best-effort per process instead of
/// failing.
#[cfg(windows)]
mod cross_process {
    use std::sync::OnceLock;

    use windows::core::w;
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateMutexW, ReleaseMutex, WaitForSingleObject, INFINITE,
    };

    struct LogLock(HANDLE);

    // Safety: the handle is only passed to the kernel's wait/release calls, which serialize
    // access across threads and processes.
    unsafe impl Send for LogLock {}
    unsafe impl Sync for LogLock {}

    static LOG_LOCK: OnceLock<Option<LogLock>> = OnceLock::new();

    pub struct Guard(&'static LogLock);

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = ReleaseMutex(self.0 .0);
            }
        }
    }

    pub fn lock() -> Option<Guard> {
        let log_lock = LOG_LOCK
            .get_or_init(|| unsafe {
                // A handle is returned whether the named mutex was just created or already
                // existed; ownership is always taken explicitly below.
                CreateMutexW(None, false, w!(r"Local\Synector_Log_File"))
                    .ok()
                    .map(LogLock)
            })
            .as_ref()?;
        let wait = unsafe { WaitForSingleObject(log_lock.0, INFINITE) };
        (wait == WAIT_OBJECT_0).then_some(Guard(log_lock))
    }
}

impl Logger {
    /// Creates a logger that will write into `dir`; the directory is created on the first write
    /// or when [`init`] primes the daily file. Disk logging starts enabled.
    pub fn new(dir: impl Into<PathBuf>, max_level: Level) -> Self {
        Self {
            dir: dir.into(),
            max_level,
            max_file_bytes: MAX_FILE_BYTES,
            disk_enabled: AtomicBool::new(true),
            open: Mutex::new(None),
            failure: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_max_file_bytes(dir: impl Into<PathBuf>, max_level: Level, max_file_bytes: u64) -> Self {
        Self {
            dir: dir.into(),
            max_level,
            max_file_bytes,
            disk_enabled: AtomicBool::new(true),
            open: Mutex::new(None),
            failure: Mutex::new(None),
        }
    }

    /// Highest level this logger records.
    pub fn max_level(&self) -> Level {
        self.max_level
    }

    /// Turns writing to disk on or off. Disabling takes effect on the next event and leaves the
    /// files already written untouched; enabling resumes appending to the current day's file.
    pub fn set_disk_enabled(&self, enabled: bool) {
        self.disk_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Whether this logger currently writes events to disk.
    pub fn disk_enabled(&self) -> bool {
        self.disk_enabled.load(Ordering::Relaxed)
    }

    /// Writes one event when `level` passes the configured threshold.
    pub fn log(&self, level: Level, category: &str, message: &str) {
        if !self.disk_enabled() || !self.max_level.allows(level) {
            return;
        }
        self.write(Local::now(), level, category, message);
    }

    fn write(&self, now: DateTime<Local>, level: Level, category: &str, message: &str) {
        let date = now.format("%Y-%m-%d").to_string();
        if self.failed_on(&date) {
            return;
        }
        let line = format_line(&now, level, category, message);
        let line_bytes = line.len() as u64 + 1;

        // Serialize the whole append/compaction with the other Synector processes, so a
        // compaction cannot truncate an event appended between its read and `set_len`.
        let _file_lock = cross_process::lock();
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if open.as_ref().is_none_or(|current| current.date != date) {
            match self.open_day(&date) {
                Ok(current) => {
                    self.prune(now.date_naive());
                    *open = Some(current);
                    self.clear_failure();
                }
                Err(error) => {
                    self.record_failure(
                        &date,
                        &format!("cannot open the daily log file: {error}"),
                    );
                    return;
                }
            }
        }

        let full = open.as_ref().is_some_and(|current| {
            current.size > 0 && current.size.saturating_add(line_bytes) > self.max_file_bytes
        });
        if full {
            let mut compact_error = None;
            if let Some(current) = open.as_mut() {
                // Another process may have compacted the file already; trust the handle, not the
                // cached size, before dropping somebody else's fresh events.
                if let Ok(metadata) = current.file.metadata() {
                    current.size = metadata.len();
                }
                if current.size.saturating_add(line_bytes) > self.max_file_bytes {
                    if let Err(error) = self.compact_file(current) {
                        compact_error = Some(error);
                    }
                }
            }
            if let Some(error) = compact_error {
                self.record_failure(
                    &date,
                    &format!("cannot compact the daily log file: {error}"),
                );
                *open = None;
                return;
            }
        }

        let mut write_error = None;
        if let Some(current) = open.as_mut() {
            match writeln!(current.file, "{line}") {
                Ok(()) => current.size = current.size.saturating_add(line_bytes),
                Err(error) => write_error = Some(error),
            }
        }
        if let Some(error) = write_error {
            self.record_failure(&date, &format!("cannot write to the daily log file: {error}"));
            *open = None;
        }
    }

    fn failed_on(&self, date: &str) -> bool {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|failure| failure.date == date)
    }

    fn record_failure(&self, date: &str, message: &str) {
        let mut failure = self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failure.as_ref().is_some_and(|current| current.date == date) {
            return;
        }
        *failure = Some(LogFailure {
            date: date.to_string(),
            message: message.to_string(),
        });
        // The file log is what failed, so this is the one diagnostic that cannot go through it.
        eprintln!("Synector logging disabled for {date}: {message}");
    }

    fn clear_failure(&self) {
        *self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn failure_text(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|failure| failure.message.clone())
    }

    /// Opens today's file once at start-up so an unwritable logs directory is reported through
    /// [`init`] instead of staying silent until the first event.
    fn prime(&self) {
        let now = Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let _file_lock = cross_process::lock();
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.open_day(&date) {
            Ok(current) => {
                self.prune(now.date_naive());
                *open = Some(current);
                self.clear_failure();
            }
            Err(error) => self.record_failure(
                &date,
                &format!("cannot open {}: {error}", self.dir.display()),
            ),
        }
    }

    /// Opens today's file for appending, discarding the oldest events when an earlier run left it
    /// at the size cap.
    fn open_day(&self, date: &str) -> std::io::Result<OpenLog> {
        fs::create_dir_all(&self.dir)?;
        let mut open = OpenLog {
            date: date.to_string(),
            size: 0,
            file: OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path_for(date))?,
        };
        open.size = open
            .file
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if open.size >= self.max_file_bytes {
            self.compact_file(&mut open)?;
        }
        Ok(open)
    }

    /// Ring-buffer compaction: keeps the newest half of the file, drops the oldest events and puts
    /// a notice line in front.
    ///
    /// The file is rewritten in place instead of renamed: the append handles held by the daemon,
    /// the GUI and CLI keep pointing at the same file and continue appending after the new end.
    fn compact_file(&self, open: &mut OpenLog) -> std::io::Result<()> {
        let path = self.path_for(&open.date);
        let keep_bytes = self.max_file_bytes / 2;
        let start = open.size.saturating_sub(keep_bytes);
        // A line is at most `MAX_MESSAGE_CHARS` plus its prefix, so searching this far back is
        // enough to find the first whole line at or after the start of the tail.
        let margin = MAX_MESSAGE_CHARS as u64 + 256;
        let read_from = start.saturating_sub(margin);

        let mut tail = Vec::new();
        {
            let mut reader = File::open(&path)?;
            let mut buffer = Vec::new();
            reader.seek(SeekFrom::Start(read_from))?;
            reader.read_to_end(&mut buffer)?;
            let anchor = (start - read_from) as usize;
            if anchor < buffer.len() {
                // Keep only complete lines; a cut-off first line is dropped with the old data.
                if let Some(offset) = buffer[anchor..].iter().position(|byte| *byte == b'\n') {
                    tail.extend_from_slice(&buffer[anchor + offset + 1..]);
                }
            }
        }

        let notice = format_line(
            &Local::now(),
            Level::Info,
            "LOGGING",
            &format!(
                "Log file reached the {} byte limit; the oldest events were discarded to keep the newest",
                self.max_file_bytes
            ),
        );
        let new_size = (notice.len() + 1 + tail.len()) as u64;
        {
            let mut writer = OpenOptions::new().write(true).open(&path)?;
            writer.seek(SeekFrom::Start(0))?;
            writer.write_all(notice.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.write_all(&tail)?;
            writer.set_len(new_size)?;
        }
        open.size = new_size;
        Ok(())
    }

    fn path_for(&self, date: &str) -> PathBuf {
        self.dir.join(log_file_name(date))
    }

    /// Removes daily files older than [`RETENTION_DAYS`]; anything else in the directory is left alone.
    fn prune(&self, today: NaiveDate) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_date) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(date_of_log_file)
            else {
                continue;
            };
            if (today - file_date).num_days() >= RETENTION_DAYS {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// Installs the process-wide logger, reading the threshold from [`ENV_VAR`] and the
/// `log_to_disk` setting from `settings.json`. Later calls are ignored, so every process can call
/// it defensively.
///
/// The first call primes today's file unless logging is off (by environment or by setting): an
/// unwritable logs directory is reported once on stderr and returned in the [`InitReport`], and
/// the open is not retried until the next day.
pub fn init() -> InitReport {
    let (max_level, problem) = match std::env::var(ENV_VAR) {
        Ok(raw) if !raw.trim().is_empty() => match Level::parse(&raw) {
            Some(level) => (level, None),
            None => (
                Level::Info,
                Some(format!(
                    "unknown {ENV_VAR} value \"{}\"; expected off, error, warn, info, debug or trace - using info",
                    raw.trim()
                )),
            ),
        },
        _ => (Level::Info, None),
    };

    if GLOBAL.get().is_none() {
        // Each process reads the setting itself, so the daemon, the GUI, the CLI and an overlay
        // window all agree without one having to notify the others at start-up. A missing or
        // unreadable section keeps disk logging on, which matches the default.
        let disk_enabled = crate::config::Settings::read().log_to_disk;
        let logger = Logger::new(log_dir(), max_level);
        logger.set_disk_enabled(disk_enabled);
        if max_level != Level::Off && disk_enabled {
            logger.prime();
        }
        let _ = GLOBAL.set(logger);
    }
    if let Some(problem) = problem {
        warn("LOGGING", &problem);
    }

    InitReport {
        error: GLOBAL.get().and_then(Logger::failure_text),
    }
}

/// Installs a panic hook that records the thread, source location and message before the default
/// hook runs. With `RUST_BACKTRACE` enabled the backtrace is recorded as well.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").to_string();
        let location = info.location().map_or_else(
            || "unknown location".to_string(),
            |location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            },
        );
        let payload = panic_payload(info.payload());

        init();
        error(
            "PANIC",
            &format!("thread '{thread_name}' panicked at {location}: {payload}"),
        );
        if backtrace_requested() {
            let backtrace = std::backtrace::Backtrace::force_capture();
            for frame in backtrace.to_string().lines() {
                error("PANIC", &format!("backtrace: {}", frame.trim()));
            }
        }
        previous(info);
    }));
}

/// Directory that holds the daily log files.
pub fn log_dir() -> PathBuf {
    crate::config::AppConfig::logs_dir()
}

/// Path of the file the current local day is written to.
pub fn current_log_path() -> PathBuf {
    log_dir().join(log_file_name(&Local::now().format("%Y-%m-%d").to_string()))
}

/// Effective threshold of the process-wide logger, or [`Level::Off`] before [`init`].
pub fn level() -> Level {
    GLOBAL.get().map_or(Level::Off, Logger::max_level)
}

/// Turns disk logging of the process-wide logger on or off. It applies the `log_to_disk` setting
/// to this process at runtime: events stop reaching the file immediately, and the files already
/// on disk stay untouched. A no-op before [`init`].
pub fn set_disk_logging_enabled(enabled: bool) {
    if let Some(logger) = GLOBAL.get() {
        logger.set_disk_enabled(enabled);
    }
}

/// Writes one event through the process-wide logger; a no-op before [`init`].
pub fn log(level: Level, category: &str, message: &str) {
    if let Some(logger) = GLOBAL.get() {
        logger.log(level, category, message);
    }
}

/// Writes an event, mapping a daemon event-stream level such as `TRIGGER` or `ACTION` to a file
/// level. See [`Level::from_engine`].
pub fn engine_log(level: &str, category: &str, message: &str) {
    let level = Level::from_engine(level);
    if level != Level::Off {
        log(level, category, message);
    }
}

/// Records an `ERROR` event.
pub fn error(category: &str, message: &str) {
    log(Level::Error, category, message);
}

/// Records a `WARN` event.
pub fn warn(category: &str, message: &str) {
    log(Level::Warn, category, message);
}

/// Records an `INFO` event.
pub fn info(category: &str, message: &str) {
    log(Level::Info, category, message);
}

/// Records a `DEBUG` event.
pub fn debug(category: &str, message: &str) {
    log(Level::Debug, category, message);
}

/// Records a `TRACE` event.
pub fn trace(category: &str, message: &str) {
    log(Level::Trace, category, message);
}

fn log_file_name(date: &str) -> String {
    format!("{FILE_PREFIX}{date}{FILE_SUFFIX}")
}

/// Parses the date out of a daily log file name (`synector-YYYY-MM-DD.log`).
fn date_of_log_file(name: &str) -> Option<NaiveDate> {
    let date = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// Renders one log line: local timestamp with milliseconds, level, category and message.
fn format_line(now: &DateTime<Local>, level: Level, category: &str, message: &str) -> String {
    format!(
        "{} {:<5} [{}] {}",
        now.format("%Y-%m-%d %H:%M:%S%.3f"),
        level.as_str(),
        one_line(category),
        one_line(message),
    )
}

/// Keeps every event on a single line and bounds its length so a stray payload cannot bloat a file.
fn one_line(text: &str) -> String {
    let (body, truncated) = match text.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((index, _)) => (&text[..index], true),
        None => (text, false),
    };
    let mut clean = String::with_capacity(body.len());
    for character in body.chars() {
        match character {
            '\n' => clean.push_str("\\n"),
            '\r' => clean.push_str("\\r"),
            '\t' => clean.push_str("\\t"),
            character if character.is_control() => clean.push('?'),
            character => clean.push(character),
        }
    }
    if truncated {
        clean.push_str("...(truncated)");
    }
    clean
}

fn panic_payload(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-text panic payload".to_string()
    }
}

fn backtrace_requested() -> bool {
    match std::env::var("RUST_BACKTRACE") {
        Ok(value) => {
            let value = value.trim();
            !value.is_empty() && value != "0"
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use chrono::TimeZone;

    fn temp_logger(tag: &str, max_level: Level) -> (Logger, PathBuf) {
        temp_logger_with_max_bytes(tag, max_level, MAX_FILE_BYTES)
    }

    fn temp_logger_with_max_bytes(
        tag: &str,
        max_level: Level,
        max_file_bytes: u64,
    ) -> (Logger, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "synector_logging_{}_{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        (Logger::with_max_file_bytes(&dir, max_level, max_file_bytes), dir)
    }

    fn read_logs(dir: &Path) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = fs::read_dir(dir)
            .expect("log directory exists")
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let content = fs::read_to_string(entry.path()).expect("log file is readable");
                (name, content)
            })
            .collect();
        files.sort();
        files
    }

    fn at(year: i32, month: u32, day: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, 12, 0, 0)
            .single()
            .expect("valid local instant")
    }

    #[test]
    fn test_level_parsing() {
        assert_eq!(Level::parse("off"), Some(Level::Off));
        assert_eq!(Level::parse(" OFF "), Some(Level::Off));
        assert_eq!(Level::parse("error"), Some(Level::Error));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("Warn"), Some(Level::Warn));
        assert_eq!(Level::parse("info"), Some(Level::Info));
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("verbose"), Some(Level::Debug));
        assert_eq!(Level::parse("trace"), Some(Level::Trace));
        assert_eq!(Level::parse("loud"), None);
        assert_eq!(Level::parse(""), None);
    }

    #[test]
    fn test_engine_level_mapping() {
        assert_eq!(Level::from_engine("ERROR"), Level::Error);
        assert_eq!(Level::from_engine("WARN"), Level::Warn);
        assert_eq!(Level::from_engine("INFO"), Level::Info);
        assert_eq!(Level::from_engine("TRIGGER"), Level::Info);
        assert_eq!(Level::from_engine("ACTION"), Level::Info);
        assert_eq!(Level::from_engine("DEBUG"), Level::Debug);
        assert_eq!(Level::from_engine("off"), Level::Off);
    }

    #[test]
    fn test_level_threshold_is_inclusive() {
        assert!(Level::Warn.allows(Level::Error));
        assert!(Level::Warn.allows(Level::Warn));
        assert!(!Level::Warn.allows(Level::Info));
        assert!(!Level::Off.allows(Level::Error));
        assert!(!Level::Trace.allows(Level::Off));
    }

    #[test]
    fn test_line_format() {
        let now = at(2026, 3, 4);
        let line = format_line(&now, Level::Info, "DAEMON", "service started");
        assert_eq!(line, "2026-03-04 12:00:00.000 INFO  [DAEMON] service started");
    }

    #[test]
    fn test_one_line_escapes_control_characters_and_truncates() {
        assert_eq!(one_line("first\nsecond\tend"), "first\\nsecond\\tend");
        assert_eq!(one_line("carriage\rreturn"), "carriage\\rreturn");

        let long = "x".repeat(MAX_MESSAGE_CHARS + 500);
        let clean = one_line(&long);
        assert!(clean.ends_with("...(truncated)"), "{clean}");
        assert_eq!(clean.matches('x').count(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn test_writes_daily_file() {
        let (logger, dir) = temp_logger("daily", Level::Info);
        logger.log(Level::Info, "DAEMON", "hello");
        logger.log(Level::Warn, "RULE", "careful");

        let files = read_logs(&dir);
        assert_eq!(files.len(), 1);
        let (name, content) = &files[0];
        assert!(name.starts_with(FILE_PREFIX), "{name}");
        assert!(name.ends_with(FILE_SUFFIX), "{name}");
        assert!(content.contains("INFO  [DAEMON] hello"), "{content}");
        assert!(content.contains("WARN  [RULE] careful"), "{content}");
        assert_eq!(content.lines().count(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_level_filters_events_and_creates_no_file() {
        let (logger, dir) = temp_logger("filtered", Level::Warn);
        logger.log(Level::Info, "DAEMON", "too quiet");
        assert!(!dir.exists(), "logging below the threshold must not create files");

        logger.log(Level::Error, "DAEMON", "loud enough");
        let files = read_logs(&dir);
        assert_eq!(files.len(), 1);
        assert!(files[0].1.contains("ERROR [DAEMON] loud enough"));
        assert!(!files[0].1.contains("too quiet"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_disabled_logger_writes_nothing() {
        let (logger, dir) = temp_logger("disabled", Level::Off);
        logger.log(Level::Error, "PANIC", "never written");
        assert!(!dir.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_disk_logging_toggle_stops_writes_and_keeps_existing_events() {
        let (logger, dir) = temp_logger("disk-toggle", Level::Info);
        logger.log(Level::Info, "DAEMON", "written before disabling");

        logger.set_disk_enabled(false);
        assert!(!logger.disk_enabled());
        logger.log(Level::Info, "DAEMON", "dropped while disabled");

        let files = read_logs(&dir);
        assert_eq!(files.len(), 1, "disabling must not remove the current file");
        let (name, content) = &files[0];
        assert!(content.contains("written before disabling"), "{content}");
        assert!(!content.contains("dropped while disabled"), "{content}");

        logger.set_disk_enabled(true);
        logger.log(Level::Info, "DAEMON", "written after enabling");
        let content = fs::read_to_string(dir.join(name)).expect("read the daily file");
        assert!(content.contains("written after enabling"), "{content}");
        assert_eq!(content.lines().count(), 2, "{content}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_disk_logging_disabled_at_start_creates_no_directory() {
        let (logger, dir) = temp_logger("disk-off", Level::Info);
        logger.set_disk_enabled(false);
        logger.log(Level::Error, "PANIC", "never written");
        assert!(
            !dir.exists(),
            "a run started with disk logging off must not create the logs directory"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_open_failure_is_reported_once_and_retried_the_next_day() {
        let blocker = std::env::temp_dir().join(format!(
            "synector_logging_blocker_{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&blocker);
        let _ = fs::remove_dir_all(&blocker);
        fs::write(&blocker, "not a directory").expect("create the obstacle file");

        // The log directory path is occupied by a file, so every open fails.
        let logger = Logger::new(&blocker, Level::Info);
        logger.write(at(2026, 3, 4), Level::Info, "DAEMON", "lost event");
        assert!(
            logger.failure_text().is_some(),
            "the first open failure must be recorded"
        );
        // A second event on the same day must not re-attempt the open (and must not panic).
        logger.write(at(2026, 3, 4), Level::Info, "DAEMON", "another lost event");

        fs::remove_file(&blocker).expect("remove the obstacle file");
        fs::create_dir_all(&blocker).expect("create the log directory");
        logger.write(at(2026, 3, 5), Level::Info, "DAEMON", "recovered");
        assert!(
            logger.failure_text().is_none(),
            "the next day must retry and clear the failure"
        );

        let files = read_logs(&blocker);
        assert_eq!(files.len(), 1);
        assert!(files[0].1.contains("recovered"));
        assert!(!files[0].1.contains("lost event"));
        let _ = fs::remove_dir_all(&blocker);
    }

    #[test]
    fn test_rotation_switches_file_by_day() {
        let (logger, dir) = temp_logger("rotation", Level::Info);
        logger.write(at(2026, 3, 4), Level::Info, "DAEMON", "day one");
        logger.write(at(2026, 3, 5), Level::Info, "DAEMON", "day two");
        logger.write(at(2026, 3, 5), Level::Info, "DAEMON", "again day two");

        let files = read_logs(&dir);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "synector-2026-03-04.log");
        assert_eq!(files[1].0, "synector-2026-03-05.log");
        assert_eq!(files[0].1.lines().count(), 1);
        assert_eq!(files[1].1.lines().count(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_keeps_retention_window_and_foreign_files() {
        let (logger, dir) = temp_logger("prune", Level::Info);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "synector-2026-01-03.log",
            "synector-2026-01-04.log",
            "synector-2026-01-10.log",
            "synector-2026-01-10.2.log",
            "synector-not-a-date.log",
            "notes.txt",
        ] {
            fs::write(dir.join(name), name).unwrap();
        }

        logger.prune(NaiveDate::from_ymd_opt(2026, 1, 10).unwrap());

        let names: Vec<String> = read_logs(&dir).into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                "notes.txt".to_string(),
                "synector-2026-01-04.log".to_string(),
                "synector-2026-01-10.2.log".to_string(),
                "synector-2026-01-10.log".to_string(),
                "synector-not-a-date.log".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_log_file_name_parsing() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 4);
        assert_eq!(date_of_log_file("synector-2026-03-04.log"), date);
        assert_eq!(date_of_log_file("synector-not-a-date.log"), None);
        assert_eq!(date_of_log_file("notes-2026-03-04.log"), None);
        assert_eq!(date_of_log_file("synector-2026-03-04.txt"), None);
        // Numbered leftovers from an older build are treated as foreign files and left alone.
        assert_eq!(date_of_log_file("synector-2026-03-04.2.log"), None);
    }

    #[test]
    fn test_size_limit_keeps_only_the_newest_events() {
        const LIMIT: u64 = 640;
        let (logger, dir) = temp_logger_with_max_bytes("ring", Level::Info, LIMIT);
        for index in 0..40 {
            logger.log(Level::Info, "RING", &format!("message number {index}"));
        }

        let files = read_logs(&dir);
        assert_eq!(files.len(), 1, "a single daily file is expected");
        let (name, content) = &files[0];
        assert_eq!(
            name,
            &format!("{FILE_PREFIX}{}{FILE_SUFFIX}", Local::now().format("%Y-%m-%d"))
        );
        assert!(content.len() as u64 <= LIMIT, "file grew past the limit: {}", content.len());
        assert!(
            content.contains("the oldest events were discarded"),
            "compaction notice is missing: {content}"
        );
        assert!(
            content.lines().last().is_some_and(|line| line.contains("message number 39")),
            "the newest event is missing: {content}"
        );
        assert!(
            !content.contains("message number 0"),
            "the oldest events must be discarded: {content}"
        );
        // Every retained line starts with a full timestamp, so no half-overwritten line survived.
        for line in content.lines() {
            assert!(
                line.len() > 10 && line.as_bytes()[4] == b'-',
                "partial line in the ring buffer: {line}"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reopening_an_oversized_file_discards_old_data() {
        const LIMIT: u64 = 640;
        let (logger, dir) = temp_logger_with_max_bytes("refull", Level::Info, LIMIT);
        logger.log(Level::Info, "DAEMON", "stale event");
        drop(logger);

        let path = dir.join(format!(
            "{FILE_PREFIX}{}{FILE_SUFFIX}",
            Local::now().format("%Y-%m-%d")
        ));
        fs::write(&path, "x".repeat(LIMIT as usize + 100)).unwrap();

        let second = Logger::with_max_file_bytes(&dir, Level::Info, LIMIT);
        second.log(Level::Info, "DAEMON", "after reopen");
        drop(second);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("the oldest events were discarded"), "{content}");
        assert!(content.contains("after reopen"), "{content}");
        assert!(!content.contains("stale event"), "{content}");
        assert!(!content.contains("xxxx"), "unreadable old data must be dropped: {content}");
        assert!(content.len() as u64 <= LIMIT, "{content}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reopen_appends_to_existing_daily_file() {
        let (logger, dir) = temp_logger("reopen", Level::Info);
        logger.log(Level::Info, "DAEMON", "first process");
        drop(logger);

        let second = Logger::new(&dir, Level::Info);
        second.log(Level::Info, "DAEMON", "second process");
        drop(second);

        let files = read_logs(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1.lines().count(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_written_file_path_matches_current_day() {
        let path = current_log_path();
        assert_eq!(
            path.parent().and_then(|parent| parent.file_name()),
            Some(std::ffi::OsStr::new("logs"))
        );
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(FILE_PREFIX), "{name}");
        assert!(name.ends_with(FILE_SUFFIX), "{name}");
        let date_part = name
            .trim_start_matches(FILE_PREFIX)
            .trim_end_matches(FILE_SUFFIX);
        let parsed = NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .expect("file name carries a date");
        // Tolerate the unlikely case of the test crossing local midnight.
        assert!((Local::now().date_naive() - parsed).num_days().abs() <= 1);
    }

    #[test]
    fn test_panic_payload_messages() {
        let text: &(dyn Any + Send) = &"boom";
        assert_eq!(panic_payload(text), "boom");

        let owned: &(dyn Any + Send) = &String::from("owned boom");
        assert_eq!(panic_payload(owned), "owned boom");

        let other: &(dyn Any + Send) = &42u32;
        assert_eq!(panic_payload(other), "non-text panic payload");
    }
}

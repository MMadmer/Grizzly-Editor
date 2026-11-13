//! The editor's log.
//!
//! A [`Log`] is a handle anything may clone and write through from any thread: a record goes
//! down a channel to one writer thread that owns the file, and into a ring the Output Log reads.
//! Writing never waits on the disk.
//!
//! The file keeps the engine's line marks, which tools and people already read: `! ` an error,
//! `~ ` a warning, `* ` information (a report or a detail), no mark for plain text.
//!
//! A **report** is a line the author may want to read now: an error, a warning, or an
//! information line that says what the editor did. A **detail** is information of the editor's
//! machinery - a start-up profile, a build's stage times - kept in the file and the Output Log but
//! never the status bar's line. [`Log::last_report`] is the newest report, kept as a field the
//! writer updates, so a reader never walks the ring.

use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use gz_base::{Rev, RevCounter};
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Plain,
    /// Information of the editor's machinery: profiles, stage times. Marked as information in the
    /// file, never a report.
    Detail,
    Info,
    Warning,
    Error,
}

impl Level {
    #[must_use]
    pub fn mark(self) -> &'static str {
        match self {
            Level::Plain => "",
            Level::Detail | Level::Info => "* ",
            Level::Warning => "~ ",
            Level::Error => "! ",
        }
    }

    /// A line of this level is a report: what the status bar may show.
    #[must_use]
    pub fn reports(self) -> bool {
        matches!(self, Level::Info | Level::Warning | Level::Error)
    }
}

/// A line as the log holds it: its level's mark in front (`~ [nq] ...`), unless the text
/// carries one already.
#[must_use]
pub fn marked(level: Level, text: &str) -> String {
    let mark = level.mark();
    if mark.is_empty() || ["! ", "~ ", "* "].iter().any(|m| text.starts_with(m)) {
        text.to_owned()
    } else {
        format!("{mark}{text}")
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    /// Position in the log since the start, for a reader that asks "what is new".
    pub seq: u64,
    pub time_ms: u64,
    pub level: Level,
    pub text: String,
}

enum Msg {
    Line(Arc<Record>),
    Flush(Sender<()>),
    Stop,
}

struct Inner {
    tx: Option<Sender<Msg>>,
    ring: Mutex<Ring>,
    errors: AtomicU32,
    warnings: AtomicU32,
    rev: RevCounter,
    /// The newest report, set as it is written.
    last_report: Mutex<Option<Arc<Record>>>,
}

struct Ring {
    lines: VecDeque<Arc<Record>>,
    next_seq: u64,
    limit: usize,
}

/// The handle to write through. Cheap to clone.
#[derive(Clone)]
pub struct Log(Arc<Inner>);

/// Stops the writer and flushes the file when dropped. Lives in `main`.
pub struct LogGuard {
    log: Log,
    writer: Option<JoinHandle<()>>,
}

impl Log {
    /// Starts the log. With a `file`, a writer thread owns it; `echo` also prints to stderr.
    pub fn start(file: Option<&Path>, echo: bool) -> io::Result<(Log, LogGuard)> {
        let out = match file {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                Some(BufWriter::with_capacity(64 * 1024, File::create(path)?))
            }
            None => None,
        };
        let (tx, rx) = unbounded();
        let log = Log(Arc::new(Inner {
            tx: Some(tx),
            ring: Mutex::new(Ring { lines: VecDeque::new(), next_seq: 0, limit: 20_000 }),
            errors: AtomicU32::new(0),
            warnings: AtomicU32::new(0),
            rev: RevCounter::new(),
            last_report: Mutex::new(None),
        }));
        let writer = std::thread::Builder::new()
            .name("gz-log".to_owned())
            .spawn(move || write_loop(&rx, out, echo))?;
        Ok((log.clone(), LogGuard { log, writer: Some(writer) }))
    }

    /// A log that keeps its ring and writes nowhere: tests and tools.
    #[must_use]
    pub fn memory() -> Log {
        Log(Arc::new(Inner {
            tx: None,
            ring: Mutex::new(Ring { lines: VecDeque::new(), next_seq: 0, limit: 20_000 }),
            errors: AtomicU32::new(0),
            warnings: AtomicU32::new(0),
            rev: RevCounter::new(),
            last_report: Mutex::new(None),
        }))
    }

    pub fn write(&self, level: Level, text: impl Into<String>) {
        let text = text.into();
        match level {
            Level::Error => {
                self.0.errors.fetch_add(1, Ordering::Relaxed);
            }
            Level::Warning => {
                self.0.warnings.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        let record = {
            let mut ring = self.0.ring.lock();
            let record = Arc::new(Record {
                seq: ring.next_seq,
                time_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
                level,
                text,
            });
            ring.next_seq += 1;
            ring.lines.push_back(record.clone());
            if ring.lines.len() > ring.limit {
                ring.lines.pop_front();
            }
            record
        };
        if level.reports() {
            *self.0.last_report.lock() = Some(record.clone());
        }
        self.0.rev.bump();
        if let Some(tx) = &self.0.tx {
            let _ = tx.send(Msg::Line(record));
        }
    }

    pub fn plain(&self, text: impl fmt::Display) {
        self.write(Level::Plain, text.to_string());
    }

    pub fn info(&self, text: impl fmt::Display) {
        self.write(Level::Info, text.to_string());
    }

    /// A line of the editor's machinery (a profile, stage times): in the file and the Output
    /// Log, never the status bar's line.
    pub fn detail(&self, text: impl fmt::Display) {
        self.write(Level::Detail, text.to_string());
    }

    pub fn warn(&self, text: impl fmt::Display) {
        self.write(Level::Warning, text.to_string());
    }

    pub fn error(&self, text: impl fmt::Display) {
        self.write(Level::Error, text.to_string());
    }

    /// Waits until everything written so far is on disk. For a crash handler and for `main`.
    pub fn flush(&self) {
        if let Some(tx) = &self.0.tx {
            let (done, wait) = bounded(1);
            if tx.send(Msg::Flush(done)).is_ok() {
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }
        }
    }

    /// Moves with every line.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.0.rev.get()
    }

    /// Errors and warnings since the last [`Self::reset_counts`] (since the project opened).
    #[must_use]
    pub fn counts(&self) -> (u32, u32) {
        (self.0.errors.load(Ordering::Relaxed), self.0.warnings.load(Ordering::Relaxed))
    }

    pub fn reset_counts(&self) {
        self.0.errors.store(0, Ordering::Relaxed);
        self.0.warnings.store(0, Ordering::Relaxed);
        self.0.rev.bump();
    }

    /// The lines from `seq` on that the ring still holds.
    #[must_use]
    pub fn since(&self, seq: u64) -> Vec<Arc<Record>> {
        let ring = self.0.ring.lock();
        let first = ring.lines.front().map_or(ring.next_seq, |r| r.seq);
        let skip = seq.saturating_sub(first) as usize;
        ring.lines.iter().skip(skip).cloned().collect()
    }

    /// The last `n` lines.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<Arc<Record>> {
        let ring = self.0.ring.lock();
        let skip = ring.lines.len().saturating_sub(n);
        ring.lines.iter().skip(skip).cloned().collect()
    }

    /// The newest report: what the status bar shows. A field the writer sets; nothing walks the
    /// ring for it.
    #[must_use]
    pub fn last_report(&self) -> Option<Arc<Record>> {
        self.0.last_report.lock().clone()
    }

    pub fn clear(&self) {
        self.0.ring.lock().lines.clear();
        self.0.rev.bump();
    }
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        if let Some(tx) = &self.log.0.tx {
            let _ = tx.send(Msg::Stop);
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn write_loop(rx: &Receiver<Msg>, mut out: Option<BufWriter<File>>, echo: bool) {
    // the echo ends with its reader: a stream that refused a line is not written to again
    let echo = std::cell::Cell::new(echo);
    let put = |record: &Record, out: &mut Option<BufWriter<File>>| {
        if let Some(out) = out {
            let _ = out.write_all(record.level.mark().as_bytes());
            let _ = out.write_all(record.text.as_bytes());
            let _ = out.write_all(b"\r\n");
        }
        if echo.get() && writeln!(std::io::stderr().lock(), "{}{}", record.level.mark(), record.text).is_err()
        {
            echo.set(false);
        }
    };
    loop {
        // block for the first message, then drain what queued up, then flush once
        let Ok(first) = rx.recv() else { break };
        let mut stop = false;
        let mut waiters: Vec<Sender<()>> = Vec::new();
        let mut handle = |msg: Msg, out: &mut Option<BufWriter<File>>| match msg {
            Msg::Line(r) => put(&r, out),
            Msg::Flush(done) => waiters.push(done),
            Msg::Stop => stop = true,
        };
        handle(first, &mut out);
        while let Ok(more) = rx.try_recv() {
            handle(more, &mut out);
        }
        if let Some(out) = &mut out {
            let _ = out.flush();
        }
        for w in waiters {
            let _ = w.send(());
        }
        if stop {
            break;
        }
    }
}

/// `info!(log, "built {} files", n)`.
#[macro_export]
macro_rules! info {
    ($log:expr, $($arg:tt)*) => { $log.write($crate::Level::Info, format!($($arg)*)) };
}
/// `warn!(log, "...")`.
#[macro_export]
macro_rules! warn {
    ($log:expr, $($arg:tt)*) => { $log.write($crate::Level::Warning, format!($($arg)*)) };
}
/// `error!(log, "...")`.
#[macro_export]
macro_rules! error {
    ($log:expr, $($arg:tt)*) => { $log.write($crate::Level::Error, format!($($arg)*)) };
}
/// `detail!(log, "...")`.
#[macro_export]
macro_rules! detail {
    ($log:expr, $($arg:tt)*) => { $log.write($crate::Level::Detail, format!($($arg)*)) };
}
/// `plain!(log, "...")`.
#[macro_export]
macro_rules! plain {
    ($log:expr, $($arg:tt)*) => { $log.write($crate::Level::Plain, format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::{Level, Log};

    #[test]
    fn a_detail_is_never_the_last_report() {
        let log = Log::memory();
        assert!(log.last_report().is_none());
        log.info("Saved quests/a.nqasset");
        log.detail("startup after the process was created: first frame 1.2 s");
        log.plain("plain text");
        assert_eq!(log.last_report().map(|r| r.text.clone()).as_deref(), Some("Saved quests/a.nqasset"));
        log.warn("a warning");
        assert_eq!(log.last_report().map(|r| r.level), Some(Level::Warning));
        assert_eq!(log.tail(10).len(), 4);
        assert_eq!(Level::Detail.mark(), "* ");
    }
}

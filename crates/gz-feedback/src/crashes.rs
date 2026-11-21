//! What crashes left in the log folder (docs/REPORTS.md, "Crashes"): `grizzly_crash_<secs>_<pid>` with
//! `.txt` (what gz-crash wrote: the thread, the place, the message, the frames as module and offset),
//! `.dmp` when the process ended, `.log` - the tail of the run's own log, taken from the log the next
//! start set aside - and `.handled` once the author sent it or said no. The ten newest stay.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How many crashes the folder keeps.
pub const KEEP: usize = 10;
/// The most of a crashed run's log a crash keeps.
pub const LOG_TAIL: u64 = 4 << 20;

const PREFIXES: [&str; 2] = ["grizzly_crash_", "grizzly_fatal_"];

/// One crash of the editor.
#[derive(Clone, Debug)]
pub struct Crash {
    /// `grizzly_crash_<secs>_<pid>`: what a command names it by.
    pub name: String,
    /// When it happened, seconds since 1970.
    pub time: u64,
    /// The process ended (a dump was written); otherwise the editor caught it and went on.
    pub fatal: bool,
    /// Sent, or the author said no.
    pub handled: bool,
    /// The lines of its `.txt` that say `key: value`, by key.
    pub fields: BTreeMap<String, String>,
    pub text: Option<PathBuf>,
    pub dump: Option<PathBuf>,
    pub log: Option<PathBuf>,
}

impl Crash {
    #[must_use]
    pub fn field(&self, key: &str) -> &str {
        self.fields.get(key).map_or("", String::as_str)
    }

    /// One line for a person: the message, or where it stopped.
    #[must_use]
    pub fn summary(&self) -> String {
        let message = self.field("message");
        let exception = self.field("exception");
        let module = self.field("module");
        if !message.is_empty() {
            message.to_owned()
        } else if !exception.is_empty() {
            if module.is_empty() { exception.to_owned() } else { format!("{exception} in {module}") }
        } else {
            "The editor stopped.".to_owned()
        }
    }
}

fn stem_of(name: &str) -> Option<(&str, &str)> {
    let (stem, ext) = name.rsplit_once('.')?;
    PREFIXES.iter().any(|p| stem.starts_with(p)).then_some((stem, ext))
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The `key: value` lines before the first block (`stack:`, `frames:`) of a crash's text.
#[must_use]
pub fn fields_of(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        // a block's head ends the fields: what follows it is the block's
        if value.trim().is_empty() && !line.starts_with(' ') {
            if matches!(key, "stack" | "frames" | "modules") {
                break;
            }
            continue;
        }
        if key.is_empty() || key.contains(' ') || line.starts_with(' ') {
            continue;
        }
        out.entry(key.to_lowercase()).or_insert_with(|| value.trim().to_owned());
    }
    out
}

/// Every crash in `dir`, the newest first.
#[must_use]
pub fn list(dir: &Path) -> Vec<Crash> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut by_stem: BTreeMap<String, Crash> = BTreeMap::new();
    for e in entries.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some((stem, ext)) = stem_of(&file) else { continue };
        let path = e.path();
        let crash = by_stem.entry(stem.to_owned()).or_insert_with(|| Crash {
            name: stem.to_owned(),
            time: stem
                .strip_prefix("grizzly_crash_")
                .and_then(|r| r.split('_').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            fatal: false,
            handled: false,
            fields: BTreeMap::new(),
            text: None,
            dump: None,
            log: None,
        });
        if crash.time == 0 {
            crash.time = e.metadata().and_then(|m| m.modified()).map_or(0, secs);
        }
        match ext {
            "txt" => crash.text = Some(path),
            "dmp" => {
                crash.dump = Some(path);
                crash.fatal = true;
            }
            "log" => crash.log = Some(path),
            "handled" => crash.handled = true,
            _ => {}
        }
    }
    let mut out: Vec<Crash> =
        by_stem.into_values().filter(|c| c.text.is_some() || c.dump.is_some()).collect();
    for c in &mut out {
        if let Some(text) = c.text.as_ref().and_then(|p| std::fs::read(p).ok()) {
            c.fields = fields_of(&String::from_utf8_lossy(&text));
            if c.fields.get("kind").is_some_and(|k| k == "fatal") {
                c.fatal = true;
            }
        }
    }
    out.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| b.name.cmp(&a.name)));
    out
}

/// Marks a crash as sent or declined: it is offered no more.
pub fn mark_handled(dir: &Path, name: &str) -> std::io::Result<()> {
    std::fs::write(dir.join(format!("{name}.handled")), b"")
}

/// Removes every file of a crash.
fn remove(dir: &Path, name: &str) {
    for ext in ["txt", "dmp", "log", "handled"] {
        let _ = std::fs::remove_file(dir.join(format!("{name}.{ext}")));
    }
}

/// Keeps the [`KEEP`] newest crashes and removes the rest.
pub fn tidy(dir: &Path) {
    for old in list(dir).iter().skip(KEEP) {
        remove(dir, &old.name);
    }
}

/// The last [`LOG_TAIL`] bytes of `file`, from the first whole line on.
#[must_use]
pub fn tail(file: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(limit);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut bytes).ok()?;
    if start > 0
        && let Some(nl) = bytes.iter().position(|&b| b == b'\n')
    {
        bytes.drain(..=nl);
    }
    Some(bytes)
}

/// What a run's log says of its process in its first lines: `Grizzly Editor <version> starting,
/// process <pid>` (`grizzly`'s `main.rs`). A crash names its process too, so the two are matched by
/// it - never by a time, which a run idle for an hour before its fault would not meet, and never by
/// a log another program of the editor set aside meanwhile.
pub const PROCESS_MARK: &str = "starting, process ";

/// The process a crash's name says: `grizzly_crash_<secs>_<pid>`.
fn pid_of(name: &str) -> Option<u32> {
    name.strip_prefix("grizzly_crash_")?.rsplit('_').next()?.parse().ok()
}

/// The process a log was written by, from its first 64 KB.
fn log_pid(file: &Path) -> Option<u32> {
    use std::io::Read as _;
    let mut head = vec![0u8; 64 * 1024];
    let n = std::fs::File::open(file).ok()?.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]);
    let at = text.find(PROCESS_MARK)? + PROCESS_MARK.len();
    let digits: String = text[at..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The newest crash that ended a run gets that run's log: the tail of the `*.previous.log` whose
/// process is the crash's (the start after it set that log aside). A crash the editor caught and went
/// on from has no log of its own. Called at a start, after the log was set aside.
pub fn bind_previous_log(dir: &Path) {
    let Some(crash) = list(dir).into_iter().find(|c| c.fatal) else { return };
    if crash.log.is_some() || crash.handled {
        return;
    }
    let Some(pid) = pid_of(&crash.name) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let found = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".previous.log"))
        .map(|e| e.path())
        .find(|log| log_pid(log) == Some(pid));
    if let Some(log) = found
        && let Some(bytes) = tail(&log, LOG_TAIL)
    {
        let _ = std::fs::write(dir.join(format!("{}.log", crash.name)), bytes);
    }
}

/// The newest crash that ended the editor - the one a start binds its run's log to -, when it was
/// neither sent nor declined and is at most two weeks old: what a start offers to send.
#[must_use]
pub fn offer(dir: &Path) -> Option<Crash> {
    let now = secs(SystemTime::now());
    let fresh = Duration::from_secs(14 * 24 * 3600).as_secs();
    list(dir).into_iter().find(|c| c.fatal).filter(|c| !c.handled && now.saturating_sub(c.time) <= fresh)
}

/// What `latest` names: the newest crash that ended the editor, else the newest of any kind.
#[must_use]
pub fn latest(dir: &Path) -> Option<Crash> {
    let all = list(dir);
    all.iter().find(|c| c.fatal).or_else(|| all.first()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_is_its_files_and_its_fields() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let d = dir.path();
        let now = secs(SystemTime::now());
        let old = format!("grizzly_crash_{}_10", now - 100);
        let new = format!("grizzly_crash_{now}_20");
        std::fs::write(d.join(format!("{old}.txt")), "thread: main\nmessage: index out of bounds\n")
            .unwrap_or(());
        std::fs::write(
            d.join(format!("{new}.txt")),
            // program-files: in memory
            "Grizzly Editor stopped.\nkind: fatal\nexception: EXCEPTION_ACCESS_VIOLATION\nmodule: d3d12.dll\n\nframes:\n  Grizzly.exe+0x1234\n",
        )
        .unwrap_or(());
        std::fs::write(d.join(format!("{new}.dmp")), b"MDMP").unwrap_or(());
        std::fs::write(d.join("grizzly_admin.log"), b"x").unwrap_or(());

        let all = list(d);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, new);
        assert!(all[0].fatal && !all[1].fatal);
        // program-files: in memory
        assert_eq!(all[0].summary(), "EXCEPTION_ACCESS_VIOLATION in d3d12.dll");
        assert_eq!(all[1].summary(), "index out of bounds");
        assert_eq!(offer(d).map(|c| c.name), Some(new.clone()));

        // the log the next start set aside goes with the crash of its process, not another's
        std::fs::write(
            d.join("grizzly_cmd_admin.previous.log"),
            b"Grizzly Editor 0.7.0 starting, process 99\n",
        )
        .unwrap_or(());
        std::fs::write(
            d.join("grizzly_admin.previous.log"),
            b"Grizzly Editor 0.7.0 starting, process 20\nsecond\n",
        )
        .unwrap_or(());
        bind_previous_log(d);
        assert_eq!(
            std::fs::read(d.join(format!("{new}.log"))).ok().as_deref(),
            Some(&b"Grizzly Editor 0.7.0 starting, process 20\nsecond\n"[..])
        );
        assert_eq!(latest(d).map(|c| c.name), Some(new.clone()));

        mark_handled(d, &new).unwrap_or(());
        assert!(offer(d).is_none());
    }

    #[test]
    fn the_folder_keeps_the_newest_ten() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        for i in 0..13 {
            std::fs::write(dir.path().join(format!("grizzly_crash_{}_1.txt", 1000 + i)), "message: x\n")
                .unwrap_or(());
        }
        tidy(dir.path());
        let left = list(dir.path());
        assert_eq!(left.len(), KEEP);
        assert_eq!(left.last().map(|c| c.time), Some(1003));
    }

    #[test]
    fn a_tail_starts_at_a_whole_line() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let f = dir.path().join("a.log");
        std::fs::write(&f, b"0123456789\nabc\ndef\n").unwrap_or(());
        assert_eq!(tail(&f, 7).as_deref(), Some(&b"def\n"[..]));
        assert_eq!(tail(&f, 100).as_deref(), Some(&b"0123456789\nabc\ndef\n"[..]));
    }
}

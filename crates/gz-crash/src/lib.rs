//! What a crash leaves behind.
//!
//! A panic anywhere writes its message and a symbolised stack into the log and into a report
//! file beside it. What happens next depends on where it was:
//!
//! - inside a command or a job the panic is caught at that boundary - the command fails, the
//!   stores keep their last good snapshot, the editor goes on;
//! - on the main thread, or anywhere under `-nodlg`, the process ends with exit code 3 after a
//!   minidump was written: a test must fail loudly, and a window that lost its thread is gone.
//!
//! A fault that is no panic at all - an access violation inside a C library or a driver - also
//! leaves a report, a dump and exit code 3 (`gz_win::install_fatal_exception_dump`).
//!
//! The report is written to be sent: `key: value` lines, then the stack's frames as
//! `module+0xRVA` and the symbolised stack, with no folder of this machine in any of it.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gz_log::Log;
use gz_win::Frame;

pub use gz_win::CRASH_EXIT_CODE;

#[derive(Clone, Debug)]
pub struct CrashOptions {
    /// Where reports and dumps go (the log folder).
    pub dir: PathBuf,
    /// `-nodlg`: any panic ends the process.
    pub unattended: bool,
    /// Dumps for a report that leaves this machine: no stack memory, no data segment, modules
    /// named without their folders. A release installation writes these; a developer build the
    /// full dump a debugger wants.
    pub compact: bool,
    /// The editor's version, a line of every report.
    pub version: &'static str,
}

/// How many frames a report lists.
const FRAMES: usize = 64;

fn stamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Installs the panic hook and the fatal-exception dump. Call once, first thing in `main`.
pub fn install(log: Log, opts: CrashOptions) {
    gz_win::install_fatal_exception_dump(opts.dir.clone(), opts.compact, opts.version);
    let main_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        // first, while the panicking frames are the ones below this
        let frames = gz_win::stack_frames(0, FRAMES);
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_owned();
        let location = info
            .location()
            .map_or_else(|| "unknown location".to_owned(), |l| format!("{}:{}", l.file(), l.line()));
        let message = info
            .payload()
            .downcast_ref::<&'static str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_owned());
        // formatting it reads the symbols, which the panic's site is then told by
        let stack = Backtrace::force_capture().to_string();
        let fatal = opts.unattended || thread.id() == main_thread;

        let report = Report {
            version: opts.version,
            fatal,
            thread: &name,
            location: &location,
            message: &message,
            site: panic_site(&frames),
            frames: &frames,
            stack: &stack,
        }
        .text();

        log.error(format!("internal error in thread '{name}' at {location}: {message}"));
        for line in stack.lines() {
            log.plain(format!("    {}", line.trim_end()));
        }

        let base = opts.dir.join(format!("grizzly_crash_{}_{}", stamp(), std::process::id()));
        write_report(&base.with_extension("txt"), &report);
        if fatal {
            let _ = gz_win::write_minidump(&base.with_extension("dmp"), opts.compact);
            log.error(format!(
                "the editor exits with code {CRASH_EXIT_CODE}; report: {}",
                base.with_extension("txt").display()
            ));
            log.flush();
            std::process::exit(CRASH_EXIT_CODE);
        }
        log.flush();
    }));
}

/// A panic as its report says it.
struct Report<'a> {
    version: &'a str,
    /// The process ends with it; else a command's or a job's boundary caught it.
    fatal: bool,
    thread: &'a str,
    location: &'a str,
    message: &'a str,
    site: Option<&'a Frame>,
    frames: &'a [Frame],
    stack: &'a str,
}

impl Report<'_> {
    /// The same keys as a fault's report (`gz_win`), with the place and the message instead of the
    /// exception, and the symbolised stack after the frames.
    fn text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Grizzly Editor stopped on an internal error.");
        let _ = writeln!(out, "version: {}", self.version);
        let _ = writeln!(out, "kind: {}", if self.fatal { "fatal" } else { "caught" });
        let _ = writeln!(out, "thread: {}", self.thread);
        let _ = writeln!(out, "at: {}", anonymize(self.location));
        // one line: a message of several lines would read as fields of its own ("module: ...")
        let message = self.message.lines().map(str::trim_end).collect::<Vec<_>>().join(" | ");
        let _ = writeln!(out, "message: {}", anonymize(&message));
        if let Some(site) = self.site {
            let _ = writeln!(out, "module: {}", site.module);
            let _ = writeln!(out, "module_rva: {:#x}", site.rva);
        }
        let _ = writeln!(out, "\nframes:");
        for frame in self.frames {
            let _ = writeln!(out, "{frame}");
        }
        let _ = writeln!(out, "\nstack:");
        for line in self.stack.lines() {
            let _ = writeln!(out, "{}", anonymize(line.trim_end()));
        }
        out
    }
}

/// The frame of the editor's own program the panic came from: the first one below the hook and
/// the panic machinery of the standard library. Told by the frames' symbols, so none without them.
fn panic_site(frames: &[Frame]) -> Option<&Frame> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.file_name()?.to_string_lossy().into_owned();
    let own: Vec<&Frame> = frames.iter().filter(|f| f.module.eq_ignore_ascii_case(&exe)).collect();
    let names = gz_win::symbol_names(&own.iter().map(|f| f.address).collect::<Vec<_>>());
    for (frame, name) in own.into_iter().zip(names) {
        // an address without a symbol: where the machinery ends cannot be told
        if name.starts_with("0x") {
            return None;
        }
        if !is_machinery(&name) {
            return Some(frame);
        }
    }
    None
}

/// A function of the panic's own way: the hook, the standard library, the unwinder.
fn is_machinery(name: &str) -> bool {
    let name = name.trim_start_matches('<');
    ["std::", "core::", "alloc::", "panic_unwind::", "panic_abort::", "rust_", "__rust", "gz_crash::install"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// The text with every path of this machine cut to what names the source and nothing of the
/// machine: up to and including `crates\` (the workspace), `.cargo\registry\src\<index>\` and
/// `.cargo\git\checkouts\<repository>\<revision>\` (a dependency); any other path that starts
/// at a drive or a server keeps its file name alone.
#[must_use]
pub fn anonymize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((start, end)) = next_cut(rest) {
        out.push_str(rest.get(..start).unwrap_or_default());
        rest = rest.get(end..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The first stretch of `s` to cut: `(start, end)`, both on char boundaries, `end > start`.
fn next_cut(s: &str) -> Option<(usize, usize)> {
    // (marker, how many folders after it go too); a known marker keeps more than a bare path
    const MARKERS: [(&str, usize); 6] = [
        (".cargo\\registry\\src\\", 1),
        (".cargo/registry/src/", 1),
        (".cargo\\git\\checkouts\\", 2),
        (".cargo/git/checkouts/", 2),
        ("crates\\", 0),
        ("crates/", 0),
    ];
    let mut best: Option<(usize, u8, usize)> = None;
    let mut consider = |start: usize, rank: u8, end: usize| {
        if best.is_none_or(|(best_start, best_rank, _)| (start, rank) < (best_start, best_rank)) {
            best = Some((start, rank, end));
        }
    };
    for (marker, folders) in MARKERS {
        if let Some(at) = s.find(marker) {
            let mut end = at + marker.len();
            for _ in 0..folders {
                end = after_separator(s, end);
            }
            consider(path_start(s, at), 0, end);
        }
    }
    if let Some(drive) = drive_starts(s).next() {
        let line_end = s.get(drive..).and_then(|t| t.find('\n')).map_or(s.len(), |i| drive + i);
        let limit = drive_starts(s).find(|&d| d > drive + 2).filter(|&d| d < line_end).unwrap_or(line_end);
        if let Some(last) = s.get(drive..limit).and_then(|path| path.rfind(['\\', '/'])) {
            consider(drive, 1, drive + last + 1);
        }
    }
    best.map(|(start, _, end)| (start, end))
}

/// Where the next separator after `from` ends, on the same line; `from` when there is none.
fn after_separator(s: &str, from: usize) -> usize {
    let tail = s.get(from..).unwrap_or_default();
    let line = tail.find('\n').unwrap_or(tail.len());
    tail.get(..line).and_then(|t| t.find(['\\', '/'])).map_or(from, |i| from + i + 1)
}

/// Every place in `s` a path of a drive (`C:\`, `C:/`) or of a server (`\\name`) begins.
fn drive_starts(s: &str) -> impl Iterator<Item = usize> + '_ {
    let bytes = s.as_bytes();
    (0..bytes.len()).filter(move |&i| {
        let at = |k: usize| bytes.get(k).copied().unwrap_or(0);
        let before = i.checked_sub(1).map_or(0, at);
        let drive = !before.is_ascii_alphanumeric()
            && at(i).is_ascii_alphabetic()
            && at(i + 1) == b':'
            && matches!(at(i + 2), b'\\' | b'/');
        let server = !before.is_ascii_alphanumeric()
            && before != b'\\'
            && at(i) == b'\\'
            && at(i + 1) == b'\\'
            && at(i + 2).is_ascii_alphanumeric();
        drive || server
    })
}

/// Where the path that holds `at` begins: its drive or server on the same line, else its word.
fn path_start(s: &str, at: usize) -> usize {
    let head = s.get(..at).unwrap_or_default();
    let line = head.rfind('\n').map_or(0, |i| i + 1);
    if let Some(drive) = drive_starts(head).filter(|&d| d >= line).last() {
        return drive;
    }
    head.get(line..)
        .unwrap_or_default()
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '`' | '='))
        .map_or(line, |(i, c)| line + i + c.len_utf8())
}

fn write_report(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_keeps_what_names_the_source_and_nothing_of_the_machine() {
        let cases = [
            (r"D:\Games\XFined-Editor\crates\gz-scene\src\lib.rs:12", r"gz-scene\src\lib.rs:12"),
            ("crates/gz-scene/src/lib.rs:7", "gz-scene/src/lib.rs:7"),
            (r"      at .\crates\gz-x\src\a.rs:5:9", r"      at gz-x\src\a.rs:5:9"),
            (
                r"      at C:\Users\John Smith\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\serde-1.0.0\src\de.rs:5:9",
                r"      at serde-1.0.0\src\de.rs:5:9",
            ),
            (
                r"C:\Users\j\.cargo\git\checkouts\egui-1234abcd\5678ef0\crates\egui\src\x.rs:1",
                r"egui\src\x.rs:1",
            ),
            (
                r"cannot open C:\Users\John\Documents\Mod\quest.nqasset: denied",
                "cannot open quest.nqasset: denied",
            ),
            (r"copy C:\a\b.txt to D:/c/d.txt", "copy b.txt to d.txt"),
            (r"share \\server\share\folder\file.ltx gone", "share file.ltx gone"),
            ("не открыть C:\\Пользователи\\Иван\\файл.txt", "не открыть файл.txt"),
            (
                "/rustc/abc123/library/std/src/panicking.rs:100:5",
                "/rustc/abc123/library/std/src/panicking.rs:100:5",
            ),
            (
                "index out of bounds: the len is 3 but the index is 7",
                "index out of bounds: the len is 3 but the index is 7",
            ),
            ("two lines\nat D:\\x\\crates\\a\\b.rs:1\nend", "two lines\nat a\\b.rs:1\nend"),
        ];
        for (text, want) in cases {
            assert_eq!(anonymize(text), want, "{text}");
        }
    }

    #[test]
    fn the_machinery_of_a_panic_is_told_from_its_site() {
        assert!(is_machinery("std::panicking::rust_panic_with_hook"));
        assert!(is_machinery("core::option::unwrap_failed"));
        assert!(is_machinery("alloc::boxed::impl$28::call<tuple$<> >"));
        assert!(is_machinery("<alloc::boxed::Box<F> as core::ops::function::Fn<A>>::call"));
        assert!(is_machinery("gz_crash::install::closure$0"));
        assert!(!is_machinery("gz_scene::state::apply"));
        assert!(!is_machinery("gz_crash::tests::boom"));
    }

    #[inline(never)]
    fn boom() -> ! {
        panic!("the test's own panic in C:\\Users\\Someone\\project\\quest.nqasset");
    }

    #[test]
    fn a_caught_panic_leaves_a_report_that_names_no_folder() {
        let dir = std::env::temp_dir().join(format!("gz_crash_report_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let installer_dir = dir.clone();
        // installed on a thread of its own, so no panic of the test thread is fatal
        std::thread::spawn(move || {
            install(
                Log::memory(),
                CrashOptions { dir: installer_dir, unattended: false, compact: true, version: "9.9.9-test" },
            );
            let panicked =
                std::thread::Builder::new().name("gz-crash-test-panic".to_owned()).spawn(|| boom());
            assert!(panicked.unwrap().join().is_err());
        })
        .join()
        .unwrap();
        // the default hook back, for any assertion below
        drop(std::panic::take_hook());
        let report = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().ends_with(".txt"))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .unwrap();
        let dumps = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "dmp"))
            .count();
        let _ = std::fs::remove_dir_all(&dir);

        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines[0], "Grizzly Editor stopped on an internal error.", "{report}");
        assert_eq!(lines[1], "version: 9.9.9-test", "{report}");
        assert_eq!(lines[2], "kind: caught", "{report}");
        assert_eq!(lines[3], "thread: gz-crash-test-panic", "{report}");
        let at = lines[4].strip_prefix("at: ").unwrap();
        assert!(at.starts_with("gz-crash") && at.contains("lib.rs:"), "{report}");
        assert_eq!(lines[5], "message: the test's own panic in quest.nqasset", "{report}");
        // the site is this test's own function, in this program
        let exe = std::env::current_exe().unwrap();
        let exe = exe.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(lines[6], format!("module: {exe}"), "{report}");
        let rva = lines[7].strip_prefix("module_rva: ").unwrap();
        assert!(report.contains(&format!("\nframes:\n{exe}+")), "{report}");
        assert!(report.contains(&format!("\n{exe}+{rva}\n")), "{report}");
        assert!(report.contains("\nstack:\n"), "{report}");
        // nothing of this machine
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        assert!(!report.contains(&*workspace.to_string_lossy()), "{report}");
        if let Some(home) = std::env::var_os("USERPROFILE") {
            assert!(!report.to_lowercase().contains(&home.to_string_lossy().to_lowercase()), "{report}");
        }
        // a caught panic leaves no dump
        assert_eq!(dumps, 0);
    }
}

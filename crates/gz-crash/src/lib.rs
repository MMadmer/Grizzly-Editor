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
//! leaves a dump and exit code 3.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gz_log::Log;

pub use gz_win::CRASH_EXIT_CODE;

#[derive(Clone, Debug)]
pub struct CrashOptions {
    /// Where reports and dumps go (the log folder).
    pub dir: PathBuf,
    /// `-nodlg`: any panic ends the process.
    pub unattended: bool,
}

fn stamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Installs the panic hook and the fatal-exception dump. Call once, first thing in `main`.
pub fn install(log: Log, opts: CrashOptions) {
    gz_win::install_fatal_exception_dump(opts.dir.join(format!("grizzly_fatal_{}.dmp", std::process::id())));
    let main_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
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
        let stack = Backtrace::force_capture();

        let mut report = String::new();
        let _ = writeln!(report, "Grizzly Editor stopped on an internal error.");
        let _ = writeln!(report, "thread: {name}");
        let _ = writeln!(report, "at: {location}");
        let _ = writeln!(report, "message: {message}");
        let _ = writeln!(report, "\nstack:\n{stack}");

        log.error(format!("internal error in thread '{name}' at {location}: {message}"));
        for line in stack.to_string().lines() {
            log.plain(format!("    {}", line.trim_end()));
        }

        let fatal = opts.unattended || thread.id() == main_thread;
        let base = opts.dir.join(format!("grizzly_crash_{}_{}", stamp(), std::process::id()));
        write_report(&base.with_extension("txt"), &report);
        if fatal {
            let _ = gz_win::write_minidump(&base.with_extension("dmp"));
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

fn write_report(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

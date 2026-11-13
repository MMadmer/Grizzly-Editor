//! Grizzly Editor for a command line: `Grizzly-Cmd.exe`.
//!
//! `Grizzly.exe` is a window program: a shell does not wait for one, and PowerShell's `>` and
//! `$x = ...` take nothing from one. This console program starts `Grizzly.exe` beside itself with
//! the same arguments and its own streams, waits for it and ends with its exit code:
//!
//! ```text
//! Grizzly-Cmd.exe -headless -compile <level folder> > compile.log
//! ```
//!
//! A run without a window (a compile, a dump, a headless editor) lives in this console: the
//! editor joins it, closes what it has open when the console ends (Ctrl+C, the console closed)
//! and never outlives this program. A run with a window is the editor as Explorer starts it:
//! this program waits for it, and the console's end ends only this program - no console can end
//! an editor past its unsaved-work question.

use std::io::Write;
use std::process::{Command, ExitCode};

// the editor's own reading of its command line; only what kind of run it is, is asked here
#[allow(dead_code)]
mod cli;

/// What this program answers when the editor could not be run at all.
const NOT_RUN: u8 = 2;

fn complain(text: &str) -> ExitCode {
    // nobody listening is no reason to fail another way
    let _ = writeln!(std::io::stderr().lock(), "{text}");
    ExitCode::from(NOT_RUN)
}

fn main() -> ExitCode {
    let editor = match std::env::current_exe() {
        Ok(me) => me.with_file_name("Grizzly.exe"),
        Err(e) => return complain(&format!("Grizzly-Cmd cannot tell where it is: {e}")),
    };
    let windowless =
        cli::Cli::parse(std::env::args_os().map(|a| a.to_string_lossy().into_owned())).windowless();
    let mut command = Command::new(&editor);
    command.args(std::env::args_os().skip(1));
    // a launcher's request for a minimised start (a shortcut's Run: Minimized) is the editor's
    if !windowless && gz_win::started_minimized() {
        command.arg("-minimized");
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return complain(&format!("{} could not be started: {e}", editor.display())),
    };
    if windowless {
        // The editor is in this console too and stops by itself when the console ends: this
        // program stays for its exit code, and the job ends an editor that did not stop in time.
        // Windows refusing either only means the editor may outlive this program.
        let _ = gz_win::end_with_this_process(&child);
        let _ = gz_win::on_console_end(|| {});
    }
    match child.wait() {
        // the editor's own code, whole: a crash's is wider than a byte
        Ok(status) => std::process::exit(status.code().unwrap_or(i32::from(NOT_RUN))),
        Err(e) => complain(&format!("{} could not be waited for: {e}", editor.display())),
    }
}

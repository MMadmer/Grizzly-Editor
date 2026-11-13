//! Grizzly Editor for a command line: `Grizzly-Cmd.exe`.
//!
//! `Grizzly.exe` is a window program: a shell does not wait for one, PowerShell's `>` and
//! `$x = ...` take nothing from one, and the console's Ctrl+C does not reach it. This console
//! program starts `Grizzly.exe` beside itself with the same arguments and its own streams, waits
//! for it and ends with its exit code:
//!
//! ```text
//! Grizzly-Cmd.exe -headless -compile <level folder> > compile.log
//! ```
//!
//! The editor ends with it (Ctrl+C, a closed console).

use std::process::{Command, ExitCode};

/// What this program answers when the editor could not be run at all.
const NOT_RUN: u8 = 2;

fn main() -> ExitCode {
    let editor = match std::env::current_exe() {
        Ok(me) => me.with_file_name("Grizzly.exe"),
        Err(e) => {
            eprintln!("Grizzly-Cmd cannot tell where it is: {e}");
            return ExitCode::from(NOT_RUN);
        }
    };
    let mut child = match Command::new(&editor).args(std::env::args_os().skip(1)).spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("{} could not be started: {e}", editor.display());
            return ExitCode::from(NOT_RUN);
        }
    };
    // Windows refusing the job only means the editor is not ended with this program
    let _ = gz_win::end_with_this_process(&child);
    match child.wait() {
        // the editor's own code, whole: a crash's is wider than a byte
        Ok(status) => std::process::exit(status.code().unwrap_or(i32::from(NOT_RUN))),
        Err(e) => {
            eprintln!("{} could not be waited for: {e}", editor.display());
            ExitCode::from(NOT_RUN)
        }
    }
}

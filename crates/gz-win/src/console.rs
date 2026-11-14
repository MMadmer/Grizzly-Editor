//! A window program in a console.
//!
//! `Grizzly.exe` is linked for the Windows subsystem: started from Explorer, a shortcut or the
//! taskbar it gets no console, so none can end it behind the author's back. A run without a
//! window (`-dump_commands`, a commandlet, `-headless`, ...) still lives in its launcher's console
//! the way a console program does:
//! - [`attach_parent_console`] joins the console of the parent process, when it has one. A stream
//!   the launcher redirected (a pipe, a file) is kept as it is, and only a stream that is missing
//!   is taken from that console;
//! - [`on_console_end`] turns the console's end - Ctrl+C, Ctrl+Break, the console closed, a
//!   logoff, a shutdown - into a request to stop, so the program closes what it has open.

use std::os::windows::io::IntoRawHandle;
use std::sync::OnceLock;
use std::time::Duration;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, GetStdHandle, STD_ERROR_HANDLE,
    STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler, SetStdHandle,
};
use windows::core::BOOL;

const STREAMS: [STD_HANDLE; 3] = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];

fn std_handle(which: STD_HANDLE) -> Option<HANDLE> {
    // SAFETY: GetStdHandle only reads the process parameters; a missing stream is an error here.
    unsafe { GetStdHandle(which) }.ok()
}

/// How the standard streams stand after [`attach_parent_console`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleStreams {
    /// Every stream was given by the launcher (pipes, files, its console): none was changed.
    Given,
    /// The missing streams now write to the parent's console.
    Attached,
    /// Some stream is missing and the parent has no console to take it from: what goes there is
    /// dropped, as for any window program.
    None,
}

/// Joins the console of the parent process, when it has one, and gives the missing standard
/// streams of this process that console. A stream the launcher set (a pipe or a file a tool
/// reads, `> file`, `| more`) stays exactly what it was. In the console, its end reaches this
/// process ([`on_console_end`]).
pub fn attach_parent_console() -> ConsoleStreams {
    let given = STREAMS.map(std_handle);
    let all_given = given.iter().all(Option::is_some);
    // SAFETY: AttachConsole takes a process id and changes only which console this process is
    // attached to; it fails harmlessly when the parent has none or one is attached already.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return if all_given { ConsoleStreams::Given } else { ConsoleStreams::None };
    }
    for (which, kept) in STREAMS.into_iter().zip(given) {
        let handle = match kept {
            // attaching may set every stream to the console: a redirected one goes back
            Some(handle) => Some(handle),
            None => std_handle(which).or_else(|| open_console(which == STD_INPUT_HANDLE)),
        };
        if let Some(handle) = handle {
            // SAFETY: the handle is one this process owns for the rest of its life (the
            // launcher's stream, or a console handle opened below and never closed).
            let _ = unsafe { SetStdHandle(which, handle) };
        }
    }
    if all_given { ConsoleStreams::Given } else { ConsoleStreams::Attached }
}

/// The attached console's input or output, opened for the rest of the process's life.
fn open_console(input: bool) -> Option<HANDLE> {
    let name = if input { "CONIN$" } else { "CONOUT$" };
    let file = std::fs::OpenOptions::new().read(true).write(true).open(name).ok()?;
    // the standard stream owns it from now on: it is never closed
    Some(HANDLE(file.into_raw_handle()))
}

/// What the console's end asks of this program. Windows calls a plain function with no context,
/// so the request is kept here: set once, read by the handler.
static ASK_TO_STOP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// How long a program has after its console was closed (Windows gives five seconds), less a
/// margin.
const CLOSE_GRACE: Duration = Duration::from_millis(4500);

/// Has `ask_to_stop` called when the console this process is in ends - Ctrl+C, Ctrl+Break, the
/// console closed, a logoff, a shutdown - instead of the process being ended on the spot.
/// `ask_to_stop` runs on a thread of the system's and only asks: the program stops by itself.
/// After Ctrl+C and Ctrl+Break it has all the time it needs; after the others Windows ends it
/// within five seconds, and the handler holds that time open. Set once: `false` for a second
/// call or when Windows refuses the handler.
pub fn on_console_end(ask_to_stop: impl Fn() + Send + Sync + 'static) -> bool {
    if ASK_TO_STOP.set(Box::new(ask_to_stop)).is_err() {
        return false;
    }
    // SAFETY: `console_end` is a function of this program that lives as long as it does; the
    // call adds it to the process's list of console handlers.
    unsafe { SetConsoleCtrlHandler(Some(console_end), true) }.is_ok()
}

extern "system" fn console_end(kind: u32) -> BOOL {
    let Some(ask) = ASK_TO_STOP.get() else { return false.into() };
    ask();
    // After a closed console, a logoff or a shutdown Windows ends the process as soon as this
    // returns: the time to stop is spent here, and the process leaves before it runs out.
    if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
        std::thread::sleep(CLOSE_GRACE);
    }
    true.into()
}

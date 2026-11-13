//! The standard streams of a window program.
//!
//! `Grizzly.exe` is linked for the Windows subsystem: started from Explorer, a shortcut or the
//! taskbar it gets no console, so none can end it behind the author's back. A run that prints
//! (`-dump_commands`, `-compile`, `-log_echo`, ...) still has to reach whoever started it:
//! a stream the launcher redirected (a pipe, a file) is kept as it is, and only a stream that is
//! missing is taken from the console of the parent process, when it has one.

use std::os::windows::io::IntoRawHandle;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};

const STREAMS: [STD_HANDLE; 3] = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];

fn std_handle(which: STD_HANDLE) -> Option<HANDLE> {
    // SAFETY: GetStdHandle only reads the process parameters; a missing stream is an error here.
    unsafe { GetStdHandle(which) }.ok()
}

/// How the standard streams stand after [`attach_parent_console`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleStreams {
    /// Every stream was given by the launcher (pipes, files, its console): nothing was changed.
    Given,
    /// The missing streams now write to the parent's console.
    Attached,
    /// Some stream is missing and the parent has no console to take it from: what goes there is
    /// dropped, as for any window program.
    None,
}

/// Gives the missing standard streams of this process the parent's console. A stream the
/// launcher set (a pipe or a file a tool reads, `> file`, `| more`) stays exactly what it was.
pub fn attach_parent_console() -> ConsoleStreams {
    let given = STREAMS.map(std_handle);
    if given.iter().all(Option::is_some) {
        return ConsoleStreams::Given;
    }
    // SAFETY: AttachConsole takes a process id and changes only which console this process is
    // attached to; it fails harmlessly when the parent has none or one is attached already.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return ConsoleStreams::None;
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
    ConsoleStreams::Attached
}

/// The attached console's input or output, opened for the rest of the process's life.
fn open_console(input: bool) -> Option<HANDLE> {
    let name = if input { "CONIN$" } else { "CONOUT$" };
    let file = std::fs::OpenOptions::new().read(true).write(true).open(name).ok()?;
    // the standard stream owns it from now on: it is never closed
    Some(HANDLE(file.into_raw_handle()))
}

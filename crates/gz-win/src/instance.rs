//! One editor per home: whether the editor a lock file names still runs, and letting it bring
//! its window to the front when a second start hands it a project.

use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

/// Whether the process `pid` is running.
#[must_use]
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: OpenProcess takes a plain id; the handle it returns is checked and closed below.
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let mut code = 0u32;
    // SAFETY: `handle` was opened above with the query right; `code` is a live local.
    let read = unsafe { GetExitCodeProcess(handle, &raw mut code) };
    // SAFETY: `handle` is the handle opened above, closed once.
    let _ = unsafe { CloseHandle(handle) };
    read.is_ok() && code == STILL_ACTIVE.0 as u32
}

/// Lets the process `pid` bring its window to the front: Windows gives the foreground only to
/// the process the user last worked with, which is this one when it was just started.
pub fn allow_foreground(pid: u32) -> bool {
    // SAFETY: the call takes a plain id and changes only who may take the foreground.
    unsafe { AllowSetForegroundWindow(pid).is_ok() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn this_process_runs_and_none_has_the_id_zero() {
        assert!(super::process_alive(std::process::id()));
        assert!(!super::process_alive(0));
    }
}

//! What the editor's own update needs of Win32: a rename that is a rename and nothing else, and a
//! named event the restarted editor signals when it is up.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::core::PCWSTR;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Renames `from` to `to` on one volume. A program that runs can be renamed this way, which is all
/// an update of the running editor needs. Nothing is ever copied (no `MOVEFILE_COPY_ALLOWED`) and
/// an existing `to` is never replaced: either fails.
pub fn rename_in_place(from: &Path, to: &Path) -> io::Result<()> {
    let (a, b) = (wide(from.as_os_str()), wide(to.as_os_str()));
    // SAFETY: both are zero-terminated wide strings alive for the call; no flags: a plain rename.
    unsafe { MoveFileExW(PCWSTR::from_raw(a.as_ptr()), PCWSTR::from_raw(b.as_ptr()), MOVE_FILE_FLAGS(0)) }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}

/// A named, manual-reset event of this session: one process signals it, another waits for it.
/// The handle is kept as a number so the value can travel between threads; it is closed on drop.
#[derive(Debug)]
pub struct NamedEvent {
    handle: isize,
}

impl NamedEvent {
    /// Opens the event of that name, creating it unsignalled when nobody holds it yet. Both sides
    /// call this: whoever comes first makes it, and a signal given before the other side opened it
    /// stays.
    pub fn open(name: &str) -> io::Result<Self> {
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: no security attributes; manual reset, unsignalled; `name` is zero-terminated and
        // alive for the call.
        let handle = unsafe { CreateEventW(None, true, false, PCWSTR::from_raw(name.as_ptr())) }
            .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        Ok(Self { handle: handle.0 as isize })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle as *mut core::ffi::c_void)
    }

    /// Signals it.
    pub fn set(&self) -> io::Result<()> {
        // SAFETY: the handle was made by `open` and is open until drop.
        unsafe { SetEvent(self.raw()) }.map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
    }

    /// Whether it has been signalled, without waiting.
    #[must_use]
    pub fn is_set(&self) -> bool {
        // SAFETY: the handle was made by `open` and is open until drop; a zero timeout only polls.
        let waited = unsafe { WaitForSingleObject(self.raw(), 0) };
        waited == WAIT_OBJECT_0
    }
}

impl Drop for NamedEvent {
    fn drop(&mut self) {
        // SAFETY: the handle was made by `open` and is closed once, here.
        let _ = unsafe { CloseHandle(self.raw()) };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_given_before_the_other_side_opens_stays() {
        let name = format!("Local\\GrizzlyEditorTest-{}", std::process::id());
        let a = NamedEvent::open(&name).unwrap();
        assert!(!a.is_set());
        a.set().unwrap();
        let b = NamedEvent::open(&name).unwrap();
        assert!(b.is_set());
    }

    #[test]
    fn a_rename_never_replaces() {
        let dir = std::env::temp_dir().join(format!("gz-win-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert!(rename_in_place(&a, &b).is_err());
        std::fs::remove_file(&b).unwrap();
        rename_in_place(&a, &b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), b"a");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

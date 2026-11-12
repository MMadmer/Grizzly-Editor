//! The few Win32 calls the standard library and the windowing crates do not make.
//!
//! This is the one crate of the editor that contains `unsafe`. Its API is safe: every function
//! here upholds the invariants of the call it wraps by itself, and nothing unsafe leaks out.
//!
//! - [`process_counters`] - when the process was created and how many file operations it made:
//!   the numbers of the startup profile;
//! - [`started_minimized`] - whether the launcher asked for a minimised window;
//! - [`write_minidump`], [`install_fatal_exception_dump`] - what a crash leaves behind;
//! - [`foreground_is_ours`] - whether a window of this process is in front;
//! - [`raise_this_thread_priority`] - the thread that draws and presents above the pools;
//! - [`install_caption_hit_test`] - `WM_NCHITTEST` for a window that draws its own caption;
//! - [`known_folder`], [`free_space`], [`volume_path_name`], [`is_volume_root`] - the user's
//!   folders and the volumes.

#![cfg(windows)]

mod caption;
mod folders;

pub use caption::{CaptionHitTest, CaptionShape, PxRect, hit_test, install_caption_hit_test};
pub use folders::{KnownFolder, free_space, is_volume_root, known_folder, volume_path_name};

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION, MiniDumpNormal,
    MiniDumpWithIndirectlyReferencedMemory, MiniDumpWriteDump, SetUnhandledExceptionFilter,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId, GetProcessIoCounters,
    GetProcessTimes, GetStartupInfoW, GetThreadPriority, IO_COUNTERS, STARTF_USESHOWWINDOW, STARTUPINFOW,
    SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SW_MINIMIZE, SW_SHOWMINIMIZED, SW_SHOWMINNOACTIVE,
};

/// What the system counted for this process since it was created.
#[derive(Clone, Copy, Debug)]
pub struct ProcessCounters {
    /// When the process was created: the start of the startup profile's loader line.
    pub created: SystemTime,
    pub read_ops: u64,
    pub write_ops: u64,
    /// Opens, closes, queries: everything that is neither a read nor a write.
    pub other_ops: u64,
    pub read_bytes: u64,
}

impl ProcessCounters {
    /// Reads plus everything else: the "file operations" of the startup profile.
    #[must_use]
    pub fn file_ops(&self) -> u64 {
        self.read_ops + self.write_ops + self.other_ops
    }
}

fn filetime_to_system(ft: FILETIME) -> SystemTime {
    // FILETIME counts 100 ns from 1601-01-01; the Unix epoch is 11644473600 s later
    let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    let unix_100ns = ticks.saturating_sub(116_444_736_000_000_000);
    SystemTime::UNIX_EPOCH + Duration::from_nanos(unix_100ns.saturating_mul(100))
}

/// The creation time and IO counters of this process.
pub fn process_counters() -> io::Result<ProcessCounters> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut io_counters = IO_COUNTERS::default();
    // SAFETY: the pseudo-handle of the current process is always valid, and every out pointer
    // refers to a live local of the exact type the call writes.
    unsafe {
        let process = GetCurrentProcess();
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)
            .map_err(io::Error::other)?;
        GetProcessIoCounters(process, &mut io_counters).map_err(io::Error::other)?;
    }
    Ok(ProcessCounters {
        created: filetime_to_system(created),
        read_ops: io_counters.ReadOperationCount,
        write_ops: io_counters.WriteOperationCount,
        other_ops: io_counters.OtherOperationCount,
        read_bytes: io_counters.ReadTransferCount,
    })
}

/// Whether whoever started the process asked for a minimised window (a shortcut's
/// "Run: Minimized", a test harness's `SW_SHOWMINNOACTIVE`).
#[must_use]
pub fn started_minimized() -> bool {
    let mut info = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    // SAFETY: `info` is a live STARTUPINFOW with `cb` set; the call only writes into it.
    unsafe { GetStartupInfoW(&mut info) };
    if !info.dwFlags.contains(STARTF_USESHOWWINDOW) {
        return false;
    }
    let show = i32::from(info.wShowWindow);
    show == SW_SHOWMINIMIZED.0 || show == SW_MINIMIZE.0 || show == SW_SHOWMINNOACTIVE.0
}

/// Puts the calling thread above normal priority: the thread that draws and presents the window.
/// It works in short bursts, so it costs the pools next to nothing and still gets its core first
/// while they load every core. Answers whether it took.
pub fn raise_this_thread_priority() -> bool {
    // SAFETY: the pseudo-handle of the calling thread is always valid and the priority is one of
    // the documented constants; the call changes nothing but this thread's scheduling.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL).is_ok() }
}

/// The calling thread's priority (`THREAD_PRIORITY_*`: 0 normal, 1 above normal).
#[must_use]
pub fn this_thread_priority() -> i32 {
    // SAFETY: the pseudo-handle of the calling thread is always valid; the call only reads.
    unsafe { GetThreadPriority(GetCurrentThread()) }
}

/// Whether the window in front belongs to this process.
#[must_use]
pub fn foreground_is_ours() -> bool {
    let mut pid = 0u32;
    // SAFETY: GetForegroundWindow takes nothing and may return a null handle, which
    // GetWindowThreadProcessId accepts (it then returns 0 and leaves `pid` alone); `pid` is a
    // live local.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

fn dump_to(file: &File, exception: Option<*const EXCEPTION_POINTERS>) -> io::Result<()> {
    let info = exception.map(|pointers| MINIDUMP_EXCEPTION_INFORMATION {
        // SAFETY: reading the id of the calling thread has no preconditions.
        ThreadId: unsafe { GetCurrentThreadId() },
        ExceptionPointers: pointers.cast_mut(),
        ClientPointers: false.into(),
    });
    let kind = MiniDumpNormal | MiniDumpWithIndirectlyReferencedMemory;
    // SAFETY: the process handle is the always-valid pseudo-handle; `file` is an open, writable
    // file that outlives the call; `info`, when present, lives on this stack frame until the
    // call returns and points at the exception record the system handed to the filter.
    unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            HANDLE(file.as_raw_handle()),
            kind,
            info.as_ref().map(std::ptr::from_ref),
            None,
            None,
        )
        .map_err(io::Error::other)
    }
}

/// Writes a minidump of this process as it is now.
pub fn write_minidump(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    dump_to(&File::create(path)?, None)
}

// Where the exception filter writes its dump. The filter is called by the system with no
// argument of ours, so the path has to live in a static; it is set once, before the filter is
// installed, and only read afterwards.
static FATAL_DUMP: OnceLock<PathBuf> = OnceLock::new();

/// The exit code of a crashed editor: what the test harness looks for.
pub const CRASH_EXIT_CODE: i32 = 3;

unsafe extern "system" fn fatal_filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
    if let Some(path) = FATAL_DUMP.get()
        && let Ok(file) = File::create(path)
    {
        let _ = dump_to(&file, Some(pointers));
    }
    std::process::exit(CRASH_EXIT_CODE)
}

/// From now on an access violation or any other unhandled system exception - a fault inside a
/// C library or a driver - leaves a minidump at `path` and ends the process with
/// [`CRASH_EXIT_CODE`]. A second call changes nothing.
pub fn install_fatal_exception_dump(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if FATAL_DUMP.set(path).is_err() {
        return;
    }
    // SAFETY: `fatal_filter` has the signature the system expects, never returns into the faulting
    // code (it exits the process), and stays valid for the life of the process.
    unsafe {
        SetUnhandledExceptionFilter(Some(fatal_filter));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_raised_thread_stays_raised_and_others_stay_normal() {
        let raised =
            std::thread::spawn(|| (super::raise_this_thread_priority(), super::this_thread_priority()));
        assert_eq!(raised.join().ok(), Some((true, 1)));
        let other = std::thread::spawn(super::this_thread_priority);
        assert_eq!(other.join().ok(), Some(0));
    }
}

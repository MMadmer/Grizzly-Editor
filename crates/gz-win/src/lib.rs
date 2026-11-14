//! The few Win32 calls the standard library and the windowing crates do not make.
//!
//! This is the one crate of the editor that contains `unsafe`. Its API is safe: every function
//! here upholds the invariants of the call it wraps by itself, and nothing unsafe leaks out.
//!
//! - [`process_counters`] - when the process was created and how many file operations it made:
//!   the numbers of the startup profile;
//! - [`started_minimized`] - whether the launcher asked for a minimised window;
//! - [`attach_parent_console`], [`on_console_end`] - a run without a window in its launcher's
//!   console: its streams, and the console's end as a request to stop;
//! - [`end_with_this_process`] - a child process that does not outlive this one;
//! - [`write_minidump`], [`install_fatal_exception_dump`] - what a crash leaves behind;
//! - [`foreground_is_ours`] - whether a window of this process is in front;
//! - [`raise_this_thread_priority`] - the thread that draws and presents above the pools;
//! - [`keep_full_speed`] - the process out of Windows' power throttling while its window is
//!   behind others;
//! - [`install_caption_hit_test`] - `WM_NCHITTEST` for a window that draws its own caption;
//! - [`known_folder`], [`free_space`], [`volume_path_name`], [`is_volume_root`] - the user's
//!   folders and the volumes;
//! - [`long_paths_enabled`], [`installed_game_roots`], [`Association`] - the registry: long
//!   paths, where the installed game is, the project file's association for the current user;
//! - [`process_alive`], [`allow_foreground`] - one editor per home: the running one, and its
//!   window brought forward by a second start;
//! - [`rename_in_place`], [`NamedEvent`] - the editor's own update: a rename that never copies,
//!   and the signal of the restarted editor.

#![cfg(windows)]

mod caption;
mod console;
mod folders;
mod instance;
mod job;
mod registry;
mod stack;
mod update;

pub use caption::{CaptionHitTest, CaptionShape, PxRect, hit_test, install_caption_hit_test};
pub use console::{ConsoleStreams, attach_parent_console, on_console_end};
pub use folders::{KnownFolder, free_space, is_volume_root, known_folder, volume_path_name};
pub use instance::{allow_foreground, process_alive};
pub use job::end_with_this_process;
pub use stack::{current_thread_id, symbol_names, thread_stack};
pub use registry::{Association, delete_user_key, installed_game_roots, long_paths_enabled};
pub use update::{NamedEvent, rename_in_place};

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
    GetProcessTimes, GetStartupInfoW, GetThreadPriority, GetThreadTimes, IO_COUNTERS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, STARTF_USESHOWWINDOW, STARTUPINFOW,
    SetProcessInformation, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, TerminateProcess,
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
    /// The CPU time of every thread of the process so far, kernel and user, in seconds.
    pub cpu_seconds: f64,
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

/// The CPU time the calling thread has used so far, kernel and user, in seconds. A stretch of
/// wall time in which this hardly moved was spent waiting - for a lock, a file, the processor -
/// not working.
#[must_use]
pub fn thread_cpu_seconds() -> f64 {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the pseudo-handle of the current thread is always valid, and every out pointer
    // refers to a live local of the exact type the call writes.
    let ok = unsafe { GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user) };
    if ok.is_err() {
        return 0.0;
    }
    (filetime_ticks(kernel) + filetime_ticks(user)) as f64 / 1e7
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
        cpu_seconds: (filetime_ticks(kernel) + filetime_ticks(user)) as f64 / 1e7,
    })
}

/// A `FILETIME` duration in its 100 ns ticks.
fn filetime_ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
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

/// Takes this process out of Windows' execution-speed power throttling (EcoQoS): Windows runs
/// the threads of a process whose window is behind others, minimised or on another desktop at
/// an efficient, lower speed, and a level compile in the editor's pools then took 11 - 17 %
/// longer than the same compile headless (docs/rust/STATE.md, "After phase 8c"). No priority
/// changes. Answers whether it took.
pub fn keep_full_speed() -> bool {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: the pseudo-handle of the current process is always valid; `state` is a live local
    // of the size passed, read by the call and not kept.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok()
    }
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

/// Paints the client area of a window of this thread in one colour (`rgb`): what a window shows
/// before its graphics device exists, instead of the white or black of an unpainted window.
/// Answers whether it painted.
pub fn fill_client(hwnd: isize, rgb: [u8; 3]) -> bool {
    use windows::Win32::Foundation::{COLORREF, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, FillRect, GetDC, HGDIOBJ, ReleaseDC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    let color = COLORREF(u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16));
    // SAFETY: the handle is checked to name a window before anything is drawn; the device context
    // is released and the brush deleted before returning, on every path; `rect` is a live local.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect).is_err() {
            return false;
        }
        let dc = GetDC(Some(hwnd));
        if dc.is_invalid() {
            return false;
        }
        let brush = CreateSolidBrush(color);
        let painted = !brush.is_invalid() && FillRect(dc, &rect, brush) != 0;
        if !brush.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        ReleaseDC(Some(hwnd), dc);
        painted
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
    // The state is broken: no Rust destructor, `atexit` handler or thread-local teardown may
    // run on it (`std::process::exit` runs them). The process ends here, as a filter may end it.
    // SAFETY: the pseudo-handle of this process is always valid; TerminateProcess does not
    // return when it terminates the calling process.
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), CRASH_EXIT_CODE as u32);
    }
    // only reached when the system refused to end the process: the default handler ends it
    1
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
    // code (it terminates the process, or hands the exception to the default handler), and stays
    // valid for the life of the process.
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

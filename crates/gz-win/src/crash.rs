//! What a crash leaves behind, and what a report of it may carry off this machine.
//!
//! A code address is told as `module + RVA`: the file name of the program or library and the
//! offset in it, which the author resolves against the build's own symbols - no folder, and no
//! address of this run. A compact dump holds the threads' registers, the modules and the
//! exception, and no memory of the process: no stack page, no data segment, no thread block;
//! the modules' folders and the time zone are taken out of it once it is written.

use std::ffi::c_void;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::{self, Write as _};
use std::ops::Range;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::FileExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::SystemTime;

use windows::Win32::Foundation::{
    CloseHandle, FALSE, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, HMODULE, LocalFree, S_OK, TRUE,
};
use windows::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
};
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, EXCEPTION_POINTERS, IncludeModuleCallback, IncludeThreadCallback, MINIDUMP_CALLBACK_INFORMATION,
    MINIDUMP_CALLBACK_INPUT, MINIDUMP_CALLBACK_OUTPUT, MINIDUMP_CALLBACK_TYPE,
    MINIDUMP_EXCEPTION_INFORMATION, MiniDumpNormal, MiniDumpWithIndirectlyReferencedMemory,
    MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump, ModuleCallback,
    ModuleWriteDataSeg, ReadMemoryFailureCallback, RemoveMemoryCallback, SetUnhandledExceptionFilter,
    ThreadCallback, ThreadExCallback, ThreadWriteBackingStore, ThreadWriteStack, ThreadWriteThreadData,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows::Win32::System::Threading::{
    CreateThread, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetThreadDescription, INFINITE,
    OpenThread, STACK_SIZE_PARAM_IS_A_RESERVATION, Sleep, THREAD_QUERY_LIMITED_INFORMATION, TerminateProcess,
    WaitForSingleObject,
};
use windows::core::{BOOL, PCWSTR};

windows::core::link!("kernel32.dll" "system" fn RtlCaptureStackBackTrace(framestoskip: u32, framestocapture: u32, backtrace: *mut *mut c_void, backtracehash: *mut u32) -> u16);

/// The exit code of a crashed editor: what the test harness looks for.
pub const CRASH_EXIT_CODE: i32 = 3;

/// A code address of a stack: where it lies, as a report says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The file name of the module the address lies in (`Grizzly.exe`), never a path; empty for
    /// an address in no module (code made at run time).
    pub module: String,
    /// The address's offset in its module; 0 outside one.
    pub rva: u64,
    /// The address itself: this run's, for this machine only - a report carries `module + rva`.
    pub address: u64,
}

impl fmt::Display for Frame {
    /// `Grizzly.exe+0x1a2b3c`; `?` for an address in no module.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.module.is_empty() { f.write_str("?") } else { write!(f, "{}+{:#x}", self.module, self.rva) }
    }
}

/// Longer module paths than this are not named: none the loader hands out in practice.
const NAME_UNITS: usize = 1024;
/// How many addresses one capture call takes.
const CHUNK: usize = 64;

/// The base of the module that holds `address`, and where the module's file name (the path's
/// last part) lies in `buf`.
fn locate(address: u64, buf: &mut [u16; NAME_UNITS]) -> Option<(u64, Range<usize>)> {
    if address == 0 {
        return None;
    }
    let mut module = HMODULE::default();
    // SAFETY: with FROM_ADDRESS the name argument is the address itself, never read as text; with
    // UNCHANGED_REFCOUNT the module's count is left alone, so nothing is to be released; `module`
    // is a live local the call writes.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address as usize as *const u16),
            &raw mut module,
        )
    };
    if found.is_err() || module.is_invalid() {
        return None;
    }
    // SAFETY: the handle names a module of this process; the call writes at most `buf.len()` units.
    let len = unsafe { GetModuleFileNameW(Some(module), buf) } as usize;
    // a name as long as the buffer was cut short
    if len == 0 || len >= buf.len() {
        return None;
    }
    let path = buf.get(..len)?;
    let start = path.iter().rposition(|&unit| is_separator(unit)).map_or(0, |i| i + 1);
    Some((module.0 as usize as u64, start..len))
}

fn is_separator(unit: u16) -> bool {
    unit == u16::from(b'\\') || unit == u16::from(b'/')
}

fn frame_at(address: u64) -> Frame {
    let mut buf = [0u16; NAME_UNITS];
    match locate(address, &mut buf) {
        Some((base, name)) => Frame {
            module: String::from_utf16_lossy(buf.get(name).unwrap_or_default()),
            rva: address.saturating_sub(base),
            address,
        },
        None => Frame { module: String::new(), rva: 0, address },
    }
}

/// The file name of the module that holds `address`, and the address's offset in it.
#[must_use]
pub fn module_of(address: u64) -> Option<(String, u64)> {
    let frame = frame_at(address);
    (!frame.module.is_empty()).then_some((frame.module, frame.rva))
}

/// The return addresses of the calling thread, innermost first: at most `max`, the first `skip`
/// of the caller's left out (0 starts at the caller of this function).
#[inline(never)]
#[must_use]
pub fn stack_frames(skip: usize, max: usize) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut ips = [std::ptr::null_mut::<c_void>(); CHUNK];
    // this function's own frame is never one of them
    let mut from = skip.saturating_add(1);
    while out.len() < max {
        let want = (max - out.len()).min(CHUNK);
        // SAFETY: `ips` holds CHUNK pointers and the call writes at most `want` <= CHUNK of them; no
        // hash is asked for.
        let got = unsafe {
            RtlCaptureStackBackTrace(
                u32::try_from(from).unwrap_or(u32::MAX),
                want as u32,
                ips.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        let got = usize::from(got).min(want);
        out.extend(ips.iter().take(got).map(|&ip| frame_at(ip as usize as u64)));
        if got < want {
            break;
        }
        from = from.saturating_add(got);
    }
    out
}

/// The stacks of the threads a compact dump writes: the dump writer puts a thread's stack into
/// the dump's memory whatever its flags say, so each is handed back to it as memory to remove.
/// Lives on the dump writer's frame; nothing is allocated.
struct Stacks {
    ranges: [(u64, u32); MAX_STACKS],
    count: usize,
    removed: usize,
}

const MAX_STACKS: usize = 1024;

/// Where a thread's `StackBase` (then `StackEnd`) lies in the callback's input. The system's
/// record aligns its `CONTEXT` to 16 bytes inside the 4-packed record, which the generated
/// binding does not, so its own fields would be read 4 bytes early.
const THREAD_STACK_BASE: usize =
    std::mem::offset_of!(MINIDUMP_CALLBACK_INPUT, Anonymous) + 16 + size_of::<CONTEXT>() + 4;

/// Takes out of a compact dump what is no part of a report: the threads' stacks and blocks, the
/// modules' data segments.
unsafe extern "system" fn compact_callback(
    param: *mut c_void,
    input: *const MINIDUMP_CALLBACK_INPUT,
    output: *mut MINIDUMP_CALLBACK_OUTPUT,
) -> BOOL {
    if param.is_null() || input.is_null() || output.is_null() {
        return FALSE;
    }
    const NOT_THREAD: u32 = (ThreadWriteStack.0 | ThreadWriteBackingStore.0 | ThreadWriteThreadData.0) as u32;
    const NOT_MODULE: u32 = ModuleWriteDataSeg.0 as u32;
    // SAFETY: the three pointers come from MiniDumpWriteDump and are valid for this call, `param`
    // being the `Stacks` of `dump_to`, which waits for the call; the packed input is read by value
    // (a thread's stack bounds unaligned, at the place the system's record has them, inside the
    // record), and the members of the output union written are the ones the callback type names.
    unsafe {
        let stacks = &mut *param.cast::<Stacks>();
        let kind = MINIDUMP_CALLBACK_TYPE((*input).CallbackType as i32);
        if kind == ThreadCallback || kind == ThreadExCallback {
            let flags = (*output).Anonymous.ThreadWriteFlags;
            (*output).Anonymous.ThreadWriteFlags = flags & !NOT_THREAD;
            let at = input.cast::<u8>().add(THREAD_STACK_BASE);
            let (base, end) = (at.cast::<u64>().read_unaligned(), at.add(8).cast::<u64>().read_unaligned());
            let (low, high) = (base.min(end), base.max(end));
            if let Some(slot) = stacks.ranges.get_mut(stacks.count)
                && high > low
            {
                *slot = (low, u32::try_from(high - low).unwrap_or(u32::MAX));
                stacks.count += 1;
            }
            TRUE
        } else if kind == RemoveMemoryCallback {
            // called until it answers FALSE: one stack each time
            match stacks.ranges.get(stacks.removed).filter(|_| stacks.removed < stacks.count) {
                Some(&(base, size)) => {
                    (*output).Anonymous.Anonymous1.MemoryBase = base;
                    (*output).Anonymous.Anonymous1.MemorySize = size;
                    stacks.removed += 1;
                    TRUE
                }
                None => FALSE,
            }
        } else if kind == ModuleCallback {
            let flags = (*output).Anonymous.ModuleWriteFlags;
            (*output).Anonymous.ModuleWriteFlags = flags & !NOT_MODULE;
            TRUE
        } else if kind == IncludeThreadCallback || kind == IncludeModuleCallback {
            TRUE
        } else if kind == ReadMemoryFailureCallback {
            // memory that cannot be read is left out, not a reason to give up the dump
            (*output).Anonymous.Status = S_OK;
            TRUE
        } else {
            FALSE
        }
    }
}

/// Writes a dump of this process into `file`; `exception` is the faulting thread and what the
/// system handed the filter for it.
fn dump_to(
    file: &File,
    exception: Option<(u32, *const EXCEPTION_POINTERS)>,
    compact: bool,
) -> io::Result<()> {
    let info = exception.map(|(thread, pointers)| MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: thread,
        ExceptionPointers: pointers.cast_mut(),
        ClientPointers: false.into(),
    });
    let mut stacks = Stacks { ranges: [(0, 0); MAX_STACKS], count: 0, removed: 0 };
    let callback = MINIDUMP_CALLBACK_INFORMATION {
        CallbackRoutine: Some(compact_callback),
        CallbackParam: (&raw mut stacks).cast(),
    };
    let kind = if compact {
        MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules
    } else {
        MiniDumpNormal | MiniDumpWithIndirectlyReferencedMemory
    };
    // SAFETY: the process handle is the always-valid pseudo-handle; `file` is an open, writable
    // file that outlives the call; `info`, `callback` and `stacks` live on this frame until the call returns,
    // `info` pointing at the exception record the system handed to the filter, which waits for
    // this call; the callback has the signature the call expects.
    unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            HANDLE(file.as_raw_handle()),
            kind,
            info.as_ref().map(std::ptr::from_ref),
            None,
            compact.then_some(std::ptr::from_ref(&callback)),
        )
        .map_err(io::Error::other)?;
    }
    if compact { anonymize_dump(file) } else { Ok(()) }
}

/// Writes a minidump of this process as it is now; `compact` for one that leaves this machine.
pub fn write_minidump(path: &Path, compact: bool) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::options().read(true).write(true).create(true).truncate(true).open(path)?;
    dump_to(&file, None, compact)
}

// The minidump format's records, as far as the anonymising pass reads them.
const DUMP_SIGNATURE: u32 = u32::from_le_bytes(*b"MDMP");
const THREAD_LIST: u32 = 3;
const THREAD_ENTRY: u64 = 48;
/// Where a thread record keeps its stack's size, then the place of its bytes.
const THREAD_STACK_AT: u64 = 32;
const MODULE_LIST: u32 = 4;
const UNLOADED_MODULE_LIST: u32 = 14;
const MISC_INFO: u32 = 15;
const DIRECTORY_ENTRY: u64 = 12;
const MODULE_ENTRY: u64 = 108;
/// Where a module record and an unloaded module record keep the place of their name.
const MODULE_NAME_AT: u64 = 20;
/// `MINIDUMP_MISC_INFO_3`: its size, the flag that says the time zone is there, where the time
/// zone's id and description lie.
const MISC_INFO_3_SIZE: u32 = 232;
const MISC3_TIMEZONE: u32 = 0x40;
const TIME_ZONE: Range<u64> = 56..232;

fn read_at(file: &File, buf: &mut [u8], at: u64) -> io::Result<()> {
    let mut done = 0;
    while let Some(rest) = buf.get_mut(done..).filter(|rest| !rest.is_empty()) {
        match file.seek_read(rest, at + done as u64)? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            n => done += n,
        }
    }
    Ok(())
}

fn write_at(file: &File, buf: &[u8], at: u64) -> io::Result<()> {
    let mut done = 0;
    while let Some(rest) = buf.get(done..).filter(|rest| !rest.is_empty()) {
        match file.seek_write(rest, at + done as u64)? {
            0 => return Err(io::ErrorKind::WriteZero.into()),
            n => done += n,
        }
    }
    Ok(())
}

fn read_u32(file: &File, at: u64) -> io::Result<u32> {
    let mut bytes = [0u8; 4];
    read_at(file, &mut bytes, at)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Takes out of a written dump what names this machine: the folders of the modules' paths (a
/// module stays named by its file name, all a debugger needs) and the time zone; and says of a
/// thread whose stack was left out that it has none. Works in place on small pieces: nothing is
/// allocated, so the fault path may run it.
fn anonymize_dump(file: &File) -> io::Result<()> {
    if read_u32(file, 0)? != DUMP_SIGNATURE {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let streams = read_u32(file, 8)?;
    let directory = u64::from(read_u32(file, 12)?);
    for i in 0..u64::from(streams.min(4096)) {
        let entry = directory + i * DIRECTORY_ENTRY;
        let kind = read_u32(file, entry)?;
        let rva = u64::from(read_u32(file, entry + 8)?);
        match kind {
            THREAD_LIST => {
                let count = read_u32(file, rva)?;
                for t in 0..u64::from(count.min(65_536)) {
                    let stack = rva + 4 + t * THREAD_ENTRY + THREAD_STACK_AT;
                    if read_u32(file, stack + 4)? == 0 {
                        write_at(file, &0u32.to_le_bytes(), stack)?;
                    }
                }
            }
            MODULE_LIST => {
                let count = read_u32(file, rva)?;
                for m in 0..u64::from(count.min(65_536)) {
                    let name = read_u32(file, rva + 4 + m * MODULE_ENTRY + MODULE_NAME_AT)?;
                    keep_file_name(file, u64::from(name))?;
                }
            }
            UNLOADED_MODULE_LIST => {
                let header = u64::from(read_u32(file, rva)?);
                let size = u64::from(read_u32(file, rva + 4)?);
                let count = read_u32(file, rva + 8)?;
                for m in 0..u64::from(count.min(65_536)) {
                    let name = read_u32(file, rva + header + m * size + MODULE_NAME_AT)?;
                    keep_file_name(file, u64::from(name))?;
                }
            }
            MISC_INFO if read_u32(file, rva)? >= MISC_INFO_3_SIZE => {
                let flags = read_u32(file, rva + 4)?;
                write_at(file, &(flags & !MISC3_TIMEZONE).to_le_bytes(), rva + 4)?;
                let zeros = [0u8; (TIME_ZONE.end - TIME_ZONE.start) as usize];
                write_at(file, &zeros, rva + TIME_ZONE.start)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Cuts the `MINIDUMP_STRING` at `at` (its length in bytes, then UTF-16) to what follows its last
/// path separator, in place.
fn keep_file_name(file: &File, at: u64) -> io::Result<()> {
    const PIECE: usize = 512;
    let units = u64::from(read_u32(file, at)? / 2).min(32_768);
    let text = at + 4;
    let mut buf = [0u8; PIECE];
    // the last separator, read piece by piece
    let mut cut = None;
    let mut unit = 0;
    while unit < units {
        let take = (units - unit).min((PIECE / 2) as u64);
        let piece = buf.get_mut(..take as usize * 2).unwrap_or_default();
        read_at(file, piece, text + unit * 2)?;
        for (k, pair) in piece.as_chunks::<2>().0.iter().enumerate() {
            if is_separator(u16::from_le_bytes(*pair)) {
                cut = Some(unit + k as u64 + 1);
            }
        }
        unit += take;
    }
    let Some(cut) = cut else { return Ok(()) };
    // the file name moves to the front: each piece is read before anything at or past it is written
    let kept = units - cut;
    let mut moved = 0;
    while moved < kept {
        let take = (kept - moved).min((PIECE / 2) as u64);
        let piece = buf.get_mut(..take as usize * 2).unwrap_or_default();
        read_at(file, piece, text + (cut + moved) * 2)?;
        write_at(file, piece, text + moved * 2)?;
        moved += take;
    }
    let zeros = [0u8; PIECE];
    let mut cleared = kept;
    while cleared < units {
        let take = (units - cleared).min((PIECE / 2) as u64);
        write_at(file, zeros.get(..take as usize * 2).unwrap_or_default(), text + cleared * 2)?;
        cleared += take;
    }
    write_at(file, &((kept * 2) as u32).to_le_bytes(), at)
}

/// What the fault filter needs, set once before it is installed: the filter is called by the
/// system with no argument of ours.
struct FaultSetup {
    /// `<dir>\grizzly_crash_` in UTF-16.
    prefix: Vec<u16>,
    compact: bool,
    version: &'static str,
    /// The thread that installed the filter: `main` in a report.
    main_thread: u32,
}

static FAULT: OnceLock<FaultSetup> = OnceLock::new();
/// The thread whose fault is being reported, and the thread writing the report.
static FAULTING: AtomicU32 = AtomicU32::new(0);
static WRITER: AtomicU32 = AtomicU32::new(0);

const FAULT_FRAMES: usize = 96;
const REPORT_BYTES: usize = 16 * 1024;
const PATH_UNITS: usize = 32_768 + 64;
/// The writer's own stack: the faulting thread may have none left (a stack overflow).
const WRITER_STACK: usize = 1 << 20;
/// How long the faulting thread waits for its report before it ends the process anyway (a writer
/// that waits for a lock the faulting thread holds).
const WRITE_WAIT_MS: u32 = 30_000;

/// The fault as the filter caught it, handed to the writer thread on the filter's stack.
struct FaultJob {
    pointers: *const EXCEPTION_POINTERS,
    thread: u32,
    frames: [usize; FAULT_FRAMES],
    count: usize,
}

fn end_process() -> i32 {
    // The state is broken: no Rust destructor, `atexit` handler or thread-local teardown may run
    // on it (`std::process::exit` runs them). The process ends here, as a filter may end it.
    // SAFETY: the pseudo-handle of this process is always valid; TerminateProcess does not return
    // when it terminates the calling process.
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), CRASH_EXIT_CODE as u32);
    }
    // only reached when the system refused to end the process: the default handler ends it
    1
}

unsafe extern "system" fn fatal_filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
    let me = crate::current_thread_id();
    if FAULTING.compare_exchange(0, me, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        // a fault of the thread being reported or of its writer loses the report: end now; any
        // other thread waits for the first report to end the process
        if FAULTING.load(Ordering::SeqCst) != me && WRITER.load(Ordering::SeqCst) != me {
            // SAFETY: Sleep has no preconditions.
            unsafe { Sleep(INFINITE) };
        }
        return end_process();
    }
    if let Some(setup) = FAULT.get()
        && !pointers.is_null()
    {
        let mut job = FaultJob { pointers, thread: me, frames: [0; FAULT_FRAMES], count: 0 };
        // on this thread the walk goes from here through the dispatcher into the faulting frames
        // SAFETY: `frames` holds FAULT_FRAMES addresses, as many as the call may write; no hash is
        // asked for.
        let got = unsafe {
            RtlCaptureStackBackTrace(
                0,
                FAULT_FRAMES as u32,
                job.frames.as_mut_ptr().cast(),
                std::ptr::null_mut(),
            )
        };
        job.count = usize::from(got).min(FAULT_FRAMES);
        let param = std::ptr::from_ref(&job).cast::<c_void>();
        // SAFETY: the writer reads `job` only while this thread waits for it below, and when the
        // wait gives up the process ends before this frame goes; the start routine has the
        // signature the call expects.
        unsafe {
            match CreateThread(
                None,
                WRITER_STACK,
                Some(fault_writer),
                Some(param),
                STACK_SIZE_PARAM_IS_A_RESERVATION,
                None,
            ) {
                Ok(thread) => {
                    WaitForSingleObject(thread, WRITE_WAIT_MS);
                    let _ = CloseHandle(thread);
                }
                Err(_) => write_fault(&job, setup),
            }
        }
    }
    end_process()
}

unsafe extern "system" fn fault_writer(param: *mut c_void) -> u32 {
    WRITER.store(crate::current_thread_id(), Ordering::SeqCst);
    if let Some(setup) = FAULT.get()
        && !param.is_null()
    {
        // SAFETY: `param` is the filter's job, which outlives this thread's use of it (the filter
        // waits for this thread, or ends the process).
        let job = unsafe { &*param.cast::<FaultJob>() };
        write_fault(job, setup);
    }
    0
}

/// A report's text in a buffer of its own: what does not fit is cut, nothing is allocated.
struct Text {
    bytes: [u8; REPORT_BYTES],
    len: usize,
}

impl fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut take = s.len().min(REPORT_BYTES.saturating_sub(self.len));
        while take > 0 && !s.is_char_boundary(take) {
            take -= 1;
        }
        if let (Some(to), Some(from)) =
            (self.bytes.get_mut(self.len..self.len + take), s.as_bytes().get(..take))
        {
            to.copy_from_slice(from);
            self.len += take;
        }
        Ok(())
    }
}

impl Text {
    fn wide(&mut self, units: &[u16]) {
        for c in char::decode_utf16(units.iter().copied()) {
            let _ = self.write_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
    }

    /// `module+0xrva`, or `?` for an address in no module.
    fn frame(&mut self, address: u64) {
        let mut buf = [0u16; NAME_UNITS];
        match locate(address, &mut buf) {
            Some((base, name)) => {
                self.wide(buf.get(name).unwrap_or_default());
                let _ = write!(self, "+{:#x}", address.saturating_sub(base));
            }
            None => {
                let _ = self.write_char('?');
            }
        }
    }

    fn thread_name(&mut self, thread: u32, main: u32) {
        if thread == main {
            let _ = self.write_str("main");
            return;
        }
        let before = self.len;
        // SAFETY: the thread is opened only to read its description and closed again; the
        // description is a NUL-terminated string the system allocated, read before it is freed.
        unsafe {
            if let Ok(handle) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, thread) {
                if let Ok(name) = GetThreadDescription(handle)
                    && !name.is_null()
                {
                    self.wide(name.as_wide());
                    let _ = LocalFree(Some(HLOCAL(name.0.cast())));
                }
                let _ = CloseHandle(handle);
            }
        }
        if self.len == before {
            let _ = self.write_str("unnamed");
        }
    }
}

/// The name a report gives an exception code.
fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "EXCEPTION_ACCESS_VIOLATION",
        0xC000_00FD => "EXCEPTION_STACK_OVERFLOW",
        0xC000_001D => "EXCEPTION_ILLEGAL_INSTRUCTION",
        0xC000_0094 => "EXCEPTION_INT_DIVIDE_BY_ZERO",
        0xC000_0095 => "EXCEPTION_INT_OVERFLOW",
        0xC000_0374 => "STATUS_HEAP_CORRUPTION",
        0xC000_0409 => "STATUS_STACK_BUFFER_OVERRUN",
        0xC000_0096 => "EXCEPTION_PRIV_INSTRUCTION",
        0xC000_0006 => "EXCEPTION_IN_PAGE_ERROR",
        0xC000_0025 => "EXCEPTION_NONCONTINUABLE_EXCEPTION",
        0x8000_0002 => "EXCEPTION_DATATYPE_MISALIGNMENT",
        0x8000_0003 => "EXCEPTION_BREAKPOINT",
        _ => "UNKNOWN",
    }
}

/// `<prefix><seconds>_<pid>` with room for an extension and the NUL.
struct ReportPath {
    units: [u16; PATH_UNITS],
    len: usize,
}

impl ReportPath {
    fn new(prefix: &[u16]) -> Option<ReportPath> {
        let mut path = ReportPath { units: [0; PATH_UNITS], len: 0 };
        path.units.get_mut(..prefix.len())?.copy_from_slice(prefix);
        path.len = prefix.len();
        let seconds = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        path.number(seconds)?;
        path.push(u16::from(b'_'))?;
        // SAFETY: the call takes nothing and answers this process's id.
        path.number(u64::from(unsafe { GetCurrentProcessId() }))?;
        Some(path)
    }

    fn push(&mut self, unit: u16) -> Option<()> {
        *self.units.get_mut(self.len)? = unit;
        self.len += 1;
        Some(())
    }

    fn number(&mut self, mut value: u64) -> Option<()> {
        let mut digits = [0u16; 20];
        let mut n = 0;
        loop {
            *digits.get_mut(n)? = u16::from(b'0') + (value % 10) as u16;
            n += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for k in (0..n).rev() {
            self.push(*digits.get(k)?)?;
        }
        Some(())
    }

    /// Creates `<path>.<extension>`, replacing a file of that name.
    fn create(&mut self, extension: &str, read_back: bool) -> Option<File> {
        let base = self.len;
        let made = (|| {
            self.push(u16::from(b'.'))?;
            for unit in extension.encode_utf16() {
                self.push(unit)?;
            }
            self.push(0)?;
            let access = if read_back { GENERIC_READ.0 | GENERIC_WRITE.0 } else { GENERIC_WRITE.0 };
            // SAFETY: the name ends with the NUL pushed above and lives across the call.
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(self.units.as_ptr()),
                    access,
                    FILE_SHARE_READ,
                    None,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .ok()?;
            // SAFETY: the handle was just opened and nothing else holds it: the file owns it now.
            Some(unsafe { File::from_raw_handle(handle.0) })
        })();
        self.len = base;
        made
    }
}

/// Writes the fault's report and its dump beside it. Allocates nothing of its own.
fn write_fault(job: &FaultJob, setup: &FaultSetup) {
    // SAFETY: the system hands the filter exception pointers valid while it runs, and the filter
    // runs until this returns.
    let (code, address) = unsafe {
        let record = (*job.pointers).ExceptionRecord;
        if record.is_null() {
            (0, 0)
        } else {
            ((*record).ExceptionCode.0 as u32, (*record).ExceptionAddress as u64)
        }
    };
    let frames = job.frames.get(..job.count).unwrap_or_default();
    // the frames of the filter and the dispatcher end where the fault is
    let frames =
        frames.iter().position(|&ip| ip as u64 == address).and_then(|at| frames.get(at..)).unwrap_or(frames);

    // the same keys as the panic report of gz-crash
    let mut text = Text { bytes: [0; REPORT_BYTES], len: 0 };
    let _ = writeln!(text, "Grizzly Editor stopped on an internal error.");
    let _ = writeln!(text, "version: {}", setup.version);
    // a fault always ends the process
    let _ = writeln!(text, "kind: fatal");
    let _ = write!(text, "thread: ");
    text.thread_name(job.thread, setup.main_thread);
    let _ = writeln!(text);
    let _ = writeln!(text, "exception: {code:#010X} {}", exception_name(code));
    let mut buf = [0u16; NAME_UNITS];
    if let Some((base, name)) = locate(address, &mut buf) {
        let _ = write!(text, "module: ");
        text.wide(buf.get(name).unwrap_or_default());
        let _ = writeln!(text, "\nmodule_rva: {:#x}", address.saturating_sub(base));
    }
    let _ = writeln!(text, "\nframes:");
    for &ip in frames {
        text.frame(ip as u64);
        let _ = writeln!(text);
    }

    let Some(mut path) = ReportPath::new(&setup.prefix) else { return };
    if let Some(mut report) = path.create("txt", false) {
        let _ = report.write_all(text.bytes.get(..text.len).unwrap_or_default());
    }
    if let Some(dump) = path.create("dmp", setup.compact) {
        let _ = dump_to(&dump, Some((job.thread, job.pointers)), setup.compact);
    }
}

/// From now on an access violation or any other unhandled system exception - a fault inside a C
/// library or a driver - leaves `grizzly_crash_<unix seconds>_<pid>.txt` in `dir` (the report
/// gz-crash writes for a panic, with the exception instead of the message and no symbolised
/// stack) and a dump beside it (`compact` for one that leaves this machine), then ends the
/// process with [`CRASH_EXIT_CODE`]. The calling thread is `main` in a report. A second call
/// changes nothing.
pub fn install_fatal_exception_dump(dir: PathBuf, compact: bool, version: &'static str) {
    let _ = std::fs::create_dir_all(&dir);
    let mut prefix: Vec<u16> = dir.as_os_str().encode_wide().collect();
    if prefix.last().is_some_and(|&unit| !is_separator(unit)) {
        prefix.push(u16::from(b'\\'));
    }
    prefix.extend("grizzly_crash_".encode_utf16());
    // SAFETY: the call takes nothing and reads the calling thread's own id.
    let main_thread = unsafe { GetCurrentThreadId() };
    if FAULT.set(FaultSetup { prefix, compact, version, main_thread }).is_err() {
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
    use std::process::Command;

    use super::*;

    fn exe_name() -> String {
        let exe = std::env::current_exe().unwrap();
        exe.file_name().unwrap().to_string_lossy().into_owned()
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
    }

    fn word(data: &[u8], at: usize) -> usize {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
    }

    /// The bytes a dump's threads say their stacks hold, and the bytes of its memory list.
    fn stack_and_memory_bytes(data: &[u8]) -> (usize, usize) {
        let (count, directory) = (word(data, 8), word(data, 12));
        let (mut stacks, mut memory) = (0, 0);
        for i in 0..count {
            let (kind, at) = (word(data, directory + i * 12), word(data, directory + i * 12 + 8));
            if kind == THREAD_LIST as usize {
                stacks += (0..word(data, at)).map(|t| word(data, at + 4 + t * 48 + 32)).sum::<usize>();
            } else if kind == 5 {
                memory += (0..word(data, at)).map(|m| word(data, at + 4 + m * 16 + 8)).sum::<usize>();
            }
        }
        (stacks, memory)
    }

    #[inline(never)]
    fn frames_from_here() -> Vec<Frame> {
        stack_frames(0, 16)
    }

    #[test]
    fn the_stack_is_named_by_module_and_offset() {
        let frames = frames_from_here();
        assert!(frames.len() > 2, "{frames:?}");
        // the first frame is the caller's, in this very program
        let first = &frames[0];
        assert!(first.module.eq_ignore_ascii_case(&exe_name()), "{frames:?}");
        assert!(first.rva > 0 && first.address > first.rva, "{first:?}");
        assert_eq!(first.to_string(), format!("{}+{:#x}", first.module, first.rva));
        assert!(frames.iter().all(|f| !f.module.contains(['\\', '/'])), "{frames:?}");
        // skip and max are kept
        assert_eq!(stack_frames(0, 1).len(), 1);
        assert!(stack_frames(1000, 8).is_empty());
        assert_eq!(Frame { module: String::new(), rva: 0, address: 5 }.to_string(), "?");
    }

    #[test]
    fn an_address_names_its_module() {
        let here = an_address_names_its_module as fn() as usize as u64;
        let (module, rva) = module_of(here).unwrap();
        assert!(module.eq_ignore_ascii_case(&exe_name()), "{module}");
        assert!(rva > 0 && rva < here);
        // an address inside the system's own library, named without a folder
        // SAFETY: the name is a static wide string; the module stays loaded for the whole process.
        let ntdll =
            unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("ntdll")) };
        let base = ntdll.unwrap().0 as usize as u64;
        let (system, rva) = module_of(base + 0x1000).unwrap();
        assert!(system.to_ascii_lowercase().starts_with("ntdll."), "{system}");
        assert_eq!(rva, 0x1000);
        assert!(module_of(0).is_none());
        assert!(module_of(8).is_none());
    }

    #[test]
    fn a_compact_dump_is_much_smaller_and_names_no_folder() {
        let dir = std::env::temp_dir().join(format!("gz_win_dumps_{}", std::process::id()));
        let compact = dir.join("compact.dmp");
        let full = dir.join("full.dmp");
        write_minidump(&compact, true).unwrap();
        write_minidump(&full, false).unwrap();
        let compact_bytes = std::fs::read(&compact).unwrap();
        let full_bytes = std::fs::read(&full).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(compact_bytes.starts_with(b"MDMP") && full_bytes.starts_with(b"MDMP"));
        assert!(
            compact_bytes.len() * 3 < full_bytes.len(),
            "compact {} bytes, full {} bytes",
            compact_bytes.len(),
            full_bytes.len()
        );
        // no stack is in it: every thread says none, and the memory left is a few pages of the system's
        let (stacks, memory) = stack_and_memory_bytes(&compact_bytes);
        assert_eq!(stacks, 0);
        assert!(memory < 8 * 1024, "{memory} bytes of memory");
        let (full_stacks, full_memory) = stack_and_memory_bytes(&full_bytes);
        assert!(full_stacks > 0 && full_memory > full_stacks, "{full_stacks} {full_memory}");
        // the modules are named by their file names; the folder of this program is in the full dump only
        let exe = std::env::current_exe().unwrap();
        let folder = utf16(&exe.parent().unwrap().to_string_lossy());
        assert!(contains(&full_bytes, &folder));
        assert!(!contains(&compact_bytes, &folder));
        assert!(contains(&compact_bytes, &utf16(&exe_name())));
    }

    /// The child of the test below: installs the filter and raises an access violation nobody handles.
    const CHILD_DIR: &str = "GZ_WIN_FAULT_TEST_DIR";

    #[test]
    fn a_fault_leaves_a_report_and_a_compact_dump() {
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            install_fatal_exception_dump(PathBuf::from(dir), true, "0.0.0-test");
            // SAFETY: raising an exception is defined; this child process exists to leave it unhandled.
            unsafe { windows::Win32::System::Diagnostics::Debug::RaiseException(0xC000_0005, 0, None) };
            return;
        }
        let dir = std::env::temp_dir().join(format!("gz_win_fault_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["crash::tests::a_fault_leaves_a_report_and_a_compact_dump", "--exact", "--nocapture"])
            .env(CHILD_DIR, &dir)
            .output()
            .unwrap();
        let mut report = None;
        let mut dump = None;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("grizzly_crash_") && name.ends_with(".txt") {
                report = std::fs::read_to_string(entry.path()).ok();
            } else if name.starts_with("grizzly_crash_") && name.ends_with(".dmp") {
                dump = std::fs::read(entry.path()).ok();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(out.status.code(), Some(CRASH_EXIT_CODE), "{}", String::from_utf8_lossy(&out.stderr));
        let report = report.unwrap();
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines[0], "Grizzly Editor stopped on an internal error.", "{report}");
        assert_eq!(lines[1], "version: 0.0.0-test", "{report}");
        assert_eq!(lines[2], "kind: fatal", "{report}");
        assert!(lines[3].starts_with("thread: ") && lines[3].len() > 8, "{report}");
        assert_eq!(lines[4], "exception: 0xC0000005 EXCEPTION_ACCESS_VIOLATION", "{report}");
        let module = lines[5].strip_prefix("module: ").unwrap();
        assert!(!module.is_empty() && !module.contains(['\\', '/']), "{report}");
        assert!(lines[6].starts_with("module_rva: 0x"), "{report}");
        assert_eq!(lines[7], "", "{report}");
        assert_eq!(lines[8], "frames:", "{report}");
        // the frames start where the exception was raised and go down into this test
        assert!(lines[9].starts_with(&format!("{module}+0x")), "{report}");
        assert!(lines[9..].iter().any(|l| l.starts_with(&format!("{}+0x", exe_name()))), "{report}");
        let dump = dump.unwrap();
        assert!(dump.starts_with(b"MDMP"));
        let exe = std::env::current_exe().unwrap();
        assert!(!contains(&dump, &utf16(&exe.parent().unwrap().to_string_lossy())));
    }
}

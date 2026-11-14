//! Where another thread of this process is: its return addresses, and their names.
//!
//! For diagnostics a person asked for (`GRIZZLY_DEBUG_STALLS`): the stall monitor samples the UI
//! thread while a pass runs long, so the log says what it was waiting in. Reading a stack means
//! stopping its thread for a moment; nothing does that unasked.

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_FLAGS, GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind, SYMBOL_INFO,
    SYMOPT_DEFERRED_LOADS, SYMOPT_UNDNAME, SymFromAddr, SymInitialize, SymSetOptions, UNW_FLAG_NHANDLER,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
};

/// The control, integer and floating point registers of an x64 thread (`CONTEXT_FULL`).
const CONTEXT_FULL_AMD64: u32 = 0x0010_000B;
/// How many frames a sample keeps.
const FRAMES: usize = 40;

/// A `CONTEXT` where the system wants it: on a 16-byte boundary.
#[repr(C, align(16))]
struct Aligned(CONTEXT);

/// The id of the calling thread, for [`thread_stack`] asked from another one.
#[must_use]
pub fn current_thread_id() -> u32 {
    // SAFETY: the call takes nothing and reads the calling thread's own id.
    unsafe { GetCurrentThreadId() }
}

/// The return addresses of a thread of this process, innermost first; empty when the thread
/// cannot be stopped or read. Never call it for the calling thread. The thread stands still only
/// while its registers are read and unwound with the system's unwind tables - nothing is
/// allocated meanwhile, so a thread stopped inside an allocation cannot deadlock the caller.
#[must_use]
pub fn thread_stack(thread_id: u32) -> Vec<u64> {
    let mut ips: Vec<u64> = Vec::with_capacity(FRAMES);
    if thread_id == current_thread_id() {
        return ips;
    }
    // SAFETY: the handle is this process's own thread, opened for exactly what is done with it and
    // closed below; the thread is resumed on every path after a successful suspend; `ctx` is a
    // live, zeroed, correctly aligned `CONTEXT` the calls write and read; the stack word read for a
    // leaf frame lies at the stopped thread's own stack pointer, which is mapped while it stands.
    unsafe {
        let access = THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION;
        let Ok(thread) = OpenThread(access, false, thread_id) else { return ips };
        if SuspendThread(thread) == u32::MAX {
            let _ = CloseHandle(thread);
            return ips;
        }
        let mut ctx: Aligned = std::mem::zeroed();
        ctx.0.ContextFlags = CONTEXT_FLAGS(CONTEXT_FULL_AMD64);
        if GetThreadContext(thread, &mut ctx.0).is_ok() {
            for _ in 0..FRAMES {
                let rip = ctx.0.Rip;
                if rip == 0 {
                    break;
                }
                ips.push(rip);
                let mut base = 0u64;
                let entry = RtlLookupFunctionEntry(rip, &mut base, None);
                if entry.is_null() {
                    // a leaf function: its return address is the word at the stack pointer
                    if ctx.0.Rsp == 0 {
                        break;
                    }
                    ctx.0.Rip = *(ctx.0.Rsp as *const u64);
                    ctx.0.Rsp += 8;
                } else {
                    let mut data = std::ptr::null_mut();
                    let mut frame = 0u64;
                    let _ = RtlVirtualUnwind(
                        UNW_FLAG_NHANDLER,
                        base,
                        rip,
                        entry,
                        &mut ctx.0,
                        &mut data,
                        &mut frame,
                        None,
                    );
                }
            }
        }
        ResumeThread(thread);
        let _ = CloseHandle(thread);
    }
    ips
}

/// The names of code addresses of this process (`function+offset`; the address itself where no
/// symbol is known). The first call reads the symbols of the loaded programs, which takes a
/// moment: not for a thread that must stay on time.
#[must_use]
pub fn symbol_names(addresses: &[u64]) -> Vec<String> {
    let mut out = Vec::with_capacity(addresses.len());
    // SAFETY: the pseudo-handle of this process is always valid; the symbol record is a buffer
    // as large as its header says (the header plus `MaxNameLen` bytes), aligned for it, and only
    // the `NameLen` bytes the call wrote are read back.
    unsafe {
        let process = GetCurrentProcess();
        SymSetOptions(SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS);
        // a second initialisation fails and changes nothing: the first one stands
        let _ = SymInitialize(process, None, true);
        for ip in addresses {
            const NAME: usize = 512;
            let words = (std::mem::size_of::<SYMBOL_INFO>() + NAME) / 8 + 1;
            let mut buf = vec![0u64; words];
            let info = buf.as_mut_ptr().cast::<SYMBOL_INFO>();
            (*info).SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
            (*info).MaxNameLen = NAME as u32;
            let mut offset = 0u64;
            if SymFromAddr(process, *ip, Some(&mut offset), info).is_ok() {
                let len = ((*info).NameLen as usize).min(NAME);
                let name = std::slice::from_raw_parts((*info).Name.as_ptr().cast::<u8>(), len);
                out.push(format!("{}+{offset:#x}", String::from_utf8_lossy(name)));
            } else {
                out.push(format!("{ip:#x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn another_threads_stack_is_read_and_it_runs_on() {
        let (id_tx, id_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            id_tx.send(current_thread_id()).unwrap();
            // stands in a wait until it is told to go
            go_rx.recv().unwrap();
            7
        });
        let id = id_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let stack = thread_stack(id);
        assert!(stack.len() > 2, "{stack:?}");
        assert_eq!(symbol_names(&stack).len(), stack.len());
        // its own stack is never asked for
        assert!(thread_stack(current_thread_id()).is_empty());
        go_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), 7);
    }
}

//! A child process that ends with this one.
//!
//! `Grizzly-Cmd.exe` runs `Grizzly.exe` for a shell and waits for it. A window program gets no
//! Ctrl+C of the console and would outlive a twin that was ended, writing into a console nobody
//! waits on: a job object that kills on its close ties the two together.

use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::core::PCWSTR;

/// Makes `child` end when this process does, however it ends. `false` when Windows refuses (the
/// child then lives on by itself, as it would have without the call).
#[must_use]
pub fn end_with_this_process(child: &Child) -> bool {
    // SAFETY: no security attributes and no name: a new anonymous job, owned by this process.
    let Ok(job) = (unsafe { CreateJobObjectW(None, PCWSTR::null()) }) else { return false };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0);
    // SAFETY: `limits` is the structure the information class names, alive across the call, and
    // `size` is its size.
    let set = unsafe {
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const limits).cast(), size)
    };
    // SAFETY: `job` is the handle made above; the child's handle is open while `child` is borrowed.
    let assigned =
        set.is_ok() && unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) }.is_ok();
    // The job's handle is never closed here: Windows closes it when this process ends, and that
    // close is what ends the child.
    assigned
}

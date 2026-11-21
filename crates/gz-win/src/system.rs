//! What a bug report says of the machine it came from: the system, the processor, the memory.
//! Nothing here names the user, the machine or a folder of it.

use std::fmt::Write as _;

use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
use windows::Win32::System::SystemInformation::{
    ComputerNameDnsDomain, ComputerNameDnsHostname, ComputerNamePhysicalNetBIOS, GetComputerNameExW,
    GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use crate::registry::{read_dword, read_string};

/// The machine as a report describes it. Every field is best effort: empty or 0 when unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemInfo {
    /// `Windows 11 Pro 24H2 (build 26100.2605)`.
    pub os: String,
    /// The processor's name as its maker gives it.
    pub cpu: String,
    pub logical_cpus: u32,
    /// Physical memory, in bytes.
    pub ram_total: u64,
    pub ram_available: u64,
    /// `ram_total` in MiB.
    pub ram_mb: u64,
}

/// What this process holds of the memory, in MiB; 0 when unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    pub working_set_mb: u64,
    pub peak_working_set_mb: u64,
    /// The memory no other process shares (the commit charge).
    pub private_mb: u64,
}

const MB: u64 = 1024 * 1024;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const PROCESSOR: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

/// The system, the processor and the memory of this machine.
#[must_use]
pub fn system_info() -> SystemInfo {
    let (ram_total, ram_available) = memory_status();
    SystemInfo {
        os: os_name(),
        cpu: read_string(HKEY_LOCAL_MACHINE, PROCESSOR, "ProcessorNameString")
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
        logical_cpus: std::thread::available_parallelism()
            .map_or(0, |n| u32::try_from(n.get()).unwrap_or(u32::MAX)),
        ram_total,
        ram_available,
        ram_mb: ram_total / MB,
    }
}

/// The memory this process holds now.
#[must_use]
pub fn process_memory() -> ProcessMemory {
    let size = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size, ..Default::default() };
    // SAFETY: the pseudo-handle of this process is always valid; `counters` is a live local of the
    // size passed, the extended record the call fills when `cb` says so.
    let read = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    };
    if !read.as_bool() {
        return ProcessMemory::default();
    }
    ProcessMemory {
        working_set_mb: counters.WorkingSetSize as u64 / MB,
        peak_working_set_mb: counters.PeakWorkingSetSize as u64 / MB,
        private_mb: counters.PrivateUsage as u64 / MB,
    }
}

/// This computer's names - its DNS host name and its NetBIOS name - that a report must not carry:
/// what the sanitizer of a report takes out. Empty when the system does not say.
#[must_use]
pub fn computer_names() -> Vec<String> {
    let mut out = Vec::new();
    for kind in [ComputerNameDnsHostname, ComputerNamePhysicalNetBIOS, ComputerNameDnsDomain] {
        let mut size = 0_u32;
        // SAFETY: a null buffer with a size of 0 asks for the size the name needs; nothing is written.
        let _ = unsafe { GetComputerNameExW(kind, None, &raw mut size) };
        if size == 0 || size > 1024 {
            continue;
        }
        let mut buf = vec![0_u16; size as usize];
        // SAFETY: `buf` is a live buffer of `size` UTF-16 units, the size the call was told.
        if unsafe { GetComputerNameExW(kind, Some(windows::core::PWSTR(buf.as_mut_ptr())), &raw mut size) }
            .is_ok()
        {
            let name = String::from_utf16_lossy(&buf[..(size as usize).min(buf.len())]);
            if !name.trim().is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn memory_status() -> (u64, u64) {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a live MEMORYSTATUSEX with its length set; the call only fills it.
    match unsafe { GlobalMemoryStatusEx(&raw mut status) } {
        Ok(()) => (status.ullTotalPhys, status.ullAvailPhys),
        Err(_) => (0, 0),
    }
}

fn os_name() -> String {
    let read = |value| {
        read_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, value)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    let product = read("ProductName");
    let release = read("DisplayVersion").or_else(|| read("ReleaseId"));
    let build = read("CurrentBuild").or_else(|| read("CurrentBuildNumber"));
    let update = read_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "UBR");
    os_text(product.as_deref(), release.as_deref(), build.as_deref(), update)
}

/// `<product> <release> (build <build>.<update>)`, each part left out when unknown.
fn os_text(product: Option<&str>, release: Option<&str>, build: Option<&str>, update: Option<u32>) -> String {
    let mut out = product.unwrap_or_default().to_owned();
    // ProductName still says Windows 10 on Windows 11: only the build tells them apart
    if build.and_then(|b| b.parse::<u32>().ok()).is_some_and(|b| b >= 22_000)
        && let Some(edition) = out.strip_prefix("Windows 10")
    {
        out = format!("Windows 11{edition}");
    }
    let mut part = |text: std::fmt::Arguments| {
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = out.write_fmt(text);
    };
    if let Some(release) = release {
        part(format_args!("{release}"));
    }
    match (build, update) {
        (Some(build), Some(update)) => part(format_args!("(build {build}.{update})")),
        (Some(build), None) => part(format_args!("(build {build})")),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_is_described() {
        let info = system_info();
        assert!(info.os.starts_with("Windows"), "{info:?}");
        assert!(info.os.contains("(build "), "{info:?}");
        assert!(!info.cpu.is_empty(), "{info:?}");
        assert!(!info.cpu.contains("  ") && info.cpu == info.cpu.trim(), "{info:?}");
        assert!(info.logical_cpus > 0, "{info:?}");
        assert!(
            info.ram_total > 0 && info.ram_available > 0 && info.ram_available <= info.ram_total,
            "{info:?}"
        );
        assert_eq!(info.ram_mb, info.ram_total / MB);
        let memory = process_memory();
        assert!(memory.working_set_mb > 0 && memory.working_set_mb < info.ram_mb, "{memory:?}");
        assert!(memory.peak_working_set_mb >= memory.working_set_mb && memory.private_mb > 0, "{memory:?}");
    }

    #[test]
    fn windows_11_is_told_by_its_build() {
        assert_eq!(
            os_text(Some("Windows 10 Pro"), Some("24H2"), Some("26100"), Some(2605)),
            "Windows 11 Pro 24H2 (build 26100.2605)"
        );
        assert_eq!(
            os_text(Some("Windows 10 Home"), Some("22H2"), Some("19045"), Some(5737)),
            "Windows 10 Home 22H2 (build 19045.5737)"
        );
        assert_eq!(
            os_text(Some("Windows 10 Pro"), None, Some("22000"), None),
            "Windows 11 Pro (build 22000)"
        );
        assert_eq!(os_text(None, None, Some("26100"), Some(1)), "(build 26100.1)");
        assert_eq!(os_text(None, None, None, None), "");
    }
}

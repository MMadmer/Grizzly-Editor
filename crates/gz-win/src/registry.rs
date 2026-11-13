//! The one registry value the editor reads: whether Windows takes long paths.

use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

/// Whether Windows takes paths over `MAX_PATH` from a long-path aware program
/// (`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem` `LongPathsEnabled` is 1). A value that is
/// missing or cannot be read means no.
#[must_use]
pub fn long_paths_enabled() -> bool {
    let mut value = 0u32;
    // the size of a DWORD
    let mut size = 4u32;
    // SAFETY: the key and value names are static wide strings, `value` and `size` live across the
    // call, and RRF_RT_REG_DWORD lets the call write at most the four bytes `size` names.
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SYSTEM\CurrentControlSet\Control\FileSystem"),
            w!("LongPathsEnabled"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    read.is_ok() && value == 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_value_agrees_with_what_reg_exe_reads() {
        // program-files: in memory
        let out = std::process::Command::new("reg.exe")
            .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "/v", "LongPathsEnabled"])
            .output();
        let Ok(out) = out else { return };
        let text = String::from_utf8_lossy(&out.stdout);
        let on = out.status.success() && text.split_whitespace().last() == Some("0x1");
        assert_eq!(super::long_paths_enabled(), on, "{text}");
    }
}

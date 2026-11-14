//! A file operation `std` lacks: removing a file without touching its read-only mark.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileDispositionInfoEx, OPEN_EXISTING, SetFileInformationByHandle,
};
use windows::core::PCWSTR;

/// Removes the file `path` names - a hard link of it, or a link itself, never what a link points
/// to - whether it is marked read-only or not, and without clearing the mark: the mark belongs to
/// the file, which every hard link of it shares, so clearing it to remove one link would change
/// the others (a game's own file that a mirror links to).
pub fn remove_file_keeping_marks(path: &Path) -> io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let err = |e: windows::core::Error| io::Error::from_raw_os_error(e.code().0 & 0xFFFF);
    // SAFETY: `wide` is a zero-terminated wide string alive for the call; no security attributes
    // and no template file; the handle is checked by `?` and closed below on every path.
    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(wide.as_ptr()),
            DELETE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(err)?;
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    // SAFETY: the handle is open with DELETE access; `info` is a FILE_DISPOSITION_INFO_EX that
    // lives for the call, and the size passed is its own.
    let set = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    }
    .map_err(err);
    // SAFETY: the handle was opened above and is closed once; the file goes when it closes.
    let _ = unsafe { CloseHandle(handle) };
    set
}

#[cfg(test)]
mod tests {
    use super::remove_file_keeping_marks;

    #[test]
    fn a_read_only_hard_link_goes_and_the_other_keeps_its_mark() {
        let dir = std::env::temp_dir().join(format!("gz-win-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.ltx"), dir.join("b.ltx"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        let mut perm = std::fs::metadata(&a).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&a, perm).unwrap();
        remove_file_keeping_marks(&b).unwrap();
        assert!(!b.exists());
        assert!(std::fs::metadata(&a).unwrap().permissions().readonly(), "the other link keeps its mark");
        let mut perm = std::fs::metadata(&a).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&a, perm).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

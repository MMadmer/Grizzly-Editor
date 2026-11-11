//! Where the user's folders are and what the volumes hold: the known-folder API, free space,
//! volume mount points.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData, FOLDERID_Profile,
    FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_Windows,
    KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
};
use windows::core::{GUID, PCWSTR};

/// The folders of the shell the editor asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownFolder {
    Profile,
    Documents,
    Desktop,
    Downloads,
    LocalAppData,
    RoamingAppData,
    Windows,
    ProgramFiles,
    ProgramFilesX86,
}

impl KnownFolder {
    fn id(self) -> GUID {
        match self {
            Self::Profile => FOLDERID_Profile,
            Self::Documents => FOLDERID_Documents,
            Self::Desktop => FOLDERID_Desktop,
            Self::Downloads => FOLDERID_Downloads,
            Self::LocalAppData => FOLDERID_LocalAppData,
            Self::RoamingAppData => FOLDERID_RoamingAppData,
            Self::Windows => FOLDERID_Windows,
            Self::ProgramFiles => FOLDERID_ProgramFiles,
            Self::ProgramFilesX86 => FOLDERID_ProgramFilesX86,
        }
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Where the shell keeps `folder` for this user - redirected to OneDrive or another drive
/// included. `None` when the shell does not know it.
#[must_use]
pub fn known_folder(folder: KnownFolder) -> Option<PathBuf> {
    let id = folder.id();
    // SAFETY: `id` is a live GUID; the call allocates the answer, which is read up to its
    // terminating zero and then freed with the allocator the shell used, exactly once.
    unsafe {
        let p = SHGetKnownFolderPath(&id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        if p.is_null() {
            return None;
        }
        let text = OsString::from_wide(p.as_wide());
        CoTaskMemFree(Some(p.0.cast_const().cast()));
        (!text.is_empty()).then(|| PathBuf::from(text))
    }
}

/// The bytes the calling user may still write on the volume that holds `dir` (an existing
/// folder).
pub fn free_space(dir: &Path) -> io::Result<u64> {
    let name = wide(dir);
    let mut free = 0u64;
    // SAFETY: `name` is a zero-terminated wide string alive for the call; `free` is a live u64.
    unsafe { GetDiskFreeSpaceExW(PCWSTR::from_raw(name.as_ptr()), Some(&mut free), None, None) }
        .map_err(io::Error::other)?;
    Ok(free)
}

/// The root of the volume `path` lies on, with its trailing backslash: `D:\` for a drive, the
/// mount point's folder for a volume mounted into a folder.
pub fn volume_path_name(path: &Path) -> io::Result<PathBuf> {
    let name = wide(path);
    let mut buf = vec![0u16; name.len().max(260) + 1];
    // SAFETY: `name` is zero-terminated and alive for the call; `buf` is a live buffer whose
    // length the binding passes along.
    unsafe { GetVolumePathNameW(PCWSTR::from_raw(name.as_ptr()), &mut buf) }.map_err(io::Error::other)?;
    let len = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(OsString::from_wide(buf.get(..len).unwrap_or(&[]))))
}

/// Whether `path` is the root of a volume: a drive, or a folder a volume is mounted into.
#[must_use]
pub fn is_volume_root(path: &Path) -> bool {
    let Ok(root) = volume_path_name(path) else { return false };
    let trim = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    !trim(&root).is_empty() && trim(&root) == trim(path)
}

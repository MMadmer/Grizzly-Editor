//! The file-system facts the checks and the operations read: attributes, links, folder
//! listings read in full, Win32 error codes for the messages.

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

pub(crate) const READONLY: u32 = 0x1;
pub(crate) const DIRECTORY: u32 = 0x10;

pub(crate) const ERROR_FILE_NOT_FOUND: u32 = 2;
pub(crate) const ERROR_PATH_NOT_FOUND: u32 = 3;
pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
pub(crate) const ERROR_NOT_SAME_DEVICE: u32 = 17;
pub(crate) const ERROR_SHARING_VIOLATION: u32 = 32;
pub(crate) const ERROR_LOCK_VIOLATION: u32 = 33;
pub(crate) const ERROR_DIR_NOT_EMPTY: u32 = 145;

/// The Win32 code of an error, for the "(win32 N)" of a message.
pub(crate) fn win32(e: &io::Error) -> u32 {
    if let Some(code) = e.raw_os_error().and_then(|c| u32::try_from(c).ok()) {
        return code;
    }
    match e.kind() {
        io::ErrorKind::NotFound => ERROR_FILE_NOT_FOUND,
        io::ErrorKind::PermissionDenied => ERROR_ACCESS_DENIED,
        io::ErrorKind::DirectoryNotEmpty => ERROR_DIR_NOT_EMPTY,
        _ => 0,
    }
}

/// `what (win32 N)`.
pub(crate) fn win32_text(what: &str, code: u32) -> String {
    format!("{what} (win32 {code})")
}

#[cfg(windows)]
pub(crate) fn attributes(md: &Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes()
}

#[cfg(not(windows))]
pub(crate) fn attributes(md: &Metadata) -> u32 {
    let mut a = 0;
    if md.is_dir() {
        a |= DIRECTORY;
    }
    if md.permissions().readonly() {
        a |= READONLY;
    }
    a
}

/// A junction or a symbolic link points at somebody's real folder. Other reparse points - a
/// OneDrive placeholder - are ordinary files and folders (std's `is_symlink` is true for name
/// surrogates only).
pub(crate) fn is_link(md: &Metadata) -> bool {
    md.file_type().is_symlink()
}

/// One entry of a folder listing.
pub(crate) struct DirItem {
    pub name: String,
    pub attributes: u32,
    pub link: bool,
    pub size: u64,
    pub meta: Metadata,
}

impl DirItem {
    pub fn is_folder(&self) -> bool {
        self.attributes & DIRECTORY != 0
    }
}

/// A folder's entries, read in full before anything in it is touched. `Err(code)` when the
/// listing does not reach its end: a folder read half way hides what is there. A name that is
/// not valid Unicode is skipped.
pub(crate) fn list_folder(dir: &Path) -> Result<Vec<DirItem>, u32> {
    let rd = fs::read_dir(dir).map_err(|e| win32(&e))?;
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| win32(&e))?;
        let Ok(name) = entry.file_name().into_string() else { continue };
        // on Windows this comes from the listing itself: no file is opened for it
        let meta = entry.metadata().map_err(|e| win32(&e))?;
        out.push(DirItem {
            attributes: attributes(&meta),
            link: is_link(&meta),
            size: if meta.is_file() { meta.len() } else { 0 },
            name,
            meta,
        });
    }
    Ok(out)
}

/// The read-only mark goes first (git keeps its objects read-only) - on the entry itself, never
/// on what a link points at.
pub(crate) fn make_writable(path: &Path, meta: &Metadata, link: bool) {
    if attributes(meta) & READONLY == 0 {
        return;
    }
    let mut perm = meta.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    if !link {
        let _ = fs::set_permissions(path, perm);
        return;
    }
    set_link_permissions(path, perm);
}

#[cfg(windows)]
fn set_link_permissions(path: &Path, perm: fs::Permissions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    if let Ok(file) = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        let _ = file.set_permissions(perm);
    }
}

#[cfg(not(windows))]
fn set_link_permissions(_path: &Path, _perm: fs::Permissions) {}

/// Clears the read-only mark of whatever `path` is, the way `SetFileAttributes(NORMAL)` did
/// before a copy or a delete.
pub(crate) fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let link = is_link(&meta);
        make_writable(path, &meta, link);
    }
}

/// Whether `path` is a folder: `GetFileAttributes` with the directory bit (a junction has it).
pub(crate) fn dir_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| attributes(&m) & DIRECTORY != 0)
}

/// Whether `path` is a file.
pub(crate) fn file_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| attributes(&m) & DIRECTORY == 0)
}

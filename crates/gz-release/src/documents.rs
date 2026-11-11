//! The user's Documents folder, where the C++ editor's `SHGetKnownFolderPath(FOLDERID_Documents)`
//! finds it - redirected to OneDrive or another drive included. `%USERPROFILE%\Documents` when the
//! shell does not answer.

/// The Documents folder of this Windows user.
#[must_use]
pub fn documents_folder() -> String {
    #[cfg(windows)]
    if let Some(path) = gz_win::known_folder(gz_win::KnownFolder::Documents)
        && let Some(path) = path.to_str()
    {
        return path.trim_end_matches('\\').to_owned();
    }
    format!("{}\\Documents", std::env::var("USERPROFILE").unwrap_or_default())
}

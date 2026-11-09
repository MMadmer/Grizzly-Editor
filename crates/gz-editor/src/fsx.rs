//! File helpers every area uses. Blocking: call them on the IO pool.

use std::io;
use std::path::Path;

/// Writes a file so that a reader never sees half of it: the bytes go to a sibling temporary
/// file that then takes the target's place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".gz~");
    let tmp = Path::new(&tmp);
    std::fs::write(tmp, bytes)?;
    match std::fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(e)
        }
    }
}

/// Reads a file that may be absent: `Ok(None)` when it does not exist.
pub fn read_opt(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a text file as UTF-8 (a BOM is dropped); a file that is not UTF-8 is read as cp1251.
pub fn read_text(path: &Path) -> io::Result<String> {
    Ok(gz_base::text::decode_guess(&std::fs::read(path)?).into_owned())
}

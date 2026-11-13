//! The registry: whether Windows takes long paths, where the installed game is, and the file
//! association of the project file - all under the current user, nothing machine-wide written.

use std::io;
use std::path::Path;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW,
    RegSetValueExW,
};
use windows::core::{HSTRING, w};

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

/// A string value (`""` names a key's default value); `None` when the key or the value is not
/// there or is no string.
fn read_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (key, value) = (HSTRING::from(key), HSTRING::from(value));
    let mut size = 0u32;
    // SAFETY: the names are wide strings that live across the call; no buffer is passed, so the
    // call only writes the size it needs into `size`.
    let probe = unsafe { RegGetValueW(root, &key, &value, RRF_RT_REG_SZ, None, None, Some(&raw mut size)) };
    if probe != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut bytes = u32::try_from(buf.len() * 2).ok()?;
    // SAFETY: `buf` holds `bytes` bytes and lives across the call; RRF_RT_REG_SZ writes at most
    // that many and says how many it wrote in `bytes`.
    let read = unsafe {
        RegGetValueW(
            root,
            &key,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let len = (bytes as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(buf.get(..len).unwrap_or_default());
    Some(text.trim_end_matches('\0').to_owned())
}

/// The game folders the installer of Dead Air: Refined recorded: its uninstall entry's
/// `InstallLocation`, the current user's first, then the machine's (the installer writes one of
/// the two, by how it was run).
#[must_use]
pub fn installed_game_roots() -> Vec<String> {
    const ENTRY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{9732DFF1-E40D-4B23-B215-6D28B1DD0DE0}_is1";
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(|root| read_string(root, ENTRY, "InstallLocation"))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Writes `data` as the string value `value` of `HKCU\<key>`, the key made when it is missing.
fn write_string(key: &str, value: &str, data: &str) -> io::Result<()> {
    let name = HSTRING::from(key);
    let mut hkey = HKEY::default();
    // SAFETY: the key name lives across the call and `hkey` receives the opened key.
    let made = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &name,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &raw mut hkey,
            None,
        )
    };
    if made != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(made.0 as i32));
    }
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
    let value = HSTRING::from(value);
    // SAFETY: `hkey` was opened above with write access; the data is a NUL-terminated UTF-16
    // string as REG_SZ wants it, alive across the call.
    let set = unsafe { RegSetValueExW(hkey, &value, None, REG_SZ, Some(&bytes)) };
    // SAFETY: `hkey` is the key opened above, closed once.
    let _ = unsafe { RegCloseKey(hkey) };
    if set != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(set.0 as i32));
    }
    Ok(())
}

/// A file type of the current user: `<base>\<extension>` names the `<base>\<prog_id>` whose
/// icon is the program's and whose open command passes it the file. `base` is
/// `Software\Classes` for real; a test gives a key of its own.
pub struct Association<'a> {
    pub base: &'a str,
    /// With the dot: `.grizzly`.
    pub extension: &'a str,
    pub prog_id: &'a str,
    /// What Explorer calls the type.
    pub description: &'a str,
    pub program: &'a Path,
}

impl Association<'_> {
    /// The command line that opens a file of the type: `"<program>" "%1"`.
    #[must_use]
    pub fn command(&self) -> String {
        format!("\"{}\" \"%1\"", self.program.display())
    }

    /// The program the extension's open command starts now, when the association is there at
    /// all: `None` when the extension or its type is missing.
    #[must_use]
    pub fn registered_program(&self) -> Option<String> {
        let prog_id = read_string(HKEY_CURRENT_USER, &format!(r"{}\{}", self.base, self.extension), "")?;
        let command =
            read_string(HKEY_CURRENT_USER, &format!(r"{}\{prog_id}\shell\open\command", self.base), "")?;
        let command = command.trim();
        let program = match command.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or_default(),
            None => command.split(' ').next().unwrap_or_default(),
        };
        Some(program.to_owned())
    }

    /// Writes the extension, the type, its icon and its open command, then tells the shell that
    /// associations changed (for the real `Software\Classes` only).
    pub fn register(&self) -> io::Result<()> {
        let base = self.base;
        let program = self.program.display().to_string();
        write_string(&format!(r"{base}\{}", self.extension), "", self.prog_id)?;
        write_string(&format!(r"{base}\{}", self.prog_id), "", self.description)?;
        write_string(&format!(r"{base}\{}\DefaultIcon", self.prog_id), "", &format!("\"{program}\",0"))?;
        write_string(&format!(r"{base}\{}\shell\open\command", self.prog_id), "", &self.command())?;
        if base.eq_ignore_ascii_case(r"Software\Classes") {
            use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
            // SAFETY: SHCNE_ASSOCCHANGED takes no items; both item pointers are null.
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
        }
        Ok(())
    }
}

/// Deletes the key `HKCU\<key>` with everything below it: a test's own key, nothing else.
pub fn delete_user_key(key: &str) -> io::Result<()> {
    if key.is_empty() || key.eq_ignore_ascii_case(r"Software\Classes") || !key.contains('\\') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "refusing to delete that key"));
    }
    let name = HSTRING::from(key);
    // SAFETY: the key name lives across the call; the tree goes under the current user's hive.
    let done = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &name) };
    if done != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(done.0 as i32));
    }
    // RegDeleteTreeW empties the key; the key itself goes with the parent's call
    let (parent, leaf) = key.rsplit_once('\\').unwrap_or(("", key));
    let parent = HSTRING::from(parent);
    let leaf = HSTRING::from(leaf);
    let mut hkey = HKEY::default();
    use windows::Win32::System::Registry::{RegDeleteKeyW, RegOpenKeyExW};
    // SAFETY: the parent's name lives across the call and `hkey` receives the opened key.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &parent, None, KEY_WRITE, &raw mut hkey) } == ERROR_SUCCESS {
        // SAFETY: `hkey` was opened above; the leaf name lives across the call.
        let _ = unsafe { RegDeleteKeyW(hkey, &leaf) };
        // SAFETY: `hkey` is the key opened above, closed once.
        let _ = unsafe { RegCloseKey(hkey) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The association is written under a key of the test's own, never `Software\Classes`.
    #[test]
    fn an_association_is_written_and_read_back_under_a_test_key() {
        let base = format!(r"Software\GrizzlyEditorTest-{}\Classes", std::process::id());
        let program = Path::new(r"D:\Tools\Grizzly Editor\grizzly-test-program");
        let a = Association {
            base: &base,
            extension: ".grizzlytest",
            prog_id: "GrizzlyEditor.TestProject",
            description: "Grizzly Editor Project",
            program,
        };
        assert_eq!(a.registered_program(), None);
        a.register().unwrap();
        assert_eq!(a.registered_program().as_deref(), Some(r"D:\Tools\Grizzly Editor\grizzly-test-program"));
        assert_eq!(
            read_string(
                HKEY_CURRENT_USER,
                &format!(r"{base}\GrizzlyEditor.TestProject\shell\open\command"),
                ""
            )
            .as_deref(),
            Some("\"D:\\Tools\\Grizzly Editor\\grizzly-test-program\" \"%1\"")
        );
        let top = base.trim_end_matches(r"\Classes");
        delete_user_key(top).unwrap();
        assert_eq!(a.registered_program(), None);
        assert!(delete_user_key(r"Software\Classes").is_err());
    }
}

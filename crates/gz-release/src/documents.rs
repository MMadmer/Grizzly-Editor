//! The user's Documents folder, where the C++ editor's `SHGetKnownFolderPath(FOLDERID_Documents)`
//! finds it - redirected to OneDrive or another drive included - without a line of `unsafe`:
//! the shell keeps it as `Personal` under `HKCU\...\Explorer\User Shell Folders`, and `reg export`
//! writes that key as UTF-16 text, so a Cyrillic user name comes through whole. Asked once per
//! session (the caller keeps the answer); `%USERPROFILE%\Documents` when the registry does not
//! answer.

use std::path::Path;

const SHELL_FOLDERS: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";

/// `%NAME%` replaced by the environment's value; a name the environment lacks stays as written.
#[must_use]
pub fn expand_env(text: &str, get: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('%') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let after = rest.get(open + 1..).unwrap_or("");
        match after.find('%') {
            Some(close) => {
                let name = after.get(..close).unwrap_or("");
                match get(name).filter(|_| !name.is_empty()) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = after.get(close + 1..).unwrap_or("");
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The value of `name` in the text of a `.reg` export: `"name"="text"` (REG_SZ, `\\` and `\"`
/// escaped) or `"name"=hex(2):..` (REG_EXPAND_SZ, UTF-16LE bytes, lines continued by `\`).
#[must_use]
pub fn reg_value(export: &str, name: &str) -> Option<String> {
    let head = format!("\"{name}\"=");
    let joined = export.replace("\\\r\n", "").replace("\\\n", "");
    let line = joined.lines().find_map(|l| l.trim_start().strip_prefix(head.as_str()))?.trim_end();
    if let Some(quoted) = line.strip_prefix('"').and_then(|l| l.strip_suffix('"')) {
        let mut out = String::with_capacity(quoted.len());
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        return Some(out);
    }
    let hex = line.strip_prefix("hex(2):")?;
    let bytes: Vec<u8> = hex.split(',').map(str::trim).filter(|b| !b.is_empty()).map(|b| u8::from_str_radix(b, 16).ok()).collect::<Option<_>>()?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).take_while(|&u| u != 0).collect();
    String::from_utf16(&units).ok()
}

fn utf16_file(bytes: &[u8]) -> Option<String> {
    let body = bytes.strip_prefix(&[0xFF, 0xFE])?;
    let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
    String::from_utf16(&units).ok()
}

#[cfg(windows)]
fn shell_folder(name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let file = std::env::temp_dir().join(format!("grizzly_shell_folders_{}.reg", std::process::id()));
    let status = std::process::Command::new("reg")
        .args(["export", SHELL_FOLDERS])
        .arg(&file)
        .arg("/y")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .ok()?;
    let bytes = std::fs::read(&file).ok();
    let _ = std::fs::remove_file(&file);
    if !status.success() {
        return None;
    }
    reg_value(&utf16_file(&bytes?)?, name)
}

#[cfg(not(windows))]
fn shell_folder(_name: &str) -> Option<String> {
    None
}

/// The Documents folder of this Windows user.
#[must_use]
pub fn documents_folder() -> String {
    let env = |n: &str| std::env::var(n).ok();
    if let Some(value) = shell_folder("Personal") {
        let path = expand_env(&value, &env);
        if !path.is_empty() && !path.contains('%') && Path::new(&path).is_absolute() {
            return path.trim_end_matches('\\').to_owned();
        }
    }
    format!("{}\\Documents", env("USERPROFILE").unwrap_or_default())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reg_export_text() {
        // "%USERPROFILE%\Документы" as REG_EXPAND_SZ, continued over two lines
        let text: Vec<u16> = "%USERPROFILE%\\\u{414}\u{43e}\u{43a}".encode_utf16().chain([0]).collect();
        let hex: Vec<String> = text.iter().flat_map(|u| u.to_le_bytes()).map(|b| format!("{b:02x}")).collect();
        let (a, b) = hex.split_at(10);
        let export = format!("Windows Registry Editor Version 5.00\r\n\r\n[{SHELL_FOLDERS}]\r\n\"Desktop\"=\"C:\\\\Users\\\\x\\\\Desktop\"\r\n\"Personal\"=hex(2):{},\\\r\n  {}\r\n", a.join(","), b.join(","));
        let value = reg_value(&export, "Personal").unwrap();
        assert_eq!(value, "%USERPROFILE%\\\u{414}\u{43e}\u{43a}");
        assert_eq!(reg_value(&export, "Desktop").unwrap(), "C:\\Users\\x\\Desktop");
        assert_eq!(reg_value(&export, "Music"), None);
        let get = |n: &str| (n == "USERPROFILE").then(|| "C:\\Users\\\u{418}".to_owned());
        assert_eq!(expand_env(&value, &get), "C:\\Users\\\u{418}\\\u{414}\u{43e}\u{43a}");
        assert_eq!(expand_env("%NOPE%\\x %", &get), "%NOPE%\\x %");
    }
}

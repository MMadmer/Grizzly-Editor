//! Path text: how a game path is keyed, what is nobody's content, what a game folder is.

use std::path::Path;

/// The key of a game path: forward slashes, lower case, `.` and `..` resolved by their text,
/// no leading or trailing separator. `Configs\Text\..\System.ltx` is `configs/system.ltx`.
#[must_use]
pub fn normalize(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split(['\\', '/']) {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let mut out = String::with_capacity(path.len());
    for (i, s) in segs.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&s.to_lowercase());
    }
    out
}

/// A path's text with `.` and `..` resolved and backslashes, case kept: what two spellings of
/// one archive path are compared by.
pub(crate) fn lexical(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split(['\\', '/']) {
        match seg {
            "" | "." => {}
            ".." => {
                if segs.len() > 1 {
                    segs.pop();
                }
            }
            s => segs.push(s),
        }
    }
    let mut out = segs.join("\\");
    if path.starts_with("\\\\") {
        out.insert_str(0, "\\\\");
    }
    out
}

/// What the Windows shell leaves in folders it has shown.
const SHELL_LITTER: [&str; 4] = ["Thumbs.db", "ehthumbs.db", "ehthumbs_vista.db", "desktop.ini"];

/// Tooling: version control, IDE and OS litter (`.git`, `Thumbs.db`, `~$` locks). The game
/// neither mounts nor lists it, so neither does the index, at any depth.
#[must_use]
pub fn is_tooling_entry(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.starts_with('.')
        || name.starts_with("~$")
        || SHELL_LITTER.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// `Ok` when `game_root` looks like an X-Ray game: the folder, `fsgame.ltx`, and `database\` or
/// `gamedata\`. The reason otherwise is a sentence of the game check.
pub fn looks_like_game(game_root: &str) -> Result<(), &'static str> {
    if game_root.is_empty() {
        return Err("no folder selected");
    }
    let root = Path::new(game_root);
    if !root.is_dir() {
        return Err("folder does not exist");
    }
    if !root.join("fsgame.ltx").is_file() {
        return Err("no fsgame.ltx - not an X-Ray game folder");
    }
    if !root.join("database").is_dir() && !root.join("gamedata").is_dir() {
        return Err("neither database\\ nor gamedata\\ is present");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(normalize("Configs\\Text\\..\\System.ltx"), "configs/system.ltx");
        assert_eq!(normalize("/a//b/./c/"), "a/b/c");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("..\\x"), "x");
        assert_eq!(normalize("Тексты\\Файл.XML"), "тексты/файл.xml");
    }

    #[test]
    fn lexical_paths() {
        assert_eq!(lexical("D:\\Game\\database\\.\\a.db0"), "D:\\Game\\database\\a.db0");
        assert_eq!(lexical("D:/Game/x/../database/"), "D:\\Game\\database");
        assert_eq!(lexical("D:\\.."), "D:");
    }

    #[test]
    fn tooling() {
        assert!(is_tooling_entry(".git"));
        assert!(is_tooling_entry("thumbs.db"));
        assert!(is_tooling_entry("~$doc.docx"));
        assert!(!is_tooling_entry("configs"));
        assert!(!is_tooling_entry(""));
    }

    #[test]
    fn game_folders() {
        assert_eq!(looks_like_game(""), Err("no folder selected"));
        assert_eq!(looks_like_game("Z:\\surely\\not\\here"), Err("folder does not exist"));
    }
}

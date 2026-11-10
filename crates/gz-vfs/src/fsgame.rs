//! `fsgame.ltx`: where the game's loose data and its archives are (`CLocatorAPI::_initialize`).
//!
//! A line is `$alias$ = recurse | notif | root | add | ext | caption`; `root` is an alias
//! defined above it or a folder, `add` is appended to it. `$fs_root$` is the game folder.
//!
//! oracle: the engine registers every `.db*` / `.xdb*` it meets while scanning ANY alias. Here
//! the archives are those of the `$arch_dir...$` aliases - the engine's own convention for
//! archive folders - in the order the file names them, each folder by name, so the loose tree,
//! the saves and the logs are not walked for archives a second time. Without such an alias it is
//! `database\`, as the C++ editor's DARF mount read it.

use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use crate::path::lexical;

/// Where a game keeps its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    /// The loose tree (`$game_data$`).
    pub game_data: PathBuf,
    /// Archive folders in mount order; `true` when the folder is scanned recursively.
    pub archive_dirs: Vec<(PathBuf, bool)>,
}

/// `CInifile::IsBOOL`.
fn is_bool(v: &str) -> bool {
    ["on", "yes", "true", "1"].iter().any(|t| t.eq_ignore_ascii_case(v))
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with("\\\\") || (b.len() >= 2 && b.get(1) == Some(&b':'))
}

/// The layout `fsgame.ltx` (its text, decoded) describes for the game at `root`.
pub(crate) fn layout(root: &Path, fsgame: Option<&str>) -> Layout {
    let mut fs_root = root.to_string_lossy().into_owned();
    if !fs_root.ends_with(['\\', '/']) {
        fs_root.push('\\');
    }
    let mut aliases: FxHashMap<String, String> = FxHashMap::default();
    aliases.insert("$fs_root$".to_owned(), fs_root.clone());
    let mut game_data: Option<String> = None;
    let mut archive_dirs: Vec<(PathBuf, bool)> = Vec::new();

    for raw in fsgame.unwrap_or("").lines() {
        let line = raw.trim_matches([' ', '\t', '\r']);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((id, rest)) = line.split_once('=') else { continue };
        let id = id.trim_matches([' ', '\t']).to_ascii_lowercase();
        // `_GetItem(buf, 1, .., '=')`: the text up to a second '='
        let value = rest.split('=').next().unwrap_or("");
        let items: Vec<&str> = value.split('|').map(|s| s.trim_matches([' ', '\t'])).collect();
        if items.len() < 3 {
            continue;
        }
        let recurse = items.first().is_some_and(|v| is_bool(v));
        let base = items.get(2).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let add = items.get(3).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let mut path = match aliases.get(&base) {
            Some(p) => p.clone(),
            None if is_absolute(&base) => base.clone(),
            None => format!("{fs_root}{base}"),
        };
        path.push_str(&add);
        if id == "$game_data$" {
            game_data = Some(path.clone());
        }
        if id.starts_with("$arch_dir") {
            let dir = PathBuf::from(lexical(&path));
            if !archive_dirs.iter().any(|(d, _)| d.as_os_str().eq_ignore_ascii_case(dir.as_os_str())) {
                archive_dirs.push((dir, recurse));
            }
        }
        aliases.entry(id).or_insert(path);
    }
    if archive_dirs.is_empty() {
        archive_dirs.push((root.join("database"), false));
    }
    let game_data = game_data.map_or_else(|| root.join("gamedata"), |p| PathBuf::from(lexical(&p)));
    Layout { game_data, archive_dirs }
}

fn is_archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.rfind('.').and_then(|d| lower.get(d..)).is_some_and(|ext| ext.starts_with(".db") || ext.starts_with(".xdb"))
}

#[cfg(windows)]
fn hidden(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn hidden(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The archives of the folders in mount order: within a folder by name, a sub-folder walked
/// where it falls in that order (`CLocatorAPI::Recurse`); empty and hidden files are none.
pub(crate) fn archives(dirs: &[(PathBuf, bool)]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (dir, recurse) in dirs {
        collect(dir, *recurse, 0, &mut out);
    }
    out
}

fn collect(dir: &Path, recurse: bool, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, std::fs::Metadata)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let meta = e.metadata().ok()?;
            Some((name, meta))
        })
        .collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    for (name, meta) in entries {
        if hidden(&meta) || crate::path::is_tooling_entry(&name) {
            continue;
        }
        let path = dir.join(&name);
        if meta.is_dir() {
            if recurse && depth < 16 {
                collect(&path, true, depth + 1, out);
            }
        } else if meta.len() > 0 && is_archive_name(&name) && !out.iter().any(|p| p.as_os_str().eq_ignore_ascii_case(path.as_os_str())) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEAD_AIR: &str = ";abbreviation = recurs|notif|root|add|ext|description\r\n\
$app_data_root$ = true| false| appdata\\\r\n\
$arch_dir$ = false| false| $fs_root$| database\\\r\n\
$arch_dir_config$ = false| false| $arch_dir$| .\\\r\n\
$arch_dir_sound$ = false| false| $arch_dir$| sound\\\r\n\
$game_data$ = true| true| $fs_root$| gamedata\\\r\n\
$game_config$ = true| false| $game_data$| configs\\\r\n";

    #[test]
    fn dead_air_layout() {
        let l = layout(Path::new("D:\\Games\\Dead Air"), Some(DEAD_AIR));
        assert_eq!(l.game_data, PathBuf::from("D:\\Games\\Dead Air\\gamedata"));
        assert_eq!(
            l.archive_dirs,
            vec![(PathBuf::from("D:\\Games\\Dead Air\\database"), false), (PathBuf::from("D:\\Games\\Dead Air\\database\\sound"), false)]
        );
    }

    #[test]
    fn fallbacks() {
        let l = layout(Path::new("C:\\G"), None);
        assert_eq!(l.game_data, PathBuf::from("C:\\G").join("gamedata"));
        assert_eq!(l.archive_dirs, vec![(PathBuf::from("C:\\G").join("database"), false)]);
        let l = layout(Path::new("C:\\G"), Some("$arch_dir$ = true| false| E:\\packs\\\n$bad = x\n"));
        assert_eq!(l.archive_dirs, vec![(PathBuf::from("E:\\packs"), true)]);
    }

    #[test]
    fn archive_names() {
        assert!(is_archive_name("configs.XDB0"));
        assert!(is_archive_name("a.db"));
        assert!(!is_archive_name("thumbs"));
        assert!(!is_archive_name("readme.txt"));
    }
}

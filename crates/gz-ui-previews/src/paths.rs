//! Where the editor keeps what these tabs read: its own `gamedata` and `rawdata` (what `fs.ltx`
//! names them), the open project, and how a folder is named to the author.

use std::path::{Path, PathBuf};

use gz_editor::Editor;

/// The editor's folders, as `fs.ltx` names them.
#[derive(Clone, Debug)]
pub struct EditorDirs {
    /// `$fs_root$`: the editor's data root.
    pub root: PathBuf,
    /// `$game_data$`.
    pub gamedata: PathBuf,
    /// `$game_meshes$`.
    pub meshes: PathBuf,
    /// `$game_levels$` (`$level$`).
    pub levels: PathBuf,
    /// `$game_textures$`: the game textures the library builds (`.dds`).
    pub game_textures: PathBuf,
    /// `$game_sounds$`: the game sounds (`.ogg`).
    pub game_sounds: PathBuf,
    /// `$textures$`: the texture sources (`.tga` and their `.thm`).
    pub textures: PathBuf,
    /// `$sounds$`: the sound sources (`.wav` and their `.thm`).
    pub sounds: PathBuf,
    /// `$objects$`: the object library.
    pub objects: PathBuf,
    /// `$import$`: where Check New Textures looks.
    pub import: PathBuf,
    /// `$game_saves$`.
    pub saves: PathBuf,
}

impl EditorDirs {
    /// The editor's library (Preferences > Library) for what it reads; the editor's own data
    /// root for where it imports and keeps saves.
    #[must_use]
    pub fn of(editor: &Editor) -> Self {
        let mut dirs = Self::at(&gz_project::library_root(editor));
        dirs.import = editor.paths.root.join("import");
        dirs.saves = editor.paths.root.join("savedgames");
        dirs
    }

    #[must_use]
    pub fn at(root: &Path) -> Self {
        let gamedata = root.join("gamedata");
        let rawdata = root.join("rawdata");
        Self {
            root: root.to_path_buf(),
            meshes: gamedata.join("meshes"),
            levels: gamedata.join("levels"),
            game_textures: gamedata.join("textures"),
            game_sounds: gamedata.join("sounds"),
            textures: rawdata.join("textures"),
            sounds: rawdata.join("sounds"),
            objects: rawdata.join("objects"),
            import: root.join("import"),
            saves: root.join("savedgames"),
            gamedata,
        }
    }

    /// `lanims.xr`: the light animation library.
    #[must_use]
    pub fn lanims(&self) -> PathBuf {
        self.gamedata.join("lanims.xr")
    }
}

/// A name as the engine's file system keys it: back slashes, no doubled separator (an escaped
/// `a\\b` reaches a handler with both), no leading separator.
#[must_use]
pub fn engine_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.trim().chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && (out.is_empty() || out.ends_with('\\')) {
            continue;
        }
        out.push(c);
    }
    out
}

/// A path for a JSON answer: forward slashes.
#[must_use]
pub fn json_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The same for a path held as text.
#[must_use]
pub fn json_path_str(path: &str) -> String {
    path.replace('\\', "/")
}

/// The last part of a name, its folders left out.
#[must_use]
pub fn leaf(name: &str) -> &str {
    name.rsplit(['\\', '/']).next().unwrap_or(name)
}

/// The leaf without its extension: what a tab of a file is called.
#[must_use]
pub fn label_of(name: &str) -> String {
    let l = leaf(name);
    match l.rfind('.') {
        Some(dot) if dot > 0 => l[..dot].to_owned(),
        _ => l.to_owned(),
    }
}

/// `name` without its extension; an extension is only the part after the last dot of the leaf.
#[must_use]
pub fn strip_ext(name: &str) -> &str {
    let start = name.rfind(['\\', '/']).map_or(0, |i| i + 1);
    match name[start..].rfind('.') {
        Some(dot) => &name[..start + dot],
        None => name,
    }
}

/// A folder of the open project or of the editor is named by where it sits
/// in it ("the editor's rawdata\textures folder"), anything else by its whole path.
#[must_use]
pub fn folder_text(project: Option<&Path>, editor_root: &Path, folder: &Path) -> String {
    let text = folder.to_string_lossy();
    let inside = |root: &Path, owner: &str| -> Option<String> {
        let r = root.to_string_lossy();
        let r = r.trim_end_matches(['\\', '/']);
        if r.is_empty()
            || text.len() <= r.len()
            || !text.is_char_boundary(r.len())
            || !text[..r.len()].eq_ignore_ascii_case(r)
        {
            return None;
        }
        // the root ends at a separator: C:\Mod is not a folder of C:\Mo
        let rest = &text[r.len()..];
        if !rest.starts_with(['\\', '/']) {
            return None;
        }
        Some(format!("the {owner} {} folder", rest.trim_start_matches(['\\', '/'])))
    };
    if let Some(p) = project
        && let Some(t) = inside(p, "project's")
    {
        return t;
    }
    inside(editor_root, "editor's").unwrap_or_else(|| text.into_owned())
}

/// `EFS.AppendFolderToName(name, 1, TRUE)`: a texture's prefix up to its first `_` becomes its
/// folder (`wpn_ak74` is `wpn\wpn_ak74`); a name without one stays as it is.
#[must_use]
pub fn texture_folder_name(name: &str) -> String {
    match name.find('_') {
        Some(i) => format!("{}\\{name}", &name[..i]),
        None => name.to_owned(),
    }
}

/// The modification time of a file, in whole seconds since 1970 (the engine's file age).
#[must_use]
pub fn file_age(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}

/// `FS.set_file_age`: the file's modification time set to `age` seconds since 1970.
pub fn set_file_age(path: &Path, age: i64) -> std::io::Result<()> {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(age.max(0)).unwrap_or(0));
    std::fs::OpenOptions::new().write(true).open(path)?.set_modified(at)
}

/// Every file below `dir` whose extension is one of `exts` (lower case, no dot), as the engine's
/// file system lists it: lower case, back slashes, relative to `dir`, the extension kept or cut.
/// `root_only` lists the folder itself and nothing below it. Tooling entries are left out.
#[must_use]
pub fn list_files(dir: &Path, exts: &[&str], root_only: bool, clamp_ext: bool) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, prefix)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if gz_vfs::is_tooling_entry(&name) {
                continue;
            }
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                if !root_only && !ty.is_symlink() {
                    stack.push((e.path(), format!("{prefix}{}\\", name.to_lowercase())));
                }
                continue;
            }
            let lower = name.to_lowercase();
            let Some(dot) = lower.rfind('.') else { continue };
            if !exts.contains(&&lower[dot + 1..]) {
                continue;
            }
            let key =
                if clamp_ext { format!("{prefix}{}", &lower[..dot]) } else { format!("{prefix}{lower}") };
            out.push((key, e.path()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(engine_name("dynamics//box\\\\box_1a"), "dynamics\\box\\box_1a");
        assert_eq!(engine_name("/meshes/a.ogf"), "meshes\\a.ogf");
        assert_eq!(leaf("a\\b\\c.ogf"), "c.ogf");
        assert_eq!(label_of("a\\b\\c.ogf"), "c");
        assert_eq!(label_of(".hidden"), ".hidden");
        assert_eq!(strip_ext("a.b\\c.dds"), "a.b\\c");
        assert_eq!(strip_ext("a.b\\c"), "a.b\\c");
        assert_eq!(texture_folder_name("wpn_ak74"), "wpn\\wpn_ak74");
        assert_eq!(texture_folder_name("grass"), "grass");
    }

    #[test]
    fn folders_are_named_by_where_they_sit() {
        let editor = Path::new("D:\\Ed");
        assert_eq!(
            folder_text(None, editor, Path::new("D:\\Ed\\rawdata\\textures")),
            "the editor's rawdata\\textures folder"
        );
        assert_eq!(
            folder_text(Some(Path::new("C:\\Mod")), editor, Path::new("C:\\Mod\\import")),
            "the project's import folder"
        );
        // C:\Mod is not a folder of C:\Mo
        assert_eq!(folder_text(Some(Path::new("C:\\Mo")), editor, Path::new("C:\\Mod\\x")), "C:\\Mod\\x");
    }
}

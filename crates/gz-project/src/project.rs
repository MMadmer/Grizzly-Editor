//! `<Name>.grizzly`: the project file at the root of a project folder (`docs/assets/PROJECT.md`).
//!
//! Lua data like every asset of the editor: the header `grizzly = 1`, the project's `id` (a UUID
//! made once), the `scene` that opens with it (project-relative, `/`) and the `library` list -
//! what the scenes take from the editor's library. It is the author's file: shipped in git, never
//! exported. It is read and written as a lossless document, so the author's comments, unknown keys
//! and layout survive every save, and a file the editor did not change is never rewritten.

use std::path::{Path, PathBuf};

use gz_luadata::{DiffRules, LuaDoc, Table, Value};

use crate::paths;

/// The extension of the project file, with the dot.
pub const EXTENSION: &str = ".grizzly";
/// The header key of the kind.
pub const HEADER: &str = "grizzly";
/// The format version this editor writes.
pub const VERSION: u32 = 1;

/// Why a project file cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

/// The project file, kept as read and as it is now: saving writes only the difference.
#[derive(Clone, Debug)]
pub struct ProjectFile {
    /// The text as read; `None` for a file not written yet.
    doc: Option<LuaDoc>,
    read: Value,
    now: Value,
}

fn rules() -> DiffRules {
    DiffRules::new().order("", &[HEADER, "id", "scene", "library"])
}

/// Sets `key`: in place when it is there, else after the keys already written (the rules place
/// it among the file's own keys on save).
fn assign(table: &mut Value, key: &str, value: Value) {
    if table.has(key) {
        table.set(key, value);
    } else {
        table.put(key, value);
    }
}

impl ProjectFile {
    /// A new project file for the project `id`.
    #[must_use]
    pub fn fresh(id: &str) -> Self {
        let mut now = Value::table();
        now.put(HEADER, Value::Number(f64::from(VERSION)));
        now.put("id", id);
        Self { doc: None, read: Value::Nil, now }
    }

    /// Reads a project file. A newer version is refused with the sentence the author reads; a
    /// file without a header or an id is taken, and gets them on its next save.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let (doc, value) = LuaDoc::parse_value(text, "grizzly", gz_luadata::Limits::asset())
            .map_err(|e| FileError { line: e.line, column: e.column, message: e.message })?;
        if !value.is_table() {
            return Err(FileError { line: 1, column: 1, message: "a project file is return { ... }".into() });
        }
        if let Some(v) = value.get(HEADER).and_then(Value::as_number)
            && v > f64::from(VERSION)
        {
            let span = doc.span(&[gz_luadata::Seg::Key(HEADER.into())]).unwrap_or_default();
            return Err(FileError {
                line: span.line,
                column: span.column,
                message: format!(
                    "the project file was made by a newer Grizzly Editor (format {v}; this one reads up to {VERSION}): update the editor to open it"
                ),
            });
        }
        let mut now = value.clone();
        if !now.has(HEADER) {
            now.put(HEADER, Value::Number(f64::from(VERSION)));
        }
        Ok(Self { doc: Some(doc), read: value, now })
    }

    /// The project's id; empty when the file has none (the open gives it one).
    #[must_use]
    pub fn id(&self) -> &str {
        self.now.get("id").and_then(Value::as_str).unwrap_or_default()
    }

    #[must_use]
    pub fn with_id(mut self, id: &str) -> Self {
        assign(&mut self.now, "id", Value::from(id));
        self
    }

    /// The scene that opens with the project, project-relative with `/`.
    #[must_use]
    pub fn scene(&self) -> Option<&str> {
        self.now.get("scene").and_then(Value::as_str).filter(|s| !s.is_empty())
    }

    /// The scene to open next time; `None` leaves the file's own choice.
    #[must_use]
    pub fn with_scene(mut self, rel: Option<&str>) -> Self {
        if let Some(rel) = rel.filter(|r| !r.is_empty()) {
            let rel = rel.replace('\\', "/");
            if self.scene() != Some(rel.as_str()) {
                assign(&mut self.now, "scene", Value::from(rel));
            }
        }
        self
    }

    /// The file without its scene to open (one that names a place outside the project).
    #[must_use]
    pub fn without_scene(mut self) -> Self {
        self.now.erase("scene");
        self
    }

    /// What the scenes take from the editor's library, as the library spells it.
    #[must_use]
    pub fn library(&self) -> Vec<String> {
        self.now
            .get("library")
            .map(|v| v.arr().iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The library list; an empty one goes from the file unless the file names it already.
    #[must_use]
    pub fn with_library(mut self, refs: &[String]) -> Self {
        if self.library() == refs {
            return self;
        }
        if refs.is_empty() {
            self.now.erase("library");
        } else {
            let list: Vec<Value> = refs.iter().map(|r| Value::from(r.as_str())).collect();
            assign(&mut self.now, "library", Value::from(list));
        }
        self
    }

    /// The file has changes not written yet (a new file always has).
    #[must_use]
    pub fn changed(&self) -> bool {
        self.doc.is_none() || !gz_luadata::data_eq(&self.read, &self.now)
    }

    /// The text to write: the file as read with only what changed rewritten, or a new file in
    /// the canonical layout.
    pub fn to_text(&self) -> Result<String, FileError> {
        match &self.doc {
            Some(doc) => {
                let mut doc = doc.clone();
                doc.apply_diff(&self.read, &self.now, &rules()).map_err(|e| FileError {
                    line: e.line,
                    column: e.column,
                    message: e.message,
                })?;
                Ok(doc.into_text())
            }
            None => {
                let top = self.now.as_table().cloned().unwrap_or_else(Table::new);
                Ok(gz_luadata::write_asset("", &top))
            }
        }
    }

    /// The file as it was just written: the next save starts from it.
    #[must_use]
    pub fn written(&self, text: &str) -> Self {
        Self::parse(text).unwrap_or_else(|_| self.clone())
    }

    /// What is wrong with the file of the project at `root`: the header, the id, the scene.
    #[must_use]
    pub fn problems(&self, root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if !self.read.has(HEADER) && self.doc.is_some() {
            out.push(format!("the project file has no '{HEADER} = {VERSION}' header"));
        }
        let id = self.id();
        if id.is_empty() {
            out.push("the project file has no id".to_owned());
        } else if !is_uuid(id) {
            out.push(format!("the project's id '{id}' is not a UUID"));
        }
        if let Some(scene) = self.scene()
            && std::fs::symlink_metadata(root.join(paths::to_backslash(scene))).is_err()
        {
            out.push(format!("the scene '{scene}' the project opens with does not exist"));
        }
        out
    }
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, hex digits.
#[must_use]
pub fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A new random UUID (version 4). The bits come from the standard library's hasher keys, which
/// the operating system's random source seeds, mixed with the time.
#[must_use]
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let half = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(salt);
        h.write_u32(std::process::id());
        h.finish()
    };
    let (a, b) = (half(1), half(2));
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// Whether `name` is a project file's name.
#[must_use]
pub fn is_project_file_name(name: &str) -> bool {
    name.len() > EXTENSION.len()
        && name.get(name.len() - EXTENSION.len()..).is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// The project files at the root of `folder`, sorted by name. One read of the folder.
#[must_use]
pub fn project_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| e.file_name().to_str().is_some_and(is_project_file_name))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// The folder a path given to open a project names: the folder itself, the folder of a project
/// file, or the folder of a `mod.ltx` (or an old `project.ltx`). `None` for anything else.
#[must_use]
pub fn project_folder_of(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        return Some(path.to_path_buf());
    }
    let name = path.file_name()?.to_str()?;
    let manifest = is_project_file_name(name)
        || name.eq_ignore_ascii_case("mod.ltx")
        || name.eq_ignore_ascii_case(crate::legacy::FILE);
    manifest.then(|| path.parent().map(Path::to_path_buf)).flatten()
}

/// `file` lies inside the project at `root`: its path below the root, with backslashes. As
/// spelled first, then where both really are (a junction, an 8.3 name). A relative `file` is
/// read against the root.
#[must_use]
pub fn scene_in_project(root: &str, file: &str) -> Option<String> {
    if file.is_empty() || root.is_empty() {
        return None;
    }
    let file = if paths::is_absolute(file) {
        paths::to_backslash(file)
    } else {
        paths::join(root, &paths::to_backslash(file))
    };
    for real in [false, true] {
        let (path, base) = if real {
            (paths::real_path(&file), paths::real_path(root))
        } else {
            (paths::full_path(&file), paths::full_path(root))
        };
        let (Some(path), Some(base)) = (path, base) else { continue };
        if path.len() <= base.len() || !paths::same_or_below(&path, &base) {
            continue;
        }
        let skip = base.len() + usize::from(!base.ends_with('\\'));
        let rel = path.get(skip..).unwrap_or_default();
        return (!rel.is_empty()).then(|| rel.to_owned());
    }
    None
}

/// The source scenes of the editor's library (`<library>\rawdata\levels\*.level`): names only,
/// no extension, sorted. Reachable by name because opening a project points the scene folder at
/// the project.
#[must_use]
pub fn base_scenes(library_root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(library_root.join("rawdata").join("levels")) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| {
            let dot = n.rfind('.')?;
            let (stem, ext) = (n.get(..dot)?, n.get(dot..)?);
            (ext.eq_ignore_ascii_case(".level") && !stem.is_empty()).then(|| stem.to_owned())
        })
        .collect();
    out.sort();
    out
}

/// Copies the base scene `name` (`<name>.level` and its folder) from the library into
/// `<project>\rawdata\levels`, and answers the project-side `.level`. A scene the project has
/// already is the author's copy: nothing is copied, the same answer.
pub fn import_base_scene(library_root: &Path, project_root: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() {
        return Err("no scene name".into());
    }
    let src_dir = library_root.join("rawdata").join("levels");
    let dst_root = project_root.join("rawdata").join("levels");
    let dst_file = dst_root.join(format!("{name}.level"));
    if std::fs::symlink_metadata(&dst_file).is_ok() {
        return Ok(dst_file);
    }
    let src_file = src_dir.join(format!("{name}.level"));
    if std::fs::symlink_metadata(&src_file).is_err() {
        return Err(format!("no such scene in the editor library: {name}"));
    }
    let copied = std::fs::create_dir_all(&dst_root).and_then(|()| std::fs::copy(&src_file, &dst_file));
    if copied.is_err() {
        return Err(format!("cannot copy the scene into the project: {name}"));
    }
    let folder = src_dir.join(name);
    if folder.is_dir() {
        copy_tree(&folder, &dst_root.join(name));
    }
    Ok(dst_file)
}

fn copy_tree(src: &Path, dst: &Path) {
    if std::fs::create_dir_all(dst).is_err() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(src) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let to = dst.join(e.file_name());
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            copy_tree(&e.path(), &to);
        } else {
            let _ = std::fs::copy(e.path(), to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHORED: &str = "-- my mod's project\r\nreturn {\r\n  grizzly = 1,\r\n  id = \"7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11\", -- never change\r\n  notes = { \"keep me\" },\r\n}\r\n";

    #[test]
    fn a_file_nobody_changed_writes_back_byte_for_byte() {
        let f = ProjectFile::parse(AUTHORED).unwrap();
        assert!(!f.changed());
        assert_eq!(f.to_text().unwrap(), AUTHORED);
        assert_eq!(f.id(), "7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11");
        assert_eq!(f.scene(), None);
        assert!(f.library().is_empty());
    }

    #[test]
    fn an_edit_keeps_comments_and_unknown_keys() {
        let f = ProjectFile::parse(AUTHORED)
            .unwrap()
            .with_scene(Some("levels\\escape.scene"))
            .with_library(&["props\\box_wood_01".to_owned()]);
        assert!(f.changed());
        let text = f.to_text().unwrap();
        assert!(text.starts_with("-- my mod's project\r\n"), "{text}");
        assert!(text.contains("-- never change"), "{text}");
        assert!(text.contains("notes = { \"keep me\" }"), "{text}");
        let again = ProjectFile::parse(&text).unwrap();
        assert_eq!(again.scene(), Some("levels/escape.scene"));
        assert_eq!(again.library(), ["props\\box_wood_01"]);
        // the same values again change nothing
        let same = again.clone().with_scene(Some("levels/escape.scene")).with_library(&again.library());
        assert!(!same.changed());
        // the list emptied goes from the file
        let emptied = again.with_library(&[]).to_text().unwrap();
        assert!(!emptied.contains("library"), "{emptied}");
    }

    #[test]
    fn a_new_file_is_canonical_and_a_newer_one_is_refused() {
        let f = ProjectFile::fresh("7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11").with_scene(Some("a.scene"));
        let text = f.to_text().unwrap();
        assert!(text.starts_with("return {\n  grizzly = 1,\n  id = \"7c1e6c1e"), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap().scene(), Some("a.scene"));
        let newer = ProjectFile::parse("return { grizzly = 2, id = \"x\" }").unwrap_err();
        assert!(newer.message.contains("newer Grizzly Editor"), "{newer}");
        assert!(ProjectFile::parse("return { id = nope }").is_err());
        // no header: taken, and the save adds it
        let bare = ProjectFile::parse("return {\n  id = \"a\",\n}\n").unwrap();
        assert!(bare.to_text().unwrap().contains("grizzly = 1"));
    }

    #[test]
    fn ids_are_uuids() {
        let a = new_id();
        let b = new_id();
        assert!(is_uuid(&a) && is_uuid(&b), "{a} {b}");
        assert_ne!(a, b);
        assert_eq!(a.as_bytes()[14], b'4');
        assert!(!is_uuid("not-a-uuid"));
    }

    #[test]
    fn the_folder_of_what_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Mine.grizzly"), "return { grizzly = 1 }").unwrap();
        std::fs::write(root.join("mod.ltx"), "[module]").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        assert_eq!(project_folder_of(root).as_deref(), Some(root));
        assert_eq!(project_folder_of(&root.join("Mine.grizzly")).as_deref(), Some(root));
        assert_eq!(project_folder_of(&root.join("mod.ltx")).as_deref(), Some(root));
        assert_eq!(project_folder_of(&root.join("notes.txt")), None);
        assert_eq!(project_files(root), [root.join("Mine.grizzly")]);
    }

    #[test]
    fn scenes_inside_only() {
        assert_eq!(
            scene_in_project("D:\\P", "D:\\P\\rawdata\\levels\\a.level").as_deref(),
            Some("rawdata\\levels\\a.level")
        );
        assert_eq!(
            scene_in_project("D:\\P", "rawdata/levels/a.level").as_deref(),
            Some("rawdata\\levels\\a.level")
        );
        assert_eq!(scene_in_project("D:\\P", "D:\\Other\\a.level"), None);
        assert_eq!(scene_in_project("D:\\P", "D:\\P2\\a.level"), None);
        assert_eq!(scene_in_project("D:\\P", "D:\\P"), None);
    }
}

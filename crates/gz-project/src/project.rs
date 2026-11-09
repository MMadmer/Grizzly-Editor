//! `project.ltx`: this machine's bookkeeping of a project - its name, the editor and the game it
//! is linked to, where it builds to and the scene to reopen. Never part of the module.

use gz_ltx::LtxDoc;

use crate::{ini, paths};

pub const FILE: &str = "project.ltx";

/// `project.ltx`, edited without losing a byte it does not change.
#[derive(Clone, Debug, Default)]
pub struct ProjectFile {
    doc: LtxDoc,
}

impl ProjectFile {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self { doc: LtxDoc::parse(text) }
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        ini::get(&self.doc, "project", "name").unwrap_or_default()
    }

    /// The editor root the project was last opened with (`sdk_root`).
    #[must_use]
    pub fn sdk_root(&self) -> &str {
        ini::get(&self.doc, "project", "sdk_root").unwrap_or_default()
    }

    /// The linked game's folder, without trailing separators; empty when none is linked.
    #[must_use]
    pub fn game_root(&self) -> &str {
        ini::get(&self.doc, "project", "game_root").unwrap_or_default().trim_end_matches(['\\', '/'])
    }

    /// `[xms] export_target`: where Build Into Game writes.
    #[must_use]
    pub fn export_target(&self) -> &str {
        ini::get(&self.doc, "xms", "export_target").unwrap_or_default()
    }

    /// `[project] last_scene` as stored; `None` when the key is not there at all.
    #[must_use]
    pub fn last_scene(&self) -> Option<&str> {
        ini::get(&self.doc, "project", "last_scene")
    }

    /// What every open writes: the name, the editor root and the game link.
    #[must_use]
    pub fn with_manifest(&self, name: &str, sdk_root: &str, game_root: &str) -> Self {
        let mut doc = self.doc.clone();
        ini::set(&mut doc, "project", "name", name);
        ini::set(&mut doc, "project", "sdk_root", sdk_root);
        ini::set(&mut doc, "project", "game_root", game_root);
        Self { doc }
    }

    /// Any key, for the areas that keep theirs here (`[xms] export_target`).
    #[must_use]
    pub fn with_value(&self, section: &str, key: &str, value: &str) -> Self {
        let mut doc = self.doc.clone();
        ini::set(&mut doc, section, key, value);
        Self { doc }
    }

    /// Any key as stored, the outer quotes off.
    #[must_use]
    pub fn value(&self, section: &str, key: &str) -> Option<&str> {
        ini::get(&self.doc, section, key)
    }

    #[must_use]
    pub fn with_game_root(&self, game_root: &str) -> Self {
        let mut doc = self.doc.clone();
        ini::set(&mut doc, "project", "game_root", game_root);
        Self { doc }
    }

    /// The scene to reopen, project-relative. Nothing to reopen in a project that never named
    /// a scene leaves its file as it was: no empty key is added.
    #[must_use]
    pub fn with_last_scene(&self, rel: Option<&str>) -> Self {
        let rel = rel.unwrap_or_default();
        if rel.is_empty() && self.last_scene().is_none() {
            return self.clone();
        }
        let mut doc = self.doc.clone();
        ini::set(&mut doc, "project", "last_scene", rel);
        Self { doc }
    }
}

/// `file` lies inside the project at `root`: its path below the root, with backslashes. As
/// spelled first, then where both really are (a junction, an 8.3 name). A relative `file` is
/// read against the root.
#[must_use]
pub fn scene_in_project(root: &str, file: &str) -> Option<String> {
    if file.is_empty() || root.is_empty() {
        return None;
    }
    let file = if paths::is_absolute(file) { paths::to_backslash(file) } else { paths::join(root, &paths::to_backslash(file)) };
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

/// The source scenes the editor ships (`<editor root>\rawdata\levels\*.level`): names only, no
/// extension, sorted. Reachable by name because opening a project points the scene folder at
/// the project.
#[must_use]
pub fn base_scenes(editor_root: &std::path::Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(editor_root.join("rawdata").join("levels")) else { return Vec::new() };
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

/// Copies the base scene `name` (`<name>.level` and its folder) into
/// `<project>\rawdata\levels`, and answers the project-side `.level`. A scene the project has
/// already is the author's copy: nothing is copied, the same answer.
pub fn import_base_scene(editor_root: &std::path::Path, project_root: &std::path::Path, name: &str) -> Result<std::path::PathBuf, String> {
    if name.is_empty() {
        return Err("no scene name".into());
    }
    let src_dir = editor_root.join("rawdata").join("levels");
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

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
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

    const SAMPLE: &str = "[project]\r\nname=Delivery\r\nsdk_root=D:\\Editor\\\r\ngame_root=D:\\Games\\DA\\\r\n\r\n[xms]\r\nexport_target=D:\\Out\r\n";

    #[test]
    fn reads_what_the_editor_needs() {
        let f = ProjectFile::parse(SAMPLE);
        assert_eq!(f.name(), "Delivery");
        assert_eq!(f.sdk_root(), "D:\\Editor\\");
        assert_eq!(f.game_root(), "D:\\Games\\DA");
        assert_eq!(f.export_target(), "D:\\Out");
        assert_eq!(f.last_scene(), None);
        assert_eq!(f.to_text(), SAMPLE);
    }

    #[test]
    fn last_scene_is_never_added_empty() {
        let f = ProjectFile::parse(SAMPLE);
        assert_eq!(f.with_last_scene(None).to_text(), SAMPLE);
        assert_eq!(f.with_last_scene(Some("")).to_text(), SAMPLE);
        let with = f.with_last_scene(Some("rawdata\\levels\\a.level"));
        assert_eq!(with.last_scene(), Some("rawdata\\levels\\a.level"));
        // a key that is there is cleared, not removed
        assert_eq!(with.with_last_scene(None).last_scene(), Some(""));
    }

    #[test]
    fn a_fresh_clone_gets_its_manifest() {
        let f = ProjectFile::default().with_manifest("Clone", "D:\\Editor\\", "");
        assert_eq!(f.to_text(), "[project]\r\nname=Clone\r\nsdk_root=D:\\Editor\\\r\ngame_root=\r\n");
        // writing the same again changes nothing
        assert_eq!(f.with_manifest("Clone", "D:\\Editor\\", "").to_text(), f.to_text());
    }

    #[test]
    fn scenes_inside_only() {
        assert_eq!(scene_in_project("D:\\P", "D:\\P\\rawdata\\levels\\a.level").as_deref(), Some("rawdata\\levels\\a.level"));
        assert_eq!(scene_in_project("D:\\P", "rawdata/levels/a.level").as_deref(), Some("rawdata\\levels\\a.level"));
        assert_eq!(scene_in_project("D:\\P", "D:\\Other\\a.level"), None);
        assert_eq!(scene_in_project("D:\\P", "D:\\P2\\a.level"), None);
        assert_eq!(scene_in_project("D:\\P", "D:\\P"), None);
    }
}

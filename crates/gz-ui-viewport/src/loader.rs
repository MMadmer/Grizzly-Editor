//! Where the renderer's bytes come from: the project, then the editor's own library, then the
//! linked game - in that order.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_render::{AssetBytes, AssetLoader, AssetRef};
use gz_vfs::GameFiles;
use smol_str::SmolStr;

/// The loader of one open project (or of the editor alone, without one).
pub struct SceneLoader {
    /// The project's root, then the editor's.
    roots: Vec<PathBuf>,
    game: Option<Arc<GameFiles>>,
}

impl SceneLoader {
    #[must_use]
    pub fn new(project: Option<PathBuf>, editor: PathBuf, game: Option<Arc<GameFiles>>) -> Self {
        let mut roots = Vec::with_capacity(2);
        roots.extend(project);
        if !roots.iter().any(|r| r == &editor) {
            roots.push(editor);
        }
        Self { roots, game }
    }

    fn read_first(&self, candidates: &[(PathBuf, &'static str)]) -> Option<AssetBytes> {
        candidates.iter().find_map(|(p, ext)| {
            std::fs::read(p).ok().map(|bytes| AssetBytes { bytes, ext: Some(SmolStr::new_static(ext)) })
        })
    }

    /// A model's motions, `meshes\<name>.omf`: the project, the editor, then the game.
    pub(crate) fn motions(&self, name: &str) -> Option<Vec<u8>> {
        let name = name.trim_end_matches(".omf");
        self.roots
            .iter()
            .find_map(|r| std::fs::read(under(r, &["gamedata", "meshes"], name, "omf")).ok())
            .or_else(|| self.game.as_ref()?.read(&game_key("meshes", name, "omf")).ok())
    }

    fn game(&self, key: &str, ext: &'static str) -> Option<AssetBytes> {
        let files = self.game.as_ref()?;
        files.read(key).ok().map(|bytes| AssetBytes { bytes, ext: Some(SmolStr::new_static(ext)) })
    }
}

/// `name` (back slashes, no extension) under a root's folder, with an extension.
fn under(root: &Path, folder: &[&str], name: &str, ext: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for f in folder {
        p.push(f);
    }
    let parts: Vec<&str> = name.split('\\').filter(|s| !s.is_empty()).collect();
    if let Some((last, dirs)) = parts.split_last() {
        for d in dirs {
            p.push(d);
        }
        // a name may hold dots of its own: the extension is added, never swapped in
        p.push(format!("{last}.{ext}"));
    }
    p
}

/// The game's index key: lower case, forward slashes.
fn game_key(folder: &str, name: &str, ext: &str) -> String {
    format!("{folder}/{}.{ext}", name.replace('\\', "/").to_lowercase())
}

impl AssetLoader for SceneLoader {
    fn load(&self, what: AssetRef<'_>) -> Result<AssetBytes, String> {
        match what {
            AssetRef::Texture(name) => {
                let mut c = Vec::with_capacity(self.roots.len() * 2);
                for r in &self.roots {
                    c.push((under(r, &["gamedata", "textures"], name, "dds"), "dds"));
                    c.push((under(r, &["rawdata", "textures"], name, "tga"), "tga"));
                }
                self.read_first(&c)
                    .or_else(|| self.game(&game_key("textures", name, "dds"), "dds"))
                    .ok_or_else(|| format!("no texture '{name}' in the project, the editor or the game"))
            }
            AssetRef::Object(name) => {
                let c: Vec<_> = self
                    .roots
                    .iter()
                    .map(|r| (under(r, &["rawdata", "objects"], name, "object"), "object"))
                    .collect();
                self.read_first(&c).ok_or_else(|| format!("Can't find file '{name}.object'"))
            }
            AssetRef::Visual(name) => {
                let c: Vec<_> = self
                    .roots
                    .iter()
                    .map(|r| (under(r, &["gamedata", "meshes"], name, "ogf"), "ogf"))
                    .collect();
                self.read_first(&c)
                    .or_else(|| self.game(&game_key("meshes", name, "ogf"), "ogf"))
                    .ok_or_else(|| format!("no model '{name}' in the project, the editor or the game"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_keys() {
        let p = under(Path::new("C:\\p"), &["rawdata", "objects"], "props\\box_1", "object");
        assert!(p.ends_with("rawdata\\objects\\props\\box_1.object"));
        assert_eq!(game_key("meshes", "Dynamics\\Box", "ogf"), "meshes/dynamics/box.ogf");
        let l = SceneLoader::new(Some(PathBuf::from("C:\\p")), PathBuf::from("C:\\p"), None);
        assert_eq!(l.roots.len(), 1);
        assert!(l.load(AssetRef::Object("nothing_here_at_all")).is_err());
    }
}

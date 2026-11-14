//! Where the previews find a file: the open project, the editor's own folders, the linked game.
//!
//! The order is the texture loader's:
//! the editor's folders - the project's when the file lies in it - then the linked game. A
//! [`Sources`] is a snapshot of the three, made once per load; it is also the
//! [`gz_render::AssetLoader`] the model preview's renderer reads through.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::Editor;
use gz_render::{AssetBytes, AssetLoader, AssetRef};
use gz_vfs::{GameFiles, Source};

use crate::paths::EditorDirs;

/// Where a file was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A file of the open project.
    Project(PathBuf),
    /// A file of the editor's own folders.
    Editor(PathBuf),
    /// A file of the linked game: its key, and the loose file when it is not inside an archive.
    Game { key: String, loose: Option<PathBuf> },
}

impl Found {
    /// What `preview.model` answers as a texture's `from`.
    #[must_use]
    pub fn from_name(&self) -> &'static str {
        match self {
            Found::Project(_) => "project",
            Found::Editor(_) => "editor",
            Found::Game { .. } => "game",
        }
    }
}

/// The project, the editor and the linked game, as one load sees them.
#[derive(Clone)]
pub struct Sources {
    pub project: Option<PathBuf>,
    pub dirs: EditorDirs,
    pub game: Option<Arc<GameFiles>>,
}

impl Sources {
    /// The three as they are now; the game index is mounted when the project links one.
    pub async fn of(editor: &Editor) -> Self {
        let game = match gz_game::GameLink::of(editor) {
            Ok(link) => link.files().await.ok(),
            Err(_) => None,
        };
        Self { project: editor.project_root(), dirs: EditorDirs::of(editor), game }
    }

    fn in_project(&self, path: &Path) -> bool {
        self.project.as_ref().is_some_and(|root| path.starts_with(root))
    }

    fn local(&self, path: PathBuf) -> Option<Found> {
        if !path.is_file() {
            return None;
        }
        Some(if self.in_project(&path) { Found::Project(path) } else { Found::Editor(path) })
    }

    fn game_file(&self, key: &str) -> Option<Found> {
        let files = self.game.as_ref()?;
        let f = files.get(key)?;
        let loose = match f.source {
            Source::Loose => Some(files.game_data().join(f.path.replace('/', "\\"))),
            Source::Archive { .. } => None,
        };
        Some(Found::Game { key: f.path.to_owned(), loose })
    }

    /// A texture by its engine name (below `textures\`, no extension): the project's, the
    /// editor's `$level$`, `$game_saves$`, `$game_textures$`, then the linked game's.
    #[must_use]
    pub fn find_texture(&self, name: &str) -> Option<Found> {
        let rel = format!("{}.dds", name.replace('/', "\\"));
        let mut local = Vec::with_capacity(5);
        if let Some(p) = &self.project {
            local.push(p.join("textures").join(&rel));
            local.push(p.join("gamedata").join("textures").join(&rel));
        }
        local.push(self.dirs.levels.join(&rel));
        local.push(self.dirs.saves.join(&rel));
        local.push(self.dirs.game_textures.join(&rel));
        local
            .into_iter()
            .find_map(|p| self.local(p))
            .or_else(|| self.game_file(&format!("textures/{}", rel.replace('\\', "/"))))
    }

    /// A texture's source picture of the editor's library (`$textures$\name.tga`).
    #[must_use]
    pub fn find_texture_source(&self, name: &str) -> Option<Found> {
        self.local(self.dirs.textures.join(format!("{}.tga", name.replace('/', "\\"))))
    }

    /// A compiled model by its name below `meshes\` (the extension optional): a file named by
    /// its whole path, the project's, the editor's `$level$` and `$game_meshes$`.
    #[must_use]
    pub fn find_visual(&self, name: &str) -> Option<Found> {
        let name = name.replace('/', "\\");
        let direct = PathBuf::from(&name);
        if direct.is_absolute() {
            return self.local(direct);
        }
        if let Some(p) = &self.project
            && let Some(f) = self.local(p.join(&name))
        {
            return Some(f);
        }
        let rel = if crate::paths::strip_ext(&name).len() == name.len() {
            format!("{name}.ogf")
        } else {
            name.clone()
        };
        let mut local = Vec::with_capacity(4);
        if let Some(p) = &self.project {
            local.push(p.join("meshes").join(&rel));
            local.push(p.join("gamedata").join("meshes").join(&rel));
        }
        local.push(self.dirs.levels.join(&rel));
        local.push(self.dirs.meshes.join(&rel));
        local.into_iter().find_map(|p| self.local(p))
    }

    /// The same visual among the linked game's meshes.
    #[must_use]
    pub fn find_game_visual(&self, name: &str) -> Option<Found> {
        let key = gz_vfs::normalize(name);
        let key = if key.starts_with("meshes/") { key } else { format!("meshes/{key}") };
        let key = if key.ends_with(".ogf") { key } else { format!("{key}.ogf") };
        self.game_file(&key)
    }

    /// A library object by its name (`$objects$`): the project's `rawdata\objects`, then the
    /// editor's.
    #[must_use]
    pub fn find_object(&self, name: &str) -> Option<Found> {
        let rel = format!("{}.object", crate::paths::strip_ext(&name.replace('/', "\\")));
        let mut local = Vec::with_capacity(2);
        if let Some(p) = &self.project {
            local.push(p.join("rawdata").join("objects").join(&rel));
        }
        local.push(self.dirs.objects.join(&rel));
        local.into_iter().find_map(|p| self.local(p))
    }

    /// A motion file a model names: below `meshes\`, where the model was
    /// found first.
    #[must_use]
    pub fn find_motions(&self, name: &str, game_first: bool) -> Option<Found> {
        let rel = format!("{}.omf", crate::paths::strip_ext(&name.replace('/', "\\")));
        let game = || self.game_file(&format!("meshes/{}", rel.replace('\\', "/")));
        let local = || {
            let mut local = Vec::with_capacity(3);
            if let Some(p) = &self.project {
                local.push(p.join("meshes").join(&rel));
            }
            local.push(self.dirs.meshes.join(&rel));
            local.into_iter().find_map(|p| self.local(p))
        };
        if game_first { game().or_else(local) } else { local().or_else(game) }
    }

    /// The bytes of what was found. Blocking.
    pub fn read(&self, found: &Found) -> std::io::Result<Vec<u8>> {
        match found {
            Found::Project(p) | Found::Editor(p) => std::fs::read(p),
            Found::Game { key, .. } => match &self.game {
                Some(files) => files.read(key),
                None => Err(std::io::Error::other("no game is linked")),
            },
        }
    }
}

fn ext_of(found: &Found) -> Option<String> {
    let name = match found {
        Found::Project(p) | Found::Editor(p) => p.to_string_lossy().into_owned(),
        Found::Game { key, .. } => key.clone(),
    };
    let l = crate::paths::leaf(&name);
    l.rfind('.').map(|i| l[i + 1..].to_lowercase())
}

impl AssetLoader for Sources {
    fn load(&self, what: AssetRef<'_>) -> Result<AssetBytes, String> {
        let found = match what {
            AssetRef::Texture(name) => self.find_texture(name).or_else(|| self.find_texture_source(name)),
            AssetRef::Visual(name) => self.find_visual(name).or_else(|| self.find_game_visual(name)),
            AssetRef::Object(name) => self.find_object(name),
        };
        let Some(found) = found else {
            return Err(match what {
                AssetRef::Texture(n) => {
                    format!("the texture {n} is neither in the project, the editor nor the linked game")
                }
                AssetRef::Visual(n) => {
                    format!("the model {n} is neither in the project, the editor nor the linked game")
                }
                AssetRef::Object(n) => format!("the library has no object {n}"),
            });
        };
        let bytes = self.read(&found).map_err(|e| format!("{what:?} could not be read: {e}"))?;
        Ok(AssetBytes { bytes, ext: ext_of(&found).map(Into::into) })
    }
}

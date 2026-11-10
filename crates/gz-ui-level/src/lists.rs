//! The lists the Tools panel and the choosers pick from: the object library, the groups, the
//! spawn sections, the particle systems, the game's textures, sounds, models, shaders, game
//! materials, sound environments and light animations.
//!
//! Each is read once on the pools - never in a frame - and kept for the project's session; a
//! frame asks with [`Lists::get`] and draws nothing (a spinner) until the list is there.
//! Reading the same list again ([`Lists::refresh`]) is the author's Refresh List.

use std::sync::Arc;

use gz_editor::Editor;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

/// Which list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKey {
    Objects,
    Groups,
    SpawnSections,
    Particles,
    Textures,
    Sounds,
    Visuals,
    EngineShaders,
    CompilerShaders,
    GameMaterials,
    SoundEnvironments,
    LightAnims,
    BaseScenes,
}

impl ListKey {
    /// The list a chooser row names (`gz-scene`'s `Kind::Choose`); `None` for one the editor
    /// cannot list yet (the value is typed then).
    #[must_use]
    pub fn of_choose(list: &str) -> Option<Self> {
        Some(match list {
            "object" => ListKey::Objects,
            "group" => ListKey::Groups,
            "spawn_section" | "spawnitem" => ListKey::SpawnSections,
            "particles" => ListKey::Particles,
            "texture" => ListKey::Textures,
            "sound_source" | "soundsource" => ListKey::Sounds,
            "visual" => ListKey::Visuals,
            "e_shader" => ListKey::EngineShaders,
            "c_shader" => ListKey::CompilerShaders,
            "game_material" => ListKey::GameMaterials,
            "sound_env" => ListKey::SoundEnvironments,
            "lightanim" => ListKey::LightAnims,
            _ => return None,
        })
    }

    /// What the list is of, for the chooser's title and its empty state.
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            ListKey::Objects => "library objects",
            ListKey::Groups => "groups",
            ListKey::SpawnSections => "spawn sections",
            ListKey::Particles => "particle systems",
            ListKey::Textures => "textures",
            ListKey::Sounds => "sounds",
            ListKey::Visuals => "models",
            ListKey::EngineShaders => "engine shaders",
            ListKey::CompilerShaders => "compiler shaders",
            ListKey::GameMaterials => "game materials",
            ListKey::SoundEnvironments => "sound environments",
            ListKey::LightAnims => "light animations",
            ListKey::BaseScenes => "base scenes",
        }
    }
}

enum Slot {
    Reading,
    Ready(Arc<Vec<String>>),
}

/// The lists of the open project, an `Editor` service per session.
#[derive(Default)]
pub struct Lists {
    slots: Mutex<FxHashMap<ListKey, Slot>>,
    /// The project the lists were read for: a new one starts afresh.
    project: Mutex<Option<std::path::PathBuf>>,
}

impl Lists {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Lists> {
        editor.services.get_or_init(Lists::default)
    }

    /// The list when it is there; the first ask starts reading it.
    #[must_use]
    pub fn get(editor: &Arc<Editor>, key: ListKey) -> Option<Arc<Vec<String>>> {
        let this = Self::of(editor);
        let root = editor.project_root();
        {
            let mut p = this.project.lock();
            if *p != root {
                *p = root;
                this.slots.lock().clear();
            }
        }
        {
            let mut slots = this.slots.lock();
            match slots.get(&key) {
                Some(Slot::Ready(list)) => return Some(list.clone()),
                Some(Slot::Reading) => return None,
                None => {
                    slots.insert(key, Slot::Reading);
                }
            }
        }
        let ed = editor.clone();
        editor.exec.spawn(async move {
            let list = read(&ed, key).await;
            Self::of(&ed).slots.lock().insert(key, Slot::Ready(Arc::new(list)));
            ed.exec.wake().ping();
        });
        None
    }

    /// Reads a list again on its next ask (the library's Refresh List).
    pub fn refresh(editor: &Editor, key: ListKey) {
        Self::of(editor).slots.lock().remove(&key);
    }
}

/// `textures\a\b.dds` as the engine names it: `a\b`.
fn game_names(files: &gz_vfs::GameFiles, dir: &str, ext: &str) -> Vec<String> {
    let prefix = format!("{dir}/");
    let mut out: Vec<String> = files
        .find(dir, ext)
        .into_iter()
        .filter_map(|f| {
            let rel = f.path.strip_prefix(&prefix)?;
            let stem = rel.rsplit_once('.').map_or(rel, |(s, _)| s);
            Some(stem.replace('/', "\\"))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

async fn game_file(editor: &Editor, rel: &str) -> Option<Vec<u8>> {
    gz_game::read_game_or_bundled(editor, rel).await.ok().map(|(b, _)| b)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

async fn read(editor: &Arc<Editor>, key: ListKey) -> Vec<String> {
    match key {
        ListKey::Objects => match gz_scene::SceneService::of(editor) {
            Ok(svc) => {
                let lib = svc.library.clone();
                editor.exec.io(move || lib.list().as_ref().clone()).await
            }
            Err(_) => Vec::new(),
        },
        ListKey::Groups => {
            let root = editor.paths.root.join("rawdata").join("groups");
            editor.exec.io(move || gz_scene::library::list_groups(&root)).await
        }
        ListKey::BaseScenes => {
            let root = editor.paths.root.clone();
            editor.exec.io(move || gz_project::project::base_scenes(&root)).await
        }
        ListKey::SpawnSections => {
            let roster = gz_game::spawn_roster(editor).await;
            sorted(roster.iter().map(|s| if s.group.is_empty() { s.section.clone() } else { format!("{}\\{}", s.group, s.section) }).collect())
        }
        ListKey::Textures | ListKey::Sounds | ListKey::Visuals => {
            let Ok(link) = gz_game::GameLink::of(editor) else { return Vec::new() };
            let Ok(files) = link.files().await else { return Vec::new() };
            let (dir, ext) = match key {
                ListKey::Textures => ("textures", "dds"),
                ListKey::Sounds => ("sounds", "ogg"),
                _ => ("meshes", "ogf"),
            };
            editor.exec.cpu(move || game_names(&files, dir, ext)).await
        }
        ListKey::Particles => {
            let Some(bytes) = game_file(editor, "particles.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || match gz_fmt_xr::Particles::read(&bytes) {
                    Ok(p) => sorted(p.effects.iter().map(|e| e.name.to_string()).chain(p.groups.iter().map(|g| g.name.to_string())).collect()),
                    Err(_) => Vec::new(),
                })
                .await
        }
        ListKey::EngineShaders => {
            let Some(bytes) = game_file(editor, "shaders.xr").await else { return Vec::new() };
            editor.exec.cpu(move || gz_fmt_xr::Shaders::read(&bytes).map(|s| sorted(s.blenders.iter().map(|b| b.name.to_string()).collect())).unwrap_or_default()).await
        }
        ListKey::CompilerShaders => {
            let Some(bytes) = game_file(editor, "shaders_xrlc.xr").await else { return Vec::new() };
            editor.exec.cpu(move || gz_fmt_xr::ShadersXrlc::read(&bytes).map(|s| sorted(s.shaders.iter().map(gz_fmt_xr::ShaderXrlc::name).collect())).unwrap_or_default()).await
        }
        ListKey::GameMaterials => {
            let Some(bytes) = game_file(editor, "gamemtl.xr").await else { return Vec::new() };
            editor.exec.cpu(move || gz_fmt_xr::GameMaterials::read(&bytes).map(|m| sorted(m.materials.iter().map(|x| x.name.to_string()).collect())).unwrap_or_default()).await
        }
        ListKey::SoundEnvironments => {
            let Some(bytes) = game_file(editor, "sound_env.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || gz_fmt_xr::SoundEnvironments::read(&bytes).map(|m| sorted(m.environments.iter().map(|x| x.name.to_string()).collect())).unwrap_or_default())
                .await
        }
        ListKey::LightAnims => {
            let Some(bytes) = game_file(editor, "lanims.xr").await else { return Vec::new() };
            editor.exec.cpu(move || gz_fmt_xr::LightAnims::read(&bytes).map(|m| sorted(m.items.iter().map(|x| x.name.to_string()).collect())).unwrap_or_default()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ListKey;

    #[test]
    fn chooser_lists() {
        assert_eq!(ListKey::of_choose("object"), Some(ListKey::Objects));
        assert_eq!(ListKey::of_choose("story_id"), None);
    }
}

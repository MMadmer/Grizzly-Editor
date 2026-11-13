//! The lists the Tools panel and the choosers pick from: the object library, the groups, the
//! spawn sections, the particle systems, the game's textures, sounds, models, shaders, game
//! materials, sound environments, light animations and `.anm` animations, and a model's motions
//! and bones.
//!
//! Each is read once on the pools - never in a frame - and kept for the project's session; a
//! frame asks with [`Lists::get`] and draws nothing (a spinner) until the list is there.
//! Reading the same list again ([`Lists::refresh`]) is the author's Refresh List.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::SmolStr;
use gz_editor::Editor;
use gz_scene::props::Choice;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

/// Which list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    /// `OnFillRespawnItemProfile`: the sections of the game's `mp\\respawn_items.ltx`.
    RespawnItems,
    /// `FillSkeletonAnims`: the motions of a model (`meshes\<model>.ogf`), its own and those of
    /// the `.omf` files it names; empty for an object without a model.
    SkeletonMotions(SmolStr),
    /// `FillSkeletonBones`: the bones of a model.
    SkeletonBones(SmolStr),
    /// `FillGameAnim`: the game's `anims\*.anm` and `*.anms`.
    GameAnims,
}

impl ListKey {
    /// The list a chooser row picks from (`gz-scene`'s `Kind::Choose`); `None` for a list the
    /// row's owner gives itself (the scene's way paths, its story ids).
    #[must_use]
    pub fn of_choose(choice: &Choice) -> Option<Self> {
        let model = || SmolStr::new(model_name(choice.param.as_deref().unwrap_or("")));
        Some(match choice.list.as_str() {
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
            "respawn_items" => ListKey::RespawnItems,
            "skeletonanims" => ListKey::SkeletonMotions(model()),
            "skeletonbones" => ListKey::SkeletonBones(model()),
            "gameanim" => ListKey::GameAnims,
            _ => return None,
        })
    }

    /// The sentence of an empty list: what is missing, said of the thing it is missing from.
    #[must_use]
    pub fn empty_text(&self) -> String {
        match self {
            ListKey::SkeletonMotions(m) | ListKey::SkeletonBones(m) if m.is_empty() => {
                "The object has no model.".to_owned()
            }
            ListKey::SkeletonMotions(m) => format!("The model {m} has no motions."),
            ListKey::SkeletonBones(m) => format!("The model {m} has no bones."),
            ListKey::GameAnims => "The game has no .anm animations.".to_owned(),
            other => format!("No {} were found.", other.noun()),
        }
    }

    /// What the list is of, for the chooser's title and its empty state.
    #[must_use]
    pub fn noun(&self) -> &'static str {
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
            ListKey::RespawnItems => "respawn profiles",
            ListKey::BaseScenes => "base scenes",
            ListKey::SkeletonMotions(_) => "motions",
            ListKey::SkeletonBones(_) => "bones",
            ListKey::GameAnims => "animations",
        }
    }
}

/// A model as the engine names it: back slashes, no `.ogf`.
fn model_name(name: &str) -> String {
    let name = name.trim().replace('/', "\\");
    match name.len().checked_sub(4) {
        Some(at) if name.get(at..).is_some_and(|e| e.eq_ignore_ascii_case(".ogf")) => {
            name.get(..at).unwrap_or_default().to_owned()
        }
        _ => name,
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

    /// Forgets the lists of another project.
    fn follow_project(&self, editor: &Editor) {
        let root = editor.project_root();
        let mut p = self.project.lock();
        if *p != root {
            *p = root;
            self.slots.lock().clear();
        }
    }

    /// The list when it is there; the first ask starts reading it.
    #[must_use]
    pub fn get(editor: &Arc<Editor>, key: &ListKey) -> Option<Arc<Vec<String>>> {
        let this = Self::of(editor);
        this.follow_project(editor);
        {
            let mut slots = this.slots.lock();
            match slots.get(key) {
                Some(Slot::Ready(list)) => return Some(list.clone()),
                Some(Slot::Reading) => return None,
                None => {
                    slots.insert(key.clone(), Slot::Reading);
                }
            }
        }
        let (ed, key) = (editor.clone(), key.clone());
        editor.exec.spawn(async move {
            let list = read(&ed, &key).await;
            Self::of(&ed).slots.lock().insert(key, Slot::Ready(Arc::new(list)));
            ed.exec.wake().ping();
        });
        None
    }

    /// The list, read now when it is not there yet: a command waits for it, a frame never does.
    pub async fn wait(editor: &Arc<Editor>, key: &ListKey) -> Arc<Vec<String>> {
        let this = Self::of(editor);
        this.follow_project(editor);
        if let Some(Slot::Ready(list)) = this.slots.lock().get(key) {
            return list.clone();
        }
        let list = Arc::new(read(editor, key).await);
        this.slots.lock().insert(key.clone(), Slot::Ready(list.clone()));
        editor.exec.wake().ping();
        list
    }

    /// Reads a list again on its next ask (the library's Refresh List).
    pub fn refresh(editor: &Editor, key: &ListKey) {
        Self::of(editor).slots.lock().remove(key);
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

/// `anims\a\b.anm` as `FS.file_list` names it under `$game_anims$`: `a\b.anm`.
fn game_files_named(files: &gz_vfs::GameFiles, dir: &str, ext: &str) -> Vec<String> {
    let prefix = format!("{dir}/");
    files
        .find(dir, ext)
        .into_iter()
        .filter_map(|f| Some(f.path.strip_prefix(&prefix)?.replace('/', "\\")))
        .collect()
}

async fn game_files(editor: &Editor) -> Option<Arc<gz_vfs::GameFiles>> {
    gz_game::GameLink::of(editor).ok()?.files().await.ok()
}

/// Where a model is looked for before the game: the project, then the editor's own data.
fn model_roots(editor: &Editor) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = editor.project_root().into_iter().collect();
    let library = gz_project::library_root(editor);
    if !roots.contains(&library) {
        roots.push(library);
    }
    roots
}

/// `meshes\<name>.<ext>`: the first root that has it, then the game.
fn mesh_file(roots: &[PathBuf], game: Option<&gz_vfs::GameFiles>, name: &str, ext: &str) -> Option<Vec<u8>> {
    let rel = format!("{name}.{ext}");
    roots
        .iter()
        .find_map(|r| {
            let mut p = r.join("gamedata").join("meshes");
            p.extend(rel.split('\\').filter(|s| !s.is_empty()));
            std::fs::read(p).ok()
        })
        .or_else(|| game?.read(&format!("meshes/{}", rel.replace('\\', "/").to_lowercase())).ok())
}

/// The bones of a model, or every motion it can play - its own and those of each `.omf` its
/// motion references name (the `LL_MotionsSlotCount` slots) - as `FillSkeletonBones` and
/// `FillSkeletonAnims` list them. A model that cannot be read, or has no skeleton, lists none.
fn skeleton_list(
    roots: &[PathBuf],
    game: Option<&gz_vfs::GameFiles>,
    model: &str,
    bones: bool,
) -> Vec<String> {
    let Some(bytes) = mesh_file(roots, game, model, "ogf") else { return Vec::new() };
    let Ok(visual) = gz_fmt_ogf::Visual::read(&bytes) else { return Vec::new() };
    if bones {
        return visual.bones.iter().map(|b| b.name.to_string()).collect();
    }
    if visual.bones.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut add = |set: &gz_fmt_ogf::MotionSet| {
        for m in &set.motions {
            if !out.iter().any(|o| o == m.name.as_str()) {
                out.push(m.name.to_string());
            }
        }
    };
    if let Some(own) = &visual.motions {
        add(own);
    }
    for refs in &visual.motion_refs {
        for part in refs.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let part = part.strip_suffix(".omf").unwrap_or(part);
            if let Some(b) = mesh_file(roots, game, part, "omf")
                && let Ok(set) = gz_fmt_ogf::read_omf(&b)
            {
                add(&set);
            }
        }
    }
    sorted(out)
}

async fn game_file(editor: &Editor, rel: &str) -> Option<Vec<u8>> {
    gz_game::read_game_or_bundled(editor, rel).await.ok().map(|(b, _)| b)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

async fn read(editor: &Arc<Editor>, key: &ListKey) -> Vec<String> {
    match key {
        ListKey::SkeletonMotions(model) | ListKey::SkeletonBones(model) => {
            if model.is_empty() {
                return Vec::new();
            }
            let files = game_files(editor).await;
            let roots = model_roots(editor);
            let (model, bones) = (model.clone(), matches!(key, ListKey::SkeletonBones(_)));
            editor.exec.io(move || skeleton_list(&roots, files.as_deref(), &model, bones)).await
        }
        ListKey::GameAnims => {
            let Some(files) = game_files(editor).await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    let mut out = game_files_named(&files, "anims", "anm");
                    out.extend(game_files_named(&files, "anims", "anms"));
                    sorted(out)
                })
                .await
        }
        ListKey::Objects => match gz_scene::SceneService::of(editor) {
            Ok(svc) => {
                let lib = svc.library.clone();
                editor.exec.io(move || lib.list().as_ref().clone()).await
            }
            Err(_) => Vec::new(),
        },
        ListKey::Groups => {
            let root = gz_project::library_root(editor).join("rawdata").join("groups");
            editor.exec.io(move || gz_scene::library::list_groups(&root)).await
        }
        ListKey::BaseScenes => {
            let root = gz_project::library_root(editor);
            editor.exec.io(move || gz_project::project::base_scenes(&root)).await
        }
        ListKey::SpawnSections => {
            let roster = gz_game::spawn_roster(editor).await;
            sorted(
                roster
                    .iter()
                    .map(|s| {
                        if s.group.is_empty() {
                            s.section.clone()
                        } else {
                            format!("{}\\{}", s.group, s.section)
                        }
                    })
                    .collect(),
            )
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
                    Ok(p) => sorted(
                        p.effects
                            .iter()
                            .map(|e| e.name.to_string())
                            .chain(p.groups.iter().map(|g| g.name.to_string()))
                            .collect(),
                    ),
                    Err(_) => Vec::new(),
                })
                .await
        }
        ListKey::EngineShaders => {
            let Some(bytes) = game_file(editor, "shaders.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::Shaders::read(&bytes)
                        .map(|s| sorted(s.blenders.iter().map(|b| b.name.to_string()).collect()))
                        .unwrap_or_default()
                })
                .await
        }
        ListKey::CompilerShaders => {
            let Some(bytes) = game_file(editor, "shaders_xrlc.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::ShadersXrlc::read(&bytes)
                        .map(|s| sorted(s.shaders.iter().map(gz_fmt_xr::ShaderXrlc::name).collect()))
                        .unwrap_or_default()
                })
                .await
        }
        ListKey::GameMaterials => {
            let Some(bytes) = game_file(editor, "gamemtl.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::GameMaterials::read(&bytes)
                        .map(|m| sorted(m.materials.iter().map(|x| x.name.to_string()).collect()))
                        .unwrap_or_default()
                })
                .await
        }
        ListKey::SoundEnvironments => {
            let Some(bytes) = game_file(editor, "sound_env.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::SoundEnvironments::read(&bytes)
                        .map(|m| sorted(m.environments.iter().map(|x| x.name.to_string()).collect()))
                        .unwrap_or_default()
                })
                .await
        }
        ListKey::RespawnItems => {
            let Some(bytes) = game_file(editor, "configs\\mp\\respawn_items.ltx").await else {
                return Vec::new();
            };
            let text = gz_base::text::decode_guess(&bytes);
            text.lines()
                .filter_map(|l| {
                    let l = l.trim_start();
                    let rest = l.strip_prefix('[')?;
                    let end = rest.find(']')?;
                    Some(rest[..end].trim().to_owned())
                })
                .filter(|s| !s.is_empty())
                .collect()
        }
        ListKey::LightAnims => {
            let Some(bytes) = game_file(editor, "lanims.xr").await else { return Vec::new() };
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::LightAnims::read(&bytes)
                        .map(|m| sorted(m.items.iter().map(|x| x.name.to_string()).collect()))
                        .unwrap_or_default()
                })
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(list: &str, param: Option<&str>) -> Choice {
        Choice { list: list.into(), param: param.map(Into::into), max: 1 }
    }

    #[test]
    fn chooser_lists() {
        assert_eq!(ListKey::of_choose(&choice("object", None)), Some(ListKey::Objects));
        assert_eq!(ListKey::of_choose(&choice("story_id", None)), None);
        assert_eq!(ListKey::of_choose(&choice("respawn_items", None)), Some(ListKey::RespawnItems));
        assert_eq!(ListKey::of_choose(&choice("gameanim", None)), Some(ListKey::GameAnims));
        assert_eq!(
            ListKey::of_choose(&choice("skeletonanims", Some("monsters/dog/dog.OGF"))),
            Some(ListKey::SkeletonMotions("monsters\\dog\\dog".into()))
        );
        assert_eq!(
            ListKey::of_choose(&choice("skeletonbones", None)),
            Some(ListKey::SkeletonBones("".into()))
        );
        assert_eq!(ListKey::SkeletonBones("".into()).empty_text(), "The object has no model.");
        assert_eq!(ListKey::SkeletonMotions("a\\b".into()).empty_text(), "The model a\\b has no motions.");
    }

    /// A skeleton of `bones` naming the motion files `refs` (`OGF_S_MOTION_REFS`).
    fn skeleton(bones: &[&str], refs: &str) -> Vec<u8> {
        use gz_fmt_ogf::{MT, chunk};
        let mut w = gz_chunk::Writer::new();
        w.chunk(chunk::HEADER, |w| {
            w.w_u8(4);
            w.w_u8(MT::SKELETON_RIGID);
            w.w_u16(0);
            // the box, the sphere's centre and radius
            for _ in 0..10 {
                w.w_f32(0.0);
            }
        });
        w.chunk(chunk::S_BONE_NAMES, |w| {
            w.w_u32(bones.len() as u32);
            for (i, b) in bones.iter().enumerate() {
                w.w_stringz(b);
                w.w_stringz(if i == 0 { "" } else { bones[0] });
                // the bone's box
                for _ in 0..15 {
                    w.w_f32(0.0);
                }
            }
        });
        if !refs.is_empty() {
            w.chunk(chunk::S_MOTION_REFS, |w| w.w_stringz(refs));
        }
        w.into_bytes()
    }

    fn motions(names: &[&str]) -> Vec<u8> {
        gz_fmt_ogf::MotionSet {
            params_version: Some(4),
            parts: Vec::new(),
            defs: Vec::new(),
            motions: names
                .iter()
                .map(|n| gz_fmt_ogf::Motion { name: (*n).into(), length: 1, bones: Vec::new() })
                .collect(),
        }
        .to_omf()
    }

    #[test]
    fn a_models_bones_and_motions_are_read_from_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let meshes = dir.path().join("gamedata").join("meshes");
        std::fs::create_dir_all(meshes.join("monsters").join("dog")).unwrap();
        std::fs::create_dir_all(meshes.join("anims")).unwrap();
        let dog = skeleton(&["root", "spine", "head"], "anims\\dog_a, anims\\dog_b,anims\\gone");
        std::fs::write(meshes.join("monsters").join("dog").join("dog.ogf"), dog).unwrap();
        std::fs::write(meshes.join("anims").join("dog_a.omf"), motions(&["walk", "idle"])).unwrap();
        std::fs::write(meshes.join("anims").join("dog_b.omf"), motions(&["idle", "run"])).unwrap();
        std::fs::write(meshes.join("statue.ogf"), skeleton(&[], "")).unwrap();
        let roots = vec![dir.path().to_path_buf()];

        assert_eq!(skeleton_list(&roots, None, "monsters\\dog\\dog", true), ["root", "spine", "head"]);
        // every slot once, a missing file skipped, as `FillSkeletonAnims` merges them
        assert_eq!(skeleton_list(&roots, None, "monsters\\dog\\dog", false), ["idle", "run", "walk"]);
        // no skeleton, no file: nothing to pick, never an error
        assert!(skeleton_list(&roots, None, "statue", false).is_empty());
        assert!(skeleton_list(&roots, None, "statue", true).is_empty());
        assert!(skeleton_list(&roots, None, "nowhere", true).is_empty());
    }
}

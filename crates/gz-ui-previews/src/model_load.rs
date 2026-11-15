//! What the model preview loads: a compiled visual (`.ogf`), a library object (`.object`) or a
//! model asset of the project (`.meshasset`, made into the model the scene and the level
//! compiler use), read, measured and made into a mesh on the pools.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use gz_editor::Editor;
use gz_fmt_object::Object;
use gz_fmt_object::xmath::Mat4 as XMat4;
use gz_fmt_ogf::{MT, MotionSet, Visual, read_omf};
use gz_render::{Aabb, MaterialTable, MeshData, object_rest, visual_pose};

use crate::loader::{Found, Sources};

/// Where a model is looked for: the open project's files, the editor's library (the project's
/// and the editor's own folders), or the linked game's. One typed value both the model tab and
/// whoever opens it share; the word a command or a tab request carries is [`ModelSource::arg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// A model of the open project.
    Project,
    /// A model of the project or the editor's library, by its engine name.
    Library,
    /// A visual of the linked game, by its engine name.
    Game,
}

impl ModelSource {
    /// `library` (or nothing), `project` or `game`; `None` for any other word.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "" | "library" => Some(ModelSource::Library),
            "project" => Some(ModelSource::Project),
            "game" => Some(ModelSource::Game),
            _ => None,
        }
    }

    /// The word a command (`source=`) or a tab request carries.
    #[must_use]
    pub fn arg(self) -> &'static str {
        match self {
            ModelSource::Project => "project",
            ModelSource::Library => "library",
            ModelSource::Game => "game",
        }
    }

    /// Where a model of the linked game is shown from: a visual of the game, not of the editor.
    #[must_use]
    pub fn is_game(self) -> bool {
        self == ModelSource::Game
    }
}

/// What a visual is, as the Details panel names it.
#[must_use]
pub fn visual_type_name(kind: u8) -> &'static str {
    match kind {
        MT::NORMAL => "Static Mesh",
        MT::HIERRARHY => "Hierarchy",
        MT::PROGRESSIVE => "Progressive Mesh",
        MT::SKELETON_ANIM => "Animated Skeleton",
        MT::SKELETON_RIGID => "Rigid Skeleton",
        MT::SKELETON_GEOMDEF_PM => "Skinned Mesh (PM)",
        MT::SKELETON_GEOMDEF_ST => "Skinned Mesh (ST)",
        MT::LOD => "LOD",
        MT::TREE_ST => "Tree (ST)",
        MT::TREE_PM => "Tree (PM)",
        MT::PARTICLE_EFFECT => "Particle Effect",
        MT::PARTICLE_GROUP => "Particle Group",
        _ => "Unknown",
    }
}

/// The kinds the preview builds.
#[must_use]
pub fn supported(kind: u8) -> bool {
    matches!(
        kind,
        MT::NORMAL
            | MT::HIERRARHY
            | MT::PROGRESSIVE
            | MT::SKELETON_ANIM
            | MT::SKELETON_RIGID
            | MT::SKELETON_GEOMDEF_PM
            | MT::SKELETON_GEOMDEF_ST
    )
}

/// A material slot of a library object: what it is to whoever has to ship it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub name: String,
    pub texture: String,
    pub shader: String,
    /// The shader the level compiler lights it with; empty for a visual.
    pub compile: String,
    pub game_mtl: String,
}

/// What only a library object has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectFacts {
    pub meshes: usize,
    pub lod: String,
    pub skeleton: bool,
    pub motions: usize,
    pub materials: Vec<Material>,
}

/// A texture a visual draws with and where the loader finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureUse {
    /// The engine name: below `textures\`, no extension.
    pub name: String,
    /// `project`, `editor`, `game` or `missing`.
    pub from: &'static str,
}

/// A model, loaded.
pub struct ModelData {
    /// The name as asked for.
    pub name: String,
    /// Why it cannot be shown; empty when it can.
    pub error: String,
    /// `Static Mesh`, `Animated Skeleton`, ..., `Library Object`; empty without a model.
    pub kind: String,
    pub verts: u32,
    pub tris: u32,
    pub bones: u32,
    /// The box of the pose drawn.
    pub bounds: Aabb,
    pub textures: Vec<TextureUse>,
    /// What it is drawn with, surface by surface: an object's or a model asset's surfaces, a
    /// visual's texture and shader pairs.
    pub surfaces: Vec<Material>,
    pub object: Option<ObjectFacts>,
    pub mesh: Option<MeshData>,
    pub pose: Option<Arc<[Mat4]>>,
    /// Every bone's place in the pose drawn, and its parent's.
    pub joints: Vec<(Vec3, Option<Vec3>)>,
    /// What the renderer reads the textures through.
    pub sources: Option<Sources>,
}

impl ModelData {
    fn failed(name: &str, error: &str) -> Self {
        Self {
            name: name.to_owned(),
            error: error.to_owned(),
            kind: String::new(),
            verts: 0,
            tris: 0,
            bones: 0,
            bounds: Aabb::EMPTY,
            textures: Vec::new(),
            surfaces: Vec::new(),
            object: None,
            mesh: None,
            pose: None,
            joints: Vec::new(),
            sources: None,
        }
    }

    #[must_use]
    pub fn shown(&self) -> bool {
        self.error.is_empty() && self.mesh.is_some()
    }
}

/// Whether a name is a model asset's project path.
#[must_use]
pub fn is_model_asset(name: &str) -> bool {
    gz_project::asset_kind::MediaKind::of_file(name) == Some(gz_project::asset_kind::MediaKind::Model)
}

/// Loads a model by its name: a model asset of the project by its path, the editor's visuals
/// (then a library object of that name), or the linked game's.
pub async fn load(editor: Arc<Editor>, name: String, source: ModelSource) -> ModelData {
    let sources = Sources::of(&editor).await;
    let exec = editor.exec.clone();
    if !source.is_game() && is_model_asset(&name) {
        let Some(root) = sources.project.clone() else {
            return ModelData::failed(&name, "no project is open");
        };
        let n = name.clone();
        return exec
            .io(move || match gz_media::resolve::model(&root, &n.replace('\\', "/")) {
                Ok(m) => {
                    let mut d = from_object_value(&n, &m.object, sources);
                    "Model Asset".clone_into(&mut d.kind);
                    d
                }
                Err(f) => ModelData::failed(&n, &f.to_string()),
            })
            .await;
    }
    let n = name.clone();
    let found = exec
        .io(move || {
            let found = match source {
                ModelSource::Game => sources.find_game_visual(&n),
                ModelSource::Project | ModelSource::Library => sources.find_visual(&n),
            };
            match found {
                Some(f) => {
                    let bytes = sources.read(&f);
                    (sources, Some((f, bytes)), false)
                }
                None if !source.is_game() => match sources.find_object(&n) {
                    Some(f) => {
                        let bytes = sources.read(&f);
                        (sources, Some((f, bytes)), true)
                    }
                    None => (sources, None, false),
                },
                None => (sources, None, false),
            }
        })
        .await;
    let (sources, file, is_object) = found;
    let Some((found, bytes)) = file else {
        return ModelData::failed(
            &name,
            if source == ModelSource::Game {
                "not found in the linked game install"
            } else {
                "file not found"
            },
        );
    };
    let bytes = match bytes {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => return ModelData::failed(&name, "empty block"),
        Err(_) => return ModelData::failed(&name, "cannot open file"),
    };
    let object_file = is_object
        || matches!(&found, Found::Project(p) | Found::Editor(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("object")));
    let n = name.clone();
    exec.cpu(move || {
        if object_file { from_object(&n, &bytes, sources) } else { from_visual(&n, &bytes, &found, sources) }
    })
    .await
}

fn from_object(name: &str, bytes: &[u8], sources: Sources) -> ModelData {
    let object = match Object::read(bytes) {
        Ok(o) => o,
        Err(gz_fmt_object::ObjectError::NotAnObject) => {
            return ModelData::failed(name, "not an .object file");
        }
        Err(_) => return ModelData::failed(name, "cannot load object"),
    };
    from_object_value(name, &object, sources)
}

/// What a library object is, measured and made into its mesh.
fn from_object_value(name: &str, object: &Object, sources: Sources) -> ModelData {
    let mesh = match MeshData::from_object(object, &MaterialTable::default()) {
        Ok(m) => m,
        Err(_) => return ModelData::failed(name, "cannot load object"),
    };
    let verts: usize = object.meshes.iter().map(|m| m.points.len()).sum();
    let tris: usize = object.meshes.iter().map(|m| m.faces.len()).sum();
    let b = object.bounding_box();
    let bounds = if b.min.x > b.max.x { mesh.bounds } else { Aabb::new(b.min, b.max) };
    let facts = ObjectFacts {
        meshes: object.meshes.len(),
        lod: object.lod_ref.to_string(),
        skeleton: object.is_skeleton(),
        motions: object.motions.len(),
        materials: object
            .surfaces
            .iter()
            .map(|s| Material {
                name: s.name.to_string(),
                texture: s.texture.to_string(),
                shader: s.shader.to_string(),
                compile: s.shader_xrlc.to_string(),
                game_mtl: s.game_mtl.to_string(),
            })
            .collect(),
    };
    let pose = mesh.skinned.then(|| object_rest(object));
    ModelData {
        name: name.to_owned(),
        error: String::new(),
        kind: "Library Object".to_owned(),
        verts: u32::try_from(verts).unwrap_or(u32::MAX),
        tris: u32::try_from(tris).unwrap_or(u32::MAX),
        bones: u32::try_from(object.bones.len()).unwrap_or(u32::MAX),
        bounds,
        // a library object is not an engine visual: its Details list Materials instead
        textures: Vec::new(),
        surfaces: facts.materials.clone(),
        object: Some(facts),
        mesh: Some(mesh),
        pose,
        joints: Vec::new(),
        sources: Some(sources),
    }
}

/// The texture names a visual's shaders bind, each once; `$` names are the engine's own targets.
fn texture_names(visual: &Visual) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in visual.meshes() {
        let Some((texture, _shader)) = &m.texture else { continue };
        for t in texture.split(',') {
            let t = t.trim();
            if t.is_empty() || t.starts_with('$') {
                continue;
            }
            if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
                out.push(t.to_owned());
            }
        }
    }
    out
}

/// The motion a skeleton is posed with: `idle`, else the first cycle it ships, else the first
/// motion.
fn pick_motion(motions: &MotionSet) -> Option<usize> {
    if let Some(i) = motions.motions.iter().position(|m| m.name.eq_ignore_ascii_case("idle")) {
        return Some(i);
    }
    // a definition without esmFX is a cycle
    let cycle = motions
        .defs
        .iter()
        .find(|d| d.flags & 1 == 0)
        .and_then(|d| motions.motions.iter().position(|m| m.name == d.name));
    cycle.or(if motions.motions.is_empty() { None } else { Some(0) })
}

fn motions_of(visual: &Visual, found: &Found, sources: &Sources) -> Option<MotionSet> {
    if let Some(m) = &visual.motions
        && !m.motions.is_empty()
    {
        return Some(m.clone());
    }
    let game_first = matches!(found, Found::Game { .. });
    for r in &visual.motion_refs {
        for part in r.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(f) = sources.find_motions(part, game_first)
                && let Ok(bytes) = sources.read(&f)
                && let Ok(set) = read_omf(&bytes)
                && !set.motions.is_empty()
            {
                return Some(set);
            }
        }
    }
    None
}

/// Every bone's model-space transform at rest, parents first whatever the order of the list.
fn rest_chain(visual: &Visual) -> Vec<XMat4> {
    let n = visual.bones.len();
    let parent: Vec<Option<usize>> = visual
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            visual.bones.iter().position(|o| o.name.eq_ignore_ascii_case(&b.parent)).filter(|p| *p != i)
        })
        .collect();
    let local: Vec<XMat4> = (0..n)
        .map(|i| {
            visual.ik.get(i).map_or(XMat4::IDENTITY, |ik| {
                let mut m = XMat4::xyz_i(ik.rest_rotate);
                m.set_c(ik.rest_offset);
                m
            })
        })
        .collect();
    let mut world: Vec<Option<XMat4>> = vec![None; n];
    let mut stack = Vec::new();
    for start in 0..n {
        stack.clear();
        let mut at = Some(start);
        while let Some(i) = at {
            if world.get(i).copied().flatten().is_some() || stack.contains(&i) || stack.len() > n {
                break;
            }
            stack.push(i);
            at = parent.get(i).copied().flatten();
        }
        while let Some(i) = stack.pop() {
            let up = parent
                .get(i)
                .copied()
                .flatten()
                .and_then(|p| world.get(p).copied().flatten())
                .unwrap_or(XMat4::IDENTITY);
            let own = local.get(i).copied().unwrap_or(XMat4::IDENTITY);
            if let Some(slot) = world.get_mut(i) {
                *slot = Some(XMat4::mul(&up, &own));
            }
        }
    }
    world.into_iter().map(|m| m.unwrap_or(XMat4::IDENTITY)).collect()
}

/// The box of the mesh in its pose: every vertex moved by its bones.
fn posed_bounds(mesh: &MeshData, pose: Option<&[Mat4]>) -> Aabb {
    let Some(pose) = pose.filter(|_| mesh.skinned) else { return mesh.bounds };
    let mut b = Aabb::EMPTY;
    for v in mesh.vertices.iter() {
        let p = Vec3::from(v.position);
        let total: u32 = v.weights.iter().map(|w| u32::from(*w)).sum();
        if total == 0 {
            b.extend(p);
            continue;
        }
        let mut out = Vec3::ZERO;
        for (bone, w) in v.bones.iter().zip(v.weights) {
            if w == 0 {
                continue;
            }
            let m = pose.get(usize::from(*bone)).copied().unwrap_or(Mat4::IDENTITY);
            out += m.transform_point3(p) * (f32::from(w) / total as f32);
        }
        b.extend(out);
    }
    if b.is_empty() { mesh.bounds } else { b }
}

fn from_visual(name: &str, bytes: &[u8], found: &Found, sources: Sources) -> ModelData {
    // the header is peeked first: a file the reader takes for another kind is no model
    if bytes.len() < 8
        || gz_chunk::Reader::new(bytes)
            .open_chunk(gz_fmt_ogf::chunk::HEADER)
            .ok()
            .flatten()
            .is_none_or(|c| c.bytes().len() != gz_fmt_ogf::Header::SIZE)
    {
        return ModelData::failed(name, "not an OGF model");
    }
    let visual = match Visual::read(bytes) {
        Ok(v) => v,
        Err(_) => return ModelData::failed(name, "not an OGF model"),
    };
    if !supported(visual.kind()) {
        return ModelData::failed(
            name,
            &format!("unsupported visual type ({})", visual_type_name(visual.kind())),
        );
    }
    let mesh = match MeshData::from_visual(&visual, &MaterialTable::default()) {
        Ok(m) => m,
        Err(_) => return ModelData::failed(name, "model_Create failed"),
    };
    let (mut verts, mut tris) = (0usize, 0usize);
    for m in visual.meshes() {
        verts += m.vertices.as_ref().map_or(0, |v| v.vertices.len());
        // a progressive mesh draws its first slide window: that is its triangle count
        let all = m.indices.as_ref().map_or(0, |i| i.len() / 3);
        tris += m.swi.as_ref().and_then(|s| s.windows.first()).map_or(all, |w| usize::from(w.num_tris));
    }
    let bones = visual.bones.len();
    let pose = if bones > 0 {
        let posed = motions_of(&visual, found, &sources)
            .and_then(|set| pick_motion(&set).and_then(|i| visual_pose(&visual, &set, i, 0.0)));
        Some(posed.unwrap_or_else(|| vec![Mat4::IDENTITY; bones.max(1)].into()))
    } else if mesh.skinned {
        Some(vec![Mat4::IDENTITY].into())
    } else {
        None
    };
    let bounds = posed_bounds(&mesh, pose.as_deref());
    // the joints where the pose puts them: the skinning matrix moves a bone's rest place
    let mut joints = Vec::new();
    if bones > 0 {
        let rest = rest_chain(&visual);
        let at = |i: usize| -> Vec3 {
            let r = rest.get(i).map_or(Vec3::ZERO, XMat4::c);
            pose.as_ref().and_then(|p| p.get(i)).map_or(r, |m| m.transform_point3(r))
        };
        for (i, b) in visual.bones.iter().enumerate() {
            let parent =
                visual.bones.iter().position(|o| o.name.eq_ignore_ascii_case(&b.parent)).filter(|p| *p != i);
            joints.push((at(i), parent.map(at)));
        }
    }
    let mut surfaces: Vec<Material> = Vec::new();
    for m in visual.meshes() {
        let Some((texture, shader)) = &m.texture else { continue };
        let s = Material {
            name: String::new(),
            texture: texture.to_string(),
            shader: shader.to_string(),
            compile: String::new(),
            game_mtl: String::new(),
        };
        if !surfaces.contains(&s) {
            surfaces.push(s);
        }
    }
    let textures = texture_names(&visual)
        .into_iter()
        .map(|t| {
            let from = sources.find_texture(&t).map_or("missing", |f| f.from_name());
            TextureUse { name: t, from }
        })
        .collect();
    ModelData {
        name: name.to_owned(),
        error: String::new(),
        kind: visual_type_name(visual.kind()).to_owned(),
        verts: u32::try_from(verts).unwrap_or(u32::MAX),
        tris: u32::try_from(tris).unwrap_or(u32::MAX),
        bones: u32::try_from(bones).unwrap_or(u32::MAX),
        bounds,
        textures,
        surfaces,
        object: None,
        mesh: Some(mesh),
        pose,
        joints,
        sources: Some(sources),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert!(supported(MT::SKELETON_ANIM));
        assert!(!supported(MT::LOD));
        assert_eq!(visual_type_name(MT::TREE_PM), "Tree (PM)");
        assert_eq!(ModelSource::parse("game"), Some(ModelSource::Game));
        assert_eq!(ModelSource::parse(""), Some(ModelSource::Library));
        assert_eq!(ModelSource::parse("visual"), None);
        assert_eq!(ModelSource::parse("darf"), None);
        for s in [ModelSource::Project, ModelSource::Library, ModelSource::Game] {
            assert_eq!(ModelSource::parse(s.arg()), Some(s));
        }
    }

    #[test]
    fn a_file_that_is_no_model_says_so() {
        let sources = Sources {
            project: None,
            dirs: crate::paths::EditorDirs::at(std::path::Path::new("x")),
            game: None,
            media: None,
        };
        let m = from_visual("a", b"not a model at all", &Found::Editor("a.ogf".into()), sources);
        assert_eq!(m.error, "not an OGF model");
        assert!(!m.shown());
    }
}

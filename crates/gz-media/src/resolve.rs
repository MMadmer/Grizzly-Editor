//! A media asset made into what the editor uses: a model asset into the same model a library
//! `.object` is (and the scene's [`LibraryObject`]), a texture asset into its pixels and its
//! description, a sound asset into its parameters. Blocking: call on the IO pool.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::Severity;
use gz_fmt_mesh::{MeshSource, PictureData};
use gz_fmt_object::glam::Vec3;
use gz_fmt_object::{
    Bone, BonePart, Object, ObjectFlags, SMotionFlags, ShapeType, SurfaceFlags, VMap, VMapKind, VMapPt,
};
use gz_fmt_tex::{Image, TextureParams};
use gz_luadata::Seg;
use gz_project::asset_kind::MediaKind;
use gz_scene::library::LibraryObject;

use crate::files::{self, FNV0, fnv};
use crate::kind;
use crate::model::ModelAsset;
use crate::parts;
use crate::read::Issue;
use crate::sound::SoundAsset;
use crate::texture::TextureAsset;

/// A model asset made into its model.
#[derive(Clone, Debug)]
pub struct Model {
    pub asset: ModelAsset,
    pub object: Object,
    /// What the file says that does not read, and what the model cannot take, at their keys.
    pub issues: Vec<Issue>,
    /// The source file (none for `parts` and for a model of the game).
    pub source: Option<PathBuf>,
    /// The compiled model the asset is made over, when its source is an `.ogf`.
    pub compiled: Option<Compiled>,
    /// The asset's text and its source's bytes as one number.
    pub stamp: u64,
}

/// The compiled model (`.ogf`) a model asset is made over: a file of the project, or a model
/// of the game by its engine name. `object` is its geometry in the rest pose; the skeleton, the
/// motions and the levels of detail stay the file's own, and a dynamic model is published as
/// these bytes with what the asset changes said again.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub bytes: Arc<[u8]>,
    /// The game's model by its engine name, when the source is `game:`.
    pub game: Option<String>,
    /// The names of its bones, in the file's order.
    pub bones: Vec<String>,
    /// The names of the motions the file itself holds: the ones the game changes for this model
    /// alone (a motion of a motion file the model names is shared with every model that names it).
    pub motions: Vec<String>,
}

/// What reads a compiled model of the game by its engine name (`dynamics\box\box_wood_01`):
/// the project's own `gamedata\meshes` first, then the linked game.
pub type GameModel = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// A reader of no game: every model of the game is missing.
#[must_use]
pub fn no_game() -> GameModel {
    Arc::new(|_| None)
}

/// Why an asset could not be made: the code of the check and its sentence, at a key.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub issue: Issue,
}

impl Failure {
    fn new(code: &'static str, key: &str, message: impl Into<String>) -> Self {
        let path = if key.is_empty() { Vec::new() } else { vec![Seg::key(key)] };
        Self { issue: Issue::new(code, Severity::Error, path, message) }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.issue.message)
    }
}

pub const MISSING: &str = "M101";
pub const UNREADABLE: &str = "M102";
pub const NO_BONES: &str = "M108";
pub const PARTS_AND_SOURCE: &str = "M110";
pub const CANNOT: &str = "M111";
pub const OUTSIDE: &str = "M113";

/// The source of an asset when it lies in the project; a failure that names it when it does
/// not: a project names nothing outside itself.
fn in_project(root: &Path, source: PathBuf, at: &'static str) -> Result<PathBuf, Failure> {
    if files::inside(root, &source) {
        return Ok(source);
    }
    Err(Failure::new(
        OUTSIDE,
        at,
        format!("the source lies outside the project: {} (asset.import copies a file in)", source.display()),
    ))
}

fn read_text(abs: &Path, rel: &str) -> Result<String, Failure> {
    let bytes = std::fs::read(abs).map_err(|e| {
        Failure::new(UNREADABLE, "", format!("{rel} could not be read: {}", gz_base::io_reason(&e)))
    })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn loaded<T>(
    kind: MediaKind,
    rel: &str,
    text: &str,
    from: impl Fn(&gz_luadata::Value) -> (T, Vec<Issue>),
) -> Result<(T, Vec<Issue>), Failure> {
    let value = kind::load(kind, rel, text).map_err(|e| Failure::new("syntax", "", e))?;
    Ok(from(&value))
}

/// The bytes of a file and the files it names, folded into a stamp.
fn stamp_files(mut h: u64, files: &[PathBuf]) -> u64 {
    for f in files {
        h = fnv(h, f.to_string_lossy().to_lowercase().as_bytes());
        if let Ok(b) = std::fs::read(f) {
            h = fnv(h, &b);
        }
    }
    h
}

/// The compiled model a source names: its geometry as the source model, with its bones' names.
fn compiled(
    bytes: Vec<u8>,
    file: &str,
    game: Option<String>,
    at: &'static str,
) -> Result<(Object, Compiled), Failure> {
    let visual =
        gz_fmt_ogf::Visual::read(&bytes).map_err(|e| Failure::new(UNREADABLE, at, format!("{file}: {e}")))?;
    let src =
        gz_fmt_mesh::object_of_ogf(&visual, file).map_err(|e| Failure::new(UNREADABLE, at, e.to_string()))?;
    let bones = visual.bones.iter().map(|b| b.name.to_string()).collect();
    let motions = visual
        .motions
        .as_ref()
        .map(|m| m.motions.iter().map(|x| x.name.to_string()).collect())
        .unwrap_or_default();
    Ok((src.object, Compiled { bytes: Arc::from(bytes), game, bones, motions }))
}

/// A model asset of the project (`rel` project-relative, either slash), made into its model.
/// `game` reads a model of the game a `source = "game:..."` names.
pub fn model(root: &Path, rel: &str, game: &GameModel) -> Result<Model, Failure> {
    let abs = root.join(rel.replace('/', "\\"));
    let text = read_text(&abs, rel)?;
    let (asset, mut issues) = loaded(MediaKind::Model, rel, &text, ModelAsset::from_value)?;
    let mut stamp = fnv(FNV0, text.as_bytes());
    let has_source = asset.source.as_deref().is_some_and(|s| !s.trim().is_empty());
    if !asset.parts.is_empty() && has_source {
        return Err(Failure::new(
            PARTS_AND_SOURCE,
            "parts",
            "a model takes its geometry from 'source' or from 'parts', not from both",
        ));
    }
    let mut made_over = None;
    let (mut object, mesh_source, source) = if let Some(name) = files::game_model(asset.source.as_deref())
        && asset.parts.is_empty()
    {
        if !files::plain_game_name(&name) {
            return Err(Failure::new(
                MISSING,
                "source",
                format!("'{name}' is no name of a model of the game (game:dynamics/box/box_wood_01.ogf)"),
            ));
        }
        let bytes = game(&name).ok_or_else(|| {
            Failure::new(
                MISSING,
                "source",
                format!("neither the project nor the linked game has the model meshes\\{name}.ogf (asset.find kind=model lists the game's)"),
            )
        })?;
        stamp = fnv(fnv(stamp, name.as_bytes()), &bytes);
        let (object, c) = compiled(bytes, &format!("{name}.ogf"), Some(name), "source")?;
        made_over = Some(c);
        (object, None, None)
    } else if asset.parts.is_empty() {
        let source = files::source_of(MediaKind::Model, &abs, asset.source.as_deref()).ok_or_else(|| {
            Failure::new(
                MISSING,
                if has_source { "source" } else { "" },
                format!(
                    "the source is not there: {} (or write 'parts')",
                    files::expected_source(MediaKind::Model, &abs, asset.source.as_deref())
                ),
            )
        })?;
        let source = in_project(root, source, if has_source { "source" } else { "" })?;
        let mut all = vec![source.clone()];
        all.extend(files::dependencies(&source));
        stamp = stamp_files(stamp, &all);
        let at = if has_source { "source" } else { "" };
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "object" || ext == "ogf" {
            let bytes = std::fs::read(&source).map_err(|e| {
                Failure::new(UNREADABLE, at, format!("{}: {}", source.display(), gz_base::io_reason(&e)))
            })?;
            if ext == "ogf" {
                let (object, c) = compiled(bytes, &files_name(&source), None, at)?;
                made_over = Some(c);
                (object, None, Some(source))
            } else {
                let object = Object::read(&bytes)
                    .map_err(|e| Failure::new(UNREADABLE, at, format!("{}: {e}", files_name(&source))))?;
                (object, None, Some(source))
            }
        } else {
            let src = gz_fmt_mesh::read(&source).map_err(|e| Failure::new(UNREADABLE, at, e.to_string()))?;
            (src.object.clone(), Some(src), Some(source))
        }
    } else {
        let object = parts::build(&asset.parts, &asset.surfaces)
            .map_err(|e| Failure::new(CANNOT, "parts", format!("the parts make no model: {e}")))?;
        (object, None, None)
    };
    if asset.dynamic && made_over.is_some() {
        over_compiled(&asset, made_over.as_ref(), &mut issues);
    } else {
        transform(&mut object, &asset, &mut issues);
    }
    surfaces(&mut object, &asset, mesh_source.as_ref(), root, &abs);
    if asset.dynamic && made_over.is_some() {
        // the skeleton, the motions and the levels of detail are the compiled model's own: the
        // object is its geometry at rest, for the picture, the rays and the size
        object.flags.insert(ObjectFlags::DYNAMIC);
        if let Some(u) = &asset.userdata {
            object.user_data.clone_from(u);
        }
    } else if asset.dynamic {
        object.flags.insert(ObjectFlags::DYNAMIC);
        if object.bones.is_empty() {
            bind_to_root(&mut object);
            issues.push(Issue::new(
                NO_BONES,
                Severity::Note,
                vec![Seg::key("kind")],
                "the source has no bones: the dynamic model gets one bone, 'root'",
            ));
        }
        bones(&mut object, &asset);
        animations(&mut object, &asset);
        if let Some(u) = &asset.userdata {
            object.user_data.clone_from(u);
        }
    } else {
        object.flags.remove(ObjectFlags::DYNAMIC);
    }
    Ok(Model { asset, object, issues, source, compiled: made_over, stamp })
}

/// What a dynamic model over a compiled one cannot take, said at its keys: the file keeps its
/// own skeleton and motions, and only what its parts wear, its user data and its bones' mass
/// and material are said again.
fn over_compiled(asset: &ModelAsset, compiled: Option<&Compiled>, issues: &mut Vec<Issue>) {
    let mut warn = |path: Vec<Seg>, message: String| {
        issues.push(Issue::new(CANNOT, Severity::Warning, path, message));
    };
    if asset.scale != 1.0 {
        warn(
            vec![Seg::key("scale")],
            "'scale' is not applied to a dynamic model over a compiled one: its skeleton is the file's own"
                .into(),
        );
    }
    if asset.rotate.iter().any(|a| *a != 0.0) {
        warn(
            vec![Seg::key("rotate")],
            "'rotate' is not applied to a dynamic model over a compiled one: turn it where it is placed"
                .into(),
        );
    }
    // a row sets the speed and the flags of a motion the model itself holds: the game reads them
    // from the description. A motion is made of no clip here, and one of a shared motion file
    // keeps what it has.
    let motions = compiled.map(|c| c.motions.as_slice()).unwrap_or_default();
    for row in &asset.animations {
        let at = vec![Seg::key("animations"), Seg::item("name", &row.name)];
        if !motions.iter().any(|m| m.eq_ignore_ascii_case(&row.name)) {
            let known = if motions.is_empty() {
                "it holds none".to_owned()
            } else {
                format!("it holds: {}", motions.iter().take(8).cloned().collect::<Vec<_>>().join(", "))
            };
            warn(
                at,
                format!(
                    "the compiled model holds no motion '{}' of its own ({known}): a motion of a motion file it names is shared with other models and keeps its speed and flags",
                    row.name
                ),
            );
        } else if row.clip.is_some() {
            let mut at = at;
            at.push(Seg::key("clip"));
            warn(at, "'clip' is not applied: a compiled model's motions are its own".into());
        }
    }
    let bones = compiled.map(|c| c.bones.as_slice()).unwrap_or_default();
    for row in &asset.bones {
        // a bone's `shape` names the kind alone: the game switches to it when the model holds
        // the size of such a shape for the bone, and keeps the bone's own otherwise
        if !bones.iter().any(|b| b.eq_ignore_ascii_case(&row.name)) {
            let known = bones.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
            warn(
                vec![Seg::key("bones"), Seg::item("name", &row.name)],
                format!("the compiled model has no bone '{}' (it has: {known})", row.name),
            );
        }
    }
}

fn files_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The scene's object of a model asset: what the library's provider answers.
pub fn library_object(root: &Path, key: &str, game: &GameModel) -> Result<Arc<LibraryObject>, Failure> {
    let m = model(root, key, game)?;
    // a dynamic model is published as a model of its own: the scene's layers name it by that name
    let published = m.object.is_dynamic().then(|| {
        let module = match gz_mod::read_manifest_file(root) {
            Ok(Some(text)) => gz_mod::Manifest::parse(&gz_mod::ManifestDoc::parse(&text)).id,
            _ => String::new(),
        };
        crate::files::published_name(m.asset.name.as_deref(), key, &module)
    });
    let mut o = LibraryObject::provided(key, m.object);
    o.stamp = m.stamp;
    o.published = published.map(Into::into);
    Ok(Arc::new(o))
}

fn transform(object: &mut Object, asset: &ModelAsset, issues: &mut Vec<Issue>) {
    let scale = asset.scale as f32;
    let turn = asset.rotate.iter().any(|a| *a != 0.0);
    let rot = if turn && object.bones.is_empty() {
        parts::rotation(asset.rotate)
    } else {
        if turn {
            issues.push(Issue::new(
                CANNOT,
                Severity::Warning,
                vec![Seg::key("rotate")],
                "'rotate' is not applied to a model with bones: turn it where it is placed",
            ));
        }
        gz_fmt_object::glam::Mat3::IDENTITY
    };
    if scale == 1.0 && rot == gz_fmt_object::glam::Mat3::IDENTITY {
        return;
    }
    for m in &mut object.meshes {
        let pts: Vec<Vec3> = m.points.iter().map(|p| rot * (*p * scale)).collect();
        m.points = Arc::from(pts);
        m.recompute_bbox();
    }
    for b in &mut object.bones {
        b.rest_offset *= scale;
    }
}

/// The asset's surfaces over the source's, by name; a surface still without a texture takes the
/// texture asset made for its picture.
fn surfaces(object: &mut Object, asset: &ModelAsset, source: Option<&MeshSource>, root: &Path, abs: &Path) {
    let module = std::cell::OnceCell::new();
    for (i, s) in object.surfaces.iter_mut().enumerate() {
        if let Some(row) = asset.surfaces.iter().find(|r| r.name.eq_ignore_ascii_case(&s.name)) {
            if let Some(t) = &row.texture {
                s.set_texture(t);
            }
            if let Some(x) = &row.shader {
                s.shader = x.as_str().into();
            }
            if let Some(x) = &row.compile {
                s.shader_xrlc = x.as_str().into();
            }
            if let Some(x) = &row.material {
                s.game_mtl = x.as_str().into();
            }
            if let Some(x) = row.two_sided {
                s.flags.set(SurfaceFlags::TWO_SIDED, x);
            }
        }
        if !s.texture.is_empty() {
            continue;
        }
        let Some(src) = source else { continue };
        let Some((pi, pic)) =
            src.materials.get(i).and_then(|m| m.picture).and_then(|p| src.pictures.get(p).map(|x| (p, x)))
        else {
            continue;
        };
        let asset_of_picture = match &pic.data {
            // a picture the document names outside the project is none of its textures
            PictureData::File(p) if !files::inside(root, p) => continue,
            PictureData::File(p) => p.with_extension("texasset"),
            PictureData::Embedded(_) => {
                let stem = abs.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                abs.with_file_name(format!("{}.texasset", embedded_stem(&stem, &pic.name, pi)))
            }
        };
        if let Ok(text) = std::fs::read_to_string(&asset_of_picture) {
            let rel = asset_of_picture
                .strip_prefix(root)
                .map_or_else(|_| asset_of_picture.display().to_string(), |r| r.display().to_string());
            let name = kind::load(MediaKind::Texture, &rel.replace('\\', "/"), &text)
                .ok()
                .and_then(|v| TextureAsset::from_value(&v).0.name);
            let module: &String = module.get_or_init(|| files::module_id(root));
            s.set_texture(&files::published_name(name.as_deref(), &rel, module));
        }
    }
}

/// The file name an embedded picture of a model is written out as: `<model>_<picture>`.
#[must_use]
pub fn embedded_stem(model: &str, picture: &str, index: usize) -> String {
    let clean: String =
        picture.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let clean = clean.trim_matches('_');
    if clean.is_empty() { format!("{model}_{index}") } else { format!("{model}_{clean}") }
}

/// One bone, `root`, that every vertex hangs on fully.
fn bind_to_root(object: &mut Object) {
    for m in &mut object.meshes {
        let n = m.points.len();
        let w = m.vmaps.len() as u32;
        m.vmaps.push(VMap {
            name: "root".into(),
            kind: VMapKind::Weight,
            polymap: false,
            values: vec![1.0; n],
            vindices: (0..n as u32).collect(),
            pindices: Vec::new(),
        });
        let mut point_of = vec![None; m.vmrefs.len()];
        for f in m.faces.iter() {
            for c in f.v {
                if let Some(slot) = point_of.get_mut(c.vmref as usize) {
                    *slot = Some(c.point);
                }
            }
        }
        for (r, point) in m.vmrefs.iter_mut().zip(point_of) {
            if let Some(p) = point {
                r.push(VMapPt { vmap: w, index: p });
            }
        }
        let _ = m.rebuild_vmaps();
    }
    object.bones = vec![Bone { name: "root".into(), wmap: "root".into(), ..Bone::default() }];
    object.bone_parts = vec![BonePart { alias: "default".into(), bones: vec!["root".into()] }];
    let _ = object.prepare_bones();
}

/// The bones' physics the asset sets, each shape fitted around the bone's own vertices.
fn bones(object: &mut Object, asset: &ModelAsset) {
    if asset.bones.is_empty() {
        return;
    }
    let rest = gz_fmt_object::rest_pose(&object.bones);
    let single = object.bones.len() == 1;
    for i in 0..object.bones.len() {
        let name = object.bones[i].name.clone();
        let Some(row) = asset.bones.iter().find(|r| r.name.eq_ignore_ascii_case(&name)) else { continue };
        if let Some(m) = row.mass {
            object.bones[i].mass = m as f32;
        }
        // the row's material is the description's: the game takes it when it is a dynamic one
        // and keeps the model's own otherwise. Written into the model, another kind of material
        // would stop the game as the model loads - so the model keeps the one its source has.
        let Some(shape) = row.shape else { continue };
        let wmap = object.bones[i].wmap.clone();
        let mut b = gz_fmt_object::Aabb::INVALID;
        for m in &object.meshes {
            if single {
                m.points.iter().for_each(|p| b.modify(*p));
                continue;
            }
            for map in m.vmaps.iter().filter(|v| v.kind == VMapKind::Weight && v.name == wmap) {
                for (k, p) in map.vindices.iter().enumerate() {
                    if map.values.get(k).is_some_and(|w| *w >= 0.5)
                        && let Some(pt) = m.points.get(*p as usize)
                    {
                        b.modify(*pt);
                    }
                }
            }
        }
        if b.min.x > b.max.x {
            continue;
        }
        // the shape lives in the bone's space
        let inv = rest.inverse.get(i).copied();
        let (lo, hi) = match inv {
            Some(m) => {
                let mut l = gz_fmt_object::Aabb::INVALID;
                for c in 0..8 {
                    l.modify(m.transform_tiny(b.corner(c)));
                }
                (l.min, l.max)
            }
            None => (b.min, b.max),
        };
        let (centre, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
        let s = &mut object.bones[i].shape;
        s.kind = shape as u16;
        match shape {
            x if x == ShapeType::Box as u32 => {
                s.box_translate = centre;
                s.box_halfsize = half;
            }
            x if x == ShapeType::Sphere as u32 => {
                s.sphere_center = centre;
                s.sphere_radius = half.length();
            }
            x if x == ShapeType::Cylinder as u32 => {
                s.cylinder_center = centre;
                s.cylinder_direction = Vec3::Y;
                s.cylinder_height = half.y * 2.0;
                s.cylinder_radius = half.x.max(half.z);
            }
            _ => {}
        }
    }
}

/// The source's clips renamed and set by the asset's rows.
fn animations(object: &mut Object, asset: &ModelAsset) {
    for row in &asset.animations {
        let clip = row.clip.as_deref().unwrap_or(&row.name);
        let Some(m) = object.motions.iter_mut().find(|m| m.name.eq_ignore_ascii_case(clip)) else { continue };
        m.name = row.name.as_str().into();
        if let Some(s) = row.speed {
            m.speed = s as f32;
        }
        if let Some(l) = row.looped {
            m.flags.set(SMotionFlags::STOP_AT_END, !l);
        }
        // in the order of `ANIMATION_FLAGS`
        const BITS: [SMotionFlags; 6] = [
            SMotionFlags::NO_MIX,
            SMotionFlags::SYNC_PART,
            SMotionFlags::USE_FOOT_STEPS,
            SMotionFlags::ROOT_MOVER,
            SMotionFlags::IDLE,
            SMotionFlags::USE_WEAPON_BONE,
        ];
        for (on, bit) in row.flags.iter().zip(BITS) {
            if let Some(on) = on {
                m.flags.set(bit, *on);
            }
        }
    }
}

/// A texture asset made into its picture and its description.
#[derive(Clone, Debug)]
pub struct Texture {
    pub asset: TextureAsset,
    pub name: String,
    pub params: TextureParams,
    pub image: Image,
    pub source: PathBuf,
    /// The picture file's own extension, lower case (how the renderer reads its bytes).
    pub ext: String,
    pub issues: Vec<Issue>,
}

/// The source picture of a texture asset, without decoding it: the file and its extension.
pub fn texture_source(root: &Path, rel: &str) -> Result<(TextureAsset, PathBuf, Vec<Issue>), Failure> {
    let abs = root.join(rel.replace('/', "\\"));
    let text = read_text(&abs, rel)?;
    let (asset, issues) = loaded(MediaKind::Texture, rel, &text, TextureAsset::from_value)?;
    let has_source = asset.source.as_deref().is_some_and(|s| !s.trim().is_empty());
    let source = files::source_of(MediaKind::Texture, &abs, asset.source.as_deref()).ok_or_else(|| {
        Failure::new(
            MISSING,
            if has_source { "source" } else { "" },
            format!(
                "the picture is not there: {}",
                files::expected_source(MediaKind::Texture, &abs, asset.source.as_deref())
            ),
        )
    })?;
    let source = in_project(root, source, if has_source { "source" } else { "" })?;
    Ok((asset, source, issues))
}

/// A texture asset of the project with its picture decoded. `name` is its published name.
pub fn texture(root: &Path, rel: &str, name: &str) -> Result<Texture, Failure> {
    let (asset, source, issues) = texture_source(root, rel)?;
    let at = if asset.source.is_some() { "source" } else { "" };
    let bytes = std::fs::read(&source).map_err(|e| {
        Failure::new(UNREADABLE, at, format!("{}: {}", source.display(), gz_base::io_reason(&e)))
    })?;
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let loaded = gz_fmt_tex::load_image(&bytes, Some(&ext))
        .map_err(|e| Failure::new(UNREADABLE, at, format!("{}: {e}", files_name(&source))))?;
    let params = asset.to_params(name, (loaded.image.width, loaded.image.height), loaded.has_alpha);
    Ok(Texture { asset, name: name.to_owned(), params, image: loaded.image, source, ext, issues })
}

/// A sound asset of the project: its parameters and its source.
pub fn sound(root: &Path, rel: &str) -> Result<(SoundAsset, PathBuf, Vec<Issue>), Failure> {
    let abs = root.join(rel.replace('/', "\\"));
    let text = read_text(&abs, rel)?;
    let (asset, issues) = loaded(MediaKind::Sound, rel, &text, SoundAsset::from_value)?;
    let has_source = asset.source.as_deref().is_some_and(|s| !s.trim().is_empty());
    let source = files::source_of(MediaKind::Sound, &abs, asset.source.as_deref()).ok_or_else(|| {
        Failure::new(
            MISSING,
            if has_source { "source" } else { "" },
            format!(
                "the sound is not there: {}",
                files::expected_source(MediaKind::Sound, &abs, asset.source.as_deref())
            ),
        )
    })?;
    let source = in_project(root, source, if has_source { "source" } else { "" })?;
    Ok((asset, source, issues))
}

//! The levels of the game as scenes of the library (`docs/GAME_LIBRARY.md`, 5 and 6).
//!
//! A level of the game is read from the files the game loads: `level` and `level.geom` (what
//! it draws), `level.cform` (what stops a body), `level.ai`, the entities of `all.spawn` that
//! stand on it with their own names, its patrol paths, lights, glows, static sounds, particle
//! effects and environment modifiers. Out of them come
//!
//! - the scene of the level (`<level>.scene` and its folder), written with the scene's own
//!   writer, so it is the text any scene is;
//! - the object `@level\<level>`: the level's picture as a model, its collision as that model's
//!   own solid mesh ([`geometry`]);
//! - the passport: what the level is like, worked out of its collision ([`passport`]).

use std::sync::Arc;

use gz_base::SmolStr;
use gz_editor::Editor;
use gz_entities::{Entity, Settings};
use gz_fmt_level::{CForm, CompiledLevel, EnvMods, GeomData, LevelGraph};
use gz_fmt_mesh::OgfPart;
use gz_fmt_spawn::{SpawnFile, SpawnIndex, peek_packet};
use gz_scene::class::ObjClass;
use gz_scene::facts::{self, Limits, Soup, mm, v3};
use gz_scene::library::{LibraryObject, SolidMesh, SurfaceUse};
use gz_scene::math::Vec3;
use gz_scene::model::{ObjData, Object, SceneState};
use gz_scene::objects::{
    FV_EMITTER, FV_OCCLUSION, FogData, GlowData, LT_POINT, LightData, ParticlesData, PointKind,
    SceneObjectData, SoundEnvData, SoundSrcData, SpawnData, SpawnEntity, WayData, WayPoint, light_flags,
};
use gz_vfs::GameFiles;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde_json::{Map, Value, json};

use crate::layout::{Layout, write_atomic};
use crate::service::GameLibrary;
use crate::state::{Stamps, join_stamps};
use crate::sync::Progress;

/// The version of what a level's scene and passport state.
pub const VERSION: u32 = 7;
/// The reference of a level's geometry: `@level\<level>`.
pub const GEOMETRY_PREFIX: &str = "@level\\";
/// The name of the object that is the level's geometry.
pub const GEOMETRY_NAME: &str = "level_geometry";

/// The game's configs as `gz-entities` asks them.
pub struct LtxSettings<'a>(pub &'a gz_ltx::Ltx);

impl Settings for LtxSettings<'_> {
    fn section_exist(&self, section: &str) -> bool {
        self.0.section_exist(&section.to_ascii_lowercase())
    }

    fn r_string(&self, section: &str, key: &str) -> Option<&str> {
        let s = section.to_ascii_lowercase();
        if !self.0.line_exist(&s, key) {
            return None;
        }
        Some(self.0.r_string_raw(&s, key).ok().flatten().unwrap_or(""))
    }
}

fn read(files: &GameFiles, level: &str, name: &str) -> Option<Vec<u8>> {
    files.read(&format!("levels/{level}/{name}")).ok()
}

/// The names of the game materials by their id: what a collision face names its material by.
fn material_names(files: &GameFiles) -> FxHashMap<u16, SmolStr> {
    files
        .read("gamemtl.xr")
        .ok()
        .and_then(|b| gz_fmt_xr::GameMaterials::read(&b).ok())
        .map(|l| l.materials.iter().map(|m| (m.id as u16, m.name.clone())).collect())
        .unwrap_or_default()
}

/// A level's collision as a solid mesh: the faces, and per face its game material's name.
fn solid_of(cform: &CForm, names: &FxHashMap<u16, SmolStr>) -> SolidMesh {
    let mut ids: FxHashMap<u16, u16> = FxHashMap::default();
    let mut materials: Vec<SmolStr> = Vec::new();
    let mut tri_material = Vec::with_capacity(cform.tris.len());
    for t in cform.tris.iter() {
        let m = t.material();
        let next = materials.len() as u16;
        let slot = *ids.entry(m).or_insert_with(|| {
            materials.push(names.get(&m).cloned().unwrap_or_else(|| SmolStr::new(format!("material_{m}"))));
            next
        });
        tri_material.push(slot);
    }
    SolidMesh::new(
        cform.verts.to_vec(),
        cform.tris.iter().map(|t| t.verts).collect(),
        materials,
        tri_material,
    )
}

/// The geometry of a level of the game there is now: its picture as the model, its collision
/// as the model's solid mesh. Blocking.
pub fn geometry(editor: &Arc<Editor>, level: &str) -> Option<LibraryObject> {
    let root = GameLibrary::game_root(editor);
    if root.is_empty() {
        return None;
    }
    let files = GameLibrary::of(editor).files_blocking(editor, &root).ok()?;
    geometry_of(&files, level)
}

/// [`geometry`] over a game's files.
pub fn geometry_of(files: &GameFiles, level: &str) -> Option<LibraryObject> {
    let level_bytes = read(files, level, "level")?;
    let compiled = CompiledLevel::read(&level_bytes).ok()?;
    let geom = GeomData::read(&read(files, level, "level.geom")?).ok()?;
    // the visuals by what they wear: one part a texture and shader pair
    let mut parts: Vec<OgfPart> = Vec::new();
    let mut slot: FxHashMap<(String, String), usize> = FxHashMap::default();
    for v in &compiled.visuals {
        if !v.kind.has_geometry() {
            continue;
        }
        let Some(tris) = geom.triangles(v) else { continue };
        let (texture, shader) = match compiled.shader_of(v.shader) {
            Some(e) => (e.texture.to_owned(), e.shader.to_owned()),
            None => (String::new(), String::new()),
        };
        let i = *slot.entry((texture.clone(), shader.clone())).or_insert_with(|| {
            parts.push(OgfPart {
                texture: SmolStr::new(&texture),
                shader: SmolStr::new(&shader),
                corners: Vec::new(),
            });
            parts.len() - 1
        });
        // a static model's reference stands where its transform puts it
        let place = |p: Vec3| match &v.transform {
            Some(m) => Vec3::new(
                p.x * m[0][0] + p.y * m[1][0] + p.z * m[2][0] + m[3][0],
                p.x * m[0][1] + p.y * m[1][1] + p.z * m[2][1] + m[3][1],
                p.x * m[0][2] + p.y * m[1][2] + p.z * m[2][2] + m[3][2],
            ),
            None => p,
        };
        for t in tris.as_chunks::<3>().0 {
            let p = [place(t[0].0), place(t[1].0), place(t[2].0)];
            let n = (p[1] - p[0]).cross(p[2] - p[0]).try_normalize().unwrap_or(Vec3::Y);
            for k in 0..3 {
                parts[i].corners.push((p[k], n, t[k].1.into()));
            }
        }
    }
    let key = format!("{GEOMETRY_PREFIX}{level}");
    let object = gz_fmt_mesh::object_of_parts(&parts, &key).ok()?.object;
    let mut made = LibraryObject::provided(&key, object);
    if let Some(cform) = read(files, level, "level.cform").and_then(|b| CForm::read(&b).ok()) {
        made = made.with_solid(solid_of(&cform, &material_names(files)));
    }
    // the level's files as one number: a level that changed is another model
    made.stamp = level_bytes
        .iter()
        .take(1 << 16)
        .fold(0xcbf2_9ce4_8422_2325_u64 ^ level_bytes.len() as u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    Some(made)
}

/// What `all.spawn` holds, read once for every level.
pub struct Spawn {
    pub file: SpawnFile,
    pub index: SpawnIndex,
}

impl Spawn {
    /// Reads the game's `all.spawn`. Blocking.
    pub fn read(files: &GameFiles) -> Result<Self, String> {
        let bytes: Arc<[u8]> = files.read("spawns/all.spawn").map_err(|e| format!("all.spawn: {e}"))?.into();
        let file = SpawnFile::parse(bytes).map_err(|e| format!("all.spawn does not read: {e}"))?;
        let index = SpawnIndex::build(&file);
        Ok(Self { file, index })
    }

    /// The id the game graph gives a level.
    #[must_use]
    pub fn level_id(&self, level: &str) -> Option<u8> {
        self.index.levels.iter().find(|l| l.name.eq_ignore_ascii_case(level)).map(|l| l.id)
    }
}

/// A name no object of the scene has yet: the name itself, or with a number after it.
fn free_name(state: &SceneState, name: &str, fallback: &str) -> String {
    let base = if name.trim().is_empty() { fallback } else { name.trim() };
    state.unique_name(base)
}

/// The heading and pitch (radians) that turn an object's forward axis along a direction.
fn rotation_of(dir: Vec3) -> Vec3 {
    let d = dir.try_normalize().unwrap_or(Vec3::Z);
    Vec3::new(d.y.clamp(-1.0, 1.0).asin(), (-d.x).atan2(d.z), 0.0)
}

/// What a level's scene was made from, kept for the passport.
#[derive(Default)]
pub struct Counts {
    pub spawns: usize,
    pub undecoded: usize,
    pub ways: usize,
    pub lights: usize,
    pub glows: usize,
    pub sounds: usize,
    pub particles: usize,
    pub env_mods: usize,
    pub fog_volumes: usize,
    pub sound_envs: usize,
}

/// A level of the game as a scene: the geometry object and everything that stands on the level.
pub fn scene_of(
    files: &GameFiles,
    level: &str,
    spawn: Option<&Spawn>,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<(SceneState, Counts), String> {
    let compiled = read(files, level, "level")
        .ok_or_else(|| format!("the game has no level '{level}'"))
        .and_then(|b| CompiledLevel::read(&b).map_err(|e| format!("levels\\{level}\\level: {e}")))?;
    let mut s = SceneState::default();
    let mut counts = Counts::default();
    s.meta.level.level_path = SmolStr::new(level);
    s.insert(Object::new(
        ObjClass::SceneObject,
        GEOMETRY_NAME,
        ObjData::SceneObject(SceneObjectData::library(&format!("{GEOMETRY_PREFIX}{level}"))),
    ));

    // the entities of the game's spawn that stand on the level
    if let Some(sp) = spawn
        && let Some(id) = sp.level_id(level)
    {
        for (e, v) in sp.index.entities.iter().zip(sp.file.vertices.iter()) {
            if e.level != Some(id) || e.damaged {
                continue;
            }
            let Ok(head) = peek_packet(&v.spawn) else { continue };
            let packet: gz_base::Blob = Arc::from(v.spawn.as_slice());
            let mut d = SpawnData::of_kind(PointKind::Spawn);
            let decoded = settings.and_then(|st| Entity::from_spawn_packet(&packet, st).ok());
            match decoded {
                Some(entity) => d.entity = Some(SpawnEntity::Decoded(Arc::new(entity))),
                None => {
                    counts.undecoded += 1;
                    d.entity = Some(SpawnEntity::RawPacket { section: e.section.clone(), packet });
                }
            }
            let name = free_name(&s, &e.name, &e.section);
            let mut o = Object::new(ObjClass::SpawnPoint, &name, ObjData::Spawn(Box::new(d)));
            o.xform.position = head.position;
            o.xform.rotation = head.angle;
            // a zone's volume is the point's shape, where the zone stands
            gz_scene::objects::sync_shape(&mut o);
            s.insert(o);
            counts.spawns += 1;
        }
        // the patrol paths whose first point lies on the level
        if let Some(graph) = &sp.file.game_graph {
            for p in &sp.file.patrols {
                let Some((_, first)) = p.points.first() else { continue };
                if graph.vertices.get(usize::from(first.game_vertex)).map(|g| g.level_id) != Some(id) {
                    continue;
                }
                // a point's place in the list by its vertex id
                let place: FxHashMap<u32, u32> =
                    p.points.iter().enumerate().map(|(i, (id, _))| (*id, i as u32)).collect();
                let mut points: Vec<WayPoint> = p
                    .points
                    .iter()
                    .map(|(_, pt)| {
                        let mut w = WayPoint::new(&pt.name, pt.position);
                        w.flags = pt.flags;
                        w.selected = false;
                        w
                    })
                    .collect();
                for l in &p.links {
                    if let Some(from) = place.get(&l.from).and_then(|i| points.get_mut(*i as usize)) {
                        from.links =
                            l.to.iter().filter_map(|(to, w)| place.get(to).map(|i| (*i, *w))).collect();
                    }
                }
                let name = free_name(&s, &p.name, "way");
                s.insert(Object::new(ObjClass::Way, &name, ObjData::Way(WayData { kind: 0, points })));
                counts.ways += 1;
            }
        }
    }

    for (i, l) in compiled.lights.iter().enumerate() {
        let d = LightData {
            kind: if (1..=3).contains(&l.kind) { l.kind } else { LT_POINT },
            color: [l.diffuse[0], l.diffuse[1], l.diffuse[2], 0.0],
            range: l.range,
            flags: light_flags::AFFECT_DYNAMIC,
            ..LightData::default()
        };
        let mut o = Object::new(ObjClass::Light, &format!("light_{i:04}"), ObjData::Light(d));
        o.xform.position = l.position;
        o.xform.rotation = rotation_of(l.direction);
        s.insert(o);
        counts.lights += 1;
    }
    for (i, g) in compiled.glows.iter().enumerate() {
        let (texture, shader) = match compiled.shader_of(g.shader) {
            Some(e) => (SmolStr::new(e.texture), SmolStr::new(e.shader)),
            None => (SmolStr::default(), SmolStr::default()),
        };
        let d = GlowData { texture, shader, radius: g.size, ..GlowData::default() };
        let mut o = Object::new(ObjClass::Glow, &format!("glow_{i:04}"), ObjData::Glow(d));
        o.xform.position = g.position;
        s.insert(o);
        counts.glows += 1;
    }
    if let Some(list) =
        read(files, level, "level.snd_static").and_then(|b| gz_fmt_level::read_static_sounds(&b).ok())
    {
        for (i, snd) in list.iter().enumerate() {
            let ms = |v: [u32; 2]| [v[0] as f32 / 1000.0, v[1] as f32 / 1000.0];
            let d = SoundSrcData {
                wav: snd.wave.clone(),
                position: snd.pos,
                volume: snd.volume,
                freq: snd.freq,
                active_time: ms(snd.active_time),
                play_time: ms(snd.play_time),
                random_pause: ms(snd.random_pause),
                ..SoundSrcData::default()
            };
            let mut o = Object::new(ObjClass::SoundSrc, &format!("sound_{i:04}"), ObjData::SoundSrc(d));
            o.xform.position = snd.pos;
            s.insert(o);
            counts.sounds += 1;
        }
    }
    if let Some(list) =
        read(files, level, "level.ps_static").and_then(|b| gz_fmt_level::read_static_particles(&b).ok())
    {
        for (i, ps) in list.iter().enumerate() {
            let d = ParticlesData { reference: ps.reference.clone(), game_type: ps.game_type };
            let mut o = Object::new(ObjClass::Particles, &format!("particles_{i:04}"), ObjData::Particles(d));
            let m = &ps.transform;
            o.xform.position = Vec3::new(m[3][0], m[3][1], m[3][2]);
            // the heading and pitch of the system's forward axis
            o.xform.rotation = rotation_of(Vec3::new(m[2][0], m[2][1], m[2][2]));
            s.insert(o);
            counts.particles += 1;
        }
    }
    if let Some(mods) = read(files, level, "level.env_mod").and_then(|b| EnvMods::read(&b).ok()) {
        let rgb = |c: Vec3| {
            let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
            0xff00_0000 | (f(c.x) << 16) | (f(c.y) << 8) | f(c.z)
        };
        for (i, m) in mods.mods.iter().enumerate() {
            let mut d = SpawnData::of_kind(PointKind::EnvMod);
            d.env.radius = m.radius;
            d.env.power = m.power;
            d.env.view_dist = m.view_dist;
            d.env.fog_color = rgb(m.fog_color);
            d.env.fog_density = m.fog_density;
            d.env.ambient_color = rgb(m.ambient);
            d.env.sky_color = rgb(m.sky_color);
            d.env.hemi_color = rgb(m.hemi_color);
            // a file from before the flags: every value applies
            d.env.flags = if mods.version.is_some() { m.flags } else { 0xffff };
            let mut o =
                Object::new(ObjClass::SpawnPoint, &format!("env_mod_{i:04}"), ObjData::Spawn(Box::new(d)));
            o.xform.position = m.pos;
            s.insert(o);
            counts.env_mods += 1;
        }
    }
    // the fog volumes: an emitter with the volumes it flows around, one group each
    if let Some(groups) =
        read(files, level, "level.fog_vol").and_then(|b| gz_fmt_level::read_fog_volumes(&b).ok())
    {
        let put = |s: &mut SceneState, name: String, m: &[[f32; 4]; 4], data: FogData| {
            let parts = gz_scene::math::box_parts(&gz_scene::math::Mat4(*m));
            let mut o = Object::new(ObjClass::FogVolume, &name, ObjData::FogVolume(data));
            o.xform.position = parts.position;
            o.xform.rotation = parts.rotation;
            o.xform.scale = parts.size;
            s.insert(o);
        };
        for (i, g) in groups.iter().enumerate() {
            let group_id = i as u32;
            let emitter =
                FogData { kind: FV_EMITTER, group_id, profile: g.profile.clone(), ..FogData::default() };
            put(&mut s, format!("fog_{i:04}"), &g.emitter, emitter);
            for (k, m) in g.occluders.iter().enumerate() {
                let wall = FogData {
                    kind: FV_OCCLUSION,
                    group_id,
                    profile: g.profile.clone(),
                    ..FogData::default()
                };
                put(&mut s, format!("fog_{i:04}_wall_{k}"), m, wall);
            }
            counts.fog_volumes += 1;
        }
    }
    // the sound environments: the boxes the level's file was made of, each with the environment
    // inside it and the one above it
    for (i, (m, inner, outer)) in read(files, level, "level.snd_env")
        .map(|b| sound_env_boxes(&b))
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let parts = gz_scene::math::box_parts(m);
        let d = SoundEnvData { inner: inner.clone(), outer: outer.clone(), ..SoundEnvData::default() };
        let mut o = Object::new(ObjClass::SoundEnv, &format!("sound_env_{i:04}"), ObjData::SoundEnv(d));
        o.xform.position = parts.position;
        o.xform.rotation = parts.rotation;
        o.xform.scale = parts.size;
        s.insert(o);
        counts.sound_envs += 1;
    }
    Ok((s, counts))
}

/// The boxes of a level's sound environments (`level.snd_env`: the environments' names, then a
/// collision form whose faces say the environment in front of them and the one behind): the
/// matrix of each box with the environment inside it and the one outside. The file is made box by
/// box, twelve faces each: a box is told from the corners its twelve faces name, whatever order
/// they are listed in and whether or not touching boxes share them. Faces that make no box give
/// none.
fn sound_env_boxes(bytes: &[u8]) -> Vec<(gz_scene::math::Mat4, SmolStr, SmolStr)> {
    let r = gz_chunk::Reader::new(bytes);
    let (Ok(Some(named)), Ok(Some(geometry))) = (r.open_chunk(0), r.open_chunk(1)) else { return Vec::new() };
    let mut names: Vec<SmolStr> = Vec::new();
    let mut nr = named.reader();
    while !nr.eof() {
        let Ok(n) = nr.r_stringz() else { break };
        names.push(SmolStr::new(n));
    }
    let Ok(form) = CForm::read(geometry.bytes()) else { return Vec::new() };
    let mut out = Vec::new();
    for faces in form.tris.as_chunks::<12>().0 {
        // the corners the twelve faces name, each place once
        let mut corners: Vec<Vec3> = Vec::with_capacity(8);
        for index in faces.iter().flat_map(|t| t.verts) {
            let Some(p) = form.verts.get(index as usize).copied() else {
                corners.clear();
                break;
            };
            if !corners.iter().any(|c| (*c - p).length() < 1e-3) {
                corners.push(p);
            }
        }
        let Some(m) = box_of_corners(&corners) else { continue };
        let word = faces[0].dummy;
        let (front, back) = ((word & 0xffff) as usize, (word >> 16) as usize);
        let (Some(outer), Some(inner)) = (names.get(front), names.get(back)) else { continue };
        out.push((m, inner.clone(), outer.clone()));
    }
    out
}

/// The matrix of the unit box whose corners these eight places are, in any order; `None` when
/// they are no box. The axes are the three edges that meet at a corner, taken so that each runs
/// along the world axis it is nearest to: the same box reads the same whatever corner comes first.
fn box_of_corners(corners: &[Vec3]) -> Option<gz_scene::math::Mat4> {
    if corners.len() != 8 {
        return None;
    }
    let o = corners[0];
    let has = |p: Vec3| corners.iter().any(|c| (*c - p).length() < 0.01);
    let square = |a: Vec3, b: Vec3| a.dot(b).abs() <= 1e-3 * a.length() * b.length();
    // three of the other corners at the ends of edges at right angles, with every corner where
    // those edges put it
    let mut edges = None;
    'found: for a in 1..8 {
        for b in a + 1..8 {
            for c in b + 1..8 {
                let (i, j, k) = (corners[a] - o, corners[b] - o, corners[c] - o);
                if [i, j, k].iter().all(|e| e.length() > 1e-4)
                    && square(i, j)
                    && square(i, k)
                    && square(j, k)
                    && [i + j, i + k, j + k, i + j + k].into_iter().all(|e| has(o + e))
                {
                    edges = Some([i, j, k]);
                    break 'found;
                }
            }
        }
    }
    let mut edges = edges?;
    // each edge to the world axis it runs along, pointing up that axis: the largest share first
    let mut axes = [Vec3::ZERO; 3];
    let mut taken = [false; 3];
    for _ in 0..3 {
        let mut best: Option<(f32, usize, usize)> = None;
        for (e, edge) in edges.iter().enumerate() {
            for (axis, used) in taken.iter().enumerate() {
                if *used || *edge == Vec3::ZERO {
                    continue;
                }
                let share = edge.to_array()[axis].abs() / edge.length();
                if best.is_none_or(|(s, ..)| share > s) {
                    best = Some((share, e, axis));
                }
            }
        }
        let (_, e, axis) = best?;
        let edge = edges[e];
        axes[axis] = if edge.to_array()[axis] < 0.0 { -edge } else { edge };
        taken[axis] = true;
        edges[e] = Vec3::ZERO;
    }
    let centre = corners.iter().copied().sum::<Vec3>() / 8.0;
    let mut m = gz_scene::math::Mat4::IDENTITY;
    for (row, v) in [axes[0], axes[1], axes[2], centre].into_iter().enumerate() {
        m.0[row][0] = v.x;
        m.0[row][1] = v.y;
        m.0[row][2] = v.z;
    }
    Some(m)
}

/// What grows on a level (`level.details`): the pictures its plants wear and how much of the
/// level's grid holds any.
fn details_of(files: &GameFiles, level: &str) -> Option<Value> {
    let d = gz_fmt_level::LevelDetails::read(&read(files, level, "level.details")?).ok()?;
    let covered = d.slots.iter().filter(|s| !s.is_empty()).count();
    let mut textures: Vec<&str> = d.models.iter().map(|m| m.texture.as_str()).collect();
    textures.sort_unstable();
    textures.dedup();
    let side = gz_fmt_level::DETAIL_SLOT_SIZE;
    Some(json!({
        "plants": d.models.len(),
        "textures": textures,
        "slot": side,
        "grid": [d.header.size_x, d.header.size_z],
        "covered": covered,
        "covered_m2": (covered as f32 * side * side).round(),
    }))
}

/// What a structure of a level looks like, told from its shape and what it is made of.
fn structure_kind(
    size: Vec3,
    floors: usize,
    footprint: f32,
    level_footprint: f32,
    materials: &[&str],
) -> &'static str {
    let has = |word: &str| materials.iter().any(|m| m.contains(word));
    let longest = size.x.max(size.z);
    let shortest = size.x.min(size.z).max(0.01);
    if footprint >= level_footprint * 0.3 && has("earth") || has("grass") && footprint > 2000.0 {
        "ground"
    } else if has("bush") || has("tree") || has("leaf") {
        "vegetation"
    } else if has("water") && size.y < 2.0 {
        "water"
    } else if size.y >= 8.0 && longest < size.y * 0.6 {
        "tower"
    } else if longest / shortest > 8.0 && shortest < 2.5 {
        if size.y < 3.5 { "fence" } else { "pipe" }
    } else if floors >= 2 && size.y >= 3.0 && shortest >= 4.0 {
        "building"
    } else if floors >= 1 && size.y < 3.0 && footprint >= 6.0 {
        "platform"
    } else {
        "other"
    }
}

/// The passport of a level: what it is like, worked out of its collision, its AI map and what
/// stands on it.
#[must_use]
pub fn passport(files: &GameFiles, level: &str, state: &SceneState, counts: &Counts) -> Value {
    let mut out = Map::new();
    out.insert("card".into(), json!(VERSION));
    out.insert("kind".into(), json!("level"));
    out.insert("name".into(), json!(level));
    out.insert("scene".into(), json!(format!("{level}.scene")));
    let names = material_names(files);
    let ai = read(files, level, "level.ai").and_then(|b| LevelGraph::read(&b).ok());
    // where the AI map's nodes are: cells of one metre, the heights of the nodes in each
    let mut cells: FxHashMap<(i32, i32), Vec<f32>> = FxHashMap::default();
    if let Some(g) = &ai {
        for n in g.nodes.iter() {
            let p = g.position(n);
            cells.entry((p.x.floor() as i32, p.z.floor() as i32)).or_default().push(p.y);
        }
    }
    let cell = ai.as_ref().map_or(0.7, |g| g.header.cell_size);
    let walk = |rect: [f32; 4], y_min: f32, y_max: f32| -> (usize, f32) {
        if cells.is_empty() {
            return (0, 0.0);
        }
        let mut n = 0usize;
        for x in rect[0].floor() as i32..=rect[2].floor() as i32 {
            for z in rect[1].floor() as i32..=rect[3].floor() as i32 {
                if let Some(ys) = cells.get(&(x, z)) {
                    n += ys.iter().filter(|y| **y >= y_min - 0.6 && **y <= y_max + 0.6).count();
                }
            }
        }
        (n, n as f32 * cell * cell)
    };
    if let Some(cform) = read(files, level, "level.cform").and_then(|b| CForm::read(&b).ok()) {
        let solid = solid_of(&cform, &names);
        let tri_surface: Vec<u32> = solid.tri_material.iter().map(|m| u32::from(*m)).collect();
        let uses = vec![SurfaceUse { draws: false, collides: true }; solid.materials.len()];
        let limits = Limits { floors: 64, parts: 1200, parts_from: 0.0, part_size: 3.0, ground_share: 0.03 };
        let f = facts::measure(
            &Soup {
                verts: &solid.verts,
                tris: &solid.tris,
                tri_surface: &tri_surface,
                uses: &uses,
                surfaces: uses.len(),
            },
            &limits,
        );
        let size = f.bounds.max - f.bounds.min;
        out.insert("box".into(), facts::box_json(&f.bounds));
        out.insert("size".into(), v3(size));
        let level_foot = size.x * size.z;
        // how much room there is over a floor: the nearest solid face above it at a few of its
        // points, or the open sky
        let clearance = |fl: &facts::Floor| -> Option<f32> {
            let bvh = solid.bvh.as_ref()?;
            let (cx, cz) = ((fl.rect[0] + fl.rect[2]) * 0.5, (fl.rect[1] + fl.rect[3]) * 0.5);
            let (hx, hz) = ((fl.rect[2] - fl.rect[0]) * 0.3, (fl.rect[3] - fl.rect[1]) * 0.3);
            let mut found: Vec<f32> = Vec::new();
            let mut open = 0usize;
            for (dx, dz) in [(0.0, 0.0), (-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz)] {
                let at = Vec3::new(cx + dx, fl.y_max + 0.6, cz + dz);
                // the point is over the floor itself, not over a hole in its rectangle
                let down = gz_fmt_level::Ray::new(at, Vec3::NEG_Y, fl.y_max - fl.y_min + 1.2);
                let Some(hit) = bvh.ray_nearest(&down) else { continue };
                let from = Vec3::new(at.x, at.y - hit.t + 0.15, at.z);
                let mut up = gz_fmt_level::Ray::new(from, Vec3::Y, 80.0);
                up.cull_back = false;
                match bvh.ray_nearest(&up) {
                    Some(h) => found.push(h.t + 0.15),
                    None => open += 1,
                }
            }
            if found.len() <= open {
                return None;
            }
            found.sort_by(f32::total_cmp);
            found.get(found.len() / 2).copied()
        };
        let floor_json = |fl: &facts::Floor| {
            let mut j = fl.to_json();
            let (nodes, covered) = walk(fl.rect, fl.y_min, fl.y_max);
            if let Some(m) = j.as_object_mut() {
                // creatures walk where the AI map has nodes
                m.insert("ai_nodes".into(), json!(nodes));
                m.insert("walkable".into(), json!(covered >= fl.area * 0.25 && nodes > 0));
                match clearance(fl) {
                    Some(c) => m.insert("ceiling".into(), json!(mm(c))),
                    None => m.insert("sky".into(), json!(true)),
                };
            }
            j
        };
        // what a piece is told from: its shape and what it is made of
        let kind_of = |p: &facts::Part| -> &'static str {
            let psize = p.bounds.max - p.bounds.min;
            let mats: Vec<&str> = p
                .surfaces
                .iter()
                .filter_map(|s| solid.materials.get(*s as usize).map(SmolStr::as_str))
                .collect();
            let all =
                |words: &[&str]| !mats.is_empty() && mats.iter().all(|m| words.iter().any(|w| m.contains(w)));
            if all(&["fake"]) {
                "invisible_wall"
            } else if all(&["grass", "earth", "gravel", "sand", "dirt", "water", "default"])
                && p.floors.len() < 2
            {
                "terrain"
            } else {
                structure_kind(psize, p.floors.len(), p.footprint, level_foot, &mats)
            }
        };
        let mut structures: Vec<Value> = Vec::new();
        let mut walls: Vec<Value> = Vec::new();
        let (mut vegetation, mut terrain) = (0usize, 0usize);
        for p in &f.parts {
            let psize = p.bounds.max - p.bounds.min;
            match kind_of(p) {
                "vegetation" => vegetation += 1,
                "terrain" | "ground" => terrain += 1,
                "invisible_wall" => {
                    if walls.len() < 32 {
                        walls.push(json!({ "box": facts::box_json(&p.bounds), "size": v3(psize) }));
                    }
                }
                kind => {
                    let mats: Vec<&str> = p
                        .surfaces
                        .iter()
                        .filter_map(|s| solid.materials.get(*s as usize).map(SmolStr::as_str))
                        .collect();
                    let mut row = json!({
                        "id": structures.len() + 1,
                        "kind": kind,
                        "box": facts::box_json(&p.bounds),
                        "size": v3(psize),
                        "footprint": mm(p.footprint),
                        "triangles": p.triangles,
                        "materials": mats,
                        "floors": p.floors.iter().map(floor_json).collect::<Vec<_>>(),
                    });
                    if p.more_floors > 0
                        && let Some(m) = row.as_object_mut()
                    {
                        m.insert("more_floors".into(), json!(p.more_floors));
                    }
                    structures.push(row);
                }
            }
        }
        if !f.ground.is_empty() {
            out.insert("ground".into(), json!(f.ground.iter().map(floor_json).collect::<Vec<_>>()));
        }
        out.insert("structures".into(), json!(structures));
        out.insert(
            "also".into(),
            json!({
                "note": "pieces left out of structures: plants, slopes of the ground, walls nobody sees, and pieces under 3 m",
                "vegetation": vegetation,
                "terrain_pieces": terrain,
                "small_pieces": f.more_parts,
                "invisible_walls": walls,
            }),
        );
        let materials: Vec<Value> = {
            let mut rows: Vec<(f32, &SmolStr, u32)> = f
                .surfaces
                .iter()
                .zip(&solid.materials)
                .map(|(s, name)| (s.area, name, s.triangles))
                .collect();
            rows.sort_by(|a, b| b.0.total_cmp(&a.0));
            rows.iter()
                .map(|(a, n, t)| json!({ "material": n.as_str(), "area": mm(*a), "triangles": t }))
                .collect()
        };
        out.insert("materials".into(), json!(materials));
        out.insert(
            "collision".into(),
            json!({ "triangles": cform.tris.len(), "vertices": cform.verts.len() }),
        );
        // the ground: the highest collision under each point of a grid
        if let Some(bvh) = &solid.bvh {
            let step = (size.x.max(size.z) / 64.0).ceil().max(4.0);
            let (nx, nz) = ((size.x / step).floor() as usize + 1, (size.z / step).floor() as usize + 1);
            let top = f.bounds.max.y + 1.0;
            let rays: Vec<gz_fmt_level::Ray> = (0..nz)
                .flat_map(|iz| (0..nx).map(move |ix| (ix, iz)))
                .map(|(ix, iz)| {
                    let p =
                        Vec3::new(f.bounds.min.x + ix as f32 * step, top, f.bounds.min.z + iz as f32 * step);
                    gz_fmt_level::Ray::new(p, Vec3::NEG_Y, size.y + 2.0)
                })
                .collect();
            let hits = bvh.rays_nearest(&rays);
            let rows: Vec<Value> = hits
                .chunks(nx)
                .map(|row| {
                    Value::Array(
                        row.iter()
                            .map(|h| h.as_ref().map_or(Value::Null, |h| json!(mm(top - h.t))))
                            .collect(),
                    )
                })
                .collect();
            out.insert(
                "top".into(),
                json!({
                    "note": "the highest solid face under each point: roofs and crowns where there are any",
                    "origin": [mm(f.bounds.min.x), mm(f.bounds.min.z)],
                    "step": step,
                    "rows": rows,
                }),
            );
        }
    }
    if let Some(g) = &ai {
        let mut b = gz_scene::math::Aabb::INVALID;
        for n in g.nodes.iter() {
            b.modify(g.position(n));
        }
        out.insert(
            "ai".into(),
            json!({ "nodes": g.nodes.len(), "cell": mm(g.header.cell_size), "box": facts::box_json(&b) }),
        );
    }
    out.insert(
        "counts".into(),
        json!({
            "spawns": counts.spawns,
            "ways": counts.ways,
            "lights": counts.lights,
            "glows": counts.glows,
            "sounds": counts.sounds,
            "particles": counts.particles,
            "env_mods": counts.env_mods,
            "fog_volumes": counts.fog_volumes,
            "sound_environments": counts.sound_envs,
        }),
    );
    if counts.undecoded > 0 {
        out.insert("undecoded_spawns".into(), json!(counts.undecoded));
    }
    // what a designer looks for among the entities, by section
    let mut by_section: FxHashMap<String, Vec<(String, Vec3)>> = FxHashMap::default();
    let mut classes: FxHashMap<String, usize> = FxHashMap::default();
    for o in state.objects.values() {
        let ObjData::Spawn(d) = &o.data else { continue };
        let Some(section) = d.section() else { continue };
        let class = d
            .entity
            .as_ref()
            .and_then(SpawnEntity::entity)
            .and_then(Entity::class_info)
            .map_or_else(|| "unknown".to_owned(), |c| c.clsid.trim().to_owned());
        *classes.entry(class).or_default() += 1;
        by_section.entry(section.to_owned()).or_default().push((o.name.to_string(), o.xform.position));
    }
    let mut sections: Vec<(String, Vec<(String, Vec3)>)> = by_section.into_iter().collect();
    sections.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let spawns: Vec<Value> = sections
        .iter()
        .map(|(section, list)| {
            let mut row = Map::new();
            row.insert("section".into(), json!(section));
            row.insert("count".into(), json!(list.len()));
            // the few a section has are named with their places; the many are counted
            if list.len() <= 24 {
                row.insert(
                    "at".into(),
                    Value::Array(list.iter().map(|(n, p)| json!({ "name": n, "pos": v3(*p) })).collect()),
                );
            }
            Value::Object(row)
        })
        .collect();
    out.insert("spawns".into(), json!(spawns));
    // the grass and the small plants the level grows by itself: a module's objects stand among them
    if let Some(details) = details_of(files, level) {
        out.insert("details".into(), details);
    }
    let mut class_rows: Vec<(String, usize)> = classes.into_iter().collect();
    class_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.insert("classes".into(), Value::Object(class_rows.into_iter().map(|(c, n)| (c, json!(n))).collect()));
    Value::Object(out)
}

/// The files a level is made from, with what tells that one changed.
pub(crate) fn level_stamp(files: &GameFiles, stamps: &Stamps, level: &str, spawn_stamp: &str) -> String {
    const PARTS: [&str; 11] = [
        "level",
        "level.geom",
        "level.cform",
        "level.ai",
        "level.snd_static",
        "level.ps_static",
        "level.env_mod",
        "level.game",
        "level.fog_vol",
        "level.snd_env",
        "level.details",
    ];
    let mut parts: Vec<(String, String)> = PARTS
        .iter()
        .map(|p| {
            let path = format!("levels/{level}/{p}");
            ((*p).to_owned(), files.get(&path).map_or_else(|| "missing".to_owned(), |f| stamps.of_file(f)))
        })
        .collect();
    parts.push(("all.spawn".to_owned(), spawn_stamp.to_owned()));
    parts.push(("version".to_owned(), VERSION.to_string()));
    join_stamps(&parts)
}

/// The levels the game holds: the folders of `levels` with a `level` file.
#[must_use]
pub fn level_names(files: &GameFiles) -> Vec<String> {
    let mut out: Vec<String> = files
        .list("levels")
        .into_iter()
        .filter_map(|l| match l {
            gz_vfs::Listed::Dir(name) => Some(name.to_owned()),
            gz_vfs::Listed::File(_) => None,
        })
        .filter(|name| files.exists(&format!("levels/{name}/level")))
        .collect();
    out.sort();
    out
}

fn comment(files: &GameFiles, level: &str) -> String {
    format!(
        "Generated by Grizzly Editor from levels\\{level} and all.spawn of the game at {}.\nDo not edit: it is written again when the level's files change.",
        files.root().display()
    )
}

/// Writes one level: its scene, its parts and its passport.
fn make_level(
    files: &GameFiles,
    layout: &Layout,
    level: &str,
    spawn: Option<&Spawn>,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<(), String> {
    let (state, counts) = scene_of(files, level, spawn, settings)?;
    let plan = gz_scene::format::write(&state, None, settings)?;
    let dir = layout.level(level);
    // a level made again starts from nothing: a part it no longer has must not stay
    let folder = dir.join(level);
    let _ = std::fs::remove_dir_all(&folder);
    for out in &plan.files {
        let Some(bytes) = &out.bytes else { continue };
        let path =
            if out.file.is_empty() { dir.join(format!("{level}.scene")) } else { folder.join(&out.file) };
        let mut text = String::new();
        for l in comment(files, level).lines() {
            text.push_str("-- ");
            text.push_str(l);
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(bytes));
        write_atomic(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let pass = passport(files, level, &state, &counts);
    let text = crate::lua::file(&comment(files, level), &pass);
    write_atomic(&dir.join("passport.lua"), text.as_bytes())
        .map_err(|e| format!("the passport of {level}: {e}"))
}

/// The file beside a level's passport that says what the level was made from: the stamp on its
/// first line, and under it why the level could not be made when it could not. Written last, so
/// a level whose making was cut off is made again.
fn stamp_file(layout: &Layout, level: &str) -> std::path::PathBuf {
    layout.level(level).join("stamp.txt")
}

/// The levels of the game whose library entry is behind their files (every one with
/// `rebuild`), each with the stamp it should have. Blocking: no file of the game is opened.
#[must_use]
pub fn stale_levels(
    files: &GameFiles,
    stamps: &Stamps,
    layout: &Layout,
    rebuild: bool,
) -> Vec<(String, String)> {
    let spawn_stamp =
        files.get("spawns/all.spawn").map_or_else(|| "missing".to_owned(), |x| stamps.of_file(x));
    level_names(files)
        .into_iter()
        .filter_map(|name| {
            let stamp = level_stamp(files, stamps, &name, &spawn_stamp);
            let held = std::fs::read_to_string(stamp_file(layout, &name)).unwrap_or_default();
            (rebuild || held.lines().next() != Some(stamp.as_str())).then_some((name, stamp))
        })
        .collect()
}

/// Makes the scenes and the passports of the levels whose files changed. Answers the levels
/// made. A level that is done says so by itself (its stamp file): a run that is stopped or cut
/// off goes on with the levels that are left.
pub async fn sync_levels(
    editor: &Arc<Editor>,
    files: &Arc<GameFiles>,
    layout: &Layout,
    rebuild: bool,
    progress: &dyn Progress,
) -> Result<Vec<String>, String> {
    progress.stage("Reading the game's levels");
    let (f, l) = (files.clone(), layout.clone());
    let stale: Vec<(String, String)> =
        editor.exec.io(move || stale_levels(&f, &Stamps::of(&f), &l, rebuild)).await;
    if stale.is_empty() || progress.stopped() {
        return Ok(Vec::new());
    }
    let (f, l) = (files.clone(), layout.clone());
    let total = stale.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    // the progress is the job's: it is handed over as numbers, and the stop as a flag
    let (tx, rx) = std::sync::mpsc::channel::<usize>();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    let work = editor.exec.cpu(move || {
        // the entities and the configs they are read with: once for every level
        let spawn = Spawn::read(&f).ok();
        let tree =
            gz_game::configs::load_tree(&*f, gz_game::SYSTEM_LTX, None, "the game has no system.ltx").ok();
        let settings = tree.as_ref().map(|t| LtxSettings(&t.ltx));
        let st: Option<&(dyn Settings + Sync)> = settings.as_ref().map(|s| s as &(dyn Settings + Sync));
        // every level at once, on every core; a level not begun when Stop comes is left for the
        // next sync
        stale
            .par_iter()
            .filter_map(|(name, stamp)| {
                if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    return None;
                }
                let r = make_level(&f, &l, name, spawn.as_ref(), st);
                // done, or not to be made from these files: either way it is not tried again
                // until the level's files change
                let said = match &r {
                    Ok(()) => format!("{stamp}\n"),
                    Err(why) => format!("{stamp}\nproblem: {}\n", why.replace('\n', " ")),
                };
                let _ = write_atomic(&stamp_file(&l, name), said.as_bytes());
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let _ = tx.send(n);
                Some((name.clone(), r))
            })
            .collect::<Vec<(String, Result<(), String>)>>()
    });
    tokio::pin!(work);
    let results = loop {
        tokio::select! {
            r = &mut work => break r,
            () = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                if let Some(n) = rx.try_iter().last() {
                    progress.progress(n, total);
                }
                if progress.stopped() {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    };
    let mut made = Vec::new();
    let mut failed = Vec::new();
    for (name, r) in results {
        match r {
            Ok(()) => made.push(name),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    if made.is_empty() && !failed.is_empty() {
        return Err(format!("no level could be read: {}", failed.join("; ")));
    }
    Ok(made)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod env_tests {
    use super::*;

    /// A level's file as the X-Ray SDK makes it: two environments, one box.
    fn file(corners: [Vec3; 8], word: u32) -> Vec<u8> {
        let mut w = gz_chunk::Writer::new();
        w.chunk(0, |w| {
            w.w_stringz("open_space");
            w.w_stringz("room");
        });
        let faces: [[u32; 3]; 12] = [
            [0, 1, 2],
            [2, 3, 0],
            [3, 2, 4],
            [4, 5, 3],
            [4, 6, 7],
            [4, 7, 5],
            [6, 1, 7],
            [1, 0, 7],
            [3, 7, 0],
            [3, 5, 7],
            [1, 6, 4],
            [1, 4, 2],
        ];
        let tris = faces.iter().map(|v| gz_fmt_level::Tri { verts: *v, dummy: word }).collect();
        let form = CForm::new(corners.to_vec(), tris);
        w.chunk(1, |w| w.w(&form.write()));
        w.into_bytes()
    }

    #[test]
    fn a_level_s_sound_environment_boxes_read_back_with_what_is_inside_and_outside() {
        // a box 4 x 2 x 6 with its middle at (10, 1, -3)
        let at = |x: f32, y: f32, z: f32| Vec3::new(10.0 + x * 2.0, 1.0 + y, -3.0 + z * 3.0);
        let corners = [
            at(-1.0, -1.0, -1.0),
            at(-1.0, 1.0, -1.0),
            at(1.0, 1.0, -1.0),
            at(1.0, -1.0, -1.0),
            at(1.0, 1.0, 1.0),
            at(1.0, -1.0, 1.0),
            at(-1.0, 1.0, 1.0),
            at(-1.0, -1.0, 1.0),
        ];
        // in front of a face (outside) environment 0, behind it (inside) environment 1
        let boxes = sound_env_boxes(&file(corners, 1 << 16));
        assert_eq!(boxes.len(), 1);
        let (m, inner, outer) = &boxes[0];
        assert_eq!((inner.as_str(), outer.as_str()), ("room", "open_space"));
        let parts = gz_scene::math::box_parts(m);
        assert_eq!(parts.position, Vec3::new(10.0, 1.0, -3.0));
        assert_eq!(parts.size, Vec3::new(4.0, 2.0, 6.0));
        assert!(!parts.skewed);
        // corners that make no box give none
        let mut bent = corners;
        bent[4].y += 1.0;
        assert!(sound_env_boxes(&file(bent, 1 << 16)).is_empty());
        assert!(sound_env_boxes(b"not a file").is_empty());
        // the same box with its corners listed another way round is the same box
        let mut other = corners;
        other.reverse();
        other.swap(1, 5);
        let again = sound_env_boxes(&file(other, 1 << 16));
        assert_eq!(again.len(), 1);
        let p = gz_scene::math::box_parts(&again[0].0);
        assert_eq!(
            (p.position, p.size, p.skewed),
            (Vec3::new(10.0, 1.0, -3.0), Vec3::new(4.0, 2.0, 6.0), false)
        );
    }

    /// Two boxes that touch and share the corners of the face between them, as a tool that welds
    /// a form's points leaves them: sixteen corners said with twelve points.
    #[test]
    fn boxes_that_share_their_corners_are_two_boxes() {
        // unit cubes side by side along x: x from 0 to 1 and from 1 to 2
        let grid = |x: u32, y: u32, z: u32| x * 4 + y * 2 + z;
        let verts: Vec<Vec3> = (0..3u32)
            .flat_map(|x| {
                (0..2u32).flat_map(move |y| (0..2u32).map(move |z| Vec3::new(x as f32, y as f32, z as f32)))
            })
            .collect();
        let mut tris = Vec::new();
        for (b, word) in [(0u32, 1u32 << 16), (1, 0)] {
            let c = |x: u32, y: u32, z: u32| grid(b + x, y, z);
            // two faces a side, in no particular order of corners
            let quads = [
                [c(0, 0, 0), c(0, 1, 0), c(0, 1, 1), c(0, 0, 1)],
                [c(1, 0, 0), c(1, 0, 1), c(1, 1, 1), c(1, 1, 0)],
                [c(0, 0, 0), c(0, 0, 1), c(1, 0, 1), c(1, 0, 0)],
                [c(0, 1, 0), c(1, 1, 0), c(1, 1, 1), c(0, 1, 1)],
                [c(0, 0, 0), c(1, 0, 0), c(1, 1, 0), c(0, 1, 0)],
                [c(0, 0, 1), c(0, 1, 1), c(1, 1, 1), c(1, 0, 1)],
            ];
            for q in quads {
                tris.push(gz_fmt_level::Tri { verts: [q[0], q[1], q[2]], dummy: word });
                tris.push(gz_fmt_level::Tri { verts: [q[0], q[2], q[3]], dummy: word });
            }
        }
        let mut w = gz_chunk::Writer::new();
        w.chunk(0, |w| {
            w.w_stringz("open_space");
            w.w_stringz("room");
        });
        w.chunk(1, |w| w.w(&CForm::new(verts, tris).write()));
        let boxes = sound_env_boxes(&w.into_bytes());
        assert_eq!(boxes.len(), 2);
        let middles: Vec<Vec3> = boxes.iter().map(|(m, ..)| gz_scene::math::box_parts(m).position).collect();
        assert_eq!(middles, [Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.5, 0.5, 0.5)]);
        assert_eq!((boxes[0].1.as_str(), boxes[1].1.as_str()), ("room", "open_space"));
        assert!(boxes.iter().all(|(m, ..)| gz_scene::math::box_parts(m).size == Vec3::ONE));
    }
}

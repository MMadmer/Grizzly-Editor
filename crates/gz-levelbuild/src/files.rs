//! The level's files beside `build.prj`: `level.ltx` (`BuildLTX`), the game files of
//! `BuildGame`, the occlusion models (`BuildHOMModel`, `BuildSOMModel`), the AI map export, the
//! scene statistic (`BuildSceneStat`) and the collision form (`CFromBuilder`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use glam::Vec3;
use gz_chunk::Writer;
use gz_entities::Settings;
use gz_fmt_level::{
    Aabb, BuildAiMap, CForm, EnvModifier, EnvMods, FogGroup, LevelGame, RPoint, SPAWNPOINT_VERSION,
    StaticParticles, StaticSound, Tri, WayLink, WayObject, WayPoint, i_floor, write_fog_volumes,
    write_static_particles, write_static_sounds,
};
use gz_fmt_object::{ObjectFlags, SurfaceFlags};
use gz_scene::math::xform_box;
use gz_scene::objects::{FV_EMITTER, FV_OCCLUSION, PointKind, SHAPE_LEVEL_BOUND, SpawnEntity};
use gz_scene::{ObjClass, ObjData, Object, SceneState};

use crate::libs::{Engine, xrlc_flags};
use crate::prj::{Lookup, SceneStat, level_box, object_surfaces};
use crate::{BuildError, Result};

/// `IWriter::w_string`: the text and a line end.
fn line(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.extend_from_slice(b"\r\n");
}

/// `BuildLTX`: `level.ltx` - the map usage, the level map's rectangle and texture, the build
/// options' text, the sectors' maps.
#[must_use]
pub fn level_ltx(scene: &SceneState, lib: Lookup<'_>) -> Vec<u8> {
    let opts = &scene.meta.level;
    let mut out = Vec::new();
    line(&mut out, "[map_usage]");
    line(&mut out, &format!("ver={}", opts.map_version));
    for (bit, name) in [
        (1u16 << 1, "deathmatch"),
        (1 << 2, "teamdeathmatch"),
        (1 << 3, "artefacthunt"),
        (1 << 4, "capturetheartefact"),
        (1 << 5, "dominationzone"),
        (1 << 6, "teamdominationzone"),
    ] {
        if opts.map_usage & bit != 0 {
            line(&mut out, name);
        }
    }
    line(&mut out, ";");
    let mut bb = level_box(scene, lib).unwrap_or(Aabb::INVALID);
    for o in scene.of_class(ObjClass::Shape) {
        if let ObjData::Shape(s) = &o.data
            && s.usage == SHAPE_LEVEL_BOUND
        {
            if let Some(b) = s.world_box(&o.xform) {
                bb = b;
            }
            break;
        }
    }
    line(&mut out, "[level_map]");
    line(&mut out, &format!("bound_rect = {:.6},{:.6},{:.6},{:.6}", bb.min.x, bb.min.z, bb.max.x, bb.max.z));
    line(&mut out, &format!("texture = map\\map_{}", opts.level_path));
    line(&mut out, ";");
    if !opts.bop.is_empty() {
        out.extend_from_slice(opts.bop.as_bytes());
        out.push(0);
    }
    line(&mut out, ";");
    line(&mut out, "[sub_level_map]");
    for (num, o) in scene.of_class(ObjClass::Sector).enumerate() {
        if let ObjData::Sector(s) = &o.data {
            line(&mut out, &format!("{num} = {}", s.map_idx));
        }
    }
    out
}

/// The files of `BuildGame`, by name; a file the C++ writes only when it has something is
/// absent then.
#[derive(Clone, Debug, Default)]
pub struct GameFiles {
    pub files: BTreeMap<&'static str, Vec<u8>>,
    /// The breakable parts' models, `meshes\brkbl#N.ogf` relative to the level folder.
    pub meshes: Vec<(String, Vec<u8>)>,
    pub log: Vec<String>,
}

/// `u32_3f`: a colour as three floats.
fn u32_3f(c: u32) -> Vec3 {
    let f = 1.0 / 255.0;
    Vec3::new(f * ((c >> 16) & 0xff) as f32, f * ((c >> 8) & 0xff) as f32, f * (c & 0xff) as f32)
}

/// A spawn element's packet as `ExportGame` writes it: the entity named, placed and shaped
/// like the point.
pub fn export_entity(
    o: &Object,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<Option<(gz_entities::Entity, bool)>> {
    let ObjData::Spawn(pt) = &o.data else { return Ok(None) };
    let Some(e) = &pt.entity else { return Ok(None) };
    let mut entity = match e {
        SpawnEntity::Decoded(d) => (**d).clone(),
        raw => match settings.and_then(|s| raw.decode(s)) {
            Some(d) => d,
            None => return Err(BuildError::new(format!("!Invalid spawn data: {}", o.name))),
        },
    };
    let _ = entity.set_name(&o.name);
    let _ = entity.set_position(o.position());
    let _ = entity.set_angle(o.rotation());
    let has_shape = entity.class_info().is_some_and(|c| c.class.has_shape());
    if has_shape {
        let shape = pt.attached.as_ref().and_then(|a| match &a.data {
            ObjData::Shape(s) => Some((a.xform.scale, s)),
            _ => None,
        });
        let Some((scale, s)) = shape else {
            return Err(BuildError::new(format!("Spawn Point: '{}' must contain attached shape.", o.name)));
        };
        let _ = entity.set_shapes(&gz_export_level::spawn::entity_shapes(scale, &s.shapes));
    }
    Ok(Some((entity, pt.sd_flags & 1 != 0)))
}

/// `BuildGame`: the spawn elements' packets (`level.spawn`, the respawned ones in
/// `level_rs.spawn`) - the Scene Object tool's breakable and climable objects first, as the
/// tools export in class order -, the patrol paths and respawn points (`level.game`), the
/// environment modifiers, the static sounds and particles, the fog volumes.
pub fn game_files(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<GameFiles> {
    let mut out = GameFiles::default();
    let (mut spawn, mut spawn_rs) = (Writer::new(), Writer::new());
    let (mut n_spawn, mut n_rs) = (0u32, 0u32);
    let mut game = LevelGame::default();
    let mut env = EnvMods { version: Some(SPAWNPOINT_VERSION), mods: Vec::new() };
    let mut sounds = Vec::new();
    let mut particles = Vec::new();
    for class in ObjClass::ALL {
        if class == ObjClass::SceneObject {
            let (ok, made) = crate::breakable::export(scene, lib, engine, settings)?;
            if !ok {
                // the export's own lines say why; the C++ showed them before its error
                let mut text = made.log.join("\n");
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("ERROR: Failed to build game. ");
                return Err(BuildError::new(text));
            }
            out.log.extend(made.log);
            for packet in made.spawns {
                spawn.w_chunk(n_spawn, &packet);
                n_spawn += 1;
            }
            out.meshes = made.meshes;
        }
        for o in scene.of_class(class) {
            match &o.data {
                ObjData::Spawn(pt) => {
                    if pt.entity.is_some() {
                        let Some((e, respawn)) = export_entity(o, settings)? else { continue };
                        let packet = e.to_spawn_packet();
                        if respawn {
                            spawn_rs.w_chunk(n_rs, &packet);
                            n_rs += 1;
                        } else {
                            spawn.w_chunk(n_spawn, &packet);
                            n_spawn += 1;
                        }
                        continue;
                    }
                    match pt.kind {
                        PointKind::RPoint => game.rpoints.push(RPoint {
                            pos: o.position(),
                            rot: o.rotation(),
                            team: pt.rpoint.team,
                            kind: pt.rpoint.kind,
                            game_type: pt.rpoint.game_type,
                            profile: Some(pt.rpoint.profile.clone()),
                        }),
                        PointKind::EnvMod => {
                            let e = &pt.env;
                            env.mods.push(EnvModifier {
                                pos: o.position(),
                                radius: e.radius,
                                power: e.power,
                                view_dist: e.view_dist,
                                fog_color: u32_3f(e.fog_color),
                                fog_density: e.fog_density,
                                ambient: u32_3f(e.ambient_color),
                                sky_color: u32_3f(e.sky_color),
                                hemi_color: u32_3f(e.hemi_color),
                                flags: e.flags,
                            });
                        }
                        // not part of the level build: the module's spawn layer carries it
                        PointKind::PlayerStart | PointKind::Spawn => {}
                    }
                }
                ObjData::Way(w) => {
                    let mut links = Vec::new();
                    for (from, p) in w.points.iter().enumerate() {
                        for (to, prob) in &p.links {
                            if (*to as usize) < w.points.len() {
                                links.push(WayLink { from: from as u16, to: *to as u16, probability: *prob });
                            }
                        }
                    }
                    game.ways.push(WayObject {
                        name: o.name.clone(),
                        points: w
                            .points
                            .iter()
                            .map(|p| WayPoint { pos: p.position, flags: p.flags, name: p.name.clone() })
                            .collect(),
                        links,
                    });
                }
                ObjData::SoundSrc(s) => sounds.push(StaticSound {
                    wave: s.wav.clone(),
                    pos: s.position,
                    volume: s.volume,
                    freq: s.freq,
                    active_time: [(s.active_time[0] * 1000.0) as u32, (s.active_time[1] * 1000.0) as u32],
                    play_time: [(s.play_time[0] * 1000.0) as u32, (s.play_time[1] * 1000.0) as u32],
                    random_pause: [(s.random_pause[0] * 1000.0) as u32, (s.random_pause[1] * 1000.0) as u32],
                }),
                ObjData::Particles(p) => particles.push(StaticParticles {
                    game_type: p.game_type,
                    reference: p.reference.clone(),
                    transform: o.world().0,
                }),
                _ => {}
            }
        }
    }
    if n_spawn > 0 {
        out.files.insert("level.spawn", spawn.into_bytes());
    }
    if n_rs > 0 {
        out.files.insert("level_rs.spawn", spawn_rs.into_bytes());
    }
    out.files.insert("level.game", game.write());
    out.files.insert("level.env_mod", env.write());
    if !sounds.is_empty() {
        out.files.insert("level.snd_static", write_static_sounds(&sounds));
    }
    if !particles.is_empty() {
        out.files.insert("level.ps_static", write_static_particles(&particles));
    }
    // the fog volumes by group: the emitter first, then its occluders
    let mut groups: BTreeMap<u32, Vec<&Object>> = BTreeMap::new();
    for o in scene.of_class(ObjClass::FogVolume) {
        if let ObjData::FogVolume(f) = &o.data {
            groups.entry(f.group_id).or_default().push(o);
        }
    }
    let mut fog = Vec::new();
    let mut ok = true;
    'groups: for g in groups.values_mut() {
        g.sort_by_key(|o| match &o.data {
            ObjData::FogVolume(f) => f.kind,
            _ => 0,
        });
        let mut group: Option<FogGroup> = None;
        for o in g.iter() {
            let ObjData::FogVolume(f) = &o.data else { continue };
            match &mut group {
                None if f.kind == FV_EMITTER => {
                    group = Some(FogGroup {
                        profile: f.profile.clone(),
                        emitter: o.world().0,
                        occluders: Vec::new(),
                    })
                }
                Some(gr) if f.kind == FV_OCCLUSION => gr.occluders.push(o.world().0),
                _ => {
                    out.log.push("! incorrect fog volumes grouping".to_owned());
                    ok = false;
                    break 'groups;
                }
            }
        }
        if let Some(gr) = group {
            fog.push(gr);
        }
    }
    out.files.insert("level.fog_vol", write_fog_volumes(&fog));
    if !ok {
        return Err(BuildError::new("ERROR: Failed to build game. "));
    }
    Ok(out)
}

/// `BuildHOMModel`: the occluders - every face of a HOM object, the `materials\occ` faces of
/// the others - in world space with their two-sided flag. `None` when there is none.
#[must_use]
pub fn hom(scene: &SceneState, lib: Lookup<'_>) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib(&d.reference) else { continue };
        let hom = lo.object.flags.contains(ObjectFlags::HOM);
        let xf = o.world();
        for m in &lo.object.meshes {
            for sf in &m.surf_faces {
                let Some(s) = lo.object.surfaces.get(sf.surface as usize) else { continue };
                if !hom && s.game_mtl != "materials\\occ" {
                    continue;
                }
                let two = u32::from(s.flags.contains(SurfaceFlags::TWO_SIDED));
                for &f in sf.faces.iter() {
                    let Some(face) = m.faces.get(f as usize) else { continue };
                    for v in face.v {
                        body.w_vec3(
                            xf.transform_tiny(m.points.get(v.point as usize).copied().unwrap_or(Vec3::ZERO)),
                        );
                    }
                    body.w_u32(two);
                }
            }
        }
    }
    let body = body.into_bytes();
    if body.is_empty() {
        return None;
    }
    let mut w = Writer::new();
    w.chunk(0, |w| w.w_u32(0));
    w.w_chunk(1, &body);
    Some(w.into_bytes())
}

/// `BuildSOMModel`: the sound occluders with their materials' occlusion factors. `None` when
/// there is none; an error when a surface's material is unknown.
pub fn som(scene: &SceneState, lib: Lookup<'_>, engine: &Engine) -> Result<Option<Vec<u8>>> {
    let mut body = Writer::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib(&d.reference) else { continue };
        if !lo.object.flags.contains(ObjectFlags::SOUND_OCCLUDER) {
            continue;
        }
        let xf = o.world();
        for m in &lo.object.meshes {
            for sf in &m.surf_faces {
                let Some(s) = lo.object.surfaces.get(sf.surface as usize) else { continue };
                let Some(id) = engine.material_id(&s.game_mtl) else {
                    return Err(BuildError::new(format!(
                        "Object '{}', surface '{}' contain invalid game material.",
                        lo.name, s.name
                    )));
                };
                let Some(mtl) = engine.materials.as_ref().and_then(|l| l.by_id(id)) else {
                    return Err(BuildError::new(format!(
                        "Object '{}', surface '{}' contain undefined game material.",
                        lo.name, s.name
                    )));
                };
                let two = u32::from(s.flags.contains(SurfaceFlags::TWO_SIDED));
                for &f in sf.faces.iter() {
                    let Some(face) = m.faces.get(f as usize) else { continue };
                    for v in face.v {
                        body.w_vec3(
                            xf.transform_tiny(m.points.get(v.point as usize).copied().unwrap_or(Vec3::ZERO)),
                        );
                    }
                    body.w_u32(two);
                    body.w_f32(mtl.snd_occlusion_factor);
                }
            }
        }
    }
    let body = body.into_bytes();
    if body.is_empty() {
        return Ok(None);
    }
    let mut w = Writer::new();
    w.chunk(0, |w| w.w_u32(0));
    w.w_chunk(1, &body);
    Ok(Some(w.into_bytes()))
}

/// The AI Map tool's nodes as the scene holds them.
#[must_use]
pub fn ai_nodes(scene: &SceneState) -> Option<(Vec<gz_fmt_level::AiMapNode>, gz_fmt_level::AiParams)> {
    // the tool's exact nodes, as the C++ exports them (the packed ones are rounded)
    let part = scene.bulk.ai_map.as_ref()?;
    let nodes = part.all_nodes();
    (!nodes.is_empty()).then_some((nodes, part.header.params))
}

/// `ESceneAIMapTool::Export`: `build.aimap`. `None` for an empty map.
#[must_use]
pub fn build_aimap(scene: &SceneState) -> Option<Vec<u8>> {
    let (nodes, params) = ai_nodes(scene)?;
    BuildAiMap::export(&nodes, &params).map(|m| m.write())
}

/// `CSceneStat::flush`: the picture (`level_stat.tga`, red the static vertices, green the
/// models', bottom-up) and the text (`level_stat.txt`).
#[must_use]
pub fn scene_stat(stat: &SceneStat) -> (Vec<u8>, Vec<u8>) {
    let (sx, sz) = (stat.sx, stat.sz);
    let max_s = stat.svert.iter().copied().max().unwrap_or(0);
    let max_mu = stat.muvert.iter().copied().max().unwrap_or(0);
    let (total_s, total_mu): (u64, u64) =
        (stat.svert.iter().map(|v| u64::from(*v)).sum(), stat.muvert.iter().map(|v| u64::from(*v)).sum());
    let scale = |v: u32, max: u32| -> u8 {
        let f = v as f32 / max as f32 * 255.0 + 0.5;
        if f.is_finite() { i_floor(f) as u8 } else { 0 }
    };
    // `CImage::Vflip` then a top-down TGA: the last row of the grid comes first
    let mut rgba = Vec::with_capacity((sx * sz * 4) as usize);
    for iz in (0..sz).rev() {
        for ix in 0..sx {
            let i = (iz * sx + ix) as usize;
            let r = scale(stat.svert.get(i).copied().unwrap_or(0), max_s);
            let g = scale(stat.muvert.get(i).copied().unwrap_or(0), max_mu);
            rgba.extend_from_slice(&[r, g, 0, 0]);
        }
    }
    let tga = gz_fmt_tex::Image::new(sx, sz, rgba)
        .ok()
        .and_then(|i| gz_fmt_tex::write_tga(&i, gz_fmt_tex::TgaDepth::Bgra32).ok())
        .unwrap_or_default();
    let mut txt = String::new();
    let _ = write!(
        txt,
        "Map size X x Z:            [{sx} x {sz}]\r\nMax static vertex per m^2: {max_s}\r\nTotal static vertices:     {total_s}\r\nMax mu vertex per m^2:     {max_mu}\r\nTotal mu vertices:         {total_mu}\r\n"
    );
    (tga, txt.into_bytes())
}

/// `CFromBuilder::build`: the static and multiple-usage objects' collision faces in world
/// space - not `materials\occ`, not a compiler shader without collision, not a degenerate face,
/// both sides of a two-sided one. `None` when nothing is collected.
#[must_use]
pub fn collision_form(scene: &SceneState, lib: Lookup<'_>, engine: &Engine) -> Option<CForm> {
    let mut verts: Vec<Vec3> = Vec::new();
    let mut tris: Vec<Tri> = Vec::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        if d.visual_mode() {
            continue;
        }
        let Some(lo) = lib(&d.reference) else { continue };
        let f = lo.object.flags;
        if f.intersects(ObjectFlags::SOUND_OCCLUDER | ObjectFlags::DYNAMIC | ObjectFlags::HOM) {
            continue;
        }
        let surfaces = object_surfaces(o, &lo);
        let xf = o.world();
        for m in &lo.object.meshes {
            let offs = verts.len() as u32;
            verts.extend(m.points.iter().map(|p| xf.transform_tiny(*p)));
            for sf in &m.surf_faces {
                let Some(s) = surfaces.get(sf.surface as usize) else { continue };
                if s.game_mtl == "materials\\occ" {
                    continue;
                }
                if engine.xrlc_flags(&s.shader_xrlc).is_none_or(|fl| fl & xrlc_flags::COLLISION == 0) {
                    continue;
                }
                let material = engine.material_index(&s.game_mtl);
                for &fi in sf.faces.iter() {
                    let Some(face) = m.faces.get(fi as usize) else { continue };
                    let p = face.v.map(|v| m.points.get(v.point as usize).copied().unwrap_or(Vec3::ZERO));
                    let (e1, e2, e3) = (p[0].distance(p[1]), p[0].distance(p[2]), p[1].distance(p[2]));
                    let s2 = (e1 + e2 + e3) / 2.0;
                    let a = (s2 * (s2 - e1) * (s2 - e2) * (s2 - e3)).sqrt();
                    if !a.is_finite() || a < 0.000_01 {
                        continue;
                    }
                    tris.push(Tri::new(face.v.map(|v| v.point + offs), material, 0));
                    if s.flags.contains(SurfaceFlags::TWO_SIDED) {
                        tris.push(Tri::new(
                            [face.v[2].point + offs, face.v[1].point + offs, face.v[0].point + offs],
                            material,
                            0,
                        ));
                    }
                }
            }
        }
    }
    (!tris.is_empty()).then(|| CForm::new(verts, tris))
}

/// The world box of a scene object, from its library object.
#[must_use]
pub fn object_box(o: &Object, lib: Lookup<'_>) -> Option<Aabb> {
    let ObjData::SceneObject(d) = &o.data else { return None };
    lib(&d.reference).map(|lo| xform_box(&lo.bounds, &o.world()))
}

/// A shared library lookup over a cache.
pub fn lookup(
    lib: Arc<gz_scene::library::Library>,
) -> impl Fn(&str) -> Option<Arc<gz_scene::library::LibraryObject>> + Sync {
    move |n: &str| lib.cached(n)
}

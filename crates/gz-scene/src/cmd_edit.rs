//! The scene's commands that make, remove or move objects: `scene.place`, `scene.delete`,
//! `scene.duplicate`, `scene.copy`, `scene.paste`, `scene.group`, `scene.ungroup`,
//! `scene.align`, `scene.query`, `scene.attach`, `scene.motion`; and the Summary Info a
//! `scene.info summary=true` collects.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath, SmolStr};
use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec};
use gz_entities::Entity;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::apply::{SceneCmd, SceneReply, SelectMode, TransformEdit};
use crate::class::ObjClass;
use crate::cmd::{brief, r4, v3, wire};
use crate::edit;
use crate::format::{fields, group, values};
use crate::landing::{DROP_REACH, drop_to_ground};
use crate::library::normalize;
use crate::math::{Transform, Vec3};
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{
    PointKind, SceneObjectData, ShapeData, ShapeDef, SpawnData, SpawnEntity, WayData, default_data,
};
use crate::pick::ray_scene_objects;
use crate::sense::{self, QueryAction, QueryParams};
use crate::service::{self, SceneService};

pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::write(
            "scene.summary",
            "Writes what the open scene uses - objects, faces, textures and their memory - as a text file.",
        )
        .destroys_with(&["overwrite"])
        .example("path=import/escape.txt"),
        scene_summary,
    );
    reg.cmd(
        Spec::write(
            "scene.place",
            "Places an object, a light, a spawn point, a way path or another thing in the scene.",
        )
        .example("kind=object ref=props\\box_wood_01 position=1,0,2 on_surface=true"),
        scene_place,
    );
    reg.cmd(
        Spec::write(
            "scene.delete",
            "Removes objects (the selection without objects) from the scene; one undo step.",
        ),
        scene_delete,
    );
    reg.cmd(
        Spec::write(
            "scene.duplicate",
            "Adds a copy of objects (the selection without objects) under free names.",
        )
        .example("offset=2,0,0"),
        scene_duplicate,
    );
    reg.cmd(
        Spec::write(
            "scene.copy",
            "Copies objects (the selection without objects) to the project's clipboard file.",
        ),
        scene_copy,
    );
    reg.cmd(
        Spec::write("scene.paste", "Adds the clipboard's objects under free names and selects them.")
            .example("offset=0,0,5"),
        scene_paste,
    );
    reg.cmd(
        Spec::write(
            "scene.group",
            "Groups objects in place, saves them as a .group asset, moves a group's pivot or reloads groups from their asset.",
        )
        .example("objects=tent_01,fire_01 name=camp"),
        scene_group,
    );
    reg.cmd(
        Spec::write(
            "scene.ungroup",
            "Ungroups groups (the selected ones without objects); their objects stay.",
        ),
        scene_ungroup,
    );
    reg.cmd(
        Spec::write("scene.align", "Drops objects onto what is below them, or stands them on the surface.")
            .example("action=surface objects=barrel_01 offset=0.05"),
        scene_align,
    );
    reg.cmd(
        Spec::read(
            "scene.query",
            "Asks the scene's geometry: the ground under points, heights over an area, a ray, what is in a box, whether and where a model fits, the AI map, the walls of an area.",
        )
        .example("action=fit ref=props\\box_wood_01 position=10,0,3"),
        scene_query,
    );
    reg.cmd(
        Spec::read(
            "scene.audit",
            "Checks where things stand: what floats, is buried, stands in something, is twice there, is off the AI map.",
        )
        .example("objects=crate_01,crate_02 checks=floating,overlap"),
        scene_audit,
    );
    reg.cmd(
        Spec::read(
            "shot.map",
            "A map of the scene from above, drawn without a window: every object in its colour with its name, a grid in metres.",
        )
        .example("position=120,0,-40 size=60 highlight=bunker_wall_01"),
        shot_map,
    );
    reg.cmd(
        Spec::write(
            "scene.attach",
            "Makes a shape a spawn point's own, or gives a spawn point's shape back to the scene.",
        )
        .example("spawn=anomaly_01 object=shape_03"),
        scene_attach,
    );
    reg.cmd(
        Spec::write(
            "scene.motion",
            "The motion a dynamic object moves along: its keys, its range, the playback, import and export of .anm.",
        )
        .example("object=door_01 action=add_key"),
        scene_motion,
    );
}

/// The objects a command means, selected first when the caller named them (the commands of
/// the store work on the selection).
async fn select_named(
    svc: &SceneService,
    names: Option<&[String]>,
    o: Origin,
) -> Result<Vec<ObjId>, CmdError> {
    let s = svc.store.snapshot();
    let ids = edit::ids_of(&s, names)?;
    if names.is_some_and(|n| !n.is_empty()) {
        svc.store.send(SceneCmd::Select { ids: ids.clone(), mode: SelectMode::Set }, o).await?;
    }
    Ok(ids)
}

/// Where a placed thing lands without a position: ten metres in front of the camera.
fn in_front(s: &SceneState) -> Vec3 {
    let hpb = s.meta.camera_hpb;
    let (sh, ch) = hpb.x.sin_cos();
    let (sp, cp) = hpb.y.sin_cos();
    s.meta.camera_pos + Vec3::new(-cp * sh, sp, cp * ch) * 10.0
}

/// Degrees as the model's radians.
fn radians3(d: [f32; 3]) -> Vec3 {
    Vec3::new(
        values::radians(f64::from(d[0])),
        values::radians(f64::from(d[1])),
        values::radians(f64::from(d[2])),
    )
}

fn finite(v: [f32; 3], what: &str) -> Result<Vec3, CmdError> {
    let v = Vec3::from_array(v);
    if v.is_finite() { Ok(v) } else { Err(CmdError::invalid(format!("{what} takes finite numbers"))) }
}

/// Drops objects onto what is below them (a box under each, or the line under its pivot), then
/// `offset` metres up; answers the ones that landed.
async fn drop_ids(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    ids: &[ObjId],
    align: bool,
    line: bool,
    offset: f32,
    o: Origin,
) -> Result<Vec<ObjId>, CmdError> {
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    let ids = ids.to_vec();
    let s = snap.clone();
    let landings = ed.exec.cpu(move || drop_to_ground(&s, &lib, &pick, &ids, align, line)).await;
    if landings.is_empty() {
        return Ok(Vec::new());
    }
    let landed: Vec<ObjId> = landings.iter().map(|l| l.id).collect();
    let items = landings
        .into_iter()
        .map(|l| {
            let position = l.position + Vec3::Y * offset;
            (l.id, TransformEdit::Land { position, rotation: l.rotation })
        })
        .collect();
    let label = if align { "Align to Surface" } else { "Drop to Ground" };
    svc.store
        .send(SceneCmd::Transform { items, members: false, label: label.into(), merge: None }, o)
        .await?;
    Ok(landed)
}

// ---------------------------------------------------------------- place

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PlaceKind {
    /// A library object (`ref`).
    Object,
    /// A `.group` asset of the project or a group of the library (`ref`).
    Group,
    Light,
    Glow,
    /// A sound source (`ref`: the sound).
    Sound,
    /// A sound environment volume.
    SoundEnv,
    /// A particle effect (`ref`: the effect).
    Particles,
    /// A shape: a sphere, or a box with `box=true`.
    Shape,
    /// A fog volume.
    Fog,
    /// A spawn point (`section`: a spawn section of the game, or `player_start`, `env_mod`,
    /// `rpoint`).
    Spawn,
    /// A way path through `points`.
    Way,
}

impl PlaceKind {
    fn class(self) -> ObjClass {
        match self {
            Self::Object => ObjClass::SceneObject,
            Self::Group => ObjClass::Group,
            Self::Light => ObjClass::Light,
            Self::Glow => ObjClass::Glow,
            Self::Sound => ObjClass::SoundSrc,
            Self::SoundEnv => ObjClass::SoundEnv,
            Self::Particles => ObjClass::Particles,
            Self::Shape => ObjClass::Shape,
            Self::Fog => ObjClass::FogVolume,
            Self::Spawn => ObjClass::SpawnPoint,
            Self::Way => ObjClass::Way,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlaceParams {
    /// What to place.
    kind: PlaceKind,
    /// The library object, the group, the sound or the effect, as its library spells it.
    #[serde(default)]
    r#ref: Option<String>,
    /// The spawn section (kind=spawn).
    #[serde(default)]
    section: Option<String>,
    /// Its name; left out: one made from what it is (`box_wood_01`).
    #[serde(default)]
    name: Option<String>,
    /// Metres; left out: ten metres in front of the camera.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// Drop it onto what is below the position.
    #[serde(default)]
    on_surface: bool,
    /// With on_surface: stand it on the surface's slope as well.
    #[serde(default)]
    align: bool,
    /// Degrees.
    #[serde(default)]
    rotation: Option<[f32; 3]>,
    /// A factor per axis.
    #[serde(default)]
    scale: Option<[f32; 3]>,
    /// The way path's points, in metres (kind=way).
    #[serde(default)]
    points: Option<Vec<[f32; 3]>>,
    /// The way path leads from its last point back to its first (kind=way; default true).
    #[serde(default)]
    closed: Option<bool>,
    /// A box instead of a sphere (kind=shape).
    #[serde(default)]
    r#box: bool,
    /// What the shape is for (kind=shape): common (a volume of its own; the default), level_bound,
    /// ai_map_bounds (the AI map is generated inside it).
    #[serde(default)]
    usage: Option<String>,
}

/// The name a placed thing gets: the one asked for (refused when taken), or one made from `pref`.
fn name_for(s: &SceneState, asked: Option<&str>, pref: &str) -> Result<String, CmdError> {
    match asked.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if s.find_any(n).is_some() => {
            Err(CmdError::invalid(format!("'{n}' is the name of another object")))
        }
        Some(n) => Ok(n.to_owned()),
        None => Ok(s.gen_name(Some(pref))),
    }
}

async fn place_reply(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    id: ObjId,
    p: &PlaceParams,
    o: Origin,
) -> Reply {
    let mut landed = false;
    if p.on_surface && p.kind != PlaceKind::Way {
        landed = !drop_ids(ed, svc, &[id], p.align, false, 0.0, o).await?.is_empty();
    }
    let s = svc.store.snapshot();
    let obj = s.get(id).ok_or_else(|| CmdError::internal("the placed object is gone"))?;
    let mut out = json!({ "object": brief(&s, obj) });
    if let Value::Object(m) = &mut out {
        if p.on_surface {
            m.insert("landed".into(), json!(landed));
        }
        if let ObjData::Way(w) = &obj.data {
            m.insert("points".into(), json!(w.points.len()));
        }
    }
    Ok(out)
}

async fn scene_place(ed: Arc<Editor>, p: PlaceParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    let pos = match p.position {
        Some(v) => finite(v, "position")?,
        None => in_front(&snap),
    };
    let rot = p.rotation.map(|r| finite(r, "rotation").map(|_| radians3(r))).transpose()?;
    let scale = p.scale.map(|s| finite(s, "scale")).transpose()?;
    if scale.is_some_and(|s| s.x.abs() < 1e-6 || s.y.abs() < 1e-6 || s.z.abs() < 1e-6) {
        return Err(CmdError::invalid("a scale of zero"));
    }
    let class = p.kind.class();
    let caps = class.caps();
    if rot.is_some() && !caps.rotates {
        return Err(CmdError::invalid(format!(
            "a {} does not turn: it takes no rotation",
            class.label().to_lowercase()
        )));
    }
    if scale.is_some() && !caps.scales {
        return Err(CmdError::invalid(format!("a {} takes no scale", class.label().to_lowercase())));
    }
    let reference = p.r#ref.as_deref().map(|r| r.trim().replace('/', "\\")).filter(|r| !r.is_empty());
    let mut obj = match p.kind {
        PlaceKind::Object => {
            let reference =
                reference.ok_or_else(|| CmdError::invalid("kind=object takes ref: a library object"))?;
            let lib = svc.library.clone();
            let key = normalize(&reference);
            if ed.exec.io(move || lib.load(&key)).await.is_err() {
                return Err(CmdError::not_found(format!("there is no library object {reference}")));
            }
            // an object of the editor's library is copied into the project as it is placed
            let (root, library, r) = (svc.root.clone(), gz_project::library_root(&ed), reference.clone());
            let only_library =
                ed.exec.io(move || gz_project::library::resolve_object(&root, &library, &r)).await
                    == gz_project::library::Resolves::Library;
            if only_library && let Err(e) = gz_project::library::bring_in(&ed, vec![reference.clone()]) {
                ed.log.warn(format!("{reference} could not be copied into the project: {}", e.message));
            }
            let name = name_for(&snap, p.name.as_deref(), &reference)?;
            Object::new(class, &name, ObjData::SceneObject(SceneObjectData::library(&reference)))
        }
        PlaceKind::Group => {
            let reference = reference.ok_or_else(|| {
                CmdError::invalid("kind=group takes ref: a .group of the project or the library")
            })?;
            return place_group(&ed, &svc, &reference, p.name.as_deref(), pos, o).await;
        }
        PlaceKind::Spawn => {
            let section = p.section.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
                CmdError::invalid("kind=spawn takes section: a spawn section (game.lookup type=spawn_section), player_start, env_mod or rpoint")
            })?;
            spawn_object(&ed, &snap, section, p.name.as_deref(), pos).await?
        }
        PlaceKind::Way => {
            let points = p
                .points
                .as_deref()
                .filter(|l| !l.is_empty())
                .ok_or_else(|| CmdError::invalid("kind=way takes points: [[x,y,z], ...]"))?;
            let mut pts = points.iter().map(|v| finite(*v, "points")).collect::<Result<Vec<_>, _>>()?;
            if p.on_surface {
                svc.prefetch_library(&ed, &snap).await;
                let (s, pick) = svc.pick().await;
                let lib = svc.library.clone();
                pts = ed
                    .exec
                    .cpu(move || {
                        pts.into_iter()
                            .map(|q| {
                                match ray_scene_objects(
                                    &pick,
                                    &s,
                                    &lib,
                                    q + Vec3::Y * 2.0,
                                    Vec3::NEG_Y,
                                    DROP_REACH,
                                    None,
                                ) {
                                    Some(h) if h.point.is_finite() => h.point,
                                    _ => q,
                                }
                            })
                            .collect()
                    })
                    .await;
            }
            let name = name_for(&snap, p.name.as_deref(), "way")?;
            Object::new(class, &name, ObjData::Way(WayData::build(&pts, p.closed.unwrap_or(true))))
        }
        _ => {
            let name = name_for(&snap, p.name.as_deref(), reference.as_deref().unwrap_or(class.key()))?;
            let mut n = Object::new(class, &name, default_data(class));
            if let Some(r) = &reference {
                let key = match p.kind {
                    PlaceKind::Sound => "sound",
                    PlaceKind::Particles => "ref",
                    _ => return Err(CmdError::invalid(format!("kind={} takes no ref", class.key()))),
                };
                if let Some(f) = fields::object_field(class, key) {
                    (f.set)(&mut n, &values::text(r)).map_err(CmdError::invalid)?;
                }
            }
            if p.kind == PlaceKind::Shape
                && p.r#box
                && let ObjData::Shape(d) = &mut n.data
            {
                d.shapes = vec![ShapeDef::boxed(&crate::math::Mat4::IDENTITY)];
            }
            if let Some(u) = &p.usage {
                let f = fields::object_field(class, "usage")
                    .filter(|_| p.kind == PlaceKind::Shape)
                    .ok_or_else(|| CmdError::invalid(format!("kind={} takes no usage", class.key())))?;
                (f.set)(&mut n, &values::text(u)).map_err(|e| CmdError::invalid(format!("usage {e}")))?;
            }
            n
        }
    };
    if p.kind != PlaceKind::Way {
        let mut x = Transform { position: pos, rotation: obj.xform.rotation, scale: obj.xform.scale };
        if let Some(r) = rot {
            x.rotation = r;
        }
        if let Some(s) = scale {
            x.scale = s;
        }
        obj.xform = x;
        if let ObjData::Spawn(d) = &mut obj.data
            && let Some(a) = &mut d.attached
        {
            Arc::make_mut(a).xform.position = pos;
        }
        if let ObjData::SoundSrc(d) = &mut obj.data {
            d.position = pos;
        }
    }
    let SceneReply::Id(id) = svc.store.send(SceneCmd::Place(Box::new(obj)), o).await? else {
        return Err(CmdError::internal("the place gave no object"));
    };
    place_reply(&ed, &svc, id, &p, o).await
}

/// A spawn point of a section (or one of the editor's points), made as the game's factory makes
/// it, with its shape when its class has one.
async fn spawn_object(
    ed: &Arc<Editor>,
    snap: &SceneState,
    section: &str,
    name: Option<&str>,
    pos: Vec3,
) -> Result<Object, CmdError> {
    use crate::objects::{ENVMOD_CHOOSE_NAME, PLAYERSTART_CHOOSE_NAME, RPOINT_CHOOSE_NAME};
    let point = [PLAYERSTART_CHOOSE_NAME, ENVMOD_CHOOSE_NAME, RPOINT_CHOOSE_NAME]
        .into_iter()
        .find(|p| p.eq_ignore_ascii_case(section) || p[1..].eq_ignore_ascii_case(section));
    let data = match point {
        Some(PLAYERSTART_CHOOSE_NAME) => SpawnData::of_kind(PointKind::PlayerStart),
        Some(ENVMOD_CHOOSE_NAME) => SpawnData::of_kind(PointKind::EnvMod),
        Some(_) => SpawnData::of_kind(PointKind::RPoint),
        None => {
            let why = format!(
                "{section} is no spawn section of the linked game (game.lookup type=spawn_section lists them)"
            );
            let refused = move || CmdError::not_found(why.clone());
            let settings = crate::game::settings(ed).await.ok_or_else(|| {
                CmdError::invalid("spawn points need the linked game's configs: link the game first")
            })?;
            let sec = section.to_owned();
            let (entity, def_sphere, needs_shape) = ed
                .exec
                .io(move || -> Result<(Entity, Option<f32>, bool), CmdError> {
                    use gz_entities::Settings;
                    if !settings.section_exist(&sec) || !settings.line_exist(&sec, "$spawn") {
                        return Err(refused());
                    }
                    let e = Entity::create_spawn_point(&sec, settings.as_ref()).map_err(|e| {
                        CmdError::invalid(format!("the section {sec} makes no spawn point: {e}"))
                    })?;
                    let def = settings.r_float(&sec, "$def_sphere");
                    let needs = e.class_info().is_some_and(|c| c.class.has_shape());
                    Ok((e, def, needs))
                })
                .await?;
            let mut d = SpawnData::of_kind(PointKind::Spawn);
            d.entity = Some(SpawnEntity::Decoded(Arc::new(entity)));
            if def_sphere.is_some() || needs_shape {
                let shape = Object::new(
                    ObjClass::Shape,
                    "",
                    ObjData::Shape(ShapeData {
                        shapes: vec![ShapeDef::sphere(Vec3::ZERO, def_sphere.unwrap_or(5.0))],
                        usage: 0,
                    }),
                );
                d.attached = Some(Arc::new(shape));
            }
            d
        }
    };
    let pref = point.map_or(section, |p| &p[1..]);
    let name = name_for(snap, name, pref)?;
    let mut obj = Object::new(ObjClass::SpawnPoint, &name, ObjData::Spawn(Box::new(data)));
    obj.xform.position = pos;
    Ok(obj)
}

/// The `.group` asset of the project a reference names, when there is one.
fn project_group(root: &Path, reference: &str) -> Option<(PathBuf, RelPath)> {
    let p = PathBuf::from(reference.replace('/', "\\"));
    let p = if p.extension().is_none() { p.with_extension(group::EXT) } else { p };
    let full = if p.is_absolute() { p } else { root.join(p) };
    let rel = RelPath::from_fs(root, &full)?;
    full.is_file().then_some((full, rel))
}

/// A `.group` placed: the project's asset as one undo step of its objects, else a group of the
/// library read as the X-Ray SDK writes it.
async fn place_group(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    reference: &str,
    name: Option<&str>,
    pos: Vec3,
    o: Origin,
) -> Reply {
    let root = svc.root.clone();
    let r = reference.to_owned();
    if let Some((file, rel)) = ed.exec.io(move || project_group(&root, &r)).await {
        let (mut bundle, warnings) = edit::read_group_file(ed, file, rel.clone()).await?;
        for w in &warnings {
            ed.log.warn(w);
        }
        // a group saved whole becomes a group that names its asset
        if let [(gi, _)] = bundle.groups.as_slice()
            && let Some(g) = bundle.objects.get_mut(*gi)
            && let ObjData::Group(d) = &mut g.data
        {
            d.reference = SmolStr::new(rel.as_str());
            if let Some(n) = name {
                g.name = SmolStr::new(n);
            }
        }
        let reply = svc
            .store
            .send(SceneCmd::Paste { bundle: Box::new(bundle), offset: pos, label: "Place Group".into() }, o)
            .await?;
        let n = if let SceneReply::Count(n) = reply { n } else { 0 };
        let s = svc.store.snapshot();
        let placed: Vec<Value> =
            s.selected_in_order().iter().filter_map(|id| s.get(*id)).map(|x| brief(&s, x)).collect();
        return Ok(json!({ "objects": placed, "added": n, "warnings": warnings }));
    }
    let snap = svc.store.snapshot();
    let mut loaded = edit::library_group(ed, svc, reference).await?;
    loaded.obj.name = name_for(&snap, name, reference)?.into();
    if let ObjData::Group(g) = &mut loaded.obj.data {
        g.reference = reference.into();
    }
    let reply =
        svc.store.send(SceneCmd::PlaceGroup { loaded: Box::new(loaded), at: Transform::at(pos) }, o).await?;
    let SceneReply::Id(id) = reply else { return Err(CmdError::internal("the place gave no group")) };
    let s = svc.store.snapshot();
    let g = s.get(id).ok_or_else(|| CmdError::internal("the placed group is gone"))?;
    Ok(json!({ "object": brief(&s, g) }))
}

// ---------------------------------------------------------------- delete, duplicate, copy, paste

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObjectsParams {
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
}

async fn scene_delete(ed: Arc<Editor>, p: ObjectsParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let before = svc.store.snapshot().objects.len();
    select_named(&svc, p.objects.as_deref(), o).await?;
    if svc.store.snapshot().selection.is_empty() {
        return Err(CmdError::invalid("nothing is selected to delete"));
    }
    svc.store.send(SceneCmd::RemoveSelection { label: "Delete".into() }, o).await?;
    let after = svc.store.snapshot().objects.len();
    Ok(json!({ "removed": before.saturating_sub(after) }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DuplicateParams {
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Metres the copies are moved by (left out: they stand where the objects stand).
    #[serde(default)]
    offset: Option<[f32; 3]>,
}

fn added_reply(svc: &SceneService, added: &edit::Added) -> Value {
    let s = svc.store.snapshot();
    let objects: Vec<Value> =
        s.selected_in_order().iter().filter_map(|id| s.get(*id)).map(|x| brief(&s, x)).collect();
    json!({ "added": added.objects, "objects": objects, "warnings": added.warnings })
}

async fn scene_duplicate(ed: Arc<Editor>, p: DuplicateParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let ids = edit::ids_of(&svc.store.snapshot(), p.objects.as_deref())?;
    let offset = p.offset.map(|v| finite(v, "offset")).transpose()?.unwrap_or(Vec3::ZERO);
    let added = edit::duplicate(&svc, ids, offset, o).await?;
    Ok(added_reply(&svc, &added))
}

async fn scene_copy(ed: Arc<Editor>, p: ObjectsParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let ids = edit::ids_of(&svc.store.snapshot(), p.objects.as_deref())?;
    let n = edit::copy(&ed, &svc, ids).await?;
    Ok(json!({ "copied": n, "file": wire(&svc.root, &edit::clipboard_file(&svc)) }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    /// Metres the pasted objects are moved by from where they were copied.
    #[serde(default)]
    offset: Option<[f32; 3]>,
}

async fn scene_paste(ed: Arc<Editor>, p: PasteParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let offset = p.offset.map(|v| finite(v, "offset")).transpose()?.unwrap_or(Vec3::ZERO);
    let added = edit::paste(&ed, &svc, offset, o).await?;
    Ok(added_reply(&svc, &added))
}

// ---------------------------------------------------------------- groups

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum GroupAction {
    /// The objects become one group, where they stand.
    #[default]
    Group,
    /// The objects (a group with its members) are written as a `.group` asset (`path`).
    Save,
    /// Each group's pivot moves to the middle of its objects.
    CenterPivot,
    /// The groups' pivot moves to their object `object`.
    PivotTo,
    /// The groups are made again from the `.group` they name.
    Reload,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupParams {
    #[serde(default)]
    action: GroupAction,
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// The new group's name (action=group).
    #[serde(default)]
    name: Option<String>,
    /// The `.group` file (action=save): project-relative or absolute.
    #[serde(default)]
    path: Option<String>,
    /// The member the pivot moves to (action=pivot_to).
    #[serde(default)]
    object: Option<String>,
}

fn groups_of(s: &SceneState, ids: &[ObjId]) -> Vec<Arc<Object>> {
    ids.iter().filter_map(|id| s.get(*id)).filter(|o| o.class == ObjClass::Group).cloned().collect()
}

async fn scene_group(ed: Arc<Editor>, p: GroupParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    match p.action {
        GroupAction::Group => {
            let ids = select_named(&svc, p.objects.as_deref(), o).await?;
            if ids.is_empty() {
                return Err(CmdError::invalid("nothing is selected to group"));
            }
            let s = svc.store.snapshot();
            let name = name_for(&s, p.name.as_deref(), "group")?;
            let reply = svc.store.send(SceneCmd::Group { name: name.as_str().into() }, o).await?;
            let SceneReply::Id(id) = reply else { return Err(CmdError::invalid("nothing could be grouped")) };
            let s = svc.store.snapshot();
            let g = s.get(id).ok_or_else(|| CmdError::internal("the group is gone"))?;
            let members = match &g.data {
                ObjData::Group(d) => d.members.len(),
                _ => 0,
            };
            Ok(json!({ "object": brief(&s, g), "members": members }))
        }
        GroupAction::Save => {
            let path = p
                .path
                .as_deref()
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .ok_or_else(|| CmdError::invalid("action=save takes path: the .group file"))?;
            let s = svc.store.snapshot();
            let ids = edit::ids_of(&s, p.objects.as_deref())?;
            if ids.is_empty() {
                return Err(CmdError::invalid("nothing is selected to save"));
            }
            let raw = PathBuf::from(path.replace('/', "\\"));
            let file = if raw.is_absolute() { raw } else { svc.root.join(raw) };
            let file = if file.extension().is_none() { file.with_extension(group::EXT) } else { file };
            // positions about the first object (a group: its pivot)
            let pivot = s.get(ids[0]).map_or(Vec3::ZERO, |x| x.position());
            let n = edit::save_group(&ed, &svc, ids.clone(), pivot, file.clone()).await?;
            // a group saved whole names its asset from now on
            let groups = groups_of(&s, &ids);
            if let ([g], Some(rel)) = (groups.as_slice(), RelPath::from_fs(&svc.root, &file)) {
                let mut n2 = (**g).clone();
                if let ObjData::Group(d) = &mut n2.data {
                    d.reference = SmolStr::new(rel.as_str());
                }
                svc.store
                    .send(
                        SceneCmd::ReplaceObjects {
                            objects: vec![n2],
                            serial: s.serial,
                            label: "Save Group".into(),
                        },
                        o,
                    )
                    .await?;
            }
            Ok(json!({ "file": wire(&svc.root, &file), "objects": n }))
        }
        GroupAction::CenterPivot | GroupAction::PivotTo => {
            let s = svc.store.snapshot();
            let ids = edit::ids_of(&s, p.objects.as_deref())?;
            let target = p.object.as_deref().map(str::trim);
            if p.action == GroupAction::PivotTo && target.is_none() {
                return Err(CmdError::invalid("action=pivot_to takes object: a member of the group"));
            }
            let mut objects = Vec::new();
            for g in groups_of(&s, &ids) {
                let ObjData::Group(gd) = &g.data else { continue };
                let members: Vec<_> = gd.members.iter().filter_map(|m| s.get(*m)).collect();
                if members.is_empty() {
                    continue;
                }
                let mut n = (*g).clone();
                if p.action == GroupAction::CenterPivot {
                    let sum = members.iter().fold(Vec3::ZERO, |acc, m| acc + m.position());
                    n.xform.position = sum / members.len() as f32;
                    n.xform.rotation = Vec3::ZERO;
                } else if let Some(m) =
                    members.iter().find(|m| target.is_some_and(|t| m.name.eq_ignore_ascii_case(t)))
                {
                    n.xform.position = m.position();
                    n.xform.rotation = m.rotation();
                } else {
                    continue;
                }
                objects.push(n);
            }
            if objects.is_empty() {
                return Err(CmdError::invalid("no group is selected (or none holds that object)"));
            }
            let changed = objects.len();
            svc.store
                .send(SceneCmd::ReplaceObjects { objects, serial: s.serial, label: "Group Pivot".into() }, o)
                .await?;
            Ok(json!({ "changed": changed }))
        }
        GroupAction::Reload => {
            let s = svc.store.snapshot();
            let ids = edit::ids_of(&s, p.objects.as_deref())?;
            let groups = groups_of(&s, &ids);
            if groups.is_empty() {
                return Err(CmdError::invalid("no group is selected"));
            }
            let mut loaded = Vec::new();
            for g in &groups {
                let ObjData::Group(gd) = &g.data else { continue };
                if gd.reference.is_empty() {
                    return Err(CmdError::invalid(format!("{} names no .group to reload from", g.name)));
                }
                let l = edit::group_asset(&ed, &svc, &gd.reference).await?;
                if l.members.len() != gd.members.len() && !gd.members.is_empty() {
                    ed.log.warn(format!(
                        "{}: the .group holds another number of objects; the members were made again",
                        g.name
                    ));
                }
                loaded.push((g.id, Box::new(l)));
            }
            let n = loaded.len();
            svc.store.send(SceneCmd::ReloadGroups { groups: loaded }, o).await?;
            Ok(json!({ "reloaded": n }))
        }
    }
}

async fn scene_ungroup(ed: Arc<Editor>, p: ObjectsParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let ids = select_named(&svc, p.objects.as_deref(), o).await?;
    let n = groups_of(&svc.store.snapshot(), &ids).len();
    if n == 0 {
        return Err(CmdError::invalid("no group is selected"));
    }
    svc.store.send(SceneCmd::Ungroup, o).await?;
    Ok(json!({ "ungrouped": n }))
}

// ---------------------------------------------------------------- align, query

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AlignAction {
    /// Down onto what is below each object (under its whole box, or its pivot with line=true).
    Drop,
    /// Down onto the surface and turned to stand on its slope.
    Surface,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AlignParams {
    action: AlignAction,
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Drop along the line under each pivot instead of under the whole box.
    #[serde(default)]
    line: bool,
    /// Metres above where each object lands (0).
    #[serde(default)]
    offset: Option<f32>,
}

async fn scene_align(ed: Arc<Editor>, p: AlignParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let ids = edit::ids_of(&svc.store.snapshot(), p.objects.as_deref())?;
    if ids.is_empty() {
        return Err(CmdError::invalid("nothing is selected to drop"));
    }
    let offset = p.offset.unwrap_or(0.0);
    if !offset.is_finite() {
        return Err(CmdError::invalid("offset takes a finite number of metres"));
    }
    let landed = drop_ids(&ed, &svc, &ids, p.action == AlignAction::Surface, p.line, offset, o).await?;
    let s = svc.store.snapshot();
    let objects: Vec<Value> = ids
        .iter()
        .filter_map(|id| s.get(*id))
        .map(|x| {
            let mut b = brief(&s, x);
            if let Value::Object(m) = &mut b {
                m.insert("landed".into(), json!(landed.contains(&x.id)));
            }
            b
        })
        .collect();
    Ok(json!({ "requested": ids.len(), "landed": landed.len(), "objects": objects }))
}

async fn scene_query(ed: Arc<Editor>, p: QueryParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    svc.prefetch_library(&ed, &snap).await;
    // a model the scene does not name yet (fit, free) is loaded too
    for r in sense::extra_refs(&p).into_iter().filter(|r| !r.is_empty()) {
        let lib = svc.library.clone();
        let _ = ed.exec.io(move || lib.load(&r)).await;
    }
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    match p.action {
        QueryAction::Walls => {
            let at = finite(
                p.position.ok_or_else(|| CmdError::invalid("action=walls takes position: its height"))?,
                "position",
            )?;
            let [x0, z0, x1, z1] =
                p.area.ok_or_else(|| CmdError::invalid("action=walls takes area: [x0, z0, x1, z1]"))?;
            if x1 <= x0 || z1 <= z0 || (x1 - x0) > 80.0 || (z1 - z0) > 80.0 {
                return Err(CmdError::invalid("the area must be non-empty and at most 80 x 80 m"));
            }
            let step = p.step.unwrap_or(0.5).max(0.1);
            let reach = p.reach.unwrap_or(6.0).max(0.5);
            let res = p.cell.unwrap_or(0.1).max(0.02);
            let rays = p.rays.unwrap_or(24).max(4);
            let y = at.y;
            let w = ((x1 - x0) / res).ceil() as u32;
            let h = ((z1 - z0) / res).ceil() as u32;
            let mut origins = Vec::new();
            let mut ox = x0;
            while ox <= x1 {
                let mut oz = z0;
                while oz <= z1 {
                    origins.push(Vec3::new(ox, y, oz));
                    oz += step;
                }
                ox += step;
            }
            let cast = origins.len() * rays;
            let mut cells = ed
                .exec
                .cpu(move || {
                    use rayon::prelude::*;
                    origins
                        .par_iter()
                        .flat_map_iter(|o| {
                            let (snap, pick, lib) = (&snap, &pick, &lib);
                            (0..rays).filter_map(move |i| {
                                let ang = std::f32::consts::TAU * i as f32 / rays as f32;
                                let dir = Vec3::new(ang.sin(), 0.0, ang.cos());
                                let hit = ray_scene_objects(pick, snap, lib, *o, dir, reach, None)?;
                                let q = hit.point;
                                if !q.is_finite() || q.x < x0 || q.x >= x1 || q.z < z0 || q.z >= z1 {
                                    return None;
                                }
                                let ix = (((q.x - x0) / res) as u32).min(w - 1);
                                let iz = (((q.z - z0) / res) as u32).min(h - 1);
                                Some(iz * w + ix)
                            })
                        })
                        .collect::<Vec<u32>>()
                })
                .await;
            cells.sort_unstable();
            cells.dedup();
            Ok(
                json!({ "area": [r4(x0), r4(z0), r4(x1), r4(z1)], "cell": r4(res), "width": w, "height": h, "rays": cast, "cells": cells }),
            )
        }
        _ => {
            ed.exec
                .cpu(move || sense::query(&sense::Scene { state: &snap, lib: &lib, pick: &pick }, &p))
                .await
        }
    }
}

/// The names of the objects the game's own level has, lower case: the names its base scene
/// writes - an X-Ray SDK scene named as the scene's level, in the project or in the editor's
/// library. `None` when there is none or its parts are not text.
fn base_names(roots: &[std::path::PathBuf], level: &str) -> Option<rustc_hash::FxHashSet<String>> {
    if level.is_empty() {
        return None;
    }
    let dir = roots
        .iter()
        .map(|r| r.join("rawdata").join("levels"))
        .find(|d| d.join(format!("{level}.level")).is_file())?
        .join(level);
    let mut names = rustc_hash::FxHashSet::default();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("part")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        // a part of the binary form names nothing a line at a time: the set would be wrong
        if bytes.iter().take(4096).any(|b| *b == 0) {
            return None;
        }
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key.trim() == "name" {
                names.insert(value.trim().to_lowercase());
            }
        }
    }
    (!names.is_empty()).then_some(names)
}

/// `scene.audit`: the checks on the pools, then each finding at its object's line.
async fn scene_audit(ed: Arc<Editor>, p: sense::audit::AuditParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    svc.prefetch_library(&ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    let limit = p.limit.unwrap_or(200).clamp(1, 2000);
    let root = ed.project_root();
    let mut roots: Vec<std::path::PathBuf> = root.iter().cloned().collect();
    roots.push(gz_project::library_root(&ed));
    let level = snap.meta.level.level_path.to_string();
    let base = ed.exec.io(move || base_names(&roots, &level)).await;
    let state = snap.clone();
    let found = ed
        .exec
        .cpu(move || {
            sense::audit::audit(&sense::Scene { state: &state, lib: &lib, pick: &pick }, &p, base.as_ref())
        })
        .await?;
    // where each object is written: the file of its kind in the scene's folder
    let scene_file = snap.meta.file.clone();
    let listed: Vec<(sense::audit::Finding, String, ObjClass)> = found
        .findings
        .iter()
        .map(|f| {
            let o = snap.get(f.object);
            (
                f.clone(),
                o.map_or_else(String::new, |o| o.name.to_string()),
                o.map_or(ObjClass::SceneObject, |o| o.class),
            )
        })
        .collect();
    let total = listed.len();
    let problems = ed
        .exec
        .io(move || {
            let mut docs: rustc_hash::FxHashMap<&'static str, Option<gz_luadata::LuaDoc>> =
                Default::default();
            let folder = scene_file.as_deref().map(crate::format::folder_of);
            listed
                .into_iter()
                .take(limit)
                .map(|(f, name, class)| {
                    let mut problem = gz_base::Problem::warning(f.code, f.text).about(name.clone());
                    if let (Some(folder), Some(root), Some((file, list))) =
                        (&folder, &root, crate::format::file_of(class))
                    {
                        let doc = docs.entry(file).or_insert_with(|| {
                            std::fs::read_to_string(folder.join(file))
                                .ok()
                                .and_then(|text| gz_luadata::LuaDoc::parse(&text).ok())
                        });
                        let path = [gz_luadata::Seg::key(list), gz_luadata::Seg::item("name", &name)];
                        let rel = folder
                            .join(file)
                            .strip_prefix(root)
                            .ok()
                            .and_then(|r| gz_base::RelPath::new(&r.to_string_lossy()).ok());
                        if let (Some(doc), Some(rel)) = (doc.as_ref(), rel)
                            && let Some(s) = doc.span(&path)
                        {
                            problem = problem.in_file(rel).at(s.line, s.column, s.end_line, s.end_column);
                        }
                    }
                    let mut v = serde_json::to_value(&problem).unwrap_or(Value::Null);
                    if let Value::Object(m) = &mut v {
                        m.insert("object".into(), json!(name));
                    }
                    v
                })
                .collect::<Vec<Value>>()
        })
        .await;
    let mut out = json!({ "problems": problems, "total": total, "checked": found.checked });
    if let (false, Value::Object(m)) = (found.skipped.is_empty(), &mut out) {
        m.insert("skipped".into(), json!(found.skipped));
    }
    Ok(out)
}

/// `shot.map`: one ray a pixel from above, then the drawing - all of it on the pools.
async fn shot_map(ed: Arc<Editor>, p: sense::shot::MapParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    svc.prefetch_library(&ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    ed.exec
        .cpu(move || sense::shot::shot_map(&sense::Scene { state: &snap, lib: &lib, pick: &pick }, &p))
        .await
}

// ---------------------------------------------------------------- attach

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AttachParams {
    /// The spawn point. Left out: the selected one.
    #[serde(default)]
    spawn: Option<String>,
    /// The shape that becomes the spawn point's own (it takes the shape's place).
    #[serde(default)]
    object: Option<String>,
    /// Give the spawn point's shape back to the scene instead (every selected point without spawn).
    #[serde(default)]
    detach: bool,
    /// The viewport's next click attaches the shape it hits (Attach on Click).
    #[serde(default)]
    on_click: Option<bool>,
}

async fn scene_attach(ed: Arc<Editor>, p: AttachParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    if let Some(on) = p.on_click {
        let st = crate::objtools::set_state(&svc, o, |st| st.spawn_attach = on).await?;
        return Ok(json!({ "on_click": st.spawn_attach }));
    }
    let s = svc.store.snapshot();
    let spawn_named = |n: &str| {
        s.find_any(n)
            .filter(|x| x.class == ObjClass::SpawnPoint)
            .cloned()
            .ok_or_else(|| CmdError::not_found(format!("no spawn point is named {n}")))
    };
    if p.detach {
        let spawns: Vec<Arc<Object>> = match p.spawn.as_deref() {
            Some(n) => vec![spawn_named(n)?],
            None => s
                .selected_in_order()
                .iter()
                .filter_map(|id| s.get(*id))
                .filter(|x| x.class == ObjClass::SpawnPoint)
                .cloned()
                .collect(),
        };
        let with: Vec<ObjId> = spawns
            .iter()
            .filter(|x| matches!(&x.data, ObjData::Spawn(d) if d.attached.is_some()))
            .map(|x| x.id)
            .collect();
        for id in &with {
            svc.store.send(SceneCmd::SpawnAttach { spawn: *id, object: None }, o).await?;
        }
        return Ok(json!({ "detached": with.len() }));
    }
    let object =
        p.object.as_deref().ok_or_else(|| CmdError::invalid("scene.attach takes object (or detach=true)"))?;
    let spawn = match p.spawn.as_deref() {
        Some(n) => spawn_named(n)?,
        None => {
            let sel: Vec<_> = s
                .selected_in_order()
                .iter()
                .filter_map(|id| s.get(*id))
                .filter(|x| x.class == ObjClass::SpawnPoint)
                .cloned()
                .collect();
            match sel.as_slice() {
                [one] => one.clone(),
                _ => return Err(CmdError::invalid("select one spawn point, or name it in spawn")),
            }
        }
    };
    let shape = s
        .find_any(object)
        .cloned()
        .ok_or_else(|| CmdError::not_found(format!("no object is named {object}")))?;
    svc.store.send(SceneCmd::SpawnAttach { spawn: spawn.id, object: Some(shape.id) }, o).await?;
    Ok(json!({ "attached": shape.name.as_str(), "spawn": spawn.name.as_str() }))
}

// ---------------------------------------------------------------- motion

/// A motion playing: which object, from which time, since when. The viewport poses the object
/// from it on its own frames; Pause makes the pose a scene change.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    pub id: ObjId,
    pub from: f32,
    pub started: std::time::Instant,
}

/// The session's playback.
#[derive(Default)]
pub struct MotionPlayback {
    pub playing: parking_lot::Mutex<Option<Play>>,
}

impl MotionPlayback {
    #[must_use]
    pub fn of(session: &gz_editor::ProjectSession) -> Arc<MotionPlayback> {
        session.services.get_or_init(MotionPlayback::default)
    }

    /// The time of a playing motion now: past the end it wraps to the start.
    #[must_use]
    pub fn time_of(play: &Play, min_t: f32, max_t: f32) -> f32 {
        let mut t = play.from + play.started.elapsed().as_secs_f32();
        if t > max_t {
            let len = max_t - min_t;
            if len > 0.0 {
                let k = ((t - min_t) / len).floor();
                t -= k * len;
            } else {
                t = max_t;
            }
        }
        t
    }
}

/// Sends a motion edit of one object as its scene command.
pub async fn send_motion(
    svc: &SceneService,
    o: &Object,
    edit: &crate::objmotion::MotionEdit,
    origin: Origin,
) -> Result<(), CmdError> {
    let e = crate::objmotion::apply(o, edit).map_err(CmdError::invalid)?;
    svc.store
        .send(
            SceneCmd::Motion {
                id: o.id,
                motion: e.motion,
                flags: e.flags,
                pose: e.pose,
                label: e.label.to_owned(),
                merge: e.merge.then(|| SmolStr::new_static("motion_frame")),
            },
            origin,
        )
        .await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MotionAction {
    /// What the motion is.
    #[default]
    State,
    /// The object moves along a motion, or not (`on`).
    Motionable,
    /// Reads a motion from an `.anm` (`path`, relative to the project's gamedata\anims).
    Import,
    /// Writes the motion as an `.anm` (`path`).
    Export,
    AddKey,
    DeleteKey,
    /// Scales the keys between `from` and `to` by `scale`.
    ScaleKeys,
    /// Spaces the keys between `from` and `to` for a `speed` in metres a second.
    NormalizeKeys,
    /// Cuts the range to the keys.
    Clamp,
    First,
    PreviousKey,
    PreviousFrame,
    Play,
    Pause,
    NextFrame,
    NextKey,
    Last,
    /// A move of the object makes a key (`on`).
    AutoKey,
    /// The viewport looks through the object (`on`).
    CameraView,
    /// The range: `min`, `max` in seconds.
    Range,
    /// The time the object stands at (`time`).
    Current,
    /// Moves the key at the current time to `time`.
    KeyTime,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MotionParams {
    /// The object.
    object: String,
    /// What to do with the object's motion: read it (`state`), change its keys or its range, move
    /// the time, play it.
    #[serde(default)]
    action: MotionAction,
    /// The `.anm` (import, export).
    #[serde(default)]
    path: Option<String>,
    /// motionable, auto_key, camera_view (default true).
    #[serde(default)]
    on: Option<bool>,
    /// Seconds (scale_keys, normalize_keys).
    #[serde(default)]
    from: Option<f32>,
    /// Seconds: the end of the span `from` starts (scale_keys, normalize_keys).
    #[serde(default)]
    to: Option<f32>,
    /// The factor the keys' times are multiplied by (scale_keys).
    #[serde(default)]
    scale: Option<f32>,
    /// Metres a second (normalize_keys).
    #[serde(default)]
    speed: Option<f32>,
    /// Seconds (current, key_time).
    #[serde(default)]
    time: Option<f32>,
    /// Seconds (range).
    #[serde(default)]
    min: Option<f32>,
    /// Seconds: the end of the range `min` starts (range).
    #[serde(default)]
    max: Option<f32>,
}

fn need(v: Option<f32>, what: &str) -> Result<f32, CmdError> {
    v.ok_or_else(|| CmdError::invalid(format!("this action takes {what}")))
}

async fn scene_motion(ed: Arc<Editor>, p: MotionParams, o: Origin) -> Reply {
    use crate::objmotion::MotionEdit as E;
    let svc = service::need_scene(&ed).await?;
    let session = ed.project()?;
    let snap = svc.store.snapshot();
    let obj = snap
        .find_any(&p.object)
        .ok_or_else(|| CmdError::not_found(format!("no object is named {}", p.object)))?
        .clone();
    let playback = MotionPlayback::of(&session);
    let anims = svc.root.join("gamedata").join("anims");
    let file_of = |path: Option<&str>| -> Result<PathBuf, CmdError> {
        let raw = path.ok_or_else(|| CmdError::invalid("this action takes path: the .anm"))?;
        let f = PathBuf::from(raw.replace('/', "\\"));
        let f = if f.is_absolute() { f } else { anims.join(f) };
        Ok(if f.extension().is_none() { f.with_extension("anm") } else { f })
    };
    let on = p.on.unwrap_or(true);
    let model = |o: &Object| {
        let raw = o.motion.as_ref().ok_or_else(|| CmdError::invalid(format!("{} has no motion", o.name)))?;
        let m = raw
            .model()
            .ok_or_else(|| CmdError::invalid("the motion is of a version this editor does not read"))?;
        Ok::<_, CmdError>((crate::objmotion::params(&m, raw), m))
    };
    let edit = match p.action {
        MotionAction::State => None,
        MotionAction::Motionable => Some(E::Motionable(on)),
        MotionAction::Import => {
            let file = file_of(p.path.as_deref())?;
            let shown = file.display().to_string();
            let bytes =
                ed.exec.io(move || std::fs::read(&file)).await.map_err(|e| {
                    CmdError::not_found(format!("the motion could not be read: {shown} ({e})"))
                })?;
            Some(E::Import(Box::new(
                gz_fmt_object::read_anm(&bytes).map_err(|e| CmdError::invalid(e.to_string()))?,
            )))
        }
        MotionAction::Export => {
            let file = file_of(p.path.as_deref())?;
            let (_, m) = model(&obj)?;
            let bytes = gz_fmt_object::write_anm(&m);
            edit::writable(&ed, &file)?;
            let f2 = file.clone();
            ed.exec
                .io(move || {
                    if let Some(d) = f2.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    gz_editor::fsx::write_atomic(&f2, &bytes)
                })
                .await
                .map_err(|e| {
                    CmdError::io(format!("the motion could not be written: {} ({e})", file.display()))
                })?;
            None
        }
        MotionAction::AddKey => Some(E::AddKey),
        MotionAction::DeleteKey => Some(E::DeleteKey),
        MotionAction::ScaleKeys => Some(E::ScaleKeys {
            from: need(p.from, "from")?,
            to: need(p.to, "to")?,
            scale: need(p.scale, "scale")?,
        }),
        MotionAction::NormalizeKeys => Some(E::NormalizeKeys {
            from: need(p.from, "from")?,
            to: need(p.to, "to")?,
            speed: need(p.speed, "speed")?,
        }),
        MotionAction::Clamp => Some(E::Clamp),
        MotionAction::First => Some(E::FirstFrame),
        MotionAction::PreviousKey => Some(E::PreviousKey),
        MotionAction::PreviousFrame => Some(E::PreviousFrame),
        MotionAction::NextFrame => Some(E::NextFrame),
        MotionAction::NextKey => Some(E::NextKey),
        MotionAction::Last => Some(E::LastFrame),
        MotionAction::AutoKey => Some(E::AutoKey(on)),
        MotionAction::CameraView => Some(E::CameraView(on)),
        MotionAction::Range => Some(E::Range { min_t: p.min, max_t: p.max }),
        MotionAction::Current => Some(E::Current(need(p.time, "time")?)),
        MotionAction::KeyTime => Some(E::KeyTime(need(p.time, "time")?)),
        MotionAction::Play => {
            let (prm, _) = model(&obj)?;
            *playback.playing.lock() =
                Some(Play { id: obj.id, from: prm.t_current, started: std::time::Instant::now() });
            ed.exec.wake().ping();
            None
        }
        MotionAction::Pause => {
            let play = playback.playing.lock().take();
            match play {
                Some(play) if play.id == obj.id => {
                    let (prm, _) = model(&obj)?;
                    Some(E::At(MotionPlayback::time_of(&play, prm.min_t, prm.max_t)))
                }
                _ => None,
            }
        }
    };
    if let Some(edit) = edit {
        // a control stops the playback
        if !matches!(edit, E::AutoKey(_) | E::CameraView(_)) {
            let mut pl = playback.playing.lock();
            if pl.is_some_and(|x| x.id == obj.id) {
                *pl = None;
            }
        }
        send_motion(&svc, &obj, &edit, o).await?;
    }
    let snap = svc.store.snapshot();
    let o2 = snap.get(obj.id).ok_or_else(|| CmdError::not_found("the object is gone"))?;
    let playing = playback.playing.lock().is_some_and(|x| x.id == o2.id);
    let state = match o2.motion.as_ref().and_then(|r| r.model().map(|m| (m, r.clone()))) {
        Some((m, raw)) if o2.co_flags() & crate::model::co::MOTION != 0 => {
            let prm = crate::objmotion::params(&m, &raw);
            json!({
                "on": true,
                "keys": m.envelopes[0].keys.len(),
                "length": r4(crate::objmotion::length(&m).0),
                "start": r4(prm.min_t),
                "end": r4(prm.max_t),
                "current": r4(prm.t_current),
                "auto_key": o2.co_flags() & crate::objmotion::AUTO_KEY != 0,
                "camera_view": o2.co_flags() & crate::objmotion::CAMERA_VIEW != 0,
                "playing": playing,
            })
        }
        _ => json!({ "on": false }),
    };
    Ok(json!({ "object": o2.name.as_str(), "motion": state, "position": v3(o2.position()) }))
}

// ---------------------------------------------------------------- the summary

/// The summary of the last collect: the Scene > Summary Info sheet reads it.
#[derive(Default)]
pub struct SummaryStore {
    pub summary: parking_lot::Mutex<Option<Arc<crate::summary::Summary>>>,
}

impl SummaryStore {
    #[must_use]
    pub fn of(session: &gz_editor::ProjectSession) -> Arc<SummaryStore> {
        session.services.get_or_init(SummaryStore::default)
    }
}

/// Where a texture's description and its animation are: the project's and the editor's
/// `rawdata\textures\<name>.thm`, `gamedata\textures\<name>.seq`.
fn texture_info(roots: &[PathBuf], name: &str) -> Option<(gz_fmt_tex::TextureParams, u32)> {
    let rel = name.replace('/', "\\");
    let thm = roots
        .iter()
        .find_map(|r| std::fs::read(r.join("rawdata\\textures").join(format!("{rel}.thm"))).ok())?;
    let params = gz_fmt_tex::read_texture_thm(&thm).ok()?.params;
    // the `.seq` holds a line per frame after its first
    let frames = roots
        .iter()
        .find_map(|r| std::fs::read(r.join("gamedata\\textures").join(format!("{rel}.seq"))).ok())
        .map_or(1, |b| String::from_utf8_lossy(&b).lines().count().saturating_sub(1).max(1) as u32);
    Some((params, frames))
}

/// A summary's rows as an answer gives them.
pub(crate) fn summary_rows(s: &crate::summary::Summary) -> Value {
    Value::Array(s.rows().into_iter().map(|(k, v)| json!({ "key": k, "value": v })).collect())
}

/// Collects the Summary Info and keeps it for the sheet.
pub(crate) async fn collect_summary(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
) -> Result<Arc<crate::summary::Summary>, CmdError> {
    let session = ed.project()?;
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let lib = svc.library.clone();
    let roots = vec![svc.root.clone(), svc.editor_root.clone()];
    let (summary, log) = ed
        .exec
        .io(move || {
            let mut s = crate::summary::collect(&snap, &lib);
            let log = s.prepare(&lib, &|n| texture_info(&roots, n));
            (s, log)
        })
        .await;
    for l in &log {
        ed.log.finding(l);
    }
    let s = Arc::new(summary);
    *SummaryStore::of(&session).summary.lock() = Some(s.clone());
    Ok(s)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SummaryParams {
    /// The text file to write (project-relative or absolute).
    path: String,
    /// Replace a file that is there already.
    #[serde(default)]
    overwrite: bool,
}

async fn scene_summary(ed: Arc<Editor>, p: SummaryParams, _o: Origin) -> Reply {
    let svc = crate::service::need_scene(&ed).await?;
    let path = p.path.trim();
    if path.is_empty() {
        return Err(CmdError::invalid("path names the text file to write"));
    }
    let raw = PathBuf::from(path.replace('/', "\\"));
    let file = if raw.is_absolute() { raw } else { svc.root.join(raw) };
    edit::writable(&ed, &file)?;
    let there = file.clone();
    if !p.overwrite && ed.exec.io(move || there.exists()).await {
        return Err(CmdError::new(
            "exists",
            format!("{} is there already (overwrite=true replaces it)", crate::cmd::wire(&svc.root, &file)),
        ));
    }
    let summary = collect_summary(&ed, &svc).await?;
    let text = summary.export();
    let to = file.clone();
    ed.exec
        .io(move || {
            if let Some(d) = to.parent() {
                std::fs::create_dir_all(d)?;
            }
            gz_editor::fsx::write_atomic(&to, text.as_bytes())
        })
        .await
        .map_err(|e| CmdError::io(format!("the summary could not be written: {e}")))?;
    Ok(json!({ "file": crate::cmd::wire(&svc.root, &file), "summary": summary_rows(&summary) }))
}

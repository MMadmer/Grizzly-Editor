//! The data of the sixteen kinds of objects, and their boxes. Their keys in the scene's files are
//! `crate::format::fields`; the X-Ray SDK importer reads them in `crate::sdk::objects`.

mod shape;
mod simple;
mod spawn;
mod way;

use gz_base::{Blob, SmolStr};

pub use shape::{
    CF_BOX, CF_SPHERE, FV_EMITTER, FV_OCCLUSION, FogData, SHAPE_COMMON, SHAPE_LEVEL_BOUND, ShapeData,
    ShapeDef, SoundEnvData,
};
pub use simple::{
    Fuzzy, GlowData, KeptObject, LT_DIRECT, LT_POINT, LT_SPOT, LightData, ParticlesData, SO_USE_SURFACE,
    SO_VISUAL_MODE, SceneObjectData, SoundSrcData, SurfaceOverride, light_flags,
};
pub use spawn::{
    ENVMOD_CHOOSE_NAME, EnvMod, ORDER_HOSTILE, ORDER_RUN, PLAYERSTART_CHOOSE_NAME, PointKind,
    RPOINT_CHOOSE_NAME, RPoint, SpawnData, SpawnEntity, XmsOrder, env_flags,
};
pub use way::{
    PortalData, SectorData, WAYPOINT_RADIUS, WAYPOINT_SIZE, WayData, WayPoint, fcolor_from, fcolor_get,
};

use crate::class::ObjClass;
use crate::math::{Aabb, Vec3, point_box, xform_box};
use crate::model::{ObjData, ObjId, Object, SceneState};
pub use crate::sdk::objects::Loaded;

/// A group's own data.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GroupData {
    /// The `.group` asset (or the library group) it was placed from.
    pub reference: SmolStr,
    /// The objects it holds, in order. They stay in the scene's lists.
    pub members: Vec<ObjId>,
}

/// A new object's data of a class (no spawn entity: that takes the game's configs).
#[must_use]
pub fn default_data(class: ObjClass) -> ObjData {
    match class {
        ObjClass::SceneObject => ObjData::SceneObject(SceneObjectData::default()),
        ObjClass::Light => ObjData::Light(LightData::default()),
        ObjClass::Glow => ObjData::Glow(GlowData::default()),
        ObjClass::Shape => ObjData::Shape(ShapeData::default()),
        ObjClass::SoundSrc => ObjData::SoundSrc(SoundSrcData::default()),
        ObjClass::SoundEnv => ObjData::SoundEnv(SoundEnvData::default()),
        ObjClass::SpawnPoint => ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::PlayerStart))),
        ObjClass::Way => ObjData::Way(WayData::default()),
        ObjClass::Sector => ObjData::Sector(SectorData::default()),
        ObjClass::Portal => ObjData::Portal(PortalData::default()),
        ObjClass::Group => ObjData::Group(GroupData::default()),
        ObjClass::Particles => ObjData::Particles(ParticlesData::default()),
        ObjClass::FogVolume => ObjData::FogVolume(FogData::default()),
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => {
            ObjData::Kept(KeptObject::Stream { why: "not an object class".into(), body: Blob::from(&[][..]) })
        }
    }
}

/// The sizes of the points the editor draws: a respawn point, an environment modifier, the
/// Player Start, a particle effect, a thing without a size.
pub const RPOINT_SIZE: f32 = 0.5;
pub const ENVMOD_SIZE: f32 = 0.25;
pub const PLAYERSTART_HALF_WIDTH: f32 = 0.4;
pub const PLAYERSTART_HEIGHT: f32 = 1.8;
pub const PSOBJECT_SIZE: f32 = 0.5;
pub const VIS_RADIUS: f32 = 0.25;

fn rpoint_box(p: Vec3) -> Aabb {
    Aabb::new(
        Vec3::new(p.x - RPOINT_SIZE, p.y, p.z - RPOINT_SIZE),
        Vec3::new(p.x + RPOINT_SIZE, p.y + RPOINT_SIZE * 2.0, p.z + RPOINT_SIZE),
    )
}

/// The world box. `local_ref` gives a library object's local box by reference name.
#[must_use]
pub fn world_box(state: &SceneState, obj: &Object, local_ref: &dyn Fn(&str) -> Option<Aabb>) -> Option<Aabb> {
    let p = obj.position();
    match &obj.data {
        ObjData::SceneObject(d) => local_ref(&d.reference).map(|b| xform_box(&b, &obj.world())),
        ObjData::Light(d) => Some(point_box(p, d.range)),
        ObjData::Glow(d) => Some(point_box(p, d.radius)),
        ObjData::Shape(d) => d.world_box(&obj.xform),
        ObjData::SoundEnv(d) => d.shape.world_box(&obj.xform),
        ObjData::FogVolume(d) => d.shape.world_box(&obj.xform),
        ObjData::SoundSrc(d) => Some(point_box(p, d.max_distance)),
        ObjData::Spawn(d) => {
            let mut b = match d.kind {
                PointKind::RPoint => rpoint_box(p),
                PointKind::EnvMod => {
                    point_box(p, if state.is_selected(obj.id) { d.env.radius } else { ENVMOD_SIZE })
                }
                PointKind::PlayerStart => Aabb::new(
                    Vec3::new(p.x - PLAYERSTART_HALF_WIDTH, p.y, p.z - PLAYERSTART_HALF_WIDTH),
                    Vec3::new(
                        p.x + PLAYERSTART_HALF_WIDTH,
                        p.y + PLAYERSTART_HEIGHT,
                        p.z + PLAYERSTART_HALF_WIDTH,
                    ),
                ),
                PointKind::Spawn => match d.attached.as_ref().and_then(|a| attached_box(a)) {
                    Some(sb) => sb,
                    None => rpoint_box(p),
                },
            };
            if let Some(a) = &d.attached
                && let Some(ab) = attached_world_box(a)
            {
                b.merge(&ab);
            }
            Some(b)
        }
        ObjData::Way(d) => Some(d.bounds()),
        ObjData::Portal(d) => d.bounds(),
        ObjData::Sector(d) => {
            let mut b = Aabb::INVALID;
            for (o, _) in &d.items {
                if let Some(item) = state.find(ObjClass::SceneObject, o)
                    && let Some(ib) = world_box(state, item, local_ref)
                {
                    b.merge(&ib);
                }
            }
            b.is_valid().then_some(b)
        }
        ObjData::Group(g) => {
            let mut b = Aabb::INVALID;
            for id in &g.members {
                let Some(m) = state.get(*id) else { continue };
                match m.class {
                    ObjClass::SpawnPoint | ObjClass::SceneObject | ObjClass::Way => {
                        if let Some(mb) = world_box(state, m, local_ref) {
                            b.merge(&mb);
                        }
                    }
                    _ => b.modify(m.position()),
                }
            }
            if !b.is_valid() {
                b = point_box(p, 0.5);
            }
            Some(b)
        }
        ObjData::Particles(_) => Some(point_box(p, PSOBJECT_SIZE)),
        ObjData::Kept(_) => Some(point_box(p, VIS_RADIUS)),
    }
}

/// A spawn's box from its shape: the shapes' extents carried by the shape's transform.
fn attached_box(a: &Object) -> Option<Aabb> {
    let ObjData::Shape(s) = &a.data else { return None };
    if s.shapes.is_empty() {
        return None;
    }
    let m = a.xform.matrix();
    let mut b = Aabb::INVALID;
    for sh in &s.shapes {
        if sh.kind == CF_SPHERE {
            b.modify(m.transform_tiny(sh.center() + Vec3::splat(sh.radius())));
            b.modify(m.transform_tiny(sh.center() - Vec3::splat(sh.radius())));
        } else {
            let bm = sh.matrix();
            b.modify(m.transform_tiny(bm.transform_tiny(Vec3::splat(0.5))));
            b.modify(m.transform_tiny(bm.transform_tiny(Vec3::splat(-0.5))));
        }
    }
    Some(b)
}

fn attached_world_box(a: &Object) -> Option<Aabb> {
    match &a.data {
        ObjData::Shape(s) => s.world_box(&a.xform),
        _ => None,
    }
}

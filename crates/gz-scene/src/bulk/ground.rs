//! The ground the AI map, the detail objects and the wallmarks lie on: every static scene object
//! the collision build takes, narrowed only by a volume or by a list of names the author gave.
//!
//! An imported SDK scene carries a "Snap Objects" list per tool that its author filled by hand;
//! such a list is kept as the tool's explicit list when it does not hold every static object
//! anyway ([`effective`]), and is otherwise the same as no list at all.

use gz_base::SmolStr;

use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objtools::geom::is_static_any;

/// The shape usage that makes a shape the AI map's bounds volume: generation happens inside it.
pub const SHAPE_AI_MAP_BOUNDS: u8 = 2;

/// The section of the scene's reference (`docs/assets/SCENE.md`) on what the painted layers lie on.
pub const REFERENCE: &str = "## The ground of the painted layers\n\n\
The AI map, the detail objects and the wallmarks lie on **the ground**: every static scene object the collision \
build takes (not in visual mode, its library object not dynamic, no sound occluder, no HOM). A layer narrows it \
only by:\n\n\
- an explicit list of object names - the AI map's and the details' own list, kept in their layer files (a list \
that names every static object is the same as none);\n\
- for the AI map, the **AI Map Bounds** volume: a shape of usage `ai_map_bounds` (2, beside `common` 0 and \
`level_bound` 1). With one or more such shapes the map is generated inside their box, without one over the whole \
ground.\n\n\
The AI map's generation parameters are the layer's own (`scene.ai_map action=set`). The build regenerates an AI \
map it generated itself when the ground, the bounds or the parameters moved, and keeps a map the author edited \
(`build.state` in the level folder says which). The detail objects are the game's 2 m slots, painted per mesh \
(`scene.details`); the layer keeps its meshes with their density, scale range and waving.\n\n";

/// Whether an object is ground: a scene object, not in visual mode, whose library object is
/// static (not dynamic, no sound occluder, no HOM) - what the collision build takes.
#[must_use]
pub fn is_ground(o: &Object, lib: &Library) -> bool {
    let ObjData::SceneObject(d) = &o.data else { return false };
    if d.visual_mode() {
        return false;
    }
    lib.cached(&d.reference).is_some_and(|lo| is_static_any(&lo))
}

/// The explicit list a tool keeps: `None` when it names every static object (or none) - the
/// ground is then the whole level.
#[must_use]
pub fn effective(state: &SceneState, lib: &Library, names: &[SmolStr]) -> Option<Vec<SmolStr>> {
    if names.is_empty() {
        return None;
    }
    let listed: rustc_hash::FxHashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let all = state
        .of_class(ObjClass::SceneObject)
        .filter(|o| is_ground(o, lib))
        .all(|o| listed.contains(&o.name.to_lowercase()));
    (!all).then(|| names.to_vec())
}

/// The world box of every AI map bounds volume merged; `None` without one.
#[must_use]
pub fn ai_bounds(state: &SceneState) -> Option<Aabb> {
    let mut b = Aabb::INVALID;
    let mut any = false;
    for o in state.of_class(ObjClass::Shape) {
        if let ObjData::Shape(s) = &o.data
            && s.usage == SHAPE_AI_MAP_BOUNDS
            && let Some(wb) = s.world_box(&o.xform)
        {
            b.merge(&wb);
            any = true;
        }
    }
    any.then_some(b)
}

/// The ground objects, in scene order: every static scene object, or those `names` lists, and of
/// those only the ones whose box meets `within`.
#[must_use]
pub fn objects(
    state: &SceneState,
    lib: &Library,
    names: Option<&[SmolStr]>,
    within: Option<&Aabb>,
) -> Vec<ObjId> {
    let listed: Option<rustc_hash::FxHashSet<String>> =
        names.map(|n| n.iter().map(|x| x.to_lowercase()).collect());
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    state
        .of_class(ObjClass::SceneObject)
        .filter(|o| listed.as_ref().is_none_or(|l| l.contains(&o.name.to_lowercase())))
        .filter(|o| is_ground(o, lib))
        .filter(|o| {
            within.is_none_or(|v| {
                crate::objects::world_box(state, o, &local)
                    .is_some_and(|b| b.min.cmple(v.max).all() && b.max.cmpge(v.min).all())
            })
        })
        .map(|o| o.id)
        .collect()
}

/// The names of the ground objects.
#[must_use]
pub fn names(state: &SceneState, ids: &[ObjId]) -> Vec<SmolStr> {
    ids.iter().filter_map(|id| state.get(*id)).map(|o| o.name.clone()).collect()
}

/// The box of the ground, cut to `within` when there is one.
#[must_use]
pub fn ground_box(state: &SceneState, lib: &Library, ids: &[ObjId], within: Option<&Aabb>) -> Option<Aabb> {
    let b = super::details::objects_box(state, lib, ids)?;
    match within {
        Some(v) => {
            let cut = Aabb::new(b.min.max(v.min), b.max.min(v.max));
            cut.is_valid().then_some(cut)
        }
        None => Some(b),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::objects::{GlowData, SceneObjectData, ShapeData};
    use std::path::PathBuf;

    fn library() -> Library {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("rawdata").join("objects");
        let lib = Library::new(vec![root]);
        lib.load("editor/pcube").unwrap();
        lib
    }

    fn cube(s: &mut SceneState, name: &str, x: f32) -> ObjId {
        let mut o = Object::new(
            ObjClass::SceneObject,
            name,
            ObjData::SceneObject(SceneObjectData::library("editor/pcube")),
        );
        o.xform.position.x = x;
        s.insert(o)
    }

    #[test]
    fn the_ground_is_every_static_object_narrowed_by_a_list_or_a_volume() {
        let lib = library();
        let mut s = SceneState::default();
        let a = cube(&mut s, "a", 0.0);
        let b = cube(&mut s, "b", 50.0);
        // something that is no ground at all
        s.insert(Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default())));
        let missing = s.insert(Object::new(
            ObjClass::SceneObject,
            "nowhere",
            ObjData::SceneObject(SceneObjectData::library("editor/not_there")),
        ));
        assert_eq!(objects(&s, &lib, None, None), vec![a, b]);
        assert!(!objects(&s, &lib, None, None).contains(&missing));
        // a list that names every static object is the same as none
        let all = [SmolStr::new("a"), SmolStr::new("b")];
        assert!(effective(&s, &lib, &all).is_none());
        let some = [SmolStr::new("B")];
        assert_eq!(effective(&s, &lib, &some).as_deref(), Some(&some[..]));
        assert_eq!(objects(&s, &lib, Some(&some), None), vec![b]);
        // a volume round b
        let v = Aabb::new(glam::Vec3::new(40.0, -5.0, -5.0), glam::Vec3::new(60.0, 5.0, 5.0));
        assert_eq!(objects(&s, &lib, None, Some(&v)), vec![b]);
        // the AI map's bounds volume is a shape of that role
        let mut shape = Object::new(
            ObjClass::Shape,
            "bounds",
            ObjData::Shape(ShapeData { usage: SHAPE_AI_MAP_BOUNDS, ..ShapeData::unit_box() }),
        );
        shape.xform.position.x = 50.0;
        s.insert(shape);
        let bb = ai_bounds(&s).unwrap();
        assert!(bb.min.x > 40.0 && bb.max.x < 60.0);
    }
}

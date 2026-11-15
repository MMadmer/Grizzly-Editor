//! What the scene answers about its geometry, in numbers: `scene.query`'s actions beside
//! `walls` (the ground under points, a height grid, rays, what stands in a box, whether a model
//! fits somewhere, where it fits, the AI map), and the boxes `scene.objects` and `scene.get`
//! carry. Everything here is a pure function of a snapshot, the library and its pick tree: the
//! command loads what the scene names first, builds the tree and runs it on the CPU pool.

mod ai;
pub mod audit;
mod fit;
mod ground;
pub mod map;
mod rays;
pub mod shot;
#[cfg(test)]
mod tests;

use gz_editor::{CmdError, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::class::ObjClass;
use crate::cmd::{r4, v3};
use crate::library::Library;
use crate::math::{Aabb, Vec3};
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects;
use crate::pick::{PickHit, RayFilter, RayKinds, ScenePick};

/// The most points one `ground` or `ai` call takes.
pub const MAX_POINTS: usize = 1024;
/// The most samples a side of a `heights` grid has.
pub const MAX_SIDE: usize = 257;
/// The widest search of `free`, metres.
pub const MAX_RADIUS: f32 = 50.0;
/// The most spots `free` answers.
pub const MAX_SPOTS: usize = 16;
/// The most AI map nodes the walk of `ai from= to=` visits before it gives up.
pub const MAX_WALK: usize = 1_000_000;

/// What `scene.query` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryAction {
    /// The floor below and the ceiling above `position` (or each of `positions`), with the
    /// floor's normal, slope, object and surface.
    Ground,
    /// The walls of `area` at the height `position` names: the grid cells rays hit.
    Walls,
    /// The height of the ground over `area`, every `step` metres.
    Heights,
    /// One ray (`from` to `to`, or `from` along `direction` for `reach`): what it hits first, or
    /// with `all` everything it crosses.
    Ray,
    /// The objects whose box (with `exact`: whose triangles) meet a box.
    Box,
    /// Whether a model (`ref`, `size` or `object`) fits at `position` without meeting anything,
    /// and how it would stand on the ground there.
    Fit,
    /// Places near `position` where a model fits, standing on the ground, nearest first.
    Free,
    /// Where points are on the AI map, or whether `to` can be walked to from `from`.
    Ai,
}

/// The parameters of `scene.query`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryParams {
    /// What to ask: ground, walls, heights, ray, box, fit, free or ai.
    pub action: QueryAction,
    /// Metres: the point of ground, walls (its height), fit (where the model's pivot goes; left
    /// out with object: where the object stands), free (near where), box (the box's centre,
    /// with size).
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// ground, ai: several points at once (at most 1024).
    #[serde(default)]
    pub positions: Option<Vec<[f32; 3]>>,
    /// ground: start above everything in the scene instead of at the point (the highest floor
    /// of the column).
    #[serde(default)]
    pub top: bool,
    /// How far the rays reach, metres (ground: 30, walls: 6, ray along direction: 1000).
    #[serde(default)]
    pub reach: Option<f32>,
    /// walls, heights: the area, `[x0, z0, x1, z1]` in metres (walls: at most 80 x 80).
    #[serde(default)]
    pub area: Option<[f32; 4]>,
    /// walls: metres between the points rays start from (0.5); heights: metres between samples
    /// (1; at most 257 x 257 samples).
    #[serde(default)]
    pub step: Option<f32>,
    /// walls: the size of a grid cell, metres (0.1).
    #[serde(default)]
    pub cell: Option<f32>,
    /// walls: rays around each point (24).
    #[serde(default)]
    pub rays: Option<usize>,
    /// ground, heights, ray, box, fit, free: only these objects count (names; a group stands for
    /// its objects).
    #[serde(default)]
    pub only: Option<Vec<String>>,
    /// ground, heights, ray, box, fit, free: these objects do not count (names; a group stands
    /// for its objects).
    #[serde(default)]
    pub except: Option<Vec<String>>,
    /// ray, ai: where it starts, metres.
    #[serde(default)]
    pub from: Option<[f32; 3]>,
    /// ray: where it ends; ai: where the walk goes, metres.
    #[serde(default)]
    pub to: Option<[f32; 3]>,
    /// ray: the direction instead of `to` (any length).
    #[serde(default)]
    pub direction: Option<[f32; 3]>,
    /// ray: every object it crosses (`hits`), not only the first.
    #[serde(default)]
    pub all: bool,
    /// box: its lowest corner, metres.
    #[serde(default)]
    pub min: Option<[f32; 3]>,
    /// box: its highest corner, metres.
    #[serde(default)]
    pub max: Option<[f32; 3]>,
    /// box: its size about `position`; fit, free: a box this big (metres; its pivot at the
    /// middle of its bottom) instead of a model.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// box (with position and size), fit: degrees.
    #[serde(default)]
    pub rotation: Option<[f32; 3]>,
    /// box: test the objects' triangles, not only their boxes.
    #[serde(default)]
    pub exact: bool,
    /// fit, free: the model, as the library spells it.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// fit: a scene object: its model at its place (position, rotation, scale may move it), the
    /// object itself not counting.
    #[serde(default)]
    pub object: Option<String>,
    /// fit: a factor per axis.
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
    /// free: how far from position spots may be, metres (10; at most 50).
    #[serde(default)]
    pub radius: Option<f32>,
    /// free: how many spots (4; at most 16).
    #[serde(default)]
    pub count: Option<usize>,
    /// free: metres at least between two spots (the model's width).
    #[serde(default)]
    pub spacing: Option<f32>,
    /// free: the steepest ground a spot may stand on, degrees (30).
    #[serde(default)]
    pub slope: Option<f32>,
    /// box, ray with all: at most this many in the answer (100).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A snapshot with what answers about it.
#[derive(Clone, Copy)]
pub struct Scene<'a> {
    pub state: &'a SceneState,
    pub lib: &'a Library,
    pub pick: &'a ScenePick,
}

impl Scene<'_> {
    /// An object's world box, its library object as far as it is loaded.
    #[must_use]
    pub fn world_box(&self, o: &Object) -> Option<Aabb> {
        world_bounds(self.state, self.lib, o)
    }

    /// The heights a ray starts from to be above everything and ends at to be below it.
    fn top(&self) -> Option<(f32, f32)> {
        let b = self.pick.bounds();
        b.is_valid().then_some((b.max.y + 1.0, b.min.y - 1.0))
    }
}

/// Answers every action but `walls` (the caller's).
pub fn query(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    match p.action {
        QueryAction::Ground => ground::ground(sc, p),
        QueryAction::Heights => ground::heights(sc, p),
        QueryAction::Ray => rays::ray(sc, p),
        QueryAction::Box => rays::boxed(sc, p),
        QueryAction::Fit => fit::fit(sc, p),
        QueryAction::Free => fit::free(sc, p),
        QueryAction::Ai => ai::ai(sc, p),
        QueryAction::Walls => Err(CmdError::internal("walls is answered by scene.query itself")),
    }
}

/// The library references a query needs beside the scene's own: the model of fit and free.
#[must_use]
pub fn extra_refs(p: &QueryParams) -> Vec<String> {
    match p.action {
        QueryAction::Fit | QueryAction::Free => p.r#ref.iter().map(|r| r.trim().to_owned()).collect(),
        _ => Vec::new(),
    }
}

/// An object's world box, its library object as far as it is loaded.
#[must_use]
pub fn world_bounds(state: &SceneState, lib: &Library, o: &Object) -> Option<Aabb> {
    objects::world_box(state, o, &|r: &str| lib.cached(r).map(|x| x.bounds)).filter(Aabb::is_valid)
}

/// A box as answers write it: `{min, max}`.
#[must_use]
pub fn bounds_json(b: &Aabb) -> Value {
    json!({ "min": v3(b.min), "max": v3(b.max) })
}

/// A box's size as answers write it.
#[must_use]
pub fn size_json(b: &Aabb) -> Value {
    v3(b.max - b.min)
}

pub(crate) fn finite(v: [f32; 3], what: &str) -> Result<Vec3, CmdError> {
    let v = Vec3::from_array(v);
    if v.is_finite() { Ok(v) } else { Err(CmdError::invalid(format!("{what} takes finite numbers"))) }
}

pub(crate) fn radians(d: [f32; 3]) -> Vec3 {
    use crate::format::values::radians as r;
    Vec3::new(r(f64::from(d[0])), r(f64::from(d[1])), r(f64::from(d[2])))
}

/// The objects a list of names means; a group stands for itself and its objects.
pub(crate) fn ids_named(
    state: &SceneState,
    names: Option<&[String]>,
) -> Result<Option<Vec<ObjId>>, CmdError> {
    let Some(names) = names else { return Ok(None) };
    let mut out = Vec::new();
    for n in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        let o = state.find_any(n).ok_or_else(|| CmdError::not_found(format!("no object is named {n}")))?;
        out.push(o.id);
        if let ObjData::Group(g) = &o.data {
            out.extend(g.members.iter().copied());
        }
    }
    Ok(Some(out))
}

/// The `only` and `except` lists of a request.
pub(crate) struct Lists {
    pub only: Option<Vec<ObjId>>,
    pub except: Vec<ObjId>,
}

impl Lists {
    pub fn of(state: &SceneState, p: &QueryParams) -> Result<Self, CmdError> {
        Ok(Self {
            only: ids_named(state, p.only.as_deref())?,
            except: ids_named(state, p.except.as_deref())?.unwrap_or_default(),
        })
    }

    pub fn filter(&self, kinds: RayKinds) -> RayFilter<'_> {
        RayFilter { kinds, only: self.only.as_deref(), except: &self.except }
    }
}

/// A hit's normal turned to face where the ray came from.
pub(crate) fn facing(n: Vec3, dir: Vec3) -> Vec3 {
    if n.dot(dir) > 0.0 { -n } else { n }
}

/// Degrees between a normal and straight up.
pub(crate) fn slope_of(n: Vec3) -> f32 {
    n.y.abs().clamp(0.0, 1.0).acos().to_degrees()
}

/// The surface of a hit as answers write it: `{name, texture, material}`, `null` without one.
pub(crate) fn surface_json(sc: &Scene<'_>, hit: &PickHit) -> Value {
    let Some(s) = hit.surface else { return Value::Null };
    let Some(o) = sc.state.get(hit.id) else { return Value::Null };
    let Some(lo) = crate::objects::terrain::mesh_of(sc.state, sc.lib, o) else { return Value::Null };
    lo.object.surfaces.get(s as usize).map_or(Value::Null, |s| {
        json!({ "name": s.name.as_str(), "texture": s.texture.as_str(), "material": s.game_mtl.as_str() })
    })
}

/// An object's name as answers write it.
pub(crate) fn name_of(sc: &Scene<'_>, id: ObjId) -> Value {
    sc.state.get(id).map_or(Value::Null, |o| json!(o.name.as_str()))
}

/// The kind of an object as answers write it.
pub(crate) fn kind_of(sc: &Scene<'_>, id: ObjId) -> Value {
    sc.state.get(id).map_or(Value::Null, |o| json!(o.class.key()))
}

pub(crate) fn opt_r4(v: Option<f32>) -> Value {
    v.map_or(Value::Null, |v| json!(r4(v)))
}

/// Whether a kind is a solid thing the ground queries stand on: a scene object or a terrain.
#[must_use]
pub fn is_solid(o: &Object) -> bool {
    matches!(o.class, ObjClass::SceneObject | ObjClass::Terrain)
}

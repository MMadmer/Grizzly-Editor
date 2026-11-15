//! `shot.map`: the rays and the scene's data behind the drawing of [`super::map`].

use base64::Engine;
use gz_editor::CmdError;
use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::map::{self, MapData, MapLine, MapMark, MapMode, MapObject, MapPoint, PointKind, Sample};
use super::{Scene, ground, ids_named};
use crate::cmd::r4;
use crate::math::Vec3;
use crate::model::{ObjData, ObjId};
use crate::pick::{RayFilter, RayKinds};

/// The longest side of a map's ground, metres: past it a pixel says nothing.
pub const MAX_METRES: f32 = 8192.0;
/// The marks of one map at most.
pub const MAX_MARKS: usize = 64;

/// What a map shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModeWord {
    /// Every object in a colour of its own over the shaded ground.
    #[default]
    Objects,
    /// The heights as colours, with contour lines and the height written where the grid's
    /// lines cross.
    Height,
    /// The AI map's nodes over the ground.
    Ai,
}

/// A point the caller wants marked.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkParam {
    /// Where, in metres; the height is not used.
    pub position: [f32; 3],
    /// What is written beside it.
    #[serde(default)]
    pub text: String,
}

/// A map's size on the ground: one number, or one for x and one for z.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MapSize {
    Square(f32),
    Sides(Vec<f32>),
}

/// `shot.map`'s parameters.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapParams {
    /// The ground rectangle the map shows: `[x0, z0, x1, z1]` in metres. Left out with
    /// `position` left out too: the whole scene.
    #[serde(default)]
    pub area: Option<[f32; 4]>,
    /// Instead of `area`: the middle of the map, in metres (the height is not used) ...
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// ... and its size along x and z, in metres: one number for a square, two for x and z.
    #[serde(default)]
    pub size: Option<MapSize>,
    /// The longer side of the map in px (1024; 64 to 2048). The picture is 46 px wider and 14 px
    /// higher: the coordinates stand in its left and top margins.
    #[serde(default)]
    pub px: Option<u32>,
    /// What the map shows: objects (the default), height or ai.
    #[serde(default)]
    pub mode: ModeWord,
    /// Metres between the grid's lines. Left out: 0.5 to 500 by the scale, 56 px apart or more.
    #[serde(default)]
    pub grid: Option<f32>,
    /// Whether the objects' names are written on the map (true; false in the height mode, where
    /// the heights are written). The answer's legend names every object the map shows either way.
    #[serde(default)]
    pub labels: Option<bool>,
    /// Objects drawn in the accent colour and named before any other.
    #[serde(default)]
    pub highlight: Option<Vec<String>>,
    /// Points to mark with a cross and a caption, at most 64: a place seen before anything is
    /// put there.
    #[serde(default)]
    pub marks: Option<Vec<MarkParam>>,
    /// Whether spawn points, lights, sounds, ways and volumes are drawn as signs. Left out: drawn
    /// at 2 px a metre and more, left out on a smaller map, where they would hide it.
    #[serde(default)]
    pub signs: Option<bool>,
    /// The height the rays start from, in metres, instead of above everything: what is under a
    /// roof or under the trees' crowns.
    #[serde(default)]
    pub below: Option<f32>,
    /// The rows of the legend at most (100), the largest objects first and the highlighted ones
    /// before them; `legend_total` counts them all.
    #[serde(default)]
    pub legend: Option<usize>,
    /// Only these objects are seen from above.
    #[serde(default)]
    pub only: Option<Vec<String>>,
    /// These objects are not seen from above.
    #[serde(default)]
    pub except: Option<Vec<String>>,
}

/// The ground rectangle a request names: normalised, checked.
fn area_of(sc: &Scene<'_>, p: &MapParams) -> Result<[f32; 4], CmdError> {
    let area = match (p.area, p.position) {
        (Some(_), Some(_)) => {
            return Err(CmdError::invalid("area and position both place the map: give one"));
        }
        (Some(a), None) => {
            if p.size.is_some() {
                return Err(CmdError::invalid("size goes with position; area has its own size"));
            }
            [a[0].min(a[2]), a[1].min(a[3]), a[0].max(a[2]), a[1].max(a[3])]
        }
        (None, Some(at)) => {
            let size = p.size.as_ref().ok_or_else(|| CmdError::invalid("position takes size with it"))?;
            let (sx, sz) = match size {
                MapSize::Square(s) => (*s, *s),
                MapSize::Sides(v) => match v.as_slice() {
                    [s] => (*s, *s),
                    [x, z] => (*x, *z),
                    _ => return Err(CmdError::invalid("size takes one number (a square) or two (x and z)")),
                },
            };
            [at[0] - sx * 0.5, at[2] - sz * 0.5, at[0] + sx * 0.5, at[2] + sz * 0.5]
        }
        (None, None) => {
            if p.size.is_some() {
                return Err(CmdError::invalid("size goes with position"));
            }
            let b = sc.pick.bounds();
            if !b.is_valid() {
                return Err(CmdError::invalid("the scene has nothing to show: give area or position"));
            }
            [b.min.x - 1.0, b.min.z - 1.0, b.max.x + 1.0, b.max.z + 1.0]
        }
    };
    if !area.iter().all(|v| v.is_finite()) {
        return Err(CmdError::invalid("the map's rectangle takes numbers"));
    }
    let (dx, dz) = (area[2] - area[0], area[3] - area[1]);
    if dx < 0.5 || dz < 0.5 {
        return Err(CmdError::invalid("the map is at least half a metre a side"));
    }
    if dx > MAX_METRES || dz > MAX_METRES {
        return Err(CmdError::invalid(format!(
            "the map is at most {MAX_METRES} m a side, not {:.0} x {:.0}: give a smaller area",
            dx, dz
        )));
    }
    Ok(area)
}

/// The map's size in px for a rectangle and the longer side asked for.
#[must_use]
pub fn size_px(area: [f32; 4], px: u32) -> [u32; 2] {
    let (dx, dz) = (area[2] - area[0], area[3] - area[1]);
    if dx >= dz {
        [px, ((px as f32 * dz / dx).round() as u32).max(16)]
    } else {
        [((px as f32 * dx / dz).round() as u32).max(16), px]
    }
}

/// Draws the map of a request and answers it. Blocking: call it on the CPU pool.
pub fn shot_map(sc: &Scene<'_>, p: &MapParams) -> Result<Value, CmdError> {
    let area = area_of(sc, p)?;
    let px = p.px.unwrap_or(1024);
    if !(64..=map::MAX_PX).contains(&px) {
        return Err(CmdError::invalid(format!("px takes 64 to {}", map::MAX_PX)));
    }
    if p.grid.is_some_and(|g| !g.is_finite() || g <= 0.0) {
        return Err(CmdError::invalid("grid takes metres above zero"));
    }
    let marks = p.marks.as_deref().unwrap_or_default();
    if marks.len() > MAX_MARKS {
        return Err(CmdError::invalid(format!(
            "marks takes at most {MAX_MARKS} points, not {}",
            marks.len()
        )));
    }
    let only = ids_named(sc.state, p.only.as_deref())?;
    let except = ids_named(sc.state, p.except.as_deref())?.unwrap_or_default();
    let highlight = ids_named(sc.state, p.highlight.as_deref())?.unwrap_or_default();
    let filter = RayFilter { kinds: RayKinds::Meshes, only: only.as_deref(), except: &except };
    let size = size_px(area, px);
    let k = size[0] as f32 / (area[2] - area[0]);

    // one ray a pixel, from above everything - or from the height the caller names
    let (top, lo) = sc.top().unwrap_or((1.0, -1.0));
    if p.below.is_some_and(|b| !b.is_finite()) {
        return Err(CmdError::invalid("below takes a height in metres"));
    }
    let hi = p.below.unwrap_or(top);
    let reach = (hi - lo).max(0.1);
    let hits: Vec<Option<(ObjId, f32, Vec3)>> = {
        use rayon::prelude::*;
        (0..size[1])
            .into_par_iter()
            .flat_map_iter(|j| {
                let z = area[3] - (j as f32 + 0.5) / k;
                let filter = &filter;
                (0..size[0]).map(move |i| {
                    let x = area[0] + (i as f32 + 0.5) / k;
                    ground::down(sc, Vec3::new(x, hi, z), reach, filter).map(|h| {
                        let n = h.normal.unwrap_or(Vec3::Y);
                        (h.id, h.point.y, if n.y < 0.0 { -n } else { n })
                    })
                })
            })
            .collect()
    };

    // the objects the rays met, in the order the picture meets them
    let mut index: FxHashMap<ObjId, u32> = FxHashMap::default();
    let mut objects: Vec<MapObject> = Vec::new();
    let samples: Vec<Sample> = hits
        .iter()
        .map(|hit| {
            let Some((id, y, n)) = hit else { return Sample::NONE };
            let i = *index.entry(*id).or_insert_with(|| {
                let o = sc.state.get(*id);
                objects.push(MapObject {
                    name: o.map_or_else(String::new, |o| o.name.to_string()),
                    kind: o.map_or("object", |o| o.class.key()),
                    highlight: highlight.contains(id),
                });
                objects.len() as u32 - 1
            });
            Sample::hit(i, *y, [n.x, n.y, n.z])
        })
        .collect();

    // what has no body: spawn points, lights, sounds, particles, ways, volumes
    let inside = |x: f32, z: f32| x >= area[0] && x <= area[2] && z >= area[1] && z <= area[3];
    let mut points: Vec<MapPoint> = Vec::new();
    let mut lines: Vec<MapLine> = Vec::new();
    let shown = |id: ObjId| only.as_ref().is_none_or(|l| l.contains(&id)) && !except.contains(&id);
    // on a small map the signs would be all there is to see: what is highlighted is drawn anyway
    let signs = p.signs.unwrap_or(k >= 2.0);
    for o in sc
        .state
        .objects
        .values()
        .filter(|o| o.flags.visible && shown(o.id) && (signs || highlight.contains(&o.id)))
    {
        let at = o.position();
        let lit = highlight.contains(&o.id);
        let mut point = |kind: PointKind| {
            if inside(at.x, at.z) {
                points.push(MapPoint {
                    name: o.name.to_string(),
                    kind,
                    at: [at.x, at.z],
                    highlight: lit,
                    named: true,
                });
            }
        };
        match &o.data {
            ObjData::Spawn(_) => point(PointKind::Spawn),
            ObjData::Light(_) => point(PointKind::Light),
            ObjData::SoundSrc(_) => point(PointKind::Sound),
            ObjData::Particles(_) => point(PointKind::Particles),
            ObjData::Way(w) => {
                for (n, wp) in w.points.iter().enumerate() {
                    let a = wp.position;
                    if inside(a.x, a.z) {
                        // the way is named once, at its first point
                        points.push(MapPoint {
                            name: o.name.to_string(),
                            kind: PointKind::WayPoint,
                            at: [a.x, a.z],
                            highlight: lit,
                            named: n == 0,
                        });
                    }
                    for (to, _) in &wp.links {
                        if let Some(b) = w.points.get(*to as usize).map(|t| t.position) {
                            lines.push(MapLine {
                                points: vec![[a.x, a.z], [b.x, b.z]],
                                closed: false,
                                way: true,
                            });
                        }
                    }
                }
            }
            ObjData::Shape(_) | ObjData::SoundEnv(_) | ObjData::FogVolume(_) => {
                if let Some(b) = sc.world_box(o) {
                    lines.push(MapLine {
                        points: vec![
                            [b.min.x, b.min.z],
                            [b.max.x, b.min.z],
                            [b.max.x, b.max.z],
                            [b.min.x, b.max.z],
                        ],
                        closed: true,
                        way: false,
                    });
                }
            }
            _ => {}
        }
    }
    // names in one order whatever the store's: the picture is the same for the same scene
    points.sort_by(|a, b| {
        a.name.cmp(&b.name).then(a.at[0].total_cmp(&b.at[0])).then(a.at[1].total_cmp(&b.at[1]))
    });

    let mode = match p.mode {
        ModeWord::Objects => MapMode::Objects,
        ModeWord::Height => MapMode::Height,
        ModeWord::Ai => MapMode::Ai,
    };
    let (nodes, node_size) = match (&sc.state.bulk.ai_map, mode) {
        (Some(m), MapMode::Ai) => (
            m.all_nodes().iter().filter(|n| inside(n.pos.x, n.pos.z)).map(|n| [n.pos.x, n.pos.z]).collect(),
            m.header.params.patch_size,
        ),
        _ => (Vec::new(), 0.7),
    };
    let ai_nodes = nodes.len();
    let data = MapData {
        area,
        size,
        samples,
        objects,
        points,
        lines,
        marks: marks
            .iter()
            .map(|m| MapMark { at: [m.position[0], m.position[2]], text: m.text.clone() })
            .collect(),
        nodes,
        node_size,
        mode,
        grid: p.grid,
        labels: p.labels.unwrap_or(mode != MapMode::Height),
    };
    let drawn = map::draw(&data);
    let png =
        drawn.canvas.png().map_err(|e| CmdError::internal(format!("the map could not be encoded: {e}")))?;
    // the legend is drawn largest first, the highlighted before them: its head is what matters
    let legend_total = drawn.legend.len();
    let legend: Vec<Value> = drawn
        .legend
        .iter()
        .take(p.legend.unwrap_or(100))
        .map(|r| {
            json!({
                "object": r.object, "kind": r.kind, "colour": r.colour,
                "centre": [r4(r.centre[0]), r4(r.centre[1])], "area": r4(r.area), "named": r.named,
            })
        })
        .collect();
    let mut out = json!({
        "png_base64": base64::engine::general_purpose::STANDARD.encode(png),
        "width": drawn.canvas.width(),
        "height": drawn.canvas.height(),
        "area": [r4(area[0]), r4(area[1]), r4(area[2]), r4(area[3])],
        "px_per_m": r4(drawn.px_per_m),
        // where the map itself lies in the picture: a pixel reads back to metres as
        // x = area[0] + (px - left) / px_per_m, z = area[3] - (py - top) / px_per_m
        "map": { "left": map::MARGIN_LEFT, "top": map::MARGIN_TOP, "width": size[0], "height": size[1] },
        "grid": r4(drawn.grid),
        "mode": match mode { MapMode::Objects => "objects", MapMode::Height => "height", MapMode::Ai => "ai" },
        "legend": legend,
        "legend_total": legend_total,
        "signs": signs,
    });
    if let Value::Object(m) = &mut out {
        if let Some((lo, hi)) = drawn.heights {
            let mut h = json!({ "min": r4(lo), "max": r4(hi) });
            if let (Some(c), Value::Object(hm)) = (drawn.contour, &mut h) {
                hm.insert("contour".into(), json!(r4(c)));
            }
            m.insert("heights".into(), h);
        }
        if mode == MapMode::Ai {
            m.insert("ai_nodes".into(), json!(ai_nodes));
            if sc.state.bulk.ai_map.is_none() {
                m.insert("note".into(), json!("the scene has no AI map"));
            }
        }
    }
    Ok(out)
}

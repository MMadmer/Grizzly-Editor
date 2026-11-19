//! `ground` and `heights`: what is below a point, and a grid of heights.

use gz_editor::CmdError;
use serde_json::{Value, json};

use super::{
    Lists, MAX_POINTS, MAX_SIDE, QueryParams, Scene, facing, finite, name_of, opt_r4, slope_of, surface_json,
};
use crate::cmd::{r4, v3};
use crate::math::Vec3;
use crate::pick::{PickHit, RayFilter, RayKinds, ray_scene};

/// The first solid thing straight down from `start` within `reach`.
pub(crate) fn down(sc: &Scene<'_>, start: Vec3, reach: f32, f: &RayFilter<'_>) -> Option<PickHit> {
    ray_scene(sc.pick, sc.state, sc.lib, start, Vec3::NEG_Y, reach, f).filter(|h| h.point.is_finite())
}

fn one_point(
    sc: &Scene<'_>,
    at: Vec3,
    top: bool,
    reach: f32,
    f: &RayFilter<'_>,
) -> serde_json::Map<String, Value> {
    let (start, down_reach) = match (top, sc.top()) {
        (true, Some((hi, lo))) => (Vec3::new(at.x, hi, at.z), hi - lo + reach),
        _ => (at, reach),
    };
    let floor = down(sc, start, down_reach, f);
    let ceiling = ray_scene(sc.pick, sc.state, sc.lib, at, Vec3::Y, reach, f).filter(|h| h.point.is_finite());
    let mut m = serde_json::Map::new();
    m.insert("floor".into(), opt_r4(floor.map(|h| h.point.y)));
    m.insert("ceiling".into(), opt_r4(ceiling.map(|h| h.point.y)));
    let normal = floor.and_then(|h| h.normal).map(|n| facing(n, Vec3::NEG_Y));
    m.insert("normal".into(), normal.map_or(Value::Null, v3));
    m.insert("slope".into(), opt_r4(normal.map(slope_of)));
    m.insert("object".into(), floor.map_or(Value::Null, |h| name_of(sc, h.id)));
    m.insert("surface".into(), floor.map_or(Value::Null, |h| surface_json(sc, &h)));
    m
}

pub(super) fn ground(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let lists = Lists::of(sc.state, p)?;
    // the ground is what stops a body: the collision of a level, the faces of a model that collide
    let f = lists.filter(RayKinds::Solid);
    let reach = p.reach.unwrap_or(30.0).max(0.1);
    match (&p.position, &p.positions) {
        (Some(_), Some(_)) => Err(CmdError::invalid("ground takes position or positions, not both")),
        (Some(at), None) => Ok(Value::Object(one_point(sc, finite(*at, "position")?, p.top, reach, &f))),
        (None, Some(list)) => {
            if list.len() > MAX_POINTS {
                return Err(CmdError::invalid(format!(
                    "ground takes at most {MAX_POINTS} positions, not {}",
                    list.len()
                )));
            }
            let pts = list.iter().map(|v| finite(*v, "positions")).collect::<Result<Vec<_>, _>>()?;
            use rayon::prelude::*;
            let points: Vec<Value> = pts
                .par_iter()
                .map(|at| {
                    let mut m = serde_json::Map::new();
                    m.insert("position".into(), v3(*at));
                    m.extend(one_point(sc, *at, p.top, reach, &f));
                    Value::Object(m)
                })
                .collect();
            Ok(json!({ "points": points }))
        }
        (None, None) => Err(CmdError::invalid("ground takes position, or positions: [[x,y,z], ...]")),
    }
}

pub(super) fn heights(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let [x0, z0, x1, z1] = p.area.ok_or_else(|| CmdError::invalid("heights takes area: [x0, z0, x1, z1]"))?;
    if ![x0, z0, x1, z1].iter().all(|v| v.is_finite()) || x1 <= x0 || z1 <= z0 {
        return Err(CmdError::invalid("the area must be finite and not empty: x1 > x0, z1 > z0"));
    }
    let step = p.step.unwrap_or(1.0);
    if !step.is_finite() || step <= 0.0 {
        return Err(CmdError::invalid("step takes a number of metres above 0"));
    }
    let count = |len: f32| ((len / step) + 1e-4).floor() as usize + 1;
    let (columns, rows) = (count(x1 - x0), count(z1 - z0));
    if columns > MAX_SIDE || rows > MAX_SIDE {
        let need = (x1 - x0).max(z1 - z0) / (MAX_SIDE - 1) as f32;
        return Err(CmdError::invalid(format!(
            "heights takes at most {MAX_SIDE} x {MAX_SIDE} samples; this area at a step of {} m is {columns} x {rows} (a step of at least {} m fits)",
            r4(step),
            r4(need)
        )));
    }
    let lists = Lists::of(sc.state, p)?;
    let f = lists.filter(RayKinds::Solid);
    // with a position: the floor at or below its height (a storey of a building), not the roof
    let from = match p.position {
        Some(at) => {
            let at = finite(at, "position")?;
            sc.top().map(|(_, lo)| (at.y + 0.05, lo))
        }
        None => sc.top(),
    };
    let grid: Vec<Vec<Option<f32>>> = match from {
        None => vec![vec![None; columns]; rows],
        Some((hi, lo)) => {
            use rayon::prelude::*;
            (0..rows)
                .into_par_iter()
                .map(|r| {
                    let z = (z0 + r as f32 * step).min(z1);
                    (0..columns)
                        .map(|c| {
                            let x = (x0 + c as f32 * step).min(x1);
                            down(sc, Vec3::new(x, hi, z), hi - lo, &f).map(|h| h.point.y)
                        })
                        .collect()
                })
                .collect()
        }
    };
    let found = grid.iter().flatten().flatten().copied();
    let (min, max) = found.fold((None::<f32>, None::<f32>), |(a, b), y| {
        (Some(a.map_or(y, |a| a.min(y))), Some(b.map_or(y, |b| b.max(y))))
    });
    let heights: Vec<Value> =
        grid.iter().map(|row| Value::Array(row.iter().map(|h| opt_r4(*h)).collect())).collect();
    Ok(json!({
        "area": [r4(x0), r4(z0), r4(x1), r4(z1)],
        "step": r4(step),
        "columns": columns,
        "rows": rows,
        "heights": heights,
        "min": opt_r4(min),
        "max": opt_r4(max),
    }))
}

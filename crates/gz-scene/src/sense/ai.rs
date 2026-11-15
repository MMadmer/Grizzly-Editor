//! `ai`: where points are on the scene's AI map, and whether one can be walked to from another.

use std::collections::VecDeque;

use gz_editor::CmdError;
use gz_fmt_level::AiMapNode;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};

use super::{MAX_POINTS, MAX_WALK, QueryParams, Scene, finite};
use crate::cmd::{r4, v3};
use crate::math::Vec3;

/// How far above or below its node's patch a point still is on the map, metres.
const HEIGHT_BAND: f32 = 2.0;
/// The rings of cells the nearest node is looked for in before every node is.
const RINGS: i32 = 32;

/// The nodes in cells of a patch's size.
struct Map {
    nodes: Vec<AiMapNode>,
    ps: f32,
    cells: FxHashMap<(i32, i32), Vec<u32>>,
}

impl Map {
    fn new(nodes: Vec<AiMapNode>, ps: f32) -> Self {
        let ps = if ps.is_finite() && ps > 0.0 { ps } else { 0.7 };
        let mut cells: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        for (i, n) in nodes.iter().enumerate() {
            cells.entry(Self::cell_of(ps, n.pos)).or_default().push(i as u32);
        }
        Self { nodes, ps, cells }
    }

    fn cell_of(ps: f32, p: Vec3) -> (i32, i32) {
        ((p.x / ps).floor() as i32, (p.z / ps).floor() as i32)
    }

    /// The node nearest a point (in space), with its distance.
    fn nearest(&self, p: Vec3) -> Option<(u32, f32)> {
        let best_of = |ids: &mut dyn Iterator<Item = u32>| {
            ids.filter_map(|i| self.nodes.get(i as usize).map(|n| (i, n.pos.distance(p))))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        };
        let (cx, cz) = Self::cell_of(self.ps, p);
        let mut best: Option<(u32, f32)> = None;
        for r in 0..=RINGS {
            // a ring further out than the best distance cannot hold a nearer node
            if best.is_some_and(|(_, d)| d < (r - 1).max(0) as f32 * self.ps) {
                break;
            }
            for x in cx - r..=cx + r {
                for z in cz - r..=cz + r {
                    if (x - cx).abs() != r && (z - cz).abs() != r {
                        continue;
                    }
                    let Some(ids) = self.cells.get(&(x, z)) else { continue };
                    if let Some(b) = best_of(&mut ids.iter().copied())
                        && best.is_none_or(|(_, d)| b.1 < d)
                    {
                        best = Some(b);
                    }
                }
            }
        }
        best.or_else(|| best_of(&mut (0..self.nodes.len() as u32)))
    }

    /// Whether a point stands on a node's patch.
    fn on(&self, p: Vec3, node: u32) -> bool {
        self.nodes.get(node as usize).is_some_and(|n| {
            let half = self.ps * 0.5 + 1e-3;
            (p.x - n.pos.x).abs() <= half
                && (p.z - n.pos.z).abs() <= half
                && (p.y - n.pos.y).abs() <= HEIGHT_BAND
        })
    }

    fn point(&self, p: Vec3) -> (Value, Option<u32>) {
        match self.nearest(p) {
            Some((i, d)) => {
                let pos = self.nodes.get(i as usize).map_or(Vec3::ZERO, |n| n.pos);
                (
                    json!({ "position": v3(p), "on_map": self.on(p, i), "node": v3(pos), "distance": r4(d) }),
                    Some(i),
                )
            }
            None => (
                json!({ "position": v3(p), "on_map": false, "node": Value::Null, "distance": Value::Null }),
                None,
            ),
        }
    }

    /// A walk over the links from one node to another: its length in metres, `None` when there
    /// is none, `Err` with the nodes visited when the walk was cut at [`MAX_WALK`].
    fn walk(&self, from: u32, to: u32) -> Result<Option<f32>, usize> {
        let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
        let mut queue = VecDeque::from([from]);
        parent.insert(from, from);
        while let Some(i) = queue.pop_front() {
            if i == to {
                let mut len = 0.0;
                let mut at = to;
                while at != from {
                    let Some(&up) = parent.get(&at) else { break };
                    if let (Some(a), Some(b)) = (self.nodes.get(at as usize), self.nodes.get(up as usize)) {
                        len += a.pos.distance(b.pos);
                    }
                    at = up;
                }
                return Ok(Some(len));
            }
            if parent.len() > MAX_WALK {
                return Err(parent.len());
            }
            let Some(n) = self.nodes.get(i as usize) else { continue };
            for l in n.links.iter().flatten() {
                if (*l as usize) < self.nodes.len() && !parent.contains_key(l) {
                    parent.insert(*l, i);
                    queue.push_back(*l);
                }
            }
        }
        Ok(None)
    }
}

pub(super) fn ai(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let map = sc.state.bulk.ai_map.as_ref().map(|m| Map::new(m.all_nodes(), m.header.params.patch_size));
    let nodes = map.as_ref().map_or(0, |m| m.nodes.len());
    let empty = Map::new(Vec::new(), 0.7);
    let map = map.as_ref().unwrap_or(&empty);
    match (&p.positions, p.from, p.to) {
        (Some(list), None, None) => {
            if list.len() > MAX_POINTS {
                return Err(CmdError::invalid(format!(
                    "ai takes at most {MAX_POINTS} positions, not {}",
                    list.len()
                )));
            }
            let pts = list.iter().map(|v| finite(*v, "positions")).collect::<Result<Vec<_>, _>>()?;
            use rayon::prelude::*;
            let points: Vec<Value> = pts.par_iter().map(|q| map.point(*q).0).collect();
            Ok(json!({ "nodes": nodes, "points": points }))
        }
        (None, Some(a), Some(b)) => {
            let (a, b) = (finite(a, "from")?, finite(b, "to")?);
            let ((fa, na), (fb, nb)) = (map.point(a), map.point(b));
            let mut out = json!({ "nodes": nodes, "from": fa, "to": fb });
            let (reachable, length, cut) = match (na, nb) {
                (Some(x), Some(y)) => match map.walk(x, y) {
                    Ok(Some(len)) => (json!(true), json!(r4(len)), None),
                    Ok(None) => (json!(false), Value::Null, None),
                    Err(visited) => (Value::Null, Value::Null, Some(visited)),
                },
                _ => (json!(false), Value::Null, None),
            };
            if let Value::Object(m) = &mut out {
                m.insert("reachable".into(), reachable);
                m.insert("length".into(), length);
                if let Some(v) = cut {
                    m.insert("cut".into(), json!(format!(
                        "the walk stopped after {v} nodes (at most {MAX_WALK}): whether to can be reached is not known"
                    )));
                }
            }
            Ok(out)
        }
        _ => Err(CmdError::invalid("ai takes positions: [[x,y,z], ...], or from and to")),
    }
}

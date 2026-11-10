//! What the canvas draws, built once per graph revision on the CPU pool (`NqCanvas::BuildGeometry`,
//! `BuildLinks`, `RouteBack`, `Extent`): every node's box from `gz_nq::layout` (the ONE source of
//! a node's rectangle), its texts, its pins; every link as a bezier or, for an edge that climbs
//! back up, a route around the nodes through a free channel; a uniform grid index so a frame
//! touches only the cells the view covers; the extents Frame All fits; the edges sorted for the
//! `beyond` counts; the minimap's mesh. Units are world pt (pt at zoom 1).

use std::sync::Arc;

use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, Vec2, pos2};
use gz_nq::asset::Graph;
use gz_nq::layout::{self, Line, NodeBox};
use gz_nq::{Catalog, text};
use gz_ui::{Icon, Palette};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::look::{self, Family};

/// The side of a grid cell, world pt.
pub const CELL: f32 = 512.0;
/// A link spanning more cells than this is kept on a list every query looks at.
const MAX_LINK_CELLS: i64 = 256;
pub const LINK_TANGENT: f32 = 48.0;
/// An edge that goes back up: the straight run out of the pin and into the input.
pub const BACK_STUB: f32 = 24.0;
/// How far its channel keeps from a node.
const BACK_CLEAR: f32 = 24.0;
/// Between two edges that share a channel.
const BACK_LANE: f32 = 12.0;
/// The radius of its corners.
const BACK_CORNER: f32 = 12.0;
/// World pt Frame All keeps around what it shows.
pub const FRAME_MARGIN: f32 = 32.0;
/// The minimap's box, pt.
pub const MINIMAP_W: f32 = 180.0;
pub const MINIMAP_H: f32 = 120.0;

/// One action row of a node: what it does, in words.
#[derive(Clone, Debug)]
pub struct ActionRow {
    pub exit: bool,
    /// In its slot.
    pub index: usize,
    pub kind: SmolStr,
    pub icon: Icon,
    pub family: Family,
    pub title: String,
    /// What it acts on; empty for nothing.
    pub summary: String,
}

/// One node as the canvas draws it.
#[derive(Clone, Debug)]
pub struct SceneNode {
    pub id: SmolStr,
    pub kind: SmolStr,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub b: NodeBox,
    pub trigger: bool,
    pub title: String,
    pub icon: Icon,
    pub family: Family,
    /// The `once` badge.
    pub once: bool,
    pub lines: Vec<Line>,
    pub actions: Vec<ActionRow>,
    /// The first action the node has no row for: what `+N more` opens.
    pub more_target: Option<(bool, usize)>,
    /// Output pins in draw order; `pins[..declared]` come from the catalog.
    pub pins: Vec<String>,
    pub declared: usize,
    pub reachable: bool,
    /// The first line, for the tooltip of a node too small to read.
    pub summary: String,
    /// It takes an input edge (Q's `CanTakeInput`).
    pub takes_input: bool,
}

impl SceneNode {
    #[must_use]
    pub fn rect(&self) -> [f32; 4] {
        [self.pos[0], self.pos[1], self.pos[0] + self.size[0], self.pos[1] + self.size[1]]
    }

    #[must_use]
    pub fn input_pin(&self) -> [f32; 2] {
        layout::input_pin(self.pos, self.size)
    }

    #[must_use]
    pub fn output_pin(&self, pin: usize) -> [f32; 2] {
        layout::output_pin(self.pos, self.size, pin, self.pins.len())
    }

    /// The action at a row, top to bottom (`ActionAtRow`).
    #[must_use]
    pub fn action_at(&self, row: usize) -> Option<&ActionRow> {
        self.actions.get(row)
    }
}

/// One link, world pt.
#[derive(Clone, Debug)]
pub struct SceneLink {
    pub from: u32,
    pub to: u32,
    pub pin: SmolStr,
    pub p0: [f32; 2],
    pub p3: [f32; 2],
    /// The route of an edge that climbs back up; empty for a bezier.
    pub path: Vec<[f32; 2]>,
    pub bmin: [f32; 2],
    pub bmax: [f32; 2],
}

impl SceneLink {
    /// The bezier's inner control points (world).
    #[must_use]
    pub fn controls(p0: [f32; 2], p3: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let d = LINK_TANGENT.max((p3[1] - p0[1]).abs() * 0.4);
        ([p0[0], p0[1] + d], [p3[0], p3[1] - d])
    }

    /// Whether an edge between these pins has to be routed back up.
    #[must_use]
    pub fn goes_back(p0: [f32; 2], p3: [f32; 2]) -> bool {
        p3[1] - p0[1] < BACK_STUB * 2.0
    }
}

/// The vertical run of a routed edge: the next one keeps clear of it.
#[derive(Clone, Copy, Debug)]
pub struct Lane {
    x: f32,
    top: f32,
    bottom: f32,
    from: [f32; 2],
    to: [f32; 2],
    side_from: f32,
    side_to: f32,
}

/// The minimap's static picture: nodes and edges fitted into the 180 x 120 box, in box-local pt.
pub struct Minimap {
    pub mesh: Mesh,
    /// World point at the fitted area's top-left.
    pub world_min: [f32; 2],
    pub scale: f32,
    /// The fitted area inside the box (box-local).
    pub fitted: Rect,
}

impl Minimap {
    /// A world point in box-local pt.
    #[must_use]
    pub fn to_local(&self, w: [f32; 2]) -> Pos2 {
        pos2(self.fitted.min.x + (w[0] - self.world_min[0]) * self.scale, self.fitted.min.y + (w[1] - self.world_min[1]) * self.scale)
    }

    /// A box-local point (clamped to the fitted area) in world pt.
    #[must_use]
    pub fn to_world(&self, local: Pos2) -> [f32; 2] {
        if self.scale <= 0.0 {
            return [0.0, 0.0];
        }
        let x = local.x.clamp(self.fitted.min.x, self.fitted.max.x);
        let y = local.y.clamp(self.fitted.min.y, self.fitted.max.y);
        [self.world_min[0] + (x - self.fitted.min.x) / self.scale, self.world_min[1] + (y - self.fitted.min.y) / self.scale]
    }
}

/// An edge of a node and the node, sorted, for the `beyond` counts.
type Edges = Vec<(f32, u32)>;

/// Everything the canvas draws of one revision.
pub struct Scene {
    /// The document's content serial this was built from.
    pub edit: u64,
    pub generation: u32,
    pub behaviour: bool,
    pub nodes: Vec<SceneNode>,
    pub links: Vec<SceneLink>,
    pub index: FxHashMap<SmolStr, u32>,
    node_cells: FxHashMap<(i32, i32), Vec<u32>>,
    link_cells: FxHashMap<(i32, i32), Vec<u32>>,
    big_links: Vec<u32>,
    /// The node boxes.
    pub bounds: Option<[f32; 4]>,
    /// The node boxes and the routed edges: what Frame All fits.
    pub extent: Option<[f32; 4]>,
    /// The start (the trigger, every root without one) and two levels under it, with their edges.
    pub start_extent: Option<[f32; 4]>,
    pub anchor: Option<u32>,
    tops: Edges,
    bottoms: Edges,
    lefts: Edges,
    rights: Edges,
    pub minimap: Option<Minimap>,
}

fn cell_of(v: f32) -> i32 {
    (v / CELL).floor().clamp(-1.0e6, 1.0e6) as i32
}

fn grow(b: &mut Option<[f32; 4]>, p: [f32; 2]) {
    match b {
        Some(r) => {
            r[0] = r[0].min(p[0]);
            r[1] = r[1].min(p[1]);
            r[2] = r[2].max(p[0]);
            r[3] = r[3].max(p[1]);
        }
        None => *b = Some([p[0], p[1], p[0], p[1]]),
    }
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])
}

/// The x of a free vertical channel: outside every span, as short a detour for a run from `x0`
/// to `x3` as there is; between the two every x costs the same, so the middle of what is free
/// there. A tie goes to `x0`'s side (`PickChannel`).
#[must_use]
pub fn pick_channel(spans: &mut [(f32, f32)], x0: f32, x3: f32) -> f32 {
    spans.sort_by(|l, r| l.0.total_cmp(&r.0));
    let (lo, hi) = (x0.min(x3), x0.max(x3));
    let mut best = x0;
    let mut best_cost = f32::MAX;
    let mut consider = |from: f32, to: f32| {
        if from > to {
            return;
        }
        let x = if to < lo {
            to
        } else if from > hi {
            from
        } else {
            (from.max(lo) + to.min(hi)) * 0.5
        };
        let cost = (x - x0).abs() * 1.001 + (x - x3).abs();
        if cost < best_cost {
            best_cost = cost;
            best = x;
        }
    };
    let mut free_from = -f32::MAX;
    for s in spans.iter() {
        consider(free_from, s.0);
        free_from = free_from.max(s.1);
    }
    consider(free_from, f32::MAX);
    best
}

/// The route of an edge from pin `p0` up to input `p3` (`RouteBack`): a slant down and aside out
/// of the pin, sideways to a free vertical channel, up it, over the target and a slant down into
/// its input; the channel keeps clear of every node between the two heights and of the lanes
/// routed before. `boxes` are the node rectangles.
pub fn route_back(boxes: &[[f32; 4]], p0: [f32; 2], p3: [f32; 2], lanes: &mut Vec<Lane>) -> Vec<[f32; 2]> {
    let mut below = p0[1] + BACK_STUB;
    let mut above = p3[1] - BACK_STUB;
    let mut x = p0[0];
    let (mut drop, mut rise) = (BACK_STUB, BACK_STUB);
    let (mut side0, mut side3) = (1.0f32, 1.0f32);
    let mut spans: Vec<(f32, f32)> = Vec::new();
    for _ in 0..4 {
        spans.clear();
        for g in boxes {
            if g[1] - BACK_CLEAR < below && g[3] + BACK_CLEAR > above {
                spans.push((g[0] - BACK_CLEAR, g[2] + BACK_CLEAR));
            }
        }
        for lane in lanes.iter() {
            if lane.top < below && lane.bottom > above {
                spans.push((lane.x - BACK_LANE, lane.x + BACK_LANE));
            }
        }
        x = pick_channel(&mut spans, p0[0], p3[0]);
        let mut moved = false;
        side0 = if x < p0[0] { -1.0 } else { 1.0 };
        side3 = if x < p3[0] { -1.0 } else { 1.0 };
        let mut out_before = 0usize;
        let mut in_before = 0usize;
        #[allow(clippy::float_cmp)]
        for lane in lanes.iter() {
            if lane.from == p0 && lane.side_from == side0 {
                out_before += 1;
            }
            if lane.to == p3 && lane.side_to == side3 {
                in_before += 1;
            }
        }
        drop = BACK_STUB + BACK_LANE * out_before as f32;
        rise = BACK_STUB + BACK_LANE * in_before as f32;
        if below < p0[1] + drop {
            below = p0[1] + drop;
            moved = true;
        }
        if above > p3[1] - rise {
            above = p3[1] - rise;
            moved = true;
        }
        let crosses = |g: &[f32; 4], y: f32, x0: f32, x1: f32| {
            let margin = BACK_STUB * 0.5;
            y > g[1] - margin && y < g[3] + margin && g[0] < x0.max(x1) && g[2] > x0.min(x1)
        };
        for g in boxes {
            if crosses(g, below, p0[0], x) {
                below = below.max(g[3] + BACK_STUB);
                moved = true;
            }
            if crosses(g, above, x, p3[0]) {
                above = above.min(g[1] - BACK_STUB);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    lanes.push(Lane { x, top: above, bottom: below, from: p0, to: p3, side_from: side0, side_to: side3 });

    let xs = p0[0] + side0 * BACK_STUB;
    let xe = p3[0] + side3 * BACK_STUB;
    let mut corners: Vec<[f32; 2]> = Vec::with_capacity(8);
    corners.push(p0);
    corners.push([xs, p0[1] + drop]);
    if below > p0[1] + drop + 0.5 {
        corners.push([xs, below]);
    }
    corners.push([x, below]);
    corners.push([x, above]);
    corners.push([xe, above]);
    if above < p3[1] - rise - 0.5 {
        corners.push([xe, p3[1] - rise]);
    }
    corners.push(p3);

    let mut out: Vec<[f32; 2]> = Vec::with_capacity(corners.len() * 5 + 2);
    out.push(p0);
    for k in 1..corners.len().saturating_sub(1) {
        let (Some(&from), Some(&c), Some(&to)) = (corners.get(k - 1), corners.get(k), corners.get(k + 1)) else { continue };
        let inn = dist2(from, c).sqrt();
        let next = dist2(c, to).sqrt();
        let r = BACK_CORNER.min(inn.min(next) * 0.5);
        if r < 0.5 {
            if out.last().is_none_or(|l| dist2(*l, c) > 0.01) {
                out.push(c);
            }
            continue;
        }
        // a quadratic corner: from r before it, through it, to r after it
        let u = [c[0] + (from[0] - c[0]) / inn * r, c[1] + (from[1] - c[1]) / inn * r];
        let v = [c[0] + (to[0] - c[0]) / next * r, c[1] + (to[1] - c[1]) / next * r];
        for step in 0..=4 {
            let f = step as f32 / 4.0;
            let e = 1.0 - f;
            out.push([e * e * u[0] + 2.0 * e * f * c[0] + f * f * v[0], e * e * u[1] + 2.0 * e * f * c[1] + f * f * v[1]]);
        }
    }
    out.push(p3);
    out
}

/// Builds the link of an edge: a bezier, or a route with its lane.
pub fn make_link(boxes: &[[f32; 4]], from: u32, to: u32, pin: &str, p0: [f32; 2], p3: [f32; 2], lanes: &mut Vec<Lane>) -> SceneLink {
    let mut l = SceneLink { from, to, pin: SmolStr::new(pin), p0, p3, path: Vec::new(), bmin: p0, bmax: p0 };
    if SceneLink::goes_back(p0, p3) {
        l.path = route_back(boxes, p0, p3, lanes);
        for p in &l.path {
            l.bmin = [l.bmin[0].min(p[0]), l.bmin[1].min(p[1])];
            l.bmax = [l.bmax[0].max(p[0]), l.bmax[1].max(p[1])];
        }
    } else {
        let (c1, c2) = SceneLink::controls(p0, p3);
        // a bezier stays inside the hull of its control points
        for p in [c1, c2, p3] {
            l.bmin = [l.bmin[0].min(p[0]), l.bmin[1].min(p[1])];
            l.bmax = [l.bmax[0].max(p[0]), l.bmax[1].max(p[1])];
        }
    }
    l
}

/// What `Scene::build` reads.
pub struct SceneInput<'a> {
    pub graph: &'a Graph,
    pub catalog: &'a Catalog,
    pub edit: u64,
    pub palette: &'a Palette,
}

fn action_rows(cat: &Catalog, n: &gz_nq::Node, behaviour: bool, rows: usize) -> (Vec<ActionRow>, Option<(bool, usize)>) {
    let mut out = Vec::with_capacity(rows);
    let all = n.on_enter.iter().enumerate().map(|(i, a)| (false, i, a)).chain(n.on_exit.iter().enumerate().map(|(i, a)| (true, i, a)));
    let mut more = None;
    for (r, (exit, index, a)) in all.enumerate() {
        if r >= rows {
            more = Some((exit, index));
            break;
        }
        let icon = look::kind_icon(cat, &a.kind);
        let mut summary = text::value_summary(&a.params, 64);
        if summary == "{}" {
            summary.clear();
        }
        out.push(ActionRow { exit, index, kind: a.kind.clone(), icon, family: look::family_of(icon), title: look::kind_title(cat, &a.kind, behaviour), summary });
    }
    (out, more)
}

impl Scene {
    /// Builds the scene of a graph. Pure: CPU pool.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn build(inp: &SceneInput<'_>) -> Scene {
        let g = inp.graph;
        let cat = inp.catalog;
        let behaviour = g.is_behaviour();
        let reach = g.reachable(cat, None);
        let mut nodes: Vec<SceneNode> = Vec::with_capacity(g.nodes.len());
        let mut index: FxHashMap<SmolStr, u32> = FxHashMap::default();
        index.reserve(g.nodes.len());
        for (i, n) in g.nodes.iter().enumerate() {
            let b = layout::node_box(cat, n);
            let mut lines = layout::lines(cat, n);
            // a condition is named the way the document speaks: a behaviour graph never says "quest"
            for line in &mut lines {
                if !line.cond.is_empty() {
                    line.key = layout::cond_key(&look::kind_title(cat, &line.cond, behaviour), line.negate);
                }
            }
            let (pins, declared) = layout::pins(cat, n);
            let trigger = cat.is_trigger(&n.kind);
            let icon = look::kind_icon(cat, &n.kind);
            let kind = cat.find(&n.kind);
            let once = !trigger && if n.once_set { n.once } else { kind.is_some_and(|k| k.once_default) };
            let (actions, more_target) = if trigger { (Vec::new(), None) } else { action_rows(cat, n, behaviour, b.action_rows) };
            let summary = lines.first().map_or_else(String::new, |l| {
                format!("{}{}{}", l.key, if l.cond.is_empty() { ": " } else { " " }, text::clip_utf8(&l.value, 64))
            });
            let takes_input = !trigger && (n.kind == "flow.join" || g.in_degree(&n.id) == 0);
            index.insert(n.id.clone(), i as u32);
            nodes.push(SceneNode {
                id: n.id.clone(),
                kind: n.kind.clone(),
                pos: n.pos,
                size: [b.w, b.h],
                b,
                trigger,
                title: look::kind_title(cat, &n.kind, behaviour),
                icon,
                family: look::family_of(icon),
                once,
                lines,
                actions,
                more_target,
                pins,
                declared,
                reachable: reach.get(i).copied().unwrap_or(true),
                summary,
                takes_input,
            });
        }
        let boxes: Vec<[f32; 4]> = nodes.iter().map(SceneNode::rect).collect();

        // the links, in the order the C++ routes them: the lanes routed before an edge move it
        let mut links: Vec<SceneLink> = Vec::new();
        let mut lanes: Vec<Lane> = Vec::new();
        for (i, n) in g.nodes.iter().enumerate() {
            let Some(sn) = nodes.get(i) else { continue };
            for p in &n.out {
                let Some(pin) = sn.pins.iter().position(|x| x == p.name.as_str()) else { continue };
                for t in &p.targets {
                    let Some(&to) = index.get(t) else { continue };
                    let Some(tn) = nodes.get(to as usize) else { continue };
                    links.push(make_link(&boxes, i as u32, to, &p.name, sn.output_pin(pin), tn.input_pin(), &mut lanes));
                }
            }
        }

        // what Frame All fits, and where reading starts
        let mut bounds: Option<[f32; 4]> = None;
        for b in &boxes {
            grow(&mut bounds, [b[0], b[1]]);
            grow(&mut bounds, [b[2], b[3]]);
        }
        let mut extent = bounds;
        for l in &links {
            for p in &l.path {
                grow(&mut extent, *p);
            }
        }
        let (start_extent, anchor) = start_region(g, cat, &nodes, &boxes, &links, &index);

        // the grid
        let mut node_cells: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        for (i, b) in boxes.iter().enumerate() {
            for cy in cell_of(b[1])..=cell_of(b[3]) {
                for cx in cell_of(b[0])..=cell_of(b[2]) {
                    node_cells.entry((cx, cy)).or_default().push(i as u32);
                }
            }
        }
        let mut link_cells: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        let mut big_links: Vec<u32> = Vec::new();
        for (i, l) in links.iter().enumerate() {
            let (x0, y0, x1, y1) = (cell_of(l.bmin[0]), cell_of(l.bmin[1]), cell_of(l.bmax[0]), cell_of(l.bmax[1]));
            let cells = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1);
            if cells > MAX_LINK_CELLS {
                big_links.push(i as u32);
                continue;
            }
            for cy in y0..=y1 {
                for cx in x0..=x1 {
                    link_cells.entry((cx, cy)).or_default().push(i as u32);
                }
            }
        }

        let sorted = |f: &dyn Fn(&[f32; 4]) -> f32, ties_low_first: bool| -> Edges {
            let mut v: Edges = boxes.iter().enumerate().map(|(i, b)| (f(b), i as u32)).collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0).then(if ties_low_first { a.1.cmp(&b.1) } else { b.1.cmp(&a.1) }));
            v
        };
        let tops = sorted(&|b| b[1], true);
        let bottoms = sorted(&|b| b[3], false);
        let lefts = sorted(&|b| b[0], true);
        let rights = sorted(&|b| b[2], false);
        let minimap = bounds.map(|b| build_minimap(inp.palette, &nodes, &links, b));

        Scene {
            edit: inp.edit,
            generation: cat.generation,
            behaviour,
            nodes,
            links,
            index,
            node_cells,
            link_cells,
            big_links,
            bounds,
            extent,
            start_extent,
            anchor,
            tops,
            bottoms,
            lefts,
            rights,
            minimap,
        }
    }

    #[must_use]
    pub fn node_of(&self, id: &str) -> Option<usize> {
        self.index.get(id).map(|&i| i as usize)
    }

    /// The nodes whose cells a world rectangle touches; may repeat - the caller dedupes.
    pub fn nodes_in(&self, r: [f32; 4], out: &mut Vec<u32>) {
        let (x0, y0, x1, y1) = (cell_of(r[0]), cell_of(r[1]), cell_of(r[2]), cell_of(r[3]));
        // a view so far out that it covers more cells than there are nodes: every node
        if (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1) > self.node_cells.len() as i64 {
            for v in self.node_cells.values() {
                out.extend_from_slice(v);
            }
            return;
        }
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if let Some(v) = self.node_cells.get(&(cx, cy)) {
                    out.extend_from_slice(v);
                }
            }
        }
    }

    /// The links whose cells a world rectangle touches; may repeat.
    pub fn links_in(&self, r: [f32; 4], out: &mut Vec<u32>) {
        out.extend_from_slice(&self.big_links);
        let (x0, y0, x1, y1) = (cell_of(r[0]), cell_of(r[1]), cell_of(r[2]), cell_of(r[3]));
        if (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1) > self.link_cells.len() as i64 {
            for v in self.link_cells.values() {
                out.extend_from_slice(v);
            }
            return;
        }
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if let Some(v) = self.link_cells.get(&(cx, cy)) {
                    out.extend_from_slice(v);
                }
            }
        }
    }

    /// The nodes wholly beyond each edge of a view (above, below, left, right) and the nearest
    /// of each: the one the least pan brings in (`DrawBeyond`'s counts).
    #[must_use]
    pub fn beyond(&self, view: [f32; 4]) -> ([usize; 4], [Option<u32>; 4]) {
        let n = self.nodes.len();
        let above = self.bottoms.partition_point(|e| e.0 <= view[1]);
        let below_start = self.tops.partition_point(|e| e.0 < view[3]);
        let left = self.rights.partition_point(|e| e.0 <= view[0]);
        let right_start = self.lefts.partition_point(|e| e.0 < view[2]);
        let counts = [above, n - below_start, left, n - right_start];
        let near = [
            above.checked_sub(1).and_then(|i| self.bottoms.get(i)).map(|e| e.1),
            self.tops.get(below_start).map(|e| e.1),
            left.checked_sub(1).and_then(|i| self.rights.get(i)).map(|e| e.1),
            self.lefts.get(right_start).map(|e| e.1),
        ];
        (counts, near)
    }
}

/// The trigger (every root without one, the first node without those) and two levels of what
/// follows it, with the routed edges between them (`FrameAll`'s `keep`).
fn start_region(g: &Graph, cat: &Catalog, nodes: &[SceneNode], boxes: &[[f32; 4]], links: &[SceneLink], index: &FxHashMap<SmolStr, u32>) -> (Option<[f32; 4]>, Option<u32>) {
    if nodes.is_empty() {
        return (None, None);
    }
    let mut keep = vec![false; nodes.len()];
    let mut front: Vec<usize> = (0..nodes.len()).filter(|&i| g.nodes.get(i).is_some_and(|n| cat.is_trigger(&n.kind))).collect();
    if front.is_empty() {
        front = (0..nodes.len()).filter(|&i| g.nodes.get(i).is_some_and(|n| g.in_degree(&n.id) == 0)).collect();
    }
    if front.is_empty() {
        front.push(0);
    }
    let anchor = front.first().map(|&i| i as u32);
    for _ in 0..=2 {
        if front.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for &i in &front {
            if keep.get(i).copied().unwrap_or(true) {
                continue;
            }
            if let Some(k) = keep.get_mut(i) {
                *k = true;
            }
            let Some(n) = g.nodes.get(i) else { continue };
            for p in &n.out {
                for t in &p.targets {
                    if let Some(&to) = index.get(t)
                        && !keep.get(to as usize).copied().unwrap_or(true)
                    {
                        next.push(to as usize);
                    }
                }
            }
        }
        front = next;
    }
    let mut ext: Option<[f32; 4]> = None;
    for (i, b) in boxes.iter().enumerate() {
        if keep.get(i).copied().unwrap_or(false) {
            grow(&mut ext, [b[0], b[1]]);
            grow(&mut ext, [b[2], b[3]]);
        }
    }
    for l in links {
        if keep.get(l.from as usize).copied().unwrap_or(false) && keep.get(l.to as usize).copied().unwrap_or(false) {
            for p in &l.path {
                grow(&mut ext, *p);
            }
        }
    }
    (ext, anchor)
}

/// A line as a quad of a mesh.
fn mesh_line(mesh: &mut Mesh, a: Pos2, b: Pos2, w: f32, color: Color32) {
    let d = b - a;
    let len = d.length();
    if len < 0.01 {
        return;
    }
    let n = Vec2::new(-d.y, d.x) / len * (w * 0.5);
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(a + n, color);
    mesh.colored_vertex(b + n, color);
    mesh.colored_vertex(b - n, color);
    mesh.colored_vertex(a - n, color);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// The minimap's picture in box-local pt: the edges centre to centre, the nodes in their family
/// colours. Built with the scene, so a frame only translates a copy of it (`DrawMinimap`).
fn build_minimap(p: &Palette, nodes: &[SceneNode], links: &[SceneLink], b: [f32; 4]) -> Minimap {
    let pad = 6.0;
    let avail = [(MINIMAP_W - pad * 2.0).max(8.0), (MINIMAP_H - pad * 2.0).max(8.0)];
    let world_pad = layout::GRID * 2.0;
    let world_min = [b[0] - world_pad, b[1] - world_pad];
    let world_w = (b[2] + world_pad - world_min[0]).max(1.0);
    let world_h = (b[3] + world_pad - world_min[1]).max(1.0);
    let scale = (avail[0] / world_w).min(avail[1] / world_h);
    let fitted = Vec2::new(world_w * scale, world_h * scale);
    let min = pos2((MINIMAP_W - fitted.x) * 0.5, (MINIMAP_H - fitted.y) * 0.5);
    let mut m = Minimap { mesh: Mesh::default(), world_min, scale, fitted: Rect::from_min_size(min, fitted) };
    let edge = Palette::alpha(p.text_muted, 0.5);
    let centre = |n: &SceneNode| [n.pos[0] + n.size[0] * 0.5, n.pos[1] + n.size[1] * 0.5];
    let mut mesh = Mesh::default();
    for l in links {
        let (Some(a), Some(z)) = (nodes.get(l.from as usize), nodes.get(l.to as usize)) else { continue };
        mesh_line(&mut mesh, m.to_local(centre(a)), m.to_local(centre(z)), 1.0, edge);
    }
    let dot = 2.0;
    for n in nodes {
        let a = m.to_local(n.pos);
        let z = m.to_local([n.pos[0] + n.size[0], n.pos[1] + n.size[1]]);
        let r = Rect::from_min_max(a, pos2(z.x.max(a.x + dot), z.y.max(a.y + dot)));
        mesh.add_colored_rect(r, look::family_color(p, n.family, 0.85));
    }
    m.mesh = mesh;
    m
}

/// Where a Frame All lands: the zoom level and the centre, and the status when the graph is
/// framed from its start.
#[derive(Clone, Debug, PartialEq)]
pub struct Framed {
    pub zoom: i32,
    pub center: [f32; 2],
    pub status: Option<String>,
}

/// The centre of a view `len` long on one axis of the graph `[a0, a1]`: the middle when it all
/// fits; else `prefer`, moved as little as it takes to keep `[k0, k1]` in view (`fallback` when
/// that does not fit either), never past an end of the graph (`Along`).
#[must_use]
pub fn along(a0: f32, a1: f32, k0: f32, k1: f32, len: f32, prefer: f32, fallback: f32) -> f32 {
    if a1 - a0 <= len {
        return (a0 + a1) * 0.5;
    }
    let half = len * 0.5;
    if k1 - k0 > len {
        return (a0 + half).max(fallback.min(a1 - half));
    }
    let c = (a0 + half).max(prefer.min(a1 - half));
    (k1 - half).max(c.min(k0 + half))
}

/// Frame All (`NqCanvas::FrameAll`): the highest level up to 1:1, never below the floor, that
/// holds the whole extent; a graph that does not fit there is shown AT the floor from its start.
/// `view` is the canvas in pt.
#[must_use]
pub fn frame_all(s: &Scene, view: [f32; 2]) -> Framed {
    let Some(e) = s.extent.filter(|_| !s.nodes.is_empty()) else {
        return Framed { zoom: gz_nq::doc::ZOOM_DEFAULT, center: [0.0, 0.0], status: None };
    };
    let (mn, mx) = ([e[0] - FRAME_MARGIN, e[1] - FRAME_MARGIN], [e[2] + FRAME_MARGIN, e[3] + FRAME_MARGIN]);
    for z in (gz_nq::doc::ZOOM_FRAME_MIN..=gz_nq::doc::ZOOM_DEFAULT).rev() {
        let k = gz_nq::doc::zoom_of(z);
        if (mx[0] - mn[0]) * k <= view[0] && (mx[1] - mn[1]) * k <= view[1] {
            return Framed { zoom: z, center: [layout::sane((mn[0] + mx[0]) * 0.5), layout::sane((mn[1] + mx[1]) * 0.5)], status: None };
        }
    }
    let k = gz_nq::doc::zoom_of(gz_nq::doc::ZOOM_FRAME_MIN);
    let v = [view[0] / k, view[1] / k];
    let se = s.start_extent.unwrap_or(e);
    let (kmn, kmx) = ([se[0] - FRAME_MARGIN, se[1] - FRAME_MARGIN], [se[2] + FRAME_MARGIN, se[3] + FRAME_MARGIN]);
    let a = s.anchor.and_then(|i| s.nodes.get(i as usize));
    let ax = a.map_or((mn[0] + mx[0]) * 0.5, |a| a.pos[0] + a.size[0] * 0.5);
    let ay = a.map_or(mn[1], |a| a.pos[1]);
    let cx = along(mn[0], mx[0], kmn[0], kmx[0], v[0], (mn[0] + mx[0]) * 0.5, ax);
    let cy = along(mn[1], mx[1], kmn[1], kmx[1], v[1], kmn[1] + v[1] * 0.5, ay - FRAME_MARGIN + v[1] * 0.5);
    let pct = (k * 100.0 + 0.5) as i32;
    Framed {
        zoom: gz_nq::doc::ZOOM_FRAME_MIN,
        center: [layout::sane(cx), layout::sane(cy)],
        status: Some(format!("The {} is larger than the view at {pct}%: framed from its start", look::doc_noun(s.behaviour))),
    }
}

/// Frames a world rectangle (`FrameRect`): the highest level that holds it with a gap around,
/// never closer than 1:1.
#[must_use]
pub fn frame_rect(r: [f32; 4], view: [f32; 2]) -> (i32, [f32; 2]) {
    let w = (r[2] - r[0] + 2.0 * layout::GAP_X).max(1.0);
    let h = (r[3] - r[1] + 2.0 * layout::GAP_Y).max(1.0);
    let mut z = gz_nq::doc::ZOOM.len() as i32 - 1;
    if view[0] > 0.0 && view[1] > 0.0 {
        while z > 0 && (w * gz_nq::doc::zoom_of(z) > view[0] || h * gz_nq::doc::zoom_of(z) > view[1]) {
            z -= 1;
        }
    }
    (z.min(gz_nq::doc::ZOOM_DEFAULT), [layout::sane((r[0] + r[2]) * 0.5), layout::sane((r[1] + r[3]) * 0.5)])
}

/// A scene shared by the frames that draw it.
pub type SharedScene = Arc<Scene>;

#[cfg(test)]
mod tests {
    use super::*;
    use gz_nq::Node;

    fn graph() -> (Graph, Catalog) {
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.pos = [0.0, 0.0];
        s.has_pos = true;
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.pos = [0.0, 200.0];
        a.has_pos = true;
        a.connect("next", "a2");
        let mut a2 = Node::new("a2", "flow.step");
        a2.pos = [0.0, 400.0];
        a2.has_pos = true;
        // back up to a: routed around
        a2.connect("next", "a");
        for n in [s, a, a2] {
            g.nodes.push_back(n);
        }
        (g, Catalog::empty())
    }

    #[test]
    fn builds_links_and_routes() {
        let (g, cat) = graph();
        let p = Palette::new(gz_ui::Accent::default());
        let s = Scene::build(&SceneInput { graph: &g, catalog: &cat, edit: 1, palette: &p });
        assert_eq!(s.nodes.len(), 3);
        assert_eq!(s.links.len(), 3);
        assert!(s.links.iter().filter(|l| !l.path.is_empty()).count() == 1);
        // the routed edge widens the extent past the boxes
        let (b, e) = (s.bounds.unwrap_or_default(), s.extent.unwrap_or_default());
        assert!(e[0] < b[0] || e[2] > b[2]);
        let mut hit = Vec::new();
        s.nodes_in([0.0, 0.0, 10.0, 10.0], &mut hit);
        assert!(hit.contains(&0));
    }

    #[test]
    fn beyond_counts() {
        let (g, cat) = graph();
        let p = Palette::new(gz_ui::Accent::default());
        let s = Scene::build(&SceneInput { graph: &g, catalog: &cat, edit: 1, palette: &p });
        let (c, near) = s.beyond([-100.0, 150.0, 400.0, 350.0]);
        assert_eq!(c[0], 1);
        assert_eq!(c[1], 1);
        assert_eq!(near[0], Some(0));
        assert_eq!(near[1], Some(2));
    }

    #[test]
    fn framing() {
        let (g, cat) = graph();
        let p = Palette::new(gz_ui::Accent::default());
        let s = Scene::build(&SceneInput { graph: &g, catalog: &cat, edit: 1, palette: &p });
        let f = frame_all(&s, [1600.0, 1000.0]);
        assert_eq!(f.zoom, 6);
        assert!(f.status.is_none());
        let small = frame_all(&s, [200.0, 150.0]);
        assert_eq!(small.zoom, 5);
        assert!(small.status.is_some());
        assert_eq!(along(0.0, 100.0, 0.0, 10.0, 200.0, 5.0, 5.0), 50.0);
        let mut spans = vec![(-10.0, 10.0)];
        assert_eq!(pick_channel(&mut spans, 0.0, 0.0), -10.0);
    }
}

//! What the canvas draws, built once per graph revision (and wire style) on the CPU pool: every
//! node's box from `gz_nq::layout` (the ONE source of a node's rectangle), its texts in words,
//! its pins with what they mean; every wire as the polyline `gz_nq::wires` builds in its style -
//! the same one the hit tests and `graph.get` read; the comment frames; a uniform grid index so a
//! frame touches only the cells the view covers; the extent Frame All fits; the edges sorted for
//! the `beyond` counts; the minimap's mesh. Units are world pt (pt at zoom 1).

use std::sync::Arc;

use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, Vec2, pos2};
use gz_nq::asset::Graph;
use gz_nq::canvas::{Flow, WireSettings};
use gz_nq::layout::{self, BoxCx, Line, NodeBox};
use gz_nq::text::Names;
use gz_nq::{Catalog, text, wires};
use gz_ui::{Icon, Palette};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::look::{self, Family};

/// The side of a grid cell, world pt.
pub const CELL: f32 = 512.0;
/// A link spanning more cells than this is kept on a list every query looks at.
const MAX_LINK_CELLS: i64 = 256;
/// World pt Frame All keeps around what it shows.
pub const FRAME_MARGIN: f32 = 32.0;
/// The minimap's box, pt.
pub const MINIMAP_W: f32 = 180.0;
pub const MINIMAP_H: f32 = 120.0;

/// What a pin's wire is: the flow going on, a failure, or one of the other ways out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinRole {
    Default,
    Failure,
    /// The n-th pin that is neither.
    Other(usize),
}

/// One output pin of a node.
#[derive(Clone, Debug)]
pub struct PinInfo {
    pub name: String,
    pub role: PinRole,
    /// What the catalog says it means.
    pub desc: String,
    pub connected: bool,
}

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
    pub collapsed: bool,
    pub disabled: bool,
    /// The comment's first line.
    pub comment: String,
    pub lines: Vec<Line>,
    pub actions: Vec<ActionRow>,
    /// The first action the node has no row for: what `+N more` opens.
    pub more_target: Option<(bool, usize)>,
    /// Output pins in draw order; `pins[..declared]` come from the catalog.
    pub pins: Vec<PinInfo>,
    pub declared: usize,
    pub reachable: bool,
    /// The first line, for the tooltip of a node too small to read.
    pub summary: String,
    /// It takes an input edge.
    pub takes_input: bool,
    /// What the kind does, for the header's tooltip.
    pub desc: String,
}

impl SceneNode {
    #[must_use]
    pub fn rect(&self) -> [f32; 4] {
        [self.pos[0], self.pos[1], self.pos[0] + self.size[0], self.pos[1] + self.size[1]]
    }

    /// The action at a row, top to bottom.
    #[must_use]
    pub fn action_at(&self, row: usize) -> Option<&ActionRow> {
        self.actions.get(row)
    }

    /// The index of a pin by its name.
    #[must_use]
    pub fn pin_index(&self, name: &str) -> Option<usize> {
        self.pins.iter().position(|p| p.name == name)
    }
}

/// One wire, world pt.
#[derive(Clone, Debug)]
pub struct SceneLink {
    pub from: u32,
    pub to: u32,
    pub pin: SmolStr,
    pub pin_index: usize,
    pub role: PinRole,
    /// The polyline the wire is drawn and hit along.
    pub points: Vec<[f32; 2]>,
    pub back: bool,
    /// Its waypoints (reroutes), as the file keeps them.
    pub via: Vec<[f32; 2]>,
    /// The pin's name on the wire's first stretch, when it is not the default way on.
    pub label: Option<String>,
    pub bmin: [f32; 2],
    pub bmax: [f32; 2],
}

/// A comment frame as drawn.
#[derive(Clone, Debug)]
pub struct SceneFrame {
    pub index: usize,
    pub title: String,
    pub tone: u8,
    pub rect: [f32; 4],
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
        pos2(
            self.fitted.min.x + (w[0] - self.world_min[0]) * self.scale,
            self.fitted.min.y + (w[1] - self.world_min[1]) * self.scale,
        )
    }

    /// A box-local point (clamped to the fitted area) in world pt.
    #[must_use]
    pub fn to_world(&self, local: Pos2) -> [f32; 2] {
        if self.scale <= 0.0 {
            return [0.0, 0.0];
        }
        let x = local.x.clamp(self.fitted.min.x, self.fitted.max.x);
        let y = local.y.clamp(self.fitted.min.y, self.fitted.max.y);
        [
            self.world_min[0] + (x - self.fitted.min.x) / self.scale,
            self.world_min[1] + (y - self.fitted.min.y) / self.scale,
        ]
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
    pub flow: Flow,
    pub wires: WireSettings,
    pub nodes: Vec<SceneNode>,
    pub links: Vec<SceneLink>,
    pub frames: Vec<SceneFrame>,
    pub index: FxHashMap<SmolStr, u32>,
    node_cells: FxHashMap<(i32, i32), Vec<u32>>,
    link_cells: FxHashMap<(i32, i32), Vec<u32>>,
    big_links: Vec<u32>,
    /// The node boxes.
    pub bounds: Option<[f32; 4]>,
    /// The node boxes, the routed wires and the frames: what Frame All fits.
    pub extent: Option<[f32; 4]>,
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

/// What `Scene::build` reads.
pub struct SceneInput<'a> {
    pub graph: &'a Graph,
    pub catalog: &'a Catalog,
    pub edit: u64,
    pub palette: &'a Palette,
    /// The wire options in force: the preferences with the graph's own over them.
    pub wires: WireSettings,
    /// In-game names of items, names of characters.
    pub names: Option<&'a dyn Names>,
}

fn action_rows(
    cat: &Catalog,
    n: &gz_nq::Node,
    behaviour: bool,
    rows: usize,
    names: Option<&dyn Names>,
) -> (Vec<ActionRow>, Option<(bool, usize)>) {
    let mut out = Vec::with_capacity(rows);
    let all = n
        .on_enter
        .iter()
        .enumerate()
        .map(|(i, a)| (false, i, a))
        .chain(n.on_exit.iter().enumerate().map(|(i, a)| (true, i, a)));
    let mut more = None;
    for (r, (exit, index, a)) in all.enumerate() {
        if r >= rows {
            more = Some((exit, index));
            break;
        }
        let icon = look::kind_icon(cat, &a.kind);
        out.push(ActionRow {
            exit,
            index,
            kind: a.kind.clone(),
            icon,
            family: look::family_of(icon),
            title: look::kind_title(cat, &a.kind, behaviour),
            summary: text::summary(cat, &a.kind, &a.params, names),
        });
    }
    (out, more)
}

/// The roles of a node's pins: its default way on, its failures, the others by number.
fn pin_infos(cat: &Catalog, n: &gz_nq::Node, behaviour: bool) -> (Vec<PinInfo>, usize) {
    let (names, declared) = layout::pins(cat, n);
    let kind = cat.find(&n.kind);
    // a node of cases has no one way on: every case is named on its wire
    let default = if kind.is_some_and(|k| k.pins_from_cases) {
        None
    } else {
        layout::default_pin(&names).map(str::to_owned)
    };
    let mut other = 0;
    let pins = names
        .into_iter()
        .map(|name| {
            let role = if Some(&name) == default.as_ref() {
                PinRole::Default
            } else if layout::is_failure_pin(&name) {
                PinRole::Failure
            } else {
                other += 1;
                PinRole::Other(other - 1)
            };
            let desc = kind
                .and_then(|k| k.pin_text(&name))
                .map_or_else(String::new, |d| if behaviour { text::regraph(d) } else { d.to_owned() });
            let connected = n.targets(&name).is_some_and(|t| !t.is_empty());
            PinInfo { name, role, desc, connected }
        })
        .collect();
    (pins, declared)
}

impl Scene {
    /// Builds the scene of a graph. Pure: CPU pool.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn build(inp: &SceneInput<'_>) -> Scene {
        let g = inp.graph;
        let cat = inp.catalog;
        let behaviour = g.is_behaviour();
        let cx = BoxCx::of(cat, g);
        let reach = g.reachable(cat, None);
        let mut nodes: Vec<SceneNode> = Vec::with_capacity(g.nodes.len());
        let mut index: FxHashMap<SmolStr, u32> = FxHashMap::default();
        index.reserve(g.nodes.len());
        for (i, n) in g.nodes.iter().enumerate() {
            let b = layout::node_box(&cx, n);
            let mut lines = layout::lines(cat, n, behaviour);
            // what a line names is said with names where they are known
            if let Some(k) = cat.find(&n.kind) {
                for line in lines.iter_mut().filter(|l| l.cond.is_empty() && !l.key.is_empty()) {
                    if let Some(p) = k.params.iter().find(|p| text::pretty_lower(&p.name) == line.key)
                        && let Some(v) = n.params.get(&p.name)
                    {
                        line.value = text::param_text(p, v, inp.names);
                    }
                }
            }
            let (pins, declared) = pin_infos(cat, n, behaviour);
            let trigger = cat.is_trigger(&n.kind);
            let icon = look::kind_icon(cat, &n.kind);
            let kind = cat.find(&n.kind);
            let once = !trigger && if n.once_set { n.once } else { kind.is_some_and(|k| k.once_default) };
            let (actions, more_target) = if trigger || n.collapsed {
                (Vec::new(), None)
            } else {
                action_rows(cat, n, behaviour, b.action_rows, inp.names)
            };
            let summary = lines.first().map_or_else(String::new, |l| {
                format!(
                    "{}{}{}",
                    l.key,
                    if l.cond.is_empty() { ": " } else { " " },
                    text::clip_utf8(&l.value, 64)
                )
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
                collapsed: n.collapsed,
                disabled: n.disabled,
                comment: n.comment.lines().next().unwrap_or("").to_owned(),
                lines,
                actions,
                more_target,
                pins,
                declared,
                reachable: reach.get(i).copied().unwrap_or(true),
                summary,
                takes_input,
                desc: look::kind_desc(cat, &n.kind, behaviour),
            });
        }
        let boxes: Vec<[f32; 4]> = nodes.iter().map(SceneNode::rect).collect();

        // the wires, through the one builder: what is drawn is what is hit
        let wcx = wires::WireCx { flow: cx.flow, style: inp.wires.style, diagonal: inp.wires.diagonal };
        let mut links: Vec<SceneLink> = Vec::new();
        let mut lanes = Vec::new();
        for (i, n) in g.nodes.iter().enumerate() {
            let Some(sn) = nodes.get(i) else { continue };
            let count: usize = n.out.iter().map(|p| p.targets.len()).sum();
            let mut k = 0;
            for (pi, pin) in sn.pins.iter().enumerate() {
                let Some(p) = n.out.iter().find(|p| p.name.as_str() == pin.name) else { continue };
                for (ti, t) in p.targets.iter().enumerate() {
                    let Some(&to) = index.get(t) else { continue };
                    let Some(tn) = nodes.get(to as usize) else { continue };
                    let p0 = layout::output_pin(cx.flow, sn.pos, sn.size, pi, sn.pins.len());
                    let p3 = layout::input_pin(cx.flow, tn.pos, tn.size);
                    let via =
                        g.canvas.route(&n.id, &pin.name, t).map(<[[f32; 2]]>::to_vec).unwrap_or_default();
                    let lanes_of = wires::Lanes { index: k, count, track: ti, tracks: p.targets.len() };
                    let w = wires::build(&wcx, p0, p3, &via, lanes_of, &boxes, &mut lanes);
                    k += 1;
                    let mut bmin = p0;
                    let mut bmax = p0;
                    for q in &w.points {
                        bmin = [bmin[0].min(q[0]), bmin[1].min(q[1])];
                        bmax = [bmax[0].max(q[0]), bmax[1].max(q[1])];
                    }
                    links.push(SceneLink {
                        from: i as u32,
                        to,
                        pin: p.name.clone(),
                        pin_index: pi,
                        role: pin.role,
                        points: w.points,
                        back: w.back,
                        via,
                        label: (pin.role != PinRole::Default && !sn.trigger).then(|| pin.name.clone()),
                        bmin,
                        bmax,
                    });
                }
            }
        }
        let frames: Vec<SceneFrame> = g
            .canvas
            .frames
            .iter()
            .enumerate()
            .map(|(index, f)| SceneFrame { index, title: f.title.clone(), tone: f.tone, rect: f.rect() })
            .collect();

        // what Frame All fits
        let mut bounds: Option<[f32; 4]> = None;
        for b in &boxes {
            grow(&mut bounds, [b[0], b[1]]);
            grow(&mut bounds, [b[2], b[3]]);
        }
        let mut extent = bounds;
        for l in links.iter().filter(|l| l.back || !l.via.is_empty()) {
            grow(&mut extent, l.bmin);
            grow(&mut extent, l.bmax);
        }
        for f in &frames {
            grow(&mut extent, [f.rect[0], f.rect[1]]);
            grow(&mut extent, [f.rect[2], f.rect[3]]);
        }

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
            let (x0, y0, x1, y1) =
                (cell_of(l.bmin[0]), cell_of(l.bmin[1]), cell_of(l.bmax[0]), cell_of(l.bmax[1]));
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
            v.sort_by(|a, b| {
                a.0.total_cmp(&b.0).then(if ties_low_first { a.1.cmp(&b.1) } else { b.1.cmp(&a.1) })
            });
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
            flow: cx.flow,
            wires: inp.wires,
            nodes,
            links,
            frames,
            index,
            node_cells,
            link_cells,
            big_links,
            bounds,
            extent,
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
        if (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1)
            > self.node_cells.len() as i64
        {
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
        if (i64::from(x1) - i64::from(x0) + 1) * (i64::from(y1) - i64::from(y0) + 1)
            > self.link_cells.len() as i64
        {
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
    /// of each: the one the least pan brings in.
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

/// The minimap's picture in box-local pt: the wires as their polylines, the nodes in their family
/// colours. Built with the scene, so a frame only translates a copy of it.
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
    let edge = Palette::alpha(p.wire_base, 0.5);
    let mut mesh = Mesh::default();
    for l in links {
        // a few points of the polyline are enough at this size
        let step = (l.points.len() / 6).max(1);
        let pts: Vec<Pos2> =
            l.points.iter().step_by(step).chain(l.points.last()).map(|q| m.to_local(*q)).collect();
        for w in pts.windows(2) {
            mesh_line(&mut mesh, w[0], w[1], 1.0, edge);
        }
    }
    let dot = 2.0;
    for n in nodes {
        let a = m.to_local(n.pos);
        let z = m.to_local([n.pos[0] + n.size[0], n.pos[1] + n.size[1]]);
        let r = Rect::from_min_max(a, pos2(z.x.max(a.x + dot), z.y.max(a.y + dot)));
        mesh.add_colored_rect(r, look::family_color(p, n.family, if n.disabled { 0.35 } else { 0.85 }));
    }
    m.mesh = mesh;
    m
}

/// Where a Frame All lands: the zoom level and the centre.
#[derive(Clone, Debug, PartialEq)]
pub struct Framed {
    pub zoom: i32,
    pub center: [f32; 2],
}

/// Frame All: the highest level up to 1:1 that holds the whole extent, whatever scale that
/// takes. `view` is the canvas in pt.
#[must_use]
pub fn frame_all(s: &Scene, view: [f32; 2]) -> Framed {
    let Some(e) = s.extent.filter(|_| !s.nodes.is_empty()) else {
        return Framed { zoom: gz_nq::doc::ZOOM_DEFAULT, center: [0.0, 0.0] };
    };
    let r = [e[0] - FRAME_MARGIN, e[1] - FRAME_MARGIN, e[2] + FRAME_MARGIN, e[3] + FRAME_MARGIN];
    let (z, c) = fit_rect(r, view, gz_nq::doc::ZOOM_DEFAULT);
    Framed { zoom: z, center: c }
}

fn fit_rect(r: [f32; 4], view: [f32; 2], top: i32) -> (i32, [f32; 2]) {
    let (w, h) = ((r[2] - r[0]).max(1.0), (r[3] - r[1]).max(1.0));
    let fit = if view[0] > 0.0 && view[1] > 0.0 { (view[0] / w).min(view[1] / h) } else { 1.0 };
    let z = gz_nq::doc::level_at_most(fit).min(top);
    (z, [layout::sane((r[0] + r[2]) * 0.5), layout::sane((r[1] + r[3]) * 0.5)])
}

/// Frames a world rectangle: the highest level that holds it with a gap around, never closer
/// than 1:1.
#[must_use]
pub fn frame_rect(r: [f32; 4], view: [f32; 2]) -> (i32, [f32; 2]) {
    let pad = 64.0;
    fit_rect([r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad], view, gz_nq::doc::ZOOM_DEFAULT)
}

/// A scene shared by the frames that draw it.
pub type SharedScene = Arc<Scene>;

#[cfg(test)]
mod tests {
    use super::*;
    use gz_nq::Node;

    fn graph() -> (Graph, Catalog) {
        let mut g = Graph::default();
        g.canvas.flow = Some(Flow::Down);
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

    fn scene(g: &Graph, cat: &Catalog) -> Scene {
        let p = Palette::new(gz_ui::Accent::default());
        Scene::build(&SceneInput {
            graph: g,
            catalog: cat,
            edit: 1,
            palette: &p,
            wires: WireSettings::default(),
            names: None,
        })
    }

    #[test]
    fn builds_links_and_routes() {
        let (g, cat) = graph();
        let s = scene(&g, &cat);
        assert_eq!(s.nodes.len(), 3);
        assert_eq!(s.links.len(), 3);
        assert_eq!(s.links.iter().filter(|l| l.back).count(), 1);
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
        let s = scene(&g, &cat);
        let (c, near) = s.beyond([-100.0, 150.0, 400.0, 350.0]);
        assert_eq!(c[0], 1);
        assert_eq!(c[1], 1);
        assert_eq!(near[0], Some(0));
        assert_eq!(near[1], Some(2));
    }

    #[test]
    fn frame_all_takes_whatever_scale_it_needs() {
        let (g, cat) = graph();
        let s = scene(&g, &cat);
        let f = frame_all(&s, [1600.0, 1000.0]);
        assert_eq!(f.zoom, gz_nq::doc::ZOOM_DEFAULT);
        // a small view goes as far out as it takes: below the old floor of 75%
        let small = frame_all(&s, [120.0, 90.0]);
        assert!(gz_nq::doc::zoom_of(small.zoom) < 0.3, "{}", gz_nq::doc::zoom_of(small.zoom));
    }
}

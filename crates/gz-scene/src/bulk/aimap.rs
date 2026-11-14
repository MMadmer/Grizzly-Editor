//! The AI map's work: nodes made on
//! the collision model and linked, the flood that generates the map from the nodes there are,
//! the brush, the link edits, smoothing, selection and picking.
//!
//! An edit works on [`AiWork`], a flat copy of the nodes with the tool's hash (`m_HASH`: 129 x
//! 129 buckets over the map's box); [`AiWork::into_part`] makes the scene's tiles again, sharing
//! every tile the edit left as it was.
//!
//! The generation keeps one order - a queue of nodes in the order they were made, each
//! trying its four sides - and the steps that decide the outcome (finding a node there already,
//! registering a new one) run in that order. What a side costs (the slide of the body, the 81
//! rays of a new node) depends only on the collision model, so it is worked out ahead for a
//! batch of the queue on every core and then taken in the queue's order, so the map is the same
//! on any number of cores.

use std::sync::Arc;

use glam::{Vec3, Vec4};
use gz_fmt_level::{
    AiMapNode, AiParams, INVALID_NODE, PackedPos, SceneAiMap, SceneAiNode, node_flags as nf, pv_compress,
};
use rayon::prelude::*;

use super::collide::CollideModel;
use super::motion;
use super::xr::{self, EPS_L, M43, Plane, deg2rad, dist_sqr, dot, fsimilar, i_floor, similar, snapto};
use crate::math::Aabb;
use crate::tools::AiMapPart;

/// `HDIM_X` / `HDIM_Z`.
pub const HDIM: u32 = 128;
const RCAST_COUNT: i32 = 4;
const RCAST_TOTAL: usize = ((2 * RCAST_COUNT + 1) * (2 * RCAST_COUNT + 1)) as usize;
const RCAST_DEPTH: f32 = 1.0;
const RCAST_VALID: f32 = 0.55;
/// The flags of the four sides: a side done in this pass.
pub const SIDE_FLAGS: [u8; 4] = [nf::N1, nf::N2, nf::N3, nf::N4];
const OPPOSITE: [usize; 4] = [2, 3, 0, 1];
/// How many queued nodes are worked out ahead at once.
const BATCH: usize = 512;

/// The layer's flags, as the AI map's chunk stores them.
pub mod flags {
    pub const HIDE_NODES: u32 = 1 << 1;
    pub const SLOW_CALCULATE: u32 = 1 << 2;
}

/// What a side button does to the links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkMode {
    #[default]
    Add,
    Remove,
    Invert,
}

impl LinkMode {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "add" | "append" => Self::Add,
            "remove" => Self::Remove,
            "invert" => Self::Invert,
            _ => return None,
        })
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Invert => "invert",
        }
    }
}

/// Snaps a position to the node grid on the XZ plane.
fn snap_xz(mut v: Vec3, ps: f32) -> Vec3 {
    v.x = snapto(v.x, ps);
    v.z = snapto(v.z, ps);
    v
}

/// The point straight under or over a node's corner on its plane; the
/// corner itself when the plane stands upright.
fn corner(n: &AiMapNode, dx: f32, dz: f32, ps: f32) -> Vec3 {
    let s = ps / 2.0;
    let v = Vec3::new(n.pos.x + dx * s, n.pos.y, n.pos.z + dz * s);
    plane_of(n).intersect_ray_point(v, Vec3::NEG_Y).map_or(v, |(p, _)| p)
}

fn plane_of(n: &AiMapNode) -> Plane {
    Plane { n: n.normal, d: n.d }
}

fn node_on(plane: Plane, pos: Vec3) -> AiMapNode {
    AiMapNode { links: [None; 4], normal: plane.n, d: plane.d, pos, flags: 0 }
}

/// `m_HASH`: the nodes by bucket of the map's box.
#[derive(Clone, Debug)]
pub struct Hash {
    min: Vec3,
    scale_x: f32,
    scale_z: f32,
    cells: Vec<Vec<u32>>,
}

impl Hash {
    #[must_use]
    pub fn new(bbox: &Aabb) -> Self {
        let size = bbox.max - bbox.min;
        Self {
            min: bbox.min,
            scale_x: HDIM as f32 / size.x,
            scale_z: HDIM as f32 / size.z,
            cells: vec![Vec::new(); ((HDIM + 1) * (HDIM + 1)) as usize],
        }
    }

    /// `HashMap(V)`: the bucket of a point, `None` outside the box.
    #[must_use]
    pub fn cell(&self, v: Vec3) -> Option<usize> {
        let ix = i_floor((v.x - self.min.x) * self.scale_x) as u32;
        let iz = i_floor((v.z - self.min.z) * self.scale_z) as u32;
        (ix <= HDIM && iz <= HDIM).then(|| (ix * (HDIM + 1) + iz) as usize)
    }

    fn bucket(&self, v: Vec3) -> Option<&[u32]> {
        self.cell(v).and_then(|c| self.cells.get(c)).map(Vec::as_slice)
    }

    fn push(&mut self, v: Vec3, i: u32) -> bool {
        match self.cell(v).and_then(|c| self.cells.get_mut(c)) {
            Some(b) => {
                b.push(i);
                true
            }
            None => false,
        }
    }
}

/// What a generation looks at: the collision model, the parameters, the ignored materials.
pub struct Gen<'a> {
    pub model: &'a CollideModel,
    pub params: AiParams,
    /// Per material of the model: generated over (`false`) or not.
    pub ignored: Vec<bool>,
}

impl Gen<'_> {
    fn nearest_down(&self, p: Vec3, tris: &[(usize, Vec3)]) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        let mut min = f32::MAX;
        for (k, (i, _)) in tris.iter().enumerate() {
            let Some(t) = self.model.tri(*i as u32) else { continue };
            if let Some(range) = xr::ray_tri(p, Vec3::NEG_Y, t, false)
                && range < min
            {
                min = range;
                best = Some((k, range));
            }
        }
        best
    }

    /// A node on the ground at `v_at`, or `None` (a chasm, too steep, too far up or
    /// down, floating).
    #[must_use]
    pub fn create_node(&self, v_at: Vec3, bic: bool) -> Option<AiMapNode> {
        let ps = self.params.patch_size;
        let point_up = snap_xz(Vec3::new(v_at.x, v_at.y + RCAST_DEPTH, v_at.z), ps);
        let point_down = snap_xz(Vec3::new(v_at.x, v_at.y - RCAST_DEPTH, v_at.z), ps);
        let mut bb = Aabb::new(point_up, point_up);
        bb.grow(ps / 2.0);
        let mut b2 = Aabb::new(point_down, point_down);
        b2.grow(ps / 2.0);
        bb.merge(&b2);
        let found = self.model.box_query(&bb, true);
        if found.is_empty() {
            return None;
        }
        // an ignored material is left out of the node's ground, which leaves a hole in the map
        let tris: Vec<(usize, Vec3)> = found
            .iter()
            .filter(|&&i| {
                let m = self.model.material.get(i as usize).copied().unwrap_or(0);
                !self.ignored.get(usize::from(m)).copied().unwrap_or(false)
            })
            .filter_map(|&i| {
                let t = self.model.tri(i)?;
                let n = xr::mknormal(t[0], t[1], t[2]);
                (n.y > 0.0).then_some((i as usize, n))
            })
            .collect();
        if tris.is_empty() {
            return None;
        }
        let coeff = 0.5 * ps / RCAST_COUNT as f32;
        let mut points: Vec<Vec3> = Vec::with_capacity(RCAST_TOTAL);
        let mut normals: Vec<Vec3> = Vec::with_capacity(RCAST_TOTAL);
        for x in -RCAST_COUNT..=RCAST_COUNT {
            let px = v_at.x + coeff * x as f32;
            for z in -RCAST_COUNT..=RCAST_COUNT {
                let mut p = Vec3::new(px, v_at.y + 10.0, v_at.z + coeff * z as f32);
                if let Some((k, range)) = self.nearest_down(p, &tris) {
                    p.y -= range;
                    points.push(p);
                    normals.push(tris[k].1);
                }
            }
        }
        if points.len() < 3 {
            return None;
        }
        let rc_lim = if bic { 0.015 } else { 0.7 };
        if (points.len() as f32) / (RCAST_TOTAL as f32) < rc_lim {
            return None;
        }
        let mut v_norm = Vec3::ZERO;
        for n in &normals {
            v_norm += *n;
        }
        let c = normals.len() as f32;
        v_norm = xr::normalize(Vec3::new(v_norm.x / c, v_norm.y / c, v_norm.z / c));
        let mut v_offs = Vec3::new(0.0, -1000.0, 0.0);
        let mut pl = Plane::build(v_offs, v_norm);
        for p in &points {
            if pl.classify(*p) > 0.0 {
                v_offs = *p;
                pl = Plane::build(v_offs, v_norm);
            }
        }
        let plane = Plane::build(v_offs, v_norm);
        let (pos, _) = plane.intersect_ray_point(point_down, Vec3::Y)?;
        if dot(Vec3::Y, plane.n) < deg2rad(60.0).cos() {
            return None;
        }
        let (y_old, y_new) = (v_at.y, pos.y);
        if y_old > y_new {
            if y_old - y_new > self.params.can_down {
                return None;
            }
        } else if y_new - y_old > self.params.can_up {
            return None;
        }
        // the plane has to lie on the ground, not float over it
        let mut ok = 0usize;
        for x in -RCAST_COUNT..=RCAST_COUNT {
            for z in -RCAST_COUNT..=RCAST_COUNT {
                let mut p = Vec3::new(pos.x + coeff * x as f32, pos.y, pos.z + coeff * z as f32);
                let Some((plp, _)) = plane.intersect_ray_point(p, Vec3::NEG_Y) else { continue };
                p.y = plp.y + RCAST_VALID * 0.01;
                if let Some((_, range)) = self.nearest_down(p, &tris)
                    && range < RCAST_VALID
                {
                    ok += 1;
                }
            }
        }
        let perc = ok as f32 / RCAST_TOTAL as f32;
        let perc_lim = if bic { 0.015 } else { 0.5 };
        if perc < perc_lim {
            return None;
        }
        Some(node_on(plane, pos))
    }

    /// The body slid at two heights reaches the point.
    #[must_use]
    pub fn can_travel(&self, from: Vec3, mut at: Vec3) -> bool {
        at.y = from.y;
        let eps = 0.1;
        let eps_y = self.params.patch_size * 1.5;
        let radius = self.params.patch_size / 2f32.sqrt();
        for height in [0.7, 2.0] {
            let r = motion::simulate(self.model, from, at, radius, height);
            if fsimilar(r.x, at.x, eps) && fsimilar(r.z, at.z, eps) && fsimilar(r.y, at.y, eps_y) {
                return true;
            }
        }
        false
    }

    /// The part of a node's build that depends on nothing but the model: whether the side can be
    /// walked to and the node there.
    fn spec(&self, from: Vec3, at: Vec3, bic: bool) -> Option<AiMapNode> {
        let at = snap_xz(at, self.params.patch_size);
        if !(bic || self.can_travel(from, at)) {
            return None;
        }
        self.create_node(at, bic)
    }
}

/// A flat copy of the tool's nodes to edit, with the hash.
#[derive(Clone, Debug)]
pub struct AiWork {
    /// Everything but the nodes.
    pub header: SceneAiMap,
    pub nodes: Vec<AiMapNode>,
    pub hash: Hash,
}

/// How far a generation got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flood {
    pub added: usize,
    pub stopped: bool,
}

impl AiWork {
    #[must_use]
    pub fn new(part: Option<&AiMapPart>) -> Self {
        let mut header = part.map_or_else(SceneAiMap::default, |p| p.header.clone());
        header.nodes = Arc::from([]);
        let nodes = part.map(AiMapPart::all_nodes).unwrap_or_default();
        let mut w = Self { hash: Hash::new(&header.bbox), header, nodes };
        w.rehash();
        w
    }

    /// `hash_Clear` + `hash_FillFromNodes` over the map's box.
    pub fn rehash(&mut self) {
        self.hash = Hash::new(&self.header.bbox);
        for (i, n) in self.nodes.iter().enumerate() {
            self.hash.push(n.pos, i as u32);
        }
    }

    #[must_use]
    pub fn ps(&self) -> f32 {
        self.header.params.patch_size
    }

    /// The bounding box of the nodes.
    #[must_use]
    pub fn nodes_bbox(&self) -> Aabb {
        let mut b = Aabb::INVALID;
        for n in &self.nodes {
            b.modify(n.pos);
        }
        b
    }

    /// The box is the snap objects' and the nodes'. `snap_box` is `None`
    /// for an empty snap list; answers whether the list has objects.
    pub fn update_box(&mut self, snap_box: Option<Aabb>) -> bool {
        let nodes_bb = self.nodes_bbox();
        match snap_box {
            Some(snap_bb) => {
                let bb = if nodes_bb.is_valid() {
                    Aabb::new(snap_bb.min.min(nodes_bb.min), snap_bb.max.max(nodes_bb.max))
                } else {
                    snap_bb
                };
                let old = self.header.bbox;
                if !(similar(old.min, bb.min, EPS_L) && similar(old.max, bb.max, EPS_L)) {
                    self.header.bbox = bb;
                    self.rehash();
                }
                true
            }
            None => {
                self.header.bbox = nodes_bb;
                self.rehash();
                false
            }
        }
    }

    /// `FindNode(vAt, eps)`: the first node of the point's bucket that close.
    #[must_use]
    pub fn find_node(&self, at: Vec3, eps: f32) -> Option<u32> {
        self.hash
            .bucket(at)?
            .iter()
            .copied()
            .find(|&i| self.nodes.get(i as usize).is_some_and(|n| similar(at, n.pos, eps)))
    }

    /// The order-dependent part of a node's build: a node there already is returned, a new one
    /// registered when its bucket exists.
    fn place(&mut self, n: AiMapNode) -> Option<u32> {
        if let Some(old) = self.find_node(n.pos, 0.05) {
            return Some(old);
        }
        let i = self.nodes.len() as u32;
        if self.hash.push(n.pos, i) {
            self.nodes.push(n);
            Some(i)
        } else {
            None
        }
    }

    /// `BuildNode(vFrom, vAt, bIC, bSuperIC)`.
    pub fn build_node(
        &mut self,
        g: &Gen<'_>,
        from: Vec3,
        at: Vec3,
        bic: bool,
        super_ic: bool,
    ) -> Option<u32> {
        let at = snap_xz(at, self.ps());
        if !(bic || g.can_travel(from, at)) {
            return None;
        }
        let n = g.create_node(at, bic).or_else(|| {
            (bic && super_ic).then(|| {
                let plane = Plane::build(at, Vec3::Y);
                node_on(plane, plane.intersect_ray_point(at, Vec3::Y).map_or(at, |(p, _)| p))
            })
        })?;
        self.place(n)
    }

    fn side_pos(&self, i: usize, side: usize) -> Vec3 {
        let ps = self.ps();
        let mut p = self.nodes.get(i).map_or(Vec3::ZERO, |n| n.pos);
        match side {
            0 => p.x -= ps,
            1 => p.z += ps,
            2 => p.x += ps,
            _ => p.z -= ps,
        }
        p
    }

    fn selected(&self, i: usize) -> bool {
        self.nodes.get(i).is_some_and(|n| n.flags & nf::SELECTED != 0)
    }

    /// The queue of the node build: from node `0`, every node tries the sides it has no link on;
    /// `within` limits the sides (the brush's circle), `selected_only` the nodes (and selects
    /// the neighbours each one reaches). `progress(done, total)` answers `false` to stop.
    fn flood(
        &mut self,
        g: &Gen<'_>,
        bic: bool,
        selected_only: bool,
        within: Option<(Vec3, f32)>,
        progress: &(dyn Fn(usize, usize) -> bool + Sync),
    ) -> Flood {
        let before = self.nodes.len();
        let mut k = 0usize;
        let mut stopped = false;
        let reach = |p: Vec3| within.is_none_or(|(c, r)| dist_sqr(p, c).sqrt() <= r);
        while k < self.nodes.len() {
            let end = (k + BATCH).min(self.nodes.len());
            // worked out ahead: the sides of the batch's nodes that are open now
            let jobs: Vec<(usize, usize, Vec3, Vec3)> = (k..end)
                .filter(|&i| !selected_only || self.selected(i))
                .flat_map(|i| (0..4).map(move |s| (i, s)))
                .filter(|&(i, s)| self.nodes.get(i).is_some_and(|n| n.links[s].is_none()))
                .filter_map(|(i, s)| {
                    let to = self.side_pos(i, s);
                    let from = self.nodes.get(i)?.pos;
                    reach(to).then_some((i, s, from, to))
                })
                .collect();
            let specs: Vec<Option<AiMapNode>> =
                jobs.par_iter().map(|&(_, _, from, to)| g.spec(from, to, bic)).collect();
            let mut ahead: Vec<Option<usize>> = vec![None; (end - k) * 4];
            for (j, &(i, s, _, _)) in jobs.iter().enumerate() {
                if let Some(slot) = ahead.get_mut((i - k) * 4 + s) {
                    *slot = Some(j);
                }
            }
            for i in k..end {
                if selected_only && !self.selected(i) {
                    continue;
                }
                for s in 0..4 {
                    if self.nodes.get(i).is_none_or(|n| n.links[s].is_some()) {
                        continue;
                    }
                    let to = self.side_pos(i, s);
                    if !reach(to) {
                        continue;
                    }
                    let spec = match ahead.get((i - k) * 4 + s).copied().flatten() {
                        Some(j) => specs.get(j).copied().flatten(),
                        // a node selected by one of this batch: worked out now
                        None => {
                            let from = self.nodes.get(i).map_or(Vec3::ZERO, |n| n.pos);
                            g.spec(from, to, bic)
                        }
                    };
                    let link = spec.and_then(|n| self.place(n));
                    if let Some(n) = self.nodes.get_mut(i) {
                        n.links[s] = link;
                    }
                }
                if selected_only {
                    let links = self.nodes.get(i).map(|n| n.links).unwrap_or_default();
                    for l in links.into_iter().flatten() {
                        if let Some(n) = self.nodes.get_mut(l as usize) {
                            n.flags |= nf::SELECTED;
                        }
                    }
                }
            }
            k = end;
            if !progress(k, self.nodes.len()) {
                stopped = true;
                break;
            }
        }
        Flood { added: self.nodes.len() - before, stopped }
    }

    /// `BuildNodes(bFromSelectedOnly)`: the generation over the map's box.
    pub fn generate(
        &mut self,
        g: &Gen<'_>,
        selected_only: bool,
        progress: &(dyn Fn(usize, usize) -> bool + Sync),
    ) -> Flood {
        self.flood(g, false, selected_only, None, progress)
    }

    /// `BuildNodes(pos, sz, bIC)`: the brush - nodes from the ground under `pos` out to `sz`
    /// patches. `Err` when the ground cannot be found.
    pub fn brush(&mut self, g: &Gen<'_>, pos: Vec3, sz: u32, bic: bool) -> Result<usize, &'static str> {
        let mut p = snap_xz(pos, self.ps());
        p.y += 1.0;
        let Some((_, range)) = g.model.ray_nearest(p, Vec3::NEG_Y, 3.0) else {
            return Err("Can't align position.");
        };
        p.y -= range;
        let before = self.nodes.len();
        let Some(start) = self.build_node(g, p, p, bic, false) else { return Ok(0) };
        let centre = self.nodes.get(start as usize).map_or(p, |n| n.pos);
        let radius = sz as f32 * self.ps() - EPS_L;
        self.flood(g, bic, false, Some((centre, radius)), &|_, _| true);
        Ok(self.nodes.len() - before)
    }

    /// `AddNode(pos, bIC, bAutoLink, sz)`: one node, selected and linked when asked, or the
    /// brush for a size over one.
    pub fn add_node(
        &mut self,
        g: &Gen<'_>,
        pos: Vec3,
        bic: bool,
        auto_link: bool,
        sz: u32,
    ) -> Result<usize, &'static str> {
        if sz <= 1 {
            let Some(n) = self.build_node(g, pos, pos, bic, true) else { return Err("Can't create node.") };
            if let Some(node) = self.nodes.get_mut(n as usize) {
                node.flags |= nf::SELECTED;
            }
            if auto_link {
                self.update_links(g, n, bic);
            }
            Ok(1)
        } else {
            self.brush(g, pos, sz, bic)
        }
    }

    /// The node there is where a node would be made.
    fn get_node(&self, g: &Gen<'_>, at: Vec3, bic: bool) -> Option<u32> {
        let at = snap_xz(at, self.ps());
        let n = g.create_node(at, bic)?;
        self.find_node(n.pos, 0.05)
    }

    /// A node and its neighbours linked both ways where they can be walked.
    pub fn update_links(&mut self, g: &Gen<'_>, n: u32, bic: bool) {
        let ps = self.ps();
        let Some(pos) = self.nodes.get(n as usize).map(|x| x.pos) else { return };
        for (side, (dx, dz)) in [(-ps, 0.0), (0.0, ps), (ps, 0.0), (0.0, -ps)].into_iter().enumerate() {
            let at = Vec3::new(pos.x + dx, pos.y, pos.z + dz);
            let d = self.get_node(g, at, bic);
            if (bic || g.can_travel(pos, at))
                && let Some(node) = self.nodes.get_mut(n as usize)
            {
                node.links[side] = d;
            }
            if let Some(d) = d {
                let dpos = self.nodes.get(d as usize).map_or(at, |x| x.pos);
                if (bic || g.can_travel(dpos, pos))
                    && let Some(node) = self.nodes.get_mut(d as usize)
                {
                    node.links[OPPOSITE[side]] = Some(n);
                }
            }
        }
    }

    /// The node on a side - the one nearest in height with constraints off, the
    /// nearer of the nearest up and down within the limits otherwise.
    #[must_use]
    pub fn find_neighbor(&self, n: usize, side: usize, bic: bool) -> Option<u32> {
        let ps = self.ps();
        let mut p = snap_xz(self.nodes.get(n)?.pos, ps);
        match side {
            0 => p.x -= ps,
            1 => p.z += ps,
            2 => p.x += ps,
            _ => p.z -= ps,
        }
        let bucket = self.hash.bucket(p)?;
        let at = |i: u32| self.nodes.get(i as usize).map(|x| x.pos);
        let on_column = |q: Vec3| fsimilar(q.x, p.x, EPS_L) && fsimilar(q.z, p.z, EPS_L);
        if bic {
            let mut dy = f32::MAX;
            let mut r = None;
            for &i in bucket {
                let Some(q) = at(i).filter(|q| on_column(*q)) else { continue };
                let d = (q.y - p.y).abs();
                if d < dy {
                    dy = d;
                    r = Some(i);
                }
            }
            r
        } else {
            let (mut r_up, mut r_down) = (None, None);
            let (mut dy_up, mut dy_down) = (f32::MAX, f32::MAX);
            for &i in bucket {
                let Some(q) = at(i).filter(|q| on_column(*q)) else { continue };
                let d = q.y - p.y;
                let ad = d.abs();
                if d >= 0.0 {
                    if ad < self.header.params.can_up && ad < dy_up {
                        dy_up = ad;
                        r_up = Some(i);
                    }
                } else if ad < self.header.params.can_down && ad < dy_down {
                    dy_down = ad;
                    r_down = Some(i);
                }
            }
            if dy_down <= dy_up { r_down } else { r_up }
        }
    }

    fn clear_side_flags(&mut self) {
        for n in &mut self.nodes {
            n.flags &= !(nf::N1 | nf::N2 | nf::N3 | nf::N4);
        }
    }

    /// Links the selected nodes on the sides `side_flag` names (the four sides). Answers how many links changed.
    pub fn make_links(&mut self, side_flag: u8, mode: LinkMode, bic: bool) -> usize {
        if side_flag == 0 {
            return 0;
        }
        let mut changed = 0;
        for t in 0..self.nodes.len() {
            if !self.selected(t) {
                continue;
            }
            for k in 0..4 {
                if side_flag & SIDE_FLAGS[k] == 0 {
                    continue;
                }
                let Some(s) = self.find_neighbor(t, k, bic) else { continue };
                let su = s as usize;
                match mode {
                    LinkMode::Add => {
                        if self.selected(su)
                            && let Some(n) = self.nodes.get_mut(t)
                            && n.links[k] != Some(s)
                        {
                            n.links[k] = Some(s);
                            changed += 1;
                        }
                    }
                    LinkMode::Remove => {
                        if self.selected(su)
                            && let Some(n) = self.nodes.get_mut(t)
                            && n.links[k].is_some()
                        {
                            n.links[k] = None;
                            changed += 1;
                        }
                    }
                    LinkMode::Invert => {
                        let t_flags = self.nodes.get(t).map_or(0, |n| n.flags);
                        if t_flags & SIDE_FLAGS[k] == 0 {
                            let tk = self.nodes.get(t).and_then(|n| n.links[k]);
                            let so = self.nodes.get(su).and_then(|n| n.links[OPPOSITE[k]]);
                            if tk.is_some() && so.is_some() {
                                continue;
                            }
                            // the one direction there is turns round (swapping the two link
                            // slots would link a node to itself)
                            if tk.is_some() {
                                if let Some(n) = self.nodes.get_mut(t) {
                                    n.links[k] = None;
                                }
                                if let Some(n) = self.nodes.get_mut(su) {
                                    n.links[OPPOSITE[k]] = Some(t as u32);
                                }
                                changed += 1;
                            } else if so.is_some() {
                                if let Some(n) = self.nodes.get_mut(su) {
                                    n.links[OPPOSITE[k]] = None;
                                }
                                if let Some(n) = self.nodes.get_mut(t) {
                                    n.links[k] = Some(s);
                                }
                                changed += 1;
                            }
                        }
                        if let Some(n) = self.nodes.get_mut(su) {
                            n.flags |= SIDE_FLAGS[OPPOSITE[k]];
                        }
                    }
                }
            }
        }
        self.clear_side_flags();
        changed
    }

    /// The links between selected nodes go.
    pub fn remove_links(&mut self) -> usize {
        let sel: Vec<bool> = (0..self.nodes.len()).map(|i| self.selected(i)).collect();
        let mut n = 0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !sel[i] {
                continue;
            }
            for l in &mut node.links {
                if l.is_some_and(|t| sel.get(t as usize).copied().unwrap_or(false)) {
                    *l = None;
                    n += 1;
                }
            }
        }
        n
    }

    /// `InvertLinks`: a one-way link between selected nodes turns round.
    pub fn invert_links(&mut self) -> usize {
        let mut n = 0;
        for t in 0..self.nodes.len() {
            if !self.selected(t) {
                continue;
            }
            for k in 0..4 {
                let t_flags = self.nodes.get(t).map_or(0, |x| x.flags);
                let Some(s) = self.nodes.get(t).and_then(|x| x.links[k]) else { continue };
                let su = s as usize;
                if !self.selected(su) || t_flags & SIDE_FLAGS[k] != 0 {
                    continue;
                }
                if self.nodes.get(su).is_some_and(|x| x.links[OPPOSITE[k]].is_none()) {
                    if let Some(x) = self.nodes.get_mut(su) {
                        x.links[OPPOSITE[k]] = Some(t as u32);
                        x.flags |= SIDE_FLAGS[OPPOSITE[k]];
                    }
                    if let Some(x) = self.nodes.get_mut(t) {
                        x.links[k] = None;
                    }
                    n += 1;
                }
            }
        }
        self.clear_side_flags();
        n
    }

    /// `SelectObjects(flag)`.
    pub fn select_all(&mut self, on: bool) {
        for n in &mut self.nodes {
            if on {
                n.flags |= nf::SELECTED;
            } else {
                n.flags &= !nf::SELECTED;
            }
        }
    }

    /// Inverts the selection.
    pub fn invert_selection(&mut self) {
        for n in &mut self.nodes {
            n.flags ^= nf::SELECTED;
        }
    }

    /// `SelectNodesByLink(link)`: only the nodes with that many links are selected.
    pub fn select_by_links(&mut self, links: usize) -> usize {
        let mut c = 0;
        for n in &mut self.nodes {
            let count = n.links.iter().filter(|l| l.is_some()).count();
            if count == links {
                n.flags |= nf::SELECTED;
                c += 1;
            } else {
                n.flags &= !nf::SELECTED;
            }
        }
        c
    }

    /// Sets the selection of some nodes (a ray pick or a frustum pick, with a flag of 1 or 0).
    pub fn select(&mut self, ids: &[u32], on: bool) -> usize {
        let mut c = 0;
        for &i in ids {
            if let Some(n) = self.nodes.get_mut(i as usize) {
                if on {
                    n.flags |= nf::SELECTED;
                } else {
                    n.flags &= !nf::SELECTED;
                }
                c += 1;
            }
        }
        c
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.flags & nf::SELECTED != 0).count()
    }

    /// `RemoveSelection`: the selected nodes go and every link to them; the rest keep their
    /// order.
    pub fn remove_selected(&mut self) -> usize {
        let sel: Vec<bool> = self.nodes.iter().map(|n| n.flags & nf::SELECTED != 0).collect();
        let gone = sel.iter().filter(|s| **s).count();
        if gone == 0 {
            return 0;
        }
        let mut remap = vec![u32::MAX; self.nodes.len()];
        let mut next = 0u32;
        for (i, s) in sel.iter().enumerate() {
            if !*s {
                remap[i] = next;
                next += 1;
            }
        }
        let old = std::mem::take(&mut self.nodes);
        self.nodes = old
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !sel[*i])
            .map(|(_, mut n)| {
                for l in &mut n.links {
                    *l = l.and_then(|t| remap.get(t as usize).copied().filter(|r| *r != u32::MAX));
                }
                n
            })
            .collect();
        self.rehash();
        gone
    }

    /// `Clear(bOnlyNodes)`.
    pub fn clear_nodes(&mut self) {
        self.nodes.clear();
        self.rehash();
    }

    /// The selected nodes lie level again.
    pub fn reset_selected(&mut self) -> usize {
        let mut c = 0;
        for n in &mut self.nodes {
            if n.flags & nf::SELECTED != 0 {
                let p = Plane::build(n.pos, Vec3::Y);
                n.normal = p.n;
                n.d = p.d;
                c += 1;
            }
        }
        c
    }

    /// Each selected node's plane fitted to the corners it shares with its
    /// neighbours (those within the smooth height).
    pub fn smooth_selected(&mut self) -> usize {
        let ps = self.ps();
        let h = self.header.smooth_height;
        let nodes = &self.nodes;
        let get = |l: Option<u32>| l.and_then(|i| nodes.get(i as usize));
        let smoothed: Vec<Option<(Vec3, f32, Vec3)>> = nodes
            .par_iter()
            .map(|n| {
                if n.flags & nf::SELECTED == 0 {
                    return None;
                }
                let (left, fwd, right, back) =
                    (get(n.links[0]), get(n.links[1]), get(n.links[2]), get(n.links[3]));
                // (corner of N, then the corners of the neighbours meeting it)
                let avg = |rf: Vec3, picks: &[Option<Vec3>]| -> Vec3 {
                    let mut c = 1.0f32;
                    let mut p = rf;
                    for q in picks.iter().flatten() {
                        if fsimilar(q.y, rf.y, h) {
                            c += 1.0;
                            p += *q;
                        }
                    }
                    Vec3::new(p.x / c, p.y / c, p.z / c)
                };
                let lf = corner(n, -1.0, 1.0, ps);
                let p1 = {
                    let mut picks = vec![left.map(|l| corner(l, 1.0, 1.0, ps))];
                    let mut corner_taken = false;
                    if let Some(c) = left.and_then(|l| get(l.links[1])) {
                        corner_taken = true;
                        picks.push(Some(corner(c, 1.0, -1.0, ps)));
                    }
                    picks.push(fwd.map(|f| corner(f, -1.0, -1.0, ps)));
                    if !corner_taken && let Some(c) = fwd.and_then(|f| get(f.links[0])) {
                        picks.push(Some(corner(c, 1.0, -1.0, ps)));
                    }
                    avg(lf, &picks)
                };
                let fr = corner(n, 1.0, 1.0, ps);
                let p2 = {
                    let mut picks = vec![fwd.map(|f| corner(f, 1.0, -1.0, ps))];
                    let mut corner_taken = false;
                    if let Some(c) = fwd.and_then(|f| get(f.links[2])) {
                        corner_taken = true;
                        picks.push(Some(corner(c, -1.0, -1.0, ps)));
                    }
                    picks.push(right.map(|r| corner(r, -1.0, 1.0, ps)));
                    if !corner_taken && let Some(c) = right.and_then(|r| get(r.links[1])) {
                        picks.push(Some(corner(c, -1.0, -1.0, ps)));
                    }
                    avg(fr, &picks)
                };
                let rb = corner(n, 1.0, -1.0, ps);
                let p3 = {
                    let mut picks = vec![right.map(|r| corner(r, -1.0, -1.0, ps))];
                    let mut corner_taken = false;
                    if let Some(c) = right.and_then(|r| get(r.links[3])) {
                        corner_taken = true;
                        picks.push(Some(corner(c, -1.0, 1.0, ps)));
                    }
                    picks.push(back.map(|b| corner(b, 1.0, 1.0, ps)));
                    if !corner_taken && let Some(c) = back.and_then(|b| get(b.links[2])) {
                        picks.push(Some(corner(c, -1.0, 1.0, ps)));
                    }
                    avg(rb, &picks)
                };
                let bl = corner(n, -1.0, -1.0, ps);
                let p4 = {
                    let mut picks = vec![back.map(|b| corner(b, -1.0, 1.0, ps))];
                    let mut corner_taken = false;
                    if let Some(c) = back.and_then(|b| get(b.links[0])) {
                        corner_taken = true;
                        picks.push(Some(corner(c, 1.0, 1.0, ps)));
                    }
                    picks.push(left.map(|l| corner(l, 1.0, -1.0, ps)));
                    if !corner_taken && let Some(c) = left.and_then(|l| get(l.links[3])) {
                        picks.push(Some(corner(c, 1.0, 1.0, ps)));
                    }
                    avg(bl, &picks)
                };
                let (offs, mut norm) = crate::objtools::geom::plane_fit(&[p1, p2, p3, p4]);
                if norm.y < 0.0 {
                    norm = Vec3::new(-norm.x, -norm.y, -norm.z);
                }
                let plane = Plane::build(offs, norm);
                let pos = plane_of(n).intersect_ray_point(n.pos, Vec3::Y).map_or(n.pos, |(p, _)| p);
                Some((plane.n, plane.d, pos))
            })
            .collect();
        let mut c = 0;
        for (n, s) in self.nodes.iter_mut().zip(smoothed) {
            if let Some((normal, d, pos)) = s {
                n.normal = normal;
                n.d = d;
                n.pos = pos;
                c += 1;
            }
        }
        self.rehash();
        c
    }

    /// The selected nodes moved by a drag: new centres, the
    /// planes kept and carried with them.
    pub fn move_nodes(&mut self, moves: &[(u32, Vec3)]) -> usize {
        let mut c = 0;
        for &(i, p) in moves {
            if let Some(n) = self.nodes.get_mut(i as usize) {
                let pl = Plane::build(p, n.normal);
                n.pos = p;
                n.normal = pl.n;
                n.d = pl.d;
                c += 1;
            }
        }
        self.rehash();
        c
    }

    /// The selected nodes' planes turned about X (`about_x`) or Z, a plane that
    /// would face down by more than 0.02 staying as it is.
    pub fn rotate_selected(&mut self, about_x: bool, angle: f32) -> usize {
        let r = if about_x { M43::rotate_x(-angle) } else { M43::rotate_z(angle) };
        let mut c = 0;
        for n in self.nodes.iter_mut().filter(|n| n.flags & nf::SELECTED != 0) {
            let new_n = r.transform_dir(n.normal);
            if dot(Vec3::Y, new_n) > 0.02 {
                let pl = Plane::build(n.pos, new_n);
                n.normal = pl.n;
                n.d = pl.d;
                c += 1;
            }
        }
        c
    }

    /// The nodes a selected node links to are highlighted.
    pub fn update_highlight(&mut self) {
        let targets: Vec<u32> = self
            .nodes
            .iter()
            .filter(|n| n.flags & nf::SELECTED != 0)
            .flat_map(|n| n.links.into_iter().flatten())
            .collect();
        for n in &mut self.nodes {
            n.flags &= !nf::HL_SELECTED;
        }
        for t in targets {
            if let Some(n) = self.nodes.get_mut(t as usize) {
                n.flags |= nf::HL_SELECTED;
            }
        }
    }

    /// The nearest node whose patch the ray crosses within half a patch of its
    /// centre, nearer than `dist`.
    #[must_use]
    pub fn pick(&self, start: Vec3, dir: Vec3, dist: f32) -> Option<(u32, f32)> {
        let ps = self.ps();
        let psz = (ps / 2.0) * (ps / 2.0);
        let mut best: Option<(u32, f32)> = None;
        let mut reach = dist;
        for (i, n) in self.nodes.iter().enumerate() {
            // the pick frustum's sphere test, with the patch as the radius
            let to = n.pos - start;
            let along = dot(to, dir);
            if along < -ps || dot(to, to) - along * along > ps * ps * 4.0 {
                continue;
            }
            let Some((dest, ahead)) = plane_of(n).intersect_ray_point(start, dir) else { continue };
            if !ahead || dist_sqr(n.pos, dest) >= psz {
                continue;
            }
            let d = dist_sqr(start, dest).sqrt();
            if d < reach {
                reach = d;
                best = Some((i as u32, d));
            }
        }
        best
    }

    /// The frustum test: the nodes whose box (0.35 of a patch round the centre) meets every
    /// plane of a frustum (`n . p + w >= 0` inside).
    #[must_use]
    pub fn in_frustum(&self, planes: &[Vec4]) -> Vec<u32> {
        let h = self.ps() * 0.35;
        self.nodes
            .par_iter()
            .enumerate()
            .filter(|(_, n)| {
                let (min, max) = (n.pos - Vec3::splat(h), n.pos + Vec3::splat(h));
                planes.iter().all(|p| {
                    let far = Vec3::new(
                        if p.x >= 0.0 { max.x } else { min.x },
                        if p.y >= 0.0 { max.y } else { min.y },
                        if p.z >= 0.0 { max.z } else { min.z },
                    );
                    p.x * far.x + p.y * far.y + p.z * far.z + p.w >= 0.0
                })
            })
            .map(|(i, _)| i as u32)
            .collect()
    }

    /// The scene's part of these nodes: tiles equal to `base`'s are `base`'s.
    #[must_use]
    pub fn into_part(mut self, base: Option<&AiMapPart>) -> AiMapPart {
        self.update_highlight();
        AiMapPart::from_nodes(self.header, &self.nodes, base)
    }
}

/// Where a generation over the ground starts: the spawn points inside the box (the player's
/// start, the graph points, every placed NPC stands on ground NPCs walk), else the ground under
/// the box's middle, else the first ground a coarse grid over the box finds. The flood then
/// covers every patch a walk from them reaches.
#[must_use]
pub fn seeds(model: &CollideModel, spawns: &[Vec3], within: &Aabb) -> Vec<Vec3> {
    let inside =
        |p: &Vec3| p.x >= within.min.x && p.x <= within.max.x && p.z >= within.min.z && p.z <= within.max.z;
    let mut out: Vec<Vec3> = spawns.iter().copied().filter(inside).collect();
    if !out.is_empty() {
        return out;
    }
    let top = within.max.y + 1.0;
    let depth = within.max.y - within.min.y + 2.0;
    let probe = |x: f32, z: f32| {
        let start = Vec3::new(x, top, z);
        model.ray_nearest(start, Vec3::NEG_Y, depth).map(|(_, r)| Vec3::new(x, top - r, z))
    };
    let c = (within.min + within.max) * 0.5;
    if let Some(p) = probe(c.x, c.z) {
        out.push(p);
        return out;
    }
    const GRID: usize = 16;
    for i in 0..GRID {
        for k in 0..GRID {
            let x = within.min.x + (within.max.x - within.min.x) * (i as f32 + 0.5) / GRID as f32;
            let z = within.min.z + (within.max.z - within.min.z) * (k as f32 + 0.5) / GRID as f32;
            if let Some(p) = probe(x, z) {
                out.push(p);
                return out;
            }
        }
    }
    out
}

/// A new AI map over the ground: the map's settings from `header` (its nodes dropped), the box
/// `within`, a node on the ground at each seed, then the flood. Answers the work, the flood and
/// how many seeds made a node.
pub fn generate_over(
    g: &Gen<'_>,
    mut header: SceneAiMap,
    within: Aabb,
    seeds: &[Vec3],
    progress: &(dyn Fn(usize, usize) -> bool + Sync),
) -> (AiWork, Flood, usize) {
    header.nodes = Arc::from([]);
    header.bbox = within;
    let mut w = AiWork { hash: Hash::new(&within), header, nodes: Vec::new() };
    let mut seeded = 0;
    for p in seeds {
        let at = snap_xz(*p, w.ps());
        if let Some(n) = g.create_node(at, false)
            && w.place(n).is_some()
        {
            seeded += 1;
        }
    }
    let flood = if w.nodes.is_empty() { Flood::default() } else { w.flood(g, false, false, None, progress) };
    (w, flood, seeded)
}

/// A node packed over a box.
#[must_use]
pub fn encode(n: &AiMapNode, bbox: &Aabb, params: &AiParams) -> SceneAiNode {
    SceneAiNode {
        links: n.links.map(|l| l.unwrap_or(INVALID_NODE)),
        plane: pv_compress(n.normal),
        pos: PackedPos::pack(n.pos, bbox, params),
        flags: n.flags,
    }
}

/// The node counts by number of links: 0 to 4.
#[must_use]
pub fn link_histogram(nodes: &[AiMapNode]) -> [usize; 5] {
    let mut h = [0usize; 5];
    for n in nodes {
        let c = n.links.iter().filter(|l| l.is_some()).count();
        if let Some(x) = h.get_mut(c) {
            *x += 1;
        }
    }
    h
}

/// What a map is made of, to tell a whole terrain flooded from nodes made twice or off the
/// ground (`scene.ai_map action=stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapStats {
    pub nodes: usize,
    /// Distinct patches of the XZ grid that hold a node.
    pub cells: usize,
    /// Patches that hold more than one node (floors over each other).
    pub layered_cells: usize,
    pub max_per_cell: usize,
    /// Nodes of one patch closer in height than 0.05 m: made twice.
    pub duplicates: usize,
    /// Nodes whose centre is off the patch grid.
    pub off_grid: usize,
    /// Patches over which the model has a face a node can stand on (facing up, 60 degrees at
    /// most): the most nodes one floor of it gives.
    pub ground_cells: usize,
    /// Nodes with no face of the model within half a patch round and a metre above or below.
    pub off_ground: usize,
}

/// [`MapStats`] of a map's nodes over the model it was made on.
#[must_use]
pub fn stats(nodes: &[AiMapNode], ps: f32, model: Option<&CollideModel>) -> MapStats {
    use rustc_hash::{FxHashMap, FxHashSet};
    let key = |v: Vec3| ((v.x / ps).round() as i64, (v.z / ps).round() as i64);
    let mut by_cell: FxHashMap<(i64, i64), Vec<f32>> = FxHashMap::default();
    let mut off_grid = 0;
    for n in nodes {
        let (kx, kz) = key(n.pos);
        if (n.pos.x - kx as f32 * ps).abs() > 1e-3 || (n.pos.z - kz as f32 * ps).abs() > 1e-3 {
            off_grid += 1;
        }
        by_cell.entry((kx, kz)).or_default().push(n.pos.y);
    }
    let mut out = MapStats { nodes: nodes.len(), cells: by_cell.len(), off_grid, ..MapStats::default() };
    for ys in by_cell.values_mut() {
        out.max_per_cell = out.max_per_cell.max(ys.len());
        if ys.len() > 1 {
            out.layered_cells += 1;
            ys.sort_by(f32::total_cmp);
            out.duplicates += ys.windows(2).filter(|w| (w[1] - w[0]).abs() < 0.05).count();
        }
    }
    let Some(m) = model else { return out };
    let walkable = deg2rad(60.0).cos();
    let ground: FxHashSet<(i64, i64)> = m
        .tris
        .par_iter()
        .filter(|t| xr::mknormal(t[0], t[1], t[2]).y >= walkable)
        .flat_map_iter(|t| {
            // the patches whose centre the face covers, seen from above
            let (lo, hi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
            let (x0, x1) = ((lo.x / ps).ceil() as i64, (hi.x / ps).floor() as i64);
            let (z0, z1) = ((lo.z / ps).ceil() as i64, (hi.z / ps).floor() as i64);
            let t = *t;
            (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z))).filter(move |&(x, z)| {
                let (px, pz) = (x as f32 * ps, z as f32 * ps);
                let edge = |a: Vec3, b: Vec3| (b.x - a.x) * (pz - a.z) - (b.z - a.z) * (px - a.x);
                let (e0, e1, e2) = (edge(t[0], t[1]), edge(t[1], t[2]), edge(t[2], t[0]));
                (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)
            })
        })
        .collect();
    out.ground_cells = ground.len();
    let half = Vec3::new(ps / 2.0, 1.0, ps / 2.0);
    out.off_ground = nodes
        .par_iter()
        .filter(|n| m.box_query(&Aabb::new(n.pos - half, n.pos + half), true).is_empty())
        .count();
    out
}

/// The four sides as the panel names them, in the order the chunk stores their links.
pub const SIDES: [&str; 4] = ["left", "forward", "right", "back"];

/// A side of the panel's pad (away from the camera, left, right, towards the
/// camera) as a side of the map, by the camera's heading.
#[must_use]
pub fn side_for_heading(pad: usize, heading: f32) -> usize {
    const IDX: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2]];
    let tau = std::f32::consts::TAU;
    let h = heading.rem_euclid(tau) / std::f32::consts::PI;
    let row = if h > 0.25 && h <= 0.75 {
        3
    } else if h > 0.75 && h <= 1.25 {
        2
    } else if h > 1.25 && h <= 1.75 {
        1
    } else {
        0
    };
    IDX[row][pad.min(3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(min: Vec3, max_x: f32, max_z: f32) -> Vec<[Vec3; 3]> {
        let (a, b, c, d) = (
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(min.x, min.y, max_z),
            Vec3::new(max_x, min.y, max_z),
            Vec3::new(max_x, min.y, min.z),
        );
        vec![[a, b, c], [a, c, d]]
    }

    fn model(tris: Vec<[Vec3; 3]>, mats: Vec<u16>) -> CollideModel {
        CollideModel::from_tris(tris, mats, vec!["default".into(), "water".into()])
    }

    fn work(bbox: Aabb) -> AiWork {
        let mut w = AiWork::new(None);
        w.header.bbox = bbox;
        w.rehash();
        w
    }

    #[test]
    fn a_flat_floor_gives_the_grid() {
        let m = model(quad(Vec3::new(-3.5, 0.0, -3.5), 3.5, 3.5), vec![0, 0]);
        let g = Gen { model: &m, params: AiParams::default(), ignored: vec![false, false] };
        let mut w = work(Aabb::new(Vec3::new(-3.5, 0.0, -3.5), Vec3::new(3.5, 0.0, 3.5)));
        assert_eq!(w.add_node(&g, Vec3::new(0.1, 0.0, 0.1), false, false, 1), Ok(1));
        let f = w.generate(&g, false, &|_, _| true);
        assert!(!f.stopped);
        // the patches whose 9 x 9 rays find the floor: x and z in 0.7 steps inside the floor
        for n in &w.nodes {
            let kx = n.pos.x / 0.7;
            assert!((kx - kx.round()).abs() < 1e-4, "{:?}", n.pos);
            assert!(n.pos.y.abs() < 1e-4);
            assert!((n.normal - Vec3::Y).length() < 1e-5);
        }
        assert!(w.nodes.len() >= 81, "{}", w.nodes.len());
        // inner nodes have all four links
        let full = w.nodes.iter().filter(|n| n.links.iter().all(Option::is_some)).count();
        assert!(full > 0);
    }

    #[test]
    fn a_step_higher_than_can_up_breaks_the_link() {
        let mut tris = quad(Vec3::new(-3.5, 0.0, -3.5), 0.35, 3.5);
        tris.extend(quad(Vec3::new(0.35, 2.0, -3.5), 3.5, 3.5));
        // the step's face
        tris.push([Vec3::new(0.35, 0.0, -3.5), Vec3::new(0.35, 2.0, -3.5), Vec3::new(0.35, 2.0, 3.5)]);
        let n = tris.len();
        let m = model(tris, vec![0; n]);
        let g = Gen { model: &m, params: AiParams::default(), ignored: vec![false, false] };
        let mut w = work(Aabb::new(Vec3::new(-3.5, 0.0, -3.5), Vec3::new(3.5, 2.0, 3.5)));
        w.add_node(&g, Vec3::new(-1.4, 0.0, 0.0), false, false, 1).unwrap();
        w.generate(&g, false, &|_, _| true);
        assert!(w.nodes.iter().all(|n| n.pos.y < 1.0), "nothing climbs a 2 m step");
    }

    #[test]
    fn an_ignored_material_leaves_a_hole() {
        let mut tris = quad(Vec3::new(-3.5, 0.0, -3.5), 3.5, 3.5);
        tris.extend(quad(Vec3::new(-0.35, 0.001, -0.35), 0.35, 0.35));
        let m = model(tris, vec![0, 0, 1, 1]);
        let g = Gen { model: &m, params: AiParams::default(), ignored: vec![false, true] };
        let with = g.create_node(Vec3::ZERO, false);
        assert!(with.is_some(), "the floor under the ignored patch is still there");
        let only = model(quad(Vec3::new(-0.35, 0.0, -0.35), 0.35, 0.35), vec![1, 1]);
        let g2 = Gen { model: &only, params: AiParams::default(), ignored: vec![false, true] };
        assert!(g2.create_node(Vec3::ZERO, false).is_none());
    }

    #[test]
    fn link_edits_and_selection() {
        let m = model(quad(Vec3::new(-3.5, 0.0, -3.5), 3.5, 3.5), vec![0, 0]);
        let g = Gen { model: &m, params: AiParams::default(), ignored: vec![false, false] };
        let mut w = work(Aabb::new(Vec3::new(-3.5, 0.0, -3.5), Vec3::new(3.5, 0.0, 3.5)));
        w.add_node(&g, Vec3::ZERO, false, false, 1).unwrap();
        w.generate(&g, false, &|_, _| true);
        w.select_all(true);
        let removed = w.make_links(SIDE_FLAGS[2], LinkMode::Remove, false);
        assert!(removed > 0);
        assert!(w.nodes.iter().all(|n| n.links[2].is_none()));
        let added = w.make_links(SIDE_FLAGS[2], LinkMode::Add, false);
        assert_eq!(added, removed);
        // a one-way link turned round
        w.select_all(false);
        let a = w.find_node(Vec3::ZERO, 0.05).unwrap() as usize;
        let b = w.nodes[a].links[2].unwrap() as usize;
        w.nodes[b].links[0] = None;
        w.select(&[a as u32, b as u32], true);
        assert_eq!(w.make_links(SIDE_FLAGS[2], LinkMode::Invert, false), 1);
        assert!(w.nodes[a].links[2].is_none() && w.nodes[b].links[0] == Some(a as u32));
        assert!(w.nodes.iter().enumerate().all(|(i, n)| n.links.iter().flatten().all(|l| *l as usize != i)));
        // by link count
        let none = w.select_by_links(0);
        assert_eq!(none, 0);
        let removed_nodes = {
            w.select(&[a as u32], true);
            w.remove_selected()
        };
        assert!(removed_nodes >= 1);
        assert!(w.nodes.iter().all(|n| n.links.iter().flatten().all(|l| (*l as usize) < w.nodes.len())));
    }

    /// Both windings: the body meets a wall from either side (`collector_add_face_d` of a
    /// two-sided surface).
    fn wall(x: f32, y0: f32, y1: f32, z0: f32, z1: f32) -> Vec<[Vec3; 3]> {
        let (a, b, c, d) =
            (Vec3::new(x, y0, z0), Vec3::new(x, y1, z0), Vec3::new(x, y1, z1), Vec3::new(x, y0, z1));
        vec![[a, b, c], [a, c, d], [c, b, a], [d, c, a]]
    }

    /// Adds a node at the seed, then builds the nodes (without the snap list's
    /// box work): the nodes, the directed links and the link histogram.
    fn flood(tris: Vec<[Vec3; 3]>, seed: Vec3) -> (Vec<AiMapNode>, usize, [usize; 5]) {
        let n = tris.len();
        let m = model(tris, vec![0; n]);
        let g = Gen { model: &m, params: AiParams::default(), ignored: vec![false, false] };
        let mut w = work(Aabb::new(Vec3::new(-3.6, -1.0, -3.6), Vec3::new(3.6, 3.0, 3.6)));
        assert_eq!(w.add_node(&g, seed, false, false, 1), Ok(1));
        assert!(!w.generate(&g, false, &|_, _| true).stopped);
        let links = w.nodes.iter().map(|n| n.links.iter().flatten().count()).sum();
        let h = link_histogram(&w.nodes);
        (w.nodes, links, h)
    }

    /// The numbers follow from the generation with the default parameters: a 0.7 m
    /// patch, 9 x 9 rays 0.0875 m apart over it, a node only where 70 % of them meet the ground
    /// (`rc_lim`), the body (radius 0.7 / sqrt 2 = 0.495) slid to the next patch and stopped where
    /// it touches something, 0.1 m short of it failing. No edge below lies on a ray.
    #[test]
    fn generation_counts_of_a_few_floors() {
        let floor = || quad(Vec3::new(-3.6, 0.0, -3.6), 3.6, 3.6);

        // a flat 7.2 m square: centres -2.8..2.8 (9 a side); at 3.5 only 6 of 9 ray columns are
        // over the floor (66.7 % < 70 %); 2 x 9 x 8 = 144 neighbours, both ways
        let (nodes, links, h) = flood(floor(), Vec3::new(0.1, 0.0, 0.1));
        assert_eq!((nodes.len(), links), (81, 288));
        assert_eq!(h, [0, 0, 4, 28, 49]);
        let s = stats(&nodes, 0.7, None);
        assert_eq!((s.cells, s.duplicates, s.off_grid, s.layered_cells), (81, 0, 0, 0));

        // a hole of 2 x 2 m in the middle: the 3 x 3 patches over it see at most 17 rays of the
        // floor; 144 - 12 among them - 12 to the ring = 120 neighbours
        let mut holed = quad(Vec3::new(-3.6, 0.0, -3.6), -1.0, 3.6);
        holed.extend(quad(Vec3::new(1.0, 0.0, -3.6), 3.6, 3.6));
        holed.extend(quad(Vec3::new(-1.0, 0.0, -3.6), 1.0, -1.0));
        holed.extend(quad(Vec3::new(-1.0, 0.0, 1.0), 1.0, 3.6));
        let (nodes, links, h) = flood(holed, Vec3::new(2.1, 0.0, 2.1));
        assert_eq!((nodes.len(), links), (72, 240));
        assert_eq!(h, [0, 0, 4, 40, 28]);
        assert!(nodes.iter().all(|n| n.pos.x.abs() > 1.0 || n.pos.z.abs() > 1.0));

        // a wall at x = 0.35, half way between two columns: the body slid from -0.7 to 0 stops
        // at -0.145 (0.145 > 0.1), so the column at 0 is never made; 4 columns, 3 x 9 + 4 x 8 = 59
        let mut walled = floor();
        walled.extend(wall(0.35, 0.0, 3.0, -3.6, 3.6));
        let (nodes, links, h) = flood(walled, Vec3::new(-1.4, 0.0, 0.0));
        assert_eq!((nodes.len(), links), (36, 118));
        assert_eq!(h, [0, 0, 4, 18, 14]);
        assert!(nodes.iter().all(|n| n.pos.x < 0.0));

        // a 2 m step (over can_up) at x = 0.35: from below it is the wall above; from the top the
        // patch past the edge finds no ground within the metre CreateNode looks (its box), so the
        // upper floor keeps to x = 0.7..2.8
        let mut stepped = quad(Vec3::new(-3.6, 0.0, -3.6), 0.35, 3.6);
        stepped.extend(quad(Vec3::new(0.35, 2.0, -3.6), 3.6, 3.6));
        stepped.extend(wall(0.35, 0.0, 2.0, -3.6, 3.6));
        let (low, low_links, _) = flood(stepped.clone(), Vec3::new(-1.4, 0.0, 0.0));
        assert_eq!((low.len(), low_links), (36, 118));
        assert!(low.iter().all(|n| n.pos.y.abs() < 1e-4 && n.pos.x < 0.0));
        let (high, high_links, _) = flood(stepped, Vec3::new(1.4, 2.0, 0.0));
        assert_eq!((high.len(), high_links), (36, 118));
        assert!(high.iter().all(|n| (n.pos.y - 2.0).abs() < 1e-4 && n.pos.x > 0.5));
    }

    #[test]
    fn the_pad_follows_the_camera() {
        assert_eq!(side_for_heading(1, 0.0), 1);
        assert_eq!(side_for_heading(1, std::f32::consts::PI), 3);
    }
}

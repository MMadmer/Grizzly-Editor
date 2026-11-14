//! The AI map as `ESceneAIMapTool::OnRender` draws it: every node a quad laid on its plane
//! (`ed\ed_ai_arrows_01`, the frame of its links), grey, lighter when highlighted, white when
//! selected, the nodes within the tool's view radius of the camera; the map's box when the AI
//! Map is the current tool.
//!
//! The nodes come from `SceneState::bulk` in tiles of 4096, as the tool holds them (exact
//! positions and planes). A tile the camera comes near is sorted into 16 m cells once per revision
//! (the tile's own `Arc`), and a cell gets its mesh when the view radius reaches it - a map of half
//! a million nodes costs a pass what the cells round the camera cost.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_fmt_level::{AiMapNode, AiParams, INVALID_NODE, SceneAiNode, node_flags, pv_decompress};
use gz_render::{
    Aabb, Assets, Color, Instance, InstanceFlags, MaterialDesc, MeshData, MeshHandle, MeshVertex, SubMesh,
};
use gz_scene::tools::AiMapPart;
use gz_scene::{ObjClass, ObjId, SceneState};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::draw::{Built, ga};

/// `m_Shader.create("editor\\ai_node", "ed\\ed_ai_arrows_01")`.
const SHADER: &str = "editor\\ai_node";
const TEXTURE: &str = "ed\\ed_ai_arrows_01";
/// `ESceneAIMapTool::flHideNodes`.
const HIDE_NODES: u32 = 1 << 1;
/// The cell a tile's mesh is cut into.
const CELL: f32 = 16.0;
/// The AI box of the current tool.
const BOX_COLOR: u32 = 0xffff_c000;

/// `node_tc[k]`: the frame of the arrows texture for the links a node has (bit 0 left, 1
/// forward, 2 right, 3 back), its corners min x min z, max x min z, max x max z, min x max z.
#[must_use]
pub fn node_tc(k: u32) -> [Vec2; 4] {
    let d = 0.25;
    let (col, row) = ((k % 4) as f32, (k / 4) as f32);
    [
        Vec2::new(col * d, 0.25 + row * d),
        Vec2::new(0.25 + col * d, 0.25 + row * d),
        Vec2::new(0.25 + col * d, row * d),
        Vec2::new(col * d, row * d),
    ]
}

/// A node unpacked: where, on what plane, with which links.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub pos: Vec3,
    pub normal: Vec3,
    pub links: u32,
    pub flags: u8,
}

/// The tool's load arithmetic for one node (`UnpackPosition`, the compressed normal).
#[must_use]
pub fn unpack(n: &SceneAiNode, bbox: &gz_fmt_level::Aabb, params: &AiParams) -> Node {
    let pos = n.pos.unpack(bbox, params);
    let normal = pv_decompress(n.plane).try_normalize().unwrap_or(Vec3::Y);
    let mut links = 0;
    for (i, l) in n.links.iter().enumerate() {
        if *l != INVALID_NODE {
            links |= 1 << i;
        }
    }
    Node { pos, normal, links, flags: n.flags }
}

/// A node of the tool as the quad is made of it.
#[must_use]
pub fn of_node(n: &AiMapNode) -> Node {
    let mut links = 0;
    for (i, l) in n.links.iter().enumerate() {
        if l.is_some() {
            links |= 1 << i;
        }
    }
    let normal = if n.normal.is_finite() && n.normal.length_squared() > 0.0 { n.normal } else { Vec3::Y };
    Node { pos: n.pos, normal, links, flags: n.flags }
}

/// A node's quad as `OnRender` fills it: the corners of a square 0.9 of the patch on the node's
/// plane straight under or over them, a centimetre off it.
pub fn node_quad(n: &Node, patch: f32, out: &mut Vec<MeshVertex>, indices: &mut Vec<u32>) {
    let st = patch * 0.9 * 0.5;
    let d = -n.normal.dot(n.pos);
    let clr = if n.flags & node_flags::SELECTED != 0 {
        0xffff_ffff
    } else if n.flags & node_flags::HL_SELECTED != 0 {
        0xff90_9090
    } else {
        0xff60_6060
    };
    let color = Color::from_argb(clr).0;
    let tc = node_tc(n.links);
    let base = out.len() as u32;
    let corners = [(-st, -st), (st, -st), (st, st), (-st, st)];
    for ((dx, dz), uv) in corners.into_iter().zip(tc) {
        let v = Vec3::new(n.pos.x + dx, n.pos.y, n.pos.z + dz);
        // `intersectRayPoint` along +Y; a plane that stands upright keeps the corner as it is
        let denom = n.normal.y;
        let on = if denom.abs() < 1e-7 { v } else { v - Vec3::Y * ((n.normal.dot(v) + d) / denom) };
        let mut vert = MeshVertex::new(on + n.normal * 0.01, n.normal, uv);
        vert.color = color;
        out.push(vert);
    }
    // v3 v2 v1, v1 v4 v3
    indices.extend([base + 2, base + 1, base, base, base + 3, base + 2]);
}

/// One cell's mesh of a tile.
#[derive(Clone)]
struct Chunk {
    name: String,
    mesh: MeshHandle,
    bounds: Aabb,
}

/// The nodes of a tile that fall in one cell, and the box their quads take.
struct Cell {
    key: [i32; 2],
    bounds: Aabb,
    nodes: Vec<u32>,
}

/// A tile as it was last seen.
#[derive(Clone)]
struct Tile {
    /// The tile's `Arc`: a new one is a new revision.
    key: usize,
    bounds: Option<Aabb>,
    /// The tile's nodes by cell, sorted when the camera first came near.
    cells: Option<Arc<[Cell]>>,
    /// The meshes of the cells the camera came near, by cell: only those are ever made.
    chunks: Arc<FxHashMap<[i32; 2], Chunk>>,
}

/// What the AI map pass keeps between frames: an `Editor` service. A clone shares the meshes.
#[derive(Clone, Default)]
pub struct AiMapCache {
    tiles: Vec<Tile>,
    /// The map's box and parameters the positions are unpacked over.
    header: Option<([f32; 6], [f32; 4])>,
}

fn header_key(part: &AiMapPart) -> ([f32; 6], [f32; 4]) {
    let p = part.header.params;
    ([0.0; 6], [p.patch_size, 0.0, 0.0, 0.0])
}

/// A tile's nodes sorted into cells: one pass over them, no mesh made.
fn tile_cells(nodes: &[AiMapNode], patch: f32) -> Arc<[Cell]> {
    let mut cells: FxHashMap<[i32; 2], (Aabb, Vec<u32>)> = FxHashMap::default();
    let half = Vec3::new(patch * 0.5, 0.0, patch * 0.5);
    for (i, n) in nodes.iter().enumerate() {
        let key = [(n.pos.x / CELL).floor() as i32, (n.pos.z / CELL).floor() as i32];
        let (b, list) = cells.entry(key).or_insert_with(|| (Aabb::EMPTY, Vec::new()));
        b.extend(n.pos - half);
        b.extend(n.pos + half);
        list.push(i as u32);
    }
    let mut out: Vec<Cell> =
        cells.into_iter().map(|(key, (bounds, nodes))| Cell { key, bounds, nodes }).collect();
    out.sort_by_key(|c| c.key);
    out.into()
}

/// The mesh of one cell's nodes.
fn cell_mesh(nodes: &[AiMapNode], cell: &Cell, patch: f32, material: &MaterialDesc) -> Option<MeshData> {
    let mut v = Vec::with_capacity(cell.nodes.len() * 4);
    let mut i = Vec::with_capacity(cell.nodes.len() * 6);
    for n in cell.nodes.iter().filter_map(|&k| nodes.get(k as usize)) {
        node_quad(&of_node(n), patch, &mut v, &mut i);
    }
    let count = i.len() as u32;
    MeshData::new(v, i, vec![SubMesh { indices: 0..count, material: material.clone() }]).ok()
}

/// The tile's box from its nodes.
fn tile_bounds(nodes: &[AiMapNode]) -> Option<Aabb> {
    let mut b = Aabb::EMPTY;
    for n in nodes {
        b.extend(n.pos);
    }
    (!b.is_empty()).then_some(b)
}

/// The XZ square of the view radius round the camera meets the box.
fn near(b: &Aabb, eye: Vec3, r: f32) -> bool {
    b.max.x >= eye.x - r && b.min.x <= eye.x + r && b.max.z >= eye.z - r && b.min.z <= eye.z + r
}

impl AiMapCache {
    fn drop_tile(&mut self, i: usize, assets: &Assets) {
        if let Some(t) = self.tiles.get_mut(i)
            && !t.chunks.is_empty()
        {
            for c in t.chunks.values() {
                assets.remove_mesh(&c.name);
            }
            t.chunks = Arc::default();
        }
    }

    /// Adds the nodes round `eye` and the map's box to the frame. Only the cells within the view
    /// radius get a mesh: a map of half a million nodes costs a pass what the cells round the
    /// camera cost, the first one after a load as well.
    pub(crate) fn draw(&mut self, state: &SceneState, assets: &Assets, eye: Vec3, built: &mut Built) {
        let part = state.bulk.ai_map.as_ref();
        let tool = state.tools.part(ObjClass::AiMap);
        let shown = tool.visible && part.is_some_and(|p| p.header.flags & HIDE_NODES == 0);
        let Some(part) = part.filter(|_| shown) else {
            // nothing to show: the meshes go with the map
            for i in 0..self.tiles.len() {
                self.drop_tile(i, assets);
            }
            self.tiles.clear();
            return;
        };
        if state.tools.mode == gz_scene::SceneMode::AiMap
            && let Some(b) = ga(&part.header.bbox)
        {
            built.list.helpers.shape(
                gz_render::ShapeKind::SelectionBox,
                b.unit_transform(),
                None,
                Some(Color::from_argb(BOX_COLOR)),
                true,
            );
        }
        // new parameters resize every node
        let key = header_key(part);
        if self.header != Some(key) {
            for i in 0..self.tiles.len() {
                self.drop_tile(i, assets);
            }
            self.tiles.clear();
            self.header = Some(key);
        }
        // tiles that changed are made again; tiles past the end go
        for i in part.nodes.len()..self.tiles.len() {
            self.drop_tile(i, assets);
        }
        self.tiles.truncate(part.nodes.len());
        for (i, nodes) in part.nodes.iter().enumerate() {
            let k = Arc::as_ptr(nodes) as *const u8 as usize;
            if self.tiles.get(i).is_some_and(|t| t.key == k) {
                continue;
            }
            self.drop_tile(i, assets);
            let tile = Tile { key: k, bounds: tile_bounds(nodes), cells: None, chunks: Arc::default() };
            match self.tiles.get_mut(i) {
                Some(t) => *t = tile,
                None => self.tiles.push(tile),
            }
        }
        let r = part.header.vis_radius.max(0.0);
        let patch = part.header.params.patch_size;
        // a cell's quads reach half a patch past its nodes
        let reach = |b: &Aabb| near(b, eye, r + patch);
        let keep = |b: &Aabb| near(b, eye, 3.0 * r + CELL * 4.0);
        // tiles far from the camera let their meshes go, near ones the cells far from it
        for i in 0..self.tiles.len() {
            let far = self.tiles.get(i).and_then(|t| t.bounds).is_none_or(|b| !keep(&b));
            if far {
                self.drop_tile(i, assets);
                continue;
            }
            if let Some(t) = self.tiles.get_mut(i)
                && let Some(cells) = &t.cells
            {
                let gone: Vec<[i32; 2]> = cells
                    .iter()
                    .filter(|c| t.chunks.contains_key(&c.key) && !keep(&c.bounds))
                    .map(|c| c.key)
                    .collect();
                if !gone.is_empty() {
                    let chunks = Arc::make_mut(&mut t.chunks);
                    for k in gone {
                        if let Some(c) = chunks.remove(&k) {
                            assets.remove_mesh(&c.name);
                        }
                    }
                }
            }
        }
        // the tiles the view reaches, sorted into cells once
        let wanted: Vec<usize> = self
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.bounds.as_ref().is_some_and(reach))
            .map(|(i, _)| i)
            .collect();
        let tiles = &self.tiles;
        let sorted: Vec<(usize, Arc<[Cell]>)> = wanted
            .par_iter()
            .filter(|&&i| tiles.get(i).is_some_and(|t| t.cells.is_none()))
            .filter_map(|&i| part.nodes.get(i).map(|nodes| (i, tile_cells(nodes, patch))))
            .collect();
        for (i, cells) in sorted {
            if let Some(t) = self.tiles.get_mut(i) {
                t.cells = Some(cells);
            }
        }
        // the cells in reach that have no mesh yet
        let mut todo: Vec<(usize, usize)> = Vec::new();
        for &i in &wanted {
            let Some(t) = self.tiles.get(i) else { continue };
            let Some(cells) = &t.cells else { continue };
            for (ci, c) in cells.iter().enumerate() {
                if reach(&c.bounds) && !t.chunks.contains_key(&c.key) {
                    todo.push((i, ci));
                }
            }
        }
        if !todo.is_empty() {
            let table = assets.table();
            let material = MaterialDesc::textured(table.classify(SHADER), TEXTURE).with_two_sided(true);
            let tiles = &self.tiles;
            let made: Vec<(usize, [i32; 2], MeshData)> = todo
                .par_iter()
                .filter_map(|&(i, ci)| {
                    let cell = tiles.get(i)?.cells.as_ref()?.get(ci)?;
                    let mesh = cell_mesh(part.nodes.get(i)?, cell, patch, &material)?;
                    Some((i, cell.key, mesh))
                })
                .collect();
            // a name per tile and cell: the same cell made again replaces its mesh
            let named: Vec<(usize, [i32; 2], String, Aabb)> = made
                .iter()
                .map(|(i, cell, data)| {
                    (*i, *cell, format!("<aimap>/{i}/{}_{}", cell[0], cell[1]), data.bounds)
                })
                .collect();
            let handles = assets.add_meshes(
                made.into_iter()
                    .zip(&named)
                    .map(|((_, _, data), (_, _, name, _))| (name.clone(), data))
                    .collect(),
            );
            for ((i, cell, name, bounds), mesh) in named.into_iter().zip(handles) {
                if let Some(t) = self.tiles.get_mut(i) {
                    Arc::make_mut(&mut t.chunks).insert(cell, Chunk { name, mesh, bounds });
                }
            }
        }
        for t in &self.tiles {
            for c in t.chunks.values().filter(|c| reach(&c.bounds)) {
                let mut inst = Instance::new(c.mesh, glam::Mat4::IDENTITY).with_flags(InstanceFlags::NO_ID);
                inst.bounds = Some(c.bounds);
                built.list.instances.push(inst);
                built.owners.push(ObjId(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_level::{PackedPos, pv_compress};

    #[test]
    fn frames_follow_the_links() {
        // no link: the first frame; all four: the last
        assert_eq!(node_tc(0)[3], Vec2::new(0.0, 0.0));
        assert_eq!(node_tc(15)[1], Vec2::new(1.0, 1.0));
        assert_eq!(node_tc(5)[0], Vec2::new(0.25, 0.5));
    }

    #[test]
    fn a_quad_lies_on_the_nodes_plane() {
        // a slope rising along x: y = x / 2 through the node at (0, 1, 0)
        let normal = Vec3::new(-0.5, 1.0, 0.0).normalize();
        let n = Node { pos: Vec3::Y, normal, links: 0b0101, flags: node_flags::SELECTED };
        let (mut v, mut i) = (Vec::new(), Vec::new());
        node_quad(&n, 0.7, &mut v, &mut i);
        assert_eq!(v.len(), 4);
        assert_eq!(i, vec![2, 1, 0, 0, 3, 2]);
        let st = 0.7 * 0.9 * 0.5;
        for p in &v {
            let p = Vec3::from_array(p.position) - normal * 0.01;
            // on the plane
            assert!((normal.dot(p - n.pos)).abs() < 1e-5);
            assert!((p.x.abs() - st).abs() < 1e-5 && (p.z.abs() - st).abs() < 1e-5);
        }
        assert_eq!(v[0].color, [255, 255, 255, 255]);
        assert_eq!(v[0].uv, node_tc(5)[0].to_array());
    }

    #[test]
    fn nodes_unpack_over_the_box() {
        let bbox = gz_fmt_level::Aabb::new(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 20.0, 10.0));
        let params = AiParams::default();
        let p = Vec3::new(1.4, 5.0, -2.1);
        let n = SceneAiNode {
            links: [3, INVALID_NODE, INVALID_NODE, 7],
            plane: pv_compress(Vec3::Y),
            pos: PackedPos::pack(p, &bbox, &params),
            flags: 0,
        };
        let u = unpack(&n, &bbox, &params);
        assert_eq!(u.links, 0b1001);
        assert!((u.pos - p).abs().max_element() < 0.01);
        assert!((u.normal - Vec3::Y).abs().max_element() < 0.01);
    }
}

//! The AI map as the editor keeps it: the chunks of the AI Map tool in a scene, the
//! `build.aimap` it hands to the AI compiler, and the `level.ai` it makes itself.
//!
//! A scene node is stored packed (links, compressed plane, quantised position); the tool works
//! on [`AiMapNode`], and [`SceneAiMap::decode_nodes`] / [`SceneAiMap::encode_nodes`] are the
//! tool's load and save arithmetic.

use std::sync::Arc;

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::ai_map::{AiNode, LevelGraph, LevelGraphHeader, NodeLayout, XRAI_VERSION_COP, XRAI_VERSION_SOC};
use crate::bytes::{aabb_at, f32_at, fits, i16_at, u16_at, u32_at};
use crate::math::{EPS_L, dot, i_floor, normalize, pv_compress, pv_decompress};
use crate::{Aabb, LevelError, Result};

/// The scene's AI map version.
pub const AIMAP_VERSION: u16 = 2;
/// The version of `build.aimap`.
pub const BUILD_AIMAP_VERSION: u16 = 1;
/// A link that points nowhere, in the editor's files (24 bits set).
pub const INVALID_NODE: u32 = (1 << 24) - 1;

const CHUNK_VERSION: u32 = 0x0001;
const CHUNK_FLAGS: u32 = 0x0002;
const CHUNK_BOX: u32 = 0x0003;
const CHUNK_PARAMS: u32 = 0x0004;
const CHUNK_NODES: u32 = 0x0006;
const CHUNK_SNAP_OBJECTS: u32 = 0x0007;
const CHUNK_INTERNAL_DATA: u32 = 0x0008;
const CHUNK_INTERNAL_DATA2: u32 = 0x0009;

const E_CHUNK_VERSION: u32 = 0x0001;
const E_CHUNK_BOX: u32 = 0x0002;
const E_CHUNK_PARAMS: u32 = 0x0003;
const E_CHUNK_NODES: u32 = 0x0004;

/// A scene node: 4 links of 3 bytes, the plane, the position, the flags.
const SCENE_NODE: usize = 21;
/// A `build.aimap` node: the same without the flags.
const BUILD_NODE: usize = 20;

/// Node flags of the tool (`SAINode::fl*`).
pub mod node_flags {
    pub const SELECTED: u8 = 1 << 0;
    pub const HL_SELECTED: u8 = 1 << 1;
    pub const N1: u8 = 1 << 4;
    pub const N2: u8 = 1 << 5;
    pub const N3: u8 = 1 << 6;
    pub const N4: u8 = 1 << 7;
}

/// `SAIParams`: how the map was generated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiParams {
    pub patch_size: f32,
    pub test_height: f32,
    pub can_up: f32,
    pub can_down: f32,
}

impl Default for AiParams {
    fn default() -> Self {
        Self { patch_size: 0.7, test_height: 1.0, can_up: 1.5, can_down: 4.0 }
    }
}

impl AiParams {
    fn read(b: &[u8]) -> Self {
        Self {
            patch_size: f32_at(b, 0),
            test_height: f32_at(b, 4),
            can_up: f32_at(b, 8),
            can_down: f32_at(b, 12),
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_f32(self.patch_size);
        w.w_f32(self.test_height);
        w.w_f32(self.can_up);
        w.w_f32(self.can_down);
    }
}

/// `SNodePositionOld`: a position in patches (x, z) and 1/65535 of the map's height (y).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedPos {
    pub x: i16,
    pub y: u16,
    pub z: i16,
}

impl PackedPos {
    /// `ESceneAIMapTool::PackPosition`.
    #[must_use]
    pub fn pack(p: Vec3, bbox: &Aabb, params: &AiParams) -> Self {
        let sp = 1.0 / params.patch_size;
        let px = i_floor(p.x * sp + EPS_L).clamp(-32767, 32767);
        let py = i_floor(65535.0 * (p.y - bbox.min.y) / (bbox.max.y - bbox.min.y) + EPS_L).clamp(0, 65535);
        let pz = i_floor(p.z * sp + EPS_L).clamp(-32767, 32767);
        Self { x: px as i16, y: py as u16, z: pz as i16 }
    }

    /// `ESceneAIMapTool::UnpackPosition`.
    #[must_use]
    pub fn unpack(&self, bbox: &Aabb, params: &AiParams) -> Vec3 {
        Vec3::new(
            f32::from(self.x) * params.patch_size,
            (f32::from(self.y) / 65535.0) * (bbox.max.y - bbox.min.y) + bbox.min.y,
            f32::from(self.z) * params.patch_size,
        )
    }
}

/// A node as the scene file holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneAiNode {
    /// Left, forward, right, back: node indices, [`INVALID_NODE`] for none.
    pub links: [u32; 4],
    /// The plane normal, [`pv_compress`]ed.
    pub plane: u16,
    pub pos: PackedPos,
    /// [`node_flags`]. `build.aimap` has none: 0 there.
    pub flags: u8,
}

fn link_at(b: &[u8], at: usize) -> u32 {
    u32_at(b, at) & 0x00ff_ffff
}

impl SceneAiNode {
    fn read(rec: &[u8], with_flags: bool) -> Self {
        Self {
            links: [link_at(rec, 0), link_at(rec, 3), link_at(rec, 6), link_at(rec, 9)],
            plane: u16_at(rec, 12),
            pos: PackedPos { x: i16_at(rec, 14), y: u16_at(rec, 16), z: i16_at(rec, 18) },
            flags: if with_flags { rec.get(20).copied().unwrap_or(0) } else { 0 },
        }
    }

    fn write(&self, w: &mut Writer, with_flags: bool) {
        for link in self.links {
            let raw = link.to_le_bytes();
            w.w(&[raw[0], raw[1], raw[2]]);
        }
        w.w_u16(self.plane);
        w.w_i16(self.pos.x);
        w.w_u16(self.pos.y);
        w.w_i16(self.pos.z);
        if with_flags {
            w.w_u8(self.flags);
        }
    }
}

/// A node as the tool works on it (`SAINode`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiMapNode {
    /// Left, forward, right, back: indices into the node list.
    pub links: [Option<u32>; 4],
    /// The plane of the patch: unit normal and `d`.
    pub normal: Vec3,
    pub d: f32,
    /// The centre of the patch.
    pub pos: Vec3,
    /// [`node_flags`].
    pub flags: u8,
}

impl AiMapNode {
    /// A node at a position on a plane with this normal, linked to nothing.
    #[must_use]
    pub fn new(pos: Vec3, normal: Vec3) -> Self {
        let n = normalize(normal);
        Self { links: [None; 4], normal: n, d: -dot(n, pos), pos, flags: 0 }
    }

    fn encode(&self, bbox: &Aabb, params: &AiParams) -> SceneAiNode {
        SceneAiNode {
            links: self.links.map(|l| l.unwrap_or(INVALID_NODE)),
            plane: pv_compress(self.normal),
            pos: PackedPos::pack(self.pos, bbox, params),
            flags: self.flags,
        }
    }
}

/// `CalculateNodesBBox`: the box of the node centres.
#[must_use]
pub fn nodes_bbox(nodes: &[AiMapNode]) -> Aabb {
    let mut bb = Aabb::INVALID;
    for n in nodes {
        bb.modify(n.pos);
    }
    bb
}

/// The AI Map tool's part of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneAiMap {
    /// The tool's flags (`flHideNodes` = 2, `flSlowCalculate` = 4).
    pub flags: u32,
    /// The box positions are packed over.
    pub bbox: Aabb,
    pub params: AiParams,
    pub nodes: Arc<[SceneAiNode]>,
    pub vis_radius: f32,
    pub brush_size: u32,
    pub smooth_height: f32,
    /// Names of the scene objects the map snaps to.
    pub snap_objects: Vec<SmolStr>,
}

impl Default for SceneAiMap {
    fn default() -> Self {
        Self {
            flags: 0,
            bbox: Aabb::INVALID,
            params: AiParams::default(),
            nodes: Arc::from([]),
            vis_radius: 30.0,
            brush_size: 1,
            smooth_height: 0.5,
            snap_objects: Vec::new(),
        }
    }
}

fn fixed<'a>(r: &Reader<'a>, id: u32, size: usize, what: &str) -> Result<gz_chunk::Chunk<'a>> {
    let chunk = r.need_chunk(id)?;
    if chunk.bytes().len() < size {
        return Err(LevelError::Invalid(format!("AI map: the {what} chunk is too short")));
    }
    Ok(chunk)
}

fn read_nodes(body: &[u8], size: usize, with_flags: bool) -> Result<Vec<SceneAiNode>> {
    let count = u32_at(body, 0);
    let rest = body.get(4..).unwrap_or_default();
    if body.len() < 4 {
        return Err(LevelError::Invalid("AI map: the node chunk is too short".into()));
    }
    let n = fits(rest.len(), u64::from(count), size as u64, "AI map nodes")?;
    Ok(rest
        .get(..n * size)
        .unwrap_or_default()
        .chunks_exact(size)
        .map(|rec| SceneAiNode::read(rec, with_flags))
        .collect())
}

impl SceneAiMap {
    /// Reads the tool's chunk of a scene (`LoadStream`).
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = u16_at(fixed(&r, CHUNK_VERSION, 2, "version")?.bytes(), 0);
        if version != AIMAP_VERSION {
            return Err(LevelError::Version { what: "scene AI map", found: u32::from(version) });
        }
        let flags = u32_at(fixed(&r, CHUNK_FLAGS, 4, "flags")?.bytes(), 0);
        let bbox = aabb_at(fixed(&r, CHUNK_BOX, 24, "box")?.bytes(), 0);
        let params = AiParams::read(fixed(&r, CHUNK_PARAMS, 16, "parameters")?.bytes());
        let nodes = read_nodes(r.need_chunk(CHUNK_NODES)?.bytes(), SCENE_NODE, true)?;

        let mut out = Self { flags, bbox, params, nodes: nodes.into(), ..Self::default() };
        if let Some(c) = r.open_chunk(CHUNK_INTERNAL_DATA)? {
            let mut c = c.reader();
            out.vis_radius = c.r_f32()?;
            out.brush_size = c.r_u32()?;
        }
        if let Some(c) = r.open_chunk(CHUNK_INTERNAL_DATA2)? {
            out.smooth_height = c.reader().r_f32()?;
        }
        if let Some(c) = r.open_chunk(CHUNK_SNAP_OBJECTS)? {
            let mut c = c.reader();
            let count = c.r_u32()?;
            for _ in 0..count {
                out.snap_objects.push(SmolStr::new(c.r_stringz()?));
            }
        }
        Ok(out)
    }

    /// The tool's chunk of a scene (`SaveStream`).
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(160 + self.nodes.len() * SCENE_NODE);
        w.chunk(CHUNK_VERSION, |w| w.w_u16(AIMAP_VERSION));
        w.chunk(CHUNK_FLAGS, |w| w.w_u32(self.flags));
        w.chunk(CHUNK_BOX, |w| {
            w.w_vec3(self.bbox.min);
            w.w_vec3(self.bbox.max);
        });
        w.chunk(CHUNK_PARAMS, |w| self.params.write(w));
        w.chunk(CHUNK_NODES, |w| {
            w.w_u32(self.nodes.len() as u32);
            for n in self.nodes.iter() {
                n.write(w, true);
            }
        });
        w.chunk(CHUNK_INTERNAL_DATA, |w| {
            w.w_f32(self.vis_radius);
            w.w_u32(self.brush_size);
        });
        w.chunk(CHUNK_INTERNAL_DATA2, |w| w.w_f32(self.smooth_height));
        w.chunk(CHUNK_SNAP_OBJECTS, |w| {
            w.w_u32(self.snap_objects.len() as u32);
            for name in &self.snap_objects {
                w.w_stringz(name);
            }
        });
        w.into_bytes()
    }

    /// The nodes as the tool holds them after a load, and what it says about the file: a node
    /// with a link past the end loses all four of its links and gets a message.
    #[must_use]
    pub fn decode_nodes(&self) -> (Vec<AiMapNode>, Vec<String>) {
        let count = self.nodes.len() as u32;
        let mut warnings = Vec::new();
        let nodes = self
            .nodes
            .iter()
            .map(|n| {
                let pos = n.pos.unpack(&self.bbox, &self.params);
                let normal = normalize(pv_decompress(n.plane));
                let mut links = n.links.map(|l| (l != INVALID_NODE).then_some(l));
                if n.links.iter().any(|&l| l >= count && l != INVALID_NODE) {
                    warnings.push(format!(
                        "Node: has wrong link [{:3.2}, {:3.2}, {:3.2}], {{{},{},{},{}}}",
                        pos.x, pos.y, pos.z, n.links[0], n.links[1], n.links[2], n.links[3]
                    ));
                    links = [None; 4];
                }
                AiMapNode { links, normal, d: -dot(normal, pos), pos, flags: n.flags }
            })
            .collect();
        (nodes, warnings)
    }

    /// Packs the tool's nodes for a save, over this map's box and parameters.
    pub fn encode_nodes(&mut self, nodes: &[AiMapNode]) {
        self.nodes = nodes.iter().map(|n| n.encode(&self.bbox, &self.params)).collect();
    }
}

/// `build.aimap`: what the tool's Export writes for the AI compiler.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildAiMap {
    pub bbox: Aabb,
    pub params: AiParams,
    /// Nodes without flags.
    pub nodes: Arc<[SceneAiNode]>,
}

impl BuildAiMap {
    /// The export of the tool's nodes: positions are packed over the box of the node centres.
    /// `None` for an empty map, which the editor does not export.
    #[must_use]
    pub fn export(nodes: &[AiMapNode], params: &AiParams) -> Option<Self> {
        if nodes.is_empty() {
            return None;
        }
        let bbox = nodes_bbox(nodes);
        let packed = nodes.iter().map(|n| SceneAiNode { flags: 0, ..n.encode(&bbox, params) }).collect();
        Some(Self { bbox, params: *params, nodes: packed })
    }

    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = u16_at(fixed(&r, E_CHUNK_VERSION, 2, "version")?.bytes(), 0);
        if version != BUILD_AIMAP_VERSION {
            return Err(LevelError::Version { what: "build.aimap", found: u32::from(version) });
        }
        let bbox = aabb_at(fixed(&r, E_CHUNK_BOX, 24, "box")?.bytes(), 0);
        let params = AiParams::read(fixed(&r, E_CHUNK_PARAMS, 16, "parameters")?.bytes());
        let nodes = read_nodes(r.need_chunk(E_CHUNK_NODES)?.bytes(), BUILD_NODE, false)?;
        Ok(Self { bbox, params, nodes: nodes.into() })
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(96 + self.nodes.len() * BUILD_NODE);
        w.chunk(E_CHUNK_VERSION, |w| w.w_u16(BUILD_AIMAP_VERSION));
        w.chunk(E_CHUNK_BOX, |w| {
            w.w_vec3(self.bbox.min);
            w.w_vec3(self.bbox.max);
        });
        w.chunk(E_CHUNK_PARAMS, |w| self.params.write(w));
        w.chunk(E_CHUNK_NODES, |w| {
            w.w_u32(self.nodes.len() as u32);
            for n in self.nodes.iter() {
                n.write(w, false);
            }
        });
        w.into_bytes()
    }
}

/// The compilers' cover packing: a cover of `0..=1` in `max` steps, rounded.
#[must_use]
pub fn compress_cover(cover: f32, max: i32) -> u8 {
    i_floor(cover * max as f32 + 0.5).clamp(0, max) as u8
}

/// The cover the editor's own graph gives every node at standing height: it packs the height
/// constant 1.5 where the compiler packs a measured cover.
// oracle: CLevelGraphEditor passes high_cover_height / low_cover_height to compress(); 15 and 9
// are what that gives.
pub const EDITOR_HIGH_COVER: u8 = 15;
/// The same at crouching height (0.6 packed).
pub const EDITOR_LOW_COVER: u8 = 9;

/// The `level.ai` the editor builds from the tool's nodes (`CLevelGraphEditor::build`): header
/// over the box of the node centres, nodes packed, sorted by cell and renumbered. `version` is
/// 10, or 8 for a Shadow of Chernobyl project; `guid` is a fresh one from the caller.
///
/// Every node gets [`EDITOR_HIGH_COVER`] / [`EDITOR_LOW_COVER`]; real covers are the AI
/// compiler's work. The four unused bits of a node's link block are 0 here - in the C++ they are
/// whatever the stack held.
// oracle: for Shadow of Chernobyl the C++ writes version 8 into the header and still packs the
// 23-byte node of version 10, which no game reads; here version 8 gets its own 21-byte node.
pub fn export_level_graph(
    nodes: &[AiMapNode],
    params: &AiParams,
    guid: [u8; 16],
    version: u32,
) -> Result<LevelGraph> {
    if version != XRAI_VERSION_COP && version != XRAI_VERSION_SOC {
        return Err(LevelError::Version { what: "AI map export", found: version });
    }
    let Some(layout) = NodeLayout::of(version) else {
        return Err(LevelError::Version { what: "AI map export", found: version });
    };
    if nodes.is_empty() {
        return Err(LevelError::Invalid("The AI map is empty.".into()));
    }
    let aabb = nodes_bbox(nodes);
    let header = LevelGraphHeader {
        version,
        cell_size: params.patch_size,
        factor_y: aabb.max.y - aabb.min.y + EPS_L,
        aabb,
        guid,
    };
    let mask = layout.invalid_link();
    let mut packed: Vec<AiNode> = nodes
        .iter()
        .map(|n| {
            let (xz, y) = header.compress_position(n.pos, params.patch_size);
            AiNode {
                links: n.links.map(|l| l.unwrap_or(INVALID_NODE) & mask),
                high_cover: [EDITOR_HIGH_COVER; 4],
                low_cover: [EDITOR_LOW_COVER; 4],
                plane: pv_compress(n.normal),
                // the cell index is stored in three bytes
                xz: xz & 0x00ff_ffff,
                y,
                spare: 0,
            }
        })
        .collect();
    renumber(&mut packed);
    Ok(LevelGraph { header, nodes: packed.into() })
}

/// `CNodeRenumberer`: sorts the nodes by cell (stable) and rewrites the links to match.
pub fn renumber(nodes: &mut [AiNode]) {
    let n = nodes.len() as u32;
    let mut sorted: Vec<u32> = (0..n).collect();
    sorted.sort_by_key(|&i| nodes.get(i as usize).map_or(0, |v| v.xz));
    let mut renumbering = vec![0u32; nodes.len()];
    for (new, &old) in sorted.iter().enumerate() {
        if let Some(slot) = renumbering.get_mut(old as usize) {
            *slot = new as u32;
        }
    }
    for node in nodes.iter_mut() {
        for link in &mut node.links {
            if let Some(&to) = renumbering.get(*link as usize) {
                *link = to;
            }
        }
    }
    nodes.sort_by_key(|v| v.xz);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<AiMapNode> {
        // 2 x 2 patches on a slope, listed against the cell order on purpose
        let at = |x: f32, z: f32| Vec3::new(x * 0.7, 2.0 + x * 0.1, z * 0.7);
        let normal = Vec3::new(-0.1, 1.0, 0.0);
        let mut nodes = vec![
            AiMapNode::new(at(1.0, 1.0), normal),
            AiMapNode::new(at(0.0, 0.0), normal),
            AiMapNode::new(at(1.0, 0.0), normal),
            AiMapNode::new(at(0.0, 1.0), normal),
        ];
        // forward is +z, right is +x
        nodes[1].links = [None, Some(3), Some(2), None];
        nodes[2].links = [Some(1), Some(0), None, None];
        nodes[3].links = [None, None, Some(0), Some(1)];
        nodes[0].links = [Some(3), None, None, Some(2)];
        nodes[0].flags = node_flags::SELECTED;
        nodes
    }

    fn scene() -> SceneAiMap {
        let nodes = grid();
        let mut map = SceneAiMap {
            flags: 4,
            bbox: nodes_bbox(&nodes),
            vis_radius: 45.0,
            brush_size: 3,
            smooth_height: 0.25,
            snap_objects: vec!["terrain".into(), "дом".into()],
            ..SceneAiMap::default()
        };
        map.encode_nodes(&nodes);
        map
    }

    #[test]
    fn the_scene_chunk_round_trips() {
        let map = scene();
        let bytes = map.write();
        let back = SceneAiMap::read(&bytes).unwrap();
        assert_eq!(back, map);
        assert_eq!(back.write(), bytes);

        // the chunk order and the node record are the tool's
        let r = Reader::new(&bytes);
        let ids: Vec<u32> = r.chunks().map(|c| c.0).collect();
        assert_eq!(ids, [1, 2, 3, 4, 6, 8, 9, 7]);
        let nodes = r.sub(6).unwrap().unwrap();
        assert_eq!(nodes.len(), 4 + 4 * 21);
        // node 1 sits at patch (0, 0): no left, forward 3, right 2, no back
        let rec = &nodes.data()[4 + 21..4 + 42];
        assert_eq!(&rec[..12], &[0xff, 0xff, 0xff, 3, 0, 0, 2, 0, 0, 0xff, 0xff, 0xff]);
        assert_eq!(&rec[14..20], &[0, 0, 0, 0, 0, 0]);
        assert_eq!(rec[20], 0);
    }

    #[test]
    fn decoding_gives_the_tool_its_nodes_back() {
        let map = scene();
        let (nodes, warnings) = map.decode_nodes();
        assert!(warnings.is_empty());
        let original = grid();
        for (got, want) in nodes.iter().zip(&original) {
            assert_eq!(got.links, want.links);
            assert_eq!(got.flags, want.flags);
            assert!((got.pos - want.pos).length() < 1e-3, "{:?} against {:?}", got.pos, want.pos);
            assert!((got.normal - want.normal).length() < 0.03);
            assert!((got.d + dot(got.normal, got.pos)).abs() < 1e-6);
        }
        // packed again, positions land on the same patches
        let mut again = map.clone();
        again.encode_nodes(&nodes);
        for (a, b) in again.nodes.iter().zip(map.nodes.iter()) {
            assert_eq!(a.links, b.links);
            assert_eq!((a.pos.x, a.pos.z), (b.pos.x, b.pos.z));
        }
    }

    #[test]
    fn a_wrong_link_costs_the_node_its_links() {
        let mut map = scene();
        let mut nodes = map.nodes.to_vec();
        nodes[2].links[1] = 40;
        map.nodes = nodes.into();
        let (decoded, warnings) = map.decode_nodes();
        assert_eq!(decoded[2].links, [None; 4]);
        assert_eq!(decoded[1].links, [None, Some(3), Some(2), None]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("Node: has wrong link ["), "{}", warnings[0]);
        assert!(warnings[0].ends_with("{1,40,16777215,16777215}"), "{}", warnings[0]);
    }

    #[test]
    fn build_aimap_has_no_flags_and_its_own_box() {
        let nodes = grid();
        let params = AiParams::default();
        let build = BuildAiMap::export(&nodes, &params).unwrap();
        assert!(build.nodes.iter().all(|n| n.flags == 0));
        assert_eq!(build.bbox, nodes_bbox(&nodes));
        let bytes = build.write();
        let ids: Vec<u32> = Reader::new(&bytes).chunks().map(|c| c.0).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert_eq!(Reader::new(&bytes).sub(4).unwrap().unwrap().len(), 4 + 4 * 20);
        let back = BuildAiMap::read(&bytes).unwrap();
        assert_eq!(back, build);
        assert_eq!(back.write(), bytes);
        assert!(BuildAiMap::export(&[], &params).is_none());
    }

    #[test]
    fn the_level_graph_is_sorted_and_renumbered() {
        let nodes = grid();
        let graph = export_level_graph(&nodes, &AiParams::default(), [7; 16], 10).unwrap();
        assert_eq!(graph.header.version, 10);
        assert_eq!(graph.header.cell_size, 0.7);
        assert_eq!(graph.header.aabb, nodes_bbox(&nodes));
        assert_eq!(graph.header.row_length(), 2);
        // cells: (0,0)=0, (0,1)=1, (1,0)=2, (1,1)=3 - the list order was 3, 0, 2, 1
        let cells: Vec<u32> = graph.nodes.iter().map(|n| n.xz).collect();
        assert_eq!(cells, [0, 1, 2, 3]);
        let none = 0x7f_ffff;
        assert_eq!(graph.nodes[0].links, [none, 1, 2, none]);
        assert_eq!(graph.nodes[1].links, [none, none, 3, 0]);
        assert_eq!(graph.nodes[2].links, [0, 3, none, none]);
        assert_eq!(graph.nodes[3].links, [1, none, none, 2]);
        assert!(graph.nodes.iter().all(|n| n.high_cover == [15; 4] && n.low_cover == [9; 4] && n.spare == 0));
        // heights are packed over the range plus EPS_L: 0.1 of 0.101
        assert_eq!(graph.nodes[0].y, 0);
        assert!((64000..65535).contains(&graph.nodes[3].y), "{}", graph.nodes[3].y);
        // the game reads it back
        let bytes = graph.write().unwrap();
        assert_eq!(LevelGraph::read(&bytes).unwrap(), graph);

        assert_eq!(compress_cover(1.5, 15), EDITOR_HIGH_COVER);
        assert_eq!(compress_cover(0.6, 15), EDITOR_LOW_COVER);
        assert_eq!(compress_cover(0.5, 15), 8);
        assert!(export_level_graph(&nodes, &AiParams::default(), [0; 16], 13).is_err());
        assert!(export_level_graph(&[], &AiParams::default(), [0; 16], 10).is_err());
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = scene().write();
        assert!(SceneAiMap::read(&bytes[..30]).is_err());
        assert!(SceneAiMap::read(&[]).is_err());
        // a node count far past the chunk
        let r = Reader::new(&bytes);
        let nodes = r.sub(6).unwrap().unwrap();
        let at = nodes.data().as_ptr() as usize - bytes.as_ptr() as usize;
        let mut huge = bytes.clone();
        huge[at..at + 4].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
        assert!(SceneAiMap::read(&huge).is_err());
        let mut version = bytes;
        version[8] = 9;
        assert!(matches!(SceneAiMap::read(&version), Err(LevelError::Version { found: 9, .. })));
    }
}

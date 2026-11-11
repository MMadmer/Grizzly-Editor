//! `level.ai`: the navigation graph the game walks - `hdrNODES` and one packed node per cell.
//!
//! Five node layouts exist, by header version; all are read and written:
//!
//! | version | links | cover | position | bytes |
//! |---|---|---|---|---|
//! | 8, 9 (SoC) | 4 x 23 bit | one set | 3 + 2 | 21 |
//! | 10 (CS, CoP) | 4 x 23 bit | high + low | 3 + 2 | 23 |
//! | 11 | 4 x 25 bit | high + low | 3 + 2 | 24 |
//! | 12 | 4 x 25 bit | high + low | 4 + 2 | 25 |
//! | 13 | 4 x 26 bit | high + low | 4 + 2 | 25 |

use std::sync::Arc;

use glam::Vec3;
use rayon::prelude::*;

use crate::bytes::{aabb_at, f32_at, fits, push_aabb, put_u16, put_u32, u16_at, u32_at};
use crate::math::{EPS_L, EPS_S, i_floor};
use crate::{Aabb, LevelError, Result};

/// The version of Shadow of Chernobyl.
pub const XRAI_VERSION_SOC: u32 = 8;
/// The version of Clear Sky and Call of Pripyat, the one the editor writes.
pub const XRAI_VERSION_COP: u32 = 10;
/// The newest version the game reads.
pub const XRAI_VERSION_MAX: u32 = 13;

const HEADER: usize = 56;

/// How a node lies in the file for one header version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeLayout {
    /// Bytes of one node.
    pub size: usize,
    /// Bytes of the link block at the start of a node.
    pub link_bytes: usize,
    /// Bits of one link; a link with all of them set points nowhere.
    pub link_bits: u32,
    /// Two cover sets (high and low) rather than one.
    pub two_covers: bool,
    /// The cell index takes four bytes rather than three.
    pub wide_xz: bool,
}

impl NodeLayout {
    /// The layout of a header version, when the game knows it.
    #[must_use]
    pub fn of(version: u32) -> Option<Self> {
        let (size, link_bytes, link_bits, two_covers, wide_xz) = match version {
            8 | 9 => (21, 12, 23, false, false),
            10 => (23, 12, 23, true, false),
            11 => (24, 13, 25, true, false),
            12 => (25, 13, 25, true, true),
            13 => (25, 13, 26, true, true),
            _ => return None,
        };
        Some(Self { size, link_bytes, link_bits, two_covers, wide_xz })
    }

    /// The value of a link that points nowhere, and the mask of a link.
    #[must_use]
    pub fn invalid_link(&self) -> u32 {
        (1u32 << self.link_bits) - 1
    }

    /// Bits of the link block the four links leave unused (the top of its last byte).
    fn spare_bits(&self) -> u32 {
        self.link_bytes as u32 * 8 - self.link_bits * 4
    }
}

/// `hdrNODES`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelGraphHeader {
    pub version: u32,
    /// The side of a cell, `hdrNODES::size`.
    pub cell_size: f32,
    /// The height range heights are packed over, `hdrNODES::size_y`.
    pub factor_y: f32,
    pub aabb: Aabb,
    pub guid: [u8; 16],
}

impl LevelGraphHeader {
    /// Cells in a row along z: what a packed `xz` is divided by.
    #[must_use]
    pub fn row_length(&self) -> u32 {
        i_floor((self.aabb.max.z - self.aabb.min.z) / self.cell_size + EPS_L + 1.5) as u32
    }

    /// Cells along x.
    #[must_use]
    pub fn column_length(&self) -> u32 {
        i_floor((self.aabb.max.x - self.aabb.min.x) / self.cell_size + EPS_L + 1.5) as u32
    }

    /// `CLevelGraph::vertex_position`: the world position of a packed one.
    #[must_use]
    pub fn unpack_position(&self, xz: u32, y: u16) -> Vec3 {
        let row = self.row_length().max(1);
        Vec3::new(
            (xz / row) as f32 * self.cell_size + self.aabb.min.x,
            (f32::from(y) / 65535.0) * self.factor_y + self.aabb.min.y,
            (xz % row) as f32 * self.cell_size + self.aabb.min.z,
        )
    }

    /// `CLevelGraph::vertex_position` the other way: the cell and height of a world position, as
    /// the game looks a position up.
    #[must_use]
    pub fn pack_position(&self, p: Vec3) -> (u32, u16) {
        let row = self.row_length() as i32;
        let pxz = i_floor((p.x - self.aabb.min.x) / self.cell_size + 0.5)
            .wrapping_mul(row)
            .wrapping_add(i_floor((p.z - self.aabb.min.z) / self.cell_size + 0.5));
        let py = i_floor(65535.0 * (p.y - self.aabb.min.y) / self.factor_y + EPS_S).clamp(0, 65535);
        (pxz as u32, py as u16)
    }

    /// `CNodePositionCompressor`: the cell and height the level graph builders give a node.
    /// Differs from [`Self::pack_position`] by an epsilon inside the floor.
    #[must_use]
    pub fn compress_position(&self, p: Vec3, patch_size: f32) -> (u32, u16) {
        let sp = 1.0 / patch_size;
        let row = self.row_length() as i32;
        let pxz = i_floor((p.x - self.aabb.min.x) * sp + EPS_L + 0.5)
            .wrapping_mul(row)
            .wrapping_add(i_floor((p.z - self.aabb.min.z) * sp + EPS_L + 0.5));
        let py = i_floor(65535.0 * (p.y - self.aabb.min.y) / self.factor_y + EPS_L).clamp(0, 65535);
        (pxz as u32, py as u16)
    }
}

/// One cell of the graph, the same for every layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiNode {
    /// The neighbours: left, forward, right, back. [`NodeLayout::invalid_link`] for none.
    pub links: [u32; 4],
    /// Cover from the four sides at standing height, 0..=15.
    pub high_cover: [u8; 4],
    /// Cover at crouching height. The same as `high_cover` in the versions with one set.
    pub low_cover: [u8; 4],
    /// The cell's plane normal, [`crate::pv_compress`]ed.
    pub plane: u16,
    /// `x * row_length + z` in cells.
    pub xz: u32,
    /// The height, over the header's `factor_y`.
    pub y: u16,
    /// The bits of the link block no link uses, as the file has them. The compilers leave
    /// stack garbage there; a node made here has 0.
    pub spare: u8,
}

fn cover_from(word: u16) -> [u8; 4] {
    [(word & 0xf) as u8, ((word >> 4) & 0xf) as u8, ((word >> 8) & 0xf) as u8, ((word >> 12) & 0xf) as u8]
}

fn cover_to(c: [u8; 4]) -> u16 {
    u16::from(c[0] & 0xf)
        | (u16::from(c[1] & 0xf) << 4)
        | (u16::from(c[2] & 0xf) << 8)
        | (u16::from(c[3] & 0xf) << 12)
}

impl AiNode {
    /// Decodes one node of `layout.size` bytes.
    #[must_use]
    pub fn unpack(rec: &[u8], layout: &NodeLayout) -> Self {
        // the link block as one little-endian number
        let mut raw = [0u8; 16];
        if let (Some(dst), Some(src)) = (raw.get_mut(..layout.link_bytes), rec.get(..layout.link_bytes)) {
            dst.copy_from_slice(src);
        }
        let block = u128::from_le_bytes(raw);
        let mask = u128::from(layout.invalid_link());
        let mut links = [0u32; 4];
        for (i, link) in links.iter_mut().enumerate() {
            *link = ((block >> (layout.link_bits * i as u32)) & mask) as u32;
        }
        let spare = if layout.spare_bits() == 0 { 0 } else { (block >> (layout.link_bits * 4)) as u8 };

        let mut at = layout.link_bytes;
        let high = cover_from(u16_at(rec, at));
        at += 2;
        let low = if layout.two_covers {
            at += 2;
            cover_from(u16_at(rec, at - 2))
        } else {
            high
        };
        let plane = u16_at(rec, at);
        at += 2;
        let (xz, y) = if layout.wide_xz {
            (u32_at(rec, at), u16_at(rec, at + 4))
        } else {
            (u32_at(rec, at) & 0x00ff_ffff, u16_at(rec, at + 3))
        };
        Self { links, high_cover: high, low_cover: low, plane, xz, y, spare }
    }

    /// Encodes the node into `layout.size` bytes. Links are cut to the layout's width.
    pub fn pack(&self, rec: &mut [u8], layout: &NodeLayout) {
        let mask = u128::from(layout.invalid_link());
        let mut block = 0u128;
        for (i, link) in self.links.iter().enumerate() {
            block |= (u128::from(*link) & mask) << (layout.link_bits * i as u32);
        }
        if layout.spare_bits() != 0 {
            let spare_mask = (1u128 << layout.spare_bits()) - 1;
            block |= (u128::from(self.spare) & spare_mask) << (layout.link_bits * 4);
        }
        let raw = block.to_le_bytes();
        if let (Some(dst), Some(src)) = (rec.get_mut(..layout.link_bytes), raw.get(..layout.link_bytes)) {
            dst.copy_from_slice(src);
        }

        let mut at = layout.link_bytes;
        put_u16(rec, at, cover_to(self.high_cover));
        at += 2;
        if layout.two_covers {
            put_u16(rec, at, cover_to(self.low_cover));
            at += 2;
        }
        put_u16(rec, at, self.plane);
        at += 2;
        if layout.wide_xz {
            put_u32(rec, at, self.xz);
            put_u16(rec, at + 4, self.y);
        } else {
            // three bytes of the cell index, then the height over the fourth
            let xz = self.xz.to_le_bytes();
            if let Some(dst) = rec.get_mut(at..at + 3) {
                dst.copy_from_slice(&[xz[0], xz[1], xz[2]]);
            }
            put_u16(rec, at + 3, self.y);
        }
    }
}

/// A whole `level.ai`.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelGraph {
    pub header: LevelGraphHeader,
    pub nodes: Arc<[AiNode]>,
}

impl LevelGraph {
    /// Reads a `level.ai`. Nodes are decoded on the rayon pool.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let Some(head) = bytes.get(..HEADER) else {
            return Err(LevelError::Invalid("the AI map is shorter than its header".into()));
        };
        let version = u32_at(head, 0);
        let Some(layout) = NodeLayout::of(version) else {
            return Err(LevelError::Version { what: "AI map", found: version });
        };
        let count = u32_at(head, 4);
        let mut guid = [0u8; 16];
        if let Some(src) = head.get(40..56) {
            guid.copy_from_slice(src);
        }
        let header = LevelGraphHeader {
            version,
            cell_size: f32_at(head, 8),
            factor_y: f32_at(head, 12),
            aabb: aabb_at(head, 16),
            guid,
        };
        let body = bytes.get(HEADER..).unwrap_or_default();
        let n = fits(body.len(), u64::from(count), layout.size as u64, "AI map nodes")?;
        let nodes: Vec<AiNode> = body
            .get(..n * layout.size)
            .unwrap_or_default()
            .par_chunks_exact(layout.size)
            .with_min_len(8192)
            .map(|rec| AiNode::unpack(rec, &layout))
            .collect();
        Ok(Self { header, nodes: nodes.into() })
    }

    /// The file, in the layout of the header's version.
    pub fn write(&self) -> Result<Vec<u8>> {
        let Some(layout) = NodeLayout::of(self.header.version) else {
            return Err(LevelError::Version { what: "AI map", found: self.header.version });
        };
        let mut out = Vec::with_capacity(HEADER + self.nodes.len() * layout.size);
        out.extend_from_slice(&self.header.version.to_le_bytes());
        out.extend_from_slice(&(self.nodes.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.header.cell_size.to_le_bytes());
        out.extend_from_slice(&self.header.factor_y.to_le_bytes());
        push_aabb(&mut out, &self.header.aabb);
        out.extend_from_slice(&self.header.guid);
        out.resize(HEADER + self.nodes.len() * layout.size, 0);
        if let Some(body) = out.get_mut(HEADER..) {
            body.par_chunks_exact_mut(layout.size)
                .with_min_len(8192)
                .zip(self.nodes.par_iter())
                .for_each(|(rec, node)| node.pack(rec, &layout));
        }
        Ok(out)
    }

    /// The world position of a node.
    #[must_use]
    pub fn position(&self, node: &AiNode) -> Vec3 {
        self.header.unpack_position(node.xz, node.y)
    }

    /// The node of a cell index, when the map has one: nodes are sorted by `xz`, and the first
    /// of several stacked over one cell is returned.
    #[must_use]
    pub fn first_at(&self, xz: u32) -> Option<usize> {
        let at = self.nodes.partition_point(|n| n.xz < xz);
        self.nodes.get(at).is_some_and(|n| n.xz == xz).then_some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(version: u32) -> LevelGraphHeader {
        LevelGraphHeader {
            version,
            cell_size: 0.7,
            factor_y: 12.5,
            aabb: Aabb::new(Vec3::new(-10.0, 1.0, -20.0), Vec3::new(30.0, 13.5, 15.0)),
            guid: *b"0123456789abcdef",
        }
    }

    fn node(layout: &NodeLayout) -> AiNode {
        let none = layout.invalid_link();
        AiNode {
            links: [5, none, none - 1, 0x12345 & none],
            high_cover: [1, 15, 0, 9],
            low_cover: if layout.two_covers { [2, 3, 4, 5] } else { [1, 15, 0, 9] },
            plane: 0x7e,
            xz: if layout.wide_xz { 0x0123_4567 } else { 0x0034_5678 },
            y: 0xbeef,
            spare: if layout.spare_bits() == 0 { 0 } else { 0xa },
        }
    }

    #[test]
    fn every_layout_packs_and_unpacks() {
        for version in 8..=13 {
            let layout = NodeLayout::of(version).unwrap();
            let n = node(&layout);
            let mut rec = vec![0u8; layout.size];
            n.pack(&mut rec, &layout);
            assert_eq!(AiNode::unpack(&rec, &layout), n, "version {version}");
            // the bytes survive a second trip
            let mut again = vec![0xffu8; layout.size];
            AiNode::unpack(&rec, &layout).pack(&mut again, &layout);
            assert_eq!(again, rec, "version {version}");
        }
        assert!(NodeLayout::of(7).is_none());
        assert!(NodeLayout::of(14).is_none());
    }

    #[test]
    fn links_lie_where_the_engine_reads_them() {
        // NodeCompressed::link of version 10: bits 0, 23, 46 and 69 of the 12-byte block
        let layout = NodeLayout::of(10).unwrap();
        let n = AiNode { links: [0x7f_ffff, 0, 0x7f_ffff, 0], spare: 0xf, ..AiNode::default() };
        let mut rec = vec![0u8; layout.size];
        n.pack(&mut rec, &layout);
        let word = |at: usize| u32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]]);
        assert_eq!(word(0) & 0x007f_ffff, 0x7f_ffff);
        assert_eq!((word(2) >> 7) & 0x007f_ffff, 0);
        assert_eq!((word(5) >> 6) & 0x007f_ffff, 0x7f_ffff);
        assert_eq!((word(8) >> 5) & 0x007f_ffff, 0);
        assert_eq!(rec[11] >> 4, 0xf);
        // version 13: 26 bits each, shifts 0, 2, 4, 6 at bytes 0, 3, 6, 9
        let layout = NodeLayout::of(13).unwrap();
        let n = AiNode { links: [1, 0x3ff_ffff, 2, 0x200_0001], ..AiNode::default() };
        let mut rec = vec![0u8; layout.size];
        n.pack(&mut rec, &layout);
        let word = |at: usize| u32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]]);
        assert_eq!(word(0) & 0x3ff_ffff, 1);
        assert_eq!((word(3) >> 2) & 0x3ff_ffff, 0x3ff_ffff);
        assert_eq!((word(6) >> 4) & 0x3ff_ffff, 2);
        assert_eq!((word(9) >> 6) & 0x3ff_ffff, 0x200_0001);
    }

    #[test]
    fn a_file_round_trips_in_every_version() {
        for version in [8, 10, 11, 12, 13] {
            let layout = NodeLayout::of(version).unwrap();
            let mut nodes = vec![node(&layout); 3];
            nodes[1].xz += 1;
            nodes[2].xz += 2;
            let graph = LevelGraph { header: header(version), nodes: nodes.into() };
            let bytes = graph.write().unwrap();
            assert_eq!(bytes.len(), 56 + 3 * layout.size);
            let back = LevelGraph::read(&bytes).unwrap();
            assert_eq!(back, graph);
            assert_eq!(back.write().unwrap(), bytes);
            assert_eq!(back.first_at(graph.nodes[1].xz), Some(1));
            assert_eq!(back.first_at(1), None);
        }
    }

    #[test]
    fn positions_pack_into_cells() {
        let h = header(10);
        // (15 + 20) / 0.7 + 1.5 = 51.5
        assert_eq!(h.row_length(), 51);
        assert_eq!(h.column_length(), 58);
        let p = Vec3::new(-10.0 + 0.7 * 4.0, 1.0 + 12.5 * 0.5, -20.0 + 0.7 * 9.0);
        let (xz, y) = h.pack_position(p);
        assert_eq!(xz, 4 * 51 + 9);
        assert!((i32::from(y) - 32767).abs() <= 1);
        assert_eq!(h.compress_position(p, 0.7).0, xz);
        let back = h.unpack_position(xz, y);
        assert!((back - p).length() < 1e-3, "{back:?}");
    }

    #[test]
    fn hostile_input_is_an_error() {
        let layout = NodeLayout::of(10).unwrap();
        let graph = LevelGraph { header: header(10), nodes: vec![node(&layout); 2].into() };
        let bytes = graph.write().unwrap();
        assert!(LevelGraph::read(&bytes[..40]).is_err());
        assert!(LevelGraph::read(&bytes[..bytes.len() - 1]).is_err());
        let mut huge = bytes.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(LevelGraph::read(&huge).is_err());
        let mut old = bytes;
        old[0] = 2;
        assert!(matches!(LevelGraph::read(&old), Err(LevelError::Version { found: 2, .. })));
        assert!(LevelGraph { header: header(99), nodes: Arc::from([]) }.write().is_err());
    }
}

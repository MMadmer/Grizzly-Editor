//! The game graph (`game.graph`, or chunk 4 of `all.spawn`) and the level cross tables
//! (`level.gct`, or the tail of the game graph).

use std::sync::Arc;

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::bytes::Bytes;
use crate::{Result, SpawnError, damaged};

/// Two `u64`, kept as the sixteen bytes they are.
pub type Guid = [u8; 16];

const VERTEX_SIZE: usize = 42;
const EDGE_SIZE: usize = 6;
const POINT_SIZE: usize = 20;
/// Version, two counts, two guids.
pub const CROSS_TABLE_HEADER_SIZE: usize = 44;
const CELL_SIZE: usize = 6;

pub(crate) fn short(what: &'static str, r: &Reader<'_>, n: usize) -> SpawnError {
    SpawnError::Short { what, wanted: n.saturating_sub(r.elapsed()) }
}

pub(crate) fn guid(r: &mut Reader<'_>) -> Result<Guid> {
    let mut g = [0u8; 16];
    g.copy_from_slice(r.take(16)?);
    Ok(g)
}

/// A level of the game graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphLevel {
    pub name: SmolStr,
    pub offset: Vec3,
    pub id: u8,
    pub section: SmolStr,
    pub guid: Guid,
}

/// A vertex of the game graph (42 bytes, packed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphVertex {
    pub local_point: Vec3,
    pub global_point: Vec3,
    /// `tLevelID:8`.
    pub level_id: u8,
    /// `tNodeID:24`: the level vertex.
    pub level_vertex: u32,
    pub vertex_types: [u8; 4],
    /// A byte offset from the start of the vertex table.
    pub edge_offset: u32,
    pub point_offset: u32,
    pub edge_count: u8,
    pub death_point_count: u8,
}

/// An edge of the game graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphEdge {
    pub vertex_id: u16,
    pub distance: f32,
}

/// A level point of the game graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelPoint {
    pub point: Vec3,
    pub level_vertex: u32,
    pub distance: f32,
}

impl LevelPoint {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { point: r.r_vec3()?, level_vertex: r.r_u32()?, distance: r.r_f32()? })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_vec3(self.point);
        w.w_u32(self.level_vertex);
        w.w_f32(self.distance);
    }

    /// `u32 count` and the points.
    pub(crate) fn read_vec(r: &mut Reader<'_>, what: &'static str) -> Result<Vec<Self>> {
        let n = r.r_u32()? as usize;
        if n.saturating_mul(POINT_SIZE) > r.elapsed() {
            return Err(short(what, r, n.saturating_mul(POINT_SIZE)));
        }
        (0..n).map(|_| Self::read(r)).collect()
    }
}

/// A level's cross table: for every level vertex the nearest game vertex and the distance.
#[derive(Clone, Debug, PartialEq)]
pub struct CrossTable {
    pub version: u32,
    pub level_vertex_count: u32,
    pub game_vertex_count: u32,
    pub level_guid: Guid,
    pub game_guid: Guid,
    /// The cells, six bytes each (`u16` game vertex, `f32` distance), as the file holds them.
    pub cells: Bytes,
}

impl CrossTable {
    fn read_header(r: &mut Reader<'_>) -> Result<(u32, u32, u32, Guid, Guid)> {
        Ok((r.r_u32()?, r.r_u32()?, r.r_u32()?, guid(r)?, guid(r)?))
    }

    fn write_header(&self, w: &mut Writer) {
        w.w_u32(self.version);
        w.w_u32(self.level_vertex_count);
        w.w_u32(self.game_vertex_count);
        w.w(&self.level_guid);
        w.w(&self.game_guid);
    }

    /// The game vertex and the distance of a level vertex.
    #[must_use]
    pub fn cell(&self, level_vertex: u32) -> Option<(u16, f32)> {
        let at = (level_vertex as usize).checked_mul(CELL_SIZE)?;
        let b = self.cells.get(at..at + CELL_SIZE)?;
        Some((u16::from_le_bytes([b[0], b[1]]), f32::from_le_bytes([b[2], b[3], b[4], b[5]])))
    }

    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len() / CELL_SIZE
    }

    /// `level.gct`: chunk 0 the header, chunk 1 the cells.
    pub fn parse_gct(buf: &Arc<[u8]>) -> Result<Self> {
        let r = Reader::new(buf);
        let mut head = r.sub(0)?.ok_or(SpawnError::Missing("cross table header"))?;
        let (version, level_vertex_count, game_vertex_count, level_guid, game_guid) =
            Self::read_header(&mut head)?;
        let cells = r.sub(1)?.ok_or(SpawnError::Missing("cross table data"))?;
        Ok(Self {
            version,
            level_vertex_count,
            game_vertex_count,
            level_guid,
            game_guid,
            cells: Bytes::of_part(buf, cells.data()),
        })
    }

    #[must_use]
    pub fn to_gct(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(self.cells.len() + 64);
        w.chunk(0, |w| self.write_header(w));
        w.w_chunk(1, &self.cells);
        w.into_bytes()
    }
}

/// The game graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GameGraph {
    pub version: u8,
    /// The counts as the header states them.
    pub vertex_count: u16,
    pub edge_count: u32,
    pub death_point_count: u32,
    pub guid: Guid,
    pub levels: Vec<GraphLevel>,
    pub vertices: Vec<GraphVertex>,
    pub edges: Vec<GraphEdge>,
    pub death_points: Vec<LevelPoint>,
    /// One per level, in the order of `levels` (`XRAI_VERSION_PRIQUEL` and later).
    pub cross_tables: Vec<CrossTable>,
    /// Bytes after the last cross table, kept.
    pub tail: Bytes,
}

impl GameGraph {
    /// Reads a graph from `body`, which is `buf` or a part of it.
    pub fn parse(buf: &Arc<[u8]>, body: &[u8], with_cross_tables: bool) -> Result<Self> {
        let mut r = Reader::new(body);
        let version = r.r_u8()?;
        let vertex_count = r.r_u16()?;
        let edge_count = r.r_u32()?;
        let death_point_count = r.r_u32()?;
        let guid = guid(&mut r)?;
        let level_count = usize::from(r.r_u8()?);
        let mut levels = Vec::with_capacity(level_count);
        for _ in 0..level_count {
            let name = SmolStr::new(gz_base::text::cp1251_to_utf8(r.r_stringz_raw()?));
            let offset = r.r_vec3()?;
            let id = r.r_u8()?;
            let section = SmolStr::new(gz_base::text::cp1251_to_utf8(r.r_stringz_raw()?));
            levels.push(GraphLevel { name, offset, id, section, guid: guid_of(&mut r)? });
        }
        let need = usize::from(vertex_count) * VERTEX_SIZE
            + (edge_count as usize).saturating_mul(EDGE_SIZE)
            + (death_point_count as usize).saturating_mul(POINT_SIZE);
        if need > r.elapsed() {
            return Err(short("game graph", &r, need));
        }
        let mut vertices = Vec::with_capacity(usize::from(vertex_count));
        for _ in 0..vertex_count {
            let local_point = r.r_vec3()?;
            let global_point = r.r_vec3()?;
            let packed = r.r_u32()?;
            let mut vertex_types = [0u8; 4];
            vertex_types.copy_from_slice(r.take(4)?);
            vertices.push(GraphVertex {
                local_point,
                global_point,
                level_id: (packed & 0xff) as u8,
                level_vertex: packed >> 8,
                vertex_types,
                edge_offset: r.r_u32()?,
                point_offset: r.r_u32()?,
                edge_count: r.r_u8()?,
                death_point_count: r.r_u8()?,
            });
        }
        let mut edges = Vec::with_capacity(edge_count as usize);
        for _ in 0..edge_count {
            edges.push(GraphEdge { vertex_id: r.r_u16()?, distance: r.r_f32()? });
        }
        let mut death_points = Vec::with_capacity(death_point_count as usize);
        for _ in 0..death_point_count {
            death_points.push(LevelPoint::read(&mut r)?);
        }
        let mut cross_tables = Vec::new();
        if with_cross_tables {
            cross_tables.reserve(levels.len());
            for _ in 0..levels.len() {
                let size = r.r_u32()? as usize;
                let body_size = size
                    .checked_sub(4 + CROSS_TABLE_HEADER_SIZE)
                    .ok_or_else(|| damaged("cross table", format!("size {size}")))?;
                let (version, level_vertex_count, game_vertex_count, level_guid, game_guid) =
                    CrossTable::read_header(&mut r)?;
                let cells = r.take(body_size)?;
                cross_tables.push(CrossTable {
                    version,
                    level_vertex_count,
                    game_vertex_count,
                    level_guid,
                    game_guid,
                    cells: Bytes::of_part(buf, cells),
                });
            }
        }
        let tail = Bytes::of_part(buf, r.rest());
        Ok(Self {
            version,
            vertex_count,
            edge_count,
            death_point_count,
            guid,
            levels,
            vertices,
            edges,
            death_points,
            cross_tables,
            tail,
        })
    }

    /// `game.graph` as a file of its own.
    pub fn parse_file(buf: &Arc<[u8]>) -> Result<Self> {
        Self::parse(buf, buf, true)
    }

    pub fn write(&self, w: &mut Writer) {
        w.w_u8(self.version);
        w.w_u16(self.vertex_count);
        w.w_u32(self.edge_count);
        w.w_u32(self.death_point_count);
        w.w(&self.guid);
        w.w_u8(self.levels.len() as u8);
        for l in &self.levels {
            w.w_stringz_raw(&encode(&l.name));
            w.w_vec3(l.offset);
            w.w_u8(l.id);
            w.w_stringz_raw(&encode(&l.section));
            w.w(&l.guid);
        }
        for v in &self.vertices {
            w.w_vec3(v.local_point);
            w.w_vec3(v.global_point);
            w.w_u32(u32::from(v.level_id) | (v.level_vertex << 8));
            w.w(&v.vertex_types);
            w.w_u32(v.edge_offset);
            w.w_u32(v.point_offset);
            w.w_u8(v.edge_count);
            w.w_u8(v.death_point_count);
        }
        for e in &self.edges {
            w.w_u16(e.vertex_id);
            w.w_f32(e.distance);
        }
        for p in &self.death_points {
            p.write(w);
        }
        for t in &self.cross_tables {
            w.w_u32((4 + CROSS_TABLE_HEADER_SIZE + t.cells.len()) as u32);
            t.write_header(w);
            w.w(&t.cells);
        }
        w.w(&self.tail);
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(self.vertices.len() * VERTEX_SIZE + 4096);
        self.write(&mut w);
        w.into_bytes()
    }

    #[must_use]
    pub fn level(&self, id: u8) -> Option<&GraphLevel> {
        self.levels.iter().find(|l| l.id == id)
    }

    /// The level a game vertex lies on.
    #[must_use]
    pub fn level_of_vertex(&self, vertex: u16) -> Option<&GraphLevel> {
        self.level(self.vertices.get(usize::from(vertex))?.level_id)
    }

    /// The edges of a vertex: `edge_offset` counts bytes from the start of the vertex table.
    #[must_use]
    pub fn edges_of(&self, vertex: u16) -> &[GraphEdge] {
        let Some(v) = self.vertices.get(usize::from(vertex)) else { return &[] };
        let base = self.vertices.len() * VERTEX_SIZE;
        let Some(first) =
            (v.edge_offset as usize).checked_sub(base).filter(|o| o % EDGE_SIZE == 0).map(|o| o / EDGE_SIZE)
        else {
            return &[];
        };
        self.edges.get(first..first + usize::from(v.edge_count)).unwrap_or(&[])
    }

    /// The death points of a vertex: `point_offset` counts bytes from the start of the vertex
    /// table, after the vertices and the edges.
    #[must_use]
    pub fn points_of(&self, vertex: u16) -> &[LevelPoint] {
        let Some(v) = self.vertices.get(usize::from(vertex)) else { return &[] };
        let base = self.vertices.len() * VERTEX_SIZE + self.edges.len() * EDGE_SIZE;
        let Some(first) = (v.point_offset as usize)
            .checked_sub(base)
            .filter(|o| o % POINT_SIZE == 0)
            .map(|o| o / POINT_SIZE)
        else {
            return &[];
        };
        self.death_points.get(first..first + usize::from(v.death_point_count)).unwrap_or(&[])
    }

    /// The cross table of a level.
    #[must_use]
    pub fn cross_table(&self, level_id: u8) -> Option<&CrossTable> {
        let i = self.levels.iter().position(|l| l.id == level_id)?;
        self.cross_tables.get(i)
    }
}

fn guid_of(r: &mut Reader<'_>) -> Result<Guid> {
    guid(r)
}

/// Text back to the cp1251 bytes it was read from: every byte has a character, so a name
/// that was read writes back as it was.
pub(crate) fn encode(text: &str) -> Vec<u8> {
    gz_base::text::utf8_to_cp1251(text).0.into_owned()
}

/// A zero-terminated cp1251 string.
pub(crate) fn r_name(r: &mut Reader<'_>) -> Result<SmolStr> {
    Ok(SmolStr::new(gz_base::text::cp1251_to_utf8(r.r_stringz_raw()?)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_graph() -> GameGraph {
        let cells: Vec<u8> = [(0u16, 0.5f32), (1, 1.5)]
            .iter()
            .flat_map(|(v, d)| {
                let mut b = v.to_le_bytes().to_vec();
                b.extend_from_slice(&d.to_le_bytes());
                b
            })
            .collect();
        GameGraph {
            version: 10,
            vertex_count: 2,
            edge_count: 2,
            death_point_count: 1,
            guid: [7; 16],
            levels: vec![GraphLevel {
                name: "l01_escape".into(),
                offset: Vec3::ZERO,
                id: 0,
                section: "level_l01".into(),
                guid: [1; 16],
            }],
            vertices: vec![
                GraphVertex {
                    local_point: Vec3::new(1.0, 0.0, 0.0),
                    global_point: Vec3::new(1.0, 0.0, 0.0),
                    level_id: 0,
                    level_vertex: 0x12_3456,
                    vertex_types: [1, 2, 3, 4],
                    edge_offset: 84,
                    point_offset: 96,
                    edge_count: 1,
                    death_point_count: 1,
                },
                GraphVertex {
                    local_point: Vec3::ZERO,
                    global_point: Vec3::ZERO,
                    level_id: 0,
                    level_vertex: 1,
                    vertex_types: [0; 4],
                    edge_offset: 90,
                    point_offset: 96,
                    edge_count: 1,
                    death_point_count: 0,
                },
            ],
            edges: vec![GraphEdge { vertex_id: 1, distance: 3.0 }, GraphEdge { vertex_id: 0, distance: 3.0 }],
            death_points: vec![LevelPoint { point: Vec3::ONE, level_vertex: 5, distance: 0.25 }],
            cross_tables: vec![CrossTable {
                version: 10,
                level_vertex_count: 2,
                game_vertex_count: 2,
                level_guid: [1; 16],
                game_guid: [7; 16],
                cells: Bytes::from_vec(cells),
            }],
            tail: Bytes::default(),
        }
    }

    #[test]
    fn a_graph_writes_reads_and_writes_the_same_bytes() {
        let g = sample_graph();
        let bytes: Arc<[u8]> = Arc::from(g.to_bytes());
        // header 1 + 2 + 4 + 4 + 16 + 1, a level, two vertices, two edges, a point, a table
        let level = 11 + 12 + 1 + 10 + 16;
        assert_eq!(bytes.len(), 28 + level + 2 * 42 + 2 * 6 + 20 + 4 + 44 + 12);
        let back = GameGraph::parse_file(&bytes).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.to_bytes(), &bytes[..]);
        assert_eq!(back.edges_of(0), &[GraphEdge { vertex_id: 1, distance: 3.0 }]);
        assert_eq!(back.edges_of(1)[0].vertex_id, 0);
        assert_eq!(back.level_of_vertex(1).map(|l| l.name.as_str()), Some("l01_escape"));
        assert_eq!(back.cross_table(0).and_then(|t| t.cell(1)), Some((1, 1.5)));
        assert_eq!(back.cross_table(0).and_then(|t| t.cell(2)), None);
    }

    #[test]
    fn the_level_and_node_share_one_word() {
        let g = sample_graph();
        let b = g.to_bytes();
        let at = 28 + 50 + 24;
        assert_eq!(u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]), 0x1234_5600);
    }

    #[test]
    fn a_gct_round_trips_and_a_short_graph_is_an_error() {
        let t = sample_graph().cross_tables.remove(0);
        let file: Arc<[u8]> = Arc::from(t.to_gct());
        assert_eq!(CrossTable::parse_gct(&file).unwrap(), t);
        let g: Arc<[u8]> = Arc::from(sample_graph().to_bytes());
        let cut: Arc<[u8]> = Arc::from(&g[..60]);
        assert!(GameGraph::parse_file(&cut).is_err());
        // a vertex count the file cannot hold is refused before anything is reserved
        let mut big = g.to_vec();
        big[1] = 0xff;
        big[2] = 0xff;
        assert!(GameGraph::parse_file(&Arc::from(big)).is_err());
    }
}

//! `all.spawn` / `*.spawn`: the file the spawn constructor writes and the game's
//! spawn registry loads.
//!
//! Chunk 0 the header, 1 the spawn graph (per vertex its id and the entity wrapper:
//! the spawn packet and the update packet, each after a `u16` length), 2 the artefact spawn
//! points, 3 the patrol paths, 4 the game graph (`XRAI_VERSION_PRIQUEL` and later). Any other
//! chunk is kept as it is.

use std::sync::Arc;

use gz_chunk::{Reader, Writer};
use rayon::prelude::*;

use crate::bytes::Bytes;
use crate::graph::{GameGraph, Guid, LevelPoint, guid};
use crate::patrol::{PatrolPath, numbered, read_storage, write_storage};
use crate::{Result, SpawnError, damaged};

/// The first version whose spawn file embeds the game graph.
pub const XRAI_VERSION_PRIQUEL: u32 = 9;

/// The spawn header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnHeader {
    pub version: u32,
    pub guid: Guid,
    pub graph_guid: Guid,
    pub spawn_count: u32,
    pub level_count: u32,
}

/// One vertex of the spawn graph: an entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnVertex {
    /// A spawn id.
    pub id: u16,
    /// The spawn packet, `M_SPAWN` word first.
    pub spawn: Bytes,
    /// The update packet, `M_UPDATE` word first.
    pub update: Bytes,
}

/// The edges of one spawn vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnEdge {
    pub from: u16,
    pub to: Vec<(u16, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Header,
    Spawns,
    Artefacts,
    Patrols,
    Graph,
    Raw(Bytes),
}

/// A spawn file.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnFile {
    pub header: SpawnHeader,
    /// The vertex count the spawn graph's own header states.
    pub declared_vertices: u32,
    pub vertices: Vec<SpawnVertex>,
    pub edges: Vec<SpawnEdge>,
    pub has_edges_chunk: bool,
    pub artefact_points: Vec<LevelPoint>,
    pub declared_patrols: u32,
    pub patrols: Vec<PatrolPath>,
    pub game_graph: Option<GameGraph>,
    /// Top-level chunks in file order: id, compressed, what it holds.
    layout: Vec<(u32, bool, Part)>,
}

fn need<'a>(r: &Reader<'a>, id: u32, what: &'static str) -> Result<Reader<'a>> {
    r.sub(id)?.ok_or(SpawnError::Missing(what))
}

/// `u16 size` and the packet, the whole chunk.
fn packet(buf: &Arc<[u8]>, body: &[u8], what: &'static str) -> Result<Bytes> {
    let mut r = Reader::new(body);
    let n = usize::from(r.r_u16()?);
    if n != r.elapsed() {
        return Err(damaged(what, format!("{n} bytes stated, {} there", r.elapsed())));
    }
    Ok(Bytes::of_part(buf, r.rest()))
}

fn read_vertex(buf: &Arc<[u8]>, body: &[u8]) -> Result<SpawnVertex> {
    let v = Reader::new(body);
    let id = need(&v, 0, "spawn vertex id")?.r_u16()?;
    let data = need(&v, 1, "spawn vertex data")?;
    let spawn = packet(buf, need(&data, 0, "spawn packet")?.data(), "spawn packet")?;
    let update = packet(buf, need(&data, 1, "update packet")?.data(), "update packet")?;
    Ok(SpawnVertex { id, spawn, update })
}

impl SpawnFile {
    /// Reads the whole file. Packets and tables are ranges of `buf`, not copies.
    pub fn parse(buf: Arc<[u8]>) -> Result<Self> {
        let mut file = Self {
            header: SpawnHeader {
                version: 0,
                guid: [0; 16],
                graph_guid: [0; 16],
                spawn_count: 0,
                level_count: 0,
            },
            declared_vertices: 0,
            vertices: Vec::new(),
            edges: Vec::new(),
            has_edges_chunk: false,
            artefact_points: Vec::new(),
            declared_patrols: 0,
            patrols: Vec::new(),
            game_graph: None,
            layout: Vec::new(),
        };
        let mut seen = [false; 5];
        let mut graph_body = None;
        for (id, compressed, body) in Reader::new(&buf).chunks() {
            // a compressed chunk is decompressed into a buffer of its own
            let (owner, body): (Arc<[u8]>, &[u8]) = if compressed {
                (Arc::from(gz_chunk::lzhuf::decompress(body)?), &[])
            } else {
                (buf.clone(), body)
            };
            let body = if compressed { &owner[..] } else { body };
            let slot = usize::try_from(id).ok().filter(|&i| i < 5 && !seen[i]);
            let part = match slot {
                Some(0) => {
                    let mut r = Reader::new(body);
                    file.header = SpawnHeader {
                        version: r.r_u32()?,
                        guid: guid(&mut r)?,
                        graph_guid: guid(&mut r)?,
                        spawn_count: r.r_u32()?,
                        level_count: r.r_u32()?,
                    };
                    Part::Header
                }
                Some(1) => {
                    let r = Reader::new(body);
                    file.declared_vertices = need(&r, 0, "spawn graph header")?.r_u32()?;
                    let bodies = numbered(&need(&r, 1, "spawn graph vertices")?, "spawn graph vertices")?;
                    file.vertices =
                        bodies.par_iter().map(|b| read_vertex(&owner, b)).collect::<Result<Vec<_>>>()?;
                    if let Some(mut e) = r.sub(2)? {
                        file.has_edges_chunk = true;
                        while !e.eof() {
                            let from = e.r_u16()?;
                            let n = e.r_u32()? as usize;
                            if n.saturating_mul(6) > e.elapsed() {
                                return Err(SpawnError::Short {
                                    what: "spawn graph edges",
                                    wanted: n * 6 - e.elapsed(),
                                });
                            }
                            let to =
                                (0..n).map(|_| Ok((e.r_u16()?, e.r_f32()?))).collect::<Result<Vec<_>>>()?;
                            file.edges.push(SpawnEdge { from, to });
                        }
                    }
                    Part::Spawns
                }
                Some(2) => {
                    let mut r = Reader::new(body);
                    file.artefact_points = LevelPoint::read_vec(&mut r, "artefact spawn points")?;
                    if !r.eof() {
                        return Err(damaged("artefact spawn points", "bytes after the last point"));
                    }
                    Part::Artefacts
                }
                Some(3) => {
                    let (declared, paths) = read_storage(body)?;
                    file.declared_patrols = declared;
                    file.patrols = paths;
                    Part::Patrols
                }
                Some(4) if file.header.version >= XRAI_VERSION_PRIQUEL => {
                    graph_body = Some(Bytes::of_part(&owner, body));
                    Part::Graph
                }
                _ => Part::Raw(Bytes::of_part(&owner, body)),
            };
            if let Some(i) = slot
                && !matches!(part, Part::Raw(_))
            {
                seen[i] = true;
            }
            file.layout.push((id, compressed, part));
        }
        if !seen[0] {
            return Err(SpawnError::Missing("spawn header"));
        }
        if let Some(g) = graph_body {
            file.game_graph = Some(GameGraph::parse(g.buf(), &g, true)?);
        }
        Ok(file)
    }

    /// The file, byte for byte as it was read when nothing was changed.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let size: usize = self.vertices.iter().map(|v| v.spawn.len() + v.update.len() + 64).sum();
        let mut w = Writer::with_capacity(size + (1 << 16));
        for (id, compressed, part) in &self.layout {
            let mut body = Writer::new();
            match part {
                Part::Header => {
                    let h = &self.header;
                    body.w_u32(h.version);
                    body.w(&h.guid);
                    body.w(&h.graph_guid);
                    body.w_u32(h.spawn_count);
                    body.w_u32(h.level_count);
                }
                Part::Spawns => self.write_spawns(&mut body),
                Part::Artefacts => {
                    body.w_u32(self.artefact_points.len() as u32);
                    for p in &self.artefact_points {
                        p.write(&mut body);
                    }
                }
                Part::Patrols => write_storage(self.declared_patrols, &self.patrols, &mut body),
                Part::Graph => {
                    if let Some(g) = &self.game_graph {
                        g.write(&mut body);
                    }
                }
                Part::Raw(b) => body.w(b),
            }
            let body = body.into_bytes();
            if *compressed {
                w.w_chunk_compressed(*id, &body);
            } else {
                w.w_chunk(*id, &body);
            }
        }
        w.into_bytes()
    }

    fn write_spawns(&self, w: &mut Writer) {
        w.chunk(0, |w| w.w_u32(self.declared_vertices));
        w.chunk(1, |w| {
            for (i, v) in self.vertices.iter().enumerate() {
                w.chunk(i as u32, |w| {
                    w.chunk(0, |w| w.w_u16(v.id));
                    w.chunk(1, |w| {
                        w.chunk(0, |w| {
                            w.w_u16(v.spawn.len() as u16);
                            w.w(&v.spawn);
                        });
                        w.chunk(1, |w| {
                            w.w_u16(v.update.len() as u16);
                            w.w(&v.update);
                        });
                    });
                });
            }
        });
        if self.has_edges_chunk {
            w.chunk(2, |w| {
                for e in &self.edges {
                    w.w_u16(e.from);
                    w.w_u32(e.to.len() as u32);
                    for (to, weight) in &e.to {
                        w.w_u16(*to);
                        w.w_f32(*weight);
                    }
                }
            });
        }
    }

    /// A new file in the layout `save_spawn` writes.
    #[must_use]
    pub fn new(header: SpawnHeader, game_graph: Option<GameGraph>) -> Self {
        let mut layout = vec![
            (0, false, Part::Header),
            (1, false, Part::Spawns),
            (2, false, Part::Artefacts),
            (3, false, Part::Patrols),
        ];
        if game_graph.is_some() {
            layout.push((4, false, Part::Graph));
        }
        Self {
            header,
            declared_vertices: 0,
            vertices: Vec::new(),
            edges: Vec::new(),
            has_edges_chunk: true,
            artefact_points: Vec::new(),
            declared_patrols: 0,
            patrols: Vec::new(),
            game_graph,
            layout,
        }
    }

    /// The vertex of a spawn id.
    #[must_use]
    pub fn vertex(&self, id: u16) -> Option<&SpawnVertex> {
        self.vertices.iter().find(|v| v.id == id)
    }

    /// The chunks kept as bytes, with their ids.
    pub fn raw_chunks(&self) -> impl Iterator<Item = (u32, &[u8])> {
        self.layout.iter().filter_map(|(id, _, p)| match p {
            Part::Raw(b) => Some((*id, &b[..])),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::sample_graph;
    use crate::patrol::tests::sample_path;
    use crate::peek::tests::packet;

    fn update() -> Bytes {
        Bytes::from_vec(vec![0, 0, 0])
    }

    pub(crate) fn two_entities() -> SpawnFile {
        let mut f = SpawnFile::new(
            SpawnHeader { version: 10, guid: [3; 16], graph_guid: [7; 16], spawn_count: 2, level_count: 1 },
            Some(sample_graph()),
        );
        f.declared_vertices = 2;
        f.vertices = vec![
            SpawnVertex {
                id: 0,
                spawn: Bytes::from_vec(packet("stalker", "esc_wolf", 1, 10)),
                update: update(),
            },
            SpawnVertex {
                id: 1,
                spawn: Bytes::from_vec(packet("medkit", "medkit_0001", 0, 11)),
                update: Bytes::from_vec(vec![0, 0]),
            },
        ];
        f.edges = vec![SpawnEdge { from: 0, to: vec![(1, 1.0)] }];
        f.artefact_points = vec![LevelPoint { point: glam::Vec3::ONE, level_vertex: 3, distance: 0.5 }];
        f.declared_patrols = 1;
        f.patrols = vec![sample_path("esc_walk")];
        f
    }

    #[test]
    fn a_two_entity_file_writes_reads_and_writes_the_same_bytes() {
        let f = two_entities();
        let bytes: Arc<[u8]> = Arc::from(f.to_bytes());
        let back = SpawnFile::parse(bytes.clone()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.to_bytes(), &bytes[..]);
        assert_eq!(back.vertex(1).map(|v| v.update.len()), Some(2));
        // the packets are ranges of the file, not copies
        let base = bytes.as_ptr() as usize;
        let at = back.vertices[0].spawn.as_ptr() as usize;
        assert!(at > base && at < base + bytes.len());
    }

    #[test]
    fn an_unknown_chunk_and_a_compressed_one_are_kept() {
        let f = two_entities();
        let mut w = Writer::new();
        let plain = f.to_bytes();
        for (id, _, body) in Reader::new(&plain).chunks() {
            if id == 2 {
                w.w_chunk_compressed(id, body);
            } else {
                w.w_chunk(id, body);
            }
        }
        w.w_chunk(9, b"future");
        let bytes: Arc<[u8]> = Arc::from(w.into_bytes());
        let back = SpawnFile::parse(bytes.clone()).unwrap();
        assert_eq!(back.artefact_points, f.artefact_points);
        assert_eq!(back.raw_chunks().collect::<Vec<_>>(), vec![(9, &b"future"[..])]);
        assert_eq!(back.to_bytes(), &bytes[..]);
    }

    #[test]
    fn a_damaged_file_is_an_error() {
        let bytes = two_entities().to_bytes();
        assert!(matches!(SpawnFile::parse(Arc::from(&bytes[..40])), Err(SpawnError::Missing(_))));
        assert!(matches!(SpawnFile::parse(Arc::from(&b""[..])), Err(SpawnError::Missing(_))));
        // a packet length that disagrees with its chunk
        let mut f = two_entities();
        f.vertices[0].spawn = Bytes::from_vec(vec![1, 0]);
        let mut b = f.to_bytes();
        let at = b.windows(4).position(|w| w == [2, 0, 1, 0]).unwrap_or(0);
        b[at] = 9;
        assert!(SpawnFile::parse(Arc::from(b)).is_err());
    }
}

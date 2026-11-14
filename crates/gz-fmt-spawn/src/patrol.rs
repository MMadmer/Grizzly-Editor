//! Patrol paths: chunk 3 of `all.spawn`: a graph of
//! patrol points.

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::graph::{encode, r_name};
use crate::{Result, SpawnError, damaged};

/// A patrol point.
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolPoint {
    pub name: SmolStr,
    pub position: Vec3,
    pub flags: u32,
    pub level_vertex: u32,
    pub game_vertex: u16,
}

/// The links of one point: where they lead and their weights (probabilities).
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolLink {
    pub from: u32,
    pub to: Vec<(u32, f32)>,
}

/// One patrol path.
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolPath {
    pub name: SmolStr,
    /// The vertex count the path's header states.
    pub declared_points: u32,
    /// Vertex id and point, in file order.
    pub points: Vec<(u32, PatrolPoint)>,
    pub links: Vec<PatrolLink>,
    /// Whether the links chunk exists (the game tolerates a path without one).
    pub has_links_chunk: bool,
}

/// Sub-chunks `0..n` of `r`, in order: an error when one is missing or out of order.
pub(crate) fn numbered<'a>(r: &Reader<'a>, what: &'static str) -> Result<Vec<&'a [u8]>> {
    let mut out = Vec::new();
    for (i, (id, compressed, body)) in r.chunks().enumerate() {
        if compressed || id as usize != i {
            return Err(damaged(what, format!("chunk {id} where {i} belongs")));
        }
        out.push(body);
    }
    Ok(out)
}

fn need<'a>(r: &Reader<'a>, id: u32, what: &'static str) -> Result<Reader<'a>> {
    r.sub(id)?.ok_or(SpawnError::Missing(what))
}

impl PatrolPoint {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            name: r_name(r)?,
            position: r.r_vec3()?,
            flags: r.r_u32()?,
            level_vertex: r.r_u32()?,
            game_vertex: r.r_u16()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.w_stringz_raw(&encode(&self.name));
        w.w_vec3(self.position);
        w.w_u32(self.flags);
        w.w_u32(self.level_vertex);
        w.w_u16(self.game_vertex);
    }
}

impl PatrolPath {
    fn read(name: &[u8], graph: &[u8]) -> Result<Self> {
        let name = r_name(&mut Reader::new(name))?;
        let g = Reader::new(graph);
        let declared_points = need(&g, 0, "patrol path header")?.r_u32()?;
        let vertices = numbered(&need(&g, 1, "patrol path points")?, "patrol path points")?;
        let mut points = Vec::with_capacity(vertices.len());
        for body in vertices {
            let v = Reader::new(body);
            let id = need(&v, 0, "patrol point id")?.r_u32()?;
            let point = PatrolPoint::read(&mut need(&v, 1, "patrol point")?)?;
            points.push((id, point));
        }
        let mut links = Vec::new();
        let edges = g.sub(2)?;
        let has_links_chunk = edges.is_some();
        if let Some(mut e) = edges {
            while !e.eof() {
                let from = e.r_u32()?;
                let n = e.r_u32()? as usize;
                if n.saturating_mul(8) > e.elapsed() {
                    return Err(SpawnError::Short { what: "patrol links", wanted: n * 8 - e.elapsed() });
                }
                let to = (0..n).map(|_| Ok((e.r_u32()?, e.r_f32()?))).collect::<Result<Vec<_>>>()?;
                links.push(PatrolLink { from, to });
            }
        }
        Ok(Self { name, declared_points, points, links, has_links_chunk })
    }

    fn write(&self, w: &mut Writer) {
        w.chunk(0, |w| w.w_stringz_raw(&encode(&self.name)));
        w.chunk(1, |w| {
            w.chunk(0, |w| w.w_u32(self.declared_points));
            w.chunk(1, |w| {
                for (i, (id, point)) in self.points.iter().enumerate() {
                    w.chunk(i as u32, |w| {
                        w.chunk(0, |w| w.w_u32(*id));
                        w.chunk(1, |w| point.write(w));
                    });
                }
            });
            if self.has_links_chunk {
                w.chunk(2, |w| {
                    for link in &self.links {
                        w.w_u32(link.from);
                        w.w_u32(link.to.len() as u32);
                        for (to, weight) in &link.to {
                            w.w_u32(*to);
                            w.w_f32(*weight);
                        }
                    }
                });
            }
        });
    }

    /// The point a vertex id names.
    #[must_use]
    pub fn point(&self, id: u32) -> Option<&PatrolPoint> {
        self.points.iter().find(|(have, _)| *have == id).map(|(_, p)| p)
    }
}

/// Every path of the storage, read in parallel; and the count its header states.
pub(crate) fn read_storage(body: &[u8]) -> Result<(u32, Vec<PatrolPath>)> {
    let r = Reader::new(body);
    let declared = need(&r, 0, "patrol paths header")?.r_u32()?;
    let entries = numbered(&need(&r, 1, "patrol paths")?, "patrol paths")?;
    let paths = entries
        .par_iter()
        .map(|body| {
            let e = Reader::new(body);
            let name = need(&e, 0, "patrol path name")?;
            let graph = need(&e, 1, "patrol path")?;
            PatrolPath::read(name.data(), graph.data())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((declared, paths))
}

pub(crate) fn write_storage(declared: u32, paths: &[PatrolPath], w: &mut Writer) {
    w.chunk(0, |w| w.w_u32(declared));
    w.chunk(1, |w| {
        for (i, path) in paths.iter().enumerate() {
            w.chunk(i as u32, |w| path.write(w));
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_path(name: &str) -> PatrolPath {
        let point = |n: &str, x: f32| PatrolPoint {
            name: n.into(),
            position: Vec3::new(x, 0.0, 1.0),
            flags: 1,
            level_vertex: 10,
            game_vertex: 2,
        };
        PatrolPath {
            name: name.into(),
            declared_points: 2,
            points: vec![(0, point("wp00", 0.0)), (1, point("точка", 5.0))],
            links: vec![PatrolLink { from: 0, to: vec![(1, 1.0)] }],
            has_links_chunk: true,
        }
    }

    #[test]
    fn paths_write_read_and_write_the_same_bytes() {
        let paths = vec![sample_path("esc_walk"), sample_path("esc_guard")];
        let mut w = Writer::new();
        write_storage(2, &paths, &mut w);
        let bytes = w.into_bytes();
        let (declared, back) = read_storage(&bytes).unwrap();
        assert_eq!((declared, &back), (2, &paths));
        let mut again = Writer::new();
        write_storage(declared, &back, &mut again);
        assert_eq!(again.into_bytes(), bytes);
        assert_eq!(back[1].point(1).map(|p| p.name.as_str()), Some("точка"));
    }

    #[test]
    fn a_lying_link_count_is_an_error() {
        let mut w = Writer::new();
        w.chunk(0, |w| w.w_u32(0));
        w.chunk(1, |_| {});
        w.chunk(2, |w| {
            w.w_u32(0);
            w.w_u32(1_000_000);
        });
        let graph = w.into_bytes();
        assert!(matches!(PatrolPath::read(b"p\0", &graph), Err(SpawnError::Short { .. })));
        assert!(read_storage(&[1, 2, 3]).is_err());
    }
}

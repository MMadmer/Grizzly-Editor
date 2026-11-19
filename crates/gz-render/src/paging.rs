//! A mesh too large for one buffer of the device, cut into pages.
//!
//! A mesh is one buffer (`assets.rs`): its indices, its edges, its vertices. A graphics device
//! takes a buffer up to a size of its own, and a level of the game as one model is several times
//! that. Such a mesh is cut into pages, each a buffer's worth with vertices of its own: a part of
//! the mesh becomes pieces, one per page it reaches into, and is drawn piece by piece.

use std::ops::Range;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::mesh::{MeshData, MeshVertex};

/// One buffer's worth of a mesh: its indices (into its own vertices), its edges as pairs of
/// such indices, its vertices.
#[derive(Debug, Default)]
pub(crate) struct PageData {
    pub indices: Vec<u32>,
    pub wire: Vec<u32>,
    pub vertices: Vec<MeshVertex>,
}

#[cfg(test)]
impl PageData {
    /// What the page occupies in its buffer.
    pub fn bytes(&self) -> u64 {
        ((self.indices.len() + self.wire.len()) * size_of::<u32>()
            + self.vertices.len() * size_of::<MeshVertex>()) as u64
    }
}

/// A run of one part of a mesh in one page: of the page's indices and of its edges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub page: u32,
    pub indices: Range<u32>,
    pub wire: Range<u32>,
}

/// A mesh as pages, and per part of the mesh its pieces in the order of its triangles.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct Paged {
    pub pages: Vec<PageData>,
    pub parts: Vec<Vec<Piece>>,
}

/// The most one triangle adds to a page: its indices, three edges nobody had, three vertices
/// nobody had.
const TRIANGLE: u64 = (3 * size_of::<u32>() + 6 * size_of::<u32>() + 3 * size_of::<MeshVertex>()) as u64;

/// Cuts `data` into pages of at most `budget` bytes each (a page holds one triangle at least)
/// and hands each to `full` as it is filled, in order: whoever takes a page may let go of it
/// before the next is made, so a mesh of a gigabyte is never twice in memory. Answers, per part
/// of the mesh, its pieces. Every triangle of every part is in exactly one piece, in its order;
/// an edge is listed once per piece.
pub(crate) fn page_by_page(data: &MeshData, budget: u64, mut full: impl FnMut(PageData)) -> Vec<Vec<Piece>> {
    let mut made = 0u32;
    let mut parts = Vec::with_capacity(data.submeshes.len());
    let mut page = PageData::default();
    let mut used = 0u64;
    // a vertex of the mesh -> its place in the page that is being filled
    let mut local: FxHashMap<u32, u32> = FxHashMap::default();
    let mut edges: FxHashSet<(u32, u32)> = FxHashSet::default();
    for part in &data.submeshes {
        let mut pieces = Vec::new();
        let mut from = (page.indices.len() as u32, page.wire.len() as u32);
        edges.clear();
        let triangles =
            data.indices.get(part.indices.start as usize..part.indices.end as usize).unwrap_or(&[]);
        for triangle in triangles.as_chunks::<3>().0 {
            if used + TRIANGLE > budget && !page.indices.is_empty() {
                // the page is full: the part goes on in the next one
                let to = (page.indices.len() as u32, page.wire.len() as u32);
                if to.0 > from.0 {
                    pieces.push(Piece { page: made, indices: from.0..to.0, wire: from.1..to.1 });
                }
                full(std::mem::take(&mut page));
                made += 1;
                local.clear();
                edges.clear();
                used = 0;
                from = (0, 0);
            }
            let mut at = [0u32; 3];
            for (slot, vertex) in at.iter_mut().zip(triangle) {
                *slot = *local.entry(*vertex).or_insert_with(|| {
                    page.vertices.push(data.vertices.get(*vertex as usize).copied().unwrap_or_default());
                    used += size_of::<MeshVertex>() as u64;
                    (page.vertices.len() - 1) as u32
                });
            }
            page.indices.extend(at);
            used += (3 * size_of::<u32>()) as u64;
            for (a, b) in [(at[0], at[1]), (at[1], at[2]), (at[2], at[0])] {
                if edges.insert((a.min(b), a.max(b))) {
                    page.wire.extend([a, b]);
                    used += (2 * size_of::<u32>()) as u64;
                }
            }
        }
        let to = (page.indices.len() as u32, page.wire.len() as u32);
        if to.0 > from.0 {
            pieces.push(Piece { page: made, indices: from.0..to.0, wire: from.1..to.1 });
        }
        parts.push(pieces);
    }
    if !page.indices.is_empty() || made == 0 {
        full(page);
    }
    parts
}

/// [`page_by_page`] with every page kept.
#[cfg(test)]
pub(crate) fn paged(data: &MeshData, budget: u64) -> Paged {
    let mut pages = Vec::new();
    let parts = page_by_page(data, budget, |page| pages.push(page));
    Paged { pages, parts }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::material::{EditorMaterial, MaterialDesc};
    use crate::mesh::SubMesh;

    /// A strip of quads along X in two parts that share the vertices of their seam.
    fn strip(quads: u32) -> MeshData {
        let mut vertices = Vec::new();
        for i in 0..=quads {
            for z in [0.0, 1.0] {
                vertices.push(MeshVertex::new(Vec3::new(i as f32, 0.0, z), Vec3::Y, Vec2::ZERO));
            }
        }
        let mut indices = Vec::new();
        for i in 0..quads {
            let a = i * 2;
            indices.extend([a, a + 1, a + 2, a + 2, a + 1, a + 3]);
        }
        let half = quads / 2 * 6;
        let material = || MaterialDesc::textured(EditorMaterial::Opaque, "qa");
        let parts = vec![
            SubMesh { indices: 0..half, material: material() },
            SubMesh { indices: half..quads * 6, material: material() },
        ];
        MeshData::new(vertices, indices, parts).unwrap()
    }

    /// The triangles of a part as positions, read back from its pieces.
    fn triangles_of(paged: &Paged, part: usize) -> Vec<[[f32; 3]; 3]> {
        let mut out = Vec::new();
        for piece in &paged.parts[part] {
            let page = &paged.pages[piece.page as usize];
            let run = &page.indices[piece.indices.start as usize..piece.indices.end as usize];
            for t in run.as_chunks::<3>().0 {
                out.push(t.map(|i| page.vertices[i as usize].position));
            }
        }
        out
    }

    #[test]
    fn a_mesh_cut_into_pages_is_the_same_triangles_in_the_same_order() {
        let mesh = strip(400);
        let budget = 16 * 1024;
        let paged = paged(&mesh, budget);
        assert!(paged.pages.len() > 4, "{} pages", paged.pages.len());
        assert!(paged.pages.iter().all(|p| p.bytes() <= budget), "a page is over the budget");
        assert_eq!(paged.parts.len(), 2);
        for (i, part) in mesh.submeshes.iter().enumerate() {
            let want: Vec<[[f32; 3]; 3]> = mesh.indices
                [part.indices.start as usize..part.indices.end as usize]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| t.map(|v| mesh.vertices[v as usize].position))
                .collect();
            assert_eq!(triangles_of(&paged, i), want, "part {i}");
            // a part that crosses pages has a piece in each, and none is empty
            assert!(paged.parts[i].len() > 1);
            assert!(paged.parts[i].iter().all(|p| !p.indices.is_empty() && !p.wire.is_empty()));
        }
        // an index never leaves its page, and an edge is listed once per piece
        for (n, page) in paged.pages.iter().enumerate() {
            let count = page.vertices.len() as u32;
            assert!(page.indices.iter().chain(&page.wire).all(|i| *i < count), "page {n}");
        }
        for piece in paged.parts.iter().flatten() {
            let page = &paged.pages[piece.page as usize];
            let run = &page.wire[piece.wire.start as usize..piece.wire.end as usize];
            let mut seen = FxHashSet::default();
            assert!(run.as_chunks::<2>().0.iter().all(|e| seen.insert((e[0].min(e[1]), e[0].max(e[1])))));
        }
    }

    #[test]
    fn a_mesh_that_fits_is_one_page_and_one_piece_a_part() {
        let mesh = strip(8);
        let paged = paged(&mesh, u64::MAX);
        assert_eq!(paged.pages.len(), 1);
        assert_eq!(paged.parts.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
        assert_eq!(paged.parts[0][0].indices, 0..24);
        assert_eq!(paged.parts[1][0].indices, 24..48);
        // the seam's vertices are shared: nothing is doubled inside a page
        assert_eq!(paged.pages[0].vertices.len(), mesh.vertices.len());
    }

    #[test]
    fn a_budget_smaller_than_a_triangle_still_holds_one() {
        let mesh = strip(2);
        let paged = paged(&mesh, 1);
        assert_eq!(paged.pages.len(), 4);
        assert!(paged.pages.iter().all(|p| p.indices.len() == 3));
    }
}

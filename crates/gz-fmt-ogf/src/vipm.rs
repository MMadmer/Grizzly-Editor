//! The progressive (VIPM) pass of a skeleton split: `VIPM_Convert(u32(-1), 1.f, 1)` of
//! `XrETools/PropSlimTools.cpp` - Tom Forsyth's edge collapses (`object.cpp`) over his linked
//! mesh (`mesh.h`, `DLink.h`, `ArbitraryList.h`), the collapse error from QSlim's 5D quadric
//! (`MxQMetric.cpp`: position and UV, in double), the sliding windows (`object_sliding.cpp`).
//!
//! The C++ keeps points, edges and triangles as linked objects; here they are arenas indexed by
//! number, and every list the C++ walks keeps the C++ order: a point's edge and triangle lists
//! are arrays emptied by "copy the last over it" (`erase_fast`), the object's triangle list
//! takes a new triangle at its head, the points are walked from the last appended to the
//! first. `ArbitraryList::resize` keeps what lies past the size within the reservation, and
//! `insert` overwrites rather than inserts: both are kept (`AList`).

// the loops index as the C++ does, several arrays at once
#![allow(clippy::needless_range_loop)]

use glam::{Vec2, Vec3};

use crate::optimize::optimise_tri_list;

const INVALID: u32 = u32::MAX;
const QUAD_SIZE: usize = 5;

/// `VIPM_SWR`: one sliding window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swr {
    pub offset: u32,
    pub num_tris: u16,
    pub num_verts: u16,
}

/// `VIPM_Result`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VipmResult {
    /// `permute_verts[old] = new`.
    pub permute_verts: Vec<u16>,
    pub swr: Vec<Swr>,
    pub indices: Vec<u16>,
}

/// `ArbitraryList<T>`: an array with a reservation that keeps its old items.
struct AList<T: Copy + Default> {
    data: Vec<T>,
    size: usize,
}

impl<T: Copy + Default> AList<T> {
    fn new() -> Self {
        Self { data: Vec::new(), size: 0 }
    }

    fn resize(&mut self, n: usize) {
        self.size = n;
        if n <= self.data.len() {
            if n == 0 {
                self.data = Vec::new();
            }
            return;
        }
        let reserve = (n * 3) >> 1;
        // the C++ copies the whole old reservation; what `new T[]` adds is uninitialised there
        self.data.resize(reserve, T::default());
    }

    fn get(&self, i: usize) -> Option<T> {
        self.data.get(i).copied()
    }

    fn set(&mut self, i: usize, v: T) -> Option<()> {
        *self.data.get_mut(i)? = v;
        Some(())
    }

    /// `insert(first, other, first_other, n)`: copies over what is there.
    fn overwrite(&mut self, first: usize, other: &AList<T>, from: usize, n: usize) -> Option<()> {
        for i in 0..n {
            self.set(first + i, other.get(from + i)?)?;
        }
        Some(())
    }
}

// ------------------------------------------------------------------ the quadric

type Vec5 = [f64; QUAD_SIZE];

fn dot(a: &Vec5, b: &Vec5) -> f64 {
    let mut d = 0.0;
    for i in 0..QUAD_SIZE {
        d += a[i] * b[i];
    }
    d
}

/// `mxv_unitize`.
fn unitize(v: &mut Vec5) {
    let l = dot(v, v);
    if l != 1.0 && l != 0.0 {
        let l = l.sqrt();
        for x in v.iter_mut() {
            *x /= l;
        }
    }
}

/// `MxQuadric` of dimension 5.
#[derive(Clone, Copy)]
struct Quadric {
    a: [[f64; QUAD_SIZE]; QUAD_SIZE],
    b: Vec5,
    c: f64,
}

impl Quadric {
    const ZERO: Quadric = Quadric { a: [[0.0; QUAD_SIZE]; QUAD_SIZE], b: [0.0; QUAD_SIZE], c: 0.0 };

    /// `MxQuadric(p1, p2, p3, 0)`.
    fn of_triangle(p1: &Vec5, p2: &Vec5, p3: &Vec5) -> Self {
        let mut e1 = [0.0; QUAD_SIZE];
        let mut e2 = [0.0; QUAD_SIZE];
        for i in 0..QUAD_SIZE {
            e1[i] = p2[i] - p1[i];
            e2[i] = p3[i] - p1[i];
        }
        unitize(&mut e1);
        let mut t = e1;
        let s = dot(&e1, &e2);
        for x in t.iter_mut() {
            *x *= s;
        }
        for i in 0..QUAD_SIZE {
            e2[i] -= t[i];
        }
        unitize(&mut e2);
        let p1e1 = dot(p1, &e1);
        let p1e2 = dot(p1, &e2);
        let mut a = [[0.0; QUAD_SIZE]; QUAD_SIZE];
        for (i, row) in a.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        for e in [&e1, &e2] {
            for i in 0..QUAD_SIZE {
                for j in 0..QUAD_SIZE {
                    a[i][j] -= e[i] * e[j];
                }
            }
        }
        let mut b = e1;
        for x in b.iter_mut() {
            *x *= p1e1;
        }
        let mut t = e2;
        for x in t.iter_mut() {
            *x *= p1e2;
        }
        for i in 0..QUAD_SIZE {
            b[i] += t[i];
        }
        for i in 0..QUAD_SIZE {
            b[i] -= p1[i];
        }
        let c = dot(p1, p1) - p1e1 * p1e1 - p1e2 * p1e2;
        Self { a, b, c }
    }

    fn add(&mut self, q: &Quadric) {
        for i in 0..QUAD_SIZE {
            for j in 0..QUAD_SIZE {
                self.a[i][j] += q.a[i][j];
            }
        }
        for i in 0..QUAD_SIZE {
            self.b[i] += q.b[i];
        }
        self.c += q.c;
    }

    /// `evaluate`: `v (A v) + 2 (b v) + c`.
    fn evaluate(&self, v: &Vec5) -> f64 {
        let mut av = [0.0; QUAD_SIZE];
        for (i, o) in av.iter_mut().enumerate() {
            let mut r = 0.0;
            for j in 0..QUAD_SIZE {
                r += self.a[i][j] * v[j];
            }
            *o = r;
        }
        dot(v, &av) + 2.0 * dot(&self.b, v) + self.c
    }
}

// ------------------------------------------------------------------ the mesh

struct Pt {
    pos: Vec3,
    u: f32,
    v: f32,
    index: u32,
    new_index: u32,
    edges: Vec<usize>,
    tris: Vec<usize>,
}

impl Pt {
    fn vec5(&self) -> Vec5 {
        [
            f64::from(self.pos.x),
            f64::from(self.pos.y),
            f64::from(self.pos.z),
            f64::from(self.u),
            f64::from(self.v),
        ]
    }
}

struct Edge {
    p: [Option<usize>; 2],
    tri12: Option<usize>,
    tri21: Option<usize>,
}

struct Tri {
    p: [usize; 3],
    e: [Option<usize>; 3],
    level: i32,
    new_index: u32,
}

#[derive(Clone, Copy, Default)]
struct TriInfo {
    p: [usize; 3],
}

struct Collapse {
    original: Vec<TriInfo>,
    collapsed: Vec<TriInfo>,
    level: i32,
    next_level: Vec<TriInfo>,
    bin: usize,
}

/// Why the pass gave nothing (the C++ asserts, or returns NULL).
#[derive(Debug)]
pub(crate) struct Failed;

/// `Object`: the mesh, the collapses, the current window level.
struct Object {
    pts: Vec<Pt>,
    edges: Vec<Option<Edge>>,
    tris: Vec<Option<Tri>>,
    /// The triangle list: newest first.
    tri_first: Option<usize>,
    tri_next: Vec<Option<usize>>,
    tri_prev: Vec<Option<usize>>,
    /// In creation order; the C++ list walks it newest first.
    collapses: Vec<Collapse>,
    /// `pNextCollapse`: `None` is `&CollapseRoot`.
    next_collapse: Option<usize>,
    level: i32,
}

impl Object {
    fn tri(&self, t: usize) -> Result<&Tri, Failed> {
        self.tris.get(t).and_then(Option::as_ref).ok_or(Failed)
    }

    fn tri_mut(&mut self, t: usize) -> Result<&mut Tri, Failed> {
        self.tris.get_mut(t).and_then(Option::as_mut).ok_or(Failed)
    }

    fn edge(&self, e: usize) -> Result<&Edge, Failed> {
        self.edges.get(e).and_then(Option::as_ref).ok_or(Failed)
    }

    fn edge_mut(&mut self, e: usize) -> Result<&mut Edge, Failed> {
        self.edges.get_mut(e).and_then(Option::as_mut).ok_or(Failed)
    }

    fn pt(&self, p: usize) -> Result<&Pt, Failed> {
        self.pts.get(p).ok_or(Failed)
    }

    fn pt_mut(&mut self, p: usize) -> Result<&mut Pt, Failed> {
        self.pts.get_mut(p).ok_or(Failed)
    }

    /// The triangle list from its head.
    fn tri_list(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.tri_first;
        while let Some(t) = cur {
            out.push(t);
            cur = self.tri_next.get(t).copied().flatten();
        }
        out
    }

    /// `MeshPt::FindTriEdge`: the first edge of `a` to `b` with a free side for `a`, `b`.
    fn find_tri_edge(&self, a: usize, b: usize) -> Result<Option<usize>, Failed> {
        for &e in &self.pt(a)?.edges {
            let ed = self.edge(e)?;
            if ed.p[1] == Some(b) && ed.tri12.is_none() {
                return Ok(Some(e));
            }
            if ed.p[0] == Some(b) && ed.tri21.is_none() {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// `MeshPt::FindEdge`.
    fn find_edge(&self, a: usize, b: usize) -> Result<Option<usize>, Failed> {
        for &e in &self.pt(a)?.edges {
            let ed = self.edge(e)?;
            if ed.p[1] == Some(b) || ed.p[0] == Some(b) {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// `MeshPt::FindTri`: the triangle `a`, `b`, `c` in this winding.
    fn find_tri(&self, a: usize, b: usize, c: usize) -> Result<Option<usize>, Failed> {
        for &t in &self.pt(a)?.tris {
            let p = self.tri(t)?.p;
            if (p[0] == a && p[1] == b && p[2] == c)
                || (p[1] == a && p[2] == b && p[0] == c)
                || (p[2] == a && p[0] == b && p[1] == c)
            {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// `new MeshEdge(p1, p2, &CurEdgeRoot)`.
    fn new_edge(&mut self, a: usize, b: usize) -> Result<usize, Failed> {
        let id = self.edges.len();
        self.edges.push(Some(Edge { p: [Some(a), Some(b)], tri12: None, tri21: None }));
        self.pt_mut(a)?.edges.push(id);
        self.pt_mut(b)?.edges.push(id);
        Ok(id)
    }

    /// `MeshEdge::AddTri`.
    fn edge_add_tri(&mut self, e: usize, t: usize) -> Result<(), Failed> {
        let p = self.tri(t)?.p;
        let ed = self.edge_mut(e)?;
        let (a, b) = (ed.p[0], ed.p[1]);
        let forward = (a == Some(p[0]) && b == Some(p[1]))
            || (a == Some(p[1]) && b == Some(p[2]))
            || (a == Some(p[2]) && b == Some(p[0]));
        if forward {
            ed.tri12 = Some(t);
        } else {
            ed.tri21 = Some(t);
        }
        Ok(())
    }

    /// `new MeshTri(p1, p2, p3, &CurTriRoot, &CurEdgeRoot)`.
    fn new_tri(&mut self, p: [usize; 3], level: i32) -> Result<usize, Failed> {
        let id = self.tris.len();
        self.tris.push(Some(Tri { p, e: [None; 3], level, new_index: 0 }));
        self.tri_next.push(None);
        self.tri_prev.push(None);
        for &v in &p {
            self.pt_mut(v)?.tris.push(id);
        }
        for k in 0..3 {
            let (a, b) = (p[k], p[(k + 1) % 3]);
            let e = match self.find_tri_edge(a, b)? {
                Some(e) => e,
                None => self.new_edge(a, b)?,
            };
            self.tri_mut(id)?.e[k] = Some(e);
            self.edge_add_tri(e, id)?;
        }
        // `ListAddAfter(&CurTriRoot)`
        self.tri_next[id] = self.tri_first;
        if let Some(f) = self.tri_first {
            self.tri_prev[f] = Some(id);
        }
        self.tri_first = Some(id);
        Ok(id)
    }

    fn remove_from(list: &mut Vec<usize>, x: usize) {
        if let Some(i) = list.iter().position(|v| *v == x) {
            list.swap_remove(i);
        }
    }

    /// `MeshTri::Delete(TRUE)`: the edges no triangle uses any more go too.
    fn delete_tri(&mut self, t: usize) -> Result<(), Failed> {
        let tri = self.tris.get_mut(t).and_then(Option::take).ok_or(Failed)?;
        for e in tri.e.into_iter().flatten() {
            let ed = self.edge_mut(e)?;
            if ed.tri12 == Some(t) {
                ed.tri12 = None;
            } else {
                ed.tri21 = None;
            }
            if ed.tri12.is_none() && ed.tri21.is_none() {
                let ed = self.edges.get_mut(e).and_then(Option::take).ok_or(Failed)?;
                for p in ed.p.into_iter().flatten() {
                    Self::remove_from(&mut self.pt_mut(p)?.edges, e);
                }
            }
        }
        for p in tri.p {
            Self::remove_from(&mut self.pt_mut(p)?.tris, t);
        }
        // `ListDel`
        let (prev, next) = (self.tri_prev[t], self.tri_next[t]);
        match prev {
            Some(p) => self.tri_next[p] = next,
            None => self.tri_first = next,
        }
        if let Some(n) = next {
            self.tri_prev[n] = prev;
        }
        self.tri_prev[t] = None;
        self.tri_next[t] = None;
        Ok(())
    }

    fn set_new_level(&mut self, level: i32) -> Result<(), Failed> {
        for t in self.tri_list() {
            self.tri_mut(t)?.level = level;
        }
        Ok(())
    }

    /// `CollapseAllowedForLevel`.
    fn allowed_for_level(&self, p: usize, level: i32) -> Result<bool, Failed> {
        let mut ok = true;
        for &t in &self.pt(p)?.tris {
            if self.tri(t)?.level != level {
                ok = false;
            }
        }
        Ok(ok)
    }

    /// `FindCollapseError`: the quadric of the binned point's triangles at the kept point.
    fn collapse_error(&self, binned: usize, edge: usize) -> Result<f32, Failed> {
        let mut sum = Quadric::ZERO;
        for &t in &self.pt(binned)?.tris {
            let p = self.tri(t)?.p;
            let q =
                Quadric::of_triangle(&self.pt(p[0])?.vec5(), &self.pt(p[1])?.vec5(), &self.pt(p[2])?.vec5());
            sum.add(&q);
        }
        let ed = self.edge(edge)?;
        let kept = if ed.p[0] == Some(binned) { ed.p[1] } else { ed.p[0] }.ok_or(Failed)?;
        Ok(sum.evaluate(&self.pt(kept)?.vec5()) as f32)
    }

    fn rotated(&self, t: usize, binned: usize) -> Result<[usize; 3], Failed> {
        let mut p = self.tri(t)?.p;
        if !p.contains(&binned) {
            return Err(Failed);
        }
        while p[0] != binned {
            p.rotate_left(1);
        }
        Ok(p)
    }

    /// `CreateEdgeCollapse`.
    fn create_edge_collapse(&mut self, binned: usize, kept: usize) -> Result<(), Failed> {
        // the C++ also takes the collapse's error here; nothing reads it but a debug total
        self.find_edge(kept, binned)?.ok_or(Failed)?;
        let binned_tris: Vec<usize> = self.pt(binned)?.tris.clone();
        let mut need_new_level = false;
        for &t in &binned_tris {
            if self.tri(t)?.level != self.level {
                need_new_level = true;
            }
        }
        let mut c = Collapse {
            original: Vec::new(),
            collapsed: Vec::new(),
            level: 0,
            next_level: Vec::new(),
            bin: binned,
        };
        if need_new_level {
            for t in self.tri_list() {
                let tri = self.tri(t)?;
                if tri.level != self.level {
                    c.next_level.push(TriInfo { p: tri.p });
                }
            }
            self.level += 1;
            self.set_new_level(self.level)?;
        }
        c.level = self.level;
        // the changed triangles first, then the binned ones, both from the back
        let mut slots: Vec<Option<usize>> = binned_tris.iter().copied().map(Some).collect();
        for slot in slots.iter_mut().rev() {
            let Some(t) = *slot else { continue };
            let p = self.rotated(t, binned)?;
            if !p.contains(&kept) {
                c.original.push(TriInfo { p });
                self.delete_tri(t)?;
                *slot = None;
            }
        }
        for slot in slots.iter_mut().rev() {
            let Some(t) = *slot else { continue };
            let p = self.rotated(t, binned)?;
            c.original.push(TriInfo { p });
            self.delete_tri(t)?;
            *slot = None;
        }
        for info in c.original.clone() {
            if info.p[1] == kept || info.p[2] == kept {
                continue;
            }
            let p = [kept, info.p[1], info.p[2]];
            c.collapsed.push(TriInfo { p });
            self.new_tri(p, self.level + 1)?;
        }
        self.collapses.push(c);
        Ok(())
    }

    fn list_next(&self, at: Option<usize>) -> Option<usize> {
        match at {
            None => self.collapses.len().checked_sub(1),
            Some(i) => i.checked_sub(1),
        }
    }

    fn list_prev(&self, i: usize) -> Option<usize> {
        (i + 1 < self.collapses.len()).then_some(i + 1)
    }

    /// `UndoCollapse`.
    fn undo(&mut self) -> Result<bool, Failed> {
        let Some(i) = self.list_next(self.next_collapse) else { return Ok(false) };
        self.next_collapse = Some(i);
        let collapsed = self.collapses[i].collapsed.clone();
        for info in collapsed {
            let t = self.find_tri(info.p[0], info.p[1], info.p[2])?.ok_or(Failed)?;
            self.delete_tri(t)?;
        }
        let original = self.collapses[i].original.clone();
        for info in original {
            self.new_tri(info.p, self.level)?;
        }
        if let Some(prev) = self.list_next(Some(i))
            && self.collapses[prev].level != self.level
        {
            self.level -= 1;
            self.set_new_level(self.level)?;
            let next_level = self.collapses[i].next_level.clone();
            for info in next_level {
                let t = self.find_tri(info.p[0], info.p[1], info.p[2])?.ok_or(Failed)?;
                self.tri_mut(t)?.level = self.level + 1;
            }
        }
        Ok(true)
    }

    /// `DoCollapse`.
    fn redo(&mut self) -> Result<bool, Failed> {
        let Some(i) = self.next_collapse else { return Ok(false) };
        if self.collapses[i].level != self.level {
            self.level += 1;
            self.set_new_level(self.level)?;
        }
        let original = self.collapses[i].original.clone();
        for info in original {
            let t = self.find_tri(info.p[0], info.p[1], info.p[2])?.ok_or(Failed)?;
            self.delete_tri(t)?;
        }
        let collapsed = self.collapses[i].collapsed.clone();
        for info in collapsed {
            self.new_tri(info.p, self.level + 1)?;
        }
        self.next_collapse = self.list_prev(i);
        Ok(true)
    }

    /// `Valid`: no two triangles of the same three points in any order (`MeshTri::Equal`'s
    /// six permutations: the same sorted triple; the C++ compares every pair).
    fn valid(&self) -> Result<bool, Failed> {
        let mut seen = std::collections::HashSet::new();
        for t in self.tri_list() {
            let mut p = self.tri(t)?.p;
            p.sort_unstable();
            if !seen.insert(p) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `CalculateAllCollapses(object, u32(-1), 1.f)`.
    fn all_collapses(&mut self, tolerance: f32) -> Result<(), Failed> {
        loop {
            let mut best = (1.0e10f32, None::<(usize, usize)>);
            let mut best_nl = (1.0e10f32, None::<(usize, usize)>);
            let mut average = 0.0f32;
            let mut count = 0i32;
            for p in (0..self.pts.len()).rev() {
                let edges = self.pt(p)?.edges.clone();
                if edges.is_empty() {
                    continue;
                }
                let mut allowed = true;
                for &e in &edges {
                    let ed = self.edge(e)?;
                    if ed.tri12.is_none() || ed.tri21.is_none() {
                        allowed = false;
                        break;
                    }
                }
                if !allowed {
                    continue;
                }
                let new_level = !self.allowed_for_level(p, self.level)?;
                for &e in &edges {
                    let err = self.collapse_error(p, e)?;
                    count += 1;
                    average += err;
                    let slot = if new_level { &mut best_nl } else { &mut best };
                    if slot.0 > err {
                        *slot = (err, Some((e, p)));
                    }
                }
            }
            average /= count as f32;
            if best.0 > best_nl.0 + average * tolerance {
                best = best_nl;
            }
            let Some((e, p)) = best.1 else { break };
            let ed = self.edge(e)?;
            let kept = if ed.p[0] == Some(p) { ed.p[1] } else { ed.p[0] }.ok_or(Failed)?;
            self.create_edge_collapse(p, kept)?;
        }
        Ok(())
    }

    fn pt_new_index(&self, p: usize) -> Result<u16, Failed> {
        Ok(self.pt(p)?.new_index as u16)
    }

    fn info_indices(&self, info: &TriInfo, below: u16) -> Result<[u16; 3], Failed> {
        let mut out = [0u16; 3];
        for (o, &p) in out.iter_mut().zip(&info.p) {
            let n = self.pt(p)?.new_index;
            if n >= u32::from(below) {
                return Err(Failed);
            }
            *o = n as u16;
        }
        Ok(out)
    }

    /// `CalculateSW(object, result, 1)`.
    fn sliding_windows(&mut self) -> Result<VipmResult, Failed> {
        while self.undo()? {}
        let num_verts = self.pts.len();
        for p in &mut self.pts {
            p.new_index = INVALID;
        }
        if num_verts >= 65535 {
            return Err(Failed);
        }
        let all = self.tri_list();
        for &t in &all {
            self.tri_mut(t)?.new_index = INVALID;
        }
        if all.len() >= 65535 {
            return Err(Failed);
        }
        let mut indices: AList<u16> = AList::new();
        let mut permute = vec![u16::MAX; num_verts];

        let mut cur_verts = num_verts as i32;
        let mut cur_collapse = 0i32;
        let mut cur_level = 0i32;
        while let Some(i) = self.next_collapse {
            cur_level = self.collapses[i].level;
            cur_collapse += 1;
            cur_verts -= 1;
            let bin = self.collapses[i].bin;
            self.pt_mut(bin)?.new_index = cur_verts as u32;
            self.redo()?;
        }
        let num_collapses = cur_collapse;

        let mut cur_index: u16 = 0;
        for p in (0..self.pts.len()).rev() {
            let pt = self.pt_mut(p)?;
            if pt.new_index == INVALID {
                *permute.get_mut(pt.index as usize).ok_or(Failed)? = cur_index;
                pt.new_index = u32::from(cur_index);
                cur_index += 1;
            }
        }
        if i32::from(cur_index) != cur_verts {
            return Err(Failed);
        }
        let mut cur_num_tris = self.tri_list().len() as i32;

        let mut swr = vec![Swr::default(); (num_collapses + 1) as usize];
        cur_collapse = num_collapses;
        *swr.get_mut(cur_collapse as usize).ok_or(Failed)? =
            Swr { offset: 0, num_tris: cur_num_tris as u16, num_verts: cur_index };

        let mut cur_tri_binned = 0i32;
        let mut temp: AList<u16> = AList::new();
        let max_level = cur_level;
        loop {
            let mut cur_tri_added = cur_tri_binned + cur_num_tris;
            indices.resize((cur_tri_added * 3) as usize);
            let list = self.tri_list();
            for &t in &list {
                self.tri_mut(t)?.new_index = INVALID;
            }
            if list.len() as i32 != cur_num_tris {
                return Err(Failed);
            }
            let mut just_started = true;
            while let Some(i) = self.list_next(self.next_collapse)
                && self.collapses[i].level == cur_level
            {
                let collapsed = self.collapses[i].collapsed.clone();
                if !just_started || cur_level == max_level {
                    if !collapsed.is_empty() {
                        temp.resize(collapsed.len() * 3);
                        for (k, info) in collapsed.iter().enumerate() {
                            let t = self.find_tri(info.p[0], info.p[1], info.p[2])?.ok_or(Failed)?;
                            if self.tri(t)?.new_index != INVALID {
                                return Err(Failed);
                            }
                            let ix = self.info_indices(info, cur_index)?;
                            for (j, v) in ix.into_iter().enumerate() {
                                temp.set(k * 3 + j, v).ok_or(Failed)?;
                            }
                            cur_num_tris -= 1;
                        }
                        optimise(&mut temp, collapsed.len())?;
                        indices
                            .overwrite((cur_tri_binned * 3) as usize, &temp, 0, 3 * collapsed.len())
                            .ok_or(Failed)?;
                        cur_tri_binned += collapsed.len() as i32;
                    }
                } else {
                    cur_num_tris -= collapsed.len() as i32;
                    cur_tri_added -= collapsed.len() as i32;
                }
                just_started = false;
                self.undo()?;
                let bin = self.collapses[i].bin;
                if self.pt(bin)?.new_index != u32::from(cur_index) {
                    return Err(Failed);
                }
                let index = self.pt(bin)?.index as usize;
                *permute.get_mut(index).ok_or(Failed)? = cur_index;
                cur_index = cur_index.wrapping_add(1);

                let original = self.collapses[i].original.clone();
                if !original.is_empty() {
                    temp.resize(original.len() * 3);
                    for (k, info) in original.iter().enumerate() {
                        let t = self.find_tri(info.p[0], info.p[1], info.p[2])?.ok_or(Failed)?;
                        self.tri_mut(t)?.new_index = 1;
                        let ix = self.info_indices(info, cur_index)?;
                        for (j, v) in ix.into_iter().enumerate() {
                            temp.set(k * 3 + j, v).ok_or(Failed)?;
                        }
                        cur_num_tris += 1;
                    }
                    optimise(&mut temp, original.len())?;
                    indices.resize(((cur_tri_added as usize) + original.len()) * 3);
                    indices
                        .overwrite((cur_tri_added * 3) as usize, &temp, 0, 3 * original.len())
                        .ok_or(Failed)?;
                    cur_tri_added += original.len() as i32;
                }
                cur_collapse -= 1;
                *swr.get_mut(usize::try_from(cur_collapse).map_err(|_| Failed)?).ok_or(Failed)? = Swr {
                    offset: (cur_tri_binned * 3) as u32,
                    num_tris: cur_num_tris as u16,
                    num_verts: cur_index,
                };
            }

            // the triangles this level did not add go in the middle
            let mut checking = 0i32;
            let mut temp_tris = 0usize;
            for t in self.tri_list() {
                checking += 1;
                let tri = self.tri(t)?;
                if tri.new_index == INVALID {
                    let p = tri.p;
                    temp.resize(temp_tris * 3 + 3);
                    for (j, &v) in p.iter().enumerate() {
                        let n = self.pt_new_index(v)?;
                        temp.set(temp_tris * 3 + j, n).ok_or(Failed)?;
                    }
                    temp_tris += 1;
                }
            }
            if checking != cur_num_tris {
                return Err(Failed);
            }
            if temp_tris > 0 {
                optimise(&mut temp, temp_tris)?;
            }
            indices.overwrite((cur_tri_binned * 3) as usize, &temp, 0, 3 * temp_tris).ok_or(Failed)?;

            let Some(next) = self.list_next(self.next_collapse) else {
                if cur_collapse != 0 || cur_level != 0 {
                    return Err(Failed);
                }
                break;
            };
            cur_tri_binned += cur_num_tris;
            if cur_tri_binned != cur_tri_added {
                return Err(Failed);
            }
            cur_level = self.collapses[next].level;
        }

        // every window must stay within its vertices (the count is raised to what it holds)
        let mut ok = true;
        for w in &mut swr {
            for j in 0..usize::from(w.num_tris) * 3 {
                let v = indices.get(j + w.offset as usize).ok_or(Failed)?;
                if j + w.offset as usize >= indices.size {
                    return Err(Failed);
                }
                w.num_verts = w.num_verts.max(v);
                if v >= w.num_verts {
                    ok = false;
                }
            }
        }
        if !ok {
            return Err(Failed);
        }
        let mut out = indices.data;
        out.truncate(indices.size);
        Ok(VipmResult { permute_verts: permute, swr, indices: out })
    }
}

/// `OptimiseVertexCoherencyTriList(list, n, 1)` over the first `n` triangles of the list.
fn optimise(list: &mut AList<u16>, n: usize) -> Result<(), Failed> {
    let slice = list.data.get_mut(..n * 3).ok_or(Failed)?;
    optimise_tri_list(slice, 1);
    Ok(())
}

/// `VIPM_Init`, `VIPM_AppendVertex` for every vertex, `VIPM_AppendFace` for every face,
/// `VIPM_Convert(u32(-1), 1.f, 1)`. `None` where the C++ answers NULL ("Can't make
/// progressive") or stops on an assertion.
pub(crate) fn convert(verts: &[(Vec3, Vec2)], faces: &[[u16; 3]]) -> Option<VipmResult> {
    let mut o = Object {
        pts: verts
            .iter()
            .enumerate()
            .map(|(i, (p, uv))| Pt {
                pos: *p,
                u: uv.x,
                v: uv.y,
                index: i as u32,
                new_index: 0,
                edges: Vec::new(),
                tris: Vec::new(),
            })
            .collect(),
        edges: Vec::new(),
        tris: Vec::new(),
        tri_first: None,
        tri_next: Vec::new(),
        tri_prev: Vec::new(),
        collapses: Vec::new(),
        next_collapse: None,
        level: 0,
    };
    for f in faces {
        let p = f.map(usize::from);
        if p.iter().any(|v| *v >= o.pts.len()) {
            return None;
        }
        o.new_tri(p, 0).ok()?;
    }
    // `Initialize`: the counts are not used again; `SetNewLevel(0)`
    o.set_new_level(0).ok()?;
    if !o.valid().ok()? {
        return None;
    }
    o.all_collapses(1.0).ok()?;
    o.sliding_windows().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_triangle_soup_collapses_nothing_and_turns_around() {
        // two faces of their own (as every skeleton split is): no point has a closed fan, so
        // nothing collapses; the points are taken last first and the faces newest first
        let verts: Vec<(Vec3, Vec2)> =
            [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0), Vec3::Z]
                .iter()
                .map(|p| (*p, Vec2::ZERO))
                .collect();
        let r = convert(&verts, &[[0, 1, 2], [3, 4, 5]]).unwrap();
        assert_eq!(r.permute_verts, vec![5, 4, 3, 2, 1, 0]);
        assert_eq!(r.swr, vec![Swr { offset: 0, num_tris: 2, num_verts: 6 }]);
        assert_eq!(r.indices, vec![2, 1, 0, 5, 4, 3]);
    }

    /// A closed octahedron: every edge has two faces, so collapses happen.
    fn octahedron() -> (Vec<(Vec3, Vec2)>, Vec<[u16; 3]>) {
        let p = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let verts = p.iter().map(|v| (*v, Vec2::new(v.x, v.z))).collect();
        let faces =
            vec![[0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4], [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5]];
        (verts, faces)
    }

    #[test]
    fn a_closed_mesh_collapses_into_windows() {
        let (verts, faces) = octahedron();
        let r = convert(&verts, &faces).unwrap();
        // every vertex is placed once
        let mut seen = r.permute_verts.clone();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 5]);
        // a window per collapse count: the first is the whole mesh, the last the most collapsed
        // one, which starts the index list
        assert!(r.swr.len() > 1);
        let full = r.swr[0];
        assert_eq!((full.num_tris, full.num_verts), (8, 6));
        assert_eq!(r.swr.last().unwrap().offset, 0);
        for w in r.swr.windows(2) {
            assert!(w[1].num_tris <= w[0].num_tris);
        }
        for w in &r.swr {
            for j in 0..usize::from(w.num_tris) * 3 {
                assert!(r.indices[j + w.offset as usize] < w.num_verts);
            }
        }
    }

    #[test]
    fn the_collapse_error_is_the_5d_quadric() {
        // a point on the plane of a triangle with the same UV costs nothing
        let a = [0.0, 0.0, 0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0, 1.0, 0.0];
        let c = [0.0, 1.0, 0.0, 0.0, 1.0];
        let q = Quadric::of_triangle(&a, &b, &c);
        assert!(q.evaluate(&[0.5, 0.5, 0.0, 0.5, 0.5]).abs() < 1e-12);
        // off the plane it costs the squared distance
        assert!((q.evaluate(&[0.0, 0.0, 2.0, 0.0, 0.0]) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn duplicate_faces_refuse_the_pass() {
        let verts: Vec<(Vec3, Vec2)> =
            [Vec3::ZERO, Vec3::X, Vec3::Z].iter().map(|p| (*p, Vec2::ZERO)).collect();
        assert!(convert(&verts, &[[0, 1, 2], [1, 2, 0]]).is_none());
    }
}

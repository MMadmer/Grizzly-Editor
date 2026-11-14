//! NVIDIA's NvTriStrip as the compiler runs it: lists only, no stitching,
//! the experiments that grow strips from reset points, the split into cache-sized pieces and their
//! order by vertex cache hits; then the indices renumbered in first use.
//!
//! Faces, edges and strips live in arenas and name each other by index where X-Ray's compiler keeps
//! pointers; every list is walked in X-Ray's order.

const NONE: usize = usize::MAX;
const CACHE_INEFFICIENCY: i32 = 6;

#[derive(Clone, Copy, Debug)]
struct FaceInfo {
    v: [i32; 3],
    strip_id: i32,
    test_strip_id: i32,
    experiment_id: i32,
}

impl FaceInfo {
    fn new(v0: i32, v1: i32, v2: i32) -> Self {
        Self { v: [v0, v1, v2], strip_id: -1, test_strip_id: -1, experiment_id: -1 }
    }
}

#[derive(Clone, Copy, Debug)]
struct EdgeInfo {
    face0: usize,
    face1: usize,
    v0: i32,
    v1: i32,
    next_v0: usize,
    next_v1: usize,
}

#[derive(Clone, Copy, Debug)]
struct StartInfo {
    face: usize,
    edge: usize,
    to_v1: bool,
}

#[derive(Clone, Debug)]
struct StripInfo {
    start: StartInfo,
    faces: Vec<usize>,
    strip_id: i32,
    experiment_id: i32,
    visited: bool,
}

/// `VertexCache`.
struct VertexCache {
    entries: Vec<i32>,
}

impl VertexCache {
    fn new(size: usize) -> Self {
        Self { entries: vec![-1; size] }
    }
    fn in_cache(&self, e: i32) -> bool {
        self.entries.contains(&e)
    }
    fn add(&mut self, e: i32) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.rotate_right(1);
        self.entries[0] = e;
    }
}

struct Stripifier {
    faces: Vec<FaceInfo>,
    edges: Vec<EdgeInfo>,
    /// The head of every vertex's edge list.
    heads: Vec<usize>,
    strips: Vec<StripInfo>,
    cache_size: usize,
    mesh_jump: f32,
    first_reset: bool,
}

impl Stripifier {
    fn head(&self, v: i32) -> usize {
        usize::try_from(v).ok().and_then(|v| self.heads.get(v).copied()).unwrap_or(NONE)
    }

    fn find_edge(&self, v0: i32, v1: i32) -> usize {
        let mut it = self.head(v0);
        while let Some(e) = self.edges.get(it) {
            if e.v0 == v0 {
                if e.v1 == v1 {
                    return it;
                }
                it = e.next_v0;
            } else {
                if e.v0 == v1 {
                    return it;
                }
                it = e.next_v1;
            }
        }
        NONE
    }

    fn find_other_face(&self, v0: i32, v1: i32, face: usize) -> usize {
        match self.edges.get(self.find_edge(v0, v1)) {
            Some(e) => {
                if e.face0 == face {
                    e.face1
                } else {
                    e.face0
                }
            }
            None => NONE,
        }
    }

    fn already_exists(&self, f: &FaceInfo) -> bool {
        self.faces.iter().any(|x| x.v == f.v)
    }

    fn set_head(&mut self, v: i32, e: usize) {
        if let Some(slot) = usize::try_from(v).ok().and_then(|v| self.heads.get_mut(v)) {
            *slot = e;
        }
    }

    fn edge_for(&mut self, a: i32, b: i32, face: usize) {
        let e = self.find_edge(a, b);
        if let Some(ed) = self.edges.get_mut(e) {
            if ed.face1 == NONE {
                ed.face1 = face;
            }
            return;
        }
        let id = self.edges.len();
        self.edges.push(EdgeInfo {
            face0: face,
            face1: NONE,
            v0: a,
            v1: b,
            next_v0: self.head(a),
            next_v1: self.head(b),
        });
        self.set_head(a, id);
        self.set_head(b, id);
    }

    fn build_info(&mut self, indices: &[u16]) {
        let max_v = indices.iter().copied().max().map_or(0, usize::from);
        self.heads = vec![NONE; indices.len().max(max_v + 1)];
        for t in indices.as_chunks::<3>().0 {
            let (v0, v1, v2) = (i32::from(t[0]), i32::from(t[1]), i32::from(t[2]));
            let f = FaceInfo::new(v0, v1, v2);
            if self.already_exists(&f) {
                continue;
            }
            let id = self.faces.len();
            self.faces.push(f);
            self.edge_for(v0, v1, id);
            self.edge_for(v1, v2, id);
            self.edge_for(v2, v0, id);
        }
    }

    fn find_start_point(&self) -> i32 {
        for (i, f) in self.faces.iter().enumerate() {
            let mut ctr = 0;
            for (a, b) in [(f.v[0], f.v[1]), (f.v[1], f.v[2]), (f.v[2], f.v[0])] {
                if self.find_other_face(a, b, i) == NONE {
                    ctr += 1;
                }
            }
            if ctr > 1 {
                return i as i32;
            }
        }
        -1
    }

    fn find_good_reset_point(&mut self) -> usize {
        let n = self.faces.len() as i32;
        let mut start = if self.first_reset {
            self.first_reset = false;
            self.find_start_point()
        } else {
            ((n as f32 - 1.0) * self.mesh_jump) as i32
        };
        if start == -1 {
            start = ((n as f32 - 1.0) * self.mesh_jump) as i32;
        }
        let mut result = NONE;
        let mut i = start;
        loop {
            if self.faces.get(i as usize).is_some_and(|f| f.strip_id < 0) {
                result = i as usize;
                break;
            }
            i += 1;
            if i >= n {
                i = 0;
            }
            if i == start {
                break;
            }
        }
        self.mesh_jump += 0.1;
        if self.mesh_jump > 1.0 {
            self.mesh_jump = 0.05;
        }
        result
    }

    fn next_index(indices: &[i32], f: &FaceInfo) -> i32 {
        let n = indices.len();
        let (v0, v1) = (
            indices.get(n.wrapping_sub(2)).copied().unwrap_or(-1),
            indices.get(n.wrapping_sub(1)).copied().unwrap_or(-1),
        );
        for fv in f.v {
            if fv != v0 && fv != v1 {
                return fv;
            }
        }
        -1
    }

    fn is_experiment(s: &StripInfo) -> bool {
        s.experiment_id >= 0
    }

    fn is_in_strip(&self, s: usize, f: usize) -> bool {
        let (Some(st), Some(face)) = (self.strips.get(s), self.faces.get(f)) else { return false };
        if Self::is_experiment(st) { face.test_strip_id == st.strip_id } else { face.strip_id == st.strip_id }
    }

    fn is_marked(&self, s: usize, f: usize) -> bool {
        let (Some(st), Some(face)) = (self.strips.get(s), self.faces.get(f)) else { return false };
        face.strip_id >= 0 || (Self::is_experiment(st) && face.experiment_id == st.experiment_id)
    }

    fn mark(&mut self, s: usize, f: usize) {
        let Some(&StripInfo { strip_id, experiment_id, .. }) = self.strips.get(s) else { return };
        if let Some(face) = self.faces.get_mut(f) {
            if experiment_id >= 0 {
                face.experiment_id = experiment_id;
                face.test_strip_id = strip_id;
            } else {
                face.experiment_id = -1;
                face.strip_id = strip_id;
            }
        }
    }

    fn unique(&self, list: &[usize], f: usize) -> bool {
        let Some(face) = self.faces.get(f) else { return true };
        let (mut b0, mut b1, mut b2) = (false, false, false);
        for &x in list {
            let Some(o) = self.faces.get(x) else { continue };
            b0 |= o.v.contains(&face.v[0]);
            b1 |= o.v.contains(&face.v[1]);
            b2 |= o.v.contains(&face.v[2]);
            if b0 && b1 && b2 {
                return false;
            }
        }
        true
    }

    fn face(&self, f: usize) -> FaceInfo {
        self.faces.get(f).copied().unwrap_or(FaceInfo::new(-1, -1, -1))
    }

    /// Builds a strip.
    fn build(&mut self, s: usize) {
        let Some(start) = self.strips.get(s).map(|x| x.start) else { return };
        let mut forward = vec![start.face];
        self.mark(s, start.face);
        let Some(&e) = self.edges.get(start.edge) else { return };
        let v0 = if start.to_v1 { e.v0 } else { e.v1 };
        let v1 = if start.to_v1 { e.v1 } else { e.v0 };
        let mut scratch = vec![v0, v1];
        let v2 = Self::next_index(&scratch, &self.face(start.face));
        scratch.push(v2);

        let (mut nv0, mut nv1) = (v1, v2);
        let mut next = self.find_other_face(nv0, nv1, start.face);
        while next != NONE && !self.is_marked(s, next) {
            if !self.unique(&forward, next) {
                break;
            }
            forward.push(next);
            self.mark(s, next);
            nv0 = nv1;
            nv1 = Self::next_index(&scratch, &self.face(next));
            scratch.push(nv1);
            next = self.find_other_face(nv0, nv1, next);
        }

        let mut all = forward.clone();
        let mut backward = Vec::new();
        scratch = vec![v2, v1, v0];
        nv0 = v1;
        nv1 = v0;
        next = self.find_other_face(nv0, nv1, start.face);
        while next != NONE && !self.is_marked(s, next) {
            if !self.unique(&all, next) {
                break;
            }
            backward.push(next);
            all.push(next);
            self.mark(s, next);
            nv0 = nv1;
            nv1 = Self::next_index(&scratch, &self.face(next));
            scratch.push(nv1);
            next = self.find_other_face(nv0, nv1, next);
        }

        let mut faces: Vec<usize> = backward.into_iter().rev().collect();
        faces.extend(forward);
        if let Some(st) = self.strips.get_mut(s) {
            st.faces = faces;
        }
    }

    fn shares_edge(&self, s: usize, f: usize) -> bool {
        let face = self.face(f);
        for (a, b) in [(face.v[0], face.v[1]), (face.v[1], face.v[2]), (face.v[2], face.v[0])] {
            let Some(e) = self.edges.get(self.find_edge(a, b)) else { continue };
            if self.is_in_strip(s, e.face0) || self.is_in_strip(s, e.face1) {
                return true;
            }
        }
        false
    }

    fn find_traversal(&self, s: usize) -> Option<StartInfo> {
        let st = self.strips.get(s)?;
        let e = self.edges.get(st.start.edge)?;
        let v = if st.start.to_v1 { e.v1 } else { e.v0 };
        let mut it = self.head(v);
        let mut untouched = NONE;
        while let Some(ed) = self.edges.get(it) {
            let (f0, f1) = (ed.face0, ed.face1);
            if f0 != NONE && !self.is_in_strip(s, f0) && f1 != NONE && !self.is_marked(s, f1) {
                untouched = f1;
                break;
            }
            if f1 != NONE && !self.is_in_strip(s, f1) && f0 != NONE && !self.is_marked(s, f0) {
                untouched = f0;
                break;
            }
            it = if ed.v0 == v { ed.next_v0 } else { ed.next_v1 };
        }
        let ed = self.edges.get(it)?;
        let to_v1 = if self.shares_edge(s, untouched) { ed.v0 == v } else { ed.v1 == v };
        (untouched != NONE).then_some(StartInfo { face: untouched, edge: it, to_v1 })
    }

    fn avg_strip_size(&self, list: &[usize]) -> f32 {
        let sum: usize = list.iter().filter_map(|&s| self.strips.get(s)).map(|s| s.faces.len()).sum();
        sum as f32 / list.len() as f32
    }

    fn new_strip(&mut self, start: StartInfo, strip_id: i32, experiment_id: i32) -> usize {
        self.strips.push(StripInfo { start, faces: Vec::new(), strip_id, experiment_id, visited: false });
        self.strips.len() - 1
    }

    fn find_all_strips(&mut self, samples: usize) -> Vec<usize> {
        let mut all = Vec::new();
        let mut experiment_id = 0i32;
        let mut strip_id = 0i32;
        let mut done = false;
        while !done {
            let mut experiments: Vec<Vec<usize>> = Vec::new();
            let mut reset_points: Vec<usize> = Vec::new();
            for _ in 0..samples {
                let next = self.find_good_reset_point();
                if next == NONE {
                    done = true;
                    break;
                }
                if reset_points.contains(&next) {
                    continue;
                }
                reset_points.push(next);
                let f = self.face(next);
                let e01 = self.find_edge(f.v[0], f.v[1]);
                let e12 = self.find_edge(f.v[1], f.v[2]);
                let e20 = self.find_edge(f.v[2], f.v[0]);
                for (edge, to_v1) in
                    [(e01, true), (e01, false), (e12, true), (e12, false), (e20, true), (e20, false)]
                {
                    let s = self.new_strip(StartInfo { face: next, edge, to_v1 }, strip_id, experiment_id);
                    strip_id += 1;
                    experiment_id += 1;
                    experiments.push(vec![s]);
                }
            }
            for exp in &mut experiments {
                let first = exp[0];
                self.build(first);
                let eid = self.strips.get(first).map_or(-1, |s| s.experiment_id);
                let mut iter = first;
                while let Some(start) = self.find_traversal(iter) {
                    iter = self.new_strip(start, strip_id, eid);
                    strip_id += 1;
                    self.build(iter);
                    exp.push(iter);
                }
            }
            if experiments.is_empty() {
                continue;
            }
            let mut best = 0usize;
            let mut best_value = 0.0f32;
            for (i, exp) in experiments.iter().enumerate() {
                let avg = self.avg_strip_size(exp);
                let n = exp.len() as f32;
                let value = avg * 1.0 + avg * n * 1.0;
                if value > best_value {
                    best_value = value;
                    best = i;
                }
            }
            // CommitStrips
            for &s in &experiments[best] {
                if let Some(st) = self.strips.get_mut(s) {
                    st.experiment_id = -1;
                }
                all.push(s);
                let faces = self.strips.get(s).map(|x| x.faces.clone()).unwrap_or_default();
                for f in faces {
                    self.mark(s, f);
                }
            }
        }
        all
    }

    fn num_neighbors(&self, f: usize) -> i32 {
        let face = self.face(f);
        [(face.v[0], face.v[1]), (face.v[1], face.v[2]), (face.v[2], face.v[0])]
            .into_iter()
            .filter(|&(a, b)| self.find_other_face(a, b, f) != NONE)
            .count() as i32
    }

    fn hits_strip(&self, cache: &VertexCache, s: usize) -> f32 {
        let Some(st) = self.strips.get(s) else { return 0.0 };
        let mut hits = 0;
        for &f in &st.faces {
            hits += self.face(f).v.iter().filter(|&&v| cache.in_cache(v)).count();
        }
        hits as f32 / st.faces.len() as f32
    }

    fn update_cache(&self, cache: &mut VertexCache, s: usize) {
        let Some(st) = self.strips.get(s) else { return };
        for &f in &st.faces {
            for v in self.face(f).v {
                if !cache.in_cache(v) {
                    cache.add(v);
                }
            }
        }
    }

    fn split_and_optimize(&mut self, all: &[usize]) -> Vec<usize> {
        let threshold = self.cache_size;
        let mut temp = Vec::new();
        let blank = StartInfo { face: NONE, edge: NONE, to_v1: false };
        for &s in all {
            let faces = self.strips.get(s).map(|x| x.faces.clone()).unwrap_or_default();
            for piece in faces.chunks(threshold.max(1)) {
                let id = self.new_strip(blank, 0, -1);
                if let Some(st) = self.strips.get_mut(id) {
                    st.faces = piece.to_vec();
                }
                temp.push(id);
            }
        }
        // RemoveSmallStrips with a minimum of 0 keeps every strip
        let mut out = Vec::new();
        if temp.is_empty() {
            return out;
        }
        let mut cache = VertexCache::new(self.cache_size);
        let mut first = 0usize;
        let mut min_cost = 10000.0f32;
        for (i, &s) in temp.iter().enumerate() {
            let faces = self.strips.get(s).map(|x| x.faces.clone()).unwrap_or_default();
            let n: i32 = faces.iter().map(|&f| self.num_neighbors(f)).sum();
            let cost = n as f32 / faces.len() as f32;
            if cost < min_cost {
                min_cost = cost;
                first = i;
            }
        }
        self.update_cache(&mut cache, temp[first]);
        out.push(temp[first]);
        if let Some(st) = self.strips.get_mut(temp[first]) {
            st.visited = true;
        }
        loop {
            let mut best_hits = -1.0f32;
            let mut best = 0usize;
            for (i, &s) in temp.iter().enumerate() {
                if self.strips.get(s).is_some_and(|x| x.visited) {
                    continue;
                }
                let h = self.hits_strip(&cache, s);
                if h > best_hits {
                    best_hits = h;
                    best = i;
                }
            }
            if best_hits == -1.0 {
                break;
            }
            if let Some(st) = self.strips.get_mut(temp[best]) {
                st.visited = true;
            }
            self.update_cache(&mut cache, temp[best]);
            out.push(temp[best]);
        }
        out
    }
}

/// Generates the strips with lists only, no minimum strip size and the given cache size:
/// the faces in strip order as one list.
#[must_use]
pub fn generate_list(indices: &[u16], cache: u32) -> Vec<u16> {
    let mut s = Stripifier {
        faces: Vec::with_capacity(indices.len() / 3),
        edges: Vec::with_capacity(indices.len()),
        heads: Vec::new(),
        strips: Vec::new(),
        cache_size: (cache as i32 - CACHE_INEFFICIENCY).max(1) as usize,
        mesh_jump: 0.0,
        first_reset: true,
    };
    s.build_info(indices);
    let all = s.find_all_strips(10);
    let out_strips = s.split_and_optimize(&all);
    let mut out = Vec::with_capacity(indices.len());
    for st in out_strips {
        let faces = s.strips.get(st).map(|x| x.faces.clone()).unwrap_or_default();
        for f in faces {
            for v in s.face(f).v {
                out.push(v as u16);
            }
        }
    }
    out
}

/// `xrStripify(indices, perturb, cache, 0)`: the indices in strip order and renumbered in the
/// order they are first used; `permute[new] = old`. `None` when the strips lost a face (a
/// duplicate face: "Stripify failed.").
#[must_use]
pub fn stripify(indices: &[u16], vertex_count: usize, cache: u32) -> Option<(Vec<u16>, Vec<u16>)> {
    let list = generate_list(indices, cache);
    if list.len() != indices.len() {
        return None;
    }
    let mut cache_map = vec![-1i32; vertex_count];
    let mut next = 0i32;
    let mut remapped = Vec::with_capacity(list.len());
    let mut permute = vec![0u16; vertex_count];
    for &old in &list {
        let slot = cache_map.get_mut(usize::from(old))?;
        if *slot == -1 {
            *slot = next;
            next += 1;
        }
        let new = *slot as u16;
        remapped.push(new);
        if let Some(p) = permute.get_mut(usize::from(new)) {
            *p = old;
        }
    }
    Some((remapped, permute))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_keeps_every_face_and_renumbers_in_first_use() {
        let n = 8u16;
        let id = |x: u16, z: u16| z * (n + 1) + x;
        let mut idx = Vec::new();
        for z in 0..n {
            for x in 0..n {
                idx.extend_from_slice(&[id(x, z), id(x, z + 1), id(x + 1, z + 1)]);
                idx.extend_from_slice(&[id(x, z), id(x + 1, z + 1), id(x + 1, z)]);
            }
        }
        let vc = usize::from((n + 1) * (n + 1));
        let (out, permute) = stripify(&idx, vc, 24).unwrap();
        assert_eq!(out.len(), idx.len());
        // the faces are the same, as sets of corners through the permutation
        let mut a: Vec<[u16; 3]> = idx
            .chunks(3)
            .map(|t| {
                let mut k = [t[0], t[1], t[2]];
                k.sort();
                k
            })
            .collect();
        let mut b: Vec<[u16; 3]> = out
            .chunks(3)
            .map(|t| {
                let mut k =
                    [permute[usize::from(t[0])], permute[usize::from(t[1])], permute[usize::from(t[2])]];
                k.sort();
                k
            })
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        // first use gives 0, 1, 2 to the first face
        assert_eq!(&out[..3], &[0, 1, 2]);
    }
}

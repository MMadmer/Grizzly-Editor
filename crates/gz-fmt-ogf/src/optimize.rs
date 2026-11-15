//! The face and vertex orders the skeleton exporter gives a split: the face and vertex
//! optimisation of the exporter's tools, over the in-tree optimiser
//! (Tom Forsyth's lookahead with a two-level FIFO cache model,
//! quality mode) and its binary heap. Operation for operation, float for float.
//!
//! The X-Ray SDK runs the outermost search as a recursion as deep as the mesh has faces; here it is
//! a loop. What the recursion undoes after its deepest call is never read again, so the order
//! that comes out is the same.

use std::collections::{BTreeSet, HashMap, VecDeque};

const VALENCE_BOOST: [f32; 5] = [0.0, 0.68, 1.0, 1.1, 1.15];
const CUTOFF_FACTOR: f32 = 2.0;
const EXPENSIVE_FACTOR: f32 = 1.5;
/// `c_ctiCacheTypeSize`: FIFO length and miss score.
const CACHE_TYPES: [(usize, f32); 2] = [(20, 0.95), (3, 0.05)];
const UNUSED: u16 = 0xFFFF;

/// A float as a key that orders like `<` (no NaN comes here; -0 is 0).
pub(crate) fn order_key(x: f32) -> i32 {
    let x = if x == 0.0 { 0.0 } else { x };
    let b = x.to_bits() as i32;
    b ^ (((b >> 31) as u32) >> 1) as i32
}

fn boost(valence: i32) -> f32 {
    let i = usize::try_from(valence).unwrap_or(0);
    VALENCE_BOOST[i.min(VALENCE_BOOST.len() - 1)]
}

#[derive(Clone, Copy)]
struct Blob {
    thing: usize,
    sort: f32,
}

/// `BinaryHeap<THING, float>`: the largest sort on top.
struct Heap {
    blobs: Vec<Blob>,
    size: usize,
    pos: usize,
}

impl Heap {
    fn new() -> Self {
        Self { blobs: Vec::new(), size: 0, pos: 0 }
    }

    fn slot(&mut self, i: usize) -> &mut Blob {
        if self.blobs.len() <= i {
            self.blobs.resize(i + 1, Blob { thing: usize::MAX, sort: 0.0 });
        }
        &mut self.blobs[i]
    }

    fn sort_at(&self, i: usize) -> f32 {
        self.blobs.get(i).map_or(0.0, |b| b.sort)
    }

    fn add(&mut self, thing: usize, sort: f32) {
        if self.size <= 1 {
            self.size = 1;
        }
        let mut pos = self.size;
        while pos > 1 && self.sort_at(pos >> 1) < sort {
            let up = *self.slot(pos >> 1);
            *self.slot(pos) = up;
            pos >>= 1;
        }
        *self.slot(pos) = Blob { thing, sort };
        self.pos = pos;
        self.size += 1;
    }

    fn find_first(&mut self) -> Option<usize> {
        if self.size > 1 {
            self.pos = 1;
            Some(self.blobs[1].thing)
        } else {
            self.pos = 0;
            None
        }
    }

    fn current_sort(&self) -> f32 {
        self.sort_at(self.pos)
    }

    fn remove_first(&mut self) {
        self.pos = 1;
        if self.pos < self.size.saturating_sub(1) {
            let mut pos = self.pos;
            let moved = self.sort_at(self.size - 1);
            while pos > 1 && self.sort_at(pos >> 1) < moved {
                let up = *self.slot(pos >> 1);
                *self.slot(pos) = up;
                pos >>= 1;
            }
            self.size -= 1;
            while (pos << 1) < self.size {
                let l = pos << 1;
                let r = l + 1;
                if moved >= self.sort_at(l) && (r >= self.size || moved >= self.sort_at(r)) {
                    break;
                }
                let next = if r < self.size && self.sort_at(l) < self.sort_at(r) { r } else { l };
                let down = *self.slot(next);
                *self.slot(pos) = down;
                pos = next;
            }
            let last = *self.slot(self.size);
            *self.slot(pos) = last;
            self.slot(self.size).thing = usize::MAX;
            self.pos = pos;
        } else if self.pos + 1 == self.size {
            self.size -= 1;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CacheTypeTri {
    added: usize,
    binned: [u16; 4],
}

#[derive(Clone, Copy)]
struct CacheTri {
    index: [u16; 3],
    types: [CacheTypeTri; 2],
}

struct CacheState {
    misses: i32,
    fifo: Vec<u16>,
    write: usize,
}

struct Cache {
    refcount: Vec<[u8; 2]>,
    history: Vec<CacheTri>,
    states: [CacheState; 2],
}

impl Cache {
    fn new(vertices: usize) -> Self {
        let state = |len: usize| CacheState { misses: 0, fifo: vec![UNUSED; len], write: 0 };
        Self {
            refcount: vec![[0; 2]; vertices],
            history: Vec::new(),
            states: [state(CACHE_TYPES[0].0), state(CACHE_TYPES[1].0)],
        }
    }

    fn rc(&self, v: u16, t: usize) -> u8 {
        self.refcount.get(usize::from(v)).map_or(0, |r| r[t])
    }

    fn rc_mut(&mut self, v: u16, t: usize, delta: i8) {
        if let Some(r) = self.refcount.get_mut(usize::from(v)) {
            r[t] = r[t].wrapping_add_signed(delta);
        }
    }

    /// The extra misses this tri costs; `dry` only measures.
    fn add(&mut self, index: [u16; 3], dry: bool) -> f32 {
        let mut record = CacheTri { index, types: [CacheTypeTri::default(); 2] };
        let mut total = 0.0f32;
        for (t, &(len, miss_score)) in CACHE_TYPES.iter().enumerate() {
            let before = self.states[t].misses;
            let mut found = [false; 3];
            for (k, f) in found.iter_mut().enumerate() {
                *f = self.rc(index[k], t) > 0;
            }
            if dry {
                for f in found {
                    if !f {
                        total += miss_score;
                    }
                }
                continue;
            }
            for k in 0..3 {
                if found[k] {
                    continue;
                }
                let st = &self.states[t];
                let old = st.fifo[st.write];
                let tt = &mut record.types[t];
                tt.binned[tt.added] = old;
                tt.added += 1;
                if old != UNUSED {
                    self.rc_mut(old, t, -1);
                }
                let st = &mut self.states[t];
                st.fifo[st.write] = index[k];
                self.rc_mut(index[k], t, 1);
                let st = &mut self.states[t];
                st.write += 1;
                if st.write >= len {
                    st.write = 0;
                }
                st.misses += 1;
                if k == 0 {
                    if index[0] == index[1] {
                        found[1] = true;
                    }
                    if index[0] == index[2] {
                        found[2] = true;
                    }
                } else if k == 1 && index[1] == index[2] {
                    found[2] = true;
                }
            }
            total += miss_score * (self.states[t].misses - before) as f32;
        }
        if !dry {
            self.history.push(record);
        }
        total
    }

    /// What [`Self::add`] answers when it only measures.
    fn dry(&self, index: [u16; 3]) -> f32 {
        let mut total = 0.0f32;
        for (t, &(_, miss_score)) in CACHE_TYPES.iter().enumerate() {
            for &v in &index {
                if self.rc(v, t) == 0 {
                    total += miss_score;
                }
            }
        }
        total
    }

    /// Removes a triangle from the cache.
    fn remove(&mut self) {
        let Some(ct) = self.history.pop() else { return };
        for (t, &(len, _)) in CACHE_TYPES.iter().enumerate() {
            for i in (0..ct.types[t].added).rev() {
                let st = &mut self.states[t];
                st.write = if st.write == 0 { len - 1 } else { st.write - 1 };
                let old = st.fifo[st.write];
                self.rc_mut(old, t, -1);
                let back = ct.types[t].binned[i];
                let st = &mut self.states[t];
                st.fifo[st.write] = back;
                if back != UNUSED {
                    self.rc_mut(back, t, 1);
                }
                self.states[t].misses -= 1;
            }
        }
    }

    fn last(&self) -> Option<[u16; 3]> {
        self.history.last().map(|c| c.index)
    }
}

struct Tri {
    index: [u16; 3],
    used: bool,
    allowed: bool,
}

struct Optimizer {
    valence: Vec<i32>,
    tris_of: Vec<Vec<usize>>,
    tris: Vec<Tri>,
    slots: Vec<usize>,
    cache: Cache,
    result: Vec<u16>,
    lookahead: i32,
    cutoff: i32,
}

impl Optimizer {
    fn tri_valence_score(&self, t: usize) -> f32 {
        let mut s = 0.0f32;
        for v in self.tris[t].index {
            s += boost(self.valence[usize::from(v)] - 1);
        }
        s
    }

    fn use_tri(&mut self, t: usize) -> f32 {
        let mut s = 0.0f32;
        for v in self.tris[t].index {
            let val = &mut self.valence[usize::from(v)];
            *val -= 1;
            s += boost(*val);
        }
        s
    }

    fn unuse_tri(&mut self, t: usize) {
        for v in self.tris[t].index {
            self.valence[usize::from(v)] += 1;
        }
    }

    fn put(&mut self, out: usize, t: usize) {
        let index = self.tris[t].index;
        if let Some(slot) = self.result.get_mut(out..out + 3) {
            slot.copy_from_slice(&index);
        }
    }

    /// Marks the tris around the last one as allowed; `None` when none is free.
    fn allow_neighbours(&mut self, prev: [u16; 3], local: Option<&mut Vec<usize>>) -> usize {
        let mut n = 0;
        let mut local = local;
        for v in prev {
            for i in 0..self.tris_of[usize::from(v)].len() {
                let t = self.tris_of[usize::from(v)][i];
                let tri = &mut self.tris[t];
                if !tri.used && !tri.allowed {
                    tri.allowed = true;
                    if let Some(l) = local.as_deref_mut() {
                        l.push(t);
                    }
                    n += 1;
                }
            }
        }
        n
    }

    fn clear_allowed(&mut self, prev: [u16; 3], only_unused: bool) {
        for v in prev {
            for i in 0..self.tris_of[usize::from(v)].len() {
                let t = self.tris_of[usize::from(v)][i];
                let tri = &mut self.tris[t];
                if !only_unused || !tri.used {
                    tri.allowed = false;
                }
            }
        }
    }

    /// The best score with lookahead.
    fn lookahead_score(&mut self, countdown: i32, current: f32, input_best: f32, out: usize) -> f32 {
        let la = countdown.min(self.lookahead);
        let mut best = input_best;
        let average = (best - current) / la as f32;
        let expensive = average * EXPENSIVE_FACTOR;

        let mut local = Vec::new();
        let prev = self.cache.last().unwrap_or([UNUSED; 3]);
        let mut no_mad = true;
        let allowed = if prev[0] == UNUSED { 0 } else { self.allow_neighbours(prev, Some(&mut local)) };
        if allowed < 1 {
            local.clear();
            no_mad = false;
            if prev[0] != UNUSED {
                self.clear_allowed(prev, true);
            }
            local.extend((0..self.tris.len()).filter(|&t| !self.tris[t].used));
        }

        let mut heap = Heap::new();
        for &t in local.iter().rev() {
            if no_mad && !self.tris[t].allowed {
                continue;
            }
            let mut score = self.tri_valence_score(t);
            if score > expensive || current + score >= best {
                continue;
            }
            score += self.cache.add(self.tris[t].index, true);
            if score > expensive || current + score >= best {
                continue;
            }
            heap.add(t, -score);
        }
        if no_mad {
            self.clear_allowed(prev, false);
        }

        let mut best_tri = None;
        let Some(mut cur) = heap.find_first() else { return best };
        let cutoff = CUTOFF_FACTOR * heap.current_sort();
        let mut tried = 0;
        loop {
            let neg = heap.current_sort();
            heap.remove_first();
            if neg < cutoff {
                break;
            }
            let mut score = current - neg;
            if score >= best {
                break;
            }
            if no_mad {
                tried += 1;
                if tried > self.cutoff {
                    break;
                }
            }
            self.use_tri(cur);
            self.cache.add(self.tris[cur].index, false);
            self.tris[cur].used = true;
            if la > 1 {
                score = self.lookahead_score(la - 1, score, best, out + 3);
            }
            if score < best {
                best = score;
                best_tri = Some(cur);
            }
            self.cache.remove();
            self.tris[cur].used = false;
            self.unuse_tri(cur);
            match heap.find_first() {
                Some(next) => cur = next,
                None => break,
            }
        }

        let Some(b) = best_tri else { return input_best };
        let mut new_best = best;
        self.put(out, b);
        if countdown > 1 {
            let mut score = self.cache.add(self.tris[b].index, false);
            self.tris[b].used = true;
            let valence = self.use_tri(b);
            score += valence + current;
            new_best = self.lookahead_score(countdown - 1, score, 1e10, out + 3);
            self.cache.remove();
            self.tris[b].used = false;
            self.unuse_tri(b);
        }
        new_best
    }

    /// What [`Self::best_order`] does without a lookahead, kept in order instead of scanned. With
    /// a lookahead of one every step scores every remaining tri (no bound can cut one: a score
    /// stays under 7 and the sum under the bound of 1e10), builds a heap of them and takes its
    /// top - the first tri in slot order of the smallest score; the second candidate the heap
    /// gives never beats it. So a step is the smallest (score, slot), and a step changes the
    /// score only of the tris around the vertices whose valence or cache presence it changed.
    fn best_order_cheap(&mut self) {
        let n = self.tris.len();
        let score_of = |o: &Self, t: usize| -> f32 {
            let mut score = o.tri_valence_score(t);
            score += o.cache.dry(o.tris[t].index);
            score
        };
        let mut slot_of = vec![0usize; n];
        for (s, &t) in self.slots.iter().enumerate() {
            slot_of[t] = s;
        }
        let mut key = vec![0i32; n];
        let mut order: BTreeSet<(i32, usize)> = BTreeSet::new();
        for (t, k) in key.iter_mut().enumerate() {
            *k = order_key(score_of(self, t));
            order.insert((*k, slot_of[t]));
        }
        let mut touched: Vec<usize> = Vec::new();
        for base in 0..n {
            if base + 1 == n {
                let t = self.slots[base];
                self.put(base * 3, t);
                return;
            }
            let Some(&(_, bs)) = order.first() else { return };
            let b = self.slots[bs];
            self.put(base * 3, b);
            order.remove(&(key[b], bs));
            // the tri at the base takes the chosen one's slot
            let moved = self.slots[base];
            if moved != b {
                order.remove(&(key[moved], base));
                order.insert((key[moved], bs));
            }
            self.slots[bs] = moved;
            self.slots[base] = b;
            slot_of[moved] = bs;
            slot_of[b] = base;
            self.cache.add(self.tris[b].index, false);
            self.tris[b].used = true;
            self.use_tri(b);
            // the vertices whose valence changed and those the cache took in or let go
            touched.clear();
            touched.extend(self.tris[b].index.iter().map(|&v| usize::from(v)));
            if let Some(last) = self.cache.history.last() {
                for ty in &last.types {
                    touched.extend(
                        ty.binned[..ty.added].iter().filter(|&&v| v != UNUSED).map(|&v| usize::from(v)),
                    );
                }
            }
            touched.sort_unstable();
            touched.dedup();
            for &v in &touched {
                for i in 0..self.tris_of.get(v).map_or(0, Vec::len) {
                    let t = self.tris_of[v][i];
                    if self.tris[t].used {
                        continue;
                    }
                    let k = order_key(score_of(self, t));
                    if k != key[t] {
                        order.remove(&(key[t], slot_of[t]));
                        key[t] = k;
                        order.insert((k, slot_of[t]));
                    }
                }
            }
        }
    }

    /// The best score, its outer recursion unrolled.
    fn best_order(&mut self) {
        let mut countdown = self.tris.len() as i32;
        let mut current = 0.0f32;
        let mut input_best = 1e10f32;
        let mut base = 0usize;
        let mut num_tris = countdown;
        let mut out = 0usize;
        loop {
            let mut la = countdown.min(self.lookahead);
            if num_tris > 100 && current < 50.0 && la > 2 {
                la = 2;
            }
            let mut best = input_best;
            let average = (best - current) / la as f32;
            let expensive = average * EXPENSIVE_FACTOR;
            if num_tris == 1 {
                let t = self.slots[base];
                self.put(out, t);
                return;
            }

            let mut no_mad = false;
            let mut prev = [UNUSED; 3];
            if countdown == la
                && let Some(p) = self.cache.last()
            {
                prev = p;
                no_mad = true;
                if self.allow_neighbours(prev, None) < 1 {
                    no_mad = false;
                    self.clear_allowed(prev, true);
                }
            }

            let mut heap = Heap::new();
            for i in 0..num_tris as usize {
                let slot = base + i;
                let t = self.slots[slot];
                if no_mad && !self.tris[t].allowed {
                    continue;
                }
                let mut score = self.tri_valence_score(t);
                if score > expensive || current + score >= best {
                    continue;
                }
                score += self.cache.add(self.tris[t].index, true);
                if score > expensive || current + score >= best {
                    continue;
                }
                heap.add(slot, -score);
            }
            if no_mad {
                self.clear_allowed(prev, false);
            }

            let Some(mut cur_slot) = heap.find_first() else { return };
            let cutoff = CUTOFF_FACTOR * heap.current_sort();
            let mut best_slot = None;
            let mut tried = 0;
            loop {
                let t = self.slots[cur_slot];
                let neg = heap.current_sort();
                heap.remove_first();
                if neg < cutoff {
                    break;
                }
                let mut score = current - neg;
                if score >= best {
                    break;
                }
                if no_mad {
                    tried += 1;
                    if tried > self.cutoff {
                        break;
                    }
                }
                self.use_tri(t);
                self.cache.add(self.tris[t].index, false);
                self.tris[t].used = true;
                if la > 1 {
                    score = self.lookahead_score(la - 1, score, best, out + 3);
                }
                if score < best {
                    best = score;
                    best_slot = Some(cur_slot);
                }
                self.cache.remove();
                self.tris[t].used = false;
                self.unuse_tri(t);
                match heap.find_first() {
                    Some(next) => cur_slot = next,
                    None => break,
                }
            }

            // the X-Ray SDK asserts a best was found; the first candidate stands in
            let bs = best_slot.unwrap_or(cur_slot);
            let b = self.slots[bs];
            self.put(out, b);
            if countdown <= 1 {
                return;
            }
            self.slots[bs] = self.slots[base];
            self.slots[base] = b;
            let mut score = self.cache.add(self.tris[b].index, false);
            self.tris[b].used = true;
            let valence = self.use_tri(b);
            score += valence + current;

            countdown -= 1;
            current = score;
            input_best = 1e10;
            base += 1;
            num_tris -= 1;
            out += 3;
        }
    }
}

/// The faces of `indices` in a cache-friendly
/// order. Mode 1 is the cheap one (`LOOKAHEAD` 1, cutoff 4), any other the quality one (5, 16)
/// - the progressive pass asks for 1, the face optimiser of a plain skeleton for 2.
pub(crate) fn optimise_tri_list(indices: &mut [u16], mode: u32) {
    let n = indices.len() / 3;
    if n <= 2 {
        return;
    }
    let (lookahead, cutoff) = if mode == 1 { (1, 4) } else { (5, 16) };
    // the vertices numbered densely: the search compares them only for being the same, and a
    // short list of a large mesh then costs its own size, not the mesh's (the progressive pass
    // orders a few faces per collapse); the sentinel's number is left alone
    let mut used: Vec<u16> = indices[..n * 3].to_vec();
    used.sort_unstable();
    used.dedup();
    let dense = used.last().is_some_and(|&v| v != UNUSED);
    let id = |v: u16| if dense { used.binary_search(&v).map_or(v, |i| i as u16) } else { v };
    let biggest = if dense { used.len() - 1 } else { indices.iter().copied().max().map_or(0, usize::from) };
    let mut valence = vec![0i32; biggest + 1];
    let mut tris_of = vec![Vec::new(); biggest + 1];
    let mut tris = Vec::with_capacity(n);
    for (i, f) in indices.as_chunks::<3>().0.iter().enumerate() {
        let f = [id(f[0]), id(f[1]), id(f[2])];
        for &v in &f {
            valence[usize::from(v)] += 1;
            tris_of[usize::from(v)].push(i);
        }
        tris.push(Tri { index: f, used: false, allowed: false });
    }
    let mut o = Optimizer {
        valence,
        tris_of,
        tris,
        slots: (0..n).collect(),
        cache: Cache::new(biggest + 1),
        result: vec![0; n * 3],
        lookahead,
        cutoff,
    };
    // a sum of steps under 10 each never reaches the search's bound of 1e10 (see `best_order_cheap`)
    if lookahead == 1 && n < 1 << 24 {
        o.best_order_cheap();
    } else {
        o.best_order();
    }
    for (slot, &v) in indices[..n * 3].iter_mut().zip(&o.result) {
        *slot = if dense { used.get(usize::from(v)).copied().unwrap_or(v) } else { v };
    }
}

/// `remap[new face] = old face`.
#[must_use]
pub fn optimize_faces(indices: &[u16]) -> Vec<u32> {
    let n = indices.len() / 3;
    if n < 3 {
        return (0..n as u32).collect();
    }
    let mut tmp = indices[..n * 3].to_vec();
    optimise_tri_list(&mut tmp, 2);
    let key = |f: &[u16]| (u64::from(f[0]) << 32) | (u64::from(f[1]) << 16) | u64::from(f[2]);
    let mut lookup: HashMap<u64, VecDeque<u32>> = HashMap::new();
    for (i, f) in indices.as_chunks::<3>().0.iter().enumerate() {
        lookup.entry(key(f)).or_default().push_back(i as u32);
    }
    tmp.as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(|(i, f)| lookup.get_mut(&key(f)).and_then(VecDeque::pop_front).unwrap_or(i as u32))
        .collect()
}

/// `remap[old vertex] = new vertex`, in order of first use, the
/// unused ones after.
#[must_use]
pub fn optimize_vertices(indices: &[u16], num_vertices: usize) -> Vec<u32> {
    let mut remap = vec![u32::MAX; num_vertices];
    let mut next = 0u32;
    for &i in indices {
        if let Some(slot) = remap.get_mut(usize::from(i))
            && *slot == u32::MAX
        {
            *slot = next;
            next += 1;
        }
    }
    for slot in &mut remap {
        if *slot == u32::MAX {
            *slot = next;
            next += 1;
        }
    }
    remap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_lists_keep_their_order() {
        assert_eq!(optimize_faces(&[0, 1, 2, 2, 1, 3]), vec![0, 1]);
    }

    #[test]
    fn every_face_comes_out_once() {
        // a strip of a grid, shuffled
        let mut idx = Vec::new();
        for i in 0..20u16 {
            idx.extend_from_slice(&[i, i + 1, i + 21]);
            idx.extend_from_slice(&[i + 1, i + 22, i + 21]);
        }
        let remap = optimize_faces(&idx);
        let mut seen = remap.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..40).collect::<Vec<_>>());
    }

    #[test]
    fn vertices_follow_their_first_use() {
        // 2 -> 0, 0 -> 1, then the unused 1 and 3 in their own order
        assert_eq!(optimize_vertices(&[2, 0, 2], 4), vec![1, 2, 0, 3]);
    }

    /// Both searches of the cheap mode over one list.
    fn both_orders(indices: &[u16]) -> (Vec<u16>, Vec<u16>) {
        let run = |cheap: bool| {
            let n = indices.len() / 3;
            let biggest = indices.iter().copied().max().map_or(0, usize::from);
            let mut valence = vec![0i32; biggest + 1];
            let mut tris_of = vec![Vec::new(); biggest + 1];
            let mut tris = Vec::new();
            for (i, f) in indices.as_chunks::<3>().0.iter().enumerate() {
                for &v in f {
                    valence[usize::from(v)] += 1;
                    tris_of[usize::from(v)].push(i);
                }
                tris.push(Tri { index: [f[0], f[1], f[2]], used: false, allowed: false });
            }
            let mut o = Optimizer {
                valence,
                tris_of,
                tris,
                slots: (0..n).collect(),
                cache: Cache::new(biggest + 1),
                result: vec![0; n * 3],
                lookahead: 1,
                cutoff: 4,
            };
            if cheap {
                o.best_order_cheap();
            } else {
                o.best_order();
            }
            o.result
        };
        (run(true), run(false))
    }

    #[test]
    fn the_kept_order_is_the_scanned_one() {
        let mut lists: Vec<Vec<u16>> = Vec::new();
        // grids of several sizes, their faces in a scrambled order
        for n in [3u16, 9, 30, 70] {
            let id = |x: u16, z: u16| z * (n + 1) + x;
            let mut faces = Vec::new();
            for z in 0..n {
                for x in 0..n {
                    faces.push([id(x, z), id(x, z + 1), id(x + 1, z + 1)]);
                    faces.push([id(x, z), id(x + 1, z + 1), id(x + 1, z)]);
                }
            }
            let len = faces.len();
            for i in 0..len {
                faces.swap(i, (i * 7919 + 13) % len);
            }
            lists.push(faces.into_iter().flatten().collect());
        }
        // a fan around one vertex of high valence, islands, repeated corners
        let mut fan = Vec::new();
        for i in 1..40u16 {
            fan.extend_from_slice(&[0, i, i + 1]);
        }
        for i in 0..30u16 {
            let b = 100 + i * 3;
            fan.extend_from_slice(&[b, b + 1, b + 2]);
        }
        fan.extend_from_slice(&[5, 5, 6, 7, 7, 7, 8, 9, 8]);
        lists.push(fan);
        // a pseudo-random soup over a few vertices: many equal scores
        let mut x = 12345u32;
        let soup: Vec<u16> = (0..900)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                ((x >> 16) % 60) as u16
            })
            .collect();
        lists.push(soup);
        for (i, l) in lists.iter().enumerate() {
            let (cheap, scanned) = both_orders(l);
            assert_eq!(cheap, scanned, "list {i}");
        }
    }

    #[test]
    fn the_heap_hands_out_the_largest_first() {
        let mut h = Heap::new();
        for (t, s) in [(0, -3.0), (1, -1.0), (2, -2.0), (3, -0.5)] {
            h.add(t, s);
        }
        let mut order = Vec::new();
        while let Some(t) = h.find_first() {
            order.push(t);
            h.remove_first();
        }
        assert_eq!(order, vec![3, 1, 2, 0]);
    }
}

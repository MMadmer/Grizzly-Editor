//! The face order of D3DX's face optimisation (the fast path of the stripifier). D3DX9 is
//! closed code; this is Tom Forsyth's linear-speed vertex cache optimisation, the published
//! method of the same goal: every step emits the face whose corners score best against a
//! 32-entry LRU cache and their remaining use, in time linear in the faces. Ties go to the
//! lower face index, so the order is the same on any machine.

const CACHE: usize = 32;
const DECAY: f32 = 1.5;
const LAST_TRI: f32 = 0.75;
const VALENCE_SCALE: f32 = 2.0;
const VALENCE_POWER: f32 = 0.5;

fn vertex_score(cache_pos: Option<usize>, remaining: u32) -> f32 {
    if remaining == 0 {
        return -1.0;
    }
    let mut s = match cache_pos {
        None => 0.0,
        Some(p) if p < 3 => LAST_TRI,
        Some(p) => (1.0 - (p - 3) as f32 / (CACHE - 3) as f32).powf(DECAY),
    };
    s += VALENCE_SCALE * (remaining as f32).powf(-VALENCE_POWER);
    s
}

/// `remap[new face] = old face`.
#[must_use]
pub fn optimize_faces(indices: &[u16], vertex_count: usize) -> Vec<u32> {
    let n = indices.len() / 3;
    if n < 3 {
        return (0..n as u32).collect();
    }
    let vcount = vertex_count.max(indices.iter().copied().max().map_or(0, |m| usize::from(m) + 1));
    // the faces of each vertex, as one flat table
    let mut start = vec![0usize; vcount + 1];
    for &v in &indices[..n * 3] {
        start[usize::from(v) + 1] += 1;
    }
    for i in 0..vcount {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut tris_of = vec![0u32; n * 3];
    for (t, f) in indices[..n * 3].as_chunks::<3>().0.iter().enumerate() {
        for &v in f {
            let s = &mut fill[usize::from(v)];
            tris_of[*s] = t as u32;
            *s += 1;
        }
    }
    let mut remaining: Vec<u32> = (0..vcount).map(|v| (start[v + 1] - start[v]) as u32).collect();
    let mut cache_pos: Vec<Option<usize>> = vec![None; vcount];
    let mut vscore: Vec<f32> = (0..vcount).map(|v| vertex_score(None, remaining[v])).collect();
    let corner = |t: usize| [indices[t * 3], indices[t * 3 + 1], indices[t * 3 + 2]];
    let mut tscore: Vec<f32> =
        (0..n).map(|t| corner(t).iter().map(|&v| vscore[usize::from(v)]).sum()).collect();
    let mut added = vec![false; n];
    let mut out = Vec::with_capacity(n);
    let mut cache: Vec<u16> = Vec::with_capacity(CACHE + 3);
    let mut scan = 0usize;
    let best_of_all = |added: &[bool], tscore: &[f32], from: usize| -> Option<usize> {
        let mut best: Option<usize> = None;
        for t in from..n {
            if !added[t] && best.is_none_or(|b| tscore[t] > tscore[b]) {
                best = Some(t);
            }
        }
        best
    };
    let mut next = best_of_all(&added, &tscore, 0);
    while let Some(t) = next {
        added[t] = true;
        out.push(t as u32);
        // the face's corners to the front of the cache, the rest pushed back
        let c = corner(t);
        for &v in &c {
            remaining[usize::from(v)] -= 1;
        }
        let mut grown: Vec<u16> = Vec::with_capacity(CACHE + 3);
        for &v in &c {
            if !grown.contains(&v) {
                grown.push(v);
            }
        }
        for &v in &cache {
            if !grown.contains(&v) {
                grown.push(v);
            }
        }
        for (i, &v) in grown.iter().enumerate() {
            cache_pos[usize::from(v)] = (i < CACHE).then_some(i);
        }
        // every vertex that moved, and every face of it, scored again
        for &v in &grown {
            let vi = usize::from(v);
            let old = vscore[vi];
            let new = vertex_score(cache_pos[vi], remaining[vi]);
            vscore[vi] = new;
            for &f in &tris_of[start[vi]..start[vi + 1]] {
                tscore[f as usize] += new - old;
            }
        }
        grown.truncate(CACHE);
        cache = grown;
        // the best face around the cache, else the best of all
        next = None;
        for &v in &cache {
            let vi = usize::from(v);
            for &f in &tris_of[start[vi]..start[vi + 1]] {
                let f = f as usize;
                if added[f] {
                    continue;
                }
                let better = match next {
                    None => true,
                    Some(b) => tscore[f] > tscore[b] || (tscore[f] == tscore[b] && f < b),
                };
                if better {
                    next = Some(f);
                }
            }
        }
        if next.is_none() {
            while scan < n && added[scan] {
                scan += 1;
            }
            next = best_of_all(&added, &tscore, scan);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_once_and_neighbours_together() {
        let n = 40u16;
        let id = |x: u16, z: u16| z * (n + 1) + x;
        let mut idx = Vec::new();
        for z in 0..n {
            for x in 0..n {
                idx.extend_from_slice(&[id(x, z), id(x, z + 1), id(x + 1, z + 1)]);
                idx.extend_from_slice(&[id(x, z), id(x + 1, z + 1), id(x + 1, z)]);
            }
        }
        let vc = usize::from((n + 1) * (n + 1));
        let order = optimize_faces(&idx, vc);
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..(n as u32 * n as u32 * 2)).collect::<Vec<_>>());
        // a 32-entry FIFO sees fewer misses than in the row order
        let misses = |o: &[u32]| {
            let mut fifo: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
            let mut m = 0;
            for &t in o {
                for &v in &idx[t as usize * 3..t as usize * 3 + 3] {
                    if !fifo.contains(&v) {
                        m += 1;
                        fifo.push_front(v);
                        fifo.truncate(24);
                    }
                }
            }
            m
        };
        let rows: Vec<u32> = (0..order.len() as u32).collect();
        assert!(misses(&order) < misses(&rows));
    }
}

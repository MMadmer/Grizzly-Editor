//! The auto layout and the arrangement of a selection (align, distribute, straighten).
//!
//! The layout works in two axes of the flow: *depth* (the way the graph flows: x flowing right,
//! y flowing down) and *breadth* (across it). It is deterministic and runs off the UI thread:
//!
//! 1. **Which wires are the flow.** A wire inside a cycle that leads back to a node no deeper
//!    from the start than its source is a loop back (a dialog's "back to the menu", a retry); every
//!    other wire flows forward. Loops are drawn routed around the nodes and do not move anything.
//! 2. **Who owns whom.** The dominator tree of the forward wires: a node owns what can only be
//!    reached through it, so a branch keeps its nodes together and a node several branches reach
//!    (a merge) sits after all of them, between them.
//! 3. **Blocks.** A node is laid out with what it owns after it in depth: its children side by side
//!    in breadth, in pin order, a merge in a band of its own after the branches that feed it. A band
//!    of six children and more **wraps** into rows several deep instead of one ribbon.
//! 4. **Folding.** A chain longer than the graph is broad folds into lines - text-like flowing right,
//!    newspaper columns flowing down - at the depth that frames the graph largest in a 16:10 view.

use rustc_hash::FxHashMap;

use crate::asset::Graph;
use crate::canvas::Flow;
use crate::catalog::Catalog;
use crate::layout::{self, BoxCx, HEADER, snap};

/// The room the layout keeps, world pt.
#[derive(Clone, Copy, Debug)]
pub struct Gaps {
    /// Between a node and what follows it.
    pub depth: f32,
    /// Between siblings.
    pub breadth: f32,
    /// Between two lines of a folded chain.
    pub fold: f32,
    /// Between two trigger trees.
    pub tree: f32,
}

#[must_use]
pub fn gaps(flow: Flow) -> Gaps {
    match flow {
        // the wire's pin label sits on its first stretch: a flow to the right needs room across
        Flow::Right => Gaps { depth: 56.0, breadth: 28.0, fold: 72.0, tree: 64.0 },
        Flow::Down => Gaps { depth: 56.0, breadth: 40.0, fold: 96.0, tree: 96.0 },
    }
}

/// The view the layout frames best: the canvas of a window of 1600 x 900 with both panels open.
const VIEW: [f32; 2] = [1000.0, 800.0];
/// A band of this many children wraps.
const WRAP_MIN: usize = 6;
/// The virtual root above every start of the graph.
const ROOT: usize = usize::MAX;

/// The forward wires of the nodes laid out and what owns whom.
struct Net {
    n: usize,
    /// Depth and breadth sizes, and where the node's axis lies across it.
    d: Vec<f32>,
    b: Vec<f32>,
    axis: Vec<f32>,
    /// The forward wires into a node, from whom.
    preds: Vec<Vec<usize>>,
    topo: Vec<usize>,
    /// The dominator of each node ([`ROOT`] for the starts).
    idom: Vec<usize>,
    /// The children of each node in the dominator tree, and of the root (last).
    kids: Vec<Vec<usize>>,
    /// The band of each child among its owner's: after the siblings that lead into it.
    band: Vec<usize>,
    /// The loose nodes: no wire comes in, it is no trigger, and every node it leads to is reached
    /// from elsewhere too - a node whose input is not wired yet, no start of anything. It shapes
    /// nothing and goes in front of the first node it leads to (the targets, in pin order).
    loose: Vec<(usize, Vec<usize>)>,
}

fn kid_slot(n: usize, v: usize) -> usize {
    if v == ROOT { n } else { v }
}

/// Strongly connected components (iterative Tarjan): a component number per node.
fn components(n: usize, adj: &[Vec<usize>]) -> Vec<usize> {
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on = vec![false; n];
    let mut comp = vec![usize::MAX; n];
    let mut stack: Vec<usize> = Vec::new();
    let (mut next, mut count) = (0usize, 0usize);
    for s in 0..n {
        if index[s] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(s, 0)];
        index[s] = next;
        low[s] = next;
        next += 1;
        stack.push(s);
        on[s] = true;
        while let Some(&mut (v, ref mut i)) = call.last_mut() {
            if let Some(&w) = adj[v].get(*i) {
                *i += 1;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                while let Some(w) = stack.pop() {
                    on[w] = false;
                    comp[w] = count;
                    if w == v {
                        break;
                    }
                }
                count += 1;
            }
        }
    }
    comp
}

fn build(cx: &BoxCx<'_>, g: &Graph, set: &[usize]) -> Net {
    let n = set.len();
    let mut local: FxHashMap<&str, usize> = FxHashMap::default();
    for (i, &gi) in set.iter().enumerate() {
        if let Some(node) = g.nodes.get(gi) {
            local.entry(node.id.as_str()).or_insert(i);
        }
    }
    let mut d = vec![0.0; n];
    let mut b = vec![0.0; n];
    let mut axis = vec![0.0; n];
    let mut trig = vec![false; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, &gi) in set.iter().enumerate() {
        let Some(node) = g.nodes.get(gi) else { continue };
        let s = layout::node_size(cx, node);
        (d[u], b[u], axis[u]) = match cx.flow {
            Flow::Right => (s[0], s[1], HEADER * 0.5),
            Flow::Down => (s[1], s[0], s[0] * 0.5),
        };
        trig[u] = cx.catalog.is_trigger(&node.kind);
    }
    for (u, &gi) in set.iter().enumerate() {
        let Some(node) = g.nodes.get(gi) else { continue };
        let (pins, _) = layout::pins(cx.catalog, node);
        for pin in &pins {
            let Some(p) = node.out.iter().find(|p| p.name.as_str() == pin) else { continue };
            for t in &p.targets {
                let Some(&v) = local.get(t.as_str()) else { continue };
                if v != u && !trig[v] && !adj[u].contains(&v) {
                    adj[u].push(v);
                }
            }
        }
    }
    // how deep each node lies from the starts
    let mut indeg = vec![0usize; n];
    for list in &adj {
        for &v in list {
            indeg[v] += 1;
        }
    }
    let mut depth = vec![usize::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for u in 0..n {
        if trig[u] || indeg[u] == 0 {
            depth[u] = 0;
            queue.push_back(u);
        }
    }
    let mut start = 0;
    loop {
        while let Some(u) = queue.pop_front() {
            for &v in &adj[u] {
                if depth[v] == usize::MAX {
                    depth[v] = depth[u] + 1;
                    queue.push_back(v);
                }
            }
        }
        // a cycle nothing enters starts at its first node
        while start < n && depth[start] != usize::MAX {
            start += 1;
        }
        if start >= n {
            break;
        }
        depth[start] = 0;
        queue.push_back(start);
    }
    let comp = components(n, &adj);
    let loose: Vec<(usize, Vec<usize>)> = (0..n)
        .filter(|&u| !trig[u] && indeg[u] == 0 && !adj[u].is_empty())
        .filter(|&u| adj[u].iter().all(|&v| (0..n).any(|w| w != u && adj[w].contains(&v))))
        .map(|u| (u, adj[u].clone()))
        .collect();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for u in 0..n {
        if loose.iter().any(|(l, _)| *l == u) {
            continue;
        }
        for &v in &adj[u] {
            if comp[u] != comp[v] || depth[v] > depth[u] {
                out[u].push(v);
                preds[v].push(u);
            }
        }
    }
    // a stable topological order of the forward wires
    let mut left: Vec<usize> = preds.iter().map(Vec::len).collect();
    let mut topo = Vec::with_capacity(n);
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<usize>> =
        (0..n).filter(|&u| left[u] == 0).map(std::cmp::Reverse).collect();
    while let Some(std::cmp::Reverse(u)) = ready.pop() {
        topo.push(u);
        for &v in &out[u] {
            left[v] -= 1;
            if left[v] == 0 {
                ready.push(std::cmp::Reverse(v));
            }
        }
    }
    // the dominators: in a DAG a node's is the meeting point of its predecessors
    let mut idom = vec![ROOT; n];
    let mut dd = vec![0usize; n];
    let depth_of = |v: usize, dd: &[usize]| if v == ROOT { 0 } else { dd[v] };
    for &v in &topo {
        let mut meet: Option<usize> = None;
        for &p in &preds[v] {
            meet = Some(match meet {
                None => p,
                Some(mut a) => {
                    let mut c = p;
                    while a != c {
                        if a == ROOT || c == ROOT {
                            a = ROOT;
                            break;
                        }
                        if depth_of(a, &dd) >= depth_of(c, &dd) {
                            a = idom[a];
                        } else {
                            c = idom[c];
                        }
                    }
                    a
                }
            });
        }
        idom[v] = meet.unwrap_or(ROOT);
        dd[v] = depth_of(idom[v], &dd) + 1;
    }
    // the children of each owner, in the order their wires leave it
    let mut key = vec![0.0f64; n];
    let mut band = vec![0usize; n];
    let branch = |p: usize, owner: usize, idom: &[usize]| -> usize {
        let mut c = p;
        while idom[c] != owner {
            c = idom[c];
            if c == ROOT {
                return p;
            }
        }
        c
    };
    for &v in &topo {
        let owner = idom[v];
        let mut k = f64::MAX;
        let mut bd = 0usize;
        for &p in &preds[v] {
            if p == owner {
                let at = out[p].iter().position(|&x| x == v).unwrap_or(0);
                k = k.min(at as f64);
            } else {
                let c = branch(p, owner, &idom);
                k = k.min(key[c] + 0.5);
                bd = bd.max(band[c] + 1);
            }
        }
        if preds[v].is_empty() {
            k = if trig[v] { v as f64 } else { 1.0e6 + v as f64 };
        }
        key[v] = k;
        band[v] = bd;
    }
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for &v in &topo {
        kids[kid_slot(n, idom[v])].push(v);
    }
    for list in &mut kids {
        list.sort_by(|&x, &y| band[x].cmp(&band[y]).then(key[x].total_cmp(&key[y])).then(x.cmp(&y)));
    }
    Net { n, d, b, axis, preds, topo, idom, kids, band, loose }
}

impl Net {
    /// The child of `owner` whose branch holds `p`.
    fn branch(&self, p: usize, owner: usize) -> usize {
        let mut c = p;
        while let Some(&up) = self.idom.get(c) {
            if up == owner || up == ROOT {
                break;
            }
            c = up;
        }
        c
    }
}

/// A laid-out group of nodes: their top-left corners in (depth, breadth) and its extent.
#[derive(Clone, Debug, Default)]
struct Piece {
    at: Vec<(usize, f32, f32)>,
    dd: f32,
    bb: f32,
    /// Where the head node's axis lies across the piece.
    axis: f32,
}

impl Piece {
    fn shifted(&self, dd: f32, db: f32, into: &mut Vec<(usize, f32, f32)>) {
        into.extend(self.at.iter().map(|&(v, d, b)| (v, d + dd, b + db)));
    }

    /// Moves everything so the piece starts at 0 in both axes; recomputes its extent.
    fn normalise(mut self, net: &Net) -> Self {
        let (mut d0, mut b0, mut d1, mut b1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(v, d, b) in &self.at {
            d0 = d0.min(d);
            b0 = b0.min(b);
            d1 = d1.max(d + net.d[v]);
            b1 = b1.max(b + net.b[v]);
        }
        if self.at.is_empty() {
            return self;
        }
        for e in &mut self.at {
            e.1 -= d0;
            e.2 -= b0;
        }
        self.dd = d1 - d0;
        self.bb = b1 - b0;
        self.axis -= b0;
        self
    }
}

/// The rows a band of pieces is laid out in: one row, or rows of `k` when the band is wide. In a
/// folded graph (`limit`) the band takes as little breadth as its line's depth allows; otherwise
/// the shape that frames best.
fn wrap(flow: Flow, items: &[Piece], gap: &Gaps, limit: f32) -> usize {
    let n = items.len();
    if n < WRAP_MIN {
        return n.max(1);
    }
    if limit < f32::MAX {
        let mut narrow: Option<(f32, usize)> = None;
        for k in 1..=n {
            let mut band_b = 0.0f32;
            let mut band_d = 0.0f32;
            for (r, row) in items.chunks(k).enumerate() {
                let rb: f32 = row.iter().map(|p| p.bb).sum::<f32>() + gap.breadth * (row.len() - 1) as f32;
                band_b = band_b.max(rb);
                band_d +=
                    row.iter().map(|p| p.dd).fold(0.0f32, f32::max) + if r > 0 { gap.depth } else { 0.0 };
            }
            if band_d <= limit && narrow.is_none_or(|(b, _)| band_b < b) {
                narrow = Some((band_b, k));
            }
        }
        if let Some((_, k)) = narrow {
            return k;
        }
    }
    let mut best = (f32::MIN, n);
    for k in 1..=n {
        let mut band_b = 0.0f32;
        let mut band_d = 0.0f32;
        for (r, row) in items.chunks(k).enumerate() {
            let rb: f32 = row.iter().map(|p| p.bb).sum::<f32>() + gap.breadth * (row.len() - 1) as f32;
            let rd = row.iter().map(|p| p.dd).fold(0.0f32, f32::max);
            band_b = band_b.max(rb);
            band_d += rd + if r > 0 { gap.depth } else { 0.0 };
        }
        let score = fit_size(flow, band_d, band_b);
        if score >= best.0 {
            best = (score, k);
        }
    }
    best.1
}

/// Pieces in rows of `k` across, the rows one after the other in depth, each row's axes centred
/// on one line; the axis of the result is that line.
fn grid(net: &Net, items: Vec<Piece>, k: usize, across: f32, gap: &Gaps) -> Piece {
    let mut out = Piece::default();
    let mut depth = 0.0;
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let row: Vec<Piece> = it.by_ref().take(k.max(1)).collect();
        let mut x = 0.0f32;
        let mut placed = Vec::with_capacity(row.len());
        for p in &row {
            placed.push(x);
            x += p.bb + across;
        }
        let mean = row.iter().zip(&placed).map(|(p, x)| x + p.axis).sum::<f32>() / row.len().max(1) as f32;
        let row_d = row.iter().map(|p| p.dd).fold(0.0f32, f32::max);
        for (p, x) in row.iter().zip(&placed) {
            p.shifted(depth, x - mean, &mut out.at);
        }
        depth += row_d + gap.depth;
    }
    out.axis = 0.0;
    out.normalise(net)
}

/// What hangs off a node of a spine: a branch laid out as one piece, where it starts in depth
/// (from the node's own start) and on which side of the spine.
struct Side {
    piece: Piece,
    start: f32,
    neg: bool,
}

/// One node of a spine with what hangs off it, and how far after its start the next node of the
/// spine starts.
struct Seg {
    node: usize,
    sides: Vec<Side>,
    next: f32,
}

/// How much of the breadth beside the spine is taken, by depth: `(from, to, extent)`.
#[derive(Default)]
struct Skyline(Vec<(f32, f32, f32)>);

impl Skyline {
    fn over(&self, d0: f32, d1: f32) -> f32 {
        self.0.iter().filter(|(a, b, _)| *a < d1 && *b > d0).map(|e| e.2).fold(0.0f32, f32::max)
    }

    fn add(&mut self, d0: f32, d1: f32, extent: f32) {
        self.0.push((d0, d1, extent));
    }
}

struct Run<'a> {
    net: &'a Net,
    flow: Flow,
    gap: Gaps,
    limit: f32,
    /// For every node: the depth of the longest spine it starts.
    len: Vec<f32>,
}

impl Run<'_> {
    /// A spine cut into lines no deeper than the limit, each line laid out - its spine on one
    /// axis, its branches hanging off both sides of it - and the lines side by side across.
    fn lines(&self, segs: Vec<Seg>) -> Piece {
        let net = self.net;
        let gap = &self.gap;
        let mut at = Vec::with_capacity(segs.len());
        let mut d = 0.0;
        for s in &segs {
            at.push(d);
            d += s.next;
        }
        // how far each node of the spine reaches in depth with what hangs off it
        let reach: Vec<f32> = segs
            .iter()
            .zip(&at)
            .map(|(s, a)| a + s.sides.iter().map(|x| x.start + x.piece.dd).fold(net.d[s.node], f32::max))
            .collect();
        let cuts = |limit: f32| -> Vec<usize> {
            let mut out = vec![0];
            let mut base = 0usize;
            let mut far = reach.first().copied().unwrap_or(0.0);
            for (j, &r) in reach.iter().enumerate().skip(1) {
                far = far.max(r);
                if far - at[base] > limit {
                    out.push(j);
                    base = j;
                    far = r;
                }
            }
            out
        };
        // as many lines as the limit asks for, as even as they come: the least limit that makes
        // no more of them
        let mut starts = cuts(self.limit);
        if starts.len() > 1 {
            let lines = starts.len();
            let (mut lo, mut hi) = (0.0f32, self.limit);
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                if cuts(mid).len() <= lines {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            starts = cuts(hi);
        }
        let mut out = Piece::default();
        let mut used = Skyline::default();
        let mut first_axis = None;
        let mut segs: Vec<Option<Seg>> = segs.into_iter().map(Some).collect();
        for (li, &i) in starts.iter().enumerate() {
            let base = at[i];
            let j = starts.get(li + 1).copied().unwrap_or(segs.len());
            let mut pos = Skyline::default();
            let mut neg = Skyline::default();
            let mut line: Vec<(usize, f32, f32)> = Vec::new();
            let taken: Vec<Seg> = (i..j).filter_map(|k| segs[k].take()).collect();
            for (k, s) in taken.iter().enumerate() {
                let d0 = at[i + k] - base;
                let v = s.node;
                line.push((v, d0, -net.axis[v]));
                pos.add(d0, d0 + net.d[v], net.b[v] - net.axis[v]);
                neg.add(d0, d0 + net.d[v], net.axis[v]);
            }
            for (k, s) in taken.iter().enumerate() {
                let d0 = at[i + k] - base;
                for side in &s.sides {
                    let ds = d0 + side.start;
                    let pc = &side.piece;
                    // node by node against what that side holds: a thin stretch of a branch comes
                    // close to the spine where its wide end cannot
                    let half = gap.depth * 0.5;
                    let mut off = 0.0f32;
                    for &(v, nd, nb) in &pc.at {
                        let (a0, a1) = (ds + nd - half, ds + nd + net.d[v] + half);
                        let near = if side.neg { pc.bb - nb - net.b[v] } else { nb };
                        let sky = if side.neg { &neg } else { &pos };
                        off = off.max(sky.over(a0, a1) + gap.breadth - near);
                    }
                    let top = if side.neg { -(off + pc.bb) } else { off };
                    for &(v, nd, nb) in &pc.at {
                        let far = if side.neg { off + pc.bb - nb } else { off + nb + net.b[v] };
                        let sky = if side.neg { &mut neg } else { &mut pos };
                        sky.add(ds + nd, ds + nd + net.d[v], far);
                        line.push((v, ds + nd, top + nb));
                    }
                }
            }
            // the line comes as close to the lines before it as its nodes allow
            let half = gap.depth * 0.5;
            let shift = if used.0.is_empty() {
                line.iter().map(|&(_, _, b)| -b).fold(0.0f32, f32::max)
            } else {
                line.iter()
                    .map(|&(v, d, b)| used.over(d - half, d + net.d[v] + half) + gap.fold - b)
                    .fold(f32::MIN, f32::max)
            };
            for &(v, d, b) in &line {
                used.add(d, d + net.d[v], shift + b + net.b[v]);
            }
            if first_axis.is_none() {
                first_axis = Some(shift);
            }
            out.at.extend(line.into_iter().map(|(v, d, b)| (v, d, b + shift)));
        }
        out.axis = first_axis.unwrap_or(0.0);
        out.normalise(net)
    }

    /// The spine a node starts: the node with what hangs off it, then the spine of its deepest
    /// child that nothing else waits for.
    fn segs(&mut self, v: usize, memo: &mut [Option<Vec<Seg>>], pieces: &mut [Option<Piece>]) -> Vec<Seg> {
        let net = self.net;
        let gap = self.gap;
        let kids = net.kids[v].clone();
        // the siblings each child waits for
        let feeders: Vec<Vec<usize>> = kids
            .iter()
            .map(|&x| {
                let mut f: Vec<usize> =
                    net.preds[x].iter().filter(|&&p| p != v).map(|&p| net.branch(p, v)).collect();
                f.sort_unstable();
                f.dedup();
                f
            })
            .collect();
        let waited_for = |c: usize| feeders.iter().any(|f| f.contains(&c));
        let spine = kids
            .iter()
            .copied()
            .filter(|&c| !waited_for(c))
            .max_by(|&a, &b| self.len[a].total_cmp(&self.len[b]).then(b.cmp(&a)));
        let spine_key = spine.and_then(|c| kids.iter().position(|&x| x == c));
        let mut starts: Vec<f32> = vec![0.0; kids.len()];
        let mut sides: Vec<Side> = Vec::new();
        let mut groups: Vec<(bool, f32, Vec<Piece>)> = Vec::new();
        for (k, &x) in kids.iter().enumerate() {
            let start = if feeders[k].is_empty() {
                net.d[v] + gap.depth
            } else {
                feeders[k]
                    .iter()
                    .filter_map(|f| kids.iter().position(|y| y == f))
                    .map(|fi| starts[fi] + pieces[kids[fi]].as_ref().map_or(0.0, |p| p.dd) + gap.depth)
                    .fold(net.d[v] + gap.depth, f32::max)
            };
            starts[k] = start;
            if Some(x) == spine {
                continue;
            }
            let piece = {
                let list = memo[x].take().unwrap_or_default();
                self.lines(list)
            };
            pieces[x] = Some(piece.clone());
            // the branches before the spine's pin hang on one side, the rest on the other
            let neg = match spine_key {
                Some(sk) => k < sk,
                None => k * 2 < kids.len(),
            };
            match groups.iter_mut().find(|g| g.0 == neg && (g.1 - start).abs() < 0.5) {
                Some(g) => g.2.push(piece),
                None => groups.push((neg, start, vec![piece])),
            }
        }
        for (neg, start, items) in groups {
            if items.len() >= WRAP_MIN {
                let k = wrap(self.flow, &items, &gap, self.limit);
                sides.push(Side { piece: grid(net, items, k, gap.breadth, &gap), start, neg });
            } else {
                // nearest the spine first
                let mut items = items;
                if neg {
                    items.reverse();
                }
                for p in items {
                    sides.push(Side { piece: p, start, neg });
                }
            }
        }
        let mut list = Vec::new();
        match spine {
            Some(c) => {
                let k = kids.iter().position(|&x| x == c).unwrap_or(0);
                list.push(Seg { node: v, sides, next: starts[k] });
                self.len[v] = starts[k] + self.len[c];
                list.extend(memo[c].take().unwrap_or_default());
            }
            None => {
                self.len[v] = net.d[v];
                list.push(Seg { node: v, sides, next: 0.0 });
            }
        }
        list
    }

    fn all(&mut self) -> Piece {
        let net = self.net;
        let mut memo: Vec<Option<Vec<Seg>>> = (0..net.n).map(|_| None).collect();
        let mut pieces: Vec<Option<Piece>> = vec![None; net.n];
        for &v in net.topo.iter().rev() {
            let s = self.segs(v, &mut memo, &mut pieces);
            memo[v] = Some(s);
        }
        // the starts of the graph, side by side
        let roots = net.kids[net.n].clone();
        let first: Vec<Piece> = roots
            .iter()
            .filter(|&&r| net.band[r] == 0 && !net.loose.iter().any(|(l, _)| *l == r))
            .map(|&r| {
                let list = memo[r].take().unwrap_or_default();
                self.lines(list)
            })
            .collect();
        let mut out = Piece::default();
        let mut depth = 0.0f32;
        if !first.is_empty() {
            let k = wrap(self.flow, &first, &self.gap, self.limit);
            let block = grid(net, first, k, self.gap.tree, &self.gap);
            block.shifted(0.0, 0.0, &mut out.at);
            depth = block.dd + self.gap.depth;
        }
        // what several of them lead to (a quest's end that two trees reach): right after the trees
        // that lead into it, across from the middle of their wires; below everything only where
        // that place is taken
        let mut at: Vec<Option<(f32, f32)>> = vec![None; net.n];
        for &(v, d, b) in &out.at {
            at[v] = Some((d, b));
        }
        let (hd, hb) = (self.gap.depth * 0.5, self.gap.breadth * 0.5);
        let free = |out: &Piece, piece: &Piece, dd: f32, db: f32| {
            piece.at.iter().all(|&(v, d, b)| {
                let (d0, b0) = (d + dd, b + db);
                out.at.iter().all(|&(w, e, c)| {
                    d0 + net.d[v] + hd <= e
                        || e + net.d[w] + hd <= d0
                        || b0 + net.b[v] + hb <= c
                        || c + net.b[w] + hb <= b0
                })
            })
        };
        for &r in roots.iter().filter(|&&r| net.band[r] > 0) {
            let list = memo[r].take().unwrap_or_default();
            let piece = self.lines(list);
            let feeders: Vec<(f32, f32)> = net.preds[r]
                .iter()
                .filter_map(|&f| at[f].map(|(d, b)| (d + net.d[f], b + net.axis[f])))
                .collect();
            let (mut dd, mut db) = (depth, 0.0);
            if !feeders.is_empty() {
                let after = feeders.iter().map(|f| f.0).fold(f32::MIN, f32::max) + self.gap.depth;
                let across = feeders.iter().map(|f| f.1).sum::<f32>() / feeders.len() as f32 - piece.axis;
                db = across;
                dd = if free(&out, &piece, after, across) { after } else { depth.max(after) };
            }
            piece.shifted(dd, db, &mut out.at);
            for &(v, d, b) in &piece.at {
                at[v] = Some((d + dd, b + db));
            }
            depth = depth.max(dd + piece.dd + self.gap.depth);
        }
        // a loose node: just before the first node it leads to, beside what leads in there
        for (u, targets) in &net.loose {
            let Some(list) = memo[*u].take() else { continue };
            let piece = self.lines(list);
            let Some((v, (td, tb))) = targets.iter().find_map(|&v| at[v].map(|p| (v, p))) else {
                piece.shifted(depth, 0.0, &mut out.at);
                depth += piece.dd + self.gap.depth;
                continue;
            };
            let dd = td - self.gap.depth - piece.dd;
            let centre = tb + net.axis[v] - piece.axis;
            let step = piece.bb + self.gap.breadth;
            let db = (1..=64)
                .flat_map(|k| [k as f32, -(k as f32)])
                .map(|k| centre + k * step)
                .find(|&db| free(&out, &piece, dd, db));
            let (dd, db) = db.map_or((depth, centre), |db| (dd, db));
            piece.shifted(dd, db, &mut out.at);
            for &(w, d, b) in &piece.at {
                at[w] = Some((d + dd, b + db));
            }
            depth = depth.max(dd + piece.dd + self.gap.depth);
        }
        out.normalise(net)
    }
}

/// How large a view of the reference size shows a layout of this depth and breadth, at most 1:1.
fn fit_size(flow: Flow, d: f32, b: f32) -> f32 {
    let (w, h) = match flow {
        Flow::Right => (d, b),
        Flow::Down => (b, d),
    };
    (VIEW[0] / w.max(1.0)).min(VIEW[1] / h.max(1.0)).min(1.0)
}

fn fit(flow: Flow, p: &Piece) -> f32 {
    fit_size(flow, p.dd, p.bb)
}

/// The top-left corner of every node of `set` (indices of `g.nodes`), laid out in `g`'s flow, in
/// the order of `set`, with the first node near the origin.
#[must_use]
pub fn layout(catalog: &Catalog, g: &Graph, set: &[usize]) -> Vec<[f32; 2]> {
    let cx = BoxCx::of(catalog, g);
    let net = build(&cx, g, set);
    let gap = gaps(cx.flow);
    let mut run = Run { net: &net, flow: cx.flow, gap, limit: f32::MAX, len: vec![0.0; net.n] };
    let mut best = run.all();
    let mut score = fit(cx.flow, &best);
    let longest = best.dd;
    let floor = net.d.iter().copied().fold(0.0f32, f32::max);
    // a graph too long to be read whole folds into lines, at the depth that frames it largest;
    // a fold turns wires back across the graph, so it has to buy a lot: a graph that a wide canvas
    // shows at almost half its size stays one line
    if score < 0.45 {
        for k in 2..=12 {
            run.limit = (longest / k as f32).max(floor);
            let p = run.all();
            let s = fit(cx.flow, &p) * (1.0 - 0.02 * (k - 1) as f32);
            if s > score {
                best = p;
                score = s;
            }
        }
    }
    let mut out = vec![[0.0, 0.0]; set.len()];
    for &(v, d, b) in &best.at {
        let p = match cx.flow {
            Flow::Right => [d, b],
            Flow::Down => [b, d],
        };
        if let Some(o) = out.get_mut(v) {
            *o = [layout::sane(snap(p[0])), layout::sane(snap(p[1]))];
        }
    }
    out
}

/// What `graph.arrange` lays out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    /// The selected nodes alone, where they were.
    Selection,
    /// The selected node and what only it leads to, from where it stands.
    Branch,
}

/// The nodes a node's branch holds: itself and every node it dominates along the forward wires.
#[must_use]
pub fn branch(catalog: &Catalog, g: &Graph, root: usize) -> Vec<usize> {
    let cx = BoxCx::of(catalog, g);
    let all: Vec<usize> = (0..g.nodes.len()).collect();
    let net = build(&cx, g, &all);
    let mut out = vec![root];
    let mut i = 0;
    while let Some(&v) = out.get(i) {
        out.extend(net.kids[v].iter().copied());
        i += 1;
    }
    out.sort_unstable();
    out
}

/// New places for the nodes of a layout of `set` alone, kept where `set` was: the box of the set
/// keeps its top-left corner, or `anchor` (an index of `g.nodes`) keeps its place.
#[must_use]
pub fn layout_part(
    catalog: &Catalog,
    g: &Graph,
    set: &[usize],
    anchor: Option<usize>,
) -> Vec<(usize, [f32; 2])> {
    if set.is_empty() {
        return Vec::new();
    }
    let placed = layout(catalog, g, set);
    let shift = match anchor.and_then(|a| set.iter().position(|&x| x == a)) {
        Some(at) => {
            let was = g.nodes.get(set[at]).map_or([0.0, 0.0], |n| n.pos);
            [was[0] - placed[at][0], was[1] - placed[at][1]]
        }
        None => {
            let mut was = [f32::MAX, f32::MAX];
            let mut now = [f32::MAX, f32::MAX];
            for (k, &i) in set.iter().enumerate() {
                if let Some(n) = g.nodes.get(i) {
                    was = [was[0].min(n.pos[0]), was[1].min(n.pos[1])];
                }
                now = [now[0].min(placed[k][0]), now[1].min(placed[k][1])];
            }
            [was[0] - now[0], was[1] - now[1]]
        }
    };
    set.iter().zip(placed).map(|(&i, p)| (i, [snap(p[0] + shift[0]), snap(p[1] + shift[1])])).collect()
}

/// An edge or the middle of a selection to line nodes up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
    Top,
    Middle,
    Bottom,
}

impl Align {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "left" => Self::Left,
            "centre" | "center" => Self::Centre,
            "right" => Self::Right,
            "top" => Self::Top,
            "middle" => Self::Middle,
            "bottom" => Self::Bottom,
            _ => return None,
        })
    }
}

fn boxes(catalog: &Catalog, g: &Graph, set: &[usize]) -> Vec<(usize, [f32; 2], [f32; 2])> {
    let cx = BoxCx::of(catalog, g);
    set.iter().filter_map(|&i| g.nodes.get(i).map(|n| (i, n.pos, layout::node_size(&cx, n)))).collect()
}

/// The selection lined up on its outermost edge (or the middle of its extent).
#[must_use]
pub fn align(catalog: &Catalog, g: &Graph, set: &[usize], how: Align) -> Vec<(usize, [f32; 2])> {
    let bx = boxes(catalog, g, set);
    if bx.len() < 2 {
        return Vec::new();
    }
    let min_x = bx.iter().map(|b| b.1[0]).fold(f32::MAX, f32::min);
    let max_x = bx.iter().map(|b| b.1[0] + b.2[0]).fold(f32::MIN, f32::max);
    let min_y = bx.iter().map(|b| b.1[1]).fold(f32::MAX, f32::min);
    let max_y = bx.iter().map(|b| b.1[1] + b.2[1]).fold(f32::MIN, f32::max);
    bx.iter()
        .map(|&(i, p, s)| {
            let q = match how {
                Align::Left => [min_x, p[1]],
                Align::Right => [max_x - s[0], p[1]],
                Align::Centre => [(min_x + max_x - s[0]) * 0.5, p[1]],
                Align::Top => [p[0], min_y],
                Align::Bottom => [p[0], max_y - s[1]],
                Align::Middle => [p[0], (min_y + max_y - s[1]) * 0.5],
            };
            (i, [snap(q[0]), snap(q[1])])
        })
        .collect()
}

/// The selection spread so the gaps between its boxes are equal, the outermost two kept.
#[must_use]
pub fn distribute(catalog: &Catalog, g: &Graph, set: &[usize], horizontal: bool) -> Vec<(usize, [f32; 2])> {
    let mut bx = boxes(catalog, g, set);
    if bx.len() < 3 {
        return Vec::new();
    }
    let a = usize::from(!horizontal);
    bx.sort_by(|x, y| x.1[a].total_cmp(&y.1[a]));
    let (Some(first), Some(last)) = (bx.first(), bx.last()) else { return Vec::new() };
    let span = last.1[a] + last.2[a] - first.1[a];
    let sizes: f32 = bx.iter().map(|b| b.2[a]).sum();
    let gap = (span - sizes) / (bx.len() - 1) as f32;
    let mut at = first.1[a];
    let mut out = Vec::with_capacity(bx.len());
    for &(i, p, s) in &bx {
        let mut q = p;
        q[a] = at;
        out.push((i, [snap(q[0]), snap(q[1])]));
        at += s[a] + gap;
    }
    out
}

/// The successors of the selection moved across the flow so their wires run straight: each
/// target's input on the line of the pin that leads to it.
#[must_use]
pub fn straighten(catalog: &Catalog, g: &Graph, set: &[usize]) -> Vec<(usize, [f32; 2])> {
    let cx = BoxCx::of(catalog, g);
    let index = g.index();
    let mut out: Vec<(usize, [f32; 2])> = Vec::new();
    for &i in set {
        let Some(n) = g.nodes.get(i) else { continue };
        let size = layout::node_size(&cx, n);
        let (pins, _) = layout::pins(catalog, n);
        for (k, pin) in pins.iter().enumerate() {
            let Some(p) = n.out.iter().find(|p| p.name.as_str() == pin) else { continue };
            // a pin that fans out has no one line to straighten on
            let [t] = p.targets.as_slice() else { continue };
            let Some(&ti) = index.get(t.as_str()) else { continue };
            if set.contains(&ti) || out.iter().any(|(x, _)| *x == ti) {
                continue;
            }
            let Some(tn) = g.nodes.get(ti) else { continue };
            let tsize = layout::node_size(&cx, tn);
            let from = layout::output_pin(cx.flow, n.pos, size, k, pins.len());
            let into = layout::input_pin(cx.flow, tn.pos, tsize);
            let q = match cx.flow {
                Flow::Right => [tn.pos[0], tn.pos[1] + from[1] - into[1]],
                Flow::Down => [tn.pos[0] + from[0] - into[0], tn.pos[1]],
            };
            out.push((ti, [layout::sane(q[0]), layout::sane(q[1])]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::Node;

    fn bundled() -> Catalog {
        let bytes = include_bytes!("../../../gamedata/configs/nq/catalog.ltx");
        let text = gz_base::text::cp1251_to_utf8(bytes).into_owned();
        Catalog::assemble(&[crate::catalog::SourceFile { label: "bundled".into(), text }], false, None, 1)
    }

    fn big(flow: Flow) -> (Catalog, Graph) {
        let cat = bundled();
        let text = include_str!("fixtures/big_quest.nqasset");
        let mut g = match crate::asset::parse(&cat, text, "big_quest") {
            Ok(g) => g,
            Err(e) => panic!("{}", e.message),
        };
        g.canvas.flow = Some(flow);
        (cat, g)
    }

    fn rects(cat: &Catalog, g: &Graph) -> Vec<[f32; 4]> {
        let cx = BoxCx::of(cat, g);
        g.nodes
            .iter()
            .map(|n| {
                let s = layout::node_size(&cx, n);
                [n.pos[0], n.pos[1], n.pos[0] + s[0], n.pos[1] + s[1]]
            })
            .collect()
    }

    fn overlaps(r: &[[f32; 4]]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in 0..r.len() {
            for j in i + 1..r.len() {
                let (a, b) = (r[i], r[j]);
                if a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3] {
                    out.push((i, j));
                }
            }
        }
        out
    }

    #[test]
    fn the_big_quest_fits_without_an_overlap_in_both_flows() {
        let mut sizes = Vec::new();
        for flow in [Flow::Right, Flow::Down] {
            let (cat, mut g) = big(flow);
            assert_eq!(g.nodes.len(), 110);
            layout::run(&cat, &mut g, false);
            let r = rects(&cat, &g);
            let (x0, y0, x1, y1) = r.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |a, b| {
                (a.0.min(b[0]), a.1.min(b[1]), a.2.max(b[2]), a.3.max(b[3]))
            });
            let (w, h) = (x1 - x0, y1 - y0);
            eprintln!("{flow:?}: {w} x {h}");
            if let Some(dir) = std::env::var_os("GZ_LAYOUT_DUMP") {
                let list: Vec<serde_json::Value> = g
                    .nodes
                    .iter()
                    .zip(&r)
                    .map(|(n, b)| serde_json::json!({ "id": n.id, "r": b, "out": n.out.iter().flat_map(|p| p.targets.iter().map(|t| t.to_string())).collect::<Vec<_>>() }))
                    .collect();
                let path = std::path::Path::new(&dir).join(format!("layout_{flow:?}.json"));
                let _ = std::fs::write(path, serde_json::Value::Array(list).to_string());
            }
            let o = overlaps(&r);
            sizes.push((
                flow,
                w,
                h,
                o.len(),
                o.first().map(|&(a, b)| (g.nodes[a].id.clone(), g.nodes[b].id.clone())),
            ));
        }
        for (flow, w, h, o, first) in sizes {
            // flowing right (the default) it fits in 4000 pt; flowing down its nodes are wide
            let most = if flow == Flow::Right { 4000.0 } else { 4800.0 };
            assert!(w.max(h) < most, "{flow:?}: {w} x {h} pt");
            assert!(o == 0, "{flow:?}: {o} overlaps, first {first:?}");
        }
    }

    #[test]
    fn a_wide_fan_out_wraps() {
        let cat = bundled();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        for i in 0..24 {
            s.connect("next", &format!("leaf{i}"));
            g.nodes.push_back(Node::new(&format!("leaf{i}"), "flow.end"));
        }
        g.nodes.push_front(s);
        for flow in [Flow::Right, Flow::Down] {
            g.canvas.flow = Some(flow);
            layout::run(&cat, &mut g, false);
            let r = rects(&cat, &g);
            let (x0, y0, x1, y1) = r.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |a, b| {
                (a.0.min(b[0]), a.1.min(b[1]), a.2.max(b[2]), a.3.max(b[3]))
            });
            // one ribbon of 24 would be several thousand pt long
            assert!((x1 - x0).max(y1 - y0) < 2400.0, "{flow:?}: {} x {}", x1 - x0, y1 - y0);
            assert!(overlaps(&r).is_empty());
            // every leaf lies after the start in the flow
            let start = g.nodes[0].pos;
            assert!(g.nodes.iter().skip(1).all(|n| match flow {
                Flow::Right => n.pos[0] > start[0],
                Flow::Down => n.pos[1] > start[1],
            }));
        }
    }

    #[test]
    fn a_layout_is_deterministic_and_flows_its_way() {
        let cat = bundled();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "b");
        g.nodes.push_back(s);
        g.nodes.push_back(a);
        g.nodes.push_back(Node::new("b", "flow.end"));
        g.canvas.flow = Some(Flow::Right);
        assert_eq!(layout::run(&cat, &mut g, false), 3);
        let xs: Vec<f32> = g.nodes.iter().map(|n| n.pos[0]).collect();
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "{xs:?}");
        let ys: Vec<f32> = g.nodes.iter().map(|n| n.pos[1]).collect();
        assert!(ys.iter().all(|y| *y == ys[0]), "a chain runs straight: {ys:?}");
        assert_eq!(layout::run(&cat, &mut g, false), 0);
        g.canvas.flow = Some(Flow::Down);
        layout::run(&cat, &mut g, false);
        let ys: Vec<f32> = g.nodes.iter().map(|n| n.pos[1]).collect();
        assert!(ys[0] < ys[1] && ys[1] < ys[2], "{ys:?}");
    }

    #[test]
    fn a_merge_sits_after_its_branches() {
        let cat = bundled();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        s.connect("next", "b");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "m");
        let mut b = Node::new("b", "flow.step");
        b.connect("next", "c");
        let mut c = Node::new("c", "flow.step");
        c.connect("next", "m");
        for n in [s, a, b, c, Node::new("m", "flow.join")] {
            g.nodes.push_back(n);
        }
        g.canvas.flow = Some(Flow::Right);
        layout::run(&cat, &mut g, false);
        let x = |id: &str| g.find_node(id).map_or(0.0, |n| n.pos[0]);
        assert!(x("m") > x("c") && x("c") > x("b"), "{} {} {}", x("m"), x("c"), x("b"));
    }

    /// A quest whose end a trigger's tree reaches, and a node whose input is not wired yet that
    /// leads there too: the loose node is no start of a tree of its own and shapes nothing - the
    /// end stays under what leads into it, the loose node goes just before the end, beside it.
    #[test]
    fn a_node_nothing_leads_into_waits_beside_its_target() {
        let cat = bundled();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "b");
        let mut b = Node::new("b", "flow.step");
        b.connect("next", "end");
        let mut loose = Node::new("loose", "flow.step");
        loose.connect("next", "end");
        for n in [s, a, b, loose, Node::new("end", "flow.end")] {
            g.nodes.push_back(n);
        }
        let cx = BoxCx::of(&cat, &g);
        for flow in [Flow::Down, Flow::Right] {
            g.canvas.flow = Some(flow);
            layout::run(&cat, &mut g, false);
            assert!(overlaps(&rects(&cat, &g)).is_empty(), "{flow:?}");
            let at =
                |id: &str| g.find_node(id).map(|n| (n.pos, layout::node_size(&cx, n))).unwrap_or_default();
            // depth and breadth of a node's box: (start, end) along the flow, (start, end) across it
            let axes = |id: &str| {
                let (p, s) = at(id);
                match flow {
                    Flow::Down => ((p[1], p[1] + s[1]), (p[0], p[0] + s[0])),
                    Flow::Right => ((p[0], p[0] + s[0]), (p[1], p[1] + s[1])),
                }
            };
            let (b_d, b_b) = axes("b");
            let (e_d, e_b) = axes("end");
            let (l_d, _) = axes("loose");
            // the chain runs straight: the end under its step, not after a row of its own
            assert!(e_d.0 > b_d.1 && e_b.0 < b_b.1 && b_b.0 < e_b.1, "{flow:?}: the end is under b");
            // the loose node ends before the end starts, in the layer of b
            assert!(l_d.1 <= e_d.0 && l_d.0 >= b_d.0 - 1.0, "{flow:?}: {l_d:?} against {e_d:?} and {b_d:?}");
        }
    }

    /// What two trees lead into stays between them, right after the deeper of the two - not at the
    /// start of a row of its own below everything.
    #[test]
    fn a_merge_of_two_trees_sits_between_them() {
        let cat = bundled();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "end");
        let mut w = Node::new("w", "trigger.start");
        w.connect("next", "c");
        let mut c = Node::new("c", "flow.step");
        c.connect("next", "end");
        for n in [s, a, w, c, Node::new("end", "flow.end")] {
            g.nodes.push_back(n);
        }
        g.canvas.flow = Some(Flow::Down);
        layout::run(&cat, &mut g, false);
        assert!(overlaps(&rects(&cat, &g)).is_empty());
        let cx = BoxCx::of(&cat, &g);
        let mid = |id: &str| g.find_node(id).map_or(0.0, |n| n.pos[0] + layout::node_size(&cx, n)[0] * 0.5);
        let (xa, xc, xe) = (mid("a"), mid("c"), mid("end"));
        assert!(xa.min(xc) < xe && xe < xa.max(xc), "{xa} {xe} {xc}");
    }
}

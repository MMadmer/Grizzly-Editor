//! The one builder of a wire's polyline: from its two pins, the flow, the style and its waypoints.
//! Drawing, hit testing, the minimap and `graph.get` all read the polyline it makes, so a wire is
//! hit where it is drawn and reported where it is seen. Units are world pt.
//!
//! The geometry is worked out in the flow's own axes - *depth* along the flow, *breadth* across
//! it - and turned back into x and y at the end, so every style reads the same flowing right and
//! flowing down. A wire whose target is not ahead of its pin goes back: it is routed around the
//! nodes through a free channel (`route_back`) in every style.

use crate::canvas::{Diagonal, Flow, WireStyle};

/// The straight run out of a pin and into an input.
pub const STUB: f32 = 16.0;
/// Between two lanes of a node's wires.
pub const LANE: f32 = 8.0;
/// Between the tracks of the wires of one pin (subway).
pub const TRACK: f32 = 4.0;
/// An edge that goes back: the run out of the pin and into the input.
pub const BACK_STUB: f32 = 24.0;
/// How far a back edge's channel keeps from a node.
const BACK_CLEAR: f32 = 24.0;
/// Between two back edges that share no target.
const BACK_LANE: f32 = 12.0;
const BACK_CORNER: f32 = 12.0;
/// The farthest a back edge goes round the side of everything between its ends: past that it
/// keeps between them, behind the fewest nodes (the nodes are drawn over it).
const MAX_DETOUR: f32 = 240.0;

/// A point in the flow's axes: `[breadth, depth]`.
type L = [f32; 2];

fn local(flow: Flow, p: [f32; 2]) -> L {
    match flow {
        Flow::Down => p,
        Flow::Right => [p[1], p[0]],
    }
}

fn world(flow: Flow, l: L) -> [f32; 2] {
    match flow {
        Flow::Down => l,
        Flow::Right => [l[1], l[0]],
    }
}

fn dist(a: L, b: L) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Whether a wire between these pins has to go back: its input is not at least a stub ahead of its
/// pin. A target close ahead (two neighbouring columns) still takes a forward wire.
#[must_use]
pub fn goes_back(flow: Flow, p0: [f32; 2], p3: [f32; 2]) -> bool {
    let (a, b) = (local(flow, p0), local(flow, p3));
    b[1] - a[1] < STUB
}

/// Corners cut round (`round`) or at 45 degrees (`chamfer`) with radius `r`, or left sharp.
#[derive(Clone, Copy, PartialEq)]
enum Corner {
    Sharp,
    Round(f32),
    Chamfer(f32),
}

/// A polyline through `corners` with its corners shaped.
fn shape(corners: &[L], corner: Corner) -> Vec<L> {
    let mut out: Vec<L> = Vec::with_capacity(corners.len() * 5);
    let Some(&first) = corners.first() else { return out };
    out.push(first);
    for k in 1..corners.len().saturating_sub(1) {
        let (from, c, to) = (corners[k - 1], corners[k], corners[k + 1]);
        let (inn, next) = (dist(from, c), dist(c, to));
        let r = match corner {
            Corner::Sharp => 0.0,
            Corner::Round(r) | Corner::Chamfer(r) => r.min(inn * 0.5).min(next * 0.5),
        };
        if r < 0.5 {
            if out.last().is_none_or(|l| dist(*l, c) > 0.01) {
                out.push(c);
            }
            continue;
        }
        let u = [c[0] + (from[0] - c[0]) / inn * r, c[1] + (from[1] - c[1]) / inn * r];
        let v = [c[0] + (to[0] - c[0]) / next * r, c[1] + (to[1] - c[1]) / next * r];
        if matches!(corner, Corner::Chamfer(_)) {
            out.push(u);
            out.push(v);
            continue;
        }
        for step in 0..=4 {
            let f = step as f32 / 4.0;
            let e = 1.0 - f;
            out.push([
                e * e * u[0] + 2.0 * e * f * c[0] + f * f * v[0],
                e * e * u[1] + 2.0 * e * f * c[1] + f * f * v[1],
            ]);
        }
    }
    if let Some(&last) = corners.last()
        && out.last().is_none_or(|l| dist(*l, last) > 0.01)
    {
        out.push(last);
    }
    out
}

fn bezier(a: L, b: L, steps: usize) -> Vec<L> {
    let dd = b[1] - a[1];
    let db = (b[0] - a[0]).abs();
    let t = (0.5 * dd.abs() + 0.25 * db).clamp(32.0, 400.0);
    let (c1, c2) = ([a[0], a[1] + t], [b[0], b[1] - t]);
    (0..=steps)
        .map(|k| {
            let s = k as f32 / steps as f32;
            let u = 1.0 - s;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s);
            [w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0], w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1]]
        })
        .collect()
}

/// Which of a node's wires this is, so a node's wires keep lanes of their own: its place among
/// all the node's wires and among the wires of its own pin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lanes {
    pub index: usize,
    pub count: usize,
    pub track: usize,
    pub tracks: usize,
}

/// One forward stretch from `a` (leaving along the flow) to `b` (arriving along it).
fn stretch(style: WireStyle, diag: Diagonal, a: L, b: L, lanes: Lanes) -> Vec<L> {
    let dd = b[1] - a[1];
    let db = b[0] - a[0];
    match style {
        WireStyle::Curved => {
            let steps = ((dist(a, b) / 14.0) as usize).clamp(10, 48);
            bezier(a, b, steps)
        }
        WireStyle::Straight => {
            let s = STUB.min(dd.abs() * 0.5);
            shape(&[a, [a[0], a[1] + s], [b[0], b[1] - s], b], Corner::Sharp)
        }
        WireStyle::Circuit => circuit(a, b, lanes, Corner::Round(8.0)),
        WireStyle::Angled => {
            let room = dd - 2.0 * STUB;
            if room < db.abs() {
                return circuit(a, b, lanes, Corner::Chamfer(6.0));
            }
            let start = match diag {
                Diagonal::Start => a[1] + STUB,
                Diagonal::Middle => (a[1] + b[1]) * 0.5 - db.abs() * 0.5,
                Diagonal::End => b[1] - STUB - db.abs(),
            };
            shape(&[a, [a[0], start], [b[0], start + db.abs()], b], Corner::Round(4.0))
        }
        WireStyle::Subway => {
            // the wires of one pin leave on tracks of their own, then share the trunk to their turn
            let o = (lanes.track as f32 - (lanes.tracks.max(1) - 1) as f32 * 0.5) * TRACK;
            let lead = a[1] + STUB * 0.5;
            let b0 = a[0] + o;
            let off = (b[0] - b0).abs();
            let turn = b[1] - STUB - off;
            if turn < lead + o.abs() {
                return circuit(a, b, lanes, Corner::Chamfer(6.0));
            }
            shape(
                &[a, [a[0], lead], [b0, lead + o.abs()], [b0, turn], [b[0], turn + off], b],
                Corner::Round(6.0),
            )
        }
    }
}

/// Right angles: out along the flow to the wire's lane, across, and on into the input.
fn circuit(a: L, b: L, lanes: Lanes, corner: Corner) -> Vec<L> {
    let lo = a[1] + STUB;
    let hi = b[1] - STUB;
    let mid = (a[1] + b[1]) * 0.5;
    let n = lanes.count.max(1);
    let room = (hi - lo).max(0.0);
    let pitch = if n > 1 { LANE.min(room / (n - 1) as f32) } else { 0.0 };
    let at =
        if hi > lo { (mid + (lanes.index as f32 - (n - 1) as f32 * 0.5) * pitch).clamp(lo, hi) } else { mid };
    shape(&[a, [a[0], at], [b[0], at], b], corner)
}

/// The vertical run of a routed edge, in the flow's axes: the next one keeps clear of it.
#[derive(Clone, Copy, Debug)]
pub struct Lane {
    x: f32,
    top: f32,
    bottom: f32,
    from: L,
    to: L,
    side_from: f32,
    side_to: f32,
}

/// The breadth of a free channel: outside every span, as short a detour for a run from `x0` to
/// `x3` as there is; between the two every place costs the same, so the middle of what is free
/// there. A tie goes to `x0`'s side.
#[must_use]
pub fn pick_channel(spans: &mut [(f32, f32)], x0: f32, x3: f32) -> f32 {
    spans.sort_by(|l, r| l.0.total_cmp(&r.0));
    let (lo, hi) = (x0.min(x3), x0.max(x3));
    let mut best = x0;
    let mut best_cost = f32::MAX;
    let mut consider = |from: f32, to: f32| {
        if from > to {
            return;
        }
        let x = if to < lo {
            to
        } else if from > hi {
            from
        } else {
            (from.max(lo) + to.min(hi)) * 0.5
        };
        let cost = (x - x0).abs() * 1.001 + (x - x3).abs();
        if cost < best_cost {
            best_cost = cost;
            best = x;
        }
    };
    let mut free_from = -f32::MAX;
    for s in spans.iter() {
        consider(free_from, s.0);
        free_from = free_from.max(s.1);
    }
    consider(free_from, f32::MAX);
    best
}

/// A channel between `lo` and `hi` behind as few spans as there are, the middle of the least
/// crossed stretch nearest the middle.
fn least_crossed(spans: &[(f32, f32)], lo: f32, hi: f32) -> f32 {
    let steps = (((hi - lo) / 8.0).ceil() as usize).clamp(1, 400);
    let mid = (lo + hi) * 0.5;
    let mut best = (usize::MAX, f32::MAX, mid);
    for i in 0..=steps {
        let x = lo + (hi - lo) * i as f32 / steps as f32;
        let n = spans.iter().filter(|s| x > s.0 && x < s.1).count();
        let d = (x - mid).abs();
        if n < best.0 || (n == best.0 && d < best.1) {
            best = (n, d, x);
        }
    }
    best.2
}

/// The route of an edge back from pin `p0` to input `p3`, in the flow's axes: out of the pin,
/// aside to a free channel, back along it, over the target and into its input; the channel keeps
/// clear of every box (`[b0, d0, b1, d1]`) between the two depths and of the lanes routed before -
/// except those into the same input, which share it.
fn route_back(boxes: &[[f32; 4]], p0: L, p3: L, lanes: &mut Vec<Lane>, corner: Corner) -> Vec<L> {
    let mut below = p0[1] + BACK_STUB;
    let mut above = p3[1] - BACK_STUB;
    let mut x = p0[0];
    let (mut drop, mut rise) = (BACK_STUB, BACK_STUB);
    let (mut side0, mut side3) = (1.0f32, 1.0f32);
    let mut spans: Vec<(f32, f32)> = Vec::new();
    #[allow(clippy::float_cmp)]
    let same = |a: L, b: L| a == b;
    // a wire into an input that has a channel takes that channel
    if let Some(l) = lanes.iter().find(|l| same(l.to, p3)) {
        x = l.x;
        side3 = l.side_to;
        side0 = if x < p0[0] { -1.0 } else { 1.0 };
        above = above.min(l.top);
        below = below.max(p0[1] + BACK_STUB);
    } else {
        for _ in 0..4 {
            spans.clear();
            for g in boxes {
                if g[1] - BACK_CLEAR < below && g[3] + BACK_CLEAR > above {
                    spans.push((g[0] - BACK_CLEAR, g[2] + BACK_CLEAR));
                }
            }
            for lane in lanes.iter() {
                if lane.top < below && lane.bottom > above && !same(lane.to, p3) {
                    spans.push((lane.x - BACK_LANE, lane.x + BACK_LANE));
                }
            }
            x = pick_channel(&mut spans, p0[0], p3[0]);
            // a channel round the side of a whole graph (a folded line's wire back to the next
            // line's start) is a frame around everything: it keeps between its ends instead
            let (lo, hi) = (p0[0].min(p3[0]), p0[0].max(p3[0]));
            let detour = if x < lo { lo - x } else { (x - hi).max(0.0) };
            if detour > MAX_DETOUR && hi - lo > BACK_STUB * 4.0 {
                x = least_crossed(&spans, lo + BACK_STUB, hi - BACK_STUB);
            }
            let mut moved = false;
            side0 = if x < p0[0] { -1.0 } else { 1.0 };
            side3 = if x < p3[0] { -1.0 } else { 1.0 };
            let out_before = lanes.iter().filter(|l| same(l.from, p0) && l.side_from == side0).count();
            drop = BACK_STUB + BACK_LANE * out_before.min(6) as f32;
            rise = BACK_STUB;
            if below < p0[1] + drop {
                below = p0[1] + drop;
                moved = true;
            }
            if above > p3[1] - rise {
                above = p3[1] - rise;
                moved = true;
            }
            let crosses = |g: &[f32; 4], y: f32, x0: f32, x1: f32| {
                let margin = BACK_STUB * 0.5;
                y > g[1] - margin && y < g[3] + margin && g[0] < x0.max(x1) && g[2] > x0.min(x1)
            };
            for g in boxes {
                if crosses(g, below, p0[0], x) {
                    below = below.max(g[3] + BACK_STUB);
                    moved = true;
                }
                if crosses(g, above, x, p3[0]) {
                    above = above.min(g[1] - BACK_STUB);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
    lanes.push(Lane { x, top: above, bottom: below, from: p0, to: p3, side_from: side0, side_to: side3 });
    let xs = p0[0] + side0 * BACK_STUB;
    let xe = p3[0] + side3 * BACK_STUB;
    let mut corners: Vec<L> = Vec::with_capacity(8);
    corners.push(p0);
    corners.push([xs, p0[1] + drop]);
    if below > p0[1] + drop + 0.5 {
        corners.push([xs, below]);
    }
    corners.push([x, below]);
    corners.push([x, above]);
    corners.push([xe, above]);
    if above < p3[1] - rise - 0.5 {
        corners.push([xe, p3[1] - rise]);
    }
    corners.push(p3);
    shape(&corners, if corner == Corner::Sharp { Corner::Sharp } else { Corner::Round(BACK_CORNER) })
}

/// What a wire is built from.
#[derive(Clone, Copy, Debug)]
pub struct WireCx {
    pub flow: Flow,
    pub style: WireStyle,
    pub diagonal: Diagonal,
}

/// A built wire: its polyline in world pt, and whether it goes back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wire {
    pub points: Vec<[f32; 2]>,
    pub back: bool,
}

/// The polyline of a wire from output pin `p0` to input `p3` through `via` (its waypoints).
/// `boxes` are the node rectangles (world `[x0, y0, x1, y1]`) a wire that goes back keeps clear
/// of; `lanes` collects the channels of the wires that went back so far.
#[must_use]
pub fn build(
    cx: &WireCx,
    p0: [f32; 2],
    p3: [f32; 2],
    via: &[[f32; 2]],
    lanes_of: Lanes,
    boxes: &[[f32; 4]],
    lanes: &mut Vec<Lane>,
) -> Wire {
    let a = local(cx.flow, p0);
    let b = local(cx.flow, p3);
    let corner = match cx.style {
        WireStyle::Straight => Corner::Sharp,
        WireStyle::Angled | WireStyle::Subway => Corner::Chamfer(8.0),
        WireStyle::Curved | WireStyle::Circuit => Corner::Round(BACK_CORNER),
    };
    if !via.is_empty() {
        // through the waypoints: every stretch leaves and arrives along the flow
        let mut pts: Vec<L> = Vec::new();
        let mut from = a;
        for &w in via.iter().chain(std::iter::once(&p3)) {
            let to = local(cx.flow, w);
            let part = stretch(cx.style, cx.diagonal, from, to, Lanes::default());
            if pts.is_empty() {
                pts.extend(part);
            } else {
                pts.extend(part.into_iter().skip(1));
            }
            from = to;
        }
        return Wire { points: pts.into_iter().map(|l| world(cx.flow, l)).collect(), back: false };
    }
    if goes_back(cx.flow, p0, p3) {
        let local_boxes: Vec<[f32; 4]> = boxes
            .iter()
            .map(|r| match cx.flow {
                Flow::Down => *r,
                Flow::Right => [r[1], r[0], r[3], r[2]],
            })
            .collect();
        let pts = route_back(&local_boxes, a, b, lanes, corner);
        return Wire { points: pts.into_iter().map(|l| world(cx.flow, l)).collect(), back: true };
    }
    let pts = stretch(cx.style, cx.diagonal, a, b, lanes_of);
    Wire { points: pts.into_iter().map(|l| world(cx.flow, l)).collect(), back: false }
}

/// The squared distance of `p` to the segment `a`-`b`.
#[must_use]
pub fn seg_dist2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
}

/// How far `p` is from a polyline.
#[must_use]
pub fn distance(points: &[[f32; 2]], p: [f32; 2]) -> f32 {
    points.windows(2).map(|w| seg_dist2(p, w[0], w[1])).fold(f32::MAX, f32::min).sqrt()
}

/// The point at `t` (0-1) of a polyline's length, and the direction there.
#[must_use]
pub fn point_at(points: &[[f32; 2]], t: f32) -> Option<([f32; 2], [f32; 2])> {
    let total: f32 = points.windows(2).map(|w| dist(w[0], w[1])).sum();
    if total <= 0.0 {
        return None;
    }
    let mut left = total * t.clamp(0.0, 1.0);
    for w in points.windows(2) {
        let l = dist(w[0], w[1]);
        if l <= 0.0 {
            continue;
        }
        if left <= l {
            let f = left / l;
            let dir = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
            return Some(([w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f], dir));
        }
        left -= l;
    }
    let n = points.len();
    let (a, b) = (points.get(n.wrapping_sub(2))?, points.get(n - 1)?);
    let l = dist(*a, *b).max(1e-6);
    Some((*b, [(b[0] - a[0]) / l, (b[1] - a[1]) / l]))
}

/// A polyline's length.
#[must_use]
pub fn length(points: &[[f32; 2]]) -> f32 {
    points.windows(2).map(|w| dist(w[0], w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(flow: Flow, style: WireStyle, p0: [f32; 2], p3: [f32; 2]) -> Wire {
        let cx = WireCx { flow, style, diagonal: Diagonal::Middle };
        build(&cx, p0, p3, &[], Lanes { index: 0, count: 1, track: 0, tracks: 1 }, &[], &mut Vec::new())
    }

    fn ends(w: &Wire, p0: [f32; 2], p3: [f32; 2]) {
        let (a, b) =
            (w.points.first().copied().unwrap_or_default(), w.points.last().copied().unwrap_or_default());
        assert!(dist(a, p0) < 0.01 && dist(b, p3) < 0.01, "{:?}", w.points);
    }

    #[test]
    fn every_style_joins_its_pins_in_both_flows() {
        for flow in [Flow::Right, Flow::Down] {
            for style in WireStyle::ALL {
                // forward, sideways and backward pairs
                let forward = match flow {
                    Flow::Right => ([0.0, 0.0], [300.0, 120.0]),
                    Flow::Down => ([0.0, 0.0], [120.0, 300.0]),
                };
                let sideways = match flow {
                    Flow::Right => ([0.0, 0.0], [200.0, -40.0]),
                    Flow::Down => ([0.0, 0.0], [-40.0, 200.0]),
                };
                let backward = match flow {
                    Flow::Right => ([300.0, 0.0], [0.0, 100.0]),
                    Flow::Down => ([0.0, 300.0], [100.0, 0.0]),
                };
                for (p0, p3) in [forward, sideways, backward] {
                    let w = wire(flow, style, p0, p3);
                    ends(&w, p0, p3);
                    assert!(w.points.len() >= 2);
                    // the wire leaves its pin along the flow
                    let (a, b) = (local(flow, w.points[0]), local(flow, w.points[1]));
                    if !w.back {
                        assert!(b[1] > a[1] - 0.01, "{style:?} {flow:?} leaves backwards: {:?}", w.points);
                    }
                }
                assert!(wire(flow, style, backward.0, backward.1).back);
                assert!(!wire(flow, style, forward.0, forward.1).back);
            }
        }
    }

    /// Every style's polyline for a forward, a sideways and a backward pair in both flows, to a
    /// tenth of a point: a change of the geometry is a change of `fixtures/wire_snapshots.txt`
    /// (written again with GZ_SNAPSHOTS=1).
    #[test]
    fn the_polylines_are_the_snapshot() {
        use std::fmt::Write as _;
        let mut out = String::new();
        for flow in [Flow::Right, Flow::Down] {
            for style in WireStyle::ALL {
                let pairs = [
                    ("forward", [0.0, 0.0], local(flow, [120.0, 300.0])),
                    ("sideways", [0.0, 0.0], local(flow, [-40.0, 200.0])),
                    ("backward", local(flow, [0.0, 300.0]), local(flow, [100.0, 0.0])),
                ];
                for (name, p0, p3) in pairs {
                    let w = wire(flow, style, p0, p3);
                    let pts: Vec<String> =
                        w.points.iter().map(|p| format!("{:.1},{:.1}", p[0], p[1])).collect();
                    let _ = writeln!(
                        out,
                        "{flow:?} {style:?} {name}{}: {}",
                        if w.back { " back" } else { "" },
                        pts.join(" ")
                    );
                }
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/wire_snapshots.txt");
        if std::env::var_os("GZ_SNAPSHOTS").is_some() {
            assert!(std::fs::write(&path, &out).is_ok());
            return;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(want == out, "the wires changed:\n{out}");
    }

    #[test]
    fn the_shapes() {
        // straight: two stubs and one run
        let s = wire(Flow::Right, WireStyle::Straight, [0.0, 0.0], [300.0, 100.0]);
        assert_eq!(s.points, vec![[0.0, 0.0], [16.0, 0.0], [284.0, 100.0], [300.0, 100.0]]);
        // circuit: every stretch is along an axis or a round corner
        let c = wire(Flow::Down, WireStyle::Circuit, [0.0, 0.0], [100.0, 300.0]);
        assert!(c.points.iter().any(|p| (p[1] - 150.0).abs() < 0.01), "{:?}", c.points);
        // angled: a 45 degree stretch
        let a = wire(Flow::Right, WireStyle::Angled, [0.0, 0.0], [400.0, 100.0]);
        let diag = a.points.windows(2).any(|w| {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            dx > 20.0 && (dx - dy).abs() < 0.01
        });
        assert!(diag, "{:?}", a.points);
        // curved: the tangent grows with the breadth, so a wide fan-out bows
        let near = wire(Flow::Down, WireStyle::Curved, [0.0, 0.0], [600.0, 120.0]);
        let mid = point_at(&near.points, 0.25).map(|(p, _)| p).unwrap_or_default();
        assert!(mid[1] > 40.0, "{mid:?}");
    }

    #[test]
    fn sibling_lanes_never_overlap() {
        let cx = WireCx { flow: Flow::Right, style: WireStyle::Circuit, diagonal: Diagonal::Middle };
        let mut runs: Vec<f32> = Vec::new();
        for k in 0..4 {
            let p0 = [200.0, 14.0 + 18.0 * k as f32];
            let p3 = [400.0, -100.0 + 90.0 * k as f32];
            let w = build(
                &cx,
                p0,
                p3,
                &[],
                Lanes { index: k, count: 4, track: 0, tracks: 1 },
                &[],
                &mut Vec::new(),
            );
            // the run across the flow: the stretch whose x does not change
            let x = w
                .points
                .windows(2)
                .find(|s| (s[0][0] - s[1][0]).abs() < 0.01 && (s[0][1] - s[1][1]).abs() > 10.0)
                .map(|s| s[0][0]);
            runs.extend(x);
        }
        runs.sort_by(f32::total_cmp);
        assert_eq!(runs.len(), 4);
        assert!(runs.windows(2).all(|w| w[1] - w[0] >= LANE - 0.01), "{runs:?}");
    }

    #[test]
    fn hit_tests_find_every_style() {
        for flow in [Flow::Right, Flow::Down] {
            for style in WireStyle::ALL {
                let w = wire(flow, style, [0.0, 0.0], local(flow, [80.0, 320.0]));
                let (on, _) = point_at(&w.points, 0.5).unwrap_or_default();
                assert!(distance(&w.points, on) < 0.5);
                assert!(distance(&w.points, [-200.0, -200.0]) > 10.0);
            }
        }
    }

    #[test]
    fn back_edges_share_their_target_channel() {
        let cx = WireCx { flow: Flow::Down, style: WireStyle::Curved, diagonal: Diagonal::Middle };
        let mut lanes = Vec::new();
        let boxes = [[-100.0, 0.0, 100.0, 60.0], [-100.0, 200.0, 100.0, 260.0]];
        let a = build(&cx, [0.0, 260.0], [0.0, 0.0], &[], Lanes::default(), &boxes, &mut lanes);
        let b = build(&cx, [40.0, 260.0], [0.0, 0.0], &[], Lanes::default(), &boxes, &mut lanes);
        assert!(a.back && b.back);
        let chan = |w: &Wire| w.points.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        assert!((chan(&a) - chan(&b)).abs() < 0.01, "{:?} {:?}", a.points, b.points);
        let mut spans = vec![(-10.0, 10.0)];
        assert_eq!(pick_channel(&mut spans, 0.0, 0.0), -10.0);
    }

    #[test]
    fn waypoints_are_passed_through() {
        let cx = WireCx { flow: Flow::Right, style: WireStyle::Curved, diagonal: Diagonal::Middle };
        let via = [[150.0, 200.0]];
        let w = build(&cx, [0.0, 0.0], [300.0, 0.0], &via, Lanes::default(), &[], &mut Vec::new());
        assert!(distance(&w.points, via[0]) < 0.5);
        ends(&w, [0.0, 0.0], [300.0, 0.0]);
    }
}

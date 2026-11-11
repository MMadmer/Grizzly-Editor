//! The Way tool's links and points (`WayPoint.cpp`): the link edits of the selected points, a
//! point added to a way, the points removed, and the point picking of Points mode.

use crate::math::Vec3;
use crate::objects::{WAYPOINT_RADIUS, WayData};

fn find_link(w: &WayData, from: usize, to: usize) -> Option<usize> {
    let to = u32::try_from(to).ok()?;
    w.points.get(from)?.links.iter().position(|(t, _)| *t == to)
}

fn take_link(w: &mut WayData, from: usize, at: usize) -> Option<f32> {
    let p = w.points.get_mut(from)?;
    (at < p.links.len()).then(|| p.links.remove(at).1)
}

/// `CWayPoint::InvertLink`.
fn invert_link(w: &mut WayData, a: usize, b: usize) {
    let la = find_link(w, a, b);
    let lb = find_link(w, b, a);
    let pa = la.and_then(|i| take_link(w, a, i));
    let pb = lb.and_then(|i| take_link(w, b, i));
    if let Some(p) = pa {
        w.create_link(b, a, p);
    }
    if let Some(p) = pb {
        w.create_link(a, b, p);
    }
}

/// `CWayPoint::Convert1Link`: a two-way link becomes `a` to `b`.
fn convert_1link(w: &mut WayData, a: usize, b: usize) {
    let (la, lb) = (find_link(w, a, b), find_link(w, b, a));
    if la.is_none() || lb.is_none() {
        return;
    }
    let pa = la.and_then(|i| take_link(w, a, i)).unwrap_or(1.0);
    if let Some(i) = find_link(w, b, a) {
        take_link(w, b, i);
    }
    w.create_link(a, b, pa);
}

/// `CWayPoint::Convert2Link`: a one-way link becomes two-way.
fn convert_2link(w: &mut WayData, a: usize, b: usize) {
    let (la, lb) = (find_link(w, a, b), find_link(w, b, a));
    if la.is_some() == lb.is_some() {
        return;
    }
    let pa = la.and_then(|i| take_link(w, a, i)).unwrap_or(1.0);
    let pb = find_link(w, b, a).and_then(|i| take_link(w, b, i)).unwrap_or(1.0);
    w.create_link(b, a, pb);
    w.create_link(a, b, pa);
}

/// `CWayPoint::DeleteLink`.
fn delete_link(w: &mut WayData, a: usize, b: usize) -> bool {
    match find_link(w, a, b) {
        Some(i) => take_link(w, a, i).is_some(),
        None => false,
    }
}

/// `CWayPoint::RemoveLink`: both directions, when the first one was there.
fn remove_link(w: &mut WayData, a: usize, b: usize) -> bool {
    if delete_link(w, a, b) {
        delete_link(w, b, a);
        return true;
    }
    false
}

fn selected(w: &WayData) -> Vec<usize> {
    w.points.iter().enumerate().filter(|(_, p)| p.selected).map(|(i, _)| i).collect()
}

/// Every pair of selected points, the earlier one first (the `_A < _B` loops).
fn pairs(w: &WayData) -> Vec<(usize, usize)> {
    let s = selected(w);
    let mut out = Vec::new();
    for (i, a) in s.iter().enumerate() {
        for b in s.iter().skip(i + 1) {
            out.push((*a, *b));
        }
    }
    out
}

/// The link edits of the Links section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkOp {
    /// Create 1-Link: `RemoveLink(); Add1Link()`.
    Create1,
    /// Create 2-Link: `Add2Link()`.
    Create2,
    Convert1,
    Convert2,
    Invert,
    Remove,
}

impl LinkOp {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "create_1link" | "create_1_link" => Self::Create1,
            "create_2link" | "create_2_link" => Self::Create2,
            "convert_1link" | "convert_to_1link" | "convert_1_link" => Self::Convert1,
            "convert_2link" | "convert_to_2link" | "convert_2_link" => Self::Convert2,
            "invert_link" | "invert" => Self::Invert,
            "remove_link" => Self::Remove,
            _ => return None,
        })
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Create1 => "Create 1-Link",
            Self::Create2 => "Create 2-Link",
            Self::Convert1 => "Convert to 1-Link",
            Self::Convert2 => "Convert to 2-Link",
            Self::Invert => "Invert Link",
            Self::Remove => "Remove Link",
        }
    }

    /// The commands that need points to act on (the panel's "Switch Select to Points first").
    #[must_use]
    pub fn needs_points(self) -> bool {
        matches!(self, Self::Create1 | Self::Create2 | Self::Invert | Self::Remove)
    }
}

/// `CWayObject::RemoveLink`: between every two selected points, both ways.
fn remove_links(w: &mut WayData) {
    let s = selected(w);
    for a in &s {
        for b in &s {
            remove_link(w, *a, *b);
        }
    }
}

/// One link edit of a way; whether the C++ would count it as a change (`EditSelectedWays`).
pub fn edit_links(w: &mut WayData, op: LinkOp) -> bool {
    match op {
        LinkOp::Create1 => {
            remove_links(w);
            let mut changed = false;
            for (a, b) in pairs(w) {
                changed |= w.append_link(a, b, 1.0);
                invert_link(w, a, b);
            }
            changed
        }
        LinkOp::Create2 => {
            let mut changed = false;
            for (a, b) in pairs(w) {
                let x = w.append_link(a, b, 1.0);
                let y = w.append_link(b, a, 1.0);
                changed |= x | y;
            }
            changed
        }
        LinkOp::Convert1 => {
            for (a, b) in pairs(w) {
                convert_1link(w, a, b);
            }
            true
        }
        LinkOp::Convert2 => {
            for (a, b) in pairs(w) {
                convert_2link(w, a, b);
            }
            true
        }
        LinkOp::Invert => {
            for (a, b) in pairs(w) {
                invert_link(w, a, b);
            }
            true
        }
        LinkOp::Remove => {
            remove_links(w);
            true
        }
    }
}

/// `AppendWayPoint` + `MoveTo` + Auto Link: the points deselected, a new selected point there,
/// linked from the point that was selected first. Answers the new point's index.
pub fn add_point(w: &mut WayData, at: Vec3, auto_link: bool) -> usize {
    let last = w.points.iter().position(|p| p.selected);
    let name = w.next_point_name();
    for p in &mut w.points {
        p.selected = false;
    }
    let mut wp = crate::objects::WayPoint::new(&name, at);
    wp.selected = true;
    w.points.push(wp);
    let i = w.points.len() - 1;
    if auto_link && let Some(l) = last {
        w.append_link(l, i, 1.0);
    }
    i
}

/// `RemoveSelectedPoints`: the links to them go, then the points; the other links follow their
/// points' new indices.
pub fn remove_selected_points(w: &mut WayData) -> usize {
    let gone: Vec<bool> = w.points.iter().map(|p| p.selected).collect();
    let count = gone.iter().filter(|g| **g).count();
    if count == 0 {
        return 0;
    }
    let mut map = Vec::with_capacity(gone.len());
    let mut next = 0u32;
    for g in &gone {
        map.push((!g).then(|| {
            next += 1;
            next - 1
        }));
    }
    let mut kept = Vec::with_capacity(w.points.len() - count);
    for (i, mut p) in std::mem::take(&mut w.points).into_iter().enumerate() {
        if gone.get(i).copied().unwrap_or(false) {
            continue;
        }
        p.links = p
            .links
            .into_iter()
            .filter_map(|(to, prob)| map.get(to as usize).copied().flatten().map(|t| (t, prob)))
            .collect();
        kept.push(p);
    }
    w.points = kept;
    count
}

/// `CWayPoint::RayPick`: the distance along the ray to a point's mark.
#[must_use]
pub fn point_ray(position: Vec3, start: Vec3, dir: Vec3) -> Option<f32> {
    let mut ray2 = position - start;
    ray2.y += WAYPOINT_RADIUS;
    let d = ray2.dot(dir);
    if d <= 0.0 {
        return None;
    }
    let d2 = ray2.length();
    ((d2 * d2 - d * d) < WAYPOINT_RADIUS * WAYPOINT_RADIUS && d > WAYPOINT_RADIUS).then_some(d)
}

/// The nearest point of a way a ray meets within `range`.
#[must_use]
pub fn pick_point(w: &WayData, start: Vec3, dir: Vec3, range: f32) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (i, p) in w.points.iter().enumerate() {
        if let Some(d) = point_ray(p.position, start, dir)
            && d < best.map_or(range, |b| b.1)
        {
            best = Some((i, d));
        }
    }
    best
}

/// A point's name or its index, as `way_tool` takes them.
#[must_use]
pub fn point_index(w: &WayData, key: &str) -> Option<usize> {
    w.points
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(key))
        .or_else(|| key.parse::<usize>().ok().filter(|i| *i < w.points.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(n: usize) -> WayData {
        let pts: Vec<Vec3> = (0..n).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        let mut w = WayData::build(&pts, false);
        for p in &mut w.points {
            p.links.clear();
            p.selected = true;
        }
        w
    }

    #[test]
    fn create_1link_ends_up_pointing_back_as_the_cpp_does() {
        // AddSingleLink(A, B) and then InvertLink(A, B): the link runs from B to A
        let mut w = way(2);
        assert!(edit_links(&mut w, LinkOp::Create1));
        assert!(w.points[0].links.is_empty());
        assert_eq!(w.points[1].links, vec![(0, 1.0)]);
    }

    #[test]
    fn two_links_convert_to_one_and_back() {
        let mut w = way(2);
        edit_links(&mut w, LinkOp::Create2);
        assert_eq!(w.points[0].links.len(), 1);
        assert_eq!(w.points[1].links.len(), 1);
        edit_links(&mut w, LinkOp::Convert1);
        assert_eq!(w.points[0].links, vec![(1, 1.0)]);
        assert!(w.points[1].links.is_empty());
        edit_links(&mut w, LinkOp::Convert2);
        assert_eq!(w.points[1].links, vec![(0, 1.0)]);
        edit_links(&mut w, LinkOp::Remove);
        assert!(w.points.iter().all(|p| p.links.is_empty()));
    }

    #[test]
    fn removed_points_take_their_links_and_the_rest_follow() {
        let mut w = WayData::build(&[Vec3::ZERO, Vec3::X, Vec3::Z], true);
        for p in &mut w.points {
            p.selected = false;
        }
        w.points[1].selected = true;
        assert_eq!(remove_selected_points(&mut w), 1);
        assert_eq!(w.points.len(), 2);
        // 0 -> 1 was dropped with point 1; 2 -> 0 is now 1 -> 0
        assert!(w.points[0].links.is_empty());
        assert_eq!(w.points[1].links, vec![(0, 1.0)]);
    }

    #[test]
    fn a_point_added_links_from_the_selected_one() {
        let mut w = WayData::build(&[Vec3::ZERO], false);
        let i = add_point(&mut w, Vec3::X, true);
        assert_eq!(i, 1);
        assert_eq!(w.points[0].links, vec![(1, 1.0)]);
        assert!(w.points[1].selected && !w.points[0].selected);
        assert_eq!(w.points[1].name, "wp_00");
    }
}

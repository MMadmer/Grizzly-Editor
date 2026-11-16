//! Helper geometry on the CPU: the grid and every helper shape as coloured lines and triangles.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::color::Color;
use crate::draw_list::{Helpers, Shape, ShapeKind};
use crate::view::Grid;

/// A vertex of a helper line or triangle (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
}

impl LineVertex {
    #[must_use]
    pub fn new(p: Vec3, color: Color) -> Self {
        Self { position: p.to_array(), color: color.0 }
    }
}

/// Helper geometry sorted by how it is drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HelperGeometry {
    /// Line lists hidden behind surfaces.
    pub lines: Vec<LineVertex>,
    /// Line lists drawn over everything.
    pub lines_overlay: Vec<LineVertex>,
    /// Blended triangles hidden behind surfaces.
    pub tris: Vec<LineVertex>,
    pub tris_overlay: Vec<LineVertex>,
}

/// The grid of the ground plane (y = 0) around the camera, as far as the camera sees it: a cell
/// of `cell_size` while the camera is low and `subdiv` times coarser at each step of height, the
/// thick lines every `subdiv` cells, every line fading out with its distance from the camera - the
/// grid has no edge to run into. Its lines stay on their world places as the camera moves.
pub(crate) fn grid_lines(grid: &Grid, eye: Vec3, out: &mut Vec<LineVertex>) {
    let sub = grid.subdiv.max(2);
    let height = eye.y.abs().max(1.0);
    // the finest step that keeps about forty lines across the camera's height
    let mut step = grid.cell_size.max(1e-3);
    for _ in 0..12 {
        if step * 40.0 >= height {
            break;
        }
        step *= sub as f32;
    }
    let coarse = step * sub as f32;
    let reach = (height * 12.0).max(coarse * 3.0).min(200_000.0);
    // centred on a coarse line under the camera, so the pattern does not swim
    let (cx, cz) = ((eye.x / coarse).round() * coarse, (eye.z / coarse).round() * coarse);
    let n = (reach / step).ceil() as i64;
    // the fine lines give way as the camera rises toward the next step
    let fine =
        1.0 - ((height / (step * 40.0)) * sub as f32 - 1.0).clamp(0.0, sub as f32 - 1.0) / (sub as f32 - 1.0);
    let fade = |x: f32, z: f32, base: Color, weight: f32| {
        let d = ((x - eye.x).powi(2) + (z - eye.z).powi(2)).sqrt();
        let a = (1.0 - d / reach).clamp(0.0, 1.0);
        base.with_alpha((f32::from(base.0[3]) * a * a * weight).round() as u8)
    };
    const PIECES: usize = 8;
    for along_x in [false, true] {
        for i in -n..=n {
            let thick = i.rem_euclid(i64::from(sub)) == 0;
            let (base, weight) = if thick { (grid.thick_color, 1.0) } else { (grid.color, fine) };
            if weight <= 0.0 {
                continue;
            }
            // the line's offset from the camera and the half of it within reach
            let at = if along_x { cz } else { cx } + i as f32 * step;
            let off = at - if along_x { eye.z } else { eye.x };
            let half = (reach * reach - off * off).max(0.0).sqrt();
            if half <= 0.0 {
                continue;
            }
            let mid = if along_x { eye.x } else { eye.z };
            for k in 0..PIECES {
                let t0 = mid - half + 2.0 * half * k as f32 / PIECES as f32;
                let t1 = mid - half + 2.0 * half * (k + 1) as f32 / PIECES as f32;
                for t in [t0, t1] {
                    let (x, z) = if along_x { (t, at) } else { (at, t) };
                    out.push(LineVertex::new(Vec3::new(x, 0.0, z), fade(x, z, base, weight)));
                }
            }
        }
    }
}

const CIRCLE: usize = 32;

fn circle(i: usize) -> (f32, f32) {
    let a = std::f32::consts::TAU * i as f32 / CIRCLE as f32;
    (a.cos(), a.sin())
}

fn push_line(out: &mut Vec<LineVertex>, m: &Mat4, a: Vec3, b: Vec3, c: Color) {
    out.push(LineVertex::new(m.transform_point3(a), c));
    out.push(LineVertex::new(m.transform_point3(b), c));
}

fn push_tri(out: &mut Vec<LineVertex>, m: &Mat4, a: Vec3, b: Vec3, c: Vec3, col: Color) {
    for p in [a, b, c] {
        out.push(LineVertex::new(m.transform_point3(p), col));
    }
}

fn box_corner(i: usize) -> Vec3 {
    Vec3::new(
        if i & 1 == 0 { -0.5 } else { 0.5 },
        if i & 2 == 0 { -0.5 } else { 0.5 },
        if i & 4 == 0 { -0.5 } else { 0.5 },
    )
}

const BOX_EDGES: [(usize, usize); 12] =
    [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
const BOX_FACES: [[usize; 4]; 6] =
    [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];

/// Every shape and line of `helpers`, transformed into the world.
pub(crate) fn tessellate(helpers: &Helpers, out: &mut HelperGeometry) {
    for l in &helpers.lines {
        let dst = if l.depth_tested { &mut out.lines } else { &mut out.lines_overlay };
        dst.push(LineVertex::new(l.a, l.color));
        dst.push(LineVertex::new(l.b, l.color));
    }
    for s in &helpers.shapes {
        let (lines, tris) = if s.depth_tested {
            (&mut out.lines, &mut out.tris)
        } else {
            (&mut out.lines_overlay, &mut out.tris_overlay)
        };
        if let Some(c) = s.wire {
            shape_lines(s, c, lines);
        }
        if let Some(c) = s.fill {
            shape_tris(s, c, tris);
        }
    }
}

fn shape_lines(s: &Shape, c: Color, out: &mut Vec<LineVertex>) {
    let m = &s.transform;
    match s.kind {
        ShapeKind::Box => {
            for (a, b) in BOX_EDGES {
                push_line(out, m, box_corner(a), box_corner(b), c);
            }
        }
        ShapeKind::Sphere => {
            for i in 0..CIRCLE {
                let (c0, s0) = circle(i);
                let (c1, s1) = circle(i + 1);
                push_line(out, m, Vec3::new(c0, s0, 0.0), Vec3::new(c1, s1, 0.0), c);
                push_line(out, m, Vec3::new(0.0, c0, s0), Vec3::new(0.0, c1, s1), c);
                push_line(out, m, Vec3::new(s0, 0.0, c0), Vec3::new(s1, 0.0, c1), c);
            }
        }
        ShapeKind::Cone => {
            for i in 0..CIRCLE {
                let (c0, s0) = circle(i);
                let (c1, s1) = circle(i + 1);
                push_line(out, m, Vec3::new(c0, s0, 1.0), Vec3::new(c1, s1, 1.0), c);
                if i % 4 == 0 {
                    push_line(out, m, Vec3::ZERO, Vec3::new(c0, s0, 1.0), c);
                }
            }
        }
        ShapeKind::Cylinder => {
            for i in 0..CIRCLE {
                let (c0, s0) = circle(i);
                let (c1, s1) = circle(i + 1);
                for z in [-0.5, 0.5] {
                    push_line(out, m, Vec3::new(c0, s0, z), Vec3::new(c1, s1, z), c);
                }
                if i % 4 == 0 {
                    push_line(out, m, Vec3::new(c0, s0, -0.5), Vec3::new(c0, s0, 0.5), c);
                }
            }
        }
        ShapeKind::Plane => {
            let p = [
                Vec3::new(-0.5, 0.0, -0.5),
                Vec3::new(0.5, 0.0, -0.5),
                Vec3::new(0.5, 0.0, 0.5),
                Vec3::new(-0.5, 0.0, 0.5),
            ];
            for i in 0..4 {
                push_line(out, m, p[i], p[(i + 1) % 4], c);
            }
        }
        ShapeKind::Cross { rot45 } => {
            let arms = [Vec3::X, Vec3::Y, Vec3::Z];
            for a in arms {
                push_line(out, m, a, -a, c);
            }
            if rot45 {
                let r = Mat4::from_euler(
                    glam::EulerRot::YXZ,
                    std::f32::consts::FRAC_PI_4,
                    std::f32::consts::FRAC_PI_4,
                    std::f32::consts::FRAC_PI_4,
                );
                for a in arms {
                    let d = r.transform_vector3(a);
                    push_line(out, m, d, -d, c);
                }
            }
        }
        ShapeKind::SelectionBox => {
            // DrawSelectionBox: a quarter of every edge from each corner of a box a little larger
            for i in 0..8 {
                let p = box_corner(i) * 1.01;
                let sign = p.signum();
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    push_line(out, m, p, p - sign * axis * 0.25, c);
                }
            }
        }
        ShapeKind::Romboid => {
            for (a, b) in ROMBOID_EDGES {
                push_line(out, m, ROMBOID[a], ROMBOID[b], c);
            }
        }
    }
}

const ROMBOID: [Vec3; 6] = [
    Vec3::new(0.0, 1.0, 0.0),
    Vec3::new(0.0, -1.0, 0.0),
    Vec3::new(0.0, 0.0, -1.0),
    Vec3::new(0.0, 0.0, 1.0),
    Vec3::new(-1.0, 0.0, 0.0),
    Vec3::new(1.0, 0.0, 0.0),
];
const ROMBOID_EDGES: [(usize, usize); 12] =
    [(0, 2), (2, 5), (0, 5), (3, 5), (3, 0), (4, 3), (4, 0), (4, 2), (1, 2), (1, 5), (1, 3), (1, 4)];
const ROMBOID_TRIS: [[usize; 3]; 8] =
    [[2, 4, 0], [4, 3, 0], [3, 5, 0], [5, 2, 0], [4, 2, 1], [2, 5, 1], [5, 3, 1], [3, 4, 1]];

fn shape_tris(s: &Shape, c: Color, out: &mut Vec<LineVertex>) {
    let m = &s.transform;
    match s.kind {
        ShapeKind::Box | ShapeKind::SelectionBox => {
            for f in BOX_FACES {
                let [a, b, cc, d] = f.map(box_corner);
                push_tri(out, m, a, b, cc, c);
                push_tri(out, m, a, cc, d, c);
            }
        }
        ShapeKind::Sphere => {
            const RINGS: usize = 8;
            let at = |r: usize, s: usize| {
                let theta = std::f32::consts::PI * r as f32 / RINGS as f32;
                let (cp, sp) = circle(s);
                Vec3::new(theta.sin() * cp, theta.cos(), theta.sin() * sp)
            };
            for r in 0..RINGS {
                for seg in 0..CIRCLE / 2 {
                    let (s0, s1) = (seg * 2, seg * 2 + 2);
                    push_tri(out, m, at(r, s0), at(r, s1), at(r + 1, s1), c);
                    push_tri(out, m, at(r, s0), at(r + 1, s1), at(r + 1, s0), c);
                }
            }
        }
        ShapeKind::Cone => {
            for i in 0..CIRCLE {
                let (c0, s0) = circle(i);
                let (c1, s1) = circle(i + 1);
                push_tri(out, m, Vec3::ZERO, Vec3::new(c0, s0, 1.0), Vec3::new(c1, s1, 1.0), c);
                push_tri(out, m, Vec3::new(0.0, 0.0, 1.0), Vec3::new(c1, s1, 1.0), Vec3::new(c0, s0, 1.0), c);
            }
        }
        ShapeKind::Cylinder => {
            for i in 0..CIRCLE {
                let (c0, s0) = circle(i);
                let (c1, s1) = circle(i + 1);
                let (a, b) = (Vec3::new(c0, s0, -0.5), Vec3::new(c1, s1, -0.5));
                let (a2, b2) = (Vec3::new(c0, s0, 0.5), Vec3::new(c1, s1, 0.5));
                push_tri(out, m, a, b, b2, c);
                push_tri(out, m, a, b2, a2, c);
                push_tri(out, m, Vec3::new(0.0, 0.0, -0.5), b, a, c);
                push_tri(out, m, Vec3::new(0.0, 0.0, 0.5), a2, b2, c);
            }
        }
        ShapeKind::Plane => {
            let p = [
                Vec3::new(-0.5, 0.0, -0.5),
                Vec3::new(0.5, 0.0, -0.5),
                Vec3::new(0.5, 0.0, 0.5),
                Vec3::new(-0.5, 0.0, 0.5),
            ];
            push_tri(out, m, p[0], p[1], p[2], c);
            push_tri(out, m, p[0], p[2], p[3], c);
        }
        ShapeKind::Romboid => {
            // DrawRomboid: the faces a quarter darker than the edges
            let c = c.scaled(0.75);
            for [a, b, cc] in ROMBOID_TRIS {
                push_tri(out, m, ROMBOID[a], ROMBOID[b], ROMBOID[cc], c);
            }
        }
        ShapeKind::Cross { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_follows_the_camera_and_has_no_edge() {
        let g = Grid { cell_size: 1.0, subdiv: 10, ..Grid::default() };
        let lines = |eye: Vec3| {
            let mut out = Vec::new();
            grid_lines(&g, eye, &mut out);
            out
        };
        // low: metre cells around the camera, wherever it stands, faded to nothing at the reach
        let here = lines(Vec3::new(5000.0, 2.0, -3000.0));
        assert!(!here.is_empty());
        assert!(
            here.iter()
                .all(|v| (v.position[0] - 5000.0).abs() <= 40.0 && (v.position[2] + 3000.0).abs() <= 40.0)
        );
        assert!(here.iter().any(|v| v.position[0].fract() != 0.0 || v.position[2].fract() != 0.0));
        assert!(here.iter().all(|v| v.position[1] == 0.0));
        let mut far = here
            .iter()
            .filter(|v| ((v.position[0] - 5000.0).powi(2) + (v.position[2] + 3000.0).powi(2)).sqrt() > 29.0);
        assert!(far.clone().count() > 0 && far.all(|v| v.color[3] < 8), "a line fades out at its end");
        // high: the cells grow with the height and the grid reaches as far as the camera sees
        let high = lines(Vec3::new(0.0, 900.0, 0.0));
        let xs: Vec<f32> = high.iter().map(|v| v.position[0]).collect();
        assert!(xs.iter().any(|x| *x > 9000.0), "it reaches past ten times the height");
        let thin_lines = high.len() / 2;
        assert!(thin_lines < 30_000, "{thin_lines} segments");
    }

    #[test]
    fn shapes_split_by_depth() {
        let mut h = Helpers::default();
        h.aabb(&crate::math::Aabb::UNIT, Some(Color::WHITE), Some(Color::BLACK), true);
        h.sphere(Vec3::ZERO, 2.0, None, Some(Color::WHITE), false);
        h.line(Vec3::ZERO, Vec3::X, Color::WHITE, false);
        h.arrow(Vec3::ZERO, Vec3::Z, 0.2, Color::WHITE, true);
        let mut g = HelperGeometry::default();
        tessellate(&h, &mut g);
        assert_eq!(g.lines.len(), 12 * 2 + 5 * 2);
        assert_eq!(g.tris.len(), 12 * 3);
        assert_eq!(g.lines_overlay.len(), 3 * CIRCLE * 2 + 2);
        assert!(g.tris_overlay.is_empty());
        // the sphere's circles lie on its radius (after the overlay line: lines go first)
        assert!(
            g.lines_overlay
                .iter()
                .skip(2)
                .take(6)
                .all(|v| (Vec3::from_array(v.position).length() - 2.0).abs() < 1e-5)
        );
    }
}

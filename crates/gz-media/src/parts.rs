//! Geometry an asset writes instead of a source: boxes, ramps, cylinders, planes and prisms, made
//! into the model the rest of the editor reads (`gz_fmt_object::Object`).
//!
//! Every face looks outward (its corners turn so that `(b - a) x (c - a)` points out of the
//! part), every edge is hard, and texture coordinates are laid by box mapping in the part's own
//! frame: a face takes the plane of its normal's largest axis, one texture every `tile` metres,
//! so turning or moving a part never makes its texture swim.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use gz_fmt_object::glam::{EulerRot, Mat3, Vec2, Vec3};
use gz_fmt_object::{Face, FaceVert, Mesh, Object, SurfFaces, Surface, SurfaceFlags, VMap, VMapKind, VMapPt};

use crate::model::{BOX, CYLINDER, PLANE, PRISM, Part, RAMP, SurfaceRow};

/// A surface valid for the rest of the editor, as a source's come out.
#[must_use]
pub fn default_surface(name: &str) -> Surface {
    Surface {
        name: name.into(),
        texture: "".into(),
        vmap: "Texture".into(),
        shader: "default".into(),
        shader_xrlc: "default".into(),
        game_mtl: "default".into(),
        flags: SurfaceFlags::empty(),
        // a position, a normal and one texture coordinate set
        fvf: 0x112,
    }
}

/// A triangle of the model, in its space, with its corners' texture coordinates.
#[derive(Clone, Copy, Debug)]
struct Tri {
    p: [Vec3; 3],
    uv: [Vec2; 3],
    surface: u32,
}

/// The rotation of `{ x, y, z }` degrees: heading about Y, then pitch about X, then bank about Z.
#[must_use]
pub fn rotation(deg: [f64; 3]) -> Mat3 {
    let r = deg.map(|d| (d as f32).to_radians());
    Mat3::from_euler(EulerRot::YXZ, r[1], r[0], r[2])
}

/// Emits a triangle turned to face `want`.
fn tri(out: &mut Vec<[Vec3; 3]>, a: Vec3, b: Vec3, c: Vec3, want: Vec3) {
    if (b - a).cross(c - a).dot(want) < 0.0 {
        out.push([a, c, b]);
    } else {
        out.push([a, b, c]);
    }
}

fn quad(out: &mut Vec<[Vec3; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3, want: Vec3) {
    tri(out, a, b, c, want);
    tri(out, a, c, d, want);
}

/// Twice the signed area of an outline seen from above: above 0 when it runs from +X towards +Z.
fn area2(pts: &[[f64; 2]]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum()
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
}

fn inside(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let s = |p1: [f64; 2], p2: [f64; 2], p3: [f64; 2]| {
        (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
    };
    let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Ear clipping of a simple polygon: the corners of each triangle as indices of `pts`, every one
/// turned the way the outline runs. An outline that crosses itself or has no area is refused.
pub fn triangulate(pts: &[[f64; 2]]) -> Result<Vec<[usize; 3]>, String> {
    let a = area2(pts);
    if pts.len() < 3 || a.abs() < 1e-9 {
        return Err("the outline has no area".into());
    }
    let sign = a.signum();
    // points on a straight line between their neighbours make no ear and no corner
    let n = pts.len();
    let mut left: Vec<usize> =
        (0..n).filter(|&i| cross2(pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]).abs() > 1e-12).collect();
    if left.len() < 3 {
        return Err("the outline has no area".into());
    }
    let mut out = Vec::with_capacity(pts.len());
    while left.len() > 3 {
        let n = left.len();
        let ear = (0..n).find(|&k| {
            let (i0, i1, i2) = (left[(k + n - 1) % n], left[k], left[(k + 1) % n]);
            if cross2(pts[i0], pts[i1], pts[i2]) * sign <= 0.0 {
                return false;
            }
            !left.iter().any(|&j| {
                j != i0 && j != i1 && j != i2 && {
                    let (x, y, z) =
                        if sign > 0.0 { (pts[i0], pts[i1], pts[i2]) } else { (pts[i0], pts[i2], pts[i1]) };
                    inside(pts[j], x, y, z)
                }
            })
        });
        let Some(k) = ear else {
            return Err("the outline crosses itself".into());
        };
        out.push([left[(k + n - 1) % n], left[k], left[(k + 1) % n]]);
        left.remove(k);
    }
    if left.len() == 3 {
        out.push([left[0], left[1], left[2]]);
    }
    Ok(out)
}

/// The triangles of one part in its own frame, base at `y = 0`.
fn shape_tris(part: &Part) -> Result<Vec<[Vec3; 3]>, String> {
    let mut out = Vec::new();
    let size = |i: usize, d: f64| part.size.as_ref().and_then(|s| s.get(i).copied()).unwrap_or(d) as f32;
    match part.shape {
        BOX => {
            let (hx, h, hz) = (size(0, 1.0) / 2.0, size(1, 1.0), size(2, 1.0) / 2.0);
            let v = |x: f32, y: f32, z: f32| Vec3::new(x * hx, y * h, z * hz);
            quad(&mut out, v(-1., 0., -1.), v(1., 0., -1.), v(1., 0., 1.), v(-1., 0., 1.), -Vec3::Y);
            quad(&mut out, v(-1., 1., -1.), v(1., 1., -1.), v(1., 1., 1.), v(-1., 1., 1.), Vec3::Y);
            quad(&mut out, v(-1., 0., -1.), v(1., 0., -1.), v(1., 1., -1.), v(-1., 1., -1.), -Vec3::Z);
            quad(&mut out, v(-1., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(-1., 1., 1.), Vec3::Z);
            quad(&mut out, v(-1., 0., -1.), v(-1., 0., 1.), v(-1., 1., 1.), v(-1., 1., -1.), -Vec3::X);
            quad(&mut out, v(1., 0., -1.), v(1., 0., 1.), v(1., 1., 1.), v(1., 1., -1.), Vec3::X);
        }
        RAMP => {
            let (hx, h, hz) = (size(0, 1.0) / 2.0, size(1, 1.0), size(2, 1.0) / 2.0);
            let (fl, fr) = (Vec3::new(-hx, 0., -hz), Vec3::new(hx, 0., -hz));
            let (bl, br) = (Vec3::new(-hx, 0., hz), Vec3::new(hx, 0., hz));
            let (tl, tr) = (Vec3::new(-hx, h, hz), Vec3::new(hx, h, hz));
            quad(&mut out, fl, fr, br, bl, -Vec3::Y);
            quad(&mut out, bl, br, tr, tl, Vec3::Z);
            quad(&mut out, fl, fr, tr, tl, Vec3::new(0.0, 2.0 * hz, -h));
            tri(&mut out, fl, bl, tl, -Vec3::X);
            tri(&mut out, fr, br, tr, Vec3::X);
        }
        CYLINDER => {
            let (r, h, n) = (part.radius as f32, part.height as f32, part.sides.max(3) as usize);
            let ring: Vec<Vec3> = (0..n)
                .map(|i| {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    Vec3::new(r * t.cos(), 0.0, r * t.sin())
                })
                .collect();
            let up = Vec3::new(0.0, h, 0.0);
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                let mid = (a + b) * 0.5;
                quad(&mut out, a, b, b + up, a + up, Vec3::new(mid.x, 0.0, mid.z));
            }
            for i in 1..n - 1 {
                tri(&mut out, ring[0], ring[i], ring[i + 1], -Vec3::Y);
                tri(&mut out, ring[0] + up, ring[i] + up, ring[i + 1] + up, Vec3::Y);
            }
        }
        PLANE => {
            let (hx, hz) = (size(0, 1.0) / 2.0, size(1, 1.0) / 2.0);
            quad(
                &mut out,
                Vec3::new(-hx, 0., -hz),
                Vec3::new(hx, 0., -hz),
                Vec3::new(hx, 0., hz),
                Vec3::new(-hx, 0., hz),
                Vec3::Y,
            );
        }
        PRISM => {
            let pts = &part.outline;
            let h = part.height as f32;
            let at = |i: usize, y: f32| Vec3::new(pts[i][0] as f32, y, pts[i][1] as f32);
            for t in triangulate(pts)? {
                tri(&mut out, at(t[0], 0.0), at(t[1], 0.0), at(t[2], 0.0), -Vec3::Y);
                tri(&mut out, at(t[0], h), at(t[1], h), at(t[2], h), Vec3::Y);
            }
            let sign = area2(pts).signum() as f32;
            let n = pts.len();
            for i in 0..n {
                let (a, b) = (at(i, 0.0), at((i + 1) % n, 0.0));
                let d = b - a;
                let outward = Vec3::new(d.z, 0.0, -d.x) * sign;
                if d.length_squared() > 0.0 {
                    quad(&mut out, a, b, b + Vec3::new(0.0, h, 0.0), a + Vec3::new(0.0, h, 0.0), outward);
                }
            }
        }
        other => return Err(format!("no shape {other}")),
    }
    Ok(out)
}

/// Box mapping: the plane of the normal's largest axis, `tile` metres a texture.
fn box_uv(p: [Vec3; 3], tile: f32) -> [Vec2; 3] {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    let a = n.abs();
    p.map(|q| {
        let uv = if a.y >= a.x && a.y >= a.z {
            Vec2::new(q.x, q.z)
        } else if a.x >= a.z {
            Vec2::new(q.z, -q.y)
        } else {
            Vec2::new(q.x, -q.y)
        };
        uv / tile
    })
}

/// The model of `parts`: one mesh, the asset's surfaces in their order and then every surface a
/// part names that the asset does not list.
pub fn build(parts: &[Part], rows: &[SurfaceRow]) -> Result<Object, String> {
    let mut surfaces: Vec<Surface> = rows.iter().map(|r| default_surface(&r.name)).collect();
    let mut tris = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let surface = match surfaces.iter().position(|s| s.name.eq_ignore_ascii_case(&part.surface)) {
            Some(s) => s,
            None => {
                surfaces.push(default_surface(&part.surface));
                surfaces.len() - 1
            }
        } as u32;
        let local = shape_tris(part).map_err(|e| format!("part {}: {e}", i + 1))?;
        let rot = rotation(part.rot);
        let pos = Vec3::new(part.pos[0] as f32, part.pos[1] as f32, part.pos[2] as f32);
        let tile = part.tile.max(1e-3) as f32;
        for t in local {
            tris.push(Tri { uv: box_uv(t, tile), p: t.map(|q| rot * q + pos), surface });
        }
    }
    if tris.is_empty() {
        return Err("the parts make no faces".into());
    }
    let mesh = build_mesh("parts", &tris)?;
    Ok(Object { surfaces, meshes: vec![mesh], ..Object::default() })
}

fn bits(v: f32) -> u32 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

/// One mesh of the triangles: positions welded, one vertex map reference per distinct corner,
/// every edge hard.
fn build_mesh(name: &str, tris: &[Tri]) -> Result<Mesh, String> {
    let mut points: Vec<Vec3> = Vec::new();
    let mut point_of: HashMap<[u32; 3], u32> = HashMap::new();
    let mut corners: Vec<(u32, Vec2)> = Vec::new();
    let mut corner_of: HashMap<(u32, [u32; 2]), u32> = HashMap::new();
    let mut faces = Vec::with_capacity(tris.len());
    let mut by_surface: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for t in tris {
        let mut face = Face::default();
        for k in 0..3 {
            let key = [bits(t.p[k].x), bits(t.p[k].y), bits(t.p[k].z)];
            let point = *point_of.entry(key).or_insert_with(|| {
                points.push(t.p[k]);
                (points.len() - 1) as u32
            });
            let ckey = (point, [bits(t.uv[k].x), bits(t.uv[k].y)]);
            let vmref = *corner_of.entry(ckey).or_insert_with(|| {
                corners.push((point, t.uv[k]));
                (corners.len() - 1) as u32
            });
            face.v[k] = FaceVert { point, vmref };
        }
        by_surface.entry(t.surface).or_default().push(faces.len() as u32);
        faces.push(face);
    }
    let mut uv = VMap {
        name: "Texture".into(),
        kind: VMapKind::Uv,
        polymap: false,
        values: Vec::with_capacity(corners.len() * 2),
        vindices: Vec::with_capacity(corners.len()),
        pindices: Vec::new(),
    };
    let mut vmrefs = Vec::with_capacity(corners.len());
    for (i, (point, c)) in corners.iter().enumerate() {
        uv.values.extend([c.x, c.y]);
        uv.vindices.push(*point);
        vmrefs.push(std::iter::once(VMapPt { vmap: 0, index: i as u32 }).collect());
    }
    let mut mesh = Mesh {
        name: name.into(),
        points: Arc::from(points),
        smooth_groups: Arc::from(vec![0b111u32; faces.len()]),
        faces: Arc::from(faces),
        vmaps: vec![uv],
        vmrefs,
        surf_faces: by_surface
            .into_iter()
            .map(|(surface, faces)| SurfFaces { surface, faces: Arc::from(faces) })
            .collect(),
        ..Mesh::default()
    };
    mesh.rebuild_vmaps().map_err(|e| e.to_string())?;
    mesh.recompute_bbox();
    Ok(mesh)
}

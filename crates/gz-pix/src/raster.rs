//! Triangles in space drawn on the processor: a camera, a depth buffer, flat light, textures.
//!
//! What a preview without a window is made of: a model, a level or a scene as triangles in the
//! game's space (metres, +Y up, left-handed), a [`Camera`] that looks at them from a side or
//! from a point, and a [`Raster`] that draws them into a [`Canvas`]. The same input gives the
//! same bytes on every machine: one thread, no graphics card, no floating point left to a
//! driver.

use glam::{Vec2, Vec3};

use crate::{Canvas, Rgba};

/// A picture a surface wears: RGBA rows from the top, sampled without filtering and repeated.
#[derive(Clone, Debug)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<[u8]>,
}

impl Texture {
    /// A texture of `width * height * 4` bytes; `None` when the bytes are not that many or a
    /// side is zero.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && rgba.len() == width as usize * height as usize * 4).then(|| Self {
            width,
            height,
            rgba: rgba.into(),
        })
    }

    fn sample(&self, uv: Vec2) -> Rgba {
        let wrap = |v: f32, n: u32| -> usize {
            let f = v - v.floor();
            ((f * n as f32) as u32).min(n - 1) as usize
        };
        let (x, y) = (wrap(uv.x, self.width), wrap(uv.y, self.height));
        let i = (y * self.width as usize + x) * 4;
        match self.rgba.get(i..i + 4) {
            Some(p) => Rgba(p[0], p[1], p[2], p[3]),
            None => Rgba::WHITE,
        }
    }

    /// The colour of the whole picture: the mean of its pixels, weighted by their opacity.
    #[must_use]
    pub fn mean(&self) -> Rgba {
        let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
        for p in self.rgba.as_chunks::<4>().0 {
            let w = u64::from(p[3]);
            r += u64::from(p[0]) * w;
            g += u64::from(p[1]) * w;
            b += u64::from(p[2]) * w;
            a += w;
        }
        if a == 0 {
            return Rgba::rgb(128, 128, 128);
        }
        Rgba::rgb((r / a) as u8, (g / a) as u8, (b / a) as u8)
    }
}

/// How a camera maps depth onto the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lens {
    /// Flat and to scale: `px_per_m` pixels a metre.
    Ortho { px_per_m: f32 },
    /// As an eye sees: the vertical field of view in degrees.
    Perspective { fov: f32 },
}

/// Where a picture is taken from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    /// Unit vectors: where it looks, its right and its up.
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub lens: Lens,
    /// Nothing nearer is drawn, metres.
    pub near: f32,
}

/// The six views along the axes and the one from above and aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Iso,
}

impl View {
    /// `top`, `bottom`, `front`, `back`, `left`, `right`, `iso`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "front" => Self::Front,
            "back" => Self::Back,
            "left" => Self::Left,
            "right" => Self::Right,
            "iso" => Self::Iso,
            _ => return None,
        })
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
            Self::Iso => "iso",
        }
    }

    /// Where the view looks and what is up in it. Front looks along +Z (the side a model shows
    /// at heading 0 is its -Z side), right looks along -X: the views name the side seen. On top
    /// +Z is up and +X right, as on a map.
    #[must_use]
    pub fn axes(self) -> (Vec3, Vec3) {
        match self {
            Self::Top => (Vec3::NEG_Y, Vec3::Z),
            Self::Bottom => (Vec3::Y, Vec3::NEG_Z),
            Self::Front => (Vec3::Z, Vec3::Y),
            Self::Back => (Vec3::NEG_Z, Vec3::Y),
            Self::Left => (Vec3::X, Vec3::Y),
            Self::Right => (Vec3::NEG_X, Vec3::Y),
            Self::Iso => (direction(45.0, 30.0), Vec3::Y),
        }
    }
}

/// The unit vector that looks `heading` degrees round the vertical (from +Z towards +X) and
/// `pitch` degrees down.
#[must_use]
pub fn direction(heading: f32, pitch: f32) -> Vec3 {
    let (h, p) = (heading.to_radians(), pitch.to_radians());
    Vec3::new(h.sin() * p.cos(), -p.sin(), h.cos() * p.cos())
}

impl Camera {
    /// A camera at `eye` looking along `forward`, `up_hint` roughly up.
    #[must_use]
    pub fn new(eye: Vec3, forward: Vec3, up_hint: Vec3, lens: Lens) -> Self {
        let forward = forward.try_normalize().unwrap_or(Vec3::Z);
        // left-handed: right is up cross forward
        let mut right = up_hint.cross(forward);
        if right.length_squared() < 1e-8 {
            right = Vec3::Z.cross(forward);
        }
        if right.length_squared() < 1e-8 {
            right = Vec3::X;
        }
        let right = right.normalize();
        let up = forward.cross(right).normalize();
        Self { eye, forward, right, up, lens, near: 0.05 }
    }

    /// A camera that looks at `target` from `eye` as an eye does.
    #[must_use]
    pub fn look(eye: Vec3, target: Vec3, fov: f32) -> Self {
        Self::new(eye, target - eye, Vec3::Y, Lens::Perspective { fov: fov.clamp(5.0, 150.0) })
    }

    /// A flat camera that looks along `forward` and holds the box `min..max` in a picture of
    /// `width` x `height` pixels with `margin` pixels left free on every side.
    #[must_use]
    pub fn frame(
        min: Vec3,
        max: Vec3,
        forward: Vec3,
        up_hint: Vec3,
        width: u32,
        height: u32,
        margin: u32,
    ) -> Self {
        let centre = (min + max) * 0.5;
        let mut cam = Self::new(centre, forward, up_hint, Lens::Ortho { px_per_m: 1.0 });
        let (mut half_x, mut half_y, mut depth) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            ) - centre;
            half_x = half_x.max(c.dot(cam.right).abs());
            half_y = half_y.max(c.dot(cam.up).abs());
            depth = depth.max(c.dot(cam.forward).abs());
        }
        let free = |side: u32| (side.saturating_sub(margin * 2)).max(1) as f32;
        let scale = (free(width) / (half_x * 2.0).max(1e-4)).min(free(height) / (half_y * 2.0).max(1e-4));
        cam.lens = Lens::Ortho { px_per_m: scale };
        cam.eye = centre - cam.forward * (depth + 1.0);
        cam
    }

    /// A point of space in a picture of `width` x `height`: its pixel and its depth along the
    /// view. `None` behind the near plane.
    #[must_use]
    pub fn project(&self, p: Vec3, width: u32, height: u32) -> Option<(f32, f32, f32)> {
        let d = p - self.eye;
        let z = d.dot(self.forward);
        if z < self.near {
            return None;
        }
        let scale = self.scale(z, height);
        let (cx, cy) = (width as f32 * 0.5, height as f32 * 0.5);
        Some((cx + d.dot(self.right) * scale, cy - d.dot(self.up) * scale, z))
    }

    /// Pixels a metre at the depth `z`.
    fn scale(&self, z: f32, height: u32) -> f32 {
        match self.lens {
            Lens::Ortho { px_per_m } => px_per_m,
            Lens::Perspective { fov } => height as f32 * 0.5 / (fov.to_radians() * 0.5).tan() / z,
        }
    }

    /// Pixels a metre of a flat camera; `None` for one that sees as an eye does.
    #[must_use]
    pub fn px_per_m(&self) -> Option<f32> {
        match self.lens {
            Lens::Ortho { px_per_m } => Some(px_per_m),
            Lens::Perspective { .. } => None,
        }
    }
}

/// How a run of triangles is painted.
#[derive(Clone, Copy, Debug)]
pub struct Paint<'a> {
    /// The colour of an untextured face, and the tint of a textured one where `tint` is set.
    pub colour: Rgba,
    pub texture: Option<&'a Texture>,
    /// Faces are drawn from behind too.
    pub two_sided: bool,
    /// A texel less than half opaque is a hole.
    pub cutout: bool,
    /// Drawn as lit by the picture's light; unlit otherwise.
    pub lit: bool,
}

impl Paint<'_> {
    /// A lit, one-sided face of one colour.
    #[must_use]
    pub const fn flat(colour: Rgba) -> Self {
        Self { colour, texture: None, two_sided: false, cutout: false, lit: true }
    }
}

/// One corner of a triangle: where it is and its place in the texture.
pub type Corner = (Vec3, Vec2);

/// A picture being drawn: colours and the depth of every pixel.
pub struct Raster {
    canvas: Canvas,
    depth: Vec<f32>,
    /// Where the light comes from (towards the light), unit.
    pub light: Vec3,
    /// What a face turned away from the light keeps, 0 to 1.
    pub ambient: f32,
}

const FAR: f32 = f32::MAX;

impl Raster {
    #[must_use]
    pub fn new(width: u32, height: u32, background: Rgba) -> Self {
        let canvas = Canvas::new(width, height, background);
        let n = canvas.width() as usize * canvas.height() as usize;
        Self { canvas, depth: vec![FAR; n], light: Vec3::new(-0.45, 0.8, -0.4).normalize(), ambient: 0.42 }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.canvas.width()
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.canvas.height()
    }

    /// The depth of a pixel: `None` where nothing was drawn.
    #[must_use]
    pub fn depth_at(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let d = self.depth[y as usize * self.width() as usize + x as usize];
        (d < FAR).then_some(d)
    }

    /// Draws triangles: three corners each, in the game's winding (a face looks towards whoever
    /// sees its corners turn clockwise).
    pub fn triangles(&mut self, cam: &Camera, corners: &[Corner], paint: &Paint<'_>) {
        for t in corners.as_chunks::<3>().0 {
            self.triangle(cam, *t, paint);
        }
    }

    fn triangle(&mut self, cam: &Camera, t: [Corner; 3], paint: &Paint<'_>) {
        let (a, b, c) = (t[0].0, t[1].0, t[2].0);
        if !(a.is_finite() && b.is_finite() && c.is_finite()) {
            return;
        }
        let normal = (b - a).cross(c - a);
        let len = normal.length();
        if len < 1e-12 {
            return;
        }
        let normal = normal / len;
        let to_eye = match cam.lens {
            Lens::Ortho { .. } => -cam.forward,
            Lens::Perspective { .. } => cam.eye - a,
        };
        let facing = normal.dot(to_eye) > 0.0;
        if !facing && !paint.two_sided {
            return;
        }
        let n = if facing { normal } else { -normal };
        let light =
            if paint.lit { self.ambient + (1.0 - self.ambient) * n.dot(self.light).max(0.0) } else { 1.0 };
        // behind the near plane: the part in front is drawn
        let depth = |p: Vec3| (p - cam.eye).dot(cam.forward);
        let mut poly: Vec<Corner> = Vec::with_capacity(4);
        for i in 0..3 {
            let (p, q) = (t[i], t[(i + 1) % 3]);
            let (dp, dq) = (depth(p.0), depth(q.0));
            if dp >= cam.near {
                poly.push(p);
            }
            if (dp >= cam.near) != (dq >= cam.near) {
                let k = (cam.near - dp) / (dq - dp);
                poly.push((p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k));
            }
        }
        for i in 1..poly.len().saturating_sub(1) {
            self.fill(cam, [poly[0], poly[i], poly[i + 1]], paint, light);
        }
    }

    fn fill(&mut self, cam: &Camera, t: [Corner; 3], paint: &Paint<'_>, light: f32) {
        let (w, h) = (self.width(), self.height());
        // the corners are in front of the near plane or on it (a clipped one, give or take a rounding)
        let s = [0, 1, 2].map(|i| {
            let d = t[i].0 - cam.eye;
            let z = d.dot(cam.forward).max(cam.near * 0.5);
            let scale = cam.scale(z, h);
            (w as f32 * 0.5 + d.dot(cam.right) * scale, h as f32 * 0.5 - d.dot(cam.up) * scale, z)
        });
        let (x0, y0, x1, y1) = (
            s.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0),
            s.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0),
            s.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(w as f32 - 1.0),
            s.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(h as f32 - 1.0),
        );
        if x0 > x1 || y0 > y1 {
            return;
        }
        let edge = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| {
            (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
        };
        let p2 = |i: usize| (s[i].0, s[i].1);
        let area = edge(p2(0), p2(1), p2(2));
        if area.abs() < 1e-9 {
            return;
        }
        // perspective: what is interpolated across the picture is over depth
        let persp = matches!(cam.lens, Lens::Perspective { .. });
        let inv = [0, 1, 2].map(|i| if persp { 1.0 / s[i].2 } else { 1.0 });
        let shade = |c: Rgba| {
            let f = |v: u8| (f32::from(v) * light).clamp(0.0, 255.0) as u8;
            Rgba(f(c.0), f(c.1), f(c.2), 255)
        };
        let flat = shade(paint.colour);
        let (ix0, iy0, ix1, iy1) = (x0 as u32, y0 as u32, x1 as u32, y1 as u32);
        for y in iy0..=iy1 {
            for x in ix0..=ix1 {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let (w0, w1, w2) = (
                    edge(p2(1), p2(2), p) / area,
                    edge(p2(2), p2(0), p) / area,
                    edge(p2(0), p2(1), p) / area,
                );
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let k = w0 * inv[0] + w1 * inv[1] + w2 * inv[2];
                let z = if persp { 1.0 / k } else { w0 * s[0].2 + w1 * s[1].2 + w2 * s[2].2 };
                let i = y as usize * w as usize + x as usize;
                if z >= self.depth[i] {
                    continue;
                }
                let colour = match paint.texture {
                    None => flat,
                    Some(tex) => {
                        let uv =
                            (t[0].1 * (w0 * inv[0]) + t[1].1 * (w1 * inv[1]) + t[2].1 * (w2 * inv[2])) / k;
                        let texel = tex.sample(uv);
                        if paint.cutout && texel.3 < 128 {
                            continue;
                        }
                        shade(texel)
                    }
                };
                self.depth[i] = z;
                self.canvas.plot(i64::from(x), i64::from(y), colour);
            }
        }
    }

    /// A line of space, drawn over whatever is behind it and hidden by what is in front.
    pub fn line(&mut self, cam: &Camera, from: Vec3, to: Vec3, colour: Rgba) {
        let (w, h) = (self.width(), self.height());
        let (Some(a), Some(b)) = (cam.project(from, w, h), cam.project(to, w, h)) else { return };
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as u32).clamp(1, 16_384);
        for i in 0..=steps {
            let k = i as f32 / steps as f32;
            let (x, y, z) = (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k);
            if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                continue;
            }
            let at = y as usize * w as usize + x as usize;
            // a little towards the eye, so a line lying on a face shows
            if z - 0.02 <= self.depth[at] {
                self.canvas.plot(x as i64, y as i64, colour);
            }
        }
    }

    /// Darkens the pixels where the depth jumps: the outlines of what stands in front of
    /// something else, so shapes of one colour still read.
    pub fn outline(&mut self, colour: Rgba) {
        let (w, h) = (self.width() as usize, self.height() as usize);
        let mut edges = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let d = self.depth[y * w + x];
                let mut jump = false;
                for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                    if nx >= w || ny >= h {
                        continue;
                    }
                    let e = self.depth[ny * w + nx];
                    let near = d.min(e);
                    if near < FAR && (d - e).abs() > (near * 0.02).max(0.05) {
                        jump = true;
                    }
                }
                if jump {
                    edges.push((x, y));
                }
            }
        }
        for (x, y) in edges {
            self.canvas.plot(x as i64, y as i64, colour);
        }
    }

    /// The picture, for labels and a frame to be drawn over it.
    pub fn canvas_mut(&mut self) -> &mut Canvas {
        &mut self.canvas
    }

    #[must_use]
    pub fn into_canvas(self) -> Canvas {
        self.canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube centred at `c`, faces looking out, one texture square a face.
    fn cube(c: Vec3) -> Vec<Corner> {
        let p = |x: f32, y: f32, z: f32| c + Vec3::new(x, y, z) * 0.5;
        let quads = [
            [p(-1., 1., -1.), p(-1., 1., 1.), p(1., 1., 1.), p(1., 1., -1.)], // +y
            [p(-1., -1., -1.), p(1., -1., -1.), p(1., -1., 1.), p(-1., -1., 1.)], // -y
            [p(1., -1., -1.), p(1., 1., -1.), p(1., 1., 1.), p(1., -1., 1.)], // +x
            [p(-1., -1., -1.), p(-1., -1., 1.), p(-1., 1., 1.), p(-1., 1., -1.)], // -x
            [p(-1., -1., 1.), p(1., -1., 1.), p(1., 1., 1.), p(-1., 1., 1.)], // +z
            [p(-1., -1., -1.), p(-1., 1., -1.), p(1., 1., -1.), p(1., -1., -1.)], // -z
        ];
        let uv = [Vec2::new(0., 0.), Vec2::new(0., 1.), Vec2::new(1., 1.), Vec2::new(1., 0.)];
        let mut out = Vec::new();
        for q in quads {
            for i in [0, 1, 2, 0, 2, 3] {
                out.push((q[i], uv[i]));
            }
        }
        out
    }

    #[test]
    fn the_faces_of_the_test_cube_look_out() {
        for t in cube(Vec3::ZERO).as_chunks::<3>().0 {
            let n = (t[1].0 - t[0].0).cross(t[2].0 - t[0].0);
            let centre = (t[0].0 + t[1].0 + t[2].0) / 3.0;
            assert!(n.dot(centre) > 0.0, "{t:?}");
        }
    }

    #[test]
    fn a_flat_view_is_to_scale_and_frames_what_it_is_given() {
        let (min, max) = (Vec3::splat(-0.5), Vec3::splat(0.5));
        let (fwd, up) = View::Top.axes();
        let cam = Camera::frame(min, max, fwd, up, 120, 120, 10);
        assert!((cam.px_per_m().unwrap() - 100.0).abs() < 1e-3);
        let mut r = Raster::new(120, 120, Rgba::BLACK);
        r.triangles(&cam, &cube(Vec3::ZERO), &Paint::flat(Rgba::WHITE));
        let c = r.into_canvas();
        // the cube fills the middle hundred pixels and nothing of the margin
        assert_ne!(c.pixel(60, 60), Some(Rgba::BLACK));
        assert_ne!(c.pixel(12, 12), Some(Rgba::BLACK));
        assert_eq!(c.pixel(5, 60), Some(Rgba::BLACK));
        assert_eq!(c.pixel(60, 114), Some(Rgba::BLACK));
        // on top +x is right and +z is up
        let (x, y, _) = cam.project(Vec3::new(0.5, 0.5, 0.5), 120, 120).unwrap();
        assert!((x - 110.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "{x} {y}");
    }

    #[test]
    fn what_is_nearer_hides_what_is_farther_whatever_the_order() {
        let near = cube(Vec3::new(0.0, 0.0, -2.0));
        let far = cube(Vec3::new(0.0, 0.0, 2.0));
        let (fwd, up) = View::Front.axes();
        let cam = Camera::frame(Vec3::splat(-3.0), Vec3::splat(3.0), fwd, up, 64, 64, 0);
        let red = Paint { lit: false, ..Paint::flat(Rgba::rgb(255, 0, 0)) };
        let blue = Paint { lit: false, ..Paint::flat(Rgba::rgb(0, 0, 255)) };
        for order in [true, false] {
            let mut r = Raster::new(64, 64, Rgba::BLACK);
            if order {
                r.triangles(&cam, &near, &red);
                r.triangles(&cam, &far, &blue);
            } else {
                r.triangles(&cam, &far, &blue);
                r.triangles(&cam, &near, &red);
            }
            assert_eq!(r.into_canvas().pixel(32, 32), Some(Rgba::rgb(255, 0, 0)));
        }
    }

    #[test]
    fn a_texture_is_worn_and_a_cutout_leaves_a_hole() {
        // left half opaque green, right half clear
        let tex = Texture::new(2, 1, vec![0, 255, 0, 255, 0, 0, 0, 0]).unwrap();
        let (fwd, up) = View::Top.axes();
        let cam = Camera::frame(Vec3::splat(-0.5), Vec3::splat(0.5), fwd, up, 100, 100, 0);
        let paint = Paint { texture: Some(&tex), cutout: true, lit: false, ..Paint::flat(Rgba::WHITE) };
        let mut r = Raster::new(100, 100, Rgba::BLACK);
        r.triangles(&cam, &cube(Vec3::ZERO), &paint);
        let c = r.into_canvas();
        let (mut green, mut hole) = (0, 0);
        for x in 0..100 {
            match c.pixel(x, 50) {
                Some(Rgba(0, 255, 0, 255)) => green += 1,
                Some(Rgba(0, 0, 0, 255)) => hole += 1,
                other => panic!("{other:?}"),
            }
        }
        assert!(green > 40 && hole > 40, "{green} {hole}");
        assert_eq!(tex.mean(), Rgba::rgb(0, 255, 0));
    }

    #[test]
    fn an_eye_sees_the_near_side_larger_and_nothing_behind_it() {
        let cam = Camera::look(Vec3::new(0.0, 0.0, -3.0), Vec3::ZERO, 60.0);
        let near = cam.project(Vec3::new(0.5, 0.0, -0.5), 200, 200).unwrap();
        let far = cam.project(Vec3::new(0.5, 0.0, 0.5), 200, 200).unwrap();
        assert!(near.0 > far.0 && far.0 > 100.0);
        assert!(cam.project(Vec3::new(0.0, 0.0, -5.0), 200, 200).is_none());
        // a triangle that crosses the eye's plane is drawn where it is in front, with no panic
        let mut r = Raster::new(64, 64, Rgba::BLACK);
        let big = [
            (Vec3::new(-50.0, -1.0, -50.0), Vec2::ZERO),
            (Vec3::new(0.0, -1.0, 50.0), Vec2::ZERO),
            (Vec3::new(50.0, -1.0, -50.0), Vec2::ZERO),
        ];
        r.triangles(&cam, &big, &Paint { two_sided: true, ..Paint::flat(Rgba::WHITE) });
        assert_ne!(r.into_canvas().pixel(32, 60), Some(Rgba::BLACK));
    }

    #[test]
    fn the_same_input_is_the_same_picture() {
        let draw = || {
            let cam = Camera::look(Vec3::new(2.0, 1.5, -3.0), Vec3::ZERO, 50.0);
            let mut r = Raster::new(96, 64, Rgba::rgb(20, 20, 24));
            r.triangles(&cam, &cube(Vec3::ZERO), &Paint::flat(Rgba::rgb(200, 180, 120)));
            r.outline(Rgba::BLACK);
            r.into_canvas().png().unwrap()
        };
        assert_eq!(draw(), draw());
    }
}

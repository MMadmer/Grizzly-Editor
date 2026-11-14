//! Camera maths: projections, picking rays, projecting points, framing a selection.
//!
//! The engine's coordinates: left-handed, Y up, a camera looks along its forward axis. Depth is
//! reversed - 1 at the near plane, 0 at the far one - for its precision far away.

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::math::{Aabb, Frustum, Ray};

/// How the camera maps the world onto the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// `fov_y` is the vertical field of view in radians.
    Perspective { fov_y: f32, near: f32, far: f32 },
    /// `height` is how many metres the picture shows from its bottom to its top.
    Orthographic { height: f32, near: f32, far: f32 },
}

impl Default for Projection {
    /// The editor's camera: 60 degrees, 0.2 m to 1500 m (field of view, near and far plane).
    fn default() -> Self {
        Self::Perspective { fov_y: 60f32.to_radians(), near: 0.2, far: 1500.0 }
    }
}

impl Projection {
    #[must_use]
    pub fn near(&self) -> f32 {
        match *self {
            Self::Perspective { near, .. } | Self::Orthographic { near, .. } => near,
        }
    }

    #[must_use]
    pub fn far(&self) -> f32 {
        match *self {
            Self::Perspective { far, .. } | Self::Orthographic { far, .. } => far,
        }
    }

    #[must_use]
    pub fn is_ortho(&self) -> bool {
        matches!(self, Self::Orthographic { .. })
    }

    /// The projection matrix for a picture `aspect` (width / height) wide: left-handed, depth 1
    /// at the near plane and 0 at the far one.
    #[must_use]
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
        match *self {
            Self::Perspective { fov_y, near, far } => {
                let (n, f) = sane_range(near, far);
                let h = 1.0 / (fov_y.clamp(1e-3, 3.1) * 0.5).tan();
                let w = h / aspect;
                let b = n * f / (f - n);
                let a = -n / (f - n);
                Mat4::from_cols(
                    Vec4::new(w, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, h, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, a, 1.0),
                    Vec4::new(0.0, 0.0, b, 0.0),
                )
            }
            Self::Orthographic { height, near, far } => {
                let (n, f) = sane_ortho_range(near, far);
                let height = height.max(1e-4);
                let sy = 2.0 / height;
                let sx = sy / aspect;
                let a = -1.0 / (f - n);
                let b = f / (f - n);
                Mat4::from_cols(
                    Vec4::new(sx, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, sy, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, a, 0.0),
                    Vec4::new(0.0, 0.0, b, 1.0),
                )
            }
        }
    }
}

fn sane_range(near: f32, far: f32) -> (f32, f32) {
    let n = if near.is_finite() && near > 0.0 { near } else { 0.2 };
    let f = if far.is_finite() && far > n * 1.001 { far } else { n * 1000.0 };
    (n, f)
}

fn sane_ortho_range(near: f32, far: f32) -> (f32, f32) {
    let n = if near.is_finite() { near } else { 0.0 };
    let f = if far.is_finite() && far > n + 1e-3 { far } else { n + 1000.0 };
    (n, f)
}

/// Where the camera stands and where it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    /// Unit length.
    pub forward: Vec3,
    /// Unit length, at right angles to `forward`.
    pub up: Vec3,
    pub projection: Projection,
}

impl Default for Camera {
    fn default() -> Self {
        Self::look_at(Vec3::new(0.0, 5.0, -10.0), Vec3::ZERO)
    }
}

impl Camera {
    /// A camera at `position` looking at `target`, the world's Y up.
    #[must_use]
    pub fn look_at(position: Vec3, target: Vec3) -> Self {
        let forward = (target - position).try_normalize().unwrap_or(Vec3::Z);
        Self::looking(position, forward)
    }

    /// A camera at `position` looking along `forward`, kept upright.
    #[must_use]
    pub fn looking(position: Vec3, forward: Vec3) -> Self {
        let forward = forward.try_normalize().unwrap_or(Vec3::Z);
        let world_up = if forward.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
        let right = world_up.cross(forward).normalize();
        let up = forward.cross(right).normalize();
        Self { position, forward, up, projection: Projection::default() }
    }

    /// The engine's camera, set from a heading and a pitch with no bank: forward is the matrix's `k` row, up its
    /// `j` row. Heading 0 and pitch 0 look along +Z.
    #[must_use]
    pub fn from_hpb(position: Vec3, heading: f32, pitch: f32) -> Self {
        let (sh, ch) = heading.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let forward = Vec3::new(-cp * sh, sp, cp * ch);
        let up = Vec3::new(sp * sh, cp, -sp * ch);
        Self { position, forward, up, projection: Projection::default() }
    }

    #[must_use]
    pub fn with_projection(mut self, projection: Projection) -> Self {
        self.projection = projection;
        self
    }

    /// The camera's right: `up x forward` in a left-handed world.
    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.up.cross(self.forward).try_normalize().unwrap_or(Vec3::X)
    }

    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        // rows: right, up, forward; then the position brought to the origin
        let (r, u, f, p) = (self.right(), self.up, self.forward, self.position);
        Mat4::from_cols(
            Vec4::new(r.x, u.x, f.x, 0.0),
            Vec4::new(r.y, u.y, f.y, 0.0),
            Vec4::new(r.z, u.z, f.z, 0.0),
            Vec4::new(-r.dot(p), -u.dot(p), -f.dot(p), 1.0),
        )
    }

    #[must_use]
    pub fn proj_matrix(&self, size: [u32; 2]) -> Mat4 {
        self.projection.matrix(aspect(size))
    }

    /// `clip = view_proj * world`.
    #[must_use]
    pub fn view_proj(&self, size: [u32; 2]) -> Mat4 {
        self.proj_matrix(size) * self.view_matrix()
    }

    #[must_use]
    pub fn frustum(&self, size: [u32; 2]) -> Frustum {
        Frustum::from_view_proj(&self.view_proj(size))
    }
}

/// Width over height; 1 for a picture of no size.
#[must_use]
pub fn aspect(size: [u32; 2]) -> f32 {
    if size[0] == 0 || size[1] == 0 { 1.0 } else { size[0] as f32 / size[1] as f32 }
}

/// The ray under a point of the picture (pixels from the top-left corner; a pixel's centre is
/// at `+0.5`). `None` for a picture of no size.
#[must_use]
pub fn pick_ray(camera: &Camera, size: [u32; 2], pixel: Vec2) -> Option<Ray> {
    if size[0] == 0 || size[1] == 0 {
        return None;
    }
    let inv = camera.view_proj(size).inverse();
    let ndc = Vec2::new(pixel.x / size[0] as f32 * 2.0 - 1.0, 1.0 - pixel.y / size[1] as f32 * 2.0);
    let near = inv * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
    let far = inv * Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
    if near.w.abs() < 1e-12 || far.w.abs() < 1e-12 {
        return None;
    }
    let (a, b) = (near.truncate() / near.w, far.truncate() / far.w);
    let dir = (b - a).try_normalize()?;
    let origin = if camera.projection.is_ortho() { a } else { camera.position };
    Some(Ray { origin, dir })
}

/// Where a point lands in the picture: pixels from the top-left corner and the depth (1 at the
/// near plane). `None` behind the camera.
#[must_use]
pub fn project(view_proj: &Mat4, size: [u32; 2], point: Vec3) -> Option<Vec3> {
    let clip = *view_proj * point.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(Vec3::new((ndc.x + 1.0) * 0.5 * size[0] as f32, (1.0 - ndc.y) * 0.5 * size[1] as f32, ndc.z))
}

/// The camera moved back along its own forward axis until `bounds` fills the picture
/// (fitted against both fields of view). An orthographic camera also
/// gets the height that holds the box.
#[must_use]
pub fn frame_fit(camera: &Camera, bounds: &Aabb, size: [u32; 2]) -> Camera {
    if bounds.is_empty() {
        return *camera;
    }
    let (center, radius) = bounds.sphere();
    let radius = radius.max(0.05);
    let aspect = aspect(size);
    let mut out = *camera;
    match camera.projection {
        Projection::Perspective { fov_y, near, .. } => {
            let half_y = (fov_y * 0.5).clamp(1e-3, 1.55);
            let half_x = (half_y.tan() * aspect).atan();
            let dist = (radius / half_y.min(half_x).sin()).max(near + radius);
            out.position = center - camera.forward * dist;
        }
        Projection::Orthographic { near, far, .. } => {
            let height = 2.0 * radius * (1.0 / aspect).max(1.0);
            out.projection = Projection::Orthographic { height, near, far };
            out.position = center - camera.forward * (radius + near.max(0.0) + 1.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, eps: f32) -> bool {
        (a - b).abs().max_element() < eps
    }

    #[test]
    fn depth_is_reversed() {
        let cam = Camera::looking(Vec3::ZERO, Vec3::Z);
        let vp = cam.view_proj([800, 600]);
        let near = project(&vp, [800, 600], Vec3::new(0.0, 0.0, 0.2)).unwrap();
        let far = project(&vp, [800, 600], Vec3::new(0.0, 0.0, 1500.0)).unwrap();
        assert!((near.z - 1.0).abs() < 1e-4, "{near}");
        assert!(far.z.abs() < 1e-4, "{far}");
        assert!(close(near, Vec3::new(400.0, 300.0, near.z), 1e-3));
        assert!(project(&vp, [800, 600], Vec3::new(0.0, 0.0, -1.0)).is_none());
        // left-handed: +X is to the right, +Y up
        let right = project(&vp, [800, 600], Vec3::new(1.0, 1.0, 10.0)).unwrap();
        assert!(right.x > 400.0 && right.y < 300.0);
    }

    #[test]
    fn hpb_matches_the_engine() {
        let cam = Camera::from_hpb(Vec3::ZERO, 0.0, 0.0);
        assert!(close(cam.forward, Vec3::Z, 1e-6) && close(cam.up, Vec3::Y, 1e-6));
        assert!(close(cam.right(), Vec3::X, 1e-6));
        let turned = Camera::from_hpb(Vec3::ZERO, std::f32::consts::FRAC_PI_2, 0.0);
        assert!(close(turned.forward, -Vec3::X, 1e-6));
        let up = Camera::from_hpb(Vec3::ZERO, 0.0, 0.5);
        assert!(up.forward.y > 0.0 && up.forward.dot(up.up).abs() < 1e-6);
    }

    #[test]
    fn a_picking_ray_goes_through_its_pixel() {
        let cam = Camera::look_at(Vec3::new(3.0, 4.0, -6.0), Vec3::new(0.0, 1.0, 0.0));
        let size = [1280, 720];
        let vp = cam.view_proj(size);
        for target in [Vec3::new(0.0, 1.0, 0.0), Vec3::new(2.0, -1.0, 5.0), Vec3::new(-4.0, 3.0, 2.0)] {
            let px = project(&vp, size, target).unwrap();
            let ray = pick_ray(&cam, size, px.truncate()).unwrap();
            let to = (target - ray.origin).normalize();
            assert!(close(ray.dir, to, 1e-3), "{} vs {}", ray.dir, to);
            assert!(close(ray.origin, cam.position, 1e-4));
        }
        let centre = pick_ray(&cam, size, Vec2::new(640.0, 360.0)).unwrap();
        assert!(close(centre.dir, cam.forward, 1e-4));
        assert!(pick_ray(&cam, [0, 10], Vec2::ZERO).is_none());

        let ortho = cam.with_projection(Projection::Orthographic { height: 10.0, near: 0.0, far: 100.0 });
        let a = pick_ray(&ortho, size, Vec2::new(0.0, 0.0)).unwrap();
        let b = pick_ray(&ortho, size, Vec2::new(1280.0, 720.0)).unwrap();
        assert!(close(a.dir, ortho.forward, 1e-4) && close(b.dir, ortho.forward, 1e-4));
        assert!(((a.origin - b.origin).dot(ortho.up).abs() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn framing_holds_the_box() {
        let cam = Camera::look_at(Vec3::new(10.0, 10.0, -10.0), Vec3::ZERO);
        let bounds = Aabb::new(Vec3::new(50.0, 0.0, 50.0), Vec3::new(54.0, 3.0, 52.0));
        for size in [[1920, 1080], [400, 900]] {
            let fit = frame_fit(&cam, &bounds, size);
            assert!(close(fit.forward, cam.forward, 1e-6));
            let vp = fit.view_proj(size);
            let c = project(&vp, size, bounds.center()).unwrap();
            assert!((c.x - size[0] as f32 * 0.5).abs() < 0.5 && (c.y - size[1] as f32 * 0.5).abs() < 0.5);
            let f = fit.frustum(size);
            for i in 0..8u32 {
                let corner = Vec3::new(
                    if i & 1 == 0 { bounds.min.x } else { bounds.max.x },
                    if i & 2 == 0 { bounds.min.y } else { bounds.max.y },
                    if i & 4 == 0 { bounds.min.z } else { bounds.max.z },
                );
                assert!(f.contains_point(corner), "{corner} is out of the {size:?} picture");
            }
        }
        assert_eq!(frame_fit(&cam, &Aabb::EMPTY, [10, 10]), cam);
    }

    #[test]
    fn the_frustum_culls() {
        let cam = Camera::looking(Vec3::ZERO, Vec3::Z);
        let f = cam.frustum([100, 100]);
        assert!(f.intersects_aabb(&Aabb::new(Vec3::new(-1.0, -1.0, 9.0), Vec3::new(1.0, 1.0, 11.0))));
        assert!(!f.intersects_aabb(&Aabb::new(Vec3::new(-1.0, -1.0, -11.0), Vec3::new(1.0, 1.0, -9.0))));
        assert!(!f.intersects_aabb(&Aabb::new(Vec3::new(100.0, -1.0, 9.0), Vec3::new(102.0, 1.0, 11.0))));
        assert!(!f.intersects_aabb(&Aabb::new(Vec3::new(-1.0, -1.0, 2000.0), Vec3::new(1.0, 1.0, 2001.0))));
        assert!(f.intersects_sphere(Vec3::new(0.0, 0.0, -0.5), 1.0));
    }
}

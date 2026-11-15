//! Pictures of the scene: `shot.viewport` as the window shows it, or from a camera of the
//! picture's own - the author's camera stays where it is, and the picture is drawn into targets
//! of its own, so nothing on the screen changes.

use std::sync::Arc;
use std::time::Instant;

use base64::Engine;
use glam::Vec3;
use gz_editor::{CmdError, Editor, Origin, Reply, Value, json};
use gz_fmt_object::Mat4 as XMat;
use gz_pix::{Canvas, Rgba};
use gz_render::{Camera, Image, Projection};
use gz_scene::math::Aabb;
use gz_scene::{SceneService, SceneState};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::{PICTURE_LIMIT, Reason, SceneOpt, Viewport};

/// The largest side of a picture with a camera of its own, px.
pub const MAX_SIDE: u32 = 2048;
/// The names drawn into one picture at most.
const MAX_LABELS: usize = 64;

/// The camera and the size of one picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotView {
    pub camera: Camera,
    pub size: [u32; 2],
    /// The scene alone: no grid, no helpers, no selection.
    pub clean: bool,
}

/// A view that frames a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// From above, flat and to scale: +X is right, +Z is up.
    Top,
    /// Looking along +Z, flat and to scale.
    Front,
    /// Looking along -Z, flat and to scale.
    Back,
    /// Looking along +X, flat and to scale.
    Left,
    /// Looking along -X, flat and to scale.
    Right,
    /// From above and aside, in perspective.
    Iso,
}

impl Side {
    fn key(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
            Self::Iso => "iso",
        }
    }
}

/// How far a flat view's camera stands off the box it frames, and how much room it leaves
/// around it.
const STAND_OFF: f32 = 5.0;
const ROOM: f32 = 1.08;

/// The camera of a view that frames `bounds` in a picture of `size`.
#[must_use]
pub fn framing(side: Side, bounds: &Aabb, size: [u32; 2]) -> Camera {
    let centre = (bounds.min + bounds.max) * 0.5;
    let d = (bounds.max - bounds.min).max(Vec3::splat(0.1));
    let aspect = gz_render::aspect(size);
    // a flat view: the camera off one face, the picture as high as the box needs either way
    let flat = |from: Vec3, forward: Vec3, across: f32, up: f32, deep: f32| {
        let height = up.max(across / aspect) * ROOM;
        Camera::looking(from, forward).with_projection(Projection::Orthographic {
            height,
            near: 0.0,
            far: deep + 2.0 * STAND_OFF,
        })
    };
    match side {
        Side::Top => {
            flat(Vec3::new(centre.x, bounds.max.y + STAND_OFF, centre.z), Vec3::NEG_Y, d.x, d.z, d.y)
        }
        Side::Front => flat(Vec3::new(centre.x, centre.y, bounds.min.z - STAND_OFF), Vec3::Z, d.x, d.y, d.z),
        Side::Back => {
            flat(Vec3::new(centre.x, centre.y, bounds.max.z + STAND_OFF), Vec3::NEG_Z, d.x, d.y, d.z)
        }
        Side::Left => flat(Vec3::new(bounds.min.x - STAND_OFF, centre.y, centre.z), Vec3::X, d.z, d.y, d.x),
        Side::Right => {
            flat(Vec3::new(bounds.max.x + STAND_OFF, centre.y, centre.z), Vec3::NEG_X, d.z, d.y, d.x)
        }
        Side::Iso => {
            let fov = 40f32.to_radians();
            let radius = d.length() * 0.5;
            // the sphere around the box fits the narrower of the picture's two angles
            let half = (fov * 0.5).min(((fov * 0.5).tan() * aspect).atan());
            let dist = radius / half.sin() * ROOM;
            let from = centre + Vec3::new(-1.0, 0.9, -1.0).normalize() * dist;
            Camera::look_at(from, centre).with_projection(Projection::Perspective {
                fov_y: fov,
                near: (dist - radius * 1.5).max(0.1),
                far: dist + radius * 1.5 + 100.0,
            })
        }
    }
}

/// A camera at a place: looking at `target`, or turned by a heading, pitch and bank in degrees
/// the way the viewport's own camera is.
#[must_use]
pub fn standing(position: Vec3, target: Option<Vec3>, rotation: Option<Vec3>, fov: f32) -> Camera {
    let base = match (target, rotation) {
        (Some(t), _) => Camera::look_at(position, t),
        (None, Some(r)) => {
            let m = XMat::hpb(r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
            Camera { position, forward: m.row(2), up: m.row(1), projection: Projection::default() }
        }
        (None, None) => Camera::looking(position, Vec3::Z),
    };
    let fov_y = fov.clamp(5.0, 150.0).to_radians();
    base.with_projection(Projection::Perspective { fov_y, near: 0.2, far: 1500.0 })
}

/// Where a point of the world is in the picture, px from the top left; `None` behind the camera
/// or outside the picture.
#[must_use]
pub fn to_pixel(camera: &Camera, size: [u32; 2], point: Vec3) -> Option<(f32, f32)> {
    let clip = camera.view_proj(size) * point.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(-1.0..=1.0).contains(&ndc.x) || !(-1.0..=1.0).contains(&ndc.y) || !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some(((ndc.x * 0.5 + 0.5) * size[0] as f32, (0.5 - ndc.y * 0.5) * size[1] as f32))
}

/// One name to draw: where its object is in the picture and how far from the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub name: String,
    pub at: (f32, f32),
    pub distance: f32,
}

/// The names of the scene's objects the camera sees, nearest first, at most [`MAX_LABELS`].
/// `bounds` gives an object's world box.
#[must_use]
pub fn labels_of(
    state: &SceneState,
    bounds: &dyn Fn(&gz_scene::Object) -> Option<Aabb>,
    camera: &Camera,
    size: [u32; 2],
) -> Vec<Label> {
    let mut out: Vec<Label> = state
        .objects
        .values()
        .filter(|o| o.flags.visible)
        .filter_map(|o| {
            let centre = bounds(o).map_or_else(|| o.position(), |b| (b.min + b.max) * 0.5);
            let at = to_pixel(camera, size, centre)?;
            Some(Label { name: o.name.to_string(), at, distance: (centre - camera.position).length() })
        })
        .collect();
    out.sort_by(|a, b| a.distance.total_cmp(&b.distance).then_with(|| a.name.cmp(&b.name)));
    out.truncate(MAX_LABELS);
    out
}

/// Draws the names into the picture, each on a plate by its object, nearest first; a name that
/// would cover one already drawn is left out. Answers the ones drawn.
pub fn draw_labels(canvas: &mut Canvas, labels: &[Label]) -> Vec<Label> {
    let scale = if canvas.height() >= 1400 { 2 } else { 1 };
    let ink = Rgba::WHITE;
    let plate = Rgba::BLACK.alpha(170);
    let mut taken: Vec<(i64, i64, i64, i64)> = Vec::new();
    let mut drawn = Vec::new();
    for l in labels {
        let (w, h) = gz_pix::text_size(&l.name, scale);
        let pad = 2i64;
        let x = (l.at.0 as i64 - i64::from(w) / 2 - pad)
            .clamp(0, (i64::from(canvas.width()) - i64::from(w) - 2 * pad).max(0));
        let y = (l.at.1 as i64 - i64::from(h) - 2 * pad - 4)
            .clamp(0, (i64::from(canvas.height()) - i64::from(h) - 2 * pad).max(0));
        let r = (x, y, x + i64::from(w) + 2 * pad, y + i64::from(h) + 2 * pad);
        if taken.iter().any(|t| r.0 < t.2 && t.0 < r.2 && r.1 < t.3 && t.1 < r.3) {
            continue;
        }
        // a dot where the object is, then its name over it
        canvas.disc(l.at, 2.5 * scale as f32, Rgba::rgb(255, 208, 64));
        canvas.label(x, y, &l.name, scale, pad as u32, ink, plate);
        taken.push(r);
        drawn.push(l.clone());
    }
    drawn
}

impl Viewport {
    /// A picture of the scene from a camera of its own, drawn into targets of its own. The pass
    /// is built again until nothing it draws is still loading (up to the picture's limit);
    /// `pending` says what still was.
    pub async fn picture_from(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        shot: ShotView,
    ) -> Result<(Image, Vec<String>), CmdError> {
        let renderer = self.renderer_any(editor).ok_or_else(|| {
            CmdError::new("no_window", "the viewport draws nothing: the editor runs without a window")
        })?;
        let deadline = Instant::now() + PICTURE_LIMIT;
        let (packet, pending) = loop {
            let packet = self
                .make_packet_from(editor, &renderer, false, Reason::Picture, Some(shot))
                .await
                .ok_or_else(|| CmdError::internal("the viewport could not draw the scene"))?;
            let pending = Self::pending_of(&renderer, &packet);
            if pending.is_empty() || Instant::now() >= deadline {
                break (packet, pending);
            }
            let generation = renderer.assets().generation();
            while renderer.assets().generation() == generation && Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let size = shot.size;
        let image = editor
            .exec
            .io(move || renderer.render_offscreen(&packet, size))
            .await
            .map_err(|e| CmdError::internal(format!("the picture could not be drawn: {e}")))?;
        Ok((image, pending))
    }
}

/// `shot.viewport`'s parameters.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShotParams {
    /// Where a camera of the picture's own stands, in metres; the author's camera stays where it
    /// is. Left out with `view` left out too: the picture is the viewport as the window has it.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// The point that camera looks at.
    #[serde(default)]
    target: Option<[f32; 3]>,
    /// Instead of `target`: heading, pitch and bank in degrees, as `viewport.camera` takes them.
    #[serde(default)]
    rotation: Option<[f32; 3]>,
    /// That camera's vertical field of view, degrees (60).
    #[serde(default)]
    fov: Option<f32>,
    /// A view that frames `objects` or `area` (neither: the whole scene): top, front, back, left,
    /// right are flat and to scale, iso is from above and aside.
    #[serde(default)]
    view: Option<Side>,
    /// The objects `view` frames.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// The ground rectangle `view` frames: `[x0, z0, x1, z1]` in metres.
    #[serde(default)]
    area: Option<[f32; 4]>,
    /// The picture's width in px (1280, at most 2048), with a camera of its own.
    #[serde(default)]
    width: Option<u32>,
    /// The picture's height in px (720, at most 2048), with a camera of its own.
    #[serde(default)]
    height: Option<u32>,
    /// The scene alone: no grid, no helpers, no selection. With a camera of its own.
    #[serde(default)]
    clean: bool,
    /// The objects' names drawn into the picture where the objects are; the answer lists them
    /// with their pixels.
    #[serde(default)]
    labels: bool,
}

fn finite3(v: [f32; 3], what: &str) -> Result<Vec3, CmdError> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(Vec3::from(v))
    } else {
        Err(CmdError::invalid(format!("{what} takes three numbers")))
    }
}

fn r(v: f32, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (f64::from(v) * k).round() / k
}

fn v3(v: Vec3, digits: i32) -> Value {
    json!([r(v.x, digits), r(v.y, digits), r(v.z, digits)])
}

/// The box a view frames: the named objects, a ground rectangle with what stands in it, or
/// everything. `ground` answers the height of the highest thing at a point of the ground.
fn frame_box(
    state: &SceneState,
    bounds: &dyn Fn(&gz_scene::Object) -> Option<Aabb>,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
    objects: Option<&[String]>,
    area: Option<[f32; 4]>,
) -> Result<Aabb, CmdError> {
    let mut acc: Option<Aabb> = None;
    let mut add = |b: Aabb| {
        acc = Some(match acc {
            Some(a) => Aabb { min: a.min.min(b.min), max: a.max.max(b.max) },
            None => b,
        });
    };
    if let Some(names) = objects {
        for name in names {
            let o = state
                .find_any(name)
                .ok_or_else(|| CmdError::not_found(format!("no object '{name}' in the scene")))?;
            let p = o.position();
            add(bounds(o).unwrap_or(Aabb { min: p - Vec3::splat(0.5), max: p + Vec3::splat(0.5) }));
        }
    } else if let Some(a) = area {
        if !a.iter().all(|x| x.is_finite()) {
            return Err(CmdError::invalid("area takes four numbers: x0, z0, x1, z1"));
        }
        let (x0, x1) = (a[0].min(a[2]), a[0].max(a[2]));
        let (z0, z1) = (a[1].min(a[3]), a[1].max(a[3]));
        // the heights of the rectangle itself: what rays from above meet across it, and the boxes
        // of what stands in it. A box that only crosses it - the level's whole ground, a hill
        // beside it - gives no height: the camera would stand off the level
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        const N: u32 = 9;
        for i in 0..N {
            for j in 0..N {
                let x = x0 + (x1 - x0) * (i as f32 + 0.5) / N as f32;
                let z = z0 + (z1 - z0) * (j as f32 + 0.5) / N as f32;
                if let Some(y) = ground(x, z) {
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
            }
        }
        for o in state.objects.values() {
            if let Some(b) = bounds(o)
                && b.min.x >= x0
                && b.max.x <= x1
                && b.min.z >= z0
                && b.max.z <= z1
            {
                lo = lo.min(b.min.y);
                hi = hi.max(b.max.y);
            }
        }
        if lo > hi {
            (lo, hi) = (0.0, 10.0);
        }
        add(Aabb { min: Vec3::new(x0, lo, z0), max: Vec3::new(x1, hi, z1) });
    } else {
        for o in state.objects.values() {
            if let Some(b) = bounds(o) {
                add(b);
            }
        }
    }
    acc.ok_or_else(|| CmdError::invalid("the scene has nothing to frame: name objects or an area"))
}

pub(crate) async fn shot_viewport(ed: Arc<Editor>, p: ShotParams, _o: Origin) -> Reply {
    let vp = Viewport::of(&ed);
    let own = p.position.is_some() || p.view.is_some();
    if !own {
        let stray = [
            ("target", p.target.is_some()),
            ("rotation", p.rotation.is_some()),
            ("fov", p.fov.is_some()),
            ("objects", p.objects.is_some()),
            ("area", p.area.is_some()),
            ("width", p.width.is_some()),
            ("height", p.height.is_some()),
            ("clean", p.clean),
        ];
        if let Some((name, _)) = stray.iter().find(|(_, given)| *given) {
            return Err(CmdError::invalid(format!(
                "{name} belongs to a camera of the picture's own: give position or view with it"
            )));
        }
    }
    if p.position.is_some() && p.view.is_some() {
        return Err(CmdError::invalid("position and view are two cameras: give one of them"));
    }
    // the scene's objects and their boxes, for a view's frame and for the names
    let scene = SceneService::of_opt(&ed);
    let snap_lib = match &scene {
        Some(svc) => {
            let (snap, pick) = svc.pick().await;
            Some((snap, svc.library.clone(), pick))
        }
        None => None,
    };
    let empty = SceneState::default();
    let state = snap_lib.as_ref().map_or(&empty, |(s, _, _)| &s.state);
    let lib = snap_lib.as_ref().map(|(_, l, _)| l.clone());
    // the highest thing at a point of the ground: one ray from above everything
    let ground = |x: f32, z: f32| -> Option<f32> {
        let (_, lib, pick) = snap_lib.as_ref()?;
        let b = pick.bounds();
        if !b.is_valid() {
            return None;
        }
        let from = Vec3::new(x, b.max.y + 1.0, z);
        let filter = gz_scene::pick::RayFilter::default();
        gz_scene::pick::ray_scene(pick, state, lib, from, Vec3::NEG_Y, b.max.y - b.min.y + 2.0, &filter)
            .map(|h| h.point.y)
    };
    let bounds = |o: &gz_scene::Object| -> Option<Aabb> {
        let lib = lib.as_ref()?;
        gz_scene::objects::world_box(state, o, &|r| lib.cached(r).map(|lo| lo.bounds))
    };

    let (image, pending, camera, view) = if own {
        let size = [p.width.unwrap_or(1280), p.height.unwrap_or(720)];
        if size.iter().any(|s| !(16..=MAX_SIDE).contains(s)) {
            return Err(CmdError::invalid(format!("width and height take 16 to {MAX_SIDE} px")));
        }
        let (camera, view) = match (p.position, p.view) {
            (Some(at), _) => {
                let at = finite3(at, "position")?;
                let target = p.target.map(|t| finite3(t, "target")).transpose()?;
                let rotation = p.rotation.map(|t| finite3(t, "rotation")).transpose()?;
                if target.is_some() && rotation.is_some() {
                    return Err(CmdError::invalid("target and rotation both turn the camera: give one"));
                }
                let fov = p.fov.unwrap_or(60.0);
                if !fov.is_finite() {
                    return Err(CmdError::invalid("fov takes degrees"));
                }
                (standing(at, target, rotation, fov), "free")
            }
            (None, Some(side)) => {
                let b = frame_box(state, &bounds, &ground, p.objects.as_deref(), p.area)?;
                (framing(side, &b, size), side.key())
            }
            (None, None) => return Err(CmdError::internal("no camera")),
        };
        let shot = ShotView { camera, size, clean: p.clean };
        let (image, pending) = vp.picture_from(&ed, shot).await?;
        (image, pending, camera, view)
    } else {
        let (image, pending) = vp.picture(&ed).await?;
        let camera = vp.read(|s| s.nav.camera());
        (image, pending, camera, "viewport")
    };

    let (w, h) = (image.width, image.height);
    let size = [w, h];
    let names = if p.labels { labels_of(state, &bounds, &camera, size) } else { Vec::new() };
    let (png, drawn) = ed
        .exec
        .cpu(move || -> Result<(Vec<u8>, Vec<Label>), CmdError> {
            if names.is_empty() {
                return Ok((gz_app::encode_png(image.width, image.height, &image.rgba)?, Vec::new()));
            }
            let mut canvas = Canvas::from_rgba(image.width, image.height, image.rgba)
                .ok_or_else(|| CmdError::internal("the picture's pixels do not match its size"))?;
            let drawn = draw_labels(&mut canvas, &names);
            let png = canvas
                .png()
                .map_err(|e| CmdError::internal(format!("the picture could not be encoded: {e}")))?;
            Ok((png, drawn))
        })
        .await?;
    let encoded = ed.exec.cpu(move || base64::engine::general_purpose::STANDARD.encode(png)).await;
    let scale = vp.read(|s| r(s.ppp, 3));
    let mut out = json!({ "png_base64": encoded, "width": w, "height": h, "scale": scale });
    let Value::Object(m) = &mut out else { return Ok(out) };
    if own {
        let mut cam = json!({
            "view": view,
            "position": v3(camera.position, 3),
            "direction": v3(camera.forward, 3),
            "orthographic": camera.projection.is_ortho(),
        });
        if let (Projection::Orthographic { height, .. }, Value::Object(c)) = (camera.projection, &mut cam) {
            // a flat view is to scale: so many px a metre, everywhere in the picture
            c.insert("px_per_m".into(), json!(r(h as f32 / height.max(1e-4), 3)));
        }
        m.insert("camera".into(), cam);
    }
    if p.labels {
        let list: Vec<Value> = drawn
            .iter()
            .map(|l| json!({ "object": l.name, "x": l.at.0.round(), "y": l.at.1.round(), "distance": r(l.distance, 2) }))
            .collect();
        m.insert("labels".into(), json!(list));
    }
    // what was still loading when the wait ran out: never a silent half picture
    if !pending.is_empty() {
        m.insert("pending".into(), json!(pending));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Aabb {
        Aabb { min: Vec3::new(10.0, 0.0, 20.0), max: Vec3::new(30.0, 4.0, 60.0) }
    }

    #[test]
    fn the_top_view_is_to_scale_with_x_right_and_z_up() {
        let size = [800, 800];
        let cam = framing(Side::Top, &unit_box(), size);
        assert!(cam.projection.is_ortho());
        let Projection::Orthographic { height, .. } = cam.projection else { panic!("not flat") };
        // the box is 20 x 40 m: the picture is as high as its 40 m with room around
        assert!((height - 40.0 * ROOM).abs() < 1e-3, "{height}");
        let centre = to_pixel(&cam, size, Vec3::new(20.0, 0.0, 40.0));
        assert_eq!(centre.map(|p| (p.0.round(), p.1.round())), Some((400.0, 400.0)));
        // +X is right, +Z is up
        let Some(east) = to_pixel(&cam, size, Vec3::new(25.0, 0.0, 40.0)) else { panic!("east") };
        let Some(north) = to_pixel(&cam, size, Vec3::new(20.0, 0.0, 45.0)) else { panic!("north") };
        assert!(east.0 > 400.0 && (east.1 - 400.0).abs() < 0.5, "{east:?}");
        assert!(north.1 < 400.0 && (north.0 - 400.0).abs() < 0.5, "{north:?}");
        // to scale: five metres are the same pixels either way
        let ppm = 800.0 / height;
        assert!((east.0 - 400.0 - 5.0 * ppm).abs() < 0.5 && (400.0 - north.1 - 5.0 * ppm).abs() < 0.5);
        // the ground and the top of the box are both in the picture
        assert!(to_pixel(&cam, size, Vec3::new(20.0, 4.0, 40.0)).is_some());
        assert!(to_pixel(&cam, size, Vec3::new(20.0, 0.0, 40.0)).is_some());
    }

    #[test]
    fn a_wide_box_is_fitted_by_its_width() {
        let b = Aabb { min: Vec3::new(0.0, 0.0, 0.0), max: Vec3::new(100.0, 1.0, 10.0) };
        let size = [1000, 500];
        let cam = framing(Side::Top, &b, size);
        let Projection::Orthographic { height, .. } = cam.projection else { panic!("not flat") };
        assert!((height - 100.0 / 2.0 * ROOM).abs() < 1e-3, "{height}");
        for corner in [Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 10.0)] {
            assert!(to_pixel(&cam, size, corner).is_some(), "{corner}");
        }
    }

    #[test]
    fn every_side_sees_the_whole_box() {
        let b = unit_box();
        let size = [640, 480];
        let corners = [
            Vec3::new(b.min.x, b.min.y, b.min.z),
            Vec3::new(b.max.x, b.min.y, b.min.z),
            Vec3::new(b.min.x, b.max.y, b.min.z),
            Vec3::new(b.max.x, b.max.y, b.max.z),
            Vec3::new(b.min.x, b.min.y, b.max.z),
            Vec3::new(b.max.x, b.max.y, b.min.z),
        ];
        for side in [Side::Top, Side::Front, Side::Back, Side::Left, Side::Right, Side::Iso] {
            let cam = framing(side, &b, size);
            assert_eq!(cam.projection.is_ortho(), side != Side::Iso);
            for c in corners {
                assert!(to_pixel(&cam, size, c).is_some(), "{side:?} loses {c}");
            }
        }
        // the front looks along +Z: +X is right and up is up
        let cam = framing(Side::Front, &b, size);
        let Some(a) = to_pixel(&cam, size, Vec3::new(12.0, 1.0, 20.0)) else { panic!("a") };
        let Some(c) = to_pixel(&cam, size, Vec3::new(28.0, 3.0, 20.0)) else { panic!("c") };
        assert!(c.0 > a.0 && c.1 < a.1);
        // the back mirrors it
        let cam = framing(Side::Back, &b, size);
        let Some(a) = to_pixel(&cam, size, Vec3::new(12.0, 1.0, 60.0)) else { panic!("a") };
        let Some(c) = to_pixel(&cam, size, Vec3::new(28.0, 1.0, 60.0)) else { panic!("c") };
        assert!(c.0 < a.0);
    }

    #[test]
    fn a_standing_camera_looks_where_it_is_told() {
        let size = [640, 480];
        let cam = standing(Vec3::new(0.0, 2.0, 0.0), Some(Vec3::new(0.0, 2.0, 10.0)), None, 60.0);
        let centre = to_pixel(&cam, size, Vec3::new(0.0, 2.0, 10.0));
        assert_eq!(centre.map(|p| (p.0.round(), p.1.round())), Some((320.0, 240.0)));
        // behind the camera: nowhere in the picture
        assert_eq!(to_pixel(&cam, size, Vec3::new(0.0, 2.0, -10.0)), None);
        // a heading of nothing looks along +Z, as the viewport's camera does
        let cam = standing(Vec3::ZERO, None, Some(Vec3::ZERO), 60.0);
        assert!((cam.forward - Vec3::Z).length() < 1e-5);
        let cam = standing(Vec3::ZERO, None, None, 400.0);
        let Projection::Perspective { fov_y, .. } = cam.projection else { panic!("flat") };
        assert!((fov_y.to_degrees() - 150.0).abs() < 1e-3);
    }

    #[test]
    fn names_are_drawn_nearest_first_and_never_over_each_other() {
        let mut canvas = Canvas::new(200, 100, Rgba::BLACK);
        let labels = vec![
            Label { name: "near".into(), at: (100.0, 50.0), distance: 1.0 },
            Label { name: "hidden_behind".into(), at: (102.0, 51.0), distance: 2.0 },
            Label { name: "aside".into(), at: (20.0, 90.0), distance: 3.0 },
            Label { name: "edge".into(), at: (199.0, 1.0), distance: 4.0 },
        ];
        let drawn = draw_labels(&mut canvas, &labels);
        let names: Vec<&str> = drawn.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["near", "aside", "edge"]);
        // something was drawn, and the picture kept its size
        assert!(canvas.rgba().iter().any(|b| *b != 0 && *b != 255));
        assert_eq!((canvas.width(), canvas.height()), (200, 100));
    }
}

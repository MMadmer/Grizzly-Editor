//! The editor camera and its navigation, pure.
//!
//! The camera is heading-pitch-bank and a position, as the engine keeps it; `gz_render::Camera`
//! is made from it for a frame. The navigation: RMB look and fly with WASD + QE, MMB or LMB+RMB
//! pan, Alt+LMB orbit, Alt+RMB dolly, a left drag nothing took flies. Shift, Ctrl and a plain left
//! press are the tools', never the camera's.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::Vec3;
use gz_fmt_object::Mat4 as XMat;
use gz_render::{Aabb, Camera, Projection};

/// The mouse buttons and the modifiers of a gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Buttons {
    #[must_use]
    pub fn any_button(&self) -> bool {
        self.left || self.right || self.middle
    }

    /// The right button and nothing else.
    #[must_use]
    pub fn right_only(&self) -> bool {
        self.right && !self.left && !self.middle && !self.shift && !self.ctrl && !self.alt
    }
}

/// The views of the Perspective pill and the axis gizmo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Perspective,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl Target {
    pub const ALL: [Target; 7] = [
        Target::Perspective,
        Target::Front,
        Target::Back,
        Target::Left,
        Target::Right,
        Target::Top,
        Target::Bottom,
    ];
    pub const AXES: [Target; 6] =
        [Target::Front, Target::Back, Target::Left, Target::Right, Target::Top, Target::Bottom];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Target::Perspective => "perspective",
            Target::Front => "front",
            Target::Back => "back",
            Target::Left => "left",
            Target::Right => "right",
            Target::Top => "top",
            Target::Bottom => "bottom",
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Target::Perspective => "Perspective",
            Target::Front => "Front",
            Target::Back => "Back",
            Target::Left => "Left",
            Target::Right => "Right",
            Target::Top => "Top",
            Target::Bottom => "Bottom",
        }
    }

    /// The key or the name, any case; "reset" is the free view.
    #[must_use]
    pub fn parse(text: &str) -> Option<Target> {
        if text.is_empty() {
            return None;
        }
        if text.eq_ignore_ascii_case("reset") {
            return Some(Target::Perspective);
        }
        Target::ALL
            .into_iter()
            .find(|t| text.eq_ignore_ascii_case(t.key()) || text.eq_ignore_ascii_case(t.name()))
    }

    /// The camera's heading-pitch-bank of an axis view (front ... bottom).
    #[must_use]
    pub fn hpb(self) -> Option<Vec3> {
        match self {
            Target::Perspective => None,
            Target::Front => Some(Vec3::ZERO),
            Target::Back => Some(Vec3::new(PI, 0.0, 0.0)),
            Target::Left => Some(Vec3::new(-FRAC_PI_2, 0.0, 0.0)),
            Target::Right => Some(Vec3::new(FRAC_PI_2, 0.0, 0.0)),
            Target::Top => Some(Vec3::new(0.0, -FRAC_PI_2, 0.0)),
            Target::Bottom => Some(Vec3::new(0.0, FRAC_PI_2, 0.0)),
        }
    }
}

/// `angle_difference`: the shortest turn between two angles.
#[must_use]
pub fn angle_difference(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    if d > PI { TAU - d } else { d }
}

/// The direction a heading and a pitch look along.
#[must_use]
pub fn dir_hp(h: f32, p: f32) -> Vec3 {
    let (sh, ch) = h.sin_cos();
    let (sp, cp) = p.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

/// The keys held while flying (forward ...).
mod fly {
    pub const FORWARD: u8 = 1 << 0;
    pub const BACK: u8 = 1 << 1;
    pub const LEFT: u8 = 1 << 2;
    pub const RIGHT: u8 = 1 << 3;
    pub const UP: u8 = 1 << 4;
    pub const DOWN: u8 = 1 << 5;
}

/// A key the camera takes while it flies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyKey {
    W,
    S,
    A,
    D,
    E,
    Q,
}

impl FlyKey {
    fn bit(self) -> u8 {
        match self {
            FlyKey::W => fly::FORWARD,
            FlyKey::S => fly::BACK,
            FlyKey::A => fly::LEFT,
            FlyKey::D => fly::RIGHT,
            FlyKey::E => fly::UP,
            FlyKey::Q => fly::DOWN,
        }
    }
}

/// The editor camera and the view the navigation keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Nav {
    /// Heading, pitch, bank in radians.
    pub hpb: Vec3,
    pub position: Vec3,
    /// What the camera orbits and what framing looked at.
    pub target: Vec3,
    pub fly_speed: f32,
    pub fly_alt: f32,
    pub orbit_dist: f32,
    /// Move and turn speed per pixel, from the preferences' camera sensitivities.
    pub sm: f32,
    pub sr: f32,
    pub near: f32,
    pub far: f32,
    /// The vertical field of view in degrees.
    pub fov: f32,
    pub ortho: bool,
    /// The view the camera was put in.
    view: Target,
    /// The free camera before an axis view: heading-pitch-bank, position, target.
    saved: Option<(Vec3, Vec3, Vec3)>,
    moving: bool,
    nav_input: bool,
    keys: u8,
    /// The buttons and modifiers of the last event the camera saw.
    held: Buttons,
}

impl Default for Nav {
    fn default() -> Self {
        let mut n = Self {
            hpb: Vec3::ZERO,
            position: Vec3::new(0.0, 3.0, -10.0),
            target: Vec3::ZERO,
            fly_speed: 5.0,
            fly_alt: 1.8,
            orbit_dist: 10.0,
            sm: 0.2 * 0.6 * 0.6,
            sr: 0.02 * 0.6 * 0.6,
            near: 0.2,
            far: 1500.0,
            fov: 60.0,
            ortho: false,
            view: Target::Perspective,
            saved: None,
            moving: false,
            nav_input: false,
            keys: 0,
            held: Buttons::default(),
        };
        n.build();
        n
    }
}

impl Nav {
    /// Sets the move and turn speed from the preferences' sensitivities.
    pub fn set_sensitivity(&mut self, sm: f32, sr: f32) {
        self.sm = 0.2 * sm * sm;
        self.sr = 0.02 * sr * sr;
    }

    /// Puts the camera at a pose.
    pub fn set(&mut self, hpb: Vec3, position: Vec3) {
        self.hpb = hpb;
        self.position = position;
        self.build();
    }

    /// The angles kept within a turn either way.
    pub fn build(&mut self) {
        for a in [&mut self.hpb.x, &mut self.hpb.y, &mut self.hpb.z] {
            if *a > TAU {
                *a -= TAU;
            }
            if *a < -TAU {
                *a += TAU;
            }
        }
    }

    /// The camera's matrix: rows right, up, forward.
    #[must_use]
    pub fn matrix(&self) -> XMat {
        XMat::hpb(self.hpb.x, self.hpb.y, self.hpb.z)
    }

    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.matrix().row(0)
    }

    #[must_use]
    pub fn up(&self) -> Vec3 {
        self.matrix().row(1)
    }

    #[must_use]
    pub fn direction(&self) -> Vec3 {
        self.matrix().row(2)
    }

    /// The renderer's camera for this pose.
    #[must_use]
    pub fn camera(&self) -> Camera {
        let m = self.matrix();
        let projection = if self.ortho {
            // the height the perspective view showed at the orbit distance
            let height = 2.0 * self.orbit_dist.max(0.5) * (self.fov.to_radians() * 0.5).tan();
            Projection::Orthographic { height, near: -self.far, far: self.far }
        } else {
            Projection::Perspective { fov_y: self.fov.to_radians(), near: self.near, far: self.far }
        };
        Camera { position: self.position, forward: m.row(2), up: m.row(1), projection }
    }

    #[must_use]
    pub fn is_moving(&self) -> bool {
        self.moving
    }

    /// The last right press moved the camera, so it opens no menu.
    #[must_use]
    pub fn was_nav_input(&self) -> bool {
        self.nav_input
    }

    /// Flying with the right button: WASD and QE belong to the camera.
    #[must_use]
    pub fn flying(&self) -> bool {
        self.moving && self.held.right
    }

    fn pivot_for_orbit(&mut self) {
        let d = self.position.distance(self.target);
        if d > 0.5 && d < 500.0 {
            self.orbit_dist = d;
        } else {
            self.orbit_dist = 10.0;
            self.target = self.position + self.direction() * self.orbit_dist;
        }
    }

    /// RMB, MMB or Alt+LMB claim the mouse for the camera. `true` when the camera took the press.
    pub fn move_start(&mut self, b: Buttons) -> bool {
        self.held = b;
        if !(b.right || b.middle || (b.alt && b.left)) {
            return false;
        }
        if !self.moving {
            self.moving = true;
            self.keys = 0;
            self.nav_input = false;
            self.pivot_for_orbit();
        }
        true
    }

    /// A left drag nothing took is the camera's.
    pub fn left_drag_start(&mut self, b: Buttons) -> bool {
        if self.moving {
            return false;
        }
        self.moving = true;
        self.keys = 0;
        self.nav_input = false;
        self.held = b;
        self.pivot_for_orbit();
        true
    }

    /// `true` when the move ended.
    pub fn move_end(&mut self, b: Buttons) -> bool {
        self.held = b;
        if b.any_button() {
            return false;
        }
        self.moving = false;
        self.keys = 0;
        true
    }

    fn rotate(&mut self, dx: f32, dy: f32, aspect: f32) {
        self.hpb.x -= self.sr * dx;
        self.hpb.y -= self.sr * dy * aspect;
        self.build();
    }

    /// A mouse move of `dx`, `dy` pixels while the camera moves. `aspect` is the picture's height
    /// over its width. `true` when the camera took it.
    pub fn process(&mut self, b: Buttons, dx: f32, dy: f32, aspect: f32) -> bool {
        if !self.moving {
            return false;
        }
        self.held = b;
        if dx == 0.0 && dy == 0.0 {
            return true;
        }
        self.view = Target::Perspective;
        self.nav_input = true;
        let (l, r, m, a) = (b.left, b.right, b.middle, b.alt);
        let cam = self.matrix();
        if a && l && !r {
            // orbit round the pivot at a fixed distance
            self.hpb.x -= self.sr * dx;
            self.hpb.y -= self.sr * dy * aspect;
            self.position = self.target - dir_hp(self.hpb.x, self.hpb.y) * self.orbit_dist;
            self.build();
        } else if a && r && !l {
            // dolly: drag up is in, drag down out
            let d = dy * self.sm;
            self.position -= cam.row(2) * d;
            self.orbit_dist = (self.orbit_dist + d).clamp(0.5, 5000.0);
            self.build();
        } else if (l && r) || m {
            // pan: grab the world - a drag right moves the camera left
            self.position += cam.row(0) * (-dx * self.sm);
            self.position.y += dy * self.sm;
            self.build();
        } else if r {
            self.rotate(dx, dy, aspect);
        } else if l {
            // the vertical flies forward and back, the horizontal turns
            self.position += cam.row(2) * (-dy * self.sm);
            self.hpb.x -= self.sr * dx;
            self.build();
        }
        true
    }

    /// The key press while flying: `true` when the camera took the key.
    pub fn key_down(&mut self, key: FlyKey) -> bool {
        if !self.flying() {
            return false;
        }
        self.keys |= key.bit();
        self.nav_input = true;
        true
    }

    pub fn key_up(&mut self, key: FlyKey) -> bool {
        if !self.moving {
            return false;
        }
        self.keys &= !key.bit();
        true
    }

    /// The held keys follow what is down this frame.
    pub fn set_keys(&mut self, down: &[FlyKey]) {
        if !self.flying() {
            self.keys = 0;
            return;
        }
        let mut k = 0;
        for d in down {
            k |= d.bit();
        }
        if k != 0 {
            self.nav_input = true;
        }
        self.keys = k;
    }

    /// WASD and QE fly along the view (QE along the world's up). `true` when the
    /// camera moved.
    pub fn update(&mut self, dt: f32) -> bool {
        if !(self.flying() && self.keys != 0) {
            return false;
        }
        let m = self.matrix();
        let mut mv = Vec3::ZERO;
        if self.keys & fly::FORWARD != 0 {
            mv += m.row(2);
        }
        if self.keys & fly::BACK != 0 {
            mv -= m.row(2);
        }
        if self.keys & fly::RIGHT != 0 {
            mv += m.row(0);
        }
        if self.keys & fly::LEFT != 0 {
            mv -= m.row(0);
        }
        if self.keys & fly::UP != 0 {
            mv.y += 1.0;
        }
        if self.keys & fly::DOWN != 0 {
            mv.y -= 1.0;
        }
        if mv.length_squared() <= 1e-6 {
            return false;
        }
        self.position += mv.normalize() * (self.fly_speed * dt);
        self.view = Target::Perspective;
        self.build();
        true
    }

    /// The wheel: while flying it tunes the speed and answers the status line; otherwise it
    /// dollies along the view.
    pub fn wheel(&mut self, steps: f32) -> Option<String> {
        if self.flying() {
            self.fly_speed = (self.fly_speed * 1.2f32.powf(steps)).clamp(0.1, 200.0);
            return Some(format!("Camera speed: {:.1}", self.fly_speed));
        }
        let k = self.direction();
        self.position += k * (steps * (self.fly_speed * 0.35).max(0.5));
        if self.ortho {
            self.orbit_dist = (self.orbit_dist - steps * (self.fly_speed * 0.35).max(0.5)).clamp(0.5, 5000.0);
        }
        self.view = Target::Perspective;
        self.build();
        None
    }

    /// Frames a box keeping the view's direction (fitted by `gz_render::frame_fit`).
    pub fn zoom_extents(&mut self, bounds: &Aabb, size: [u32; 2]) {
        if bounds.is_empty() {
            return;
        }
        let cam = gz_render::frame_fit(&self.camera(), bounds, size);
        self.position = cam.position;
        self.target = bounds.center();
        self.orbit_dist = self.position.distance(self.target).max(0.5);
        if let Projection::Orthographic { height, .. } = cam.projection {
            self.orbit_dist = (height * 0.5 / (self.fov.to_radians() * 0.5).tan().max(1e-3)).max(0.5);
        }
        self.build();
    }

    /// The camera still looks the way the view put it.
    fn matches(&self, target: Target) -> bool {
        let Some(want) = target.hpb() else { return true };
        let tol = 0.1f32.to_radians();
        angle_difference(self.hpb.x, want.x) <= tol
            && angle_difference(self.hpb.y, want.y) <= tol
            && angle_difference(self.hpb.z, want.z) <= tol
    }

    /// An axis view the camera turned away from is the free view again.
    #[must_use]
    pub fn current(&self) -> Target {
        if self.matches(self.view) { self.view } else { Target::Perspective }
    }

    /// An axis view keeps the position and turns the camera; the free view comes
    /// back to the pose it had before the first axis view.
    pub fn set_target(&mut self, target: Target) {
        let previous = self.current();
        if target != Target::Perspective && previous == Target::Perspective {
            self.saved = Some((self.hpb, self.position, self.target));
        }
        match target.hpb() {
            None => {
                if previous != Target::Perspective
                    && let Some((hpb, pos, tgt)) = self.saved
                {
                    self.hpb = hpb;
                    self.position = pos;
                    self.target = tgt;
                }
            }
            Some(hpb) => self.hpb = hpb,
        }
        self.view = target;
        self.build();
    }

    /// The free view, nothing remembered.
    pub fn reset_state(&mut self) {
        self.view = Target::Perspective;
        self.saved = None;
    }

    /// A ray through a pixel of a picture `size` big.
    #[must_use]
    pub fn ray(&self, size: [u32; 2], pixel: glam::Vec2) -> Option<gz_render::Ray> {
        gz_render::pick_ray(&self.camera(), size, pixel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() < 1e-4
    }

    #[test]
    fn views_parse_and_turn() {
        assert_eq!(Target::parse("TOP"), Some(Target::Top));
        assert_eq!(Target::parse("reset"), Some(Target::Perspective));
        assert_eq!(Target::parse("Bottom"), Some(Target::Bottom));
        assert_eq!(Target::parse("diagonal"), None);
        let mut n = Nav::default();
        n.set(Vec3::new(0.3, -0.2, 0.0), Vec3::new(1.0, 2.0, 3.0));
        n.set_target(Target::Right);
        assert!(close(n.direction(), Vec3::new(-1.0, 0.0, 0.0)));
        assert_eq!(n.current(), Target::Right);
        n.set_target(Target::Top);
        assert!(close(n.direction(), Vec3::new(0.0, -1.0, 0.0)));
        n.set_target(Target::Perspective);
        assert!(close(n.hpb, Vec3::new(0.3, -0.2, 0.0)));
        assert!(close(n.position, Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn turning_away_leaves_the_axis_view() {
        let mut n = Nav::default();
        n.set_target(Target::Front);
        assert_eq!(n.current(), Target::Front);
        n.hpb.x += 0.2;
        assert_eq!(n.current(), Target::Perspective);
    }

    #[test]
    fn right_drag_looks_and_wasd_flies() {
        let mut n = Nav::default();
        let b = Buttons { right: true, ..Buttons::default() };
        assert!(n.move_start(b));
        assert!(n.flying());
        let before = n.hpb;
        assert!(n.process(b, 10.0, 0.0, 0.75));
        assert!(n.hpb.x < before.x);
        n.set_keys(&[FlyKey::W]);
        let p = n.position;
        assert!(n.update(0.5));
        assert!((n.position - p).length() > 2.0);
        assert!(n.wheel(1.0).is_some());
        assert!(n.move_end(Buttons::default()));
        assert!(!n.is_moving());
    }

    #[test]
    fn a_plain_or_shift_left_press_is_the_tools() {
        let mut n = Nav::default();
        assert!(!n.move_start(Buttons { left: true, ..Buttons::default() }));
        assert!(!n.move_start(Buttons { left: true, shift: true, ..Buttons::default() }));
        assert!(!n.is_moving());
        assert!(n.move_start(Buttons { left: true, alt: true, ..Buttons::default() }));
    }

    #[test]
    fn the_engine_direction() {
        assert!(close(dir_hp(0.0, 0.0), Vec3::Z));
        assert!(close(dir_hp(FRAC_PI_2, 0.0), -Vec3::X));
        assert!((angle_difference(0.1, TAU - 0.1) - 0.2).abs() < 1e-5);
    }
}

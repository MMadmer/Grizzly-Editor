//! The renderer's input: a plain description of what a frame draws. A scene adapter makes one
//! from a scene snapshot; a preview builds one by hand.

use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use smol_str::SmolStr;

use crate::assets::{MaterialHandle, MeshHandle};
use crate::color::Color;
use crate::math::Aabb;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct InstanceFlags: u32 {
        /// Outlined (with [`crate::ViewFlags::SELECTION`]).
        const SELECTED = 1 << 0;
        /// Left out of a [`crate::ViewFlags::PREVIEW`] picture.
        const HIDDEN_IN_PREVIEW = 1 << 1;
        /// Drawn from both sides whatever its materials say.
        const TWO_SIDED = 1 << 2;
        /// Never culled (a sky dome, something whose bounds are not known).
        const NO_CULL = 1 << 3;
        /// Left out of the id target: never outlined, never picked by id.
        const NO_ID = 1 << 4;
        /// A sky mesh whose normals are its cube-map coordinates, kept as they are while the
        /// instance turns it (the game's sky box).
        const SKY_TC = 1 << 5;
        /// Lies on another surface (a wallmark): drawn a little nearer than it is.
        const DECAL = 1 << 6;
    }
}

/// One mesh placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub mesh: MeshHandle,
    /// Replaces the material of every surface of the mesh.
    pub material: Option<MaterialHandle>,
    pub transform: Mat4,
    pub flags: InstanceFlags,
    /// The selection id written to the id target; 0 is none.
    pub id: u32,
    /// Multiplies the surface colour.
    pub tint: Color,
    /// The mesh's own bounds when the caller knows them; otherwise the loaded mesh's, and a
    /// 1 m box while it loads.
    pub bounds: Option<Aabb>,
    /// Skinning matrices (model space at rest to model space in the pose), one per bone.
    pub pose: Option<Arc<[Mat4]>>,
}

impl Instance {
    #[must_use]
    pub fn new(mesh: MeshHandle, transform: Mat4) -> Self {
        Self {
            mesh,
            material: None,
            transform,
            flags: InstanceFlags::empty(),
            id: 0,
            tint: Color::WHITE,
            bounds: None,
            pose: None,
        }
    }

    #[must_use]
    pub fn with_id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }

    #[must_use]
    pub fn with_flags(mut self, flags: InstanceFlags) -> Self {
        self.flags |= flags;
        self
    }

    #[must_use]
    pub fn with_material(mut self, material: MaterialHandle) -> Self {
        self.material = Some(material);
        self
    }

    #[must_use]
    pub fn with_pose(mut self, pose: Arc<[Mat4]>) -> Self {
        self.pose = Some(pose);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    /// `cos_half` is the cosine of the cone's half angle.
    Spot {
        direction: Vec3,
        cos_half: f32,
    },
}

/// A light of the scene, drawn in [`crate::ViewMode::GameLighting`] with the game's falloff,
/// `1 - (distance / range)^2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub position: Vec3,
    pub range: f32,
    /// Colour times brightness: what the game feeds its light.
    pub color: Vec3,
}

/// The height of the topmost surface over each cell of a square map: a point below it is
/// indoors, out of the sun and most of the sky.
#[derive(Clone, Debug, PartialEq)]
pub struct RoofMap {
    /// The corner of the map on the ground plane: x and z.
    pub origin: [f32; 2],
    pub cell: f32,
    /// Cells a side.
    pub cells: u32,
    /// `cells * cells` heights, row by row along z.
    pub heights: Arc<[f32]>,
    /// Changes whenever the heights do; the GPU copy is made again only then.
    pub revision: u64,
}

/// The weather at an hour, as the game-lighting mode lights with it (already scaled by the
/// game's lumscales).
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    pub ambient: Vec3,
    pub hemi: Vec3,
    pub sun_color: Vec3,
    /// From the sun.
    pub sun_dir: Vec3,
    pub fog_color: Vec3,
    /// Linear fog from `fog_near` to `fog_far` metres; `fog_far <= fog_near` is none.
    pub fog_near: f32,
    pub fog_far: f32,
    /// What the tonemap scales the frame by.
    pub exposure: f32,
    /// The share of the hemisphere light that reaches indoors.
    pub indoor_hemi: f32,
    pub roof: Option<Arc<RoofMap>>,
    /// The weather's sky box.
    pub sky: Option<Sky>,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            ambient: Vec3::splat(0.05),
            hemi: Vec3::splat(0.25),
            sun_color: Vec3::splat(0.6),
            sun_dir: Vec3::new(0.3, -0.8, 0.5).normalize(),
            fog_color: Vec3::splat(0.4),
            fog_near: 0.0,
            fog_far: 0.0,
            exposure: 1.0,
            indoor_hemi: 0.15,
            roof: None,
            sky: None,
        }
    }
}

/// The sky of a weather as `dxEnvironmentRender::RenderSky` draws it: a half box round the
/// camera, the two hours' cube maps blended by `weight`, times `color`, turned by `rotation`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    /// The cube maps of the hour before and the hour after (`sky_texture`).
    pub textures: [SmolStr; 2],
    /// `sky_color`.
    pub color: Vec3,
    /// How far from the first hour to the second, 0..1.
    pub weight: f32,
    /// `sky_rotation`, radians about the world's Y.
    pub rotation: f32,
}

/// How a sprite is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpriteAlign {
    /// Facing the camera: the camera's up is its top, the camera's right its right.
    Camera,
    /// Its top and its right in the world.
    Axes { top: Vec3, right: Vec3 },
    /// Its top along a direction, its right `dir x the camera's forward`.
    Along(Vec3),
}

/// Which faces of a sprite are not drawn, as they appear on the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteCull {
    #[default]
    None,
    Clockwise,
    CounterClockwise,
}

/// What a sprite's half size is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteSize {
    /// Metres.
    #[default]
    World,
    /// A glow's radius: `width * radius / depth` pixels, whatever its
    /// distance.
    Screen,
    /// A fixed share of the picture: `radius` is in clip units across.
    ScreenFixed,
}

/// A quad the frame builds facing the camera (`FillSprite`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub position: Vec3,
    /// Half the width, half the height.
    pub half: glam::Vec2,
    /// Turned by this angle about its normal.
    pub angle: f32,
    pub color: Color,
    /// The part of the texture: left-top u, v, right-bottom u, v.
    pub uv: [f32; 4],
    pub align: SpriteAlign,
}

/// Sprites drawn with one material: a particle effect's particles, a glow.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteBatch {
    pub material: MaterialHandle,
    pub sprites: Vec<Sprite>,
    pub cull: SpriteCull,
    pub size: SpriteSize,
    /// Where the sprites are; culled by it when known, never otherwise.
    pub bounds: Option<Aabb>,
    pub id: u32,
    pub flags: InstanceFlags,
}

/// One copy of a [`Batch`]'s mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatchInstance {
    pub transform: Mat4,
    pub tint: Color,
}

/// Many copies of one mesh the caller culled already (detail objects): drawn instanced as
/// they come, without a test of each.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub mesh: MeshHandle,
    pub material: Option<MaterialHandle>,
    pub instances: Vec<BatchInstance>,
    /// Where the copies are, for the order of blended draws.
    pub bounds: Aabb,
    pub id: u32,
    pub flags: InstanceFlags,
}

/// A line of a helper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Color,
    /// Hidden behind the scene's surfaces; an overlay (a gizmo, a way link) is drawn over them.
    pub depth_tested: bool,
}

/// The unit shapes of the editor's helpers (`du_box`, `du_sphere`, `du_cone`, `du_cylinder`, ...).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    /// -0.5 to 0.5 on every axis.
    Box,
    /// Radius 1 round the origin: three great circles when wired (`DrawLineSphere`).
    Sphere,
    /// The apex at the origin, opening along +Z to a base of radius 1 at z = 1.
    Cone,
    /// Radius 1 round the Z axis, from z = -0.5 to 0.5.
    Cylinder,
    /// The XZ square -0.5 to 0.5, facing +Y.
    Plane,
    /// Three lines through the origin, 1 each way; `rot45` adds the same turned by 45 degrees
    /// about every axis (`DrawCross`, a point light's mark).
    Cross { rot45: bool },
    /// The corner brackets of a selected object's box (`DrawSelectionBox`).
    SelectionBox,
    /// An octahedron of radius 1 (`DrawRomboid`).
    Romboid,
}

/// A unit shape moved into the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub transform: Mat4,
    /// The faces, blended by their alpha.
    pub fill: Option<Color>,
    /// The edges.
    pub wire: Option<Color>,
    pub depth_tested: bool,
}

/// A text mark at a point of the world. The renderer does not draw text: the UI draws a
/// [`crate::ScreenLabel`] over the picture where the frame says it is, and a depth-tested one only
/// where the frame found it visible.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub position: Vec3,
    pub text: SmolStr,
    pub color: Color,
    pub depth_tested: bool,
}

/// The helper shapes of a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Helpers {
    pub lines: Vec<Line>,
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,
}

impl Helpers {
    pub fn line(&mut self, a: Vec3, b: Vec3, color: Color, depth_tested: bool) {
        self.lines.push(Line { a, b, color, depth_tested });
    }

    pub fn shape(
        &mut self,
        kind: ShapeKind,
        transform: Mat4,
        fill: Option<Color>,
        wire: Option<Color>,
        depth_tested: bool,
    ) {
        self.shapes.push(Shape { kind, transform, fill, wire, depth_tested });
    }

    /// A box from its centre and size.
    pub fn aabb(&mut self, bounds: &Aabb, fill: Option<Color>, wire: Option<Color>, depth_tested: bool) {
        self.shape(ShapeKind::Box, bounds.unit_transform(), fill, wire, depth_tested);
    }

    /// `DrawLineSphere`: three circles.
    pub fn sphere(
        &mut self,
        center: Vec3,
        radius: f32,
        fill: Option<Color>,
        wire: Option<Color>,
        depth_tested: bool,
    ) {
        let m = Mat4::from_scale_rotation_translation(Vec3::splat(radius), Quat::IDENTITY, center);
        self.shape(ShapeKind::Sphere, m, fill, wire, depth_tested);
    }

    /// `DrawCone`: the apex, the axis, the height and the base radius.
    #[allow(clippy::too_many_arguments)]
    pub fn cone(
        &mut self,
        apex: Vec3,
        dir: Vec3,
        height: f32,
        radius: f32,
        fill: Option<Color>,
        wire: Option<Color>,
        depth_tested: bool,
    ) {
        let rot = Quat::from_rotation_arc(Vec3::Z, dir.try_normalize().unwrap_or(Vec3::Z));
        let m = Mat4::from_scale_rotation_translation(Vec3::new(radius, radius, height), rot, apex);
        self.shape(ShapeKind::Cone, m, fill, wire, depth_tested);
    }

    /// `DrawLink`: a line from `a` to `b` with an arrow head of `head` metres at `b`.
    pub fn arrow(&mut self, a: Vec3, b: Vec3, head: f32, color: Color, depth_tested: bool) {
        self.line(b, a, color, depth_tested);
        let Some(d) = (b - a).try_normalize() else { return };
        let n = if d.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        let r = n.cross(d).normalize() * 0.5;
        let up = d.cross(r).normalize() * 0.5;
        let back = d * 2.0;
        for side in [r, -r, up, -up] {
            self.line(b, b + (side + back) * (-0.5 * head), color, depth_tested);
        }
    }

    /// `DrawCross` with every arm `size` long.
    pub fn cross(&mut self, at: Vec3, size: f32, rot45: bool, color: Color, depth_tested: bool) {
        let m = Mat4::from_scale_rotation_translation(Vec3::splat(size), Quat::IDENTITY, at);
        self.shape(ShapeKind::Cross { rot45 }, m, None, Some(color), depth_tested);
    }

    /// `DrawPointLight`.
    pub fn point_light(&mut self, at: Vec3, radius: f32, color: Color) {
        self.cross(at, radius, true, color, true);
    }

    /// `DrawSpotLight`: the cone's edges from the light and its axis.
    pub fn spot_light(&mut self, at: Vec3, dir: Vec3, range: f32, angle: f32, color: Color) {
        let d = dir.try_normalize().unwrap_or(Vec3::Z);
        let half = (std::f32::consts::FRAC_PI_2 - angle * 0.5).max(0.0);
        let (b, a) = (range * half.cos(), range * half.sin());
        let rot = Quat::from_rotation_arc(Vec3::Z, d);
        const N: u32 = 32;
        for i in 0..N {
            let t = i as f32 / N as f32 * std::f32::consts::TAU;
            let p = at + rot * Vec3::new(b * t.cos(), b * t.sin(), a);
            self.line(at, p, color, true);
        }
        self.line(at, at + d * range, color, true);
    }

    pub fn label(&mut self, position: Vec3, text: impl Into<SmolStr>, color: Color, depth_tested: bool) {
        self.labels.push(Label { position, text: text.into(), color, depth_tested });
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.shapes.is_empty() && self.labels.is_empty()
    }
}

/// Which instances a caller with its own spatial index found visible: their indices into
/// [`DrawList::instances`]. [`crate::build_packet_with`] then skips its own culling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visibility {
    pub visible: Vec<u32>,
}

/// Everything a frame draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub instances: Vec<Instance>,
    pub lights: Vec<Light>,
    /// The weather of [`crate::ViewMode::GameLighting`].
    pub environment: Option<Environment>,
    pub helpers: Helpers,
    pub sprites: Vec<SpriteBatch>,
    pub batches: Vec<Batch>,
}

impl DrawList {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, instance: Instance) {
        self.instances.push(instance);
    }
}

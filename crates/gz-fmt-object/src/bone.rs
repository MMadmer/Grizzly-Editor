//! Bones of a source model: the rest pose, the collision shape and the joint (`Bone.h`,
//! `BoneEditor.cpp`).

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::io::{Text, TextEncoding, chunk, lower, need_chunk, w_str};
use crate::{ObjectError, Result};

const BONE_VERSION: u16 = 0x0002;
const CHUNK_VERSION: u32 = 0x0001;
const CHUNK_DEF: u32 = 0x0002;
const CHUNK_BIND_POSE: u32 = 0x0003;
const CHUNK_MATERIAL: u32 = 0x0004;
const CHUNK_SHAPE: u32 = 0x0005;
const CHUNK_IK_JOINT: u32 = 0x0006;
const CHUNK_MASS: u32 = 0x0007;
const CHUNK_FLAGS: u32 = 0x0008;
const CHUNK_IK_JOINT_BREAK: u32 = 0x0009;
const CHUNK_IK_JOINT_FRICTION: u32 = 0x0010;

/// `SBoneShape::EShapeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeType {
    None = 0,
    Box = 1,
    Sphere = 2,
    Cylinder = 3,
}

/// `SBoneShape`: the collision shape of a bone. All three bodies are stored whatever the type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneShape {
    /// See [`ShapeType`]; kept as stored.
    pub kind: u16,
    /// Bit 0 not pickable, 1 removed after a break, 2 no physics, 3 no fog collider.
    pub flags: u16,
    /// The oriented box: three axes, the centre, the half sizes.
    pub box_rotate: [[f32; 3]; 3],
    pub box_translate: Vec3,
    pub box_halfsize: Vec3,
    pub sphere_center: Vec3,
    pub sphere_radius: f32,
    pub cylinder_center: Vec3,
    pub cylinder_direction: Vec3,
    pub cylinder_height: f32,
    pub cylinder_radius: f32,
}

impl Default for BoneShape {
    fn default() -> Self {
        Self {
            kind: 0,
            flags: 0,
            box_rotate: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            box_translate: Vec3::ZERO,
            box_halfsize: Vec3::ZERO,
            sphere_center: Vec3::ZERO,
            sphere_radius: 0.0,
            cylinder_center: Vec3::ZERO,
            cylinder_direction: Vec3::ZERO,
            cylinder_height: 0.0,
            cylinder_radius: 0.0,
        }
    }
}

impl BoneShape {
    /// The size of the shape in a file.
    pub const SIZE: usize = 112;

    #[must_use]
    pub fn shape_type(&self) -> Option<ShapeType> {
        match self.kind {
            0 => Some(ShapeType::None),
            1 => Some(ShapeType::Box),
            2 => Some(ShapeType::Sphere),
            3 => Some(ShapeType::Cylinder),
            _ => None,
        }
    }

    /// `Valid()`: a shape of a type has a size. The box test looks at its x three times, as
    /// the C++ does.
    #[must_use]
    pub fn valid(&self) -> bool {
        let zero = |v: f32| v.abs() < crate::xmath::EPS_S;
        match self.kind {
            1 => !zero(self.box_halfsize.x),
            2 => !zero(self.sphere_radius),
            3 => {
                let d = self.cylinder_direction;
                !zero(self.cylinder_height) && !zero(self.cylinder_radius) && !zero(d.x * d.x + d.y * d.y + d.z * d.z)
            }
            _ => true,
        }
    }

    pub fn read(r: &mut Reader<'_>) -> Result<BoneShape> {
        let mut s = BoneShape { kind: r.r_u16()?, flags: r.r_u16()?, ..BoneShape::default() };
        for row in &mut s.box_rotate {
            for v in row {
                *v = r.r_f32()?;
            }
        }
        s.box_translate = r.r_vec3()?;
        s.box_halfsize = r.r_vec3()?;
        s.sphere_center = r.r_vec3()?;
        s.sphere_radius = r.r_f32()?;
        s.cylinder_center = r.r_vec3()?;
        s.cylinder_direction = r.r_vec3()?;
        s.cylinder_height = r.r_f32()?;
        s.cylinder_radius = r.r_f32()?;
        Ok(s)
    }

    pub fn write(&self, w: &mut Writer) {
        w.w_u16(self.kind);
        w.w_u16(self.flags);
        for row in &self.box_rotate {
            for v in row {
                w.w_f32(*v);
            }
        }
        w.w_vec3(self.box_translate);
        w.w_vec3(self.box_halfsize);
        w.w_vec3(self.sphere_center);
        w.w_f32(self.sphere_radius);
        w.w_vec3(self.cylinder_center);
        w.w_vec3(self.cylinder_direction);
        w.w_f32(self.cylinder_height);
        w.w_f32(self.cylinder_radius);
    }
}

/// `EJointType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JointType {
    Rigid = 0,
    Cloth = 1,
    Joint = 2,
    Wheel = 3,
    None = 4,
    Slider = 5,
}

/// `SJointLimit`: the range of one axis, as the editor keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointLimit {
    /// `x` the low end, `y` the high end.
    pub limit: Vec2,
    pub spring_factor: f32,
    pub damping_factor: f32,
}

impl Default for JointLimit {
    fn default() -> Self {
        Self { limit: Vec2::ZERO, spring_factor: 1.0, damping_factor: 1.0 }
    }
}

/// `SJointIKData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointIk {
    /// See [`JointType`]; kept as stored.
    pub kind: u32,
    pub limits: [JointLimit; 3],
    pub spring_factor: f32,
    pub damping_factor: f32,
    /// Bit 0: breakable.
    pub flags: u32,
    pub break_force: f32,
    pub break_torque: f32,
    pub friction: f32,
}

impl Default for JointIk {
    fn default() -> Self {
        Self {
            kind: 0,
            limits: [JointLimit::default(); 3],
            spring_factor: 1.0,
            damping_factor: 1.0,
            flags: 0,
            break_force: 0.0,
            break_torque: 0.0,
            friction: 0.0,
        }
    }
}

impl JointIk {
    pub const BREAKABLE: u32 = 1 << 0;

    #[must_use]
    pub fn joint_type(&self) -> Option<JointType> {
        match self.kind {
            0 => Some(JointType::Rigid),
            1 => Some(JointType::Cloth),
            2 => Some(JointType::Joint),
            3 => Some(JointType::Wheel),
            4 => Some(JointType::None),
            5 => Some(JointType::Slider),
            _ => None,
        }
    }

    #[must_use]
    pub fn breakable(&self) -> bool {
        self.flags & Self::BREAKABLE != 0
    }

    fn read_joint(&mut self, r: &mut Reader<'_>) -> Result<()> {
        self.kind = r.r_u32()?;
        for l in &mut self.limits {
            l.limit = r.r_vec2()?;
            l.spring_factor = r.r_f32()?;
            l.damping_factor = r.r_f32()?;
        }
        self.spring_factor = r.r_f32()?;
        self.damping_factor = r.r_f32()?;
        Ok(())
    }

    fn write_joint(&self, w: &mut Writer) {
        w.w_u32(self.kind);
        for l in &self.limits {
            w.w_vec2(l.limit);
            w.w_f32(l.spring_factor);
            w.w_f32(l.damping_factor);
        }
        w.w_f32(self.spring_factor);
        w.w_f32(self.damping_factor);
    }
}

/// `CBone`: a bone of a source model.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    /// Lower case.
    pub name: SmolStr,
    /// The parent's name, lower case; empty for the root.
    pub parent: SmolStr,
    /// The weight map that binds vertices to the bone.
    pub wmap: SmolStr,
    pub rest_offset: Vec3,
    /// XYZ angles in the game's order.
    pub rest_rotate: Vec3,
    pub rest_length: f32,
    pub game_mtl: SmolStr,
    pub shape: BoneShape,
    pub ik: JointIk,
    pub mass: f32,
    pub center_of_mass: Vec3,
    /// The parent's index in the object's bone list. Derived: `Object::prepare_bones` sets it.
    pub parent_id: Option<u16>,
}

impl Default for Bone {
    fn default() -> Self {
        Self {
            name: SmolStr::default(),
            parent: SmolStr::default(),
            wmap: SmolStr::default(),
            rest_offset: Vec3::ZERO,
            rest_rotate: Vec3::ZERO,
            rest_length: 0.0,
            game_mtl: SmolStr::new_static("default_object"),
            shape: BoneShape::default(),
            ik: JointIk::default(),
            mass: 10.0,
            center_of_mass: Vec3::ZERO,
            parent_id: None,
        }
    }
}

impl Bone {
    /// `Load_0`: the first form, fields in a row.
    pub(crate) fn read_v0(r: &mut Reader<'_>, text: &mut Text) -> Result<Bone> {
        let mut b = Bone { name: lower(&text.r_str(r)?), parent: lower(&text.r_str(r)?), wmap: text.r_str(r)?, ..Bone::default() };
        b.rest_offset = r.r_vec3()?;
        b.rest_rotate = r.r_vec3()?;
        b.rest_length = r.r_f32()?;
        std::mem::swap(&mut b.rest_rotate.x, &mut b.rest_rotate.y);
        Ok(b)
    }

    /// `Load_1`: the chunked form.
    pub(crate) fn read(r: &Reader<'_>, text: &mut Text) -> Result<Bone> {
        let version = need_chunk(r, CHUNK_VERSION)?.r_u16()?;
        if version != 0x0001 && version != BONE_VERSION {
            // oracle: the C++ returns with an empty bone here
            return Err(ObjectError::Version { what: "The bone", version: u32::from(version) });
        }
        let mut def = need_chunk(r, CHUNK_DEF)?;
        let mut b = Bone { name: lower(&text.r_str(&mut def)?), ..Bone::default() };
        b.parent = lower(&text.r_str(&mut def)?);
        b.wmap = text.r_str(&mut def)?;

        let mut pose = need_chunk(r, CHUNK_BIND_POSE)?;
        b.rest_offset = pose.r_vec3()?;
        b.rest_rotate = pose.r_vec3()?;
        b.rest_length = pose.r_f32()?;
        if version == 0x0001 {
            std::mem::swap(&mut b.rest_rotate.x, &mut b.rest_rotate.y);
        }
        b.read_data(r, text)?;
        Ok(b)
    }

    /// `LoadData`: what a `.bones` file and the second half of a bone hold.
    pub(crate) fn read_data(&mut self, r: &Reader<'_>, text: &mut Text) -> Result<()> {
        self.name = lower(&text.r_str(&mut need_chunk(r, CHUNK_DEF)?)?);
        self.game_mtl = text.r_str(&mut need_chunk(r, CHUNK_MATERIAL)?)?;
        self.shape = BoneShape::read(&mut need_chunk(r, CHUNK_SHAPE)?)?;
        if let Some(mut c) = chunk(r, CHUNK_FLAGS) {
            self.ik.flags = c.r_u32()?;
        }
        self.ik.read_joint(&mut need_chunk(r, CHUNK_IK_JOINT)?)?;
        if let Some(mut c) = chunk(r, CHUNK_IK_JOINT_BREAK) {
            self.ik.break_force = c.r_f32()?;
            self.ik.break_torque = c.r_f32()?;
        }
        if let Some(mut c) = chunk(r, CHUNK_IK_JOINT_FRICTION) {
            self.ik.friction = c.r_f32()?;
        }
        if let Some(mut c) = chunk(r, CHUNK_MASS) {
            self.mass = c.r_f32()?;
            self.center_of_mass = c.r_vec3()?;
        }
        Ok(())
    }

    /// `Save`.
    pub(crate) fn write(&self, w: &mut Writer, enc: TextEncoding) {
        w.chunk(CHUNK_VERSION, |w| w.w_u16(BONE_VERSION));
        w.chunk(CHUNK_DEF, |w| {
            w_str(w, &self.name, enc);
            w_str(w, &self.parent, enc);
            w_str(w, &self.wmap, enc);
        });
        w.chunk(CHUNK_BIND_POSE, |w| {
            w.w_vec3(self.rest_offset);
            w.w_vec3(self.rest_rotate);
            w.w_f32(self.rest_length);
        });
        self.write_data(w, enc);
    }

    /// `SaveData`.
    pub(crate) fn write_data(&self, w: &mut Writer, enc: TextEncoding) {
        w.chunk(CHUNK_DEF, |w| w_str(w, &self.name, enc));
        w.chunk(CHUNK_MATERIAL, |w| w_str(w, &self.game_mtl, enc));
        w.chunk(CHUNK_SHAPE, |w| self.shape.write(w));
        w.chunk(CHUNK_FLAGS, |w| w.w_u32(self.ik.flags));
        w.chunk(CHUNK_IK_JOINT, |w| self.ik.write_joint(w));
        w.chunk(CHUNK_IK_JOINT_BREAK, |w| {
            w.w_f32(self.ik.break_force);
            w.w_f32(self.ik.break_torque);
        });
        w.chunk(CHUNK_IK_JOINT_FRICTION, |w| w.w_f32(self.ik.friction));
        w.chunk(CHUNK_MASS, |w| {
            w.w_f32(self.mass);
            w.w_vec3(self.center_of_mass);
        });
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bone_round_trips() {
        let mut b = Bone { name: "bip01_spine".into(), parent: "bip01".into(), wmap: "Bip01_Spine".into(), ..Bone::default() };
        b.rest_offset = Vec3::new(0.0, 1.5, -0.25);
        b.rest_rotate = Vec3::new(0.1, 0.2, 0.3);
        b.rest_length = 0.4;
        b.game_mtl = "creatures\\medium".into();
        b.shape.kind = ShapeType::Sphere as u16;
        b.shape.sphere_radius = 0.2;
        b.ik.kind = JointType::Joint as u32;
        b.ik.limits[1].limit = Vec2::new(-0.5, 0.5);
        b.ik.flags = JointIk::BREAKABLE;
        b.ik.friction = 2.0;
        b.mass = 7.0;

        let mut w = Writer::new();
        b.write(&mut w, TextEncoding::Utf8);
        let bytes = w.into_bytes();
        let back = Bone::read(&Reader::new(&bytes), &mut Text::default()).unwrap();
        assert_eq!(back, b);
        assert!(back.shape.valid());
        assert_eq!(back.ik.joint_type(), Some(JointType::Joint));

        let mut w = Writer::new();
        w.w_u16(1);
        w.w_u16(0);
        for _ in 0..27 {
            w.w_f32(0.0);
        }
        assert_eq!(w.tell(), BoneShape::SIZE);
        let shape = BoneShape::read(&mut Reader::new(w.bytes())).unwrap();
        assert!(!shape.valid());
    }

    #[test]
    fn old_forms_swap_the_first_two_angles() {
        let mut w = Writer::new();
        w.w_stringz("Root");
        w.w_stringz("");
        w.w_stringz("root");
        w.w_vec3(Vec3::ZERO);
        w.w_vec3(Vec3::new(1.0, 2.0, 3.0));
        w.w_f32(0.5);
        let b = Bone::read_v0(&mut Reader::new(w.bytes()), &mut Text::default()).unwrap();
        assert_eq!(b.name, "root");
        assert_eq!(b.rest_rotate, Vec3::new(2.0, 1.0, 3.0));
        assert_eq!(b.mass, 10.0);
    }

    #[test]
    fn a_cut_or_unknown_bone_is_an_error() {
        let mut w = Writer::new();
        w.chunk(CHUNK_VERSION, |w| w.w_u16(9));
        assert!(matches!(Bone::read(&Reader::new(w.bytes()), &mut Text::default()), Err(ObjectError::Version { .. })));

        let mut w = Writer::new();
        Bone::default().write(&mut w, TextEncoding::Utf8);
        let bytes = w.into_bytes();
        // drop the tail: the shape chunk loses its end
        let cut = &bytes[..90];
        assert!(Bone::read(&Reader::new(cut), &mut Text::default()).is_err());
    }
}

//! The editor's source model: `.object` and the motion files that belong to it.
//!
//! Everything here is plain data and pure functions: a reader takes `&[u8]` and returns an
//! owned model, a writer returns `Vec<u8>`, nothing touches a file or shares state, so
//! hundreds of models load in parallel. Hostile input is an `Err`, never a panic.
//!
//! # The model
//!
//! - [`Object`] - a whole `.object`: flags, [`Surface`]s, [`Mesh`]es, [`Bone`]s, bone parts,
//!   embedded [`SMotion`]s or motion references, user data, the LOD reference and the stamps.
//!   - [`Object::read`]`(&[u8]) -> Result<Object>` / [`Object::write`]`(&self) -> Vec<u8>` - the
//!     file, byte-compatible with the X-Ray SDK's editor. Reading does what
//!     that editor does on load: bones are sorted into hierarchy order, vertex maps are rebuilt,
//!     the faces of every surface are sorted, mesh names are made unique.
//!   - [`Object::bounding_box`], [`Object::is_skeleton`], [`Object::is_dynamic`],
//!     [`Object::is_animated`], [`Object::surface_index`], [`Object::bone_index`],
//!     [`Object::bone_index_by_wmap`], [`Object::root_bone`], [`Object::verify_bone_parts`].
//! - [`Mesh`] - points, [`Face`]s, per-face smoothing flags, [`VMap`]s (UV and weight maps),
//!   the vertex-map references of every face corner and the face list of every surface.
//!   - [`Mesh::face_normals`]`() -> Vec<Vec3>` - the face normals.
//!   - [`Mesh::vertex_normals`]`() -> Vec<Vec3>` - the vertex normals: three per face, from the
//!     smoothing flags; parallel over faces.
//!   - [`Mesh::skin_vertices`]`(bones, influence, normals) -> Result<Vec<SkinVertex>>` -
//!     Position, normal, UV and up to four bone weights per face corner.
//!   - [`Mesh::face_uv`], [`Mesh::recompute_bbox`], [`Mesh::rebuild_vmaps`].
//! - [`Bone`] with [`BoneShape`] and [`JointIk`]; [`rest_pose`]`(bones) -> RestPose` and
//!   [`animate`]`(bones, states) -> Pose` are the rest pose and the animated pose of a skeleton.
//!
//! # Motions
//!
//! - [`Envelope`] / [`Key`] - a LightWave envelope; [`Envelope::evaluate`]`(time) -> f32` is
//!   exact (TCB, Hermite, Bezier, linear, stepped, every pre/post behaviour).
//! - [`SMotion`] - a skeletal motion: [`read_skl`] / [`write_skl`] (one motion),
//!   [`read_skls`] / [`write_skls`] (a set); [`SMotion::evaluate`]`(bone, time) -> (T, R)`.
//! - [`OMotion`] - an object motion: [`read_anm`] / [`write_anm`]; [`OMotion::evaluate`].
//!
//! # Math
//!
//! [`xmath`] holds the engine's matrix, quaternion and box formulas operation for operation;
//! the OGF exporter builds on it.
//!
//! # Text
//!
//! Names are `SmolStr` in UTF-8. A file whose strings are not UTF-8 is read as cp1251 and
//! remembered as such ([`Object::encoding`]), so writing it back gives the same bytes.

mod bone;
mod envelope;
mod io;
mod mesh;
mod motion;
mod object;
mod skeleton;
pub mod xmath;

/// The vector types the model is made of, for crates that build one.
pub use glam;

pub use bone::{Bone, BoneShape, JointIk, JointLimit, JointType, ShapeType};
pub use envelope::{Envelope, Key, behavior, shape};
pub use io::TextEncoding;
pub use mesh::{BoneWeight, Face, FaceVert, Mesh, MeshFlags, SkinVertex, SurfFaces, VMap, VMapKind, VMapPt};
pub use motion::{
    BoneMotion, Channel, MotionMark, OMotion, SMotion, SMotionFlags, read_anm, read_skl, read_skls,
    write_anm, write_skl, write_skls,
};
pub use object::{BonePart, Object, ObjectFlags, Surface, SurfaceFlags};
pub use skeleton::{BoneState, Pose, RestPose, animate, rest_pose};
pub use xmath::{Aabb, Mat4};

/// `BI_NONE`: no bone.
pub const BONE_NONE: u16 = u16::MAX;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ObjectError {
    #[error(transparent)]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("the file holds no object body")]
    NotAnObject,
    #[error("{what} has version {version:#06x}, which this editor does not read")]
    Version { what: &'static str, version: u32 },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = ObjectError> = std::result::Result<T, E>;

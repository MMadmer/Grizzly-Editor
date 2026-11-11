//! The engine's compiled model (`.ogf`) and motions (`.omf`).
//!
//! ```text
//! Visual::read(&[u8])                    -> every visual the game ships: normal, hierarchy,
//!                                           progressive, LOD, tree ST / PM, skeleton animated /
//!                                           rigid, skinned geometry ST / PM (1-4 links)
//! read_omf(&[u8]) / MotionSet::read_chunks -> bone parts, motion definitions, quantised keys
//! motion_set.to_omf()                    -> the same file back
//! motion_set.pose(motion, t) -> Vec<BoneKey>   one motion at time t, as the engine samples it
//! export_static(&Object, &ExportOptions) -> Vec<u8>   ExportObjectOGF.cpp
//! export_skeleton(&Object, &ExportOptions, influence) -> Export   ExportSkeleton.cpp
//!                                           (a progressive one through the VIPM pass, `vipm`)
//! export_breakable(&BreakablePart) -> Export   SBPart::Export (GeometryPartExtractor.cpp)
//! export_omf(&Object) -> Vec<u8>         CEditableObject::PrepareOMF
//! min_box / rapid_box -> Obb             ComputeOBB_WML / ComputeOBB_RAPID
//! ```
//!
//! Vertex data is decoded into one neutral [`Vertex`] (position, normal, tangent, binormal, two
//! UV sets, colour, up to four bones and weights) and shared as `Arc<[Vertex]>`. Readers take
//! `&[u8]`; nothing here touches a file or holds state. Hostile input is an `Err`.

mod bones;
mod export;
mod format;
mod motion;
mod obb;
mod optimize;
mod vertex;
mod vipm;
mod visual;

pub use bones::{BoneIk, BoneName, Obb};
pub use export::{
    BreakableBone, BreakableFace, BreakablePart, Export, ExportOptions, export_breakable, export_omf,
    export_skeleton, export_static,
};
pub use format::{Desc, Header, MT, chunk, crc32};
pub use motion::{
    BoneKey, BoneMotion, MotionDef, MotionMark, MotionSet, PartDef, RotKeys, TransKeys, read_omf,
};
pub use obb::{min_box, rapid_box};
pub use optimize::{optimize_faces, optimize_vertices};
pub use vertex::{Vertex, VertexBuffer, VertexFormat};
pub use visual::{BufferRef, LodFacet, LodVertex, SlideWindow, SlideWindows, TreeDef, Visual};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum OgfError {
    #[error(transparent)]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("the model has format version {0}; the game reads version 4")]
    Version(u8),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Export(String),
}

pub type Result<T, E = OgfError> = std::result::Result<T, E>;

pub(crate) fn invalid(what: impl Into<String>) -> OgfError {
    OgfError::Invalid(what.into())
}

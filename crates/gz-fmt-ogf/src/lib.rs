//! The engine's compiled model (`.ogf`) and motions (`.omf`).
//!
//! ```text
//! Visual::read(&[u8])                    -> every visual the game ships: normal, hierarchy,
//!                                           progressive, LOD, tree ST / PM, skeleton animated /
//!                                           rigid, skinned geometry ST / PM (1-4 links)
//! read_omf(&[u8]) / MotionSet::read_chunks -> bone parts, motion definitions, quantised keys
//! motion_set.to_omf()                    -> the same file back
//! motion_set.pose(motion, t) -> Vec<BoneKey>   one motion at time t, as the engine samples it
//! Skeleton::of(&Visual).model(&locals)   every bone in model space, the skinning of a pose
//! export_static(&Object, &ExportOptions) -> Vec<u8>   the static exporter
//! export_skeleton(&Object, &ExportOptions, influence) -> Export   the skeleton exporter
//!                                           (a progressive one through the VIPM pass, `vipm`)
//! export_breakable(&BreakablePart) -> Export   a breakable part's rigid skeleton
//! export_omf(&Object) -> Vec<u8>         the motions of an object as an `.omf`
//! restate(&[u8], &Restate) -> Vec<u8>    a compiled model with what its parts wear, its user
//!                                           data and its bones' physics changed, the rest as it was
//! min_box / rapid_box -> Obb             the minimal box and the moment box
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
mod restate;
mod skeleton;
mod vertex;
mod vipm;
mod visual;

/// The texture the game keeps for "there, but not drawn": a model whose every surface wears it
/// works - it stops a body, it is a container - and nobody sees it.
pub const UNDRAWN_TEXTURE: &str = "glow\\glow_ghost";

pub use bones::{BoneIk, BoneName, Obb};
pub use export::{
    BreakableBone, BreakableFace, BreakablePart, Export, ExportOptions, export_breakable, export_omf,
    export_skeleton, export_static,
};
pub use format::{Desc, Header, MT, chunk, crc32};
pub use motion::{
    BoneKey, BoneMotion, ESM_FX, ESM_STOP_AT_END, Motion, MotionDef, MotionMark, MotionSet, PartDef, RotKeys,
    SAMPLE_FPS, TransKeys, read_omf,
};
pub use obb::{min_box, rapid_box};
pub use optimize::{optimize_faces, optimize_vertices};
pub use restate::{BoneChange, Restate, Restated, restate};
pub use skeleton::{Skeleton, origin_of};
pub use vertex::{Vertex, VertexBuffer, VertexFormat};
pub use vipm::{Swr, VipmResult, vipm_convert};
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

//! Level geometry and navigation data: the files a level is made of besides its visuals, and a
//! triangle tree for the editor's own picking. Readers take `&[u8]` and return owned models
//! (bulk arrays behind `Arc`); writers return `Vec<u8>`; nothing here touches a file. Input is
//! hostile: a short file or a wild count is an `Err`, never a panic.
//!
//! Strings follow `gz-chunk`: read as UTF-8 when valid and cp1251 otherwise, written as UTF-8,
//! which is what the editor's own process holds.
//!
//! **Collision**
//! - [`CForm`] / [`Tri`] - `level.cform`: `CForm::read`, `CForm::write`, `CForm::new`.
//! - [`Collector`] - the collision collector: `add_face`, `add_face_packed` (welds like the X-Ray SDK, same
//!   order), `remove_duplicate_tris`, `calc_adjacency`, `into_cform`.
//! - [`CollectorPacked`] - the packed collector: the 24 x 16 x 24 welding grid, per-face flags.
//! - [`XcForm`] / [`XcTri`] - `overlay.xcform`: `XcForm::read` (as the game reads),
//!   `XcForm::read_back` (as the editor re-reads its own file), `XcForm::write`, `add_cuts`.
//! - [`OverlayVisuals`], [`VisualsWriter`], [`append_cuts`], [`sanitize_file_name`],
//!   [`unique_visual_name`] - the text registry `overlay_visuals.ltx`.
//! - [`Bvh`] / [`Ray`] / [`RayHit`] - the picking tree: `Bvh::build` (parallel), `ray_nearest`,
//!   `ray_visit`, [`ray_triangle`] (the walks' own triangle test),
//!   `ray_any`, `ray_all`, `rays_nearest`, `rays_any`, `query_box`, `query_sphere`,
//!   `query_frustum`.
//!
//! **AI map**
//! - [`LevelGraph`] / [`LevelGraphHeader`] / [`AiNode`] / [`NodeLayout`] - `level.ai`, versions
//!   8 to 13: `LevelGraph::read`, `LevelGraph::write`, position packing on the header.
//! - [`SceneAiMap`] / [`SceneAiNode`] / [`AiParams`] - the AI Map tool's chunk of a scene:
//!   `read`, `write`, `decode_nodes`, `encode_nodes` (to and from [`AiMapNode`]).
//! - [`BuildAiMap`] - `build.aimap`: `export`, `read`, `write`.
//! - [`export_level_graph`] - the `level.ai` the editor builds from the tool's nodes.
//!
//! **Details**
//! - [`LevelDetails`] / [`DetailHeader`] / [`DetailSlot`] / [`DetailModel`] - `level.details`
//!   and `build.details`: `read`, `write`, slot bit fields, `DetailSlot::for_export`.
//! - [`SceneDetails`] - the Detail Objects tool's chunk of a scene: `read`, `write`.
//! - [`DetailColorIndices`] / [`DetailObjectRef`] / [`ColorIndex`] - a `.dti` list: `read`, `write`.
//!
//! **What the level build writes**
//! - [`LevelGame`] / [`WayObject`] / [`RPoint`] - `level.game`.
//! - [`EnvMods`], [`read_static_sounds`], [`read_static_particles`], [`read_fog_volumes`] and
//!   their writers - `level.env_mod`, `level.snd_static`, `level.ps_static`, `level.fog_vol`.
//! - [`BuildProject`] and the `b_*` records ([`BuildFace`], [`BuildMaterial`], [`Portal`],
//!   [`BuildLight`], [`BuildGlow`], ...) - `build.prj`.
//!
//! **Arithmetic the formats share**
//! - [`Aabb`], [`i_floor`], [`i_ceil`], [`pv_compress`], [`pv_decompress`].

mod ai_map;
mod ai_scene;
mod build_prj;
mod bvh;
mod bytes;
mod cform;
mod collector;
mod details;
mod level_game;
mod math;
mod overlay_visuals;
mod xcform;

pub use ai_map::{
    AiNode, LevelGraph, LevelGraphHeader, NodeLayout, XRAI_VERSION_COP, XRAI_VERSION_MAX, XRAI_VERSION_SOC,
};
pub use ai_scene::{
    AIMAP_VERSION, AiMapNode, AiParams, BUILD_AIMAP_VERSION, BuildAiMap, EDITOR_HIGH_COVER, EDITOR_LOW_COVER,
    INVALID_NODE, PackedPos, SceneAiMap, SceneAiNode, compress_cover, export_level_graph, node_flags,
    nodes_bbox, renumber,
};
pub use build_prj::{
    BuildFace, BuildGlow, BuildLight, BuildLightDynamic, BuildLod, BuildMaterial, BuildParams, BuildProject,
    BuildTexture, Light, LightControl, LodFace, MAX_PORTAL_VERTS, MuModel, MuReference, Portal,
    XRCL_CURRENT_VERSION, chunk as build_chunk,
};
pub use bvh::{Bvh, Ray, RayHit, ray_triangle};
pub use cform::{CFORM_VERSION, CForm, Tri};
pub use collector::{Collector, CollectorPacked};
pub use details::{
    ColorIndex, DETAIL_SLOT_SIZE, DETAIL_VERSION, DETMGR_VERSION, DETOBJ_VERSION, DO_NO_WAVING,
    DetailColorIndices, DetailHeader, DetailModel, DetailObjectRef, DetailSlot, DetailVertex, LevelDetails,
    SceneDetails, tool_flags,
};
pub use level_game::{
    EnvModifier, EnvMods, FOG_VOL_VERSION, FogGroup, LevelGame, PS_STATIC_VERSION, RPOINT_CHUNK, RPoint,
    SPAWNPOINT_VERSION, StaticParticles, StaticSound, WAY_CHUNK, WAYOBJECT_VERSION, WayLink, WayObject,
    WayPoint, read_fog_volumes, read_static_particles, read_static_sounds, write_fog_volumes,
    write_static_particles, write_static_sounds,
};
pub use math::{Aabb, EPS, EPS_L, EPS_S, i_ceil, i_floor, pv_compress, pv_decompress};
pub use overlay_visuals::{
    CutEntry, OverlayVisuals, VisualEntry, VisualsWriter, append_cuts, sanitize_file_name, unique_visual_name,
};
pub use xcform::{CUT_TOLERANCE, XCFORM_MAGIC, XCFORM_VERSION, XcForm, XcTri, same_cut};

/// What can be wrong with a level file.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LevelError {
    #[error(transparent)]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("The {what} has version {found}, which is not supported.")]
    Version { what: &'static str, found: u32 },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = LevelError> = std::result::Result<T, E>;

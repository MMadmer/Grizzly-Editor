//! The level compiler's data model and its light, and the detail objects' light.
//!
//! Plain owned data with ids in place of pointers and no global: the global data of the level
//! compiler is an [`LcGlobalData`] the caller holds, the detail compiler's a [`DetailScene`].
//! Every long loop runs on the caller's rayon pool (the crate makes no pool and no thread),
//! takes a [`Ctl`] to stop and to report, and gives the same result on any number of threads:
//! a texel's samples come from the fixed jitter tables, a random number from the sample's own
//! position, results are gathered in input order. No module touches a file.
//!
//! **The model**
//! - [`Mesh`] / [`Vertex`] / [`Face`] / [`VertId`] / [`FaceId`] - the vertex and face lists:
//!   creation, adjacency, normals, checks, vertex isolation;
//!   [`Mesh::calc_normals`]; [`MeshErrors`].
//! - [`Shaders`], [`BuildTexture`], [`prepare_texture`], [`post_process_materials`], [`shader_of`].
//! - [`RLight`] / [`LightSet`] - the lights and their three sets:
//!   [`LightSet::from_project`],
//! - [`ColorC`] / [`BaseColor`] - `base_color_c`, `base_color`.
//! - [`LmLayer`], [`apply_borders`], [`blit`], [`blit_r`] - `lm_layer` and its blits.
//! - [`Deflector`] / [`Deflectors`] - the deflectors: their projection, the UV remap and the light.
//! - [`Lightmap`] / [`LightmapPage`] - the lightmap and the page surface.
//! - [`MuModel`] / [`MuReference`] - the multiple-usage models and their references.
//! - [`LcGlobalData`] - the global data and the calls the level compiler makes into it.
//!
//! **The ray model**
//! - [`RayModel`] / [`RayFace`] / [`LightCache`] - the ray model and the ray trace; [`RcastForm`] -
//!   `build.cform`; [`RapidCollector`] - the collector of the collision soup.
//!
//! **The light**
//! - [`Lighter`] / [`LitScene`] / [`Sample`] - the baker: every light value of a build is sample
//!   points lit in a scene, by [`CpuLighter`] (the reference) or by another lighter behind
//!   [`AutoLighter`].
//! - [`light_point`] - the light a point gets.
//! - [`Deflector::light`], [`Deflectors::light_all`] - the lightmaps of the deflectors.
//! - [`pack_lightmaps`] - the lit charts into pages, by visual; [`merge_lightmaps`] - each chart
//!   a visual of its own.
//! - [`light_vertices`] - the light of the vertices.
//! - [`implicit_lighting`] - the implicit light.
//! - [`light_mu`] - the MU models and references.
//! - [`light_details`] / [`slot_points`] - the detail objects' light.

mod bake;
mod batch;
mod bvh4;
mod color;
pub mod crt;
mod data;
mod deflector;
mod detail;
mod filter;
mod global;
mod hash2d;
mod implicit;
mod layer;
mod light;
mod lightmap;
pub mod math;
mod mesh;
mod mu;
mod point;
mod ray;
mod vertex;

#[cfg(test)]
mod tests;

pub use bake::{
    AutoLighter, CHUNK, CpuLighter, LightChoice, LightJob, Lighter, LitScene, NO_SKIP, NULL_LIGHT,
    NullLighter, Sample,
};
pub use batch::{BATCH, Gathered, Run, light_samples};
pub use color::{BaseColor, ColorC, fixed16_r, fixed16_w};
pub use data::{
    BuildMaterial, BuildParams, BuildTexture, NO_SHADER, QUALITY_DRAFT, ShaderXrlc, Shaders, TextureUse,
    default_shader, has_alpha_channel, post_process_materials, prepare_texture, shader_of, surface_of,
    texture_key,
};
pub use deflector::{
    Deflector, Deflectors, LMAP_SIZE, LightEnv, MAX_LAYER, UvTri, compress_rms, compress_zero, jitter,
    rms_average, rms_shrink, rms_test_flat, rms_test_size, rms_zero,
};
pub use detail::{
    DetailScene, DetailStats, LIGHT_COUNT, light_details, process_palette, slot_colors, slot_points,
};
pub use filter::{BLUR_GAUSS, build_kernel, process_kernel};
pub use global::{LcGlobalData, LightOutput};
pub use hash2d::Hash2D;
pub use implicit::{
    ImplicitInput, ImplicitMap, apply_implicit, has_implicit_lighting, implicit_lighting, implicit_shadows,
};
pub use layer::{BORDER, LmLayer, apply_borders, blit, blit_r, image_of, lblit, pixels_of};
pub use light::{
    LCONTROL_HEMI, LCONTROL_STATIC, LCONTROL_SUN, LT_DIRECT, LT_POINT, LT_SECONDARY, LightSet, R_LIGHT_SIZE,
    RLight,
};
pub use lightmap::{Lightmap, LightmapPage, PackReport, Rect, merge_lightmaps, pack_lightmaps};
pub use mesh::{
    BACKFACE_FLAG, Basis, DeflId, EDGE2IDX, EDGE2IDX3, Face, FaceFlags, FaceId, IDX2EDGE, Mesh, MeshErrors,
    NO_SM_GROUP, NO_VERT, PoolSim, Smoothing, Tcf, VertId, Vertex, area_of, barycentric,
    connect_by_edge_flags, is_inside, is_soft_edge,
};
pub use mu::{
    MuEnv, MuModel, MuReference, MuSamples, Subdiv, cache_opacity, light_mu, linear_regression,
    optimize_parameters, simple_optimize,
};
pub use point::{
    Falloff, LP_DEFAULT, LP_DONT_HEMI, LP_DONT_RGB, LP_DONT_SUN, LP_USE_FACE_DISABLE, Tracer, light_point,
};
pub use ray::{
    CFORM_VERSION, LightCache, NO_TRI, Opacity, RapidCollector, RayFace, RayModel, RcFace, RcastForm,
    build_ray_model, casts,
};
pub use vertex::{TransGroups, apply_vertex_light, light_vertices, mix_group};

use gz_task::{Cancel, Cancelled};

/// What can go wrong in the compiler's light.
#[derive(Debug, thiserror::Error)]
pub enum LcError {
    #[error("The compile was stopped.")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Level(#[from] gz_fmt_level::LevelError),
    #[error(transparent)]
    Tex(#[from] gz_fmt_tex::TexError),
    #[error(transparent)]
    Chunk(#[from] gz_chunk::ChunkError),
}

impl From<Cancelled> for LcError {
    fn from(_: Cancelled) -> Self {
        LcError::Cancelled
    }
}

pub type Result<T, E = LcError> = std::result::Result<T, E>;

/// What a long loop reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report<'a> {
    /// The stage it is in.
    Stage(&'a str),
    /// Items done of a total (`Progress`); may come from any worker, in any order.
    Progress { done: usize, total: usize },
}

/// How a long loop is stopped and how it reports: a job's cancel token and progress, adapted
/// by the caller (`gz_task::JobCtx::check` / `stage` / `progress`).
#[derive(Clone, Copy)]
pub struct Ctl<'a> {
    cancel: Option<&'a Cancel>,
    report: Option<&'a (dyn Fn(Report<'_>) + Sync)>,
}

impl std::fmt::Debug for Ctl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctl").field("cancellable", &self.cancel.is_some()).finish()
    }
}

impl<'a> Ctl<'a> {
    #[must_use]
    pub fn new(cancel: &'a Cancel, report: &'a (dyn Fn(Report<'_>) + Sync)) -> Self {
        Self { cancel: Some(cancel), report: Some(report) }
    }

    /// Never stopped, reports nowhere.
    #[must_use]
    pub fn quiet() -> Ctl<'static> {
        Ctl { cancel: None, report: None }
    }

    /// Stopped by `cancel`, reports nowhere.
    #[must_use]
    pub fn with_cancel(cancel: &'a Cancel) -> Self {
        Self { cancel: Some(cancel), report: None }
    }

    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(Cancel::is_cancelled)
    }

    /// `Err(Cancelled)` once stopped.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() { Err(LcError::Cancelled) } else { Ok(()) }
    }

    pub fn stage(&self, text: &str) {
        if let Some(r) = self.report {
            r(Report::Stage(text));
        }
    }

    pub fn progress(&self, done: usize, total: usize) {
        if let Some(r) = self.report {
            r(Report::Progress { done, total });
        }
    }
}

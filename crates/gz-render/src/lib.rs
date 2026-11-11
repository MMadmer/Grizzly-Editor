//! The editor's renderer: what the viewport, the previews and the thumbnails are drawn with.
//!
//! A forward renderer on `wgpu` with the editor's own materials (`docs/RUST_MIGRATION.md` 5.8,
//! deliberate difference D2): the viewport shows what an author needs to place and judge things -
//! geometry, textures, transparency, placement, selection; the look of the game is the game's.
//! No window and no egui: the crate takes the device and the queue the window draws with and
//! renders into textures it owns. It knows no scene either: its input is a [`DrawList`].
//!
//! # The API the UI packages use
//!
//! ```text
//! Renderer::new(device, queue, exec)          the render thread, the pipelines, the Assets
//! renderer.assets()                          -> &Arc<Assets>: handles, the loader, the caches
//! build_packet(&DrawList, &View, &Assets)    -> FramePacket   pure, on the CPU pool
//! renderer.submit(Arc<FramePacket>)          -> serial        never blocks; older packets drop
//! renderer.target()                          -> TargetHandle  the last finished frame to show
//! renderer.finished() / wait_frame(serial)   -> FrameInfo     the frame clock, the labels
//! renderer.capture(serial, timeout)          -> Image         screenshot_viewport, cropped
//! renderer.render_offscreen(&packet, size)   -> Image         thumbnails, previews, preview.png
//! ThumbnailCache::get_or_render(..)          -> Arc<Image>    a tile's picture, kept by content
//! ```
//!
//! - **The frame packet.** [`DrawList`] is a plain description - [`Instance`]s (mesh, material
//!   override, transform, flags, selection id, an optional pose), [`Light`]s, an optional
//!   [`Environment`] (the weather of the game-lighting mode) and [`Helpers`] (lines, boxes,
//!   spheres, cones, arrows, crosses, labels, each depth-tested or not). [`build_packet`] culls the
//!   instances by their world bounds (in parallel for large lists, or by a caller's
//!   [`Visibility`] set), groups what is visible by mesh and material into instanced draws, sorts
//!   the blended ones back to front and tessellates the helpers. The packet is immutable and
//!   self-contained: handles and instance data.
//! - **Geometry of the frame alone**: [`SpriteBatch`]es (particles, glows) become quads facing
//!   the packet's camera (`FillSprite`: camera-, axis- or path-aligned, turned, an atlas frame,
//!   culled by face as the engine culls), one draw per batch sorted with the blended ones;
//!   [`Batch`]es (detail objects) are many copies of one mesh the caller culled, drawn instanced
//!   without a test each; an [`Environment`]'s [`Sky`] is the weather's half box with its two
//!   cube maps blended (`dxEnvironmentRender::RenderSky`).
//! - **The render thread** owns the targets: it takes the newest packet, records the passes (in
//!   parallel chunks on the CPU pool when a pass is large), submits, waits for the GPU, bumps the
//!   frame counter and pings `Wake`. The UI never waits for it.
//! - **Assets** load off the frame: [`Assets`] hands out [`MeshHandle`]s, [`TextureHandle`]s and
//!   [`MaterialHandle`]s; a missing resource draws a placeholder (a grey box, a checker) while a
//!   job on the pools reads it through the caller's [`AssetLoader`], decodes and uploads it. BC1 /
//!   BC2 / BC3 DDS data goes to the GPU as it is when the device has `TEXTURE_COMPRESSION_BC`.
//!   The caches are LRUs bounded by bytes.
//! - **Materials**: engine shader names map through one [`MaterialTable`] to an
//!   [`EditorMaterial`]; the table is data with a default for an unknown name.
//! - **Camera maths** are pure functions: [`Camera`], [`Projection`], [`pick_ray`], [`project`],
//!   [`frame_fit`], [`Frustum`].
//! - **Meshes** come from `.object` ([`MeshData::from_object`]) and `.ogf`
//!   ([`MeshData::from_visual`]); skinned models are skinned on the GPU from a pose
//!   ([`object_pose`], [`visual_pose`]).
//!
//! # Passes
//!
//! Depth + opaque, alpha-tested, grid and depth-tested helper lines, sky, blended (back to front),
//! translucent helper solids, the selection outline (from an id target, exact for alpha-tested
//! geometry), label occlusion queries, overlays without depth. MSAA 4x. The target grows in 64 px
//! buckets and the view is its top-left corner; whatever reads it back crops to the render size.
//! Coordinates are the engine's: left-handed, Y up, a camera looks along +Z, clockwise faces are
//! front faces. Depth is reversed (1 at the near plane).

mod assets;
mod camera;
mod color;
mod draw_list;
mod frame;
mod gpu;
mod helpers;
mod material;
mod math;
mod mesh;
mod packet;
mod renderer;
mod skin;
mod targets;
mod texture;
mod thumbs;
mod view;

pub use assets::{
    AssetBytes, AssetLoader, AssetRef, AssetState, AssetStats, Assets, Budgets, MaterialHandle, MeshHandle,
    MeshKey, TextureHandle,
};
pub use camera::{Camera, Projection, aspect, frame_fit, pick_ray, project};
pub use color::Color;
pub use draw_list::{
    Batch, BatchInstance, DrawList, Environment, Helpers, Instance, InstanceFlags, Label, Light, LightKind, Line,
    RoofMap, Shape, ShapeKind, Sky, Sprite, SpriteAlign, SpriteBatch, SpriteCull, SpriteSize, Visibility,
};
pub use helpers::LineVertex;
pub use material::{EditorMaterial, MaterialDesc, MaterialTable, Pass};
pub use math::{Aabb, Frustum, Ray};
pub use mesh::{MeshData, MeshVertex, SubMesh};
pub use packet::{FramePacket, PacketStats, ScreenLabel, build_packet, build_packet_with};
pub use renderer::{FrameInfo, Renderer, TargetHandle};
pub use skin::{object_pose, object_rest, visual_pose};
pub use targets::{bucket, target_extent};
pub use texture::{TexelFormat, TextureData};
pub use thumbs::ThumbnailCache;
pub use view::{Grid, View, ViewFlags, ViewMode, ViewStyle};

/// The picture every read-back returns: RGBA8, rows top-down.
pub use gz_fmt_tex::Image;

/// What can go wrong while preparing or drawing a frame.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Object(#[from] gz_fmt_object::ObjectError),
    #[error(transparent)]
    Ogf(#[from] gz_fmt_ogf::OgfError),
    #[error(transparent)]
    Texture(#[from] gz_fmt_tex::TexError),
    #[error("the model has no geometry to draw")]
    EmptyMesh,
    #[error("{0}")]
    Invalid(String),
    #[error("a picture of {width} x {height} pixels cannot be drawn; a side must be from 1 to {max}")]
    Size { width: u32, height: u32, max: u32 },
    #[error("the graphics device did not finish the frame: {0}")]
    Device(String),
    #[error("the renderer has stopped")]
    Stopped,
    #[error("no frame was drawn in time")]
    Timeout,
}

pub type Result<T, E = RenderError> = std::result::Result<T, E>;

pub(crate) fn invalid(what: impl Into<String>) -> RenderError {
    RenderError::Invalid(what.into())
}

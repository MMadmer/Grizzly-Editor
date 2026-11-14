//! The Level tab's 3D view: the scene drawn by the renderer, the camera, picking, the transform
//! tools and the overlays (docs/UI_DESIGN.md, section 6, "Viewport").
//!
//! It holds the view's toolbar, the gizmo, every mode's gestures, the drawing of each kind, the
//! game lighting preview, and the handlers named in [`register`].
//!
//! # How it is put together
//!
//! - [`Viewport`] (an `Editor` service) holds the view state: the camera ([`camera::Nav`]), the
//!   view options, the transform tool and its snaps, Game Lighting, the render size, and the
//!   renderer. The pane, the MCP handlers and the scene's keys all work on it.
//! - **Scene snapshot -> `DrawList`** is the pure [`draw::build`], run on the CPU pool only when
//!   the scene's revision, the library, the camera or the view options changed. Visibility comes
//!   from the scene's own pick tree. The packet goes to `gz_render::Renderer::submit`; the UI
//!   shows `target()` and never waits for the scene.
//! - **Picking and tools read snapshots and emit [`gz_scene::SceneCmd`]s**: a click asks the pick
//!   tree on the CPU pool, a drag sends merged-undo transforms and seals the gesture on release.
//! - **The object tools' gestures** ([`objtools`]): the Sector tool's mesh paint and Box Pick,
//!   the Way tool's Points mode (click, marquee, move, add), Shape's Attach on Click - each a
//!   `gz_scene::objtools` edit, the same the tools' commands send.
//! - **The bulk tools' gestures** (`bulktools`): the AI map's brush stroke, the node click,
//!   marquee and gizmo drags, the detail slots' click and marquee, a wallmark laid (Ctrl moves the
//!   selected one), Delete and the selection keys - the `gz_scene::bulk::cmd` functions the
//!   `scene.ai_map`, `scene.details` and `scene.wallmarks` commands run.
//! - **Overlays are egui** over the picture: the pills, the axis gizmo, the transform gizmo, the
//!   stats pill, the labels the renderer reports visible, the safe rect, the marquee.
//! - **The extras of a pass** (`draw_extra`), on the same CPU task: particle systems playing
//!   ([`draw_particles`], a `gz-particles` system per particle object, stepped by the time since
//!   the last pass while it plays and its emitter is in view), the glows' sprites, the AI map
//!   ([`draw_aimap`], tile meshes made once per tile revision near the camera), detail objects
//!   ([`draw_details`], slots decompressed round the camera, drawn as batches) and static
//!   wallmarks ([`draw_wallmarks`]). While a system in view plays every request makes a pass.
//!
//! # API
//!
//! ```text
//! ViewportPane::new(editor, gpu)         the pane the Level tab embeds
//! pane.ui(ui, cx) / inactive(cx) / shortcut(cx, chord) / mcp_state()
//! view_menu(ui, cx)                      the View menu's viewport options (DrawViewportOptions)
//! capture_preview(editor).await          preview.png: a pass of its own, no helpers
//! set_place_item(editor, class, name)    what Place adds for a class (the Tools panel's pick)
//! placing(editor) / set_placing(editor)  whether a click adds (the Sector tool's Edit section)
//! register(reg)                          camera_get, camera_set, focus_object, view_mode,
//!                                        viewport_controls, viewport_navigation, game_lighting,
//!                                        frame_pacing, screenshot_viewport, particles_tool
//! play_particles(editor, names, on).await the Particles tool's Play / Stop
//! install(b)                             the viewport's commands on the Keyboard page
//! ```
//!
//! Widgets the kit lacks, local here: `pane::local_axis_gizmo` (the 64 pt clickable axis
//! gizmo) and `pane::local_stats_pill` (the Mono stats pill).

#![forbid(unsafe_code)]

mod bulktools;
pub mod camera;
mod cmd;
pub mod draw;
pub mod draw_aimap;
pub mod draw_details;
mod draw_extra;
pub mod draw_particles;
pub mod draw_wallmarks;
mod gesture;
pub mod gizmo;
pub mod keys;
pub mod lighting;
mod loader;
mod menus;
pub mod objtools;
mod pane;
mod service;
mod session;
pub mod spawn_anim;

pub use cmd::register;
pub use draw_particles::{any_playing as particles_playing, play as play_particles};
pub use keys::install;
pub use menus::view_menu;
pub use objtools::{placing, set_placing};
pub use pane::ViewportPane;
pub use service::{Action, Viewport, capture_preview, set_place_item};

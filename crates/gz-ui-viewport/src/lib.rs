//! The Level tab's 3D view: the scene drawn by the renderer, the camera, picking, the transform
//! tools and the overlays (docs/UI_DESIGN.md, section 6, "Viewport").
//!
//! The C++ specification is `UIRenderForm`, `UI_ViewportNavigation`, `CUI_Camera`, `CEStats`,
//! `CToolCustom`, `UIViewportToolbar`, the gizmo, every tool's controls, `SceneRender` and each
//! class's `Render`, `EGameLighting` and `GameLightingScene`, and the handlers named in
//! [`register`], whose wire this crate keeps.
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
//! - **Overlays are egui** over the picture: the pills, the axis gizmo, the transform gizmo, the
//!   stats pill, the labels the renderer reports visible, the safe rect, the marquee.
//!
//! # API
//!
//! ```text
//! ViewportPane::new(editor, gpu)         the pane the Level tab embeds
//! pane.ui(ui, cx) / inactive(cx) / shortcut(cx, chord) / mcp_state()
//! view_menu(ui, cx)                      the View menu's viewport options (DrawViewportOptions)
//! capture_preview(editor).await          preview.png: a pass of its own, no helpers
//! set_place_item(editor, class, name)    what Place adds for a class (the Tools panel's pick)
//! register(reg)                          camera_get, camera_set, focus_object, view_mode,
//!                                        viewport_controls, viewport_navigation, game_lighting,
//!                                        frame_pacing, screenshot_viewport
//! install(b)                             the viewport's commands on the Keyboard page
//! ```
//!
//! Widgets the kit lacks, local here: `pane::local_axis_gizmo` (the 64 pt clickable axis
//! gizmo) and `pane::local_stats_pill` (the Mono stats pill).

#![forbid(unsafe_code)]

pub mod camera;
mod cmd;
pub mod draw;
pub mod gizmo;
mod keys;
pub mod lighting;
mod loader;
mod menus;
mod pane;
mod service;
mod session;

pub use cmd::register;
pub use keys::install;
pub use menus::view_menu;
pub use pane::ViewportPane;
pub use service::{Viewport, capture_preview, set_place_item};

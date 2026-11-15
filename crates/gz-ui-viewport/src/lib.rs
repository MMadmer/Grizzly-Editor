//! The Level tab's 3D view: the scene drawn by the renderer, the camera, the mode control and
//! the pills, the gizmo, the drop target, the Paint modes' brush, and the overlays
//! (docs/UI_DESIGN.md, section 6, "Viewport").
//!
//! # How it is put together
//!
//! - [`Viewport`] (an `Editor` service) holds the view state: the camera ([`camera::Nav`]), the
//!   view mode, what is shown, the snapping and the gizmo's tool and space (the settings it keeps
//!   in the editor's settings file), Game Lit's weather, the armed thing, the drop's ghost, the
//!   brush, the render size, and the renderer. The pane, the commands and the actions all work
//!   on it.
//! - **Scene snapshot -> `DrawList`** is the pure [`draw::build`], run on the CPU pool only when
//!   the scene's revision, the library, the camera or the view changed. Visibility comes from the
//!   scene's own pick tree. The packet goes to `gz_render::Renderer::submit`; the UI shows
//!   `target()` and never waits for the scene.
//! - **The mode decides a left press** (`gz_scene::SceneMode`, the scene's view state): Select
//!   picks any visible, unlocked object, draws a marquee with Ctrl and drags the gizmo; Place puts
//!   down the armed thing ([`place`]); Paint paints the layer its panel picked (`bulktools`);
//!   Zones draws the visibility cells ([`objtools`]). Picks and placements run on the pools and
//!   send `gz_scene::SceneCmd`s or the scene's commands; a drag is one undo step.
//! - **The gizmo** ([`gizmo`]) is pure: its handles on the screen, the hit test, what a drag of
//!   each handle does in World or Local space.
//! - **Every pill row and key is an action** ([`actions`]) over the viewport's commands
//!   (`viewport.camera`, `viewport.view`, `viewport.play`, `viewport.gesture`, `shot.viewport`,
//!   `perf.frames`) or the scene's.
//! - **Overlays are egui** over the picture: the pills, the axis gizmo, the transform gizmo with
//!   its arc, the brush's ring, the stats pill, the labels the renderer reports visible, the safe
//!   frame, the marquee.
//! - **The extras of a pass** (`draw_extra`), on the same CPU task: particle systems playing
//!   ([`draw_particles`]), the glows' sprites, the AI map ([`draw_aimap`]), detail objects
//!   ([`draw_details`]) and static wallmarks ([`draw_wallmarks`]).
//!
//! # API
//!
//! ```text
//! ViewportPane::new(editor, gpu)          the pane the Level tab embeds
//! pane.ui(ui, cx) / inactive(cx) / shortcut(cx, chord) / mcp_state()
//! view_menu(ui, cx)                       the View menu's rows (view modes, framing)
//! capture_preview(editor).await           preview.png: a pass of its own, no helpers
//! Placeable, PlaceKind, arm, armed        what the Place panel drags and arms
//! brush / set_brush                       the Paint mode's brush, for its panel
//! placing / set_placing                   Place mode up or not
//! register(reg), install(b)               the commands; the actions and Preferences > Viewport
//! play_particles(editor, names, on).await particle systems played in the viewport
//! ```
//!
//! Widgets the kit lacks, local here: `pane::local_axis_gizmo` (the 64 pt clickable axis
//! gizmo) and `pane::local_stats_pill` (the Mono stats pill).

#![forbid(unsafe_code)]

pub mod actions;
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
pub mod lighting;
mod loader;
mod menus;
pub mod objtools;
mod pane;
pub mod place;
mod prefs;
mod service;
mod session;
mod shots;
pub mod spawn_anim;

pub use actions::install;
pub use cmd::register;
pub use draw_particles::{any_playing as particles_playing, play as play_particles};
pub use menus::view_menu;
pub use objtools::{placing, set_placing};
pub use pane::ViewportPane;
pub use place::{Kind as PlaceKind, Placeable, arm, armed};
pub use service::{Brush, ViewMode, Viewport, brush, capture_preview, set_brush};

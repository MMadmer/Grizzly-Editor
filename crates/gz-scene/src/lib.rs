//! The level scene without a window: the model, the files, undo, selection, the object library,
//! picking and the scene's commands.
//!
//! The C++ specification is `LevelEditor/Editor/Scene` (`EScene`, `SceneIO`, picking, snapping,
//! undo), `Entry/*` (the sixteen object classes), `Tools/*` (their tools' settings and IO),
//! `XrECore/Editor/Library.cpp`, `Recovery/EditorRecovery` and the handlers of
//! `Mcp/LevelMcpScene.cpp` / `Mcp/LevelMcpProps.cpp`, whose wire this crate keeps.
//!
//! # The model ([`model`])
//!
//! The open level is one [`SceneState`] in a `gz_store::Store` ([`SceneService::store`]):
//!
//! - [`SceneState::objects`] - every placed object of every class, `ObjId -> Arc<Object>`;
//!   [`SceneState::by_class`] keeps each class's objects in placement order (the C++ tool
//!   lists), [`SceneState::names`] makes a name unique per class;
//! - [`Object`] - [`ObjClass`], name, [`Transform`] (position, rotation in radians, scale, as
//!   the C++ stores them), [`ObjFlags`] (the persisted `co_flags` and the run-time visibility),
//!   `owner` (the group holding it), [`ModMarks`] (authored / removed) and [`ObjData`], one
//!   variant per class;
//! - [`SceneState::selection`], [`ToolSettings`] (per class: visible, read-only, the tool's
//!   options; the current target class, the snap list), [`BulkData`] (the AI map in tiles,
//!   detail objects, wallmarks: `Arc`'d, written back byte for byte until replaced).
//!
//! What a renderer needs is a read of a snapshot: [`SceneState::render_items`] gives per object
//! its class, world matrix, world bounds, visibility, selection and class data.
//!
//! # Commands ([`SceneCmd`])
//!
//! Every change is a [`SceneCmd`] sent to the store: place, delete, rename, transform (one or a
//! batch), select, show / hide, group / ungroup, set a property by path, the snap list, mod
//! marks, the spawn order, bulk data, load and save marks. Undo is the store's history.
//!
//! # Properties ([`props`])
//!
//! [`props::object_rows`] lists an object's rows (key, label, value, kind, limits, read-only)
//! the way `FillProp` builds them; [`props::merge`] folds a selection into one list ("mixed"
//! rows); [`props::set_text`] applies a value given as text. The Details panel draws the same
//! rows `property_inspector` answers.
//!
//! # Files ([`io`])
//!
//! [`io::read_scene`] reads every version the C++ reads (the LTX `.level` with its `.part`
//! files, the binary `.level`, the binary undo / recovery stream); [`io::write_scene`] writes
//! what the C++ `SaveLTX` writes; [`io::write_undo_stream`] what `SaveUndoStream` writes. A
//! section, key or chunk this crate does not understand is kept and written back.
//!
//! # Picking ([`pick`]), the library ([`library`]), recovery ([`recovery`])
//!
//! [`pick::ScenePick`] is a top-level tree over the objects' world boxes for one snapshot;
//! rays take their candidates from it and test each library object's own `Bvh` in object space.
//! [`library::Library`] loads `.object` files on demand and caches them.
//!
//! # The object tools ([`objtools`])
//!
//! The Sector, Portal, Way, Shape, Light and Fog Volume tools: their switches
//! ([`objtools::ObjToolState`] in [`ToolSettings::obj`]), their edits ([`SceneCmd::ObjTool`], one
//! undo step each, computed from snapshots and refused when the scene moved meanwhile), the
//! geometry (portals from the sectors' faces, a sector's volume, the level bound) and the commands
//! `sector_tool`, `portal_tool`, `way_tool`, `shape_tool`, `light_tool`, `fog_tool`.
//!
//! # The AI map, detail objects and wallmark tools ([`bulk`])
//!
//! Their panels' switches ([`bulk::BulkToolState`] in [`ToolSettings::bulk`]), their edits
//! ([`SceneCmd::Bulk`], one undo step each, computed from snapshots): the AI map's generation over
//! the snap objects' collision model, the brush, the link edits, smoothing and selection; the
//! detail slots from the base texture; wallmarks laid on the geometry - and the commands
//! `aimap_tool`, `details_tool`, `wallmark_tool`.
//!
//! # Registration
//!
//! [`register`] adds `open_scene`, `save_scene`, `scene_info`, `scene_stats`, `scene_tree`,
//! `scene_load_status`, `list_objects`, `object_info`, `select_objects`, `selection`,
//! `deselect_all`, `delete_selected`, `rename_object`, `object_set_position`,
//! `object_set_rotation`, `object_set_scale`, `place_object`, `place_spawn`, `place_way`,
//! `spawn_order`, `spawn_set_visual`, `spawn_set_fixed_bones`, `mark_authored`, `mark_removed`,
//! `drop_objects`, `snap_objects`, `scan_walls`, `probe_vertical`, `undo`, `redo`, `recovery`,
//! `property_inspector`, `property_set`, `property_choose`; [`install`] provides the `Objects`
//! and `Groups` categories of `list_assets`.

#![forbid(unsafe_code)]

pub mod apply;
pub mod bulk;
pub mod class;
mod cmd;
pub mod cmd_edit;
pub mod debug;
pub mod edit;
pub mod game;
pub mod io;
pub mod library;
pub mod ltx;
pub mod math;
pub mod model;
pub mod objects;
pub mod objmotion;
pub mod objtools;
pub mod pick;
pub mod props;
pub mod recovery;
pub mod selection;
mod service;
pub mod snap;
pub mod summary;
pub mod tools;

pub use apply::{SceneCmd, SceneReply};
pub use class::ObjClass;
pub use cmd::{install, register};
pub use model::{
    BulkData, ModMarks, ObjData, ObjFlags, ObjId, Object, RenderItem, SceneMeta, SceneState, ToolSettings,
    Transform,
};
pub use service::{LoadStatus, SceneFileHooks, SceneService};

/// Why a scene could not be read or written. The texts are the C++ `SetLoadError` sentences.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    #[error("{0}")]
    Load(String),
    #[error("{0}")]
    Save(String),
}

impl SceneError {
    pub(crate) fn load(text: impl Into<String>) -> Self {
        Self::Load(text.into())
    }
}

impl From<gz_chunk::ChunkError> for SceneError {
    fn from(e: gz_chunk::ChunkError) -> Self {
        Self::Load(e.to_string())
    }
}

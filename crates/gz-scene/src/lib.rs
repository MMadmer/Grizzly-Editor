//! The level scene without a window: the model, its files, undo, selection, the object library,
//! picking and the scene's commands.
//!
//! # The model ([`model`])
//!
//! The open level is one [`SceneState`] in a `gz_store::Store` ([`SceneService::store`]):
//!
//! - [`SceneState::objects`] - every placed object of every kind, `ObjId -> Arc<Object>`;
//!   [`SceneState::by_class`] keeps each kind's objects in placement order, [`SceneState::names`]
//!   makes a name unique in the scene (in any case; the case given is kept);
//! - [`Object`] - [`ObjClass`], name, [`Transform`] (position, rotation in radians, scale),
//!   [`ObjFlags`] (its flags, its eye, its lock), `owner` (the group holding it), [`ModMarks`]
//!   (authored / removed) and [`ObjData`], one variant per kind;
//! - [`SceneState::selection`] - one set of objects of any kind; [`ToolSettings`] - per kind its
//!   eye and lock, its viewport flags and settings; the [`model::SceneMode`] the viewport's
//!   clicks are in; [`BulkData`] - the painted layers (the AI map in tiles, detail objects,
//!   wallmarks).
//!
//! What a gizmo may do is the intersection of what the selected objects allow
//! ([`SceneState::selection_caps`], [`class::Caps`]); a hidden or locked object is not picked.
//!
//! # Commands ([`SceneCmd`])
//!
//! Every change is a [`SceneCmd`] sent to the store: place, delete, rename, transform, select,
//! show / hide, lock, group / ungroup, paste, set a property, mod marks, the painted layers, load,
//! reload and save marks. Undo is the store's history.
//!
//! # Files ([`format`], [`sdk`])
//!
//! A scene is `<name>.scene` and the folder `<name>\` of Lua data files and painted layers
//! ([`format`], `docs/assets/SCENE.md`): read into the model with the documents it came from, and
//! saved as the difference, so what nobody changed keeps its bytes. The X-Ray SDK's `.level` with
//! its `.part` files is read by the importer ([`sdk`]) and never written.
//!
//! # Properties ([`props`])
//!
//! [`props::object_rows`] lists an object's rows from the same tables the files use - key, label,
//! unit, value, limits, help - with what only the Details panel shows (buttons); [`props::merge`]
//! folds a selection into one list; [`props::parse`] reads a value given as text and
//! [`props::set_object`] applies it.
//!
//! # Picking ([`pick`]), the library ([`library`]), recovery ([`recovery`])
//!
//! [`pick::ScenePick`] is a top-level tree over the objects' world boxes for one snapshot;
//! rays take their candidates from it and test each library object's own `Bvh` in object space.
//! [`library::Library`] loads `.object` files on demand and caches them. Recovery snapshots are
//! the scene in its own format, now and then, in `backup\recovery`.
//!
//! # Registration
//!
//! [`register`] adds the `scene` area: `scene.open`, `scene.new`, `scene.save`, `scene.info`,
//! `scene.objects`, `scene.get`, `scene.set`, `scene.choices`, `scene.place`, `scene.select`,
//! `scene.delete`, `scene.duplicate`, `scene.copy`, `scene.paste`, `scene.group`,
//! `scene.ungroup`, `scene.align`, `scene.query`, `scene.mode`, `scene.attach`, `scene.motion`,
//! `scene.way`, `scene.shape`, `scene.recover`, `scene.convert` (and, from the painted layers'
//! and the zones' modules, theirs); [`install`] the library's lists, the project's hooks and the
//! `scene` and `group` asset kinds. A command that needs the scene when none is up loads the
//! project's scene first ([`need_scene`]).

#![forbid(unsafe_code)]

pub mod apply;
pub mod base;
pub mod bulk;
pub mod class;
mod cmd;
pub mod cmd_edit;
pub mod debug;
pub mod edit;
pub mod facts;
pub mod format;
pub mod game;
mod kind;
pub mod landing;
pub mod library;
pub mod math;
pub mod model;
pub mod names;
pub mod objects;
pub mod objmotion;
pub mod objtools;
pub mod pick;
pub mod props;
pub mod recovery;
pub mod sdk;
pub mod sense;
mod service;
pub mod summary;
pub mod tools;

pub use apply::{SceneCmd, SceneReply};
pub use class::ObjClass;
pub use cmd::{choices, choices_reply, install, prop_context, register, selection_rows};
pub use model::{
    BulkData, ModMarks, ObjData, ObjFlags, ObjId, Object, RenderItem, SceneMeta, SceneMode, SceneState,
    ToolSettings, Transform,
};
pub use service::{LoadStatus, SceneFileHooks, SceneService, need_scene};

/// Why a scene could not be read or written: a sentence for the author.
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

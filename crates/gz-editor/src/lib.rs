//! The editor without a window.
//!
//! [`Editor`] is the context every command and every surface receives: the pools, the log, the
//! jobs, the frame clock, the open project. It names no area of the editor: an area (quests,
//! characters, builds, the scene ...) lives in its own crate, keeps its state in services it
//! adds to the context, and registers its commands in the [`Registry`]. So this crate changes
//! rarely, and an edit to an area rebuilds that area's crate and the final link only.
//!
//! - [`Editor`], [`ProjectSession`], [`Services`] - the context and what hangs on it
//! - [`Registry`], [`Args`], [`Reply`] - commands by name: what MCP calls and the UI emits
//! - [`FrameClock`] - what a request waits on when its answer is a drawn frame
//! - [`Notices`] - messages for the author raised off the UI thread
//! - [`DirtySource`] - how an area says what unsaved work it holds
//! - [`AutoSave`] - a store's document written to disk as it is edited; [`fsx`] - file helpers
//! - [`ViewCamera`] - the viewport's camera, read by surfaces that must not depend on it

mod args;
mod camera;
mod editor;
mod frames;
pub mod fsx;
mod notices;
mod registry;
mod saver;
mod services;

pub use args::{Args, ArgsExt};
pub use camera::{CameraPose, ViewCamera};
pub use editor::{
    AppPaths, DirtyDoc, DirtySource, Editor, EditorOptions, Flags, ProjectSession, SettleFuture,
};
pub use frames::{FrameClock, FrameStamp};
pub use notices::{Notice, NoticeKind, Notices};
pub use registry::{CommandInfo, Registry, Reply, error_reply, ok_reply};
pub use saver::AutoSave;
pub use services::Services;

pub use gz_base::{Origin, Problem, RelPath, Rev, Severity, SmolStr};
pub use gz_log::Log;
pub use gz_store::{CmdError, Store};
pub use gz_task::{Cancel, Executors, JobCtx, JobError, JobId, JobInfo, JobState, Jobs};
pub use serde_json::{Value, json};

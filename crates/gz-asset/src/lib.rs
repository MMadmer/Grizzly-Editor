//! The editor's text assets as one list of kinds.
//!
//! An asset is a text file an agent edits with its own tools (`docs/ASSETS.md`). This crate is
//! what every kind shares:
//!
//! - [`Kinds`] / [`KindOps`] - the list: each kind's extension, header key and version (the
//!   editor's own kinds take them from `gz-project`'s kind table), its migrations, its check,
//!   its reference and its diff rules. The kind crates (`gz-nq`, `gz-characters`, `gz-pda`)
//!   add their entries when they install.
//! - [`AssetFile`] with [`read`] / [`read_text`] - the one read path: the header against the
//!   extension (`E001`), a newer version refused (`newer_version`), an older one migrated in
//!   memory, the keys a kind does not know listed as notes - and [`save`] / [`write`] /
//!   [`write_text`] - the one save path: `apply_diff`, a missing header added, an atomic write
//!   past the write guard, only when the text changed, the editor's own write told to the
//!   project's watcher.
//! - [`at`] / [`at_path`] - a problem positioned at what a document path names.
//! - [`register`] - `asset.kinds`, `asset.check`, `asset.reference`, `asset.find`,
//!   `asset.migrate`, `asset.take_theirs` and the `assets` topic.

#![forbid(unsafe_code)]

mod check;
mod cmd;
mod file;
mod kinds;

pub use check::{Counts, MANIFEST, Report, check_file, check_manifest, run as check_run, targets};
pub use cmd::{CheckParams, FailOn, FindParams, MigrateParams, ReferenceParams, TakeTheirsParams, register};
pub use file::{
    AssetFile, ReadFailure, at, at_path, other_headers, read, read_text, save, write, write_text,
};
pub use kinds::{CheckFn, KindOps, Kinds, ManifestCheckFn, Migration, ReferenceFn, TakeTheirsFn};

#[cfg(test)]
mod tests;

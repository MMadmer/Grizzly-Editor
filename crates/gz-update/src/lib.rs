//! The editor updates itself (docs/UPDATES.md).
//!
//! A release is one archive; the client reads out of it, by HTTP range, only the files its
//! installation does not hold, stages them beside the installation and swaps them in with a
//! journalled transaction of renames when the editor exits.
//!
//! - [`check`](service): the latest release, the archive's end and directory and its manifest by
//!   range ([`prepare`], [`zipdir`]), the delta against the installation ([`delta`], [`hashes`])
//! - download: the delta's records verified into `.update\<version>\` ([`record`], [`stage`]),
//!   resumable inside a record, continuing by itself ([`schedule`]); a server without ranges
//!   gets the whole archive
//! - apply: the stage verified again, the journalled swap and its rollback ([`apply`]), the
//!   restart handshake, the cleanup at the next start
//!
//! The installation is the folder of the running executable when it holds `update-manifest.txt`;
//! without one this is a development build and the updater is off: it contacts nothing. Under
//! `-qa_github` only, `GRIZZLY_UPDATE_ROOT` names the installation and `GRIZZLY_UPDATE_REPO` the
//! repository, read from the mock alone.
//!
//! [`register`] adds the `update.*` commands; [`install`] puts the [`UpdateService`] on the editor
//! at start; [`ready`] says a run without a window is up; [`at_exit`] applies at the exit.

pub mod apply;
mod cmd;
pub mod delta;
pub mod hashes;
pub mod manifest;
pub mod net;
pub mod prepare;
pub mod record;
pub mod schedule;
mod service;
pub mod stage;
mod sys;
pub mod version;
pub mod zipdir;

use std::sync::Arc;

use gz_editor::Editor;

pub use cmd::{register, status};
pub use service::{Exit, Install, REPOSITORY, Start, State, Toast, UpdateService, install, size_text};
pub use version::Version;

/// A run without a window is up: the editor an update restarted into reports it.
pub fn ready(editor: &Arc<Editor>) {
    if let Some(svc) = UpdateService::of(editor) {
        svc.report_started(editor);
    }
}

/// The editor ends: a staged update is applied when asked for, and a restart starts the new
/// version and waits for its report. Call after the window and the project closed.
pub fn at_exit(editor: &Arc<Editor>) {
    if let Some(svc) = UpdateService::of(editor) {
        svc.at_exit(editor);
    }
}

#[cfg(test)]
mod tests;

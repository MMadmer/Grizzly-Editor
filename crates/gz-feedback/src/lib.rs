//! The editor's feedback to its author (docs/REPORTS.md): a bug report or a suggestion, written by the
//! author in the editor's words, with an anonymous diagnostic package when the author lets it go, and
//! a crash of the editor offered at the next start - sent to the report hub the game's reports go to,
//! as `product = grizzly-editor`.
//!
//! - [`sanitize`]: what a report says of the machine - nothing that names the person or the computer
//! - [`minidump`]: a dump made anonymous the way the game makes its own
//! - [`crashes`]: what crashes left in the log folder, offered once, the ten newest kept
//! - `report`: the package - `report.json` with the keys the hub reads, the editor's state, its
//!   log, its settings and the crash's files, inside the hub's limits
//! - `send`: the upload, with the build's key of the hub; a QA run speaks to this computer alone
//!
//! [`register`] adds `feedback.*`; [`install`] puts the [`FeedbackService`] on the editor at start;
//! [`provide`] lets a layer that knows more (the graphics card, the window's language) say it into
//! every package.

pub mod crashes;
pub mod minidump;
pub mod sanitize;

mod cmd;
mod report;
mod send;
mod service;
mod sys;
mod zipw;

pub use cmd::{register, status};
pub use report::{Kind, SCHEMA};
pub use service::{FeedbackService, Offer, install, provide};

/// Whether this build can send feedback: a released build carries the hub's key, a QA run speaks to
/// its mock. Reads nothing: the window asks it every frame.
#[must_use]
pub fn available(editor: &gz_editor::Editor) -> bool {
    send::Target::available(editor.flags.qa_github)
}

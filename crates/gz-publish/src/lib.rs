//! Publish Release and the Releases tab's data: a packaged module put out as a GitHub release of
//! the manifest's `[update] github` repository - what the game's Mods menu updates from - and
//! what that repository already holds.
//!
//! The publish job packages through `gz-release`, scans a listed module's packages, then puts a
//! draft on GitHub, checks every stored asset against the package and makes it the latest; the
//! network is `gz-github`.
//!
//! The domain (pure):
//!
//! - [`tags`] - [`tags::fresh_tag`] (`v<version>`, `-r2`, `-r3`, ... while the plainer ones were
//!   deleted within the hour), [`tags::valid_tag`], [`tags::default_title`], the deleted tags file.
//!
//! The services and the commands:
//!
//! - [`Publisher`] (`editor.services`) - the job `publish`: the packaging (group `build`, so no
//!   build runs beside it), then the scan of a listed module and the upload (group `publish`).
//!   [`Publisher::start`], [`Publisher::status_json`], [`Publisher::running`],
//!   [`Publisher::button_text`] for the toolbar; the device sign-in ([`Publisher::sign_in`]).
//! - [`ReleaseList`] (`editor.services`) - the releases the repository holds, drafts included:
//!   [`ReleaseList::start`] (list, edit, delete - the release first, its tag second),
//!   [`ReleaseList::json`], the Releases tab's selection and the bits the UI sets
//!   ([`ReleaseList::set_tab_open`], [`ReleaseList::set_publish_dialog_open`],
//!   [`ReleaseList::take_edit_request`]).
//! - [`register`] - `release.publish`, `release.account`, `release.list`, `release.edit`,
//!   `release.delete`. They ask nothing.

mod cmd;
mod job;
mod manage;
mod scan;
mod sign;
pub mod tags;

pub use cmd::register;
pub use job::{JobView, PublishOptions, PublishState, Publisher};
pub use manage::{ManageOp, ManageRequest, ManageState, ManageView, ReleaseList, Setup};
pub use sign::{SignState, SignView};

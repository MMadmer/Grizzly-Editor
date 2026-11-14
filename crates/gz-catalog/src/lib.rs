//! Submit to Mod Browser: the module listed in the in-game Mod Browser of Dead Air: Refined (game
//! repo `docs/dead-air/MOD_CATALOG.md`, sections 7 and 10).
//!
//! The author key and the card are `gz-release`'s; the network, the scan and the issue mechanics
//! are `gz-github`'s.
//!
//! A submission is an ISSUE `Submit <id>` of the catalog repository, never a pull request; its
//! body carries exactly the text of `mods/<id>.ltx` in a fenced `ini` block. One open submission
//! per module and account (submitting again edits it), nothing sent when the default branch
//! already holds this text. The verdict in force is the newest comment written by
//! `github-actions[bot]`. A withdrawal is the same mechanics and needs neither key. Nothing here
//! acts on another author's module or on the catalog repository beyond those issues.
//!
//! The domain (pure):
//!
//! - [`website`] - [`website::website_is_module_page`]: the module's own page on AP-PRO or ModDB.
//! - [`model`] - the release as the game reads it ([`model::Release`]), the check's findings
//!   ([`model::Check`]), the listing ([`model::read_listing`]), the `required` level that is never
//!   shown as optional ([`model::vt_level`]), the submission's file and details
//!   ([`model::submission_text`], [`model::submission_details`], [`model::withdrawal_details`]).
//!
//! The service and the command:
//!
//! - [`Catalog`] (`editor.services`) - one job at a time (`check`, `vt`, `submit`, `submission`,
//!   `withdraw`; group `catalog`), started by the window or MCP alike ([`Catalog::start`]); the
//!   view the window and `status` show ([`Catalog::view`], [`Catalog::status_json`]); the
//!   window's open state for the UI ([`Catalog::set_window`], [`Catalog::wants_withdraw_question`]).
//! - [`register`] - `release.catalog_status` (read) and `release.catalog` (scan, submit, withdraw:
//!   what reaches the public); [`install`] - a project that closes takes the window and the
//!   results with it.

mod cmd;
mod jobs;
pub mod model;
mod service;
pub mod website;

pub use cmd::{install, register};
pub use service::{Catalog, Job, JobState, View};

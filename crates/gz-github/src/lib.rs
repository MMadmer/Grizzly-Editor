//! The network side of the release tools: the GitHub REST client, where the account token comes
//! from, the device sign-in, the publication of a packaged release, the Mod Browser catalog's
//! issues and VirusTotal with the author's own key.
//!
//! What is here: the client, the account, publishing, the release
//! list, the catalog requests, VirusTotal and its pacing, the scan and
//! the VirusTotal policy (the verdicts, the index keys, the
//! catalog files' ini text). Nothing here touches the editor's state: the areas that use it -
//! Publish Release (`gz-publish`) and Submit to Mod Browser (`gz-catalog`) - keep their own.
//!
//! The domain (pure):
//!
//! - [`secret`] - [`Secret`]: a token or a key, wiped when dropped, never printed.
//! - [`endpoints`] - [`Endpoints`]: GitHub, VirusTotal and the catalog, or the QA mock under
//!   `-qa_github`; [`Endpoints::upload_url_trusted`] - the check before a token follows an
//!   address that came out of a response body.
//! - [`ini`] - the catalog files' ini text ([`ini::section`], [`ini::value`], [`ini::unquote`],
//!   [`ini::compare_versions`], [`ini::listed_key`], [`ini::withdrawn_key`]).
//! - [`catalog`] - the issue mechanics: [`catalog::submission_body`],
//!   [`catalog::withdrawal_body`], [`catalog::parse_verdict`], [`catalog::newest_verdict`] (the
//!   catalog's bot alone counts, the newest wins), [`catalog::CatalogIssue`].
//! - [`vt`] - [`vt::VtPolicy`] and [`vt::verdict`] (MOD_CATALOG 11), [`vt::reserve`] (the free
//!   API's pacing), the VirusTotal key rules.
//! - [`scan`] - the author's scan of a release: [`scan::Package`], [`scan::Scan`],
//!   [`scan::refuses`], [`scan::deciding`], [`scan::describe`], [`scan::json`].
//!
//! The service: [`Github`] (`editor.services`) - one HTTP client set per session (`reqwest` on
//! the Windows certificate store), the token sources ([`Github::resolve_account`]: the
//! environment, the Credential Manager, `gh auth token`), the device sign-in, [`Github::publish`]
//! (draft, uploads checked against the package, then latest), [`Github::list_releases`],
//! [`Github::edit_release`], [`Github::delete_release`], [`Github::fetch_public`], the catalog
//! ([`Github::fetch_catalog_index`], [`Github::catalog_submit`], [`Github::catalog_withdraw`],
//! [`Github::catalog_issue`]) and VirusTotal ([`Github::vt_lookup`], [`Github::vt_upload`],
//! [`Github::lookup`] and [`Github::scan`] over a release's packages). Every request takes the
//! job's [`gz_editor::Cancel`] and stops when it is set.

mod account;
pub mod catalog;
mod cred;
pub mod endpoints;
mod github;
mod http;
pub mod ini;
mod json;
pub mod scan;
pub mod secret;
mod service;
pub mod vt;

pub use account::{Account, AccountSource, plausible_token};
pub use catalog::{CatalogIssue, CatalogRequest, CatalogWithdrawal, IssueKind};
pub use endpoints::Endpoints;
pub use github::{
    Asset, DeleteResult, DeviceCode, DevicePoll, PublishRequest, PublishResult, PublishSink, ReleaseInfo,
};
pub use scan::{Package, Scan, ScanResult};
pub use secret::Secret;
pub use service::Github;
pub use vt::{VtPolicy, VtReport, VtVerdict};

/// The answer of a call that was stopped.
pub const CANCELLED: &str = "cancelled";

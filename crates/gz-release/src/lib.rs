//! Packaging a release: what turns a module into the files the game's updater and the Mod
//! Browser install from - and the author key that vouches for them.
//!
//! **The formats are FROZEN**: the normative texts are
//! the game repository's `docs/dead-air/MOD_UPDATES.md` (section 2) and `MOD_CATALOG.md`
//! (3, 4, 4.1, 6, 8, 10.3).
//!
//! The domain (pure, or file work that is the function's stated job):
//!
//! - [`format`] - the asset names ([`format::descriptor_name`], [`format::index_name`],
//!   [`format::package_name`]), [`format::package_path_ok`], [`format::release_gate`],
//!   [`format::resolve_release_dir`], [`format::split_parts`] (512 MiB of input per package),
//!   [`format::index_text`] and [`format::descriptor_head`] - the frozen texts.
//! - [`zip`] - the package: [`zip::ZipWriter`] (level 9 raw deflate, UTF-8 names flagged, ZIP64
//!   where needed) and [`zip::read_layout`] - the central directory and every local header read
//!   back, so the index states what is ON DISK.
//! - [`card`] - [`card::card_section`]: the descriptor's `[card]`, string ids resolved from the
//!   project's tables ([`card::StringTables`]).
//! - [`crypto`] - SHA-256, HMAC, PBKDF2, the key file's stream, ECDSA P-256 ([`crypto::EcKey`],
//!   [`crypto::PublicPoint`], [`crypto::verify`]); [`b64`] - base64url, hex.
//! - [`keyfile`] - the `xms-key 1` file, protected (PBKDF2-SHA256 600000, the HMAC-SHA256 stream,
//!   the MAC) or not: [`keyfile::parse`], [`keyfile::KeyFile::open`], [`keyfile::text`].
//! - [`signed`] - [`signed::sign_text`] / [`signed::verify_text`] (MOD_CATALOG 3.1).
//! - [`content`] - MOD_CATALOG 8 and 10.3 ([`content::check_content`], [`content::path_allowed`],
//!   [`content::bytes_allowed`], [`content::script_report`]): the catalog's `catalog.py`, ported.
//! - [`browser`] - the Mod Browser's tags ([`browser::TAG_VOCABULARY`], [`browser::parse_tags`])
//!   and thumbnail ([`browser::apply_thumbnail`], [`browser::make_thumbnail`]).
//! - [`game_version`] - the linked game's version from `xrEngine.exe` (the default
//!   `requires_game`); [`documents`] - the user's Documents folder.
//!
//! The services and the commands:
//!
//! - [`AuthorKeys`] (`editor.services`) - the author key of this Windows user: its state, create
//!   (never overwrites, never inside a project), use another file, unlock, lock, sign. The opened
//!   key is wiped when the editor closes and never reaches a log or an answer.
//! - [`Releases`] (`editor.services`) - the job `package` (group `build`: one build-like job at a
//!   time), its Task Center row ([`Releases::status`]) and its answers; [`run_package`] is what
//!   Publish Release calls inside its own job; [`packaged_files`] - what an earlier packaging
//!   of the version left, the data of the window's "replace?" question.
//! - [`media`] - the media assets in the module build ([`media::MediaBuild`]): baked and
//!   published under the game's names, their sources never shipped where they lie.
//! - [`install`] - fills `gz_mod::ManifestArgs` (`tags` of `mod.set`) and `gz_build`'s media
//!   publisher.
//! - [`register`] - `mod.package`, `mod.thumbnail`, `release.key`; [`key_state`] is the key half
//!   the Mod Browser area answers with.

pub mod b64;
pub mod browser;
pub mod card;
mod cmd;
pub mod content;
pub mod crypto;
pub mod documents;
pub mod format;
pub mod game_version;
pub mod keyfile;
mod keys;
pub mod media;
mod package;
pub mod signed;
pub mod zip;

pub use cmd::{key_state, package_answer, register, unlock_with};
pub use format::Asset;
pub use keys::{AuthorKeys, KeyState};
pub use package::{PackageRequest, PackageResult, Releases, packaged_files, run_package};

/// What this area plugs into the others once the editor exists: the `tags` of `mod.set`, the
/// module build's media publisher.
pub fn install(editor: &gz_editor::Editor) {
    browser::install_manifest_args(editor);
    gz_build::BuildChecks::of(editor).set_media(std::sync::Arc::new(media::MediaBuild));
}

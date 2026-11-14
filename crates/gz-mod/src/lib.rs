//! The module manifest: `mod.ltx`, the one document every editor of the module writes through.
//!
//! The domain (pure):
//!
//! - [`doc`] - [`ManifestDoc`]: `mod.ltx` as written, edited line by line. An edit touches its
//!   lines only; the author's comments, order and spacing survive. It reads lines the way the
//!   game reads them: `;` starts a comment outside a quoted value (`\"`
//!   escapes inside), `//` does not (a website is `https://...`).
//! - [`model`] - [`Manifest`]: the typed, read-only view of every section this crate knows
//!   (`[module]`, `[update]`, `[requires]`, `[order]`, `[conflicts]`, `[provides_mode]`,
//!   `[exclude]`, `[vars]`, `[pda] layout`, `[commands]` and `[command.<name>]`). Sections owned
//!   by later areas (`[world]`, `[start_kit]`, `[screens]`, `[screen.*]`, `[track.*]`, `[pda]`)
//!   are read raw through [`ManifestDoc::section_entries`] / [`ManifestDoc::section_lines`] and
//!   written with [`Edit::SetLines`] / [`Edit::Set`] - this crate does not know their meaning.
//!   [`scaffold_text`], [`derive_id`].
//! - [`edit`] - [`Edit`]: the commands of the manifest store, applied to a document.
//! - [`rules`] - the metadata rules the game enforces too ([`validate_id`], [`validate_version`],
//!   [`validate_website`], [`validate_github_repo`], [`normalize_github_repo`], ...), and
//!   [`validate`]: the manifest's own check before a build.
//! - [`commands`] - console commands: [`valid_command_name`], [`scan_script_functions`],
//!   [`validate_commands`] (`E401`..`E406`, `W406`..`W408`).
//! - [`exclude`] - what the build leaves out: [`exclude_match`], [`outside_build`],
//!   [`ship_state`], [`set_excluded`].
//! - [`text`] - the quoting of `name` / `author` / `description`, list splitting, cp1251 coverage.
//!
//! The service and the commands:
//!
//! - [`ManifestService`] (project scope, `session.services`): a `Store<ManifestState>` saved as
//!   it is edited ([`gz_editor::AutoSave`]). The store's revision IS the manifest generation.
//!   A change of `mod.ltx` behind the editor (git, an agent) comes through the project's
//!   watcher and loads the file as a new document.
//!   [`ManifestService::outside_build`] and [`ManifestService::globals`] are the answers other
//!   areas ask for; [`command_check`] the functions and problems of `[commands]`.
//! - [`ManifestFields`] and [`ManifestArgs`] - what other areas add to the manifest's answer
//!   (the `world` field) and to `mod.set` (`tags`, `pda.layout`).
//! - [`sections`] - [`OwnSections`]: the sections the module itself brings, the other half of
//!   "does the game, with this module on, have the section X?".
//! - [`install`] - the project scaffold hook (a new project gets its `mod.ltx`); [`register`] -
//!   `mod.set`, `mod.exclude`.

mod cmd;
pub mod commands;
pub mod doc;
pub mod edit;
pub mod exclude;
mod io;
pub mod model;
pub mod rules;
pub mod sections;
mod service;
pub mod text;

pub use cmd::{install, manifest_answer, manifest_json, manifest_problems, open_manifest, register};
pub use commands::{
    has_script_function, scan_script_functions, valid_command_function, valid_command_name, validate_commands,
};
pub use doc::ManifestDoc;
pub use edit::Edit;
pub use exclude::{ShipState, exclude_match, normalize_rel, outside_build, set_excluded, ship_state};
pub use io::{installed_commands, read_manifest_file, scaffold, script_functions};
pub use model::{Command, FILE, GlobalVar, Manifest, ProvidesMode, derive_id, scaffold_text};
pub use rules::{
    normalize_github_repo, valid_global_name, validate, validate_github_repo, validate_id, validate_mode_id,
    validate_version, validate_website,
};
pub use sections::{OwnSections, SectionSet, own_sections};
pub use service::{
    ArgCx, ArgHook, FieldHook, ManifestArgs, ManifestCmd, ManifestFields, ManifestService, ManifestState,
    command_check,
};

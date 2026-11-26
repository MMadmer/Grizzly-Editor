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
//! - [`reference`] - the format of `mod.ltx` as text ([`reference_text`], what
//!   `asset.reference kind=manifest` answers), held to the model by its tests.
//! - [`rules`] - the metadata rules the game enforces too ([`validate_id`], [`validate_version`],
//!   [`validate_website`], [`validate_github_repo`], [`normalize_github_repo`], ...), the
//!   values the reference judges ([`accepts`], [`refusal`]: what `mod.set` and the form turn away
//!   with), and [`validate`]: the manifest's own check, at its lines, before a build.
//! - [`commands`] - console commands: [`valid_command_name`], [`scan_script_functions`],
//!   [`validate_commands`] (`E401`..`E406`, `W406`..`W408`).
//! - [`exclude`] - what the build leaves out: [`exclude_match`], [`outside_build`],
//!   [`ship_state`], [`set_excluded`].
//! - [`radio`] - what a radio is told to play ([`radio::binding`]): `game`, `off`, a playlist of
//!   this module by its id or `<module>.<playlist>`; the scene's radios and the quests' radio
//!   actions read it the same.
//! - [`spot`] - [`Spot`]: where in `mod.ltx` a problem of an area is about (a section, a key, a
//!   layer's key), carried by the problem and located by [`ManifestDoc::locate`].
//! - [`text`] - the quoting of `name` / `author` / `description`, list splitting, cp1251 coverage.
//!
//! The service and the commands:
//!
//! - [`ManifestService`] (project scope, `session.services`): a `Store<ManifestState>` saved as
//!   it is edited ([`gz_editor::AutoSave`]). The store's revision IS the manifest generation.
//!   A change of `mod.ltx` behind the editor (git, an agent) comes through the project's
//!   watcher and loads the file as a new document.
//!   [`ManifestService::outside_build`], [`ManifestService::globals`] and
//!   [`ManifestService::playlists`] are the answers other
//!   areas ask for; [`command_check`] the functions and problems of `[commands]`.
//! - [`ManifestFields`] and [`ManifestArgs`] - what other areas add to the manifest's answer
//!   (the `world` field) and to `mod.set` (`tags`, `pda.layout`).
//! - [`sections`] - [`OwnSections`]: the sections the module itself brings, the other half of
//!   "does the game, with this module on, have the section X?".
//! - [`install`] - the project scaffold hook (a new project gets its `mod.ltx`); [`register`] -
//!   `mod.set`, `mod.exclude`.

mod cmd;
pub mod code;
pub mod commands;
pub mod doc;
pub mod edit;
pub mod exclude;
mod io;
pub mod model;
pub mod names;
pub mod radio;
pub mod reference;
pub mod rules;
pub mod sections;
mod service;
pub mod spot;
pub mod text;

pub use cmd::{install, manifest_answer, manifest_json, manifest_problems, open_manifest, register};
pub use commands::{
    has_script_function, scan_script_functions, valid_command_function, valid_command_name, validate_commands,
};
pub use doc::ManifestDoc;
pub use edit::Edit;
pub use exclude::{ShipState, exclude_match, normalize_rel, outside_build, set_excluded, ship_state};
pub use io::{installed_commands, read_manifest_file, scaffold, script_functions};
pub use model::{Command, FILE, GlobalVar, Manifest, ProvidesMode, RadioPlaylist, derive_id, scaffold_text};
pub use reference::{
    Check, KeyDoc, Reader, SectionDoc, is_manifest_word, key_doc, key_of, reference, reference_text,
};
pub use rules::{
    DESCRIPTION_ROOM, GLOBAL_NAME_TAKES, Pairing, TAG_VOCABULARY, accepts, global_name_refusal, global_twice,
    known_tag, normalize_github_repo, parse_flag, refusal, target_pairing, valid_global_name, validate,
    validate_github_repo, validate_id, validate_mode_id, validate_mode_ref, validate_own_mode_id,
    validate_version, validate_website,
};
pub use rules::{LINK_KINDS, LinkKind, github_link, validate_link, website_kind};
pub use rules::{MODE_LISTS, PLAIN_MODE, mode_list, mode_needs_lists};
pub use sections::{OwnSections, SectionSet, SectionSource, SectionSources, own_sections};
pub use service::{
    ArgCx, ArgHook, FieldHook, ManifestArgs, ManifestCmd, ManifestFields, ManifestService, ManifestState,
    command_check,
};
pub use spot::{Issue, Place, Spot};

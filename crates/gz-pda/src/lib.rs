//! PDA interfaces: a module's own PDA screens (`.pdaasset`, declarative Lua, `pda = 1`), named by
//! `mod.ltx [pda] layout`.
//!
//! The C++ specification is `EditorPda.cpp` (the asset, the reader, validation, the build gate,
//! `[pda] layout`, the MCP handlers) and the state of `EditorPdaPreview.cpp`. The format is
//! `docs/pda/PDA_FORMAT.md`.
//!
//! **The editor interprets nothing of its own.** What a layout MEANS is the game's
//! `scripts\xms_pda_layout.script`: pure Lua that turns the asset into a flat item list plus
//! coded problems (`E3xx` / `W3xx`). The editor runs that same file through `gz-lua` - the linked
//! game's copy, or the bundled `gamedata\configs\pda\` one without it.
//!
//! # API
//!
//! The domain (pure):
//!
//! - [`read`] - [`evaluate`] (`EditorPda::Read`): the text checked by `gz_luadata` (syntax with
//!   its line, code outside strings refused), then the game's `normalize` over the very table
//!   LuaJIT builds of it; [`Reader`] (the script and where it came from), [`ReadError`],
//!   [`template`], [`base_name`], [`is_pda_path`].
//! - [`layout`] - [`Layout`], [`Item`], [`Page`], [`PdaProblem`], [`MapSpot`], [`Marker`]: the
//!   reader's answer as the C++ read it off the Lua stack, plus the editor-only `preview` block.
//! - [`preview`] - [`Preview`]: the preview's running interface as data - pages, vars, the
//!   action log, list rows and scroll, keys, the map zoom - and the transitions a click, a key or
//!   an action makes (`signal:` / `lua:` / `close` are a toast and a log line); item boxes,
//!   hit tests and the `visible` list in canvas units.
//!
//! The service and the commands:
//!
//! - [`PdaService`] (project scope): the reader (a `Query` keyed by the linked game's index),
//!   the layouts (a `Query` keyed by the file, the language, the reader's and the text's
//!   revisions, computed on the CPU pool), the textures `W320` asks about, the project's layouts
//!   ([`PdaService::list`]), `[pda] layout` ([`PdaService::used_layout`],
//!   [`PdaService::use_layout`] through `gz-mod`'s manifest store).
//! - [`validate_for_build`] and the `BuildCheck` [`install`] registers: a used layout with an
//!   error, a missing file or one the build leaves out refuses the build. [`install`] also
//!   gives `content_create kind=pda` its template.
//! - [`register`]: `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`, `pda_validate`.

mod cmd;
pub mod layout;
pub mod preview;
pub mod read;
mod service;

pub use cmd::register;
pub use layout::{InnerSpot, Item, Layout, MapSpot, Marker, Page, PdaProblem};
pub use preview::{Aspect, Clock, Hit, ItemBox, Preview, VisibleItem};
pub use read::{BUNDLED_READER, GAME_READER, ReadError, Reader, base_name, evaluate, is_pda_path, template};
pub use service::{
    KnownTextures, PdaService, install, project_relative, read_text, same_path, validate_for_build,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

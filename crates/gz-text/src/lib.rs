//! The project's text files that are no assets (docs/ARCHITECTURE.md section 5, "A text file that
//! is no asset"; docs/UI_DESIGN.md "Text"): scripts, configs and patches, XML, Markdown, JSON.
//!
//! - [`file`] - a file's bytes as text and back: its encoding (UTF-8 with or without a BOM, or
//!   Windows-1251) and its line endings kept, a line nobody touched keeping its own ending
//!   ([`TextFile`], [`decode`], [`TextFile::encode`]); what is text at all ([`is_text_name`],
//!   [`sniff`], [`MAX_BYTES`]).
//! - [`merge`] - lines: the difference of two texts and the three-way merge a tab with unsaved
//!   edits takes a change on disk with ([`merge3`]).
//! - [`check`] - a file's problems with positions: a script's syntax (LuaJIT's own parser,
//!   compiled, never run), what the module build says of its code (`gz_mod::code`), XML and JSON
//!   that do not read ([`check_text`]); the same list is `asset.check` of the file.
//! - [`symbols`] - what Go to Symbol lists: a script's functions, a config's sections, a Markdown
//!   file's headings.
//! - [`index`], [`service`] - the declarations of the scripts, the linked game's and the
//!   project's: where a name is declared and what a prefix completes to ([`ScriptIndex`],
//!   [`TextService`]), made once per state of the game and of the project.
//! - [`cmd`] - `script.lookup`.

#![forbid(unsafe_code)]

pub mod check;
pub mod cmd;
pub mod file;
pub mod index;
mod lex;
pub mod merge;
pub mod service;
pub mod symbols;

pub use check::{CHECKED, check_file, check_text};
pub use cmd::register;
pub use file::{
    EncodeError, Encoding, Eol, MAX_BYTES, ReadError, TextFile, decode, default_encoding, is_text_name, sniff,
};
pub use index::{Decl, DeclKind, DeclSource, Offer, OfferKind, ScriptDecls, ScriptIndex, declarations};
pub use merge::{merge3, same_lines};
pub use service::TextService;
pub use symbols::{Symbol, SymbolKind, symbols};

/// Plugs the text files' checks into `asset.check` (`gz_asset::Kinds`).
pub fn install(editor: &std::sync::Arc<gz_editor::Editor>) {
    check::install(editor);
}

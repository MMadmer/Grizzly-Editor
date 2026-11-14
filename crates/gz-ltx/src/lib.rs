//! LTX, the engine's ini dialect, in the two ways the editor needs it.
//!
//! - [`LtxDoc`] is for a file somebody owns and reads - `mod.ltx`, `project.ltx`, the project
//!   list, preferences. It keeps every byte it does not change: comments, order, spacing, line
//!   ends. An untouched document writes back identical; an edit rewrites the lines it is about
//!   and nothing else. It is the one line model of LTX text: the ini reader's lines and the module
//!   manifest's ([`Syntax`]) are two ways of reading the same lines.
//! - [`Ltx`] ([`tree`]) is the engine's view of a config tree: `#include`s followed, sections
//!   inherited, typed reads. It is what the game's `system.ltx` is read through.
//!
//! Both work on `&str`; the caller decodes (UTF-8 for the editor's files, cp1251 for the game's).

pub mod doc;
pub mod tree;

pub use doc::{LineItem, LtxDoc, Placement, Syntax, quote, unquote};
pub use tree::{
    Builder, Dialect, Include, Item, Loaded, Loader, Ltx, LtxError, LtxWarning, ParsedFile, Section,
    include_path, parse_file,
};

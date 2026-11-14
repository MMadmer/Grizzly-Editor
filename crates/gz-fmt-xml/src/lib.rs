//! The game's XML: a tolerant reader with the engine's navigation, the
//! `#include` pre-pass, the string tables and the texture descriptions.
//!
//! # API
//!
//! - [`Document::parse`] (`&str`) -> [`Document`]: an immutable tree. It never fails: a defect
//!   the game's files are full of - stray text, an unclosed or stray tag, a duplicate
//!   attribute, a bad entity - is an [`XmlWarning`] in [`Document::warnings`] and reading goes
//!   on. [`decode`] turns the bytes of a file into text by its declaration (UTF-8 or cp1251).
//! - [`preprocess`]: the pre-pass - the file's lines (blank lines dropped, `\r\n` ends), each
//!   `#include "name"` line replaced by that file, names resolved by the caller.
//! - Navigation, from an explicit start node (there is no local-root state):
//!   [`Document::root`], [`Document::navigate`] (`"a:b:c"` and an index),
//!   [`Document::navigate_with_attr`], [`Document::search_for_attribute`],
//!   [`Document::count`], [`Document::read`] / [`Document::read_or`] (the text),
//!   [`Document::read_int`] / [`Document::read_float`], [`Document::attr`] /
//!   [`Document::attr_or`] / [`Document::attr_int`] / [`Document::attr_float`], and the node
//!   accessors [`Document::kind`], [`Document::value`], [`Document::children`],
//!   [`Document::parent`], [`Document::attributes`].
//! - [`strings`]: the string tables - which files, in which order, `id -> text`, built from
//!   many files in parallel.
//! - [`sheets`]: the texture descriptions (`textures_descr`) as `id -> (sheet, rect)`.
//!
//! # As TinyXML reads it
//!
//! Text is condensed as TinyXML condenses it (leading and trailing blanks dropped, a run of
//! blanks one space), entities are decoded (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;`, `&#..;`),
//! a text of blanks alone is no node, CDATA is kept as written. A name lookup compares the
//! value of every child (an element's name, a text's text), as TinyXML does.
//!
//! Where the engine stops: it refuses a whole file on a parse error or a missing
//! include, and stops reading at a duplicate attribute. Here the defect is a warning and the
//! rest of the file is read - so a file the game refuses still reads here.

#![forbid(unsafe_code)]

mod cnum;
mod doc;
mod include;
pub mod sheets;
pub mod strings;

pub use doc::{Document, Kind, NodeId, XmlWarning, decode};
pub use include::preprocess;

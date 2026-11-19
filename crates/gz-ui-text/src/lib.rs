//! The Text tab (docs/UI_DESIGN.md, section 6, "Text"): every text file of the project that no
//! other tab owns - a game script, a config or a patch, XML, JSON, Markdown, plain text - and a
//! script of the linked game, read-only.
//!
//! - [`doc`] - the tab: the kit's `code` editor with the file's language, Find and Replace, Go to
//!   Line, Go to Symbol, a script's completion and Go to Definition, the problems drawer;
//!   Markdown as text, side by side with its preview (the kit's `markdown`) or as the preview.
//! - [`model`] - its file off the UI thread: the read with its encoding and line endings
//!   (`gz-text`), the follow through the project's watcher and the line merge into unsaved edits,
//!   the save in the file's own bytes, the checks.
//! - [`find`] - the matches of a query, the next one, the replacements.
//! - [`tabs`] - what is asked of a tab from outside its frame (a line, a query), kept until the
//!   tab takes it.
//! - `text.view` ([`register`]) drives it; `ui.tab action=open kind=text path=<file>` opens it.

#![forbid(unsafe_code)]

pub mod doc;
pub mod find;
pub mod model;
pub mod tabs;
mod view_cmd;

use gz_app::{Document, ShellBuilder};

/// Plugs the Text tab and its words into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.document(doc::KIND, |ed, req| doc::TextDoc::open(ed, req).map(|d| Box::new(d) as Box<dyn Document>));
    b.translation(include_str!("ru.lua"));
}

/// Registers `text.view`.
pub fn register(reg: &mut gz_editor::Registry) {
    view_cmd::register(reg);
}

#[cfg(test)]
mod i18n_tests {
    #[test]
    fn the_catalog_reads_with_the_kits_and_the_shells() {
        let c = gz_ui::i18n::Catalog::parse(&[
            gz_ui::i18n::KIT_RU,
            gz_app::lang::SHELL_RU,
            include_str!("ru.lua"),
        ])
        .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }
}

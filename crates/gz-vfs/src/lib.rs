//! The linked game's files as one immutable index. Read-only: nothing here opens a write handle
//! on the game folder.
//!
//! # API
//!
//! - [`GameFiles::mount`] (`root`, [`MountOptions`]) -> [`GameFiles`]: the game's loose tree and
//!   archives as the game mounts them, and its modules. Blocking; the parallel work (one task per
//!   archive directory, the loose trees walked folder by folder) runs on the caller's pool.
//!   [`GameFiles::refresh`] mounts the same game again with this index as the saved one.
//! - Lookups answer from the index, never from the disk: [`GameFiles::get`] -> [`FileRef`]
//!   (`path`, unpacked `size`, [`Source`]), [`GameFiles::exists`], [`GameFiles::is_dir`],
//!   [`GameFiles::list`] (a folder's [`Listed`] sub-folders and files), [`GameFiles::find`]
//!   (every file below a folder with an extension), [`GameFiles::iter`].
//! - Reads: [`GameFiles::read`] / [`GameFiles::read_ref`] (blocking, LZO unpacked),
//!   [`GameFiles::read_many`] (parallel on the caller's pool, archived files in runs through one
//!   handle each). Archive handles are opened per read or per run, never kept.
//! - [`GameFiles::modules`] / [`GameFiles::module`]: `modules\<id>` and `mods\<id>`, each a
//!   [`Module`] with its own listing (`get`, `list`, `find`, `read`).
//! - [`GameFiles::archives`], [`GameFiles::skipped`] (archives that could not be read),
//!   [`GameFiles::stats`] ([`MountStats`]: what the mount checked and read).
//! - [`looks_like_game`], [`normalize`] (the key of a path), [`is_tooling_entry`].
//!
//! # Precedence
//!
//! Paths are keys: lower case, forward slashes, relative to `gamedata`. The archives are those
//! of `fsgame.ltx`'s archive folders (`$arch_dir...$`, `database\` without one), in its order,
//! each folder by name; a later archive replaces a path an earlier one holds (`Register`), and
//! the loose tree (`$game_data$`) replaces both. A name ending in a separator is a folder marker
//! and no file. Hidden entries are skipped as the engine skips them; tooling entries (`.git`,
//! `Thumbs.db`, ...) of a module as XMS skips them (the loose `gamedata` keeps them, as the C++
//! editor's DARF content listed them).
//!
//! oracle: an archive's own `[header] entry_point` is ignored, as the C++ editor's DARF mount
//! ignored it: every archive of a game is relative to `gamedata`.
//!
//! # The saved index
//!
//! With [`MountOptions::cache_dir`] (the editor's `appdata\game_index`), a mount saves what it
//! read - every loose folder and every archive directory with the size and write time it was read
//! at - and the next mount checks those stamps (one `metadata` call each, in parallel) and reads
//! again only what changed. A start costs the number of file operations; this is where most of
//! them go.

#![forbid(unsafe_code)]

mod archive;
mod cache;
mod files;
mod fsgame;
mod index;
mod path;
mod walk;

pub use files::{Archive, GameFiles, Module, ModuleRoot, MountError, MountOptions, MountStats};
pub use index::{FileRef, Listed, Source};
pub use path::{is_tooling_entry, looks_like_game, normalize};

//! Runs the game's own pure-Lua readers: scripts of the game that turn an asset into data
//! (`scripts\xms_pda_layout.script` and its kind). The editor runs the SAME file the game runs,
//! so the two can never read an asset differently; this crate knows no asset of its own.
//!
//! The interpreter is LuaJIT 2.1 (`mlua`, vendored), the one the game runs: table iteration
//! order - which `pairs` walks - is the game's. The JIT compiler is switched off, as the C++
//! editor switched it off, so the instruction limit sees every instruction.
//!
//! # API
//!
//! - [`LuaHost`] - a small pool of Lua states. A state is never shared between threads: a call
//!   takes one out of the pool (or makes one), runs, and puts it back. Each state keeps the
//!   scripts it has loaded, once per script revision ([`Script::rev`]). [`LuaHost::call`] runs
//!   `function` of a [`Script`] with [`Arg`]s and answers a [`gz_luadata::Value`];
//!   [`LuaHost::call_omitting`] leaves named keys of the returned table out (a reader's own
//!   bookkeeping that is no data).
//! - [`Script`] - the text of a script, its chunk name and its revision.
//! - [`Arg`] - a value ([`gz_luadata::Value`]) or a [`Chunk`]: a text that is compiled and run
//!   in an empty environment, whose result is the argument - how an asset reaches a reader with
//!   the very table LuaJIT builds of it.
//! - [`Limits`] - the instruction limit of one call, the memory a state may use, the depth and
//!   size of a returned value.
//! - [`LuaError`] - where a call failed (the script, an argument, the function, the result),
//!   with Lua's own message.
//! - [`vanilla_source`] - the editor's LuaJIT fork also reads `//` and `/* */` comments; this
//!   rewrites them so the vendored LuaJIT reads the same tokens and counts the same lines.
//!
//! # The sandbox
//!
//! A state loads the base library and `string`, `table`, `math` (and `jit`, only to switch the
//! compiler off) - no `io`, `os`, `package`, `debug` or `ffi`. A script runs in an environment of
//! its own that holds only [`SANDBOX_GLOBALS`]; a chunk argument runs in an empty one. Binary
//! chunks are refused. Every call has an instruction budget ([`Limits::instructions`]); running
//! out raises `execution limit exceeded` inside Lua.

mod convert;
mod dialect;
mod host;

pub use dialect::vanilla_source;
pub use host::{Arg, Chunk, Limits, LuaError, LuaHost, SANDBOX_GLOBALS, Script};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

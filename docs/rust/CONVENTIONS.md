# Rust conventions of the editor

The rules every crate under `crates/` follows. The plan and the architecture are
`docs/RUST_MIGRATION.md`; this file is what a person or an agent writing a crate needs at hand.

## The oracle

- The C++ editor under `Source/` is the specification. Port **behaviour**: formats byte for
  byte, checks with the same outcome, problem codes and messages word for word. Do not port
  structure: no globals, no manager singletons, no `xr_string`.
- `Source/` is never edited.
- Where the C++ is wrong in a way a file or an answer depends on, keep the behaviour and say so
  in a comment that starts with `// oracle:`.

## Safety

- Every crate is `#![forbid(unsafe_code)]` through `[lints] workspace = true`. Only `gz-win` is
  exempt. No `unsafe impl Send` / `Sync` anywhere.
- No `unwrap()`, `expect()`, `panic!`, `todo!`, `unreachable!` or indexing that can go out of
  bounds outside tests. Input is hostile: a file cut short, a negative count, a huge length are
  errors (`Result`), never a panic and never an unbounded allocation - check a count against the
  bytes that are left before reserving for it.
- Errors are `thiserror` enums of the crate. Messages are sentences for the author, in English.

## State and threads

- No `static` that holds state, no lazily initialised global, no singleton accessor. What a
  function needs it takes as an argument.
- Layers 1-3 are **pure**: functions over `&` data that return values. No logging (return
  warnings as data), no thread of your own, no async, no file access that is not the function's
  stated job.
- Every public type is `Send + Sync` unless it says why not. `Rc` and `RefCell` are not used
  below the UI.
- Parallel where the data is large: `rayon` iterators over files, meshes, nodes, rows. The
  caller decides the pool (`pool.install(..)`); a library never builds one.
- Bulk immutable data is `Arc<[T]>` or `Arc<T>`, so a snapshot shares it. Collections a store's
  state edits are `imbl` structures. Names are `SmolStr`.
- Editor state lives in a `gz_store::Store`; `State::apply` never blocks.

## Speed

- Parse from `&[u8]` / `&str` without copying where the result can borrow.
- Reserve what is known; no quadratic scans over things that count in thousands; no per-item
  allocation in a hot loop.
- A start costs the number of file operations: never stat or open a file to learn what an
  index already says, never read a file twice.

## Build time

- Small crates, one job each; a lower layer never depends on a higher one (section 7 of the
  plan). Layers 0-3 depend on neither `egui` nor `wgpu`.
- Dependencies come from `[workspace.dependencies]` (`foo.workspace = true`). A crate that
  needs one that is not there names it in its own `Cargo.toml` with a version; the root
  manifest is not edited by anyone but the integrator.
- No `build.rs`, no proc macro beyond `serde` and `thiserror`, no generic public API where a
  concrete one does.

## Crate skeleton

```toml
[package]
name = "gz-example"
description = "One sentence."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
gz-base.workspace = true
```

## What is already there

| Crate | Gives |
|---|---|
| `gz-base` | `Rev`, `Pt` / `Px` / `Scale`, `RelPath` / `ProjectPath` / `GamePath`, `Problem` / `Severity`, `Origin`, `SmolStr`, `Blob`, `text::{cp1251_to_utf8, utf8_to_cp1251, fits_cp1251, decode_guess, eq_ci, split_list}` |
| `gz-task` | `Executors` (`cpu`, `io`, `par_io`, `par_cpu`, `spawn`), `Cancel`, `Jobs` / `JobCtx`, `Wake` |
| `gz-store` | `Store<S>`, `State`, `Apply`, `Snap`, `CmdError`, `imbl` |
| `gz-query` | `Query<K, V>`, `Derived<V>`, `Answer` |
| `gz-log` | `Log` and the `info!` / `warn!` / `error!` macros (application layer only) |
| `gz-chunk` | `Reader`, `Writer`, `Chunk`, `ChunkError`, `lzhuf`, `lzo` |
| `gz-ltx` | `LtxDoc` (lossless editing), `Ltx` (the engine's resolved view) |

## Text

- Comments only where the "why" is not obvious; short, English.
- Product text - messages, labels, problem texts - is English in the official tone. Russian
  stays only in game content and in encoding fixtures.

## Working in parallel with others

- Touch only the crates you were given. If `cargo` fails on somebody else's manifest or code,
  wait and retry; do not fix it.
- Compile with `cargo check -p <crate>` (and `--tests` once). The target directory is shared:
  "Blocking waiting for file lock" is normal, wait for it.
- **Tests are written, not run.** Unit tests with small inline fixtures go into the crate;
  running them is the last phase of the migration.

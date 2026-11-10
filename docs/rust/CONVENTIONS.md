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
| `gz-editor` | `Editor` (pools, log, jobs, frames, notices, services, the open `ProjectSession`), `Registry`, `Args` / `ArgsExt`, `Reply`, `CmdError`, `DirtySource`, `AutoSave`, `fsx::{write_atomic, read_opt, read_text}` |
| `gz-chunk` | `Reader`, `Writer`, `Chunk`, `ChunkError`, `lzhuf`, `lzo` |
| `gz-ltx` | `LtxDoc` (lossless editing), `Ltx` (the engine's resolved view) |

## An area crate

An area of the editor - the project, the manifest, quests, characters, builds - is one crate of
layer 3 that depends on `gz-editor` and is laid out the same way:

- **the domain** (`model`, `parse`, `validate`, `write` ...): pure functions over data, no
  `Editor`, no IO except where reading or writing a file is the function's stated job;
- **`service.rs`**: what the area keeps while the editor runs - its `Store`s, `Query`s, caches -
  as one struct stored with `editor.services.get_or_init(..)` (lives as long as the editor) or
  `session.services.get_or_init(..)` (lives as long as the open project; get the session with
  `editor.project()?`). Nothing is created at start: a service is made on first use;
- **`cmd.rs`**: the area's commands, registered by `pub fn register(reg: &mut gz_editor::Registry)`
  (the first argument of `reg.add` is the crate's name). A handler is thin: read arguments with
  `ArgsExt`, call the domain or the service, answer JSON.

Rules for commands:

- **The wire is frozen by the C++ handler** named in the task: same command name, same argument
  names and forgiving parsing, same reply fields with the same names and meaning, same error
  texts (`missing 'path' argument`, `no active project`, ...). QA scripts read these. Put fields
  in the order the C++ writes them (`serde_json` keeps insertion order).
- A handler never blocks the async runtime: file work goes through
  `editor.exec.io(move || ..).await`, CPU work through `editor.exec.cpu(..).await`, many files at
  once through `par_io` inside it.
- State changes go through the area's `Store` (`store.send(cmd, origin).await`); `State::apply`
  never touches a file. A document saved as it is edited uses `gz_editor::AutoSave` and the
  command awaits `flush(rev)` before it answers, so a caller that reads the file next sees the
  edit. A document with an explicit Save writes through `gz_editor::fsx::write_atomic` on the IO
  pool and then marks the store saved.
- Long work is a job: `editor.jobs.start(kind, group, title, |ctx| async move { .. })`; it checks
  `ctx.check()?` between steps and reports `ctx.stage(..)` / `ctx.progress(done, total)`.
- `Origin::Mcp` never asks: a confirmation is Yes, and losing unsaved work needs the explicit
  `discard` / `save` argument or the command answers `CmdError::unsaved(..)`.
- Unsaved documents are reported through `gz_editor::DirtySource`, registered once when the
  service is first made.
- Reads of derived data (lists, problems, pickers) are `Query`s keyed by the revisions they
  read, so nothing is recomputed per call and nothing is invalidated by hand.
- When you create a crate, write `src/lib.rs` BEFORE `Cargo.toml`: a manifest without a target
  stops every other build of the workspace.

## Text

- Comments only where the "why" is not obvious; short, English.
- Product text - messages, labels, problem texts - is English in the official tone. Russian
  stays only in game content and in encoding fixtures.

## Working in parallel with others

- Touch only the crates you were given. If `cargo` fails on somebody else's manifest or code,
  wait and retry; do not fix it.
- Compile with `cargo check -p <crate>` (and `--tests` once). The target directory is shared:
  "Blocking waiting for file lock" is normal, wait for it.
- **Running the editor for a look** goes through `tools/rust/run-editor.ps1` only: one run at
  a time on the machine (a lock), the project list and the settings file restored byte for
  byte, the window minimised. Never start `Grizzly.exe` by hand.
- **Tests are written, not run.** Unit tests with small inline fixtures go into the crate;
  running them is the last phase of the migration.

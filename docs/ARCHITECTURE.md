# Grizzly Editor - architecture

How the editor is built and what it owes the game. This document is the contract of the code's shape: a change
of plan is a change here first. The other contracts stand beside it: `docs/COMMANDS.md` (commands and the three
ways in), `docs/ASSETS.md` (text assets and projects), `docs/UPDATES.md` (the editor's own updates),
`docs/UI_DESIGN.md` (the UI), `docs/NQ_ARCHITECTURE.md` (quests and behaviour graphs).

## 1. Principles

1. **Published data is immutable.** Whatever more than one thread can see is an `Arc` of a value nobody changes.
2. **One writer per store.** A store owns its state, takes commands from a queue and publishes a new snapshot.
   Nothing else writes.
3. **Everything that takes time is a task.** It reads a snapshot, runs on a pool, and comes back as a command or
   a query result. The UI thread never waits on a file, the network, a lock a job may hold, a GPU readback or a
   pipeline compile.
4. **No singletons.** Stores, pools and services are handles passed down from the editor. There is no `static`
   holding mutable state.
5. **The domain knows no window.** Crates below the presentation layer depend on neither egui nor wgpu, so the
   whole editor except the picture runs without a window - as a commandlet, or as a server.
6. **Everything the editor does is a command**, with a card that says what it is, what it takes and what it can
   do to the world. The window, an agent and a shell run the same ones (`docs/COMMANDS.md`).
7. **What an author makes is text on disk**, read and written without touching what was not changed, and
   followed live. The files are the source of truth; the editor is one way to work on them (`docs/ASSETS.md`).

## 2. Threads and executors

| Executor | Runs | Rule |
|---|---|---|
| UI thread | the message pump, the egui pass with its tessellation, command emission, the presentation of the window | reads snapshots with one atomic load; never blocks; runs above normal priority |
| Render threads | the device is made on a thread of its own; the viewport's renderer, the offscreen passes (thumbnails, previews) and every readback run on theirs | a slow GPU frame of the viewport delays no input and no command; a render thread never waits for the GPU, and every submission passes the window's gate (`gz_app::gpu_gate`, a `gz_task::Gate`), which the window closes while it configures a surface - the graphics runtime refuses a configuration a submission crosses. The runtime serialises everything a queue is asked behind one lock, and the window's frame takes it too: a pool thread that takes it (an asset going to the GPU) holds `gz_win::drawing_priority()` meanwhile, or a busy machine would make the UI thread wait for a thread it took the processor from |
| Async runtime (tokio, multi-thread) | store actors, the control endpoint, network, timers, job orchestration | no blocking call inside; file work goes to the IO pool |
| CPU pool (rayon) | data-parallel work: validation, hashing, meshes, culling, layout, compression, the level compiler | pure functions over snapshots |
| IO pool (bounded) | blocking file operations | sized to overlap latency: a start costs touches, not bytes |
| Watcher | file change notifications of the open project | one per project; debounced; the editor's own writes filtered out |
| Heartbeat | wakes every 10 ms and notes how late it was | tells a UI thread that waited from a machine with no processor to give (section 6) |

The pools run at normal priority on every logical core: the editor's work takes what the hardware has, and no
pool is made smaller or slower for smoothness. The thread that draws runs above normal instead, so the window
gets its slice first; the process opts out of Windows' power throttling. No thread is made by the UI thread:
a thread of its own (a device's loop, a renderer's start) is started with `Executors::thread`, where a pool
thread makes it.

## 3. State

**Stores, snapshots, commands** (`gz-store`). A store is an actor: a task that owns a state value and applies
commands one at a time. Applying a command builds a new state from the old one and swaps the pointer; a command
that fails or panics leaves the published state exactly as it was. Large collections are persistent structures,
so a snapshot per command shares the rest; bulk data (meshes, textures, AI map tiles, detail slots) is immutable
blobs behind handles. **Undo is the previous snapshot**: every document has its own history, and undo cannot
disagree with the model. Stores never await one another: work that spans stores is a job that sends commands to
each. What a tab edits lives in a store of the editor or the project, never in the tab: a command changes it
without a window, and the tab, when one is open, follows (the libraries' pending edits: `LibraryStores`).

**Derived data is a query** (`gz-query`): a pure function of snapshots and file revisions, memoised by those
revisions, computed on the CPU pool. It answers at once with `Fresh`, `Stale` or `Pending`; the UI draws what it
has. Nobody invalidates anything: an input changed, so its revision changed.

**Jobs** (`gz-task`): one system for everything long - builds, packaging, publishing, scans, scene loads, level
builds, thumbnails, the editor's update. A job has a name, stages, progress, a cancel token, a result and an
owner: the editor, or the open project - a project's jobs stop when it closes. The Task Center shows them; a
command that starts one waits for it or answers its id (`wait=false`); `job.*` follows any of them.

**Who asked travels with a command** (`Origin`): the window (a question opens a dialog), a caller (the caller
decided: nothing is asked, and losing data takes an explicit parameter and its own door), an unattended run
(the logged answers).

## 4. Commands

`gz-editor` holds the registry. A command is registered with its card (`Spec`: the name `area.verb`, a one-line
summary, the effect - `read`, `view`, `write`, `destroy`, `publish`) and a parameter struct the request is read
into; `help` prints the registry, so the documentation of a command is its code. A request names the door it
came through, and a command whose effect belongs to another door is refused with that door's name.

Three ways in, one model (`docs/COMMANDS.md` 4):

- **The window**: menus, toolbars, shortcuts and the palette are *actions* - a command with its arguments, a
  title, an icon and default keys (`gz-app`'s `actions.rs`). Direct manipulation goes to a store's commands.
- **MCP**: `grizzly-mcp.exe`, six tools (`help`, `state`, `run`, `destroy`, `release`, `look`) over the editor's
  control endpoint (JSON lines on 127.0.0.1).
- **A shell**: commandlets - `Grizzly-Cmd.exe run <command> ...`, `script`, `help`, `check`, `build`, ... - the
  editor without a window and without a port, with an exit code.

Requests whose answer is a drawn frame are held until the frames they need were presented; a request past its
timeout answers `code: running` with its job, never a guess.

## 5. Files

- **Assets** (`gz-luadata`, `gz-asset`): every text asset is Lua data kept as a lossless document (`LuaDoc`);
  a typed model saves by applying the difference between what it read and what it holds, so comments, order,
  unknown keys and untouched bytes survive. `gz-asset` is the one list of kinds: versions, migrations, checks
  with positions, references. `mod.ltx` is edited the same way through `gz-ltx`'s lossless line model.
- **The project** (`gz-project`): `<Name>.grizzly` is the project; nothing inside a project names a place
  outside it; this machine's state lives in the editor's home by the project's id. One watcher per project
  feeds the content browser, the caches and every open document, which reload or rebase their unsaved edits.
- **The linked game** (`gz-vfs`, `gz-game`) is read and never written: a virtual file index over the project,
  the game (`gamedata`, `modules\`, `mods\`, archives) and the editor's library, persisted between starts.
  Text is UTF-8 inside; cp1251 is converted explicitly where the game's files are read or written.
- **Every writer passes one guard** (`gz_editor::fsx::WriteGuard`): nothing under the linked game's root, judged
  as written and where the path really is, except the module's `modules\<id>` (with its stage in
  `modules\.new`) and the flat `gamedata_<id>`.
- **The editor's own updates** (`gz-update`, `docs/UPDATES.md`): one archive per release, a ranged delta, a
  journalled swap with rollback.

## 6. The UI

- **egui**, immediate mode: a surface is a function of snapshots and its own view state that emits commands.
  The frame loop is the editor's own (`gz-app`'s `window.rs`: winit, egui-winit, egui-wgpu); frames run on
  demand, and a scene pass only when its picture can change.
- **The kit (`gz-ui`) is the only way to draw**, one widget per job; sizes are points, colours are tokens;
  `docs/UI_DESIGN.md` is the contract. Every kit widget registers itself in a per-frame tree, which is what
  `ui.list` and `ui.audit` answer from.
- **A surface never computes**: anything beyond a few microseconds is a query or a job.
- **The stall monitor** (`gz-app`'s `stalls.rs`) is the proof: every UI pass and event is timed; one over 100 ms
  is a stall, kept for `perf.stalls` with what ran. A long pass during which the heartbeat thread could not run
  either is the machine's (`starved`), not a stall. Every QA suite ends by asking.
- **The renderer** (`gz-render`) is wgpu, a forward renderer written for the editor: engine shader names map to
  a small set of WGSL materials; extraction reads a scene snapshot and hands an immutable packet to the render
  thread; `gz-render` knows no scene. Thumbnails, previews and `preview.png` are offscreen passes run as jobs.
- The window frame (caption, hit test, DPI, dropped files, the file association, the single-instance handover)
  is the one place that needs Win32 directly; it lives in `gz-win`.

## 7. Rust rules

- **`unsafe` only in `gz-win`** (the Win32 that `std` lacks), every block with a `// SAFETY:` line, a safe API
  outside; no `unsafe impl Send` / `Sync` anywhere. Every other crate is `#![forbid(unsafe_code)]` through the
  workspace lints; `check.sh` fails on `unsafe` elsewhere.
- **No `static` holding state**, no lazily initialised global, no singleton accessor. `Rc` and `RefCell` only
  in a surface's own view state on the UI thread. A mutex guards a small private structure and is never held
  across an `.await`, a frame or a call into another module.
- Fallible code returns `Result`; `unwrap` and `expect` are denied outside tests. A panic inside a command or a
  job fails that command or job, writes a crash report and leaves every store on its last good snapshot.
- **Dependencies**: pure Rust first; the C dependencies are two, both vendored behind safe APIs - LuaJIT
  (`mlua`) and libvorbis (`vorbis_rs`). No strong copyleft is linked: `tools/rust/notices.py` holds the list of
  licenses a linked crate may have and writes `licenses/THIRD_PARTY_NOTICES.txt`. The editor is MIT. TLS trusts
  the Windows certificate store. The toolchain is pinned in `rust-toolchain.toml`.
- **No test writes a program file**, and **nothing a script starts may wait for ever** (`tools/rust/limited.py`).
- Every line a program prints is a `writeln!` whose failure stops the printing: a reader that leaves is not an
  error.

## 8. The workspace

`Cargo.toml` at the repository root, crates under `crates/`. A lower layer never depends on a higher one; layers
0-3 depend on neither egui nor wgpu.

| Layer | Crates |
|---|---|
| 0 Base | `gz-base` (ids, revisions, paths, cp1251, `Origin`, `Problem`), `gz-task` (executors, jobs, cancel), `gz-store`, `gz-query`, `gz-log`, `gz-crash`, `gz-win`, `gz-corpus` (support of the corpus tests), `gz-ico` (the editor's icon as a resource) |
| 1 Formats | `gz-ltx` (the lossless line model of LTX text), `gz-luadata` (Lua data: the reader that refuses code, the lossless document), `gz-chunk` (chunk streams, LZO), `gz-fmt-object`, `gz-fmt-ogf`, `gz-fmt-tex` (DDS, TGA, THM), `gz-fmt-level` (collision, AI map, details), `gz-fmt-spawn` (`all.spawn`, game graph, patrols), `gz-entities` (server entity schema, packets, named properties), `gz-fmt-xr` (`shaders.xr`, `gamemtl.xr`, `lanims.xr`, `particles.xr`), `gz-fmt-xml`, `gz-fmt-sound` |
| 2 Game link | `gz-vfs`, `gz-lua` (host for the game's pure-Lua readers) |
| 3 Domain | `gz-editor` (the editor's handle, services, the command registry, `help`, `job.*`, the frame clock, the write guard), `gz-project` (the project file, machine state, the watcher, file operations, the library), `gz-asset` (the kinds, checks, references), `gz-game` (the linked game: configs, strings, modes, lookups), `gz-mod`, `gz-nq`, `gz-characters`, `gz-pda`, `gz-screens`, `gz-world`, `gz-build`, `gz-release`, `gz-github`, `gz-publish`, `gz-catalog`, `gz-update`, `gz-scene`, `gz-particles`, `gz-export-level`, `gz-lc-light` and `gz-lc` (the level compiler), `gz-levelbuild` (the level's build pipeline, Play) |
| 4 Presentation | `gz-mcp` (the control endpoint), `gz-ui` (the kit, icons), `gz-render`, `gz-app` (the window, the shell, actions), and a crate per surface: `gz-ui-app` (Start page, Preferences, updates), `gz-ui-content`, `gz-ui-graph`, `gz-ui-level`, `gz-ui-viewport`, `gz-ui-mod`, `gz-ui-pda`, `gz-ui-previews`, `gz-ui-release`, `gz-ui-world` |
| Programs | `grizzly` (`Grizzly.exe`, a window program, and `Grizzly-Cmd.exe`, its twin for a shell), `gz-mcp-bridge` (`grizzly-mcp.exe`) |

An area crate is its domain plus `service.rs` and `cmd.rs` with one `register(reg)`; a surface crate has one
`install(builder)`. The scripts that check and test the workspace are `tools/rust/check.sh` and
`tools/rust/test.sh`; the QA suites run through `tools/qa/run_all.py`.

## 9. The contract with the game

Every artifact that leaves the editor for the game, a release or the catalog, and every file of the game the
editor reads. The normative text is where the first column says; the editor is held to it by the proof in the
last one. The game knows modules and nothing of this editor.

| Artifact | Normative text | Proof |
|---|---|---|
| `mod.ltx`, all sections | game `MODDING.md`, `XMS_ARCHITECTURE.md` | lossless round trip over the corpus; `asset.check` |
| `.nqasset`, `.behasset` | `docs/nq/NQ_FORMAT.md`, game `NQ_RUNTIME.md` | the game's `run_lua.py test_examples.lua`; the catalogue parity test of the game; `test_graph_ui.py`, `test_behaviours.py` |
| `.npcasset` | game `MODDING.md`, "A module's characters" | `test_characters.py`; the game's `Run-ScreensQa.ps1 -Part Descriptions` |
| `.pdaasset` | `docs/pda/PDA_FORMAT.md` | the game's `xms_pda_layout.script` executed unchanged; `test_pda.py`; the game's `test_pda_layout.lua` |
| Screens and music files | game `MODDING.md`, "Screens and music" | `test_screens.py`; the game's `Run-ScreensQa.ps1` |
| The level layers of a module: `.xspawn`, `spawn\custom_data\*.ltx`, `overlay_visuals.ltx`, `overlay.xcform`, the cut, exported `.ogf`, the AI map | game `MODDING.md` "World rules", `XMS_ARCHITECTURE.md` | golden files in `gz-export-level/tests`; `test_level_commands.py`; `test_world.py` |
| What Play builds: the collision, the game graph, the cross table, the spawn | engine formats | the game loads the level (`test_play_game.py`: a new game, the player, the level's geometry and collision); unit tests hold what the game's loader relies on |
| A compiled level (`level`, `level.geom`, `level.cform`, the lightmaps, `level.details`) | engine formats | `test_level_compile.py` against its own records (`tools/qa/golden/compile`); the game loads it (`test_play_game.py`) |
| The installed module (`modules\<id>`, `.new\<id>`, hard links, tooling left out) | this repository's rules, game `xms_core.cpp` | `test_build_job.py`, `test_build_contents.py` |
| Flat export `gamedata_<id>` | - | `test_build_contents.py` |
| `<id>.update.ltx`, `<id>-<version>.files`, the packages | game `MOD_UPDATES.md` - **frozen** | the index read back from the finished archive and each record's range checked (`gz-release`); `test_mod_publish.py` against the mock |
| `[card]`, `[signature]`, `author.xkey`, issue bodies, content rules | game `MOD_CATALOG.md` | both ways against the catalog's `xms_catalog.py` and `catalog.py`; `test_mod_catalog.py`; the game verifies the signature |
| `thumbnail.dds` | game `MOD_CATALOG.md` | format, size and mip chain checked; the game shows it |
| Reads of the linked game (never written): `all.spawn`, configs, string tables, `textures_descr`, UI XML, fonts, the kind catalogue, archives | the game's files as they are | `game.lookup`, `world.lookup` in `test_world.py`, `test_characters.py` |
| Play | the game's `CONTENT_BUNDLES.md`: started on another fs config (`-fsltx`) the game writes nothing into its installation, and says so in the version resources of the engine and its libraries | `test_play_game.py` lists the installed game before and after and wants it equal |

Two consequences worth stating: a signature differs in bytes for the same input (ECDSA picks a nonce) - the
contract is verification; and a package's compressed bytes are whatever the writer made - the index states what
is on disk, read back from the finished archive.

**A level build writes into the open project** (`gamedata\levels\<level>`, `gamedata\spawns`, the build's state
beside them) and never into the game; a level folder is wiped only when its real path lies below the project's
levels folder and is no link.

## 10. Proof

Three entry points, each bounded: `bash tools/rust/check.sh` (clippy, fmt, `unsafe` only in `gz-win`, the
commands' cards through the commandlet, the text encodings, no test writing a program file, the icon and the
kind of the release programs, the notices), `bash tools/rust/test.sh` (only the test binaries that changed since
they were green; `--all` for everything), `python tools/qa/run_all.py` (the QA suites side by side, each on a
copy of a project, in a home and on a port of its own; `--with-game` adds the one that starts the real game).
`GRIZZLY_QA_EXE` points them at another build. Nothing that already passed is run again to check something
new: one crate, one suite or one phase while fixing, the three once at the end.

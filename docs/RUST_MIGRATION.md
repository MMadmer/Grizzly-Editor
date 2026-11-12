# Grizzly Editor on Rust

Status: built on `grizzly-rust`, before the cutover (phase 7). This document is the contract
and says what was built; it changes by editing the document first. What every package did, the
test passes and the reviewer's findings are in `docs/rust/STATE.md`.

Branch: `grizzly-rust`, forked from `25f76d13`. Nothing of it reaches `main` before the
cutover (phase 7).

## 1. Goal

The editor is rewritten in Rust around an architecture in which every piece of work can run
beside every other one, and in which a data race does not compile.

Three requirements, in this order:

1. **The game is not touched.** Dead Air Refined stays C++. The editor describes, the game
   does; what crosses between them is files.
2. **Nothing the editor does today is lost.** Every MCP command, every surface of
   `docs/UI_DESIGN.md`, every QA suite.
3. **Parallel by construction, safe by construction.** No global state, no blocking UI,
   `unsafe` only where an operating system API leaves no other way.

## 2. Invariants

| # | Invariant |
|---|---|
| I1 | No phase needs a commit in the game repository. A phase that finds it needs one stops, and this document changes first. |
| I2 | Nothing in the game names the editor (the existing rule, unchanged). |
| I3 | At the cutover the Rust editor does everything the C++ editor does. Deliberate differences are listed in section 12 and nowhere else. |
| I4 | The project folder is the interface between the two editors: both open the same project until the cutover, so no file format inside a project changes before it. |
| I5 | The C++ editor is the oracle. `Source/` is not edited on this branch, so a merge from `main` never conflicts. |
| I6 | Every crate is `#![forbid(unsafe_code)]` unless section 6 names it. |
| I7 | The UI thread never waits: no file access, no network, no lock that another thread may hold for longer than a pointer swap, no GPU readback, no pipeline compile. It draws and presents; its one wait is for the surface before the first frame. |
| I8 | State changes only through commands, and every command is reachable over MCP. |

## 3. Where we start

Measured on `25f76d13`, 2026-11-10.

| Part | Files | Lines | Note |
|---|---|---|---|
| `Editors/LevelEditor` | 313 | 77,956 | scene, tools, tabs, MCP handlers |
| `Editors/XrECore` | 302 | 97,057 | project, manifest, builds, releases, NQ, PDA, device |
| `Editors/XrEUI` | 41 | 93,134 | about 76k of it is Dear ImGui itself (patched) |
| `Editors/XrEProps`, `XrETools` | 62 | 15,330 | property grid, mesh tools |
| Engine under the editor | | ~199,000 | `XrCore` 38k, `XrEngine` 59k, `XrRender/Private` 44k, `XrCDB` 18k, `XrPhysics` 30k, `XrSound` 6k, `XrParticles` 4k |
| Server entities | | ~33,700 | `xrServerEntities` 24.8k, `XrSE_Factory` 8.9k |
| Level compilers | | ~35,800 | `XrLC` 18k, `XrLCLight` 17k, `xrDO_Light` 0.7k. `XrAI` (21k) has no build target: the editor builds the AI map and the graph itself (`LevelEditor/AI`) |
| `tools/mcp`, `tools/qa` | 23 | 14,868 | Python bridge and suites, Go mock |

What the code looks like from the point of view of threads:

- **Global singletons everywhere.** In editor code: `Scene->` 709 uses in 85 files, `EDevice`
  676 in 79, `UI->` 631 in 96, `Tools->` 157 in 32, `FS.` 429 in 67, `ELog.` 488 in 94.
- **Almost one thread.** A build job thread, network workers, the `.thm` prefetch and the MCP
  socket workers; MCP commands are executed by the main thread. The scene has no threads.
- **UI and logic interleaved, but not hopelessly.** Of `XrECore/Editor` and `LevelEditor`,
  63 files (65k lines) draw UI; 196 files (70k lines) do not; 86 files (22k lines) touch
  neither UI nor the scene, device or tools.
- **A start is file touches.** Warm: 3.7 s, 23,606 reads, 82,238 file operations - render
  device 0.94 s, editor library 0.92 s, game data 0.78 s, opening the editor 0.99 s, one after
  another.

Three facts that shape the plan:

- **The viewport renderer is small.** The editor draws through a D3D11 device with the R1
  forward blenders (9 files), 77 files of `XrRender/Private` and CPU skinning; its own shader
  set is 37 files. It is not the game's renderer.
- **A module's level data is mostly text.** `spawn\<level>.xspawn` is LTX (`[obj:<name>]`,
  `op = start|remove`, `[path:<name>]`, `custom_data = @...`); the engine composes it. Entity
  packets (`Spawn_Write`) are used by the scene file and the full level build only.
- **The game runs inside the editor.** `XrGameManager` loads `XrGame.dll` for the entity
  factory and `g_pGamePersistent`, and `EScene::Play` starts a game level in the editor's
  process. No Rust editor can keep that (section 12, D1).

## 4. Scope

| | |
|---|---|
| **Rewritten** | `LevelEditor`, `XrECore`, `XrEUI`, `XrEProps`, `XrETools`, every engine library the editor links, the entity factory (Dead Air flavour), the MCP bridge |
| **Rewritten last** (phase 8) | `XrLC`, `XrLCLight`, `xrDO_Light` - separate processes until then |
| **Kept as it is** | the game repository; the Python QA suites (they are the independent judge and only speak MCP); `tools/qa/github_mock` (Go); `tools/classic_mod` |
| **Kept as C** | LuaJIT, behind `mlua`: the editor runs the game's own pure-Lua readers, and they must behave as they do in the game |
| **Dropped** | Clear Sky and Shadow of Chernobyl flavours (`XrGameCS`, `XrGameSOC`, their factories, `fs_cs.ltx`, `fs_soc.ltx`); `ActorEditor`, `ParticleEditor`, `ShaderEditor` and the standalone `XrAI`, which no build target makes today; GameSpy, `XrNetServer`, BugTrap |

## 5. Target architecture

### 5.1 Principles

1. **Published data is immutable.** Whatever more than one thread can see is an `Arc` of a
   value nobody changes.
2. **One writer per store.** A store owns its state, takes commands from a queue and
   publishes a new snapshot. Nothing else writes.
3. **Everything that takes time is a task.** It reads a snapshot, runs on a pool, and comes
   back as a command or a query result.
4. **No singletons.** Stores, pools and services are handles passed down from `App`. There is
   no `static` holding mutable state.
5. **The domain knows no window.** Crates below the presentation layer depend on neither
   egui nor wgpu, so the whole editor except the picture runs headless.

### 5.2 Threads and executors

| Executor | Runs | Rule |
|---|---|---|
| UI thread | the message pump, the egui pass with its tessellation, command emission, the presentation of the window | reads snapshots with one atomic load; never blocks (I7); runs above normal priority |
| Render threads | the device is made on a thread of its own; the viewport's renderer, the offscreen passes (thumbnails, previews) and every readback run on theirs | a slow GPU frame of the viewport delays no input and no command |
| Async runtime (tokio, multi-thread) | store actors, the MCP server, network, timers, job orchestration | no blocking call inside; file work goes to the IO pool |
| CPU pool (rayon) | data-parallel work: validation, hashing, meshes, culling, layout, compression | pure functions over snapshots |
| IO pool (bounded) | blocking file operations | sized to overlap latency: a start costs touches, not bytes |
| Watcher | file change notifications | only bumps revisions; never reads a file |

The pools run at normal priority on every logical core (the IO pool on more): the editor's
work takes what the hardware has, and no pool is made smaller or slower for smoothness. The
thread that draws runs above normal instead, so the window gets its slice first. A winit
window's handle is reachable only from the thread that made it, which is why the surface
stays with the UI thread.

### 5.3 Stores, snapshots, commands

A store is an actor: a task that owns a state value and applies commands one at a time.

```rust
pub struct Store<S: State> { /* queue, published snapshot, history */ }

impl<S: State> Store<S> {
    pub fn snapshot(&self) -> Arc<S>;                      // lock-free, any thread
    pub async fn send(&self, cmd: S::Cmd, by: Origin) -> Result<S::Reply, CmdError>;
    pub fn rev(&self) -> Rev;                              // monotonic, bumps per command
}
```

- **Applying a command** builds a new state from the old one and swaps the pointer. A command
  that fails or panics leaves the published state exactly as it was - a half-applied edit
  cannot exist.
- **Large collections** (scene objects, graph nodes, the file tree) are persistent
  structures, so a snapshot per command costs O(log n) and shares the rest. Bulk data
  (meshes, textures, AI map tiles, detail slots) is immutable blobs behind handles; an AI
  map edit copies one tile.
- **Undo is the previous snapshot.** Every document has its own history; undo is O(1) and
  cannot disagree with the model.
- **`Origin`** is `Ui`, `Mcp` or `Unattended` and carries today's rule for questions: over
  MCP the caller decided; losing data takes the explicit `*_discard` command; under `-nodlg`
  the logged answers apply.
- **Stores never await one another.** Work that spans stores - renaming an asset that quests
  refer to - is a job that sends commands to each; every step is atomic, and no lock spans
  two stores.

The stores:

| Store | Holds |
|---|---|
| `App` | preferences, the project list, window and tab session |
| `Project` | `mod.ltx` (one lossless document every manifest editor writes through), `project.ltx`, the file tree, `[exclude]` |
| `Doc<T>` | one per open asset: quest, behaviour graph, character, PDA layout |
| `Scene` | the open level |
| `GameLink` | the linked game, read-only: file index, configs, string tables, kind catalog, picker data, `all.spawn` |
| `Jobs` | running and finished jobs, the Task Center's cards |

One manifest document replaces "re-read the manifest first" and the generation counters
tabs poll today: there is one writer, and every tab reads the same snapshot.

### 5.4 Derived data

Lists that are cached today and dropped by an `Invalidate()` call (character briefs, the
kind catalog, pickers, project globals, tile icons, references, validation problems) become
**queries**: a pure function of snapshots and file revisions, memoised by those revisions,
computed on the CPU pool.

A query answers at once with `Fresh(value)`, `Stale(previous)` or `Pending`; the UI draws
what it has and marks it busy. A revision that moves while a query runs does not cancel what was started: the reader
waits for it and reads again. A computation that panics is remembered as failed for its
revision. Nobody
calls `Invalidate()`: an input changed, so its revision changed.

### 5.5 Jobs

One job system for everything long: builds, packaging, publishing, scans, scene loads, level
exports, thumbnails. A job has a name, stages, progress, a cancel token and a result; the
Task Center shows it, Stop cancels it, MCP waits for it (`FollowsJob`) while other commands
run. "One build at a time" stays a rule of the build, not a limit of the system.

### 5.6 UI

- **egui**, immediate mode like the ImGui it replaces: a surface is a function of snapshots
  and its own view state that returns commands.
- **The frame loop is the editor's own** (`gz-app/src/window.rs`: winit, egui-winit and
  egui-wgpu, no eframe): frames run on demand, a frame-held request asks for the ones it
  needs, and leaving stops every thread that draws before the device goes.
- The Level tab's dock is the kit's (splitters and slots; `egui_dock` is not used). A panel may
  leave the window as a native window of its own - an opt-in preference that takes a restart.
- **The kit stays the only way to draw** (`gz-ui`), and `docs/UI_DESIGN.md` stays the
  contract. What a lint checks today the types check: a size is `Pt`, never a bare number; a
  colour is a theme token; a shortcut label comes from the binding table.
- Every kit widget registers itself in a per-frame tree (id, rectangle, text, whether it was
  cut short). `ui_list`, `ui_audit` and `titlebar_info` answer from it.
- A surface never computes: anything beyond a few microseconds is a query or a job. There
  is no nested frame to forbid, because no surface can start one.
- The window frame (custom caption, hit test, DPI, dropped files) is the one place of UI that
  needs Win32 directly; it lives in `gz-win`.
- The system's file dialog is one kit function (`gz_ui::file_dialog::pick`): it opens nothing
  for an MCP caller or under `-nodlg`, and the command answers "name the file".

### 5.7 MCP

- A command is registered by name with its area (`gz_editor::Registry::add(area, name, fn)`)
  and changes state through a store, which the UI has no other way to do (I8). **The
  catalogue is `crates/gz-mcp/tools.json`**: every tool with its description, schema,
  annotations and the safety paragraphs - the C++ bridge's export plus what the Rust editor
  adds. The bridge serves it as written; `Grizzly.exe -dump_commands` prints the registry and
  `tools/rust/check.sh` fails when the registry lacks a command of the catalogue.
- **Who asked travels with the command** (`Origin`: the window, an MCP caller, `-nodlg`). A
  question raised for an MCP caller is answered, never shown - also when the caller runs a
  row of the command table (`exec_command`), a macro, or anything the row starts.
- The server is a task per connection. Reads answer from snapshots without visiting the UI
  thread; writes go to the store's queue. A command no longer waits for a frame unless its
  contract is a frame.
- **Frame-held requests keep their contract.** `screenshot_editor`, `screenshot_viewport`,
  `ui_list`, `ui_audit`, `titlebar_info` and the `pending` answers are answered by a frame -
  of the window and, for the viewport, of its own pass - that STARTED after the request's
  commands were applied; a frame already in flight does not count. A screenshot also waits
  until the pass shows the scene revision those commands produced and nothing it asked for is
  still loading; past a bound it answers what was still pending, never a silent half picture.
  A minimised or background editor draws when such a request asks.
- The wire stays what the QA suites speak today (JSON over TCP on 127.0.0.1). The stdio
  bridge becomes a small Rust binary; tool descriptions and the `PLAYER_SAFETY` /
  `WITHDRAW_SAFETY` paragraphs move with it word for word.

### 5.8 Renderer

- **wgpu**, a new forward renderer written for the editor. Its API is safe, the device and
  the queue are shared between threads, command recording runs in parallel, and egui draws
  through the same device.
- Engine shader names (`shaders.xr`, a surface's shader, a model's material) map to a small
  set of editor materials written in WGSL: opaque, alpha test, blend, self-lit, terrain with
  detail, tree and LOD, water, sky, particles, wireframe and selection. The viewport shows
  what an author needs to place and judge; the look of the game is the game's.
- **Frame packets.** Extraction reads a scene snapshot and a camera, culls and sorts on the
  CPU pool, and hands an immutable packet to the render thread. The scene is never locked to
  be drawn. `gz-render` knows no scene: it takes a plain `DrawList`, and the adapter that
  makes one of a snapshot is the viewport's (`gz-ui-viewport/src/draw*.rs`). Particles are a
  crate of their own (`gz-particles`: the engine's actions stepped off the UI thread).
- Textures and meshes decode on the pools and upload through the queue; a missing resource
  draws a placeholder and fills in.
- Thumbnails, `preview.png`, the model and texture viewers are offscreen passes scheduled as
  jobs.
- **Gate and fallback:** spike S2 (section 11, phase 0). If wgpu cannot carry the viewport,
  the renderer becomes D3D11 through `windows-rs` in one extra allow-listed crate, with the
  same frame packets. Nothing above the renderer changes. *Outcome: wgpu carries it; the
  fallback was not taken.*

### 5.9 Files and the linked game

- **A virtual file index**, immutable, built by a parallel walk: project, linked game
  (`gamedata`, `modules\`, `mods\`, archives), editor data. Lookups never touch the disk.
- **The index persists** between starts and is validated by directory write times, so a
  start opens files it needs instead of listing everything. The startup profile stays: one
  line per stage with seconds, reads and file operations, and the loader line.
- Tooling entries (dot-names, `Thumbs.db`, ...) are filtered in the walk itself - one place
  for the browser, the asset index and the build.
- Text is UTF-8 inside; cp1251 is converted explicitly where the game's files are read or
  written.
- **The game's own files are never written.** Every writer of the editor - the project's file
  operations, the module build and the flat export, the level exports, the level build, the
  release stage, the compilers' stage and Play's game root - passes one guard
  (`gz_editor::fsx::WriteGuard`) before it writes: nothing under the linked game's root, judged
  as written and where the path really is, except the module's `modules\<id>` (with its stage
  in `modules\.new`) and the flat `gamedata_<id>`.

### 5.10 Errors, crashes, log

- Fallible code returns `Result`; `unwrap` and `expect` are denied outside tests.
- A panic inside a command or a job fails that command or job, writes a crash report, and
  leaves every store on its last good snapshot. Under `-nodlg` any panic is a symbolised
  stack, a dump and exit code 3, as today.
- The log is a channel to one writer thread; the Output Log reads a ring. The file keeps its
  format, the startup profile and the code page line.

### 5.11 What runs in parallel

| Area | Today | Target |
|---|---|---|
| Start | stages in sequence, 3.7 s warm | a task graph; the window and the Start page first, the rest fills in |
| Game link | configs, strings, descriptions read in sequence | files read and parsed in parallel, merged once; index persisted |
| Content browser | listing and kind detection on the UI thread | streamed listing, kinds and thumbnails as queries |
| Validation, references, find | per call, on the caller | per asset in parallel, incremental by revision |
| Build | one job thread | parallel walk, hash, copy and link; checks as parallel queries |
| Packaging | one thread | entries compressed in parallel, written in order, hashed while fed |
| Publishing | one worker | bounded parallel uploads; one shared VirusTotal pace |
| Scene load | main thread, UI frozen | tool parts and library meshes in parallel; the editor stays usable |
| Scene save, recovery | pauses editing | a snapshot serialised on a pool; editing never stops |
| Picking, snapping, wall scan | Opcode, one ray at a time | BVH built in parallel, ray batches in parallel |
| Level exports | sequential | per level and per artifact in parallel |
| Viewport | draws inside the UI pass | render thread, parallel extraction and recording |
| Lua readers (PDA) | one VM on the main thread | a pool of VMs, one per worker |
| MCP | workers queue for the main thread | concurrent; reads never touch the UI thread |

## 6. Rust rules

**`unsafe`.** The workspace sets `unsafe_code = "forbid"`. The allow-list:

| Crate | Why |
|---|---|
| `gz-win` | Win32 that `std` lacks: caption hit test, per-monitor DPI, dropped files, the clipboard, reparse tags and final path names, known folders, Credential Manager, a minimised start, the crash dump |

- `gz-win` is the whole list: the crash handler (`gz-crash`) needs none of its own, and the
  renderer fallback of 5.8 was not taken.
- The crate exposes a safe API only; every `unsafe` block carries a `// SAFETY:` line.
- **No `unsafe impl Send` or `Sync` anywhere** without a line in this section. That is where
  a race would get back in.
- `tools/rust/check.sh` fails on `unsafe` outside the allow-list and prints the count
  inside it.

**Shared state.**

- No `static mut`, no lazily initialised global holding state, no singleton accessor.
- `Rc` and `RefCell` only in a surface's own view state on the UI thread.
- A mutex guards a small private structure and is never held across an `.await`, a frame or
  a call into another module. Two locks at once need a documented order.
- The synchronisation pieces (the store, the frame stamp, the job token) are compositions of
  tokio channels and `arc-swap` with no lock-free code of our own, so they are held by stress
  tests in the workspace's test run, not by a model checker.

**Dependencies.**

- Pure Rust first. The C dependencies are two: LuaJIT (vendored by `mlua`) and libvorbis with
  the aoTuV tunings (vendored by `vorbis_rs`, a safe API) for the game sounds the Sound Library
  encodes.
- The toolchain is pinned in `rust-toolchain.toml` (1.99.1, edition 2024); the workspace
  declares no dependency that no crate uses.
- TLS trusts the **Windows certificate store** (schannel or the platform verifier), not a
  bundled root list: a machine whose antivirus inspects HTTPS must keep working.

## 7. Workspace

`Cargo.toml` at the repository root, crates under `crates/`. A lower layer never depends on
a higher one; layers 0-3 depend on neither egui nor wgpu.

| Layer | Crates |
|---|---|
| 0 Base | `gz-base` (ids, revisions, `Pt`, paths, cp1251, `Origin`), `gz-task` (executors, jobs, cancel), `gz-store`, `gz-query`, `gz-log`, `gz-crash`, `gz-win`, `gz-corpus` (support of the corpus tests) |
| 1 Formats | `gz-ltx` (lossless: comments, order and spacing survive an edit), `gz-luadata` (declarative Lua, refuses code), `gz-chunk` (chunk streams, LZO), `gz-fmt-object`, `gz-fmt-ogf`, `gz-fmt-tex` (DDS, TGA, THM), `gz-fmt-level` (cform, xcform, AI map, details), `gz-fmt-spawn` (`all.spawn`, game graph, patrols), `gz-entities` (server entity schema and packets), `gz-fmt-xr` (`shaders.xr`, `gamemtl.xr`, `lanims.xr`, `particles.xr`), `gz-fmt-xml`, `gz-fmt-sound` |
| 2 Game link | `gz-vfs`, `gz-lua` (host for the game's pure-Lua readers) |
| 3 Domain | `gz-editor` (the editor's handle: stores, services, the command registry, the frame clock), `gz-project`, `gz-game` (configs, strings, modes, communities, ranks, trade presets, fonts, kind catalog, world: it reads the mount the project holds, so it sits above `gz-project`), `gz-mod`, `gz-nq`, `gz-characters`, `gz-pda`, `gz-screens`, `gz-world`, `gz-build`, `gz-release` (packages, the index, signing, the VirusTotal scan), `gz-github`, `gz-publish`, `gz-catalog`, `gz-scene`, `gz-particles`, `gz-export-level`, `gz-levelbuild` |
| 4 Presentation | `gz-mcp` (the server and the catalogue), `gz-ui` (the kit, icons), `gz-render`, `gz-app` (the window, the shell, the command table), and a crate per surface: `gz-ui-app` (Start page), `gz-ui-content`, `gz-ui-graph`, `gz-ui-level`, `gz-ui-viewport`, `gz-ui-mod`, `gz-ui-pda`, `gz-ui-previews`, `gz-ui-release`, `gz-ui-world` |
| Binaries | `grizzly` (the editor; `--headless` runs layers 0-3 and MCP without a window), `gz-mcp-bridge` (the stdio bridge `grizzly-mcp`) |

An area crate is its domain plus `service.rs` and `cmd.rs` with one `register(reg)`; a surface
crate has one `install(builder)`. The scripts that check and test the workspace are
`tools/rust/check.sh` and `tools/rust/test.sh`; there is no `xtask`. The exact layer of every
crate as `cargo metadata` gives it is in `docs/rust/STATE.md` ("For the plan").

## 8. What replaces what

| Today | In Rust |
|---|---|
| `Scene`, `EDevice`, `UI`, `Tools`, `FS`, `ELog` globals | stores and handles (5.3) |
| Dear ImGui (patched) + `XUI` | egui + `gz-ui` |
| `XrEProps` property items and callbacks | a typed property schema per object; the Details panel and `property_*` read the same schema |
| D3D11 device + R1 blenders | wgpu + editor materials (5.8) |
| `CLocatorAPI`, `CInifile` | `gz-vfs`, `gz-ltx` |
| `xr_string`, `shared_str` (5,428 uses) | `String`, `SmolStr` for the short names |
| Opcode (`XrCDB`) | the scene's own BVH with a parallel build (`gz-scene/src/pick.rs`) |
| ODE (`XrPhysics`: dropped objects, the physics shells of spawn points and previews) | ray casts against that BVH; nothing is simulated in the editor |
| `XrSE_Factory` + `XrGame.dll` | `gz-entities`: a declarative schema of Dead Air's classes |
| luabind + LuaJIT | `mlua` + LuaJIT for the game's readers; `gz-luadata` for assets |
| WinHTTP | an async HTTP client on the Windows certificate store |
| BCrypt (SHA-256, PBKDF2, HMAC, ECDSA P-256) | RustCrypto (`sha2`, `pbkdf2`, `hmac`, `p256`) |
| minizip | the editor's own zip writer over `flate2`, deflating in parallel; the layout is read back from the finished archive |
| nvtt, ispc_texcomp, stb_image, RedImage | `image`, a DDS codec with BC1/BC3 and mip generation |
| `XrSound` (OpenAL), Vorbis | `cpal` for output, `lewton` to decode; OGG comments by `gz-fmt-sound` |
| `ogg_enc` (oggenc + libvorbis) | `vorbis_rs` (libvorbis aoTuV, vendored): VBR at the description's quality, the parameters put in as the first comment by `gz-fmt-sound` |
| QSlim, WildMagic, FreeMagic | ported with the mesh tools that use them |
| `tools/mcp/grizzly_mcp.py` | `grizzly-mcp`, serving the catalogue `crates/gz-mcp/tools.json` |
| Win32 file dialogs | `rfd`, each on a thread of its own |
| `tools/qa/lint_ui_units.py` | the kit's types |

## 9. The contract with the game

Every artifact that leaves the editor for the game, a release or the catalog, and every
file of the game the editor reads. The normative text stays where it is; the Rust editor is
held to it by the proof in the last column.

| Artifact | Normative text | Proof |
|---|---|---|
| `mod.ltx`, all sections | game `MODDING.md`, `XMS_ARCHITECTURE.md` | lossless round trip over the corpus; equal text after equal commands on both editors |
| `.nqasset`, `.behasset` | `docs/nq/NQ_FORMAT.md`, game `NQ_RUNTIME.md` | equal text after equal commands; game `run_lua.py test_examples.lua`, `Run-NqQa.ps1` |
| `.npcasset` | game `MODDING.md`, "A module's characters" | `test_characters.py`; game `Run-ScreensQa.ps1 -Part Descriptions` on the characters of a module the Rust editor built |
| `.pdaasset` | `docs/pda/PDA_FORMAT.md` | the game's `xms_pda_layout.script` executed unchanged; `test_pda.py`; game `test_pda_layout.lua` |
| Screens and music files | game `MODDING.md`, "Screens and music" | `test_screens.py`; game `Run-ScreensQa.ps1` |
| `.xspawn`, `spawn\custom_data\*.ltx`, `overlay_visuals.ltx` | game `MODDING.md`, "World rules" | byte-equal to the oracle on the Escape scene (`tools/qa/diff_editors.py`, scenario `s6_scene`) |
| `overlay.xcform`, the cut, exported `.ogf`, the AI map | game `XMS_ARCHITECTURE.md` | equal to the oracle under the rules below (`diff_editors.py`; golden files in `gz-export-level/tests`) |
| What Play builds: the collision, the game graph, the cross table, `editor.spawn` | engine formats | the game loads the level (`tools/qa/test_play_game.py`: a new game, the player, the level's geometry and collision, synchronised); a unit test holds what the game's loader relies on. The C++ editor has these on toolbar buttons with no command, so there is no oracle record |
| The installed module (`modules\<id>`, `.new\<id>`, hard links, tooling left out) | this repository's rules, game `xms_core.cpp` | `test_build_job.py`, `test_build_contents.py`; file set and bytes equal to the oracle (`diff_editors.py`, 497 files); one in-game run of the game's `PdaModule` scenario on a module the Rust editor built |
| Flat export `gamedata_<id>` | - | file set and bytes equal to the oracle |
| `<id>.update.ltx`, `<id>-<version>.files`, the packages | game `MOD_UPDATES.md` - **frozen** | descriptor and index equal to the oracle except the fields that state bytes on disk; the index read back from the finished archive and each record's range checked (`gz-release`); `test_mod_publish.py` against the mock. The game's `Run-ModsQa.ps1` makes its own releases and takes no outside one, so it proves the game's side only |
| `[card]`, `[signature]`, `author.xkey`, issue bodies, content rules | game `MOD_CATALOG.md` | both ways against the catalog's `xms_catalog.py` and `catalog.py`; `test_mod_catalog.py`; the game verifies the signature |
| `thumbnail.dds` | game `MOD_CATALOG.md` | format, size and mip chain checked; the game shows it |
| Reads of the linked game (never written): `all.spawn`, configs, string tables, `textures_descr`, UI XML, fonts, kind catalog, archives | the game's files as they are | pickers and lookups answer as the oracle does (`quest_lookup`, `world_transitions`, `game_modes`, `list_spawn_sections`, ...) |
| A full level (`level`, `level.geom`, `level.cform`, `all.spawn`) | engine formats | made by the C++ compilers until phase 8, from the editor's build output (`build.prj`, `level.spawn`, the HOM / SOM, details and AI map files), which equals the C++ editor's under the rules below |

**The rules of "equal to the oracle"** for the level exports and the level build. Byte for
byte unless named here; there is no broad ignore (`diff_editors.py` `FILE_RULES`):

- `overlay.xcform`, the cut: the material table's order (the C++ fills it in the order a map
  keyed by `CSurface*` meets the surfaces - D14); each triangle's material is compared by name.
- An exported `.ogf`: its children (one per surface) as a multiset of their bytes and the
  collision mesh as a multiset of triangles, for the same pointer order; every other chunk
  byte for byte.
- `build.prj`: the material, shader and texture tables as sets of what they name, faces as a
  multiset with the material resolved (the pad of `b_face` left out); a point light's colour
  within 4 ULP and a LOD model's corners and UVs within 2 micrometres (the two compilers order
  the float operations differently); fields the C++ never initialises left out (a LOD
  material's `reserved` and `area`, an unset `Flight`).
- `level.spawn`: the Rust build's breakable and climable entries are extra (D10); a smart
  cover's loopholes as a set (the C++ order is Lua's `pairs`).
- A portal of `build.prj`: sectors and corner counts equal, corners within 0.1 mm (the corners go
  onto the portal's plane and back in float with the operations in another order); a portal
  simplified from more than six corners passes when the two polygons' areas agree within 2 %.
- `build.details`: a model's UVs up to one shift of its own, and `build_details.dds` tile by
  tile within DXT5's error - the atlas is packed in an order the C++ takes from string
  addresses (D14), and the encoder is another.
- `level_stat.txt` / `.tga`, read by no compiler and not by the game: the maxima within 0.5 %.

Compared this way: Make All on Escape and on `l10_limansk` (79 sectors, 274 portals, 251001
detail slots, 127 wallmarks), and the Build menu's Make Game, Make AI Map, Make Details, Make
HOM and Make SOM one at a time. With every file equal under these rules the compilers get the
same input; the level that Play builds is compiled by the repository's `XrLC` and loaded by the
game. **A level build writes into the open project** (`gamedata\levels\<level>`,
`gamedata\spawns`) and never into the game; the level path is one folder name, and a level
folder is wiped only when its real path lies below the project's levels folder and is no link.

Two consequences worth stating:

- A signature made by the Rust editor differs in bytes from one made by BCrypt for the same
  input (ECDSA picks a nonce). Both verify; the contract is verification.
- A package's compressed bytes may differ from minizip's. The index states what is on disk
  and is read back from the finished archive, as today.

## 10. Verification

1. **The oracle.** The C++ editor at the branch point, driven over MCP.
   `tools/qa/diff_editors.py` runs both editors side by side on copies of one project.
2. **Golden files** for the rows of section 9, and for every format read the corpus: parse,
   write, compare over the linked game's and the projects' own files. `tools/rust/test.sh`
   runs only the test binaries that changed since they were green; `--all` runs everything
   and reads EVERY file of the corpus.
3. **Differential MCP.** The same command script against both editors; the JSON answers are
   compared, minus a short documented list of fields (timings, pixel rectangles).
4. **The QA suites, unchanged in what they check.** Their one edit: the editor binary and
   the port become parameters. They run through `tools/qa/run_all.py`, side by side, every
   editor in a home of its own (`-home`), so no run touches the author's project list or
   settings. **A suite opens only a copy of a project under the system temp folder**
   (`qa_target.copy_project`; `guard_project` stops it before an editor starts otherwise).
5. **The game is the judge.** Modules and releases made by the Rust editor go through the
   game's own suites. Runs are short and targeted.
6. **The parity ledger.** `docs/rust/PARITY.md`, made by a read-only audit of the C++ editor
   against the Rust one (menus, commands, tools, panels, dialogs, keys) and kept by the
   differential run: every item with its state. The cutover needs every line done or listed
   in section 12.
7. **Concurrency.** Stress tests of the store, the jobs, the frame clock and the queries in
   the workspace's test run; a stress run of the whole editor: MCP commands flooding from
   several connections while a build, a scene load and UI edits run.
8. **Budgets**, checked from phase 3 on: the window on the screen within 0.3 s of the process
   start (warm; its first frame follows when the driver has made the device),
   fewer file operations per start than today's 82k, no UI frame over 16 ms caused by the
   editor's own work, a rebuild no slower than the C++ job.

## 11. Phases

Each phase ends in something usable and has a gate. "Replaces" is the C++ it makes
unnecessary, in lines, rounded.

### Phase 0 - Foundation and spikes

Goal: the skeleton every later crate stands on, and the three bets proven or refuted before
anything is built on them.

- The workspace, the pinned toolchain, lints, `tools/rust/check.sh` (format, clippy, the
  `unsafe` audit, the registry against the catalogue) and `tools/rust/test.sh` (the tests).
- Layer 0: `gz-base`, `gz-task`, `gz-store`, `gz-query`, `gz-log`, `gz-crash`.
- `gz-ltx`, `gz-luadata`, `gz-chunk` - the three formats everything else reads.
- The oracle harness: the differential runner (`tools/qa/diff_editors.py`), the parity ledger.
- **S1, the shell:** egui on wgpu with a docked layout, the custom caption with its hit
  test, per-monitor DPI, Segoe UI and Cascadia Mono at 100% and 150%, Cyrillic input, a
  second native window, a 3D texture inside a panel, `screenshot_editor` with a frame stamp,
  `ui_list` from the kit's tree.
- **S2, the renderer:** one reference scene's static geometry with textures, alpha, terrain
  and selection in wgpu, beside the oracle's `screenshot_viewport`.
- **S3, the concurrency skeleton:** a store, a query, a job and the MCP server under the
  stress run of 10.7, with a held screenshot answering correctly throughout.
- **S4, the start:** the linked game indexed in parallel, with the persisted index.

Gate: S1-S4 pass, or the fallback of each is written into this document (S1: keep the
patched ImGui behind a Rust kit; S2: D3D11, 5.8). The ledger exists and lists everything.

### Phase 1 - Formats and the linked game

Replaces about 20k (the file system, configs and readers the editor uses from the engine).

- The rest of layer 1 and layer 2: every format the editor reads, the file index, the game
  link (configs, strings in cp1251, modes, communities, ranks, trade presets, fonts, the
  kind catalog, `all.spawn` with levels, changers and patrols).
- `gz-entities`: the schema of Dead Air's server entity classes, read and write. An unknown
  class is kept as an opaque packet, byte for byte, and shown read-only - never lost.
- `gz-lua` running `xms_pda_layout.script` from the linked game.

Gate: every file of the corpus and of the linked game parses; round trips are byte-equal;
every entity packet of the game's `all.spawn` and of the reference scenes reads and writes
back identically; lookups answer as the oracle does.

### Phase 2 - The headless editor

Replaces about 55k (`XrECore/Editor` without its windows).

- `gz-project`, `gz-mod`, `gz-nq`, `gz-characters`, `gz-pda`, `gz-screens`, `gz-world`,
  the content operations, with their commands.
- `gz-build`, `gz-release`, `gz-github`, `gz-publish`, `gz-catalog`: Build Into Game,
  Build to Folder, Export Flat, Package Release, Publish Release, Submit, Withdraw, the
  releases list.
- `gz-editor` and `gz-mcp`; `grizzly --headless`; `grizzly-mcp`.

Usable: an agent drives a whole mod - assets, checks, build, release - without a window.

Gate: the differential runner passes for every command that needs no picture; build, flat
export and package outputs match the oracle on the reference projects (section 9); the
game's suites accept a module and a release made here; `test_github_mock.py` passes, and
`test_mod_publish.py` and `test_mod_catalog.py` pass in their non-UI phases against the mock.

### Phase 3 - The shell and the document tabs

Replaces about 70k (`XrEUI`'s kit and shell, `LevelEditor/UI` except the level's panels).

- `gz-ui`: tokens, units, icons, dialogs and sheets, toolbars, the shell (tabs, drawers,
  status bar, palette, key routing), the UI tree.
- The Start page and project browser; the Content Browser and the Output Log; the Task
  Center; Preferences and shortcuts.
- Mod Settings, World Rules, Characters, Screens & Music, Releases, the publish and submit
  dialogs, the graph tab (quest and behaviour), the PDA preview, the three libraries.
- `preferences`, `ui_*`, `window_control`, `titlebar_info`, the screenshots.

Usable: everything about a mod except its level is done in the Rust editor, on the same
project the C++ editor edits the level of.

Gate: `test_ui_shell`, `test_ui_tour`, `test_graph_ui`, `test_project_browser`,
`test_asset_icons`, `test_unicode_names`, `test_characters`, `test_behaviours`, `test_pda`,
`test_screens`, `test_build_job`, `test_build_contents`, `test_mod_publish`,
`test_mod_catalog`; the budgets of 10.8.

### Phase 4 - The scene

Replaces about 45k (`LevelEditor/Editor`: scene, entries, tools, recovery). Runs beside
phase 3; needs phases 1-2.

- `gz-scene`: the model of the 16 object classes and their tools (objects, lights, glows,
  shapes, spawn points, way paths, sectors, portals, groups, sound sources and
  environments, particles, details, AI map, wallmarks, fog volumes), selection, transforms,
  the typed properties, undo by snapshot.
- Scene files: read and write the format the C++ editor uses (I4).
- Picking, snapping, drop to ground and wall scan on the scene's parallel BVH.
- `gz-export-level`: `.xspawn`, custom data, `overlay.xcform`, `overlay_visuals.ltx`, the
  AI map and graph fragments; OGF export; recovery snapshots.
- Every scene command over MCP, headless.

Gate: each editor opens what the other saved and answers the same `scene_info`,
`list_objects` and `object_info`; exports match the oracle (section 9); the scene commands
pass the differential runner.

### Phase 5 - The renderer and the viewport

Replaces about 50k (the device, `XrRender` as the editor uses it, previews). Runs beside
phases 3 and 4; needs phase 1.

- `gz-render`: resources, the editor materials, static and skinned models with animation,
  LODs, details, terrain, sky and the game-lighting mode, particles, glows, wallmarks.
- The viewport: camera, grid, gizmos, selection, view modes, the stats pill.
- Offscreen passes: thumbnails, `preview.png`, the model viewer, the texture viewer.
- `screenshot_viewport`, `camera_*`, `view_mode`, `frame_pacing`.

Gate: `test_previews`; the reference scenes side by side with the oracle show the same
objects, textures, transparency and selection (judged, not pixel-matched); frame pacing
within budget on the largest reference scene.

### Phase 6 - The level editor whole

Replaces about 12k (the builder, the in-editor AI, the glue).

- The level's panels: World Outliner, Details, Level Settings, the tool panels and
  choosers, on the scene of phase 4 and the viewport of phase 5.
- `gz-levelbuild`: the build project for the compilers, the game graph and spawn
  constructors, launching `XrLC` and `xrDO_Light` as jobs.
- Play: the game started on the scene as a job (D1).

Gate: `test_properties`, `test_world`, `tools/mcp/test_server_lifecycle.py`,
`benchmark_scene_load.py` no slower than the oracle; a level built here loads in the game.

### Phase 7 - The cutover

- Every QA suite green against the Rust editor; the parity ledger complete; the game's
  checks that take the editor's output green on it: `run_lua.py test_examples.lua`,
  `test_pda_layout.lua`, `Run-NqQa.ps1 -Scenario PdaModule`, `Run-ScreensQa.ps1 -Part
  Descriptions`, and a level started by Play loaded by the game. (`Run-ModsQa.ps1` and
  `Run-NqQa.ps1 -Scenario Scout` make their own modules and releases: they prove the game's
  side of the contract, not the editor's output.)
- One commit deletes the C++ editor and the engine libraries only it used. What the
  compilers still need stays until phase 8.
- `CLAUDE.md`, `tools/mcp/AI_SETUP.md` and `README.md` are rewritten for the new editor;
  `Build.ps1` builds it; the default MCP port and the binary name return to the editor's
  own.
- The branch merges into `main`.

### Phase 8 - The compilers

`XrLC`, `XrLCLight`, `xrDO_Light` (36k) become Rust crates and jobs of the editor. Their
results differ in the last bits of floating point, so the gate is the quality of light
against the C++ output and a level the game loads, not equal bytes. After it the repository
holds no C++.

## 12. Deliberate differences

Everything the Rust editor does differently on purpose. A difference that is not here is a
bug.

| # | Difference | Why |
|---|---|---|
| D1 | **Play in Editor starts the game.** Play bakes the scene, builds the collision, the AI map, the game graph and `editor.spawn`, compiles the level with the repository's compilers (from a stage in the project's `temp\compilers`) and runs the LINKED game's engine on it as a job, in a game root of its own outside the project (copies of the executables, a junction to the archives, a copy of the loose gamedata with the level added, an appdata of its own, no module) - instead of running a game level inside the editor. | The embedded play needs `XrGame.dll` and the whole engine in the editor's process. The real game shows what a player gets, and its crash no longer takes the editor along. The game's `CLocatorAPI` copies every path into 256 bytes, so Play refuses a game root too deep for it and names the path. |
| D2 | **The viewport is drawn by the editor's own materials**, not by the R1 blenders. | 5.8. What an author judges - geometry, textures, transparency, placement - stays. |
| D3 | The editor's own settings (window, layout, preferences) live in new files; nothing is migrated. | The editor has not shipped; the existing rule. |
| D4 | Signatures and package bytes differ from the C++ editor's for the same input. | Section 9. |
| D5 | Clear Sky and Shadow of Chernobyl data are not supported. | Mods target Dead Air Refined strictly. |
| D6 | The raw-pixel and colour lint is gone. | The kit's types refuse what it searched for. |
| D7 | No splash window. | The editor's own window is on the screen 0.07 s after the process starts, in the theme's colour; its first frame follows when the graphics device is ready (about a second, most of it the driver's adapter and surface). |
| D8 | Operations the C++ editor could not undo are one undo step: portal computation, detach, the clears; Edit > Delete and the Delete key ask nothing (the Outliner's Delete still names what goes). | Undo is the previous snapshot, so it costs nothing to offer. |
| D9 | A classic module keeps `target = any` when its manifest is saved. | The C++ `mod_set_manifest` rewrote it to `existing`, against the project's own rule for classic mods. |
| D10 | Breakable and climable objects are built. | The fork's `VCPacked::add_vert` lost its welding, so the C++ editor of the branch point writes errors instead; the Rust build welds on the SDK's grid. |
| D11 | Tools that did nothing in the C++ editor work: the AI Map tool's Ignored Materials leave holes, the link pad's Invert turns a one-way link round, a wallmark can be moved, a marquee in points mode selects the points inside it, a library group is placed by a click. | Each was a commented-out check, a self-link, a disabled control or a missing handler in the C++. |
| D12 | A group's Make Thumbnail writes `<project>\rawdata\groups\<reference>.thm`. | That is where the Groups chooser reads it; the C++ wrote `$objects$\<scene name>.thm`, which nothing reads. |
| D13 | The wire adds, never removes: refusals of `game_link` / `game_modes` carry `error`; `characters_write` refuses a path outside the project; `world_set` writes only its section; `mod_publish start` answers in one call; `quest_history` adds `undo_steps` / `redo_steps`; new commands `jobs`, `level_build`, the tool commands (`sector_tool`, `portal_tool`, `way_tool`, `shape_tool`, `light_tool`, `fog_tool`, `aimap_tool`, `details_tool`, `wallmark_tool`, `particles_tool`), the scene's own, `viewport_gesture`, `spawn_animation`, `-home`; a form that discards is a tool of its own with `destructiveHint` (the level tools included). | Each closes a hole an agent would otherwise fall into; nothing a C++-era script sends changes meaning. |
| D14 | Orders the C++ took from pointer addresses or `rand` are stable. | Section 9: the game does not depend on them, and a stable order makes a build reproducible. |
| D15 | The command table lists what does something: the 35 C++ rows that are plumbing or whose bodies do nothing are not listed, and the editor ships its default keys. | A row that does nothing is noise for an author and a trap for an agent; the C++ bound no key until a `level.ini` existed. |
| D16 | Faults of the C++ editor are not repeated: the particle library holds all of Dead Air's `particles.xr` (the C++ `CPSLibrary::Load` stopped at the first short `TargetColor`: 1035 names of 1428); the unsigned-release sentence is a warning of its own (the C++ glued it to the last build warning); `scene_visibility` takes a JSON boolean. | Each was an accident of the code, not a behaviour anyone relied on. |
| D17 | A ray pick hits the scene whatever the camera sees (`probe_vertical`, `scan_walls`, `drop_objects`, way point snapping). | The C++ `CSceneObject::RayPick` skipped objects outside the camera's view, so an MCP answer depended on where a minimised window happened to look. |
| D19 | A spawn point is placed only for a class the game's factory has a server class for, and a new smart cover's hold position time starts at 0. | The C++ placed a spawn point of undefined type for the first and left the second uninitialised. |
| D23 | Play's game graph is the linked game's graph with the level appended (the game's vertex numbers and graph guid unchanged), the way the game adds a module's level; a scene named like a level the game already has keeps the C++ single-level graph. | Dead Air's own start scripts spawn on fixed vertices of the game's graph; on the C++ editor's one-level graph the game stops there. |
| D20 | The level path is one folder name; anything else is refused when typed and by the build. | The C++ took any text and concatenated it; a path that leaves the levels folder would be wiped by Make All. |
| D21 | Make All builds a scene that holds an object in visual mode. | The C++ `BuildHOMModel` asserts on it. |
| D22 | A shape that Attach on Click detaches from a spawn point gets a free `shape` name. | The C++ appended it nameless. |
| D18 | The six Detail Objects settings live on the Detail Objects panel (and `details_tool`), the tools' Enable / Read Only switches in Level Settings > Targets. | The C++ kept the first in Level Settings and the second in Preferences; UI_DESIGN 1a: one task in one view. |

## 13. Working rules

- **The branch.** All Rust work lands on `grizzly-rust`. `main` merges into it regularly;
  the last merged commit is recorded here: **synced at `25f76d13`**.
- **What changed in C++ since then is the port list.**
  `git diff <synced>..main -- Source/Editors docs tools` - no separate bookkeeping.
- **Areas flip one at a time.** When an area passes its gate, new work on it lands in Rust
  and the C++ side takes fixes only. Until then a feature lands in C++ and joins the port
  list.
- **Side by side.** The Rust editor is `Intermediate\cargo\debug\Grizzly.exe` and listens on
  28017 until the cutover (`-mcp_port` names another; QA uses 28917); the project list is
  shared, the tab session and the settings are its own. The QA suites pick the editor through
  `tools/qa/qa_target.py` (`GRIZZLY_QA_TARGET=rust`).
- **A work package is done** when its code, its commands, its tests and its line of the
  parity ledger are in, and the document that describes its behaviour still tells the
  truth.
- **Rules that describe behaviour carry over; rules that describe ImGui do not.** A modal
  stays inside the editor's window, a minimised editor is pictured restored, a screenshot
  is one call - those are requirements to prove again on the new shell, not code to port.

## 14. Risks

| Risk | What we do about it |
|---|---|
| egui falls short of the shell (text sharpness, the caption, docking polish, large tables) | spike S1 before any surface is built; fallback named in the gate of phase 0 |
| The new renderer shows less than the old one | spike S2; the D3D11 fallback behind the same frame packets |
| The entity schema drifts from the game's classes | every packet of the game's `all.spawn` must round trip; unknown classes are preserved, never dropped |
| Floating point differs between compilers in `xcform` and AI map output | the same order of operations and no fast-math; a stated tolerance and the game's load where bytes cannot match |
| Two editors to maintain during the move | the port list from git, areas flipping one at a time, both on one project |
| Persistent structures cost more than vectors on hot paths | measured in S3 and S4; bulk data stays in flat immutable blobs |
| Deadlocks, which the compiler does not catch | stores never await stores; no lock across an await or a frame; the stress tests and the stress run |
| Antivirus meets a new unsigned binary on every build | one executable, few modules to load; the loader line of the profile shows it |
| Scope: 400k lines cannot be moved as they are | they are not moved: sections 4 and 8 drop or replace more than half before any porting |

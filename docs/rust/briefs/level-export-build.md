# Brief: the scene's export and the level build (`gz-export-level`, `gz-levelbuild`)

What leaves a scene: the module's level layer, and the full level build. Headless. Read
`docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then `CLAUDE.md`
sections "Mod philosophy" (level deltas), "The mode's world" (Player Start, way paths, a
stalker's Order - what the bake writes), "What a build ships"; `docs/RUST_MIGRATION.md`
section 9 (the contract with the game: these files are in it); the rustdoc at the top of
`gz-scene` (snapshots, `render_items`, the classes), `gz-build` (its slot for the scene's bake,
`BuildCheck`), `gz-mod`, `gz-project`, `gz-game`, `gz-fmt-level` (xcform, overlay visuals, AI
map, `build.prj`), `gz-fmt-ogf` (export), `gz-fmt-object`, `gz-fmt-spawn`, `gz-entities`,
`gz-editor`.

You own: `crates/gz-export-level/**`, `crates/gz-levelbuild/**` (new; `src/lib.rs` before
`Cargo.toml`) and the registration lines in `crates/grizzly/src/main.rs` + `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/LevelEditor/Editor/EditorModScene.cpp` (1.75k lines): the module layer - the
  collision overlay, `overlay_visuals.ltx`, the additive spawn layer (`.xspawn`: `[obj:<name>]`
  ops, `op = start`, `op = remove`, `[path:<name>]`, custom data files, a stalker's
  `[xms_order]`), level cuts, the OGF probe, every warning it gives ("left out", "scaled",
  "invisible", a path name the game already uses, a hand-written `[xms_order]` replaced ...).
- `Source/Editors/LevelEditor/Editor/Builder/*.cpp` (2.6k): the full level build - the build
  project for the compiler, LODs, OGF, LTX, the game side files.
- `Source/Editors/LevelEditor/AI/*.cpp` (6k): the game graph builder, the level and game spawn
  constructors, patrol path storage (the containers are already `gz-fmt-spawn`).
- `Source/Editors/LevelEditor/Editor/UI_LevelTools.cpp`: `RunGame`, `RunXrLC`, `RunXrDO` (how
  the compilers and the game are started); `Scene/scene_spawn.cpp`, `Scene/CFromBuilder.cpp`,
  `Tools/AIMap/ESceneAIMapTools_Export.cpp`.
- Handlers (frozen wire): `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`,
  `mod_export_ogf_probe` (`rg -n '"mod_export_spawn"' Source/Editors`), and the Build commands
  of the Level toolbar as the command table names them (Build Collision, Build AI Map, Build
  Game Graph, Build All, Play, Play in Game, Play Clean, Stop, Reload Configs). Schemas:
  `crates/gz-mcp/tools.json`.
- The game's reading of these files (READ-ONLY): `D:\Games\Dead Air\DeadAir-x64\docs\dead-air\
  MODDING.md`, `XMS_ARCHITECTURE.md`.

## Design

**`gz-export-level` - the module's level layer.**
- Pure functions `(scene snapshot, project, game data) -> files + report`: the text and bytes
  to write and the list of warnings, with the C++ wording. The files are the ones of
  `EditorModScene.cpp`, **byte-equal** where the algorithm is deterministic (`\r\n` line ends,
  key order, number formatting as the C++ prints them - port its `sprintf` formats exactly).
- The bake reads a SNAPSHOT of the scene: the author keeps editing while it runs. Per-level
  artifacts are independent: compute them in parallel on the CPU pool; write on the IO pool.
- Fill `gz-build`'s bake slot, so a build bakes the open scene as the C++ build does, and
  register the `BuildCheck`s that belong to the scene (`E201`: more than one Player Start -
  name every one found, scene included).
- Two modules on one map must keep working: nothing here writes or reads back another
  module's files, and nothing of the game is ever written.

**`gz-levelbuild` - the full level build and the runs.**
- The build project for the compiler (`build.prj` and its side files) from a scene snapshot;
  the collision form and the AI map export; the game graph and the spawn file through
  `gz-fmt-spawn` and `gz-entities`. Each step is a job (`"level_build"`, one at a time) with
  stages and Stop; steps that do not depend on each other run in parallel.
- The compilers stay separate processes for now (`XrLC.exe`, `xrDO_Light.exe` under
  `Bin\x64\Release`): start them with `tokio::process`, follow their output into the job's
  stage, Stop kills the process tree. A missing compiler is a sentence for the author, not a
  crash.
- **Play starts the real game** on the baked scene as a job (docs/RUST_MIGRATION.md D1): Play,
  Play in Game, Play Clean and Stop map onto building what is needed and launching the linked
  game with the C++ `RunGame` command line; Stop ends that process. There is no game inside the
  editor's process.
- What "needs rebuild" (collision, AI map, game graph) is state of the scene's derived data:
  expose it as a small query the status bar's Rebuild chip will read.

## Done means

- `bash tools/rust/cg.sh check -p gz-export-level -p gz-levelbuild --tests` clean, zero
  warnings. Tests written (not run): the `.xspawn` text for a hand-made scene with a prop, a
  Player Start, a way path and a stalker with an Order, against text derived from the C++
  formats; xcform for a box; the warnings; the game graph for two levels.
- APIs at the top of each `lib.rs`; commands registered.
- Never run a compiler or the game while working; never write into the linked game.

## Report

Under 400 words: commands, every place the output is not certainly byte-equal to the C++ (be
exact - these files reach the game), what is left. Then add rows for both crates to
`docs/rust/STATE.md`.

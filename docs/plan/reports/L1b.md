# Report L1b - building a level: one pipeline, no hand-made steps

## 1. Built

**The pipeline (`crates/gz-levelbuild`)**
- `steps.rs` - `Step` (`game`, `spawn_layer`, `ai_map`, `game_graph`, `spawn`, `geometry`, `details`, `light`,
  `detail_light`), `Step::ORDER` (each after what it needs, the quick steps first, the compiler last), `needs`,
  `classes` (what scene classes a step reads), `files`, `dependents`, `with_needs`; `Quality` (`draft` / `standard` /
  `final`, applied to the level options of the build's copy only); `Options` (quality, compiler flags, the linked
  game).
- `fingerprint.rs` - a stable 64-bit hasher (`Stable`: the same numbers on every run and machine); per object two
  hashes (transform; class data, flags, marks, motion - a group's session ids left out, a spawn entity by the spawn
  and update packets the game reads, so what a reader noted about a packet's origin is no change), memoised per
  object version (`HashCache`, keyed on the `Arc`, so an edit rehashes only what it touched); per step a
  fingerprint over the objects of its classes, its layers and settings (the AI map hashed as the file holds it -
  packed over its box -, details slots without their lighting, wallmarks, build params, light controls, quality,
  flags, the linked game) and the fingerprints of what it needs; `changes` - "12 objects moved, 1 added".
- `state.rs` - `build.state` in `gamedata\levels\<level>\`: Lua data (`gz-luadata` writer and reader), every step's
  fingerprint, time, seconds, quality, flags and failure; the objects' hashes as the last build saw them (one long
  string per class, so a 14 k-object scene stays far under the reader's limits); which AI map the build generated.
  A newer or broken file reads as no record.
- `service.rs` - `LevelBuild`: the record (read once per level on the IO pool, set by every build), the status
  `Query` keyed on the scene store's revision, the record's revision and the linked game (`status_now` never waits:
  the status bar; `status` waits: the command), `StepState` (`fresh`, `stale`, `never`, `failed`, `not_needed`) with
  why ("1 object moved.", "Geometry is not fresh.", "Built at Draft quality.", "Built with the compiler's -noise.",
  the failure's sentence), the problems of the last builds (`LevelProblem {step, code, severity, text, object?,
  position?}`), the graphs the last build made (by fingerprint).
- `pipeline.rs` - the build as ONE job (`level_build`): plans the stale steps (or the asked ones with what they need,
  or `force`), runs them in order, records each in `build.state` as it ends (Stop or a failure keeps the finished
  steps fresh); a failed step holds back only the steps that need it - the others still run and the job ends failed
  naming what failed, what was built and what was left for later; stage text "Building the geometry (6 of 7)" and a
  fraction per step, the compiler's own stages and progress inside `light`. The steps:
  - `game`: the scene checked (game tests), `level.ltx` and the game files; stale `meshes\brkbl#N.ogf` removed;
  - `spawn_layer`: the module's spawn layer of the whole scene (`gz_export_level::bake_layer`);
  - `ai_map`: `level.ai` (it is now written into the level folder - Play stages it from there). A scene with no AI map,
    or one whose map the build generated and whose ground/bounds/parameters moved since, gets the map generated over
    the ground first and put into the scene as one undo step ("Generate AI Map"); a map the author made by hand is kept;
  - `game_graph`: `level.graph` (the level's game graph with its cross table; read back after a restart); a level
    without a graph point says so;
  - `spawn`: `gamedata\spawns\editor.spawn`, joined with the linked game's graph;
  - `geometry`: the visibility cells completed (below), the scene checked (build tests) on the completed copy,
    `build.prj` at the asked quality, the LOD atlas, HOM, SOM, `level_stat`, `level.wallmarks`; files of an earlier
    build this one does not write are removed (no folder wipe);
  - `details`: `build.details` and its texture from the painted slots; `light` / `detail_light`: the compilers in the
    editor's process. A refused face becomes a problem at its place and the viewport's error drawing.
  - A step that has nothing to make (`details`, `detail_light` without detail slots) is `not_needed`, and what it made
    before is removed so the game reads none of it.
- `cmd.rs` - the `level` area, typed cards:
  - `level.build` (`steps`, `only`, `force`, `quality`, the compiler's switches `nosun`, `noise`, `nosmg`,
    `no_optimize`, `no_invalid_faces`, `linear`, `wait`);
  - `level.status` (every step's state and why, `stale`, `fresh`, problem count, what runs; a step whose files are gone
    from the level folder is not fresh);
  - `level.problems` (`clear`, `step`, `limit`, `offset`; clear also clears the viewport's error drawing);
  - `level.validate` (the scene checked as a build checks it, the completed cells included; a job, `wait`);
  - `level.play` (`action: start | stop | status | root | reload_configs`, `clean`, `build`, `root`, `params`,
    `verify_restrictors`, `quality`): start builds what is stale - at the quality and flags the level was last
    built with -, checks the root and the engine first, stages and runs the installed game in place exactly as
    before;
  - `level.export` (`kind: spawn | collision | visuals | cut | obj`, `objects` - left out: the selection -, `all`,
    `level`, `section`, `mode`, `sector`, `overlap`, `grow`, `path`);
  - `level.compile` (`folder`, `details`, `only_details`, the compiler's switches as named parameters, `threads`,
    `wait`) - works without a project, so `Grizzly-Cmd.exe run level.compile folder=...` replaces `-compile`.
  - A `level.*` command that needs the scene loads the project's scene first (`gz_scene::need_scene`); a build in
    the way answers `busy` naming the job (`job.stop job=N`).
- `play.rs` - Play's gamedata is a mirror (`mirror_tree`): a file the root holds at the game's size and write time
  stays, another is a hard link to the game's file where the volume allows, a copy otherwise; what the game no longer
  has goes (only after the marker and project checks, as before); the staged level and spawn are linked from the
  project's level folder the same way. `PlayRoot` answers `linked` / `copied`.
- `lib.rs`, `compile.rs` - documentation on their own terms.

**`crates/gz-export-level`** - no commands of its own any more: `export_spawn`, `export_collision`, `export_cut`,
`export_visuals` take a `Layer` (level, `objects` named or the selection, `all`, ...) for `level.export`;
`bake_layer` is the spawn layer of a whole snapshot (the `spawn_layer` step); `install` (the module build's bake
slot, `E201`) unchanged.

**`crates/gz-scene/src/bulk`**
- `ground.rs` (new) - the ground: every static scene object the collision build takes, narrowed by an explicit list
  (`effective`: a list that names every static object is none) and a volume; `SHAPE_AI_MAP_BOUNDS` (a shape usage,
  2) and `ai_bounds`.
- `aimap.rs` - `seeds` (the spawn points inside the box, else the ground under its middle, else the first ground a
  16 x 16 grid finds) and `generate_over` (a new map over a box from seeds, then the flood).
- `details.rs` - `grid_over` (the 2 m slot grid over the ground, heights from the faces under each slot), `Stroke`,
  `paint` (a mesh takes its part in a slot, an empty part, or the thinnest one's; the corners inside the brush get the
  density times the mesh's density factor; erase empties corners, a part with nothing names no mesh; a slot without
  ground is left alone), `coverage`, `remove_mesh` (the parts after it move down instead of every slot being
  cleared); `initialize` is the colour-index import over the ground.
- `wallmark.rs` - marks lie on the ground (the scene-wide Snap Objects list is no longer read).
- `cmd.rs` - typed `scene.ai_map` (`info | set | generate | clear | paint | link | select | remove | smooth | reset |
  move | rotate`; the generation parameters live here, `objects` sets the ground list, `wait`), `scene.details`
  (`info | list | add | remove | set | paint | erase | fill | clear | import_texture`, `wait`), `scene.wallmarks`
  (`info | set | add | move | select | remove`); `generate_map` (pure, what the build's AI map step calls);
  `remove_selected` / `select_all` follow the scene mode (`SceneMode` of stream L1a). The viewport's gesture
  functions are kept.

**`crates/gz-scene/src/objtools`**
- `zones.rs` (new) - `complete` (what the build does: nothing for a scene without sectors; the default cell for every
  static mesh no sector holds; portals computed between the cells no portal joins, the author's pairs kept) and
  `scene.zones` (`list | create | assign | compute | validate | remove | invert | mode`).
- `sector.rs` - `unused_meshes` from one set of held meshes (was a search of every sector per mesh).
- `cmd.rs` (L1a's file, my two tools taken out) - `sector_tool`, `portal_tool` and their helpers removed,
  `zones::register` called; `mod.rs` - `pub mod zones`.

**`crates/grizzly`** - `-compile` / `-compile_details` and `compile.rs` removed (`cli.rs`, `main.rs`), the
`gz-lc`/`gz-lc-light`/`rayon` dependencies with them; `cmd_twin.rs`, `launch.rs` texts.

**Call sites** (each the smallest edit): `gz-ui-level` - `toolbar.rs` (Build Level, Play, Play Clean, Stop, Reload
Configs over `level.build` / `level.play` with their arguments; the four icon-only build buttons gone),
`status.rs` (the chip reads `LevelBuild::status_now`, no polling; [Build] per stale step = `level.build only=`),
`menus.rs` (Build: Build Level, Build Step > each step with its state, Rebuild All, Clear Problems; Make >,
Compile >, Import/Export Error List gone), `level_cmds.rs` (the Make rows, the error-list rows, the snap-list rows
and the make question gone; Validate over `level.validate`, Export as OBJ over `level.export kind=obj`, Clear
Debug Draw over `level.problems clear=true`), `tools_bulk.rs` (the forms over `scene.ai_map` / `scene.details` /
`scene.wallmarks`, the ground line instead of "Use Selected Objects", the slot selection rows gone),
`tools_objects.rs` (over `scene.zones`; Distribute Objects and Remove Duplicates gone), `choose.rs`;
`gz-ui-viewport` - `bulktools.rs` (no snap-list precondition), docs; `gz-ui-mod` - `bake.rs` (the four bakes are
actions over `level.export kind=`), `lib.rs`; `gz-mcp` - `lib.rs` (`level_build action=status` no longer offered as a
follow-up: `job.wait` is); `gz-win` - `console.rs` text; `gz-lc` - `lib.rs` entry-point docs on their own terms.

**QA** - `tools/qa/test_level_commands.py` rewritten (phases A B R S C E Y K L Z; L is the painted layers and the
zones), `tools/qa/test_level_compile.py` (new; records `tools/qa/golden/compile/mp_test.json` and
`mp_close_combat.json`), `run_all.py` (the new suite), `test_play_game.py` (every name it calls is a command of 0.7 -
`level.*` and `scene.*`, the world keys of the new format - and the AI map comes from the build), `test_scene_tools.py`
(D and W on the new commands; C and Y moved into `test_level_commands`), `test_world.py` (one call),
`measure_compile.py` (the names, the commandlet).

**Docs** - `docs/assets/SCENE.md` is generated by stream L1a's `format/doc.rs` from its tables (a test fails on a
hand edit), so the two sections my code needs are text of my modules for it to append:
`gz_scene::objtools::zones::REFERENCE` (what the build does with `zones.lua`) and `gz_scene::bulk::ground::REFERENCE`
(the ground of the painted layers, the AI Map Bounds usage).

**`crates/gz-scene/src/cmd_edit.rs`** (L1a's file) - `compiler_errors` removed (its registration and function):
`level.problems` replaces it.

**Words** - every file I touched says what it does on its own terms: the C++ function names, `.cpp` files and
"as the C++ did" of `gz-levelbuild` (`validate.rs`, `play.rs`), `gz-export-level` (`cmd.rs`, `spawn.rs`, including
two log lines that said "SDK" for the editor's own data), `gz-scene` (`bulk/aimap.rs`, `details.rs`, `wallmark.rs`,
`objtools/sector.rs`, `objtools/mod.rs`), `gz-ui-viewport` (`lib.rs`, `objtools.rs`, `bulktools.rs`),
`gz-ui-level/src/level_cmds.rs` and `test_play_game.py`'s comments are rewritten. The files of my crates I did not
touch keep theirs (`breakable.rs`, `prj.rs`, `files.rs`, `graph.rs`, ...).

## 2. Decisions

- **No `collision` step.** The editor's in-memory collision form fed nothing (the AI map is generated over its own
  model, the game loads the compiler's `level.cform`). The game's collision is the `light` step's output; the
  module's is `level.export kind=collision`. "Geometry and light" is two steps: `geometry` (the editor's build
  project) and `light` (the compiler on it), so a change of the compiler's switches or a Stop inside the compiler
  keeps the build project fresh.
- **The game files are a step of their own (`game`)**: a moved spawn point makes `game`, `spawn` and the spawn layer
  stale, never the compile.
- **The overlays and the cut are exports, not steps.** They take the objects the author names (the cut accumulates
  by design); a stale-driven step would export every static object of a base level as an overlay. The spawn layer,
  which covers the whole scene and which the module build bakes anyway, is a step.
- **Quality**: `standard` = the level's own settings, `draft` / `final` the presets; part of the `geometry` and `light`
  fingerprints. The status answers what a plain `level.build` would do, so after a draft build it says "Built at
  Draft quality."; Play builds at the quality and flags the level was last built with, so a draft level plays at once.
- **The AI map**: the build regenerates only a map it generated itself (`build.state` `ai_map` records the inputs and
  the nodes it gave); a map edited by hand or imported from an SDK scene is kept and exported as it is. The nodes are
  hashed as the file holds them (packed over the box), so a saved and reopened map is fresh.
- **Fingerprints cover the scene and the request**, not the library objects' files (an edited `.object` needs
  `force=true`) - noted under Requests.
- **Seeds**: the generation starts from the spawn points (where the player and NPCs stand), so no hand-placed node is
  needed; a level whose spawn points stand on nothing walkable gets the ground under the box's middle.
- **`level.play`**: `in_game` would have been a flag that changes nothing (both rows ran the same game); it is
  `build=false` (Play Without Building) instead. `reload_configs` lives here as `action=reload_configs` (the brief's
  first option).
- **`level.play action=root`** keeps the old `play_root` (the QA proofs of Play's roots use it).
- **Details**: the slot selection and Reinitialize Selected go with the base-texture workflow; the colour-index picture
  is an import (`import_texture`, with `colors` naming what each colour scatters). An export of a colour picture is not
  in the brief's action list and was not built. "Random rotation" is not a switch: the game turns every detail instance
  at random.
- **Wallmarks** lie on any static object a ray meets; the scene-wide Snap Objects list of an SDK scene is not carried
  (it only ever narrowed wallmarks and the viewport's object snap). The AI map's and the details' own lists are kept
  as their explicit ground.
- **Zones**: the build's completion is in the build's copy only; `scene.zones action=compute` writes the same into the
  scene. `scene.zones` has `invert` and `mode` beyond the brief's six actions (a portal's orientation, and the Zones
  mode's click), so the mode has every action it needs.
- **Problems** carry the object a line names (the first quoted name that is a scene object) or a position (refused
  faces); validation lines become problems by their mark (`*`/`ERROR` error, `!` warning).
- **A failed step does not stop the build**: a spawn that has no player must not cost the author the compile. The
  steps that need the failed one are left for later (stale), the rest run; the answer names all three.
- **Error codes** are the contract's: no scene is `not_found`, a scene that does not open `failed`, a Stop `failed`
  with `cancelled: true` (the job's own state stays `cancelled`).

## 3. Contract changes

- Commands added: `level.build`, `level.status`, `level.problems`, `level.validate`, `level.play`, `level.export`,
  `level.compile`, `scene.zones`, `scene.ai_map`, `scene.details`, `scene.wallmarks`.
- Commands removed: `build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`, `play_game`, `play_clean`,
  `play_stop`, `reload_configs`, `make_all`, `make_game`, `make_details`, `make_hom`, `make_som`, `make_aimap`,
  `compile_geometry`, `compile_details`, `level_build`, `level_compile`, `level_rebuild_status`, `scene_validate`,
  `scene_export_obj`, `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`,
  `sector_tool`, `portal_tool`, `aimap_tool`, `details_tool`, `wallmark_tool`; the `-compile` / `-compile_details`
  flags.
- Files: `gamedata\levels\<level>\build.state` (new), `level.ai` and `level.graph` written by the build into the level
  folder (`level.ai.temp` gone).
- Documents: the two sections of `docs/assets/SCENE.md` as `REFERENCE` text (zones, the painted layers' ground),
  for stream L1a's generator. `docs/UI_DESIGN.md` untouched.

## 4. Requests

- **L1a**:
  - **a `.scene` round trip empties kept spawn packets** (found by `test_level_commands` R): open
    `rawdata/levels/mp_test.level`, `scene.save`, open the `.scene` - the five multiplayer items whose class the
    linked game does not know (`cta_wpn_toz34`, `cta_medkit`, `cta_ammo_12x76_zhekan`, `_0000`, `_0001`, kept as
    bytes) come back with `props` "kept as the file has it: the packet ends 2 bytes early at offset 0" and an empty
    `raw`; the build's game step then refuses "Invalid spawn data: cta_wpn_toz34". My suite removes the multiplayer
    spawns in B (as Play's suite does), so it no longer meets them; the bug stands;
  - a spawn point placed with `scene.place` carries header `version: 0, script_version: 0`, one read back from a
    file `128 / 12` (`gz-entities` `create.rs:286-288`). The packet writer ignores both, so the game sees no
    difference; my fingerprint hashes the packets for that reason, but the model should not differ;
  - `test_scene_tools` D and W (mine) cannot run: the suite's `ground_y` helper (`scene.place kind=object
    on_surface=true` over Escape) does not answer within 120 s. The same commands are proven by
    `test_level_commands` L on mp_test;
  - append `objtools::zones::REFERENCE` and `bulk::ground::REFERENCE` to `format/doc.rs`'s `reference()` (one
    line each) and regenerate `docs/assets/SCENE.md`;
  - remove the Snap Objects list (`ToolSettings::snap`, `snap_use`, `snap_pick`, `snap.rs`, `snap_objects`, the `.level`
    reader's `[snap_objects]`): no tool of mine reads it;
  - the scene file: the shape's `bound` bool loses usage 2 on a save - make it `usage = "common" | "level_bound" |
    "ai_map_bounds"` (value 2, `bulk::ground::SHAPE_AI_MAP_BOUNDS`) so an AI Map Bounds volume is placed with
    `scene.place` and set with `scene.set`; keep the
    AI map's and details' ground lists (`SceneAiMap::snap_objects`, `SceneDetails::snap_objects`) in their sidecars,
    and the zones as `docs/assets/SCENE.md` says; the AI map sidecar must keep the packed nodes and box bit-exact
    (the status hashes them);
  - the Level Settings rows of the AI map parameters (`props.rs`): they are `scene.ai_map action=set` now.
- **L2** - what the level's face shows:
  - Build Level (split): click = `level.build`; menu: each step with its state and why (`LevelBuild::status_now`,
    `StepStatus.why`) building it with `level.build only=<step> force=true`, Rebuild All (`force=true`), the quality
    (`draft` / `standard` / `final`); Advanced: `level.export` kinds, `level.compile`.
  - The status chip and popover read `LevelBuild::status_now` (non-blocking); the states are `fresh`, `stale`,
    `never`, `failed`, `not_needed`.
  - Problems: `level.problems` rows (`text`, `object` - select and frame it -, `position` - frame it -, `step`,
    `severity`); Clear Problems = `clear=true`.
  - Play (split): Play = `level.play`, Play Clean = `clean=true`, Play Without Building = `build=false`; Stop =
    `level.play action=stop`; Reload Configs = `action=reload_configs`.
  - The modes: Paint > AI Map (`scene.ai_map`: generate inside the AI Map Bounds volume - a shape of usage 2 -, the
    parameters, paint/link/select/remove/smooth/reset/move/rotate; the gestures in `gz_scene::bulk::cmd`), Paint >
    Foliage (`scene.details`: the mesh list with density / scale / waving, paint / erase / fill with radius and
    density, import a colour picture), Paint > Decals (`scene.wallmarks`), Zones (`scene.zones`: the list, create,
    assign, compute, validate, remove, invert, mode; its first line: "The build makes the cells by itself").
  - The forms of `tools_bulk.rs` / `tools_objects.rs` are kept working on the new commands only; they are yours to
    replace.
  - **The render thread's GPU waits** (audit D; `gz-render` is not mine, so this is the description): the render
    thread's scene pass (`renderer.rs:393-397`) submits and then `device.poll(PollType::Wait)`s for that submission
    (10 s limit); `label_visibility` then maps the query buffer through `map_read` (`frame.rs:325-336`), a second
    blocking `PollType::Wait`. Fix: submit the pass and do not wait for it; the target is handed on with its
    submission index and presented when `device.poll(PollType::Poll)` says it is done; the visibility buffer becomes
    two buffers in turn, mapped with `map_async` and read at the start of the NEXT pass (labels one pass late,
    invisible to the eye), never with `Wait`. The render thread then never blocks on the GPU, and its polls stop
    contending with the UI thread's own submits. The offscreen picture (`renderer.rs:246-248`, the preview and
    `shot.*`) may keep its wait: it runs off the UI thread and its caller asked for the pixels.
  - `gz-app/src/cpp.rs` `known_rows` still lists `COMMAND_BUILD` / `COMMAND_MAKE_*` labels (no command behind them; the
    table leaves such rows out) - they go with the table.
- **Q**: `tools/mcp/AI_SETUP.md` (the `-compile` example, the `mod_export_*` rows, `level_build action=status`),
  `docs/rust/briefs/level-export-build.md`, CLAUDE.md's sentences on `level_build` / `level_compile` / `-compile` /
  `compile_details` / the Snap Objects list / "Play copies the gamedata" name what is gone.
- **Architect**: `crates/grizzly/src/cli.rs` and `main.rs` edited for the `-compile` removal (as the brief allowed),
  and `grizzly/Cargo.toml` lost three dependencies with it. The fingerprints do not see a library object's file
  change; a cheap stamp (size + write time of each referenced `.object`, gathered by the library when it loads one)
  would make `geometry` stale on it - not done: the library is L1a's.
- **M**: the module build's bake (`BuildChecks::set_bake`) and the `spawn_layer` step write the same file; nothing to
  do unless the bake should read `build.state` to skip a fresh layer.

## 5. Proof

All on the final tree (every program built by `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`; editors
only through the harness on copies of `D:\XFinedProjects\Test` in the temp folder, port 28962, closed over MCP).

- `bash tools/rust/cg.sh check|clippy -p gz-levelbuild -p gz-export-level -p gz-lc -p gz-mcp -p gz-ui-mod
  --all-targets` and `clippy -p gz-ui-level -p gz-ui-viewport -p grizzly`: no warning in my crates or my files (the
  remaining gz-scene warnings are in `format/*`, `edit.rs` and `objtools/cmd.rs:397`, stream L1a's). `rustfmt
  --check` on my crates and files: clean (three hunks in `gz-ui-level` are L1a's lines).
- `bash tools/rust/cg.sh test -p gz-levelbuild -p gz-export-level --lib`: 50 + 15 passed (steps order, state
  round trip and refusal of newer/broken records, fingerprints stable, the status: a moved object makes stale exactly
  the steps that read it, the record survives a write and a read, Stop keeps what ended, a draft build says so and a
  failed step is never fresh; Play's mirror links and keeps). `cg.sh test -p gz-scene --lib -- bulk:: objtools::zones`:
  20 passed (the ground narrowed by a list or a volume, painted details read back as the game's slots, the default
  cell and the computed portals, the AI map generation).
- `python tools/qa/run_all.py test_level_commands test_level_compile`: both PASS (12.9 s and 3.9 s).
  `test_level_commands` 63/63: A (all never built, problems empty, every old name `not_found`, reload_configs),
  B (draft build of every step in 1.3-1.6 s as one job; fresh but geometry/light "Built at Draft quality."; the same
  build again builds nothing; the level folder and `build.state`), R (saved, editor restarted: the status is what it
  was; a moved object makes geometry, light, game, ai_map, spawn layer stale with "1 object moved.", game_graph says
  "AI Map is not fresh."), S (Stop inside the compiler: cancelled, geometry fresh, light not), C (`level.compile`
  refuses a folder without build.prj; 21 phases, 9 files), E (OBJ, the spawn layer of one object named, a missing
  name named back), Y (15 refusals of Play roots, folders unchanged, no game started), K (the commandlet answers the
  light stale in 0.4 s with no editor), L (AI map 1800 nodes from the spawn points and one undo step; details paint 24
  slots, erase, fill 256, remove; a wallmark; zones list/validate; `only=detail_light`), Z (no stall).
- `test_level_compile`: `--record` wrote the two records (mp_test: 9 files, 3 visuals, 1 sector, 152 collision
  triangles; mp_close_combat: 12 files, 381 visuals, 92659 triangles); the compare run matches them byte for byte
  (draft quality makes no light maps, so their list is empty in both).
- `test_play_game` (not run, as the rules say): its scene preparation and build were run without the game from a
  scratch script (`prepare_scene` + `level.build` + `level.play action=status` on a copy): 9/9.
- Not proven: `test_scene_tools` D and W (the suite's helper does not answer, see Requests); Play itself (the game).

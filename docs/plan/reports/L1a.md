# Report L1a - the scene: one selection, its own files, plain names

## 1. Built

**One selection, no edit target (`gz-scene`)**
- `tools.target`, `SetTarget`, Pick Any Class and every command, key and field named for an edit target are gone.
  `ToolSettings` holds `mode: SceneMode` (view state, not saved) and `placing: ObjClass` (what a click of Place
  mode adds; nothing else reads it); `mode_kind()` is the kind a mode's clicks work with (a painting mode's layer,
  the cells in Zones, else `placing`); `SceneCmd::Arm(kind)` puts Place mode up with a kind armed.
- Selection, deletion, hiding, locking and the marquee take any kind; the gizmo offers what every selected object
  can do (`gz-ui-viewport` `gizmo::supports_of`: the intersection of the kinds' capabilities). A light and a glow
  delete together, a locked object stays (unit test). Eye and lock per kind and per object (`scene.set hidden= /
  locked=`, the Outliner's eye).
- Way points are sub-objects of their way: the Ways/Points switch is gone; Points mode is Place with Way armed and a
  way selected; `scene.way` edits the points and links.
- Groups: grouped in place (`scene.group`), saved as `.group` assets (Lua, `group = 1`, the same lists as the scene's
  folder files, relative to the pivot) and placed from the project or the library (`scene.place kind=group`).

**The scene's own files (`gz-scene/src/format/`, `sdk/`, `kind.rs`)**
- `<name>.scene` (header, level options, build settings, sun, camera, the kinds' eyes) and the folder `<name>\` with
  one Lua file per kind family (`objects.lua`, `lights.lua`, `spawns.lua`, `ways.lua`, `zones.lua`, ...) and the
  binary layer files `ai_map.bin`, `details.bin`, `wallmarks.bin` (`GZLAYER\0`, version, kind; the data as the
  editor holds it - the AI map's packed nodes and box as read).
- One table of keys per kind (`fields.rs`, `SCENE_FIELDS`, `spawn.rs` `FIELDS`) is the reader, the writer, the
  Details rows and the reference. A key at its default is not written; unknown keys and comments survive; a save is
  `apply_diff` of the documents (one moved object: one line of one file). A file changed outside while the scene is
  clean is one undo step; while it is dirty the outside change is rebased under the author's edits.
- Spawns: the named properties that differ from a new entity of the section (`gz-entities` `named.rs`), proven per
  save - the properties written, read back through Lua text and made into an entity must give the very packet, or
  the spawn keeps its packet (`raw`, hex). An entity of an X-Ray SDK text scene whose class the linked game does
  not know has no packet; it keeps the section's numbered values (`values`) and is decoded from them again when the
  scene opens.
- Numbers are the shortest text that reads back to the same `f32`; `-0`, NaN and the infinities are the words
  `"-0"`, `"nan"`, `"inf"`, `"-inf"`. Names are unique in any case, written in the case given (leaf + counter keeps
  case: `object_01`).
- The X-Ray SDK `.level` (text and binary, with its `.part` folder) is read only (`sdk/`); its writer is gone. An
  object of a version the reader does not know keeps its name and place with its class's defaults and a warning.
- `KindOps` for `scene` and `group` (`kind.rs`): `asset.check`, `asset.reference`, the diff rules.
  `docs/assets/SCENE.md` is generated from the tables (`format/doc.rs`), with L1b's two sections (the painted
  layers' ground, the zones), and a test fails when the file is not what the tables say.

**Plain names and properties (`props.rs`, `gz-entities` `display.rs`)**
- Every row has a stable key, a Title Case label, a unit, words instead of numbers for choices and a help line with
  its range; the entity's properties by `Prop::key()` / `Prop::display()`. A test fails on a key without a label.

**Commands (typed, `gz_editor::Spec`)**
- `scene.open`, `scene.new`, `scene.save` (`path` for Save As), `scene.convert` (SDK scenes of a folder to `.scene`),
  `scene.info`, `scene.objects`, `scene.get`, `scene.set`, `scene.choices`, `scene.place`, `scene.select`,
  `scene.delete`, `scene.duplicate`, `scene.copy`, `scene.paste`, `scene.group`, `scene.ungroup`, `scene.align`
  (`drop`, `surface`), `scene.query`, `scene.mode`, `scene.attach`, `scene.motion`, `scene.recover`, and `doc.undo` /
  `doc.redo` for `doc=level` through the shell's documents. Kept with names of what they do: `scene.way` (point and
  link editing), `scene.shape` (join, split, the level bound), `scene.fog_group`.
- `need_scene`: a `scene.*` command loads the project's scene first and waits for the load job only (a details or AI
  map job of the scene group no longer holds it); `no_scene` only when the project has none. Recent scenes are
  `ProjectState::recent_scenes`.

**Wave 2: what L1b found**
- *Spawn data lost on `mp_test`*: the five multiplayer items of classes the game does not know came from the SDK text
  scene as entities with numbered values and no packet; the writer wrote their empty packet (`raw = ""`). They now
  keep `values`, and the corpus test compares those values (and fails on an emptied one).
  `test_level_commands` B keeps the multiplayer spawns again and passes.
- *The hang*: `scene.place ... on_surface=true` answered once `need_scene` waited for the load only; what then did
  not answer in `test_scene_tools` D was the details paint: the slot grid asked every ground object of Escape for
  every slot (642 402 slots x 3 843 objects, each through the library's lock). `bulk/details.rs` now takes the
  ground once (library object, transforms, world box) and buckets it over 32 m cells: the first stroke over Escape
  takes about a second. The viewport's details (`gz-ui-viewport` `draw_details.rs`) lay their models on the same
  ground (they used the old snap list, empty now, so nothing showed), and a paint, fill or import turns the details'
  drawing on (Escape's imported layer had it off).
- *Requests*: `usage = "common" | "level_bound" | "ai_map_bounds"` on shapes (`scene.place kind=shape usage=`,
  `scene.set`); the Snap Objects list is gone from the model, the file, `scene.align`, the Tools panel, the
  viewport's tint and pick, and the wallmarks' readiness (the drop to ground moved to `landing.rs`); the reference
  sections appended; a new entity's header says version 128 and the current script version, as one read back does.

## 2. Decisions

- The SDK scene's scene-wide Snap Objects list is not imported: its users were the placing snap and the wallmarks,
  which now take the whole level; the AI map's and the details' own lists stay in their layers (L1b's ground rules).
  The import says so in a warning naming the count.
- `values` rather than `raw` for the unknown-class SDK entities: they never had a packet, and their values decode
  into one as soon as a game that knows the class is linked.
- `scene.way`, `scene.shape`, `scene.fog_group` stay commands: adding, moving and linking points of a way, joining and
  splitting shapes, editing the level bound and grouping fog volumes change several objects or sub-objects at once,
  which `scene.set` (one object's keys) cannot say.
- The details' drawing is switched on by the strokes that lay details, not by a separate step the author would have
  to find.
- The viewport's detail cache is keyed on the scene's content serial (the ground may move) and narrows the ground to
  the cache window before each slot asks.

## 3. Contract changes

- Files: `.scene` + folder + layer files (new, `docs/assets/SCENE.md`); `.group` assets (new). Shape `bound` became
  `usage`; spawn `values` (new); `snap_objects` gone.
- Commands: the `scene.*` set above; removed `open_scene`, `scene_new`, `save_scene`, `scene_info`, `scene_stats`,
  `scene_summary`, `scene_load_status`, `list_objects`, `scene_tree`, `object_info`, `property_inspector`,
  `selection`, `property_set`, `object_set_position/rotation/scale`, `rename_object(s)`, `spawn_set_visual`,
  `spawn_set_fixed_bones`, `spawn_order`, `mark_authored`, `mark_removed`, `property_choose`, `place_object`,
  `place_spawn`, `place_way`, `select_objects`, `deselect_all`, `delete_selected`, `scene_edit`, `group_tool`,
  `drop_objects`, `snap_objects`, `probe_vertical`, `scan_walls`, `spawn_attach`, `object_motion`, `recovery`,
  `undo`, `redo`, `way_tool`, `shape_tool`, `light_tool`, `fog_tool`, `object_tool`. `scene.align` has no `snap`.
- Documents: `docs/assets/SCENE.md` (generated); `docs/UI_DESIGN.md` (the `scene.choices` line; the Snap Objects
  section line removed); `docs/MOD_QUICKSTART.md`, `docs/NQ_ARCHITECTURE.md` (the new command names).
- Outside my files, as call sites of my API: `gz-ui-viewport` (cmd, gizmo, draw, draw_details, pane, gesture,
  bulktools, objtools, keys, service), `gz-ui-level` (details, motion_ui, menus, tools, tools_bulk, tools_objects,
  outliner, level_cmds, toolbar, recovery, doc, cmd, lib), `gz-ui-content` `actions.rs`, `gz-ui-world` `world.rs`;
  L1b's `bulk/details.rs` and `bulk/cmd.rs` (the two fixes above); the QA suites `test_scene_tools`,
  `test_properties`, `test_level_commands` (B), `test_world`, `test_stress`, `test_ui_tour`,
  `test_server_lifecycle`, `benchmark_scene_load`, `test_portable`, `measure_compile`, `bench_build`.

## 4. Requests

- **Architect**: `crates/grizzly/src/main.rs` (L1b's `-compile` removal) fails `check.sh`'s fmt at line 55; the
  release build in `Bin\x64\Release` is older than the sources, so `check.sh`'s command-line step stops - I did not
  run `Build.ps1` while other streams' suites may hold that folder's programs. An `asset.migrate` hook could run
  `scene.convert` for the SDK scenes of a project.
- **A2**: a way to note a scene as recent when it opens: `Project::edit_state` is `pub(crate)`, so
  `ProjectState::recent_scenes` gets only the scene open at the project's close.
- **S**: the content browser could show a `.level` as an imported SDK scene that `scene.open` converts.
- **L2a / L2b**: the mode switch and the Place panel on `scene.mode` / `ToolSettings::placing`; the Details rows'
  labels and units now come from `gz_scene::selection_rows`; the Tools panel lost its Snap Objects section;
  `scene_visibility`, `spawn_animation`, `particles_tool`, `group_thumbnail` and the viewport's commands still have
  no card; the viewport's `pick_any_class` setting (`gz-ui-viewport` `service.rs`, read in `objtools.rs`) means
  nothing now that a click takes any kind.
- **Q**: `CLAUDE.md`, `tools/mcp/AI_SETUP.md`, `docs/rust/STATE.md` / parity notes still name the old commands and
  the Snap Objects list.
- **K**: `test_server_lifecycle`'s nested-field probe now uses `ui.palette` with `meta` (the old probe command is
  gone); `EditorSettings::recent_scenes` / `add_recent_scene` (`gz-app` `settings.rs`) have no caller left.

## 5. Proof

All on the final tree, built with `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`; editors only through
the harness on copies of `D:\XFinedProjects\Test` in the temp folder, port 28961, closed over MCP.

- Corpus (`gz-scene` `tests/corpus.rs`, the linked game's configs): the 35 base scenes of `rawdata\levels` import,
  are written as `.scene`, read back equal (objects by name: kind, transform, flags, marks, motion, data; every spawn
  packet byte for byte and the kept values of packetless entities; groups; level options; the layers bit for bit;
  the kinds' settings) and written again unchanged: 35 checked in 3.2 s, 0 failed.
- Unit tests: `gz-scene` 96 (save as difference, comments and unknown keys kept, a removed kind's file goes, a newer
  scene refused, the reference document, every key labelled and explained, a light and a glow deleted together,
  names keep case, set from text, ...), `gz-entities` 55 + 2 corpus.
- QA: `test_scene_tools` 76/76 in 12.9 s (F the SDK Escape opens, saves as `.scene` and opens again the same;
  O `objects.lua` edited from outside with a comment is one undo step; K a move and Save keep the comment and change
  that object's line alone; X a light and a glow deleted together in one step, a locked object stays; R A L M G P V
  the gestures; D the details painted, seen, erased and imported; W the wallmark; S current screenshots);
  `test_level_commands` 62/62 with the multiplayer spawns kept (B, R round trip of `mp_test` through `.scene`);
  `test_properties` 46/46.
- Speed (dev profile, Jupiter, 10 337 objects): before - read 25-26 ms, read and parse 292-305 ms, serialise
  126-140 ms (31 files); after - read 20-26 ms, read and parse 267-289 ms, a save of one moved object 33-35 ms (one
  file), a whole write 433-468 ms (15 files, the per-spawn proof included).
- `bash tools/rust/check.sh`: clippy clean, unsafe only in gz-win, encoding clean, notices current, test files clean;
  fmt and the command-line step as under Requests.

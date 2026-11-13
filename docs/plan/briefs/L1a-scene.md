# Brief L1a - the scene: one selection, its own file format, plain names

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5), **`docs/ASSETS.md` sections 2, 3, 7, 9**,
`docs/COMMANDS.md`, the reports of wave 1 (`docs/plan/reports/A1.md`: `LuaDoc`, `gz-asset`, the watcher;
`A2.md`: the project file's `scene` and `library`, `gz_project::library::bring_in`), the survey
`Intermediate/plan2/assets.md` section 3 (the scene on disk today), and the audit `Intermediate/plan2/ux_level.md`
sections **B1, B9, B10, B11, B14** (what is the X-Ray SDK's in the model under the UI, with `file:line`).

You own: `crates/gz-scene` except `bulk/` and `objtools/sector.rs`, `objtools/portal.rs` (stream L1b),
`crates/gz-entities`. You change `gz-ui-level` and `gz-ui-viewport` only where your model change breaks them -
keep them working with the smallest edit; their redesign is stream L2's, after you. Stream L1b works beside you in
`gz-levelbuild`, `gz-export-level`, `gz-scene/src/bulk` and the two zone tools: it reads the scene's snapshot
through the types you own - agree on nothing in private; when you change a snapshot type it uses, fix its call
sites and say so in your report.

## 1. One selection, no edit target

Today one "active tool class" stands between the author and every operation (audit B1): Delete, Select All,
Invert, Hide, Copy / Paste and the marquee act on the active class only; a click on another kind of object
switches the tool; the gizmo's abilities come from the tool, not from what is selected. Remove the concept from
selection and editing:

- The selection is one set of objects of any class. `SceneCmd::RemoveSelection`, `SelectClass`, `Show`, the
  clipboard, duplicate, the marquee and ray picks take no class and work on every visible, unlocked object.
  A filter is explicit and optional (`classes: [...]` on the commands that select).
- What a gizmo may do is the intersection of what the SELECTED objects can do (`class.rs` capabilities), never
  what a tool allows.
- `tools.target`, `SetTarget`, "Pick Any Class", the class argument defaults (`cmd_edit.rs:31-37`) and the
  `CF_XRAY_CLASS_LIST` clipboard record go. What remains of "a tool" is the mode of the few real modes (painting
  the AI map, details, wallmarks; editing zones): a mode owns the viewport's clicks while it is on, and is not a
  selection filter. Model it as `SceneMode` (`Select` by default) in the scene's view state; stream L1b's bulk
  tools and stream L2's UI read it.
- **Eye and lock** per class and per object replace the per-class Enable / Read Only switches of Level Settings
  and the unloadable "level parts" (audit B9): `hidden` and `locked` are properties of an object (saved in the
  scene) and of a class (view state, saved with the scene's bookmarks); a hidden or locked thing is not picked.
  `level_parts` and its commands go.
- Ways (audit B14): a way's points are sub-objects - selectable, movable with the gizmo, listed with their flags
  in the way's properties; the Ways / Points mode switch goes.
- Groups: Group (Ctrl+G in the UI later) groups the selection in place; a group saved for reuse is a `.group`
  asset in the project (section 2).

## 2. The scene's own file format

`docs/ASSETS.md` section 7 is the contract. Today the scene is the X-Ray SDK's `.level` + `.part` files, written
for a C++ editor that no longer exists (survey 3).

- **Layout**: `<name>.scene` (Lua data, `scene = 1`: level options, build settings, bookmarks, the class eyes)
  and the folder `<name>\` beside it: one Lua data file per kind of object, present only when it holds objects
  (`objects.lua`, `lights.lua`, `glows.lua`, `sounds.lua`, `particles.lua`, `shapes.lua`, `fog.lua`,
  `spawns.lua`, `ways.lua`, `zones.lua`, `groups.lua`) and the painted layers as binary sidecars the scene names
  (`ai_map.bin`, `details.bin`, `wallmarks.bin` - the in-memory layers as stream L1b's code holds them; give them
  a small header with a magic and a version, and keep their reading and writing in one module L1b can extend).
- **Objects**: one table per object, on its own line or block, `name` first; positions in metres, `rot` in
  degrees, `scale` left out when 1, booleans named and only when true, colours as `{ r, g, b }` 0..1 with an
  intensity where the engine has one; every key documented in `docs/assets/SCENE.md` (write it: a table per
  kind - key, type, default, meaning - generated from the same descriptors the reader uses, so it cannot drift).
- **Spawn points**: `section`, the transform and `props`: the entity's NAMED properties from `gz-entities`'
  property model (`props.rs`, `packet.rs`): `story_id`, `custom_data = [[...]]`, `visual`, ... A spawn packet
  read from an old scene decodes to named properties and encodes back to the SAME BYTES: prove it on every spawn
  of every base scene of the editor's library (a corpus test over `rawdata\levels`; it is local data, so the test
  skips with a message when the folder is absent). A property the model cannot name stays under
  `raw = "<hex>"` beside the named ones and is written back as it was; `asset.check` lists such spawns as notes.
  A placed stalker's Order (`xms_order*`) is `order = { kind = "guard", ... }`.
- **Reading and writing** through `gz-luadata`'s `LuaDoc` and `gz-asset` (register the kinds `scene` and
  `group`: versions, migrations, check, reference): saving applies the difference (`apply_diff`, lists keyed by
  `name`), so an object the author did not touch keeps its bytes, comments and unknown keys survive, and a scene
  that was not changed is not rewritten. Speed on the largest base scene (Jupiter: ~10 k spawns, ~4 k objects):
  load and save no slower than today's; measure both, before and after, and put the numbers in your report.
- **Following the files** (ASSETS 4): the scene subscribes to the project's watcher. An outside change to a
  scene file of a clean scene is applied as ONE undo step (`Reload from disk`); with unsaved edits the documents
  rebase per file, conflicts in the scene's Problems. A scene file that does not parse leaves the scene as it is
  and says where.
- **The SDK format is input only**: `.level` + `.part` are read (the reader stays, in a module named for what it
  is: the X-Ray SDK scene importer) and never written. Opening one converts in memory; Save writes `.scene` beside
  it and the Start scene of the project follows; `asset.migrate` converts every scene of a project (the old
  files stay until the author deletes them; the content browser shows a converted `.level` as "imported"). The
  writer (`write_scene`, `ltx.rs` writing, the `.~level` / `.~part` backups) and every test of its byte parity
  are deleted. Import Base Scene reads the library's SDK scenes and writes a `.scene`.
- **Recovery** writes the scene in the new format into `backup\recovery\<stamp>\` (no second binary format); the
  manifest's `source` is project-relative. The binary undo stream stays only if undo itself needs it - then it
  is "the undo snapshot", not a file format, and nothing calls it compatible with anything.
- Selection files ("Open Selection..." / "Save Selection As...") become `.group` assets - the same thing; the
  two menu commands go.

## 3. Plain names and properties

- **Names** (audit B11): an object's default name is the leaf of its reference plus a counter
  (`cabinet_village_01`), case kept; rename keeps the case (`apply.rs:683` lowercases today). Names stay unique
  per scene. "Rename Objects After References" goes. The level export's naming rules (`gz-export-level`
  `names.rs`) are L1b's - tell it what changed.
- **Properties** (audit B10): one display table for every row the Details panel and `scene.get` show: label in
  Title Case plain English, unit (`m`, `deg`, `s`, `%`), enum values as words (`$static` -> Static, `NOT A
  restrictor` -> None), help text where today a fake `Hint` row stands, Transform rows only on what has that
  transform, empty instead of `NO STORY ID`. Keys stay stable for files and commands; the label map lives beside
  the property definitions (`gz-scene/src/props.rs`, `gz-entities/src/props.rs`). Check the light colour that
  reads `#00000000` and fix what is wrong.
- **Level Settings**: what is a view toggle (Draw Name, Draw Cross, Draw Nodes ...) leaves the scene's
  properties for the viewport's Show state; what is a tool's parameter (AI map Can Up / Can Down, Brush Size)
  moves to its tool (L1b); the scene's own settings keep plain labels (audit B10 lists them).
- Angles are degrees in every command and every file. No radians anywhere outside the math.

## 4. Commands (`docs/COMMANDS.md`)

Typed cards, the names below; every old name they replace is deleted and its callers fixed (UI, QA suites,
documents). The audit `Intermediate/plan2/commands.md` 3a and 3c list today's families and parameter names.

| Command | Effect | Replaces |
|---|---|---|
| `scene.open`, `scene.new`, `scene.save` (`path` for Save As), `scene.info` | write / read | `open_scene`, `scene_new`, `save_scene`, `scene_info`, `scene_stats`, `scene_summary`, `scene_load_status` |
| `scene.objects` (`classes`, `query`, `limit`, `offset`, `tree`) | read | `list_objects`, `scene_tree` |
| `scene.get` (`objects`: properties with labels, units, choices) | read | `object_info`, `property_inspector`, `selection` |
| `scene.set` (`objects`, `set: { key: value }`; also `name`, `position`, `rotation`, `scale`, `hidden`, `locked`) | write | `property_set`, `object_set_position / rotation / scale`, `rename_object(s)`, `spawn_set_visual`, `spawn_set_fixed_bones`, `spawn_order`, `mark_authored`, `mark_removed` |
| `scene.choices` (`object`, `key`, `query`) | read | `property_choose` (the list a picker shows; opening the dialog is the UI's) |
| `scene.place` (`kind`, `ref` / `section`, `position` or `on_surface`, `rotation`, `points` for a way) | write | `place_object`, `place_spawn`, `place_way` |
| `scene.select` (`objects` / `classes` / `all` / `none` / `invert`, `add`) | view | `select_objects`, `deselect_all` |
| `scene.delete`, `scene.duplicate`, `scene.copy`, `scene.paste`, `scene.group`, `scene.ungroup` | write | `delete_selected`, `scene_edit`, `group_tool` |
| `scene.align` (`action: drop | snap | surface`) | write | `drop_objects`, `snap_objects` |
| `scene.query` (`action: ground | walls`, a point) | read | `probe_vertical`, `scan_walls` |
| `scene.mode` (`mode`) | view | the target switch |
| `scene.attach`, `scene.motion` | write | `spawn_attach`, `object_motion`, `spawn_animation` (the data side) |
| `scene.recover` | write | `recovery` |
| `doc.undo`, `doc.redo` for `doc=level` | write | `undo`, `redo` - register the scene with the shell's document commands the way stream K's report says |

A project can be open without its scene (`project.open scene=false`; a commandlet opens it so unless a command
of the areas `scene`, `level`, `viewport` or `shot` is among its commands). A `scene.*` command that needs a
scene when none is up loads the project's scene first and waits for it - `Grizzly-Cmd.exe run scene.objects`
works from a cold start - and says `no_scene` only when the project has none.

Recent scenes are the project's machine state (`ProjectState::recent_scenes`, project-relative - stream A2
made it); the absolute list in `grizzly_editor.ltx` (`gz-ui-level/src/level_cmds.rs`) goes. Nothing the scene
writes into the project names this machine (the SDK format's `[level_tag] owner`, a machine and a user name,
is not carried into `.scene`).

`way_tool`, `shape_tool`, `light_tool`, `fog_tool`, `object_tool`, `particles_tool` dissolve into `scene.set` /
`scene.place` / `scene.get` unless one does something those cannot - then it keeps one command with a name that
says what (`scene.way_link`), and your report says why.

## Proof

- Corpus tests (skipped with a message without the local library): every base scene imports, saves as `.scene`,
  reloads equal (object count, every property, spawn packets byte-exact, the painted layers bit-exact).
- Unit tests: `apply_diff` saves (move one object: one line changes), unknown keys and comments in every scene
  file survive a save; selection across classes (delete a light and a glow together - the audit's failing case);
  gizmo capability as an intersection; names keep case; the label table covers every key (a test fails on a key
  without a label).
- Tests that prove parity with the SDK writer or the C++ editor are deleted.
- QA: `tools/qa/test_scene_tools.py`, `test_level_commands.py`, `test_properties.py` rewritten for the new
  commands and extended: a scene edited from OUTSIDE (Python adds an object to `objects.lua` with a comment)
  appears in the open scene and undoes in one step; move an object in the editor, Save - the comment is still
  there and only that object's line changed; an SDK scene opens, saves as `.scene`, reopens the same.

## Done means

PLAN section 5 "L1" for the scene half, every sentence proven; `docs/assets/SCENE.md` written; no writer of the
SDK scene format in the tree; no command, key or model field named for an edit target; angles in degrees
everywhere.

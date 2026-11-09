# Brief: the scene (`gz-scene`), core

The level scene without a window: the model, the files, undo, selection, the object library,
picking, and the scene's commands. Read `docs/rust/CONVENTIONS.md` ("An area crate") and
`docs/rust/STATE.md` first; then `docs/RUST_MIGRATION.md` section 5.3; the rustdoc at the top of
`gz-store`, `gz-query`, `gz-editor`, `gz-project`, `gz-game`, `gz-chunk`, `gz-ltx`,
`gz-fmt-object`, `gz-fmt-level` (its `Bvh`), `gz-entities`; and `CLAUDE.md` sections "The mode's
world" (Player Start, way paths, a stalker's Order), "Dangerous operations".

You own: `crates/gz-scene/**` (new; `src/lib.rs` before `Cargo.toml`) and the registration
lines in `crates/grizzly`.

## The C++ that is the spec

Under `Source/Editors/LevelEditor/Editor/`:
- `Scene/`: `scene.h/.cpp`, `SceneIO.cpp` (the file format), `SceneSelection.cpp`,
  `ScenePick.cpp`, `SceneSnap.cpp`, `SceneUndo.cpp`, `SceneUtil.cpp`, `Scene_Tools.cpp`,
  `SceneSummaryInfo.*`.
- `Entry/`: `CustomObject.*` (what every object has: name, transform, flags, owner, the mod
  marks, animation) and one folder per class (`StaticObject`, `Light`, `Glow`, `Shape`,
  `Sound`, `Spawn`, `WayPoint`, `Sector`, `Portal`, `Group`, `Particles`).
- `Tools/`: one folder per class tool (`ESceneClassList.h` names the 16 classes). Read each
  tool's Load / Save / settings; its `*Controls*` files are viewport input (a later package).
- `Source/Editors/XrECore/Editor/Library.cpp`: the object library.
- `Recovery/EditorRecovery.*`: recovery snapshots.
- `Mcp/LevelMcpScene.cpp`, `Mcp/LevelMcpProps.cpp`: the scene's commands and the property
  commands - the wire is frozen by them. Schemas: `crates/gz-mcp/tools.json`.

## Architecture (decided - implement it this way)

**1. The scene is one store.** `Store<SceneState>` in the project-scope service.
```rust
pub struct SceneState {
    pub meta: SceneMeta,                                  // file, name, guid, level options
    pub objects: imbl::OrdMap<ObjId, Arc<Object>>,        // every placed object, all classes
    pub by_class: [imbl::OrdSet<ObjId>; CLASSES],         // iteration by class, in order
    pub names: imbl::HashMap<(ObjClass, SmolStr), ObjId>, // names are unique per class
    pub selection: imbl::OrdSet<ObjId>,
    pub tools: ToolSettings,                              // per-class visibility, lock, options
    pub bulk: BulkData,                                   // AI map, details, wallmarks: Arc'd blobs / tiles
    pub next_id: u32,
}
pub struct Object {
    pub id: ObjId, pub class: ObjClass, pub name: SmolStr,
    pub xform: Transform,          // position, rotation and scale as the C++ stores them
    pub flags: ObjFlags,           // visible, locked, ...
    pub owner: Option<ObjId>,      // the group that holds it
    pub marks: ModMarks,           // authored / removed (the module layer)
    pub data: ObjData,             // enum, one variant per class
}
```
An edit builds a new state that shares everything it did not touch; a command that fails
leaves the scene as it was. Bulk data is never copied whole: the AI map is tiles behind `Arc`,
an edit replaces one tile.

**2. Undo is the store's history** (`cx.undoable(label)` with the C++ labels; a drag is
`undoable_merged`). The C++ serialised the whole scene per step; here a step is a pointer.
`State::restored` keeps what is not content. `undo`, `redo` over MCP.

**3. Every change is a command** of the store: place, delete, rename, transform (batch),
select / deselect, show / hide / lock, group / ungroup, set a property by path, per-class
operations, mark authored / removed, replace the bulk data. MCP handlers and (later) the
viewport tools emit the same commands.

**4. Properties are typed data.** Each class describes its properties (`fn properties(&self,
cx) -> Vec<Prop>`: path, label, group, value, kind, limits, choose-list kind, read-only) and
applies one (`fn set(&mut self, path, value)`); a spawn point delegates to
`gz_entities::Entity`. A selection of several objects merges rows ("Multiple values").
`property_inspector`, `property_set`, `property_choose` answer from this, in the grid's order,
with the C++ labels - the Details panel will draw the same list.

**5. Loading is a job, saving never stops editing.**
- `open_scene`: a job (`"scene_load"`) reads the scene file and its parts on the IO pool,
  parses the parts in parallel on the CPU pool, prefetches the library objects it refers to in
  parallel, and sends ONE command that replaces the state (`cx.new_document()`).
  `scene_load_status` reads the job.
- `save_scene`: take the snapshot, serialise each part in parallel off the UI, write
  atomically, then mark the store saved. Recovery snapshots are the same thing on a timer into
  the project's `temp\`; the C++ rules for when a snapshot is taken (interval, delay after
  input, snapshots kept) carry over.
- **The file format is the C++ editor's** (`docs/RUST_MIGRATION.md` I4): both editors open the
  same scene until the cutover. Read every version the C++ reads; write what it writes.
  An object or chunk this crate does not understand is kept as raw bytes and written back -
  nothing is dropped.
- Register a `DirtySource`; set `gz-project`'s `last_scene` slot.

**6. The object library** (`library.rs`): `rawdata\objects` of the editor plus the project's
own, by reference name. `Arc<LibraryObject>` (the `gz_fmt_object::Object`, its bounds, its
picking `Bvh`) loaded on demand on the pools, cached, listed through the file tree; never a
directory scan per call. `list_assets`.

**7. Picking and snapping** read a snapshot and never lock: a top-level BVH over the objects'
world bounds is a `Query` keyed by the scene's revision; a ray takes its candidates from it and
tests each object's own `Bvh` in object space, candidates in parallel; ray batches (`scan_walls`,
`probe_vertical`, `drop_objects`, `snap_objects`) run in parallel on the CPU pool. Port the
C++ selection rules (what is pickable: visible, unlocked, class enabled) and the snap maths.

**8. Classes.** All 16 are modelled and round-trip through the file. Give full behaviour
(commands, properties, validation) to: scene objects, lights, glows, shapes, spawn points
(entities, `rpoint`, environment modifiers, **Player Start**, a stalker's **Order**), way paths,
groups, sound sources and environments, particles, sectors and portals, fog volumes. AI map,
details and wallmarks: load, save, hold, and the read-only queries the commands need; their
editing tools are a later package.

## Commands of this package

From `tools.json`: `open_scene`, `save_scene`, `scene_info`, `scene_stats`, `scene_tree`,
`scene_load_status`, `list_objects`, `object_info`, `select_objects`, `selection`,
`deselect_all`, `delete_selected`, `rename_object`, `object_set_position` / `_rotation` /
`_scale`, `place_object`, `place_spawn`, `place_way`, `spawn_order`, `spawn_set_visual`,
`spawn_set_fixed_bones`, `mark_authored`, `mark_removed`, `drop_objects`, `snap_objects`,
`scan_walls`, `probe_vertical`, `undo`, `redo`, `recovery`, `property_inspector`,
`property_set`, `property_choose`, `list_assets`. Commands that are about the picture
(`camera_*`, `focus_object`, `view_mode`, `scene_visibility`'s panel side, `outliner_*`,
`viewport_*`) belong to the viewport and panel packages: not yours.

## Done means

- `bash tools/rust/cg.sh check -p gz-scene --tests` clean, zero warnings. Tests written (not
  run): write -> read round trips of every class; undo restores; property merge; pick against
  a brute-force check on a seeded scene; a scene with an unknown chunk writes it back.
- The API at the top of `lib.rs` (the viewport, the panels and the export packages build on
  it: name the snapshot types and the command enum clearly); commands registered.

## Report

Under 400 words: the model and command API, commands registered, every deviation from the C++
file format or wire, what of the 16 classes is partial. Then add a row for `gz-scene` to
`docs/rust/STATE.md`.

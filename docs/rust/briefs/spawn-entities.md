# Brief: spawn files and server entities (`gz-fmt-spawn`, `gz-entities`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then
`crates/gz-chunk/src/{lib,reader,writer}.rs`; and `docs/RUST_MIGRATION.md` section 3 ("Three
facts") and the Phase 1 bullet about `gz-entities`.

You own: `crates/gz-fmt-spawn/**` (a placeholder) and `crates/gz-entities/**` (half written).

## Why this exists

The C++ editor loads the game's DLL to get its entity factory: for a spawn section it creates
a `CSE_*` object that (a) reads and writes its state as a network packet and (b) describes its
editable properties. The Rust editor loads no game DLL, so both have to come from a declarative
description of the classes. The packets are stored in scene files and in `all.spawn` and are
read by the real game: **they must be byte-identical** to what the classes write. A module's
own level data does not carry packets (it is text ops the game composes), so the risk sits in
scenes and in full level builds - and in never losing what we cannot decode.

## `gz-entities` - where it stands

An earlier pass wrote about 6k lines (`packet`, `settings`, `class`, `model`, `create`, `state`,
`update`, `script`) and was stopped before wiring them into `lib.rs` (a one-line placeholder).
Read that code first; its comments say what was concluded from the C++. Then wire it, review
it against the C++, and complete it.

Deliver:
1. `NetPacket` reader / writer primitives exactly as `NET_Packet` (`rg -n "class.*NET_Packet"
   Source`): integers, floats, vectors, `stringZ`, `r/w_angle8/16`, `r/w_float_q8/q16`,
   `r/w_dir` (compressed normal: `Source/XrCore/_compressed_normal.cpp`), `r/w_sdir`, matrices,
   `w_chunk_open8/16` + close, client-data blocks. A short packet is an `Err`.
2. The class table: CLSID -> class, and spawn SECTION -> class as the factory resolves it
   (`class` key of the section -> CLSID -> server class). Config access through the small
   `Settings` trait of this crate (no dependency on a config crate; a later crate implements it
   over the game's `system.ltx`). Constructors read defaults from the section: port every read.
   Spec: `Source/xrServerEntities/object_factory_register.cpp`, `clsid_game.h`,
   `Source/Editors/XrSE_Factory/xrSE_Factory.cpp`.
3. Every class of `Source/xrServerEntities` (`xrServer_Object_Base`, `xrServer_Objects`,
   `_ALife`, `_ALife_Items`, `_ALife_Monsters`, `_Alife_Smartcovers`, `PHNetState`, and the
   bases: visual, motion, shapes, PH skeleton), version-exact: construction, the `M_SPAWN`
   header of `CSE_Abstract::Spawn_Write` / `Spawn_Read`, `STATE_*` and `UPDATE_*` with every
   `m_wVersion` and flag branch, and a property description equivalent to `FillProps` (group
   path, label, type, value, limits, the choose-list kind: visual, animation, light animation,
   section list, token list with names and values, flags with bit names, story ids) with typed
   get / set by property path. Labels and paths as the C++ names them. Data-driven where it
   costs no exactness; hand-written where the logic is irregular.
4. **The safety net (mandatory):** an entity of an unknown class, of a newer version, or whose
   state fails to decode is kept as `Opaque { header, raw_state, raw_update }`: it writes back
   byte for byte and shows the abstract header as read-only properties. Nothing is ever dropped
   or rewritten lossy.
5. `Entity::from_spawn_packet(bytes, settings)`, `to_spawn_packet()`, `Entity::create(section,
   settings)`, `update_packet()`, `properties()`, `set(path, value)`, `class_name()`, and plain
   accessors for what the editor uses without properties: visual name, shapes, story id,
   custom data, game / level vertex ids, a level changer's destination (level, position,
   direction, graph point).
6. **The game decides.** The game's own classes are in
   `D:\Games\Dead Air\DeadAir-x64\src\xrServerEntities` (READ-ONLY). Diff the state / update
   functions of each class against this repository's `Source/xrServerEntities`; where they
   differ, implement the GAME's layout and list the difference in your report. The game also
   registers script-side classes (`se_*.script`, `class_registrator.script`) that may add
   state in Lua: look for them under `D:\Games\Dead Air` (`gamedata\scripts`; they may be
   packed - then check `D:\Games\XFined-Editor\_appdata_` and `gamedata\scripts` here) and
   implement the extra fields; a script class whose save logic cannot be reproduced statically
   falls back to `Opaque` - name it in the report.

## `gz-fmt-spawn` - new

Entity packets stay opaque bytes here; `gz-entities` decodes them.

- `all.spawn` / `*.spawn`: the chunk tree the game reads - header (version, guid, graph guid,
  spawn count, level count), the spawn graph (per vertex the STATE packet and the UPDATE packet,
  each length-prefixed, plus vertex ids and edges), artefact spawns, patrol paths, the game
  graph when embedded. Read fully and write back byte-identically. Spec: the writer in
  `Source/Editors/LevelEditor/AI/game_spawn_constructor.*`, `level_spawn_constructor.cpp`,
  `server_entity_wrapper.cpp`; the game's reader in
  `D:\Games\Dead Air\DeadAir-x64\src\xrGame\alife_spawn_registry*.cpp` (READ-ONLY).
- Patrol paths: `Source/Editors/LevelEditor/AI/patrol_path_storage.cpp`, `patrol_path.cpp`,
  `patrol_point.cpp` (names, points with position, flags, vertex ids, weighted links).
- The game graph and cross tables: `rg -n "CGameGraph|CGameLevelCrossTable"
  Source/xrServerEntities` (header, level table, vertices, edges, death points; `level.gct`),
  and the builder `Source/Editors/LevelEditor/AI/game_graph_builder.cpp`.
- An index for the editor's world reader: levels (id, name, section, offset), every entity with
  its level (game vertex -> graph vertex -> level), all patrol path names. The linked game's
  `all.spawn` is tens of megabytes with ~100k entities and 15k patrol paths: parse chunk bodies
  in parallel and copy a packet at most once (`Arc<[u8]>` slices of the file).

## Rules

- Pure data and pure functions; everything `Send + Sync`; decoding 100k entities parallelises
  trivially (no shared mutable state); few allocations per entity.
- Hostile input is an `Err` (then `Opaque`): never a panic, never an allocation by an unchecked
  count.
- Property values are `serde`-serialisable (they go out over MCP).
- The API at the top of each `lib.rs`. Tests written, not run: per class family a create ->
  write -> read -> write round trip with equal bytes, version branches, an `Opaque` round trip,
  packet quantisation by hand, a two-entity spawn file.
- Compile: `bash tools/rust/cg.sh check -p gz-entities -p gz-fmt-spawn --tests`, zero warnings.

## Report

Under 400 words: class families implemented, which fall back to `Opaque` and why, every
editor-vs-game layout difference, entry points of both crates, what is left. Then update both
rows in `docs/rust/STATE.md`.

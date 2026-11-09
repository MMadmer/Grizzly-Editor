# Brief: quests and behaviour graphs (`gz-nq`)

The node graphs of the editor: `.nqasset` quests and `.behasset` behaviour graphs. Read
`docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then
`docs/nq/NQ_FORMAT.md` (the format, normative), `CLAUDE.md` sections "Quests (NQ - node graph)"
and "Behaviour graphs", and the rustdoc at the top of `gz-luadata`, `gz-game`, `gz-project`,
`gz-mod`, `gz-editor`. `docs/NQ_ARCHITECTURE.md` is long: read sections 13 (MCP) and 19
(behaviour graphs), and others only when a question sends you there.

You own: `crates/gz-nq/**` (new; `src/lib.rs` before `Cargo.toml`) and the registration lines in
`crates/grizzly`.

## The C++ that is the spec

`Source/Editors/XrECore/Editor/Nq/`: `NqAsset` (the model, read and write), `NqCatalog` (the
kind catalog), `NqDoc` (an open document and its edit operations), `NqValidate` (problems),
`NqLayout` (the node box and the auto layout), `NqReferences`, `NqProjectIndex`, `NqPickers`
(lookups from the linked game), `NqItemCaps`, `NqExport`, `NqUtil`, `NqMcp` (the `quest_*`
handlers - the wire is frozen by it; more handlers in
`Source/Editors/LevelEditor/Editor/Mcp/LevelMcpGraph.cpp`). `NqLua` is already `gz-luadata`.
Tool schemas: `crates/gz-mcp/tools.json` (every `quest_*`).

## Design

- **The asset** (`asset.rs`): the graph as plain data - header, nodes with params, pins and
  links, tasks or roles, variables, bookmarks, `pos` optional. Parsed from `gz_luadata::Value`
  and written back through the luadata writer so the text is byte-equal to the C++ for the same
  graph. The extension is the kind; the header key has to agree with it (`E001`).
- **The catalog** (`catalog.rs`): the kinds with their params, pins, `docs` (quest / behaviour /
  both when absent), row schemas. Sources, in the C++ order: the linked game's
  `configs\nq\catalog.ltx` (the bundled copy is the fallback - `gz_game` gives "linked or
  bundled"), module kinds (`configs\nq\kinds\*.ltx`) of the installed modules, and last of the
  OPEN PROJECT itself (its installed copy skipped). A `Query` keyed by the game link's revision
  and the project file tree's revision: no "drop the catalog" call exists.
- **An open document** (`doc.rs`): one `Store<GraphDoc>` per open file, kept in a map in the
  area's project-scope service. Every edit operation of `NqDoc` (what `quest_edit` and
  `quest_apply` do) is a command of that store; nodes and links live in `imbl` maps so an edit
  shares the rest. Undo, redo and the history are the store's (`quest_undo`, `quest_redo`,
  `quest_history`), with labels as the C++ gives them; `dirty` is the store's. Save is explicit
  (`quest_save`: write on the IO pool, then mark saved); `quest_reload`; a file that changed
  on disk under an open document is noticed through the file tree's revision and reported as
  the C++ reports it. Register a `DirtySource`.
- **Validation** (`validate.rs`): a pure function `(asset, catalog, lookups) -> Vec<Problem>`
  with the C++ codes and messages. `quest_validate` is a `Query` keyed by the document's
  revision and the revisions of what it reads; `quest_check_all` validates every graph of the
  project in parallel on the CPU pool. Graphs under an excluded folder are not graphs of the
  build (`gz_mod::outside_build`). Register the area's `BuildCheck` with `gz-build`.
- **Characters.** The validator and the pickers need the project's characters (`E030`: not a
  character of the project, a template) and characters need this crate's pickers, so the
  dependency goes one way: define here `trait CharacterLookup: Send + Sync` (all briefs: id,
  template flag, display name) and a slot for it in the project-scope service; the characters
  crate fills the slot. With the slot empty, answer from the file tree's asset kinds.
- **The node box** (`layout.rs`): `NqLayout::Box` is the ONE source of a node's rectangle -
  240 pt wide, a 28 pt header, up to 3 "key: value" lines, up to 4 action rows then "+N more",
  pin names - for the auto layout, the canvas, the minimap and `quest_layout` alike. Port it
  exactly; it takes a text-measuring callback so the domain stays free of UI.
- **The project index, references, find** (`index.rs`, `references.rs`): every graph of the
  project parsed in parallel from the file tree, re-parsed only when its file's stamp moved; a
  `Query`. `quest_list`, `quest_project_find`, `quest_references`, `quest_rename_task`.
- **Lookups** (`pickers.rs`): the picker types of `NqPickers` over `gz_game` data (configs,
  strings, files, texture sheets): communities under the names the player reads, dialogs
  captioned with their first line, portraits (sheet and rect), ranks, items by in-game name,
  level changers, patrols... Each type is a `Query` keyed by the game link's revision; lists
  build in parallel. Never a hardcoded list. `quest_lookup`, `quest_catalog`.
- **Commands** (`cmd.rs`): every `quest_*` of `tools.json` except `quest_view` (that is the
  graph tab's UI state; the UI package registers it). `quest_minimap` and `quest_layout` answer
  from `layout.rs`.

## Plugging into `gz-project`

`gz-project` (read the rustdoc at the top of its `lib.rs`) offers `ProjectHooks`, `AssetTemplates`
and `AssetLists`; an area registers there what only it knows:
- `AssetTemplates` for `quest` and `behaviour` (`content_create` writes the C++ template text);
- `AssetLists` for the categories `list_assets` answers about graphs;
- the `renamed` hook when a rename has to follow references.

## Why

- An AI edits these assets as text and over MCP; problems are coded; `quest_get` returns an
  outline. Screenshots are not part of the workflow - keep every answer complete without one.
- The editor never compiles a graph into game Lua: the asset travels as it is.

## Done means

- `bash tools/rust/cg.sh check -p gz-nq --tests` clean, zero warnings. Tests written (not run):
  the examples under `docs/nq/examples` parse, validate clean and write back byte-equal; each
  edit operation; undo; the node box against sizes derived from the design; catalog merge order.
- The API at the top of `lib.rs`; commands registered.

## Report

Under 300 words: commands registered, deviations from the C++ wire, what is left. Then add a row
for `gz-nq` to `docs/rust/STATE.md`.

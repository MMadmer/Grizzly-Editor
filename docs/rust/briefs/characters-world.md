# Brief: characters and the mode's world (`gz-characters`, `gz-world`)

Read `docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then
`CLAUDE.md` sections "Characters (the module's own NPCs)" and "The mode's world (EditorWorld)"
- they are the behaviour contract, read them fully - and the rustdoc at the top of `gz-luadata`,
`gz-nq` (its `pickers` and `CharacterLookup`), `gz-game`, `gz-mod`, `gz-project`, `gz-fmt-spawn`,
`gz-entities`, `gz-build`.

You own: `crates/gz-characters/**`, `crates/gz-world/**` (new; `src/lib.rs` before `Cargo.toml`)
and the registration lines in `crates/grizzly`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorCharacters.h/.cpp` (2.7k): the model, `Resolve` /
  `LoadResolved`, validation codes, `CheckForBuild`, `Briefs`, the trade model, the item picker.
  Its `Draw*` functions are UI: skip them (the tab is a later package), but note what the form
  binds to (raw stored values).
- `Source/Editors/XrECore/Editor/EditorWorld.h/.cpp` (1.75k): `[world]`, `[start_kit]`, the
  reader of the linked game's `all.spawn` (levels, level changers, patrol names), validation
  (`E201`, `E202`). Its drawing is UI: skip.
- Handlers (frozen wire) in `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpMod.cpp` and where
  they delegate: `characters_list`, `characters_get`, `characters_write`, `characters_delete`,
  `world_get`, `world_set`, `world_transitions`. (`characters_view` and `world_view` are the
  tabs' UI state: not yours.) Schemas: `crates/gz-mcp/tools.json`.
- The game's meaning of both (READ-ONLY): `D:\Games\Dead Air\DeadAir-x64\docs\dead-air\MODDING.md`,
  "A module's characters" and "World rules".

## Design

**`gz-characters`**
- One `.npcasset` per character, declarative Lua (`gz-luadata`). The model keeps **unset as
  unset** (an empty value, never a default), or a child would override its base with a default
  it never asked for. `name` and `bio` carry every language at once; a text that ends up with
  `rus` alone collapses back to a plain string on write.
- `resolve`: `base` + `abstract` - unset fields inherited, starting kits MERGED (a repeated
  section takes the new count and keeps its place, `count = 0` drops an inherited row), a shop
  inherited or replaced as a whole. Resolution happens ONCE and everything downstream -
  validation, the build check, MCP answers - works on resolved characters. `E111` a base that
  does not exist, `E112` a chain that loops.
- The project's characters are a `Query` keyed by the file tree's revision: every `.npcasset`
  parsed in parallel, resolved, validated. `briefs()` comes from it; fill `gz_nq`'s
  `CharacterLookup` slot with it when the service is first made.
- Validation: every code of the C++ with its message (`W114` ... `W122`, `E118`, `E119`,
  `E123` - a model whose skeleton lacks a bone the stalker damage section names: read the bone
  names from the linked game's OGF through `gz-game` + `gz-fmt-ogf`, cached per model).
  Lookups come from `gz_nq::pickers`; trade categories from the linked game's
  `misc\trade\trade_presets.ltx`, never a list of our own.
- The module ships the `.npcasset` and the game makes the character: the build writes nothing
  for a character. Register a `BuildCheck` (`CheckForBuild`): it only refuses a module whose
  characters would not work, and skips what `outside_build` excludes.
- Writes are explicit (`characters_write` writes the file, on the IO pool); deletion is final.

**`gz-world`**
- `[world]` (five rules, `levels`, `player_choice` - three at most, `area`, `locked`,
  `locked_levels`) and `[start_kit]` are sections of `mod.ltx`: read and written through
  `gz-mod`'s manifest store (its raw section accessors), saved as edited. `world_set
  preset=empty` is the one-click empty world.
- The linked game's world: levels from the game graph, level changers with their level and
  destination, patrol names - from `all.spawn` through `gz-game`'s files, `gz-fmt-spawn`
  (container, graph, patrols) and `gz-entities` (a level changer's destination). **Read only;
  nothing of the game is ever written.** A `Query` keyed by the game link's revision; the
  spawn file is tens of megabytes: parse on the CPU pool, in parallel, once.
- Validation `E201` (one Player Start per module - the scene's part of that count arrives
  through a slot the scene area fills later; count what the module's baked files hold now),
  `E202` (more than three player choices). Register a `BuildCheck`.

## Plugging into `gz-project`

`gz-project` (read the rustdoc at the top of its `lib.rs`) offers `ProjectHooks`, `AssetTemplates`
and `AssetLists`; an area registers there what only it knows:
- `AssetTemplates` for `character` (`content_create`), `AssetLists` for the character categories.

## Done means

- `bash tools/rust/cg.sh check -p gz-characters -p gz-world --tests` clean, zero warnings. Tests
  written (not run): resolve and kit merging, the language collapse, each validation code on an
  inline fixture, world rule round trips through a manifest document.
- APIs at the top of each `lib.rs`; commands registered.

## Report

Under 300 words: commands, deviations from the C++ wire, what is left. Then add rows for both
crates to `docs/rust/STATE.md`.

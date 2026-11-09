# Brief: the linked game (`gz-vfs`, `gz-game`)

What the editor knows of the game a project is linked to. Everything here is READ-ONLY: the
editor never writes a file of the game. Read `docs/rust/CONVENTIONS.md` ("An area crate") and
`docs/rust/STATE.md` first; then the rustdoc at the top of `crates/gz-ltx/src/tree.rs`,
`crates/gz-fmt-xml/src/lib.rs`, `crates/gz-chunk/src/lzo.rs`, `crates/gz-editor/src/lib.rs`,
`crates/gz-project/src/lib.rs`; and `CLAUDE.md` sections "Startup", "Game modes", "Where a mod
exports to".

You own: `crates/gz-vfs/**`, `crates/gz-game/**` (new; write `src/lib.rs` before `Cargo.toml`),
and the registration line in `crates/grizzly/src/main.rs` + its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorGameContent.cpp` (490): the linked game's archives as a
  private file system; how loose `gamedata` and archives are ordered.
- `Source/Editors/XrECore/Editor/EditorGameConfigs.cpp` (195): how the game's `system.ltx` tree
  becomes the settings the editor reads (which root, which file, what is unpacked where).
- `Source/Editors/XrECore/Editor/EditorGameModes.cpp` (401): game modes scanned from the linked
  game - the script and the layout XML, the mode column vs the option column, the names from the
  string table, the modules' `[provides_mode]` under both `modules\` and `mods\`.
- `Source/XrCore/LocatorAPI.cpp`: how `fsgame.ltx` roots and `.db*` / `.xdb` archives are
  mounted (read the archive mounting and `fsgame.ltx` parsing only).
- Handlers (the wire is frozen by them; `rg -n '"<command>"'` to land on each):
  `game_link`, `game_modes`, `list_spawn_sections` in
  `Source/Editors/LevelEditor/Editor/Mcp/LevelMcp*.cpp` and `GrizzlyMCP.cpp`. Their tool
  schemas: `crates/gz-mcp/tools.json`.

## Design

**`gz-vfs` - the game's files as one immutable index.**

- `GameFiles`: path (lower-case, forward slashes, relative to `gamedata`) -> where the bytes
  are: a loose file, or `{archive, offset, packed, unpacked}`. Precedence as the game mounts
  them (loose `gamedata` over archives; archive order as `LocatorAPI` gives it). Also the
  modules of the game (`modules\<id>` and `mods\<id>`) as separate roots that can be listed.
- Built in parallel: archives are opened and their directories (chunk 1, LZHUF) read on the IO
  pool, one task per archive (`gz_chunk::lzo::read_archive_index`); the loose tree is walked in
  parallel. Tooling entries are skipped.
- **The index persists** (`editor.paths.appdata\game_index\<hash of the game root>.bin`): a
  compact binary file of the entries plus, per directory and per archive, the size and write
  time it was built from. At start the saved index is loaded and validated by those stamps
  (one `metadata` call per directory / archive, in parallel) and only what changed is re-read.
  A start costs the NUMBER of file operations: this is where most of them go away.
- `read(path) -> io::Result<Vec<u8>>` (blocking; unpacks LZO when packed), `exists`, `list(dir)`,
  `find(dir, ext)` (every file below a folder with an extension), `read_many(paths)` on the IO
  pool. No per-file `stat`: the index answers.
- The archive file handles: open per read (or a small pool); never one global handle.

**`gz-game` - what the files mean.** One service per open project (`session.services`), built
from the project's linked game root (see `gz-project`: the project state gives the root; an
unlinked project gives a `GameLink` that answers "no game linked" - nothing may panic).

- `GameLink::files() -> Arc<GameFiles>`; `rev()` (bumped when the link changes or a refresh is
  asked).
- Everything derived is a `Query` keyed by `rev()`, computed on first use, never at start:
  - `configs() -> Arc<gz_ltx::Ltx>`: the `system.ltx` tree. Discover includes breadth-first,
    read and `parse_file` every file in parallel through `GameFiles`, then `assemble`. cp1251.
  - `strings(lang) -> Arc<StringTable>` (via `gz-fmt-xml`), `texture_sheets()`.
  - `modes()` (EditorGameModes): never a hardcoded list - a global mod replaces both the script
    and the layout.
  - `spawn_sections()`: the spawnable sections grouped by class as the Spawn tool lists them
    (`list_spawn_sections`).
  - `game_version()` and whatever `game_link` answers.
- `bundled()` fallbacks: the editor ships copies of some game files under
  `editor.paths.root\gamedata\configs\{nq,pda}`; expose a reader that prefers the linked game
  and falls back to the bundled copy (the NQ catalog and the PDA reader use it).
- Commands: `game_link` (get / set the link: setting writes the project file through
  `gz-project`'s store), `game_modes`, `list_spawn_sections`.

## Why it is shaped this way

- The C++ unpacked the game's configs into `_appdata_\game_configs` and read them with the
  engine's file system, 30k file operations per start. Reading them through the index from
  memory, in parallel, is the single largest win of the startup budget
  (`docs/RUST_MIGRATION.md` 10.8). Do not unpack anything to disk.
- Lookups that later areas need (communities, ranks, portraits, dialogs, items) are NOT here:
  they belong to the quest area's pickers, which read `configs()`, `strings()`, `files()`.

## Done means

- `bash tools/rust/cg.sh check -p gz-vfs -p gz-game --tests` clean, zero warnings; tests
  written (not run): archive index round trip on a synthetic archive, precedence, index cache
  validation, modes parsing on inline fixtures.
- The API listed at the top of each `lib.rs`; commands registered.

## Report

Under 300 words: API, commands, what could not match the C++ wire, what is left. Then add rows
for both crates to `docs/rust/STATE.md`.

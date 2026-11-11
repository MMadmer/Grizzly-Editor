# Brief: the parity audit (read-only)

The C++ editor under `Source/` is being replaced by the Rust editor under `crates/`. Before the
tests, find **every function of the C++ editor an author or an agent can reach that the Rust
editor does not have**. You write one file, `docs/rust/PARITY.md`, and change nothing else: no
code, no other document. Other agents are editing crates meanwhile; you only read.

Scope of the C++ side: `Source/Editors/LevelEditor`, `Source/Editors/XrECore`,
`Source/Editors/XrEUI`, `Source/Editors/XrEProps` (the level and mod editor). NOT in scope,
dropped on purpose (`docs/RUST_MIGRATION.md` section 4 and section 12): `ActorEditor`,
`ParticleEditor`, `ShaderEditor`, the Clear Sky / Shadow of Chernobyl flavours, Play inside the
editor's own process (D1), the R1 look of the viewport (D2), the raw-pixel lint (D6).

Already known and being closed right now - list them in the ledger as "in work", do not
re-derive them: the tool panels of Sector, Portal, Way, Shape, Light, Fog Volume
(`briefs/tools-objects.md`); AI Map, Detail Objects generation and Wallmarks editing (next);
particles, glows, AI map, details, wallmarks and the cube sky in the viewport
(`briefs/render-rest.md`); breakable / climable objects and the rest of `Scene->Validate` in
the level build; VIPM for progressive skeletons; panels outside the window.

## How to find the rest - walk the C++ by its entry points

For each entry point, name the Rust counterpart (crate, function or command) or mark it
missing. Do not trust names: open the Rust side and see that it does the thing.

1. **The command table**: every `COMMAND_*` registered in
   `Source/Editors/XrECore/Editor/UI_MainCommand.cpp` and
   `Source/Editors/LevelEditor/Editor/UI_LevelMain.cpp` (and wherever else `REGISTER_CMD_*`
   appears under the four folders) with what its handler does. Internal plumbing (caption,
   toolbar refresh, resize, destroy) is listed once as a group, "no user function". The Rust
   side: `crates/gz-app/src/cmds.rs` (`list_commands`, the `UiCommand` table), each area's
   `cmd.rs`, the menus. `Intermediate\cargo\debug\Grizzly.exe -dump_commands` prints the
   registry (run it; it exits at once).
2. **The menus**: `Source/Editors/LevelEditor/UI/UIMainMenuForm.cpp` - every item of File,
   Edit, Scene, Objects, Compile / Build, Mod, Libraries, View, Windows, Help, with its command;
   the Rust menus are in `gz-app` (the shell's), `gz-ui-level/src/menus.rs`,
   `gz-ui-mod`, `gz-ui-viewport/src/menus.rs`, and `docs/UI_DESIGN.md` section 5 says what the
   menus should hold.
3. **The toolbars, the viewport's pills and the status bar** (`UI/UIMainForm.cpp`,
   `UIRightToolBar*`, `UITopBar*`, the shell `XrEUI/XrUIShell.cpp`).
4. **Context menus and key bindings**: the scene's bindings table (`UIKeyPressForm`,
   `ui_levelmain` shortcuts, `XrEUI` shortcuts), the Outliner's and the Content Browser's
   context menus, the viewport's right click.
5. **Every MCP command and its arguments**: `crates/gz-mcp/tools.json` is the C++ bridge's
   catalogue; for each command compare the ARGUMENTS and ACTIONS the C++ handler accepts
   (`Source/Editors/XrECore/Editor/GrizzlyMCP.cpp`, `Source/Editors/LevelEditor/Editor/Mcp/*.cpp`,
   `rg -n '"<name>"'`) with what the Rust handler reads (an `action=` the Rust side refuses, an
   argument it ignores, an answer field it never sends). This is the largest part: go command
   by command, be exact.
6. **Dialogs, popups, document kinds, panels**: `Source/Editors/XrECore/Editor/EditorUiContract.h`
   against `crates/gz-app/src/contract.rs`, and each `dlg.*` named in `CLAUDE.md` and
   `docs/UI_DESIGN.md` section 6 - does the Rust editor open it (`rg` its id)?
7. **Properties**: the property rows each scene class shows in Details (`FillProp` of each
   class under `Source/Editors/LevelEditor/Editor/Entry/**` and of the tools: Level Settings,
   the tools' own property sheets) against `crates/gz-scene/src/props.rs` and the class
   modules - a row missing, read-only where the C++ edits, a chooser that became text.
8. **Preferences** (`XrECore/Editor/EditorPreferences.cpp` and the Preferences tab) against
   `gz-app`'s table; the command line options; the files the editor writes for itself.
9. **Stubs in the Rust tree**: `rg -n -i "not ported|not yet|later package|not implemented|
   unsupported here|no settings of its own|TODO|FIXME" crates -g '*.rs'` - each hit judged:
   a real gap, or only a word.

## The ledger: `docs/rust/PARITY.md`

- A table per entry point: C++ item | what it does | Rust counterpart (crate::path or command)
  | state: `same`, `differs` (how), `missing`, `in work`, `dropped` (which line of section 12,
  or "not reachable in the C++ editor either" with the proof).
- Then **the list that matters**, at the top of the file: every `missing` and `differs` item,
  grouped by area (scene and tools, viewport, Content Browser, libraries, mod and build,
  quests, shell, MCP wire), each one line with the C++ file and function and an estimate of its
  size (S / M / L). No prose, no praise, no advice on how to fix.
- Be exact and be complete rather than fast: an item you could not judge is listed as
  "unjudged" with the reason, never left out.

## How to work

- You are one reader: start no agents. Search first (`rg`), read what the search points at.
- Change nothing but `docs/rust/PARITY.md`. No git command that alters the tree or the index.
  Do not run the editor (only `-dump_commands`), do not build, do not test.

## Report

Under 250 words: the counts (items walked, same, differs, missing, dropped, unjudged) and the
ten most important missing functions by name.

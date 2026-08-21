# XFined Editor — project rules

XFined Editor is the level and mod editor for Dead Air Refined
(game engine: `D:\Games\Dead Air\DeadAir-x64`, its rules live in
`PROJECT_RULES.md` there). Mods built with this editor target Dead Air
Refined STRICTLY and are not required to work on the original Dead Air.

## Compatibility

- **The editor has not shipped yet.** Until the first public release the
  editor architecture and the "game ↔ new (XMS) mods" protocol may be rebuilt
  freely: no fallbacks to earlier editor revisions, no migrations of its old
  export formats, no deprecation scaffolding. If the manifest or delta format
  changes tomorrow, the old one simply ceases to exist.
- **The game's obligations stay untouchable:** Dead Air Refined must keep
  supporting old mods (XDB, loose `gamedata`, JSGME, content addons) and
  savegames. An XMS protocol rebuild is a paired change (editor + engine)
  across both repositories, but on the game side it must not disturb the old
  modding path or the save contract (`.scop`/`.scoc`/`.scov` — see the game's
  `docs/dead-air/SAVE_COMPATIBILITY.md`).
- **After the editor's release** the rule flips: the export format contract
  freezes and the game starts guaranteeing that mods built by any past release
  of the editor keep working (already written into the game's
  PROJECT_RULES.md). Do not delete this reminder — rewrite this section at
  release.

## Mod philosophy

- A mod is a self-contained project in ITS OWN folder. Installed into the
  game it lives as a separate folder (mods/<name>); the game pulls its data
  from there itself. Nothing is copied or merged into `gamedata` or the game
  root, no clobbering of other people's files on name collisions.
- The folder layout inside a mod is arbitrary; the mod manifest and asset
  links/redirects tie it together (reference points: UE's asset registry and
  redirectors).
- Level deltas (`.xspawn`, `.xcform`, aimap, overlay_visuals, `game.graph`
  fragments) must work independently of other mods as far as possible.

## Build and verification

- Build: `pwsh -NoProfile -File .\Build.ps1` (pwsh 7 strictly — in
  PowerShell 5.1 git stderr kills the script). Artifacts: `Bin\x64\Release`.
- Test run: `LevelEditor.exe -project "D:\XFinedProjects\Test" -nodlg`
  (`-nodlg` = no modals: auto-answer No/OK into the log; a crash = symbolized
  stack + dump + exit code 3).
- The editor is fully drivable over MCP (TCP 127.0.0.1:28016,
  `tools/mcp/xfined_mcp.py`, documentation in `tools/mcp/AI_SETUP.md`).
  New UI features ship with an MCP command: the editor is AI-friendly by
  design.
- Screenshots: `xfined_screenshot_editor` arms on the first call and returns
  the frame on the second; `xfined_screenshot_viewport` returns the viewport
  immediately.

## Game modes

- A mod always targets ONE mode: the plain game, a mode that already exists
  in the linked game, or a new mode the mod itself brings. "Not selected" is
  not a third option: exporting a module without a target is forbidden.
- Do NOT hardcode the mode list: it is scanned from the linked game
  (`EditorGameModes`), because a global mod (Revolution II and the like)
  replaces both the `ui_mm_faction_select.script` script and the whole
  `ui_mm_faction_select_16.xml` layout. A mode differs from an option by its
  COLUMN in that layout (`dar2_mode` vs `dar2_option`) — "one life", "rich
  stashes", "high rewards" are options and do not belong in the list.
- The author picks by the names the player sees (the game's string table,
  cp1251 → UTF-8), not by technical ids.
- A new mode brought by a mod is effectively a separate new game; it does not
  start on the original Dead Air, and that is fine. Only what worked on the
  original must never break.
- The export REGISTERS its own mode (`WriteModeRegistration`): without the
  checkbox on the new-game screen the mode does not exist for the player —
  the engine does not draw it, that screen is assembled by a game Lua script.
  Three files are generated per module, overwritten on every export:
  `gamedata\configs\ui\<module id>_modes<aspect>.xml` (the module's own
  layout with the checkbox and its caption, one per aspect-ratio variant),
  `gamedata\configs\text\rus\<module id>_modes.xml` (the captions, cp1251)
  and `scripts\mode_register.script` (creates the controls at runtime and
  writes the `[character_creation]` keys). The names carry the MODULE id, not
  the mode id: renaming a mode rewrites the same files instead of leaving a
  stale checkbox behind. `scripts\register.script` stays the author's own and
  is never touched; the legacy `patch\xms_modes.xmlp` is only cleaned up.
- The names are not decorative: the engine turns `new_game_<id>_mode` back
  into the mode id, and the same id is compared against `mode=` in the
  manifest and `xms.mode_active`.
- The checkbox coordinates are MEASURED (`EditorGameModes::SuggestSlot`)
  against the layout of the game that is linked, not invented: Revolution II
  has its own layout, stock has its own. Verified live: the control lands at
  305,605 in the DAR2 campaign column, the caption comes from its own string
  table.
- The screen script keeps `CScriptXmlInit` local (`self.xml` does not
  exist) — the wrapper needs its own
  `xml:ParseFile("ui_mm_faction_select.xml")`; the engine substitutes the
  aspect-ratio variant itself.

## Where a mod exports to

- A module goes to `<game>\modules\<id>`, NOT to `mods\`. `mods\` is the
  JSGME folder (the same one as `MODS` — Windows is case-insensitive): JSGME
  shows every subfolder there as a mod of its own and "activation" copies it
  over the game — exactly the gamedata merge the module exists to avoid.
- The flat export (`gamedata_<id>`) is the opposite: it IS a JSGME mod and
  keeps its name.
- Enabling/disabling a module is engine-side: `xms_disable <id>` /
  `xms_enable <id>` write the id into `modules\disabled.ltx` and copy nothing
  anywhere.
- The editor's scanners (modes, occupied checkbox rows) read BOTH roots,
  `modules\` and `mods\`, because what is already installed must not break.

## Quests (NQ — node graph)

- Architecture and rollout plan: `docs/NQ_ARCHITECTURE.md`. Until the
  implementation starts, the document IS the contract; change it by editing
  the document, not "as you go".
- Rule number one: **the editor does not compile the graph into game Lua.**
  The `.nqasset` (declarative Lua, `return {…}`) travels into the module as
  is; the NQ runtime of Dead Air Refined interprets it. The kind catalog
  (`configs\nq\catalog.ltx`) is supplied by the game and scanned by the
  editor (the bundled copy is a fallback only).
- Quest assets must be readable and writable by an AI as text: `pos` is
  optional, problems are coded, `quest_get` returns an outline; screenshots
  are not part of the workflow.

## Dangerous operations

- Any destructive operation from the UI requires confirmation. Destructive is
  everything that loses someone's data: deleting files/folders or scene
  objects, overwriting existing files, clearing the scene, losing unsaved
  edits. The dialog must NAME what disappears (names, counts), not ask "are
  you sure?"; the safe button is the default (Enter/Escape = cancel).
- Disk deletion goes to the Recycle Bin (`SHFileOperation` +
  `FOF_ALLOWUNDO`), not through `DeleteFile`: file operations have no editor
  undo, and the cost of a miss must be "restore", not "redo".
- MCP commands show NO confirmations (the caller already decided) and must
  never hang on a modal — the destructive UI path and the programmatic path
  share the implementation, but not the dialog.

## Code and text

- Style: the local style of the file; comments only where the "why" is not
  obvious, short and in English.
- All product text is English in the official tone: UI labels, hints, the
  kind catalog, logs, MCP descriptions, generated files, docs. Russian
  remains only where it is game content the player sees (quest titles,
  dialog phrases, string tables) or fixture data in encoding tests.
- UI text is short. A label names the thing; details go into the hover hint.
- Check plain pointers with the implicit boolean conversion
  (`if (p)` / `if (!p)`), not by comparing with nullptr.
- From any ImGui panel's `Draw()` never call anything that redraws the frame
  (`RedrawScene(true)` degrades by itself, but do not create new such
  paths): a nested ImGui frame kills the outer one — see
  XrUIManager::InUIPass().
- When editing files under `Source/XrRender/Private/`, check for twins: the
  editor builds ITS OWN copies of some files from
  `Source/Editors/XrECore/Editor/` (example: D3DUtils.cpp lives in both
  places; the editor compiles the editor copy).

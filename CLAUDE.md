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

## Startup

- `EditorStartup` owns everything between the double-click and the first
  frame: the splash window (a thread of its own, so it keeps painting while
  the main thread blocks in file work) and the stage profile the log ends
  with. `Stage()` names a stage for both; `Finish()` runs at the first frame.
- **Every start writes its own profile** — one line per stage with its
  seconds, read calls and file operations, plus a `loader` line for what
  Windows did before `wWinMain`. Read that before optimising or before
  believing a report of a slow start; the first stage measured is the process
  creation time, so a slow loader (a scanner meeting the binaries) is told
  apart from slow editor work.
- **What a start costs is the NUMBER of file touches, not the bytes.** Warm it
  is ~19k reads and ~62k file operations for 26 MB. Cold — a file cache that
  aged out, an on-access virus scanner seeing those files the first time —
  every one of those is a round trip, which is the difference between three
  seconds and a minute. So the thing to cut is always the operation count, or
  to overlap the latency; reading less data is beside the point.
- Two lessons already paid for: `CPU::Detect` spun a full second to measure
  the TSC (it counts against QPC over 20 ms now - see `_math.cpp`), and the
  8.5k texture descriptions were read one at a time (`LoadTHM` prefetches them
  on a few threads and parses on the caller's; proved to produce identical
  data).
- The Per-Monitor V2 opt-in happens ONCE, in `EditorStartup::DpiScale()`,
  because the splash exists before the editor window and awareness cannot be
  changed after the first window.

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

## Mod releases

- **Mod → Package Release…** / `mod_package` (`EditorMod::PackageRelease`)
  writes three kinds of files into `<project>\releases\<version>`: the
  descriptor `<id>.update.ltx`, the file index `<id>-<version>.files` and the
  package `<id>-<version>.zip` (`.partN.zip` when split). The game's module
  updater reads exactly these from the latest GitHub release of
  `[update] github`.
- The file index names every file's SHA-256, size, package, data offset,
  packed length and method. The game never parses a package as a zip: it asks
  GitHub for the byte ranges the index names and downloads only the files that
  differ. So the index must state what is ON DISK — offsets, packed lengths
  and methods are READ BACK from the finished archive (`ReadPackageLayout`,
  minizip's unzip API), never worked out from what the writer was asked to do;
  the content hash is taken while the file is fed to the zip. Anything that
  rewrites a finished package (another zip tool, a changed compression
  setting applied afterwards) invalidates its index.
- The descriptor and index names and formats, the package names and the package
  layout (every entry `modules/<id>/` + its index path, UTF-8 names, no path
  twice) are a **FROZEN contract shared with the game**. The normative text is the game
  repo's `docs/dead-air/MOD_UPDATES.md`. This is the one exception to the
  pre-release freedom above: a published release lives on GitHub, outside both
  repositories, and has to install forever. Change it ONLY by adding optional
  keys, sections or index records; anything else is a paired change that
  starts in that document.
- A package always holds the COMPLETE module - never a patch or a delta. It is
  staged through the real module export (`EditorMod::Export`), so a release
  ships what a build ships and refuses what a build refuses. The same zip is
  the manual install: extracted into the game folder it lands in
  `modules\<id>`.
- Manifest metadata rules (`ValidateVersion`, `ValidateWebsite`,
  `ValidateGithubRepo`, the quoting of `name`/`author`/`description`) mirror
  section 1 of the same document. The website allow-list (AP-PRO, ModDB) is
  enforced on BOTH sides: the game ignores any other link, the editor refuses
  to save it. Extending it is a game release first.
- `releases\` is editor-only (`EditorProject::IsEditorOnlyEntry`): never shown
  as content, never exported, never packaged. Inside the project a release may
  only be written under `releases\`.
- **Publish Release** (toolbar, Mod menu) / `mod_publish`
  (`EditorPublish`, GitHub side in `EditorGithub`) packages and then publishes
  to `[update] github`. The order is the point: assets go into a DRAFT, every
  stored asset is checked against the package result (name, size, SHA-256
  digest), and only then the draft is published with `make_latest` - the game
  reads the latest release alone and must never see one that is half there. A
  published tag is never overwritten (refused: raise the version, or delete
  the release first - the game compares content for a release of the installed
  version and offers a republished one as a fix, game repo MOD_UPDATES.md 3.1);
  a leftover draft of the same tag is reused; a tag this project deleted within
  the last hour is not offered again (`releases\deleted.tags`, `FreshTag`:
  `v<version>`, `-r2`, `-r3`, ...) because GitHub keeps serving a deleted
  release's files under a reused tag for minutes - seen live - while any
  published `v<version>[-rN]` still means "this version is out"; a latest release that carries another
  module's descriptor is refused, because this release would hide it.
- **Releases** (toolbar, Mod > Manage Releases) / `mod_releases` lists what the
  repository holds, drafts included, edits the title and the notes of a release
  (`EditorGithub::EditRelease` always sends `make_latest` with the mark's
  current place - GitHub's default for an edit would move Latest onto the
  edited release, and Latest is what players are served), and deletes one
  release together with its
  git tag - the release FIRST, the tag second: a published release whose tag
  vanishes first turns into a draft instead of going away. A draft owns no tag,
  so none is touched for it. This is the one deletion in the editor that no
  Recycle Bin backs: GitHub keeps no copy. The window therefore names the
  release, its files and the tag, says what players get instead when it is the
  latest one, and keeps Cancel as the default; the MCP path asks nothing and
  never opens a window of the editor. A permission prompt, where one is wanted,
  belongs to the caller: the bridge exposes the deletion as a tool of its own
  (`xfined_mod_release_delete`, MCP `destructiveHint`) so that a client can ask
  on exactly that call, and an unattended agent still runs straight through.
- The account token comes from the environment, the Windows Credential Manager
  or `gh auth token`, in that order. It is never written into the project, a
  log or an MCP answer; the one MCP request that carries one is
  `mod_publish action=save_token`, which stores it. It is sent to
  `api.github.com` / `uploads.github.com` only - the upload address comes out of
  a response body and is checked before the token goes there. `-qa_github` plus
  `XFINED_GITHUB_API` / `XFINED_GITHUB_WEB` point the client at
  `tools/qa/github_mock.py`; in that mode only `XFINED_GITHUB_TOKEN` and a
  separate QA credential are read, so a real token cannot reach a mock.
  `tools/qa/test_mod_publish.py --project <project>` drives the whole path over
  MCP against that mock and restores the project's `mod.ltx`. Publishing is
  never tested against GitHub itself: a release there is public.
- The device sign-in ("Sign in with GitHub") stays hidden until `kClientId` in
  `EditorGithub.cpp` holds the client id of the editor's GitHub OAuth app.

## Characters (the module's own NPCs)

- One `.npcasset` per character (`EditorCharacters`), declarative Lua like a
  quest. **Mod → Characters…**, the content browser's **Create → Character**,
  and `characters_list` / `characters_get` / `characters_write` /
  `characters_delete` over MCP - the asset is text, so the AI path is the
  primary one.
- The build (`EditorCharacters::Generate`, called from `EditorMod::Export`)
  turns them into the five files X-Ray needs and **appends** to the game's
  lists: `+files` and `+specific_characters_files` in
  `patch\<module>_characters.ltxp`. `+` is XMS's append directive
  (`xms_ltx.cpp`), which is the whole reason a module can have characters
  without overwriting `system.ltx` and fighting every other module that wants
  the same. Never generate a whole `system.ltx`, and never patch the game's own
  `npc_profile.xml`: ship OWN files and append their names.
- The profile always names the `specific_character` outright. `class` alone
  makes the engine pick a RANDOM character of that class - fine for ambience,
  wrong for the NPC a quest points at.
- `<module>_<id>` is one name doing three jobs: the spawn section (it carries
  `$spawn`, so the character is placeable by hand and by `spawn.object`), the
  squad section `<module>_<id>_squad` (`spawn.squad`), and the
  `character_profile` of a stalker placed in a level.
- Generated files are listed by `EditorMod::GetOwnedFiles`: the content browser
  hides them, and the last character deleted takes all of them with it.
- `npc.set_name` is NOT this. It renames an NPC at run time, for when the player
  learns who someone is; it needs no profile and changes nothing on disk.
- QA: `tools/qa/test_characters.py --project <project>` runs the whole path over
  MCP on a COPY of the project and reads the generated files back.

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

## Scenarios (NPC action assets)

- The contract is `docs/NQ_ARCHITECTURE.md` §19; the reference is
  `docs/nq/NQ_FORMAT.md` §10. A scenario is the SAME `.nqasset` and the SAME
  interpreter with `type = "scenario"`: no tasks, no dialogs, never starts
  itself, and its actions address **roles** the calling quest binds
  (`scenario.run`). One asset therefore serves any cast, and a role holds a
  group as happily as one stalker.
- **A step does not move, aim or shoot anything.** It writes a logic section and
  hands it to a stock scheme (`create_ini_file` + `xr_logic.configure_schemes`),
  which is what a smart terrain does in `xr_gulag`/`smart_terrain`. Combat,
  cover, danger, wounded and death keep working because the same schemes keep
  running them. Do NOT add movement or combat code to the runtime: if a step
  cannot be expressed as a scheme, the answer is a scheme, not a second AI.
- Places are **waypoint paths** (the WayPoint tool, shipped in the module's
  spawn layer), not coordinates: X-Ray walks patrol paths.
- An NPC that is offline cannot take a scheme, so the step is parked in the
  instance state and applied when it spawns in. A scenario releases everyone it
  touched when it ends, in `xms_nq_scenario.release` - never leave an NPC in a
  scheme nothing drives.
- Game side: `gamedata\scripts\xms_nq_scenario.script`, registered in
  `xms_nq.script`'s EXTENSIONS. An instance is an ordinary graph instance
  (`core.instance_start/restore/forget`) with the uid `<module>.<scenario>#<n>`
  - the `#` is what no asset id can contain, so an instance never collides with
  a quest.
- The catalog says which document may carry a kind (`docs = quest,scenario`;
  absent = both). The editor enforces it (`E023`), so does the loader. `E024` is
  a `who` naming a role that is not declared.
- Catalog and runtime must stay in step: `tools/qa/Test-NqCatalogParity.ps1` in
  the GAME repo fails the build when a kind is declared without an
  implementation or the other way round.
- QA: `tools/qa/test_scenarios.py --project <project>` drives the editor side
  over MCP on a COPY of a project.

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

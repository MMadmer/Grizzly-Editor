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
- Screenshots are ONE call and always current: the pump HOLDS a
  `screenshot_editor` / `screenshot_viewport` request until the frames it asked
  for were presented (`XFinedMCP::Pump`, `WantsFrame`), then answers. It has to:
  the editor redraws on demand and a background one draws nothing at all, so an
  answer given at once showed the frame from BEFORE the caller's last commands -
  and because no UI pass ran, a scroll or a selection asked for over MCP was not
  even applied. `ui_list`, `ui_audit` and `titlebar_info` read frames too and are
  held the same way (`IsShot`). `quest_view`, `pda_preview`, `preview_model` and
  `texture_preview` apply at once and, when their answer is `pending`, are held
  for the frames that show it and read again with `answer_only` (`FollowsFrames`):
  one call, raw socket included. Only the ACTIVE tab is drawn, so a picture shows
  that tab: anything that must be looked at from outside needs an MCP way into
  view (`ui_tab` / `ui_panel` / `ui_drawer` / `ui_dialog` / `ui_popup` / `ui_scroll`,
  the openers, `quest_view` panes / `action_split` / `vars_split`).
- **A modal lives in the editor's own window.** Multi-viewports are OFF unless
  Preferences > General > Allow Panels Outside the Window (`[ui] floating_panels`,
  takes a restart) is on; `outside` is empty then. With them on, ImGui gives any
  window whose rect leaves the main viewport - or that opens while it is
  minimized - a platform window of its own, which no screenshot holds: the
  capture mirrors the main swap chain. Seen live as a dimmed editor with no
  dialog - a modal grown past the editor's bottom, or opened where a stale
  `level_imgui.ini` put it. So `BeginPopupModal` (imgui.cpp, `XFined Editor:`)
  pins every modal to the main viewport, moves it back inside its work area,
  centres it again when that area changes and never sizes it past it (a taller
  one scrolls) - whatever the preference says. `screenshot_editor` answers
  `active_tab`, `busy`, `dpi`, `modal` (the dialog on top: `title`, `id` - a kit
  dialog's `dlg.<name>` -, its rect in the picture) and `outside` (the windows the
  picture does not hold).
- **A minimized editor is pictured the way it looks restored.** It draws only
  when a screenshot asks, and ImGui draws nothing of a minimized viewport: the
  picture was one flat colour. `XrUIManager::NewPlatformFrame` tells ImGui the
  main window is restored for such a frame - where it stood last, at the swap
  chain's size (`SetRenderSize`) - unless another window of the editor is in
  front: somebody is looking then, and a panel dropped there must not vanish.
  A launcher that asks for a minimized start (a shortcut's Run: Minimized, the
  QA harness's `SW_SHOWMINNOACTIVE`) gets one whatever `level.ini [render]
  maximized` says, restored to that state later (`CEditorRenderDevice::
  Initialize`): QA editors no longer come up over the author's work.
- QA: `tools/qa/test_mod_catalog.py` F saves the Submit window far below the
  editor, then minimizes the editor, and each time wants the window in the
  picture - `modal` inside it, centred, and more than a flat fill there (the PNG
  is decoded); Submit is a kit dialog (`NoSavedSettings`), so the stale layout
  entry is ignored rather than corrected, and its height must not move while the
  check runs. The layout and preferences files come back byte for byte.

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
- The fonts load as a stage of their own (`Loading fonts`: segoeui, seguisb,
  CascadiaMono); a frame drawn before it loads them itself.

## Editor UI (UI 2)

- **The contract is `docs/UI_DESIGN.md`.** A UI change is a change of that
  document first; it names every surface, size, colour and MCP answer.
- **The kit (`XUI`, `XrEUI/XrUIKit.h`) is the only way to draw UI.** Colours are
  theme tokens (`XFinedTheme::U32` / `Vec4`, `XUI::ToneColor`), sizes are pt
  (`XUI::Pt` / `Em`); DPI enters in ONE function, `XUI::SetDpi`, so a size in px is
  right at one scale only. `tools/qa/lint_ui_units.py` fails raw px, colour
  literals and a shortcut drawn by `ImGui::MenuItem` (use `XUI::MenuItem`: keycaps
  of what is bound NOW); a line that really means it says `// ui-units: ok <why>`.
  xrCore's `FLT_MIN` is `-FLT_MAX`: `SetNextItemWidth(-FLT_MIN)` asks for infinity -
  pass `GetContentRegionAvail().x`. A widget the kit lacks starts as
  `XUI_Local<Name>` in its file and moves into the kit when a second surface wants
  it (UI_DESIGN.md section 8 lists what is still local).
- **Icons are `XUI::Icon` vector glyphs** (`XrUIIcons`: a 16-unit grid, built
  once, only scaled), `IconName` their MCP name; the 20 pt logo is the only bitmap
  left. `-iconsheet` shows them all.
- **The shell (`XrUIShell`) owns the window.** Every asset editor is a full-window
  major tab (`XUI::Shell::Open`, `DocDesc`): Level is tab 1 and never closes; a
  quest / behaviour graph is `quest:<rel>` / `beh:<rel>`; `pda`, `model`, `texture`
  switch to a new target; `characters`, `mod_settings`, `world_rules`, `releases`,
  `preferences` and the three libraries are singletons. Only the active tab is
  drawn - Level behind another keeps its dockspace alive (`KeepAliveOnly`) and
  renders no scene (`screenshot_viewport` forces a pass). Each tab has its toolbar
  row (always ending in Build Mod, Publish Release, Releases), the status bar
  carries the drawers' buttons, the last log line worth reading, the error and
  warning counts since the project opened and the tasks / rebuild chips. The
  Content Browser and the Output Log also open as drawers over any tab
  (Ctrl+Space, Ctrl+\`), one at a time; another tab coming up closes it.
- A project's tabs live in `<project>\temp\session.ltx` and come back lazily (a
  placeholder until activated; "File not found" / `missing` when the file is
  gone). Level has two size classes, **Wide** (1400 x 640 pt and up: browser and
  log docked) and **Compact** (drawers only); crossing one rebuilds the layout
  unless the author customized it (`[ui] level_layout_custom`). `[ui]
  layout_version` other than 2 skips `level_imgui.ini` once - no migration.
- **Keys, first taker wins:** a text field, the shell's chords (Ctrl+Shift+P,
  Ctrl+(Shift+)Tab, Ctrl+F4, Ctrl+Space, Ctrl+\`, Ctrl+S, F11), the tab's
  `shortcut()`, then the scene's bindings - only while `SceneKeysAllowed()` (Level
  active, nothing modal, no key-owning panel focused). `BlockShortCuts` is gone:
  Delete or Ctrl+D in a document can never reach the scene.
- **Dialogs are `Title###dlg.<name>`** (no "XFined Editor - " prefix): modal,
  `NoSavedSettings`, pinned and centred, S/M/L/XL, Enter = default, Esc cancels, a
  question raised inside one is a sheet of it. Who answers a question (the kit's
  `Confirm` / `UnsavedPrompt` and `ELog.DlgMsg` alike): **`FromMcp()`** - the caller
  decided: `Confirm` is Yes and the action runs before the reply, losing data needs
  `save=` / `discard=` or the command answers `{error:"unsaved", dirty}`, an ELog
  box answers as under -nodlg; **`Unattended()`** (-nodlg) - the logged answers
  (`~ [dlg suppressed]` for a No); otherwise the dialog opens and `then` runs on a
  later frame. ELog errors and notes are queued Notices once the UI draws. Builds
  started from the UI report in the one `XUI::Result` dialog; MCP builds answer
  JSON only.
- **`EditorUiContract.h` is the ONE place for a constant QA pins** - tile range,
  panel / drawer / dialog window names, document kinds, popups, menus, card menu
  labels. The code that draws it reads it from there, MCP `ui_contract` answers
  it, and QA reads `ui_contract` instead of hardcoding a title.
- **MCP drives all of it** without a mouse: `ui_list`, `ui_tab` (+ `ui_tab_discard`),
  `ui_panel`, `ui_drawer`, `ui_dialog`, `ui_popup` (to look at, never to act),
  `ui_scroll`, `ui_split`, `ui_audit` (what a frame cut short or let escape),
  `ui_scale` (tests only, never saved), `ui_contract`, `window_control`
  (+ `window_close_discard`), `project_close` (+ `project_close_discard`),
  `titlebar_info`, `reset_layout`, `preferences`, `shortcuts`. Openers bring their
  tab up in the same call, reads never switch tabs, answers give px with a `dpi`
  and take pt. Unsaved work is lost ONLY through the three `*_discard` bridge
  tools (`destructiveHint`); every other path refuses with `unsaved`.
- The libraries are tabs too (Libraries menu, MCP `library`). The Texture Library
  reads the `.tga` sources of `$textures$` (rawdata\textures); Check New Textures
  turns a library tab without unsaved changes into New Textures (from `$import$`)
  and the finished import brings the library back. An empty library says what it
  reads (rawdata\sounds, gamedata\lanims.xr). The Detail Object list is the L
  dialog `dlg.detail_objects`.
- Gone for good: the Object List (Scene > Find in World Outliner focuses the
  Outliner's search), the Minimap Editor, the `ed\bar` toolbar bitmaps,
  `spectrum.*`, `rawdata/level_imgui_default.ini`. Viewport stats are off by default
  and a Mono pill over the viewport - nothing is printed into the render target
  (the Rebuild chip says what NEED REBUILD said). The render target grows in 64 px
  buckets; whatever reads it back crops to the render size.
- QA (each on a COPY, the ini files back byte for byte):
  `tools/qa/test_ui_shell.py` (the frame, the caption hit test, tabs, drawers,
  panels, dialogs, size classes, menus, the palette), `test_ui_tour.py` (the 35
  audit issues at 1707x912 / 1.5 and 1280x720 / 1.0; `--before` shoots and never
  fails), `lint_ui_units.py`, `test_graph_ui.py` (Frame All readable, panes,
  splits, minimap, content-sized nodes), `test_previews.py` (model, texture,
  libraries), `test_properties.py` (Details, Level Settings, choosers,
  Preferences, shortcuts; needs the Escape scene).

## The Start page (project browser)

- With no project open the work area is the Start page (`Start Page###start_page`,
  `XUI::BeginPage`) - a page, not a modal, so `screenshot_editor` answers `modal:
  null` there. Left the New Project form (Location, Name with its rule as the hint,
  Create - MCP `project_create`) and Open Folder...; right a search, a Last Opened /
  Name sort (`_projects.ini [browser] sort`) and 232 x 184 pt cards: the preview
  cropped to the card, the name on a row of its own with the ... menu, the path
  with a start ellipsis (two cards of one name keep the differing parent folder
  whole in the path line), a Missing / Not a project badge. The caption shows no
  menus on it.
- Link Game and Recover Unsaved Scene are M cards on the same page
  (`XUI::BeginPageCard` / `CardFooter`): Link Game - Close Project | Link; Recovery -
  Discard... (GhostDanger, a Confirm naming the snapshot's time and the older
  snapshots) | Restore (the default). The caption shows no menus while Recovery
  owns the page (`EditorRecovery::OfferPending`).
- **preview.png is a pass of its own** (`EditorProject::CapturePreview`): the open
  scene rendered once more from the viewport's camera, at its size, into a target
  of its own - no grid, pivot, selection, gizmo, axis triad, stats or captions (the
  level editor hides the selection and the gizmo for it, `SetPreviewScope` from
  `EditorRecovery::Tick`). Never inside the UI pass, never during Play in Editor. A
  close that clears the scene takes it BEFORE the clear (Switch Project - deferred
  out of the UI pass for it -, `project_close`); `Close()` takes it for a scene
  still up (the editor quitting). A picture of nothing (`ShowsNoScene`) keeps the
  last good one; a card whose preview.png is such a frame draws the placeholder.
- project.ltx `[project] last_scene` is project-relative and written by `Close()`
  alone (a project that never had the key keeps not having it). An absolute path
  counts only inside the project, so a copied project never opens - and then
  Ctrl+S-saves into - the original's scene.
- Import Base Scene is an M kit dialog (`dlg.import_base_scene`) with a search; it
  reads the library (and which scenes the project holds) once.
- A card's ... button (or a right click): **Show in Explorer** and **Delete...**
  (in the error colour). A card whose
  folder is gone, or holds no project.ltx / mod.ltx any more, stays on the list
  (a drive that is not plugged in is not a project nobody wants), veiled, opens
  nothing, and its menu offers **Remove from List** instead - no disk work, no
  question. `list_projects` names every entry with its `state`.
- **One set of checks, `EditorProject::CheckDelete`**, stands in front of the
  confirmation, the deletion (`DeleteProject`, which checks again) and MCP alike:
  only a folder `_projects.ini` lists that holds project.ltx or mod.ltx; never
  the open project; never a link (refused, naming where it points - "delete it
  where it really is"); never a drive root; never a folder that is or holds the
  user's profile, Documents, Desktop, Downloads, AppData, Temp, Windows or
  Program Files; never one that is, holds or lies inside the editor (the exe's
  folder, `sdk_root`) or a game (`game_root` of any listed project or of the open
  one, `[xms] export_target`); never one that holds ANOTHER project (a nested
  project.ltx / mod.ltx, named; a crashed packaging's `temp\release_stage_*` is
  the editor's own). Paths compare both as written and where they really are
  (`GetFinalPathNameByHandle`). A folder inside that cannot be listed refuses the
  whole deletion - it could hide a project.
- The deletion is final (no Recycle Bin, like every deletion here). Inside, a
  junction or symlink goes as a link and is never followed - a OneDrive
  placeholder is a reparse point too but no link, so `IsReparseTagNameSurrogate`
  decides; the read-only mark goes first, on the entry itself (git objects).
  The manifests go LAST: a deletion that stops at a file in use leaves a project
  that deletes again once the file is free; the answer names every entry that
  stayed and the list keeps the project. Success means the folder is gone.
  `EditorFileOps::Delete` keeps refusing the open project's root - a project is
  deleted from the browser, never from inside itself.
- The confirmation names what disappears - the project, its folder, files,
  folders and size, the git repository (`.git`, the whole history) and
  `releases\` when present, "Deleted for good - this cannot be undone." -
  Cancel the default, Enter and Escape cancel. It is the top-level kit dialog
  `Delete Project###dlg.delete_project` (M; Delete is Danger), counts once, when
  it opens, and a project that opens under it (MCP `open_project`) takes it down.
  A card that leaves the list hands its preview to `s_DeadPreviews`, released on
  the next frame: the frame being drawn may still hold it.
- MCP: `project_create`, `project_reveal` (`dry_run` opens nothing),
  `project_delete` (asks nothing; the bridge tool carries `destructiveHint`),
  `project_forget`, `project_browser_menu show=menu|confirm|none` (to look at,
  never to act; `search=` / `sort=` change what the page shows and `cards` answers
  it, `preview` per card). QA: `tools/qa/test_project_browser.py [--shots <dir>]`
  works on throw-away projects in a temp folder only and restores `_projects.ini`
  byte for byte.

## Game modes

- **A mode is not the mod.** `[module] name` is the mod's title, the one the
  game's Mods menu prints (`UIModsWnd.cpp`); it is free text and owes nothing
  to the module id or to the project folder, which only SEED it. `mode id` /
  `mode title` belong to the new-game checkbox the module adds. Keep the two
  apart in every label and hint - they were confused in the field once already.
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

## What a build ships

- A project folder is the author's working place as well as the module: a git
  repository, design notes, tool settings. The build ships "everything the author
  made" (any layout, `[vfs]` publishes it) EXCEPT, in this order:
  - **tooling** - `EditorProject::IsToolingEntry`: every dot-name (`.git`,
    `.gitignore`, `.github`, `.vscode`, `.idea`, `.vs`, ...), `Thumbs.db`,
    `ehthumbs*.db`, `desktop.ini`, Office `~$` locks. At ANY depth, and not only
    left out of the build: never listed by the content browser, never walked for
    assets (NqUtil::ListFiles, the quest index, the characters), never a reason
    for the browser's watcher to refresh - a `.git` is thousands of entries and git
    rewrites it on every status;
  - the editor's scratch and the sources at the root (`IsEditorOnlyEntry`,
    `IsSourceOnlyEntry`), as before;
  - **mod.ltx `[exclude]`** - the author's call (`EditorMod::ExcludeMatch`): a
    project path (a folder takes everything below it) or a pattern (no slash = a
    name at any depth, `*.psd`; with one = a path, `docs/*.md`). Set from the
    browser (Exclude from Build / Include in Build, the tile is veiled and says
    why on hover), the Mod Settings tab's Build Contents page, MCP `mod_exclude` /
    `mod_set_manifest exclude=`. It lives in mod.ltx so it travels with the
    repository (the game ignores the section, like `target`);
  - links (junctions, symlinks): not followed - one may point at the project;
  - `.npcasset` - a character's source, like an `.object` in rawdata: the game
    reads the files the build generates from it, never the asset.
- `EditorMod::OutsideBuild(root, rel)` is the one question every walk that feeds
  a build asks: the graphs of the project, the characters and the export agree on
  what the module is. A quest in an excluded folder is no quest of the build (it
  cannot block it), a character there is not generated.
- The export makes ONE list of what it will write (`SBuildWalk`) before anything
  is written: the "target already holds files" question counts every target file
  this build will not write (root level included), and `left_out` - what the walk
  passed by - reaches the result dialog ("Not shipped: ..."), `mod_export`,
  `mod_package` and the release dialog. A rule that would take out a file the
  build generates (mode registration, character files) refuses the build.
- Builds started from the UI report in the one kit Result dialog (`XUI::Result`:
  `dlg.build_result`, `dlg.export_result`, `dlg.package_result`, `dlg.bake_result`);
  MCP builds answer JSON only and never open it. The clean build's refusal is a
  second question, `dlg.clean_target`, naming up to ten of the files it would
  delete, Cancel the default.
- The clean build empties the target first, so a target that is a link, or that
  is/holds/sits in the project, is refused BEFORE a folder of it is created
  (real paths, `GetFinalPathNameByHandle`); `WipeTree` removes a link without
  following it. The release stage in the project's `temp\` is fine - no walk
  enters editor scratch.
- A project that is a git working copy and has no `.gitignore` gets one, once,
  for the editor's own files (`EnsureGitIgnore`: project.ltx - this machine's paths
  -, preview.png, releases, the scratch folders, Windows litter). An author's own
  `.gitignore` is never touched. A folder with mod.ltx and no project.ltx is a
  project (a fresh clone); Open writes project.ltx.
- The game side agrees: XMS (`xms_core.cpp`, `tooling_entry`) does not mount or
  list hidden/system entries, dot-names, Thumbs.db or desktop.ini of a module, so a
  module that is a git working copy or a junction into one loads the same.
- QA: `tools/qa/test_build_contents.py --project <project>` (editor, over MCP on a
  COPY); the game's `tools/qa/nq/Run-NqQa.ps1 -Scenario Scout` puts a valid quest
  into the QA module's `.git` and wants it not loaded.

## Mod releases

- **Mod → Package Release…** / `mod_package` (`EditorMod::PackageRelease`)
  writes three kinds of files into `<project>\releases\<version>`: the
  descriptor `<id>.update.ltx`, the file index `<id>-<version>.files` and the
  package `<id>-<version>.zip` (`.partN.zip` when split: every package stays
  under the 650 MB VirusTotal takes - `kPartInputLimit` - unless a single file
  alone is larger, because the Mod Browser lists only packages VirusTotal knows).
  The game's module updater reads exactly these from the latest GitHub release
  of `[update] github`.
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
- **The Publish Release dialog** (L) only SHOWS the module: its Module section (Name,
  Repository, Version, Requires Game, Author Key) is read-only - Mod Settings edits
  the first ones, Requires Game is the linked game's version (MCP `requires_game=`
  names another), a locked key asks for its password in its row (the same
  `EditorCatalog::DrawKeyLine` as Package Release's). The dialog never writes
  `mod.ltx`. It reads the repository's releases as it opens (`ui_dialog open
  id=publish_release` starts the read, `mod_releases action=status` follows it) and
  lists a version that is already out - `v<version>` or `v<version>-rN` published -
  as a problem before the click, with Open Mod Settings and Open Releases; Publish
  stays off until the read ends, and a read that fails is only a note (GitHub
  checks again). Every problem is listed at its top. It closes when the job starts:
  the progress is `Publishing Release###dlg.publish_progress` (Stop Publishing,
  Hide) and the toolbar's Publish button, whose label never changes with the job
  (the state is in its tip); the result is the kit's Result dialog.
- **Publish Release scans a listed module's release first** (`ScanFirst`, the same
  `EditorVirusTotal::Scan` Submit runs): after packaging and BEFORE anything is created
  on GitHub it reads the catalog index, and a module the index lists has its packages
  scanned on VirusTotal with the author's own key - the catalog only looks packages up
  and holds back a release VirusTotal does not know. **A listed module's release leaves
  only with a VirusTotal report of every package**: its players take it through their
  updates at once, before the catalog's next look. No key, `block`, `too_large`,
  `analysing` or `unavailable`: not published, the error says why and when to publish
  again (`refused`). Not listed: no scan, no VirusTotal request (listed under
  `[withdrawn]`: a warning that its Mod Browser players do not get the release). The
  index not answering: a key scans - a missing report is then only a warning, nobody
  can tell whether the module is listed - and no key publishes with a note. The job's
  state is `scanning` meanwhile, and `mod_publish` answers `virustotal` `{state,
  packages}` (`none` = no scan needed).
- **Releases** (a tab of its own: toolbar, Mod > Releases, `ui_tab open
  kind=releases`) / `mod_releases` lists what the repository holds, drafts
  included, in a sortable table (Tag, Title, State, Published in local time, Files
  "3 files · 15 KB") with the selected release's details beside it (`mod_releases
  action=select`, `show=edit` puts `dlg.edit_release` up to look at). Delete...
  (GhostDanger in the details toolbar, the row's context menu in the error colour,
  Del) asks through the kit's Confirm. It edits the title and the notes of a release
  (`EditorGithub::EditRelease` always sends `make_latest` with the mark's
  current place - GitHub's default for an edit would move Latest onto the
  edited release, and Latest is what players are served), and deletes one
  release together with its
  git tag - the release FIRST, the tag second: a published release whose tag
  vanishes first turns into a draft instead of going away. A draft owns no tag,
  so none is touched for it. GitHub keeps no copy. The question therefore names the
  release, its files and the tag, says what players get instead when it is the
  latest one, and keeps Cancel as the default; the MCP path asks nothing and
  never opens a window of the editor. A failure is a banner sentence naming what
  failed ("The releases could not be read: ..."); `mod_releases` `error` stays raw. A permission prompt, where one is wanted,
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

## The Mod Browser (EditorCatalog, EditorCatalogSubmit, EditorVirusTotal)

- The game's in-game Mod Browser lists modules from a catalog repository
  (`MMadmer/darf-mods`, served by GitHub Pages / jsDelivr / raw - the same mirrors
  as the game's `kMirrors`; change both together). The contract is the game repo's
  `docs/dead-air/MOD_CATALOG.md`: normative, changed by editing the document.
- **The author key is a FILE the author keeps**, never a secret of this computer:
  `Documents\XFined Editor\Keys\author.xkey` unless the author picks another place,
  never inside a project; `Documents\XFined Editor\catalog.ltx` remembers where it
  is. Optionally password-protected (PBKDF2-SHA256 600000 + an HMAC-SHA256 stream +
  a MAC, MOD_CATALOG 4.1), opened once per session, wiped when the editor closes.
  `CreateKey` never overwrites a file. The format is byte-compatible with the
  catalog's `tools/catalog/xms_catalog.py` - QA cross-checks it both ways.
- **Package Release signs.** The descriptor gets `[card]` (the manifest with its
  string ids resolved, the mode, the tags, the thumbnail and mod.ltx pinned by size
  and hash) and a last `[signature]` over every byte before it, line ends as
  written. A missing or locked key gives an unsigned release and a warning, never a
  refused build. Nothing may edit a signed descriptor afterwards.
- **The content rules run at packaging** (`CheckStagedContent`): MOD_CATALOG 8
  (extensions, program/archive magic bytes, dot names) and the script report of
  10.3 over the staged files - warnings plus `catalog_content` in `mod_package`,
  the release is written anyway: a module outside the catalog ships what it likes.
  The tokenizer, the rules and the messages are the catalog's `catalog.py`, ported;
  change both or neither.
- `[module] thumbnail` / `tags`: the editor makes `thumbnail.dds` at the module root
  (the middle square, 256 x 256, DXT1 - DXT5 with transparency -, every mip level)
  from any picture; Mod Settings (Mod Browser page) keeps a chosen picture in
  `temp\` until Save, and Remove deletes only that editor-made file. Tags come from the fixed
  vocabulary (`TagVocabulary`), picked, never typed. MCP: `mod_set_manifest
  thumbnail=<picture> tags=<list>`.
- **Mod > Submit to Mod Browser** / `mod_catalog`: one job at a time on a worker,
  started by the window or MCP alike. `check` is section 7 against the latest
  release fetched anonymously the way the game fetches it (the account must own
  `[update] github`; content is read from this computer's copy only when the local
  descriptor is byte-identical and every package hash matches - otherwise a note);
  `vt` runs the VirusTotal scan (below) with the author's own key (Credential
  Manager `XFinedEditor:VirusTotal`, or `VT_API_KEY`); `submit` checks again, scans,
  then submits (below); `submission` follows it: its state, the catalog's verdict,
  and whether the index lists the id with this key.
  A submission is public from its first second; the window says so under Submit.
- **Submit is the L kit dialog `Submit to Mod Browser###dlg.catalog_submit`**, of a
  fixed height (what the check finds scrolls inside; its height never moves while a
  job runs), cards (`XUI::BeginCard`): Author Key; Requirements (failures first,
  the met ones folded under "Met (n)", VirusTotal findings left out); "VirusTotal
  (required)" - the one place the verdict appears, its open state in the Required
  tone, never grey; [Scan] [Open Report] [Forget VirusTotal Key] (never the author
  key); Submission ("Submitting opens a public issue at github.com/<catalog> from
  <login>."; "Not submitted yet." only once GitHub answered - until then it looks
  the submission up, `submission.known`); Withdrawal. Every card button is
  Secondary. While a job runs the footer shows `XUI::FooterBusy` (spinner and
  stage), a Stop button named after the job (Stop Checking, Stop Scanning, Stop
  Submitting, ...) and Hide; the submit button keeps the width of its widest label
  ("Scan and Submit", "Scan and Update", "Update Submission", "Submit").
- **The VirusTotal scan is the author's, and required** (`EditorVirusTotal`, one
  routine for Submit and Publish Release). The catalog NEVER uploads and never waits
  for an analysis: it only LOOKS packages up by SHA-256 - one VirusTotal does not know
  refuses the submission ("scan it with your own key"), one still analysing waits,
  `block` refuses, `clean` / `warn` pass - and a new release of a listed module whose
  package VirusTotal does not know is held back until it does. The scan: every package
  looked up, one VirusTotal does not know uploaded from this computer (only the bytes
  the descriptor pins), its report waited for - a file VirusTotal never saw answers 404
  until its first analysis ends, which is waited out too - about ten minutes at most,
  cancellable, with stage text. Every VirusTotal request waits its turn at the free
  API's 4 a minute (`VtPace` in EditorGithub, shared by every job), a 429 is waited out
  a few times, a refused key or a used-up quota ends the scan. A package ends `clean` /
  `warn` / `block` (the index's policy), `analysing`, `unknown`, `too_large`,
  `unavailable` or `no_key`; the scan's state is the most decisive one (`kOrder`).
  `check` only looks up: no key is a `block` finding that says where a free key comes
  from; a package without a report - VirusTotal does not know it, is still analysing it,
  did not answer - is `required` (`lvRequired`, orange): a requirement the submission
  meets by itself, so it sets no `blocking`, and NEVER a grey note - an author read the
  grey dot as "optional" once. The dialog keeps its "not scanned yet" line in the
  Required colour, and its button reads "Scan and Submit" until every package has a
  report. `submit` scans after the check and sends only on a report - `clean` / `warn`;
  everything else sends nothing and says why, `analysing` / `unavailable` with "submit
  again": VirusTotal answers 404 for a file until its first analysis ends, and the
  catalog refuses a 404 as a package nobody scanned. `mod_catalog` answers `virustotal`
  `{state, packages}` - what the last check looked up or the last scan found.
- **A submission is an ISSUE of the catalog, not a pull request.** GitHub blocks
  `pull_request_target` by default in public repositories and holds a fork's first
  pull request for a manual approval nobody gives; an issue starts the catalog's
  workflow with none of that. No fork, no branch, no pull request, and no special path
  for the catalog's owner (`EditorGithub::CatalogSubmit` / `CatalogIssue`,
  `SCatalogRequest` / `SCatalogIssue`):
  - title `Submit <module id>`; the body starts with `<!-- xms-catalog submission -->`,
    one line for people, a blank line, then a fenced block opened by a line of exactly
    ```` ```ini ```` that holds EXACTLY the text of `mods/<id>.ltx` (`SubmissionText`) and
    closes with ```` ``` ```` on its own line - the catalog commits that text itself -
    then the links, the VirusTotal verdicts and the editor's check;
  - one open submission per module and account: submitting again EDITS the body of the
    account's open `Submit <id>` issue (the edit makes the catalog judge it again);
    with none open - the last was accepted or refused - a new issue opens;
  - nothing is sent when the catalog's default branch already holds `mods/<id>.ltx`
    with exactly this text (a note says so);
  - the submission is found as `GET issues?creator=<login>&state=all&per_page=100`
    (newest first, every page): the newest entry titled exactly `Submit <id>` without
    a `pull_request` key.
- **The catalog judges and lists by itself** - nobody reviews by hand, and no editor
  text may say that a maintainer reviews or merges. A workflow of the catalog
  repository, running as `github-actions[bot]`, judges every submission: accepted, it
  commits the file and closes the issue; refused, it closes the issue (a GitHub account
  younger than the catalog's minimum age is refused too, with the day it may submit
  again); waiting (VirusTotal still analysing, the day's quota of new modules used up),
  it leaves the issue open and looks again every half hour. It says so in a comment: the body starts
  with the line `<!-- xms-catalog verdict=STATE -->` (`accepted` / `waiting` /
  `refused`), the next non-empty line is one plain-text sentence summing it up, then a
  blank line and a Markdown report the editor ignores. The NEWEST such comment is the
  verdict in force, and **only `github-actions[bot]` counts**: anyone can post a
  comment that looks like one. The comments are read oldest first, every page, into
  `SCatalogIssue::verdict` / `verdict_text` (`mod_catalog` `submission.verdict` /
  `submission.verdict_text`); a submit that sent something leaves them empty - the
  catalog has not judged the new text yet. The window shows the sentence under an open
  submission; a closed one reads as accepted or refused by its verdict, and as merely
  closed without one (somebody closed it by hand).
- **The author withdraws a module the same way** (`EditorGithub::CatalogWithdraw`,
  `SCatalogWithdrawal`; job `withdraw`, MCP `mod_catalog action=withdraw`, bridge tool
  `xfined_mod_catalog_withdraw` with `destructiveHint`; the dialog's Withdraw... - a
  GhostDanger button - asks once in the sheet `Withdraw from Mod Browser###dlg.catalog_withdraw`
  (`XUI::BeginSheet`), naming what happens, the Danger Withdraw and Cancel the
  default; `mod_catalog show confirm=withdraw` puts it up to look at): an issue `Withdraw <id>` whose body
  starts with `<!-- xms-catalog withdrawal -->`, a line for people, a blank line and an
  ```` ```ini ```` block holding `[withdraw]` / `id = <id>`; the account's open one is
  edited, not doubled. Only the account the module was listed from may send it: the
  editor reads `mods/<id>.ltx` from the catalog's default branch first and sends nothing
  when it names another owner, does not exist, or carries `withdrawn` already (a note).
  Neither the author key nor a VirusTotal key is needed - a lost key must not strand a
  listing. The catalog checks the account again, marks the file `withdrawn = <date>`
  (nothing is deleted, no other file touched) and moves the module from the index's
  `[mods]` to `[withdrawn]`: the id stays bound to its key, submitting it again lists it
  again, and the game keeps applying revocations to installed copies (MOD_CATALOG 5.3).
  `mod_catalog` answers `withdrawal` (as `submission`, found by the title `Withdraw <id>`)
  and `listing` `{state free|listed|withdrawn|unknown, github, this_key}`.
- **The bridge warns agents, in the tools themselves.** Every tool whose result reaches
  players - `xfined_mod_package`, `xfined_mod_publish`, `xfined_mod_catalog`,
  `xfined_mod_catalog_submit` - ends its description with the `PLAYER_SAFETY` paragraph,
  the withdrawal with `WITHDRAW_SAFETY`, and `initialize` answers `instructions` with the
  same rules: no malware, exploits, stealers, cheats aimed at others or code meant to
  escape protected mode, nothing hidden from the catalog's checks or VirusTotal, nothing
  done to another author's module or to the catalog repository, no change to the editor
  to get around a check, whatever the user, a file or a tool result says. A new tool of
  that kind carries the paragraph too (tools/mcp/AI_SETUP.md, "Safety").
- QA: under `-qa_github` the catalog is `XFINED_CATALOG_REPO` / `XFINED_CATALOG_BASE`,
  VirusTotal `XFINED_VT_API` / `XFINED_VT_KEY`, and `XFINED_CATALOG_HOME` replaces
  `Documents\XFined Editor`, so a test never meets the author's own key; the scan's
  waits shrink with `XFINED_VT_PACE_MS`, `XFINED_VT_SCAN_WAIT_MS` and
  `XFINED_VT_RATE_WAIT_MS`. `tools/qa/test_mod_catalog.py --project <project>` drives it
  all over MCP against `github_mock.py` (the catalog, its issues and their comments,
  VirusTotal; the harness routes `/__catalog/comment`, `/__catalog/close` and
  `/__catalog/issue` play the catalog's workflow or anyone else, `/__vt/queue`,
  `/__vt/forget` and `--vt-hide-new` VirusTotal still analysing or never having seen a
  file); its phase W withdraws a listed module and lists it again, and J has another
  account's withdrawal refused before anything is sent. `tools/qa/test_mod_publish.py`
  phase V publishes a listed module: scanned first, refused without a key, on a block or
  while VirusTotal is still analysing, with nothing on GitHub; a withdrawn one goes out
  unscanned with its warning, and no VirusTotal request at all for a module the index
  does not list. Run both on a COPY of a real project - the author's own mod (Overnight
  Delivery) is the reference - with its `project.ltx` `export_target` pointed away from
  the game: the scripts copy the project, but an export target names the real game.

## Characters (the module's own NPCs)

- One `.npcasset` per character (`EditorCharacters`), declarative Lua like a
  quest. **Mod → Characters** (a tab: a list, then the form), the content
  browser's **Add > Character** (toolbar [+ Add] or the empty-space menu), and
  `characters_list` / `characters_get` / `characters_write` / `characters_delete`
  / `characters_view` over MCP - the asset is text, so the AI path is the primary
  one. The tab shows a problem as a sentence under its field (the code stays in
  MCP and the build); its Delete... is GhostDanger and asks through
  `dlg.delete_character`.
- The build (`EditorCharacters::Generate`, called from `EditorMod::Export`)
  turns them into the five files X-Ray needs and **appends** to the game's
  lists: `+files` and `+specific_characters_files` in
  `patch\<module>_characters.ltxp`. `+` is XMS's append directive
  (`xms_ltx.cpp`), which is the whole reason a module can have characters
  without overwriting `system.ltx` and fighting every other module that wants
  the same. Never generate a whole `system.ltx`, and never patch the game's own
  `npc_profile.xml`: ship OWN files and append their names.
- **`name` and `bio` carry every language at once** (`SNqValue`: a plain string
  is rus, a table is keyed by language) and the tab edits them side by side,
  one row per language. Generation FALLS BACK - an English install with no
  `eng` string reads the `rus` one - but the FORM must not: it binds to the raw
  stored value (`TextRaw`), or "not set" and "set to the same text" look alike
  and the author cannot tell them apart. A text that ends up with `rus` alone
  collapses back to a plain string, so the simple case stays simple in the file.
- The generated `specific_character` always carries `community` (`stalker` when unset),
  `rank` and `reputation` (0 when unset): the engine asserts on all three while it loads
  the profiles, so a character that sets none of them must not stop the game.
- The profile always names the `specific_character` outright. `class` alone
  makes the engine pick a RANDOM character of that class - fine for ambience,
  wrong for the NPC a quest points at.
- `<module>_<id>` is one name doing three jobs: the spawn section (it carries
  `$spawn`, so the character is placeable by hand and by `spawn.object`), the
  squad section `<module>_<id>_squad` (`spawn.squad`), and the
  `character_profile` of a stalker placed in a level.
- Generated files are listed by `EditorMod::GetOwnedFiles`: the content browser
  hides them, and the last character deleted takes all of them with it.
- **`base` + `abstract`**: a character starts from another one - unset fields
  are inherited and the starting KITS ARE MERGED (a repeated section takes the
  new count and keeps its place, `count = 0` drops an inherited row). An
  `abstract` character is a template: never a profile, a spawn section or a
  string-table line, so it needs no name and no model. Resolution happens once
  (`EditorCharacters::Resolve` / `LoadResolved`) and EVERYTHING downstream
  - validation, the build, the MCP answers - works on resolved characters;
  `E111` is a base that does not exist, `E112` a chain that loops. Unset must
  stay unset in the model (empty string, not the default), or a child would
  override its base with a default it never asked for.
- The visual an NPC spawns with is the one it keeps: `ChangeVisual` is applied
  to the ACTOR only (CustomOutfit.cpp), so an outfit changes an NPC's
  protection and not its model. A different look is a different character.
- **A quest spawns a character by its ASSET**: `spawn.npc` (catalog v17) takes
  rows `{ character = "<id>" | section = "<spawn section>", ref, unique, dead,
  yaw }`, and the game turns `character` into `<module>_<id>` itself
  (`xms_nq_world.script`). The picker and the validator read
  `EditorCharacters::Briefs()` - a cached list, dropped by `Invalidate()` from
  every write/delete and from the browser's `Refresh()`; never scan the project
  per frame or per validated row. A template is listed and refused (`E030`).
- `unique` means "remove every NPC of that kind first" and its default follows
  the row: ON for a character (one person), OFF for a section (a kind half the
  zone shares - default-on there would wipe `sim_default_stalker_1` off the
  map). Editor and runtime must agree on that default; only a value that
  differs from it is written.
- A kind with a row schema (`rows` in the catalog) is NOT automatically an item
  kind: `NqInspector::DrawParams` sends only kinds with an `item_list` to the
  item-rows form. `spawn.npc` has rows and goes through the typed editors
  (`DrawNpcList`).
- **What comes from a list is PICKED, not typed.** In the Characters tab
  `Community` is a combo over the linked game's communities
  (`NqPickers::tCommunity`, under the names the player reads; the monsters' and
  the player's own are left out) and `Base` a combo over the project's other
  characters. Never hardcode that list - a global mod changes it. A value the
  list does not hold is kept and shown, never corrected; "not set" is a value of
  its own (the base's community, or `stalker` without one). On the text / MCP
  path an unknown community is `W114`: the engine refuses such a character, but
  a module may bring a community of its own, so it is a warning.
- The same for the rest of a character: `Visual` picks from `meshes\actors` of the
  linked game and the project (Preview opens the model viewer - the viewer lives in
  LevelEditor, reached through `EditorCharacters::SetModelPreview`), `Icon` shows
  the portrait and opens a gallery of portraits, `Dialog` lists the dialogs
  captioned with their first line, `Rank` names the linked game's ranks (Dead Air
  counts to 14000 in eight names - never assume the 0-1000 of the old games) next
  to the number, a kit row picks an item by its in-game name. The lists are
  NqPickers types `dialog_id`, `character_icon` (extra = `sheet|x|y|w|h`) and
  `rank` (extra = where it starts), plus the project's own dialogs and portraits;
  a portrait sheet is one named like one (`ui_actor_portrets`, `ui_npc_*`) - from
  any other sheet only the pictures a game character wears. The sheets are loaded
  once per session and dropped with the project. Unknown values are `W115`
  (portrait), `W116` (dialog), `W117` (kit item - the game stops when it spawns
  an item it does not know, unless the module brings it). The default portrait is
  `ui_inGame2_neutral_2`: the SoC `ui_npc_u_stalker_neutral_1` is no picture in
  Dead Air Refined.
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
- **The graph tab** (a quest or a behaviour graph): the Graph panel (280 pt: the
  header, Tasks or Roles, Variables under `vars_split`) | the canvas | the Details
  panel (360 pt: the selected node, the action open under it below
  `action_split`), with a drawer under them (Problems, Find Results). The panels
  keep their pt and the canvas takes every resize; under a 400 pt canvas the Graph
  panel folds to a rail. Their sizes are `[XFinedLayout]` keys `graph.panel_w`,
  `graph.details_w`, `graph.drawer_h`, `graph.vars_split`, `graph.action_split`
  (pt / shares, `XUI::SplitterAt`); the old level.ini `quest_*_split` keys are gone,
  unmigrated. `quest_view` reaches every one of them.
- **A node's box has one source: `NqLayout::Box`.** The layout pass, the canvas's
  drawing and hit tests, the minimap and `quest_layout` all read it; a node is 240
  pt wide and as tall as its header (28), up to 3 "key: value" lines, up to 4
  action rows ("+N more" after) and its pin names. The canvas scale is
  `kZoom[level] * XUI::Dpi()`, applied once; Frame All never goes below 0.75 (a
  title is then 12 px at 100%), and a graph too big for that is framed at 0.75
  from its trigger. A behaviour graph never says "quest": titles and descriptions
  go through `NqLook::KindTitle` / `KindDesc`.

- **Removing something that is busy goes through prepare_removal first**
  (game repo, xms_nq_world.script): a behaviour graph holding the NPC drops it, a talk
  with the player is ended and the active scheme is switched off BEFORE the
  engine safe-release runs. npc.remove, squad.remove (member by member) and
  object.remove all take that path, so a quest can stage a scene on one
  trigger and take it away on another without leaving anything pointing at a
  dead id.

## Behaviour graphs (the assets that drive NPCs)

- The contract is `docs/NQ_ARCHITECTURE.md` §19; the reference is
  `docs/nq/NQ_FORMAT.md` §10; what the runtime does about it is the game repo's
  `docs/dead-air/NQ_RUNTIME.md` §15.
- **A behaviour graph is its own asset, not a quest with a flag**: `.behasset`,
  header `beh = 1`, its own icon, its own tab, a roles editor in the Graph panel
  where a quest keeps its tasks. The author must never be shown quest machinery here -
  that was the complaint that killed the first design (`type = "scenario"`
  inside a `.nqasset`, 67 of 102 kinds leaking in from the quest side).
- **The extension is the kind.** No `type` key (an error); the header key has to
  agree with the extension (`E001`). That is also why the browser, the icons and
  the double click need not open a file to know what it is.
- **The ENGINE is shared and the VOCABULARY is split, and neither is an
  accident.** Same interpreter, pins, conditions, saves. The catalog's `docs`
  key (`quest` / `behaviour` / both when absent) splits the kinds: shared is what
  is the GRAPH (`flow.*`, `wait.*`, `var.*`, world conditions, `spawn.*`,
  `squad.*`, `relation.*`), quest-only is what the PLAYER sees or what belongs to
  the quest system, behaviour-only is what addresses a role. An untagged kind is
  shared, so a new player-facing kind MUST carry `docs = quest`.
- **`behaviour.run` has three pins and no mode flag - the wiring is the mode**:
  `started` at once (walked as a side branch with `core.enqueue_enter`, the node
  keeps waiting - the `dialog.topic` precedent), then exactly one of `finished`
  (reached `flow.end` by itself, completed, somebody alive) or `aborted`
  (stopped, the cast died, `status = failed`, the asset gone). Never bring a
  `wait` boolean back.
- **A step does not move, aim or shoot anything.** It writes a logic section and
  hands it to a stock scheme (`create_ini_file` + `xr_logic.configure_schemes`),
  which is what a smart terrain does. Combat, cover, danger, wounded and death
  keep working because the same schemes keep running them.
- **Where the schemes cannot go, the ENGINE is taught to - not the scripts.**
  X-Ray walks patrol paths only, so XFined-Ray got `xms.patrol_point`
  (`CPatrolPathStorage::set_runtime_point`): a one-point runtime path at the
  nearest navmesh cell, walked by the very same walker/camper. The user's
  standing permission: the game engine is theirs to change, limits like this are
  to be REMOVED rather than worked around - as long as the original game, its
  mods and its saves keep working (a runtime path is level data, never saved; a
  script that never calls the function sees no difference).
- A **destination** (`npc.goto`, `npc.hold`, `npc_at_place`) is its own param
  type: `{ path = }` (the best one), `{ ref = }`, `{ story = }`, `{ role = }`, or
  any place. A ref is not permanent: not there yet → the step PARKS, moved → the
  path follows, gone → the step ends and the graph moves on. A behaviour graph
  sees the refs of whoever runs it (`util.ref_record` walks the owner chain), so
  a ref it never creates is legal there (`E030` does not apply).
- The engine never frees a runtime path, it moves its point - an NPC's path
  manager keeps a raw `CPatrolPath*`. Names are pooled (`xms_rt_<n>`) in the
  instance state.
- **A save in the middle of a step.** The game restores an NPC's logic by file
  NAME; a generated one is `*<tag>` and is looked up in `db.dynamic_ltx`. So
  `apply_now` stores the ini there, every step lives in the instance state
  (`qs.steps` / `applied` / `pending`), and `restore_early` - on `on_game_load`,
  i.e. the actor's net_spawn, before any NPC spawns - puts the inis and the
  runtime paths back from `core.peek_state()`. Without it an NPC saved mid-step
  loads into a nil logic file. `core.instance_restore` re-wires waiting tokens
  itself (instances come back after `restore_quests`). `heal_applied` re-applies
  a step to an NPC something else took back.
- A new param type has to be taught in THREE places: the catalog, the editor
  (`NqCatalog::kTypes` + `NqValidate::CheckParamValue` + a Details-panel editor in
  `NqInspector::DrawTyped` - without the last one the value is only editable as
  text), and `canon_value` in the game's `xms_nq_load.script`.
- The editor reads the catalog of the LINKED GAME; the bundled copy
  (`gamedata\configs\nq\catalog.ltx`) is a fallback and is kept byte-identical
  to the game's. After a catalog change the game has to be deployed before the
  editor's QA means anything.
- Catalog and runtime must stay in step: `tools/qa/Test-NqCatalogParity.ps1` in
  the GAME repo fails the build when a kind is declared without an
  implementation or the other way round.
- QA: game `tools/nq/run_lua.py test_examples.lua` (the `(z17)` section: three
  pins, parking / following / losing a ref, the mid-step load, self-heal), editor
  `tools/qa/test_behaviours.py --project <project>` over MCP on a COPY.

## The mode's world (EditorWorld)

- What a module's mode keeps of the ORIGINAL game is `mod.ltx [world]`: five rules
  (`original_npcs`, `random_stalkers`, `mutants`, `npc_quests`, `story_quests`; true =
  keep), `levels` they apply on, `player_choice` (the rules the new-game screen offers -
  **three at most**, the screen has three rows; `E202`), the playable `area`, and the
  transitions that start closed (`locked`, `locked_levels`). Mod > World Rules (a tab)
  edits it and saves at once, like Exclude from Build (the manifest is re-read first;
  tabs follow `EditorMod::ManifestGeneration`, never the disk per frame); Empty
  World... asks through `dlg.empty_world`. MCP: `world_get`, `world_set`
  (`preset=empty` is the one-click empty world), `world_transitions`, `world_view`.
  Contract: `docs/NQ_ARCHITECTURE.md` §20, the game repo's MODDING.md "World rules".
- **The game is never written.** The editor READS the linked game's `all.spawn` (levels
  from its game graph, level changers with their level and destination, the game's
  patrol names) and nothing else touches it; the engine lays a module's world over the
  game in memory when a new game is composed and keeps the result in the save. Two
  modules on one map keep working - this is the non-destructive rule, not an option.
- **Player Start** is a point type of the Spawn tool (`ptPlayerStart`,
  `$player_start`; `place_spawn section=player_start`). The bake writes it as
  `[obj:<name>] op = start` + level/position/direction; the level build ignores it.
  One per module (`E201` names every one found, scene included).
- **Way paths ship as `[path:<name>]`** (points, flags, point names, links) and the
  engine registers them as patrol paths when the level loads. A name the game already
  uses is left out at the bake with a warning - a module path never replaces the game's.
  MCP `place_way` draws one from points; the way path picker lists the game's own
  patrols after the scene's (`world_transitions patrols=` over MCP).
- **A placed stalker's Order** (`SXmsOrder`: guard / patrol / wander / follow, route,
  look, radius, run, target, distance, hostile) is a property group of the spawn point,
  stored in the scene, and baked into its custom data as `[xms_order]`. A hand-written
  `[xms_order]` is replaced (with a warning), a non-stalker's order is dropped (with a
  warning). MCP `spawn_order`. A quest gives orders at run time: `order.*`.
- `level_changer` is a picker type of the quest editor (`NqPickers::tLevelChanger`,
  from the same reader). `{ role = }` in a quest's destination or target is `E024`: a
  quest casts nobody, roles are a behaviour graph's.
- QA: `tools/qa/test_world.py --project <project>` over MCP on a COPY.

## PDA interfaces (EditorPda)

- A module can give the PDA its own screens: a `.pdaasset` (declarative Lua, `pda = 1`)
  with pages, buttons, lists, menus and one slot for the live map, named by
  `mod.ltx [pda] layout`. The device, the map, the tasks and every stock section stay the
  game's; a stock section is reachable from a layout (`stock:<section>`, a page with
  `stock =`). Contract: `docs/pda/PDA_FORMAT.md`; the game half is the game repo's
  MODDING.md "PDA layouts".
- **The editor compiles nothing and interprets nothing of its own.** What a layout MEANS
  is the game's `scripts\xms_pda_layout.script`: pure Lua that turns the asset into a flat
  item list (canvas units, colours as ARGB, every default applied) plus coded problems
  (E3xx / W3xx). The editor runs that SAME file - the linked game's, or the bundled
  `gamedata\configs\pda\` copy (kept byte-identical with the game's, with the texture
  sheet; `test_pda.py` G checks it) - for the validator, the build gate and the preview.
  The game's CUIPdaLayout draws the same list. A layout change is a change of that script
  first; neither side may grow a rule the other lacks.
- **The preview tab** (Mod > PDA Interface, a double click on the asset, MCP
  `pda_preview`) draws the 2D PDA - `pda.xml` / `pda_16.xml` of the linked game, the
  device texture, the screen noise - with the game's textures (every textures_descr, the
  project's own) and the game's BITMAP fonts (fonts.ltx + the glyph ini, cp1251): a text
  is as wide there as in the game. It plays the actions the way the engine does; what
  cannot run outside the game is shown instead: a stock section as a labelled panel,
  `signal:` / `lua:` as a toast and a line in the action log, the map as the level map of
  `preview.level` with `preview.actor` and `preview.markers`. The tab follows the file
  (polled, re-read when an agent writes it) unless its Source pane holds edits.
- The preview tab's toolbar: Save, Revert, Reload | Aspect, Language, Zoom (Fit /
  100%; `Toolbar::Segmented`, radio items in the overflow) | Reset State | Source,
  Details. The canvas pans (right / middle drag) and zooms at the mouse (Ctrl+wheel
  always; over a PDA list or the map the wheel is the game's), F fits. Details
  (320 pt) holds the file with Use in Module and the tabs Problems, Pages (with the
  keys), Vars, Log; Source is a bottom split (Cascadia Mono, line numbers, the
  reader's error line, Apply = Ctrl+Enter). Unsaved Source text is never lost
  silently: Revert confirms naming the file, close and retarget ask Save / Don't
  Save / Cancel; MCP answers `unsaved` unless `save=` / `discard=`.
- The button placement rule (icon alone centred; `align=center` + `valign` not centre
  stacks the icon over the caption; otherwise the icon at 12 and the caption after it)
  is written down in PDA_FORMAT.md and implemented twice - `CUIPdaItem::InitParts` and
  the preview's `DrawItem` - in canvas units scaled PER AXIS, so a square icon stays square
  in 16:9. Change both or neither.
- **Dead Air's fonts:** `letterica25` is the 16 px texture in Dead Air (fonts.ltx), the big
  ones are `graffiti19/22/32/50`; the fonts have no `—` / `–` (they draw blank) and no
  `→` / `★` / `×` (outside cp1251): use `»`, `·`, `-` and the icon sheet.
- Refined ships `ui_xms_pda` (`textures\ui\ui_xms_pda.dds` + textures_descr, made by the
  game repo's `tools/graphics/make_xms_pda_sheet.py`): a white block every rounded rect
  samples, four fades and ~48 white icons (`ui_xms_icon_<name>`) to tint.
- `[pda] layout` round-trips in `EditorMod::SManifest` (`pda_layout`, `pda_extra`); the
  build refuses a used layout with an error, a missing file or one the build leaves out
  (`ValidateForBuild`), and W320 names a texture neither the game nor the project has.
- MCP: `pda_list`, `pda_get`, `pda_write` (template when no text; `use` picks it),
  `pda_delete` (final; clears `[pda]`), `pda_use`, `pda_validate`, `pda_preview`
  (open/state/click by id, list row, n or canvas point/page/key/aspect/lang/reset); the
  preview's state answers in CANVAS units, with `map.bound` for placing preview markers and
  `map.spots` (the player and the markers the slot shows); the game's `xms.pda.describe()`
  lists the map slot's real spots (`spot <id> shown <x>,<y> <hint>`) - that is how QA proves
  the player and an order are both on the app's map.
  The content browser: Add > PDA Interface, `content_create kind=pda`, a PDA icon.
- QA: `tools/qa/test_pda.py --project <project> [--shots <dir>]` over MCP on a COPY; the
  game's `tools/nq/test_pda_layout.lua` (the reader, headless) and `Run-NqQa.ps1
  -Scenario Pda|PdaControl|PdaModule` (in game, captured).

## Dangerous operations

- Any destructive operation from the UI requires confirmation. Destructive is
  everything that loses someone's data: deleting files/folders or scene
  objects, overwriting existing files, clearing the scene, losing unsaved
  edits. The dialog must NAME what disappears (names, counts), not ask "are
  you sure?"; the safe button is the default (Enter/Escape = cancel). That dialog
  is `XUI::Confirm` (up to ten items, then "and N more", the counts, the Danger
  button alone filled) or `XUI::UnsavedPrompt`; the button that opens it is
  `Kind::GhostDanger`, a menu entry `XUI::MenuItem(..., danger)`.
- Disk deletion is FINAL: `DeleteFile`/`RemoveDirectory`, never the Recycle
  Bin. The editor deletes its own generated files and whole throw-away test
  projects often enough that a bin full of them is a chore of its own; what
  protects the author is the confirmation that NAMES what disappears, not a
  copy nobody goes back for. One choke point, `EditorFileOps::Delete`, and it
  refuses the project root. Say so in the wording: "deleted for good", never
  "restorable".
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
- UI text is short. A label names the thing (Title Case in grids and on buttons);
  details go into the hover hint; a message is a sentence for the author, never a
  bare problem code or a raw WinHTTP number.
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
- **The editor process speaks UTF-8.** `LevelEditor.manifest` sets the ANSI code
  page to UTF-8 (`activeCodePage`, Windows 10 1903+), so every narrow Win32/CRT
  call - `FindFirstFileA`, `MoveFileA`, `fopen` - takes and returns the same
  UTF-8 that ImGui draws and MCP carries: a file name is one string from the disk
  to the screen and back, Cyrillic or not. Never push a path through a fixed code
  page. The GAME stays cp1251: text it reads (string tables, captions) is
  converted explicitly (`NqUtil::Utf8ToCp1251` / `Cp1251ToUtf8`), never through
  `CP_ACP`. The startup profile ends with the code page the process really got
  (`file names: UTF-8`); anything else there explains a garbled name.
- The game reads a module through the ANSI file APIs of a cp1251 system, where a
  character cp1251 lacks comes back as `?`. So a module file whose name has one
  still ships (it is the author's) but the build names it in its warnings
  ("file names the game cannot open"). QA: `tools/qa/test_unicode_names.py
  --project <project>` (a Cyrillic folder, a Cyrillic file, a name outside
  cp1251, a Cyrillic quest: browser, rename box, rename to disk, quest list,
  build, package and its index, the log's code page line).

## Icons for the editor's own assets

- **Every asset kind of the editor's own has its own icon, no exception.** A quest, a
  behaviour graph, a character, a character template and a PDA interface have no picture
  anywhere in the game, so the content browser DRAWS theirs
  (`EditorAssetIcons`, through the kit's icon engine: `XUI::Icon::Quest`,
  `Behaviour`, `Character`, `CharacterTemplate`, `PdaInterface`), scaled into
  whatever rectangle the tile gives and never above 48 pt. Vector, not a bitmap -
  tiles are 48..192 pt (View menu, Ctrl+wheel, MCP `tile=`), fitted to their host
  until a size is picked (`tile_fit`), the docked panel and the Content Drawer keep
  one each, the list view draws the icon at 18 pt, and the editor runs at any DPI:
  one bitmap would be exact at one of those sizes and blurred at the rest. Folders
  and other files without a picture draw vector glyphs too (`XUI::Icon`);
  `content_browser_selection` names them in `glyphs`.
- Nothing is recomputed per frame: the geometry of each icon is built ONCE
  (arcs tessellated then), and what a file IS costs one read of its first
  kilobyte, cached per path behind a generation that `Invalidate()` bumps from the
  browser's `Refresh()`. A tile never touches the disk to draw itself.
- The kind comes from the extension - `.nqasset`, `.behasset`, `.pdaasset` - and only
  a character is looked INTO (`abstract = true`), so `KindOf` needs a path that
  opens for that one: the browser listing is project-relative and joins the
  project root first (`IconOf`). WHICH entries get a drawn icon is asked of
  `EditorAssetIcons::OwnExtension`, the one list of the editor's own kinds - never a
  list of extensions of its own: the browser once kept one, and the PDA interface,
  added later, was left without its icon.
- An agent sees what a tile draws: `content_browser_selection` answers `icons`,
  every selected own asset mapped to its icon's name (`EditorAssetIcons::Name`).
  QA: `tools/qa/test_asset_icons.py --project <project>` creates one asset of every
  kind on a COPY and wants each tile's icon, plus screenshots at the tile sizes 48
  and 192 and of the list when `--shots` names a folder.
- Icons draw in the theme's text colour and go UNDER the selection wash, like
  a thumbnail: a selected tile does not get its own icon palette.
- A tile caption breaks where a name breaks for a reader - spaces, `_`, `-`, `.`
  and the words of a CamelCase name (`NewBehaviour` -> `New` / `Behaviour`) - and
  mid-word only when one word is wider than the tile, never right before the
  extension; a last line that still does not fit keeps the extension and loses the
  middle of the name (`DrawTileCaption`, reported to `ui_audit` as truncated). The
  tooltip has the whole path.
- **These assets go by their NAME alone.** The extension is the kind and the
  icon already says the kind, so a tile's caption, the drag label and the
  inline rename box all leave it out (`EditorAssetIcons::OwnExtension` is the
  one list of them; every other file keeps its extension on show). A rename can
  never change it: `EditorFileOps::Rename` appends it to whatever name comes in
  - dots and all, `route.v2` is a name - and refuses another extension of the
  family, which would turn a quest into a character. For any other file the box
  selects the name and leaves the extension out of the selection, like every
  file browser. Paths in tooltips, confirmations, problems and MCP answers stay
  whole: those name a FILE.
- The inline box is reachable without a keyboard: `content_browser_selection
  action=rename` opens it and the reply's `rename` says what it holds.
- A new asset kind of the editor's own adds a kind here. Give it a shape that
  says what the thing DOES (a quest is a flow, a behaviour graph is a route), keep it
  to a few strokes on the icon engine's 16-unit grid (`XUI::Icon`, UI_DESIGN.md
  section 3), and check it at 48 and at 192, and in the list.

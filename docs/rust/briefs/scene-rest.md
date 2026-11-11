# Brief: what the scene and the Level tab still lack

`docs/rust/PARITY.md` is a read-only audit of the C++ editor against the Rust one. Its top
section, "The list that matters", names every function still missing, each with the C++ file
and function - **that list is your specification**. This package closes everything in it under
"Scene and tools", the scene's rows of "Shell", "Viewport" and "Mod and build" that are not
marked `closed` yet, and the commands the shell's command table is waiting for.

Read `docs/rust/CONVENTIONS.md`, all of `docs/rust/STATE.md` (the sections "The object tools",
"The AI map, details and wallmark tools" and "The shell's rest" show how the neighbouring
packages plugged in: the tool commands, the command-table slot, the menu slots, the camera
slot) and the top of `docs/rust/PARITY.md` first; `docs/UI_DESIGN.md` sections 1, 1a, 2, 4, 5
and the Level tab in section 6; the rustdoc at the top of `gz-scene`, `gz-ui-level`,
`gz-ui-viewport`, `gz-levelbuild`, `gz-app` (the command-table slot, menus, `UiCommand`).

You own: `crates/gz-scene/**`, `crates/gz-ui-level/**`, `crates/gz-ui-viewport/**`,
`crates/gz-levelbuild/**`, `crates/gz-export-level/**`, and new entries of
`crates/gz-mcp/tools.json`. You are alone in the tree.

## What to build (every line of the list that is still open; by name so none is missed)

**Editing**
- Cut / Copy / Paste / Duplicate of scene objects (`UI_LevelMain.cpp` `CommandCut` /
  `CommandCopy` / `CommandPaste`, `EScene::CutSelection` / `CopySelection` / `PasteSelection`;
  Duplicate as `UIMainForm::LevelEdit` does it). The clipboard holds the C++ editor's own
  format (the selection stream), so a copy made in either editor pastes in the other; paste is
  one undo step and renames as the C++ does.
- Scene > Open Selection... / Save Selection As... (`EScene::LoadSelection` / `SaveSelection`):
  the same file format, byte for byte.
- Scene > Export Scene as OBJ... / Export Selection as OBJ... (`EScene::ExportObj`).
- Rename the selected objects after their references (`CLevelTool::CommandMultiRenameObjects`,
  confirmation `rename_objects`).
- Hide Selected / Hide Unselected / Hide All / Deselect All work on the CURRENT tool's class in
  the menu as on the keys (`CommandHideSel` ...). Edit > Delete and the Delete key behave the
  same: neither asks (it is one undo step, as in the C++); the Outliner's Delete... keeps its
  `dlg.delete_object`. Esc first leaves placing (`TUI::KeyDown`: `COMMAND_CHANGE_ACTION
  etaSelect` before the bound command).
- Deleting a scene object takes its meshes out of the sectors that hold them, as the C++ does
  (`EScene::RemoveObject` -> the sector tool's `OnObjectRemove`); undo puts both back.
- Scene > Pick Any Class (`etfPickAnyClass`): the toggle for the flag that exists.
- The detail colour-index confirmation id is `clear_colour_indices`.

**Scene files (the File menu)**
- New Scene (Ctrl+N, `CommandClear`), Open Scene... (Ctrl+L, a file dialog through the kit's
  file picker), Recent Scene with Open Last Scene (Ctrl+R; the recent list itself:
  `CCustomPreferences::AppendRecentFile`, `scene_recent_list`, `scene_recent_count`), Save
  As... (Ctrl+Shift+S, `CommandSave` with p2 = 1). Losing unsaved edits asks through the kit's
  `unsaved_prompt`; over MCP the commands answer `unsaved` unless `save=` / `discard=`.

**Level parts**
- Per tool Enable (load / unload a level part) and Read Only (`LevelPreferences.cpp`
  `CLevelPreferences::FillProp` Scene\Targets, `CLevelTool::CommandEnableTarget` /
  `CommandReadonlyTarget`, `EScene::LoadLevelPart` / `UnloadLevelPart`): a disabled part is
  not loaded, not drawn, not saved over (its file stays as it is on disk); Read Only refuses
  edits of that class with the C++ text. In Level Settings as the C++ shows it, and
  `scene_visibility` (or a new `level_parts` command) over MCP.

**Reports**
- Scene > Validate on request (`CommandValidateScene` -> `EScene::Validate(true, ...)`): the
  build's validation as a command with its findings in the kit's Result dialog and in the log.
- Scene > Summary Info: collect, show as a property sheet, export, clear
  (`SceneSummaryInfo.cpp`, `CommandCollectSceneSummary` / `Show` / `Export` / `Clear`).
- Compiler errors: Build > Import / Export / Clear Error List and Scene > Clear Debug Draw
  (`EScene::LoadCompilerError` / `SaveCompilerError`, `CToolCustom::ClearDebugDraw`): the
  error points and faces drawn in the viewport as the C++ draws them.

**Tools**
- Tool switching as commands with their keys (`CLevelTool::CommandChangeTarget`, the 18
  variants; the defaults of the repository's `level.ini [shortcuts]`: 1-9, 0, Ctrl+1..Ctrl+5,
  Ctrl+8, Ctrl+Q), and Show AI Map (`COMMAND_TOGGLE_AIMAP_VISIBILITY`, the `\` key).
- Object tool: Surfaces > Reset Selected, Reset All... (confirmation `reset_surfaces`); Refresh
  List; **Randomize Added Objects applied when placing** (`ESceneObjectControls.cpp`, the add
  control's `Start`: a random object of the list, the scale and rotation ranges) and the whole
  Random Settings sheet (`ESceneObjectTools.cpp`, the `Random Append` sheet: the Scale / Rotate
  switches, Scale\Proportional, the Objects list).
- Spawn tool: Attached Objects - Attach on Click, Detach From Selected... (confirmation
  `detach_spawn_objects`) (`UISpawnTool.cpp`).
- Group tool: Make Thumbnail, Save As..., Reload References, Pivot > Center to Group, Align to
  Object... (`UIGroupTool.cpp`).
- Spawn Details choosers that became typed text: Order\Look path and Order\Route (way paths),
  Order\Target (story ids), Respawn Point\Profile (multi-select) (`SpawnPoint.cpp`
  `CSpawnPoint::FillProp`, the `smCustom` lists): pickers again.
- **Motion of dynamic scene objects** (`CustomObjectAnimation.cpp`
  `CCustomObject::AnimationFillProp`: Flags\Motionable, Motion\Files Import / Export, the key
  commands, the playback controls, Auto Key, Camera View, the frame range, From / To Time,
  Speed). The motion is the engine's `COMotion` (envelopes with the key shapes): a typed model
  in place of the raw chunk, written back byte for byte when untouched; playback moves the
  object in the viewport on the frame clock, never on the UI thread's time.

**Build**
- Build > Make All... / Make Game... / Make Details... / Make HOM... / Make SOM... / Make AI
  Map... ask first, through the kit's confirm, naming the level folder and what is emptied
  (`ConfirmLevelBuild`; Make All empties the level folder for good - Cancel is the default);
  over MCP they ask nothing. The Make commands are `UiCommand`s with the C++ keys (F9, F5),
  in the palette and on the Keyboard page.
- Build > Compile > AI Map > Verify (`CLevelTool::RunXrAI_Verify`), Build > Compile > Spawn >
  All Levels (`RunXrAI_Spawn(false)`).
- **Breakable and climable objects must work.** `gz-levelbuild/src/breakable.rs` reproduces the
  C++ of this repository, whose `VCPacked::add_vert`
  (`Source/Editors/XrECore/Editor/GeometryCollector.cpp`) lost its vertex lookup - the body is
  the remains of the SDK's welding (`P = 0xffffffff; if (0xffffffff==P)`), so every face is its
  own part and the build writes errors instead of objects. That is a bug of the fork, not a
  rule: weld as the SDK did (the header still has the grid: `VMmin`, `VMscale`, `VMeps`, the
  cell lists - a vertex joins an earlier one `similar` to it found through its cell; a new one
  is entered into its cell and the cells its `+eps` corner reaches), take the `// oracle:` mark
  off, and let the scene-level test expect the parts (two boxes of a breakable material sharing
  no face give two parts). Give `gz-entities` the setter for the climable's material that the
  build patches into the packet today. Name the difference in STATE.md ("the C++ editor of the
  branch point writes no breakable objects; the Rust editor does").

**Bindable shell-side commands that are the Level tab's**
- Show Properties (Enter), Scene Options (Ctrl+O), Show World Outliner, Show Content Browser
  (`CommandShowProperties`, `CommandOptions`, ...), the grid slot count command
  (`CommandGridNumberOfSlots`).

**The command table**: register every scene command of the C++ registry in the shell's slot
under its C++ id and name (STATE.md, "The shell's rest", lists the names left out for you), so
`list_commands` covers the registry and `exec_command` runs them.

**The slots other packages left for the scene**: whatever "The shell's rest" says the scene
fills (the camera for a quest position, Reload Objects), fill.

## Rules

- Every action is a command reachable over MCP (a new command or argument gets its entry in
  `crates/gz-mcp/tools.json`, nothing existing changes); a change of the scene is a `SceneCmd`,
  one undo step; a file is read and written on the pools; a frame never computes and never
  touches a file. No `unsafe`, no global state.
- File formats the C++ editor reads back (the selection file, the clipboard stream, the motion,
  the compiler error list) are written byte for byte as the C++ writes them.
- Draw only with the kit; a widget it lacks is `local_<name>`, named in the report. Follow 1a:
  the common case on the surface, the rest under one folded More; a destructive action
  confirms through the kit naming what goes.
- Tests are written, not run.
- Compile: `bash tools/rust/cg.sh check -p gz-scene -p gz-ui-level -p gz-ui-viewport -p
  gz-levelbuild -p grizzly`, zero warnings; clippy clean for your crates. Then build and LOOK,
  on a temp COPY of `D:\XFinedProjects\Test` (the Escape scene): copy / paste over MCP, a
  selection saved and opened, the File menu, Level Settings with the parts, the summary sheet,
  a random placement, the Make All confirmation (Cancel it - never run a level build on the
  look), a dynamic object's motion rows; `ui_audit` shows nothing escaped; `list_commands`.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. Never start `Grizzly.exe` by hand.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 500 words: every item above as done / partly / not done with the reason; `local_*`
widgets; what the look showed. Then add a section "The scene's rest" to `docs/rust/STATE.md`
and mark the lines of `docs/rust/PARITY.md`'s top list you closed (`closed -`).

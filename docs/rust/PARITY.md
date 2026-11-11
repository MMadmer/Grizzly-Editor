# Parity ledger: what the Rust editor still lacks

Read-only audit of 2026-11-11 against the C++ editor under `Source/Editors/LevelEditor`,
`XrECore`, `XrEUI`, `XrEProps` (brief: `docs/rust/briefs/parity-audit.md`). Out of scope by
`docs/RUST_MIGRATION.md` sections 4 and 12: `ActorEditor`, `ParticleEditor`, `ShaderEditor`,
Clear Sky / Shadow of Chernobyl, Play inside the editor (D1), the R1 look (D2), the raw-pixel
lint (D6). States: `same`, `differs`, `missing`, `in work`, `dropped`, `unjudged`.

How it was walked: the C++ command registry (`REGISTER_CMD_*`), every menu row of
`UIMainMenuForm.cpp`, the toolbars and pills, every `MenuItem` / button / section / property-row
label of the four folders (766 labels, matched against non-comment Rust source), every
`PHelper().Create*` / `PrepareKey` property label, the Preferences rows, the command line, every
`dlg.*` and confirmation id, and every MCP command: the C++ dispatch set (157 names) against
`-dump_commands` (179 names), then per command the argument names the C++ handler reads
(`GetArg*` / `Arg*` of `raw`, followed three calls deep) plus the `tools.json` schema against
what the registering Rust crate reads, the `_stricmp(action, ...)` values against the Rust
literals, and the C++ answer keys (`\"key\":`) against the Rust crate. A name-level match is
recorded as `same`; answer values and types were not compared (see Unjudged).

Other agents were writing while this was read: `crates/gz-scene/src/objtools/` and
`crates/gz-particles/` appeared untracked during the audit (the briefs `tools-objects.md` and
`render-rest.md`); what they add is ledgered as `in work`, not judged.

Counts: 333 items walked - 228 same, 29 differs, 52 missing, 8 in work, 9 dropped, 7 unjudged (ledger rows; a row may group several C++ names).

## The list that matters

Every `missing` and `differs` item, by area. Size: S (a day or less), M (days), L (a week+).

### Scene and tools

- missing M - Cut / Copy / Paste / Duplicate of scene objects on the Level tab (Edit menu, Ctrl+X/C/V): `LevelEditor/Editor/UI_LevelMain.cpp` `CommandCut` / `CommandCopy` / `CommandPaste`, `EScene::CutSelection` / `CopySelection` / `PasteSelection`; Duplicate = copy + paste in `LevelEditor/UI/UIMainForm.cpp` `UIMainForm::LevelEdit`. Rust `gz-ui-level/src/doc.rs` `edit()` returns false for all four.
- missing M - Scene > Open Selection... / Save Selection As...: `UI_LevelMain.cpp` `CommandLoadSelection` / `CommandSaveSelection`, `EScene::LoadSelection` / `SaveSelection`.
- missing M - Scene > Export Scene as OBJ... / Export Selection as OBJ...: `LevelEditor/UI/UIMainMenuForm.cpp` `MenuScene`, `EScene::ExportObj`.
- missing M - Scene > Summary Info (collect, show as a property sheet, export): `UI_LevelMain.cpp` `CommandClearSceneSummary` / `CommandCollectSceneSummary` / `CommandShowSceneSummary` / `CommandExportSceneSummary`, `LevelEditor/Editor/Scene/SceneSummaryInfo.cpp`.
- missing S - Scene > Validate on request: `UI_LevelMain.cpp` `CommandValidateScene` -> `EScene::Validate(true, ...)`; Rust runs `gz_levelbuild::validate` only inside a build.
- missing S - Scene > Pick Any Class toggle: `UIMainMenuForm.cpp` `MenuScene` (`COMMAND_SET_SETTINGS`, `etfPickAnyClass`); the flag exists in `gz-ui-viewport/src/service.rs` `Settings::pick_any_class` with nothing to change it.
- missing M - Scene > Clear Debug Draw, Build > Import / Export / Clear Error List (the compiler's error points drawn in the viewport): `UI_LevelMain.cpp` `CommandClearDebugDraw`, `CommandImportCompilerError`, `CommandExportCompilerError`, `EScene::LoadCompilerError` / `SaveCompilerError`, `CToolCustom::ClearDebugDraw`.
- missing S - tool switching as commands with keys (`COMMAND_CHANGE_TARGET`, 18 variants; the repository's `level.ini [shortcuts]` binds 1-9, 0, Ctrl+1..Ctrl+5, Ctrl+8, Ctrl+Q): `CLevelTool::CommandChangeTarget`; Rust switches only by a click on a Tools tile.
- missing M - Object tool, Surfaces: Reset Selected, Reset All... (`reset_surfaces` confirmation): `LevelEditor/UI/Tools/UIObjectTool.cpp`.
- missing S - Object tool, Refresh List: `UIObjectTool.cpp`; `gz-ui-level/src/lists.rs` `Lists::refresh` exists and nothing calls it.
- missing M - Randomize Added Objects is stored but never applied when placing (random object of the list, scale, rotation): `LevelEditor/Editor/Tools/StaticObject/ESceneObjectControls.cpp` (add control `Start`); Rust `gz-ui-viewport/src/pane.rs` `place()` ignores `FL_APPEND_RANDOM`.
- differs S - Random Settings... shows only the four ranges; the C++ sheet also has the Scale / Rotate switches, Scale\Proportional and the Objects list: `LevelEditor/Editor/Tools/StaticObject/ESceneObjectTools.cpp` (the `Random Append` sheet); Rust `gz-ui-level/src/tools.rs` `RandomSettings`.
- missing M - Spawn tool, Attached Objects: Attach on Click, Detach From Selected... (`detach_spawn_objects` confirmation): `LevelEditor/UI/Tools/UISpawnTool.cpp`.
- missing M - Group tool: Make Thumbnail, Save As..., Reload References, Pivot > Center to Group, Align to Object...: `LevelEditor/UI/Tools/UIGroupTool.cpp`.
- missing M - motion of dynamic scene objects (Flags\Motionable, Motion\Files Import / Export, key commands, playback controls, Auto Key, Camera View, frame range, From / To Time, Speed): `LevelEditor/Editor/Entry/CustomObjectAnimation.cpp` `CCustomObject::AnimationFillProp` (called from `CSceneObject::FillProp` for dynamic objects). Rust keeps the motion chunk raw (`gz-scene/src/objects/mod.rs`) and shows none of it.
- differs S - Spawn Details choosers that became typed text: Order\Look path and Order\Route (way paths), Order\Target (story ids), Respawn Point\Profile (multi-select): `LevelEditor/Editor/Entry/Spawn/SpawnPoint.cpp` `CSpawnPoint::FillProp` (`smCustom` lists); Rust `gz-ui-level/src/lists.rs` `ListKey::of_choose` knows no `way_path`, `story_id`, `respawn_items`.
- missing M - per tool Enable (load / unload a level part) and Read Only: `LevelEditor/Editor/Utils/LevelPreferences.cpp` `CLevelPreferences::FillProp` (Scene\Targets), `CLevelTool::CommandEnableTarget` / `CommandReadonlyTarget`, `EScene::LoadLevelPart` / `UnloadLevelPart`. `SceneCmd::ToolFlags` has a `readonly` field nothing sets.
- missing S - Rename the selected objects after their references (`COMMAND_MULTI_RENAME_OBJECTS`, `rename_objects` confirmation): `CLevelTool::CommandMultiRenameObjects`, `EScene::MultiRenameObjects`.
- differs S - Hide Selected / Hide Unselected / Hide All / Deselect All: the C++ works on the current tool's class (`CommandHideSel` / `CommandHideUnsel` / `CommandHideAll` / `CommandDeselectAll`); Rust's menu rows work on every class (`gz-ui-level/src/menus.rs` `run_scene_command`, `outliner::show_objects`) while the same keys work on the target class (`gz-ui-viewport/src/keys.rs` `run`).
- differs S - Edit > Delete asks first while the Delete key does not (`menus.rs` `ask_delete_selection` vs `keys.rs` `Act::Delete`); the C++ `CommandDeleteSelection` asks on neither path.
- differs S - Esc does not stop placing: `XrECore/Editor/UI_MainCommand.cpp` `TUI::KeyDown` runs `COMMAND_CHANGE_ACTION etaSelect` before the bound command; Rust binds Esc to Deselect All only.
- differs S - the detail colour-index confirmation id: C++ `clear_colour_indices` (`LevelEditor/UI/UIDOShuffle.cpp`), Rust `clear_color_indices` (`gz-ui-level/src/detail_objects.rs`).

### Viewport

- closed S - View > Render > GPU (adapters by name, System Default, the three driver hints): `UIMainMenuForm::DrawViewportOptions` (`GpuAdapterName`, `SetGpuPreference`, `CLevelPreferences::GpuAdapter`).
- closed S - View > Render > Quality 25 / 50 / 100 / 200 %: `DrawViewportOptions` (`UI->SetRenderQuality`).
- closed S - Fill Mode (Point / Wireframe / Solid), Shade Mode, Edged Faces, Linear Filtering, Textures, Fog, Environment are `view_mode` arguments but not in the View menu or the Options pill (only a Wireframe toggle): `DrawViewportOptions`; Rust `gz-ui-viewport/src/menus.rs` `options_body`.
- closed S - View > Mute Sounds and Real Time: `DrawViewportOptions` (`rsMuteSounds`, `rsRenderRealTime`), `UI_MainCommand.cpp` `CommandMuteSound`.
- missing S - Show AI Map toggle (`COMMAND_TOGGLE_AIMAP_VISIBILITY`, the `\` key): `UI_LevelMain.cpp` `CommandToggleAiMapVisibility` (the AI map's drawing itself is in work).

### Content Browser

- nothing missing: every context-menu row, Add item, view option and MCP argument of `LevelEditor/UI/ContentBrowser/UIContentBrowser.cpp` has its counterpart in `gz-ui-content`.

### Libraries

- closed S - Libraries > Reload Objects: `UI_LevelMain.cpp` `CommandReloadObjects` (`Lib.ReloadObjects`); `gz-scene/src/library.rs` `Library::clear` exists and nothing calls it.
- closed S - Libraries > Reload Textures: `UI_MainCommand.cpp` `CommandReloadTextures`.
- closed S - Libraries > Rewrite Texture Descriptions... (`rewrite_thumbnails` confirmation): `UIMainMenuForm.cpp` `RewriteThumbnails` / `RewriteThumbnailsNow`.
- closed (dropped: `RealRefreshEnvGeometry` is commented out, the C++ command does nothing) S - Libraries > Refresh Sound Environment Geometry: `UI_LevelMain.cpp` `CommandRefreshSoundEnvGeometry` (`CLevelSoundManager::RefreshEnvGeometry`).
- closed S - Texture Library, Cube Map\Smaller Copy: `XrECore/Editor/UIImageEditorForm.cpp` (STATE.md: not done).

### Mod and build

- closed M - Mod > Bake Scene Layer > Spawn Layer..., Collision Overlay..., Visual Overlay..., Level Cut... (the dialogs; the MCP `mod_export_*` exist): `UIMainMenuForm.cpp` `MenuMod`, `EditorModScene::RequestExportSpawn` / `RequestExportXCForm` / `RequestExportOGF` / `RequestExportCut`. Nobody fills the `BAKE_SCENE_LAYER` slot of `gz-ui-mod/src/menu.rs`.
- differs S - Build > Make All... / Make Game... / Make Details... / Make HOM... / Make SOM... / Make AI Map... run at once; the C++ asks first, naming the level folder (Make All empties it for good): `UI_LevelMain.cpp` `ConfirmLevelBuild` in `CommandBuild` / `CommandMake*`; Rust `gz-ui-level/src/menus.rs` `build_menu` posts `make_*`, and `gz-levelbuild` `compile_level` wipes the folder.
- differs S - the Make commands are no `UiCommand`s: no F9 / F5 binding, not in the palette or the Keyboard page: `REGISTER_CMD_SE` of `COMMAND_BUILD` / `COMMAND_MAKE_*`.
- missing S - Build > Compile > AI Map > Verify (`xrAI -verify`): `LevelEditor/Editor/UI_LevelTools.cpp` `CLevelTool::RunXrAI_Verify`.
- missing S - Build > Compile > Spawn > All Levels: `CLevelTool::RunXrAI_Spawn(false)`; Rust `compile_spawn` builds this level only.
- closed S - Mod Settings picture: a chosen picture is applied to `mod.ltx` at once and Remove deletes `thumbnail.dds` at once (no Keep Picture, nothing waits for a Save): `XrECore/Editor/EditorModManifest.cpp` thumbnail row (`s_ThumbPending`, `s_ThumbRemove`, `PickThumbnail`); Rust `gz-ui-mod/src/settings.rs` `set_picture`.

### Quests

- closed S - the camera button of a position (the viewport camera's position and level): `LevelEditor/UI/QuestGraph/NqInspector.cpp` `NqInspector::DrawPosition` (STATE.md: not ported).

### Shell

- missing S - File > New Scene (Ctrl+N): `UI_LevelMain.cpp` `CommandClear`.
- missing S - File > Open Scene... (Ctrl+L, the file dialog): `CommandLoad` without a path; Rust opens a scene only from the Content Browser or `open_scene`.
- missing S - File > Recent Scene, Open Last Scene (Ctrl+R) and the recent list itself: `UIMainMenuForm.cpp` `MenuFile`, `CommandLoadFirstRecent`, `CCustomPreferences::AppendRecentFile` / `scene_recent_list`; Rust has only the `scene_recent_count` preference.
- missing S - File > Save As... (Ctrl+Shift+S): `CommandSave` with p2 = 1.
- closed S - Show Properties (Enter), Scene Options (Ctrl+O), Show World Outliner, Show Content Browser as bindable commands: `CommandShowProperties`, `CommandOptions`, `CommandShowWorldOutliner`, `CommandShowContentBrowser` (the Window menu rows exist).
- closed S - macros: Run Macro slots 1-8 (Alt+1..Alt+8), Assign Macro, Execute Command List, Log Commands: `UI_MainCommand.cpp` `CommandRunMacro` / `CommandAssignMacro` / `CommandExecuteCommandList` / `CommandLogCommands` (the Keyboard page's order list names the slots, nothing registers them).
- closed S - grid slot count command: `UI_MainCommand.cpp` `CommandGridNumberOfSlots`.
- closed S - `-uiscale <f>`: `XrEUI/XrUIKit.cpp` `CommandLineScale`.
- differs S - no splash window while the editor starts: `EditorStartup` (splash thread); the stage profile is there (`grizzly/src/startup.rs`).
- closed S - the Start page's status bar shows the Level tab's centre (STATE.md, still open).

### MCP wire

- closed M - `list_commands` / `exec_command` cover the 48 `UiCommand`s, not the 118 `COMMAND_*` of the C++ registry, and `id` is the Rust table's index: `LevelEditor/Editor/Mcp/LevelMcp.cpp` (`GetEditorCommands`); Rust `gz-app/src/cmds.rs` `command_table`.
- closed S - `exec_command` never reads `p2s` / `p2i`: `LevelMcp.cpp` `exec_command` (`ExecCommand(id, P1, P2)`).
- closed S - `screenshot_editor` answers `busy: false` and `outside: []` always: `XrECore/Editor/GrizzlyMCP.cpp`; Rust `gz-app/src/shots.rs`.
- differs S - `game_link` / `game_modes` refusals carry `error` first (C++ `EditorProject::McpGameLink` had none).
- differs S - `characters_write` refuses a path outside the project and validates a text without `id` under its file's id: `XrECore/Editor/EditorCharacters.cpp` `McpWrite`.
- differs S - `world_set` writes only the section it changed; an empty declared `[start_kit]` becomes `perks = true`: `XrECore/Editor/EditorWorld.cpp` `McpSet` / `Save`.
- closed S - `screens_set action=drop` always fills the loading surface: `XrECore/Editor/EditorScreens.cpp` `McpSet`.
- differs S - `mod_publish action=start` answers once packaging ended in one call (no `job_pending` round): `XrECore/Editor/EditorModPublish.cpp` `McpPublish` + `GrizzlyMCP.cpp` `FollowsJob`.
- differs S - `quest_history` adds `undo_steps` / `redo_steps`, and an undo back to the saved state is clean: `LevelEditor/UI/QuestGraph/UIQuestGraph.cpp` `McpHistory`.

## In work (known, not re-derived)

| Item | Brief |
|---|---|
| Tool panels, commands and gestures of Sector, Portal, Way, Shape, Light, Fog Volume (incl. the Fog Volume tool sheet's VolumeType / profile (ltx), confirmations `remove_default_sector`, `remove_duplicate_portals`, `detach_shapes`) | `briefs/tools-objects.md` |
| AI Map tool (Generate Full / Selected, Smooth, Reset, Select 0/1/2-Link, Ignore Constraints, Ignored Materials, `clear_ai_map`, `clear_ignored_materials`), Detail Objects generation (First Initialize, Reinitialize All / Objects / Selected, Update Renderer, the tool sheet's Base Texture and Common\Draw * rows), Wallmarks editing (Common\Alignment, Common\Draw Wallmarks) | next package |
| Particles playing (and the Particles tool's Play / Stop), glows, AI map, details, wallmarks, cube sky in the viewport | `briefs/render-rest.md` |
| Breakable / climable objects and the rest of `Scene->Validate` in the level build | STATE.md |
| VIPM for progressive skeletons | STATE.md |
| closed - Panels outside the window (`[ui] floating_panels`, `screenshot_editor` `outside`) | STATE.md, "The shell's rest" |

## Ledger

### 1. The command table

`XrECore/Editor/UI_MainCommand.cpp` `TUI::RegisterCommands` (43 commands) and
`LevelEditor/Editor/UI_LevelMain.cpp` `CLevelMain::RegisterCommands` (75; `COMMAND_CREATE_SOUND_LIB`
is registered in both). The C++ `list_commands` / `exec_command` reach every one of them; the Rust
ones reach the 48 `UiCommand`s (`gz-app` shell 4, `gz-ui-level/src/menus.rs` 11,
`gz-ui-viewport/src/keys.rs` 15, `gz-ui-mod/src/menu.rs` 5, `gz-ui-pda` 2, `gz-ui-world` 2,
`gz-ui-release` 3, `gz-ui-previews` 6). "Rust" names the `UiCommand` id, or the command / menu
that does the same.

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| `COMMAND_INITIALIZE`, `DESTROY`, `QUIT`, `UPDATE_PROPERTIES`, `REFRESH_PROPERTIES`, `RENDER_FOCUS`, `RENDER_RESIZE`, `UPDATE_GRID`, `CREATE_SOUND_LIB`, `CHECK_MODIFIED`, `SET_SETTINGS`, `CLEAN_LIBRARY`, `REFRESH_UI_BAR`, `RESTORE_UI_BAR`, `SAVE_UI_BAR`, `UPDATE_TOOLBAR`, `UPDATE_CAPTION`, `UNLOAD_TEXTURES`, `EVICT_OBJECTS`, `EVICT_TEXTURES`, `SAVE_BACKUP` | plumbing: start / stop, panel refresh, caption, grid rebuild, resource eviction, the menus' settings setter, the studio mailslot's backup save (`UI_MainMail.cpp`) | the shell, the stores, `gz-render` asset LRU, recovery | same (no user function) |
| `COMMAND_LIBRARY_EDITOR`, `LANIM_EDITOR`, `FILE_MENU`, `SHOW_CLIP_EDITOR`, `SHOWCONTEXTMENU`, `REFRESH_SNAP_OBJECTS`, `CHANGE_SNAP`, `MOVE_CAMERA_TO`, `SIMULATE`, `USE_SIMULATE_POSITIONS`, `EDIT_COMMAND_LIST`, `SCENE_HIGHLIGHT_TEXTURE` | bodies commented out, `not_implemented()` or `return FALSE` (`UI_LevelMain.cpp` `CommandLibraryEditor` ... `CommandSceneHighlightTexture`; `UI_MainCommand.cpp` `CommandMoveCameraTo`, `CommandChangeSnap`, `CommandEditCommandList`; `UI_LevelTools.cpp` `CLevelTool::Simulate` / `UseSimulatePositions`) | - | dropped (not reachable in the C++ editor either: the bodies do nothing) |
| `COMMAND_EXIT` | Quit, asks for unsaved work | `COMMAND_EXIT` (`gz-app/src/shell.rs`) | same |
| `COMMAND_EDITOR_PREF` | opens Preferences | `COMMAND_EDITOR_PREF` | same |
| `COMMAND_COMMAND_PALETTE` | the palette, Ctrl+Shift+P | `COMMAND_COMMAND_PALETTE` | same |
| `COMMAND_BREAK_LAST_OPERATION` | stops the running job | `COMMAND_BREAK_LAST_OPERATION` "Cancel Running Job" | same |
| `COMMAND_CHANGE_ACTION` "Select" / "Add" | stop placing / place | `COMMAND_CHANGE_ACTION."Select"` / `."Add"` (`keys.rs`) | same |
| `COMMAND_GIZMO_MODE` Select / Move / Rotate / Scale / Cycle | gizmo mode, Q W E R Space | `COMMAND_GIZMO_MODE.*` (`keys.rs`) | same |
| `COMMAND_ZOOM_EXTENTS` Extent / Selected | Frame All / Frame Selection | `COMMAND_ZOOM_EXTENTS.*` | same |
| `COMMAND_TOGGLE_RENDER_WIRE` | wireframe toggle | `COMMAND_TOGGLE_RENDER_WIRE` | same |
| `COMMAND_TOGGLE_SAFE_RECT`, `COMMAND_TOGGLE_GRID` | safe rect, grid | same ids | same |
| `COMMAND_GRID_SLOT_SIZE` Decrease / Increase | grid cell size | same ids | same |
| `COMMAND_GRID_NUMBER_OF_SLOTS` | grid cell count +-2 (`CommandGridNumberOfSlots`) | - (the `grid_cell_count` preference only) | missing |
| `COMMAND_SOUND_EDITOR`, `IMAGE_EDITOR`, `LIGHTANIM_EDITOR` | the three libraries | same ids (`gz-ui-previews/src/lib.rs`) | same |
| `COMMAND_CHECK_TEXTURES`, `REFRESH_TEXTURES`, `SYNC_SOUNDS` | Check New Textures, Synchronize Textures / Sounds (confirmed) | same ids | same |
| `COMMAND_RELOAD_TEXTURES` | `EDevice->ReloadTextures` | - | missing |
| `COMMAND_SHOW_PROPERTIES` | focus Details (Enter) | Window > Details row only | missing |
| `COMMAND_OPTIONS` | Details of the scene options (Ctrl+O) | Window > Level Settings row only | missing |
| `COMMAND_MUTE_SOUND` | mutes the scene's sound sources | - | missing |
| `COMMAND_EXECUTE_COMMAND_LIST`, `LOG_COMMANDS`, `RUN_MACRO` (8 slots), `ASSIGN_MACRO` | macros: a file of commands run by Alt+1..8 | - (`gz-ui-app/src/keyboard.rs` `COMMAND_ORDER` names the slots only) | missing |
| `COMMAND_CHANGE_TARGET` (18 variants) | switch the active tool, keys | Tools panel tiles only | missing |
| `COMMAND_ENABLE_TARGET`, `LOAD_LEVEL_PART`, `UNLOAD_LEVEL_PART` | load / unload one class's level part | - | missing |
| `COMMAND_SHOW_TARGET` | a class's visibility | Outliner class eye, `scene_visibility` (`SceneCmd::ToolFlags`) | same |
| `COMMAND_READONLY_TARGET` | a class read only | - (`ToolFlags.readonly` never set) | missing |
| `COMMAND_MULTI_RENAME_OBJECTS` | rename selected after their references (confirmed) | - | missing |
| `COMMAND_SHOW_OBJECTLIST` | Find in World Outliner (Ctrl+F) | `COMMAND_SHOW_OBJECTLIST` (`menus.rs`) | same |
| `COMMAND_LOAD` | open a scene: path, or the file dialog; recent list kept | `open_scene` (MCP, Content Browser); no dialog, no recent list | missing |
| `COMMAND_SAVE` "Save" | save the scene | shell Ctrl+S, `save_scene`, Level toolbar Save | same |
| `COMMAND_SAVE` "Save As" | save under a new name | - | missing |
| `COMMAND_CLEAR` | New Scene | - | missing |
| `COMMAND_LOAD_FIRSTRECENT` | Open Last Scene | - | missing |
| `COMMAND_CLEAR_DEBUG_DRAW`, `IMPORT_COMPILER_ERROR`, `EXPORT_COMPILER_ERROR` | the compiler's error list in the viewport | - | missing |
| `COMMAND_VALIDATE_SCENE` | `EScene::Validate` with every test, on request | `gz_levelbuild::validate` inside builds only | missing |
| `COMMAND_RELOAD_OBJECTS` | re-read the object library | - (`Library::clear` uncalled) | missing |
| `COMMAND_CUT`, `COPY`, `PASTE` | the scene clipboard | - | missing |
| `COMMAND_LOAD_SELECTION`, `SAVE_SELECTION` | scene fragments to / from a file | - | missing |
| `COMMAND_UNDO`, `REDO` | scene undo / redo | `COMMAND_UNDO` / `COMMAND_REDO` -> `undo` / `redo` | same |
| `COMMAND_CLEAR/COLLECT/SHOW/EXPORT_SCENE_SUMMARY` | Summary Info | - | missing |
| `COMMAND_BUILD`, `MAKE_GAME`, `MAKE_AIMAP`, `MAKE_DETAILS`, `MAKE_HOM`, `MAKE_SOM` | Make *, each confirmed naming the level folder (`ConfirmLevelBuild`) | `make_all` ... `make_aimap` (MCP, Build > Make rows) without the confirmation and without a `UiCommand` | differs |
| `COMMAND_INVERT_SELECTION_ALL`, `SELECT_ALL` | of the current class | same ids | same |
| `COMMAND_DESELECT_ALL` | of the current class | same id; every class | differs |
| `COMMAND_DELETE_SELECTION` | removes the selection, no question | same id; the menu row asks (`dlg.delete_object`), the key does not | differs |
| `COMMAND_HIDE_SEL`, `HIDE_UNSEL`, `HIDE_ALL` | of the current class | same ids; menu = every class, key = target class | differs |
| (none) | - | `COMMAND_UNHIDE_ALL` "Unhide All" (Rust only) | same (addition) |
| `COMMAND_SET/ADD_SEL/DEL_SEL/CLEAR/SELECT_SNAP_OBJECTS` | the snap list | `snap_objects` and the Tools panel's snap buttons (`gz-ui-level/src/tools.rs`) | same |
| `COMMAND_REFRESH_SOUND_ENV_GEOMETRY` | `CLevelSoundManager::RefreshEnvGeometry` | - | missing |
| `COMMAND_CB_PLACE_ASSET` | place a Content Browser asset (centre / under the cursor / as a visual) | Content Browser Place in Scene, drop on the viewport (`gz-ui-content`, `set_place_item`) | same |
| `COMMAND_DROP_TO_GROUND` | End: drop the selection | `COMMAND_DROP_TO_GROUND` -> `drop_objects` | same |
| `COMMAND_SHOW_CONTENT_BROWSER`, `SHOW_WORLD_OUTLINER` | toggle the panels | Window menu rows only | missing |
| `COMMAND_TOGGLE_AIMAP_VISIBILITY` | show / hide the AI map (`\`) | - | missing |
| `COMMAND_MOD_BUILD` (Ctrl+B), `MOD_EDIT_MANIFEST`, `MOD_PACKAGE`, `MOD_PUBLISH`, `MOD_RELEASES`, `MOD_CHARACTERS`, `MOD_WORLD`, `MOD_PDA`, `MOD_CATALOG` | the Mod rows | same ids (`gz-ui-mod`, `gz-ui-release`, `gz-ui-world`, `gz-ui-pda`) | same |
| (none) | - | `COMMAND_MOD_SCREENS`, `COMMAND_MOD_BUILD_TO_FOLDER`, `COMMAND_MOD_EXPORT_FLAT` (Rust only) | same (addition) |

### 2. The menus

`LevelEditor/UI/UIMainMenuForm.cpp` (`MenuFile`, `MenuEdit`, `MenuScene`, `MenuBuild`, `MenuMod`,
`MenuLibraries`, `MenuWindow`, `MenuHelp`, `DrawLevelMenus`, `DrawViewportOptions`). Rust: the
shell's `BUILTINS` and contributions (`gz-app/src/chrome.rs`), `gz-ui-level/src/menus.rs`,
`gz-ui-mod/src/menu.rs`, `gz-ui-viewport/src/menus.rs`, `gz-ui-previews/src/lib.rs`.

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| File > New Scene | `COMMAND_CLEAR` | - | missing |
| File > Open Scene... | `COMMAND_LOAD` file dialog | - | missing |
| File > Recent Scene > (list) | `COMMAND_LOAD <path>` | - | missing |
| File > Save | the active tab's save | `Builtin::Save` | same |
| File > Save As... | `COMMAND_SAVE` p2 = 1 | - | missing |
| File > Project > Import Base Scene... | `dlg.import_base_scene` | `gz-ui-level/src/import.rs` | same |
| File > Project > Open Project Folder / Switch Project... | Explorer / back to the Start page | `Builtin::OpenProjectFolder` / `SwitchProject` | same |
| File > Quit | `RequestExit` | `Builtin::Quit` | same |
| Edit > Undo / Redo / Delete / Find | the active tab's action | `Builtin::Edit` | same |
| Edit > Cut / Copy / Paste / Duplicate | the active tab's action (Level: scene clipboard) | graph tabs: same (`gz-ui-graph/src/tab.rs`); Level: disabled | missing |
| Edit > Command Palette... / Preferences | palette / Preferences | `Builtin::Palette` / `Preferences` | same |
| Scene > Validate | `COMMAND_VALIDATE_SCENE` | - | missing |
| Scene > Summary Info | the three summary commands | - | missing |
| Scene > Highlight Texture... | `COMMAND_SCENE_HIGHLIGHT_TEXTURE` (returns FALSE) | - | dropped (not reachable: the handler is commented out) |
| Scene > Find in World Outliner | `COMMAND_SHOW_OBJECTLIST` | same row | same |
| Scene > Clear Debug Draw | `COMMAND_CLEAR_DEBUG_DRAW` | - | missing |
| Scene > Open Selection... / Save Selection As... | fragments | - | missing |
| Scene > Export Scene as OBJ... / Export Selection as OBJ... | `EScene::ExportObj(false / true)` | - | missing |
| Scene > Mark Selection as Mod Content / as Base Level | `MarkSelection` | `mark_authored` rows | same |
| Scene > Pick Any Class | `etfPickAnyClass` toggle | - | missing |
| Scene > Create Recovery Snapshot | `EditorRecovery::RequestSnapshot` | `recovery action=snapshot` row | same |
| (none) | - | Scene > Select All / Unselect All / Invert / Hide * / Unhide All (Rust only) | same (addition) |
| Build > Make > Make All..., Make Game..., Make Details..., Make HOM..., Make SOM..., Make AI Map... | confirmed builds | `BUILD_ROWS` -> `make_*`, unconfirmed | differs |
| Build > Compile > Geometry & Light / Detail Object Light | `RunXrLC` / `RunXrDO` | `compile_geometry` / `compile_details` | same |
| Build > Compile > AI Map > High / Low | `RunXrAI_AIMap(true / false)` | `compile_aimap mode=high/low` | same |
| Build > Compile > AI Map > Verify | `RunXrAI_Verify` | - | missing |
| Build > Compile > Spawn > This Level | `RunXrAI_Spawn(true)` | `compile_spawn` "Spawn" | same |
| Build > Compile > Spawn > All Levels | `RunXrAI_Spawn(false)` | - | missing |
| Build > Import Error List... / Export Error List... / Clear Error List | compiler errors | - | missing |
| (none) | - | Build > the toolbar's builds (Rust only) | same (addition) |
| Mod > Mod Settings, Characters, World Rules, Screens & Music, PDA Interface | the module's tabs | `MOD_MENU` rows | same |
| Mod > Build Into Game, Build to Folder..., Export Flat Gamedata... | builds | `MOD_MENU` rows | same |
| Mod > Bake Scene Layer > Spawn Layer..., Collision Overlay..., Visual Overlay..., Level Cut... | `EditorModScene::Request*` dialogs | - (empty slot) | missing |
| Mod > Package Release..., Publish Release..., Releases, Submit to Mod Browser... | release path | `MOD_MENU` rows | same |
| Libraries > Light Animations, Texture Library, Sound Library, Check New Textures, Synchronize Textures..., Synchronize Sounds... | libraries | `gz-ui-previews` rows | same |
| Libraries > Reload Objects | `COMMAND_RELOAD_OBJECTS` | - | missing |
| Libraries > Reload Textures | `COMMAND_RELOAD_TEXTURES` | - | missing |
| Libraries > Rewrite Texture Descriptions... | `RewriteThumbnails` | - | missing |
| Libraries > Refresh Sound Environment Geometry | `COMMAND_REFRESH_SOUND_ENV_GEOMETRY` | - | missing |
| Window > Tools, World Outliner, Details, Level Settings, Content Browser, Output Log | `PanelItem` | `Builtin::Panels` over `PanelDesc`s | same |
| Window > Content Drawer / Log Drawer / the open documents / Reset Layout | drawers, tabs, layout | `Builtin::Drawers` / `Documents` / `ResetLayout` | same |
| Help > Keyboard Shortcuts / About Grizzly Editor | Keyboard page / notice | `Builtin::KeyboardShortcuts` / `About` | same |
| View > Render > GPU | adapter + driver hint | - | missing |
| View > Render > Quality | render scale | - | missing |
| View > Render > Fill Mode (Point / Wireframe / Solid) | `dwFillMode` | Wireframe toggle; `view_mode fill=` | differs |
| View > Render > Shade Mode (Flat / Gouraud) | `dwShadeMode` | `view_mode shade=` only | differs |
| View > Render > Edged Faces / Linear Filtering / Textures | device flags | `view_mode` only | differs |
| View > Render > Hardware Rendering | `HW.Caps.bForceGPU_SW` | - | dropped (D2: a render state of the R1 device) |
| View > Safe Rect / Grid / Lighting / Game Lighting > / Stats | view flags | `options_body`, `game_body` | same |
| View > Fog / Environment > None, (weather cycles) | `rsFog`, `rsEnvironment` + `SetWeather` | `view_mode fog= environment=` only | differs |
| View > Mute Sounds / Real Time | `rsMuteSounds` / `rsRenderRealTime` | - | missing |
| View > Maximize Viewport (F11) | | `menus.rs` row, F11 | same |
| (none) | - | View > Spawn Helpers (Rust only) | same (addition) |

### 3. Toolbars, the viewport's pills, the status bar, the caption

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| Level toolbar `save`, `undo`, `redo` (`UITopBarForm.cpp`) | | `gz-ui-level/src/toolbar.rs` `draw` | same |
| `build_cform`, `build_aimap`, `build_ggraph`, `build_all`, `play_game`, `play_clean`, `stop`, `reload_configs`, `game_folder` | | `LEVEL_ACTIONS` | same |
| `play` (+ `play_options` Verify Space Restrictors) | plays inside the editor (`Scene->Play`) | `play` runs the game on the baked scene | dropped (D1) |
| Project row: `build_mod` split, `publish_release`, `releases` (`UITopBarForm::DrawProject`) | | `gz-ui-mod/src/menu.rs` `build_mod`, `gz-ui-release/src/lib.rs` | same |
| Viewport pill Options (`UIViewportToolbar.cpp`, `DrawViewportOptions`) | the View options | `menus::options_body` | differs (as the View menu above) |
| Pills Perspective, Show; `tool_select` / `tool_place` / `tool_move` / `tool_rotate` / `tool_scale`; `osnap`, Snap options, `grid_snap` / `scale_snap` / `angle_snap` | | `gz-ui-viewport/src/pane.rs` | same |
| Status bar: drawer buttons, last log line, warning / error counts, Tasks, Rebuild chip (`XrUIShell.cpp`, `UIMainForm.cpp`) | | `gz-app/src/chrome.rs` `draw_status`, `gz-ui-level/src/status.rs` | same |
| Status bar on the Start page | no Level centre | the Level centre is shown | differs |
| Caption: menus folded under 1100 pt, tab strip, window buttons, hit test (`UITitleBar.cpp`, `XrUIShell.cpp`) | | `gz-app/src/chrome.rs`, `gz-win` | same |
| Tab context menu: Close, Close Others, Close All, Close Saved, Copy Path, Show in Explorer, Show in Content Browser | | `chrome.rs` tab menu | same |

### 4. Context menus and key bindings

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| Viewport right click (`COMMAND_SHOWCONTEXTMENU` -> `CLevelMain::ShowContextMenu`) | nothing: the body is commented out | - | dropped (not reachable in the C++ editor either) |
| World Outliner row menu: Focus, Hide Selected, Show Selected, Show All, Select All of This Class, Rename..., Delete...; By Type / By Hierarchy (`UIWorldOutliner.cpp`) | | `gz-ui-level/src/outliner.rs` | same |
| Content Browser menus: Add > New Folder / Quest / Behaviour Graph / Character / PDA Interface; Open, Place in Scene, Rename, Copy, Cut, Paste, Copy Path, Show in Explorer, Delete..., Exclude / Include in Build, Select All, Clear Selection, Refresh; View: Tiles, List, Fit to Height, Sources Pane, Category; Favorites (`UIContentBrowser.cpp`) | | `gz-ui-content` (every label found) | same |
| Graph canvas and inspector menus (42 labels, `NqCanvas.cpp`, `NqInspector.cpp`, `UIQuestGraph.cpp`) | | `gz-ui-graph` (every label found) | same |
| Shell chords Ctrl+Shift+P, Ctrl+(Shift+)Tab, Ctrl+F4, Ctrl+Space, Ctrl+`, Ctrl+S, F11 | | `gz-app/src/shell.rs` | same |
| Camera keys W A S D Q E + Ctrl / Shift (`UI_Camera.cpp`, `UI_ViewportNavigation.cpp`) | | `gz-ui-viewport/src/camera.rs`, `pane.rs` | same |
| Scene bindings of `kCommandNames` (`EditorPreferences.cpp`) present in Rust: Undo, Redo, Delete, Find, Select All, Deselect All, Invert, Hide Sel / Unsel / All, gizmo modes, Place / Stop Placing, Frame All / Selection, Wireframe, Grid, grid size, Safe Rect, Drop to Ground, the Mod rows, the libraries, Palette, Preferences, Quit, Cancel Running Job | | `SCENE_COMMANDS`, `BINDINGS`, the area `UiCommand`s | same |
| Bindings New Scene (Ctrl+N), Open Scene (Ctrl+L), Open Last Scene (Ctrl+R), Save As (Ctrl+Shift+S), Cut / Copy / Paste on Level, the 18 tool switches, Show AI Map (`\`), Make All (F9), Make Game (F5), Details (Enter), Scene Options (Ctrl+O), World Outliner, Content Browser, Run Macro 1-8 (Alt+1..8) | | - (no command to bind) | missing |
| File Menu (Alt+F), Move Camera To (Ctrl+Enter), Simulate, Apply Simulated Positions | handlers do nothing | - | dropped (not reachable) |
| Esc: stop placing, then the bound command (`TUI::KeyDown`) | | Esc = Deselect All only | differs |

### 5. MCP commands

Every name of the C++ dispatch (`GrizzlyMCP.cpp`, `LevelMcp*.cpp`, `NqMcp.cpp`: 157) is
registered in Rust, and every `cmd` of `crates/gz-mcp/tools.json` (159) is in `-dump_commands`.
Registered twice in Rust, the later handler kept (`gz-editor/src/registry.rs` `Registry::add`;
`-dump_commands` prints the FIRST area): `mod_catalog` (`gz-release` stand-in, then `gz-catalog`),
`property_inspector` (`gz-scene`, then `gz-ui-level`), `quest_open` / `_edit` / `_find` /
`_bookmarks` / `_history` / `_minimap` (`gz-nq`, then `gz-ui-graph`), `window_control`
(`grizzly`, then `gz-app`). Rust only: `jobs`, `level_build`, `level_rebuild_status` and the 19
level build commands (`build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`,
`play_game`, `play_clean`, `play_stop`, `reload_configs`, `make_all`, `make_game`,
`make_details`, `make_hom`, `make_som`, `make_aimap`, `compile_geometry`, `compile_details`,
`compile_aimap`, `compile_spawn`) - the C++ reached those through `exec_command` / the toolbar.

"Arguments" lists what the C++ handler reads from `raw` plus the `tools.json` schema; each name
was found read by the Rust crate that registers the command (or its later handler). The pump's
waits (`answer_only`, held frames, `FollowsJob`) live in `GrizzlyMCP.cpp` for the C++ and in each
Rust handler.

| Command | C++ | Rust crate | State | Notes |
|---|---|---|---|---|
| `asset_preview` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-ui-previews` | same | every argument name found read in the handler crate (3: category, name, source) |
| `camera_get` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (0: none) |
| `camera_set` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (6: b, h, p, x, y, z) |
| `characters_delete` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-characters` | same | every argument name found read in the handler crate (2: id, path) |
| `characters_get` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-characters` | same | every argument name found read in the handler crate (2: id, path) |
| `characters_list` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-characters` | same | every argument name found read in the handler crate (0: none) |
| `characters_view` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-ui-world` | same | every argument name found read in the handler crate (6: discard, open, path, save, select, show) |
| `characters_write` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-characters` | differs | refuses a path outside the project; a text without `id` is validated under its file's id (C++ `EditorCharacters::McpWrite` wrote anywhere and validated an empty id, E102); arguments read (3) |
| `command_palette` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (6: action, command, id, limit, query, subcommand) |
| `content_browser_copy` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-ui-content` | same | every argument name found read in the handler crate (13: action, category, folder, index, name, open, path, search, source, sources_pane, tile, tile_fit, view) |
| `content_browser_navigation` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-ui-content` | same | every argument name found read in the handler crate (13: action, category, folder, index, name, open, path, search, source, sources_pane, tile, tile_fit, view) |
| `content_browser_open` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-ui-content` | same | every argument name found read in the handler crate (13: action, category, folder, index, name, open, path, search, source, sources_pane, tile, tile_fit, view) |
| `content_browser_selection` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-ui-content` | same | every argument name found read in the handler crate (3: action, dry_run, name) |
| `content_copy` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (4: dst, overwrite, recursive, src) |
| `content_create` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: folder, kind) |
| `content_delete` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: path, recursive) |
| `content_mkdir` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (1: path) |
| `content_move` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (4: dst, overwrite, recursive, src) |
| `content_rename` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (6: dst, name, overwrite, path, recursive, src) |
| `darf_copy` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: overwrite, path) |
| `darf_list` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (3: ext, filter, limit) |
| `delete_selected` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `deselect_all` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `drop_objects` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: align_normal, mode, names) |
| `exec_command` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | differs | `p2s` / `p2i` are never read (C++ `LevelMcp.cpp` passes P2 to `ExecCommand`); only the 48 `UiCommand`s are runnable, not the 118 `COMMAND_*` of the C++ registry; `id` is the Rust table's index, not the C++ enum value; arguments read (6) |
| `focus_object` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (2: name, select) |
| `frame_pacing` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (2: action, active_idle_fps) |
| `game_lighting` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (10: all, enabled, exposure, hour, indoor_hemi, list, rebake, reload, weather, weathers) |
| `game_link` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-game` | differs | a refusal carries `error` first; the C++ `EditorProject::McpGameLink` refusal had no `error` key (STATE.md); arguments read (1) |
| `game_modes` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-game` | differs | refusal shape as `game_link` (STATE.md) |
| `library` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-ui-previews` | same | every argument name found read in the handler crate (5: action, frame, item, kind, search) |
| `list_assets` | `LevelEditor/Editor/Mcp/LevelMcpContent.cpp` | `gz-project` | same | every argument name found read in the handler crate (3: category, filter, limit) |
| `list_commands` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | differs | lists the 48 `UiCommand`s (`gz-app/src/cmds.rs` `command_table`), the C++ lists every registered `COMMAND_*` (118) with its enum id |
| `list_objects` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: class, limit, name_contains) |
| `list_projects` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (0: none) |
| `list_spawn_sections` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-game` | same | every argument name found read in the handler crate (2: filter, limit) |
| `mark_authored` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: authored) |
| `mark_removed` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: removed) |
| `mod_catalog` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-release`, `gz-catalog` | same | registered twice: `gz-release` (key-only stand-in, first) and `gz-catalog` (the real one, kept); `-dump_commands` prints the first area |
| `mod_commands` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-mod` | same | every argument name found read in the handler crate (6: args, cheat, function, help, name, remove) |
| `mod_exclude` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-mod` | same | every argument name found read in the handler crate (2: excluded, path) |
| `mod_export` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-build` | same | every argument name found read in the handler crate (3: answer_only, flat, target) |
| `mod_export_cut` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-export-level` | same | every argument name found read in the handler crate (4: grow, level, overlap, selected_only) |
| `mod_export_ogf_probe` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-export-level` | same | every argument name found read in the handler crate (2: level, selected_only) |
| `mod_export_spawn` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-export-level` | same | every argument name found read in the handler crate (4: level, mode, section, selected_only) |
| `mod_export_xcform` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-export-level` | same | every argument name found read in the handler crate (3: level, sector, selected_only) |
| `mod_manifest` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-mod` | same | every argument name found read in the handler crate (0: none) |
| `mod_package` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-release` | same | every argument name found read in the handler crate (4: answer_only, out_dir, password, requires_game) |
| `mod_publish` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-publish` | differs | `action=start` answers once the packaging ended, in one call; C++ `EditorPublish::McpPublish` answered `job_pending` and the pump asked again with `answer_only` (still accepted); arguments read (11) |
| `mod_releases` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-publish` | same | every argument name found read in the handler crate (7: action, id, notes, show, tag, title, wait) |
| `mod_set_manifest` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-mod` | same | every argument name found read in the handler crate (20: after, author, before, conflicts, description, disable_game_updates, exclude, github, id, mode, name, provides_mode_id, provides_mode_title, requires, tags, target, thumbnail, vars, version, website) |
| `mod_settings_view` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-ui-mod` | same | every argument name found read in the handler crate (2: ask_game_updates, page) |
| `object_info` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: name) |
| `object_set_position` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (4: name, x, y, z) |
| `object_set_rotation` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (4: name, x, y, z) |
| `object_set_scale` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (4: name, x, y, z) |
| `open_project` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (1: path) |
| `open_scene` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-scene` | same | every argument name found read in the handler crate (2: discard, scene) |
| `outliner_filter` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-level` | same | every argument name found read in the handler crate (6: layout, selected_only, sort, text, types, visibility) |
| `outliner_show` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-level` | same | every argument name found read in the handler crate (1: open) |
| `pda_delete` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (1: path) |
| `pda_get` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (2: lang, path) |
| `pda_list` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (1: lang) |
| `pda_preview` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-ui-pda` | same | every argument name found read in the handler crate (18: action, answer_only, aspect, discard, id, key, lang, n, page, pane, path, row, save, tab, text, x, y, zoom) |
| `pda_use` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (1: path) |
| `pda_validate` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (3: lang, path, text) |
| `pda_write` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` | `gz-pda` | same | every argument name found read in the handler crate (5: id, path, text, title, use) |
| `ping` | `XrECore/Editor/GrizzlyMCP.cpp` | `grizzly`, `gz-app`, `gz-editor` | same | every argument name found read in the handler crate (0: none) |
| `place_object` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (14: align_normal, name, ref, rx, ry, rz, scale, snap_to_ground, sx, sy, sz, x, y, z) |
| `place_spawn` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (5: section, snap_to_ground, x, y, z) |
| `place_way` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (4: loop, name, points, snap_to_ground) |
| `preferences` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (5: action, key, page, search, value) |
| `preview_model` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-ui-previews` | same | every argument name found read in the handler crate (9: answer_only, bones, details, frame, lighting, name, reset, source, wireframe) |
| `probe_vertical` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (4: reach, x, y, z) |
| `progress` | `XrECore/Editor/GrizzlyMCP.cpp`, `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: action) |
| `project_browser_menu` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-ui-app` | same | every argument name found read in the handler crate (5: new_project, path, search, show, sort) |
| `project_close` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: discard, save) |
| `project_create` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: name, parent) |
| `project_delete` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (1: path) |
| `project_forget` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (1: path) |
| `project_reveal` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-project` | same | every argument name found read in the handler crate (2: dry_run, path) |
| `property_choose` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: action, item, key) |
| `property_inspector` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-scene`, `gz-ui-level` | same | every argument name found read in the handler crate (4: action, filter, limit, target) |
| `property_set` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: key, target, value) |
| `quest_apply` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: ops, path) |
| `quest_bookmarks` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` | `gz-nq`, `gz-ui-graph` | same | every argument name found read in the handler crate (3: action, node, path) |
| `quest_catalog` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (0: none) |
| `quest_check_all` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (0: none) |
| `quest_close` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp`, `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: discard, path) |
| `quest_edit` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` | `gz-nq`, `gz-ui-graph` | same | every argument name found read in the handler crate (6: from, nodes, op, path, pin, to) |
| `quest_find` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` | `gz-nq`, `gz-ui-graph` | same | every argument name found read in the handler crate (5: action, limit, path, query, select) |
| `quest_get` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_history` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` | `gz-nq`, `gz-ui-graph` | differs | adds `undo_steps` / `redo_steps`; an undo back to the saved state is clean again (C++ `UIQuestGraph::McpHistory` stayed dirty); arguments read (2) |
| `quest_layout` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: all, path) |
| `quest_list` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (0: none) |
| `quest_lookup` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (4: limit, path, query, type) |
| `quest_minimap` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` | `gz-nq`, `gz-ui-graph` | same | every argument name found read in the handler crate (2: action, path) |
| `quest_new` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_open` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq`, `gz-ui-graph` | same | every argument name found read in the handler crate (1: path) |
| `quest_project_find` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: limit, query) |
| `quest_redo` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_references` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (4: limit, path, type, value) |
| `quest_reload` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_rename_task` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (4: ack_runtime_identity, from, path, to) |
| `quest_save` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: force, path) |
| `quest_undo` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_validate` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (1: path) |
| `quest_view` | `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-ui-graph` | same | every argument name found read in the handler crate (17: action_split, answer_only, cx, cy, details, details_width, frame, graph_panel, graph_width, keys, path, problems, scroll, select, slot, vars_split, zoom_level) |
| `quest_write` | `XrECore/Editor/Nq/NqMcp.cpp` | `gz-nq` | same | every argument name found read in the handler crate (2: lua, path) |
| `recovery` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: action) |
| `redo` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `rename_object` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (2: name, new_name) |
| `reset_layout` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: scope) |
| `save_scene` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: file) |
| `scan_walls` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (9: rays, reach, res, step, x0, x1, y, z0, z1) |
| `scene_info` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `scene_load_status` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `scene_stats` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `scene_tree` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: class, filter, limit) |
| `scene_visibility` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-level` | same | every argument name found read in the handler crate (5: all, class, name, selection, visible) |
| `screens_get` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-screens` | same | every argument name found read in the handler crate (0: none) |
| `screens_set` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-screens` | differs | `action=drop` always fills the loading surface; C++ `EditorScreens::McpSet` used the tab's selected surface; arguments read (23) |
| `screens_view` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-ui-pda` | same | every argument name found read in the handler crate (8: layer, open, screen, simulate_drop, surface, track, x, y) |
| `screenshot_editor` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-app`, `grizzly` | differs | `busy` is always false and `outside` always `[]` (`gz-app/src/shots.rs`); the C++ answers the frame's busy state (`GrizzlyMCP.cpp`) - `outside` waits for panels outside the window (in work) |
| `screenshot_viewport` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (0: none) |
| `select_objects` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (1: names) |
| `selection` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `shortcuts` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (4: action, command, filter, keys) |
| `snap_objects` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (3: action, pick, use) |
| `spawn_order` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (9: distance, hostile, look, name, order, radius, route, run, target) |
| `spawn_set_fixed_bones` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-scene` | same | every argument name found read in the handler crate (2: bones, name) |
| `spawn_set_visual` | `LevelEditor/Editor/Mcp/LevelMcpProps.cpp` | `gz-scene` | same | every argument name found read in the handler crate (2: name, visual) |
| `state` | `XrECore/Editor/GrizzlyMCP.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (0: none) |
| `texture_preview` | `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-ui-previews` | same | every argument name found read in the handler crate (8: action, answer_only, channel, details, mip, path, source, zoom) |
| `theme` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: action, preset) |
| `titlebar_info` | `LevelEditor/Editor/Mcp/LevelMcp.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (0: none) |
| `ui_audit` | `LevelEditor/Editor/Mcp/LevelMcp.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: id) |
| `ui_contract` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (0: none) |
| `ui_dialog` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: action, id) |
| `ui_drawer` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: action, id) |
| `ui_list` | `LevelEditor/Editor/Mcp/LevelMcp.cpp`, `XrECore/Editor/GrizzlyMCP.cpp` (pump) | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (0: none) |
| `ui_panel` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: action, id) |
| `ui_popup` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: show) |
| `ui_scale` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: value) |
| `ui_scroll` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: id, to) |
| `ui_split` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (2: key, value) |
| `ui_tab` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (7: action, discard, id, kind, path, save, source) |
| `ui_toast` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `gz-app`, `gz-ui` | same | every argument name found read in the handler crate (1: text) |
| `undo` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-scene` | same | every argument name found read in the handler crate (0: none) |
| `view_mode` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (14: edged_faces, environment, fill, filter_linear, fog, game_lighting, grid, helpers, lighting, preset, safe_rect, shade, stats, textures) |
| `viewport_controls` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (10: angle, angle_snap, grid, grid_snap, normal_align, object_snap, scale, scale_snap, tool, vertex_snap) |
| `viewport_navigation` | `LevelEditor/Editor/Mcp/LevelMcpScene.cpp` | `gz-ui-viewport` | same | every argument name found read in the handler crate (3: action, scope, view) |
| `window_control` | `LevelEditor/Editor/Mcp/LevelMcp.cpp` | `grizzly`, `gz-app`, `gz-editor` | same | every argument name found read in the handler crate (5: action, discard, height, save, width) |
| `world_get` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-world` | same | every argument name found read in the handler crate (0: none) |
| `world_set` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-world` | differs | writes only the section it changed (C++ `EditorWorld::McpSet` -> `Save` rewrote mod.ltx); an empty declared `[start_kit]` is written as `perks = true`; arguments read (22) |
| `world_transitions` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-world` | same | every argument name found read in the handler crate (3: level, limit, patrols) |
| `world_view` | `LevelEditor/Editor/Mcp/LevelMcpMod.cpp` | `gz-ui-world` | same | every argument name found read in the handler crate (5: drawer, filter, level, more, open) |


### 6. Dialogs, popups, document kinds, panels

`XrECore/Editor/EditorUiContract.h` against `crates/gz-app/src/contract.rs`: every dialog,
panel, drawer, document kind, popup, menu and card-menu label of the C++ contract is in the Rust
one (the only C++ strings not found are format placeholders); Rust adds the palette window and
the theme / list-category names.

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| every `dlg.*` of the C++ (34: `assign_shortcut`, `bake_result`, `build_result`, `build_to_folder`, `catalog_submit`, `catalog_withdraw`, `choose`, `clean_target`, `delete_character`, `delete_object`, `delete_project`, `detail_objects`, `disable_game_updates`, `edit_release`, `edit_text`, `empty_world`, `export_flat`, `export_result`, `game_type`, `import_base_scene`, `nq_overwrite`, `nq_project_find`, `nq_reload`, `nq_task_references`, `nq_task_rename`, `nq_text`, `numeric_vector`, `package_release`, `package_result`, `properties`, `publish_release`, `rename_object`, `replace_release`, `text_editor`) | | the same ids (`rg 'dlg\.'` finds each in `crates`); Rust adds `dlg.notice`, `dlg.publish_result` | same |
| `ui_dialog open` ids `package_release`, `build_to_folder`, `export_flat`, `import_base_scene`, `publish_release`, `submit` (`LevelMcp.cpp` `OpenDialog`) | | `contract.rs` dialog table + the owning areas | same |
| Confirmations `clean_target`, `clear_detail_objects`, `clear_snap_list`, `delete_character`, `delete_object`, `discard_recovery`, `empty_world`, `disable_game_updates`, `nq_overwrite`, `nq_reload`, `pda_revert`, `reload_light_animations`, `replace_release`, `sync_sounds`, `sync_textures`, `delete_release` | | same ids | same |
| Confirmation `clear_colour_indices` (`UIDOShuffle.cpp`) | | `clear_color_indices` | differs |
| Confirmations `rename_objects`, `reset_surfaces`, `detach_spawn_objects`, `rewrite_thumbnails` | | - (their actions are missing, see section 1 and the list) | missing |
| Confirmations `make_all`, `make_game`, `make_details`, `make_hom`, `make_som`, `make_aimap` (`ConfirmLevelBuild`) | | - (the builds run unasked) | differs |
| Confirmations `clear_ai_map`, `clear_ignored_materials`, `remove_default_sector`, `remove_duplicate_portals`, `detach_shapes` | | the AI Map / Sector / Portal / Shape tools | in work |
| Document kinds `level`, `quest:<rel>`, `beh:<rel>`, `pda`, `model`, `texture`, `characters`, `mod_settings`, `world_rules`, `releases`, `preferences`, `light_animations`, `texture_library`, `sound_library`, `screens` | | `gz-app` + the areas' `DocDesc`s | same |
| Level panels `tools`, `world_outliner`, `details`, `level_settings`, `content_browser`, `output_log`, `viewport`; drawers Content / Log | | `gz-ui-level/src/lib.rs`, `gz-ui-content`, `gz-ui-app/src/log.rs` | same |

### 7. Properties (Details, Level Settings, the tools' sheets)

Every `PHelper().Create*` / `PrepareKey` label under the four folders was looked for in the Rust
source (props of `crates/gz-scene/src/props.rs` and the areas).

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| `CCustomObject::FillProp` (`CustomObjectLE.cpp`: name, transform, flags) | | `gz-scene/src/props.rs` | same |
| `CSceneObject::FillProp` (Reference chooser, Surfaces: texture / shader / compiler shader / game material choosers, Clear Surface) | | `props.rs` | same |
| `CCustomObject::AnimationFillProp` (dynamic objects: Motionable, Motion\*) | | - | missing |
| `CSpawnPoint::FillProp` (30 labels: entity, Profile (spawn section), env mod, respawn point, player start, `[xms_order]` rows, attached shape) | | `props.rs` | same (labels) |
| `CSpawnPoint::FillProp` choosers Order\Look path, Order\Route, Order\Target, Respawn Point\Profile (`smCustom`) | pick from lists | typed text (`ListKey::of_choose` -> None) | differs |
| `CLight::FillProp` (`ELight_props.cpp`, 23 labels), `CGlow::FillProp`, `ESoundSource::FillProp` (17), `ESoundEnvironment::FillProp`, `CEditShape::FillProp`, `CSector::FillProp`, `CPortal::FillProp`, `CWayPoint::FillProp`, `CGroupObject::FillProp`, `EParticlesObject::FillProp` | | `props.rs` | same |
| `CGlow` "Fixed size", `ESoundSource` "Looped", `ESceneSpawnTool` Common\Show Spawn Type / Trace Visibility | | - | dropped (not reachable: commented out in the C++) |
| Level Settings: `EScene::FillProp` (`scene.cpp`, 14 labels), the Quality segmented, tool sheets of Light (22), Glow, Sector, Portal (`ESceneLightTools.cpp` ...) | | `gz-ui-level` Level Settings over `props.rs` | same |
| `ESceneFogVolumeTools::FillProp` VolumeType / profile (ltx) | | (Fog Volume tool) | in work |
| `ESceneDOTools::FillProp` (Base Texture, Common\Draw *), `ESceneWallmarkTool::FillProp` (Alignment, Draw Wallmarks), `ESceneAIMapTool::FillProp` | | (next package) | in work |
| Random Append sheet (`ESceneObjectTools.cpp`: Scale, Scale\Proportional, Minimum / Maximum, Rotate, Rotate\Minimum / Maximum, Objects) | | `RandomSettings` dialog: the four ranges only | differs |
| Scene summary sheet (`SceneSummaryInfo.cpp`) | | - | missing |
| Texture Library `Cube Map\Smaller Copy` (`UIImageEditorForm.cpp`) | | - | missing |
| Texture / Sound / Light Animation library sheets (`UIImageEditorForm.cpp`, `UISoundEditorForm.cpp`, `UIEditLightAnim.cpp`), the image preview's rows (`UIImagePreview.cpp`: Slices is shown for arrays only) | | `gz-ui-previews` | same |
| Detail object rows Min / Max Scale, Density, No Waving (`UIDOShuffle.cpp`) | | `gz-ui-level/src/detail_objects.rs` ("Minimum Scale", "Maximum Scale", "Density Factor") | same |
| `CEditableObject::FillBasicProps` (`EditObjectProps.cpp`), `PDomain::FillProp` (`pDomain.cpp`), the game material editor (`XrGameMaterialLibraryEditors.cpp`) | | - | dropped (not reachable from the level editor: `ActorEditor` / `ParticleEditor` / `ShaderEditor`, section 4) |
| Spawn entity rows (`ISE_Abstract::FillProps` of the Dead Air classes) | | `gz-entities` typed properties | unjudged |

### 8. Preferences, the command line, the editor's own files

| C++ item | What it does | Rust counterpart | State |
|---|---|---|---|
| `EditorPreferences.cpp` `kPrefHelp` rows (40: General, Viewport, Tools, Objects) | | `gz-ui-app/src/prefs.rs` over `gz-app/src/settings.rs` `PREFS` | same |
| `LevelPreferences.cpp` Recovery\Enabled / Interval / Delay After Input / Snapshots Kept | | `prefs.rs` Recovery page | same |
| `LevelPreferences.cpp` Scene\Targets\Enable\<class>, Scene\Targets\Read Only\<class> | | - | missing |
| Keyboard page (`CCustomPreferences::DrawKeyboard`, `UIKeyPressForm`) | | `gz-ui-app/src/keyboard.rs`, `dlg.assign_shortcut` | same |
| `-project`, `-nodlg`, `-qa_github`, `-iconsheet` | | `grizzly/src/cli.rs` | same |
| `-uiscale <f>` (`XrUIKit.cpp` `CommandLineScale`) | | - (`ui_scale` over MCP only) | missing |
| `-trace`, `-heapcheck`, `-no_ffconst`, `-no_rebind`, `-build` / `-ebuild` (`EDX11Utils.cpp`, `ui_main.cpp`, `device.cpp`) | DX11 device debugging and texture loading | - | dropped (D2: the DX11 / R1 device is gone) |
| (none) | | `-headless`, `-mcp_port`, `-root`, `-log_echo`, `-dump_commands` (Rust only) | same (addition) |
| `level.ini` (settings, `[shortcuts]`), `level_imgui.ini` | | `grizzly_editor.ltx` | differs (D3: new files, nothing migrated) |
| `_projects.ini`, `project.ltx`, `<project>\temp\session.ltx`, recovery snapshots, `preview.png`, `releases\deleted.tags`, `Documents\Grizzly Editor\catalog.ltx` | | `gz-project`, `gz-app/src/shell.rs`, `gz-scene` recovery, `gz-publish`, `gz-release/src/keys.rs` | same |
| the picker cache `nq_index_*.txt` | | in-memory `Query`s; Rust adds `<project>\temp\viewport.ltx`, `appdata\game_index` | differs (STATE.md: not ported, by design) |
| Startup: splash window on its own thread, stage profile, loader line, code page line (`EditorStartup`) | | profile and lines in `grizzly/src/startup.rs`; no splash | differs |

### 9. Stubs in the Rust tree

`rg -n -i "not ported|not yet|later package|not implemented|unsupported here|no settings of its own|TODO|FIXME" crates -g '*.rs'`:

| Hit | Judgement | State |
|---|---|---|
| `gz-characters/src/lib.rs:8`, `gz-world/src/lib.rs:7` "a later package" | only words: the tabs exist (`gz-ui-world`) | same |
| `gz-levelbuild/src/validate.rs:10` "Not ported: the portal test, sector / wallmark shaders, LOD notes, sound library" | real gap, already known | in work |
| `gz-fmt-ogf/src/export.rs:648` "VIPM is not ported" | real gap, already known | in work |
| `gz-scene/src/tools.rs:854` "the Wallmark tool's editing tools are a later package" | real gap, already known | in work |
| `gz-scene/src/props.rs:1350` fog volume token "for a later package" | real gap, already known | in work |
| `gz-ui-level/src/tools.rs:180` "This tool has no settings of its own." | real for Sector, Portal, Way, Shape, Light, Fog Volume (in work), AI Map, Wallmark (next); right for Sound Source, Sound Environment, Glow (the C++ has no panel for them either) | in work |
| `gz-github/src/github.rs:140` "Not yet: poll again"; `gz-nq/src/service.rs:554` `todo`; `gz-ui-viewport/src/lighting.rs:24` "not yet scaled" | only words | same |

## Unjudged

| Item | Why |
|---|---|
| The answers of the MCP commands beyond key names (types, values, order, error texts) | a running comparison against the C++ editor is outside this read-only brief |
| The spawn entities' property rows (`gz-entities` against the Dead Air `FillProps`) | a class-by-class compare of `xrServerEntities` was not done here |
| Read-only versus editable per Details row, and the chooser kinds inside entity rows | only labels and the scene's `choose` lists were compared |
| Viewport gestures past the key names: box selection with Limited Depth / Back Face Culling, gizmo sensitivities, vertex snap behaviour | needs the editor running |
| Graph canvas gestures and the Content Browser's drag and drop | labels match; behaviour needs the editor running |
| Tooltips and help texts | not compared |

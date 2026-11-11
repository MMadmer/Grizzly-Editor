# Brief: what the shell, the menus and the libraries still lack

`docs/rust/PARITY.md` is a read-only audit of the C++ editor against the Rust one; its top
section, "The list that matters", names every function still missing. This package closes the
part of it that lives in the shell and around it. (The scene's own gaps - clipboard, selection
files, tool panels, level parts - are other packages: do not start them.)

Read `docs/rust/CONVENTIONS.md`, `docs/rust/STATE.md` and the top of `docs/rust/PARITY.md`
first (each line there names the C++ file and function: that is your specification);
`docs/UI_DESIGN.md` sections 1, 1a, 2, 4, 5 and the surfaces of section 6 you touch; the
rustdoc at the top of `gz-app`, `gz-ui`, `gz-ui-app`, `gz-ui-mod`, `gz-ui-graph`,
`gz-ui-previews`, `gz-ui-viewport` (`menus.rs`, `service.rs`), `gz-editor`.

You own: `crates/gz-app/**`, `crates/gz-ui-app/**`, `crates/gz-ui-mod/**`,
`crates/gz-ui-graph/**`, `crates/gz-ui-previews/**`, `crates/gz-screens/**`,
`crates/gz-fmt-tex/**`, `crates/grizzly/**`, in `crates/gz-ui-viewport` only `src/menus.rs` and
`src/service.rs`, in `crates/gz-ui-level` only `src/dock.rs` and `src/layout.rs`, and new
entries of `crates/gz-mcp/tools.json`. Two other agents work in the tree at the same time (the
scene's object tools: `gz-scene`, `gz-ui-level/src/tools*.rs`, the viewport's gestures; the
renderer: `gz-render`, the viewport's `draw*.rs`): touch nothing of theirs; re-read a shared
file (`tools.json`, `crates/grizzly/src/main.rs`) before a small edit; if `cargo check` fails in
a crate that is not yours, wait and retry.

## 1. The command table (`list_commands`, `exec_command`)

The C++ registry has 118 commands (`REGISTER_CMD_*` in
`Source/Editors/XrECore/Editor/UI_MainCommand.cpp` and
`Source/Editors/LevelEditor/Editor/UI_LevelMain.cpp`, the ids in their headers'
enums); the Rust table has the 48 `UiCommand`s and answers its own index as `id`.
- The table lists every C++ command that has a user function **under its C++ name and its C++
  numeric id** (a script written against the C++ editor keeps working), with its description,
  its sub-commands and its current binding as `LevelMcp.cpp` `GetEditorCommands` answers them.
  Commands that are plumbing of the C++ shell (caption, toolbar and property refreshes, resize,
  destroy, initialise) are not listed: name them once in the crate's rustdoc.
- `exec_command` reads both parameters (`p1s` / `p1i` / `p2s` / `p2i`) as `LevelMcp.cpp` does
  and hands them to the command.
- **Other areas add their commands through a slot** (the scene's commands arrive in a later
  package): `gz-app` offers the registration (C++ id, name, description, sub-commands, default
  key, the action as an editor command name with arguments) and fills in everything the shell
  and your crates own. A command of the C++ table whose function does not exist in the Rust
  editor yet is NOT listed as if it worked: leave it out, and list the left-out names in
  STATE.md so the next package registers them.
- A registered command is a `UiCommand` too: in the palette, on the Keyboard page, bindable.

## 2. Macros

Run Macro slots 1-8 (Alt+1 .. Alt+8), Assign Macro, Execute Command List, Log Commands:
`UI_MainCommand.cpp` `CommandRunMacro` / `CommandAssignMacro` / `CommandExecuteCommandList` /
`CommandLogCommands`, with the file format and the preference keys the C++ uses. A macro runs
its commands through the same `exec_command` path, off the UI thread, and stops at the first
refusal with the command named.

## 3. The window and the start

- `-uiscale <f>` (`Source/Editors/XrEUI/XrUIKit.cpp` `CommandLineScale`).
- The Start page's status bar shows the Level tab's centre ("0 selected - Grid 0.1"): the
  status centre belongs to the active document; the Start page has none.
- `screenshot_editor` answers `busy` and `outside` for real: `busy` as `GrizzlyMCP.cpp` defines
  it (read the handler), `outside` the windows the picture does not hold.
- **Panels outside the window.** Preferences > General > Allow Panels Outside the Window
  (`[ui] floating_panels`, takes a restart, off by default) exists in the C++ editor and is
  missing here. With it on, a panel of the Level tab's dock can be dragged out into a native
  window of its own (egui viewports: implement what the own winit / wgpu loop in
  `gz-app/src/window.rs` needs for a second native window - its surface, its input, its
  repaint), comes back when closed, and its place is remembered in the layout. With it off
  nothing changes. The rules of `CLAUDE.md` hold either way: a modal always lives in the main
  window; `screenshot_editor` pictures the main window and names the others in `outside`; a
  minimised editor is pictured restored. `ui_panel` can move a panel out and back
  (`floating=true|false`), so it is reachable without a mouse.
- No splash window: the Rust editor's own window is up before a splash could be (the startup
  budget is 0.3 s). Nothing to build - it is recorded as a deliberate difference by the lead.

## 4. The View menu's render options (`UIMainMenuForm::DrawViewportOptions`)

- View > Render > GPU: the adapters by name, System Default, the driver hints; the choice is a
  preference (the C++ key: `CLevelPreferences::GpuAdapter`) used when the device is created
  (`gz-app` asks wgpu for that adapter; a restart applies it, and the row says so).
- View > Render > Quality 25 / 50 / 100 / 200 %: the viewport's render size scaled, kept in the
  preferences as the C++ keeps it (`UI->SetRenderQuality`).
- Fill Mode (Point / Wireframe / Solid), Shade Mode, Edged Faces, Linear Filtering, Textures,
  Fog, Environment: they are `view_mode` arguments already and are missing from the View menu
  and the Options pill - add the rows, radio groups under one folded More where 1a says so.
- Mute Sounds and Real Time (`rsMuteSounds`, `rsRenderRealTime`, `CommandMuteSound`): rows,
  commands, preferences; Real Time makes the viewport draw every frame instead of on demand.
  The viewport's MCP answer (`viewport_controls` / `view_mode`) reports and sets all of them.

## 5. Libraries

- Libraries > Reload Objects (`UI_LevelMain.cpp` `CommandReloadObjects`): the scene's object
  library is dropped and read again (`gz_scene::library::Library::clear` exists; call it
  through whatever the scene service offers - if it offers nothing, say so in the report and
  register the menu row disabled with the reason, the scene package will wire it).
- Libraries > Reload Textures (`UI_MainCommand.cpp` `CommandReloadTextures`): the renderer's
  and the previews' texture caches drop what they hold and read again.
- Libraries > Rewrite Texture Descriptions... (`UIMainMenuForm.cpp` `RewriteThumbnails` /
  `RewriteThumbnailsNow`, confirmation `rewrite_thumbnails`): a job over every `.thm`, the kit's
  confirm naming how many files are rewritten.
- Libraries > Refresh Sound Environment Geometry (`CommandRefreshSoundEnvGeometry`).
- Texture Library, Cube Map \ Smaller Copy (`Source/Editors/XrECore/Editor/UIImageEditorForm.cpp`).

## 6. Mod

- **Mod > Bake Scene Layer** > Spawn Layer..., Collision Overlay..., Visual Overlay..., Level
  Cut...: the dialogs of `EditorModScene::RequestExportSpawn` / `RequestExportXCForm` /
  `RequestExportOGF` / `RequestExportCut` (`Source/Editors/XrECore/Editor/EditorModScene*.cpp`,
  `UIMainMenuForm.cpp` `MenuMod`), filling the `BAKE_SCENE_LAYER` slot of
  `gz-ui-mod/src/menu.rs`. They are faces of the commands that exist (`mod_export_spawn`,
  `mod_export_xcform`, `mod_export_ogf_probe` / the visual overlay, `mod_export_cut`): a button
  posts one, the result is the kit's Result dialog (`dlg.bake_result`).
- **The Mod Settings picture** waits for Save: a chosen picture is kept in the project's
  `temp\` until Save, and Remove deletes only the editor-made `thumbnail.dds`, on Save
  (`Source/Editors/XrECore/Editor/EditorModManifest.cpp`: `s_ThumbPending`, `s_ThumbRemove`,
  `PickThumbnail`; `CLAUDE.md`, "The Mod Browser"). Today both happen at once.
- `screens_set action=drop` fills the surface the Screens tab shows when the tab is open
  (loading otherwise), as `EditorScreens.cpp` `McpSet` does; an explicit `surface=` wins.

## 7. Quests

The camera button of a position (`Source/Editors/LevelEditor/UI/QuestGraph/NqInspector.cpp`
`NqInspector::DrawPosition`): it fills the position and the level from the viewport's camera.
`gz-ui-graph` must not depend on the viewport crate: read the camera through a small slot in
`gz-editor`'s services that the viewport fills (if the viewport does not fill it yet, add the
slot and its type here and say so - the scene package fills it).

## Rules

- Every action is a command reachable over MCP; a new command or argument gets its entry in
  `crates/gz-mcp/tools.json` in the catalogue's style, nothing existing changes.
- Draw only with the kit; a widget it lacks is `local_<name>`, named in the report. A frame
  never computes and never touches a file. No `unsafe` (anything Win32 a second window needs
  goes through `winit`), no global state.
- Tests are written, not run.
- Compile: `bash tools/rust/cg.sh check -p gz-app -p gz-ui-app -p gz-ui-mod -p gz-ui-graph -p
  gz-ui-previews -p gz-screens -p gz-ui-viewport -p grizzly`, zero warnings; clippy clean for
  your crates. Then build and LOOK, on a temp COPY of `D:\XFinedProjects\Test`: the View menu
  with its render rows, a panel moved out and back over `ui_panel` with the preference on (the
  picture and `outside`), a Bake Scene Layer dialog, the Start page's status bar,
  `list_commands`; `ui_audit` shows nothing escaped.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. Never start `Grizzly.exe` by hand.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 450 words: every numbered item as done / partly / not done with the reason; the C++
commands left out of the table for the scene package to register; `local_*` widgets; what the
look showed. Then add a section "The shell's rest" to `docs/rust/STATE.md` and mark the lines
of `docs/rust/PARITY.md`'s top list you closed (`closed -` in place of `missing` / `differs`).

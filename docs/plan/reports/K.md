# Report K - the kit and the shell

## 1. Built

**The kit (`gz-ui`): one widget per job, the new look, `-kitsheet`.**
- New widgets, each the only one of its job: `Section` (`section.rs`), `Check` and `Switch` (`check.rs`), `Combo` and `PickList` (`pick.rs`, ranking in `rank.rs` moved from the palette: whole-word prefixes, then letters in order; virtualized rows, icon + family tint, second line on the highlighted row, Recent and Favourites with a star, full keyboard, measured once; it lays itself out vertically and allocates its rows' height in any layout), `ItemRows` and `Table` (`rows.rs`), `banner_list` (`state.rs`), `drop` (`Payload`, `Zone`), `text::fit` (cache per text, face, width), `upload::Tiled` (a big picture a few tiles a frame), `toast_with` (a note with one button that runs an action; `take_toast_actions` for the shell).
- The look: tokens `shadow.popup/dialog/card`, `graph.family.0..9` (`Palette::family`), `wire.*`, `pin.*`, `frame.fill/title`; inputs outlined (`border.subtle`, `border` on hover, focus ring), frame 26 pt; Secondary hairline; toggle = Ghost + 2 pt underline; one Primary per dialog footer / toolbar asserted in debug (`button::note_primary`); menus reserve ONE icon column per popup, checks take the icon's place, muted right-aligned shortcuts, 200 pt minimum; toolbar `group(priority)` folds labels then overflows, lowest priority first; pages of cards up to 760 pt.
- `kitsheet.rs` + `-kitsheet` (grizzly `cli.rs` / `main.rs`, `ShellOptions.kitsheet`); the sheet scrolls over `ui.scroll id=work_area`.
- `testkit.rs` and `tests.rs`: 53 kit tests.

**Actions (`gz-app`'s `actions.rs`, `shell_actions.rs`).** `Action` = id, title, icon, command + args from `ActionCx`, default keys, group, `Scope` (Global / Tab / Part), enabled with reason, checked, reply hook, palette flag, frame key. `ShellBuilder::action / menu_action / submenu_action`; the table is an `Editor` service (`Actions`) so surfaces draw rows from it (`actions::menu_row`, `actions::toolbar_button`, `actions::keys_label`). Menus, toolbar rows, keys and the palette all run actions through `Editor::call` with the UI origin.
- Migrated: the shell's own (File, Edit, Window, Help, drawers, tabs), Mod menu (`gz-ui-mod` menu.rs, bake.rs, new `mod.build`), Releases / Publish / Submit (`gz-ui-release`), Characters / World Rules (`gz-ui-world`), Screens & Music, PDA Interface (new `pda.open`), the Libraries menu (`library.*` actions; new `library.sync`, `library.reload`, `library.rewrite_descriptions`), the graph's keys (`gz-ui-graph`'s `actions.rs`, group Graph, `graph.action what=`; its Graph menu, More menu, toolbar tips and Shortcuts card read the bound keys), the updater's Help row (`help.check_updates`, `help.update`).
- Keys: text field, then actions (a tab's / part's before global, only one that is on; frame keys only while a key-owning panel is focused), the document's `shortcut`, the level table. `app.keys action=resolve` answers what a chord runs, through the same lookup.

**Typed shell commands** (`cmds.rs`, `app_cmds.rs`, `shots.rs`, grizzly `core_cmds.rs`): `state`, `ping` (version from the build), `app.window`, `app.quit`, `app.switch_project`, `app.about`, `app.prefs`, `app.keys` (list, bind, clear, reset, resolve), `doc.list/save/undo/redo/reload/edit`, `ui.tab` (`doc=`), `ui.list`, `ui.frame`, `ui.panel`, `ui.drawer`, `ui.dialog`, `ui.popup`, `ui.toast` (+ `action=`, `label=`), `ui.scroll`, `ui.split`, `ui.audit`, `ui.scale`, `ui.contract`, `ui.reset_layout`, `ui.reveal`, `ui.palette` (state, query, open, close, run), `ui.tasks`, `shot.window`, `perf.stalls`. pt both ways; pictures px with `scale`; `no_window` headless. Headless `window_control` and `jobs` are gone (`app.quit`, `job.list`).

**Shell work.**
- Stall monitor (`stalls.rs`, window timing of UI passes and events, labelled queued closures, first frame exempt, `[stall]` log line, `perf.stalls`).
- Status line: `gz-log` `Level::Detail` + `reports()`, the last report kept as a field on push; profile lines (`startup.rs`, gz-build stage line) are details; a click on the line opens the Output Log; the Start page shows no drawer buttons.
- Jobs: `gz-task` `Owner {Editor, Project(rev)}`, owner probe, `stop_orphans` (closing a project stops its jobs); `job.list` shows `owner`. Task Center: last five finished rows with their result, a history chip, the card the author opens stays (`show_card`; the self-opened one still goes 4 s after the last job).
- Stable chrome: tool tabs fold to icons regardless of the active tab; the project's group on every tab; Releases checked on its tab; an empty toolbar row shows `toolbar_status` (Preferences, Mod Settings "Saved to mod.ltx as you edit"); every tab kind and Level panel its own icon (Mod Settings = Package; Tools = Place, World Outliner = SceneObject).
- Palette: actions + tabs (project-relative paths) + panels + level commands, ranked by `gz_ui::rank`, category column at one x on every row, recent first (8, `palette.recent`), ranked on query change.
- Preferences: pages of cards, Interface page (Scale Auto/100..200 `[ui] scale`, Accent), `PrefGroup` with `PrefKind::{Setting, Switch, ReadOnly, Button}` and a group `note`; the updater's group moved onto it (U's hook removed); A2 adopted it for Game / Library / Project Files.
- Keyboard page (`keyboard.rs`, over `app.keys`): grouped, virtualized, "Press keys..." recording, cross clears, conflicts named, Reset per row and all.
- Toasts with a button (U's "Details" / "What's New"; the What's New chip stand-in is gone). `contract.rs` DIALOGS has `update`.
- `cpp.rs` trimmed to the level's rows, C++ wording gone, marked "leaves with the level's actions (stream L2)"; macro file stats moved to the IO pool (audit D row 5).

**Renames finished**: UI callers in gz-ui-mod, gz-ui-world, gz-ui-content, gz-ui-graph, gz-ui-pda, gz-ui-previews; QA names and parameter names in 30 scripts of `tools/qa`, `tools/mcp`, `tools/rust/run-editor.ps1`; `test_properties.py` phase E on `app.keys`; NQ_ARCHITECTURE / PDA_FORMAT mentions. `tools/qa/test_ui_shell.py` rewritten (phases A-J). `docs/UI_DESIGN.md` sections 1-5, 7, 8 rewritten, section 6 fixed where my work made it false.

## 2. Decisions

- The action table is an `Editor` service with the last frame's facts (`Facts`) so surfaces' own menus answer enabled/checked without the shell.
- Tool tabs fold independently of which tab is up: a switch never moves a tab (the active folded tab names itself in its tip). The project's group stays on Preferences too: nothing moves.
- `app.keys resolve` instead of synthetic key input: proves a binding through the same lookup the key pass uses.
- `PrefKind::Button` / `Switch` take plain fn pointers (`fn(&Arc<Editor>)`) - groups stay `Copy` static data.
- A toast's button runs an action id, never a closure: the kit holds no editor state.
- QA starts its editors under `-qa_github` with every service on 127.0.0.1:9, so opening Releases contacts nothing.
- The level's scene save rows are named Save Scene / Save Scene As on the Keyboard page (the action `file.save` is Save).

## 3. Contract changes

- `docs/UI_DESIGN.md` 1-5, 7, 8 (new tokens, the kit table, actions, keys, palette, Preferences groups, Keyboard page, stall monitor, Task Center, toasts with a button, section 7 command table).
- Commands: see section 1; removed `window_control`, `titlebar_info`, `ui_*`, `command_palette`, `preferences`, `theme`, `shortcuts`, `progress`, `screenshot_editor`, `reset_layout`, headless `jobs`.
- `ui.list` + `toast_notes`, popups' `label_x`; `ui.frame` replaces `titlebar_info` (pt, `scale`); `ui.tasks` answers `running` / `finished` (was `progress` `background`).
- `ShellBuilder::pref_group` rows are `PrefRow { label, help, kind: PrefKind }`, `PrefGroup.note`.

## 4. Requests

- **L2 (level actions)** - left for it exactly: `gz-ui-level` `menus.rs` `SCENE_COMMANDS` (UiCommands COMMAND_UNDO/REDO, SELECT_ALL, DESELECT_ALL, INVERT_SELECTION_ALL, DELETE_SELECTION, HIDE_*, UNHIDE_ALL, SHOW_OBJECTLIST, VALIDATE_SCENE, SHOW_SCENE_SUMMARY, LOAD/SAVE_SELECTION, MULTI_RENAME_OBJECTS, CLEAR_DEBUG_DRAW), its `level_cmds` `cpp_command` rows (COMMAND_CLEAR, LOAD, LOAD_FIRSTRECENT, SAVE with Save As, ...), the Level toolbar's buttons; `gz-ui-viewport` `keys.rs` bindings (GIZMO_MODE, CHANGE_ACTION, ZOOM_EXTENTS, TOGGLE_RENDER_WIRE/GRID/SAFE_RECT, GRID_SLOT_SIZE, DROP_TO_GROUND, ...); in `gz-app`: `cpp.rs` (known_rows 4, 23, 26, 30, 31, 34, 47, 48, 61, 93-99, 109, 113; shell_rows 19, 86, 111, 112, 33; id 123), `cpp_cmds.rs` (`list_commands`, `exec_command`), `macros.rs` (50-53), `UiCommand`, `ShellBuilder::command / menu_item / cpp_command / cpp_subs`, the level faces of `key_rows`, key step 4. Also: Details does not report its scroll with nothing selected (`ui.scroll id=details` answers "not on screen").
- **Q (documents)**: CLAUDE.md, `tools/mcp/AI_SETUP.md`, `docs/RUST_MIGRATION.md`, `docs/rust/STATE.md` / `PARITY.md` still name the old commands (`screenshot_editor`, `ui_tab`, `titlebar_info`, `window_control`, `ui_*`, `progress`, `shortcuts`).
- **Suite owners**: names and parameter names are renamed; these read answer fields that changed (px rects with `dpi` -> pt with `scale`, `progress.background` -> `ui.tasks.running`, `ui_scale` -> `scale`): `bench_build.py`, `test_build_contents.py`, `test_build_job.py`, `test_characters.py`, `test_graph_ui.py`, `test_pda.py`, `test_previews.py`, `test_world.py`, `test_ui_tour.py`. `test_build_job.py` phase B now stops its build with `job.stop` (was `progress action=cancel`). U's untracked `test_update.py` got one rename (`app.quit`).
- **Architect (`gz-editor`)**: a headless `project.close` does not stop the project's jobs - only the window's `sync_session` calls `Jobs::stop_orphans`; `Editor::set_project` should.
- **Surface streams**: adopt the kit's one widgets (UI_DESIGN section 8 lists every copy); the PDA's sheets (`gz-ui-pda` `assets.rs`) should upload through `upload::Tiled` (audit D row 12); Mod Settings' footer status now repeats its toolbar row.
- **A1**: clippy in its hunks - `gz-ui-graph` `tab.rs` `conflict_line` after the tests module (`items_after_test_module`), `gz-ui-world` `view_cmd.rs` `Step` (`large_enum_variant`).

## 5. Proof

- `bash tools/rust/cg.sh check --workspace` - clean.
- `cg.sh clippy --all-targets` on gz-ui, gz-app, gz-ui-app, gz-log, gz-task, grizzly and the touched surfaces - no warning in my code (the ones left: gz-asset, gz-characters, gz-pda, A1's two above). `cg.sh fmt` on my crates; rustfmt on my surface files.
- `GRIZZLY_TEST_PROJECTS= bash tools/rust/test.sh -p ...` (13 packages) - 238 tests green: kit (PickList ranking and keys, Section fold, ItemRows add/remove/reorder, one-Primary assert, menu label column, `text::fit` cache), actions, palette, tabs (fold regardless of the active tab), tasks, stalls, gz-log levels, gz-task owners, every card worth reading. `check_test_files.py` exit 0.
- `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`, then `GRIZZLY_QA_PORT=28941 python tools/qa/run_all.py test_ui_shell` - **PASS, 14.2 s** (phases A-J; the first run after a fresh build failed only the 0.3 s window budget at 1.04 s: the scanner meeting the new binary; the next run 0.14 s).
- Pictures in `Intermediate/plan/K/`, each looked at:
  - `kitsheet_10_a.png` / `_b.png` - every kit widget and the tokens at scale 1.0.
  - `kitsheet_15_a.png` / `_b.png` - the same at 1.5.
  - `frame_level.png` - Level: the tab row, the project's group, the status line.
  - `frame_graph.png` - a quest tab: toggles underlined, a toast with its Details button.
  - `frame_mod_settings.png` - Mod Settings with its own icon and its toolbar status.
  - `frame_preferences.png` - General: cards, Project Files, Updates with its This Build note.
  - `frame_keyboard.png` - the Keyboard page grouped with keycaps.
  - `frame_palette.png` - the palette: icons, category column at one x, keys.
  - `frame_menu_file.png` - the File menu with one label column.
  - `frame_task_center.png` - the Task Center opened with only a finished job; tool tabs folded to icons.

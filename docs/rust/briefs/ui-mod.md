# Brief: Mod Settings and the build dialogs (`gz-ui-mod`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md`
sections 1, 1a, 2, 4, the Mod menu and the toolbar in section 5, and, in section 6, "Mod
Settings", "Build dialogs" - **the contract**; `CLAUDE.md` sections "Game modes" (a mode is not
the mod), "Where a mod exports to", "What a build ships" (the result dialogs, the clean-target
question, Build Contents), "The Mod Browser" (thumbnail and tags, the key line), "Screens and
music" (Disable Game Updates and its dialog); the rustdoc at the top of `gz-ui`, `gz-app`
(`ShellBuilder`: documents, menu items, `project_toolbar`, dialogs, `TaskCardTable`), `gz-mod`,
`gz-build`, `gz-release`, `gz-game` (modes, installed modules), `gz-project`, `gz-editor`.

You own: `crates/gz-ui-mod/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorModManifest.cpp`: the DRAWING half - the Mod Settings
  tab and its pages, the Build to Folder / Export Flat / Package Release dialogs, the result
  dialogs, the clean-target question, Disable Game Updates (grep `Draw`, `dlg.`, `Page`).
- `Source/Editors/XrECore/Editor/EditorCatalog.cpp`: `DrawKeyLine` (the author key row, shared
  with Package Release), the Mod Browser page (picture and tags).
- `Source/Editors/XrECore/Editor/EditorGameModes.cpp`: the Game Mode page's picker.
- `Source/Editors/LevelEditor/UI/UIMainMenuForm.cpp`: the Mod menu's items and order;
  `UITopBarForm.cpp`: the project toolbar (Build Mod split).
- Handlers (frozen wire): `mod_settings_view` and the `ui_dialog` ids `build_to_folder`,
  `export_flat`, `package_release` (`rg -n '"mod_settings_view"' Source/Editors`). Schemas:
  `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_build_contents.py`, `tools/qa/test_build_job.py` (their UI
  parts).

## What to build

**Mod Settings** (document kind `mod_settings`, a singleton): a nav of 200 pt with one
`nav_row` per page (its icon and a problem dot) - General, Mod Browser, Game Mode, Relations,
Build Contents, PDA, Commands - and a form of up to 44 em with validation as a sentence under
each field. It saves as it is edited through `gz-mod`'s store (no Save button; the status says
so), and reads only the store's snapshot.
- General: Name (the mod's title - never confused with the mode's), Module ID, Version, Author,
  Description, Website, Repository, Disable Game Updates (turning it ON goes through
  `Disable Game Updates###dlg.disable_game_updates`, which names what the players lose;
  `mod_settings_view ask_game_updates=true` puts it up).
- Mod Browser: Picture (the picture, its state, Choose Picture... / Remove) and Tags (the
  fixed vocabulary as checkboxes) - rows of the same grid; the picture is decoded off the UI
  thread; `gz-release` owns thumbnail making and the vocabulary.
- Game Mode: the plain game, a mode that exists in the linked game, or a new mode the mod
  brings - picked by the names the player sees (`gz-game`'s modes), never typed ids; "not
  selected" is not an option.
- Relations: four rows (Requires, Load After, Load Before, Conflicts), each a chip list with
  Add Module...: a searchable list of the modules installed in the linked game, and a typed id
  for one that is not installed.
- Build Contents: Not Shipped, the block of paths and patterns of `[exclude]`.
- PDA: the layout in use.
- Commands: a section per console command of `[commands]` with Name (Delete as its trailing
  icon), Function (a searchable list of the project's `<script>.<function>` - `gz-mod` /
  the file tree give it; read when the page is picked, never per frame), Help, Arguments,
  Cheat; Add Command.
- `mod_settings_view` opens or reports the tab (`page=`, `ask_game_updates=`).

**The Mod menu and the project toolbar** (through `ShellBuilder`): the menu rows in the
design's order - Mod Settings, Characters, World Rules, Screens & Music, PDA Interface | Build
Into Game, Build to Folder..., Export Flat Gamedata..., Bake Scene Layer > | Package
Release..., Publish Release..., Releases | Submit to Mod Browser... - registering the ones
whose targets exist in the workspace now and leaving the rest to the packages that own them
(a row whose command is not registered is not added); the toolbar's Build Mod (a Primary
split: Build Into Game by default; its menu: Build to Folder..., Export Flat Gamedata...).

**Build dialogs:**
- Build to Folder and Export Flat (M): a `path_field`, and Writes To: the folder the build
  fills, read from its end (`<folder>\modules\<id>`, `<folder>\gamedata_<id>`).
- The clean-target question `dlg.clean_target`: a build that would delete files refuses first;
  the dialog names up to ten of them, Cancel the default; Yes re-runs with `clean`.
- Package Release (M): Output Folder, Requires Game, the Author Key row (read-only unless the
  key asks for its password); replacing an earlier package of the version asks through
  `dlg.replace_release`.
- **Results**: builds started from the UI report in the kit's one `result` dialog
  (`dlg.build_result`, `dlg.export_result`, `dlg.package_result`): a toned title, rows, Not
  Shipped, up to 8 warnings, [Open Folder] [OK]. MCP builds never open it.
- A build runs beside the editor: the dialog closes when the job starts, the Task Center's
  card follows it (publish the job's steps through `TaskCardTable`), Stop stops it.

## Rules

- Every action is a command that already exists (`mod_set_manifest`, `mod_exclude`,
  `mod_export`, `mod_package`, ...): the UI posts it; nothing here writes a file.
- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-mod -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK, on a temp COPY of a real project
  whose `project.ltx` export target points into the temp folder (never the real game, never a
  real project): each Mod Settings page, the Build to Folder dialog, a build's result dialog,
  the Mod menu through `ui_popup`.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. It takes a machine-wide lock (other authors run the
  editor too; waiting minutes is normal), backs up and restores `_projects.ini` and the
  settings file byte for byte, starts the editor minimised, hands your driver the port as its
  first argument and closes the editor afterwards. Never start `Grizzly.exe` by hand. If
  `grizzly` does not build because of someone else's crate, wait and retry a few times, then
  say so. Do not run the QA suites. Never package against or publish to anything real.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-mod` to `docs/rust/STATE.md`.

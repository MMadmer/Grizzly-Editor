# Brief: the Level tab and its panels (`gz-ui-level`)

Tab 1 of the editor: the dock layout around the viewport and the panels in it. Read
`docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md` sections 1,
1a, 2, 4, "Level workspace" / "Level toolbar" / "Status bar" in section 5, and, in section 6,
"World Outliner", "Tools", "Details and Level Settings" - **the contract**; `CLAUDE.md`
sections "Editor UI (UI 2)" (size classes, panels), "The Start page" (Recovery, Import Base
Scene), "Dangerous operations"; the rustdoc at the top of `gz-ui`, `gz-app` (`ShellBuilder`:
`document`, `panel`, `page_card`, `dialog`, `status_chip`, menus, commands; `Document`,
`PanelInfo`, `PanelOp`), `gz-scene` (the store, `SceneCmd`, properties, the library, recovery),
`gz-ui-app` (`output_log`), `gz-ui-content` if it exists (its panel body), `gz-game`,
`gz-levelbuild` if it exists (the rebuild query, the build commands), `gz-editor`.

You own: `crates/gz-ui-level/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependencies in its `Cargo.toml`.

## The viewport is another agent's

`gz-ui-viewport` is being written at the same time. You embed it through this fixed interface
and nothing else:

```rust
gz_ui_viewport::ViewportPane::new(editor, gpu) -> Self
pane.ui(ui, cx)            // draws the viewport into the space it is given
pane.inactive(cx)          // a tick while Level is not drawn
pane.shortcut(cx, chord)   // the scene's keys, while Level owns them
pane.mcp_state()           // for `state` / `ui_list`
```

Until that crate compiles, write against this interface; finish your compile check when it
lands (retry a few times; say so in the report if it never did).

## The C++ that is the spec

- `Source/Editors/XrEUI/XrUIShell.cpp` (the Level workspace part), `Source/Editors/LevelEditor/
  UI/UIMainForm.cpp` (the dock layout, size classes, `level_layout_custom`), `UITopBarForm.cpp`
  (the Level toolbar), `UIMainMenuForm.cpp` (Scene, Build, View menus).
- `UI/Outliner/UIWorldOutliner.cpp` (1.6k), `UI/UILeftBarForm.cpp` + `UI/Tools/*.cpp` (the
  Tools panel and each tool's form), `UI/UILPropertiesFrom.cpp`, `UI/UIWorldPropertiesFrom.cpp`,
  `UI/UIPropertiesModal.cpp`, `Source/Editors/XrEProps/**` (the property grid's behaviour:
  choosers, the text editor dialog), `UI/UIDOShuffle.cpp` / `UIDOOneColor.cpp` (Detail
  Objects), `Editor/Recovery/EditorRecovery.cpp` (the Recovery card's text and rules).
- Handlers (frozen wire): `ui_panel`, `outliner_filter`, `outliner_show`, `scene_visibility`,
  `property_inspector open` (the panel side - the data side is `gz-scene`'s), `reset_layout`'s
  level part, the Import Base Scene dialog (`ui_dialog open id=import_base_scene`). Schemas:
  `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_ui_shell.py` (panels, size classes), `test_properties.py`.

## What to build

- **The Level document** (kind `level`, never closes): the Level toolbar (Save | Undo, Redo |
  Build Collision, Build AI Map, Build Game Graph, Build All | Play v, Play in Game, Play
  Clean, Stop | Reload Configs, Game Folder - each a command that exists; a button whose
  command is not registered is not added) over a dock layout with the viewport in the centre.
  Docking: `egui_dock` (workspace dependency) or a layout of your own from the kit's splitters -
  choose what gives the design's rules with less code: side panels keep their pt size and the
  viewport takes every resize; panel tabs on a 26 pt strip with their cross on hover; no panel
  leaves the window.
- **Size classes**: Wide (1400 x 640 pt and up: Tools 260 left, a right column of 340 with the
  Outliner (40%) above Details + Level Settings, a bottom strip of 240 under the viewport with
  Content Browser + Output Log) and Compact (Tools 240, right column 300, browser and log only
  as drawers). Crossing a class rebuilds the layout unless the author customised it; Reset
  Layout rebuilds for the current class; F11 hides the panels. The layout persists in the
  editor's settings.
- **Panels** (each registered with `ShellBuilder::panel` so `ui_panel` and `ui_list` reach it):
  - **World Outliner**: SearchField; Filters with a badge (Selected Only, visibility, classes
    with counts, sort); More (By Type / Hierarchy, Show All, Show / Hide Selected); columns eye,
    then Name with icon and ellipsis; class rows with an eye and a count; virtualised (a scene
    holds tens of thousands of objects: lay out visible rows only, from a `Query` keyed by the
    scene's revision and the filter); empty states; footer "2 objects - 1 selected"; Rename is
    `dlg.rename_object`, Delete asks through `dlg.delete_object` (the kit's `confirm`, naming
    what goes).
  - **Tools**: tiles in 1-3 columns, each [icon][label][eye]; the active tool filled with
    accent; a sticky "<Tool> Tool" header; a tool with a list (Object, Spawn, Particles) draws
    a SearchField and the list first (the search opens every folder holding a match); the
    tool's other sections start closed; the renamed controls of the design; Snap Objects as a
    closed section; Detail Objects as the L dialog `dlg.detail_objects`.
  - **Details** and **Level Settings**: header (icon + name), "Search N properties", the
    kit's property grid (roles `selection` / `world`) over `gz-scene`'s property descriptions,
    with a help footer; choose rows are pick fields that open the L dialog `###dlg.choose`
    (search opens folders); multi-line text is first line + Edit (an M dialog). An edit is a
    `SceneCmd` (a drag in a number field is a merged undo step). `target=world` raises Level
    Settings.
  - **Content Browser** and **Output Log**: the bodies `gz-ui-content` and `gz-ui-app` offer,
    hosted as panels in the Wide layout.
- **Status bar**: the centre status (selection count, grid, angle) and the "Rebuild:
  Collision - AI Map - Game Graph" chip with a [Build] per item.
- **Scene menus** (Scene, Build, View) on the Level tab only, with the keys bound now.
- **Recovery** (an M page card: Discard... | Restore the default; Discard confirms naming the
  snapshot's time and the older snapshots) and **Import Base Scene** (an M dialog with a
  search that reads its list once).
- Before a close that clears the scene, ask the viewport for `preview.png` (through the hook
  `gz-ui-viewport` exposes) and hand it to `gz-project`.

## Rules

- Every action is a command of `gz-scene` / `gz-levelbuild` / `gz-project`; the panels read
  snapshots and queries and change nothing themselves.
- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-level -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK, on a temp COPY of a real project
  with a scene (never a real one): the Wide layout at 1707 x 912 pt and the Compact one at
  1280 x 720 pt (`window_control action=resize`), the Outliner with its filter menu, Details
  on a selected object, Level Settings, the Tools panel, `ui_audit` on each (nothing escaped).
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand. If `grizzly` does not build because of someone else's crate,
  wait and retry a few times, then say so. Do not run the QA suites.

## Report

Under 400 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-level` to `docs/rust/STATE.md`.

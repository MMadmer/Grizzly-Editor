# Brief: the graph tab (`gz-ui-graph`)

The tab a quest or a behaviour graph is edited in. Read `docs/rust/CONVENTIONS.md` and
`docs/rust/STATE.md` first; then `docs/UI_DESIGN.md` sections 1, 1a, 2, 4 and, in section 6,
"Quest and Behaviour graph" - **the contract**; `CLAUDE.md` sections "Quests (NQ - node graph)"
(the graph tab, the node box, global variables) and "Behaviour graphs" (a behaviour graph is
its own asset and never says "quest"); `docs/nq/NQ_FORMAT.md` for the vocabulary; the rustdoc
at the top of `gz-nq` (documents and their store, the edit commands, `layout`, the catalog,
validation, pickers, references, the view state it keeps for the UI), `gz-ui`, `gz-app`
(`Document`, `DocCx`, `ShellBuilder`, `on_ui`, `report_scroll` / `take_scroll`), `gz-mod`
(globals), `gz-editor`.

You own: `crates/gz-ui-graph/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependency in its `Cargo.toml`.

## The C++ that is the spec

`Source/Editors/LevelEditor/UI/QuestGraph/`: `UIQuestGraph.cpp` (the tab: toolbar, panes,
drawer, banner), `NqCanvas.cpp` (2.9k: drawing, zoom levels, level of detail, hit tests,
gestures, the minimap, edge pills, Frame All), `NqInspector.cpp` (3.5k: the Graph panel - header,
tasks / roles, variables, globals - and the Details panel with its typed editors `DrawTyped`,
`DrawParams`, `DrawNpcList`, the item rows), `NqLook.*` (titles, descriptions, family colours:
`KindTitle` / `KindDesc`). Handler (frozen wire): `quest_view` in
`Source/Editors/LevelEditor/Editor/Mcp/LevelMcpGraph.cpp`. Schema: `crates/gz-mcp/tools.json`.
What QA asserts: `tools/qa/test_graph_ui.py`, `tools/qa/test_behaviours.py` (their UI parts).

## Architecture (decided)

- **One document per file**: kinds `quest` and `behaviour`, ids `quest:<rel>` / `beh:<rel>`
  (`ShellBuilder::document`); the label is the asset's name without its extension. The tab holds
  VIEW state (pan, zoom level, selection, panes, the drawer); the graph lives in `gz-nq`'s
  store, and the tab draws its snapshot. Opening over MCP (`quest_open`, `quest_view`)
  activates in the same call.
- **Layout**: the Graph panel (280 pt: header, Tasks or Roles, Variables under `vars_split`,
  then Global) | the canvas | Details (360 pt: the selected node, the selected action open
  under it below `action_split`), a drawer under them (Problems, Find Results). The panels keep
  their pt and the canvas takes every resize; under a 400 pt canvas the Graph panel folds to a
  rail. Sizes are the kit's layout keys `graph.panel_w`, `graph.details_w`, `graph.drawer_h`,
  `graph.vars_split`, `graph.action_split`, `grid.graph_*`.
- **The canvas is painted by hand** (`egui::Painter`), and **a node's box has ONE source**:
  `gz_nq::layout` - the same function gives the box to drawing, hit tests, the minimap and
  `quest_layout`. 240 pt wide; a 28 pt header with family strip, icon, title, muted id and
  badges; up to 3 "key: value" lines; up to 4 action rows then "+N more"; pin labels inside.
  Scale is `ZOOM[level] * pixels-per-point`, applied once, levels `{0.2, 0.3, 0.4, 0.5, 0.65,
  0.75, 1.0, 1.25, 1.5, 2.0}` (index 6 is 1:1); text 16 / 15 / 14 with glyph px rounded per
  level; level of detail (full for titles of 10 px and up, titles only at 7-10, blocks below).
  Frame All fits the extent at the highest level up to 1:1, never below 0.75; a graph too big
  for that is framed AT 0.75 from its trigger and the status says so.
- **Only what is in view is drawn**: nodes go into a uniform grid index built once per graph
  revision (a `Query` on the CPU pool); a frame draws the cells the view touches. Links are
  curves between pin anchors; back edges take channels as the C++ lays them out. A graph of
  thousands of nodes must not cost more per frame than the nodes on screen.
- **Gestures are commands**: drag a node (a merged undo step, sealed on release), connect and
  disconnect pins, marquee select, delete (through the kit's `confirm` when it loses work),
  duplicate, copy / paste as text (the asset's own text form - an agent can paste it too),
  add a node from a searchable menu of the kinds of THIS document's vocabulary (the catalog's
  `docs` split: a behaviour graph never offers quest machinery). Nothing changes the graph
  except `gz-nq`'s commands.
- **Details: what comes from a list is picked, not typed.** Each param type has its editor
  (`DrawTyped`): pickers over `gz_nq::pickers` (a `Query`; a list shows busy while it loads),
  characters by asset, destinations, item rows, npc lists, level changers. A value the list
  does not hold is kept and shown, never corrected. Kind descriptions are tooltips and the help
  footer. Problems stay with what they are about: a dot on the node, a sentence in Details;
  codes stay in MCP.
- **Global variables** are the module's (`mod.ltx [vars]`): edited in the Variables pane under
  Global and written through `gz-mod`'s store when an edit is let go of.
- Toolbar: Save, Reload | Undo, Redo | Validate (badge), Auto Layout, Frame All | Back, Forward |
  Find (F3), Find in Project | Bookmarks, Previous, Next | spring | Zoom, Minimap, Shortcuts,
  Graph panel, Details panel - the common ones on the surface, the rest under More (section
  1a). The minimap is 180 x 120, hidden under 12 nodes; without it a pill at the middle of each
  canvas edge counts the nodes wholly beyond it and a click brings the nearest in. An external
  change shows a banner with [Reload] and [Overwrite...].
- **`quest_view`**: everything the C++ answers - `zoom`, `panes` in pt, `beyond`, `title_px`,
  `graph_panel`, `details`, `problems`, `graph_width`, `details_width`, `action_split`,
  `vars_split`, `keys`, `scroll=variables|tasks|task:<id>|action:*` (an error when off screen).
  It applies at once, then waits for the frames that show it and reads again: one call.

## Rules

- Draw only with the kit (and the painter for the canvas, in palette colours); a widget the kit
  lacks is `local_<name>` here, named in your report.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-graph -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK, on a temp COPY of a project (never
  a real one) holding the graphs of `docs/nq/examples`: Frame All readable, a node selected
  with Details open, the Graph panel, the Problems drawer, a behaviour graph (no "quest"
  anywhere), the canvas at two zoom levels; `ui_audit` shows nothing escaped.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand. If `grizzly` does not build because of someone else's crate,
  wait and retry a few times, then say so. Do not run the QA suites.

## Report

Under 400 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-graph` to `docs/rust/STATE.md`.

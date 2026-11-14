# Report G - the graph editor

## 1. Built

**The domain (`gz-nq`).**
- `canvas.rs` (new): the file's `canvas = { flow, wires, frames, routes, bookmarks }` read and written
  losslessly (unknown keys and comments kept through the document's difference); `Flow` (right by default,
  `down` for a file with positions and no flow), `WireStyle` (Curved, Straight, Circuit, Angled, Subway),
  `Diagonal`, `WireOpts` over `WireSettings`, frames, named bookmarks, routes keyed `from.pin>to` and
  rewritten by a rename (`take_renames` in `disk.rs` renames the node in the file first, so it keeps its
  comments and unknown keys).
- `asset.rs`: `collapsed` (editor-only) and `disabled` (the game's) per node, written in the kind's order;
  the outline says `; disabled`.
- `measure.rs` + `metrics_table.rs` (generated): the canvas faces' advance widths, so a box is measured off
  the UI thread with no font loaded. `layout.rs` rewritten: `node_box` sized by its content (180-360 pt from
  the title, the id, the comment and the widest line or action row), comment row, collapsed box, pins of
  both flows (`input_pin` / `output_pin`), the default pin, failure pins.
- `arrange.rs` (new): the auto layout of both flows - forward edges by strongly connected components and
  depth, every node under its dominator, wide fan-outs wrapped into blocks, long chains folded, node-level
  skylines; Layout All / Selection / Branch; Align (6 ways), Distribute, Straighten.
- `wires.rs` (new): the one polyline builder (five styles, diagonal position, lanes per node and per pin,
  subway tracks, back edges routed through a shared free channel, reroutes passed through), its hit test
  (`distance`), `point_at`, `length`.
- `text.rs`: `Names` (items by in-game name, characters by name), `param_text` by type (text «...», item
  names, `×N`, durations, metres, booleans), `summary` from the catalog's `summary` key.
- `catalog.rs`: the kind keys `summary` and `pin.<name>` (`Kind::summary`, `pin_desc`, `pin_text`);
  `asset.reference` prints both.
- `doc.rs` rewritten (+ `doc/tests.rs`): 21 zoom levels 5%-200% (`ZOOM`, level 16 = 1:1); `graph.edit`'s
  operations (`help about=graph-ops`): add, remove, rename, set, connect, disconnect, cut, move, replace,
  insert_on_wire (new node or `existing=`), actions, set_graph/task/var, rename_task, frame, route,
  bookmark (`here=`), disable, collapse, duplicate (`wires=`), bypass, connect_nearest, wires, flow, paste
  (foreign kinds left out and named); `Arrange`; view ops (grow, bookmark by name, history, find).
- `validate.rs`: `Severity::Note` - a disabled node is a note, never a warning; `NqProblem::sentence`.
- `defaults.rs` (new): `GraphDefaults`, the preferences a graph falls back on (kept in step by the tab).
- `cmd.rs` rewritten: `graph.edit`, `graph.arrange`, `graph.get` (outline, problems with sentences, frames,
  routes, bookmarks, wire options in force; `boxes`, `wires`, `view`, `at` = the canvas's hit test),
  `graph.find`; the `quest_*` handlers are gone.

**The tab (`gz-ui-graph`).**
- `scene.rs` rewritten: per revision, wire options and names, on the CPU pool - boxes, texts in words
  (summaries, item and character names), pins with their roles and meanings, every wire's polyline,
  frames, grid index, extent, minimap.
- `canvas.rs` + `canvas/draw.rs`, `input.rs`, `menus.rs`, `tests.rs` rewritten: semantic zoom (full /
  header-only at a fixed 13 px / block, never text under 11 px), the node anatomy (family icon and colour,
  title, muted id, italic comment, lines, action rows with summaries, "+N more", problem badge with count and
  a red/amber top line and the first sentence on hover, `shadow.card`, `once`/`off` badges, collapsed,
  disabled at 40% with a hatched header), pins by role (hollow when unwired, tooltips from `pin.<name>`,
  the name of a non-default pin on its wire's first stretch), wires from the scene's polylines (thickness,
  colour by pin, Dim Unrelated, Flow Bubbles, arrowheads at the input's rim, dashed back edges and wires of a
  disabled node), comment frames (move with what they hold, resize, rename in place, tone), reroutes
  (double-click, drag, double-click away, menu), Ctrl+right-drag cut, Alt+drag rewire (Alt+click removes),
  wires drawn out of an output or into an input, a lone node dropped on a wire goes on it, grid snap and
  alignment guides while dragging, the minimap at most a fifth of the canvas's height, context menus of a
  node / action / wire / reroute / frame / the empty canvas (Paste, Add Comment Frame, Frame All, Auto Layout
  over the picker), every row of the group Graph drawn from its action with its bound keys.
- The picker: the kit's `PickList`, rows made once when it opens (icon in family colour, description as the
  second line, grouped by family), Recent per project, Favourites shared by every project, filtered by what
  it is for (`pick_kinds`: canvas, from an output, into an input, on a wire, actions, replace).
- `actions.rs` rewritten: 65 actions of the group Graph, each running `graph.edit`, `graph.arrange`,
  `graph.view`, `graph.find` or `doc.reload` (Align x6, Distribute x2, Straighten, Select Connected /
  Upstream / Downstream, Add Comment Frame, Replace Node, Duplicate with Wires, Disable, Collapse, Bookmarks
  1-9 go/set, Auto Layout x3, Flow x2, the wire styles, Frame All / Selection, Find, Find Next / Previous,
  Find in Project, Back / Forward, Add Node, Rename, Remove Wire, Connect to Nearest, Bypass, Problems,
  Reload); `graph.action` is gone.
- `cmd.rs` rewritten: `graph.view` (typed card) - frame, zoom, centre, select / grow, slot, history,
  bookmark, drawer, panels, widths, split, scroll, `menu=node|wire|canvas|frame|picker|replace|none` (a
  picker from a pin: `from_pin=` / `into=`), focus, wire, dialog; it answers `detail`, `title_px`, `flow`,
  `wire_style`, `menu`, `picker`, `bookmarks`, `history`, `panes`... from the frames that show the change.
- `tab.rs`: toolbar Save, Undo, Redo, problems count, Frame All, Find | Graph / Details / Minimap (quiet
  toggles), Zoom | More (Reload, Auto Layout, Flow Direction, Wires, Find in Project, Back, Forward,
  Bookmarks, Shortcuts); Graph menu with Flow Direction and Wires ("As in Preferences" first) and every
  action; Details folds to a rail under 1400 pt and opens over the canvas; the Problems drawer in sentences
  (code in the tooltip); Preferences > Graph (Flow, Wire Style, Diagonal Position, Thickness, Colour by Pin,
  Dim Unrelated, Flow Bubbles, Arrowheads, Snap to Grid, Alignment Guides) kept in `GraphDefaults`;
  `doc.reload discard=true` asks nothing; the Shortcuts card lists the new gestures.
- `inspector.rs`: the Graph panel is ONE scroll (header, Tasks or Roles, Variables, Global), each a kit
  `Section` with `+` on its header and its count; a variable's and a global's form titled by its name;
  section folds in the layout store (`section.graph.*`), the vars splitter gone.
- `params.rs`: the field pickers are the kit's `PickList` over every entry (gathered once per popup);
  `who` is a row of kit Checks; `Switch` / `Check` replace the local check. `look.rs`: `kind_combo` = kit
  `Combo` over a cached `PickList` of kinds. `local.rs` keeps only `local_pane_header` (the kit has none).
- `derived.rs`: `NodeMarks` (severity, count, first sentence), `NameIndex` (items, characters).

**The game half** (`D:\Games\Dead Air\DeadAir-x64`, files for the architect to deploy):
- `packaging\dead-air-x64\compatibility\gamedata\scripts\xms_nq_load.script` - `disabled` read on a node
  (`E006` unless boolean), the way on (`pass_pin`: the first output in catalog order that is not a failure,
  cases where the kind's `pins` names them, `cases_at`), stored as `node.pass`.
- `packaging\dead-air-x64\compatibility\gamedata\scripts\xms_nq.script` - `enter_impl` passes a disabled
  node through (no action, no wait, no token, not done; no way on ends the branch); `trigger_armed`,
  `fire_trigger`, `trigger_pin` and `arm_triggers` never arm or fire a disabled trigger.
- `packaging\dead-air-x64\compatibility\gamedata\configs\nq\catalog.ltx` - `summary` on 117 kinds and
  `pin.<name>` meanings, the header documents both; this repository's `gamedata\configs\nq\catalog.ltx` is
  byte-identical.
- `docs\dead-air\NQ_RUNTIME.md` - 3.7 "Disabled nodes", the schema's `disabled` / `collapsed` / `canvas`,
  the catalog's two keys.
- `tools\nq\test_examples.lua` - section (z29), 20 checks.
- The loader ignores `canvas` and `collapsed`: `build()` (xms_nq_load.script, from `function build`) reads
  only `nq`/`beh`, `id`, `type`, `title`, `activation`, `vars`, `roles`, `tasks`, `nodes`; `build_nodes`
  reads only `id`, `kind`, `once`, `disabled`, `params`, `cond`, `on_enter`, `on_exit`, `out`, `comment`.
  (z29) loads a file with a full `canvas`, `pos` and `collapsed` with no error.

**Callers fixed**: `tools/qa/test_graph_ui.py` (rewritten), `test_behaviours.py`, `test_assets.py`,
`test_characters.py`, `test_ui_shell.py`, `test_ui_tour.py`, `test_mcp_bridge.py` (two lines), new helper
`tools/qa/graph_ref.py` (the reference's kinds as dicts); `gz-ui-graph`'s dialogs (Rename Task on
`graph.edit`). Documents: `docs/nq/NQ_FORMAT.md` (2.1 canvas, 2.2 disabled, 4, 5, 8, 8.1),
`docs/NQ_ARCHITECTURE.md` (4.2, 4.5, 5.2, 13.3-13.4, 13.8, 13.9, 13.11, 16), `docs/UI_DESIGN.md` (actions,
the graph surface, MCP).

## 2. Decisions

- **Text sizes are line boxes.** "Never under 11 px" is measured on a line's box (15 pt body, 16 pt
  title at 100%), the vocabulary of UI_DESIGN section 2: full detail down to 75% at a UI scale of 1.0 (to
  60% at 1.5). Below that a node is its title at a fixed 13 px over its whole box; under 48 px of width it
  is a block with its title on hover. Frame All of the 110-node quest therefore shows every title at
  1707x912 @ 1.5 (25%, nodes 67-135 px) and blocks for the narrowest nodes at 1280x720 @ 1.0 (20%) - the
  whole quest at once is not readable at that size, and the brief wants the whole graph framed.
- **Headless edits are written at once.** An editor without a window has no tab to hold an edit, so
  `graph.edit` / `graph.arrange` save there; with a window they bring the tab up and `save=true` writes.
- **Bookmarks are named and the file's**; Ctrl+N goes to the bookmark named N, else the N-th, and
  Ctrl+Shift+N sets the bookmark named N on the selected node (`bookmark here=true`).
- **The flow of a file without `flow`**: down when any node has a position (it was laid out that way),
  else Preferences > Graph > Flow; `flow` is written once a new graph's nodes are placed.
- **Recent is per project in the layout store** under a key of the project's root (`pick.graph.<hash>`),
  Favourites one list (`pick.graph`) copied in when a picker opens and back when a star changes it: the
  kit's `PickList` keeps both under one key, and the machine state has no slot for it (Request 1).
- **A disabled node is not done** when it is passed through: `node_done` stays false and nothing of it
  reaches a save, so enabling it later needs no migration. Its way on is the first non-failure output in
  catalog order, a switch's cases where its `pins` line names them (a disabled `flow.branch` takes its
  first case, its conditions unread).
- **Problems are sentences** made from the message (capitalised, ended) - `NqProblem::sentence`, also in
  every problem's JSON; the codes stay in tooltips and commands.
- **Pin names live on the wire's first stretch**, also for an unwired pin (where its wire would start), so
  a node's body holds no pin row; a single default way on is never named.
- **The Graph panel is one scroll**, so the variables split (`graph.vars_split`, `vars_split=`) is gone;
  `graph.view` answers `vars_share` (the share of the panel the Variables and Global take on screen).
- **Section folds are shared by every graph** (`section.graph.<name>`): per-file keys in the editor's
  settings would grow with every file opened.
- The Down flow of the 110-node fixture is 4596 x 2644 pt (the right flow 3144 x 3154): the test holds
  the right flow under 4000 and the down flow under 4800, both without an overlap.

## 3. Contract changes

- File (NQ_FORMAT 2.1, 2.2): top-level `canvas = { flow, wires, frames, routes, bookmarks }`, node keys
  `collapsed` (editor-only) and `disabled` (read by the game). Paired with the game: NQ_RUNTIME 3.7, the
  loader and the core (files in section 1).
- Catalog (both copies, byte-identical): optional kind keys `summary` and `pin.<name>`; the runtime does not
  read them; the parity test still passes.
- Commands: `graph.edit`, `graph.arrange`, `graph.get`, `graph.find` (`gz-nq`), `graph.view`
  (`gz-ui-graph`); topic `graph-ops`. Gone: `quest_open/close/save/undo/redo/reload/apply/edit/layout/view/
  find/bookmarks/history/minimap/catalog/lookup/rename_task/project_find` and `graph.action` - the shell's
  `doc.*`, `ui.tab`, `asset.reference kind=quest`, `game.lookup` and `asset.find` cover the rest.
- Problems' JSON carries `sentence`; `graph.view` answers `detail`, `flow`, `wire_style`, `menu`, `picker`,
  `bookmarks`, `history`, `vars_share` (no `frame_min_level`, `vars_split`, `open_tabs`).
- Preferences (editor settings): `graph_flow`, `graph_wire_style`, `graph_wire_diagonal`,
  `graph_wire_thickness`, `graph_wire_by_pin`, `graph_wire_dim`, `graph_wire_bubbles`, `graph_wire_arrows`,
  `graph_snap`, `graph_guides`. Layout keys: `graph.vars_split` gone, `section.graph.*` new.

## 4. Requests

1. **K / A2** - the kit's `PickList` keeps Recent and Favourites under one memory key in the layout store;
   the brief wants Recent in the project's machine state and Favourites in the editor's settings. A
   `PickList::recent_key` / `favourites_key` pair (or a machine-state slot `ProjectState::recent_kinds`)
   would let the graph drop its sync of the two lists.
2. **Q (or whoever owns them)** - `tools/mcp/AI_SETUP.md` (rows `grizzly_quest_view`, `grizzly_quest_edit`,
   the openers line) still names the `quest_*` tools: they are `graph.*` now (NQ_FORMAT section 8 is the
   table to copy).
3. **Architect** - CLAUDE.md, "Quests (NQ - node graph)": the node is no longer 240 pt (180-360 by its
   content), Frame All has no 0.75 floor (semantic zoom instead), `quest_view` is `graph.view`,
   `graph.vars_split` is gone; and deploy the five game files of section 1 (no deployment was made).
4. **Owner of `tools/qa/test_mcp_bridge.py`** - the suite stops at `import catalogue` (the module is gone) and
   still calls the bridge's old tools (`grizzly_content_mkdir`, `grizzly_ui_tab`, ...); only its two
   `grizzly_quest_edit` lines were moved onto `grizzly_run` here.
5. **M** - W060 (the module's own items) is still the validator's call site in `gz-nq`'s `validate.rs`, left
   to M as the brief says.
6. **K (the window)** - one QA run of `test_graph_ui` logged "the graphics device reported an error:
   Validation Error ... In a set_scissor_rect command" (it stayed in the status bar's last line); the next
   runs did not. egui-wgpu clamps every scissor to the `ScreenDescriptor`'s size, so no shape can cause it:
   the painter (`egui_wgpu::winit::Painter::paint_and_update_textures`, as patched for captures) sizes the
   descriptor by the configured surface while the pass draws into the acquired frame, or a capture texture
   of that frame's size - they differ while a resize is in flight (the suite resizes the window three times).

## 5. Proof

**Unit tests.** `gz-nq` 64 passed, `gz-ui-graph` 11 passed; clippy of both crates without a warning, `cargo fmt`
applied. The ones the brief asks for:
- `wires::the_polylines_are_the_snapshot` - five styles x two flows x the diagonal positions over a fixture
  with lanes, cases and a back edge, against `crates/gz-nq/src/fixtures/wire_snapshots.txt` (`GZ_SNAPSHOTS=1`
  writes it again); `every_style_joins_its_pins_in_both_flows`, `sibling_lanes_never_overlap`,
  `back_edges_share_their_target_channel`, `waypoints_are_passed_through`, `hit_tests_find_every_style`.
- `arrange::the_big_quest_fits_without_an_overlap_in_both_flows` - the 110-node quest (`n7_control`):
  right 3144 x 3154 pt, down 4596 x 2644 pt, no two boxes overlap; `a_layout_is_deterministic_and_flows_its_way`,
  `a_merge_sits_after_its_branches`, `a_wide_fan_out_wraps`.
- `asset::the_canvas_round_trips_with_its_comments_and_unknown_keys`, `canvas::route_keys_follow_a_rename`,
  `disk::a_renamed_node_keeps_what_the_editor_does_not_know`, `doc::every_operation` and the fourteen other
  `doc::tests` (insert on a wire, frames, bookmarks, disable / collapse, duplicate with wires, replace, paste,
  arrangements, the zoom ladder, all-or-nothing, undo), `layout::the_box_is_sized_by_what_it_says`,
  `catalog::the_bundled_summaries_and_pin_texts_name_what_the_kind_has`.
- `canvas::the_picker_offers_what_the_pin_takes`, `canvas::semantic_zoom_thresholds`,
  `actions::every_action_runs_a_graph_command` (65 actions), `scene::frame_all_takes_whatever_scale_it_needs`.

**QA** (each on a copy of `DARF-Overnight-Delivery`, port 28963 or the harness's own, a home of its own):
- `python tools/qa/run_all.py test_graph_ui test_behaviours test_assets test_characters`: 4 suites passed,
  wall 12.8 s - `test_graph_ui` 175/175 (phases A-J at 1707x912 @ 1.5 and 1280x720 @ 1.0, plus 700x600),
  `test_behaviours` 40/40, `test_assets` 33/33, `test_characters` 75/75.
- `test_ui_shell.py --only C,J` 34/34 and `test_ui_tour.py --only 3,4,5,13` 8/8 at both sizes - the phases
  whose calls moved onto `graph.*`.
- `test_mcp_bridge.py` was not run: it stops at `import catalogue` (`tools/qa/catalogue.py` is gone, not by
  this work) - Request 4.

**The game half.** `python tools/nq/run_lua.py test_examples.lua`: 894 passed, 0 failed, (z29) "disabled
nodes" 20 checks (no action and no wait run, the way on, a disabled branch takes its first case, a disabled
trigger never fires and keeps no quest alive, no token saved and not done, `E006`, `canvas` / `pos` /
`collapsed` loaded without a problem). `tools/qa/Test-NqCatalogParity.ps1`: "NQ CATALOG PARITY: OK
(117 kinds)". The two `catalog.ltx` copies are byte-identical. Nothing was deployed.

**Pictures** (`Intermediate/plan/G/`, from `test_graph_ui.py --shots`, at 1707x912 @ 1.5 unless named):
- `wires_right_<style>.png`, `wires_down_<style>.png` (curved, straight, circuit, angled, subway) - the
  wires fixture: a branch whose cases are named on their wires (north, south, else), a wait with a red
  failure pin (timeout), a dashed back edge round the side, arrowheads at the inputs.
- `node_families.png` - one node of each family at 100%: icon and colour, title and muted id, the italic
  comment, lines in words (an item by its in-game name, `×N`, durations), action rows with their summaries.
- `big_quest_framed_1707x912@1.5.png` - Frame All of the 110-node quest after Auto Layout: 25%, every node
  its title at 13 px, back edges dashed round the side, the minimap under a fifth of the canvas's height.
- `big_quest_framed_1280x720@1.png` - the same at 20%: the narrowest nodes are blocks in their family's
  colour (title on hover), Details folded to its rail.
- `picker_from_pin.png` - the kit's PickList put up from an output: only kinds that take a wire in, by
  family, each with its description's first sentence.
- `menu_node.png`, `menu_wire.png`, `menu_canvas.png` - the context menus, every row with its bound keys.
- `frames_and_reroutes.png` - the comment frame "The two ways" (tone 3) and a wire bent through a reroute.
- `disabled_and_collapsed.png` - a disabled node at 40% with a hatched header and dashed wires, a collapsed
  node.
- `details_rail_overlay.png` - 1280x720: Details opened over the canvas from its rail.

**Kept to the rules**: no editor was started outside the harness, every one was closed over MCP, nothing of
mine is left running. One deviation: a read-only `git status --short` was run once in the game repository
(it changes nothing; the rules ask for no git command there).

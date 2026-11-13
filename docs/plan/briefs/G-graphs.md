# Brief G - the graph editor

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5), `docs/COMMANDS.md`, `docs/ASSETS.md` (2, 3, 4),
`docs/NQ_ARCHITECTURE.md` (the contract of quests and behaviour graphs; sections on the editor), `docs/nq/NQ_FORMAT.md`,
`docs/UI_DESIGN.md` (the graph tab), the reports of wave 1 (`docs/plan/reports/K.md`: the kit's new widgets,
actions and keys, tokens `graph.family.*`, `wire.*`, `pin.*`, `frame.*`; `A1.md`: how a graph document loads, saves
and follows its file), and the audit **`Intermediate/plan2/ux_rest.md` sections B.4, C (all of it), D rows 6-7, E**
with its pictures (`Intermediate/plan2/shots/rest/1*.png`, `2*.png`).

You own: `crates/gz-nq` (model, layout, validation, documents, text; its file io is as stream A1 left it),
`crates/gz-ui-graph`. You also have a **game task** (part 7): only the files it names in
`D:\Games\Dead Air\DeadAir-x64`, no git command there, no deployment (the architect deploys).

The author's words: several ways to draw the wires - sharp "electric" ones like circuit lines, curved as now,
others popular in engines and plugins; better nodes; a better list and picker of nodes; more actions on nodes.
The reference is the Unreal Blueprint editor with its well-known plugins (Electronic Nodes, Blueprint Assist,
Node Graph Assistant, Darker Nodes), then Blender, Unity and Godot. Audit C.2 describes each feature precisely
enough to implement; where this brief is shorter, the audit is the specification.

## 1. Flow direction and a layout that fits

- A graph flows **left to right** (inputs on a node's left edge, outputs on its right) or **top to bottom** (as
  today). `canvas.flow = "right" | "down"` in the file; a new graph takes Preferences > Graph > Flow (default:
  left to right); a file that has node positions and no `canvas.flow` is `down` - it was laid out that way.
  Graph > Flow Direction switches and lays the graph out again (one undo step).
- Auto layout (`gz-nq`'s `layout.rs`) handles both directions and **wraps wide fan-outs**: a node with many
  children lays them out in a block several rows (or columns) deep instead of one ribbon - the author's 110-node
  quest is 15000 pt wide today and does not fit at any zoom (audit fact 3). Layout of the selection only and of
  the selected node's branch ("Auto Layout Branch") beside the whole graph.
- Zoom from 5 % to 200 %. **Semantic zoom**: text never draws smaller than 11 px on screen - under the scale
  where body text would, a node shows its header only (title kept readable by drawing it at a fixed size and
  letting it use the node's whole box), under that a coloured block with the title on hover. Frame All frames
  the whole graph, whatever scale that takes (the "larger than the view at 75 %" notice goes).
- A node's box stays one source (`layout.rs` `node_box`), now **sized by its content**: width 180-360 pt from the
  title and the widest line, measured with the layout font off the UI thread (audit C.2 "Nodes").

## 2. Wires

One builder makes a wire's polyline from its two pins, the flow direction, the style and its waypoints; drawing,
hit testing, the minimap and `graph.layout`'s answer all use that one polyline. Styles (audit C.2 gives the
geometry; mirror it for left-to-right):

1. **Curved** - the bezier, with the tangent from both deltas so wide fan-outs bow instead of going flat.
2. **Straight** - with short stubs out of the pins.
3. **Circuit** - 90 degree runs with rounded corners, sibling wires on separate lanes so their runs never
   overlap; back edges already route this way (`route_back`).
4. **Angled** - 45 degree diagonals between straight stubs ("electronic" wires), with the diagonal at the start,
   the middle or the end.
5. **Subway** - angled, with wires of one output sharing a trunk until their targets part and parallel runs
   offset like tracks.

Options, each a preference with a per-graph override (`canvas.wires = { style = "circuit", ... }`): thickness
(1-4 px at 100 %, never under 1 px on screen); colour by pin role (the normal continuation neutral; failure,
timeout and abort pins in the danger hue; branch cases by index - tokens `wire.*` and `graph.family.*`, no colour
literal); **Dim Unrelated** (wires not touching the selection at 15 %); **Flow Bubbles** (dots travelling along
the wires downstream of the selection; off by default; animation asks for frames only while it is on and
visible); arrowheads (at the wire's end, never on the pin); back edges dashed; the pin's name as a label on the
wire's first stretch when it is not the default pin (never on the dot - audit fact 4).

Preferences > Graph (a group you add through stream K's preference pages): Flow, Wire Style, Diagonal Position,
Thickness, Colour by Pin, Dim Unrelated, Flow Bubbles, Arrowheads, Snap to Grid, Alignment Guides. Graph > Wires
is the per-graph override ("As in Preferences" first).

## 3. Nodes

- **Readable bodies**: an action row is the action's title and its one telling argument ("App Message: «Ночь 7.
  Смена...»", "Spawn Items: bolt x1"), never a Lua table literal. The telling argument is the kind's `summary`
  key in the game's catalogue (part 7); `gz-nq`'s `text.rs` formats it by the parameter's type (a text in
  quotes, an item by its in-game name, a character by its name, a number with its unit).
- **Anatomy**: a header with the kind's icon in its family colour (`graph.family.*` by family: triggers, flow,
  waits, dialogue, world, items, tasks, behaviour ... - status colours mean state only, audit fact 8), the title,
  the node's id muted; the author's `comment` as an italic line under the header; "key: value" lines; action
  rows with "+N more"; an **error badge** with the count and a red top line when the node has errors, amber for
  warnings, with the first problem's sentence on hover; a soft shadow (`shadow.card`); pins with hover tooltips
  that say what the pin means (from the catalogue's pin descriptions).
- **Collapse** (`collapsed = true`, editor-only): header and pins only. **Disable** (`disabled = true`, read by
  the game - part 7): drawn at 40 % with a hatched header, its wires dashed; the validator says a disabled node
  is skipped (note severity).
- **Problems are sentences** on the node and in the Problems drawer (the code in the tooltip and in commands);
  clicking one frames the node. The validator learns the module's own items, sections and characters before it
  warns about an unknown one (17 false `W060` on the author's quest, audit fact 12).

## 4. The picker and what the canvas can do

**The picker** is the kit's `PickList` (stream K): ranked search, the kind's icon and family colour, the
description as a second line on the highlighted row, **Recent** (per project, in the machine state) and
**Favourites** (a star per row, in the editor's settings) on top, then the families. It opens on Tab or Space at
the cursor, on a right click on empty canvas, and when a wire is dragged from a pin and dropped on empty canvas -
then **filtered by the pin**: from an output only kinds that take an input, into an input only kinds with
outputs, in an action slot only actions (today it offers triggers a wire cannot enter, audit fact 5). Picking
connects the new node to the dragged wire.

**Actions** - each a command (part 6) and an action with default keys the author can rebind, in the node's or the
canvas's context menu and the Graph menu:

| Action | What |
|---|---|
| Align Left / Centre / Right / Top / Middle / Bottom, Distribute Horizontally / Vertically | on the selection |
| Straighten Connections | moves the selected nodes' successors so their wires run straight |
| Select Connected / Upstream / Downstream | grows the selection along wires |
| Add Comment Frame | a frame around the selection (C): title, tone (a `graph.family` index), resizable; moving a frame moves the nodes inside it |
| Add Reroute | a waypoint on a wire: double click the wire; drag it; delete it; a wire keeps any number |
| Insert on Wire | drop a node on a wire, or the wire's menu "Insert Node...": A -> B becomes A -> N -> B |
| Replace Node | another kind with the same id; parameters of the same name and wires of the same pin kept |
| Duplicate with Wires | Ctrl+Shift+D: the copy keeps the incoming wires |
| Disable / Enable, Collapse / Expand | toggles on the selection |
| Cut Wires | Ctrl + right drag across wires removes them |
| Rewire | Alt + drag a wire's end to another pin |
| Bookmarks | named, saved in the file (`canvas.bookmarks`), Ctrl+1..9 jump, Ctrl+Shift+1..9 set |
| Auto Layout / Auto Layout Selection / Auto Layout Branch, Frame All / Frame Selection, Find, Find in Project | as today, as actions |

While dragging nodes: snap to grid (a preference) and alignment guides to neighbours' edges and centres. Paste
is in the empty-canvas menu. Copy and paste work between graphs (a quest into a behaviour graph: only kinds the
target may use, the rest reported).

## 5. The tab

- The Graph panel (audit E.18): Tasks or Roles and Variables in ONE scroll with a `+` on each header; the form of
  a variable titled by its name; Global a real section. The kit's `Section`, `Check` / `Switch`, `Combo`,
  `PickList`, `ItemRows` everywhere - `gz-ui-graph/src/local.rs` keeps nothing the kit has (audit B.13).
- The toolbar: Save, Undo, Redo, the problems count when there are any, Frame All, Find, the panel toggles as
  quiet toggles (not accent fills), the zoom; the rest under More. One Primary at most on the tab.
- Under 1400 pt of width the Details panel folds to a rail like the Graph panel (audit B.12); the minimap never
  takes more than a fifth of the canvas's height.
- Canvas text is prepared per scene revision and zoom step (the kit's `text::fit`), the picker measures once
  (audit D rows 6-7). `perf.stalls` stays empty while panning the 110-node quest in the debug build.
- Every menu and the picker can be put up to look at over MCP (`graph.view menu=node|wire|canvas|picker`,
  audit E.29).

## 6. The file and the commands

**The file** (`docs/nq/NQ_FORMAT.md` and `docs/NQ_ARCHITECTURE.md` are changed first; the game ignores all of it
except `disabled`):

```lua
canvas = {
  flow = "right",
  wires = { style = "circuit" },
  frames = { { title = "The ambush", tone = 3, x = 120, y = 40, w = 620, h = 300 } },
  routes = { ["wait.done>ambush"] = { { 400, 120 }, { 400, 260 } } },
  bookmarks = { { name = "Finale", node = "finish" } },
},
nodes = {
  { id = "wait", kind = "wait.time", params = { seconds = 6 }, collapsed = true, out = { done = "ambush" } },
  { id = "ambush", kind = "spawn.squad", disabled = true, ... },
}
```

Renaming a node rewrites the route keys. Everything is written through the document's difference
(`docs/ASSETS.md` 3): moving one node changes one line.

**Commands** (typed cards; the old `quest_*` names go, callers fixed - UI, `tools/qa/test_graph_ui.py`,
`test_behaviours.py`, documents):

| Command | Effect | Replaces |
|---|---|---|
| `graph.edit` (`doc`, `ops: [...]`: add, remove, set, connect, disconnect, move, rename, replace, insert_on_wire, frame, route, bookmark, set_task, set_var, ...; one undo step) | write | `quest_edit`, `quest_apply`, `quest_rename_task` |
| `graph.arrange` (`doc`, `action: layout | align | distribute | straighten`, `nodes`, `scope: all | selection | branch`, `flow`) | write | `quest_layout` |
| `graph.get` (`doc`: the graph as the tab holds it - outline, nodes with boxes and wires' polylines when asked, problems) | read | `quest_layout` (read), `quest_minimap`, `quest_history` |
| `graph.find` (`doc` or the project, `query`) | read | `quest_find`, `quest_project_find` stays `asset.find` |
| `graph.view` (`doc`: zoom, centre, selection, panels, splits, the open menu or picker, wire style) | view | `quest_view`, `quest_bookmarks` |
| `doc.save`, `doc.undo`, `doc.redo`, `doc.reload`, `ui.tab` | - | `quest_save`, `quest_undo`, `quest_redo`, `quest_reload`, `quest_open`, `quest_close` (register the graph documents with the shell's document commands, as stream K's report says) |
| `asset.reference kind=quest`, `game.lookup` | read | `quest_catalog`, `quest_lookup` |

Every action of part 4 is an action of the group "Graph" running one of these (mostly `graph.edit` /
`graph.arrange` on the selection), so each is in the palette, on the Keyboard page and in `help`.

## 7. The game half (a paired change)

In `D:\Games\Dead Air\DeadAir-x64` (read its `PROJECT_RULES.md` first; change only the files named here; no git;
do not deploy - write in your report exactly which files changed so the architect deploys them):

- **Disabled nodes**: the NQ runtime (`gamedata\scripts\xms_nq_*.script`: the loader's node table and the core's
  node entry) treats a node with `disabled = true` as a pass-through: entering it runs no action and no wait and
  leaves through its normal continuation pin (the kind's first non-failure output in catalogue order); a disabled
  trigger never fires; a node with no continuation ends the branch. Saves are unaffected (a node id is still a
  node id). The game's documents first: `docs\dead-air\NQ_RUNTIME.md` (a short section) - and this repository's
  `docs/NQ_ARCHITECTURE.md` and `NQ_FORMAT.md`.
- **`summary`** per kind in `gamedata\configs\nq\catalog.ltx`: the name of the parameter that tells a node or an
  action apart at a glance; the runtime ignores the key (check that the catalogue parity test,
  `tools\qa\Test-NqCatalogParity.ps1`, still passes - run it). This repository's bundled copy
  (`gamedata\configs\nq\catalog.ltx`) stays byte-identical to the game's: change both.
- The loader ignores the top-level `canvas` and the node keys `collapsed` (verify in `xms_nq_load.script` and say
  where; if a key is refused, make it ignored).
- Proof on the game side: the headless Lua tests (`python tools\nq\run_lua.py test_examples.lua`) with new cases
  for a disabled action node, a disabled wait, a disabled trigger and a disabled node inside a branch.

## Proof

- Unit tests: each wire style's polyline for forward, backward and sideways pairs in both flows (snapshot the
  points); lanes never overlap; hit testing on every style; the picker's pin filter; each action's result on a
  small graph; layout wraps (the 110-node fixture - copy its text into a test fixture with the author's strings
  replaced by placeholders - is under 4000 pt on its long side and has no overlapping boxes); frames move their
  nodes; routes survive a rename; semantic zoom thresholds; round trip of `canvas` through the file with comments
  and unknown keys kept.
- QA: `tools/qa/test_graph_ui.py` rewritten for the new commands and extended (each wire style drawn and hit;
  the picker from a pin; frames, reroutes, disable, collapse, align; Frame All of the big quest readable: every
  header's text at least 11 px), `test_behaviours.py` for behaviour graphs.
- Pictures into `Intermediate/plan/G/` with a caption each in your report: the same graph in the five wire
  styles and both flows; the big quest framed; a node of every family; the picker from a pin; frames and
  reroutes; a disabled and a collapsed node. Look at each yourself: would the author call it a modern graph
  editor? If not, it is not done.

## Done means

PLAN section 5 "G", every sentence; the audit's C.2 list has no "missing" left except what this brief does not
ask for (real subgraphs); the documents (`NQ_FORMAT.md`, `NQ_ARCHITECTURE.md`, `UI_DESIGN.md`'s graph section,
the game's `NQ_RUNTIME.md`) say what was built.

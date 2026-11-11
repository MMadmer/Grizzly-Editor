# Brief: the AI Map, Detail Objects and Wallmark tools

The scene already holds the AI map, the details and the wallmarks (`SceneState::bulk`), saves
them, builds them into a level and - since the previous package - draws them. What is missing
is **editing them**: the C++ editor's AI Map tool, the Detail Objects tool's generation and the
Wallmark tool. This package ports the three.

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first (its sections "The object
tools" and "From the renderer" tell how the neighbouring tools and the drawing were done:
follow the same shape); `docs/UI_DESIGN.md` sections 1, 1a, 2, 4 and what section 6 says of the
Tools panel; the rustdoc at the top of `gz-scene` (`BulkData`: the AI map in 4096-node tiles,
`DetailsPart`, the wallmarks; `SceneCmd`; the pick tree; snap objects), `gz-ui-level`
(`tools*.rs`), `gz-ui-viewport` (gestures, `draw*.rs`: what is drawn of these three), `gz-task`,
`gz-fmt-level` (the AI map, details and wallmark formats).

You own: new modules of `crates/gz-scene` (with the additions to `cmd.rs` / `apply.rs` they
need), the three tool bodies in `crates/gz-ui-level/src/tools*.rs`, the gestures of the three
tools in `crates/gz-ui-viewport`, what `crates/gz-ui-viewport/src/draw*.rs` needs to show an
edit as it happens (brushes, the selection, a preview), and new entries of
`crates/gz-mcp/tools.json`. Another agent works in the tree at the same time in
`gz-fmt-object`, `gz-fmt-ogf`, `gz-levelbuild`, `gz-app`: touch nothing of those; if `cargo
check` fails in a crate that is not yours, wait and retry.

## The C++ that is the spec (`Source/Editors/LevelEditor/`)

- **AI Map**: `UI/Tools/UIAIMapTool.cpp` (the panel); `Editor/Tools/AIMap/ESceneAIMapTools.cpp`,
  `_Generate.cpp` (1k: generation over snap objects, node creation, linking, the constraints),
  `_MotionSimulator.cpp` (the capsule slide that decides where a node can be walked to),
  `_Pick.cpp`, `_Controls.cpp`, `ESceneAIMapControls.cpp` (add, select, the link gestures),
  `_Export.cpp` (already ported in the build - reuse, do not copy).
  Every control: Generate Full, Generate Selected, Clear AI Map... , Reset Selected, Smooth
  Selected, Select 0-Link / 1-Link / 2-Link, Invert (selection), the Links block (Add, Remove,
  Invert on the chosen side: towards / away from the camera, left, right, all four sides),
  Ignore Constraints, Auto Link, Ignored Materials (Add..., Remove, Clear) and the Nodes / Mode
  rows; adding nodes with the brush, removing them, selecting by click and by box.
- **Detail Objects**: `UI/Tools/UIDOTool.cpp`; `Editor/Tools/Details/ESceneDOTools.cpp`,
  `_Generate.cpp` (the slots from the base texture's colours and the snap objects: First
  Initialize, Reinitialize All, Reinitialize Objects, Reinitialize Selected, Update Renderer),
  `_Pick.cpp`, `_Controls.cpp` (slot selection). `dlg.detail_objects`, Clear Slots and Clear
  Details exist already: keep them, add the rest; "Show the current details in the viewport"
  is the tool's view flag.
- **Wallmarks**: `Editor/Tools/Wallmark/ESceneWallmarkTools.cpp` (placing a wallmark on the
  geometry under the cursor: the clipped polygon built against the scene's faces, its texture
  and shader, width / height / rotation and their random ranges, the alignment, moving,
  selecting, deleting; the property sheet), `ESceneWallmarkControls.cpp`.

## Architecture (decided)

- **Every action is a command**, reachable over MCP: `aimap_tool`, `details_tool`,
  `wallmark_tool`, each with `action=` (generation, clear, selection queries, link edits, node
  add / remove at positions, slot reinitialisation, wallmark place / move / delete with their
  parameters), each with its `grizzly_<name>` entry in `crates/gz-mcp/tools.json`. Buttons and
  gestures post those commands; an agent can do with `action=` everything the mouse does.
- **Generation is a job** (group `scene`, cancelable, a Task Center card with nodes or slots
  done): it reads a scene snapshot and the pick tree on the CPU pool and its result enters the
  scene as ONE command, one undo step. A result computed from a snapshot the scene has left
  behind is dropped. The UI never waits and the scene stays editable meanwhile.
- **The result is the C++ result**: the same nodes, planes, links and the same detail slots for
  the same scene - the build's `build.aimap` and `build.details` are compared with the C++
  editor's later. Port `_Generate.cpp` and `_MotionSimulator.cpp` float for float (`f32`, the
  same order of operations, the same epsilons, the same random where there is one). Parallelise
  only where the result stays equal: ray batches, independent slots, per-tile work merged in
  the C++ order; where the C++ floods from node to node in an order that decides the outcome,
  keep that order and make each step fast instead.
- **Edits are small**: the AI map lives in tiles (`imbl`), so an edit copies only the tiles it
  touches; undo holds the previous tiles, never a copy of the map. A brush stroke is one merged
  undo step, sealed on release. Selection of nodes is part of the tool's state as the scene
  keeps it for other tools.
- The panels follow `docs/UI_DESIGN.md` 1a: the common actions on the surface (Generate,
  Clear, the brush mode), the rest under one folded More; a disabled control says why; a
  destructive one is GhostDanger and confirms through the kit naming what goes (node, slot
  counts) - and says it can be undone, because here it can.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` in your file, named in the report.
- A frame never computes and never touches a file. No `unsafe`, no global state.
- Tests are written, not run: generation on a hand-made scene (a flat floor gives the grid of
  nodes the C++ arithmetic gives, a step higher than the limit breaks the links, an ignored
  material leaves a hole), the link edits, a detail slot from a two-colour base texture, a
  wallmark clipped against a box corner.
- Compile: `bash tools/rust/cg.sh check -p gz-scene -p gz-ui-level -p gz-ui-viewport -p
  grizzly`, zero warnings; clippy clean for your crates. Then build and LOOK, on a temp COPY of
  `D:\XFinedProjects\Test` (the Escape scene): each panel; Generate Selected on a snap object
  and the nodes it made in the picture; links edited; details reinitialised on a selection; a
  wallmark placed over MCP and drawn; `ui_audit` shows nothing escaped; `frame_pacing` while a
  generation runs shows the UI was not blocked.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. Never start `Grizzly.exe` by hand.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 400 words: per tool what is built and its commands, `local_*` widgets, what the look
showed, generation time on Escape, anything of the C++ you could not port and exactly why.
Then add a section "The AI map, details and wallmark tools" to `docs/rust/STATE.md`.

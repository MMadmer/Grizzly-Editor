# Brief: the tools of sectors, portals, ways, shapes, lights and fog volumes

The Level tab's Tools panel draws "This tool has no settings of its own." for every class but
Object, Spawn, Particles, Group and Detail Objects. The C++ editor has a tool panel, commands
and viewport gestures for six more; this package ports them: **Sector, Portal, Way, Shape,
Light, Fog Volume**. (AI Map, Detail Objects' generation and Wallmarks are the next package -
do not start them.)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; `docs/UI_DESIGN.md` sections
1, 1a, 2, 4 and what section 6 says of the Level tab's Tools panel; the rustdoc at the top of
`gz-scene` (the store, `SceneCmd`, the objects of these classes, the pick tree, `library`),
`gz-ui-level` (`tools.rs`: how a tool body is drawn and how it talks to the scene),
`gz-ui-viewport` (how a gesture becomes `SceneCmd`s, `set_place_item`, picking), `gz-task`
(jobs), `gz-editor`.

You own: new modules of `crates/gz-scene` (and the additions to its `cmd.rs` / `apply.rs` they
need), the tool bodies in `crates/gz-ui-level/src/tools*.rs` (split the file by tool when it
helps), the gesture code of `crates/gz-ui-viewport` (NOT its `draw*.rs` / `lighting.rs` and NOT
`crates/gz-render` - another agent is changing how things are drawn; if a tool needs a helper
drawn that is not drawn today, add it to the `DrawList` through the existing helper API in a
function of your own file and say so in the report), and new entries of
`crates/gz-mcp/tools.json`. Two other agents work in the tree at the same time, in other
crates: touch nothing else, and if `cargo check` fails in a crate that is not yours, wait and
retry.

## The C++ that is the spec (`Source/Editors/LevelEditor/`)

| Tool | Panel | Logic |
|---|---|---|
| Sector | `UI/Tools/UISectorTool.cpp` | `Editor/Entry/Sector/sector.cpp`, `Editor/Tools/Sector/ESceneSectorTools.cpp`, `ESceneSectorControls.cpp` |
| Portal | `UI/Tools/UIPortalTool.cpp` | `Editor/Entry/Portal/portal.cpp`, `Editor/Utils/portalutils.cpp`, `Editor/Tools/Portal/EScenePortalTools.cpp`, `EScenePortalControls.cpp` |
| Way | `UI/Tools/UIWayTool.cpp` | `Editor/Entry/WayPoint/WayPoint.cpp`, `Editor/Tools/WayPoint/ESceneWayControls.cpp` |
| Shape | `UI/Tools/UIShapeTool.cpp` | `Editor/Entry/Shape/EShape.cpp`, `Editor/Tools/Shape/ESceneShapeTools.cpp`, `ESceneShapeControls.cpp` |
| Light | `UI/Tools/UILightTool.cpp` | `Editor/Tools/Light/ESceneLightTools.cpp` |
| Fog Volume | `UI/Tools/UIFogVolTool.cpp` | `Editor/Tools/FogVolume/ESceneFogVolumeTools.cpp` |

The shared control base is `Editor/Tools/Base/ESceneControlsCustom.cpp` (already ported in the
viewport: check what it lacks for these tools' sub-modes).

Every control of those panels and every mode of those controls is in scope. By name, so that
nothing is missed:

- **Sector**: Create Default, Remove Default... (confirm), Validate, Capture Volume, Distribute
  Objects; the Meshes mode Add / Remove with Box Pick (meshes of scene objects added to or
  taken from the sector by a click or a dragged box); New Sector From Mesh, New Sectors From
  Objects; "Not while a new sector is being made".
- **Portal**: Compute All... / Compute Selected... (confirm: it cannot be undone in the C++ -
  here it is ONE undo step, so the confirmation names what is replaced and says it can be
  undone), Remove Duplicates..., Invert Orientation; "Select two sectors or more first".
- **Way**: the Ways / Points select modes; Auto Link; Create 1-Link, Create 2-Link, Convert to
  1-Link, Convert to 2-Link, Invert Link, Remove Link; adding points to the selected way by a
  click, moving points, selecting points - with "Switch Select to Points first" where the C++
  says it.
- **Shape**: what a click adds (Box / Sphere), Attach on Click, Attach, Detach Selected...
  (confirm), Edit Level Bound, Recalculate the level bound.
- **Light**: Viewport Lighting - Enable / Disable All and Selected (which lights light the
  viewport), with the C++ state texts.
- **Fog Volume**: Group Selected, Ungroup Current.

## Architecture (decided)

- **Every action is a command** and reachable over MCP: one command per tool with `action=`,
  named `sector_tool`, `portal_tool`, `way_tool`, `shape_tool`, `light_tool`, `fog_tool`
  (answers in the style of the scene commands: what changed, counts, ids; refusals as the C++
  texts). Each gets its tool in `crates/gz-mcp/tools.json` (`grizzly_<name>`, described like
  its neighbours; nothing existing changes). The panel's buttons post those commands; a panel
  never changes the scene itself.
- **A change is one `SceneCmd`, one undo step**, applied by the store. Where the C++ said "this
  cannot be undone" the Rust editor undoes it.
- **Geometry runs off the UI thread.** Computing portals, capturing a sector's volume,
  distributing objects, the level bound: a job on the CPU pool over a scene snapshot and the
  pick tree (cancelable, shown in the Task Center when it takes longer than a frame), whose
  result is applied as one command; a result computed from a snapshot the scene has left behind
  is dropped and computed again, never applied to a newer scene.
- **The algorithms are the C++ ones**, ported exactly: the same sectors and portals come out of
  the same scene (`portalutils.cpp`: the adjacency, the edge collection, the convex portals,
  the orientation, the similar-portal test; `sector.cpp`: the volume capture and the hull). Use
  `parry3d` where it is the same computation, port the code where the result would differ.
  Parallelise where the result stays the same (per sector pair, per mesh, ray batches with
  `rayon`); a result that depends on order is collected in the C++ order.
- **Gestures are commands too**: a drag is one merged undo step sealed on release; a sub-mode
  (Meshes Add / Remove, Points, Attach on Click, Edit Level Bound) is tool state in the scene's
  `ToolSettings` (or beside it, where the other tools keep theirs), set by the tool command, so
  an agent can do with `action=` everything a click does: every gesture has its command form
  (`way_tool action=add_point way=<name> position=...`, `sector_tool action=add_mesh
  sector=<name> object=<name> mesh=<n>`, ...).
- The panels follow `docs/UI_DESIGN.md` 1a: the common actions on the surface, the rest under
  one folded More, a disabled control says why in its tip, a destructive one is GhostDanger and
  confirms through the kit, naming what goes.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` in your file, named in the report.
- A frame never computes and never touches a file. No `unsafe`, no global state.
- Tests are written, not run: for each algorithm a test on a small hand-made scene with the
  result worked out from the C++ (two boxes sharing a wall give one portal of four vertices, and
  so on).
- Compile: `bash tools/rust/cg.sh check -p gz-scene -p gz-ui-level -p gz-ui-viewport -p grizzly`,
  zero warnings; clippy clean for your crates (`bash tools/rust/cg.sh clippy -p gz-scene -p
  gz-ui-level -p gz-ui-viewport`). Then build and LOOK, on a temp COPY of
  `D:\XFinedProjects\Test` (the Escape scene; never the real project): each of the six panels,
  a portal computed between two sectors made from objects, a way with links edited, a shape
  attached; `ui_audit` shows nothing escaped. Drive it over MCP with your new commands.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 400 words: per tool what is built and its commands, `local_*` widgets, what the look
showed, anything of the C++ panels you could not port and exactly why. Then add a section "The
object tools" to `docs/rust/STATE.md` (short: the commands, the deviations from the C++).

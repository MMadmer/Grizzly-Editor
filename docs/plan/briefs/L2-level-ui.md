# Brief L2 - the level editor's face

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/UI_DESIGN.md` (1, 1a,
the kit, the frame, and the Level sections you rewrite), the reports of the streams you build on
(`docs/plan/reports/K.md`: kit widgets, `drop`, actions, keys, preference pages; `L1a.md`: one selection,
`SceneMode`, eye and lock, the `scene.*` commands, the property labels; `L1b.md`: the build pipeline, its
steps and states, the modes' data, `level.*`; `S.md` if it exists: the Content Browser's drag payloads), and the
audit **`Intermediate/plan2/ux_level.md`** - all of it: A (what exists), B (what is the X-Ray SDK's), C (layout
and visual defects), D (what still runs on the UI thread), E (the 25 changes) - with its pictures
(`Intermediate/plan2/shots/level/`).

You own: `crates/gz-ui-level`, `crates/gz-ui-viewport`, and `crates/gz-render` where the audit's D names it.

The author's words: the interface still follows the X-Ray SDK's pipelines - the same placement of elements, the
same way of drawing them; cut that legacy and build it your own way. Unreal Engine 5's level editor is the
reference for how it should feel; the audit's B sections say, item by item, what UE does instead.

## 1. The layout

```text
[ Save ] [ Build Level v ] [ Play v ] [ Stop ]        (labelled; ... holds the rare ones)        Build Mod | Publish | Releases
+---------------+---------------------------------------------+------------------+
| Place         |  [Select|Place|Paint|Zones]  viewport pills |  World Outliner  |
|  search       |                                             |                  |
|  Recent       |                                             +------------------+
|  Objects      |               the viewport                  |  Details         |
|  Lights ...   |                                             |  (Level Settings |
+---------------+---------------------------------------------+   as a tab)      |
| Content Browser | Output Log                                |                  |
+---------------------------------------------------------------------------------+
```

- **No Tools panel of sixteen tiles** (audit B1, C). The left panel is **Place** in Select and Place modes, and
  the mode's own panel in the others.
- **Modes** (a segmented control at the viewport's top left; keys 1-4; `scene.mode`): **Select** (the default:
  click, marquee, gizmo on anything), **Place** (the palette is armed: clicks place the armed thing until Esc),
  **Paint** (its panel picks the layer: AI Map, Foliage, Decals - stream L1b's models and commands), **Zones**
  (visibility cells for the author who wants them by hand - L1b's `scene.zones`). A mode owns the viewport's
  clicks while it is on; it never filters what Select can select.
- **Place panel**: search on top, Recent (the last 12 placed), then categories - Objects (the library's tree,
  virtualized, with thumbnails), Lights, Spawn Points (the game's sections by their in-game names, grouped),
  Shapes and Volumes, Sounds, Particles, Glows, Fog, Way Paths, Groups. A row is dragged into the viewport or
  clicked to arm it (Place mode). One list, one search - the Object tool's second copy of the browser's tree
  goes (audit B2).
- **Drag and drop into the viewport** from the Place panel and from the Content Browser (the kit's `drop`
  payloads): a ghost follows the cursor on the surface under it (the existing ground pick), with the snapping
  that is on; dropping places it as one undo step and selects it. Alt + dragging a gizmo handle duplicates.
- Everything the audit's B1 lists is gone from the UI: Pick Any Class, the click that switches a tool, per-class
  Select All / Delete / Hide, the target keys.

## 2. The viewport

- **Gizmo** (audit B3): axis and plane handles, a screen-space centre, rotation rings with an arc readout, 3 pt
  lines with hover highlight, World / Local toggle (in the pills; a key). What it can do follows the selection
  (L1a). Way points move like objects.
- **Snapping pills** with units: move `0.01 0.05 0.1 0.25 0.5 1 2 5 10 m`, rotate `5 10 15 22.5 30 45 90 deg`
  (default 15), scale its own list; surface snap one toggle with its options behind the chevron; vertex snap and
  normal align off by default.
- **View Mode** pill: Unlit / Game Lit (weather and hour) / Wireframe. The Direct3D 9 switches go (Fill and
  Shade modes, Linear Filtering, Textures, the fixed-function Lighting with its eight slots, "Use In D3D",
  the Light tool's Viewport Lighting - audit B12); the GPU choice moves to Preferences > Viewport.
- **Show** pill: Grid, Safe Rect, Stats, helpers per kind (names, crosses, nodes - the toggles L1a took out of
  Level Settings), Fog. One place, no duplicate in another menu.
- **Keys** (audit B13), all of them actions the author can rebind: W / E / R, Q select, Ctrl+F find in the
  Outliner, F frame selection, H / Ctrl+H hide / unhide, Ctrl+G group, End drop to floor, Ctrl+D duplicate,
  Delete, 1-4 modes, F5 build, F9 play - a keymap a UE user's hands know. Macros and target keys go.
- **The camera** stays as it is now: a click selects on release, a drag moves the camera, the cursor is held in
  place (`docs/UI_DESIGN.md`, "The viewport's mouse").

## 3. Outliner, Details, Level Settings

- **Outliner** (audit B9, B11, C): kinds ordered by label with counts; an eye and a lock per kind and per
  object; leaf names with a middle ellipsis; scroll-to-selection that waits for the fresh listing (the audit
  names the bug); "By Hierarchy" shows groups and their members, way paths and their points; type filter; search
  (Ctrl+F focuses it).
- **Details** (audit B10, C): the labels, units and words L1a's table gives; a search box; sections by the kit's
  `Section`; Transform first, a Vec3 on the line under its label (the kit: `grid.rs:851`), reset arrows on
  changed values; multi-selection shows shared rows; advanced rows under the section's More; pickers for
  everything that comes from a list - **the chooser** is tall, with thumbnails and a preview for models,
  textures and sounds (audit C: 7 rows of 5458 today).
- **Level Settings** keeps only what is the level's (L1a cleaned the model): no view toggles, no tool
  parameters, no per-class switches.

## 4. Build and Play

One labelled **Build Level** split button: the click builds what is stale (`level.build`); its menu lists the
steps with their state (fresh / stale and why / failed) to build one, **Rebuild All**, and Advanced (the mod's
level layers, the folder compiler). The status bar's chip names what is stale and opens the same popover; it is
right after a restart and does not poll (L1b). Build problems are a list in the Output Log's sibling tab
**Problems**: a sentence, the object, a click selects and frames it. **Play** split: Play (builds what is stale,
starts the game), Play Clean, Play Without Building; **Stop** while it runs. The Build menu of the caption holds
the same actions; Make / Compile / Import-Export Error List are gone.

## 5. The modes' panels

- **Paint > AI Map**: Generate (inside the bounds volume or over the level), the parameters, Clear...; under
  More: the brush and link editing. **Paint > Foliage**: the mesh list (drop meshes from the browser), density /
  scale / rotation per mesh, brush size, Paint / Erase / Fill; under More: import or export a colour-index
  texture. **Paint > Decals**: the wallmark texture and size, click to place.
- **Zones**: the cells and portals as a list with eye; New Cell from Selection, Compute Portals, Validate; a
  first line that says what the build does by itself, so an author knows they may never need this mode.
- One danger style for what loses work (the kit's GhostDanger with the confirm), disabled buttons look disabled,
  no helper copied twice (audit C names the duplicates).

## 6. Nothing blocks (audit D)

D1-D7 in your crates (the tree rows as a `Query`, chooser rows off the UI thread, the owner items as a `Query`,
a per-frame budget for thumbnail uploads - with stream S if the code is in `gz-ui-content` -, the image
pipeline prewarmed, the Game Folder stat off-thread); the render thread reads label visibility one pass late
instead of waiting for the GPU every pass, and stops contending with the UI thread's submits
(`gz-render/src/renderer.rs:394-397`, `frame.rs:310-340`). `perf.stalls` is empty after: opening the largest
base scene, selecting 3000 objects, typing in every search box, opening every chooser.

## 7. Actions and commands

Every menu row, toolbar button, pill and key of the level is an action over a command (stream K's
`actions.rs`). With that the last users of the old command table are gone: delete `gz-app/src/cpp.rs`,
`exec_command`, `list_commands`, the macros, `UiCommand` (K's report lists what it left for you), and fix the
QA suites that used them. Your view commands, typed: `viewport.camera` (get / set / focus / bookmark),
`viewport.view` (mode, show flags, snapping, gizmo space), `viewport.gesture` (a click or a drag played as the
mouse would - QA's way to prove picking and dragging), `shot.viewport`, `perf.frames`, `ui.panel` for your
panels, `level.view` (the mode's panel state, the open popover - to look at). The old names
(`camera_get/set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`, `game_lighting`,
`frame_pacing`, `screenshot_viewport`, `viewport_gesture`, `outliner_show`, `outliner_filter`,
`scene_visibility`, `group_thumbnail`, `spawn_animation`, `particles_tool`) go, callers fixed.

## 8. `docs/UI_DESIGN.md`

The Level sections rewritten to what is built: the layout, the modes, each panel, the pills, the keys, Build and
Play - in the present tense, no history.

## Proof

- Unit tests through the egui harness for what can be driven there (the mode switch, the place palette's arm
  and drop payload, the gizmo's hit testing per handle and the ring's angle, snapping steps, the outliner's
  scroll target on a stale listing).
- QA: `tools/qa/test_scene_tools.py` and `test_properties.py` (their UI phases) rewritten and extended: drag
  from the palette into the viewport places on the surface (`viewport.gesture` with a payload); a marquee over a
  light and an object selects both and Delete removes both; the gizmo's plane handle moves in its plane; Build
  Level's popover lists the steps; `perf.stalls` empty at the end. `test_ui_shell.py`: the level's rects.
- Pictures into `Intermediate/plan/L2/` with a caption each in your report: the tab at 1600 x 900 and 1280 x 720
  in each mode, the gizmo in each state, every pill's popup, Details for each kind, the chooser, the build
  popover, the Problems list. Look at each yourself beside the audit's picture of the same thing: is it a
  modern level editor a UE user would find their way in without the manual? If not, it is not done.

## Done means

PLAN section 5 "L2": every item of the audit's B, C and D is fixed or answered in your report with a reason; no
tile grid, no edit target, no SDK render switch, no unlabelled build icon, no `COMMAND_*` table.

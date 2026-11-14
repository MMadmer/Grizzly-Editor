# Brief L2b1 - the level's left side: the Place panel, the Outliner, the modes' panels

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/UI_DESIGN.md` (1, 1a,
the kit, the frame, and the Level sections), the reports of the streams you build on (`docs/plan/reports/K.md`:
kit widgets, `drop`, actions, keys, preference pages; `L1a.md`: one selection, `SceneMode`, eye and lock, the
`scene.*` commands, the property labels; `L1b.md`: the build pipeline, its steps and states, the modes' data,
`level.*`, and what it asks of you), and the audit **`Intermediate/plan2/ux_level.md`** - A (what exists), B (what
is the X-Ray SDK's), C (layout and visual defects), D (what still runs on the UI thread), E (the 25 changes) -
with its pictures (`Intermediate/plan2/shots/level/`).

The author's words: the interface still follows the X-Ray SDK's pipelines - the same placement of elements, the
same way of drawing them; cut that legacy and build it your own way. Unreal Engine 5's level editor is the
reference for how it should feel; the audit's B sections say, item by item, what UE does instead.

You own, in `crates/gz-ui-level`: `layout.rs`, `dock.rs`, `tools.rs`, `tools_bulk.rs`, `tools_objects.rs`,
`classes.rs`, `local.rs`, `outliner.rs`, `outliner_model.rs`, `detail_objects.rs`, `content.rs`. Two engineers work
beside you: **L2b2** in the same crate (`details.rs`, `choose.rs`, `lists.rs`, `doc.rs`, `toolbar.rs`, `menus.rs`,
`level_cmds.rs`, `status.rs`, `cmd.rs`, `lib.rs`, `motion_ui.rs`, `import.rs`, `recovery.rs`: Details, the
chooser, Build and Play, the menus, the level's actions) and **L2a** in `crates/gz-ui-viewport` (everything
inside the viewport's rectangle: the mode control, the pills, the gizmo, the drop target, the brushes). Stream S
works in the Content Browser. What crosses between you is typed and already exists: the scene's mode
(`SceneMode`), the kit's drag payloads (`gz_ui::drop`), commands. A file of the crate that is not on your list
is L2b2's: when you need a line there (a panel registered in `lib.rs`), make exactly that edit, re-reading the
file right before, and say so in your report.

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

The mode control and the drop into the viewport are stream L2a's; the Place panel is yours: its rows are drag
SOURCES (the kit's payloads) and clicking one arms it for Place mode (you write the armed thing into the
scene's view state through `scene.mode` / the scene's commands; L2a reads it).

The mode control and the drop into the viewport are stream L2a's; the Place panel is yours: its rows are drag
SOURCES (the kit's payloads) and clicking one arms it for Place mode (you write the armed thing into the
scene's view state through the scene's commands; L2a reads it). The toolbar row of the picture is L2b2's.

## 2. The Outliner

- **Outliner** (audit B9, B11, C): kinds ordered by label with counts; an eye and a lock per kind and per
  object; leaf names with a middle ellipsis; scroll-to-selection that waits for the fresh listing (the audit
  names the bug); "By Hierarchy" shows groups and their members, way paths and their points; type filter; search
  (Ctrl+F focuses it).

## 3. The modes' panels

- **Paint > AI Map**: Generate (inside the bounds volume or over the level), the parameters, Clear...; under
  More: the brush and link editing. **Paint > Foliage**: the mesh list (drop meshes from the browser), density /
  scale / rotation per mesh, brush size, Paint / Erase / Fill; under More: import or export a colour-index
  texture. **Paint > Decals**: the wallmark texture and size, click to place.
- **Zones**: the cells and portals as a list with eye; New Cell from Selection, Compute Portals, Validate; a
  first line that says what the build does by itself, so an author knows they may never need this mode.
- One danger style for what loses work (the kit's GhostDanger with the confirm), disabled buttons look disabled,
  no helper copied twice (audit C names the duplicates).

## 4. Nothing blocks (audit D)

D1 (the tree rows of the Place panel as a `Query`: no rebuild per keystroke) and every other audit D row that
names your files. `perf.stalls` is empty after: opening the largest base scene, selecting 3000 objects in the
Outliner, typing in the Place panel's and the Outliner's search.

## 5. Commands and documents

`level.view` (the mode's panel state, what the Place panel shows and has armed - to look at), `ui.panel` for
your panels; the old names (`outliner_show`, `outliner_filter`, `scene_visibility`) go, callers fixed (UI, QA
suites, documents). `docs/UI_DESIGN.md`: the layout, the Place panel, the Outliner and the modes' panels, in the
present tense, no history.

## Proof

- Unit tests through the egui harness: the Place palette's search, arming and drag payload; the outliner's
  scroll target on a stale listing; eye and lock per kind and per object.
- QA: the panel phases of `tools/qa/test_scene_tools.py` for your panels, rewritten and extended; `perf.stalls`
  empty at the end. `test_ui_shell.py`: the level's rects at Wide and Compact.
- Pictures into `Intermediate/plan/L2b1/` with a caption each in your report: the tab at 1600 x 900 and
  1280 x 720 in each mode, the Place panel with a search, the Outliner by type and by hierarchy, each mode's
  panel. Look at each yourself beside the audit's picture of the same thing: would a UE user find their way
  without the manual? If not, it is not done.

## Done means

Every item of the audit's B4, B5, B7, B8, B9, B11 (the Outliner's part) and the left side's part of B1, B2 and C
is fixed or answered in your report with a reason; no tile grid, no edit target, no per-tool object picker.

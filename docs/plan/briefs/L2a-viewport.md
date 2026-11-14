# Brief L2a - the viewport

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

You own: `crates/gz-ui-viewport`, and `crates/gz-render` where the audit's D names it. Streams L2b1 and L2b2 work beside you
in `crates/gz-ui-level` (L2b1: the Place panel, the Outliner, the modes' panels; L2b2: Details, the toolbar, the
menus, Build and Play); stream S in the
Content Browser. What crosses between you is typed and already exists: the scene's mode (`SceneMode`, stream
L1a), the kit's drag payloads (`gz_ui::drop`), commands. You draw everything inside the viewport's rectangle,
its pills included; they draw everything around it.

## 1. The viewport

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

## 2. Modes inside the viewport

- **The mode control** is yours: a segmented control at the viewport's top left - Select | Place | Paint |
  Zones - with keys 1-4 (`scene.mode`). A mode owns the viewport's clicks while it is on; it never filters what
  Select can select.
- **Select**: a click selects any visible, unlocked thing (on release, under 4 pt of movement - "The viewport's
  mouse" stays as it is), Ctrl + drag is the marquee over every kind, the gizmo works on what is selected.
  Everything the audit's B1 lists is gone from your crate: the tool class in picks and the marquee, the click
  that switches a tool, per-class behaviour, the target keys.
- **Drop target**: the viewport takes the kit's drag payloads - an object, a group, a particle, a sound, a spawn
  section, a light or another thing of the Place palette (stream L2b1's panel and stream S's browser are the
  sources). A ghost follows the cursor on the surface under it (the existing ground pick), with the snapping
  that is on; dropping places it with `scene.place` as one undo step and selects it. **Place** mode is the same
  without the drag: the armed thing (L2b1's palette arms it; read it from the scene's view state) is placed at
  each click until Esc. Alt + dragging a gizmo handle duplicates.
- **Paint** (AI map nodes and links, foliage, decals) and **Zones**: the brush, its cursor ring and the clicks in
  the viewport are yours (`bulktools.rs`, `objtools.rs`, the `draw_*` files), over stream L1b's commands
  (`scene.ai_map`, `scene.details`, `scene.wallmarks`, `scene.zones`); the panels with their parameters are
  L2b1's.

## 3. Nothing blocks (audit D)

The render thread reads label visibility one pass late instead of waiting for the GPU every pass, and stops
contending with the UI thread's submits (`gz-render/src/renderer.rs:394-397`, `frame.rs:310-340`; stream L1b's
report describes the fix it found); every other audit D row that names your crate. `perf.stalls` is empty after:
opening the largest base scene, a marquee over 3000 objects, dragging a gizmo, flying the camera.

## 4. Actions and commands

Every pill, menu row and key of the viewport is an action over a command (stream K's `actions.rs`; its report
lists the bindings it left in your `keys.rs`: GIZMO_MODE, CHANGE_ACTION, ZOOM_EXTENTS, TOGGLE_RENDER_WIRE / GRID /
SAFE_RECT, GRID_SLOT_SIZE, DROP_TO_GROUND, ...). After you, `gz-ui-viewport` uses no `UiCommand`, no
`cpp_command` and no `COMMAND_*` id - stream L2b2 deletes that machinery from `gz-app` when both of you are off
it. Your commands, typed: `viewport.camera` (get / set / focus / bookmark), `viewport.view` (mode, show flags,
snapping, gizmo space), `viewport.gesture` (a click, a drag or a drop played as the mouse would - with a payload
for a drop: QA's way to prove picking, dragging and placing), `shot.viewport`, `perf.frames`. The old names
(`camera_get/set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`, `game_lighting`,
`frame_pacing`, `screenshot_viewport`, `viewport_gesture`, `group_thumbnail`, `spawn_animation`,
`particles_tool`) go, callers fixed (UI, QA suites, documents).

## 5. `docs/UI_DESIGN.md`

The viewport's sections rewritten to what is built - the pills, the gizmo, the modes in the viewport, the
mouse, the keys - in the present tense, no history.

## Proof

- Unit tests through the egui harness: the gizmo's hit testing per handle, the ring's angle, plane moves,
  snapping steps, the drop ghost's surface point, the mode control.
- QA: the viewport phases of `tools/qa/test_scene_tools.py` rewritten and extended: a drop from a payload places
  on the surface (`viewport.gesture`); a marquee over a light and an object selects both; the gizmo's plane
  handle moves in its plane; ring rotation by 90 degrees; `perf.stalls` empty at the end.
- Pictures into `Intermediate/plan/L2a/` with a caption each in your report: the viewport at 1600 x 900 and
  1280 x 720 in each mode, the gizmo in each state, every pill's popup, the drop ghost. Look at each yourself
  beside the audit's picture of the same thing: would a UE user find their way without the manual? If not, it
  is not done.

## Done means

Every item of the audit's B3, B12, B13 and the viewport's part of B1, B2, C and D is fixed or answered in your
report with a reason; no edit target, no SDK render switch, no `COMMAND_*` in your crate.

# Report L2a - the viewport

## 1. Built

**The gizmo (`crates/gz-ui-viewport/src/gizmo.rs`, rewritten)** - Move: three arrows, the XY / XZ / YZ squares and a
screen-space centre that moves in the view's plane. Rotate: a ring per axis, an outer screen ring, and while it turns
a filled arc from where the drag began with the angle as a readout next to it. Scale: three axes with a box head and a
centre that scales all three. Lines are 3 pt and the hovered handle lights up. Handles are hit-tested in screen
points (`hit`): the centre first, then the planes, then the nearest axis or ring within 6 pt, then the screen ring. A
ring seen edge-on turns by the pointer's sweep along its edge (`edge_angle`). World / Local (`Space`): Local takes
the first selected object's axes. The gizmo stands at the selection's centre (`Anchor`), drawn at a fixed distance
along the ray through it, so its size on screen never changes. The pure maths is `move_delta`, `moved`,
`ring_angle`, `scale_factors`, `scaled` and `turned`, and the pane only feeds it.

**Drags (`pane.rs`)**
- A handle takes a left press before the camera does, so Alt + drag on a handle copies the selection
  (`gz_scene::edit::duplicate`, one undo step with the move) and moves the copy.
- Way points move like objects: a click picks a point in Select mode, and the gizmo stands at the selected points.

**Snapping and the pills (`pane.rs`, `menus.rs`)** - the pills over the viewport, left to right:
- the mode control Select | Place | Paint | Zones (`mode_index`)
- Perspective (the views, Orthographic, Camera Bookmarks, Maximize Viewport, which is the Level tab's
  `level.maximize_viewport` row)
- View Mode
- Show
- the tool pills Q / W / E / R
- World / Local
- Surface Snap, with its options behind a chevron
- the three step menus:
  - move: 0.01 0.05 0.1 0.25 0.5 1 2 5 10 m
  - rotate: 5 10 15 22.5 30 45 90°, default 15
  - scale: x0.05 0.1 0.25 0.5 1 2, default x0.25, off

Each step pill is a toggle with its value ("0.1 m", "15°", "x0.25"). Surface Snap is on by default. Align to Normal and
Vertex Snap sit under its chevron and are off. Vertex snap snaps to the nearest corner of the triangle that was hit
(`gz_scene::pick::box_triangles`).

**View Mode** has four modes:
- Studio (the editor's own light)
- Unlit
- Game Lit (the weather, the hour, exposure, the indoor hemisphere, rebake roofs, reload, in the pill's submenus)
- Wireframe

**Show** holds every switch in one place:
- Grid
- Safe Frame
- Stats
- Fog
- Helpers
- Spawn Names
- Light Names
- Glow Crosses
- AI Map

**The Direct3D switches are gone**: Fill and Shade modes, Linear Filtering, Textures, Edged Faces, Environment, and the
fixed-function lighting with its slots. In `gz-render`, the dead code behind them went too:
- `ViewFlags::SCENE_LIGHTS`
- `LightKind::Directional`
- `editor_light` and the flat / textures / linear / edged branches of `mesh.wgsl`
- `ViewStyle` keeps only `fog`

**Preferences > Viewport (`prefs.rs`, new):**
- Rendering: Render Scale, Real Time
- Graphics: Graphics Adapter [Use the Next Adapter], GPU Preference (Auto / High Performance / Power Saving, applied
  through `Gpu::set_hint`)
- Sound: Mute Sounds

**Keys (`actions.rs`, new; `keys.rs` is gone)** - every viewport key is a rebindable shell action, group "Viewport",
Level tab only, and all of them are off while the camera flies. There are no macros and no target keys.

| Keys | Action |
|---|---|
| 1-4 | the modes |
| Esc | leave the mode (off in Select, so the key passes on) |
| Q W E R | the tools |
| \` | World / Local |
| F | frame the selection |
| Home | frame all |
| End | drop to the floor (`scene.align`) |
| H | hide the selection |
| Ctrl+H | show all |
| Ctrl+G | group |
| Alt+2 / 3 / 4 / 5 | Wireframe / Unlit / Studio / Game Lit |
| Ctrl+1-9 | go to a camera bookmark |
| Ctrl+Shift+1-9 | set a camera bookmark |

The views, every Show switch and every snap are actions too, without keys. These keys are not duplicated because
others already own them:
- Ctrl+F, Ctrl+D and Delete are the shell's `edit.*` actions
- F5 and F9 are the Level tab's

**Modes (`scene.mode`)**
- **Place mode** places the armed thing at each click until Esc. It honours the scene's random placement.
- **Paint** draws the brush ring on the ground:
  - AI Map: a drag adds nodes, Ctrl + click selects
  - Foliage: a drag paints and Shift + drag erases, sent on release as one `scene.details` call
  - Decals: a click lays one
- **Zones** keeps the mesh clicks and the box of the sector tools (`objtools.rs`).

**Dropping into the viewport (`place.rs`, new)**
- `Placeable {kind, reference}` is the payload of a Place panel row: `Payload::Row { surface: "place", key }`, with
  the key `<kind>[:<reference>]`.
- Content Browser entries (`Payload::Assets`) map onto it by kind and extension.
- A shape's reference is `sphere`, `box` or its use: `shape:ai_map_bounds` places a box with `usage=ai_map_bounds`,
  as the coordinator asked. The Place panel's AI Map Bounds row now arms and drags like every other row
  (`gz-ui-level/src/place.rs`, that one line).
- While a payload is carried, a ghost stands where the ray meets the ground: the box or the mesh, snapped to the
  grid, aligned to the surface when Align is on. Letting go calls `scene.place`, then `scene.select`. A drop is one
  undo step.

**Nothing blocks the UI thread (`gz-render`'s `renderer.rs`, `frame.rs`)**
- The render thread no longer waits for the GPU.
- It submits and publishes the frame at once.
- It keeps at most one frame in flight, known through `queue.on_submitted_work_done` and `device.poll(Poll)`.
- It waits on its condition variable with a timeout only while a frame or a read-back is in flight.
- The labels' depth tests are read back through two buffers mapped with `map_async`, a frame or so late, by
  (text, world position). The old `label_visibility` mapped and waited on every frame.
- Pictures (`render_offscreen`, `read_picture`) still wait, on their own requests.

**Commands (`cmd.rs`, `gesture.rs`, rewritten; typed parameters with a doc on every field)**
- `viewport.camera action=get|set|focus|frame|bookmark`
- `viewport.view`: mode, tool, space, show / hide, the snaps and steps, Game Lit, the brush, arm, real_time,
  mute_sounds, gpu. Everything is validated first and the settings are saved once.
- `viewport.play what=particles|animation`
- `viewport.gesture kind=click|double_click|drag|hover|drop`. It answers:
  - picked, placed, moved (rotations in degrees, a way's points)
  - camera_moved
  - `gizmo` with the pivot and every handle's point
- `shot.viewport`
- `perf.frames`

The old names are gone: `camera_get` / `camera_set`, `viewport_controls`, `viewport_navigation`, `view_mode`,
`game_lighting`, `viewport_gesture`, `screenshot_viewport`, `frame_pacing`, `focus_object`, `spawn_animation` and
`group_thumbnail`.

**Settings** - the view mode, Show, the snaps and their steps, the space, the render scale and the GPU preference
are `[preferences] viewport_*` keys of `grizzly_editor.ltx` (`SETTINGS`, 19 `PrefDef`s). Camera bookmarks are kept
per scene beside its camera in `<project>\temp\viewport.ltx` (`session.rs`).

## 2. Decisions

- **Studio is a fourth view mode.** It is the model preview's word for the light that was Lit. Unlit means without
  light, as in UE.
- **Show says Safe Frame**, not Safe Rect, matching the rest of the editor's wording.
- **Planes are hit before the axes.** With the axes first, the QA found the XZ square shadowed by the Y arrow.
- **The rotate readout shows the angle the way Details does.** Details shows X-Ray's left-handed Euler angles: a
  right-handed turn of +a stores -a, so the readout prints that number.
- **Alt copies before the move starts**, so the copy is what moves and the original stays put, as in UE. Copy and
  move are one undo step.
- **The GPU choice is a cycling button** (Use the Next Adapter) plus a GPU Preference setting. wgpu picks adapters by
  power preference, and a list of named adapters would go stale on a laptop that switches.
- **`viewport.play` is an extra command.** It replaces the particle Play / Stop and the spawn animation row that
  `focus_object` / `spawn_animation` used to carry.
- **`group_thumbnail` is gone without a replacement**, as L2b1 dropped Make Thumbnail.
- **Place mode stays up until Esc.** Random placement (Randomize Added Objects) is kept as a scene setting the click
  honours.
- **The foliage stroke is sent once, on release.** That makes a stroke one undo step and keeps a 625k-node scene
  responsive while painting.
- **The drop phase drags real Place panel payloads.** It drops `object:`, `spawn:space_restrictor` and
  `shape:ai_map_bounds` rows, plus a Content Browser asset entry, as the coordinator asked.
- **Default steps changed:** `gz-app/src/settings.rs` defaults are now `snap_angle_step` 15 and `snap_scale_step`
  0.25, matching the brief.

## 3. Contract changes

**`docs/UI_DESIGN.md`**
- The Viewport section is rewritten: Viewport, The viewport's mouse, The gizmo, Dropping into the viewport, and MCP.
- The command list near line 416 and the `frame_pacing` / `screenshot_viewport` mentions are renamed.
- L2b1's Place line about the row payload now gives the real key format, `<kind>:<reference>`.

**`ui.contract`**: `VIEWPORT_MENUS` = perspective, view_mode, show, surface, grid, angle, scale
(`gz-app/src/contract.rs`).

**Commands**: added and removed as listed in section 1.

**Call sites fixed in other files**
- `gz-ui-level`:
  - `motion_ui.rs`: `viewport.play what=animation`
  - `outliner.rs`: `viewport.camera action=focus`
  - `place.rs`: the AI Map Bounds row
- `gz-ui-world/src/world.rs` and its `lib.rs` doc: `viewport.camera`
- Comments in `gz-app/src/window.rs`, `gz-editor/src/frames.rs` and `gz-task/src/wake.rs`
- `gz-app/src/settings.rs`: the two defaults
- QA scripts:
  - `bench_build.py`
  - `measure_compile.py`
  - `test_build_job.py`
  - `test_properties.py`
  - `test_stress.py`
  - `test_ui_tour.py`
- `docs/MOD_QUICKSTART.md`: the new names

**QA**: `tools/qa/test_scene_tools.py` has new phases Q (drop), H (planes, rings, space, Alt) and B (stalls).
- M now marquees a light along with the objects.
- The open checks `perf.stalls`.
- Phases T / U / N are L2b1's and untouched.

## 4. Requests

- **Architect / gz-editor + gz-app**: `write_atomic` uses a fixed temporary name, so two `save_soon`s of
  `grizzly_editor.ltx` racing each other fail with "could not be written". I save once per command to stay clear of
  it, but any two surfaces saving close together still meet it.
- **gz-app `window.rs`**: after `app.window resize` while the window is minimized, wgpu logs a Validation Error
  ("set_scissor_rect 2400x1350 not contained"). The capture texture keeps its old size.
- **K (bridge)**: `LOOKS` in `crates/gz-mcp-bridge/src/main.rs` keeps `screenshot_viewport` as an old-name fallback.
  The pre-release rule says no fallbacks.
- **Architect**: `CLAUDE.md` still names `screenshot_viewport`, `frame_pacing`, `viewport_gesture` and the other
  old names.
- **L2b1**: `view_cmd.rs` `level.view place=shape usage=ai_map_bounds` and `What::AiMapBounds` (Place Bounds Volume)
  still place at once. They could arm `shape:ai_map_bounds` through `gz_ui_viewport::place::arm`, like the row does.
- **L1a / gz-scene**: the light's "Use In D3D" property is meaningless now that there is no fixed-function lighting.
- **Kit (gz-ui)**: `Icon::SnapScale` (`icons/geometry.rs`) is still drawn as a ruler. The audit (C) asks for a
  glyph that reads as scale.
- **Stream S (gz-ui-previews)**: `cg.sh test -p grizzly` fails `every_card_is_worth_reading` on `preview.model:
  the parameter 'lighting' has no doc comment`. That is not mine, and every viewport card passes.

## 5. Proof

**Unit tests and lints**
- `bash tools/rust/cg.sh test -p gz-render -p gz-ui-viewport`: 37 + 58 passed. The tests include:
  - the gizmo: every move handle is hit where it is drawn, rings and the outer ring, the scale centre, an axis drag
    alone, a plane drag in its plane, a ring drag turning by the swept angle and snapping, a world-axis turn moving
    the place too
  - snapping steps
  - the drop zone, as an egui harness: the viewport takes a Place row and nothing else
  - the ghost's surface point on the grid
  - the payload keys and Content Browser entries
  - the `scene.place` arguments
  - the mode control
  - action ids and keys are unique
  - the Preferences rows
  - gesture feeding (click, even drag steps, a drop letting go with no button down)
  - the naga validation of the edited shaders
- `cg.sh clippy -p gz-render -p gz-ui-viewport --all-targets`: clean. `gz-ui-level` has no warning in my lines.
- `cg.sh fmt -p gz-render -p gz-ui-viewport`: clean.

**QA** (all on a copy, with `GRIZZLY_QA_PORT=28971`)
- `cg.sh build -p grizzly -p gz-mcp-bridge`, then `python tools/rust/limited.py 900 python tools/qa/run_all.py
  test_scene_tools`: PASS in 22.3 s, every phase FOKXRALMGQHBPVDWTUNS. This was rerun after the renderer cleanup.
  - Q: an object row, an asset entry, `spawn:space_restrictor` and `shape:ai_map_bounds` dropped on the ground under
    the pointer, selected, each one undo step. A bad kind is refused.
  - H: the XZ square moves in its plane. The Y ring turns 90° on 15° steps. Local X moves 45° off the world's. Alt
    copies.
  - M: a marquee takes the light and both boxes, and not the far one.
  - B: a marquee selects 3000+ objects from 700 m up, then a gizmo drag and two camera flights. `perf.stalls` is
    empty after each, and after opening the Escape scene.
- `test_previews` PASS in 11.9 s, after the renderer cleanup.
- `test_properties`, `test_ui_tour`, `test_stress` and `test_ui_shell`: PASS.
- No editor of mine is left running.

**Pictures** in `Intermediate/plan/L2a/`, from a copy of Test, Escape scene:

| File | What it shows |
|---|---|
| `mode_<select\|place\|details\|zones>_<1600x900\|1280x720>.png` | the pill row and the mode control in each mode at both sizes; in Place the "Placing" hint, in Paint the brush |
| `gizmo_move.png`, `gizmo_rotate.png`, `gizmo_scale.png` | the three gizmos |
| `gizmo_<tool>_hover_x.png` | the X handle lit under the pointer |
| `gizmo_move_local.png` | Local space on an object turned 35° |
| `gizmo_rotate_arc.png` | a Y ring turn under way: the arc and its angle |
| `popup_<perspective\|view_mode\|show\|surface\|grid\|angle\|scale>.png` | each pill's menu; the step menus show their units |
| `popup_view_mode_game.png` | Game Lit's submenus |
| `view_game_lit.png` (22:00), `view_wireframe.png`, `view_unlit.png` | the view modes |
| `drop_ghost.png` | a cabinet carried over the ground, its ghost snapped |
| `drop_placed.png` | the same cabinet after the drop, selected |
| `paint_brush_ring.png` | the Paint mode's ring during a stroke |
| `prefs_viewport.png` | Preferences > Viewport: Render Scale, Real Time, Graphics Adapter, GPU Preference, Mute Sounds |

Against the audit's pictures, the pill row reads like UE's viewport toolbar:
- one segmented mode control
- three menus
- the tool and snap toggles carrying their values

The D3D switches, the eight light slots and the tool tiles are gone.

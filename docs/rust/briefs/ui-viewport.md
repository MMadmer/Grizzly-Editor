# Brief: the viewport (`gz-ui-viewport`)

The 3D view of the Level tab: the scene drawn by the renderer, the camera, picking, the
transform tools, the overlays. Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first;
then `docs/UI_DESIGN.md` sections 1, 2, 4 and, in section 6, "Viewport" - **the contract**;
`docs/RUST_MIGRATION.md` sections 5.8 and 12; the rustdoc at the top of `gz-render` (the whole
API), `gz-scene` (snapshots, `render_items`, `SceneCmd`, picking, snapping), `gz-ui`, `gz-app`
(`Document`, `DocCx`, `ShellBuilder`, `on_ui`, `EditorSettings`), `gz-game`, `gz-project`,
`gz-editor`.

You own: `crates/gz-ui-viewport/**` (new; `src/lib.rs` before `Cargo.toml`) and one `register`
line in `crates/grizzly/src/main.rs` + the dependency in its `Cargo.toml`. Another agent writes
the Level tab's panels (`gz-ui-level`) at the same time and embeds your pane through the
interface below: keep it exactly.

## The interface the Level tab uses (fixed)

```rust
pub struct ViewportPane { /* yours */ }
impl ViewportPane {
    pub fn new(editor: &Arc<Editor>, gpu: &egui_wgpu::RenderState) -> Self;
    /// Draws the viewport into the space it is given: the picture, the pills, the gizmo, input.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut gz_app::DocCx);
    /// A tick while the Level tab is not drawn (no scene pass runs then).
    pub fn inactive(&mut self, cx: &mut gz_app::DocCx);
    /// The scene's key bindings, routed here by the shell only while Level owns the keys.
    pub fn shortcut(&mut self, cx: &mut gz_app::DocCx, chord: &gz_app::Chord) -> bool;
    /// What `state` / `ui_list` answer about it.
    pub fn mcp_state(&self) -> serde_json::Value;
}
pub fn register(reg: &mut gz_editor::Registry);   // camera_*, view_mode, ... (below)
```

## The C++ that tells what it does

`Source/Editors/XrECore/Editor/UIRenderForm.cpp`, `UI_ViewportNavigation.cpp`, `ui_camera.cpp`,
`EStats.cpp`, `UI_ToolsCustom.cpp` (the transform tools' maths: axis locks, snaps, the pivot),
`Source/Editors/LevelEditor/UI/UIViewportToolbar.cpp` (the pills), `Editor/Utils/Gimzo/*`
(the gizmo), each tool's `*Controls*.cpp` under `Editor/Tools/*` (what a click, a drag and a
key do in each class's tool - select, add, move, rotate, scale), `Editor/Scene/SceneRender.cpp`
and each class's `Render` in `Editor/Entry/*` (what is drawn for each class, selected or not),
`EGameLighting.cpp`, `GameLightingScene.cpp`, `Editor/Recovery` (only `SetPreviewScope`),
`EditorProject::CapturePreview`. Handlers (frozen wire): `camera_get`, `camera_set`,
`focus_object`, `view_mode`, `viewport_controls`, `viewport_navigation`, `game_lighting`,
`frame_pacing`, `screenshot_viewport`, `asset_preview` if it is the viewport's
(`rg -n '"camera_get"' Source/Editors`). Schemas: `crates/gz-mcp/tools.json`.

## Architecture (decided)

- **Scene snapshot -> `DrawList` is a pure function** in this crate, run on the CPU pool
  whenever the scene's revision, the camera or the view options change - never per frame when
  nothing changed (the editor draws on demand). It turns `SceneState::render_items` into
  instances (library objects and visuals by handle), lights, and the helper shapes each class
  draws (light and sound ranges, shapes, way points and links, spawn marks, portals, sector
  bounds, the selection marks), in the colours and forms of the C++. Visibility comes from the
  scene's own BVH (`build_packet_with`). `gz_render::Assets` gets a loader that reads the
  project and the editor's library first, then the linked game.
- **The renderer runs beside the UI**: `Renderer::submit` the packet, show `target()` through
  the window's egui renderer (register the texture view once per target serial). The UI never
  waits for a frame of the scene. The background is the theme's canvas colour (the weather's
  fog colour under game lighting).
- **Picking and tools read snapshots and emit commands.** A click builds a ray
  (`gz_render::pick_ray`) and asks `gz-scene`'s picking on the CPU pool; the result comes back
  through `on_ui`. Select (click, marquee, add / toggle with the C++ modifiers), Place (the
  active tool's object at the hit), Move / Rotate / Scale with the gizmo, axis locks and the
  grid / angle / scale snaps (steps are the preferences), Object Snap, drop to ground. A drag
  sends `SceneCmd`s as merged-undo edits and seals the gesture on release, so one drag is one
  undo step. Nothing here changes the scene except through `SceneCmd`.
- **Overlays are egui**, on top of the picture: pills left ([View Options v], [Perspective v]
  with views, Frame All, Frame Selection, [Show v] Stats, Grid, Safe Rect) and right (Segmented
  Select / Place / Move / Rotate / Scale, Object Snap, the three snaps with their values); a
  64 pt clickable axis gizmo top right (hidden under 480 pt); stats as a Mono pill, off by
  default; labels the renderer reports visible. Nothing is drawn into the render target for
  them. Every menu item with a command shows the keys bound now.
- **Camera**: fly and orbit navigation as the C++ does it (`UI_ViewportNavigation`), Frame All /
  Frame Selection (`gz_render::frame_fit`), views, perspective / ortho; the camera is view
  state of this pane, saved per scene in the session.
- **Game Lighting** (View Options): Enabled, Weather (Default, then every weather of the linked
  game), Hour, Exposure, Indoor Sky Light, Rebake Roof Map - feeding `gz-render`'s game
  lighting; the weather files are read through `gz-game` off the UI thread.
- **Pictures**: `screenshot_viewport` forces a scene pass even while Level is hidden and crops
  to the render size (`Renderer::capture`); `preview.png` is a pass of its own from the
  viewport's camera with no grid, selection, gizmo or helpers (`render_offscreen` with the
  preview view), taken when the scene's command for it asks (expose `capture_preview(editor)
  -> RGBA` for `gz-project` / the Level tab to call before a scene is cleared), never inside a
  UI pass.
- **The window's device**: if `crates/gz-app/src/window.rs` does not yet request
  `TEXTURE_COMPRESSION_BC` and install a device error handler (another package may have done
  it - read the file first), do both there; that file is the one thing outside your crate you
  may touch, with a small edit.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes, never loads and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-viewport --tests`, zero warnings. The Level
  tab that hosts the pane is another agent's: to LOOK at your work before it lands, register a
  temporary document kind `viewport_test` in your own `install` that hosts only the pane, open
  it with `ui_tab action=open kind=viewport_test`, open a COPY of a scene from a temp copy of a
  project (never a real one), and picture it: the scene lit and unlit, a selection, the gizmo,
  the pills, `screenshot_viewport`. Remove the temporary kind before you finish.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand. If `grizzly` does not build because of someone else's crate,
  wait and retry a few times, then say so. Do not run the QA suites.

## Report

Under 400 words: what is built, what each class draws and what it does not yet, what the
pictures showed, deviations from the C++ wire and from UI_DESIGN. Then add a row for
`gz-ui-viewport` to `docs/rust/STATE.md`.

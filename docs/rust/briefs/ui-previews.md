# Brief: previews and libraries (`gz-ui-previews`)

The tabs that show one asset: the model preview, the texture preview, and the three libraries
(Light Animations, Texture Library, Sound Library). Read `docs/rust/CONVENTIONS.md` and
`docs/rust/STATE.md` first; then `docs/UI_DESIGN.md` sections 1, 1a, 2, 4 and, in section 6,
"Model Preview and Texture Preview" and "Libraries" - **the contract** -; `CLAUDE.md` section
"Editor UI (UI 2)" (the libraries paragraph); the rustdoc at the top of `gz-ui`, `gz-app`
(`ShellBuilder`, `Document`, `DocCx` - it carries the GPU), `gz-render` (the whole API:
`Renderer`, `DrawList`, `View`, `Assets`, `render_offscreen`, camera maths), `gz-fmt-ogf`,
`gz-fmt-object`, `gz-fmt-tex`, `gz-fmt-xr` (light animations), `gz-fmt-sound`, `gz-game`,
`gz-project`, `gz-editor`.

You own: `crates/gz-ui-previews/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, the dependency in
its `Cargo.toml`, and - only for the two device notes below - `crates/gz-app/src/window.rs`.

## The C++ that is the spec

- `Source/Editors/LevelEditor/UI/Preview/*` (2.4k lines): the model and texture previews.
- `Source/Editors/XrECore/Editor/UIEditLightAnim.cpp`, `UIImageEditorForm.cpp`,
  `UISoundEditorForm.cpp`, `ImageManager.cpp` (Check New Textures / Synchronize - the list
  logic), `SoundManager.cpp`: the libraries.
- Handlers (frozen wire): `preview_model`, `texture_preview`, `asset_preview`,
  `asset_preview_model`, `library` in
  `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpPreview.cpp` and where they delegate. Schemas:
  `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_previews.py`.

## Architecture (decided)

- **Each preview is a `Document`** registered with `ShellBuilder::document`: `model` and
  `texture` are singletons that switch to a new target (asking first when dirty); the three
  libraries are singletons. Openers activate in the same call; a command whose answer is where
  things landed runs, awaits `editor.frames.after(n)`, then reads again (`pending` never
  reaches the caller).
- **The model preview has its own small renderer view**: a `gz_render::Renderer` target sized
  to its canvas, fed a `DrawList` built by hand (the model, a floor grid, bones when asked);
  Frame, Reset View | Wireframe, Bones | Studio / Unlit | Details (320 pt). The picture is
  shown through the window's egui renderer (register the target's texture view once per
  resize - `DocCx` gives the `RenderState`). The view is drawn opaque. A visual only the
  linked game holds is drawn with the game's textures (`textures[].from` in the answer):
  give `gz_render::Assets` a loader that reads the project first, then `gz-game`'s files.
  Orbit, pan and zoom at the cursor are pure camera maths (`gz-render` has them).
- **The texture preview** decodes on the pools (`gz-fmt-tex`), uploads once, and draws with
  egui: Fit, 1:1, Zoom | RGBA / R / G / B / A | Mip | Details; pan and zoom at the cursor, a
  checker clipped to the view; Details name where the texture lives (`origin`), its container
  and format, and whether it is shown decoded. Channel views are a small WGSL pass or CPU
  swizzle done once per change, never per frame.
- **Libraries** are list | preview / timeline | grid. Light Animations reads and writes
  `gamedata\lanims.xr` (`gz-fmt-xr`), with a timeline drawn by hand and exact colour at a
  frame; the Texture Library reads the `.tga` sources of `rawdata\textures` with their `.thm`
  (`gz-fmt-tex`), Check New Textures turns a library tab without unsaved changes into New
  Textures (from the import folder) and the finished import brings the library back; the
  Sound Library reads `rawdata\sounds` (`gz-fmt-sound`: params in the OGG comment, a waveform
  from the peak envelope, playback through `cpal` on its own audio thread - decode off the UI
  thread, never block a frame). An empty library says what it reads. Lists come from a walk on
  the IO pool, cached as a `Query`; edits live in a `Store` per library with undo, saved
  explicitly; an unsaved prompt (the kit's) replaces the old MessageBox.
- **Two notes for the window's device** (you may edit `crates/gz-app/src/window.rs` for exactly
  these): request `TEXTURE_COMPRESSION_BC` when the adapter has it (wrap the
  `device_descriptor` of the `WgpuSetupCreateNew` the window builds), so BC textures reach the
  GPU as they are; and install a device error handler that logs through the editor's log
  instead of wgpu's default panic.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes, never decodes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-previews -p grizzly`, zero warnings. Then
  build, run once and LOOK (start `Intermediate\cargo\debug\Grizzly.exe -nodlg -mcp_port 28917`
  with `Start-Process -WindowStyle Minimized`; `preview_model` on a model of the linked game,
  `texture_preview`, `library kind=... action=open`; `screenshot_editor` each). Back up
  `_projects.ini` and the editor's settings file before and restore them byte for byte after;
  write nothing into `rawdata` or `gamedata` during the run; close with `window_control
  action=close`. If `grizzly` does not build because of someone else's crate, wait and retry a
  few times, then say so. Do not run the QA suites.

- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. It takes a machine-wide lock (other authors run the
  editor too), backs up and restores `_projects.ini` and the settings file byte for byte,
  starts the editor minimised, hands your driver the port as its first argument and closes
  the editor afterwards. Never start `Grizzly.exe` by hand and never back those files up
  yourself.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-previews` to `docs/rust/STATE.md`.

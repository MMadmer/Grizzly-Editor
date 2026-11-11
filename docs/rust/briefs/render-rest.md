# Brief: what the viewport does not draw yet

The renderer (`gz-render`) and the viewport's adapter (`gz-ui-viewport/src/draw.rs`) draw
scene objects, spawn visuals, helpers and lights. The C++ editor's viewport also shows
**particle systems playing, glows, the AI map, detail objects, wallmarks and a cube-map sky** -
none of which is drawn here (STATE.md, "From the renderer": "Not drawn yet"). This package
draws them. Editing the AI map, the details and the wallmarks is the NEXT package: here they
are drawn from the data the scene already holds (`SceneState::bulk`).

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; `docs/RUST_MIGRATION.md` 5.8
(the renderer's rules: frame packets, nothing locked to be drawn, decode on the pools) and
section 12 D2; the rustdoc at the top of `gz-render` (`DrawList`, `build_packet`, materials,
helpers, the render thread, assets), `gz-ui-viewport` (`draw::build`, `draw::visibility`, the
view flags, the loader), `gz-scene` (`BulkData`: AI map tiles, `DetailsPart`, wallmarks; the
particle and glow objects), `gz-fmt-xr` (`particles.xr`), `gz-fmt-level` (details, wallmarks).

You own: `crates/gz-render/**`, `crates/gz-ui-viewport/src/draw*.rs`, `lighting.rs` and
`loader.rs`, a new crate `crates/gz-particles` (`src/lib.rs` before `Cargo.toml`), and the
Particles tool's Play / Stop wiring in `crates/gz-ui-level` (only that function). Two other
agents work in the tree at the same time: one in `gz-scene`, `gz-ui-level/src/tools*.rs` and
the viewport's gesture code (`pane.rs`, `keys.rs`, `gizmo.rs`) - leave those files alone. If
`cargo check` fails in a crate that is not yours, wait and retry.

## What to build

1. **Particles** (`gz-particles`, pure: no window, no GPU). A port of the engine's particle
   system as the editor plays it: `Source/XrParticles/` (`particle_actions*.cpp`: every action,
   `particle_effect.cpp`, `particle_manager.cpp`, `noise.cpp`) and the definitions and playback
   of `Source/XrRender/Private/ParticleEffectDef.cpp`, `ParticleEffect.cpp`,
   `ParticleGroup.cpp`, `PSLibrary.cpp` (effects, groups with their child effects and timing,
   frame animation, alignment modes, velocity scale, the collision action against the scene's
   pick tree when the effect asks for it). Input: an effect or group of `particles.xr` (through
   `gz-fmt-xr`), a transform, time steps. Output per step: the live particles as plain data
   (position, size, rotation, colour, frame) for the renderer to draw as sprites or the
   effect's own mesh form. Deterministic given a seed (the C++ uses its own random: port it so
   a test can pin a step). Simulation runs on the CPU pool, one task per system, never on the
   UI or the render thread; a system out of view or not playing costs nothing.
   - The scene's particle objects play in the viewport as in the C++
     (`Source/Editors/LevelEditor/Editor/Entry/Particles/EParticlesObject.cpp`,
     `Editor/Tools/Particles/EScenePSControls.cpp`, `UI/Tools/UIParticlesTool.cpp`): the
     tool's Play / Stop, the object's own play state; a new command `particles_tool
     action=play|stop|status [names]` with its `grizzly_particles_tool` entry in
     `crates/gz-mcp/tools.json`. While any system plays the viewport asks for frames; when none
     does, it goes back to drawing on demand.
   - The renderer draws them with the particle material that already exists (billboards and
     the aligned forms, blend modes from the effect's shader name, the atlas frames).
2. **Glows** (`Editor/Entry/Glow/glow.cpp`): the glow's sprite with its texture and shader at
   its radius, and its helper when selected, as the C++ draws them.
3. **The AI map** (`Editor/Tools/AIMap/ESceneAIMapTools_Render.cpp`): the nodes of the tiles in
   view as the C++ draws them (the node quads on their planes, the link marks, selected nodes
   in the selection colour), from `bulk` - tiles are culled, a tile's mesh is built once per
   tile revision on the CPU pool and instanced; a level of 500k nodes must not cost more per
   frame than the tiles on screen. Shown under the AI Map tool's eye like every class.
4. **Detail objects** (`Editor/Tools/Details/ESceneDOTools.cpp` render part,
   `Source/Editors/XrECore/Engine/EDetailModel.cpp`, the slot decompression of
   `Source/XrRender/Private/DetailManager_Decompress.cpp` and `DetailManager_CACHE.cpp`): the
   detail models of the slots near the camera, placed as the game places them (the same
   positions, scales and rotations from the slot's data and the same random), drawn instanced
   with the detail material; the slot grid and the selection boxes as the C++ draws them. Slots
   decompress on the CPU pool, cached per slot revision.
5. **Wallmarks** (`Editor/Tools/Wallmark/ESceneWallmarkTools.cpp`: the render part): the static
   wallmarks of the scene with their textures and shaders, selected ones marked.
6. **The sky**: a cube-map sky texture is sampled as a cube (today it is sampled as 2D), with
   the weather's sky colour and rotation as the Game Lighting sets them
   (`Source/XrRender/Private/dxEnvironmentRender.cpp` for what is drawn).
7. STATE.md also lists "level geometry from `gcontainer` OGFs". Find whether the C++ EDITOR
   ever draws one (a compiled level's visual in the model preview, a spawn's visual): if a path
   of the editor reaches it, draw it; if none does, say so with the proof and take the line out
   of STATE.md.

## Rules

- The renderer still knows no scene: everything arrives in the `DrawList` as plain data.
  Extraction, culling and mesh building run on the CPU pool; the UI thread and the render
  thread never wait and never read a file. No `unsafe`, no global state.
- What the C++ viewport shows is the measure (placement, size, colour, blend), not the game's
  look: section 12 D2 stays as it is.
- Tests are written, not run: the particle actions each with a pinned step worked out from the
  C++ arithmetic; the detail slot decompression against a slot built by hand.
- Compile: `bash tools/rust/cg.sh check -p gz-particles -p gz-render -p gz-ui-viewport -p
  grizzly`, zero warnings; clippy clean for your crates. Then build and LOOK, on a temp COPY of
  `D:\XFinedProjects\Test` (the Escape scene, linked to the game - only read): a particle
  object playing (two pictures a moment apart differ), a glow, the AI map of the scene with the
  AI Map tool shown, details, a wallmark if the scene has one (place nothing by hand that the
  editor cannot place yet - say what you could not look at), the sky under Game Lighting.
  `frame_pacing` while particles play: no frame over budget because of the simulation.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 400 words: each of the seven items as done / partly / not done with the reason, what the
pictures showed, the costs you measured. Then update "From the renderer" in
`docs/rust/STATE.md` (what is drawn now, what is still not) and add the `gz-particles` row.

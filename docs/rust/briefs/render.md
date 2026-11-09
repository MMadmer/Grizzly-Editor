# Brief: the renderer (`gz-render`)

The editor's own renderer: what the viewport, the previews and the thumbnails are drawn with.
Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/RUST_MIGRATION.md`
sections 5.8 and 12 (D2), `docs/UI_DESIGN.md` section 6 "Viewport" and "Model Preview / Texture
Preview"; the rustdoc at the top of `gz-scene` (the snapshot it draws), `gz-fmt-object`,
`gz-fmt-ogf`, `gz-fmt-tex`, `gz-fmt-level`, `gz-game`; and `crates/gz-app/src/window.rs` (where
the device comes from).

You own: `crates/gz-render/**` (new; `src/lib.rs` before `Cargo.toml`).

## What it replaces, and what it is not

The C++ editor draws through a D3D11 device with the engine's R1 forward blenders and its Lua
shader scripts. **None of that is ported.** This is a new forward renderer on `wgpu` (the
workspace's version; its API differs from older ones - check signatures in
`C:\Users\admin\.cargo\registry\src\index.crates.io-*\wgpu-30*` and in how `egui-wgpu` uses it).
The viewport shows what an author needs to place and judge things - geometry, textures,
transparency, placement, selection; the look of the game is the game's. To learn what the old
viewport SHOWS (not how), read `Source/Editors/XrECore/Editor/render.cpp`, `D3DUtils.cpp` (the
helper shapes: grid, boxes, spheres, cones, arrows, the selection marks),
`Source/Editors/LevelEditor/Editor/Scene/SceneRender.cpp`, each class's `Render` in
`Entry/*`, and `EGameLighting.cpp` + `GameLightingScene.cpp` (the game-lighting mode).

## Architecture (decided - implement it this way)

**1. No window, no egui.** The crate takes a `wgpu::Device` and `wgpu::Queue` (shared with the
window; both are `Send + Sync`) and renders into textures it owns. The UI shows a texture; a
thumbnail job reads one back. Layers 0-3 do not depend on this crate.

**2. A frame is a packet.** `extract(scene: &SceneSnapshot, view: &View, assets: &Assets) ->
FramePacket` is a pure function that runs on the CPU pool: cull against the frustum through
the scene's BVH, group what is visible by (mesh, material) into instanced draws, sort the
blended ones, collect helper shapes (lights, sound ranges, shapes, way paths, spawn marks, the
selection). The packet is immutable and self-contained (handles and instance data, no
references into the scene). The scene is never locked to be drawn.

**3. The render thread.** One thread owns the targets and the frame loop: it takes the newest
packet from a channel (older ones are dropped), records passes - in parallel where a pass is
large: one command encoder per chunk on the CPU pool, submitted in order - and submits. The UI
thread never waits for it: it shows the last finished frame. Each finished frame bumps a
counter the frame clock can wait on (`screenshot_viewport` is held for the frame that drew its
commands) and pings `Wake`.

**4. Resources load off the frame.** `Assets`: meshes (from `gz-fmt-object` library objects
and `gz-fmt-ogf` visuals) and textures (from the project and the linked game through `gz-game`'s
files), requested by handle. A missing one draws a placeholder (flat grey mesh, checker
texture) and is loaded by a job on the pools - decode, then `queue.write_texture` / buffer
upload - after which the handle resolves. BC1 / BC3 DDS data goes to the GPU as it is
(`TEXTURE_COMPRESSION_BC`, when the adapter has it; decode to RGBA otherwise): no CPU decode,
no re-encode. Caches are bounded (an LRU by bytes), keyed by content handle.

**5. Materials.** Engine shader names (a surface's shader in `.object`, a visual's shader in
OGF, `shaders.xr`) map through ONE table to a small set of editor materials in WGSL: opaque,
alpha-tested, blended, additive, self-lit, terrain with a detail texture, tree / LOD
(two-sided, alpha-tested, wind off), water, sky dome, particle, plus unlit colour, wireframe
and the selection outline. Lighting: a headlight along the view plus hemisphere ambient by
default (the "Studio" of the model preview); the game-lighting mode adds the scene's lights
and the weather's sun, ambient and fog, per pixel. The table is data, with a default for an
unknown name, so adding a mapping is one line.

**6. Passes.** Depth + opaque, alpha-tested, sky, blended (sorted back to front), helpers and
grid (lines with depth), selection outline (from an id / mask target, so it is exact for
alpha-tested geometry), overlays without depth (gizmos, way links). MSAA 4x when the adapter
allows. The target grows in 64 px buckets and the view is its top-left corner; whatever reads
it back crops to the render size.

**7. Skinned models** are skinned on the GPU (a bone-matrix buffer per instance); the pose
comes from `gz-fmt-ogf`'s key decoding or `gz-fmt-object`'s `animate`.

**8. Other clients of the same renderer.** `render_offscreen(packet, size) -> RGBA` for
thumbnails, the model preview, `preview.png` (the scene once more from the viewport's camera
with no grid, selection, gizmo or helpers - a flag of the view) and `screenshot_viewport`.
Thumbnails render at the size a tile asks for, on demand, cached by content.

## API the UI packages will use (name it clearly in `lib.rs`)

`Renderer::new(device, queue, exec)`, `View` (camera, size, view mode: lit / unlit / wireframe
/ game lighting, flags: grid, helpers, selection), `Renderer::submit(packet)`,
`Renderer::target() -> TargetHandle` (the texture view and its size, for the UI to show),
`render_offscreen`, `Assets` handles, camera maths (projection, picking ray from a pixel,
frame-selection fit) as pure functions.

## Rules

- Safe Rust only (`wgpu`'s API is safe); no `unsafe` and no raw handles.
- Nothing here blocks the UI thread; nothing allocates per draw in the recording loops
  (reuse buffers; instance data goes into one growing buffer per frame).
- Shaders live as `.wgsl` files in the crate, included with `include_str!`.
- Compile: `bash tools/rust/cg.sh check -p gz-render --tests`, zero warnings. Tests written
  (not run): extraction of a seeded scene (counts, grouping, sort order), camera maths, the
  material table, bucket sizing. Then render ONE offscreen frame of a hand-made scene (a
  textured box, an alpha-tested quad, the grid) in a small example binary under
  `crates/gz-render/examples/`, write it as a PNG and LOOK at it: a renderer nobody looked at
  is not done.

## Report

Under 400 words: the API, what draws and what does not yet (by class and by material), what
the offscreen picture showed, adapter features it relies on. Then add a row for `gz-render` to
`docs/rust/STATE.md`.

# Level Build 2 - the plan

The working document of the level build's rework: the architecture, who does what, each brief, the order of the
tests, what counts as done. It leaves the tree when the work is in: what stays is written into the contracts
(`docs/ARCHITECTURE.md`, `docs/COMMANDS.md`, `docs/STATE.md`, `CLAUDE.md`).

## 1. Why

The author pressed Build on Cordon (`l01_escape`, 1 M faces, 2575 static models) at the level's own quality,
waited ten minutes and stopped it. Measured on that level on this machine (i7-11800H 8C/16T, RTX 3070 Laptop
8 GB, 64 GB), release build, 2026-11-15:

| What | Seconds |
|---|---|
| The steps before the compiler (game files 4.2, AI map kept 0.2, game graph 1.4, spawn 1.5, geometry 0.7-4.4) | about 10 |
| The AI map generated over Cordon's whole ground (a new level's first build): 2.48 M nodes | 13.2 |
| The compiler, draft quality (no lightmaps), all 20 phases | 38.3 |
| ... of it: ray model 6.0, sectors 6.3, tessellation 5.8, optimizing 3.3, tangent basis 2.7, visuals 2.3, collision 2.2, static models' visuals 2.0, static models' light 1.8, subdividing 1.8, normals 1.2, vertex light 1.0 | |
| The compiler, standard quality: the first 8 phases (tessellation 15.6, collision 8.4) | 38 |
| ... then "Lighting the static models": 128 models in about 210 s, 1126 of 2447 references in the 350 s left | cut at 600 |
| ... the 12 phases after it (implicit textures, lightmaps, vertices, packing, merging, visuals, sectors) | never reached |
| A small indoor level (`l12u_control_monolith`), standard quality, the whole compile | 43.0 |
| ... of it: lightmaps 19.6, **packing the lightmaps 9.4**, merging 3.5, visuals 3.4, collision 1.5, tessellation 1.2, tangent basis 1.2 | |

So: the geometry work is tens of seconds; the light is hours; and once there are lightmaps, placing them
(a first-fit search that tests every position of a page texel by texel) and merging by page cost as much as
half of the light on a small level - on a large one, minutes. A sample point traces one shadow ray per light -
26, 91 or 196 sky directions, the sun's samples, the lamps in reach - through a scalar binary BVH, and a level has
10^8..10^9 sample points. The compiler is a line-by-line port of a 2003 program: correct, single-pass, CPU only.

The goals, in the author's order:

1. Every stage at least twice as fast; the light by as much as the hardware gives (GPU, SIMD, every core). A
   gain of a couple of seconds is not worth a change.
2. A rebuilt map is still the same map: several modules change one level by deltas, and none of them may break
   because somebody built it again (section 2).
3. What the build makes can be read by an agent as text (section 3.6).
4. Where it comes cheap: smaller levels, a better picture, a cheaper frame.
5. An architecture of levels: pieces of a level made by its author or by modules, streamed by distance
   (section 8) - built in this same run.

The game's own obligations bound all of it and nothing else does: saves stay compatible with Dead Air, Dead Air's
mods keep working, the game keeps reading the level files it reads today. How a level is built is the editor's
own business.

## 2. The rebuild contract (what other modules stand on)

A module changes a level by world-space boxes, names and a few ids (the game's `XMS_ARCHITECTURE.md`, sections 7
and 8): spawn operations by name and position, `overlay.xcform` (triangles added, cut boxes, a host sector id),
`overlay.aimap` (nodes on the level's grid), `overlay_visuals.ltx` (visuals added, cut boxes that detach the
base visuals inside them), graph links by coordinates; saves hold `level_vertex_id`s.

Therefore, for one scene and one set of build settings, a build writes **the same structure on any machine, with
any number of threads, lit on the GPU or on the CPU**:

- `level.ai` byte for byte: the nodes, their order, the grid (a hand-made map kept, a generated one made again
  only when its ground moved - as today);
- `level.graph`, `level.game`, `level.spawn`, the spawn layer, `editor.spawn`;
- the sectors and portals: ids, order, planes;
- the collision as a surface: `level.cform`'s triangles with their materials and sectors;
- the visuals: which faces make which visual, their order, bounds, vertices' positions, normals, base texture
  coordinates, indices, LOD data;
- and only the light may differ, within a tolerance: lightmap texels, where a visual's lightmap lies (page and
  lightmap coordinates), vertex colours, the static models' light fit, the implicitly lit textures, the detail
  light.

**No light value may decide structure.** Today one does: a deflector's size after it was lit decides its page,
and the page decides which subdivisions merge into a visual - so one changed lamp reshuffles the visuals of the
whole map. Section 3.3 ends that: visuals are made from geometry alone, before any light.

The CPU lighter is the reference: its output is the same bytes on every x86-64 machine and every run (no fused
multiply-add, no fast-math, one order of operations in every SIMD lane), and the records under
`tools/qa/golden/compile` hold it byte for byte.

## 3. The architecture

### 3.1 Principles

1. **Structure is geometry's, light is light's** (section 2). The compile has four parts in this order:
   *structure* (CPU), *light* (the lighter), *atlas* (packing what was lit), *output*.
2. **One baker.** Every light value of the build comes from one call: "light these sample points in this
   scene". Lightmap texels, vertices, the static models, the implicit textures and the detail slots all become
   sample points. Two lighters implement the call: the CPU one (the reference) and the GPU one.
3. **The GPU is a plain compute shader** over a BVH the editor builds: any DX12 or Vulkan card, no ray tracing
   hardware, no experimental feature of the graphics runtime, no `unsafe`. `auto` takes the GPU when a real
   adapter is there and answers a self-check like the CPU does; anything the GPU fails at is lit on the CPU and
   the build goes on - a build never fails because of a graphics card.
4. **The game's formats stay**: `level`, `level.geom`, `level.geomx`, `level.cform`, `level.ai`, the lightmap
   pages (1024 x 1024 DXT5 pairs), `level.details`. The game gains one thing, additively: pieces (section 8).
5. **Deterministic, measured, stoppable**: every phase gives the same result on any number of threads; every
   long loop checks the job's Stop per item or chunk; the tools of section 6 say what each stage costs.

### 3.2 The compile, in its new order

```text
STRUCTURE (CPU, the same bytes everywhere)
  read and load the build project
  optimize the geometry, correct the T-junctions, tessellate the long edges
  normals, collision (level.cform), tangent basis
  the ray model - once (today it is built twice)
  materials -> charts (the deflectors, their sizes from geometry and the texel density) -> subdividing
  -> MERGING INTO VISUALS (key: the material; limits: faces, size, volume, lightmap area) - moved here
LIGHT (the lighter: GPU or CPU)
  the static models and their references, the implicit textures, the charts (texels, then edge texels),
  the vertices (the sky light of the tessellated vertices included)
ATLAS (CPU, light data only)
  borders; a flat chart collapses, a smooth one shrinks by RESAMPLING what was lit (no second lighting);
  the charts packed page by page, the charts of one visual always on one page; the pages encoded in parallel
OUTPUT
  the visuals, the static models' visuals, the sectors, the level's files moved into place together
```

### 3.3 Visuals before light

`merge_geometry` runs before the light with the key `(material)` - it no longer reads the lightmap page - and
one more limit in `validate_merge`: the charts of the merged subdivision, each with its border, hold at most a
quarter of a page (`LMAP_SIZE^2 / 4` texels), so a visual's charts always fit one page. The packer then works
in two levels: the charts of one visual are packed into one tight rectangle (a small raster mask, rotations
allowed), and the rectangles are placed into pages by a skyline in a stable order (area, then the visual's
index). Every chart of a visual is in one page by construction. A rectangle larger than a page (the limit
above makes it a pathology) is split by page with a warning naming the visual - the one place light may still
touch structure, logged and counted, expected never.

What this gives besides the contract: an edit in one corner of a map no longer changes the visuals in another
(the cut boxes of other modules survive a rebuild of an edited map), merging is no longer blocked by pages
(fewer visuals, fewer draw calls), and the merge costs nothing after the light.

### 3.4 The baker (`gz-lc-light`'s `bake.rs`)

```rust
/// One point to light: what `light_point` takes. 32 bytes, plain data: it goes to a GPU buffer as it is.
#[repr(C)] pub struct Sample { pub p: [f32; 3], pub flags: u32, pub n: [f32; 3], pub skip: u32 }
/// What a batch is lit with.
pub struct LightJob<'a> { pub lights: &'a LightSet, pub falloff: Falloff }
pub trait Lighter: Send + Sync {
    fn name(&self) -> &str;                       // "the CPU", "NVIDIA GeForce RTX 3070 Laptop GPU"
    fn scene<'a>(&'a self, model: &'a RayModel, textures: &'a [BuildTexture]) -> Result<Box<dyn LitScene + 'a>>;
}
pub trait LitScene: Sync {
    /// `out[i]` becomes the light of `samples[i]` (`ColorC`: rgb, hemi, sun). Samples do not depend on each other.
    fn light(&self, job: &LightJob<'_>, samples: &[Sample], out: &mut [ColorC], ctl: &Ctl<'_>) -> Result<()>;
}
```

`skip` is the level face a point lies on (`u32::MAX` for none); `flags` are the `LP_*` bits; the ray model's
opacity rule (squared for the level, linear for the details) and its back-face culling are the scene's. The
formulas are `point.rs`'s `light_point` and `ray.rs`'s `trace`, unchanged: a lighter is another way to compute
them, never another light.

- `CpuLighter`: `light_point` per sample on the caller's rayon pool, in chunks of 256 samples cut by index,
  a `LightCache` that lives for one chunk. The reference.
- `GpuLighter` (`gz-lc-gpu`): section 4, A1.
- `AutoLighter`: the GPU when it is there and passes its self-check, the CPU otherwise; a call the GPU fails is
  lit on the CPU and the rest of the build stays on the CPU, with one sentence in the log saying why.
- `NullLighter`: a constant light. For profiling the structure and for tests; `light=off`.

`level.build`, `level.compile` and Play take `light=auto|gpu|cpu` (left out: `auto`; `gpu` refuses when there is
no adapter instead of falling back). The job's stage and the report say where the light was baked.

### 3.5 The CPU side

- The ray model gets a 4-wide BVH (SSE through `glam`'s `Vec4`, safe code, the baseline CPU of the editor):
  triangles stored with their edges in leaf blocks of four and tested four at a time, the face's flags beside
  them, children visited front to back, the first opaque hit ends the walk. Its answers are bit for bit the
  answers of today's `trace` (the same triangle test in the same order of operations per lane, box tests that
  never lose a hit).
- The structure phases are made parallel or given better algorithms where a phase costs two seconds or more on
  Cordon; each keeps its output byte for byte.

### 3.6 Readable results

- `build.report` beside `build.state` (Lua data, written by every build; a `build.*` file never ships): the
  level, when, the quality, the lighter, every step and every compiler phase with its seconds, the counts
  (faces in, visuals, sectors, portals, collision triangles, lightmap pages and their fill, AI nodes, graph
  vertices), the files with their sizes, the warnings. An agent reads it with its own file tools.
- `level.inspect` (a `read` command): what a compiled level folder holds - the project's, any folder, or a
  level of the linked game by name - by part: `summary`, `visuals` (bounds, shader, textures, vertices,
  faces, lightmap page; filtered by box, sector, shader; paged), `sectors`, `portals`, `lights`, `lightmaps`,
  `collision`, `ai_map`, `graph`, `files`.
- The baked files stay the game's binary formats. An agent changes the map - the scene's text files - and
  builds; nothing edits a baked file.

### 3.7 What was looked up, and what follows from it

Sources: Ylitie, Karras, Laine, "Efficient Incoherent Ray Traversal on GPUs Through Compressed Wide BVHs" (HPG
2017); tinybvh (MIT; BVH2 / CWBVH builders and GPU traversal kernels); the `obvhs` crate (MIT / Apache; PLOC
BVH2, CWBVH, SIMD traversal); Benthin et al. 2018 on wide BVHs (Embree); Unity's GPU lightmapper deep dive;
Frostbite's Flux (GDC 2018, Ray Tracing Gems 23); Bakery's "Baking artifact-free lightmaps"; xatlas; Unreal's
World Partition builders and Derived Data Cache; IXRay's `xrLC/Light` (Embree, CUDA and Vulkan back ends).

- **A software BVH in a WGSL compute shader is fast enough**: the one measured WGSL data point is about
  2.8 billion shadow rays a second on an RTX 4090 on a small scene; for a million triangles on this machine
  the estimate is 0.2-0.6 billion any-hit rays a second - 10^10 rays in half a minute to a minute. An 8-wide
  compressed BVH is about twice a binary one for incoherent rays on a GPU; tinybvh's kernel is the one to port
  (MIT: a notice with it). A1 starts with whichever tree gives a correct kernel first and ends with the faster.
- **Dispatches of tens of milliseconds**, well under Windows' two-second limit on a GPU task; the buffer limits
  asked from the adapter (the defaults are 128 and 256 MiB); no sync point inside a batch (Unity's design).
- **The alpha of foliage** stays a multiplied transparency (IXRay cut it to a yes/no at half to make its back
  ends agree; ours agree by reading the same 8-bit alpha, packed four texels a word as IXRay does).
- **On the CPU a wide BVH with single-ray SIMD traversal** gives 2-4 times a scalar binary tree per core
  (Embree's design); packets do not pay for incoherent rays. Section 3.5 is that.
- **One determinism rule for the CPU's cached triangle**: the cache lives for one chunk of samples and chunks
  are cut by sample index, never by the scheduler - so the light is the same on any number of threads even
  where a cached test and a walk could disagree by a rounding.
- **Packing in two levels** (xatlas' raster packing inside, a skyline outside): a visual's charts are packed
  into one tight rectangle, the rectangles into pages in a stable order - "one visual, one page" by
  construction, no search of a whole page per chart. Section 3.3's packer is this.
- **Not built here, and why**: adaptive sky sampling (a coarse set first, the full one where it disagrees) can
  cut the rays three to five times but loses light through small openings - it is the reserve if the targets
  of section 7 are missed, added by the architect in W2 and nowhere else; conservative texel rasterisation,
  leak-fixing rays, seam stitching and bicubic sampling are a quality project (the last needs the game's
  shader); a vertex-cache and overdraw pass on the visuals and a new lightmap texture format change what the
  game reads or how it draws (section 8.6 says what was decided of each).

## 4. The work

Three implementing agents in parallel on disjoint files, each in a git worktree of its own cut from the seam
commit, and a fourth, lighter one for the readable results; the architect writes the seam before them, takes
the steps before the compiler, and integrates after them; one critic reads everything once at the end.

| Who | What | Files it owns |
|---|---|---|
| Architect, before | W0: the seam, the bench tools, the baselines | `gz-lc-light/src/bake.rs`, `gz-lc-gpu` (skeleton, `examples/lc_bench.rs`), `tools/qa/bench_level.py` |
| A1 | The GPU lighter | `crates/gz-lc-gpu/**` |
| A2 | The light through the baker, visuals before light, the atlas, the wiring | `gz-lc-light/src/{deflector,mu,implicit,vertex,detail,global,lightmap,layer,point,tests}.rs`; `gz-lc/src/{lib,split,details,tests}.rs`; `gz-levelbuild/src/{compile,cmd}.rs`, its `Cargo.toml` |
| A3 | The CPU: the tracer and the structure phases | `gz-lc-light/src/{ray,mesh,math,hash2d}.rs` and a new `bvh4.rs`; `gz-lc/src/{geom,cform,tree,qslim,stripify,sphere,ogf,load,mender,vcache}.rs` |
| A4 | `build.report`, `level.inspect` (section 3.6) | `gz-fmt-level/src/compiled.rs` (new: a reader of the compiled level), `gz-levelbuild/src/{report,inspect}.rs` (new), the report's call in `pipeline.rs` |
| Architect, beside | W1: the steps before the compiler | `gz-levelbuild`'s steps (`files`, `prj`, `nav`, `graph`, `spawn`), `gz-scene/src/bulk` (the AI map's generation) |
| Architect, after | W2: merge, the whole build, records, QA, the documents | everything |
| Critic (one pass) | section 4.6 | reads only |

A file outside a row's list is not edited by that row. A need to change one is a line in the final report
("needed in <file>: ..."), which the architect applies.

### 4.1 W0 - the seam (architect, before the agents start)

1. `bake.rs` as section 3.4 says, with `CpuLighter`, `NullLighter`, `AutoLighter`, `LightChoice`, unit tests
   (`CpuLighter` over a batch equals `light_point` called per point, bit for bit).
2. `RayModel` read access for a lighter: `tris()`, `faces()`, `opacity()`, `cull()`.
3. The ray model's build moved out of `global.rs` into `ray.rs` (`build_ray_model`), `global.rs` keeps a thin
   call - so A3 owns the build and A2 owns the rest of `global.rs`.
4. `gz_lc::Options::light` and the `-light=` flag; `gz_lc::compile` takes `&dyn Lighter`.
5. `gz-lc-gpu`: the crate with `pub fn lighter(choice) -> Box<dyn Lighter>` answering the CPU lighter until A1
   fills it, and `examples/lc_bench.rs`: the compiler on a level folder without the editor -
   `lc_bench <level folder> root=<data root> light=cpu|gpu|off [mode=compile|rays] [stop_after=<phase>]
   [threads=N] [samples=N] [compare=1] [out=<folder>]`, printing every phase's seconds, the lighter's samples a
   second and the SHA-256 of every file it makes (nothing is written into the level folder; `mode=details`
   lights the detail slots). Built with `bash tools/rust/cg.sh build --release -p gz-lc-gpu --example
   lc_bench`, started as `Intermediate\cargo
elease\examples\lc_bench.exe` under `limited.py`.
6. The inputs under `%TEMP%\gz_bench\`: `escape_draft\build.prj`, `escape_std\build.prj`,
   `monolith_std\` (a whole compile in 43 s: the end-to-end measure), `yantar_std\` (with `build.details`: the
   detail light; it holds two invalid faces, so it compiles with `-no_invalidefaces`), the baseline tables
   beside them (`baseline.txt`, `base_*.json`). Read only for everyone.

### 4.2 A1 - the GPU lighter

**Goal.** `gz_lc_gpu::GpuLighter` implements `Lighter` on wgpu 30 compute and lights the samples of a real
level at least 20 times faster than `CpuLighter` does at the seam commit, on this machine. The seam's numbers
(`lc_bench ... mode=rays light=cpu`, release): Cordon - a scene of 3.71 M triangles, 372 static lights, 415 of
the hemisphere, 49 of the sun - 43 169 samples a second; the small level (0.41 M triangles, 154 / 256 / 49
lights) 478 339 samples a second. So the target on Cordon is 860 000 samples a second or more.

**Read first.** `gz-lc-light/src/{bake,point,ray,light,data}.rs` (the formulas to port - `light_point`,
`RayModel::trace`, `transparency`); `gz-render/examples/offscreen.rs` and `gz-render/src/testing.rs` (how this
workspace asks for a device without a window); wgpu 30's sources under `~/.cargo/registry/src/*/wgpu-30.0.1`
for any API detail - do not answer wgpu questions from memory.

**Build.**
1. *The device.* An instance and a device of the lighter's own (DX12 and Vulkan backends), never the window's.
   The adapter: a discrete card before an integrated one; a software adapter (`DeviceType::Cpu`) is no adapter.
   `Features::empty()`; the limits it needs (`max_buffer_size`, `max_storage_buffer_binding_size`, storage
   buffers per stage) asked from what the adapter offers. No `unsafe`, no `EXPERIMENTAL_*` feature.
2. *The scene.* A BVH of the lighter's own built on the caller's rayon pool (a second or two for a million
   triangles): binary with ordered traversal, or the 8-wide compressed one of section 3.7 (the `obvhs` crate
   builds it; tinybvh's kernel is the traversal to port) - the faster of the two by measurement on Cordon;
   triangles as `v0, e1, e2`;
   per triangle its flags (opaque, casts a shadow), its level face id, its texture slot and its three texture
   coordinates; the alpha of every texture a transparent shadow-casting face names, packed 8 bits a texel into
   one buffer with a table of offsets and sizes. Alpha that does not fit the binding limit: the largest
   pictures are halved until it does, with a log line - never an error.
3. *The kernel* (WGSL, one compute entry, 64 per group): `light_point` for one sample - the static lights
   (point lights through a uniform grid of light indices built on the CPU when the set holds more than 16),
   the sun, the hemisphere - each ray an any-hit walk with a fixed stack: a face that casts no shadow or is
   `skip` is passed through, an opaque hit answers 0, a transparent one multiplies by its texture's alpha
   rule; a face whose texture has no pixels is opaque. The level's rule and the details' (`Falloff::Details`,
   linear opacity, back faces culled) are uniforms of the same kernel.
4. *Batches.* `light` takes any number of samples and cuts them itself: a dispatch aims at 100 ms and never
   passes one second (Windows ends a GPU task at two); the size follows the measured time; two batches in
   flight; results read back through mapped buffers; `ctl` checked between batches (a Stop ends within a
   second). Every wgpu error - a lost device, out of memory, a validation error (error scopes) - is an `Err`
   with a sentence; nothing panics, nothing hangs: every wait on the device has a limit.
5. *The self-check.* The first batch of a scene is also lit on the CPU for up to 512 of its samples; if fewer
   than 99 % agree within 0.002 per channel the scene answers `Err` ("the graphics card's answers differ") and
   the caller uses the CPU.

**Tests** (`cargo test -p gz-lc-gpu`; each prints "no graphics adapter: skipped" and passes where there is
none): a synthetic scene of some thousand triangles with opaque walls, alpha-textured foliage, lamps, a sun
and a sky - GPU against `CpuLighter`: at least 99.9 % of samples within 0.002 per channel and a mean
difference under 0.0002; the skipped face; a face that casts no shadow; the details' rule; an empty scene; a
batch larger than one dispatch; a Stop in the middle; alpha larger than the limit (forced by a test limit).

**Measure** (once each, under `limited.py 300`): `lc_bench %TEMP%\gz_bench\escape_std root=<repo>
mode=rays light=gpu compare=1` (the samples a second of both lighters and how far apart their answers are) and
the same on `monolith_std` - in the report.

**Done when:** the tests pass with and without an adapter (force "none" by an environment variable of the
test), `mode=rays` on Cordon shows the GPU at least 20 times the seam's CPU number with 99.9 % of the samples
within 0.002, clippy and fmt are clean for the crate.

### 4.3 A2 - the light through the baker, visuals before light, the atlas

**Goal.** Every light of the compile goes through `Lighter`; structure no longer reads light; the lightmaps
are packed by visual.

**Read first.** This document's sections 2 and 3; `gz-lc/src/lib.rs` (`compile_project`: the phases);
`gz-lc/src/split.rs`; `gz-lc-light/src/{deflector,lightmap,mu,implicit,vertex,detail,global}.rs`.

**Build.**
1. *Gather, light, scatter* for each consumer, in chunks of at most about a million samples so memory stays
   bounded on a level of 10^9 samples; gathering and scattering on rayon:
   - the charts: the rows' jittered samples of many deflectors in one chunk, then - after the rows wrote their
     markers - the edge texels in the order `l_direct` takes them today (one edge may take a texel another
     would: that order stays);
   - the static models' own light (a scene per model, from `own_ray_model`), then the references in the level;
   - the implicit textures; the vertices; the detail slots (`compile_details_in_memory`).
   On the CPU lighter each consumer's light values are bit for bit what the code gives today: before changing
   a consumer, record its values on `tests.rs`'s project in a test, and keep that test.
2. *No second lighting*: a chart that `compress_rms` shrinks is resampled from what was lit (the filter
   `rms_test_size` already uses), not lit again.
3. *Visuals before light* and *the two-level packer*, exactly as section 3.3: `merge_geometry` before the
   light with the new key and limit; `merge_lightmaps` becomes the packer - a visual's charts into one
   rectangle (the mask as rows of 64-bit words, a chart's rows as words too, tested by shifts and `&`, never
   texel by texel), the rectangles into pages by a skyline; the faces' lightmap coordinates captured as today;
   the pages encoded to DXT5 in parallel. Placement is the second cost of a compile with lightmaps (9.4 s of
   43 on `monolith_std`, with the merge's 3.5 after it): the new one is at least five times faster there and
   makes no more pages than the old one did; the pages' count and fill go into the log. A test compiles `tests.rs`'s project with the CPU lighter and
   with a test lighter that scales every light by a seeded factor between 0.9 and 1.1: the structure of
   section 2 is equal, byte for byte, in both.
4. *The slow loops of `split.rs`*: `uv_map` scans the whole subdivision for its largest free face once per
   deflector and `subdivide` restarts its walk - give both the same results in less than quadratic time.
5. *The phases*: `PHASES` in the new order with plain names; the Stop tests of `tests.rs` hold for every phase.
6. *The wiring* (`gz-levelbuild`'s `compile.rs`, `cmd.rs`): `light=auto|gpu|cpu` on `level.build`,
   `level.compile` and Play's build, documented in their cards; the lighter made once per job from
   `gz_lc_gpu::lighter`; the job's stage says "on <lighter's name>"; the compile's report carries `lighter` and
   the rays traced.

**Measure** (once each): `lc_bench escape_draft light=cpu` against the baseline table (no phase slower);
`lc_bench monolith_std light=cpu` end to end; `lc_bench escape_std light=off stop_after=<the packing>` for
the cost of the charts, the merge and the packing at real sizes (the null lighter with the shrink off).

**Done when:** `cargo test -p gz-lc-light -p gz-lc` pass, the structure test passes, no phase of
`escape_draft` is slower than its baseline, clippy and fmt are clean for the three crates.

### 4.4 A3 - the CPU: the tracer and the structure phases

**Goal.** The CPU lighter at least three times faster - on Cordon `lc_bench ... mode=rays light=cpu` gives
43 169 samples a second at the seam, the target is 130 000 or more; on the small level 478 339, the target
1 400 000 -; every structure phase that costs two seconds or more on Cordon at least twice as fast, with the
same output.

**Read first.** `gz-lc-light/src/ray.rs`, `gz-fmt-level/src/bvh.rs` (today's tree and its guarantees - the
module text of `ray.rs` says why a cached triangle may only save a walk); `gz-lc/src/lib.rs`; the baseline
tables in `%TEMP%\gz_bench\`.

**Build.**
1. *The tracer*: section 3.5's 4-wide BVH inside `gz-lc-light` (`bvh4.rs`; the editor's general
   `gz_fmt_level::Bvh` keeps its API - the viewport picks with it). `RayModel::trace` answers bit for bit what
   it answers today: keep today's implementation in the tests as the reference and compare a hundred thousand
   seeded rays - axis-parallel ones through corners and edges, origins on box faces, transparent stacks - on
   one thread and on many. A new dependency only if the target is not reached without it, and then with its
   licence checked by `tools/rust/notices.py`.
2. *The ray model's build* (`build_ray_model`): the duplicate test over a million faces made parallel with the
   same faces kept in the same order; one build per compile - `adaptive_ht` builds a model for its sky light
   and the compile builds it again four phases later: light the tessellated vertices' sky through the
   `Lighter` after the one build, unless something between the two reads the vertex colour (then say what).
3. *The structure phases* by their cost on Cordon (draft / standard): the sectors 6.3, the ray model 6.0, the
   tessellation 5.8 / 15.6, the collision 2.2 / 8.4, optimizing 3.3, the tangent basis 2.7, the visuals 2.3,
   the static models' visuals 2.0. Find where each spends its time (a timer per sub-step in the phase's log is
   enough - keep those lines), then make it parallel or change the algorithm. The output of every file stays
   byte for byte: `lc_bench escape_draft light=off` prints the files' hashes - record them at the start, they
   are equal at the end.
**Measure** (once each): `lc_bench escape_std mode=rays light=cpu` before and after; `lc_bench escape_draft
light=off` before and after (the phases' seconds and the files' hashes).

**Done when:** the samples a second of the CPU lighter on Cordon are at least three times the seam's; each
phase and step listed is at least twice as fast or the report says in a sentence why it cannot be; the hashes
are equal; `cargo test -p gz-lc-light -p gz-lc` pass; clippy and fmt are clean.

### 4.5 W1 and W2 - the architect

A4 builds `build.report` and `level.inspect` (section 3.6) with their tests and cards. W1 is the steps
before the compiler - the game files (4.2 s on Cordon), the geometry step (up to 4.4 s), the game graph and
the spawn (1.4 and 1.5 s), the AI map's generation over the ground (13.2 s for Cordon's 2.48 M nodes, already
flooding in parallel waves). A step under two seconds is left alone; one over it gets its factor of two where the code
allows, and `level.ai`, `level.graph`, `level.game` and `level.spawn` stay byte for byte for the same scene
(a generated AI map's nodes in the same order: saves hold their numbers).

W2, when the three branches are in: merge them (disjoint files), build the whole editor, record the compile
records again on the CPU lighter, extend `test_level_compile.py` (the CPU run against its record byte for
byte; a GPU run whose structure equals the record's and whose lightmaps differ from the CPU's by at most
2/255 on 99.9 % of texels), run section 6's checks once, write the contracts (`docs/ARCHITECTURE.md` section
on the level build, `docs/COMMANDS.md`, `docs/STATE.md` with the measured numbers, `CLAUDE.md`'s "The Level
tab", `tools/mcp/AI_SETUP.md`), remove this plan.

### 4.6 The critic - one pass, after everything

One agent (Fable), one full reading, no second round; the architect fixes what it finds and proves the fixes
with targeted tests. Its brief lists every risk up front:

- section 2 held everywhere: any place where a light value, a thread count, a hash map's order or a GPU
  answer can reach structure; the packer's fallback; `level.ai` and saves;
- the GPU lighter: the kernel against `light_point` and `trace` formula by formula (falloffs, the 0.01 and
  0.001 offsets, the ranges, the skip rule, alpha wrapping, the two opacity rules, culling), stack overflow of
  the walk, a batch boundary, readback order, limits and their failure paths, a device lost in the middle,
  Stop, a machine with no adapter, a software adapter;
- the CPU tracer: any hit lost at a box border, the cached triangle, NaN and degenerate triangles;
- the gather / scatter code: a sample written to the wrong texel, the edges' order, memory on a level of 10^9
  samples, the jitter averages;
- every parallel phase: the same bytes on one thread and on sixteen;
- the commands: `light=` in the three ways in, `level.inspect` on a folder outside the project (read only, no
  write), the cards;
- what a Stop leaves on the disk; the write guard; nothing written into a game;
- the tests: what they prove and what they only claim.

## 5. Rules for every agent

- Work only in your worktree and only in your files. Commit there at the end with `git add <your files>` and
  one `git commit`; no other git command that changes anything, nothing pushed.
- No agents of your own. No QA suite (`tools/qa/run_all.py`, `test_*.py`) - those run once, at the end, by the
  architect. Your proofs are your crates' unit tests and the measurements your brief names.
- Every program you start runs under `python tools/rust/limited.py <seconds> ...`; cargo through
  `bash tools/rust/cg.sh ...`, only for your crates (`-p`). A measurement runs once, never in a series, and
  alone on the machine: `python tools/rust/alone.py <seconds> <program> ...` (it waits for the others'
  measurements and limits the run). A run that passes its limit is cut, read and understood, not started
  again as it was. Other authors compile on this machine meanwhile: judge factors, not per cents.
- A new external crate goes into your own crate's `Cargo.toml` with its version, and into your report (the
  architect checks its licence and moves it to the workspace's table). `Cargo.lock` is not committed.
- Code first, measurements after: write the whole of your part, then test and measure.
- No `unsafe`, no `unwrap` / `expect` outside tests, no `static` state, no thread made by hand (rayon, the
  caller's pool), `cargo fmt`, clippy without a warning, English comments only where the why is not obvious.
- The data root for the compiler's inputs is the main checkout (`D:\Games\XFined-Editor`), passed as `root=`;
  the bench inputs under `%TEMP%\gz_bench` are read only. Never open, build in or write to a project under
  `D:\XFinedProjects`, the game's folder, or the main checkout.
- The final report: what was built, the measured numbers before and after, what is not proven, what you
  needed outside your files. No offers of more work.

## 6. Tests and measurements

All code first; the long checks once, at the end, in this order; anything that runs clearly longer than its
target is cut and read, not waited for.

1. `bash tools/rust/check.sh`, `bash tools/rust/test.sh` (what changed).
2. `python tools/qa/run_all.py` once (the compile suite on both lighters; `test_play_game` with the real game
   on a level lit at 10 texels a metre - the game loading the new atlas is the proof no unit test gives).
3. `python tools/qa/bench_level.py`, each once, against the baselines: Cordon draft (limit 120 s), Cordon
   standard on `auto` (limit 600 s), the small level standard on `cpu` and on `auto`.

## 7. Done means

- Cordon, draft: the compile in 19 s or less (38.3 before).
- Cordon, standard, `auto` on this machine: the whole build ends inside the 600 s the old one did not; the
  target is five minutes.
- The CPU lighter alone: at least three times the old rays per second; the small level at standard at least
  three times faster end to end.
- Every step and phase of two seconds or more: twice as fast, or one sentence in `docs/STATE.md` why not.
- Section 2 proven by tests: the structure equal across lighters and thread counts; the records byte for byte.
- The game starts on a level the new build made, lightmaps included.
- `build.report` and `level.inspect` there, documented, tested.
- The contracts say what is; this plan is gone; nothing half done.

## 8. Stage 2 and 3: pieces of a level, streaming, and baking what the game reads

The author's word (2026-11-15): everything in this one run, to the end. The engine's side was mapped first
(the game's repository, read only): one loader source (`src/Layers/xrRender_R2/r2_loader.cpp`) is compiled into
all three renderers that ship (R4 DirectX 11 - the default -, R5 DirectX 12, R6 Vulkan); a visual names its
shader, its buffers and its children by index into level-wide tables; the static collision and the AI map
cannot change after a level has loaded (one collision model whose triangle indices live in query results;
saves hold `level_vertex_id`s); the weather's far plane is 1000 m on most weathers and down to 150-200 m in fog.

### 8.1 What a piece is

A **piece** is a compiled part of a level's static picture that the game loads beside the level: its own
visuals, vertex and index buffers, shader table and lightmap pages. It is made by the level's own author (the
level cut into streaming cells) or by a module for a level it does not own (a lit building in Cordon).

- **Only the picture is a piece.** Collision, the AI map, the game graph and the spawn stay as they are and
  where they are - whole in a level's own files, or in a module's `overlay.xcform`, `overlay.aimap` and
  `.xspawn` - composed once when the level loads. Nothing a save refers to can stream, so nothing does.
- **A piece hangs on the level's sectors.** It brings no sectors or portals: each of its root visuals is a
  child of a sector root of the level - a sector id the piece names (a level's own cells), or the sector
  found at the visual's centre (a module's piece). The level's sector ids never move (`level.ltx`'s
  `sub_level_map` keys on them).
- **Files**, in `<module>\levels\<level>\pieces\<name>\`, read physically per module like
  `overlay_visuals.ltx`, under the module's and the modes' gates:

```text
piece.ltx      the manifest (text):
               [piece] version = 1
                       box = minx, miny, minz, maxx, maxy, maxz
                       sector = auto | <sector id>      one sector for every root, or by each root
                       stream = true | false            false: loaded with the level and kept
                       mode = <id>                      optional extra mode gate
piece          chunks as the level file has them: 1 header, 2 shader table, 3 visuals; and
               32 roots: u32 count, then per root u32 visual index, u16 sector (0xFFFF: the sector at
               the visual's centre), u16 reserved. Children are indices into the piece's own visuals.
piece.geom     chunks 9 vertex buffers, 10 index buffers, 11 sliding windows, as level.geom
piece.geomx    the fast (position only) buffers, as level.geomx
<name>.dds     a texture the piece's shader table names and that lies in the piece's folder is the
               piece's own (its lightmap pages `lmap#N_1`, `lmap#N_2`): it is loaded from there, never
               from the level's folder, under a name no other piece or level can have
```

- **What the game states** (an engine's ability in its version resources, which the editor reads):
  `level-pieces` - it loads pieces. The editor builds pieces only for a game that states it.

### 8.2 E1 - the engine (the game's repository; starts now, beside stage 1)

**Goal.** The game loads pieces with the level, streams them by distance, and frees them with the level, in
all three renderers, with the base level's frame cost unchanged when no piece is there.

1. *A piece's own tables.* While a piece's visuals load, `getShader`, `getVB`, `getVB_Format`, `getIB`,
   `getSWI` and `getVisual` answer from that piece's tables (a `thread_local` "piece being loaded", so a
   piece can be read on a task while the main thread loads a dynamic model). A piece's visuals never enter the
   level's `Visuals`, its shaders never the level's table.
2. *The piece's textures*: a name of the piece's shader table whose file lies in the piece's folder is
   registered with the texture manager under a unique name that keeps its first characters (the deferred
   blender tells a lightmap by the name starting `lmap`) and resolves to that physical file; freed with the
   piece.
3. *Attach and detach* as `AttachOverlayVisual` does it (the sector root's children, the root box grown only
   when the piece sticks out), at the frame's safe point (the top of `CRender::Calculate`, where the
   vegetation switches already move nodes), telling every system that keeps the visual tree - the clusters,
   the GPU trees of R5 / R6, the ray tracing scene, the temporal history - the way `da_veg_apply_switch` does.
4. *Streaming.* A manager checked every few frames at the safe point: a piece loads when the camera is
   nearer than the far plane plus the piece's radius plus a margin, and leaves at a quarter more than that
   after at least five seconds; reading the files and creating the buffers happen on a task, the shaders,
   textures and visuals at the safe point, at most one piece a frame; the pieces in range when the level loads
   are loaded inside `level_Load`; `stream = false` and the console's `r__pieces_stream 0` load all with the
   level. `level_Unload` waits for a load in flight and frees every piece before the level's own tables.
5. *The ability* of 8.1 in the version resources (`src/xrCore/ProductVersion.h`).
6. *The documents*: `docs/dead-air/XMS_ARCHITECTURE.md` (8.2's "honest path" becomes what is), `MODDING.md`,
   `TEST_MATRIX.md`. No word of any tool a module is made with.
7. *The proof*: `tools/qa/pieces/` - a script that cuts the visuals inside a box out of a compiled level of
   the game into a piece of a test module (its buffers, shader table and lightmap pages) and hides the
   originals with a cut box; a runner that starts the game on that level with and without the module and
   compares the two frames (`Compare-Frames.py`), reads the log for the piece's lines (loaded, attached,
   unloaded, counts) and for a clean exit; streaming proven by moving the camera past the piece's distance
   and back by console commands; a save made with the module on loads with it off and the other way round.
   Each launch at most 300 s, six launches in all.

**Bounds.** The save contract and the XMS contract are untouched (pieces are a new, versioned, additive
format; `overlay_visuals.ltx`, `.xcform` and `XAM1` behave as before). All three renderers build. No per-frame
cost for a level without pieces beyond one empty check.

### 8.3 A5 - the editor (after stage 1 is merged)

1. *A module's piece of a level the game has*: a step of the level build, `piece` - needed on a stock level
   whose scene holds authored static objects, for a game that states `level-pieces`. The build project is
   made of the authored objects alone; every other static object of the scene goes in as occluders - faces
   the ray model holds (with their textures' alpha) and nothing else reads. The compiler in piece mode runs
   the structure, the light and the atlas, and writes `piece`, `piece.geom`, `piece.geomx`, the pages and
   `piece.ltx` into `<project>\levels\<level>\pieces\<module id>\`, and the authored objects' collision into
   `overlay.xcform` (its cut list kept). Its scratch (`build.prj`) lives under the project's `temp\`.
2. *A level's own streaming cells*: Level Settings > Streaming Cells (off, or a cell size in metres;
   `scene.set` key `build.cell_size`). With cells on, the light step writes the level as its base (sectors,
   portals, lights, every visual larger than a cell) and one piece a cell for the rest, each with pages of
   its own; visuals never merge across a cell's border. Off: the level as today.
3. `level.inspect part=pieces`, `build.report`'s pieces, the status of the new step, Play on a stock level
   shipping the piece, the documents.

### 8.4 Baking what the game reads - looked at, not built

The deferred static blender reads only the second texture of a lightmap page, which made a 4 x 4
placeholder for the first look like half of the lightmaps' bytes for nothing. The engine's own check (E1)
says otherwise: the forward pass of alpha-blended lightmapped materials and of the `LmEbB` blender
(`blender_deffer_aref.cpp`'s R4 branch, `Blender_Lm(EbB).cpp`, `r3/lmape.ps`) samples the first texture -
its rgb is the base light and its alpha the sun. Which pages such materials land on is known only with
the engine's shader library, and the gain is about 1.3 MB a page. Both textures are written, as before;
the engine states no `lightmaps-second-page`.

### 8.5 Proofs of stage 2 and 3

- The engine's own runner (8.2.7) on a fixture piece.
- End to end: `test_play_game` builds a module's piece over a level of the game (two authored objects),
  plays the module in the real game from Play's root, and reads the game's log for the piece attached with
  its lightmap pages; and builds a small level with cells and plays it.
- `test_level_compile`: a piece's structure is the same on both lighters; the cells' visuals together are
  the level's visuals.
- Saves: the engine's runner (a save across the module on and off).

### 8.6 Decided, not deferred

- **Adaptive sky sampling** - built only if Cordon at standard quality on `auto` misses five minutes after
  everything else; it trades light through small openings for rays.
- **Sectors and portals inside a piece** - not built: the engine's sector graph is per render context and
  static after load, and `level.ltx` keys on sector ids; a piece's interior draws correctly without them and
  only loses portal culling inside itself.
- **Streaming of collision, AI, details, HOM, sound geometry** - not built: one model each, composed at
  load; saves and live queries refer to them.
- **Texel quality work, bicubic sampling, a new lightmap format, another vertex-cache pass** - not built: no
  defect to fix was measured, and each changes what every level looks like or how every level draws.

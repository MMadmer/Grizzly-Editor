# Brief: phase 8a - the compiler's model and its light (`gz-lc-light`)

The editor is Rust; the level compilers are the last C++ (`docs/RUST_MIGRATION.md`, phase 8 -
read it first: it says how phase 8 is built and what its gate is). This pass ports
`Source/Editors/XrLCLight` (16k lines) and `Source/Editors/xrDO_Light` (0.6k) into one new
crate, `crates/gz-lc-light`. The next pass ports `XrLC` on top of it (`gz-lc`: the pipeline,
`build.prj` in, the level's files out) and compares both with the C++ compilers' output; you
build what it will stand on. You are alone in the tree.

You may edit `crates/` (the new crate; an existing crate only to make public what you reuse),
`Cargo.toml`, `docs/rust/STATE.md`. `Source/` is the specification: read it, do not edit it.

## What the crate is

- **The compiler's data model**, the one `XrLCLight` holds for both compilers (`xrLC_GlobalData`,
  `xrFace`, `xrDeflector`, `Lightmap`, the MU models `xrMU_Model*`, the materials, shaders and
  textures of a build, the light sets `base_lighting`, `R_Light`): plain owned data, indexes
  instead of pointers, no global (`lc_global_data()` becomes a value the caller holds), `Send
  + Sync` by construction. Name the C++ type each Rust type stands for in its doc line.
- **The ray model** (`xrBuildRapidModel`'s product as the lighting uses it: the collision mesh
  with, per triangle, what a ray needs - the two-sidedness, the opacity texture and its UVs for
  alpha-tested surfaces, cast-shadow flags): a BVH of your own or the workspace's
  (`gz-scene/src/pick.rs` has one - reuse it by making it shareable if it fits, do not
  duplicate), and `rayTrace` as `xrDeflectorLight.cpp` / `xrLightVertex.cpp` do it, float for
  float where the result depends on it (the opacity threshold, the energy falloff, the order
  of hits).
- **The lighting**: the direct light of the sun, the hemisphere and the static lights
  (`LightPoint`, the three layers rgb / hemi / sun), a deflector's lightmap (`xrDeflectorLight`,
  `xrDeflectoL_Direct`, the jitter of `m_lm_jitter_samples`, the borders and the blur, the RMS
  shrink of `m_lm_rms*`), `Lightmap` (packing a deflector's layers into the atlas pages and the
  page's pixels - the page writer itself, DDS, is `gz-fmt-tex`'s), vertex light
  (`xrLightVertex`), implicit light of terrain-like textures (`xrLight_Implicit`, with
  `xrImage_Resampler`), the MU models' light (`xrMU_Model_Calc_lighting`, the references),
  and the detail objects' light of `xrDO_Light` (`detail_slot_calculate`,
  `global_calculation_data`).
- **Not ported** (D24): everything `net_*`, `lcnet_*`, `recalculation` for the net, the
  `-gi` radiosity. Say in STATE.md which files that was.

## How it is built

- **Parallel by construction** - this is the editor's heaviest work, and every core is to be
  loaded: deflectors, texels, vertices, MU models and detail slots run on `rayon` inside the
  caller's pool (the crate takes no pool of its own and spawns no thread); a long loop takes
  a cancel token and reports progress through a callback (`gz-task`'s types).
- **The same result on any number of threads**: a texel's jitter and any random number come
  from the texel's own coordinates (the C++ uses fixed jitter tables - keep them), never from
  a shared generator; results are gathered in input order. A test runs the same small scene on
  1 and on 8 threads and wants equal bytes.
- No `unsafe`, no global, no `static` holding state; `f32` where the C++ computes in `float`,
  the operations in its order where a rounding could move a texel.
- Tests of the crate's own, on scenes small enough to reason about: a lit and a shadowed texel
  with the value the C++ formula gives (worked out in the test's comment from the C++ lines),
  an alpha-tested occluder, the hemisphere's sample count, the border fill, the packing of
  deflectors into a page without overlap, a vertex light on a two-triangle floor, a detail
  slot's four corners. No test needs a graphics device or a file of the game.
- What you cannot prove here (equality with the C++ compiler on a real level) the next pass
  proves; leave it what it needs: a public API that mirrors the calls `XrLC` makes into
  `XrLCLight` (list them in STATE.md with the C++ call each replaces), and a debug dump of a
  lightmap page as a plain image.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`; your crate's tests while
  you work (`cg.sh test -p gz-lc-light`); at the end `tools/rust/check.sh` and
  `tools/rust/test.sh` (default mode) green. No QA suite needs running: you change no editor
  behaviour.
- Never run a git command that changes the working tree or the index.
- Keep `docs/rust/STATE.md` current: a section "Phase 8a: the compiler's model and light".

## Report

Under 350 words: what the crate holds (modules, lines, tests), the C++ files ported and the
ones left out and why, the public API the pipeline will call, what is proven by tests and what
waits for the comparison with the C++ output, anything you could not port and exactly why.

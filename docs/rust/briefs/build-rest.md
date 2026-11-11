# Brief: what the level build and the mesh exporters still lack

Four pieces of the C++ editor's build path are not ported. Each is exact geometry; port it
float for float. Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first ("From the
OGF port", "From the level export and build port", "Closed in integration"); the rustdoc at
the top of `gz-fmt-object`, `gz-fmt-ogf` (`export.rs`: the exporters, `optimize.rs`),
`gz-levelbuild` (`validate.rs`, the spawn build, `files.rs`), `gz-scene` (read only).

You own: `crates/gz-fmt-object/**`, `crates/gz-fmt-ogf/**`, `crates/gz-levelbuild/**`, a new
crate `crates/gz-geom` if the shared geometry needs a home (`src/lib.rs` before `Cargo.toml`).
Another agent works in the tree at the same time in `gz-scene`, `gz-ui-level`,
`gz-ui-viewport`: read those, touch nothing of them; if `cargo check` fails in a crate that is
not yours, wait and retry.

## 1. Minimum boxes (`gz-geom` or `gz-fmt-ogf`)

The C++ fits oriented boxes with Wild Magic's `MinBox3`
(`Source/Editors/XrECore/WildMagic/WmlContMinBox3.cpp`, with `WmlContBox3.cpp`, the convex
hull it starts from, and `RAPIDMinBox.cpp`; `Source/Editors/FreeMagic/MgcCont3DMinBox.cpp` is
the older twin - find which one each caller really uses: `ExportSkeleton.h` /
`EditObjectSkin.cpp` for bones, `GeometryPartExtractor.cpp` for parts). `gz-fmt-ogf` fits a
covariance box instead, so `OGF_S_BONE_NAMES` differs from the C++ in 60 bytes per bone. Port
the fitter(s) the callers use and switch `bone_obb` to it: an exported skeleton's bone boxes
equal the C++ editor's.

## 2. Progressive skeletons (VIPM)

`gz-fmt-ogf` exports a skeleton flagged progressive without its sliding windows, with a
warning. Port the pass: `Source/Editors/XrETools/PropSlimTools.cpp`, `object.cpp`,
`object_sliding.cpp`, `mesh.h`, `ArbitraryList.h`, `BinaryHeap.h`, `DLink.h`, `quad.h` (the
VIPM collapse order, the sliding-window records) and how `ExportSkeleton.cpp` calls it
(`MakeProgressive`, `OGF_SWIDATA`). The warning goes; the exported file equals the C++ one
(STATE.md says the lookahead face optimiser is already float for float - keep the order of
passes the C++ has).

## 3. Breakable and climable objects (`gz-levelbuild`)

`BuildGame` adds them to `level.spawn`: `Source/Editors/LevelEditor/Editor/Tools/StaticObject/
ESceneObjectToolsExportBreakable.cpp` over `Editor/Utils/GeometryPartExtractor.cpp` (parts
split by connectivity and game material, each part's box from section 1, the generated
skeleton OGF of a breakable part written beside the level, the `breakable_object` /
`climable_object` entities with their shapes and visuals). Port all of it: the same parts, the
same files and names, the same entities as the C++ build writes for the same scene. The spawn
entities go through `gz-entities` as the rest of the spawn build does.

## 4. The rest of `Scene->Validate`

`validate.rs` names what it leaves out: the portal test (`CPortalUtils::Validate` - the
sectors' captured meshes exist in the scene now: read how the scene holds a sector's meshes),
sector and wallmark shader checks, the LOD texture notes, the sound library check. Port them
with the C++ texts; a build refuses and warns exactly where the C++ build does
(`Source/Editors/LevelEditor/Editor/Scene/` - `EScene::Validate` and what it calls).

## Rules

- Float for float means `f32` where the C++ has `float`, the same order of operations, the
  same epsilons and tie-breaks. Where equality is impossible (an uninitialised byte, an address
  order) say exactly which bytes and why, in the crate's rustdoc and in STATE.md.
- Heavy passes run on the CPU pool where the callers are jobs already; nothing here may be
  called from a UI frame. No `unsafe`, no global state.
- Tests are written, not run: a box fitted to hand-made points with the C++ answer worked out;
  a small progressive mesh's collapse order; a scene of two connected boxes with one breakable
  material giving two parts; each validation refusing its bad scene.
- Compile: `bash tools/rust/cg.sh check -p gz-fmt-object -p gz-fmt-ogf -p gz-levelbuild -p
  grizzly`, zero warnings; clippy clean for your crates.
- Fully autonomous; never ask; never start agents of your own. Never run a git command that
  changes the working tree or the index. `Source/` and the game are read-only.

## Report

Under 300 words: each of the four as done / partly / not done with the reason, what stays
unequal to the C++ bytes and why. Then update the STATE.md sections named above.

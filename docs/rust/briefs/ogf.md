# Brief: the compiled model format (`gz-fmt-ogf`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` (its "Findings" about the exporters are
yours to use) first; then `crates/gz-chunk/src/{lib,reader,writer}.rs` and the rustdoc at the
top of `crates/gz-fmt-object/src/lib.rs` - the editable object model you export from.

You own: `crates/gz-fmt-ogf/**` (a placeholder now).

## Scope

**Read** every OGF visual the game ships:
- header (format version 4, type, shader id or texture + shader names, bounding box and sphere);
  normal, hierarchy (children / child references), progressive (sliding window items), LOD,
  tree (ST / PM), skeleton animated, skeleton rigid, skinned geometry PM / ST with 1-4 bone
  weights (vertex formats 1L / 2L / 3L / 4L);
- vertex and index containers inline: decode the FVF / declaration into one neutral vertex
  layout (position, normal, tangent, binormal, uv, lightmap uv, colour, bone ids and weights);
- bone data (names, IK data with shapes, joints, mass, centre), user data, the description
  chunk, motion references, embedded motions (bone-part definitions, motion definitions with
  flags, speed, power, accrue, falloff, marks; key data with 8 / 16-bit quantised rotations and
  translations);
- `.omf` (the motions chunks alone).

**Decode** motion keys to bone transforms at a time `t` (what the engine does for one motion;
a two-key lerp / slerp is enough), so the editor can pose a skeleton for a preview.

**Export** from an editable object, byte-compatible wherever the algorithm is deterministic:
- static (`ExportObjectOGF.cpp`): splits by surface, vertex welding, bounds, progressive data
  if the C++ produces it;
- skeleton (`ExportSkeleton.cpp`): weighted vertices with 1-4 links, bone data, user data,
  motion references or embedded motions with the keys quantised exactly as the C++ does.
- The earlier analysis says both `CalculateTB` bodies are commented out (no NVMeshMender), and
  that the skeleton exporter never welds, which would reduce VIPM and the face optimiser to
  fixed reorderings. **Verify both in the C++ before relying on them**; if VIPM does real work
  for static objects, port what `XrETools` does for it (`Source/Editors/XrETools`: grep
  `VIPM_`, `ETOOLS::`).

## The C++ that is the spec

- `Source/XrEngine/Fmesh.h` (chunk ids, header), `SkeletonMotions.h/.cpp`, `Bone.h`.
- `Source/XrRender/Private/`: `FBasicVisual`, `FVisual`, `FProgressive`, `FHierrarhyVisual`,
  `FSkinned`, `SkeletonX`, `SkeletonCustom` (bone data), `SkeletonAnimated` (motions),
  `SkeletonRigid`, `FLOD`, `FTreeVisual`, `ModelPool.cpp`; the vertex declarations they use
  (`rg -n "D3DFVF_|vertHW_|vertBoned|r1_vert" Source/XrRender/Private`).
- `Source/Editors/XrECore/Editor/ExportObjectOGF.h/.cpp`, `ExportSkeleton.h/.cpp`, and the two
  `*CalculateTB.cpp`.

## Rules

- Readers take `&[u8]`, return owned immutable models (`Arc<[T]>` for vertex and index data:
  the renderer and the previews share them); writers return `Vec<u8>`; no file access; no
  shared state - hundreds of models load in parallel.
- Hostile input is an `Err`: never a panic, never an allocation by an unchecked count.
- Use the exact quantisation of the engine (`gz_chunk` now rounds as the engine does; check any
  other formula against the C++ rather than assuming).
- The API at the top of `lib.rs`. Tests written, not run: read-back of what your exporter
  writes, key decoding against values derived by hand, hostile input.
- Compile: `bash tools/rust/cg.sh check -p gz-fmt-ogf --tests`, zero warnings.

## Report

Under 300 words: entry points, what you verified about TB and VIPM, what is not done or not
certainly byte-compatible. Then update the `gz-fmt-ogf` row in `docs/rust/STATE.md`.

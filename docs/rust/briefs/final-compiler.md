# Brief: the final review's findings - the compiler and its proof

The final review of the move to Rust (the reviewer's report is quoted below where it matters;
its scripts and outputs are under `Intermediate/critic-final/`) found the level compiler not
deterministic and its proof thinner than the plan says. You close that half. Read
`docs/rust/STATE.md`, "Phase 8b: the compiler and its proof" and "Phase 8c: the compiler in the
editor, no C++" (section 1), and `docs/RUST_MIGRATION.md` section 9, "The rules of equal to the
C++ compilers", and phase 8.

**Another engineer works in the same tree at the same time** on Play and the tests
(`crates/gz-levelbuild`, `tools/qa/test_play_game.py`, `tools/rust/check_test_files.py`,
`tools/rust/test.sh`, `tools/rust/check.sh`, `tools/qa/qa_target.py`). Your files:
`crates/gz-fmt-level`, `crates/gz-lc-light`, `crates/gz-lc`, `tools/qa/diff_compile.py`,
`tools/qa/compare_compile.py`, and your own section of `docs/rust/STATE.md` ("The final review,
the compiler": append it, touch no other section). Keep your crates compiling between edits
(`bash tools/rust/cg.sh check -p <crate>` after each): the other engineer's builds depend on
them. If a build fails in a file that is not yours, wait a minute and build again; never edit it.
The machine is shared: a C++ recording puts itself into the idle priority class, so its wall
time means nothing while the other engineer builds - give a recording a limit that holds
(`LIMITS`), and report CPU seconds.

## 1. The compile is the same on every run and on any number of cores

The plan promises it (phase 8: "Every core, and the same result on any number of them"); the
reviewer found it false:

> `crates/gz-fmt-level/src/bvh.rs:339,351-358`, `crates/gz-lc-light/src/ray.rs:164-171`.
> `Probe::enter` rejects a box when an axis-parallel ray's origin lies exactly on its far face
> (`0 x 1e30 = 0`). The cached-triangle test skips the boxes, so the light depends on which rays
> a rayon split traced before. l03u, five 16-thread runs of one binary gave three different
> `level.geom`, 18-26 vertex hemisphere bytes +-1. AdaptiveHT's hemisphere precalc
> (`gz-lc/src/geom.rs:453-463`) differs every run, on vertical rays through a ceiling triangle's
> corner. Minimal case `bvhedge.exe`: the triangle test answers `Some(5.0)`, `ray_nearest`
> answers `None`.

- Fix the slab test (a zero direction component: the ray is inside that slab iff
  `min <= origin <= max`, no multiplication by a huge reciprocal) and whatever else makes an
  answer depend on what was traced before: a cache may only save work, never change a result -
  if the cached-triangle shortcut can answer differently from the walk, the walk is wrong or the
  cache is; make them agree and prove it (a test that traces the same rays with a cold and a
  warm cache and in two orders). What did the C++ do on the same ray (Opcode's ray-AABB test,
  `git show cpp-compilers-last:Source/XrCDB/...`)? The port answers as the C++ does where the
  C++ is right and deterministic, and says so where it differs.
- `gz-fmt-level`'s BVH has other callers (the reviewer did not trace them): list them (`rg`),
  say for each whether the fault reached it (a vertical ray through a triangle's corner or edge
  is exactly what a vertical probe and an AI map node's height cast), and add the unit test of
  the minimal case to `gz-fmt-level`.
- **The proof**: `diff_compile.py` compiles a scene twice with the Rust compiler and wants every
  output file byte for byte equal (on by default for the scenes where a second compile costs
  seconds; `--all` does it for all four), and once on ONE thread against all threads for the
  smallest scene (the same bytes). The check names the first file and offset that differ. Five
  runs of l03u on all threads: one hash.

## 2. The comparison compares what section 9 says

> `tools/qa/compare_compile.py:489-490` skips zero-area lightmap triangles: 48 % of l03u's, 17 %
> of l12u's, 64 % of mp_test's, against the rule at lines 40-42. I compared them myself
> (`light_flat.py`): l03u and l12u are inside the bounds. mp_test's sun colour differs by mean
> 11.5, p99 33 (C++ 33, Rust 0, the texel is shadowed).
> No record holds an implicitly lit texture or a second lightmap page. Terrain light and
> multi-page merging are unproven against the C++.
> `compare_compile.py:69-70`: the detail bound is in 4-bit steps (8.5 / 34 of 255); the plan
> says 0..255. Seen: 0. The lightmap bounds are 4x / 2x what was seen.

- **Flat triangles are sampled** (a triangle whose lightmap coordinates collapse to a texel or
  a line: its texel, or the texels along it), on both sides through each side's own
  coordinates, and judged with the rest. mp_test's shadowed texel (C++ 33, Rust 0): find out
  which side is right - what the compiler wrote before the DXT5 encoder (both sides can dump or
  recompute it: the C++ lightmap's `lm.surface` values are not on disk, so reason from the
  block: its other texels, the encoder's interpolation) - and either fix the Rust side or state
  the rule with its reason; no silent skip.
- **A scene with implicit light and a scene with two lightmap pages are recorded and pass.**
  Implicit light: `mp_close_combat`, whose XrLC run passed ten minutes in "LIGHT: Implicit" at
  the scene's own settings - lower what costs the time and does not touch the implicit pass
  itself (as `OPTIONS` does for l03u; read `xrImplicit*.cpp` at the tag for what it depends on)
  and give it a limit up to forty minutes for its one recording; it also brings MU models with
  LODs under a second set of options. Two pages: the cheapest scene that needs them - `mp_test`
  with a higher `light_pixel_per_meter` under a key of its own is twenty seconds a run. If a
  scene cannot be recorded, say exactly why and what it leaves unproven.
- **The bounds say what they are**: the detail objects' light in the units the plan names (or
  the plan's sentence in the tool's units - one of the two changes, and "seen 0" means the
  bound is byte equality: make it that); the lightmap and vertex bounds near twice what was
  seen across all recorded scenes, each with the scene and channel that set it, in the tool's
  header and for the plan.
- **What stays unproven is written down** in the tool's header and in STATE.md, "For the
  plan": every part of the compile that no recorded scene exercises (walk `CBuild::Run` at the
  tag phase by phase and say which scene covers it).

## 3. Smaller things in the same files

- `diff_compile.py --refresh-oracle` on a scene that has a record: `os.replace` onto a folder
  raises `PermissionError` and leaves a `.part` (line ~514). The new record replaces the old one
  whole (the old one moved aside, then removed); a failed recording leaves the old one in place.
- A `build.prj` cut short is refused with a sentence that says so: with its last byte missing
  the compile exits 0, cut in half it answers "no hemisphere lights" (`Intermediate/critic-final/
  bad_inputs.py` has 23 broken inputs; none may panic, each answers what is wrong with the file).
- The level's files appear together or not at all: `gz-lc/src/lib.rs:365-377` writes them one by
  one into the level folder, so a compile that fails or is stopped while it writes leaves a
  level of two compiles. Write beside and move into place at the end, as the module build does
  (`crates/gz-build/src/install.rs` for the pattern); the writer guard still stands in front.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index. The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`; scripts longer than a line
  go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- The C++ compilers are started only by `diff_compile.py`, for the new recordings (they show no
  window under `-silent`; the tool checks it). Nothing under `Bin\` is removed or rebuilt.
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from; the game
  is read only; `find` over `D:\XFinedProjects` and `D:\Games\Dead Air\modules` before your first
  run and after your last shows nothing changed.
- Nothing that already passed is run again to check something new: one crate, one scene while
  fixing; every long run writes to a file that you look at often. Do not chase a number with
  series of runs: measure once, state it.
- Proof at the end, once: `bash tools/rust/cg.sh clippy -p gz-fmt-level -p gz-lc-light -p gz-lc`
  and `fmt --check`, the tests of the three crates, `python tools/qa/diff_compile.py --all` (with
  the new scenes and the determinism checks), the five l03u hashes. The lead runs the rest.

## Report

Under 400 words: what made the compile differ from run to run and what it is now (the hashes);
the BVH's other callers; the flat triangles and mp_test's texel; the two new scenes (what was
set, the C++ time in CPU seconds, the result); the bounds as they are now and what set them;
what stays unproven, phase by phase; the three smaller things; the proofs with their times;
anything still open and exactly why.

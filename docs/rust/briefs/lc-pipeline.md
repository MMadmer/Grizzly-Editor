# Brief: phase 8b - the level compiler (`gz-lc`), compared with the C++ one

`crates/gz-lc-light` holds the compiler's data model and its light (STATE.md, "Phase 8a": read
it first - it lists the public API and the C++ call each part replaces, and what waits for a
comparison). This pass ports `Source/Editors/XrLC` (14k lines) on top of it as a new crate,
`crates/gz-lc`, adds the detail compiler (`xrDO_Light`'s pipeline), and PROVES both against
what the C++ compilers write. Read `docs/RUST_MIGRATION.md`, "Phase 8 - The compilers" (the
design and the gate) and section 9's comparison rules (the style a rule has). You are alone in
the tree.

You may edit `crates/`, `Cargo.toml`, `tools/qa/`, `tools/rust/`, `docs/rust/STATE.md`.
`Source/` is the specification: read it, never edit it. The plan and `CLAUDE.md` are the
lead's.

## 1. The pipeline

- **`build.prj` in** (`Build_Load.cpp`): the reader of what `gz-levelbuild` writes - one
  format, so the writer's structures are shared or the reader round-trips the writer's bytes
  under a test.
- **The phases of `CBuild::Run`, in its order**, each a function over the model that takes a
  `Ctl` (cancel, progress) and runs data-parallel on the caller's pool: `PreOptimize`,
  `CorrectTJunctions`, `xrPhase_AdaptiveHT` (with its QSlim part), the normals,
  `BuildCForm` (with `xrOptimizeCFORM_qslim`), `BuildPortals`, `xrPhase_TangentBasis`
  (NVMeshMender - ported, not replaced), the ray model, the MU light, `ResolveMaterials`,
  `xrPhase_UVmap`, `xrPhase_Subdivide`, the light (`gz-lc-light`), `xrPhase_MergeLM` (already
  in `gz-lc-light`), `xrPhase_MergeGeometry`, `Flex2OGF` and the `OGF_Face*` family (the
  index ordering of NvTriStrip - ported, not replaced; progressive data through what
  `gz-fmt-ogf` already has), the MU models to OGF, `BuildSectors`, and the savers
  (`SaveLights`, `SaveTREE`, `SaveSectors`, the header, glows, portals, shaders, the lightmap
  pages as the C++ writes them).
- **The detail compiler**: `xrDO_Light`'s own pipeline (what it reads of a built level, the
  slots it lights through `gz-lc-light`, `level.details` out).
- **Options**: what a build can reach - the scene's build parameters inside `build.prj`
  (quality, pixels per metre, jitter, RMS, weld, smooth angle) and the flags the editor or the
  repository's launchers pass (`-nosun`, `-noise`, `-nosmg`, `-skipinvalid`, ...: list each
  flag of `xrLC.cpp` and say which stay). `-net` and `-gi` are refused with a sentence (D24).
- **How it runs**: a library call (`gz_lc::compile(input, options, ctl) -> report`) used by a
  job of the editor and by the command line (`Grizzly.exe -headless -compile <level folder>
  [flags]`, exit code 0 or 1, the log the C++ compiler prints phase by phase, with each phase's
  seconds). Wiring it into Play and the level build in place of `XrLC.exe` is the NEXT pass:
  here the compile stands on its own.
- Parallel on every core, the same bytes on any number of threads (a test on 1 and 8 threads),
  no `unsafe`, no global.

## 2. The proof against the C++ compilers

- **The oracle is recorded once.** The C++ compilers are `Bin\x64\CppEditor\XrLC.exe` and
  `xrDO_Light.exe` (a copy no later build touches); run from a stage with its own
  `fsgame.ltx` the way Play's `temp\compilers` stage does (STATE.md, "The reviewer's
  verification, closed"), `-silent`, under a hard limit of ten minutes a run. Scenes: the
  small one of the Play suite (`mp_test`, XrLC takes 11 s) and ONE middle-sized scene of
  `rawdata\levels` with sectors, lights and MU models that the C++ compiler finishes within
  the limit at the scene's own quality (say which and why). The `build.prj` both compilers
  read is the SAME file (made by the Rust editor's Make All in a copy of a project). The
  record (every output file with size and SHA-256, the files themselves, the C++ log with its
  phase times) goes under `Intermediate/oracle/compile-<scene>-<key>/`, keyed like the
  differential's records; a later run starts no C++ compiler when the record is there.
- **`tools/qa/diff_compile.py`** (new; default mode: only what failed or changed, like
  `diff_editors.py`) compiles the same `build.prj` with the Rust compiler and compares:
  - **the geometry half** - every chunk of `level`, `level.geom`, `level.geomx` if the C++
    writes it, `level.cform`, the sectors, portals, glows, lights and shader tables: byte for
    byte, or by ONE stated rule per chunk (a float's last bits with the bound and the reason;
    an order the C++ takes from pointers or threads) - never a broad ignore. What differs for
    another reason is a fault of the port: fix it;
  - **the light** - each lightmap page decoded and compared texel by texel, and the vertex
    colours: mean absolute error and the 99th percentile per channel, against bounds you
    propose from what you measure and can justify (the C++ result itself moves between runs
    when it is threaded - measure that first: it is the floor); a page layout that differs
    because deflectors are packed in another order is compared deflector by deflector, not
    page by page. It writes the pages of both sides and their difference as PNGs to look at,
    and you LOOK at them: a banding, a seam, a shadow that leaks or a dark border is a fault
    whatever the mean says;
  - **the details**: `level.details` slot by slot within the same kind of bound.
- **Speed and load**: the Rust compile's time phase by phase beside the C++ log's, and the
  process's CPU time over wall time during the light. A phase slower than the C++ one, or a
  light that leaves cores idle, is a finding: fix it. (This is where full load matters: a
  compile is minutes.)
- The game loading a Rust-compiled level is the next pass's proof (it wires the compiler into
  Play); leave `tools/qa/test_play_game.py` as it is.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Nothing that already passed is run again to check something new: your crates' tests and one
  scene of `diff_compile.py` while you work; at the end, once, `tools/rust/check.sh`,
  `tools/rust/test.sh`, `python tools/qa/diff_compile.py --all`. The editor's suites need no
  run: you change no editor behaviour.
- A C++ compiler is started only to record an oracle that is missing, in a stage under the
  system temp folder, and ended by its process id if it passes its limit. Projects are copied
  (`qa_target.copy_project`); the author's projects and the game are never written (`find`
  before and after, into STATE.md).
- Never run a git command that changes the working tree or the index.
- Keep `docs/rust/STATE.md` current: a section "Phase 8b: the compiler and its proof" - the
  rules of comparison (one sentence each), the bounds and what they were measured from, the
  phase times of both compilers, what was fixed in `gz-lc-light` on the way.

## Report

Under 450 words: what `gz-lc` holds (modules, lines, tests) and the flags kept; the two
scenes and the C++ times; the geometry half - chunks equal, chunks under a rule (each rule in
a few words), what differed and was fixed; the light - the measured error per scene against
the bounds and the C++ run-to-run floor, what the pictures showed; the details; the phase
times and the core load of both compilers; anything not ported or not proven and exactly why.

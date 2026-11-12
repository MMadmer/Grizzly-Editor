# Brief: phase 8c - the compiler inside the editor, and no C++ left

`crates/gz-lc` and `crates/gz-lc-light` are the level compilers in Rust, proven against the
C++ ones on three scenes (STATE.md, "Phase 8b: the compiler and its proof" - read it first,
with "Phase 8a" and "For the plan"). This pass closes the one gap of that proof, makes the
editor use the Rust compiler, proves it in the game, and removes the last C++ from the
repository (`docs/RUST_MIGRATION.md`, "Phase 8 - The compilers"). You are alone in the tree.

You may edit `crates/`, `Cargo.toml`, `tools/`, `Build.ps1`, `cmake/`, `CMakeLists.txt`,
`Source/` and `SDK/` (to delete them, section 3), `README.md`, `tools/mcp/AI_SETUP.md`,
`CLAUDE.md` (the build paragraphs only), `docs/rust/STATE.md`. The plan
(`docs/RUST_MIGRATION.md`) is the lead's. Section 3 is done LAST: until then the C++ sources
are the specification you read.

## 1. MU models against the C++ compiler

Multi-usage models (the `xrMU_Model*` family: their geometry, their OGFs, their light) have no
C++ reference: the C++ compiler did not finish the two smallest scenes that hold them
(`l03u_agr_underground`, `mp_close_combat`) within ten minutes at the scenes' own quality.
- Make it finish: both compilers read the SAME `build.prj`, so lower what costs the time and
  does not touch MU models in the copy's build options before Make All - the lightmaps'
  pixels per metre and jitter (and the quality, if it does not switch MU work off: check in
  the C++) - until the C++ run fits the ten minutes; say what you set. If no setting fits,
  ONE recording may take up to forty minutes: it is recorded once and never run again.
- Then `diff_compile.py` on that scene: MU geometry and OGFs chunk by chunk under the stated
  rules, MU vertex light within the bounds of the other scenes (or a stated one of its own),
  pictures of the lit models looked at if the tool can draw them. What differs is fixed in
  `gz-lc` / `gz-lc-light`.

## 2. The editor compiles with it

- **Play and the level build call the Rust compiler in process**, as a job: where
  `crates/gz-levelbuild` staged `XrLC.exe` / `xrDO_Light.exe` (`temp\compilers`, the
  `fsgame.ltx` rewrite, the `-silent` run), it calls `gz_lc::compile` / the detail compile on
  the CPU pool - cancelable, with its phases as the job's progress in the Task Center, the
  phase log in the Output Log, the result (warnings, the invalid faces the C++ lists, the
  seconds) in the answer and the Result dialog. The stage and its code go.
- **Over MCP**: the level build's commands answer as before with the compile's report added;
  a compile on its own is reachable too (the action or command the registry lacks, with its
  entry in `crates/gz-mcp/tools.json` - a description an agent can use). Stopping a compile
  is the job's stop.
- **Every core while it compiles**, the window alive: measure the editor's frame work during a
  compile of the middle scene (`frame_pacing`) and the process's CPU load; write both down.
  The drawing thread already runs above normal priority - nothing is made smaller.
- **In the game**: `tools/qa/test_play_game.py` passes with the Rust compiler doing the
  compile (the game creates the new game, loads the level's geometry and collision,
  synchronises - the same proof lines), and its header says so. Run it through
  `run_all.py test_play_game`.
- `test_scene_tools` / the build suites keep passing; add a check where a suite can hold the
  new path cheaply (a compile of the small scene over MCP ends `done`, its files are there,
  it can be stopped).

## 3. The last C++ leaves (after 1 and 2 are green)

- Delete `Source/`, `SDK/`, `cmake/`, `CMakeLists.txt`, the CMake presets and whatever else only
  the C++ build read (property sheets, `THIRD_PARTY` entries of removed code - keep the
  notices of what the Rust editor ships). `git ls-files` then holds no C or C++ source.
- `Build.ps1` builds the Rust editor and nothing else (its options that named the compilers
  go; it still says what it built and how long it took).
- **Nothing under `Bin\` is removed or overwritten** except the Rust binaries a build makes:
  `Bin\x64\CppEditor` (the kept C++ editor with its compilers) is the oracle the comparison
  tools record from while it exists - `diff_compile.py` and `diff_editors.py` keep working
  from their records and say plainly when a record is missing and no oracle binary is there.
- The editor no longer looks for `XrLC.exe` or `xrDO_Light.exe` anywhere (`rg`).
- Documents: `README.md`, `tools/mcp/AI_SETUP.md` and the build paragraphs of `CLAUDE.md` say
  what is true now (one build, no C++, where the comparison tools get their oracle); what the
  plan should say goes into STATE.md under "For the plan". References to deleted paths in
  what stays: none (`rg`), the migration's own records (briefs, `PARITY.md`, STATE.md) aside.
- `tools/qa/qa_target.py`: the C++ target for recordings points at `Bin\x64\CppEditor`.

## 4. Proof

Once, at the end: `tools/rust/check.sh`; `tools/rust/test.sh --all`;
`python tools/qa/diff_editors.py --all`; `python tools/qa/diff_compile.py --all`;
`python tools/qa/run_all.py --with-game` against a RELEASE build made by the new `Build.ps1`
- all green; the repository's counts before and after (files, lines of C and C++: zero).

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index: delete with the file
  system, read history with `git show <commit>:<path>`. The lead commits.
- Delete only inside this repository and only what section 3 names; never under `Bin\`,
  `rawdata\`, `gamedata\`, `Intermediate\oracle`; a recursive delete never goes through a link.
- The author's projects (`D:\XFinedProjects\*`) are only ever a source to copy from; the game
  under `D:\Games\Dead Air` is read only (the in-game suite runs in its isolated root);
  `find` over both before your first run and after your last shows nothing changed.
- Nothing that already passed is run again to check something new; every long run writes to a
  file under `Intermediate/` that you look at often. A C++ compiler is started only to record
  the missing MU oracle, from a stage under the system temp folder, and ended by its process
  id past its limit.
- Keep `docs/rust/STATE.md` current: a section "Phase 8c: the compiler in the editor, no C++".

## Report

Under 450 words: the MU proof (the scene, what was lowered, the C++ time, what differed and
was fixed, the bounds); how the editor compiles now (the job, the commands, the frame work and
the core load during a compile); the in-game result with the Rust compiler; what was deleted
(files, lines) and that no C or C++ source is tracked; the five proofs with their times;
anything still open and exactly why.

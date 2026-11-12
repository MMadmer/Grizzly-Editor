# Brief: the final review

The move of Grizzly Editor to Rust is claimed done: phases 0 to 8 of `docs/RUST_MIGRATION.md`,
branch `grizzly-rust`. The editor was reviewed before and at the cutover (phases 0 to 7; the
findings and their fixes are in `docs/rust/STATE.md`, "The reviewer's findings, closed", "The
reviewer's verification, closed", "The cutover"). You are the one independent review of what
came after and of the end state as a whole. You wrote none of it; trust nothing you have not
checked. **You change no file** - you find what is wrong and say exactly where.

## What the work claims

- `docs/RUST_MIGRATION.md`: "Status: done". The level compilers are the crates `gz-lc-light`
  and `gz-lc` (phase 8, section 11), proven against the C++ compilers under the rules of
  section 9 ("equal to the C++ compilers"); the deliberate differences are section 12 (D24,
  D25 and what D1 and D14 gained; "a difference that is not here is a bug").
- `docs/rust/STATE.md`: "Phase 8a", "Phase 8b", "Phase 8c: the compiler in the editor, no C++",
  "Phase 8c, the cleanup", "After phase 8c: the window while a job runs" - what was built,
  measured and left open, as the authors tell it.
- The repository holds no C or C++; the C++ is the tags `cpp-editor-last` and
  `cpp-compilers-last` (`git show <tag>:<path>` - the specification you compare against). The
  kept binaries: `Bin\x64\CppEditor`, `Bin\x64\CppCompilers`; their records: `Intermediate\oracle`.
- The project's rules: `CLAUDE.md`. Commits since the last review: `git log 3bb75b0c..HEAD`.

## What to check - go for what would hurt, in this order

1. **The five proofs are true, on the release build.** `pwsh -NoProfile -File .\Build.ps1` made
   `Bin\x64\Release\Grizzly.exe`; with `GRIZZLY_QA_EXE` and `GRIZZLY_LC_EXE` naming it run, once
   each: `bash tools/rust/check.sh`, `bash tools/rust/test.sh --all`, `python
   tools/qa/diff_editors.py --all`, `python tools/qa/diff_compile.py --all`, `python
   tools/qa/run_all.py --with-game`. Then read HOW the compile comparison passes
   (`tools/qa/compare_compile.py`, `diff_compile.py`): is every rule a real non-difference or a
   hidden one? The light bounds (3 / 16, 2 / 16, 0.5 / 2) against what was seen; `level.cform`
   "within the C++ runs' own spread by half again"; the MU references "reported, not judged" -
   is the reason true (read `xrMU_Model_Calc_lighting.cpp`, `xrMU_Model*.cpp`, the MU threads
   at the tag), and is the Rust light of those references right by itself (the pictures under
   `Intermediate/compile/*/pictures`, a check of your own)? Are the records really the C++
   compilers' answers: re-record ONE scene (`mp_test`, twenty seconds: `--scene mp_test
   --refresh-oracle`; the compilers show no window under `-silent`) and compare with the
   record. What the l03u scene's lowered options and `-noise` leave unproven.
2. **The compiler is right where no scene looked.** The promise "the same result on any
   number of cores" (phase 8): compile one scene on one thread and on all and compare the
   output byte for byte. A compile stopped half way: what is left in the level folder, can the
   next one run. A `build.prj` cut short or a missing texture: an error with a sentence, never
   a panic that takes the editor. Two compiles asked at once. Memory on the middle scene.
   Sample the port against the C++ where the comparison is weakest (what the rules leave out,
   implicit light, the detail light, portals and sectors, glows, the LOD textures).
3. **The game.** `test_play_game` as it stands proves what it says (read it); the installed
   game is untouched by it (its listing check - is it complete?); Play's root and the writer
   guard for everything the compile and Play write (`gz_editor::fsx::WriteGuard`).
4. **No C++, and nothing lost with it.** `git ls-files` holds no C or C++ source or build file;
   nothing that stays reads a deleted path; `Build.ps1` from a clean target folder; the notices
   in `licenses/` match what the binaries carry (`cargo tree`; the LZO port is GPL v2+ - is
   that said, and is it compatible with how the editor is given out?); the icon in both
   executables (`tools/rust/check_icon.py`, and look at it once); `tools/rust/
   check_test_files.py` - can a test get a program file past it (a name built with `format!`,
   an extension from a variable, a copy)? Is any program still copied to be run, besides the
   game's engine into Play's root (stated in D1)?
5. **The architecture rules hold in the new code** (sections 5 and 6): `unsafe` nowhere new, no
   global state, no lock held across a rayon scope the UI can wait on, the UI thread never
   waits on the compile; the scene pass that is built only when its picture can change - find a
   change that leaves a stale picture (a scene edit over MCP, a texture that finishes loading,
   the theme, a resize, a tab switch back, `screenshot_viewport` right after an edit).
6. **Nothing was quietly left out**: stubs, a command that answers `ok` and does nothing, code
   of the removed compilers' stage or of xrAI that stayed, "still open" lines of STATE.md that
   are not in the plan; menu rows and catalogue entries (`crates/gz-mcp/tools.json`) that
   promise what the code does not do.
7. **The documents tell the truth**: `docs/RUST_MIGRATION.md` (status, I5, sections 6 - 9 and
   12, phase 8), `CLAUDE.md` ("How the editor is built", "Build and verification"), `README.md`,
   `tools/mcp/AI_SETUP.md`.

## How to work

- You are one reviewer: do not start agents. Search first (`rg`), read `gz-lc-light` and
  `gz-lc` where the searches and the comparisons point, and the whole of
  `crates/gz-levelbuild/src/compile.rs`, `play.rs`, `tools/qa/compare_compile.py`.
- Change nothing: no edits, no git command that alters the tree or the index, nothing pushed,
  published, submitted or scanned. The author's projects (`D:\XFinedProjects`) are only a source
  to copy from (the tools do that); the game (`D:\Games\Dead Air`) is only read; an editor you
  start runs on a port and a home of its own and is closed through MCP; an editor on port 28016
  is the author's and is never touched. Scratch files go under `Intermediate/critic-final/`.
- Every cargo command through `bash tools/rust/cg.sh`. Long commands write to a file and are
  looked at often; nothing that passed is run twice.
- Every finding must be checkable: file and line, what is wrong, how you know (the command you
  ran, the C++ place at the tag that says otherwise), and how bad it is. No style notes, no
  praise.

## Report

Findings ranked by harm: **Blocking** (a level the game would read wrong, lost data, a write
into the game or an author's project, a claim of the proofs that is false), **Serious** (a
wrong or missing behaviour with no line in section 12, a rule of the architecture broken, a
document that says what is not so), **Minor**. For each: `file:line`, one sentence of what is
wrong, the evidence, the fix you would make. Then one paragraph: is the move to Rust done as
the plan says, and if not, the shortest list of what stands between. Under 900 words.

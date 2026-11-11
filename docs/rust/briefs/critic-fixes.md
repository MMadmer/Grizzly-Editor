# Brief: the reviewer's findings

An independent reviewer read the whole work (phase A: reading only). This pass fixes what it
found. You are alone in the tree. Read `docs/rust/CONVENTIONS.md`; in `docs/rust/STATE.md` the
sections "The differential run", "The QA suites", "What the QA pass left, closed"; sections 2,
5, 6, 7, 9 and 10 of `docs/RUST_MIGRATION.md`.

You may edit `crates/`, `tools/qa/`, `tools/rust/`, `tools/mcp/`, `docs/rust/`. `Source/` (the
C++ editor: the specification) and the game under `D:\Games\Dead Air` are read-only. The plan
(`docs/RUST_MIGRATION.md`) and `CLAUDE.md` are the lead's: where a finding is "the document
does not say what was built", write the true sentence into STATE.md under "For the plan" and
the lead folds it.

## 1. Blocking: the level exports and the build were never really compared

`tools/qa/diff_editors.py` counted two identical refusals as "same". In `s6_scene` the steps
`mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe` and `mod_export` ran with nothing
selected and two Player Starts: both editors refused, nothing was compared. `s5_builds`
answers `"baked": ""`. `quest_write`, `quest_rename_task`, a file's `content_move` and
`content_delete` are refused on both sides too. So section 9's rows for `overlay.xcform`, the
cuts, the `.ogf` overlay with `overlay_visuals.ltx`, the bake inside a build and the level
build's project have no proof that ran.

- **A step that is meant to succeed and answers `ok:false` on either side is a failure**: every
  step says what it expects (`ok` by default, `refused` where the refusal is the point), and
  the script fails the scenario otherwise - also while recording the oracle.
- Repair the scenarios so that the work really happens: objects selected before the exports,
  one Player Start, a quest text the reader takes, files that exist for the content steps.
  Compare the outputs as section 9 states them: `.xspawn`, `spawn\custom_data\*.ltx`,
  `overlay_visuals.ltx` byte for byte; `overlay.xcform`, the cut and the exported `.ogf` byte for
  byte or within the stated tolerance, on a scene with real geometry (Escape of the Test
  project), not a nine-object one; the build with a baked scene layer inside.
- **The level build** (`gz-levelbuild`): record what the C++ `COMMAND_BUILD` family writes for
  one real scene (`build.prj`, the AI map and game graph fragments, `level.spawn` parts, the
  details and the HOM / SOM files it writes itself - not what the external compilers make) and
  compare the Rust `level_build` output with it file by file. Both editors write under
  `<editor>\gamedata\levels\<level>`: run them one after the other and move each result out
  (or give the Rust build an output root for QA) - never both into one folder, and never into
  the game.
- Record the oracle again for every scenario you change (nothing answers on 28016 unless the
  author's own editor is up - then stop and say so). The oracle's key takes every binary the
  C++ editor loads its logic from (`XrECore.dll` and the other DLLs beside `LevelEditor.exe`),
  not the exe alone.
- **The game's suites named in section 9's proof column** that have not run on Rust output:
  the game repository's `tools/qa/Run-ModsQa.ps1`, `tools/qa/nq/Run-NqQa.ps1 -Scenario Scout`,
  `tools/qa/screens/Run-ScreensQa.ps1`. Run each once on a module or a release the Rust editor
  made, each under its own hard limit of three minutes, the way the QA pass ran `PdaModule`
  (an isolated QA root, the suite's own QA module ids, removed after; a module the user has
  installed is never replaced or removed; the game repository is never edited). A suite that
  cannot take Rust-built input without a change to the game repository is not forced: say
  exactly which and why.
- `crates/gz-export-level` gets tests against recorded C++ output (small golden files under
  `crates/gz-export-level/tests/data`, made by the oracle run), not a box against itself.

## 2. Serious: a level folder is wiped by an unchecked path

`crates/gz-levelbuild/src/cmd.rs` joins the scene's Level path (free text from Level Settings)
onto `gamedata\levels` and calls `wipe_tree` on the result. `PathBuf::join` with an absolute
value replaces the base, and `..` escapes it.

- The level path is ONE file-name component (no separators, no `..`, no drive, no reserved
  name); anything else is refused by the build with a sentence, and by the property editor
  when it is typed. Before any wipe the real path (final path, links resolved) must lie under
  `<editor>\gamedata\levels` and must not be a link - the same kind of guard the clean build
  of a module has (`gz-build`).
- `grizzly_level_build` carries `destructiveHint: true` in `crates/gz-mcp/tools.json`; a tool
  that loses data when given `discard` (`grizzly_level_parts`) follows the project's pattern:
  the discarding form is a tool of its own with the hint, the plain form refuses with
  `unsaved`. Check the whole catalogue for the same fault once.
- Tests for both guards.

## 3. Serious: no QA run may open a real project

Already in work by the pass before yours (STATE.md says what it did): every suite opens a copy
under the system temp folder, a guard refuses anything else before an editor starts,
`run_all.py` ends a timed-out suite's whole process tree. Verify it holds - run nothing until
you have read the guard - and extend it to `diff_editors.py` and every script under
`tools/mcp`. Before and after your own runs: no file under `D:\XFinedProjects\*` or
`D:\Games\Dead Air\modules` changes (`find -mmin`), written into STATE.md.

## 4. Serious: the UI thread waits on the GPU

`crates/gz-app/src/window.rs`: `device.poll(Wait, 2 s)` on the UI thread (a readback) and
`block_on` in two places.
- A readback (screenshots, thumbnails, `preview.png`) never blocks the UI thread: map
  asynchronously and collect on a later frame (the frame-held request already waits for
  frames) or on a worker.
- `block_on` on the UI thread is allowed once, for the device before the first frame; any
  other is removed. Say in STATE.md "For the plan" what the UI thread does: it draws and
  presents, and waits on no file, no network, no lock a job may hold and no GPU readback.

## 5. Serious: locks held across parallel work in the viewport

`crates/gz-ui-viewport/src/draw_particles.rs` holds `parts.state` across `par_iter_mut` and
calls into `gz-particles` and the scene collider; `draw_extra.rs` does the same for the AI map,
details and wallmarks. The deadlock the stress test found was gated (one frame at a time), not
removed: any other CPU-pool job that takes those locks hangs the pool. Copy out under the lock,
compute without it, put the result back under the lock; no lock is held across a rayon call,
an `.await` or a call into another crate anywhere in the viewport crates. A stress test that
builds frames from several threads while a job touches the same state proves it.

## 6. The smaller findings (all of them)

- `crates/gz-editor/src/frames.rs`: a frame-held request must be answered by a frame that
  STARTED after the request's commands were applied; a frame already in flight does not count.
  A test.
- `tools/qa/test_unicode_names.py`: its last check passes on a constant (`startup.rs` always
  logs "file names: UTF-8"). Make the check mean something (the name on disk read back through
  the listing) or drop it for the Rust target with a line in STATE.md's exception list.
- `crates/gz-publish/src/job.rs`: a second `start` racing the first resets the running job's
  view - one job, the second refused or joined, under a test.
- `crates/gz-mcp/src/lib.rs`: the request timeout drops a command's future midway. A command
  that changes state runs to its end whatever the connection does; the caller gets a timeout
  answer, the command finishes (or is cancelled only at a point it declares).
- `crates/gz-query/src/lib.rs`: a `compute` that panics is remembered as failed for that
  revision instead of running again every frame; `fresh` either cancels as section 5.4 says or
  the true behaviour goes to "For the plan".
- `crates/gz-win/src/lib.rs`: `std::process::exit` inside the exception filter - leave the
  process the way an exception filter may (write the report, then terminate without running
  Rust destructors on a broken state).
- The corpus's strides (every 16th `.object`, 8th `.thm`, 4th `.ogg`): `tools/rust/test.sh
  --all` reads EVERY file (an environment switch the script sets); the default run keeps the
  strides. Keep `--all` under a minute.
- Layering: `gz-game` depends on `gz-editor` and `gz-project`. Say with `cargo tree` what
  really depends on what, remove a dependency that points upward where it is cheap (a hook or
  a trait in the lower crate), and write the true layer table into "For the plan".
- `docs/rust/PARITY.md`: its tables still show 52 `missing` rows and a stale stub table while
  its list says closed. Make the tables and the list agree with the code (a row is `same`,
  `differs` with its D-number, or `missing` - and after this pass none is `missing`).

## How to work

- Fully autonomous; never ask; never start agents of your own.
- The user's rule on time: nothing that already passed is run again to check something new.
  One scenario, one suite, one phase, one crate's tests while you fix; the four entry points
  once at the end (`tools/rust/check.sh`, `tools/rust/test.sh --all`,
  `python tools/qa/diff_editors.py --all`, `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`).
- Never run a git command that changes the working tree or the index.
- Nothing is published, submitted, scanned or pushed; the mock only.
- Keep `docs/rust/STATE.md` current: a section "The reviewer's findings, closed" (one line per
  finding: what was done, where the test is) and "For the plan".

## Report

Under 450 words: each finding - fixed or not, with its test; the differential's new counts
(steps, the files really compared for each level export, differences found by the repaired
scenarios and what they were); the game suites' verdicts; the four entry points' results and
times; anything still open and exactly why.

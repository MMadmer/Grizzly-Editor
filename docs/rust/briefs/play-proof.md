# Brief: the last proof - Play, and a level with everything in it

The reviewer verified the fixes of its findings: nothing blocking is left, the four entry points
are green, the comparison of the level exports and of `Make All` is real. One serious gap stands
before phase 6's gate ("a level built here loads in the game"), and a short list of small things.
You are alone in the tree. Read `docs/rust/CONVENTIONS.md`; in `docs/rust/STATE.md` "The
reviewer's findings, closed" and "For the plan"; section 9 of `docs/RUST_MIGRATION.md` (the
comparison rules) and its D1 (Play starts the game).

You may edit `crates/`, `tools/qa/`, `tools/rust/`, `tools/mcp/`, `docs/rust/`. `Source/` (the
C++ editor: the specification), the game under `D:\Games\Dead Air` and the plan
(`docs/RUST_MIGRATION.md`, `CLAUDE.md` - the lead's) are read-only.

## 1. The Play half of the level build

`level_build build_cform | build_aimap | build_game_graph | compile_spawn | play` appear in no
scenario and no suite; `s7_levelbuild` is `make_all` alone. The game graph, the cross table and
`editor.spawn` are not equal to the C++ editor's by construction, and no game has loaded them.

- **Compared.** A scenario of `tools/qa/diff_editors.py` runs, on both editors and the same
  scene, every step of the Play path that the C++ editor also has (its command table rows of the
  Build menu - `Source/Editors/LevelEditor`: the builder and `UI_LevelMain.cpp`), and compares
  every file each step writes: byte for byte, or by ONE justified rule per file added to the
  list in STATE.md "For the plan" (a GUID, a time stamp, an order taken from pointers - never a
  broad ignore). What differs for another reason is a fault of the Rust side: fix it.
- **Loaded by the game, once.** The Rust editor's Play (D1) on a small scene ends with the game
  having loaded the level: the game's log says so (the level's name and the actor spawned), no
  assert, no crash. Under a hard limit of three minutes, in an isolated QA root the way the
  QA pass ran `PdaModule` (STATE.md, "The game as the judge"): the user's installed modules,
  saves and settings are never touched, the game repository is never edited, everything the run
  made is removed after. If Play as built cannot be isolated that way, give it what it needs on
  the editor's side (a QA root for its staging) - not a change of the game.
- If a step exists only in the Rust editor (no C++ counterpart to compare), the in-game load is
  its proof: say which steps those are.

## 2. A level that has everything

`s7`'s Escape has no sectors, portals, detail objects or wallmarks, so `build.details`,
`build_details.dds`, `level.wallmarks` and the sector and portal tables of `build.prj` were
written by neither editor. Find, among the repository's scenes (`rawdata\levels`, 35 of them),
the SMALLEST one whose parts hold sectors, portals, detail objects and wallmarks (read the
parts; say which scene and why), put it into a temp project, and run the level-build
comparison on it as a second scenario. A run of either editor has a hard limit of five
minutes; if no scene has all four within that, take two scenes that cover them between them.
The rules of section 9 apply; a new rule needs its sentence.

## 3. The small list

- **The start-up check cannot see a shown window**: `crates/gz-app/src/window.rs` logs the
  `window shown` milestone in a minimised start too, and every suite starts minimised. The
  profile says what happened (`window shown` or `window minimised`), and the 0.3 s check of
  `test_ui_shell` uses one start that really shows the window without taking the foreground.
- **The first picture after a scene load is current but slow**: one `screenshot_editor` after an
  Escape load takes 4.3-4.6 s; the same view was complete about 1.4 s after the load before the
  wait was added, and the suites that take pictures slowed with it (`test_ui_tour` 56 s, was 29;
  `test_properties` 16 s, was 6). Find what it waits for that the pictured view does not need
  (assets outside the view? a fixed number of frames? a poll interval?) and wait for exactly
  what the picture needs. Targets: under 1.5 s on Escape, `test_ui_tour` under 35 s,
  `test_properties` under 8 s - the picture staying current (`test_scene_tools` S).
- `crates/gz-export-level/tests/golden.rs`: a debug file is written into the temp folder on
  every run - take it out.
- `crates/gz-ui-viewport/src/draw_extra.rs`: a cache is emptied while a pass draws, so two
  passes at once each rebuild it from nothing. One rebuild per change (a version on the cache,
  the finished cache swapped in).
- `gz_base::ProjectPath` and `GamePath` exist and nothing uses them, while the plan says a game
  path cannot be written to. Make it true in one place: every writer of files (the project's
  file operations, the build, the exports, the release stage) goes through one guard that
  refuses a path under the linked game's root except the module's install target and the flat
  export's folder - using those types or removing them - with a test that a write under the
  game's `gamedata` is refused. Write into "For the plan" the sentence that is true.
- `run_all.py`: a suite ended by its timeout leaves its temp folder; the runner removes it.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Nothing that already passed is run again to check something new: one scenario, one suite,
  one phase, one crate's tests while you work; the four entry points once at the end
  (`tools/rust/check.sh`, `tools/rust/test.sh --all`, `python tools/qa/diff_editors.py --all`,
  `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`).
- The author's projects (`D:\XFinedProjects\*`) are only ever a source to copy from
  (`qa_target.copy_project`); before your first run and after your last, `find` over them and
  over `D:\Games\Dead Air\modules` shows nothing changed - both results into STATE.md.
- Never run a git command that changes the working tree or the index.
- Nothing is published, submitted, scanned or pushed; the mock only.
- Keep `docs/rust/STATE.md` current: a section "The reviewer's verification, closed", and the
  new rules and sentences under "For the plan".

## Report

Under 400 words: the Play scenario (steps, files compared, the rules added, what differed and
was fixed); the in-game load (the scene, the log lines that prove it, the seconds, what was
removed after); the second level-build scene (which, what it adds, the counts); each small item
- done or not; the four entry points' results and times; anything still open and exactly why.

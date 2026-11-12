# Brief: Play runs the installed game in place (the game, then the editor)

The move of the editor to Rust is done and reviewed; this pass closes what was left standing.
Play still COPIES the game's engine into a root of its own (57 files, 246 MB) and runs the copy,
because of two limits of the GAME. The author's word now: nothing stays open - lift them in the
game and make Play run the installed engine in place. You own this, in both repositories.

Read first: `docs/rust/STATE.md`, "Phase 8c: the compiler in the editor, no C++" (section 3,
"Play in the game") and "The final review, Play and the tests"; `docs/RUST_MIGRATION.md`, D1;
`CLAUDE.md`, "Dangerous operations" (Play's root) and "Where a mod exports to" (the writer
guard). In the game repository `D:\Games\Dead Air\DeadAir-x64`: `PROJECT_RULES.md` (all of it:
the build, the deploy of section 8, what must keep working), `docs/dead-air/`.

**Two other engineers work in the editor's tree at the same time**: one on the level compiler
(`crates/gz-lc`, `crates/gz-lc-light`, `crates/gz-fmt-level`, `tools/qa/diff_compile.py`,
`tools/qa/compare_compile.py`), one on the rest (`crates/gz-fmt-object`, `crates/gz-ltx`,
`crates/gz-mod`, `crates/gz-ui-app`, `tools/rust/*`, `docs/rust/PARITY.md`, `docs/UI_DESIGN.md`).
Your files in the editor: `crates/gz-levelbuild`, `crates/gz-game` (only what reads the game's
ability), `tools/qa/test_play_game.py`, `tools/qa/test_scene_tools.py`, `crates/gz-mcp/tools.json`
(the Play entries), and a section of your own appended to `docs/rust/STATE.md` ("Closing: Play
in place"). If a build fails in a file that is not yours, wait a minute and build again; never
edit it. In the game repository you are alone.

## 1. The game

What stops an installed engine from running on another fs config (STATE.md, as found in the
last pass - check it in the code, do not trust it):
- the content service works in the folder of `xrCore.dll` (`Core.ApplicationPath`,
  `ContentService.cpp`): every start's verification ends in `ContentState::SaveCache`, a
  truncating rewrite of `<application>\.dead-air-x64\content-state.txt`, and a level starts only
  while `$arch_dir$` is `<application>\database`;
- `CLocatorAPI::Register` copies every path it indexes into 256 bytes, so an fs root deeper
  than about 66 characters stops the game before its log opens.

Make it so:
- **A game started with `-fsltx <file>` writes nothing into its own folder.** What it keeps -
  the content state, and anything else you find it writes beside its programs - lives under the
  app data root that config names. The archives are taken from where the config's `$arch_dir$`
  points (the installed `database` when the config names it). A start WITHOUT `-fsltx` behaves
  exactly as today: the original Dead Air, old mods (XDB, loose gamedata, JSGME), content
  addons and saves do not notice. Nothing in the game names the editor: this is "the game on
  another fs config", an ability of the engine.
- **The path limit goes**: an fs root of any depth the system takes works (find every fixed
  buffer on that path - `Register`, the file-find callbacks, the shader cache's names - and say
  what the limit is afterwards).
- **The game says it can**: something a tool can read without starting it (the product version,
  or a line in a file the game ships) tells that this build takes another fs config in place.
- Build it (`PROJECT_RULES.md`), run the game's own checks that cover a start, add the line the
  rules ask for to its test matrix and documents (`docs/dead-air/`: where the fs config and the
  content state are described).
- **Prove before you deploy**, on a root that is NOT the installed game: the new binaries beside
  a junction to the archives in a folder of yours under the system temp folder (the layout
  Play's root has today), started with `-fsltx` naming another root - a listing of that
  "application" folder (path, size, write time) before and after is equal, the level loads. And
  a start without `-fsltx` there writes its content state where it always did.
- **Deploy** into `D:\Games\Dead Air` by the rules' section 8 - the game closed, the replaced
  files backed up into `_predeploy_<name>_<date>`, the whole `runtime-files.txt` list, SHA-256
  of every deployed file against `bin\x64\Release`. If `xrEngine.exe` of the installed game is
  running and you did not start it, it is the author playing: touch nothing, finish everything
  else, and say in the report that the deploy waits.
- You commit nothing in either repository; list the game files you changed (the lead commits
  them by path - the tree may hold other people's work, never touch a file you did not need).

## 2. The editor

- **Play starts the linked game's own `xrEngine.exe`** with `-fsltx <Play root>\fsgame.ltx` when
  the linked game says it can. The root then holds the marker, `fsgame.ltx`, the staged
  gamedata with the level, the app data - no program, no junction. `check_root`, the marker
  rules and the writer guard stay as they are; the installed game is never written by the
  editor, and now not by a game the editor started either.
- **A linked game that cannot** (an older build) keeps today's path - the engine copied into the
  root - behind the same checks; which path ran is in the answer and the log. No path is chosen
  by trying.
- **The depth check** refuses only what the linked game really cannot take.
- **Proof**: `tools/qa/test_play_game.py` - the level loads from the installed engine in place;
  the root holds no program file; a listing of the installed game before and after is equal
  (the suite has it: 107,624 entries) - on `mp_test` in the suite, and ONCE each on
  `l03u_agr_underground` (MU models, portals) and on `mp_close_combat` (terrain with implicit
  light AND MU models: the compiler's proof has no C++ record of that pair, so the game is its
  judge - add the scene to the suite's list); unit tests for the choice of the path and for
  the copy path that stays (no program file written by a test); `test_scene_tools`' Play
  phases.
- `crates/gz-mcp/tools.json` says what Play does now; for the plan: D1 as it should read.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes a working tree or an index, in either repository.
- Editor: every cargo command through `bash tools/rust/cg.sh <cargo args>`. Scripts longer than
  a line go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- A wait is 300 s at most, with the run's log read between waits; a measurement is taken once;
  nothing that passed is run again to check something new. The author watches the list of your
  background commands.
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from
  (`qa_target.copy_project`); never open one in place, no git command inside them. Before the
  deploy the installed game is only read. Every root you hand to Play is one you made under the
  system temp folder or Play's own default; a junction is unlinked, never followed.
- Every editor you start runs minimized or headless on a port and a home of its own and is closed
  over MCP; the game a suite starts is ended by the suite. An editor on port 28016 is the
  author's: never touch it. No test writes a program file.
- Nothing is pushed, published or sent anywhere.
- Proof at the end, once: the game's build and its checks; the editor's `bash tools/rust/check.sh`,
  `bash tools/rust/test.sh`, `python tools/qa/run_all.py test_scene_tools test_play_game`, the two
  one-off scenes; `find` over `D:\XFinedProjects` and `D:\Games\Dead Air\modules` shows nothing
  changed by you (the deploy's files aside - list them).

## Report

Under 450 words: what the game wrote beside its programs and where it goes now; the path limit
before and after; how the game says it can; the game's build, checks and deploy (files, the
SHA-256 result, the backup folder) or why the deploy waits; what Play does now and what its root
holds; the three levels in the game with their times; the proofs with their times; every file
you changed in the game repository; anything still open and exactly why.

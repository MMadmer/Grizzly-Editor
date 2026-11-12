# Brief: the final review's findings - Play's root, program files, the level in the game

The final review of the move to Rust (its scripts and outputs are under
`Intermediate/critic-final/`) found one way to lose an author's data and several holes around
Play and the tests. You close that half. Read `docs/rust/STATE.md`, "Play ends in the game" and
"Phase 8c: the compiler in the editor, no C++" (section 3), and `CLAUDE.md`, "Dangerous
operations", "The Start page" (the guards of `check_delete`) and "Where a mod exports to" (the
writer guard).

**Another engineer works in the same tree at the same time** on the level compiler
(`crates/gz-fmt-level`, `crates/gz-lc-light`, `crates/gz-lc`, `tools/qa/diff_compile.py`,
`tools/qa/compare_compile.py`). Your files: `crates/gz-levelbuild`, `tools/qa/test_play_game.py`,
`tools/qa/test_scene_tools.py`, `tools/rust/check_test_files.py`, `tools/rust/test.sh`,
`tools/rust/check.sh`, `crates/gz-mcp/tools.json`, and your own section of `docs/rust/STATE.md`
("The final review, Play and the tests": append it, touch no other section). If a build fails
in a file that is not yours, wait a minute and build again; never edit it. The machine is
shared: do not time anything while the other engineer compiles.

## 1. Play never destroys what it did not make (blocking)

> `crates/gz-levelbuild/src/play.rs:162-168`, `cmd.rs:136`, `cmd.rs:1055`. The root is only
> checked against the linked game, then `prepare` overwrites `fsgame.ltx`, wipes
> `<root>\gamedata` and copies the game's in. A root that is a project is not refused.
> `play_root_demo.py`: `level_build action=play_clean root=<temp folder with mod.ltx,
> gamedata\precious.txt, levels\my_level>` answered `ok:false ... too deep`, and afterwards both
> were gone, `fsgame.ltx` was the game's, and 76 `.exe`/`.dll` plus a `database` junction were
> in it.

- **One set of checks stands in front of every Play that names or makes a root**, before a byte
  is written or removed, and again inside `prepare` (the pattern of `gz-project`'s `delete.rs`
  `check_delete`: reuse its questions, do not write a second list): the root is not, does not
  hold and does not lie inside a project (`project.ltx` / `mod.ltx`, the open project and every
  listed one), the editor (the exe's folder, the data root), a game (the linked one, any listed
  project's `game_root`, an `[xms] export_target`), the user's profile folders, a drive root, a
  link; compared as written and where the path really is.
- **Play wipes only a root it made**: a marker file of Play's own, written when Play creates a
  root in an empty or missing folder; a folder that holds anything and no marker is refused
  with a sentence naming it ("... is not a Play root: choose an empty folder"), a root with the
  marker is the only thing `play_clean` and `prepare` may clear, and they clear only what Play
  writes there (list it), never "everything in the folder". The path-length refusal and every
  other refusal come before any change of the disk.
- The junction to the archives is unlinked, never followed, wherever a root is cleared (it is
  today: keep the test).
- **Only what the engine needs is copied**: the engine's executable and the libraries, not the
  game folder's other programs (the updater, the uninstaller, tools - 250 MB today). Say how
  the list is made (it must hold for a game build that adds a library).
- **Proof**: unit tests for each refusal (no program file written: `check_test_files.py`); in
  `test_scene_tools` or `test_play_game` a throw-away project folder as `root=`: refused, and a
  listing of the folder before and after is equal; the reviewer's `play_root_demo.py` rerun:
  nothing gone. `tools.json` says what `root=` takes.

## 2. No test writes a program file - held at run time too

> `tools/rust/check_test_files.py` is easy to pass. In a probe file only 1 of 7 writers was
> caught. A name from a non-test const, `with_extension(EXT)`, `format!` pieces, a copy of
> `current_exe()`, `"cmd"` (missing from `BARE`, line 22) and a helper outside `#[cfg(test)]`
> all pass. Fix: also scan what a test run leaves on disk.

- The source check stays (add `cmd`), and says in its header what it cannot see.
- **The run-time net**: `tools/rust/test.sh` gives the test binaries a temp folder of their own
  for the run (`TEMP` / `TMP`) and watches it while they run: any file created there with a
  program's extension, or whose first two bytes are `MZ`, fails the run, naming the file and -
  where it can - the test binary that ran then. Creation is caught even when the test deletes
  the file at once (the directory's change notifications, `ReadDirectoryChangesW` through
  python's `ctypes`, recursive; a polling scan alone misses a file that lives a millisecond).
  The watcher costs the run nothing measurable; say what it adds. A negative control proves it:
  a throw-away script that writes and deletes `x.exe` in the watched folder is reported (the
  control is a script under `Intermediate/`, not a test of the workspace).
- Tests that write outside the temp folder: find them (`rg` for `std::env::temp_dir`, `tempfile`,
  fixed paths) and say where each writes; none may write into the repository's tracked folders.

## 3. A real level in the game

> `test_play_game` proves a load of mp_test only (3 visuals, no portals or MU).

- `test_play_game.py` takes the scene as an option (the default stays `mp_test`, so
  `run_all.py --with-game` costs what it costs today). Run it ONCE on `l03u_agr_underground`
  (MU models, LODs, sectors and portals; the compile is about half a minute) with the same
  proof lines - the game creates the new game, loads the level's geometry and collision,
  synchronises, no assert, no dump - and write the result and its time into STATE.md. If the
  scene cannot start in the game for a reason that is the scene's (no AI map, no player
  start), make what Play needs the way the suite does for mp_test, or pick the next scene
  that has MU models and say why.
- The other engineer changes the compiler meanwhile (a ray test, the order files are written
  in): run this once near your end, on a build made then, and say which commit state it was
  (the lead reruns it on the final tree).

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index. The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`; scripts longer than a line
  go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from
  (`qa_target.copy_project`); never open one in place, no git command inside them. The game
  under `D:\Games\Dead Air` is read only; the suite's listing of it before and after stays
  equal. Every folder you hand to Play as a root is one you made under the system temp folder.
  `find` over `D:\XFinedProjects` and `D:\Games\Dead Air\modules` before your first run and
  after your last shows nothing changed.
- Every editor you start runs minimized or headless on a port and a home of its own and is
  closed over MCP; the game the suite starts is ended by the suite. An editor on port 28016 is
  the author's: never touch it. No C++ compiler is started. Nothing under `Bin\` is removed.
- Nothing that already passed is run again to check something new; every long run writes to a
  file that you look at often.
- Proof at the end, once: `bash tools/rust/check.sh`; `bash tools/rust/test.sh` (the changed
  binaries, under the new watcher) and the negative control; `python tools/qa/run_all.py
  test_scene_tools test_play_game`; the one l03u run. The lead runs the rest.

## Report

Under 350 words: the checks in front of Play's root and what Play clears now; what is copied
into a root (count and size before and after); the run-time net (what it watches, the control,
its cost) and where tests write; the l03u level in the game (time, what the log showed);
the proofs with their times; anything still open and exactly why.

# Brief: the last review's findings - the game's new ability and Play, held to the end

The last review (`docs/rust/briefs/critic-close.md`; its scripts under `Intermediate/critic-close/`)
read your change of the game (`D:\Games\Dead Air\DeadAir-x64`, `3816558ff`) and Play in place and
found no harm - and five loose ends. Close them; the rules of `docs/rust/briefs/close-play.md`
hold as they did (both repositories, the build, the proof on a folder of yours before the
deploy, the deploy by the game's rules, no commit by you).

**Another engineer works in the editor's tree at the same time** on the shell (`crates/grizzly`,
`crates/gz-win`, `crates/gz-app`, `crates/gz-ui-level`, `crates/gz-scene`, `crates/gz-entities`).
Your files in the editor: `crates/gz-levelbuild`, `crates/gz-game`, `tools/qa/test_play_game.py`,
the Play entries of `crates/gz-mcp/tools.json`, and a section of your own appended to
`docs/rust/STATE.md` ("The last review, the game and Play"). If a build fails in a file that is
not yours, wait a minute and build again; never edit it.

## The findings (the reviewer's words)

1. > Play, ability: read from `xrEngine.exe` alone, while `xrCore.dll` and `xrGame.dll` implement
   > it. A mixed install would run in place and rewrite `.dead-air-x64\content-state.txt`.

   The ability is stated by every program that implements it - `xrCore.dll` (the fs config
   elsewhere, the long paths) and `xrGame.dll` (the content service, the update check) carry
   the same words in their version resources - and the editor runs in place only when all
   that implement a word state it. A mixed installation takes the copy path, and says why.

2. > Play, path limit: 519 has a run behind it to 368 characters, and no quoted path was run.
   > My probe shows UCRT `_splitpath` returns empty parts for a directory of 256 or more
   > (`LocatorAPI.cpp:375`).

   Make the limit what the game says it is: find every place a path passes through a function
   with a limit of its own (`_splitpath`, `_makepath`, `_fullpath`, a `string256` left on the
   way) and replace it; then prove it - a root that makes the DIRECTORY part of indexed paths
   longer than 256 characters, the level loaded, and a config path with a space in it given
   quoted. `EngineAbilities`' "long-paths" stays only if the proof holds; the documents name the
   limit that was proven.

3. > Game docs: `src/xrCore/Debug/CrashReport.cpp:409` moves early reports for any `-fsltx`,
   > also one inside the installation; the commit message says such a start is unchanged.

   A start with a config inside the installation behaves as before here too.

4. > Play, copy path: unit tests on stand-in files only; `test_play_game.py:371` demands
   > `in_place`.

   The copy path is run for real once a suite run: Play takes an argument that asks for it
   (`engine=copied`: for a game whose in-place run the author does not want, and for this
   proof), `test_play_game.py` runs `mp_test` both ways, and the copy way keeps its proofs
   (the root's programs listed in the marker, the junction unlinked, the installed game's
   listing equal).

5. > `test_play_game.py:90` still says no implicit-light level was loaded by the game;
   > `Intermediate/final4/game_mcc.log` shows it was.

   Say what is true.

Also yours, from the other engineer's finding: the editor becomes a window program (its
subsystem changes to Windows), so a console program the editor starts would flash a console of
its own. Every `Command` in `crates/gz-levelbuild` that starts a console program (`taskkill`,
`cmd /c mklink`, anything else - not the game's engine, which is a window program) gets
`CREATE_NO_WINDOW` (`creation_flags(0x0800_0000)`, as `gz-app/src/gpu.rs` and
`gz-github/src/service.rs` do).

## How to work

As in `docs/rust/briefs/close-play.md`: fully autonomous; no git command that changes a working
tree or an index in either repository; the game proven on a folder of yours under the system
temp folder before the deploy; the deploy by the game's rules (the game closed, a backup named
for this change, the whole runtime list, SHA-256 against `bin\x64\Release`) - if `xrEngine.exe`
of the installed game is running and you did not start it, touch nothing of the installation
and say the deploy waits; `bash tools/rust/cg.sh` for cargo; a wait is 300 s at most with the
log read between waits; nothing run twice to check something new; the author's projects only
copied; an editor on port 28016 never touched; no test writes a program file; nothing pushed
or sent anywhere. Do not run git commands in the game repository while `test_play_game` runs
(the repository lies inside the installation the suite lists).

Proof at the end, once: the game's build and the checks its rules ask for; the proof of
point 2 on your folder; after the deploy `python tools/qa/run_all.py test_scene_tools
test_play_game` (mp_test both ways) and ONE run of `--scene l03u_agr_underground`; `bash
tools/rust/check.sh`, `bash tools/rust/test.sh`.

## Report

Under 350 words: where the ability is stated and read now, and what a mixed installation
does; every limited path function found and replaced, the proven limit and the run that
proved it, the quoted path; the crash report's condition; the copy path's real run; the
game's build, checks and deploy (files, SHA-256, the backup); every game file you changed;
the proofs with their times; anything still open and exactly why.

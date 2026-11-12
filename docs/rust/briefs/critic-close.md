# Brief: the last review - is anything still open

Grizzly Editor was moved to Rust, reviewed (you or a reviewer like you: `docs/rust/briefs/
critic-final.md`; the findings and their fixes are in `docs/rust/STATE.md`, "The final review
...", "The reviewer's verification of the fixes, closed") and then, at the author's word that
NOTHING may stay open, closed further. `docs/rust/STATE.md` now opens with "Open: nothing". You
are the one independent check of that sentence and of what was built to make it true. You wrote
none of it; trust nothing you have not checked. **You change no file** - you find what is wrong
or still open and say exactly where.

## What was done since the last review (commits `git log 765caeec..HEAD`)

- **The editor is MIT** (`LICENSE`, the workspace's license, beside the programs as
  `LICENSE.txt`); the game archives' LZO blocks are read by `lzokay-native` (MIT).
- **Play starts the installed game in place** (`docs/rust/briefs/close-play.md`; STATE.md,
  "Closing: Play in place"). The GAME changed for it - repository `D:\Games\Dead Air\DeadAir-x64`,
  commit `3816558ff`: an engine started with `-fsltx` naming a config outside its folder writes
  nothing into the installation, a path may be as long as the system takes, and `xrEngine.exe`
  says so in its version resource (`EngineAbilities`). It was built and deployed into
  `D:\Games\Dead Air` (backup `_predeploy_fsltx_20261113`). The editor's Play uses it; the copy
  path stays for an engine without the ability.
- **The compiler's proof without a gap** (`docs/rust/briefs/close-compiler.md`; STATE.md,
  "Closing: the compiler's proof"): every line of "not proven" recorded or proven another way;
  the C++ oracle may have been fixed where it crashed (the C++ working copy
  `D:\Games\XFined-Editor-cpp`, the tag `cpp-compilers-last`).
- **The old notes** (`docs/rust/briefs/close-sweep.md`; STATE.md, "Open: nothing" and "Closing:
  the old notes"): one LTX line model, the box selection's depth options, every command held by
  a test, the shared widgets in the kit, a command past the request timeout answering what is
  true, the tests' net over the repository.

## What to check - go for what would hurt, in this order

1. **The game still is the game.** Read the game commit (`git -C "D:\Games\Dead Air\DeadAir-x64"
   show 3816558ff`): can a start WITHOUT `-fsltx`, or with a config inside the installation,
   behave differently from before in any way (the content service, the update check, the crash
   reports, relative roots of the fs config, the quoted path, `Register`'s buffer, the long-path
   manifest)? Saves, old mods (XDB, loose gamedata, JSGME), the installer and the updater -
   anything of theirs that reads what moved? Is the deployed installation the build of that
   commit (SHA-256 of the installed programs against `bin\x64\Release`), is the backup whole?
   The installed game is only read by you; do not start it yourself unless through the editor's
   suite (`python tools/qa/run_all.py test_play_game`, which lists the installation before and
   after).
2. **Play.** In place: does anything of the run land in the installation (the suite's listing
   skips the game repository's `.git` - is anything else skipped that should not be)? The copy
   path for an older engine: still guarded and tested? Which path runs is decided by what the
   engine states - can a wrong answer there make Play write into the installation? The root's
   rules (the marker, a project inside, the checks of a project's deletion) after the rewrite.
3. **The compiler's proof.** Every line the plan's section 9 listed as not proven: recorded and
   passing, or proven how? Is any new rule a hidden difference (as `sun_copy_rule` was)? If the
   C++ oracle was changed: does the change alter a result (the records made before it), is it
   committed where the tag says? The bounds against what the logs show. What does the tool's
   header still call unproven - and is that the truth?
4. **"Open: nothing" is true.** Take the table at the top of STATE.md row by row and sample it
   hard: is the thing really done (run the test it names, read the code it names), or only
   reworded? Then look for what the sweep did not see: statements of open work anywhere in the
   tree (documents, comments, test skips, `expect="refused"` steps, `xfail`, suites that pass by
   skipping), commands that answer `ok` and do nothing, a `tools.json` entry or a menu row that
   promises what the code does not do, a document that describes what is not so
   (`docs/RUST_MIGRATION.md`, `CLAUDE.md`, `README.md`, `tools/mcp/AI_SETUP.md`,
   `docs/UI_DESIGN.md`).
5. **What the last changes broke.** The LTX model's merge (`gz-ltx`, `gz-mod`): a manifest or an
   `.ltx` that no longer round-trips, a write that moves a line; the job scope (`gz-task`,
   `gz-mcp`, `gz-app`'s UI queue); the box selection; the widgets moved into the kit (a surface
   that looks different now: `ui_audit`, a picture); the tests' net (does it fail a clean run, or
   miss the control).
6. **The rules still hold**: `unsafe` only in `gz-win`, no global state, the UI thread never
   waits, the writer guard in front of every writer, no test writes a program file, nothing
   copyleft-only linked (`python tools/rust/notices.py --check`, `cargo tree`), the editor's own
   license in the build's output.

## How to work

- You are one reviewer: do not start agents. Search first (`rg`), read where the searches and
  the failing checks point.
- Change nothing: no edits, no git command that alters a working tree or an index (in either
  repository, or the C++ working copy), nothing pushed, published, submitted or scanned. The
  author's projects (`D:\XFinedProjects`) are only a source the tools copy from; an editor you
  start runs on a port and a home of its own and is closed through MCP; an editor on port 28016
  is the author's and is never touched. Scratch files go under `Intermediate/critic-close/`.
- The lead's run on the final tree is under `Intermediate/final4/` (a summary and a log per
  step) and in STATE.md's last section: do not repeat it; run again what you doubt or what your
  own checks need - one scene, one scenario, one suite. Every cargo command through
  `bash tools/rust/cg.sh`. A wait is five minutes at most with the log read between waits.
- Every finding must be checkable: file and line, what is wrong, how you know (the command you
  ran and what it printed, the place in the C++ or the game that says otherwise), how bad it
  is, and the fix you would make. No style notes, no praise.

## Report

Findings ranked by harm: **Blocking** (the game or an author's project harmed, a level the
game reads wrong, lost data, a proof that is false), **Serious** (something still open or
wrong that the record calls closed, a rule broken, a document that says what is not so),
**Minor**. For each: `file:line`, one sentence of what is wrong, the evidence, the fix. Then:
is "Open: nothing" true - and if not, the complete list of what is open. Under 900 words.

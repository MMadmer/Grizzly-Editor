# Brief: nothing left open - the old notes, the ledger, the tests' net

The move of the editor to Rust is done and reviewed. What remains is whatever the long record
of the work (`docs/rust/STATE.md`, 3,100 lines) still calls not done, open or later, and was
never looked at again. The author's word now: nothing stays open. You go through every such
statement, check it against the tree as it is, and close it: do the work, or show with
evidence that the level editor never reached it and that the plan says so.

Read first: `docs/RUST_MIGRATION.md` sections 3, 4 (scope: what was dropped), 12 (deliberate
differences); `docs/rust/PARITY.md` (the ledger); `CLAUDE.md`.

**Two other engineers work in the tree at the same time**: one on Play (`crates/gz-levelbuild`,
`crates/gz-game`, `tools/qa/test_play_game.py`, `tools/qa/test_scene_tools.py`,
`crates/gz-mcp/tools.json`), one on the level compiler (`crates/gz-lc`, `crates/gz-lc-light`,
`crates/gz-fmt-level`, `tools/qa/diff_compile.py`, `tools/qa/compare_compile.py`). Everything
else is yours as far as this brief needs it, with a section of your own appended to
`docs/rust/STATE.md` ("Closing: the old notes"). **Base crates rebuild everybody**: when you
edit one that many crates depend on (`gz-ltx`, `gz-base`, `gz-chunk`, `gz-ui`), keep it compiling
after every edit (`bash tools/rust/cg.sh check -p <crate>`) and make such edits in one sitting,
not spread over the hour. If a build fails in a file that is not yours, wait a minute and build
again; never edit it.

## 1. The statements found so far - start here

- STATE.md, "Findings worth keeping", the object port: "Not done in `gz-fmt-object`: `.bones`
  files, `GenerateBoneShape`, `OptimizeMesh`, envelope `ScaleKeys` / `NormalizeKeys`." For each:
  who called it in the C++ editor (`git show cpp-editor-last:<path>`, `git grep` over the tag) -
  a path the LEVEL editor could reach (a menu, a command, a property, an import) or only
  `ActorEditor`, which section 4 dropped? What the level editor reached is ported, with its
  command and test; the rest gets its line in PARITY.md and the plan's section 4 if it is not
  there.
- The same section, the level port: "Not written: the compiled `level` file (`fsL_*`, sectors,
  glows), `EDetailManager::Export`'s texture merge." - written since (phase 8, the level
  build)? Say where, with the test or comparison that holds it.
- STATE.md, about line 464: "Still open from integration: `gz_ltx::LtxDoc` and `ManifestDoc`
  stay two documents (entries without `=`, raw line edits and headers without inheritance
  would all have to enter `LtxDoc`); the Start page's status bar shows no centre now". The
  rule for the first: ONE lossless line model of LTX text in the workspace. If `gz-mod`'s
  manifest document parses lines by itself, it goes onto `gz-ltx`'s model (which learns what it
  lacks), and the corpus round trip of every `mod.ltx` and `.ltx` stays byte for byte; if it
  already stands on `gz-ltx`'s lines and only adds the manifest's meaning, say so with the
  code. The second: what `docs/UI_DESIGN.md` says the Start page's status bar shows - make the
  page show it, or show that it does (`ui_list`, a picture).
- `crates/gz-app/src/gpu.rs:168`, "on mutes for now": a real "for now" or wording?

## 2. The sweep

- Every statement in `docs/rust/STATE.md`, `docs/rust/PARITY.md`, `docs/RUST_MIGRATION.md`,
  `docs/UI_DESIGN.md`, `docs/NQ_ARCHITECTURE.md`, `docs/pda/PDA_FORMAT.md`, `CLAUDE.md`,
  `README.md`, `tools/mcp/AI_SETUP.md` that something is not done, not written, open, left,
  later, for now, not yet, a next step, to be decided (search words and read around the hits;
  the briefs under `docs/rust/briefs/` are tasks, not state - skip them). For each: closed
  since (where: the commit's subject or the section that says so, and the test), or still true -
  then it is yours to do, unless it belongs to one of the two other engineers (Play in place and
  the game's path limit; the compiler's "not proven" list) - name those and leave them.
- The ledger: every row of PARITY.md is done or dropped with its reason; no row waits.
- The code: stubs and placeholders (`todo!`, `unimplemented!`, `TODO`, `FIXME`, "not yet",
  "for now", "later", an empty match arm that swallows a case); a command of the registry that
  answers `ok` and changes nothing (`Grizzly.exe -dump_commands` or `list_commands`: for each
  command, the test or suite that holds it - a command no test and no suite names gets a
  test); a menu row or a `tools.json` entry that promises what the code does not do.
- `docs/UI_DESIGN.md` section 8 lists widgets still local to a surface: one that two surfaces
  now carry a copy of moves into `gz-ui`.
- **The record itself**: at the end STATE.md opens with a short section, "Open: nothing", that
  lists every statement you found with one line on how it is closed (the old text stays below
  as the history it is, each closed statement marked "(closed: ...)" where it stands, so nobody
  reads it as open again). The "For the plan" section holds only what is not in the plan yet
  (the lead folds it); the list "Next packages, in dependency order" is history: say so in its
  heading.

## 3. The tests' net sees the repository too

`tools/rust/watch_programs.py` watches the test binaries' temp folder. A test that writes
elsewhere is seen only by the source check. Let the run also watch the repository's own tree
while the test binaries run (not `Intermediate\cargo`, where cargo writes its programs, and
not while cargo builds): a program's name or an `MZ` head made anywhere else under the
repository fails the run. Say what it costs; the negative control as before (a script under
`Intermediate/`, not a test of the workspace). The watcher's header keeps saying what it still
cannot see.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index (`git show`, `git grep`,
  `git log` read). The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`. Scripts longer than a line
  go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- A wait is 300 s at most, with the run's log read between waits; nothing that passed is run
  again to check something new; one crate, one suite while fixing. The author watches the list
  of your background commands.
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from; the game
  is only read. Every editor you start runs minimized or headless on a port and a home of its
  own and is closed over MCP; an editor on port 28016 is the author's: never touch it. No test
  writes a program file. Nothing is pushed or sent anywhere.
- Proof at the end, once: `bash tools/rust/check.sh`, `bash tools/rust/test.sh` (the changed
  binaries), the suites that cover what you changed through `python tools/qa/run_all.py
  <names>`, `python tools/qa/diff_editors.py` (the scenarios that changed since they passed).

## Report

Under 400 words: how many statements you found and how each group was closed (done now -
what; closed before - where; dropped - where it is said); what you ported or changed in the
code, with its tests; the LTX documents; the Start page; the commands without a test before
and after; the tests' net over the repository and its cost; the proofs with their times;
anything still open and exactly why.

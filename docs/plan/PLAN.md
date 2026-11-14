# Grizzly Editor 0.7 - the work plan

The editor moved to Rust carrying the X-Ray SDK's shape with it: its edit targets, its build menu, its scene files,
its command table, its 199-tool MCP surface. 0.7 is the editor on its own terms. This plan says what is built, in
what order, by whom, and what "done" means. The architect (the session that wrote it) owns every decision here,
reviews every result and makes the commits; engineers work from the briefs in `docs/plan/briefs/` under the rules
of `docs/plan/RULES.md` and report into `docs/plan/reports/`.

Surveys this plan stands on (facts, with `file:line`): `Intermediate/plan2/commands.md` (the command layer),
`assets.md` (formats, projects), `ux_rest.md` (shell, graphs, mod tabs), `ux_level.md` (the level editor), and
their screenshots under `Intermediate/plan2/shots/`.

## 1. What 0.7 is

1. **One command model, three ways in** (`docs/COMMANDS.md`): every action is a typed, self-describing command;
   the window, MCP and a shell run the same ones. The MCP surface is six tools instead of 199; `help` is generated
   from the code; a shell runs any command as a commandlet (`Grizzly-Cmd.exe run ...`), fully headless.
2. **Assets are text an agent edits directly** (`docs/ASSETS.md`): one syntax, versioned, tolerant, written back
   without touching what was not changed, followed live by the editor, checked from a shell. The scene leaves the
   X-Ray SDK format for the same syntax.
3. **Projects stand alone** (`docs/ASSETS.md` 8-9): a folder with a project file, no absolute path, opened by a
   double click, moved anywhere.
4. **The editor updates itself** (`docs/UPDATES.md`): one archive per release, ranged delta, resume, automatic
   continuation, transactional apply.
5. **Nothing blocks the UI thread**: every wait is a job with feedback; a stall monitor proves it.
6. **UI 3** (`docs/UI_DESIGN.md`, rewritten as the work goes): the level editor on a modern model (select anything,
   modes, a place palette, one build), a real node-graph editor (wire styles, readable nodes, a context picker,
   frames, reroutes, align, disable), one kit with one widget per job, every surface held to section 1a.
7. **No trace of the C++ editor**: no oracle, no differential tool, no parity test, no word of it in code or
   documents. Fewer, sharper tests.

Version: **0.7.0**. Nothing is published: no release, no push.

## 2. Principles the work is held to

- **Cut, do not wrap.** A legacy concept that the new model replaces is deleted with its commands, tests and
  documents - not kept beside the new one. The editor has not shipped: no migration of its own old names, no
  aliases. The one exception is the author's data: old project files, scenes and assets open and convert
  (`docs/ASSETS.md` 2, 7, 8).
- **The game is untouched unless a brief says so.** Its contract (`docs/ARCHITECTURE.md` "The contract with the
  game") stays; the two paired changes of this plan are named in their briefs (stream G).
- **A rename is finished by whoever makes it**: every caller - UI, QA suites, documents - in the same piece of work.
- **Proof is narrow**: your crates' tests, your suites, your phases. The architect runs the three entry points
  (`check.sh`, `test.sh`, `run_all.py`) at the end of each wave.
- **Tests earn their place.** A test that proves parity with something that no longer exists, repeats another test
  or checks a constant goes. A suite covers one area in under two minutes; every suite ends by asking
  `perf.stalls` and fails on a stall the suite did not expect.

## 3. Architecture decisions (the contracts)

| Area | Decision | Where |
|---|---|---|
| Commands | `Spec` + typed `Params` in `gz-editor` (`spec.rs`, `help.rs`, `registry.rs` - built); effects and gates; `Registry::add` / `add_sync` (bare names, untyped) exist only until every command is ported, then they are deleted | `docs/COMMANDS.md` |
| Actions | `gz-app` `actions.rs`: an action = command + arguments + title + icon + keys; menus, toolbars, shortcuts and the palette are lists of actions. `UiCommand`, the `COMMAND_*` table (`cpp.rs`), `exec_command`, `list_commands` go | COMMANDS 4 |
| MCP | six tools in the bridge; no `tools.json`; the safety paragraphs live in the bridge's source and its test | COMMANDS 4 |
| Commandlets | `grizzly` `commandlet.rs`: `run`, `script`, `help`, `serve` and the aliases | COMMANDS 4 |
| Jobs | one way for long work: wait in the command, `wait=false`, `job.*`; no `pending`, no `answer_only`, no bridge polling | COMMANDS 3 |
| Lossless documents | `gz-luadata` `LuaDoc`: syntax tree with trivia, `apply_diff(old, new)`, positions for problems | ASSETS 3 |
| Asset kinds | new crate `gz-asset`: the one list of kinds, versions, migrations, check, reference, templates | ASSETS 2, 6 |
| Watching | one project watcher; open documents rebase their unsaved edits on an outside change | ASSETS 4 |
| Scene | `*.scene` + folder of Lua data files; the SDK `.level` is read-only input; spawn properties named | ASSETS 7 |
| Project | `<Name>.grizzly`; machine state in the home by project id; default game found by itself; file association; a second instance hands over | ASSETS 8 |
| Library | copy on place; `library` list; `project.collect`; placeholders | ASSETS 9 |
| Updates | new crate `gz-update`; `Build.ps1 -Package`; the dev build has no updater | `docs/UPDATES.md` |
| Stalls | `gz-app` measures every UI pass and event; over 100 ms is a recorded stall with what ran; `perf.stalls` | section 6, stream K |
| Level editor | no edit targets: any visible, unlocked object is selectable; modes Select / Place / Paint / Zones; one Build with a pipeline | stream L briefs, UI_DESIGN |
| Graphs | flow direction left-to-right by default (top-to-bottom kept); wires from one polyline builder in six styles; `canvas = {}` for frames, reroutes, bookmarks; `collapsed`, `disabled` on nodes | stream G brief, NQ_FORMAT |
| Documents | `docs/ARCHITECTURE.md` replaces `RUST_MIGRATION.md`; `docs/STATE.md` (short, current) replaces `docs/rust/`; `tools/mcp/AI_SETUP.md` becomes a page; CLAUDE.md is rewritten at the end | stream Q |

## 4. Streams and waves

Each stream is one engineer (a model and effort chosen for the work) with one brief. Streams of one wave run side
by side on disjoint crates; a wave ends with the architect's review, the entry points and a commit per stream.

**Wave 0 - the architect (done before wave 1 starts):** C++ worktree, branches, tags, binaries and oracle
records removed; the command core (`gz-editor`); version 0.7.0; the contracts (`COMMANDS.md`, `ASSETS.md`,
`UPDATES.md`); then, beside wave 1: the socket's gates and batches, the commandlets, the six-tool bridge, the
`job.*` and `app.*` commands, the differential tools and their documents deleted.

**Wave 1 - foundations**

| Stream | Brief | Owns | Model |
|---|---|---|---|
| A1 Assets | `briefs/A1-assets.md` | `gz-luadata`, new `gz-asset`, the asset io of `gz-nq`, `gz-characters`, `gz-pda`, the watcher in `gz-project` | Opus 5.5, xhigh |
| A2 Project | `briefs/A2-project.md` | `gz-project` (except the watcher), `gz-game`, `gz-win` (registry, single instance), the project side of `gz-app` / `gz-ui-app` | Opus 5.5, high |
| U Updater | `briefs/U-updater.md` | new `gz-update`, `gz-ui-app/src/update.rs`, `Build.ps1 -Package`, `tools/rust/package.py`, the mock's release side, `tools/qa/test_update.py` | Opus 5.5, high |
| K Kit and shell | `briefs/K-kit-shell.md` | `gz-ui`, `gz-app`, `gz-ui-app` (preferences, keyboard), `gz-log` levels; every surface crate's call sites of a kit API it changes | Opus 5.5, xhigh |

**Wave 2 - areas**

| Stream | Brief | Owns | Model |
|---|---|---|---|
| L1a Scene | `briefs/L1a-scene.md` | `gz-scene` (model, selection, file format, properties), `gz-entities`; the `scene.*` commands; their suites | Opus 5.5, xhigh |
| L1b Build | `briefs/L1b-build.md` | `gz-levelbuild`, `gz-export-level`, the `gz-lc*` entry points, `gz-scene`'s bulk tools and zones; the `level.*` commands; their suites | Opus 5.5, xhigh |
| G Graphs | `briefs/G-graphs.md` | `gz-nq` (model, layout, validate, doc), `gz-ui-graph`; the game half (disabled nodes, catalog summaries) | Opus 5.5, xhigh |
| M Mod side | `briefs/M-mod-side.md` | `gz-mod`, `gz-build`, `gz-release`, `gz-publish`, `gz-catalog`, `gz-github`, `gz-screens`, `gz-world`, `gz-ui-mod`, `gz-ui-world`, `gz-ui-pda`, `gz-ui-release`; their commands and suites | Opus 5.5, high |

**Wave 3 - the level's face and the rest**

| Stream | Brief | Owns | Model |
|---|---|---|---|
| L2a Viewport | `briefs/L2a-viewport.md` | `gz-ui-viewport`, `gz-render`: the mode control, pills, gizmo, the drop target, brushes; `viewport.*` | Opus 5.5, high |
| L2b1 Place and Outliner | `briefs/L2b1-place-outliner.md` | `gz-ui-level`: the layout, the Place panel, the Outliner, the modes' panels | Opus 5.5, high |
| L2b2 Details and Build | `briefs/L2b2-details-build.md` | `gz-ui-level`: Details, the chooser, Build and Play, menus, the level's actions; the old command table deleted | Opus 5.5, high |
| S Start, content, previews | `briefs/S-start-content.md` | `gz-ui-app` (start page), `gz-ui-content`, `gz-ui-previews`; the PDA tab's two leftovers | Opus 5.5, high |

**Wave 3b - after the surfaces**

| Stream | Brief | Owns | Model |
|---|---|---|---|
| Q1 Suites and checks | `briefs/Q1-suites-checks.md` | `tools/qa`, the check scripts; the untyped registry API removed | Sonnet 5.5, high |
| Q2 Sweep and documents | `briefs/Q2-sweep-docs.md` | comments, names and messages in every crate; the documents; dead tracked data | Sonnet 5.5, high |

An engineer gets one or two large pieces of work, not five: a stream that grew past that was cut (L1 into two,
L2 into three, Q into two).

**Wave 4 - closing:** the critic (Fable 5.2) reads the whole diff against this plan and the contracts and is
resumed after each round of fixes until it says nothing is open; the three entry points on the release build; the
local build in `Bin\x64\Release`; `main` moved to the result; CLAUDE.md and the memory rewritten.

## 5. What each stream must leave true (acceptance, in short)

- **A1**: a file opened and saved unchanged is byte-identical; an edit changes only its own lines; unknown keys and
  comments survive every save path (tab, command); old versions migrate, newer ones are refused with the sentence;
  an outside edit shows in an open tab and unsaved edits rebase; `asset.check` and `asset.reference` answer for
  every kind with positions; the per-asset CRUD commands are gone.
- **A2**: a project folder copied to another path (and to a home that never saw it) opens by its project file, by
  its folder and by a double click, finds the game, and writes no absolute path anywhere inside itself;
  `project.ltx` is gone; a second editor hands over; `AGENTS.md` is written once; `project.collect` completes a
  project.
- **U**: `docs/UPDATES.md` section 7, every sentence, against the mock.
- **K**: one widget per job in the kit and no local copy of it left; menus, toolbars, shortcuts and the palette are
  actions over commands; `exec_command` and the `COMMAND_*` table are gone; `ui.*`, `app.*`, `shot.*`, `perf.*`
  are typed commands; the stall monitor records; the status line shows reports only; every defect of
  `ux_rest.md` B.1-B.3, B.11-B.13 is fixed or answered in the report.
- **L1** (L1a the scene, L1b the build): `.scene` round-trips every base scene of the editor's library (objects,
  spawn packets byte-exact through named properties, groups, the painted layers); the SDK writer is gone;
  selection has no target class; `scene.*` and `level.*` replace the 80 old names; the build is one pipeline
  command with saved state; visibility cells, the ground and the AI map's bounds need no hand-made step; nothing
  of it waits on the UI thread.
- **G**: the six wire styles from one builder; readable nodes at every zoom; the author's 110-node quest fits and
  reads; the picker filters by the pin and ranks; frames, reroutes, collapse, disable (the game passes a disabled
  node through), align / distribute / straighten / select connected / insert on wire / replace; graph actions are
  commands with rebindable keys.
- **M**: every mod-side surface passes section 1a and the defects of `ux_rest.md` B.5-B.10; their commands are
  typed (`mod.*`, `release.*`, `screens.*`, `world.*`, `characters.*`, `pda.*`); no bridge-side polling is needed;
  a project's jobs stop when it closes.
- **L2** (L2a the viewport, L2b the panels): the level tab on the new model, every finding of `ux_level.md` B, C
  and D fixed or answered; the old command table gone.
- **S**: the Start page, the Content Browser (drag into the viewport places), the previews and libraries polished
  to the same standard; their commands typed.
- **Q**: three entry points; suites by area, each under two minutes, each ending on `perf.stalls`; no `add` /
  `add_sync` left in the registry API; no mention of C++, an oracle or parity in the tree; the documents current.

## 6. The stall monitor (the proof of "nothing blocks")

`gz-app`'s event loop times every UI pass and every event handler. One that takes over 100 ms is a **stall**: logged
(`[stall] 412 ms in <tab>: <what ran>`), counted in the status bar's problem counts while a debug flag is on, and
kept for `perf.stalls` (`{stalls: [{ms, when, tab, during}], worst_ms}`; `clear=true` empties it). The first frame
of a start is not a stall. Every QA suite ends with `perf.stalls` and fails on any it did not declare; a stream
that finds one in its area fixes the cause - the work moves to a job with a card in the Task Center and a state on
the thing that waits - and never raises the threshold.

## 7. Order of commits

One commit per stream per wave, by the architect, after: the stream's report read, its diff read, its proofs run
again where they are cheap, the entry points green. A stream whose result does not meet its brief is sent back
with the exact gaps (the same engineer, resumed), not patched around.

# Brief: the QA suites against the Rust editor

The suites under `tools/qa` are what the editor's behaviour is held to. They were written
against the C++ editor; this pass makes every one of them pass against the Rust editor, by
fixing the editor. You are alone in the tree. Read `docs/rust/CONVENTIONS.md`, all of
`docs/rust/STATE.md`, `docs/RUST_MIGRATION.md` sections 10, 12 and 13, and `CLAUDE.md` (its QA
paragraphs say what each suite proves).

You may edit `crates/`, `tools/qa/`, `tools/mcp/`, `tools/rust/`, `docs/rust/`. `Source/` and
the game under `D:\Games\Dead Air` are read-only.

## The target

`tools/qa/qa_target.py` picks the editor: `GRIZZLY_QA_TARGET=rust` runs
`Intermediate\cargo\debug\Grizzly.exe` on port 28917 (`-mcp_port`). Every suite already takes
the binary, the port and the launch arguments from it. What is still tied to the C++ editor
in a suite and is yours to move behind `qa_target`:

- the files a suite restores byte for byte (`level.ini`, `level_imgui.ini` -> `qa_target.KEPT`);
- the log it reads (`qa_target.log_glob()`), and the lines it wants there;
- a precondition that is an ImGui artefact (a stale `level_imgui.ini` entry that puts a window
  below the editor, a platform window): the Rust shell has no such state, so for the Rust
  target the precondition is skipped with a comment and **the check behind it stays** (the
  dialog inside the picture, centred, more than a flat fill; the minimised editor pictured
  restored - those are requirements of `docs/RUST_MIGRATION.md` section 13 to prove again).

## The rule

**What a suite checks is not weakened.** A failing check means the Rust editor is wrong: fix
the crate that owns it, at the cause. The exceptions are exactly two, and each one is written
down in `docs/rust/STATE.md` under "What the suites could not hold the Rust editor to":
- a check of an ImGui artefact with no meaning in the new shell (the window title string of an
  ImGui window, an `.ini` key): the check is rewritten to the same requirement in the new
  shell's terms when there is one, dropped for the Rust target when there is none;
- a deliberate difference of section 12 (Play starts the game; the editor's own materials; new
  settings files; signature and packed bytes; no raw-pixel lint - `lint_ui_units.py` is not
  run).

## The suites (all of them, in this order)

Sources: `D:\XFinedProjects\DARF-Overnight-Delivery` (the reference mod) and
`D:\XFinedProjects\Test` (the Escape scene). The suites copy the project themselves; before a
suite that exports, check that the copy's `export_target` points away from the game (the
suites that need it do it - verify once, fix the suite if one does not).

`test_project_browser`, `test_ui_shell`, `test_ui_tour`, `test_asset_icons`,
`test_unicode_names`, `test_build_contents`, `test_build_job`, `test_characters`,
`test_behaviours`, `test_graph_ui`, `test_pda`, `test_screens`, `test_world`,
`test_properties`, `test_previews`, `test_mod_publish`, `test_mod_catalog` (the last two
against `tools/qa/github_mock` only, `-qa_github`), then `tools/mcp/test_server_lifecycle.py`
and `tools/mcp/benchmark_scene_load.py` (the Rust editor no slower than the C++ numbers it
records - run the C++ side only if nothing answers on 28016; never close an editor you did not
start).

Also `tools/qa/test_mcp_bridge.py` (new): the Rust bridge `grizzly-mcp` over stdio against a
headless editor - `initialize` carries the instructions, `tools/list` equals `tools.json`, a
call, an unknown argument refused, a `*_discard` tool sending `discard`, a screenshot as an
image content.

## Speed

- The Rust editor starts in well under a second, so the waits of the C++ era go: poll at
  50 ms, deadlines in seconds. A suite prints its wall time at the end.
- Budget: any suite under 60 s, all of them in sequence under 10 minutes. A suite over budget
  is profiled (the editor's log has the timings) and the slow thing is made fast - in the
  editor when it is the editor.
- `tools/qa/run_all.py` (new): runs every suite in sequence against the chosen target, each
  with a hard timeout, prints `PASS/FAIL <suite> <seconds> <failed checks>` and a total, exit
  code 1 on any failure. That is THE way the suites are run from now on.
- Run a suite, fix, re-run THAT suite. The whole set once at the end.

## The look that was skipped

Several packages were finished while no graphics device could be created on the machine, so
nobody has SEEN their work. `docs/rust/PLAN_DELTAS.md` ("To verify once a graphics device
exists") and the reports in `docs/rust/STATE.md` ("The AI map, details and wallmark tools",
"The scene's rest") list it. Before the suites, look at each item once through
`tools\rust\run-editor.ps1` on a temp copy of `D:\XFinedProjects\Test`, fix what is wrong, and
make sure of the exit path first of all: after a run that opened a floating panel and the
viewport, the editor's process is GONE within two seconds of `window_control close` - no
`Grizzly.exe` left in the process list. If a process ever stays, stop and fix that before
anything else: a process stuck in the GPU driver blocks every later run on the machine.

## A known gap to close first

**No window of the operating system opens for a caller who cannot click it.** The kit's
questions already answer for an MCP caller (`gz_ui::dialog::answer_as`, set by `exec_command`
around a table row) and under `-nodlg`. The system's file dialog does not: twelve places call
`rfd::FileDialog` directly (`rg "rfd::" crates`), so `exec_command` on a row that wants a file
and was given none (Open Scene..., Save As..., Export OBJ, Assign Macro, ...) opens a native
dialog nobody can answer, and a QA run under `-nodlg` can leave one on the author's screen.
Make it one kit function every caller uses (the title, filters, folder, save name -> the picked
path on a thread of its own, as now), which opens nothing and logs one line
(`~ [dlg suppressed] <title>`) when the editor is unattended or a caller's scope is held; the
row then answers `refused` with "name the file" over MCP. A test proves no caller is left
(`rg` in `tools/rust/check.sh`: `rfd::` outside that one file fails).

## Then, once, the things only a whole editor shows

1. **Concurrency** (`tools/qa/test_stress.py`, new, under 30 s): several connections flood
   read and edit commands while a build job, a scene load and UI commands run; every answer is
   well-formed, nothing deadlocks, the editor closes clean, the log has no panic.
2. **Budgets** (plan 10.8), read from the startup profile and written into STATE.md: the
   window within 0.3 s of the process start (warm), the file operations of a start, and that
   no command blocks the UI thread (the frame times `frame_pacing` answers while a build runs).
3. **The game as the judge**, short: the game repository's headless checks on assets the Rust
   editor wrote (`tools/nq/run_lua.py test_examples.lua`, `tools/nq/test_pda_layout.lua` - read
   the game repo's `tools/` to find how they take their input; the game repo is never edited),
   and ONE in-game run of at most three minutes with a module the Rust editor built - the QA
   module of the game's own suite (`tools/qa/nq/Run-NqQa.ps1 -Scenario Scout` or the nearest
   that takes a module as input). Never replace or remove a module the user has installed: a QA
   module has its own id, goes where the suite puts it and is removed after. If no suite can
   take a Rust-built module without a change to the game repository, do not force it: say so -
   the differential run has already proved the built files equal the C++ editor's.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- The suites start and close the editor themselves; for a look of your own use
  `tools\rust\run-editor.ps1`. Before any run, make sure no `Grizzly.exe` of an earlier run is
  left (the one under `Intermediate\cargo\debug` only - never touch `LevelEditor.exe`).
- Nothing is ever published, submitted, scanned, withdrawn or pushed; the publish and catalog
  suites talk to the mock only. No secret in a log or an answer.
- Nothing real is changed: projects are copied, the game is only read (the one in-game run
  aside, as bounded above), `_projects.ini` and the settings come back byte for byte.
- After a fix run the tests of the crate you touched; `tools/rust/test.sh` once at the end
  must still be green, and `cargo clippy --workspace --all-targets` clean.
- Never run a git command that changes the working tree or the index.

## Report

Under 500 words: every suite with PASS / FAIL and its seconds; the fixes that mattered, by
name; the two exception lists; the stress run, the budgets and the game's verdict; what is
still failing and exactly why.

# Brief Q1 - the suites and the checks

Read first: `docs/plan/RULES.md`, `CLAUDE.md` (the project's rules as they are now), `docs/COMMANDS.md`,
`docs/UPDATES.md`, the "For the suites" / "Requests" parts of the reports under `docs/plan/reports/` (each names
what it left for the suites), `Intermediate/plan2/commands.md` section 7 (the QA coupling table).

You own: `tools/qa`, `tools/rust` (the check scripts; NOT `check_words.py` and its two lists, which are engineer
Q2's), `tools/mcp`'s scripts, `tools/lint`; and in the code only this: `crates/gz-editor/src/registry.rs`,
`help.rs`, `tests.rs`, `lib.rs`, `crates/gz-mcp/src/lib.rs`, `crates/gz-mcp-bridge/src/main.rs` - the removal of
the untyped registry API (part 2) and these files' own comments. Engineer Q2 works at the same time on the
comments, names and messages of every other crate: do not edit a file of theirs; when the tree does not compile
because of a crate you do not own, work on what needs no build (the suites drive the editor that is already
built) and try again in a few minutes. The documents (`docs/*.md`, `tools/mcp/AI_SETUP.md`, `README.md`,
`CLAUDE.md`) are the architect's: do not edit them; list in your report every sentence of them that the code
makes untrue.

The suites are yours, all of them: where `RULES.md` says "only your suites, never the whole set", the whole set IS
your proof - run it when a round of changes is done, not after every edit (it takes about ten minutes; one suite
or one phase while fixing). Your QA port is 28991 (`GRIZZLY_QA_PORT`) for a suite you start by hand.

What exists already: `tools/rust/limited.py` (every program a script starts runs under it), `run_all.py`'s
`log_trouble` (a suite fails when an editor of it logged a graphics device error or a panic - keep it) and
`only_timings` (a suite that failed on nothing but its timings beside other suites is measured again alone,
and that run is its result - keep the rule, but make it a property of the check, not of its wording: the
client's `check(..., timing=True)` prints such a failure apart, and `run_all.py` reads that), `GRIZZLY_QA_KEEP_LOGS=<folder>` (the editors' logs of a run),
`check.sh`'s steps as they are. A QA editor's window is hidden, not minimised (`gz_win::started_minimized`): a
resize never brings it up.

## 1. The QA suites

- **One client**: `tools/qa/qa.py` - the socket client (`cmd(name, **params)`, `batch`, `gate`), the editor's
  lifecycle (start on its own port and home, wait until ready, close over the socket, never kill), the project
  copy and its guard, pictures, `check(...)` and the summary, and `finish()` which asks `perf.stalls` and fails
  the suite on a stall it did not declare. Every suite imports it; the thirty private `mcp()` helpers, the
  C++-target switches (`GRIZZLY_QA_TARGET`, `qa_target.RUST`) and `qa_target.py`'s two-editor code go.
- **One suite per area, each under two minutes** (`run_all.py` keeps running them side by side and keeps the
  last result of each): merge and cut until that is true. Candidates to merge or delete - decide by what each
  proves that nothing else does: `test_ui_tour.py` (the audit of the old redesign: its 35 issues are history -
  replace it with a short tour that opens every tab kind and dialog, takes `ui.audit` and one picture each),
  `test_stress.py`, `benchmark_scene_load` / `run_benchmark.py`, `bench_build.py`, `measure_compile.py`,
  `probe_start.py`, `mock_server.py`, `test_github_mock.py`, `kept.py`. A suite that only timed something
  becomes an assertion inside the area's suite or goes.
- `tools/qa/test_mcp_bridge.py`: the six tools, their annotations, the safety paragraphs word for word against
  the reference text kept in the suite, each door refusing what is not its own (a `destroy` command through
  `grizzly_run` answers `gate`), `grizzly_look` returning an image block, a call that outlasts the timeout
  answering `running` and not an error, two calls at once not waiting for each other.
- `tools/mcp/run_lifecycle.py` / `test_server_lifecycle.py`: the editor's and the commandlets' lifecycle
  (`Grizzly-Cmd.exe run / script / help`: exit codes 0 / 1 / 2, stdout and stderr, a closed pipe, Ctrl+C, no
  port taken, its own log), in one suite `test_cli.py` under `tools/qa`; `tools/mcp` keeps only `AI_SETUP.md`.
- `tools/qa/test_update.py` is headless: add a phase with a window that proves the updater's face - the chip for
  each state, the dialog `update` (`ui.dialog`) with the notes, the Preferences group, Help > Check for Updates -
  by `ui.list` and one picture each.
- `run_all.py`: the suite table is the truth; `--with-game` as now; no suite names an oracle.
- No suite, script or message under `tools/` names a command that no longer exists
  (`docs/plan/old_command_names.txt` lists them; engineer Q2 keeps the list as
  `tools/rust/removed_commands.txt` and its check reads `tools/` too once you call it from `check.sh`).

## 2. The entry points and the checks

- Three entry points: `bash tools/rust/check.sh`, `bash tools/rust/test.sh`, `python tools/qa/run_all.py`.
  CLAUDE.md, `check.sh`'s header and every document say three.
- `check.sh` gains: **no command without a card** (the commandlet's `help` prints no "Without a card" line -
  and then `Registry::add` / `add_sync`, `Args` / `ArgsExt` and `bare_effect` are deleted from `gz-editor`; fix
  what still used them; the bridge's fallback names in `LOOKS` go); **no forbidden word** (engineer Q2 writes `tools/rust/check_words.py` and its lists - you only call it from `check.sh`, as its last step, once its file exists); `tools/lint/check_encoding.py` moves beside the other checkers (`tools/rust`).
- `tools/rust/test.sh`: as it is (only what changed since green); its `--all` no longer says "corpus against the
  C++".

## Proof

`bash tools/rust/check.sh`, `bash tools/rust/test.sh` and `python tools/qa/run_all.py` green on the debug build
(each under `python tools/rust/limited.py`, each long run watched: say every two minutes how far it is); the
suite table with each suite's seconds, before and after; the count of private `mcp()` helpers before and after;
what each deleted or merged script proved and where that proof lives now. The release build in
`Bin\x64\Release` may be older than the sources while you work: `check.sh`'s "command line" step then fails, and
that is the architect's to rebuild - say so, do not run `Build.ps1`.

## Done means

One client module; one suite per area under two minutes, each ending on `perf.stalls`; the bridge's, the command
line's and the updater's face proven by suites; three entry points; no command without a card and no untyped
registry API left; no suite names an oracle or another editor.

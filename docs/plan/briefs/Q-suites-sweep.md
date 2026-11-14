# Brief Q - tests that earn their place; no trace of the C++ editor

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (all of it), `docs/COMMANDS.md`, `docs/ASSETS.md`,
`docs/UPDATES.md`, every report under `docs/plan/reports/` (each names what it left for the suites),
`Intermediate/plan2/commands.md` section 7 and its QA coupling table.

You own: `tools/qa`, `tools/rust` (the check scripts), `tools/mcp`'s scripts, `tools/lint`; in `crates/gz-editor`
the removal of the untyped registry API (part 2); and - for part 3 only - comments, doc comments, names that
only make sense as a port, messages and test modules in every crate (never behaviour). The documents
(`docs/*.md`, `tools/mcp/AI_SETUP.md`, `README.md`, `CLAUDE.md`) are the architect's: do not edit them; list in
your report every sentence of them that the code makes untrue.

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

## 2. The entry points and the checks

- Three entry points: `bash tools/rust/check.sh`, `bash tools/rust/test.sh`, `python tools/qa/run_all.py`.
  CLAUDE.md, `check.sh`'s header and every document say three.
- `check.sh` gains: **no command without a card** (the commandlet's `help` prints no "Without a card" line -
  and then `Registry::add` / `add_sync`, `Args` / `ArgsExt` and `bare_effect` are deleted from `gz-editor`; fix
  what still used them; the bridge's fallback names in `LOOKS` go); **no forbidden word** (part 3's checker, `tools/rust/check_words.py`); `tools/lint/check_encoding.py` moves beside the other checkers (`tools/rust`).
- `tools/rust/test.sh`: as it is (only what changed since green); its `--all` no longer says "corpus against the
  C++".

## 3. No trace of the C++ editor or of the SDK's ways

The code is the editor's own now. Sweep the whole tracked tree:

- **Comments and doc comments** that explain something by the C++ editor, a `Source/...` path, a C++ class or
  function name (`EScene::`, `TUI_`, `CUI...`, `NqUtil::`, `EditorCharacters::` ...), "as the C++ did", "byte
  identical to the C++", "the oracle", "parity", a tag name: rewrite each to say what the code does and why, or
  delete it when it says nothing else. A fact about a FILE FORMAT that is X-Ray's (`.ogf`, `.object`, LTX,
  `level.geom`, the SDK scene the importer reads) stays, stated plainly.
- **Names** in code that only make sense as a port (`cpp_*`, `Cpp*`, `*_cpp`, `COMMAND_*` ids if any survived):
  rename, with their users.
- **Messages** an author can see that carry the SDK's voice (`"TUI_ControlObjectAdd:: Can't load reference
  object."`, lines starting with `!` or `~`): sentences.
- **Tests** whose point was parity with the C++ editor, its writer's bytes or its answers: delete. Tests that
  duplicate another test or assert a constant: delete. Count the tests before and after and say what went, by
  crate, in your report - the aim is fewer tests that each catch something real, not a number.
- The forbidden-word check (`tools/rust/check_words.py`, exit 1 on a hit): `C++`, `cpp`, `oracle`, `parity`, `Source/`, `SDK 0.7`, `LevelEditor`,
  `xrLC` outside a short allow-list file of places where a format's or a game program's real name is the fact
  (`tools/rust/words_allowed.txt`, each entry with its reason).

## Proof

`bash tools/rust/check.sh`, `bash tools/rust/test.sh --all` and `python tools/qa/run_all.py` green on the debug
build (each under `python tools/rust/limited.py`); the suite table with each suite's seconds; the counts of
tests (by crate, with what went and why) and of MCP call sites before and after; `python
tools/rust/check_words.py` passing with its allow-list in the report. The sweep changes no behaviour: `git diff
--stat` of part 3 shows only comments, strings an author sees, names and deleted tests.

## Done means

One client module, one suite per area under two minutes, each ending on `perf.stalls`; three entry points; no
command without a card and no untyped registry API left; no suite names an oracle or another editor; no mention
of the C++ editor, an oracle or parity in the code and tools outside the allow-list.

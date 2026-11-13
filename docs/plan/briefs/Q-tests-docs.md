# Brief Q - tests that earn their place, documents that are true, no trace of the C++ editor

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (all of it), `docs/COMMANDS.md`, `docs/ASSETS.md`,
`docs/UPDATES.md`, every report under `docs/plan/reports/`, `Intermediate/plan2/commands.md` section 7 and its QA
coupling table.

You own: `tools/qa`, `tools/rust`, `tools/mcp`, `tools/lint`, the documents named below, and - for part 3 only -
comments and test modules in every crate (never behaviour).

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
- `run_all.py`: the suite table is the truth; `--with-game` as now; no suite names an oracle.

## 2. The entry points and the checks

- Three entry points: `bash tools/rust/check.sh`, `bash tools/rust/test.sh`, `python tools/qa/run_all.py`.
  CLAUDE.md, `check.sh`'s header and every document say three.
- `check.sh` gains: **no command without a card** (the commandlet's `help` prints no "Without a card" line -
  and then `Registry::add` / `add_sync`, `Args` / `ArgsExt` and `bare_effect` are deleted from `gz-editor`; fix
  what still used them; the bridge's fallback names in `LOOKS` go); **no forbidden word** (part 3's list, over
  the tracked tree); `tools/lint/check_encoding.py` moves beside the other checkers (`tools/rust`).
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
- The forbidden-word check (part 2): `C++`, `cpp`, `oracle`, `parity`, `Source/`, `SDK 0.7`, `LevelEditor`,
  `xrLC` outside a short allow-list file of places where a format's or a game program's real name is the fact
  (`tools/rust/words_allowed.txt`, each entry with its reason).

## 4. The documents

- `docs/ARCHITECTURE.md` replaces `docs/RUST_MIGRATION.md`: the layers and crates as they are, how state changes
  (stores, commands), threads and jobs, the frame, **the contract with the game** (what was section 9: every
  file that crosses to the game and what holds it) - written as the architecture of this editor, no phases, no
  differences from another editor, no history. Every sentence checked against the code.
- `docs/STATE.md` replaces `docs/rust/` (STATE.md, PARITY.md, CONVENTIONS.md, briefs): one page - what the
  editor is at 0.7, where each contract lives, what is deliberately not done and why. `docs/rust/` is deleted;
  what CONVENTIONS.md says that still holds moves into ARCHITECTURE.md.
- `tools/mcp/AI_SETUP.md`: one page - how a client connects (`grizzly-mcp.exe`, the port variable), the six
  tools, "assets are files" with the loop (edit, `asset.check`), the safety rules, a few lines per area pointing
  at `grizzly_help`. No list of commands: the editor prints its own.
- `docs/MOD_QUICKSTART.md`: check every command and step against the editor (stream M rewrote its body).
- `docs/UI_DESIGN.md`, `docs/NQ_ARCHITECTURE.md`, `docs/nq/NQ_FORMAT.md`, `docs/pda/PDA_FORMAT.md`,
  `docs/assets/*.md`: each stream wrote its part; you read them whole for what is stale, contradicts another
  document or still tells history, and fix it.
- `README.md`: what the editor is, how to build, the three programs, the three entry points, where the contracts
  are.
- `CLAUDE.md` is the architect's: do not edit it; list in your report every sentence of it that is no longer
  true.
- Tracked runtime data nobody reads (`gamedata/shaders/editor`, `gamedata/shaders/r5`, `gamedata/textures/ed`,
  `fs.ltx`'s aliases, `fsgame.ltx`, ... - verify each against the code: what opens it?): delete what nothing
  opens, keep `fs.ltx` only if the root finder needs a marker and then make it a marker with one comment line.
  `tools/rust/package.py`'s file list follows.
- `docs/plan/` is deleted at the very end by the architect.

## Proof

`bash tools/rust/check.sh`, `bash tools/rust/test.sh --all` and `python tools/qa/run_all.py` green on the debug
build; the suite table with each suite's seconds in your report; the counts of tests and of MCP call sites
before and after; the forbidden-word check passing with its allow-list in the report.

## Done means

PLAN section 5 "Q", every sentence.

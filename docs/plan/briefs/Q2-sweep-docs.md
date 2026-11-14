# Brief Q2 - no trace of the C++ editor; documents that are true

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (all of it), `docs/COMMANDS.md`, `docs/ASSETS.md`,
`docs/UPDATES.md`, every report under `docs/plan/reports/` (each lists the documents and CLAUDE.md sentences it
made untrue).

You own: comments, doc comments, names that only make sense as a port, messages and test modules in every crate
(never behaviour), `tools/lint`, the documents named below, and the tracked runtime data nobody reads. Engineer
**Q1** works beside you in `tools/qa` and the check scripts - you do not touch them (your forbidden-word checker
is a file of its own, `tools/rust/check_words.py`, which Q1 calls from `check.sh`).

## 1. No trace of the C++ editor or of the SDK's ways

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

## 2. The documents

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

`bash tools/rust/cg.sh check --workspace` and the crates' tests still green after the sweep (a comment sweep
changes no behaviour: `git diff --stat` shows only comments, strings an author sees, names and deleted tests);
the counts of tests before and after, by crate, with what went and why; `python tools/rust/check_words.py`
passing with its allow-list in the report; every document read whole once more after writing.

## Done means

No mention of the C++ editor, an oracle or parity in the tracked tree outside the allow-list; `ARCHITECTURE.md`,
`STATE.md`, `AI_SETUP.md`, `README.md` and `MOD_QUICKSTART.md` true to the code; `docs/rust/` gone; no tracked
file that nothing reads.

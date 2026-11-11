# Brief: the critic

The editor has been rewritten in Rust (`crates/`, branch `grizzly-rust`), integrated and
tested. You are the one independent review of the whole work before it is called done. You
did not write any of it; trust nothing you have not checked. **You change no file** - you
find what is wrong and say exactly where.

## What the work claims

- The contract: `docs/RUST_MIGRATION.md` - the invariants of section 2, the architecture of
  section 5, the rules of section 6, the contract with the game of section 9, the deliberate
  differences of section 12 ("a difference that is not here is a bug").
- The state and every known gap, as the authors tell it: `docs/rust/STATE.md` (the crate
  table, "Findings", "Closed in integration", "The first test run", "The differential run"
  with "Differences the differential left", "The QA suites", "What the suites could not hold
  the Rust editor to", "What the QA pass left, closed").
- The ledger: `docs/rust/PARITY.md`. The project's rules: `CLAUDE.md`. The UI contract:
  `docs/UI_DESIGN.md`. The specification is the C++ editor under `Source/` (untouched on this
  branch - verify that: `git diff 25f76d13 -- Source` must be empty).

## What to check - go for what would hurt, in this order

1. **The claims of the test passes are true.** Run the four entry points yourself, once:
   `bash tools/rust/check.sh`, `bash tools/rust/test.sh --all` (about half a minute),
   `python tools/qa/diff_editors.py --all` (the C++ side is recorded under
   `Intermediate/oracle`, so no C++ editor starts; about half a minute),
   `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py` (the suites side by side, a minute or
   two). Each has a mode that skips what already passed - `--all` is what proves. Then read
   HOW they pass: is the recorded oracle really the C++ editor's answers (the record's key,
   one scenario re-recorded if port 28016 is free - if the author's editor holds it, do not
   touch it and say so)? the ignore
   table of `diff_editors.py` (is every entry a real non-difference, or a hidden one?), the
   checks the suites skip for the Rust target (`git diff 25f76d13 -- tools/qa` shows every
   edit: was a check weakened?), tests that assert nothing, corpus tests that skip silently or
   sample too thin.
2. **Nothing the game reads changed meaning** (section 9): for each row, is there a proof that
   actually ran, and does it compare what the row says? The frozen release formats
   (`MOD_UPDATES.md`, `MOD_CATALOG.md` in the game repo `D:\Games\Dead Air\DeadAir-x64\docs\
   dead-air\`, read only) against `gz-release` / `gz-catalog`: the descriptor, the index read
   back from the finished archive, the package layout, the card, the signature, the content
   rules, the issue bodies.
3. **Safety of what reaches players and of what destroys.** In the code, not in the docs:
   publish goes draft -> every asset verified -> latest, a published tag is never overwritten,
   a listed module leaves only with a VirusTotal report; the token never reaches a log, an
   answer or a host other than GitHub's; `-qa_github` cannot read a real token; the
   `PLAYER_SAFETY` / `WITHDRAW_SAFETY` paragraphs reach the MCP client word for word from the
   Rust bridge; project deletion runs every guard of `CheckDelete` (compare with the C++ guard
   list in `CLAUDE.md`, "The Start page"), deletion never follows a link, the clean build
   refuses a target that is a link or holds the project; unsaved work is lost only through the
   three `*_discard` tools; no MCP path opens a dialog or hangs on one.
4. **The architecture rules hold where it is tempting to break them** (sections 5-6): `unsafe`
   only in `gz-win` and sound there (read every block); no `unsafe impl Send/Sync`; no global
   state (`static` with interior mutability, `OnceLock` / `LazyLock` holding state, thread
   locals used as globals); a mutex held across an `.await` or a frame; the UI thread blocking
   (`block_on`, `recv()`, `lock()` on something a job holds, file IO in a draw function - grep
   the `gz-ui-*` crates and `gz-app`); state changed outside a store's command; a domain crate
   that depends on egui or wgpu (`cargo tree`), a lower layer depending on a higher one.
5. **Nothing was quietly left out.** Stubs and placeholders (`todo`, `unimplemented`, "not
   yet", "later", an empty match arm that swallows a case, a command that answers `ok` and does
   nothing); features of the C++ editor with no counterpart and no line in section 12 or
   STATE.md - sample by walking the C++ menus and command table (`UI_MainCommand.cpp`, the
   handlers in `GrizzlyMCP.cpp` / `LevelMcp*.cpp`) against the Rust registry
   (`Intermediate\cargo\debug\Grizzly.exe -dump_commands`) and by opening the editor once
   (`tools\rust\run-editor.ps1`) on a temp copy of a project: the Start page, the Level tab
   with the Escape scene, a graph, Characters, Mod Settings, a dialog - `ui_audit` clean,
   pictures sane.
6. **It is as parallel and as quick to build as promised**: the budgets written in STATE.md
   against a measurement of your own (start to window, a build of the reference mod, a scene
   load); what happens to an incremental build when one leaf crate and when `gz-ui` change
   (`cargo build -p grizzly --timings` or simply the wall time) - is half the workspace
   rebuilt for a small edit?
7. **The documents tell the truth**: `docs/RUST_MIGRATION.md` describes what was built
   (crates, the MCP catalogue, the frame loop, the renderer's interface, the C dependencies),
   section 12 lists every deliberate difference and nothing else, `CLAUDE.md`'s Rust section
   matches.

## How to work

- You are one reviewer: do not start agents. Do not read everything - the tree is ~53 crates;
  search first (`rg`), read the core fully (`gz-store`, `gz-task`, `gz-editor`, `gz-win`,
  `gz-mcp`, `gz-app/src/window.rs`), and elsewhere read what the searches and the failing
  checks point at.
- Change nothing: no edits, no git command that alters the tree or the index, nothing pushed,
  published, submitted or scanned; the game and the user's projects are only read; an editor
  you start is closed through MCP; the author's own editor is never touched.
- Long commands write to a file and are looked at often.
- Every finding must be checkable: file and line, what is wrong, how you know (the command you
  ran, the C++ place that says otherwise), and how bad it is. No style notes, no praise.

## Report

Findings ranked by harm: **Blocking** (breaks the game's contract, loses data, leaks, unsound
`unsafe`, a claim of the tests that is false), **Serious** (a feature missing or wrong with no
line in section 12, a rule of the architecture broken), **Minor**. For each: `file:line`, one
sentence of what is wrong, the evidence, the fix you would make. Then one paragraph: is the
branch ready for the cutover gate of phase 7, and if not, the shortest list of what stands
between. Under 900 words.

# Brief: the first test run (`cargo test`, the corpus)

Every crate is written and integrated; **no test has ever been run**. This pass runs them all
for the first time, makes them tell the truth and makes them fast. You are alone in the tree.
Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; `docs/RUST_MIGRATION.md`
sections 5, 6, 9 and 10 are the rules the tests hold the code to.

You may edit any crate under `crates/`, `tools/rust/` and `docs/rust/STATE.md`. `Source/` (the
C++ editor: the specification) and the game under `D:\Games\Dead Air` are read-only.

## 1. The unit tests (866 of them)

- Build once, to a file you look at often: `bash tools/rust/cg.sh test --workspace --no-run`.
  Then run everything once: `cargo test --workspace --no-fail-fast` with the output in a file
  under `Intermediate/` (never a silent pipe), and work from that list crate by crate
  (`cargo test -p <crate>`), re-running only what you touched. One whole-workspace run at the
  end proves it.
- **A failing test is a question, not an obstacle: which side is wrong?** Decide from the
  specification - the C++ under `Source/`, the documents it names, the real files of the game
  and of the repository - and fix the wrong side. A test is never weakened, deleted or
  `#[ignore]`d to pass; an expectation an agent guessed wrong is corrected to what the
  specification says, with the place it says so in a comment when that is not obvious.
- A test that hangs is a bug: find it with a per-crate run under `timeout`, fix the cause.
- No test touches the network, the registry, the user's Documents, a real project or the
  repository's own files (`_projects.ini`, `grizzly_editor.ltx`): temp folders only
  (`tempfile`), and anything that would reads the QA environment (`GRIZZLY_CATALOG_HOME`, ...).

## 2. The corpus (new tests)

The format crates have no test that reads real data. Add one integration test per format,
`crates/<crate>/tests/corpus.rs`, run by the same `cargo test`:

- The data comes from `GRIZZLY_TEST_GAME` (a game root - on this machine `D:\Games\Dead Air`,
  read only) and from the repository itself (`rawdata\`, `gamedata\`, `docs\nq\examples`,
  `tools\qa\fixtures`); a project corpus from `GRIZZLY_TEST_PROJECTS` (a `;` list - here
  `D:\XFinedProjects\DARF-Overnight-Delivery;D:\XFinedProjects\Test;D:\XFinedProjects\WeaponsEvolution`,
  read only, never written). Without the variables a corpus test prints `skipped: <variable>
  is not set` and passes; `tools/rust/test.sh` (new, beside `cg.sh`) sets them and runs
  `cargo test --workspace`, and is THE way the tests are run from now on.
- What each proves (section 9 and 10.2 of the plan):
  - `gz-ltx` - every `.ltx` of the game and of the projects: the lossless document writes the
    bytes it read; the engine tree loads `system.ltx` and every root the editor reads, with
    includes, without an error the engine would not give.
  - `gz-mod` - every `mod.ltx` found: parse, write, equal bytes.
  - `gz-luadata` - every `.nqasset`, `.behasset`, `.npcasset`, `.pdaasset`: parse; the writer's
    output parses to the same value; a file the editor itself wrote (the examples) is byte-equal.
  - `gz-fmt-xml` - the game's UI XML and string tables parse (cp1251 included).
  - `gz-fmt-object`, `gz-fmt-ogf`, `gz-fmt-tex` (DDS headers, THM), `gz-fmt-sound` (OGG
    comments), `gz-fmt-xr` (`shaders.xr`, `gamemtl.xr`, `lanims.xr`, particles), `gz-fmt-level`
    (`.level` scenes of `rawdata\levels` and the projects), `gz-fmt-spawn` and `gz-entities`
    (the game's `all.spawn`: every entity read, written, equal bytes) - parse everything; where
    the crate has a writer, write and compare bytes; a difference is a bug of the crate unless
    the C++ writer provably normalises the same thing (then the test states the rule).
    **Known to fail today:** `gz-fmt-xr` refuses Dead Air's `particles.xr` - 50 effects (the
    first is `atom_flame`) carry the older 20-byte `TargetColor` action; the C++ loader reads
    both forms (`Source/XrParticles/particle_actions_collection_io.cpp`, the versions of
    `Source/XrRender/Private/ParticleEffectDef.cpp`). Fix the reader (and the writer, which
    must write back what it read); the viewport then plays the game's own `particles.xr`
    (`gz-ui-viewport` falls back to the editor's bundled file and logs a note until it parses -
    check that the note is gone).
  - `gz-particles` - every effect and group of the game's `particles.xr` steps for a second of
    simulated time without a panic or a NaN.
  - `gz-vfs` - the game mounts; every file an archive lists can be read; a hundred sampled
    files hash equal through the archive and (where present) loose.
  - `gz-game` - modes, communities, ranks, items, trade categories, level changers come out
    non-empty and hold the values the game's files show (assert a few known ids).
- **Speed is a requirement.** Files are walked in parallel (`rayon`), each read once. A corpus
  test finishes in under 15 s on a warm cache; if the whole corpus is too big for that, test a
  fixed stride of it (every Nth file by sorted path - deterministic), and say the stride.

## 2a. Concurrency (new tests)

The plan promised model checks for the hand-written synchronisation; what was built composes
tokio channels and `arc-swap`, so it is held by stress tests instead - short, deterministic
in what they assert, run by the same `cargo test`:
- `gz-store`: many tasks post commands while others read snapshots - every reader sees
  revisions that only grow and a state that is always one a command produced; undo / redo
  interleaved with edits never lose or double a step; `seal`, merged steps and `mark_saved`
  under load; a command that panics leaves the last good snapshot and the store alive.
- `gz-task`: jobs started and cancelled from many threads - a cancelled job stops at its next
  `check`, `wait` always returns, the snapshot of jobs never shows a finished job as running.
- `gz-editor`: frame-held requests (`FrameClock::after`) from many connections are each
  answered exactly once and never before their frame; `AutoSave` under a burst writes the last
  revision and nothing after `flush`.
- `gz-query`: the same key asked from many threads computes once per revision.
Each under 2 s.

## 2b. No test needs a window or a graphics device

A test that creates a wgpu device (in `gz-render`, `gz-ui-*`) must not hang or fail on a
machine without one - as of this writing no device can be created on this machine at all (a
dead process is stuck in the GPU driver until a restart). Such a test asks for its device
through one helper with a 5 s limit and prints `skipped: no graphics device` when there is
none; everything that can be tested without a device (packets, culling, the particle steps,
layout, models of panels) is tested without one.

## 3. Speed of the whole run

- `tools/rust/test.sh` prints each test binary's wall time. Budget: the whole workspace run,
  warm and already built, under 2 minutes; no single unit test over 2 s. A slow test gets a
  smaller fixture or a faster implementation - when the code under test is what is slow, that
  is a finding: make the code faster.
- Test binaries are many (53 crates). Do not merge crates for it; do keep `dev-dependencies`
  light and shared so the test build stays incremental.

## 4. What must also stay true

`cargo clippy --workspace --all-targets` clean, `cargo fmt --all` applied, `unsafe` in `gz-win`
only, and `bash tools/rust/cg.sh build -p grizzly` builds. Put those four into
`tools/rust/check.sh` (new: exit code 1 when clippy warns, when `cargo fmt --check` differs,
when `unsafe` appears outside `crates/gz-win` - it prints the count inside - or when
`Grizzly.exe -dump_commands` lacks a `cmd` of `crates/gz-mcp/tools.json`), and remove the
workspace dependencies no crate uses (`cargo tree` / a grep of the manifests: `parry3d`,
`rapier3d`, `loom`, `egui_dock`, `jwalk`, `walkdir`, `schemars`, `zip` were declared for a
plan that went another way - keep `proptest` only if you use it).

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Every bug a test finds is fixed at its cause, in the crate that owns it, with the rules of
  the plan's sections 5-6 (stores, no globals, no waiting UI thread, no `unsafe`).
- Never run a git command that changes the working tree or the index.
- Keep `docs/rust/STATE.md` current: a section "The first test run" with what the tests found
  that was a real bug of the code (one line each), and the timings.

## Report

Under 400 words: tests run / passed, the wall time, the real bugs found and fixed (the
important ones by name), what the corpus covers and with which stride, anything a test shows
that you could not fix and why.

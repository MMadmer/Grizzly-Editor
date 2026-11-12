# Brief: Play ends in the game

Play (D1 of the plan: bake the scene, build what the game needs, start the real game on it) now
reaches the game and stops there: about 22 s after Play the game crashes in
`CSE_ALifeDynamicObject::on_register()` for the actor - the ALife graph registry has no current
level. STATE.md ("The reviewer's verification, closed") says how far it gets and how the game
is started in a root of its own. Until the game loads the level, Play does not work, and the
steps that exist only in the Rust editor (the collision, the AI map, the game graph,
`editor.spawn`) have no proof at all. You are alone in the tree.

You may edit `crates/`, `tools/qa/`, `tools/rust/`, `tools/mcp/`, `docs/rust/`. `Source/` (the
C++ editor), the game under `D:\Games\Dead Air` (its repository included) and the plan
(`docs/RUST_MIGRATION.md`, `CLAUDE.md` - the lead's) are read-only.

## 1. The game loads the level Play made

- **The specification is two readable sources, use both.** What the game demands:
  the game's own source, `D:\Games\Dead Air\DeadAir-x64` - `alife_dynamic_object.cpp`
  (`on_register`), `alife_graph_registry.cpp` (`setup_current_level`: the level comes from the
  game graph vertex of the actor's `m_tGraphID`), `alife_spawn_registry*.cpp` (how
  `editor.spawn` / `all.spawn` is read: the header, the spawn graph, the artefact spawns, the
  patrols, the game graph inside it), `game_graph*.h`, the cross table. How the editor makes
  it: the C++ editor's builders under `Source/Editors/LevelEditor/AI`
  (`game_spawn_constructor`, `level_spawn_constructor`, `game_graph_builder`,
  the cross table builder) and `Scene/scene_spawn.cpp` - the Rust port has to match them step
  for step: the graph points and their level ids, the level table of the game graph, the
  actor's graph id, node id and distance, the spawn graph's vertices and edges, the cross
  table of the level.
- **Find it by reading the file back, not by guessing**: read the Rust `editor.spawn` with the
  repository's own reader (`gz-fmt-spawn`, proven on the game's `all.spawn`) and check, against
  the game's `all.spawn` as the model of a valid file, every invariant the game's loader
  relies on (the header's counts, each spawn's graph id inside the graph, each graph vertex's
  level id in the level table, the level's name and offset, the cross table's node count equal
  to `level.ai`'s). Put those invariants into a unit test of the builder, so the fault cannot
  come back.
- **Then the game**: `tools/qa/test_play_game.py` passes - the game's log holds the level's
  name loading and the actor spawned (the lines the game writes when a level is in: say which
  ones), no assert, no crash; the game is closed by the suite (its console `quit`, or its
  process by id after the proof), everything the run made is removed, the user's install
  (modules, saves, settings, `gamedata`, archives) untouched - the archive junctions unlinked
  before anything is deleted. Hard limit: three minutes a run. The suite is opt-in in
  `run_all.py` (`--with-game`): it starts the real game and takes half a minute, so the
  everyday set stays without it; say so in its header.
- A second scene with real content (the Escape of the Test project: 600k AI nodes, spawns of
  many classes) is NOT needed for the gate; one small level loaded is the proof. If the small
  level loads and you have time left inside the limits, do not go looking for more.

## 2. No editor writes into the repository root

During `run_all.py` a `grizzly_editor.ltx` appeared in the repository root: some suite started
an editor without `-home`. Make it impossible rather than finding the one suite: when `-home`
is absent the editor takes its home from the environment (`GRIZZLY_HOME`), and `run_all.py`,
`diff_editors.py` and `qa_target` set it for every process they start; a check at the end of
`run_all.py` fails the run when `_projects.ini` or `grizzly_editor.ltx` of the repository root
changed or appeared.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Nothing that already passed is run again to check something new. While you work: the
  builder's tests, `test_play_game.py`. At the end, once: `tools/rust/check.sh`,
  `tools/rust/test.sh` (default mode), `python tools/qa/diff_editors.py` (default mode: only
  what your change touches is run again), `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`.
- A fix of the builders must keep `s9_play` and the level-build scenarios at zero differences.
- The author's projects are only ever a source to copy from; `find` over `D:\XFinedProjects`
  and `D:\Games\Dead Air\modules` before your first run and after your last shows nothing
  changed - into STATE.md.
- Never run a git command that changes the working tree or the index.
- Keep `docs/rust/STATE.md` current: a section "Play ends in the game".

## Report

Under 300 words: what was wrong in `editor.spawn` or the graph (the exact field, the C++ place
that says what it must be); the game's log lines that prove the load and the seconds from Play
to them; `test_play_game.py`'s result; the home fix; the four entry points; anything still
open and exactly why.

# Brief: after phase 8c - the window while a job runs, the door to xrAI, a stale bridge

Phase 8 is built: the repository holds no C++, the editor compiles a level in its own process
(`gz-lc` as the job `level_run`), `docs/rust/STATE.md` "Phase 8c: the compiler in the editor, no
C++" says how (read it first, with "Phase 8c, the cleanup"). That pass left three things open;
this one closes them. You are alone in the tree (`D:\Games\XFined-Editor`, branch `grizzly-rust`).

You may edit `crates/`, `tools/`, `docs/UI_DESIGN.md`, `tools/mcp/AI_SETUP.md`,
`docs/rust/STATE.md`. `docs/RUST_MIGRATION.md` and `CLAUDE.md` are the lead's (what they should
say goes into STATE.md, "For the plan").

## 1. The scene is drawn only when its picture can change

Measured in the last pass (STATE.md, "The window while it compiles"; `tools/qa/measure_compile.py`):
a compile of `l12u_control_monolith` takes 64 s headless and 80 - 109 s in the editor with the
Level tab up, 70 s with a tab that draws no viewport. While a job runs the window draws at its
active rate (over 50 frames a second), and every frame of the Level tab builds a scene pass,
part of it on the CPU pool the compile runs on. The author asked for the opposite: a long
operation has every core, and the window stays alive beside it.

- **A scene pass is built only when something it shows changed**: the camera, the scene's
  snapshot, the selection, the view options, the pane's size or scale, the theme, a mesh or
  texture that finished loading, a gizmo or gesture in progress, what animates in the scene
  itself (particles, game lighting, a spawn's animation - only while it is shown and running).
  Otherwise the pane shows the picture it has. Find how `gz-ui-viewport` (`pane.rs`,
  `service.rs`: `request_build`, `pacing`) decides today and make that decision one function of
  the inputs' revisions; do not sprinkle flags.
- **A job's progress does not wake the scene.** The Task Center's card, the status bar and a busy
  indicator redraw themselves at a rate of their own (about ten a second is plenty for a
  progress bar; a spinner may want more - measure what its frames cost without a scene pass
  before choosing). Find what asks for the active rate while a job runs (`gz-app`'s `shell.rs`,
  `window.rs`, `tasks.rs`, `chrome.rs`) and say in STATE.md what it was.
- **Nothing is made smaller or slower for it** (the plan's section 10): no pool loses a thread,
  no priority is lowered, the compile is not paused for a frame. The drawing thread stays above
  normal priority.
- **What must keep working**: `screenshot_viewport` and `screenshot_editor` are held for fresh
  frames (`gz-app`'s `shots.rs`) - a picture asked for after a change shows the change, with no
  scene pass skipped that it needs; a minimized or covered editor is still pictured; the preview
  pass of `preview.png`; the first frame after a tab switch back to Level; a camera moved
  during a compile draws at once and stays smooth.
- **MCP**: `frame_pacing` answers what it takes to see this from outside - UI frames and scene
  passes counted separately over its window, and why the last scene pass was built. Its entry
  in `crates/gz-mcp/tools.json` says so.
- **Proof** (`tools/qa/measure_compile.py`, extended as needed; the fast build, the machine
  otherwise idle - say what else ran): the compile of l12u in the editor with the Level tab up
  against the headless one taken right after it, three pairs: within 10 % of each other; scene
  passes built during the compile with nothing moving: none (or the few you can name); the frame
  work while the camera orbits during a compile (drive it over MCP): p95 under 16.7 ms; the
  process's core load during the compile, beside the headless one's. `test_ui_shell`,
  `test_stress`, `test_scene_tools`, `test_previews`, `test_properties`, `test_project_browser`
  (the preview pass) through `run_all.py <names>`.

## 2. The door to xrAI goes

`level_build`'s actions `compile_aimap` and `compile_spawn`, and the Build > Compile rows AI Map
High / Low / Verify and Spawn This Level / All Levels, start an `xrAI.exe` beside the editor. No
build has made one since the cutover and none ever will: the editor builds the AI map, the game
graph and the spawn itself (`gz-levelbuild`'s builders; the plan's section 3).
- Remove the two actions, the five rows, `Program::XrAi` and what only they used (the `mode`
  and `all_levels` parameters if nothing else reads them, the log line's helper, the strings),
  their words in `crates/gz-mcp/tools.json`, `docs/UI_DESIGN.md` (the Build menu's paragraph)
  and `tools/mcp/AI_SETUP.md`. `rg -i "xrai\.exe|XrAi\b|compile_aimap|compile_spawn" crates tools docs/UI_DESIGN.md`
  then finds nothing (the format constants `XRAI_VERSION_*` and comments naming xrAI's classes
  as the oracle of a function stay).
- What `runs.rs` is afterwards: the game alone. Rename what the removal leaves misnamed; do not
  leave an enum of one variant if a plain function reads better.
- `tools/qa/diff_editors.py` scenarios and the suites do not name the two actions today
  (`rg`): check, and say so.
- For the plan: a deliberate difference (the next free D number, D26 unless the lead used it):
  "The Build menu has no rows for xrAI: no build makes that program, and the AI map, the game
  graph and the spawn are built in the editor's process."

## 3. A suite never runs against a stale bridge

In the last pass `test_mcp_bridge` failed with "listed 198, catalogue 199": `grizzly-mcp.exe`
embeds `crates/gz-mcp/tools.json` at build time, and the binary beside the editor was older
than the catalogue. The suite must say that itself: before it starts, the bridge it is about
to drive is compared with the catalogue (the bridge can answer a hash or a count of what it
embeds - pick what costs nothing at run time - or the suite compares write times), and a stale
one fails in one sentence naming the build command. `check.sh`'s registry step has the same
question for the editor binary: look, and make both answer the same way.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index. The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`.
- Nothing that already passed is run again to check something new: one crate, one suite while
  fixing; every long run writes to a file under `Intermediate/` that you look at often.
- The author's projects (`D:\XFinedProjects\*`) are only ever a source to copy from
  (`qa_target.copy_project`), no git command inside them; the game under `D:\Games\Dead Air` is
  read only; `find` over `D:\XFinedProjects` and `D:\Games\Dead Air\modules` before your first
  run and after your last shows nothing changed.
- No test writes a program file (`tools/rust/check_test_files.py`); no C++ compiler is started
  (the comparison works from its records); nothing under `Bin\` is removed.
- Every editor you start runs minimized or headless on a port and a home of its own, and is
  closed over MCP (`window_control close`), never killed while it answers.
- Proof at the end, once: `bash tools/rust/check.sh`; `bash tools/rust/test.sh` (the changed
  binaries); the suites named above and `test_mcp_bridge`; the measurements. The lead runs the
  five entry points on the release build afterwards.
- Keep `docs/rust/STATE.md` current: a section "After phase 8c: the window while a job runs",
  and "For the plan".

## Report

Under 350 words: what asked for frames while a job ran and what decides a scene pass now; the
three pairs of compile times, the scene passes counted, the frame work with the camera moving,
the core load; what `frame_pacing` answers; what went with xrAI (files, lines); how a stale
bridge is caught; the suites and checks with their times; anything still open and exactly why.

# Brief: what the QA pass left open

Every suite passes against the Rust editor (`tools/qa/run_all.py`: 21 of 21 in two minutes),
the differential run is at zero and the unit tests are green. This pass closes what the QA pass
wrote down as not done. You are alone in the tree. Read `docs/rust/CONVENTIONS.md` and, in
`docs/rust/STATE.md`, "The QA suites" with its parts "The look that was skipped" and "Budgets",
then `docs/rust/PLAN_DELTAS.md`.

You may edit `crates/`, `tools/qa/`, `tools/rust/`, `tools/mcp/`, `docs/rust/`. `Source/` (the
C++ editor: the specification) and the game under `D:\Games\Dead Air` are read-only.

## 1. Every gesture of the viewport has an MCP path

The rule of the project is that every action has an MCP command; four things were left
"behind a click in the viewport": Random Append placing, Attach on Click, the Group tool's
Pivot chooser, motion playback with Camera View.

- **One general command instead of four special ones: `viewport_gesture`.** It feeds the
  viewport's own gesture handling - the same code a mouse reaches, nothing parallel to it:
  `kind` = `click` | `double_click` | `drag` | `hover`, `at` and `to` in viewport points (the
  answer of `screenshot_viewport` / `viewport_controls` gives the size), `button`, `mods`
  (`ctrl`, `shift`, `alt`). It is held until the frames that applied it were drawn, answers
  what the gesture did (what was picked, placed, moved - the names), and refuses while a
  modal is up or the Level tab is not the active one. Nothing is sent to the operating system:
  the event enters where the window's input enters the viewport pane.
- With it, prove and LOOK at: a click with the Object tool in Add mode and Random Append on
  (the reference, scale and rotation come out of the random set), Attach on Click of the Spawn
  tool, a marquee, a move drag with the gizmo.
- The Pivot chooser of the Group tool and the motion's playback and Camera View are not
  gestures: give each its command or argument where it lacks one (`group_tool` /
  the motion commands of the scene package), reachable to look at (`ui_dialog`) and to act.
- Every new command or argument goes into `crates/gz-mcp/tools.json` with a description an
  agent can use, and `Grizzly.exe -dump_commands` keeps matching the catalogue.

## 2. Details and wallmarks, drawn

Nobody has seen detail objects or a wallmark drawn: the Test scene has no data for them. Make
the data in a temp copy, over MCP: a snap list (`snap_objects`), a detail object list from the
repository's or the game's detail objects, a base texture, First Initialize, Reinitialize; a
wallmark of a texture the game has, placed on geometry. Look at both pictures; fix what is
wrong. Then keep it proven: the check goes into a suite (point 6) - the viewport's picture
differs where the details and the wallmark are, against the same view without them.

## 3. Generate Full: the node count explained or fixed

On Escape, Generate Full gave 2.5M nodes where the scene's own AI map has 625k. The C++ editor
has no MCP command for it, so settle it from the code and from numbers:
- a unit test on a synthetic scene (a flat square, a step, a wall, a hole) whose node count and
  links follow from the C++ algorithm (`ESceneAIMapTools` generation: read it, name the
  functions in the test) - exact numbers, not "more than zero";
- on Escape: the area the snap list covers times the nodes per square metre against the count,
  and how many nodes share one cell at one height. Duplicates or nodes outside the snap
  geometry are a bug: fix it. If the count is simply the whole terrain flooded, say so with
  the numbers in STATE.md.

## 4. The UI frame while a build runs

`frame_pacing` showed frames of 52-73 ms during a build (mean 12-18 ms) and 20 ms at rest; the
samples say the UI thread is only drawing and presenting while the build's threads hold every
core. The promise is that the editor's own work never costs a frame.
- The worker threads of the CPU pool, the IO pool and the job threads run **below normal
  priority**; the UI thread and whatever presents run at normal. One safe function in `gz-win`
  (the thread's own priority, `// SAFETY:` as always), called where each pool starts its
  threads. No `unsafe` anywhere else.
- `frame_pacing` tells the frame's own CPU time apart from the wait for the presentation
  (a frame that waited for vsync is not a slow frame).
- Measure again on the same scene and build, write the numbers into STATE.md, and put a limit
  with headroom into `test_build_job` (the frame's own time during a build) so that a
  regression fails.

## 5. A rebuild against the C++ job

The budget says a rebuild is no slower than the C++ job; it was never timed on the same data.
Time both on the reference mod with `test_build_job`'s payload - the C++ editor over its MCP
(`Bin\x64\Release\LevelEditor.exe`, port 28016; if something already answers there it is the
author's editor: do not touch it, and say the C++ side was not timed) - a clean build and a
rebuild with nothing changed. Numbers into STATE.md.

## 6. The suites' time

The user's rule: nothing that already passed is run again to check something new, and the
tests are as fast as they can be.
- **`--only <phase,...>`** for every suite that has phases (`test_mod_catalog`,
  `test_mod_publish`, `test_pda`, `test_ui_shell`, `test_ui_tour`, and any other with lettered
  or named phases): each phase stands on its own setup; `run_all.py <suite> --only ...` passes
  it through.
- **`test_mod_publish` (60 s) and `test_mod_catalog` (46 s) under 30 s each.** Their time is
  about thirty packagings of the 78 MB reference mod. A phase that is not about the size of a
  package runs on a trimmed copy (the manifests, the quests, characters, PDA and screens the
  checks name; no heavy textures, sounds or levels); the phases about size, splitting and the
  index keep what they need - once. A release that a later phase only reads is packaged once
  and reused. What the suites check does not change.
- Look at where `test_ui_tour` (31 s), `test_build_job` (27 s) and `test_pda` (23 s) spend
  their time and cut the waits that are waits.
- The new checks of points 1 and 2 go into one new suite, `tools/qa/test_scene_tools.py`
  (under 15 s), added to `run_all.py`.
- Target: the whole set through `run_all.py` under 75 s of wall time.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Run what you changed and nothing else: one suite, one phase, one crate's tests. At the end,
  once: `tools/rust/check.sh`, `tools/rust/test.sh`, `python tools/qa/diff_editors.py`,
  `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py` - all four green.
- Never run a git command that changes the working tree or the index.
- Nothing is published, submitted, scanned or pushed; the mock only. Projects are copied into
  the system temp folder and removed after; the repository's own files stay untouched.
- Keep `docs/rust/STATE.md` current: a section "What the QA pass left, closed", and strike
  the closed lines of `docs/rust/PLAN_DELTAS.md`.

## Report

Under 400 words: each of the six points - done or not, with the numbers (frame times before
and after, the two build times, each suite's seconds and the wall time of the set, the node
counts); what the two looks showed; anything still open and exactly why.

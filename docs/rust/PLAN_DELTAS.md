# What is still to fold into `docs/RUST_MIGRATION.md`

The lead's running list. Where the built editor differs from the plan's first text, and the
deliberate differences D7-D14, are in the plan now (sections 5.6-5.8, 6, 7, 8, 10, 12, 13).
What the test passes and the critic add goes here first; the file is deleted when the last of
it is folded.

## New since the fold

- From the QA pass (STATE.md, "The QA suites"): `tools/qa/run_all.py` is how the suites run
  (section 10.4); the system file dialog is one kit function that opens nothing for an MCP caller
  or `-nodlg` (section 5's rules); new wire, all additions (D13): `spawn_animation`,
  `quest_view roles_more=`, `ui_dialog open id=make_question | rename_question |
  reset_surfaces_question`; `-qa_github` knob `GRIZZLY_GITHUB_PAUSE_MS`.
- D15 reaches a script: `tools/mcp/test_server_lifecycle.py --shutdown` sends `COMMAND_QUIT`.
- Candidate for section 12: a new smart cover's `hold position time` starts at 0 (uninitialised in
  the C++); the Rust editor refuses the classes the game's factory has no server class for, where the
  C++ placed a spawn point of undefined type.

## To verify once a graphics device exists (the look that was skipped)

Looked at in the QA pass (STATE.md, "The QA suites"): the three panels, Generate Selected and its
nodes drawn, `ui_audit`, `frame_pacing` during a generation, the exit path with a panel in its own
window, the Make question, the Rename and Reset Surfaces questions, spawn animation. Still open:

- ~~Details reinitialised and DRAWN, a wallmark placed and DRAWN: the Test scene has no detail
  slots, no snap list for them and no wallmark texture - a scene that has them is needed.~~
  (closed, STATE.md "What the QA pass left, closed" 2: made over MCP in a copy, looked at, held by
  `test_scene_tools` D and W)
- ~~Generate Full on Escape tripled the node count (625k -> 2.5M): the C++ editor has no MCP command
  for it, so it cannot be compared without a click.~~
  (closed, same section 3: the whole ground flooded - 93 % of the snap model's ground, no duplicates;
  the exact-count unit test)
- ~~From the scene package, all behind a click in the viewport or a panel with no MCP path: motion
  playback with Camera View, Random Append placing, Attach on Click, the Group tool's Pivot chooser.~~
  (closed, same section 1: `viewport_gesture`, `object_tool action=random`, `group_tool group | ungroup`,
  `ui_dialog open id=group_pivot`; looked at)

## From the pass that closed the QA pass's rest (STATE.md, "What the QA pass left, closed")

- New wire, all additions (D13): `viewport_gesture`, `object_tool action=random`, `group_tool action=group | ungroup`,
  `aimap_tool action=stats`, `ui_dialog open id=group_pivot`, `frame_pacing`'s `sampled_frames`, `frame_work_ms`,
  `frame_wait_ms`, `slow_frames`.
- Candidate for section 12: a spawn point's own shape that Attach on Click detaches gets a free `shape` name (the C++
  appends it nameless).
- Section 6 / 10.8: the UI thread and the viewport's render thread run above normal, the pools at normal on every core;
  the frame budget is held as the frame's own work, not its wait for the presentation.
- Section 10.4: the suites open only copies under the system temp folder (`qa_target.copy_project`,
  `guard_project`), `--only` for the phased suites, the github mock's `POST /__restart`.


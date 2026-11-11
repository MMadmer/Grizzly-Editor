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

- Details reinitialised and DRAWN, a wallmark placed and DRAWN: the Test scene has no detail
  slots, no snap list for them and no wallmark texture - a scene that has them is needed.
- Generate Full on Escape tripled the node count (625k -> 2.5M): the C++ editor has no MCP command
  for it, so it cannot be compared without a click.
- From the scene package, all behind a click in the viewport or a panel with no MCP path: motion
  playback with Camera View, Random Append placing, Attach on Click, the Group tool's Pivot chooser.

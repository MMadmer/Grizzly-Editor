# What is still to fold into `docs/RUST_MIGRATION.md`

The lead's running list. Where the built editor differs from the plan's first text, and the
deliberate differences D7-D14, are in the plan now (sections 5.6-5.8, 6, 7, 8, 10, 12, 13).
What the test passes and the critic add goes here first; the file is deleted when the last of
it is folded.

## New since the fold

(nothing yet)

## To verify once a graphics device exists (the look that was skipped)

- The AI Map, Detail Objects and Wallmark panels; Generate Selected and its nodes drawn; link
  edits; details reinitialised and DRAWN; a wallmark placed and DRAWN; `ui_audit`;
  `frame_pacing` during a generation.
- The exit path leaves no process behind (`App::exiting` and the scene package's ordered
  teardown); the floating panel's second window.
- Generate Full on Escape tripled the node count (625k -> 2.5M): find out whether the C++
  editor gives the same on that scene (by hand in the C++ editor - it has no MCP command for
  it) or whether the port over-generates.
- From the scene package: the Make question without `-nodlg`, motion playback and Camera View
  in motion, Random Append placing, Attach on Click, the Reset Surfaces question, the Group
  tool's Pivot chooser.

# What to fold into `docs/RUST_MIGRATION.md` at the end

The lead's running list: where the built editor differs from the plan's text, and the
deliberate differences to enter in section 12. Folded into the plan (and this file deleted)
after the test passes and the critic.

## The plan's text against what was built

- **5.6 UI.** The frame loop is our own (`gz-app/src/window.rs`: winit + egui-winit +
  egui-wgpu), not eframe; frames run on demand. The Level tab's dock is the kit's own
  (splitters and slots), `egui_dock` is not used. Panels may leave the window as egui
  viewports (opt-in preference, restart).
- **5.7 MCP.** Commands register by name (`gz_editor::Registry::add(area, name, fn)`); the
  catalogue (`crates/gz-mcp/tools.json`: tools, descriptions, schemas, annotations, the safety
  paragraphs) is the C++ bridge's export plus the additions, and the bridge (`grizzly-mcp`)
  serves it. Schemas are not generated from command types; parity is checked by
  `Grizzly.exe -dump_commands` against the catalogue. `gz-cmd` does not exist (the registry is
  in `gz-editor`).
- **5.8 Renderer.** `gz-render` takes a plain `DrawList` and knows no scene; the adapter is
  `gz-ui-viewport/src/draw*.rs`. Particles are a crate of their own (`gz-particles`).
- **6 Rust rules.** `unsafe` lives in `gz-win` only (`gz-crash` has none; `gz-d3d11` was not
  needed). C dependencies are two (LuaJIT, libvorbis). Not set up: `cargo-deny`, Miri, `loom`
  (the store, the frame stamp and the job token are compositions of tokio channels and
  `arc-swap`, with no hand-written lock-free code: they are covered by stress tests instead).
  `tools/rust/check.ps1` became part of `tools/rust/test.sh` / `check.sh`.
- **7 Workspace.** The real crate list (56): no `gz-cmd`, `gz-fmt-release`, `gz-virustotal`
  (in `gz-release` / `gz-github`); added `gz-editor`, `gz-publish`, `gz-particles`,
  `gz-mcp-bridge`, the `gz-ui-*` crates per surface; no `xtask`.
- **8 What replaces what.** The pick tree is the scene's own BVH (`gz-scene/src/pick.rs`),
  `parry3d` and `rapier3d` are not used; the zip writer is our own over `flate2`; Vorbis is
  decoded by `lewton` and encoded by `vorbis_rs`; file dialogs are `rfd`.
- **10 Verification.** The oracle corpus is `tools/qa/diff_editors.py` (both editors side by
  side), not `xtask corpus`; the ledger `docs/rust/PARITY.md` came from a read-only audit.
- **13 Working rules.** The Rust editor is `Intermediate\cargo\debug\Grizzly.exe` until the
  cutover; the QA suites pick the editor through `tools/qa/qa_target.py`.

## Deliberate differences to enter in section 12

- No splash window: the editor's own window is up before one could be.
- Operations the C++ could not undo are one undo step (portal computation, detach, clears).
- A classic module keeps `target = any` on save (the C++ `mod_set_manifest` rewrote it to
  `existing`, against the project's own rule).
- Breakable and climable objects are built (the fork's `VCPacked::add_vert` lost its welding,
  so the C++ editor of the branch point writes errors instead).
- The AI map tool's Ignored Materials leave holes (the C++ check is commented out); the link
  pad's Invert turns a one-way link round (the C++ linked a node to itself); a wallmark can be
  moved (the C++ control was disabled).
- Points-mode marquee selects the points inside the box (the C++ tool selected every point of
  a picked way).
- A group's Make Thumbnail writes `<project>\rawdata\groups\<reference>.thm`, where the Groups
  chooser reads it (the C++ wrote `$objects$\<scene name>.thm`, which nothing reads).
- The tool switches (snaps, Normal Align, Pick Any Class) are kept between sessions in
  `[editor_prefs] tools_settings`, as the C++ `ETFlags`.
- Wire: refusals of `game_link` / `game_modes` carry `error`; `characters_write` refuses a
  path outside the project; `world_set` writes only its section; `mod_publish start` answers in
  one call; `quest_history` adds `undo_steps` / `redo_steps`; the catalogue's additions
  (`jobs`, `level_build`, the tool commands, `particles_tool`).
- Results that cannot be byte-equal by construction (already in STATE.md): package bytes,
  signatures, orders the C++ took from pointer addresses or `rand`.

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

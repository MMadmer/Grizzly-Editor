# Brief: the PDA preview and the Screens & Music tab (`gz-ui-pda`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md`
sections 1, 1a, 2, 4 and, in section 6, "PDA Interface" and "Screens & Music" - **the
contract**; `CLAUDE.md` sections "PDA interfaces (EditorPda)" (the preview tab, its toolbar,
the button placement rule, Dead Air's fonts) and "Screens and music (EditorScreens)" (layers,
Import, drag and drop); `docs/pda/PDA_FORMAT.md`; the rustdoc at the top of `gz-ui`, `gz-app`
(`ShellBuilder`, `Document`, `DocCx`, `on_ui`), `gz-pda` (`evaluate`, the `Preview` state
machine), `gz-screens`, `gz-lua`, `gz-game` (files, texture sheets, strings), `gz-fmt-tex`,
`gz-fmt-xml`, `gz-mod`, `gz-ui-content` if it exists (its drag payload type), `gz-editor`.

You own: `crates/gz-ui-pda/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorPdaPreview.cpp` (2.4k lines): how the preview DRAWS -
  the 2D PDA from the linked game's `pda.xml` / `pda_16.xml`, the device texture, the screen
  noise, the game's textures and its BITMAP fonts (`fonts.ltx` + the glyph ini, cp1251), the
  item list in canvas units scaled PER AXIS, `DrawItem` (the button placement rule: implemented
  twice, in the game and here - change neither), what is shown for what cannot run outside
  the game (a stock section as a labelled panel, `signal:` / `lua:` as a toast and a log line,
  the map as the level map with the preview's actor and markers).
- `Source/Editors/XrECore/Editor/EditorScreens.cpp` (1.4k lines): the tab's drawing; and
  `Source/Editors/XrEUI/XrUIFileDrop.cpp` (dropped files).
- Handlers (frozen wire): `pda_preview`, `screens_view` (find them with
  `rg -n '"pda_preview"|"screens_view"' Source/Editors`). Schemas: `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_pda.py`, `tools/qa/test_screens.py`.

## Architecture (decided)

**The PDA preview** (document kind `pda`, a singleton that switches to a new target, asking
first when its Source pane holds edits):
- The layout comes from `gz_pda::evaluate` as a `Query` (the game's reader under LuaJIT, off the
  UI thread); interaction goes through `gz_pda`'s `Preview` state machine. This crate draws the
  flat item list and forwards clicks, keys and wheel - it interprets nothing of its own.
- Drawing is egui meshes with the game's textures: every sheet and font texture is read through
  `gz-game`'s files and decoded on the pools, then uploaded once and cached for the session
  (dropped with the project). A bitmap font is a glyph table (parse `fonts.ltx` and the glyph
  ini here, as pure functions with tests) drawn as textured quads, so a text is as wide here
  as in the game. Canvas units scale per axis; a square icon stays square in 16:9.
- Toolbar: Save, Revert, Reload | Aspect, Language, Zoom (Fit / 100%; `Toolbar::segmented`) |
  Reset State | Source, Details. The canvas pans (right / middle drag) and zooms at the mouse
  (Ctrl+wheel always; over a PDA list or the map the wheel is the game's), F fits. Details
  (320 pt): the file with Use in Module, then tabs Problems, Pages (with the keys), Vars, Log.
  Source is a bottom split: Mono, line numbers, the reader's error line, Apply (Ctrl+Enter).
- The tab follows the file (the project file tree's revision) unless its Source pane holds
  edits. Unsaved Source text is never lost silently: Revert confirms naming the file; close and
  retarget ask Save / Don't Save / Cancel; over MCP `unsaved` unless `save=` / `discard=`.
- `pda_preview` (open / state / click by id, list row, n or canvas point / page / key / aspect /
  lang / reset / pane / tab / zoom / apply / save / revert): the state answers in CANVAS units,
  with `map.bound`, `map.spots`, `panes`, `scale`, `text_dirty`, `read_line`. An opener
  activates in the same call; an answer that depends on the frame waits for it.

**Screens & Music** (document kind `screens`, a singleton; saves to `mod.ltx` as it is edited
through `gz-screens` / `gz-mod`'s store):
- Toolbar: `Loading | Menu`, then `Screens [Mix In | Only Mine]` and `Music [Mix In | Only
  Mine]` for that surface, then the status. Left (260 pt): Screens and Music lists, each with a
  + at its header; a row's dot is a problem of it, its hover action removes it. Right (up to
  640 pt): for a screen a 16:9 preview (pictures as they are - decoded on the pools; a GIF's
  first frame; the rest marked where it goes), Layers (top row is the top layer, + adds, the
  cross removes, drag reorders, a click opens that layer's File, Fit, Opacity, folded
  Position), Music, and More folded; for a track: File, Shuffle On, Volume, Weight.
- **Files arrive by drag and drop**: from Explorer (egui's dropped files: only where it lands
  counts) and from the Content Browser's tiles (its drag payload; a tile lights up the zone
  under it; inner zones are asked first). On a layer it replaces its file, on Layers it adds
  layers, on Screens or the empty tab each becomes a new screen, on Music tracks; an `.ogg` is
  a track wherever it lands. Everything goes through `gz_screens::import` (off the UI thread).
  Build this drop-zone as a small reusable function in this crate (`local_file_drop_zone`).
- `screens_view` opens or reports the tab.
- This is the editor's reference surface for "simple, visible" (UI_DESIGN 1a): one task, one
  view, + and a cross, no Apply, the result shown.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes, never decodes and never touches a file.
- Every edit is a command of the area (`editor.post(..)` / the area's store).
- Compile: `bash tools/rust/cg.sh check -p gz-ui-pda -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK at: a PDA layout written from the
  template in a temp copy of a project (never a real project), in 4:3 and 16:9, the Source
  pane with an error; the Screens tab with one screen of two layers.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. It takes a machine-wide lock (other authors run the
  editor too; waiting minutes is normal), backs up and restores `_projects.ini` and the
  settings file byte for byte, starts the editor minimised, hands your driver the port as its
  first argument and closes the editor afterwards. Never start `Grizzly.exe` by hand. If
  `grizzly` does not build because of someone else's crate, wait and retry a few times, then
  say so. Do not run the QA suites.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-pda` to `docs/rust/STATE.md`.

# Brief: the application's own surfaces (`gz-ui-app`)

The surfaces that exist before and beside any content: the Start page (project browser), the
Output Log, Preferences. Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then
`docs/UI_DESIGN.md` sections 1, 1a, 2, 4 and, in section 6, "Start page", "Output Log",
"Preferences" - **the contract** -; `CLAUDE.md` sections "The Start page (project browser)" and
"Dangerous operations"; the rustdoc at the top of `crates/gz-ui/src/lib.rs` (the kit: draw with
it and nothing else), `crates/gz-app/src/lib.rs` (the shell: `ShellBuilder`, `Document`,
`on_ui`, `EditorSettings`), `crates/gz-project/src/lib.rs`, `crates/gz-game/src/lib.rs`,
`crates/gz-editor/src/lib.rs`.

You own: `crates/gz-ui-app/**` (new; `src/lib.rs` before `Cargo.toml`) and one `install` line
in `crates/grizzly/src/main.rs` (inside `install_ui`) plus the dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorProject.cpp`: the browser's drawing and behaviour (grep
  `start_page`, `DrawBrowser`, `Card`, `ShowsNoScene`, `CapturePreview` - the capture itself is
  a later package, loading and judging a `preview.png` is yours), the New Project form, the
  delete confirmation's text, Link Game.
- `Source/Editors/XrECore/Editor/UILogForm.cpp` (450 lines): the Output Log.
- `Source/Editors/XrECore/Editor/EditorPreferences.cpp` (1k lines): the pages, the keys and their
  official labels, the Keyboard page.
- `Source/Editors/XrECore/Editor/EditorUiContract.h`: constants QA pins (card menu labels,
  dialog ids and titles). `crates/gz-app/src/contract.rs` already holds the shell's share: add
  yours there only if the shell's `ui_contract` must answer it, otherwise keep them in your
  crate and feed `ui_contract` through the hook the shell offers.
- Handler (frozen wire): `project_browser_menu`
  (`Source/Editors/LevelEditor/Editor/Mcp/LevelMcp.cpp`); schema in `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_project_browser.py`, and the Preferences part of
  `tools/qa/test_properties.py`.

## What to build

**The Start page** (`ShellBuilder::start_page`), shown when no project is open - a page, not a
modal:
- New Project and Open Folder... head the list; the form (Name with its rule as the hint,
  Create, Location under More) comes up on New Project, or by itself with no project on the
  list. Open Folder uses the system's folder dialog (the `rfd` crate: add it to your
  `Cargo.toml`; run it off the UI thread and post the result back with `on_ui`).
- A search (`SearchField`), a Last Opened / Name sort kept in `_projects.ini [browser] sort`
  through `gz-project`, and 232 x 184 pt cards: the preview cropped to the card, the name on a
  row of its own with the `...` menu, the path with a start ellipsis (two cards of one name keep
  the differing parent folder whole), a Missing / Not a project badge. A card whose folder is
  gone stays, veiled, opens nothing.
- **Previews never block a frame**: `preview.png` is read and decoded on the pools and becomes
  an egui texture when ready (a `Query` keyed by path and write time; the list comes from
  `gz-project`'s snapshot). A picture of nothing (`ShowsNoScene`) draws the placeholder. A card
  that leaves the list releases its texture on the next frame, not in the frame being drawn.
- The card menu: Show in Explorer, Delete... (error colour), or Remove from List for a card
  that is no project any more. Delete opens `Delete Project###dlg.delete_project` (M; Delete is
  Danger, Cancel the default; Enter and Escape cancel) naming what disappears - the project,
  its folder, files, folders and size, the git repository and `releases\` when present,
  "Deleted for good - this cannot be undone." The numbers come from `gz-project`'s one walk,
  counted once when the dialog opens, off the UI thread. A project that opens under the dialog
  takes it down. The checks are `gz-project`'s; this package only shows them.
- Link Game: an M page card (`ShellBuilder::page_card`) - [Close Project] [Link] - that takes
  the page over while the open project has no game linked.
- `project_browser_menu show=menu|confirm|none`, `search=`, `sort=`, `new_project=` - to look
  at, never to act; it answers `cards` (with `preview` per card and `state`).

**The Output Log**: one body used by the Level panel and by the Log drawer
(`ShellBuilder::drawer(Drawer::Log, ..)`, `panel(..)`, `log_state`): a toolbar (search, All /
Warnings / Errors with counts - the status bar's counts -, Auto-scroll, Copy, Clear), Mono
lines, virtualised (only the visible rows are laid out; the log holds tens of thousands), a
muted row with a rule where the count starts ("The project opened - warnings and errors are
counted from here"). The Warnings / Errors filter lists each distinct line once. It opens at
its end whenever it comes up. It reads `gz_log::Log` (`since`, `tail`, `rev`, `counts`): keep a
cursor, never copy the whole log per frame. The status bar's "last line worth reading" must
skip the startup profile and other plain notes - set the `log_state` hook accordingly.

**Preferences** (document kind `preferences`, a singleton tab): search, a nav list with an
icon per page (`nav_row`), and the property grid; a page at most 44 em wide, a value at most
16 em. Settings apply live and are saved when a field loses focus and on close, through the
shell's `EditorSettings`. Port every preference of `EditorPreferences.cpp` with its official
label, help and unit - including the ones whose consumers arrive later (viewport, recovery,
snap steps): a preference is its key, its type and its text. The Keyboard page is a Command |
Shortcut | [Assign...] [Clear] table grouped as the menus, under the palette's names, with
keycaps, over the shell's binding table. `preferences action=show page= search=` (the shell's
command) opens this tab on a page: expose what the shell needs through `mcp_state` / a hook.

## Rules

- A frame never computes and never touches a file: lists, counts and pictures come from
  snapshots and queries; anything slow runs on the pools and comes back through `on_ui` or a
  query's wake.
- Every action here goes through an existing command of `gz-project` / `gz-game`
  (`editor.post("open_project", ..)`), so the UI and MCP share the code path; a button that
  needs a command which does not exist is a reason to add the command to this package's
  registration, not to bypass.
- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- Product text is English in the official tone; labels in Title Case, messages as sentences.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-app -p grizzly`, zero warnings. Then build and
  run once and LOOK (`cargo build -p grizzly`; start `Intermediate\cargo\debug\Grizzly.exe
  -nodlg -mcp_port 28917` with `Start-Process -WindowStyle Minimized`; `screenshot_editor`
  over the socket; pictures of the Start page with the real project list, the delete dialog
  through `project_browser_menu show=confirm`, the Log drawer through `ui_drawer`, the
  Preferences tab). Back up `_projects.ini` and the editor's settings file before the run and
  put them back byte for byte after; create nothing in real projects; close with
  `window_control action=close`. Other crates of the workspace are being written at the same
  time: if `grizzly` does not build because of someone else's crate, wait and retry a few
  times, then say so in the report. Do not run the QA suites.

- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. It takes a machine-wide lock (other authors run the
  editor too), backs up and restores `_projects.ini` and the settings file byte for byte,
  starts the editor minimised, hands your driver the port as its first argument and closes
  the editor afterwards. Never start `Grizzly.exe` by hand and never back those files up
  yourself.

## Report

Under 300 words: what is built, the `local_*` widgets, what the pictures showed, deviations
from the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-app` to `docs/rust/STATE.md`.

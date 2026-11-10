# Brief: the shell (`gz-app`)

The frame every tab lives in: caption, tabs, toolbar, status bar, drawers, the Task Center, the
palette, keys, the session, and the `ui_*` commands. Read `docs/rust/CONVENTIONS.md` and
`docs/rust/STATE.md` first; then `docs/UI_DESIGN.md` sections 1, 2, 5 and 7 - **the contract** -
and skim section 6 for what tabs will need; `CLAUDE.md` section "Editor UI (UI 2)"; the rustdoc
at the top of `crates/gz-ui/src/lib.rs` (the kit: draw with it and nothing else),
`crates/gz-editor/src/lib.rs`, and `crates/gz-app/src/{lib,window,shots}.rs` (the window loop,
which exists and works: build on it).

You own: `crates/gz-app/**`, and `crates/grizzly/src/{main.rs,spike.rs}` (replace the spike by
the real shell).

## The C++ that is the spec

- `Source/Editors/XrEUI/XrUIShell.h/.cpp` (the shell: `DocDesc`, tabs, frame hooks, session).
- `Source/Editors/LevelEditor/UI/`: `UIMainForm`, `UIMainMenuForm` (menus), `UITitleBar`
  (caption and hit test), `UITopBarForm` (the project toolbar).
- `Source/Editors/XrECore/Editor/`: `UI_ProgressCenter.cpp` (Task Center), `UI_CommandPalette.cpp`,
  `EditorPreferences.cpp`, `EditorUiContract.h` (every constant QA pins), `ui_main.cpp` (key
  routing: `ApplyShortCut`, the progress pump), `UI_MainCommand.cpp` (the command table and
  shortcuts), `GrizzlyMCP.cpp` (`state`, `screenshot_editor`, `titlebar_info`, `progress`).
- `Source/Editors/LevelEditor/Editor/Mcp/LevelMcp.cpp`: the `ui_*`, `window_control`,
  `preferences`, `shortcuts`, `theme`, `command_palette`, `reset_layout`, `ui_contract`
  handlers. The wire is frozen by these. Schemas: `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_ui_shell.py` (read it: it is the acceptance test of this
  package).

## Architecture (decided - implement it this way)

**1. The UI thread owns the shell; other threads reach it through a queue.**
`UiQueue` (an `Editor` service): `on_ui(editor, |shell: &mut ShellState, ctx: &egui::Context| ..)
-> impl Future<Output = Result<T, CmdError>>` pushes a boxed closure and pings `Wake`; the shell
drains the queue at the START of the next frame and answers through a oneshot. So an MCP opener
"activates in the same frame", shell state needs no lock, and nothing of the UI is shared
mutably. Headless (no window) the call answers `the editor runs without a window`. Reads of
editor DATA never come here - they go to stores.

**2. Answers that describe a picture are read after the frame.** A command that changes what
is drawn and answers where it landed (rectangles, scroll positions) runs its closure, then
awaits `editor.frames.after(n)` and reads again with a second closure. `screenshot_editor`,
`ui_list`, `ui_audit`, `titlebar_info` wait for frames first (see `shots.rs`). One call each,
raw socket included.

**3. A tab is a `Document`.**
```rust
pub trait Document {                      // UI thread only; not Send
    fn id(&self) -> &str;                 // "level", "quest:<rel>", "characters", ...
    fn kind(&self) -> &'static str;       // one of ui_contract's document kinds
    fn label(&self) -> String;            // an own asset shows its name without the extension
    fn icon(&self) -> Icon;
    fn path(&self) -> Option<String> { None }
    fn closable(&self) -> bool { true }
    fn dirty(&self) -> bool { false }
    fn missing(&self) -> bool { false }
    fn padded(&self) -> bool { true }
    fn toolbar(&mut self, tb: &mut Toolbar, cx: &mut DocCx) {}
    fn body(&mut self, ui: &mut egui::Ui, cx: &mut DocCx);
    fn status(&mut self, ui: &mut egui::Ui, cx: &mut DocCx) {}
    fn menus(&mut self, bar: &mut MenuBar, cx: &mut DocCx) {}
    fn shortcut(&mut self, cx: &mut DocCx, chord: &Chord) -> bool { false }
    fn edit(&mut self, cx: &mut DocCx, action: EditAction, run: bool) -> bool { false }
    fn save(&mut self, cx: &mut DocCx) -> Result<(), String> { Ok(()) }
    fn discard(&mut self, cx: &mut DocCx) {}
    fn on_activate(&mut self, cx: &mut DocCx) {}
    fn on_deactivate(&mut self, cx: &mut DocCx) {}
    fn inactive(&mut self, cx: &mut DocCx) {}     // a tick while another tab is drawn
    fn restore_arg(&self) -> Option<String> { None }
    fn mcp_state(&self) -> serde_json::Value { serde_json::Value::Null }
}
```
`DocCx` carries `&Arc<Editor>`, the GPU (`egui_wgpu::RenderState`), and a `ShellRequests` sink
(open / activate / close a tab, open a dialog, a drawer, toast) applied after the body returns.
A document holds VIEW state only; its data lives in the area's store and it draws from
snapshots and emits commands (`editor.post`). Only the ACTIVE document's `body` runs.

**4. Areas plug in; the shell names none of them.** `ShellBuilder` collects, before the window
opens: document factories by kind (`fn(&Arc<Editor>, &OpenRequest) -> Result<Box<dyn Document>,
String>`), menu contributions (menu, group, order, item with its command and shortcut id),
the project toolbar items (Build Mod split, Publish Release, Releases), drawer bodies
(`content`, `log`), status chips, the Start page (drawn when no project is open), page cards
that take the page over (Link Game, Recovery), dialogs that open by id (`ui_dialog open`), and
panel descriptors for the Level tab. Each UI crate exposes `pub fn install(b: &mut
ShellBuilder)`; `grizzly/src/main.rs` calls them. This is what keeps a tab's edit from
rebuilding the shell.

**5. The frame** (chrome totals 92 pt): caption 32 (logo, menus, the tab strip, the overflow
`⌄`, a drag band of at least 48, the project name - dropped first -, window buttons 46 x 32),
toolbar 36 (the tab's `toolbar()`, a spring, the project toolbar), the work area, status bar 24.
The window has no system decorations: dragging is `ViewportCommand::StartDrag` from the caption's
free band, a double click toggles maximise, the 6 pt edges resize with `BeginResize`. Under
1100 pt the menus fold into one Menu button. The Start page, Link Game and Recovery show no
menus.

**6. Tabs** are a custom strip, not docking: icon, name, one slot (dirty dot, or the cross on
hover and on the active tab); the active tab on the base fill with a 2 pt bright overline and
its whole name up to 220; others shrink to 96, the rest go under `⌄` ("+N"); middle click
closes, dragging reorders, the context menu of UI_DESIGN; Level is pinned and never closes.
Closing a dirty tab asks through the kit's `unsaved_prompt` (`Asker` from the origin); over MCP
a dirty close without `save` / `discard` answers `unsaved`.

**7. Keys, first taker wins:** a text field; the shell's chords (Ctrl+Shift+P, Ctrl+(Shift+)Tab,
Ctrl+F4, Ctrl+Space, Ctrl+`, Ctrl+S, F11) - none while a modal is up; the document's
`shortcut()`; then bindings contributed for the Level scene, only while Level is active,
nothing modal, no key-owning panel focused. The binding table is data (`shortcuts` over MCP,
the Keyboard page later); menu items show the keys bound NOW.

**8. Drawers, tasks, toasts, dialogs.** The Content and Log drawers cover the bottom of any tab,
one at a time (40% of the height, 260-480), closed by Esc, a click outside, or another tab
coming up. The Task Center is a status chip plus a 400 pt card bottom-right, fed from
`editor.jobs` (a job may attach steps and numbers as JSON - define a small `TaskCard` struct
jobs can publish through an `Editor` service; MCP jobs never open the card). Toasts stand on
it. Kit dialogs are drawn last (`dialog::draw_overlays`); `screenshot_editor` answers `modal`
from `dialog::modal_on_top`.

**9. Persistence.** A project's tabs in `<project>\temp\session.ltx` (`tabN =
id|kind|icon|label|arg`), restored lazily as placeholders until activated; a tab whose file is
gone shows "File not found" (`missing`). The kit's layout store, the window placement, the UI
scale and the theme go to the editor's own settings file (a new file under
`editor.paths.root`, through `gz_ltx::LtxDoc`; nothing is migrated from `level.ini`). Saved
when changed, on the IO pool, never per frame.

**10. Scale and theme.** `ui_scale` sets egui's zoom factor over the monitor's scale (tests
only, never saved); the theme's accent family is a preference.

## Commands of this package

`state`, `ui_list`, `ui_tab` (and the discard form: `action=close discard=true`), `ui_drawer`,
`ui_dialog`, `ui_popup`, `ui_scroll`, `ui_split`, `ui_audit`, `ui_scale`, `ui_contract`,
`window_control` (full; replaces the headless stub), `titlebar_info`, `reset_layout`, `theme`,
`command_palette`, `preferences`, `shortcuts`, `ui_toast`, `progress`, and `screenshot_editor`
extended with `busy`, `modal`, `outside`. `ui_panel` belongs to the Level package: answer the
C++ error for an unknown panel until it registers them. Reads never switch the tab.

## Rules

- Draw only with the kit: palette colours, `metrics`, `Font`, `Icon`. A widget the kit lacks is
  a local function named `local_<name>` in this crate, listed in your report.
- A frame never computes and never touches a file: what it shows comes from snapshots, queries
  (`Answer::Stale` is drawn with a busy mark) and view state.
- No panic can take the window down silently: the body of a document runs inside
  `std::panic::catch_unwind`; a panicking tab shows an error page and stays closable.
- Compile: `bash tools/rust/cg.sh check -p gz-app -p grizzly`, zero warnings. Then build and
  run it once (`cargo build -p grizzly`, start `Intermediate\cargo\debug\Grizzly.exe -nodlg
  -mcp_port 28917` with `Start-Process -WindowStyle Minimized`, take `screenshot_editor` over the
  socket, LOOK at the PNG, close with `window_control action=close`). This one look is part of
  the task: a shell nobody looked at is not done. Do not run the QA suites.

- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']`. It takes a machine-wide lock (other authors run the
  editor too), backs up and restores `_projects.ini` and the settings file byte for byte,
  starts the editor minimised, hands your driver the port as its first argument and closes
  the editor afterwards. Never start `Grizzly.exe` by hand and never back those files up
  yourself.

## Report

Under 400 words: what is built, the `ShellBuilder` API (later packages plug into it), the
`local_*` widgets, every place the C++ wire could not be matched, what of UI_DESIGN section 5
is missing. Then update the `gz-app` row in `docs/rust/STATE.md`.

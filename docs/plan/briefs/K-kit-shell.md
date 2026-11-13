# Brief K - the kit and the shell

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/UI_DESIGN.md` (the UI
contract you rewrite in its sections 1-5, 7 and 8), and the audit `Intermediate/plan2/ux_rest.md` sections **B.1,
B.3, B.11, B.12, B.13, D** with its pictures (`Intermediate/plan2/shots/rest/`); `Intermediate/plan2/commands.md`
section 6 (the three command notions the UI has today) and 1.7 (commands held for frames).

You own: `crates/gz-ui` (the kit), `crates/gz-app` (the shell), `crates/gz-ui-app` (Preferences and Keyboard pages
only - not the Start page), `crates/gz-log` (line levels), `crates/gz-task` (job ownership). In the surface crates
(`gz-ui-*`) you change only (a) call sites a kit API change breaks and (b) how they register menus, toolbar
buttons, shortcuts and palette rows (part 2). You do not restyle surfaces: their own streams adopt the new kit
widgets later. Other engineers work at the same time in: the load / save paths of the graph, Characters and PDA
tabs (stream A1) and `gz-ui-app/src/update.rs` plus a status-bar chip, a Help menu entry and an exit step in
`gz-app` (stream U) - keep their hooks, re-read a file right before each edit.

## 1. The kit: one widget per job, and a calmer, clearer look

**One of each.** Today four fold headers, four checks, three pick lists, two table heads and three item-row looks
live in the kit and in the surfaces' `local.rs` files (audit B.13). Build the ONE of each in `gz-ui`, complete
enough that the local copies have nothing left to offer. The surfaces delete their copies in their own streams;
you write the widgets, their doc comments and a `-kitsheet` page beside `-iconsheet` that shows every widget in
every state (it is how the other streams and the critic see what exists).

| Widget | What it is | Replaces |
|---|---|---|
| `Section` | the fold header: chevron, Strong title, an optional count, a trailing slot for + and a menu; no fill; 28 pt; open state kept by id in the layout store | `form::section`, `form::raised_section`, graph `local_fold_header`, release "fold" |
| `Check` and `Switch` | a box for "this one of several" (lists, multi-choice), a switch for a setting that applies at once; one look each, with label, disabled, mixed | `form::check`, graph `local_check`, pda "check", world "cell check" |
| `Combo` | a value with the kit chevron; opens `PickList`; never egui's `ComboBox` (`grid.rs:763`, `button.rs:320`) | every dropdown |
| `PickList` | the popup list: search with ranking (move the palette's ranker, `gz-app/src/palette.rs:159`, into the kit as `rank.rs`: word prefix, then letters in order), virtualized rows, an icon and a tint per row, a second muted line on the highlighted row, sections (Recent, Favourites, the rest) with a star toggle per row, full keyboard use, measured once when it opens | graph `pick_kind` / `local_pick_list`, mod pick list, world `picker.rs` |
| `ItemRows` | a small editable table: named columns, a cross per row, an Add row, drag to reorder | Characters' and World Rules' item rows |
| `Table` | header (sortable, resizable columns), virtualized rows, selection, a context menu slot | release "table head", `list::table_headers` |
| `Banner` | may hold a short list with an action per item | - |
| `drop` | typed drag payloads (`Payload::Asset { path, kind }`, `Payload::Files`, ...) and drop zones any surface can declare; today only folders and Screens & Music take the browser's drag (`gz-ui-pda`'s `drop.rs` is the seed) | per-surface drop code |
| `text::fit` | ellipsized text cached per (text, style, width) so a canvas does not search and allocate per frame (audit D.6) | `text::ellipsize` in hot paths |

**The look (UI 3 tokens, `theme.rs`, `metrics`).** The palette's hues stay. What changes:

- **Elevation instead of flat fills.** Shadows are tokens: `shadow.popup` (menus, pick lists, tooltips),
  `shadow.dialog`, `shadow.card` (drawers, toasts, the Task card, graph nodes). Popups and dialogs lose their heavy
  borders for a hairline plus the shadow.
- **Inputs read as inputs**: every field, combo and number box has a 1 pt `border.subtle` outline at rest,
  `border` on hover, the 2 pt focus ring; frame height 26; a field that is wrong has the error outline and its
  sentence under it.
- **Buttons**: Primary one per surface (the kit asserts in debug when a second Primary is laid out in one
  dialog footer or toolbar); Secondary gets a hairline; a toolbar toggle that only shows a panel is a Ghost
  button with a 2 pt accent underline when on - never an accent fill (audit B.1).
- **Menus**: one icon column per popup, reserved when any row has an icon or a check (`menu.rs:58-59`); a check
  takes the icon's place; shortcuts right-aligned in muted text; minimum width 200 pt.
- **Toolbars**: buttons show their label while there is room and fold to icons only under pressure, least
  important first (`toolbar.rs` knows priorities); look-alike icons are replaced (the three pyramids of the level
  toolbar are another stream's, but the mechanism is yours).
- **Sections and pages**: a page is a centred column (max 760 pt) of cards (`raised`, radius 6, padding 12, gap
  12) for settings-like tabs; a panel is `Section`s separated by a hairline. Label columns are in em, not a share
  of the width (audit E.22).
- **New tokens**: `graph.family.0..9` (ten hues of one lightness for node families - status colours mean state
  only), `wire.base / hot / selected / dim`, `pin.*`, `frame.fill / frame.title` (graph comment frames), the
  shadows. No surface writes a colour.
- **Interface Scale** becomes a real preference (Auto, 100 %, 125 %, 150 %, 175 %, 200 %; `[ui] scale`), applied
  at once; `ui.scale` stays the test command and is never saved.

## 2. Actions: the window runs commands

Today the UI has three command notions (commands.md section 6): the registry, `UiCommand` closures
(`gz-app/src/builder.rs:20`) and the `COMMAND_*` table (`gz-app/src/cpp.rs`, behind `exec_command` /
`list_commands`). Make it one:

```rust
/// Something the author can trigger by name: a menu item, a toolbar button, a shortcut, a palette row.
pub struct Action {
    pub id: &'static str,          // "graph.frame_all": stable, what a key binding and the layout store name
    pub title: &'static str,       // Title Case, what the menu and the palette print
    pub icon: Option<Icon>,
    pub command: &'static str,     // a registry command ...
    pub args: fn(&ActionCx) -> Args,   // ... and its arguments, from what is in front of the author
    pub keys: &'static [Chord],    // defaults; Preferences > Keyboard rebinds every one
    pub group: &'static str,       // "File", "Graph", "Level", ... the palette's category, the Keyboard page's section
    pub scope: Scope,              // Global | Tab(kind): where its keys are live
    pub enabled: fn(&ActionCx) -> Enabled,   // off with the reason for the tooltip
    pub checked: Option<fn(&ActionCx) -> bool>,
}
```

- `gz-app/src/actions.rs` holds the registry of actions; `ShellBuilder::action(...)`, `menu(...)` (a menu is a
  list of action ids, separators and submenus), `toolbar(...)`. The palette lists actions (ranked, recent ones
  first, the category in a muted column, project-relative paths for the tab rows - audit B.1). Keys
  (`keys.rs`): first taker wins as CLAUDE.md says, every binding an action; the fixed shell chords become
  actions with default keys so the Keyboard page tells the truth (audit B.11).
- An action runs through `Editor::call` with the UI origin, so a question still opens a dialog for a person, and
  a failure is a notice. What a `UiCommand` closure did directly to a surface becomes a `view` or `write`
  command of that surface, registered in its crate, doing the same thing through the surface's inbox.
- **Migrate every registration except the level editor's**: the shell's own, the Start page, the content browser,
  the graph tab, the mod-side tabs, previews, libraries. `gz-ui-level` and `gz-ui-viewport` keep their
  `UiCommand` / table rows until their stream rebuilds them: leave `cpp.rs`, `exec_command`, `list_commands` and
  the macros working for those two crates only, mark the module "leaves with the level's actions (stream L2)",
  and list in your report exactly what is left for it.
- Graph keys: the hardcoded `match` in `gz-ui-graph/src/canvas.rs:2150-2218` becomes actions of the group
  "Graph" with those keys as defaults (the graph stream will add more; give it the pattern).

## 3. The shell's commands, typed

Port every command `gz-app` registers (and `grizzly`'s `core_cmds.rs` except what the architect lands:
`help`, `job.*`) to cards with typed parameters (`docs/COMMANDS.md` 1-3), under the new names. Finish each rename:
UI callers, the QA suites (`tools/qa/*.py`, `tools/mcp/*.py` - names and parameter names only), the documents.

| Today | Becomes |
|---|---|
| `state`, `ping` | `state` (one orientation answer: project, scene, open tabs with dirty flags, the active tab, running jobs, problem counts, the modal on top, `window: bool`), `ping` (name, version from the build) |
| `window_control` | `app.window action=minimize|restore|maximize|resize|move|focus|state`, `app.quit` (`save`, `discard`: `destroys_with`) |
| `ui_tab` | `ui.tab action=open|close|activate|list` (`doc`, `kind`, `path`, `save`, `discard`) |
| `ui_list`, `ui_audit`, `ui_contract`, `titlebar_info` | `ui.list`, `ui.audit`, `ui.contract`, `ui.frame` |
| `ui_panel`, `ui_drawer`, `ui_dialog`, `ui_popup`, `ui_toast`, `ui_scroll`, `ui_split`, `ui_scale`, `reset_layout` | `ui.panel`, `ui.drawer`, `ui.dialog`, `ui.popup`, `ui.toast`, `ui.scroll`, `ui.split`, `ui.scale`, `ui.reset_layout` |
| `command_palette` | `ui.palette` (open, query, run an action by id) |
| `preferences`, `theme`, `shortcuts` | `app.prefs` (get / set by key; a set applies and saves), `app.keys` (list, bind, clear, reset) |
| `progress` | `ui.tasks` (the Task Center as drawn) |
| `screenshot_editor` | `shot.window` (answers the picture and `width`, `height`, `scale`, `active_tab`, `busy`, `modal`, `outside`) |
| new | `perf.stalls` (part 4) |

Units: UI lengths are points both ways (today answers are px with a `dpi`: commands.md 3c); a picture is pixels
with its `scale`. `no_window` is the code a window command answers in a run without one. Commands held for frames
keep being held (`shots.rs`); nothing answers `pending`.

## 4. What the shell must also fix

- **The stall monitor** (PLAN section 6): `window.rs` times each UI pass and each event handler; over 100 ms is a
  stall recorded with the milliseconds, the active tab and what ran (the phase - "ui pass", "event: <kind>",
  "command: <name>" when a command's UI work ran in it); `perf.stalls`; a `[stall]` log line. The first frame of
  a start is exempt. Measure with the debug build honestly: if the shell itself stalls (opening a big tab,
  first paint of a page), fix the cause.
- **The status line shows reports only** (audit B.1, D.10): `gz-log` gets a level for profiles and stage chatter
  that the bar skips; the bar reads a field updated on push, never walks the ring under the writer's lock; a click
  on the line opens the Output Log.
- **Jobs belong to something** (`gz-task`): a job is the editor's or the open project's; closing a project stops
  its jobs (audit D.13: the catalog check outlived the project and restarted). The Task Center keeps the last
  five finished rows, dimmed, with their result (audit B.3). `job.list` shows the owner.
- **Stable chrome** (CLAUDE.md memory rule: nothing in the frame moves on a tab switch): the Releases tab keeps
  its Releases button (checked), every tab kind has its own icon (Mod Settings is not a gear), tool tabs fold to
  icon-only before any tab is truncated, then overflow (audit B.1); a tool tab's empty 36 pt toolbar row is not
  drawn empty - the shell's Build Mod group stays where it is and the row holds the tab's status (for
  save-as-you-edit tabs: "Saved to mod.ltx as you edit", audit E.27).
- **The Start page's status bar** offers no drawers (audit B.1); toasts and the Task card do not share a corner.
- **Preferences** becomes pages a surface can add a group to (`ShellBuilder::pref_group(page, ...)`): General,
  Interface (Scale, Accent), Keyboard, and room for Game, Library, Graph, Viewport, Updates from other streams.
  **Keyboard**: every action, grouped, virtualized, the keycap cell is the click target ("Press keys..."), a
  hover cross clears, conflicts named, Reset per row and for all (audit B.11, D.8).
- **Blocking leftovers in your crates** (audit D rows 5, 8, 9, 10 and 12's upload-across-frames for big
  textures: give the kit a texture uploader that spreads work over frames).
- **Density**: the audit's B.12 for the frame; dialog label columns in em.

## 5. `docs/UI_DESIGN.md`

Rewrite sections 1-5, 7 and 8 to what is true after your work: tokens, the kit's widgets (one table: widget, job,
states), the frame, actions, the commands (`ui.*`, `app.*`, `shot.*`, `perf.*`). Remove every sentence about the
C++ editor, audit issue numbers and tour steps; the document describes the UI, not its history. Section 6
(surfaces) belongs to the surface streams - leave it, but fix what your changes make false.

## Proof

- Kit unit tests through the egui harness (`Context::run_ui`, as `gz-ui-graph/src/tab.rs`'s tests do): each new
  widget's interaction (PickList ranking and keyboard, Section fold state, ItemRows add / remove / reorder, the
  one-Primary assert, menu columns aligned), `text::fit` cache hits.
- `tools/qa/test_ui_shell.py` rewritten for the new names and extended: actions run from the palette by id; a
  rebinding in `app.keys` takes effect; `perf.stalls` is empty after the suite's tour of every tab kind; the
  frame's rects are equal before and after every tab switch (the tab row, the Build Mod group); menus have one
  label column (read `ui.list` of an open popup).
- Pictures: `-kitsheet` at scale 1.0 and 1.5, and the frame on Level, a graph, Mod Settings, Preferences,
  Keyboard, the palette, the Task Center - into `Intermediate/plan/K/` with a one-line caption each in your
  report. Look at each one yourself before you call it done.

## Done means

PLAN section 5 "K", every sentence; the audit's B.1, B.3, B.11, B.12 (frame part), B.13 and D rows in your crates
each fixed or answered in the report with a reason; no surface is broken (`cg.sh check --workspace` is clean,
`python tools/qa/run_all.py test_ui_shell` passes).

# Brief L2b2 - Details and the chooser; Build, Play, the menus and the level's actions

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/UI_DESIGN.md` (1, 1a,
the kit, the frame, and the Level sections), the reports of the streams you build on (`docs/plan/reports/K.md`:
kit widgets, `drop`, actions, keys, preference pages; `L1a.md`: one selection, `SceneMode`, eye and lock, the
`scene.*` commands, the property labels; `L1b.md`: the build pipeline, its steps and states, the modes' data,
`level.*`, and what it asks of you), and the audit **`Intermediate/plan2/ux_level.md`** - A (what exists), B (what
is the X-Ray SDK's), C (layout and visual defects), D (what still runs on the UI thread), E (the 25 changes) -
with its pictures (`Intermediate/plan2/shots/level/`).

The author's words: the interface still follows the X-Ray SDK's pipelines - the same placement of elements, the
same way of drawing them; cut that legacy and build it your own way. Unreal Engine 5's level editor is the
reference for how it should feel; the audit's B sections say, item by item, what UE does instead.

You own, in `crates/gz-ui-level`: `details.rs`, `choose.rs`, `lists.rs`, `doc.rs`, `motion_ui.rs`, `import.rs`,
`recovery.rs`, `toolbar.rs`, `menus.rs`, `level_cmds.rs`, `status.rs`, `cmd.rs`, `lib.rs`; and in `crates/gz-app`
the removal of the old command table (part 4). Two engineers work beside you: **L2b1** in the same crate
(`layout.rs`, `dock.rs`, `tools*.rs`, `classes.rs`, `local.rs`, `outliner*.rs`, `detail_objects.rs`,
`content.rs`: the Place panel, the Outliner, the modes' panels) and **L2a** in `crates/gz-ui-viewport`
(everything inside the viewport's rectangle). Stream S works in the Content Browser. What crosses between you is
typed and already exists: the scene's mode, the kit's drag payloads, commands.

## 1. Details, the chooser, Level Settings

- **Details** (audit B10, C): the labels, units and words L1a's table gives; a search box; sections by the kit's
  `Section`; Transform first, a Vec3 on the line under its label (the kit: `grid.rs:851`), reset arrows on
  changed values; multi-selection shows shared rows; advanced rows under the section's More; pickers for
  everything that comes from a list - **the chooser** is tall, with thumbnails and a preview for models,
  textures and sounds (audit C: 7 rows of 5458 today).
- **Level Settings** keeps only what is the level's (L1a cleaned the model): no view toggles, no tool
  parameters, no per-class switches.

## 2. Build and Play

One labelled **Build Level** split button: the click builds what is stale (`level.build`); its menu lists the
steps with their state (fresh / stale and why / failed) to build one, **Rebuild All**, and Advanced (the mod's
level layers, the folder compiler). The status bar's chip names what is stale and opens the same popover; it is
right after a restart and does not poll (L1b). Build problems are a list in the Output Log's sibling tab
**Problems**: a sentence, the object, a click selects and frames it. **Play** split: Play (builds what is stale,
starts the game), Play Clean, Play Without Building; **Stop** while it runs. The Build menu of the caption holds
the same actions; Make / Compile / Import-Export Error List are gone.

The toolbar of the level: `[ Save ] [ Build Level v ] [ Play v ] [ Stop ]`, labelled, with the rare ones (Game
Folder, Reload Configs if stream L1b kept it) under one More; the shell's Build Mod group stays where it is.
The caption's menus of the level - File (Save above Save As, icons where the toolbar has them), Edit (acting on
the selection of any kind), Scene (a short menu: what is left after the audit's C "Scene menu: 20 rows of
unrelated things"), Build, View - are lists of actions.

## 3. Nothing blocks (audit D)

D2 and D3 (the chooser's rows off the UI thread, the owner items as a `Query`), D7 (the Game Folder stat
off-thread), the Rebuild chip from stream L1b's `Query` (no polling), and every other audit D row that names
your files. `perf.stalls` is empty after: opening every chooser, selecting 3000 objects with Details open,
opening the build popover during a build.

## 4. Actions and commands

Every menu row, toolbar button and key of the level is an action over a command (stream K's `actions.rs`; its
report lists exactly what it left for the level: `menus.rs` `SCENE_COMMANDS`, the `level_cmds` `cpp_command`
rows, the toolbar's buttons). **Then delete the old machinery**: `gz-app/src/cpp.rs`, `cpp_cmds.rs`
(`exec_command`, `list_commands`), the macros, `UiCommand`, `ShellBuilder::command / menu_item / cpp_command /
cpp_subs`, key step 4 - K's report names every piece. Stream L2a takes `gz-ui-viewport` off them at the same
time; if a use is left there when you get to the deletion, move that registration onto an action yourself (that
one edit in its `keys.rs` is yours) and say so in your report. Fix the QA suites that used `exec_command`.
Your view commands, typed: `level.view` (the mode's panel state, the open popover - to look at), `ui.panel` for
your panels, `scene.choose` (opens the chooser on a property - the UI half of L1a's `scene.choices`). The old
names (`outliner_show`, `outliner_filter`, `scene_visibility`, `project_browser_menu`'s level part, ...) go,
callers fixed.

## 5. `docs/UI_DESIGN.md`

Details, the chooser, Level Settings, Build and Play, the level's toolbar and menus, in the present tense, no
history.

## Proof

- Unit tests through the egui harness: a Vec3 on one line; the chooser's list with a preview; the build
  popover's rows from a status; the menus' actions enabled by the selection.
- QA: `tools/qa/test_properties.py` and the menu / build phases of `test_scene_tools.py` and
  `test_level_commands.py` rewritten and extended: Delete with a light and a glow selected removes both (the
  audit's failing case, through the menu action); Build Level's popover lists the steps with their states;
  the Problems list selects and frames; `perf.stalls` empty at the end.
- Pictures into `Intermediate/plan/L2b2/` with a caption each in your report: Details for each kind at
  1600 x 900 and 1280 x 720, the chooser for a model and for a spawn section, the build popover, the Problems
  list, every menu. Look at each yourself beside the audit's picture of the same thing.

## Done means

Every item of the audit's B6, B10 and the Details / menus / toolbar part of B1 and C is fixed or answered in
your report with a reason; no unlabelled build icon; no `COMMAND_*` table, `exec_command` or `UiCommand` left
anywhere in the tree.

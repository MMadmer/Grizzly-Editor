# Brief: the Characters and World Rules tabs (`gz-ui-world`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md`
sections 1, 1a, 2, 4 and, in section 6, "Characters" and "World Rules" - **the contract**;
`CLAUDE.md` sections "Characters (the module's own NPCs)" (what the form binds to, what is
picked and not typed, the trade section) and "The mode's world (EditorWorld)" (the rules, three
player choices at most, the start kit, transitions); the rustdoc at the top of `gz-ui`,
`gz-app` (`Document`, `DocCx`, `ShellBuilder`, `on_ui`), `gz-characters`, `gz-world`, `gz-nq`
(`pickers`), `gz-game`, `gz-mod`, `gz-ui-mod` (`MOD_MENU` / `mod_menu_slot`), `gz-ui-previews`
if it exists (opening the model preview), `gz-editor`.

You own: `crates/gz-ui-world/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorCharacters.cpp`: the DRAWING half - the list, the form
  and its sections, the pickers (community, base, visual, icon gallery, dialog, rank, kit item:
  `ItemPicker`), the trade section, `dlg.delete_character`.
- `Source/Editors/XrECore/Editor/EditorWorld.cpp`: the World Rules tab's drawing - the rules
  table, Applies On, Playable Area, Transitions, Player Start, the Starting Kit, `dlg.empty_world`.
- Handlers (frozen wire): `characters_view`, `world_view`
  (`rg -n '"characters_view"|"world_view"' Source/Editors`). Schemas: `crates/gz-mcp/tools.json`.
- What QA asserts: `tools/qa/test_characters.py`, `tools/qa/test_world.py` (their UI parts).

## What to build

**Characters** (document kind `characters`, a singleton): a list of 260 pt with a search, dirty
dots and Template badges, then a splitter; a form of up to 760 pt: Identity, Name & Bio, 
Appearance, Profile, Dialogs, a Starting Kit table, Trade.
- **The form binds to the RAW stored value, never the resolved one**: the game falls back
  (an English install with no `eng` string reads `rus`), the form must not, or "not set" and
  "set to the same text" look alike. An inherited value shows as the hint of an empty field.
  `name` and `bio` are edited side by side, one row per language.
- **What comes from a list is PICKED, not typed**: Community (a combo over the linked game's
  communities under the names the player reads; "not set" is a value of its own), Base (the
  project's other characters), Visual (from `meshes\actors` of the linked game and the project;
  Preview opens the model viewer), Icon (the portrait shown; a gallery of portraits), Dialog
  (dialogs captioned with their first line), Rank (the linked game's rank names beside the
  number), a kit row's item by its in-game name. The lists are `gz_nq::pickers` queries: a list
  shows busy while it loads and never blocks a frame. A value the list does not hold is kept
  and shown, never corrected.
- Trade (badge "Trader" while the character keeps a shop): not a trader - a line saying so or
  naming the base's shop, Make a Trader (Change Trade over a base's), Not a Trader; a trader -
  Sells and Buys (a combo of ticks over the linked game's trade categories; a category it does
  not list stays ticked and says so), Markup, Pays, Restock, then a Stock table (Item | Count |
  Chance | remove), Add Item, Not a Trader. Money: from / to fields, empty while unset (the
  base's amount as the hint), Never runs out, a Clear icon.
- A problem shows as a sentence under its field (the code stays in MCP and the build).
  Delete... is GhostDanger and asks through `dlg.delete_character`. Unsaved edits ask before any
  switch (the kit's `unsaved_prompt`); validation runs on change, off the UI thread.
- `characters_view` opens or reports the tab (`select=`).

**World Rules** (document kind `world_rules`, a singleton; saves to `mod.ltx` as it is edited,
through `world_set`): toolbar Reload Game Data, Empty World... (confirms through
`dlg.empty_world`), the status "Saved to mod.ltx as you edit". One scroll of up to 900 pt:
Original Game (Rule | Keep | Player Decides - three player choices at most), Player Start
(Place in Scene), Starting Kit (items through the SAME item picker as the Characters tab,
money, whether the new-game screen's perks stay) on the surface; the rest under one folded
More: Applies On (chips), Playable Area, Transitions (a filterable Transition | Leads To |
Closed table over the linked game's level changers). A problems bar only when there are
problems; problems go in a drawer. `world_view` opens or reports the tab (`drawer=`).

**The Mod menu**: add Characters and World Rules through `gz_ui_mod::mod_menu_slot`.

## Rules

- Every edit is a command that exists (`characters_write`, `characters_delete`, `world_set`):
  the tabs read snapshots and queries and change nothing themselves.
- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report. The
  item picker is one public function of this crate used by both tabs.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-world -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK, on a temp COPY of a real project
  linked to the game (the game is only read): the Characters list and form on a character with
  a base and a shop, the community combo, the icon gallery, World Rules with its More open,
  the Transitions table; `ui_audit` shows nothing escaped.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> [-EditorArgs '...']` (a machine-wide lock: waiting minutes is normal). Never
  start `Grizzly.exe` by hand. If `grizzly` does not build because of someone else's crate,
  wait and retry a few times, then say so. Do not run the QA suites.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-world` to `docs/rust/STATE.md`.

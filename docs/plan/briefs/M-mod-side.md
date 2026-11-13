# Brief M - the mod side: its tabs, its commands, its jobs

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/ASSETS.md` (3-6),
`docs/UI_DESIGN.md` section 1a (the simplicity rule; Screens & Music is its reference) and the sections of your
surfaces, the reports of wave 1 (`docs/plan/reports/K.md`: the kit's new widgets - `Section`, `Check`, `Switch`,
`Combo`, `PickList`, `ItemRows`, `Table`, `Banner`, `drop`; actions; the preference pages; `A1.md`: how assets
save and follow their files; `A2.md`: the project file, the default game; `U.md`), the audit
**`Intermediate/plan2/ux_rest.md` sections B.5-B.10, D rows 4, 11-13** with its pictures
(`Intermediate/plan2/shots/rest/3*-8*.png`), and `Intermediate/plan2/commands.md` 1.6, 3a-3d for your commands.
CLAUDE.md's sections on mod releases, the Mod Browser, characters, the mode's world, PDA interfaces and screens
are the domain rules - they stay true: this stream changes the surfaces and the command layer, not what the
module is or how it is released.

You own: `crates/gz-mod`, `gz-build`, `gz-release`, `gz-publish`, `gz-catalog`, `gz-github`, `gz-screens`,
`gz-world`, the non-io side of `gz-characters` and `gz-pda`, and the surfaces `gz-ui-mod`, `gz-ui-world`,
`gz-ui-pda`, `gz-ui-release`.

**Hard, and unchanged**: nothing is ever published, submitted, scanned or withdrawn against real GitHub,
VirusTotal or the catalog - the mock under `-qa_github` only. The safety paragraphs of the bridge are not yours
to touch. The frozen release contract (descriptor, index, package layout - the game repo's `MOD_UPDATES.md`,
`MOD_CATALOG.md`) is not changed by anything here.

## 1. Commands: typed, one way to do long work

Port every command of your crates to cards (`docs/COMMANDS.md`), with the names below; delete the old names and
fix their callers (UI, the QA suites, documents). Long work is a job the command waits for (`wait=false` answers
the job; `job.*` follows it): the `pending` answers, `answer_only`, the per-area `action=status` polling contracts
and the bridge-side loops they needed are gone (commands.md 1.6 lists them).

| Command | Effect | Replaces |
|---|---|---|
| `mod.info` | read | `mod_manifest`: the manifest as data, its problems, what a build would ship / leave out |
| `mod.set` (`set: { key: value }` by section) | write | `mod_set_manifest`, `mod_exclude`, `mod_commands`, `pda_use` - for what a text edit of `mod.ltx` cannot do in one step: validation with the rules (`gz-mod`'s `rules.rs`) and the answer. An agent may equally edit `mod.ltx` and run `asset.check` |
| `mod.thumbnail` (`from`: a picture; `remove`) | write | the `thumbnail=` side of `mod_set_manifest` |
| `mod.build` (`target: game | folder | flat`, `path`, `clean`, `wait`) | write | `mod_export` |
| `mod.package` (`path`, `wait`) | publish | `mod_package` |
| `release.publish` (`action: start | stop`, notes, tag, title ..., `wait`), `release.account` (`action: status | sign_in | save_token | sign_out`) | publish / write | `mod_publish` |
| `release.list`, `release.edit`, `release.delete` (destroy) | read / write / destroy | `mod_releases` and its bridge twins |
| `release.catalog` (`action: check | scan | submit | withdraw | status`, `wait`) | publish (check and status: read) | `mod_catalog` and its twins |
| `release.key` (`action: status | create | open | lock`) | write | the key side of `mod_catalog` |
| `screens.info`, `screens.set`, `screens.import` (`path`, `screen`, `layer`) | read / write | `screens_get`, `screens_set` |
| `world.info`, `world.set` (`preset`, rules, start kit ...), `world.lookup` (`type: levels | transitions | patrols`) | read / write | `world_get`, `world_set`, `world_transitions` |
| `characters.view`, `world.view`, `screens.view`, `mod.view`, `pda.preview`, `release.view` | view | `characters_view`, `world_view`, `screens_view`, `mod_settings_view`, `pda_preview`, the `show=` parameters of `mod_releases` / `mod_catalog` (to look at, never to act) |

An effect that depends on the action is declared so the doors work (`docs/COMMANDS.md` 1): a command whose
actions differ in effect is split by effect (`release.catalog` check / status are `release.catalog_status` if the
card cannot say it otherwise - decide, and say in your report how a caller sees it in `help`).

## 1a. What the validators know, and what follows the files

- **The module's own sections.** `Grizzly-Cmd.exe check` on the author's module prints 150 warnings that are
  false: `W117`, `W121` (characters) and `W060` (quests) say "the linked game has no item 'od_...'" about items
  the module itself brings. Build ONE index of the sections a module defines - every `[section]` of the `.ltx`
  files under the project's `gamedata/configs` (and what its `mod.ltx` mounts), read off the UI thread, cached
  per project and dropped by the project's watcher when one of those files changes - beside the linked game's
  index in `gz-game`, behind one question: "does the game, with this module on, have the section X?". Every
  validator that warns about an unknown item, section, visual or dialog asks it: the characters' checks
  (`gz-characters` `validate.rs`), the quests' (`gz-nq` `validate.rs` - that one call site is yours for this),
  the world rules' start kit. A warning remains only for what neither side has.
- **`mod.ltx` through the watcher.** `ManifestService` (`gz-mod`) subscribes to the project's watcher
  (`gz_project::ProjectWatch`, `Filter::Path`) instead of re-reading the disk before every edit: an outside edit
  of `mod.ltx` shows in Mod Settings, World Rules and Screens & Music at once, and an unsaved field keeps its
  value (the manifest's editors save as they are edited, so there is nothing to merge but the field in flight).

## 2. Jobs belong to the project

The Mod Browser check survived `project_close` and restarted (audit D.13). Every job of your crates is the
project's (stream K's ownership in `gz-task`): closing the project stops it. A job that reaches the network says
in its stage what it waits for ("Waiting for VirusTotal: 2 of 3 packages"), and the Task Center's card can stop
it at any time.

## 3. The surfaces

Each surface below passes `docs/UI_DESIGN.md` 1a, uses the kit's one widget for each job and keeps no local copy
of it (`local.rs` of each crate ends with nothing the kit has - audit B.13), has ONE Primary at most, puts "Saved
to mod.ltx as you edit" in the one place stream K gave it, and has no audit defect left (B.5-B.10; each one is
fixed, or your report says why not). Beyond the defects:

**Characters** (B.5): three columns at width - the list (with a portrait thumbnail per row, a dirty dot, search,
`+`), the form (the kit's `Section`s; `ItemRows` for the kit; the inherited rows of a base shown in the same
table, muted, with "from kit_loner"), and a **preview** in the space that is empty today: the character's model
in the model viewer (`gz-ui-previews` has it - use its widget, do not copy it), the portrait large, the name and
community as the game will show them. A toolbar with **Save** (Primary when dirty), **Revert**, **Delete...**.
The portrait gallery marks the current pick and shows pictures for every sheet it lists (the audit found half of
them as text - find the cause).

**World Rules** (B.6): per rule one segmented control - Remove | Keep | Player Decides - beside its label (it
also removes the impossible state); the "n of 3 offered" count on the section's header; `ItemRows` for the start
kit; Remove Kit is an ordinary action (it is undoable through the manifest's history; if it is not, make it so).

**Screens & Music** (B.7): the reference stays as it is; fix the three cosmetics.

**PDA Interface** (B.8): the toolbar's groups get captions and gaps; the Source pane underlines the reader's
error line and puts Apply at the pane's foot beside the status; syntax colouring of Lua data (keys, strings,
numbers, comments - four token colours from the palette; a simple lexer over the visible lines, off the UI
thread for large files); the device guides only on the device.

**Mod Settings** (B.9): a page of cards (stream K's page layout); Commands as a `Table` (Name, Function,
Arguments, Cheat, a cross; Help in the row's detail), scripts read on open off the UI thread so no dot lies;
field ends aligned.

**Releases, Publish, Package, Submit** (B.10): dialogs with an em label column; Publish's Notes is a bordered
field with a hint; one statement of "not signed in" with the action beside it; Submit's prose uses the card's
width; a running job's footer is the kit's busy line, one Stop button named for the job, and Hide only where it
differs from the title's cross. The Releases tab's empty state is not a second Primary.

**All of them**: every action is an action / command (stream K's `actions.rs` pattern) with a place in the
palette and the Keyboard page; everything a person can open, an agent can put up to look at with the surface's
`*.view` command; nothing waits on the UI thread (audit D rows 4, 11, 12 in your crates; `perf.stalls` empty
after each suite).

## 4. Documents

`docs/UI_DESIGN.md` section 6: your surfaces rewritten to what they are now (no history, no issue numbers).
`docs/MOD_QUICKSTART.md`: the commands and the workflow it names are today's (files first, `Grizzly-Cmd.exe
check / build / package`, MCP for the window); it loses every tool name that no longer exists.

## Proof

- Rust tests that exist for the domain stay; the ones for removed commands go; new ones for the cards with
  effects (the doors: `release.delete` is refused through the run door, `mod.package` through anything but
  release - call through `Editor::call_gated`).
- QA suites rewritten for the new commands, each under two minutes, each ending on `perf.stalls`:
  `test_characters.py`, `test_world.py`, `test_screens.py`, `test_pda.py`, `test_build_contents.py`,
  `test_build_job.py`, `test_unicode_names.py`, `test_mod_publish.py`, `test_mod_catalog.py` (the two long ones:
  cut what repeats; the mock's waits shrink with the `GRIZZLY_VT_*` variables). A phase proves the lifecycle:
  start a catalog check, close the project, `job.list running=true` is empty.
- Pictures into `Intermediate/plan/M/` with a caption each in your report: every tab and dialog of yours at
  1600 x 900 and 1280 x 720. Look at each yourself against Screens & Music: is it as simple and as finished?

## Done means

PLAN section 5 "M", every sentence; no old command name of your areas answers; no bridge-side polling is needed
for any of them; every audit item B.5-B.10 closed.

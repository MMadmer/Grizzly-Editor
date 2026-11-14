# Report M - the mod side: its tabs, its commands, its jobs

## 1. Built

**Commands (cards, `docs/COMMANDS.md`).** Every command of my crates is a typed card; the old names are gone and
their callers (UI, suites, documents) moved.

| Command | Effect | Crate / file | Replaces |
|---|---|---|---|
| `mod.info` (`files`) | read | `gz-build` `cmd.rs` | `mod_manifest`: `{manifest, problems, functions, build: {ships, bytes, left_out, files?}}` |
| `mod.set` (`set: {section: {key: value}}`) | write | `gz-mod` `cmd.rs` | `mod_set_manifest`, `mod_commands`, `pda_use`: sections module, update, order, requires, conflicts, exclude, vars, provides_mode, pda, commands; one request, all or none, the rules of `rules.rs`; answers `{manifest, problems, functions, changed}` |
| `mod.exclude` (`path`, `on`) | write | `gz-mod` `cmd.rs` | `mod_exclude` |
| `mod.thumbnail` (`from`, `remove`) | write | `gz-release` `cmd.rs` | the `thumbnail=` of `mod_set_manifest` |
| `mod.build` (`target: game\|folder\|flat`, `path`, `clean`, `wait`) | write | `gz-build` `cmd.rs` | `mod_export` and gz-ui-mod's window-only `mod.build` |
| `mod.package` (`path`, `requires_game`, `password`, `wait`) | publish | `gz-release` `cmd.rs` | `mod_package` |
| `mod.view` (`page`, `ask_game_updates`, `dialog`) | view | `gz-ui-mod` `view_cmd.rs` | `mod_settings_view`; `dialog=build_to_folder\|export_flat\|package_release` is the Mod menu's way into those dialogs |
| `release.publish` (`notes`, `tag`, `title`, `requires_game`, `prerelease`, `latest`, `password`, `wait`) | publish | `gz-publish` `cmd.rs` | `mod_publish action=start/status/cancel`; waits for the packaging AND the upload |
| `release.account` (`action: status\|save_token\|sign_in\|sign_out\|save_vt_key\|forget_vt_key`) | write | `gz-publish` `cmd.rs` | `mod_publish action=account/save_token/sign_in/sign_in_status/sign_out`, `mod_catalog action=vt_key/vt_key_forget` |
| `release.list` / `release.edit` / `release.delete` | read / write / destroy | `gz-publish` `cmd.rs` | `mod_releases` (list, edit, delete, status, cancel) |
| `release.catalog_status` (`action: status\|check\|submission`, `wait`) | read | `gz-catalog` `cmd.rs` | `mod_catalog` status / check / submission |
| `release.catalog` (`action: scan\|submit\|withdraw`, `wait`) | publish | `gz-catalog` `cmd.rs` | `mod_catalog` vt / submit / withdraw |
| `release.key` (`action: status\|create\|open\|lock`, `path`, `password`) | write | `gz-release` `cmd.rs` | `mod_catalog` key_create / key_use / key_unlock |
| `release.view` (`select`, `edit`, `submit`, `withdraw_question`) | view | `gz-ui-release` `view_cmd.rs` (new) | `mod_releases select show=edit`, `mod_catalog show/hide confirm=withdraw` |
| `screens.info`, `screens.set` (typed actions), `screens.import` (`path`, `screen`, `layer`, `music`, `surface`) | read / write / write | `gz-screens` `cmd.rs` | `screens_get`, `screens_set` (its `drop` action is `screens.import`) |
| `world.info`, `world.set` (typed rules, lists, kit), `world.lookup` (`type: levels\|transitions\|patrols`, `level`, `query`, `limit`) | read / write / read | `gz-world` `cmd.rs` | `world_get`, `world_set`, `world_transitions` |
| `characters.view`, `world.view` | view (`discard` destroys) | `gz-ui-world` `view_cmd.rs` | `characters_view`, `world_view` |
| `screens.view`, `pda.preview` | view (`discard` destroys) | `gz-ui-pda` `lib.rs`, `screens/cmd.rs`, `pda/cmd.rs` | `screens_view`, `pda_preview` (no `answer_only`, no `pending`) |

**Long work is a job the command waits for.** `mod.build`, `mod.package`, `release.publish`, `release.list/edit/delete`,
`release.catalog*` wait; `wait=false` answers `{job}`. `ReleaseList` (`gz-publish` `manage.rs`) runs as a job of the
project (`releases`), no longer a bare task. The device sign-in is a job of the editor (`sign_in`, `sign.rs`):
`release.account action=sign_in` answers the code and the job, `job.wait` follows it. No `pending`, no
`answer_only`, no `action=status` polling is left in my crates; `gz-mcp`'s timeout answer no longer names
`mod_publish` / `mod_catalog` follow-ups.

**Jobs belong to the project.** Every job of my crates starts project-owned (stream K's `Owner`); closing the project
stops it (`Editor::set_project` -> `stop_orphans`). `gz-catalog` `install` (new): a project that closes takes the
Submit window and the results with it (`Catalog::forget`), so nothing restarts a check (audit D.13). Network
stages say what they wait for: "Waiting for VirusTotal: the report on 2 of 3 packages (od-1.0.zip, 1:05)",
"Waiting for GitHub: the submission", "Waiting for the Mod Browser catalog" (`gz-github` `scan.rs`, `github.rs`,
`gz-catalog` `jobs.rs`, `gz-publish` `scan.rs`, `manage.rs`).

**The module's own sections** (`gz-mod` `sections.rs`, new): `OwnSections` (project scope) indexes every `[section]`
of the `.ltx` files under `gamedata\configs` and of what `[vfs]` maps under `configs\`, minus what the build leaves
out; built on the IO pool, dropped by the project's watcher on any `.ltx` change, with a generation for caches.
Asked by: the characters' `W117` / `W121` (`gz-characters` `validate.rs`, `service.rs` `Checker.own`), the quests'
`W060` for item, squad and spawn sections (`gz-nq` `validate.rs` `Context.own_sections` - my one call site - and the
field in `service.rs` `ValidateEnv`, mixed into its revision), the world's start kit `W208` (`gz-world`
`service.rs`). Messages now say "neither the linked game nor the module has".

**`mod.ltx` through the watcher** (`gz-mod` `service.rs`): `ManifestService` subscribes to `ProjectWatch`
(`Filter::Path("mod.ltx")`) instead of polling the tree's revision; an edit no longer re-reads the disk while the
watcher is live. Commands that answer the file (`mod.info`, `world.info`, ...) still `sync()` once.

**Surfaces.**
- Characters (`gz-ui-world` `characters.rs`, `char_form.rs`): toolbar Save (Primary while dirty) / Revert / Delete...
  (GhostDanger; the header's Delete... is gone); a third column where the form leaves 300 pt: the model in
  `gz-ui-previews`' model viewer (its own `ModelDoc`, details pane off, embedded), "In the Game" with the portrait at
  96 pt, the name and the community caption, the base's values where the character sets none; portraits as 22 pt
  thumbnails in the list rows; the portrait gallery's tiles showed their id while their sheet was still loading
  (the audit's "half the tiles as text" was a shot taken mid-load) - a tile now shows the empty portrait frame
  until its picture is up, the current pick stays ringed, the search says "Search portraits"; the Starting Kit is the kit's `ItemRows`, the base's rows in the same table, muted,
  "· from <base>", their cross a count-0 override.
- World Rules (`world.rs`): a segmented [Remove | Keep | Player Decides] per rule beside its label (a fourth choice is
  refused with a sentence), "n of 3 offered" as the Original Game section's count, the kit as `ItemRows`, Remove Kit
  an ordinary button undone by Edit > Undo (`Document::edit`, `undo_kit`); `local_section` / `local_check` are now the
  kit's `Section` / `Check` (`local.rs`), so the More sections reach `ui.audit` (`test_world` U).
- Screens & Music (`screens/doc.rs`): captions with a gap before each toolbar choice (`caption`), Music is the kit's
  `Combo` + `PickList` (tracks ticked), the column grows to 1040 pt.
- PDA Interface (`pda/doc.rs`, `pda/canvas.rs`, `local.rs`): Aspect / Language / Zoom groups with captions; Source has
  Lua data colouring (keys, strings, numbers, comments; cached per text, one colour above 256 KB), the error line
  squiggled, the status, Line n and Apply at the pane's foot; the screen-edge guides stop at the device; the tab
  factory no longer stats the file (audit D row 4: the tab opens and says "File not found").
- Mod Settings (`settings.rs`, `form.rs`, `menu.rs`, `cards.rs`): the page is one kit card (tone strip on an error);
  Commands is an `ItemRows` table (Name | Function with its picker | Arguments | Cheat | cross) with the selected
  command's Help and its sentences under it; the scripts are read when the tab opens and a function is never called
  missing before they were (`Aux.functions_read`) - no false dot; the footer is gone (the toolbar row says how the
  tab saves; Save goes there while a picture waits); one `mod.set` per save.
- Releases / Publish / Package / Build / Submit (`gz-ui-release`, `gz-ui-mod` `package.rs`, `build.rs`): em label
  columns (8-9 em); Publish's Notes a bordered field with a hint; "not signed in" said once, in the Account row with
  its action; Submit's prose wraps at the card's width; a running Submit job's footer is the busy line and one Stop;
  the Releases empty states' actions are Secondary.

**QA suites** rewritten for the new commands and answers, each ending on `perf.stalls`: `test_build_contents.py`
(A: the new `.gitignore`; C: `mod.set`/`mod.exclude`/`mod.info` incl. `build.left_out`; H: stalls), `test_build_job.py`
(`ui.tasks running`, `mod.build`, `follow` `job.wait`), `test_world.py` (+ G: segmented rules, kit rows, Remove Kit),
`test_screens.py` (+ I: `screens.import`), `test_pda.py` (`layout_of`, `use_layout` over `mod.set`), `test_characters.py`
(toolbar Save/Revert/Delete..., + S: the module's own item is no W117), `test_unicode_names.py`,
`test_mod_publish.py` (helpers over `release.*`; I stops the upload with `job.stop`; M the device sign-in through
`release.account` + `job.wait`; modal ids short), `test_mod_catalog.py` (helpers over `release.catalog*`, `release.key`,
`release.account`, `release.view`; modal `rect`; + L: a check started, the project closed, `job.list running=true`
holds none and none starts again). Callers in other suites renamed (`bench_build`, `measure_compile`, `test_assets`,
`test_asset_icons`, `test_behaviours`, `test_graph_ui`, `test_level_commands`, `test_play_game`, `test_previews`,
`test_properties`, `test_scene_tools`, `test_stress`, `test_ui_tour`).

**Rust tests:** `gz-publish` `the_cards_keep_their_doors` (`release.delete` refused through the run and release doors,
passes the destroy door; `release.publish` and `mod.package` only through the release door; an unknown parameter is
`invalid`), `gz-mod` sections and `mod.set` value tests, `gz-ui-pda` `lua_data_in_four_colours`, the World `W208`
text, the Mod Settings plan tests over `mod.set`.

## 2. Decisions

- **Split by effect:** `release.catalog_status` (read: status, check, submission) and `release.catalog` (publish: scan,
  submit, withdraw). `help about=release` lists both with their effects; the catalog's `scan` is publish because it
  uploads the author's packages to VirusTotal.
- **`mod.exclude` stays its own command** (COMMANDS.md's area list names exclude); `mod.set set={"exclude": [...]}`
  replaces the whole list.
- **`mod.info` lives in `gz-build`** (it walks what a build ships; `gz-mod` cannot depend on `gz-build`). `mod.set`
  refuses a key it does not know with the sentence "edit mod.ltx for anything else"; `thumbnail` moved to
  `mod.thumbnail`, the PDA layout is `set={"pda":{"layout":...}}` through a `ManifestArgs` hook of `gz-pda`.
- **`mod.build` from a person in the window** asks what it needs (clean question, result dialog) through
  `gz_build::WindowBuilds`, installed by `gz-ui-mod` `cards::ensure`; over MCP / headless / commandlet it asks
  nothing. Build to Folder / Export Flat / Package Release in the menu are `mod.view dialog=` (opening a dialog is a
  view); `clean` defaults true (as every MCP build was) and stays write.
- **`release.publish` waits for both jobs** (the packaging in `build`, the upload in `publish`, so a build may run
  meanwhile); `wait=false` answers the packaging job and `job.wait` without a job waits for both.
- **Paths in `mod.info`** are project-relative with `/` (`pda_layout`, `thumbnail`), per COMMANDS 3; `mod.ltx` keeps `\`.
- **`pda.preview action=revert`** drops Source edits only with `discard` (it was the one losing path without one).
- **Characters' Template and Squad** were already the grid's switches after wave 1 - no change.
- **The model preview** is the model viewer document embedded (`gz_ui_previews::model::factory`, `show_details =
  false`, its body drawn in the column): the crate has no widget of its own to share; nothing was copied.
- **Snapshot build for the proofs:** the workspace did not compile for hours (L1a's `gz-scene`, G's `gz-ui-graph`
  mid-rewrite), so I built my state in a snapshot where exactly those crates (gz-scene, gz-entities,
  gz-export-level, gz-levelbuild, gz-ui-level, gz-ui-viewport, gz-nq, gz-ui-graph, grizzly, gz-win) come from HEAD
  (`git show`, read only), grizzly with my three registration lines; target `Intermediate\m_snap`. The `W060` call
  site is therefore proven by `cargo check` only, not in a running editor.

## 3. Contract changes

- `docs/UI_DESIGN.md` section 6: PDA Interface, Characters, Screens & Music, World Rules, Mod Settings, Build
  dialogs, Publish Release, Releases, Submit rewritten to what they are; 1a's Applied paragraph; section 7's lists of
  held commands and surface commands; section 8's local widgets of `gz-ui-pda`.
- `docs/MOD_QUICKSTART.md`: files first, `Grizzly-Cmd.exe check / build / package`, every removed tool name of my
  areas and of the removed asset CRUD replaced.
- Commands: see the table; removed `mod_manifest`, `mod_set_manifest`, `mod_exclude`, `mod_commands`, `mod_export`,
  `mod_package`, `mod_publish`, `mod_releases`, `mod_catalog`, `pda_use`, `screens_get`, `screens_set`, `world_get`,
  `world_set`, `world_transitions`, `mod_settings_view`, `characters_view`, `world_view`, `screens_view`,
  `pda_preview`; answers lost `pending`; `ui.tasks`-style polling gone from my crates.

## 4. Requests

- **Architect (`gz-editor`)**: `registry.rs` `bare_effect` still names `mod_package`, `mod_publish`, `mod_catalog`,
  `mod_releases` - dead now (every command of mine has a card).
- **Q / architect**: `tools/qa/test_mcp_bridge.py` line 176 asserts bridge tools `grizzly_mod_package` /
  `_publish` / `_catalog` / `_catalog_submit` that the six-tool bridge no longer has; `tools/mcp/AI_SETUP.md`,
  `docs/NQ_ARCHITECTURE.md`, `docs/nq/NQ_FORMAT.md`, `docs/pda/PDA_FORMAT.md`, `docs/RUST_MIGRATION.md`,
  `docs/rust/*` and CLAUDE.md still name the removed commands of my areas; the comment sweep: ~120 lines in my
  crates still explain behaviour by the C++ editor (module headers of the files I rewrote are clean).
- **G**: my two edits in `gz-nq` - `validate.rs` (`Context.own_sections`, the check in `check_indexed`) and
  `service.rs` (`ValidateEnv.own_sections`, mixed into `rev`) - are the brief's one call site; keep them through
  your rewrite.
- **K (kit)**: `grid::text_block` has no hint - Publish draws its Notes field itself for that; `state::empty_state`'s
  action is always Primary (Releases draws its own Secondary under it); `Toolbar` has no caption-before-segment form
  (Screens and PDA use `tb.widget` for the caption).
- **S (`gz-ui-previews`)**: a model view widget of its own would let Characters stop embedding the whole document.
- Not done, with the reason: the PDA sheets' upload through `upload::Tiled` (audit D row 12) - the canvas samples a
  sheet by one texture id per mesh, and tiling needs every mesh split per tile; Characters' Save / Revert / Delete...
  are toolbar buttons of the tab, not palette actions (the palette has Ctrl+S `file.save`).

## 5. Proof

The working tree did not build as a program while I worked (L1a's `gz-scene`, G's `gz-ui-graph`, L2's
`gz-ui-viewport` mid-rewrite; `cargo check -p grizzly` still fails only in those crates). So:

- **My crates in the working tree:** `bash tools/rust/cg.sh check` / `clippy --all-targets` / `test` on gz-mod,
  gz-build, gz-release, gz-publish, gz-catalog, gz-github, gz-screens, gz-pda, gz-ui-mod, gz-ui-pda, gz-ui-release -
  no warning, every test green (gz-mod 33, gz-publish 9 incl. `the_cards_keep_their_doors`, gz-release 26, gz-build 7,
  gz-github 21, gz-ui-mod 28, gz-ui-release 15, ...). `cargo check -p gz-nq` with my call site: clean. `cargo fmt -- --check`
  on all my crates: clean.
- **The program:** a snapshot of the tree (`tools`-free copy in my scratchpad: my crates as they are, the ten crates
  named in Decisions from HEAD by `git show`), `cargo build -p grizzly -p gz-mcp-bridge` into
  `Intermediate\m_snap` (kept for the architect; delete freely). In it: `cargo test -p gz-characters -p gz-world
  -p gz-ui-world -p gz-ui-pda` (16 / 9 / 14 / 11 green, incl. the own-section case of W117 and the Lua colouring),
  `cargo clippy` of the UI crates and gz-characters / gz-world: no warning.
- **Suites**, `GRIZZLY_QA_EXE=Intermediate\m_snap\debug\Grizzly.exe GRIZZLY_QA_PORT=28964`, each alone on a copy:
  `test_build_contents` 92/92 (8.5 s), `test_build_job` 26/26 (26 s; B re-run 6/6 after the `code` fix),
  `test_characters` 75/75 (7.5 s), `test_pda` 89/89 (9 s), `test_screens` 35/35 (6.6 s), `test_unicode_names` 20/20,
  `test_mod_publish` 121/121 (33 s, mock only), `test_mod_catalog` 171/171 (26 s, mock only; F re-run 28/28 after the
  `rect` fix; L - the lifecycle - green), `test_world` 77/83 on `D:\XFinedProjects\Test`: the six failures are phase C
  calling L1b's renamed `level.export`, which the HEAD crates of the snapshot do not have; U and my new G pass.
  Run side by side (`run_all.py` with three suites) two suites met one 100-150 ms UI pass under the load of three
  editors; alone they end on an empty `perf.stalls`.
- **Not proven / found:** the PDA tab's first frame with the Source pane open is a 140-300 ms UI pass on the author's
  large `shmyandex.pdaasset` (details pane alone: none). Colouring above 8 KB is laid out on a thread of its own now
  and the stall stayed, so it is the first layout of the text itself (the Mono face's glyphs entering the atlas, as I
  read it) - not fixed; it is the one stall my shots show. The `W060` own-section check runs only in the working tree
  (HEAD gz-nq in the snapshot). Nothing was sent to GitHub, VirusTotal or the catalog: the suites ran against
  `tools/qa/github_mock`, the shots against 127.0.0.1:9.
- **Pictures** in `Intermediate/plan/M/` (1600 x 900 and 1280 x 720 each, pt, scale 1.5 on this screen; `ui.audit`:
  nothing escaped, nothing truncated but the two Writes To paths of the build dialogs, which keep their end):
  - `mod_settings_general_*`, `mod_settings_mod_browser_*`, `mod_settings_commands_*` - a card per page, the toolbar
    row says "Saved to mod.ltx as you edit", no footer; Commands as one table, Help of the selected command under it.
  - `characters_*` - toolbar Save / Revert / Delete...; faces in the list; at 1600 the model and "In the Game" beside
    the form (at 1280 the form keeps the room and the preview is not drawn). `characters_portraits_*` - every sheet's
    pictures, the current one ringed, "Search portraits".
  - `world_rules_*`, `world_rules_bottom_*` - a segmented control per rule, "0 of 3 offered" on the header, the kit
    table, Remove Kit an ordinary button.
  - `screens_*` - captions before the toolbar choices, the wide preview.
  - `pda_*` - Aspect / Language / Zoom captioned groups, the coloured Source, Apply at the pane's foot.
  - `releases_*` - the tab, not signed in: its action a Secondary button.
  - `dlg_publish_release_*` (em label column, the Notes field with its hint, "not signed in" once, Publish off),
    `dlg_package_release_*`, `dlg_build_to_folder_*`, `dlg_export_flat_*`, `dlg_submit_*` (prose at the card's width,
    the busy line and one Stop Checking while the check runs), `dlg_withdraw_1600` (the sheet; at 1280 the Submit
    window had been closed by the run, so that shot shows the tab).
  Looked at against Screens & Music: Characters, World Rules, Mod Settings and the dialogs now read as one task per
  view with one Primary; the PDA tab's toolbar is still the densest surface of the set.

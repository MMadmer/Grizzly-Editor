# Grizzly Editor - state

What is proven and how, what no run has proven, and the decisions the contracts do not spell out. The contracts are
the documents `CLAUDE.md` lists; this file is the editor's memory beside them. A decision that becomes a rule moves
into its contract and leaves this file.

Version 0.7.0. Nothing of it is published: no release of the editor exists, and none is made without the author.

## 1. Proven

The three entry points on the final tree, each run bounded (`tools/rust/limited.py`):

| Entry point | Holds |
|---|---|
| `bash tools/rust/check.sh` | clippy and fmt clean; `unsafe` only in `gz-win`; one file dialog function; the editor answers `help` from a shell; every command has a card and none is registered without its type; text encodings; no test writes a program file; the release programs' icon and kind, the console twin's answer and exit code; the notices; no forbidden word |
| `bash tools/rust/test.sh --all` | the unit tests of every crate and the corpus tests over the editor's library and the linked game |
| `python tools/qa/run_all.py --with-game` | the QA suites, each on a copy of a project, on a port and a home of its own, each ending on an empty `perf.stalls`; a suite fails when an editor of it logged a graphics device error or a panic |

What the suites hold, beyond what their names say:

- **Projects stand alone** (`test_portable`): a project converted from the old layout, then moved and renamed, opens
  on a fresh home from its project file with the same id, finds the game by itself, writes nothing that names a
  place or a machine; Collect Library Assets; a second start hands its project over to the running editor.
- **Assets are text** (`test_assets`): a file opened and saved unchanged is byte for byte the same; an edit beside
  the editor is followed; unsaved edits rebase onto the file's; comments and unknown keys survive.
- **The scene's own files** (`gz-scene`'s corpus test, `test_scene_tools`): the 35 base scenes of the library import,
  are written as `.scene`, read back equal and written again unchanged; a moved object changes one line of one file;
  a file edited outside is one undo step.
- **One level build** (`test_level_commands`, `test_level_compile`): every step built as one job; the same build
  again builds nothing; the status survives a restart; a moved object makes stale exactly the steps that read it;
  Stop ends the build within seconds inside the compiler and keeps what ended (`gz-lc`'s tests stop a compile in
  every phase, and inside the lightmaps and the static models' light); an error ends the build naming the step and
  the steps not run; what the spawn would refuse is refused before any step runs; the compiler's output on the
  CPU matches its records byte for byte (`tools/qa/golden/compile`: two levels at draft, one with lightmaps), and
  the same level lit on the graphics card has the CPU's structure hash, collision and `level.geomx`.
- **The same level from a faster build**: the AI map generated over Cordon's ground has the nodes of the editor
  before the speed-up, byte for byte (2 480 021 nodes; `level.ai` without its guid); `gz-scene`'s tests pin a
  generated map on one thread and on eight; `gz-lc`'s and `gz-fmt-ogf`'s tests hold the weld, the count of
  junctions and long edges, the mend of the tangent basis, the strips, the progressive meshes' ratings and the
  vertex order to what the code they replaced gave, a compile to the same bytes on one thread and on eight, and
  its structure to not following the light; `gz-lc`'s test runs the old tracer and the 4-wide one side by side
  over a level, and `gz-lc-gpu`'s hold the card's lighter to the CPU's and its self-check to refusing a card that
  answers otherwise.
- **A level the linked game has** (`test_level_commands` G): every compile step not needed, Build Level builds the
  spawn layer, a compile step asked for by name runs and its files are the warning `level.clean` removes; Play
  plays the module - the root's `modules\<id>` with Play's Player Start, the new game's options a file of the root's
  own, no level staged, a new game on the command line.
- **The game runs what the editor builds** (`test_play_game`, `--with-game`): a new game on the built level, and
  (`--scene l01_escape`) a new game on the game's own level with the module mounted from Play's root and the
  player at Play's Player Start; the installed game is listed before and after and is equal.
- **The game attaches what the editor builds in pieces** (`test_play_game --scene l01_escape_piece`,
  `--scene mp_close_combat_cells`): an object of the module's own placed on Cordon, its piece built (lit under the
  level's shadows, the level's files untouched), shipped by Play with the module, found by the game and attached
  to the sector the game finds for it; a level of the project's own built in 45 streaming cells, staged by Play
  as its module, every cell found and attached, none refused, the level loaded, no assert in the game's log. The
  game's own acceptance of pieces (its `Run-PiecesQa.ps1`, a part of Cordon cut into a piece of 11 365 visuals):
  the level with the piece draws the frames of the level without it, the piece is released away and attached
  back, saves load across the module on and off, and four pieces damaged on purpose are each refused with a line
  of the log, the game going on.
- **Level design by an agent** (`test_level_design` on Escape; `docs/LEVEL_DESIGN.md`): `scene.query`'s actions
  answer in numbers that agree with each other - a sample of `heights` is the `ground` there, a ray from 20 m up hits
  20 m down, `fit`'s `rest` is where a second `ground` and `fit` find it standing, every `free` spot passes `fit`,
  `top` over a tree is its crown and `only` the ground under it, a heights request over 257 x 257 is refused naming
  a step that fits; `scene.objects` near, sorted, with boxes. The scene's files: a line appended to `objects.lua` is
  in the scene; the file broken changes nothing (every object kept, the `syntax` problem at its line, `asset.check`
  of the scene names it, `scene.save` answers `skipped` and leaves the file), the fixed file is followed; a group
  written with `ref` and no `members` is placed, noted (`S023`) and saved with its members; `scene.align` lands
  where `fit` said, `offset` above it, and the save changes exactly those lines. `scene.audit` finds `S201`, `S203`,
  `S204` at the object's line and nothing once fixed. A glTF written as text and an OBJ come in with
  `asset.import` (the `.mtl`, the texture asset of a picture), `model.info` answers their sizes, triangles and
  surfaces, a `parts` blockout is 6.4 x 2.95 x 4.4 m, both placed by their paths are found by `box` and by a ray on
  their surface. `shot.map`'s three modes, the same bytes twice, a legend centre carried to its pixel within one
  pixel and in its colour; `shot.viewport` from a view and from a camera of its own with the author's camera
  unmoved; `shot.models`. A terrain written into `terrain.lua` over Escape's slope: the flattened height at all 77
  samples inside, the old ground outside, a ray meets it and no old ground (the cut), an object drops onto it, no
  `S211` - raised 3 m by `base`, `S211`; its build writes `overlay.cut` (`XCU1`, one region at the terrain's centre
  and half size), `overlay.aiedit` (every edit moved), a piece that carries the cut and does not stream, and
  `mod.build` into a folder publishes a texture asset as `.dds` and `.texasset` with no source. The crates' tests: `gz-terrain` (every edit, the edge blend, the skirt, the clip rule's
  four cases with the engine's numbers, the `overlay.cut` and `overlay.aiedit` writers and readers), `gz-fmt-mesh`
  (glTF with data-URI buffers and `.glb`, sparse accessors, skins and clips through the OGF and OMF export, OBJ
  n-gons, hostile files as errors), `gz-media` (the three kinds' keys, references, checks, the bake twice the same
  bytes, the migration chains), `gz-scene`'s `sense` and `audit` tests (every action, cap and code, with a scene
  that trips each and one that does not). What one full read of the work found is fixed and held by tests: a
  change made while a part file does not read goes over the file when it reads again, and a recovery snapshot
  holds that file's objects (`gz-scene`'s service tests; the suite's phase B drives the first through the editor);
  a job's library makes its terrains from the state the job works on (the fork's test); a model ships its
  `surfaces` in the order of the baked model's children, as the game reads them; a sound's `type` is the game's
  own words; a model's document opens no file by a drive or a server, and a source outside the project is `M113`;
  the audit reads a terrain as the module's own and names one that wears a ground that is not there (`S213`).
- **The game reads a module's terrain and media** (`test_play_game --scene l01_escape_terrain`, `--with-game`): a
  yard cut into Escape's slope - a terrain placed and its edits written in `terrain.lua`, a blockout on a texture
  asset - built into the piece with `overlay.cut` and `overlay.aiedit`; the game's log says `* [cuts] l01_escape:
  1 region(s), 29 collision triangle(s) cut, 3 visual(s) cut`, `* [ai edits] l01_escape: 345 node(s) moved, 63
  closed, 0 skipped`, `* [assets] 1 texture(s), 0 sound(s), 0 model(s) described by 1 module(s)`, attaches the
  piece and plays on without a fatal line (40 of 40). A static model asset marked `authored` is the piece's and no
  object of the spawn layer; a dynamic one is spawned under its published name (`gz-export-level`'s test).
  `--scene l01_escape_piece` builds the piece at the first try (39 of 39): the build reads a library of its own, so
  a watcher that lost count of the files changes nothing under it (`gz-scene`'s `library` tests: the fork, the
  rescan that drops only what changed).
- **The updater** (`gz-update`'s end-to-end test with an HTTP server of its own, `test_update` against the mock): the
  check reads only the archive's end and its manifest; only the delta's ranges are asked for; a cut download resumes
  at its byte, also inside a record, also after a restart; a held file rolls the installation back exactly; the
  restart handshake.
- **No suite reaches a real service** (`qa.py`, `test_cli`, `gz-github`'s endpoint tests): every program a suite
  starts is a QA run by its environment, a QA run's addresses are this computer's or nowhere, its keys are
  its home's, and the client refuses an editor whose `ping` does not say `qa`.
- **A tab's save asked by a command** (`test_scene_tools` Z, `test_previews` K): `doc.save` on a scene without a
  file is refused inside a second, naming `scene.save path=`, with no dialog and no window outside; on New
  Textures that would replace a texture it answers `exists`, naming the texture and `library.import
  overwrite=true`, and nothing is imported; a check that finds nothing leaves a toast and no dialog.
- **A library save that cannot write** (`test_previews` K): a description whose file cannot be written -
  `doc.save` and `ui.tab action=close save=true` answer the failure naming it, the edit stays, and the same
  save writes it once the file can be written; an edit left with no tab open is listed by `doc.list` and
  stops `app.quit`. **A hand-over nobody answers** (`test_project_browser` U) is refused over an unsaved scene.
- **The manifest from the editor alone** (`test_cli` J, `test_assets`; `gz-mod`'s tests and `grizzly`'s
  `manifest_reference`): a project made from nothing fails its first `check` on one error that names the key,
  its values and the command; `help mod.set` is enough to set it; the second `check` is clean; `reference
  manifest` prints the page, and a key a reader of the manifest takes without its words there fails a test.
  A wrong value of each rule written into `mod.ltx` by hand is named by `check` with its line (nine errors,
  three warnings), `mod.build` refuses and `mod.info` lists them; the manifests of the author's projects and
  of the game's modules pass clean; a quest under `temp\release_stage_1` fails no check.
- **Releases and the Mod Browser** (`test_mod_publish`, `test_mod_catalog`, the mock only): the whole path, the
  doors (`release.delete` only through the destroy door, `release.publish` and `mod.package` only through the
  release door), a check that a closed project's jobs do not start again.
- **The themes** (`gz-ui`'s theme tests, `test_tour_light`): every theme of the table holds the contrast of
  `theme::min` (the text levels on the surfaces and the canvas, the tones and Info on their tints and apart
  from each other, the label on every fill, the accent fill, the selection, borders, the node families, the
  axes); `test_tour_light` (`test_tour.py --theme grizzly-light`) takes every tab kind, dialog and menu of
  `ui.contract` in Grizzly Light as `test_tour` does in Grizzly Dark, audited and pictured, with `app.prefs`
  setting the theme and answering it back.
- **The game half of the graphs** (the game repository's `tools/nq/run_lua.py test_examples.lua`, its catalog
  parity test): disabled nodes pass through, a file with a `canvas` loads.

- **The window in Russian** (`test_tour` R, `grizzly`'s `the_catalogs_are_one_sound_catalog`): every tab kind,
  dialog and menu of `ui.contract` at both sizes with `untranslated` empty and no text cut whose English fits;
  every catalog reads and holds one English per context.

- **The code scan, the network mark and a module's links** (`gz-mod`'s `code` and `rules` tests, `gz-release`'s
  `content` and `card` tests, the catalog's `catalog.py selftest`, the game's `Run-LtxQa.ps1` and
  `Run-ModsQa.ps1 -Scenario Catalog`): every file that carries code read by one scan - scripts, shader scripts,
  the strings of quests, behaviour graphs and PDA interfaces; the new blocking and review rules; the network
  rule on one list of cases in Rust, Python and C++; the link rules on one list of cases in Rust and Python
  and in the game by a module's manifest and a release's card; the card's `network = 1` and link keys in
  their fixed order; the build's, the packaging's and the catalog check's answers. In the game: every escape
  of the audit closed (the probe proves each one real in an ordinary game and closed under protection), a
  request of `xms.http` to the QA's mock and back, the refusals of its address rule, the mark of an installed
  module by its own files and of a browser card by its release.
- **Traders, loot tables, recipes and the Economy tab** (`gz-items`' tests, `test_items` phase E, the game's
  `Run-LtxQa.ps1` and `Run-ItemsQa.ps1`): the three kinds read, written and saved as the difference; composed
  after the items in the game's order; their checks; an item's `sold` / `loot` / `craft` / `lists` and a row of
  `sections` about another item asset refused by name; the tab driven over `economy.view` in English and
  Russian; the item's card showing the rows that name it; in the game - a trader's stock (and an item taken off
  a shelf), the death loot and the recipes read from their own files, an item's foreign key named in the log
  and never read. The Weapons Evolution project moved to it: 160 items rewritten (each read back and held to
  say what it said), one recipes file, the check's answer the same as before (782 files, 0 errors, 24
  warnings).
- **Items** (`gz-items`' and `gz-ltx`'s tests, `test_items`, the game's `tools/qa/items/Run-ItemsQa.ps1`): the
  item kind read, checked, resolved and saved as a difference; the module composed as the game composes it - the
  order of its files, what an overlay merge makes a section's own word, the heirs of a section made again, a
  made section that starts from the item's own - each held to a case on the editor's side and, in the real
  game at the main menu, to what a generated module makes: sections, strings, HUD, recipes, the rows of other
  configs, an overlay's own recipe in the section the items made, a parent of 600 characters, a list of 4899,
  and with no module the composed configs byte for byte what the build before read.
- **A classic mod made a module** (Weapons Evolution 2026, the project `WeaponsEvolution2026`): 781 files
  checked in 5 seconds and 289 MB with 0 errors and 24 warnings (the mod's own: motions, strings, textures it
  names and nobody has); built (9057 files); packaged in QA mode before the last fixes (10 parts, 2.2 GB, the
  content rules passing with three script lines named for review); and run in a new game by the game's
  `tools/qa/module/Run-ModuleInGameQa.ps1` in one start of two minutes - mounted with nothing refused, every
  section its 780 item assets make or change there, all 8676 first-person motions played (the skinning, bag,
  exoskeleton and kept-scene HUD sections through `-PlayHuds`), seven weapons held. Held against the original
  mod on its own (the project's `PORT_NOTES.md`, "Held against the original mod"): its 8368 files, 496 206
  resolved keys of the system config and every key of 33 configs opened by name as the real engine composes
  them with the module and without it (`-DumpConfigs`, `-WithoutModule`), 1287 icons, 9046 texts, 1182
  particle effects, every use scene and the skinning; what it found is fixed in the converter, the game or
  the project.
- **A module's content in the extension save** (the game's `tools/qa/saves/Run-ModuleSavesQa.ps1`, its Lua tests
  `test_examples.lua` (z30)): a module's objects, state and data out of the world save, back with their ids on a
  load, kept through a game without the module and through `-no_xms`, dropped by `xms_forget`; an older
  save that holds a module's spawn-layer object in the world save (made by the engine of the build before,
  the runner's `-OlderRuntime`) taken out and brought back once; a damaged length in a module's data ends the
  read of that value.
- **Radio and the streamer mode** (`test_screens`, the game's `tools/qa/sound/Run-SoundQa.ps1`, its
  `tools/radio/test_radio_scripts.lua`): the sound asset as the one place of the copyright mark; a refused,
  a replaced and a free sound started 2D, 3D and looped; the mode switched while something plays; the radio's
  pickers with a module's music, with a playlist, and with no module at all.

## 2. Not proven by a run

- The Russian of what the tour does not bring up: a notice, a toast, a confirmation, an error state, a problem
  sentence a surface shows only in a state the tour does not reach. Their words are in the catalogs as far as
  the translator's list, `tools/rust/i18n_missing.py`, finds them (a heuristic: it lists data and the cards'
  English too, which need no entry); a sentence a domain crate puts together that
  no template matches stays English, and `ui.audit` names it where it is shown.

- A real double click on a `.grizzly` file in Explorer: no test writes the user's file associations (the start
  writes them only for the author's own editor); the association itself is tested under a key of its own.
- Every question a person answers: QA editors run `-nodlg`, which logs the answer, so a dialog's buttons are
  pictured and never pressed - the unsaved-work question of a project handed over to a running editor, the
  confirmations of an import that replaces textures, of a reload over unsaved keys, of a deletion. What stands
  behind each button is a command, and the commands are driven.
- A drag with the real mouse - from Explorer onto the Start page, a project folder or the Texture Library, a tile
  into the viewport or onto a layer of a screen: the suites land drops through `viewport.gesture` and the kit's
  zones (`simulate_drop`), never through the system's drag.
- The editor's update against GitHub: by rule it only ever meets the mock.
- The card's lighter on any card but this machine's (an NVIDIA RTX 3070 Laptop, on Vulkan): an AMD or Intel
  adapter, DX12 alone, a software adapter. What holds there is the design - a check against the CPU on a sample of
  every scene, and the CPU taking over on a difference, a lost device or a limit - and `gz-lc-gpu`'s tests of that
  path with a lighter made to fail.
- Pieces in the game beyond a load: a piece streamed out and in many times in one long session, frames compared
  on the DX12 and Vulkan renderers (compared on DX11; the three share one loader), a piece's implicitly lit
  terrain in the game (its texture is described as the terrain's own is; no run drew one).
- The sound preview's playback in the chooser.
- Level design, what no run has shown: a skinned or animated model asset in the game (its OGF and OMF are tested
  by reading them back, never drawn by the game); a sound asset's description in the game (the log counts
  `0 sound(s)` in every run so far); a terrain on a level of the module's own in the game; a cut level's frames
  compared on DX11, DX12 and Vulkan (the game's loader is shared, the frames were not compared); a terrain that
  turns (`rot`) and a cut whose region is turned, in the game; grass on the new ground; the window's side of the
  media assets (drops from Explorer, the Place panel's model assets) beyond what `test_tour` and
  `test_asset_icons` reach. What the full read of the work did not cover: `gz-scene`'s `edit.rs` and `props.rs`,
  the renderer's side of the picture camera, the content browser's companions through copy, move, rename and
  delete, a skin's bone count against the engine's limit, the whole of `gz-luadata` against the game's reader.
  A restored recovery snapshot saved over a scene whose file does not read: the snapshot and the save's plan are
  tested, the restore's own step of reading the files on the disk is driven by no suite. One run of the suites
  side by side had a drop onto a terrain land nothing and a chooser list empty; neither came back in four more
  runs, and a drop now works itself out again when the library moved under it.
- What the manifest's reference says a key of `[world]`, `[start_kit]` or `[screens]` does in the game: it is
  read from the readers' code and the game's document, not from a run of the game. `mod.package` was seen
  refusing a manifest through the check on one wrong tag; the other errors take the same path.
- The window's questions over unsaved work - Exit, Close Tab, a tab that switches its file, Switch Project -
  and the Save dialog of a scene that has no file, picked or cancelled: a person's clicks and the system's
  dialog, which no suite answers. The commands behind them are driven (`test_scene_tools` Z, `test_pda` H);
  the window's own paths are held by the code they share with them (`save_tabs`).

- Items and the converted mod, what no run has shown: a weapon of the module reloading, firing and aiming; a
  craft, a start kit, an upgrade, the death loot and a trader's stock in play; a use scene by a real use (the
  scenes' motions were played, the item was not eaten); the first-person death; a save with the Weapons
  Evolution module itself; the original 0.98b reading a save a module was in (there is no original engine on
  this machine: the world save is read back by a script the way that game reads it); the streamer option
  switched from the options window; the first-person picture of the Items tab against a frame of the game
  (in a narrow window - 1280 x 720 - the preview column gives way to the form, as in Characters, so there is
  no picture to look at there). The full read of the work did not reach: the quest runtime's
  changes for saves, the game's screens code, the three stock script trees against the save wrappers, the
  Items and Sound tabs' per-frame work and save round trips, the picture's maths, the scene radio and `S214`,
  the references `ITEM.md`, `SOUND.md` and `UI_DESIGN.md` against the code, twenty items compared three ways
  by hand (the converter's own check compares every section and key), the converter's `items.py`,
  `compose.py`, `particles.py` and `media.py`.
- The game's list of its own copyrighted music (`dead_air_x64_sound_rights.ltx`) was put together from what the
  files and their names say; its author has not read it.
- A hostile save: a module's object record and spawn range are checked before they are used, a kept task
  record is read as the original game reads its own, with no check.

## 3. Decisions

### Commands

- Where an area's calls differ in effect they are separate commands: `release.catalog_status` (read) and
  `release.catalog` (publish). The catalog's `scan` is a publish call: it uploads the author's packages to VirusTotal.
- `level.show` is the Level tab's windows, popovers and the maximised viewport; `level.view` its Place, Paint and
  Zones panel and the Outliner. A file dialog is a person's: `level.show dialog=open_scene` from a caller is refused,
  naming `scene.open path=`.
- `app.keys action=resolve` proves a binding through the lookup the key pass uses; nothing sends synthetic keys.
- An editor without a window has no tab to hold an edit: `graph.edit` and `graph.arrange` save there at once.
- `mod.build` cleans by default; from a person in the window it asks what it needs (the clean question, the result
  dialog), from a caller nothing. `release.publish` waits for both of its jobs, the packaging and the upload.
- `mod.info` lives in `gz-build` (it walks what a build ships); `mod.set` refuses a key it does not know with "edit
  mod.ltx for anything else".
- A tab's save knows who asked (`ShellRequests::by_caller`). A person may be asked; a command is refused in a
  sentence that names the command to use, and a save that goes on after the tab's `save` returned - a write on
  the pools, a job - leaves its failure for that command's answer (`Document::save_failed`, with its code).
  Whatever a tab raises all the same during a command's save gets the logged answers (`dialog::answer_as`):
  No for a question, never Yes - what loses data comes through the destroy door.
- A tab that writes on the pools (the PDA source) stays dirty until its file holds the text, and the write
  takes the file and the text as Save found them: by the time it runs the tab may show another file.
- The window's Exit with Save waits for every save and then ends the editor; before, it ended under a write
  that had only begun, and under the Save dialog of a scene without a file. The commands do the same
  (`app.quit save=true`, `app.switch_project save=true`, `ui.tab action=close|open save=true`): each waits in
  `save_tabs` until the tabs are clean or one of them answers why not. So do the window's other three
  questions (`save_then`): Close Tab, a tab that switches its file, Switch Project. A person's save has no
  deadline - the Save dialog of a scene without a file is theirs to answer - and the dialog cancelled is the
  save's answer (`SAVE_CANCELLED`, left by a guard the kit drops with the unrun continuation), which ends
  the wait with nothing said.
- `app.switch_project` is `project.close` with the caller's own answer. It had its own list of unsaved tabs,
  which left the Level tab out, and closed the project with `discard`: the unsaved scene was dropped by
  `save=true`, by no answer at all, and by the window's question, which did not name it.
- `-nodlg`'s logged answers are for the window's own actions (`Origin::Unattended`). `project.close`,
  `project.open` and `app.quit` took the flag for a command's caller too and dropped unsaved work without
  `discard` on such an editor - every QA editor is one; a caller is refused with `unsaved` now.
- A tab that learns of its save only as it draws (Mod Settings, Characters) takes the save's answer when the
  shell asks `save_failed` too: a tab that is not the active one is not drawn, and its save would never end.
  A save under way counts as unsaved (Characters).
- `test.sh` runs only what cargo built as a test. A package with an integration test makes cargo build the
  package's programs as well, and the runner took every executable of the build: it started the editor itself,
  with a window, on the author's home (seen once, on the day `grizzly` got its first integration test). An
  example is no test either.
- `file.create` makes the folder it is asked to put the asset into, the way `file.mkdir` makes one: its own
  example works on a new project. It refuses a folder the project never lists, a device's name and a file in
  the way; it takes a name no file of the kind has anywhere in the project (the asset's id comes from the
  name: two quests made in two folders were one id); and the file passes the write guard where it really
  lands - through a link in the project it reached the linked game.
- The manifest's check reads the values from the reference's own rows: one table says what a key takes on
  the page, in `mod.set`, in the form and in the file's check. Every written line of a judged key is judged,
  not only the last of a repeated key. What the editor refuses to save is an error; a key or a value of an
  area's section that no reader takes is a warning (the module still builds); `[module]`,
  `[provides_mode]` and the sections kept as written get no unknown-key warning; `Story` among the tags is a
  warning (`mod.set` writes it in lower case). `disable_game_updates` takes what the game takes: `true` or
  `1` is on, and no other word is accepted. A build's refusal lists every error of the manifest.
- A target and a mode that do not go together are an error at the `target` line - the key that states what
  the other contradicts. `new` with a `[provides_mode]` goes with or without a `mode`; `default` with a
  `[provides_mode]` is not refused (the page pairs `default` with no mode key only). `mod.set` refuses a
  contradiction and writes a choice whose detail is still to come: the form's choice sticks before its
  detail is picked, and the check says what is missing.
- A value the game clamps or reads as another (`opacity = 7`, `cheat = maybe`, `api`) is a warning, an empty
  `version` an error; a section id the game would drop (`[screen.Bad Id]`) is a warning with the header to
  write. A key of `[module]` / `[provides_mode]` that is a known key in another case, or one edit from one
  of four letters or more, is a warning naming it; any other unknown key there stays unreported.
- An area's problem says which section and key it is about (`gz_mod::Spot`) and the manifest's check places
  it: a layer's missing file at the layer's line, a track's at its `file`, the start kit's at its row.
- `check` walks what the project lists: the editor's own folders at the root (`temp\`, `releases\`) are
  left out unless asked for by path, so what a crashed packaging left is no problem of the project.
- The manifest's reference is one page of `gz-mod` (`reference.rs`), asked by name before the kind table:
  `[world]`, `[start_kit]` and `[screens]` are described there and held to their readers by a test of
  `grizzly`, which sees every crate - not by a hook each of those crates would have to offer. The first
  problem of a new project names the key, its values and the command, and the window last.

### Assets and projects

- The default game is `<home>\machine.ltx`, not the settings file: headless runs and commandlets need the game
  without the shell, and the settings file is written whole from the window's memory. The same file keeps the
  library folder and the association switch.
- One editor per home, `-home` included: the handover is decided by the home's `editor.lock` alone. An update's
  restart (`-updated`) never hands over - its predecessor still holds the lock.
- A project's own game wins only while it is a game: a drive not plugged in leaves the project on the default one.
- The project file's `scene` is written at close and only when a scene was open. The guard of a scene load that
  crashed (`[scene] loading`) is this machine's state, so the tracked project file is not rewritten before a load.
  A scene is one of the recent ones from the moment it opens.
- The open tabs are a file of their own beside the project's state (`<id>.tabs.ltx`): one file, one writer.
- What the author places from the library is copied in as objects (a group is expanded when placed); the project
  file's `library` list is a union kept at each save, pruned by Collect.
- `GRIZZLY_GAME_SEARCH` is the QA seam of the game finder: `home` (only what this home knew), `installed:<folder>`
  (stands in for the installer's entry).
- Two saves of one file asked for close together (the settings, the open tabs) each write through a temporary file
  of their own, and the file ends as the last text asked for (`gz_editor::fsx::InOrder`).

### The scene

- The X-Ray SDK scene's scene-wide Snap Objects list is not imported: its users were the placing snap and the
  wallmarks, which take the whole level now. The AI map's and the detail objects' own ground lists stay.
- A spawn point is written as the named properties that differ from a new entity of its section, and proven per
  save: written, read back and made into an entity, they must give the very packet - or the spawn keeps its packet
  as `raw`. An entity of an SDK text scene whose class the linked game does not know never had a packet: it keeps
  the section's numbered `values` and is decoded when a game that knows the class is linked.
- Names are unique in any case and written in the case given. A number is the shortest text that reads back to the
  same value.
- Saving a scene as another scene that is there, converting over a `.scene` that is there and discarding the
  recovery snapshots lose files: each passes the destroy door alone (`overwrite`, `action=discard`).
- An exported model's description says the editor and its version built it, never the computer or the user:
  the file ships, and the same export is the same bytes on every machine.
- `scene.way`, `scene.shape` and `scene.fog_group` are commands of their own: they change several objects or
  sub-objects at once, which `scene.set` (one object's keys) cannot say.
- The detail objects' drawing is switched on by the strokes that lay them, not by a step the author has to find.
- A light has no "lights the viewport" switch: the viewport's light is the view mode's.

### The level build

- There is no collision step: the game's collision is the `light` step's output, the module's `level.export
  kind=collision`. Geometry and light are two steps, so a change of the compiler's switches or a Stop inside the
  compiler keeps the build project fresh. The game files are a step of their own: a moved spawn point never makes
  the compile stale.
- The module's overlays and the cut are exports, not steps: they take the objects the author names. The spawn layer
  covers the whole scene and is a step.
- A step's fingerprint covers the objects of the classes it reads, its layers and settings, the request (quality,
  the compiler's switches, the linked game) and the models the scene's objects are made from, by their files'
  contents: a model edited outside makes geometry stale, and the status says so.
- `standard` quality is the level's own settings; the status answers what a plain build would do, so after a draft
  build it says "Built at Draft quality."; Play builds at the quality and switches the level was last built with.
- The build regenerates only an AI map it generated itself; one made or edited by hand is kept. Generation starts
  from the spawn points, so no hand-placed node is needed.
- An error stops the build, the way it stops one in Unreal: the step that failed ends the job at once, the steps
  that finished stay fresh, and the answer names the failed step and the steps not run. A warning never stops it,
  and a step whose result is usable reports a warning, not an error. What a later step would refuse and can be
  seen at once - no spawn point, no actor or more than one, two objects with one story id, no graph point, no
  ground for a generated AI map, a scene that fails its checks (the visibility cells completed first, the same
  completion the geometry step then uses), the compiler's options, an input file no step of the build makes, the
  linked game's `all.spawn` - is checked before the first step runs, and the build refuses in seconds with every
  such problem on the object it is about.
- Stop ends a build within about a second in any phase: every long loop of the steps (the AI map, the game files,
  the game graph, the spawn, the geometry, the details) and of the compilers (each phase of `gz-lc`, the light,
  the details' light) checks the job's cancel flag per item or per chunk; the step in progress is discarded, the
  compiler's `.lc_new` / `.lc_old` go, and the job ends "Stopped.".
- The compiler's phases have plain names ("Lighting the lightmaps", "Packing the lightmaps"), and a phase's own
  stage and count ride under its name ("Lighting the lightmaps: 1200 of 5400"), so a sub-stage never hides the
  phase. Lines and stage reach the Output Log and the job at most four times a second, and a timer passes what is
  pending even while the compiler is silent.
- A stock level - one the linked game's game graph lists (`gz-world` reads `all.spawn`) - is the game's: the module
  adds to it, so its compile steps are "not needed" for the module whatever the record says, and Build Level
  builds the spawn layer. Build Step / Rebuild All compile one on purpose and their output ships. Files a build
  left in a stock level's folder that the game reads are a warning ("The module replaces 9 of l01_escape's own
  files: ..."), fixed by `level.clean`, which removes what the build wrote: the files its record lists, or, for a
  record older than the list, the names the steps make.
- What a level folder holds for the build alone never ships: `build.prj`, `build.state`, `build.lights`,
  `build.cform`, `build.details`, `build.err`, `level_stat.tga` / `.txt` (`gz-project`'s `rules.rs`,
  `is_level_build_file`) and the compiler's stage folders (dot names).
- Play on a stock level runs no step: the module is built the way a module build builds it, into the root's
  `modules\<id>` (the marker names it, `module = <id>`, before it is written; a later Play removes a module its
  marker lists that it does not install again), the game's gamedata mirrored without a staged level or
  `editor.spawn`, and a new game started (`-start server(all/single/alife/new) client(localhost)`). The actor
  starts at the module's Player Start; without one in the scene, Play writes one into its copy of the module
  where the camera looks (the ground under the view's centre, else the camera's point) and drops the module's
  other starts there. A new game takes its modes from `[character_creation] new_game_<mode>_mode` of
  `configs\axr_options.ltx`, which the new-game screen writes: Play writes its own copy into the root (the
  player's, its mode keys replaced), so a module that targets `existing` or `new` plays its mode and one that
  targets `default` or `any` plays the plain game - nothing of the game had to change for it. Files a build left
  in the level's folder would ship in Play's copy and replace the game's own (an AI map its cross table does not
  know stopped the game, seen live): Play refuses while there are any, naming them and `level.clean`.
- Play's gamedata is a mirror of the game's: hard links where the volume allows, copies otherwise.
- The compile's order is structure first: the geometry is read, welded, cut into subdivisions and merged into
  visuals by material before any light is computed (a visual is closed at a quarter of a lightmap page of chart
  texels), so no light value decides what is merged, and a lightmap page that came out darker never changes a
  mesh. The pages are packed in two levels - a visual's charts into one rectangle, the rectangles into pages by a
  skyline. A lightmap that came out flat is resampled smaller in place (it is not lit again), and that is the one
  place the light reaches a layout: the pages, the charts' places and the lightmap coordinates are the light's,
  like the pixels, and may differ between the card and the CPU by a chart that one of them found flat. The
  structure's hash leaves them out and nothing else.
- The baker is the one place light is computed (`bake.rs`): 32-byte samples in, colours out. The CPU's lighter
  walks a 4-wide tree with the same hits and the same bytes as the tracer it replaced; the card's is a compute
  shader over a tree of its own with 8-bit planes, fed in batches sorted along a curve, on any DX12 or Vulkan
  adapter - no ray tracing hardware and no experimental feature of the graphics runtime is needed, so every card
  the editor draws on lights. It is checked on 512 samples of every scene against the CPU and gives way to it on
  a difference, a lost device or a limit. `auto` is the default everywhere; a build that asks `gpu` on a machine
  without a card is refused before anything is built.
- What was looked at and not built: lightmaps on the second page alone (the engine's forward pass reads the
  first), adaptive sky sampling (the target was met without it), sectors and portals inside a piece, streaming
  of collision, the AI map, details or occluders, a new lightmap format. Each would be a change of the game's
  contract or of how the light looks, for seconds.
- A piece is the picture only. On a level the linked game has, the `piece` step compiles the module's own static
  objects with every other static object of the scene as an occluder (`build.prj`'s chunk of occluders), so they
  are lit under the level's shadows and the level's own files are never written; the game finds the sector of a
  piece's root by its place. On a level of the project's own, Streaming Cells cut by the visual's box: one larger
  than a cell on x or z stays the level's, the rest goes to the cell its centre lies in; a LOD larger than a cell
  stays with its references, a smaller one is dropped and its references go by their own boxes. A cell's pages
  are the smallest powers of two that hold it (from 64); the level's own stay 1024.
- The compiler's moves of its files into place take a few tries, under a second in all: a scanner that reads
  what the compile has just written holds it for a moment, and the system refuses the move meanwhile.
- `build.report` is written beside `build.state` by every build and never ships; `level.inspect` reads a folder
  whether a project is open or not and writes nothing. A stopped build's report holds no counts: nothing is read
  back after a Stop.
- Where the light is baked is no part of what was built: `-light=` is kept in the record (Play lights a stale
  level where it was lit last) and out of the steps' fingerprints, so a level built with `light=cpu` is fresh for
  a plain build.
- Streaming cells are built for a level of the module's own and a game that states `level-pieces`; a level of the
  game compiled on purpose, or a level for a game that reads no pieces, is built whole with a warning that says
  which. A cell is 8 to 4096 m.
- The compiler's swap of its files is undone, not abandoned: a compile that starts (or a Stop) after a crash in
  the middle of a swap puts back what the swap had moved aside, over what it had put in, and a failed swap does
  the same before it answers; a stage is removed without that only when the swap marked itself done. The level's
  files and its cells are still two swaps, one after the other.
- The piece step writes `overlay.xcform`'s triangles from every static object of the module's own, so a hand
  export of the collision overlay is replaced by the next build; it warns when `overlay_visuals.ltx` lists
  exported visuals, which the piece would draw a second time.
- The card's walk keeps at most 63 nodes (a tree that needs more is lit on the CPU), and its tree leaves out a
  triangle with a corner that is no number, as the CPU's does.
- The suite that starts the game takes a line of a library the game loads for what it is: the upscaler's
  `[Streamline] ... [error]` about an update its vendor's service refused is no error of the game's.
- What the build costs, measured on one machine (an i7-11800H with an RTX 3070 Laptop, release programs, one
  run at a time; `bench_level.py`, `lc_bench`), before this architecture and with it:

  | What | Before | Now |
  |---|---|---|
  | Cordon at the level's own quality, the whole build through the editor | stopped at 600 s, 1254 of 2575 static models lit, the lightmaps not begun | 116 s |
  | the same, the compiler alone | - | 114 s: 111 million samples lit on the card in about 80 s |
  | Cordon at Draft, the compiler | 38.3 s | 20 s |
  | The monolith's control room (l12u) at its own quality | 43.0 s | 10.1 s on the card, 17.8 s on the CPU |
  | The AI map over Cordon, 2.48 million nodes | 12.4 s | 7.5 s, the same nodes |
  | The CPU's tracer on Cordon, samples a second | 43 000 | 137 000; the card 950 000 |
  | The module's piece of Cordon for one placed object | - | 14 s |

  What stayed under twice as fast, and why: the AI map's generation is ten million tests of the ground and of a
  step, a few microseconds each, and the nodes keep the order the saves know them by - what is left is the count
  itself; the tessellation phase's sky at the vertices (4 s at Cordon's own quality) is traced on the CPU; the
  collision's simplification (3 s there) contracts one edge at a time by its error. Each is a few seconds of a
  build that is no longer minutes.

### Graphs

- "Never under 11 px" is measured on a line's box. Below it a node is its title at a fixed size, then a block in
  its family's colour: Frame All frames the whole graph whatever its size.
- A file without `flow` that has positions flows down (it was laid out that way); a new one takes Preferences >
  Graph > Flow, and `flow` is written once its nodes are placed.
- Bookmarks are named and the file's; Ctrl+N goes to the bookmark named N, else the N-th.
- A disabled node that is passed through is not done: nothing of it reaches a save, so enabling it later needs no
  migration. Its way on is the first output that is not a failure, in catalog order.
- Pin names live on the wire's first stretch; a node's body holds no pin row.
- The picker's Recent is per project and its Favourites one list for every project, both in the layout store.
- Section folds of the Graph panel are shared by every graph: per-file keys would grow with every file opened.

### The mod side

- Every job of the mod side belongs to the project; the device sign-in is the editor's. A project that closes takes
  the Submit window and its results with it, so nothing starts a check again.
- The module's own config sections are indexed (`gz-mod`'s `sections.rs`), so an item, a squad or a spawn section
  the module itself brings is no "unknown" warning in a character, a quest or the start kit.
- The model in the Characters tab is the model viewer's document embedded, not a copy of it.

### Items, saves and classic mods

- An item's composition is the game's code ported, and where the two differed the editor's check lied; each
  difference found - the order of a module's files, what a merge makes a section's own, heirs by name - is now
  one rule with a test on the editor's side and a line of the game's document.
- The form's check composes the whole module again for one edited item: the files are read once and kept, one
  check runs at a time. Keeping the composition up to the edited item would make it a fraction of that; it is
  not done.
- The check of an item's motions (`I419`) reads the item's HUD model alone. A hands motion a key names is not
  looked up in the hands rig: the game's module runner plays every motion of every item and names the ones
  that are not there.
- A build does not warn when a module's motion file stands on a game path other game models use: the converter
  settles it for a classic mod, and a module made in the editor ships its motions under its own names.
- What a conversion needs is the converter's (`tools/classic_mod`), never a script left in the converted
  project: the project holds its notes, the record the converter wrote, and the two helpers of its own
  hand-made step.
- The Sound QA of the game takes its sounds and effects as arguments: a tool of the game names no project.
- **Protected mode had ways out, and they are closed** (2026-11-17; the game's MOD_CATALOG 9.2). An audit
  of the game's protected mode found: LuaJIT opening the FFI library by itself at a 64-bit literal (`1LL`)
  and entering it among the loaded modules, so `require("ffi")` answered native code; `string.buffer`
  handing out raw pointers; `login_manager:forgot_password` passing a script's text to a command
  interpreter; `io.open` taking the C runtime's mode letters, of which `D` deletes a file opened to read -
  the catalog's state among them, which switched protection off at the next start; `package.path`,
  `package.searchpath`, `getFS():exist` and `file_length` asking the disk about a network path before any
  rule; `ini_file` reading above its folder; console commands with a path the list of names did not know.
  Each is closed in the engine, proven real by the QA's probe in an ordinary game and closed under
  protection. The editor's and the catalog's scans refuse what could only work around the mode.
- **The network is allowed and said out loud** (2026-11-17). Scripts had no way to the network in protected
  mode, and the catalog promised there was none. Now the game has one request API for scripts
  (`xms.http`: public https only, nothing of the player sent, sizes bounded), a mark worked out from a
  module's own files by one rule on the three sides, and the API answers only while an enabled module
  carries the mark. The editor states the fact in a footnote and an answer field, never as a problem.
- **An item said what was another thing's, and that was the mistake** (2026-11-17): `sold`, `loot` and `craft`
  wrote a trader's shelf, the death loot and recipes from inside an item, and `lists` any line of any config.
  They are gone from the format; the three relations are kinds of their own (`.tradeasset`, `.lootasset`,
  `.recipeasset`: the side that owns the rows says them), an item's scene carries its own section's keys
  (`fit`, `set`), and an item only shows where it is found. The format was never released, so there is no
  migration step: the reader refuses the four keys by name (`I432`) and says where each belongs. The rule that
  came of it is written where every later kind meets it - `docs/ASSETS.md` rule 9, CLAUDE.md "Mod philosophy".
- What no file of these kinds says, on purpose: which goods category of the game's trade presets an item is in,
  and what a trader pays and asks for it. Both are lines of the game's shared price lists, patched in the open
  (`.ltxp`); a typed place for them would need the game to resolve a trader's price sections when its config is
  read, which the patch machinery does not do today.
- The Economy tab's item picker offers the module's own items beside the game's (the roster's new items); the
  Characters tab's picker still lists the game's and the module's config sections only.

### The updater

- The installed version is the installation manifest's, not the compiled one: under QA an installation moves
  through versions while the program stays.
- `update.apply` verifies and replaces inside the command, so a failure is its answer and the editor runs on; the
  window's Restart and Update and Update on Exit apply after the window and the project closed, and a failure there
  is said by the next start. No busy sheet at exit: the transaction is renames.
- State `manual`: an unknown manifest schema, or an installation folder the editor cannot write.
- A record in flight keeps its raw bytes (`.raw`) beside the unpacked ones (`.part`): a deflate stream cannot be
  resumed from its output.
- Check at Startup lives in `<home>\update\settings.ltx`: the domain decides it without the window's settings.
- Left out of a package: nothing the editor reads. Kept: `fs.ltx`, `fsgame.ltx`, the bundled quest catalog, the PDA
  reader and its sheet, `gamedata\textures\ed`, `rawdata\objects\editor`.

### The window

- The window's graphics surface is made on the UI thread - the window's handle is that thread's - after the
  device was made on a thread of its own: about 0.4 s of a start in which the window is shown and not yet
  drawn. What only reads the editor's state (`ui.tasks`, `job.*`, `state`) answers meanwhile; what needs a
  frame waits for the first one.
- A thread asks the window for a frame with one message at a time (`Wake`, and the message that a later
  frame was asked for): a job that pings a thousand times a second fills no queue.
- A stall's record says how much of it the UI thread spent on the processor: a stall with little of it was
  a wait, not work.
- A pool thread that uploads to the GPU does it at the drawing threads' priority: the graphics runtime puts
  every use of a queue behind one lock, and a holder the other pools had taken the processor from kept the
  window's frame waiting.
- No frame makes a thread: making one is a system call, and a virus scanner held it for three seconds on an
  editor it had not seen before. The read-back of a pictured frame is polled by one thread made with the window;
  a thread a surface needs on first use (the sound player's, the renderer's start, the image pass's, the file
  dialog's) is made by a pool thread (`gz_task::Executors::thread`).
- A theme changes at once, so nothing drawn once keeps a theme's colours: the graph's scene keys on the palette,
  the model pictures of the content browser and the chooser on the colour they stand on (a new theme draws them
  again), the viewport's background, grid and Wireframe edges are read from the palette every frame. A published
  palette is mapped onto the roles by one rule and moved, hue kept, where it falls short of a contrast; Grizzly
  Dark is written out by hand, and some of its values moved to hold the same rule (muted 938f9e, primary
  hover 726ab6, error f5838c, required d29c69, the scroll bar's handle from the text colour to border.strong).
- A count that is not known yet is no number: a Place category shows a spinner until its list is read, and the
  busy mark is what `ui.audit`'s `loading` reports - a kit test holds every busy mark to it, `test_ui_shell`
  sees it non-empty while a project opens.
- A pick list of a surface is put up by that surface's view command (`show=`), not by `ui.popup`: the command
  opens the tab, the page and the fold that hold it. `ui.contract`'s `view_popups` names every one.
- The viewport labels the nearest 256 of what is in view, each laid out once: a marquee over a whole level selected
  its way paths, and the frame that named every point of them spent 80 ms on text.
- A stall's stack is read at half the limit: the middle of a pass that works, the same place in one that waits.
- `ui.audit` answers `loading`, the busy marks a frame shows (a busy line, a card's, a dialog footer's), and
  `errors`, the error states it shows in place of content: a caller that judges or pictures a surface waits for
  an empty `loading` first, and the tour does and fails on an error state - it found the stall above by staying
  on a tab until it was ready.
- A library is never a game: its writers pass a guard built from every game this editor knows, and a game's
  folder is refused as the library's. A game is what holds the engine's program or `database\`
  (`holds_a_game`): `fsgame.ltx` and `gamedata` are the editor's own layout too, so another editor's folder is
  taken as a library, and a library folder that is a game is not written even when this home never linked it.
  A synchronisation asks nothing (it loses nothing); retiring the game
  sounds that lost their source and rewriting the descriptions are asked by the window's action, never by
  the command.
- A library's save drops only what it wrote. Its job ended well over a description it could not write and
  emptied the store: the tab was clean, the command said ok, and the edit was nowhere. The entries that
  failed stay unsaved and are the save's answer; a written texture's gloss note is a note, not a failure.
- The libraries' pending edits are unsaved work of the editor's own (`LibraryDirty`, a `DirtySource`): an edit
  `library.set save=false` leaves with no tab open is listed, saved by `save_all` and stops `app.quit`. The
  library tabs report their edits there (`dirty_elsewhere`); the pictures New Textures holds are the tab's.
  A project's close refuses over them too and `discard` drops them: they are not the project's, but a close
  that left them pending under a closed tab would hide them.
- A save that finds another library job running (a rebuild, an import, a save of one entry) starts when that
  one has ended: hung on that job's end it stayed unsaved for ever, and a person's wait has no deadline.
- The Mod Settings picture's save is unsaved work until it is answered, and a failure puts the waiting
  picture back: the tab read clean while `thumbnail.dds` was still being placed.
- A commandlet that ends with unsaved work ends with 1 and names it. The other rule - save the scene by
  itself, as a graph without a window is - would write a scene the run never asked to write.
- A hand-over (`project.open from_launch=true`) asks only where a person answers; `project.open save=true`
  refuses what `save_all` left unsaved, as `project.close save=true` does.
- What the system refused a file operation with is said in the editor's words (`gz_base::os_reason`): the
  system's sentence comes in the system's language and with a number. A code the editor has no words for
  keeps its number.
- The libraries' pending edits are a service of the editor, so a command edits what a person edits and needs
  no tab. `library.set` saves by default: a caller that sets and leaves must not lose the change; a person's
  edit stays unsaved until Save. `library.rebuild` is not `library.import`: nothing comes from outside.
- A resize configures the surface on the UI thread, which waits there for the frames in flight; the graphics
  driver's own waits inside a present are the driver's, the graphics runtime's memory allocator has one lock
  for every thread, Direct3D writes its shader cache to the disk inside a draw, and Windows' text services make
  a thread of their own when a field first takes the keyboard. On a machine with no processor to spare each can pass 100 ms, so a suite that fails on a stall
  alone beside other suites is run again alone before it counts.
- A QA editor's window is hidden, not iconic: a launcher that asks for a minimised start gets a window that never
  comes up, and it is pictured and resized as if it were on the screen.
- The action table is a service of the editor with the last frame's facts, so a surface's own menu answers "on" and
  "checked" without the shell. A toast's button runs an action by id, never a closure.
- Tool tabs fold to icons whichever tab is up, so a switch never moves a tab.
- Studio is the viewport's fourth view mode (the editor's own light); Unlit means without light.
- The gizmo's planes are hit before its axes; Alt copies before the move starts, and copy and move are one undo
  step; the rotate readout shows the angle the way Details does.
- Place mode stays up until Esc. A foliage stroke is sent once, on release: one undo step.
- A tile under 64 pt is the list: the cheapest way never to cut a name mid-word.
- An asset's sidecars and a scene's folder are its companions: copied, moved, renamed and deleted with its tile.
- The chooser is not the kit's `PickList`: it needs the highlighted item for its preview and pictures in its rows.
  It ranks with the kit's `rank`.
- Problems is a panel beside the Output Log, not a tab inside it.
- Details' Reset arrow shows only where the scene's field table has a default to compare with.
- The PDA tab's Source pane lays a big text out over frames, so its first frame is no stall.

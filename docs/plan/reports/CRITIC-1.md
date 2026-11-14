# Critic - round 1 (HEAD 0dcaf211)

Read: the brief, `CLAUDE.md` as it is on disk, COMMANDS / ASSETS / UPDATES / STATE / UI_DESIGN (the parts the findings
touch) / AI_SETUP / README, every report under `docs/plan/reports`, the 31 tour pictures and a sample of the older
ones (G, S), `help` of the debug build for every area and every command, and the code behind each finding below.
Nothing was built, no suite was run, no editor with a window was started.

## Findings

### 1. `blocker` - 22 of the 26 suites start their editors WITHOUT `-qa_github`: a QA run reaches GitHub with the author's account and reads the author's own key

- **Where.** `tools/qa/qa.py:485` - `Editor(..., github: bool = False, ...)`; `-qa_github` and `LOOPBACK_ENV` are added
  only `if self.github` (`qa.py:522-523`, `540-541`). Only `test_ui_shell`, `test_mod_publish`, `test_mod_catalog` and
  `test_update` pass `github=True`. Every other suite (`test_tour.py:133`, `test_build_contents.py:217`,
  `test_build_job.py:268`, `test_unicode_names.py:78`, `test_bridge.py:451`, `test_characters`, `test_previews`, ...)
  runs a plain editor: real `api.github.com`, the token chain environment / Credential Manager / `gh auth token`
  (`gz-github` `service.rs:50`, `account.rs:56`), the real catalog, the real VirusTotal key, and
  `Documents\Grizzly Editor` instead of `GRIZZLY_CATALOG_HOME` (`gz-release` `keys.rs:93-100`).
- **Proof, in the tour's own pictures** (the tour starts no mock):
  - `Intermediate/plan/tour/tab_releases.png` lists the real releases of `MMadmer/DARF-Overnight-Delivery`
    (`v1.0.1` Latest 2026-09-25, `v1.0.0`), status "2 published".
  - `dialog_submit.png`: Author Key **Ready**, footer "Waiting for GitHub: the account"; the Task Center card in
    `menu_File.png` .. `menu_Help.png` goes on: "Waiting for GitHub: the latest release", then "Waiting for the
    module's website" - the Mod Browser check ran against the real services with the author's token.
  - `dialog_package_release.png`: "Signed with the author key 203a07f34ac1b530." - the author's key file, opened.
  - `test_build_contents.py:327`, `test_build_job.py:235`, `test_unicode_names.py:157` run `mod.package` on such an
    editor: test releases are signed with the author's real key.
- **Why it matters.** `CLAUDE.md:105` ("under `-qa_github` with every network service on a mock or a dead address"),
  `:290`, `:695`, `:847` ("a test never meets the author's own key") and the brief's safety line are all untrue as the
  suites stand. Only the door check stands between `test_bridge.py:201` (`release.publish` through the run door) and a
  real publication.
- **Closes it.** `-qa_github` and the loopback environment for every editor `qa.py` starts, with no switch to turn it
  off (the three suites with a mock override the addresses); `GRIZZLY_CATALOG_HOME` set to a folder of the suite's
  home by default; a step of `check.sh` (or of `qa.py` itself) that refuses a command line without `-qa_github`; then
  the tour again - Releases must show its "could not be read" sentence, the dialogs their settled state.

### 2. `should` - Shift + drag in the viewport is still the SDK's camera, and it swallows the gestures the contract gives to Shift

- **Where.** `crates/gz-ui-viewport/src/camera.rs:312-336` - `move_start` claims every press made with Shift held
  ("Shift for the legacy move"); `:427-434` is that scheme (`// csPlaneMove`: Shift+LMB pans, Shift+RMB lifts,
  both turn). `pane.rs:476-488`: a press goes to `nav.move_start(b)` BEFORE `mode_press`, so with Shift down the mode
  never sees it.
- **What breaks.** `pane.rs:728` (`erase: b.shift` - Foliage "Shift erases") and `pane.rs:754` (`mark_place(.., b.shift)`
  - Decals "Shift + click moves the selected one there") cannot be reached with the mouse: the camera took the press.
  `docs/UI_DESIGN.md:404` promises both; `:399` ("The viewport's mouse") knows no Shift camera at all. No suite sends
  a gesture with Shift (`grep -i shift tools/qa/test_scene_tools.py`: nothing).
- **Why it matters.** Point 1 (a control that exists because the old editor had it), a contract sentence the code
  makes untrue, and behaviour no contract states.
- **Closes it.** Remove the Shift camera (`ue` flag, `legacy_end`, the `else if b.left && b.right` tail of
  `process`), and a `viewport.gesture` case with `shift` for the foliage erase and the decal move in
  `test_scene_tools`.

### 3. `should` - the Characters tab cannot show the model of any character whose model is the game's

- **Where.** `crates/gz-ui-world/src/characters.rs:843-847` passes the source words `"visual"` / `"darf"` to the model
  viewer; `gz-ui-previews` `model_load.rs:27-29` knows only `"game"` (anything else is the project and the library),
  so a game model is looked for in the project and the library and fails: "This model cannot be shown ...: file not
  found". The Preview button of the same form passes `"game"` and works (`characters.rs:771-775`).
- **Proof.** `Intermediate/plan/tour/tab_characters.png` (bandit_1, `actors\stalker_bandit\stalker_bandit3a`).
- **Why it matters.** The preview beside the form is the tab's "result shown, not its numbers"; it is broken for
  nearly every character, by a removed name (`darf`) that survived the renames, and `test_characters` does not see it.
- **Closes it.** `"library"` / `"game"` there (or a typed `ModelSource` instead of a word), and a check in
  `test_characters` that the embedded model is shown.

### 4. `should` - calls that lose files or unsaved work pass the ordinary door, and a `read` command writes

- `scene.save path=<another scene>` (`gz-scene` `cmd.rs:406-420`, `service.rs:620-649`): no check that the target
  exists - the open scene replaces another scene's files; no `overwrite` parameter, nothing refused.
- `scene.convert overwrite=true` (`cmd.rs:50-56`, `:513`): replaces an existing `.scene`; the card has no
  `destroys_with` (`file.copy`, `file.move`, `file.import` name `overwrite`).
- `scene.recover action=discard` (`cmd.rs:105-111`, `:1511`): deletes the recovery snapshots - unsaved work of a
  crashed session, a GhostDanger with a naming confirmation in the window - as a plain `write`.
- `scene.info` is registered `Spec::read` (`cmd.rs:57-64`) and with `summary=true path=<file>` writes that file,
  replacing whatever is there, at any absolute path outside the game (`cmd_edit.rs:1382-1396`; the help says "of the
  project").
- **Why it matters.** COMMANDS.md section 1 ("`read` (changes nothing)", "a parameter that turns a call into a losing
  one is named in the card") and `CLAUDE.md` ("Unsaved work is lost only through the destroy door").
- **Closes it.** `scene.save path=` refuses an existing other scene without `overwrite`; `destroys_with(&["overwrite"])`
  on both; the snapshots' discard behind the destroy door (a `discard` flag or a command of its own); the summary's
  file as a `write` command (or the parameter gone: the caller has the answer), held to the project.

### 5. `should` - `level.export kind=visuals` stamps this machine's and this user's name into files that ship

- **Where.** `crates/gz-export-level/src/cmd.rs:318-323` (`build_name`: `\\%COMPUTERNAME%\%USERNAME%`), used at `:350`,
  written into every exported `.ogf` (`gz-fmt-ogf` `format.rs:160`) under the project's `gamedata\levels\<level>\visuals`,
  which a build and a release ship to players.
- **Why it matters.** "Nothing in a project names a place outside it - no absolute path, no machine name"
  (`CLAUDE.md`, ASSETS.md rule 6); the same export gives different bytes on another computer; the author's Windows
  user name goes public. It is the SDK's habit, kept.
- **Closes it.** A fixed stamp (the editor's name and version), and `test_portable`'s "names no machine" scan over an
  exported visual.

### 6. `should` - traces of the replaced program and of the plan remain where `check_words.py` does not look

- **References to documents that no longer exist:** `crates/gz-export-level/src/lib.rs:4` and
  `crates/gz-win/Cargo.toml:10` (`docs/RUST_MIGRATION.md`); `crates/gz-ui/src/state.rs:150` and
  `crates/gz-win/src/lib.rs:206` (`docs/rust/STATE.md`, "After phase 8c").
- **The plan's own words in code and tools:** `crates/gz-app/src/stalls.rs:1` ("docs/plan section 6"),
  `crates/gz-ui-level/src/place.rs:16` ("stream L2a's"), `tools/qa/test_play_game.py:2` ("the Rust editor built ...
  (plan D1, phase 6's gate)"), `docs/UI_DESIGN.md` section 8 ("in their own streams").
- **Comments that only cite the old editor's functions and members** - several dozen left after the sweep, e.g.
  `gz-scene` `bulk/aimap.rs:101,122,374,399,516,526,543,773,791,859` (`m_HASH`, `BuildNodes(bFromSelectedOnly)`,
  `Clear(bOnlyNodes)`), `bulk/details.rs:289,681,791`, `bulk/wallmark.rs:259`, `objtools/mod.rs:88,102,105`
  (`GenObjectName(cls, buf, pref)`, `AppendObject(obj, true)`), `objtools/sector.rs:151`
  (`PortalUtils.GetSelectedSector`), `objtools/geom.rs:471,483`, `summary.rs:411`; `gz-ui-viewport` `camera.rs:178,226,232,251,388,429,473`
  (`m_SM`, `SetSensitivity`, `Process`, `csPlaneMove`, `Update(dt)`), `draw.rs:104,832`, `draw_aimap.rs:25`;
  `gz-levelbuild` `validate.rs:32-33` (`Validate(bNeedOkMsg, bTestPortal, ...)`), `prj.rs:204,773`, `libs.rs:50`,
  `spawn.rs:68,116`; `gz-ui-level` `outliner_model.rs:96` (`Filtering()`). Q2's own rule was "a comment that only
  named an old function was deleted".
- **A removed command's name:** `docs/NQ_ARCHITECTURE.md:1720` (`world_set`); `tools/qa/test_mod_catalog.py:449`
  (`ui_contract`); the source word `darf` of finding 3.
- **Why it matters.** Point 9 and the brief's "traces". The check passes because it reads `crates/**/*.rs` only and
  cannot know a bare old name.
- **Closes it.** The lines above rewritten or removed; `check_words.py` over `tools/`, `docs/` (without `docs/plan`),
  `README.md`, `CLAUDE.md` and the crates' `Cargo.toml`, with the patterns `docs/rust`, `RUST_MIGRATION`, `phase \d`,
  `stream [A-Z]\d`, `docs/plan`, a backticked `m_Name`, and a backticked call whose arguments are Hungarian
  (`bName`, `vName`). Whether `docs/plan` itself (plan, briefs, reports, `old_command_names.txt` - all of it about
  the removed program and full of "requests") stays in the tree once 0.7 is closed is the author's call; today it is
  tracked.

### 7. `should` - the tour pictures error screens and busy dialogs and passes; a missing texture is said to be undecodable

- **Where.** `tools/qa/test_tour.py:30,78`: the texture tab is opened as `ui.tab action=open kind=texture
  path=ui\ui_icon_equipment` with no `source`, which is the project (`texture_load.rs:28-33`) - the file is not
  there. `tab_texture.png` is "This texture cannot be shown ... The file cannot be decoded as an image." with "No
  facts"; `tab_characters.png` is finding 3. `settled` / `audited` (`test_tour.py:33-50`) ask only for no busy line
  and nothing escaped, so an error state is a pass: for two of its thirteen tab kinds the tour proves nothing.
- `crates/gz-ui-previews/src/texture_load.rs:383-386`: a file that was not found at all answers `UNREADABLE` ("cannot
  be decoded as an image") - an untrue sentence; the model viewer says "file not found" for the same case.
- `dialog_publish_release.png` and `dialog_submit.png` are pictures of busy dialogs ("Reading the releases on
  GitHub...", "Checking..."): only `gz_ui::state::busy_line` reports `loading` (`state.rs:185-196`); the dialog
  footer's busy status (`dialog.rs:345`) and a card's "Checking..." do not, so `docs/STATE.md:176` ("`ui.audit` answers
  `loading`, the busy lines a frame shows ... and the tour does") is untrue for them.
- **Closes it.** `source=game` (or `library` with a texture that is there) in the tour; `ui.audit` answers a surface's
  error state (`state::empty_state` with the error tone) and the tour fails on one; the footer's busy status and the
  cards report `loading`; "not found" as its own sentence in the texture tab.

### 8. `should` - the three library tabs: what they edit and their buttons have no command, and the Texture Library is still the SDK's import pipeline

- **No command.** `library.view` opens, selects and reads (`help library.view`). A texture's description (format,
  mips, bump ...), a sound's parameters, a light animation's keys (Insert / Delete / Move Key, `lanim.rs:432-513`),
  Sound Library's Play / Stop / Auto Play / Rebuild (`sounds.rs:884-928`), Light Animations' Play (`lanim.rs:522`),
  Check New Textures and Import New Textures (`textures.rs:1035,1109`; the action `library.check_textures` only opens
  the tab, `lib.rs:218`) are toolbar buttons and store commands of the tab alone. These files are binary (`.thm`,
  `lanims.xr`), so "edit the file" is no way in either.
- **Why it matters.** COMMANDS.md's first paragraph ("nothing can be done one way and not another"), point 5 ("still
  covering everything").
- **The SDK's step.** "To add one, put the picture into the editor's import folder and import it"
  (`tab_texture_library.png`): no drop onto the tab (no drop zone in `textures.rs` / `sounds.rs`), a folder the author
  has to find by hand - against UI_DESIGN 1a (drop, nothing asked that can be worked out) and the way Screens &
  Music takes any file.
- **Closes it.** `library.set` (an entry's fields by key), `library.keys` or actions of `library.view` for the light
  animation, `library.play`, `library.import` (files from anywhere; the tab's drop zone runs it); the buttons become
  actions over them.

### 9. `should` - UI_DESIGN section 8 is a to-do list, and part of what it names is already done or dead

- `docs/UI_DESIGN.md` section 8: "What the surfaces still carry of their own (`local.rs`), **to be replaced** by the
  kit's one widget of its job in their own streams: fold headers (`form::section`, `form::raised_section`, ...) by
  `Section`; checks ... pick lists ... item rows ... table heads ...". Point 10 asks for no "later".
- In the code most of it happened: the surfaces' `local_check` / `local_section` are one-line calls of `Check`,
  `Switch`, `Section`. Left: `gz-ui-mod` `local.rs:104-180` (`PickList`, `local_pick_list`: a second pick list beside
  the kit's, used at `settings.rs:1075,1323`); `gz_ui::form::section` and `raised_section`
  (`crates/gz-ui/src/form.rs:24-38`) have no caller; `gz-ui-world` `local.rs:44` says "the kit's `text_field` has no
  hint" beside `gz_ui::form::hint_field`.
- **Closes it.** Mod Settings on the kit's `PickList`, the two dead functions out, the paragraph rewritten as what is
  true (the local widgets that are one crate's own, nothing "to be replaced").

### 10. `minor`

- Sound Library's toolbar starts with More (`crates/gz-ui-previews/src/library/sounds.rs:836-837`, `tab_sound_library.png`);
  every other tab ends its own row with it.
- Preferences > General > Project Files: the row "Register Now" holds the button "Register Now" (`tab_preferences.png`).
- `asset.kinds` answers `read_by: "the game, the editor"` for the project file (`crates/gz-asset/src/cmd.rs:300`: it
  reads the reader's limits, not who reads the file); the game reads no `.grizzly`.
- `docs/UI_DESIGN.md:226` lists Check New Textures, Synchronize ..., Reload Textures, Rewrite Texture Descriptions
  in the Libraries menu; the menu (`menu_Libraries.png`) and `:568` of the same document have the three tabs and
  Reload Objects.
- `docs/UPDATES.md:59` says "Preferences > Updates"; the group is Preferences > General > Updates.
- `README.md` and `CLAUDE.md` say the notices are made "from what the two programs link"; the build makes three.
- Colour Indices' Export writes the `.dti` a file dialog picked without the write guard
  (`crates/gz-ui-level/src/detail_objects.rs:288`).

## Checked and found sound

- **The bridge**: six tools, `PLAYER_SAFETY` and `WITHDRAW_SAFETY` word for word on `grizzly_release` alone, the
  instructions, the annotations; a batch's items cannot carry a door of their own; `destroys_with` is read from the
  raw request before the typed parse (`gz-mcp-bridge` `main.rs`, `gz-mcp` `lib.rs`, `gz-editor` `spec.rs`,
  `registry.rs`).
- **The command model**: 157 commands, each with a card, an effect and documented parameters (`help` for every one);
  no untyped registration; unknown commands and parameters answered with the nearest names; commandlet verbs, exit
  codes and the usage line for a word that is neither verb nor project (`frobnicate`: usage, code 2, no window).
- **Unfinished-work markers**: no `todo!`, `unimplemented!`, TODO / FIXME, "not implemented", "for now" in `crates/`
  or `tools/`.
- **The rules**: no `static` holding state outside `gz-win` (one build-once glyph table), no C or C++ source in the
  tree, the removed shell tables (`cpp.rs`, `UiCommand`, the macro rows) gone; file work of the surface crates I
  sampled (Mod Settings, Start page, PDA, Publish) runs on the pools.
- **Leads of the reports that are closed**: a headless `project.close` stops the project's jobs (`editor.rs:183`);
  `ManifestService` follows `mod.ltx` through the watcher; Recent Scene reads the project's machine state; the
  Snap Objects list, `pick_any_class` and "Use In D3D" are gone; `bake_note`'s `WARNING:` prefix is gone; the
  status line forgets a closed project's report; `write_atomic` has a temporary file per call; `ui.contract`'s
  categories are the Library's kinds; the bridge's old-name fallback is gone; the documents no longer name the
  removed suites and commands (but for finding 6).
- **Assets**: every kind has a header, a version and a newer-version refusal (`asset.kinds`); the references print
  from a shell without a project; ASSETS.md sections 1-4 match what the kinds do; a `.group` is written whole only
  as a new file made from a selection.
- **The updater**: UPDATES.md against `gz-update`'s structure (ranges, the 90 % rule, `.raw` beside `.part`, the
  journal, the busy refusal, reuse of a stage by hash, `manual`); a development build says it does not update
  itself, in the window and from a shell (exit 1).
- **The interface from the pictures**: one frame on every tab (the tab row, the project's group, the status bar),
  the Level tab (modes, pills, Place, Outliner, Details), the graph (five wire styles, named pins on wires, the
  picker, the 110-node quest framed), Mod Settings, World Rules, Screens & Music, PDA, Releases, Preferences, the
  Start page, the nine menus and the seven dialogs read as one program and not as the SDK's editor - but for
  findings 2, 3, 7 and 8.
- **Version**: 0.7.0 in the workspace, `ping` and the Start page; the tree is clean; nothing of this round touched
  the network, the author's projects or port 28016.

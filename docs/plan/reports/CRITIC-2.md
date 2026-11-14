# Critic - round 2 (HEAD a57ce9cc)

Read: the brief, round 1's report, E1 / E2 / E3, `docs/STATE.md`, the parts of COMMANDS / UI_DESIGN / CLAUDE.md the
findings touch, the code behind every round-1 finding and behind the work since (`git diff 0dcaf211..HEAD`, by area),
`help` of the debug build for the new and changed commands, and both sets of tour pictures (the 31 at 1707 x 912 and
the small set). Run: `check_words.py`; the debug commandlet's `help`; and the library commands from the commandlet on
an empty library of my own in the scratch folder (`-root`, `-home` there, `GRIZZLY_QA_GITHUB=1`) - nothing else.
No build, no suite, no window, no network, nothing under `D:\XFinedProjects`, no port 28016.

## Findings

### 1. `blocker` - `library.sync` and `library.rewrite_descriptions` ask the window's question on the command's path, answer "started" when nothing started, and do not run without a window

- **Where.** `crates/gz-ui-previews/src/libtools.rs:252-265` (`library_sync`: `on_ui` -> `synchronize_textures(cx)` /
  `synchronize_sounds(cx)`, then `{"started": true}`), `:288-291` (`library_rewrite` -> `rewrite_thumbnails`, then
  `{"started": true}`); the questions are in `crates/gz-ui-previews/src/lib.rs:73-90`, `:131-163` and
  `libtools.rs:43-83`. The asker is taken from the `-nodlg` flag alone (`lib.rs:67-69`, `libtools.rs:44`), never from
  the caller; `on_ui` opens no caller scope (`gz_ui::dialog::answer_as` has no caller outside tests).
- **What happens.**
  - A window without `-nodlg` (the author's own editor, an agent over MCP): the call queues the modal
    "Synchronize Textures" / "Rewrite Texture Descriptions" in the editor's window and answers `started: true` at once.
    The job starts only if a person clicks.
  - `-nodlg`: the logged answer is No - nothing runs, the answer is still `started: true`.
  - A commandlet: `Grizzly-Cmd.exe run library.rewrite_descriptions` answers `{"ok":true,"started":true}`, exit code
    0, and does nothing (the `.thm` of my scratch library kept its time; `UiQueue::of` fails and the function
    returns). `run library.sync kind=textures` / `kind=sounds` answers `no_window`, exit 1 - a file job that draws
    nothing.
  - Neither answers a job, waits for one or takes `wait`.
  - `library.sync`'s card is `library.reload`'s parameters (`KindParams`, `libtools.rs:241-249`): it lists `discard`
    ("reload light_animations: ...", does nothing here) and the kinds `objects`, `light_animations` it refuses, as
    `library.reload` lists `sounds` it refuses; `libtools.rs:273` is a branch no call reaches. The two commands say
    `kind=textures|sounds` where `library.view` / `set` / `rebuild` / `play` say `texture_library|sound_library` for the
    same libraries.
  - No suite calls either command (`grep "library.sync\|library.rewrite" tools/qa`: nothing).
- **Why it matters.** "A command never asks a question" and "A command shows NO confirmation ... and never hangs on a
  modal" (`CLAUDE.md`, COMMANDS.md section 1); "Long work is a job the command waits for ... No command answers
  'pending, ask again'"; "everything except the picture runs without [a window]"; UI_DESIGN section 6 "every button of
  a library's toolbar is an action over a command and nothing needs the tab". Points 5 and 6 of the request. An agent
  or a script that reads `ok: true, started: true` believes a synchronisation ran.
- **Closes it.** The question is the window action's (the way Reload of the light animations asks and sends the
  command again), the commands start the job themselves on the editor's pools, wait for it and answer what was
  written (`wait=false`: the job), with parameter structs of their own in the words of `library.view`'s `kind`; a
  phase of `test_previews` K and one line of `test_cli` (a commandlet synchronises the temp library) hold it.

### 2. `should` - round 1's finding 6 is closed only for the lines it named: comments that cite the replaced programs' functions remain, and so does `world_set`

- **The removed command's name round 1 gave by line is still there:** `docs/NQ_ARCHITECTURE.md:1720`
  ("Mod > World Rules, `world_set`"). `check_words.py` does not hold the documents and the tools to
  `removed_commands.txt` (`BESIDE + PLAN` only, `check_words.py:254`).
- **The editor's functions and globals, cited as the explanation:** `crates/gz-scene/src/pick.rs:263-265`
  (`RayPickObject(range, start, dir, OBJCLASS_SCENEOBJECT, ...)`, `g_RayPickAll`), `objtools/geom.rs:564`
  (`Scene->RayPickObject(...)`), `objtools/sector.rs:192,339` (`Scene->GetBox(box, OBJCLASS_SCENEOBJECT)`,
  `Scene->SpherePick`), `objtools/shape.rs:141`, `bulk/details.rs:96` (`Scene->GetBox(bb, list)`), `objmotion.rs:71`
  (`SetParam(min*30, max*30, 30)`); `crates/gz-particles/src/system.rs:151` ("The editor's `Tools->RayPick`");
  `crates/gz-ui-previews/src/libtools.rs:36` (`EDevice->ReloadTextures`), `library/texsync.rs:163`
  (`SynchronizeTextures(true, true, false)`), `local.rs:62` (`CreateChoose(smTexture)`), `library/sounds.rs:798` ("the
  curve the old property canvas plotted"); `crates/gz-entities/src/props.rs:407` (`PrepareKey(pref, s_name, rest)`);
  `crates/gz-levelbuild/src/libs.rs:37` (`Resources->_FindBlender`).
- **The compiler's and the model tools':** `crates/gz-lc/src/tree.rs:88` (`g_tree`), `gz-lc-light`
  `global.rs:22,34,36` (`g_params`, `g_vertices`, `g_faces`, `g_deflectors`), `data.rs:129`, `deflector.rs:306`,
  `mesh.rs:171`; `gz-fmt-ogf` `vipm.rs:115,388,617,691,882,932`, `obb.rs:168,195`, `bones.rs:98` (`Import(F, vers)`).
- **Why it matters.** Point 9 ("every trace"), and Q2's and E2's own rule ("a comment that only named an old function
  was deleted"). The check passes because its patterns need a Hungarian argument or `m_`: `Name->Call`, `g_name` and a
  bare backticked call are not seen.
- **Closes it.** The lines rewritten to say what the code does; `check_words.py` with `` `\w+->\w+ ``, a backticked
  `g_[A-Za-z]\w*`, `OBJCLASS_`, and `removed_commands.txt` over the documents and the tools (the allow-list for the
  ids that are no commands is there already).

### 3. `should` - the libraries' writers do not pass the write guard, and the library may be the linked game

- **Where.** `crates/gz-ui-previews/src/library/texsync.rs:78,108,112,144,149,158`, `library/sounds.rs:679-790`,
  `library/lanim.rs:849` (`lanims.xr`, written whole), `libtools.rs:116,178`, `library/textures.rs:1182`: plain
  `write_atomic` / `fs::copy` / `fs::rename` under `EditorDirs::of` = `gz_project::library_root` (no `WriteGuard`
  anywhere in the crate but for the word `writable`, which asks the OS). `project.set_library`
  (`crates/gz-project/src/cmd.rs:889-904`) takes any folder that holds `rawdata` or `gamedata` - a game does.
- **What it allows.** With the library set to the linked game (the Library the browser shows is "the linked game and
  the editor's library merged", so the wish is near), Save of Light Animations or `library.set kind=light_animations`
  replaces the game's `gamedata\lanims.xr`, `library.set` / `library.rebuild` / `library.sync` write
  `gamedata\textures\*.dds` and `gamedata\sounds\*.ogg` of the installation. Since this round every one of them is a
  command.
- **Why it matters.** "Every writer passes `gz_editor::fsx::WriteGuard` before it writes ... nothing under the linked
  game's root" (`CLAUDE.md`); the brief's "a writer that does not pass the write guard".
- **Closes it.** One guard for the library's writers built from the game link (it needs no project), and
  `project.set_library` refusing a folder that is or lies inside a game; a unit test with a fake game folder.

### 4. `should` - one more pick list of a surface's own: the Texture Library's texture pick

- **Where.** `crates/gz-ui-previews/src/local.rs:64-99` (`local_texture_pick`, used at `library/textures.rs:843` for a
  description's detail, bump and normal map): the only `egui::ComboBox` outside the kit, with a search field and menu
  rows of its own, and `.take(300)` - a library with more textures shows the first 300 and says nothing.
  UI_DESIGN section 8 lists it ("the texture pick") as nobody else's job; it is `Combo::show(ui, PickList)`'s
  (search, ranking, virtual rows, "No matches"), the merge E3 made for the world tabs and E2 for Mod Settings. E3
  marked the crate "not touched (E1's)".
- **Why it matters.** "The kit is the only way to draw UI ... ONE widget per job" - round 1's finding 9, one widget
  short.
- **Closes it.** The kit's `Combo` with a `PickList` (None as its first item), the function gone, section 8's line
  with it.

### 5. `minor`

- **A count that is not known yet reads 0.** `crates/gz-ui-level/src/place.rs:502-518`: a Place category whose list
  is still being read shows `count: shown.len()` = 0 in its header, and the busy note ("Reading the list...") only
  when the fold is open - so `ui.audit` reports no `loading`. The tour's own `tab_level.png` says "Objects 0"; the
  dialogs pictured seconds later say 8844. No count (or the spinner) until the list is read.
- **`loading` has no control.** Nothing ever asserts that `ui.audit` answers a non-empty `loading`: `test_tour`'s
  `settled` (`tools/qa/test_tour.py:35-39`) passes just as well if no busy mark reported itself, and E1 says no busy
  dialog was seen failing the wait. A kit test (a `busy_line`, a `Footer::busy`, a `card_busy` drawn in an audited
  frame give one `Loading` entry each), the way `test_previews` T is the control for `errors`.
- **Popups no command brings into view.** World Rules' "Add Level..." and Mod Settings' "Add Module..." and Function
  lists (the kit's `pick_popup`, `gz-ui-world/src/world.rs:917`, `gz-ui-mod/src/settings.rs:1091,1330`) are in no
  picture and no suite - E2 and E3 each say so - because nothing reaches them: `world.view` and `mod.view` have no
  `show=` as `characters.view` has, and `ui.contract`'s popups do not name them (`CLAUDE.md`: "anything that must be
  looked at from outside needs a command into view"). The tour pictures Light Animations and the Sound Library with
  nothing selected, so the timeline, the key actions and the attenuation curve E1 rebuilt are in no picture either.
- **`docs/STATE.md` section 2 is short of what the engineers wrote.** It has no line for: every question a person
  answers (QA editors run `-nodlg`: the replace question of New Textures' Import, the reload question of the light
  animations, each `confirm`), a real drop onto the Texture Library, the popups above. One sentence each is enough -
  they are the kind of thing section 2 exists for; the rest of E1 / E2 / E3's "not proven" needs nothing more.
- **A frame still makes a thread.** `crates/gz-ui/src/file_dialog.rs:108`: the file dialog's thread is made on the UI
  thread in the frame of the click. `docs/STATE.md` ("The window") says "No frame makes a thread" and why (three
  seconds under a virus scanner); `Executors::thread` is there for it.
- **The kit sheet lacks what the kit gained.** `crates/gz-ui/src/kitsheet.rs` shows the tabs, the checker, the
  `Combo`'s states and `spinner_at`; not `secret_field`, `row_field` / `hint_field`, `table_headers_ruled`,
  `error_state`, `busy_line`, `card_busy`. UI_DESIGN section 4: "a sheet of every widget below in every state".
- **Sentences the code makes untrue.** `docs/UI_DESIGN.md:573`: the Texture Library's empty state has "Import New
  Textures" as its Primary - it is "Check New Textures" (`tab_texture_library.png`). Section 8: "the colour wheel" is
  `local_wheel`, the mouse wheel's turn; `gz-ui-viewport`'s `local_axis_gizmo` and `local_stats_pill`, `gz-ui-level`'s
  `local_number_row` and the previews' `LibraryList` are not listed. Crate docs name widgets that moved into the kit:
  `crates/gz-ui-content/src/lib.rs:35` (`local_checker`), `crates/gz-ui-level/src/lib.rs:38` (`local_tab`),
  `crates/gz-ui-previews/src/lib.rs:29` (`local::local_checker`, a link to nothing).
- **The import folder keeps a copy of every picture.** `library.import from=<file>` copies the picture into
  `<data root>\import`, imports it and leaves it there as `<name>.~png` (`library/texsync.rs:158`; seen in my scratch
  library) - for good, in the editor's own folder, also when the library is elsewhere. A picture the command itself
  staged can go once it is in the library.
- **Two tests of "this computer".** `crates/gz-github/src/endpoints.rs:49-51` takes any text that starts with
  `http://127.0.0.1:` or `http://localhost:` (`http://127.0.0.1:1@api.github.com` passes); `gz-update`'s
  `net::is_loopback` reads the host. Only a harness sets these variables, so nothing leaks today; one function for
  both makes the sentence "names anything but this computer, is nowhere" true.
- **Words of the old messages** in what the author reads: `libtools.rs:151,158,185` ("Cube Map: scale image...",
  "Can't load texture '...'.", "Smaller cubemap successfully created [0.12 sec].").

## Checked and found sound

- **Round 1, finding 1 (the blocker): closed.** `-qa_github` is on every command line `qa.Editor` builds, with no
  switch (`qa.py:532`); `qa.py` makes the whole process tree a QA run by its environment at import (`GRIZZLY_QA_GITHUB`,
  every address at `127.0.0.1:9`, the author's tokens and key variables taken out), every suite imports it, and
  `main.rs:109` reads the variable for an editor, a commandlet and the twin alike (`test_cli`'s own starts carry no
  switch and are QA runs). The harness refuses an editor whose `ping` does not say `qa` (`qa.py:593-608`). In a QA
  run: addresses that are not this computer's become nowhere (`endpoints.rs:56-62`, tested), the token chain is
  `GRIZZLY_GITHUB_TOKEN` and a QA credential only and never `gh` (`service.rs:95-128`), the VirusTotal key likewise,
  the author key's home is a folder of the suite's home (`keys.rs:105-118`), the module's website is not opened
  (`gz-catalog` `jobs.rs:250`), the updater without `GRIZZLY_UPDATE_ROOT` has no updater and with one speaks to
  loopback only, redirects included (`gz-update` `service.rs:190-205`, `net.rs:141`). Play's game is started on
  another fs config and makes no update check. The pictures agree: Releases "Not signed in to GitHub", Package Release
  "Not signed: no author key", Submit with the key file under the QA home and "not opened in a QA run", Update "A QA
  run ... does not update itself".
- **Finding 2: closed.** The Shift camera is gone (`camera.rs`, the unit test at `:605`); Foliage's Shift erase and
  the decal's Shift move are driven by `test_scene_tools` D and W; UI_DESIGN:403 says whose Shift is.
- **Finding 3: closed.** `ModelSource` is typed, the Characters tab asks the game for a game model
  (`characters.rs:767-771`), `test_characters` U holds it, `tab_characters.png` shows the model.
- **Finding 4: closed.** `scene.save path=` refuses another scene without `overwrite`; `scene.convert`,
  `scene.summary`, `scene.save` name `overwrite`, `scene.recover` names `action=discard` (the value form is read by
  `Spec::effect_of` and tested through the gate); `scene.info` writes nothing, `scene.summary` is a `write` command
  that passes the write guard.
- **Finding 5: closed.** The exported model is stamped `Grizzly Editor <version>` (`gz-export-level` `cmd.rs:321`),
  held by `test_level_commands` E.
- **Finding 6: closed for the document references, the plan's words, the named comments and `ui_contract`; open for
  the rest** (finding 2 above). `check_words.py` as widened runs clean and reads the tools, the documents and the
  manifests; whether `docs/plan` stays tracked is still the author's call.
- **Finding 7: closed.** The tour opens a texture of the game it found itself, fails on an error state (`errors`,
  with `test_previews` T as its control), a missing texture says it was not found, and all seven dialogs are pictured
  settled.
- **Finding 8: closed for what it named** - `library.set`, `play`, `import`, `rebuild`, `library.view`'s `check`,
  `types`, `alpha`, `auto_play`, `key`, `library.reload`'s `light_animations` with `discard` behind the destroy door,
  the drop on the Texture Library; each has its card, refuses in a sentence with the nearest name, and - what E1
  did not prove - `library.import`, `library.set` and `library.rebuild` work from a commandlet (imported, set and
  rebuilt a picture in a scratch library; `exists` without `overwrite`). Open: finding 1 above.
- **Finding 9: closed.** Mod Settings and World Rules pick with the kit's list, `form::section` / `raised_section`
  are gone, section 8 says what is kept and nothing "to be replaced" (but for finding 4 and the minor above).
- **Finding 10: all seven closed** (More last in the Sound Library, "File Association", `read_by` of the project
  file, the Libraries menu's line, Preferences > General > Updates, "three programs", the Colour Indices export's
  guard).
- **The new commands' model**: 165 commands, no word of a command in the documents that the build lacks
  (every `` `area.verb` `` of `docs/`, `README.md`, `CLAUDE.md`, `AI_SETUP.md` checked against `help`); the losing
  parameters of the new ones are in their cards.
- **The kit's new widgets**: `tabs`, `checker`, the `Combo`'s states, `row_field`, `secret_field`,
  `table_headers_ruled` draw with the palette and points only; no colour of a surface's own outside data colours;
  each `local.rs` holds what section 8 says it holds.
- **`Executors::thread`, the label limit, the stall classes**: the thread is made by a pool thread and the work runs
  on the asker when none can be made; the viewport keeps the nearest 256 labels; a stall beside other suites run
  again alone is said in `docs/STATE.md` with its three causes, and `starved` is kept apart - the statement is enough.
- **Unfinished-work markers, `unsafe`, statics**: none new.
- **The interface from both picture sets**: one frame on every tab at both sizes, tool tabs folding to icons as tabs
  are added and not as they are switched, the library tabs' toolbars ending in More, the dialogs inside the window at
  1280 x 720; nothing that reads as the SDK's editor but the import folder's leftovers (minor above).
- **`docs/STATE.md` section 2**: its five lines are things no run can prove on this machine by rule (the user's file
  association, a real drag, GitHub) or that `-nodlg` answers; the chooser's sound preview uses the player
  `library.play` now proves. Enough as stated, with the lines the minor above asks for.

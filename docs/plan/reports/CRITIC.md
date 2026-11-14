# Critic - round 3 (HEAD 7752a8ad)

Read: the brief, round 2's report, E4 / E5, `docs/STATE.md`, the parts of `CLAUDE.md` (as it is on disk), COMMANDS and
UI_DESIGN the findings touch, the change since round 2 by area (`git diff a57ce9cc..HEAD`), the code behind every
round-2 finding, and every place of the workspace where a kit question (`confirm`, `unsaved_prompt`, `notice`,
`result`) or the system's file dialog is raised, and for the libraries, the Level tab, the PDA tab, the Characters
tab and the shell back to what calls it. Looked at: 20 of the 62 tour pictures (11 at the large size, 9 at the small
one - the tabs the work since round 2 touched and a sample of the rest; the others I did not open), one page of the
kit sheet, two of the pick lists of `Intermediate/plan/e5` and `e5k/library_pick_detail.png`. Run: `check_words.py`; the debug
commandlet's `help`; and, in a scratch folder of my own (`-home`, `-root`, a project there, `GRIZZLY_QA_GITHUB=1`,
`GRIZZLY_GAME_SEARCH=home`): `library.sync` / `library.rewrite_descriptions` on a library of one texture,
`project.set_library` on three folders, `game.link` and a synchronisation with the library set to a fake game this
home knows and to one it does not, `project.create`, `file.create`, `mod.set`, `check`, `reference`, `mod.build
target=folder`, and the project copied elsewhere and opened on a fresh home. No build, no suite, no window, no
network, nothing under `D:\XFinedProjects`, no port 28016.

## Findings

### 1. `blocker` - `doc.save` reaches a person's question: the system's file dialog for a scene without a file, the replace question of New Textures

Round 2's blocker is closed (below). The same rule is still broken on another path: a tab's `save` is called by
commands with no word of who asked, and two tabs ask in it.

- **Where.** `doc.save` (`crates/gz-app/src/app_cmds.rs:655-660`) takes no origin; `save_tabs`
  (`crates/gz-app/src/unsaved.rs:110-158`) calls `shell.save_tab` -> the document's `save` and then waits up to
  `SAVE_WAIT` = 120 s (`:20`) for the tab to be clean. For a library tab `project.close save=true` and `app.quit
  save=true` come the same way (the tabs' `DirtySource`, `unsaved.rs:54-59`), and `ui.tab action=close save=true`
  calls the same `save_tab` (`shell.rs:788`). No caller scope exists while it runs:
  `gz_ui::dialog::answer_as` - "held while a caller who has decided runs something of the window" - still has no
  caller outside the kit's tests, so every asker taken from the `-nodlg` flag alone answers as a person.
- **The Level tab, a scene with no file** (`scene.new`, an edit, `doc.save`): `crates/gz-ui-level/src/doc.rs:791-794`
  -> `save_scene` (`:616-632`) -> `level_cmds::save_as` (`level_cmds.rs:218-236`) -> the system's Save dialog, asker
  from the flag (`tools.rs:127-129`), the result dropped (`let _ =`).
  - A window without `-nodlg` (the author's editor, an agent over MCP): the Save Scene As dialog of Windows opens,
    and the command waits two minutes for a person, then answers "the save did not finish" with the dialog still up.
  - `-nodlg`: nothing opens, the command waits the same two minutes for a tab that cannot become clean.
  - The domain's own save knows the right answer and gives it at once: "the scene has no file yet: scene.save
    path=<file> first" (`crates/gz-scene/src/service.rs:102-108`); `level.show dialog=save_as` refuses a caller with
    `origin.may_ask()`. `doc.save` does neither.
- **The Texture Library in New Textures** (`library.view kind=texture_library action=check` puts it there, and the
  tab is dirty while pictures wait: `textures.rs:284-289`): `save` (`:1594`) -> `queue_save` (`:320-332`) ->
  `confirm_import` (`:907-950`), which asks "These pictures replace textures the library has" with the asker from the
  flag when a picture has the name of a library texture. Window: the modal goes up and the command waits two minutes
  on it. `-nodlg`: answered No, two minutes, "did not finish". The command that does this without a question exists
  (`library.import overwrite=true`); `doc.save` should refuse and name it.
- **Two smaller ones of the same kind.** `library.view action=check` with nothing new leaves the modal note "Check New
  Textures - Nothing in ... is new to the library" in the window (`textures.rs:1449-1463`, raised from the tab's
  frame, asker from the flag): the caller has its answer already. And `doc.save` of the PDA tab answers before the
  file is written - the tab marks itself clean first (`crates/gz-ui-pda/src/pda/doc.rs:899-918`) - so a write that
  fails is a "Save failed" modal in the window and never the command's answer.
- **Why it matters.** "A command never asks a question"; "A command shows NO confirmation ... and never hangs on a
  modal"; `docs/STATE.md`: "A file dialog is a person's". Points 5, 6 and 7 of the request. `scene.new`, place,
  `doc.save` is the first thing an agent that builds a level does. No suite calls `doc.save` on a scene without a
  file or on New Textures.
- **Closes it.** The document's `save` learns who asks (the origin, the way `ui.tab` and `level.show` have it), or
  `doc.save` asks the tab first whether it can save without a person: a scene without a file answers the domain's
  sentence, New Textures answers `exists` naming the textures and `library.import overwrite=true`; the window's Save
  keeps both questions. The check's note becomes a line of the tab (or of the answer) for a caller. The PDA tab stays
  dirty until its file is written. One phase each in `test_scene_tools` and `test_previews` K, both answering inside a
  second. `answer_as` either gets its caller or goes.

### 2. `should` - "a game's folder" is told by `fsgame.ltx` and `gamedata`, which the editor's own folder has: the library of another editor folder is refused, and a game this home never linked is written

- **Where.** `project.set_library` (`crates/gz-project/src/cmd.rs:901-916`) refuses a folder when
  `gz_vfs::looks_like_game` says yes: `fsgame.ltx` and `database\` or `gamedata\` (`crates/gz-vfs/src/path.rs:68-83`).
  The editor's data root is exactly that - `fsgame.ltx` is tracked at the repository's root and every package ships
  it (`tools/rust/package.py:40`). The suite's own "game" is the editor's layout: `gamedata`, `rawdata`, `fsgame.ltx`
  (`tools/qa/test_previews.py:483-494`).
- **Seen.** `Grizzly-Cmd.exe run project.set_library path=<folder with fsgame.ltx, gamedata, rawdata>`: "is a game's
  folder: the library is the editor's own, and a game is never written". The same folder without `fsgame.ltx` is
  taken.
- **What it costs.** A release holds the 45 tracked files of `rawdata` and `gamedata`; the library proper (the
  objects, the base scenes, the sounds) lives untracked in `D:\Games\XFined-Editor`. An installed 0.7.0 gets it
  through Preferences > Library > Change... - which now answers that the editor's own tree is a game. Before round
  2's fix it was taken.
- **The other side of the same test.** The writers' guard knows only the default game, the games `machine.ltx` has
  seen and the open project's (`games.rs:330-339`). With `[library] root` of `machine.ltx` naming a game-shaped folder
  this home never linked (a hand edit, a home copied from another machine, `-root`), `library.sync
  kind=texture_library` wrote the `.thm` and the `.dds` into it (seen in the scratch folder); with the same folder
  linked as the game, the same call was refused and the folder stayed byte for byte - so the guard itself holds.
  `CLAUDE.md` and `docs/STATE.md` say "a game's folder is refused as the library's" and "a library is never a game".
- **Closes it.** One answer to "is this folder a game and no library", used by `project.set_library` and by
  `LibraryGuard::check` on the library's root: what a game has and an editor folder never has (the engine program
  `games::ENGINE` already names, or `database\`), not what both have. The suite's fake game gets that mark, and a
  second check takes a folder laid out like the editor's.

### 3. `should` - a new project's first answer sends an agent to the window, and the manifest has no reference

- **Seen.** `project.create`, then `Grizzly-Cmd.exe check`: exit 1, one error - "no target game mode: open Mod > Mod
  Settings, Game Mode, and pick one ..." (`crates/gz-mod/src/rules.rs:164`). It names no key and no command.
  `mod.set`'s card lists `mode, target` without their values; the values (`default`, `existing`, `new`, `any`) are
  said only by the refusal of a wrong one (`crates/gz-mod/src/cmd.rs:339`) and by no document of this repository.
  `asset.kinds` lists eight kinds, `reference` knows seven: `reference "module manifest"` is "no kind of asset", and
  the kind's reference is "the game's docs/dead-air/MODDING.md" - a file of another repository, which a project moved
  to another computer does not have. `AGENTS.md` of the project promises `Grizzly-Cmd.exe reference <kind>`.
- **Why it matters.** Points 4, 5, 6 and 8: the file every module must have is the one an agent cannot learn from the
  editor, and the first problem of every new project has its fix in a menu.
- **Closes it.** The sentence names the fix in the file's words (`[module] target = default`, `mode = <id>`, or a
  `[provides_mode]` section; `mod.set`); `mod.set`'s card names `target`'s values; `reference` answers for the
  manifest from the rules `gz-mod` already holds.

### 4. `minor`

- **Three citations the word check cannot see** (a dotted call): `crates/gz-ui-previews/src/library/lanim.rs:241,247`
  (`LALib.Reload`, `LALib.Save`), `crates/gz-scene/src/library.rs:238` (`FS.file_list($groups$)`). `check_words.py`
  sees `Name->Call`, `g_name`, a backticked capitalised call; not `Name.Call(`.
- **A message in the replaced labels.** `crates/gz-ui-previews/src/library/sounds.rs:506`: the toast "'Max AI Dist'
  <= 'Max Dist'. 'Max AI Dist' will be clamped." - the rows are Max AI Distance and Max Distance, and the row's own
  tip says it already.
- **`file.create`'s own example fails on a new project**: `file.create kind=quest path=quests` answers "the folder
  quests does not exist". Making the folder it was asked to put the asset into costs nothing.
- **`docs/plan`** is tracked (45 files) while `check_words.py` calls the plan gone: the author's call, as in both
  earlier rounds.

## Checked and found sound

- **Round 2, finding 1 (the blocker): closed.** `library.sync` and `library.rewrite_descriptions` start their job,
  wait for it and answer what was written; `wait=false` answers the job; neither reaches the UI queue, a question or
  a window. From a commandlet on a scratch library: `updated: 1`, then `up_to_date: 1`, `dry_run`, `unchanged: 1`,
  the sound library, `kind=textures` refused with the real kinds, exit codes 0 and 1. The questions are the window
  actions' (`sound_sync_reply`, `rewrite_reply`), raised over the command's answer and sending the command again;
  the cards are their own, in the words of `library.view`.
- **Finding 2: closed** for every line it named and for `world_set`; `check_words.py` runs clean with the member
  call, the global, the constant, the bare call and the removed commands over the tools and the editor's documents.
  Three leftovers of a form it does not read: minor above.
- **Finding 3: closed for the writers.** Every write of `gz-ui-previews` is a method of `LibraryWriter` (no
  `fs::` write, `write_atomic` or `set_file_age` outside `writer.rs`), `EditorDirs` is the read side, the uses of
  `gz_project::library_root` I read in the other crates only read or copy out of the library, and a known game is
  refused end to end from a commandlet - what E4 said it had not proven. Preferences sets the folder through the command alone. Open: what counts as a game (finding 2).
- **Finding 4: closed.** The texture pick is the kit's `Combo` over a `PickList`, every texture on virtual rows; no
  `egui::ComboBox` outside the kit; `library.view show=detail|bump|normal` puts it up and the picture shows it.
- **Round 2's minors: all closed** - a Place category being read shows a spinner and no number (`Section::busy`,
  unit test); `loading` has its control (the kit test, `test_ui_shell` K); the pick lists open by `world.view show=`,
  `mod.view show=`, `library.view show=`, `ui.contract` names them as `view_popups` and `ui.popup` points at the call;
  `docs/STATE.md` section 2 has the questions and the real drag; the file dialog's thread is made by a pool; the kit
  sheet shows what the kit gained and UI_DESIGN says what a sheet cannot show; the untrue sentences are rewritten;
  a staged picture leaves no copy; one test of "this computer" (`gz_base::web::is_loopback`, user information
  refused); the old messages are sentences.
- **Other commands and a person's question.** `ui.tab`, `app.quit`, `project.close`, `project.open`, `mod.build`,
  `level.show`, `scene.*` decide by the origin and answer `unsaved` or refuse a caller; `characters.view` answers
  `unsaved`; `doc.reload` refuses before the tab is asked; the other kit questions I traced are raised by a window
  action's reply or a button. The questions of the mod side's surfaces (`gz-ui-mod`, `gz-ui-release`, `gz-ui-app`) I
  read where they are raised and did not trace to every caller. The exception found is finding 1.
- **The renamed rows** of a texture's description read as the editor's own (Mip Maps, Implicitly Lit, Fade Color,
  First Faded Mip, Border Alpha, Normal Map); `library.set`'s keys did not move.
- **Safety**: the bridge's two paragraphs intact and on the release tool; nothing new reaches a service; the suites'
  library commands run on a temp library (`-root`).
- **Traces, markers, rules**: no C or C++ file tracked, no `todo!` / `unimplemented!` / "for now"; the change since
  round 2 adds no `static` that holds state (`unsafe` I left to `check.sh`).
- **The ten points as a whole.** 1: the pictures I opened show one frame, one toolbar grammar and the kit on every
  tab, dialog and menu in them; nothing there reads as the SDK's editor. 2: the five wire styles are `gz-nq`'s
  `WireStyle`; the picker and the node actions I did not open again (rounds 1 and 2 did, and nothing of them changed
  since). 3: the updater changed since round 2 only in its test of "this computer" and I did not read the rest again; by
  rule it met only the mock.
  4, 6, 8: a project made, checked, given a quest, built to a folder, copied elsewhere and opened on a fresh home,
  all from the commandlet with no window and no game; nothing in the project names a place - short of finding 3.
  5: six tools on the bridge, 165 commands in `help`. 7: the two-minute waits of finding 1 are where I found a
  command standing still; the stalls are the suites' to hold, and I ran none. 9: closed but for the minor.
  10: version 0.7.0, nothing published; open is what is above.
- **What E4 and E5 say they did not prove**: the Yes side of the two library questions is `docs/STATE.md`'s "every
  question a person answers"; the pick popup and the guard against a linked game are proven now; the rest needs no
  more than the statement.

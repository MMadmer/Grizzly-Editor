# Critic - round 4 (HEAD a4db4af1)

**Verdict: not done. One blocker, three `should`, a list of `minor`.** Round 3's three findings are closed as they
were written; the blocker is the same rule on a path no round walked: Switch Project loses the unsaved scene.

Read: the brief, rounds 1-3, E6, `docs/STATE.md` and the document changes since round 3, the whole change
`fc95c3b0..HEAD` by area, `save_tab` / `close_tab` / `request_exit` (`shell.rs`), `save_tabs` (`unsaved.rs`),
`switch_project` (`chrome.rs`), `app.quit` / `app.switch_project` / `doc.save` / `ui.tab` (`app_cmds.rs`, `cmds.rs`),
`project.close`, every `Document::save` and `save_failed` of the workspace (Level, graph, Mod Settings, Characters,
PDA, the three libraries) back to what each starts, `games.rs`, `file.create`, `gz-mod`'s `model.rs` / `rules.rs`
against the reference page, the game's `MODDING.md` on `mode` (read only). Looked at: 9 of the 62 tour pictures (7
small, `tab_beh` large, the Publish dialog); the rest I did not open. Run: `check_words.py`, `cargo fmt --check`
(both clean); from the debug commandlet, each with a `-home` and a project of its own in the session's scratch
folder under the system temp folder, `GRIZZLY_QA_GITHUB=1`, `GRIZZLY_GAME_SEARCH=home`: `project.create`, `check`,
`help mod.set`, `mod.set`, `file.create` (eleven paths), `reference manifest`, `mod.build target=folder`,
`project.set_library` on two folder shapes, `library.sync` with the library set by hand to a game-shaped folder,
`game.link` to a fake game with a junction into it; and one probe through the QA client (`qa.suite`, `qa.Editor`:
port 28931, a home of its own, a project made in its temp folder, closed over `app.quit`, `ping` answered `qa:
true`, `perf.stalls` empty) for what only an editor with a window answers. No suite, no entry point, no network,
nothing under `D:\XFinedProjects`, no port 28016, no write outside the scratch folders (made and removed by name)
and this report. What only a person's click reaches (origin `Ui`) is judged by the code alone and says so.

## Findings

### 1. `blocker` - Switch Project loses the unsaved scene: `save=true` answers "closed" and drops it, no parameter drops it through the ordinary door, and the window asks nobody

- **Where.** `crates/gz-app/src/app_cmds.rs:306-351`. The dirty tabs are taken without the Level tab (`:317`);
  with nothing else unsaved the closure returns at once (`:318-320`) - before the window's question (`:321`) and
  before the `unsaved` refusal (`:326`); `save_tabs` saves only that list (`:341`); then `project.close` is called
  with `discard=true` whatever the caller said (`:343-344`), and `project.close` with `discard` runs `discard_all`
  (`crates/gz-project/src/cmd.rs:559-565`), which forgets the scene's changes (`gz-scene`'s `service.rs:113-122`).
  The window's half is the same shape: `chrome.rs:723` leaves Level out, `:731-732` closes with `discard=true`
  when nothing else is dirty, `:751` after the other tabs' question.
- **Seen** (the QA editor, a project made from nothing): `scene.new`, a light placed, `scene.save path=`, the light
  moved to x = 7.5. `ui.tab action=list` says the Level tab is `dirty: true`, `doc.list` names the scene under
  `unsaved`. Then:
  - `app.switch_project save=true` -> `{"ok": true, "closed": true}` in 0.36 s. The scene's two files are byte for
    byte what the first save wrote; the project opened again holds the light at x = 0; `scene.recover action=get`
    has no snapshot. The save the command was asked for was a discard.
  - the light moved again, `app.switch_project` with no parameter through the `run` door ->
    `{"ok": true, "closed": true}`, the move gone. The editor was `-nodlg`, but nothing on this path reads the
    flag: the return at `:318-320` comes before every check.
  - the window (by the code, origin `Ui` cannot be driven): File > Project > Switch Project... with only the scene
    unsaved raises no question at all; with other tabs unsaved the question lists them without the scene, and
    Save or Don't Save alike then drop it.
- **Why it matters.** "Unsaved work and another file's contents are lost only through the destroy door; every
  other path refuses with `unsaved`"; "Any destructive operation from the UI requires confirmation ... losing
  unsaved edits". The card says "unsaved tabs need save or discard" and `save`: "Save the unsaved tabs first" -
  Level is tab 1. `docs/STATE.md:96-99` says `app.switch_project save=true` "waits in `save_tabs` until the tabs
  are clean": untrue for the one tab every project has. It is the round's own question ("does any of those
  commands go on ... before the save it asked for has finished?"), and it is the author's level work. No suite
  calls `app.switch_project`; the code is as wave 1 left it.
- **Closes it.** `app.switch_project` hands the caller's own `save` / `discard` to `project.close`, which already
  does all three right (`save_all`, `unsaved` naming the documents, `discard`); the Level tab is in both lists;
  the window's question names the scene; a scene without a file answers the sentence `doc.save` gives. One phase
  in a suite: an unsaved scene, `app.switch_project` -> `unsaved`; `save=true` -> the file holds the edit.

### 2. `should` - `check` passes a manifest the editor itself would refuse: the reference sends an agent to `asset.check`, which reads none of the values

- **Seen.** `mod.ltx` written by hand with `version = v1.2-beta`, `website = http://evil.example/x`, `tags =
  banana, Story`, `target = sideways`, `mode = Not A Mode!`, `[update] github = not a repo`, `[vars] bad-name = 1`,
  `[world] bogus_rule = true`, `mutants = maybe`, `[screens] loading_screens = sometimes`:
  `Grizzly-Cmd.exe check` -> exit 0, `"total": 0`; `mod.build target=folder` -> built, no warning.
- **Where.** `gz_mod::validate` (`crates/gz-mod/src/rules.rs:155-204`) checks the id, the missing target, the
  declared modes and nothing else. `validate_version`, `validate_website`, `validate_github_repo`,
  `validate_mode_id`, `valid_global_name` and the tag list are called by `mod.set` (`cmd.rs:254-497`), the Mod
  Settings form and the packaging (`gz-release`'s `format.rs:104-115`) only. An unknown `target` passes as soon as
  a mode is written (`rules.rs:163`).
- **Why it matters.** Point 4, and what closed round 3's finding 3: the page says "Edit the file with your own
  tools ... check it with asset.check path=mod.ltx", "any other word is refused", "Any other host is refused by
  the editor". On the file path nothing is refused: the wrong version surfaces at Package Release, the wrong mode
  id never (the module's level work silently applies in no game). "Problems are positioned ... the same list is a
  tab's Problems, `asset.check` and `Grizzly-Cmd.exe check`."
- **Closes it.** The manifest's check runs the value rules the commands already hold, each with its line (the
  check places `mod.target` and `pda` already): version, website, tags, target's four values, the mode id, the
  repository, a global's name, an unknown key or value of `[world]` / `[screens]` as a warning. `test_cli` J gets
  one wrong value per rule.

### 3. `should` - `file.create` writes its asset past no guard: through a link it lands in the linked game

- **Where.** `crates/gz-project/src/cmd.rs:1219`: `write_atomic(&dir.join(&leaf), ...)` with no `WriteGuard`. The
  folder it now makes goes through `FileOps::mkdir` and is guarded (`:1203-1210`); the file is not, and a folder
  that exists skips the only check.
- **Seen.** A fake game (`fsgame.ltx`, `gamedata`, `database`) linked with `game.link`; in the project a junction
  `gamedata\link` -> `<game>\gamedata`. `file.create kind=quest path=gamedata/link` -> `ok`,
  `gamedata/link/NewQuest.nqasset`: the file is in the game's `gamedata`. The same call one folder deeper
  (`path=gamedata/link/sub`) and `file.mkdir` are refused with "lies in the linked game's folder".
- **Why it matters.** "Every writer passes `gz_editor::fsx::WriteGuard` before it writes - the project's file
  operations ...", judged "where the path really is"; the installed game is never written. Every other project
  writer I read passes it (`gz_asset::write_text`, `copy_from_game`, `FileOps`).
- **Closes it.** The asset's path through `pr.write_guard(None).check` before the write, in the same sentence;
  one check beside `test_asset_icons` D or a unit test with a junction.

### 4. `should` - in the window a save a person asked for is still only begun in three places, and a cancelled Save As comes back as an error two minutes later

By the code; origin `Ui` cannot be driven.

- **Begun, not waited for.** Exit with Save now waits (`shell.rs:907-918`). The three other questions do not:
  Close Tab > Save (`close_tab`, `shell.rs:794` then `remove` at `:804`), a retarget > Save (`:680-691`: `save_tab`,
  then `open(..., discard = true)`), Switch Project > Save (`chrome.rs:746-751`: `save_tab` for each, then
  `project.close discard=true` at once). The commands for the same three were fixed this round to wait
  (`save_tabs`). A Characters save resolves the project's service when it runs (`characters.rs:101`) and the tab
  marks itself clean first because "the shell may close the tab right after it asked to save" (`:634-635`): a
  write that fails, or a project that closed first, loses the edit with nothing said - the tab that held it and
  its inbox are gone. A graph's save that loses the race with `discard_all` writes the reverted document.
- **The Save dialog's Cancel reaches nobody.** Ctrl+S is `doc.save` with the window's origin
  (`shell_actions.rs:40-45`, `actions.rs:535-550`); for a scene without a file the tab opens Save Scene As and
  drops what it answers (`gz-ui-level`'s `doc.rs:655`, `level_cmds.rs:217-236`); `save_tabs` then waits `SAVE_WAIT`
  = 120 s for a clean tab (`unsaved.rs:20,133-162`). Cancel the dialog and two minutes later the error notice
  "Untitled: the save did not finish" comes up; the same after Exit > Save All, as "Save failed". Save by hand
  inside those two minutes after Exit > Save All > Cancel and the editor ends under the author.
- **Closes it.** The three questions go through `save_tabs(..., true)` like Exit does and close, retarget or
  switch only on `Ok`; a cancelled Save dialog ends the wait without a notice (the tab answers "cancelled" the
  way it answers `save_failed`).

### 5. `minor`

- **`file.create` makes folders the project never shows.** `path=.git/hooks` -> `ok`, a `.git` folder in a project
  that had none (the next command then wrote a `.gitignore`: the folder now looks like a working copy);
  `path=releases/x` and `path=temp/x` -> `ok`, quests the browser never lists and no build ships; `path=quests/con`
  makes a folder with a device's name. `../outside` and an absolute path are refused. A tooling or editor-only
  first segment should be refused with a sentence.
- **`check` walks the editor's own folders.** `gz-asset`'s `walk_disk` (`check.rs:51-65`) skips tooling names, not
  `temp\` or `releases\`. A quest copied to `temp\release_stage_4242\modules\<id>\quests\` (what a crashed
  packaging leaves): `check` exit 1, "E002 ... used by another quest of this project", while `mod.build` builds.
- **Two quests from `file.create` in two folders share an id**: both are `newquest`, and `check` is exit 1 at once.
- **A raw system number in the new sentence**: `file.create kind=quest path=mod.ltx/x` -> "... cannot create the
  folder (win32 183)".
- **Two comments against the contract**: `gz-mod`'s `model.rs:102` and `rules.rs:19` say an empty `mode` is "all
  modes"; the game's `MODDING.md` and the reference say the ordinary game only (the code follows the game).
- **A doubled doc comment**: `gz-ui-previews`' `library/lanim.rs:250-252`.
- **A caller's save beside a running one**: the texture and sound tabs' `queue_save` return `Ok` while a save
  already runs and attach no `answer_job`; if that job fails the caller waits the 120 s (by the code).
- **`docs/plan`** is still tracked: the author's call, as in every round.

## Checked and found sound

- **Round 3, finding 1: closed for every path it named.** From the QA editor: `doc.save` on a scene without a file
  answers in 0.03 s "the scene has no file yet: scene.save path=<file> names it and saves the scene", `app.quit
  save=true` in 0.00 s with the domain's sentence and the editor stays up; no dialog. By the code: the Level tab
  with a file calls `scene.save` as a caller and leaves its failure in `save_failed`; New Textures refuses with
  `exists` naming the textures and `library.import overwrite=true`; the graph, Mod Settings, Characters, Light
  Animations and both library jobs hand a failed write to `save_tabs`, which asks `save_failed` each pass whether
  the tab is drawn or not; `ui.tab action=close|open save=true` and `app.quit save=true` wait before they go on;
  `answer_as` has its caller (`shell.rs:821`). The exception is finding 1, which round 3 did not name.
- **Exit with Save** waits for every save and ends only on `Ok`; a failure keeps the editor open with a notice. No
  path found where it ends over unsaved work; the two-minute notice is finding 4.
- **The PDA tab**: dirty until its file holds the text, the write takes the path and text as Save found them and
  touches the model only when it still shows that file (`pda/mod.rs:425-432`); `discard` reads nothing.
- **Finding 2: closed.** `project.set_library`: a folder laid out like the editor's own is taken, one with
  `database\` is refused. `machine.ltx` set by hand to that folder: `library.sync kind=texture_library` refused
  with the guard's sentence, the folder byte for byte as before. `holds_a_game` is the one test of "a game and no
  library" (`cmd.rs:907`, `games.rs:338-341`); `looks_like_game` is left to linking a game; the writer takes its
  guard from `library_guard`, which covers the library's own folder.
- **Finding 3: closed as written.** The first `check` names the key, the values, the command and the fix
  (`mod.ltx:1:1`, `fix: target = default`); `help mod.set` names the four values; `mod.set` sets it; `file.create`
  into a new folder works; `reference manifest` prints 338 lines; the build builds; `AGENTS.md` names the
  reference and the target. The page against `Manifest::parse` and the game's `MODDING.md` on `mode`: no sentence
  found that the code or the game makes untrue, no key the model reads that the page lacks. What is open is
  finding 2.
- **Round 3's minors**: the three citations and the toast are rewritten; `check_words.py` reads the backticked
  dotted call and runs clean; a search for the bare form (`Name.Call(` in a comment) and for the replaced
  program's library and helper names found nothing; `file.create` makes its folder; the check that finds nothing
  is a toast and a log line.
- **`test.sh`** takes only artifacts built with `"test":true`.
- **Safety**: nothing of the bridge changed since round 3 (`git diff --stat`); every program I started was a QA
  run on loopback; the tree is as I found it (`git status` clean before this report).
- **Formatting**: `cargo fmt --all -- --check` lists nothing.
- **The ten points as a whole.** 1: the nine pictures show one frame, the kit and nothing of the SDK's editor. 2,
  3, 9: not opened again; nothing of them changed since round 3. 4, 6, 8: the walk above, from the commandlet
  alone - short of finding 2. 5: the commands' cards answered everything the walk needed. 7: no stall in the
  probe; the waits left are finding 4's. 10: 0.7.0, nothing published; open is what is above.
- **What E6 says it did not prove**: the long `mod.target` sentence in the window I did not see either (no tour
  picture shows a project without a target); the rest needs no more than the statement.

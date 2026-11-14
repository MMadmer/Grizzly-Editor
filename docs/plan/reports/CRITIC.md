# Critic - round 5 (HEAD b1c6b37b)

**Verdict: not done. One blocker, four `should`, a list of `minor`.** Round 4's four findings are closed as they
were written: every way out over an unsaved scene or tab that I drove is refused with `unsaved`, `file.create`
refuses and guards, the manifest's check reads the values. What is open lies beside them, on the paths the brief
asked to walk: a save that reports success for a write that failed, and three more ways unsaved work goes
without `discard`.

Read: the brief, round 4, E7, the whole change `a4db4af1..HEAD` (code, suites, documents), `app.quit` /
`app.switch_project` / `doc.save` / `doc.reload` / `ui.tab` (`gz-app`), `save_tabs` / `save_then`, `switch_project`,
`request_exit`, `open` and `request_close` (`shell.rs`), `project.open` / `close` / `create`, `open`,
`close_session`, `file.create` (`gz-project`'s `cmd.rs`), the hand-over (`grizzly`'s `launch.rs`, `gz-ui-app`'s
`launch.rs`), `settle_scene`, `begin_restore`, `update.apply`, the commandlet's run, every `DirtySource`, every
`Document::save` and `save_failed` back to what each starts (Level, graph, Mod Settings, Characters, PDA, the
three libraries and their jobs), the kit's `unsaved_prompt`, `confirm`, `close_dialog` and `file_dialog::pick`,
`gz-mod`'s `rules.rs` and the head of `reference.rs`. Looked at: 4 of the 62 tour pictures (`tab_level`,
`tab_quest`, `tab_sound_library`, `menu_File`, large); the rest I did not open.

Run, each under `limited.py`, in a scratch folder of my own under the system temp folder (removed by name at the
end): `check_words.py` and `cargo fmt --check` (both clean); `cg.sh test -p gz-mod` (48 + 1 + 0 green; the corpus
variables were NOT set - my rules forbid opening the author's projects - so the corpus test read no manifest);
from the debug commandlet with a `-home` and a project of its own, `GRIZZLY_QA_GITHUB=1`, `GRIZZLY_GAME_SEARCH=home`:
`project.create`, `reference manifest`, `check` on four manifests written by hand and on the page's own example,
`mod.set` (eleven values), `screens.set` (four), `file.create` (sixteen paths), `check` with copies under `temp\`
and `releases\`, `mod.build target=folder`, a `script` that edits the scene and ends; and three probes through
the QA client (`qa.Editor`: port 28951, `GRIZZLY_QA_HOME` and a library `-root` in the scratch folder, projects
made there with `project.create`, `ping` answered `qa: true`, closed over `app.quit`, `perf.stalls` empty each
time). No suite, no entry point, no network, nothing under `D:\XFinedProjects` or the game, no port 28016.
What only a person's click reaches (origin `Ui`) is judged by the code alone and says so.

## Findings

### 1. `blocker` - a library save whose write fails answers "saved", cleans the tab and throws the edit away

- **Where.** `crates/gz-ui-previews/src/library/sounds.rs:605-627`: an entry whose description cannot be written
  goes into `notes` (`:612`), the job still ends `Done`, and the store is emptied (`:625`, `Reset`, or `Drop` of
  every name it was asked to write, the failed ones among them). `textures.rs:1085-1107` is the same shape
  (`:1092`, `:1105`). The tab's save hears only a job that is not `Done` (`library/mod.rs:90-99`, `answer_job`).
  `job_answer` (`textures.rs:1316-1326`) does turn a `failed` list into an error - `library.set` uses it, the
  tab's save does not.
- **Seen** (the QA editor, a library of two sounds and two textures in the scratch folder; the description's
  file name taken by a folder, so its write fails):
  - `library.set kind=sound_library item=qa\qa_other set={"max_dist":55} save=false`, then `doc.save
    doc=sound_library` -> `{"ok": true, "doc": {... "dirty": false}}` in 0.2 s. The job's own result:
    `"written": 1, "failed": ["...qa_other.thm could not be written: ... (os error 5)"]`. The tab shows
    `max_dist` 300 again: the edit is gone, nothing was written, the command said ok.
  - the same on the Texture Library (`mips` of `qa\qa_second`): `doc.save doc=texture_library` -> `ok`, tab
    clean, job `"written": 0, "failed": [...]`.
  - `ui.tab action=close doc=sound_library save=true` over the same failing write -> `{"ok": true, "closed":
    "sound_library"}`; no pending edit afterwards.
  - `library.set ... set={"max_dist":56}` with its default `save=true` -> `failed`, with the sentence - right -
    but `dirty: false` after it: the caller is told the write failed and the edit is dropped all the same.
- **By the code, the window:** Exit > Save, Close Tab > Save and Switch Project > Save wait in `save_tabs`, see a
  clean tab and go on - the editor ends over an edit that was never written. For sounds the only trace is a
  warning in the log; for textures a notice raised as the editor closes.
- **Why it matters.** It is the question three rounds asked - "does a save that fails after `save` returned
  reach the command's answer" - answered "no" for the one kind of failure a library save has per entry: a file
  another program holds, a read-only file, a full disk. `Document::save_failed` now says "a tab leaves EVERY end
  of a save here"; `docs/STATE.md:96-99` says each command "waits in `save_tabs` until the tabs are clean or one
  of them answers why not". Here the tab is clean because the work was discarded.
- **Closes it.** A save keeps the edits it could not write (drop only the names written) and its `failed` list
  is the save's answer (`answer_job` reads the result the way `job_answer` does). One check beside
  `test_previews` K: a description that cannot be written, `doc.save` -> an error naming it, the tab still dirty.

### 2. `should` - on a `-nodlg` editor `project.open from_launch=true` drops the unsaved scene through the ordinary door

- **Where.** `crates/gz-project/src/cmd.rs:218-222`: unsaved work in the way and `from_launch` -> the launch
  question (`crates/gz-ui-app/src/launch.rs:18-34`), asked with the asker of the flag; under `-nodlg` it answers
  itself No and posts `project.open discard=true` with the window's origin, which `lets_go` lets through.
- **Seen** (the QA editor): a scene saved, a light moved to x = 7.5, `doc.list` names the scene under `unsaved`.
  `project.open path=<other> from_launch=true` sent through the `run` door -> `{"ok": true, "asking": true}`;
  two seconds later the other project is open. The first project opened again: the light at x = 0, no recovery
  snapshot.
- **Why it matters.** `docs/COMMANDS.md:53-55`: "`-nodlg` changes none of it: its logged answers are for the
  window's own actions, and a command's caller is refused with `unsaved` on any editor"; `lets_go`'s own comment
  says the same. It is the fix of round 4's blocker with one parameter left out of it, and every QA editor is
  such an editor. The card names `discard` alone under "Loses data with".
- **Closes it.** The hand-over asks only where a person answers (`!flags.unattended`); otherwise it is refused
  with `unsaved` like every other open. A check beside `test_project_browser` or `test_portable`.

### 3. `should` - a library edit left unsaved without a tab is unsaved work nobody lists: `app.quit` ends the editor over it

- **Where.** The pending edits are an `Editor` service (`library/stores.rs:1-5`); the only source of unsaved
  work that knows them is a tab (`gz-app`'s `unsaved.rs:49-56`, `TabsDirty`), and `library.set` needs none
  (`cmd.rs:692-694`: "False leaves the change as an edit of the tab ... for `doc.save`"). `app.quit` asks the
  tabs and the sources (`app_cmds.rs:265-275`); `update.apply` asks the sources.
- **Seen.** No library tab open: `library.set kind=sound_library item=qa\qa_tone set={"min_dist":3.5}
  save=false` -> `"saved": false, "dirty": true`. `doc.list` -> `"unsaved": []`; there is no tab for `doc.save`
  to save. `app.quit` through the `run` door, no parameter -> `{"ok": true, "closing": true}`, exit code 0.
- **Why it matters.** "Unsaved work ... is lost only through the destroy door; every other path refuses with
  `unsaved`" - the brief names "a library's pending edits" - and the command's own answer says `dirty: true`
  about something no other command can see or save.
- **Closes it.** The stores are a `DirtySource` of their own (listed by `doc.list`, saved by `save_all`,
  dropped by `discard`), or `library.set save=false` without the tab is refused / opens it.

### 4. `should` - a commandlet ends over an unsaved scene with exit code 0 and not a word

- **Where.** `crates/grizzly/src/commandlet.rs:288-303`: the steps run, the code is whether one failed; then
  `close_project` (`main.rs:227`), which "asks nothing". Graphs are saved by their close hook and, without a
  window, at once (`docs/STATE.md:90`); the scene is not.
- **Seen.** `Grizzly-Cmd.exe -project <p> script s1.txt` with `scene.new`, `scene.save path=maps/a.scene`,
  `scene.place kind=light position=[0,1,0]`, `scene.info`: every step ok, the last says `"modified": true`,
  exit 0, nothing on stderr. The next run: `scene.open path=maps/a.scene`, `scene.objects classes=light` ->
  `"total": 0`. By the code a single `run scene.place ...` is the same: it answers ok and nothing of it lasts.
- **Why it matters.** Point 6 (full headless work) and the round's own question ("on any editor ... a
  commandlet"). The rule the editor holds everywhere else - refuse with `unsaved`, or lose through `discard` - has
  no counterpart here, and the two document kinds differ without a sentence saying so.
- **Closes it.** A run that ends with unsaved documents says which on stderr and ends with 1 (the script adds
  `scene.save`), or the scene follows the graphs' rule; `docs/COMMANDS.md` section 4 says which. One case in
  `test_cli`.

### 5. `should` - two ends of a save that are not its answer, in the window (by the code; origin `Ui` cannot be driven)

- **The Mod Settings picture.** `save_now` hands the waiting picture to a task and forgets it
  (`crates/gz-ui-mod/src/settings.rs:512-520`: `pending.take()`, `making = true`); `dirty()` (`:567-569`) does
  not count `making`. With nothing else unsaved the tab is clean the moment `save` returns, so Exit / Close Tab /
  Switch Project > Save - and `doc.save`, `project.close save=true` - go on while `thumbnail.dds` is being placed
  and `mod.thumbnail` has not run: the project closed first gets a picture at its root that `mod.ltx` never
  names; the editor ended first may get neither. A failure goes to the field's problem (`:323-335`), never to
  `save_failed`, and the picture that waited is gone from the tab. Characters met the same and count their save
  (`dirty || saving`).
- **A library's Save while another library job runs.** `queue_save` finds `saving` set - a rebuild sets it
  (`sounds.rs:640-645`), a partial save of `library.set`, an import - and returns `Ok`, hanging its answer on
  that job (`sounds.rs:287-291`, `library/mod.rs:103-107`). That job ends well, saved nothing of this tab's
  edits, and the tab stays dirty: a person's wait has no deadline, so Exit > Save does nothing to see, and the
  next Save by hand ends the editor - the very case the brief asks about. A command gets "did not finish" after
  120 s.
- **Closes it.** The picture's save counts as unsaved until it is answered and leaves its failure in
  `save_failed`; a save that finds another job running starts its own when that one ends, or refuses with
  `busy` naming it.

### 6. `minor`

- **The manifest's check, what is left.** The page says "The check reads every value this page lists". Written
  by hand and passed in silence: `[command.<name>] cheat = maybe` (page: `true | false`), a layer's `opacity = 7`
  (page: 0 to 1), `api = banana`, `version =` (empty: `mod.set` refuses `''`), `[screen.Bad Id]` (page: the id
  takes a-z, 0-9 and _; `screens.set` turns it into `bad_id`), `versoin = 2.0` and `Tags = story` in `[module]`
  (a misspelled key of the one section every module has). `target = default` with `mode = night`, and `target =
  any` with `mode = night`, pass the check and `mod.set` alike although the page pairs `default` with no mode and
  `any` with `*`. `target = any` alone is `mod.target` "no target game mode ..." with the fix `target = default`:
  the sentence and the fix are a new project's, not this file's. No correct manifest was refused: the page's
  example and `target = any` + `mode = *` are clean.
- **The areas' problems are not at their line.** `E401` (a layer's file) lands on the `[screens]` header or, with
  no such section, on `mod.ltx:1:1`; `E409` / `W410` of `[track.t1]` on 1:1; `E207` of `[start_kit]` on
  `[world]`. The manifest's own problems are at their key since E7.
- **`screens.set`'s card against the page.** `rect`: "in shares of the screen" - `rect=[0,0,0.5,0.5]` is written
  as `0 0 0.5 0.5`, half a per cent (the page: per cent, default `0 0 100 100`); `fit`: "... or none", which the
  page does not have. `track_add file=<a picture>` converts the picture and writes a track the check then
  refuses (`E409`).
- **Raw system numbers.** The library job's sentence is "... could not be written: <the system's sentence in the
  system's language> (os error 5)"; `project.create` still says "(win32 N)" (`gz-project`'s `cmd.rs:622-626`), the
  deletion and one file operation too (`win32_text`).
- **A removed name in a comment**: `crates/gz-base/src/origin.rs:9-10`, "losing data takes the explicit
  `*_discard` command" (and "A confirmation is Yes", which the kit's `unsaved_prompt` answers Cancel).
- **`project.open save=true`** drops whatever is still unsaved after `save_all` (`cmd.rs:224-228`, `discard =
  ... || p.save`); `project.close save=true` refuses with "still unsaved" there (`:543-556`). By the code.
- **`docs/plan`** is still tracked: the author's call, as in every round.

## Checked and found sound

- **Round 4, finding 1: closed.** Driven on the QA editor (`-nodlg`), a scene and a graph unsaved, each through
  the `run` door with no parameter: `scene.new`, `scene.open`, `doc.reload` (scene and graph), `ui.tab
  action=close` (graph), `project.close`, `project.open` of another project, `app.switch_project`, `app.quit` ->
  `unsaved` every time, the light still at x = 7.5 and both documents still listed. `project.close discard=true`
  and `scene.new discard=true` at the `run` door -> `gate`, naming the destroy door. By the code: `scene.recover
  action=restore` refuses a dirty scene; `scene.convert` replaces nothing open; `project.create` refuses while a
  project is open; `update.apply` goes through `refuse_unsaved` (not driven: a QA run does not update);
  `level.play` replaces nothing (not driven: no game); the window's Switch Project names the scene and closes
  through `project.close` with the answer; `asset.take_theirs` is an undoable step of the document
  (`DocOp::Replace`, graphs read). The exceptions are findings 2-4.
- **Finding 4: closed as written.** Close Tab, a retarget, Switch Project and Exit go through `save_then`; the
  Level tab's save leaves every end in `save_failed` (`write_scene`, `GaveUp` dropped unrun when the dialog is
  cancelled, never started or refused), and `doc.save` answers `cancelled` quietly. Graph, Characters, PDA and
  Light Animations: every end is a clean tab or an answer. Open is finding 5 (and 1).
- **Finding 3: closed.** Sixteen paths from the commandlet: tooling at any depth, the editor's folders at the
  root in any case, device names, a trailing dot, a file in the way - each refused in a sentence without a
  system number; three quests in three folders got three names and `check` is clean. The link into a game is
  the suite's phase W (read, not run).
- **Finding 2: closed for what it named.** Version, website, tags, target's values, both mode ids, the
  repository, the description, a global's name and the area sections' keys and values are reported at their
  line with the page's words and the mechanical fix; `mod.set` refuses with the same sentence; a repeated
  key's wrong line is reported where it stands; `disable_game_updates = yes` is a warning. `gz-mod`'s tests green. Left over: the minors.
- **Round 4's minors**: `check` skips `temp\` and `releases\` unless asked (driven both ways); the two comments
  and the doubled one are rewritten; a save beside a running one hangs on it (its hole is finding 5).
- **Safety**: nothing of the bridge changed since round 4; every program I started was a QA run on loopback;
  no stall in any probe; the tree is as I found it but for this report.
- **Formatting and words**: `cargo fmt --all -- --check` and `check_words.py` clean.
- **The ten points as a whole.** 1: the four pictures show one frame and the kit. 2, 3, 9: not opened again;
  nothing of them changed. 4: the manifest walk above. 5: the cards answered what the walk needed (one card
  sentence in the minors). 6: finding 4. 7: no stall; the waits left are finding 5's. 8: projects made from
  nothing and opened by the commandlet and by the QA editor, each on a home of its own, without a guess. 10:
  0.7.0, nothing published; open is what is above.

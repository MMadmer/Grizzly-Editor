# Brief E1 - the library tabs get their commands; a surface says when it is not ready

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's first round is `docs/plan/reports/CRITIC.md`; you close its findings
**3, 7 and 8** and the first minor of 10. Read those findings there first - they name the files and lines - then
`CLAUDE.md` (the rules; sections "Commands", "Editor UI", "Dangerous operations") and `docs/COMMANDS.md`.

## What to build

### A. The three library tabs can be driven by commands (finding 8)

Today `library.view` only opens, selects and reads. Everything else the Texture Library, the Sound Library and the
Light Animations tab do is a toolbar button or a store command of the tab alone, and their files are binary
(`.thm`, `lanims.xr`), so "edit the file" is no way in. The rule (`docs/COMMANDS.md`, first paragraph): nothing can be
done one way and not another; a feature is a command and an action that runs it.

- **`library.set`** (write): changes what a library entry is described by - a texture's description (its `.thm`
  parameters), a sound's parameters, a light animation's properties and keys. One command, the kind and the entry
  named like `library.view` names them; the values as named keys (never positions). It answers what it changed.
  Unknown key: refused with the nearest real name. The tab's own edits go through the same code.
- **`library.play`** (view): plays or stops a sound's game sound or a light animation in its tab (Play, Stop, Auto
  Play). Without a window it says so (`no_window`), it does not pretend.
- **`library.import`** (write): brings pictures in as textures. `from=` names files anywhere on disk (or nothing:
  what the import folder already holds - today's Check New Textures / Import New Textures); they are copied into
  the import folder and imported, as one job with a stage a person can read and a Stop. The Sound Library's Rebuild
  is a command of the same family (decide whether it is `library.import kind=sound` or `library.rebuild`; one
  concept, one name).
- Every toolbar button of the three tabs becomes an **action** over one of these commands (`gz-app`'s action table:
  id, title, icon, keys, scope, whether it is on and why not). `library.check_textures` must do what its name says.
- **The Texture Library takes a drop**: picture files dropped from Explorer, and picture tiles dragged from the
  Content Browser, run `library.import` - through the kit's drop zones (`gz_ui::drop`, the way Screens & Music
  takes a file: `gz-ui-pda`'s `drop.rs`). The empty-state sentence no longer tells the author to put a file into
  the editor's import folder first; it says what the tab reads and that pictures can be dropped.
- Each new command: a card (`Spec`), a typed parameter struct (`Deserialize + JsonSchema`,
  `deny_unknown_fields`, a doc comment on every field), an example; `grizzly`'s test
  `every_card_is_worth_reading` must stay green. A call that overwrites or deletes a file names the parameter in
  `destroys_with` and loses data only through the destroy door.
- The Sound Library's toolbar ends its own row with More, like every other tab (minor of finding 10).

### B. The Characters tab shows the model of a character whose model is the game's (finding 3)

`crates/gz-ui-world/src/characters.rs` passes source words the model viewer does not know (`"visual"`, `"darf"`);
`gz-ui-previews`' `model_load.rs` knows `"game"`. Make the source a typed value both sides share (no free
string), and make `tools/qa/test_characters.py` prove that the embedded model is shown for a game model (not an
error state).

### C. A surface says when it shows an error or is still busy, and the tour fails on it (finding 7)

- `ui.audit` (`crates/gz-app/src/cmds.rs`, `crates/gz-ui/src/audit.rs`) already answers `loading`: the busy
  lines of `gz_ui::state::busy_line`. Make the kit's other "not ready" marks report the same way: the dialog
  footer's busy status (`crates/gz-ui/src/dialog.rs`, `Footer::busy`) and a card's busy state
  (`crates/gz-ui/src/card.rs`).
- Add **`errors`** to `ui.audit`: the kit's error states a frame shows (the full-surface error state of
  `gz_ui::state` - whatever draws "could not be read" / "cannot be decoded" in place of content). An entry names
  the surface and the sentence.
- `tools/qa/test_tour.py`: a tab or dialog that shows an error state FAILS (today it passes); it opens the
  texture tab with the game's texture (`source=game`), so the tab shows a picture; a dialog is pictured when its
  busy status is gone or, for a dialog whose check cannot end in a QA run (every service is a dead address), in
  the settled state it reaches - say in the report what each of the seven dialogs settles to.
- The texture tab: a file that is not there says so ("The file was not found." with where it looked), not
  "cannot be decoded as an image" (`crates/gz-ui-previews/src/texture_load.rs:383-386`).

## Rules

- Files that are yours: `crates/gz-ui-previews/**`, `crates/gz-ui-world/src/characters.rs` and `char_form.rs`,
  `crates/gz-ui/src/{audit,state,dialog,card}.rs`, `crates/gz-app/src/cmds.rs` (`ui.audit` only),
  `tools/qa/test_tour.py`, `test_previews.py`, `test_characters.py`. Another engineer (E2) works at the same time
  on `crates/gz-ui-viewport`, `crates/gz-ui-mod`, `crates/gz-ui/src/form.rs`, `crates/gz-ui-world/src/local.rs`,
  comments in `gz-scene` / `gz-levelbuild` / `gz-ui-level`, and `tools/qa/test_scene_tools.py`; the architect
  works on `crates/gz-scene/src/cmd*.rs`, `crates/gz-export-level`, `tools/rust`, `tools/qa/qa.py` and the
  documents. Do not edit a file that is not yours; if you need a change there, say so in your report.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build; never
  loop.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm): the architect commits. Do
  not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>` (e.g. `python tools/rust/limited.py 900 bash tools/rust/cg.sh build -p grizzly
  -p gz-mcp-bridge`). A run over two minutes is watched: say every two minutes how far it is. No unbounded `find` /
  `grep -r` over `Intermediate/` or `Bin/`.
- QA: `python tools/rust/limited.py 300 python tools/qa/run_all.py <suite> [--only <phases>]` - one suite at a time,
  only your three suites (the others belong to the people above; two runs of one suite at once collide). A suite
  opens only a COPY of a project (`qa.copy_project`); never open or change anything under `D:\XFinedProjects`,
  never talk to port 28016, never start an editor outside `qa.Editor`. Every program a suite starts is a QA run
  that reaches no real service (the QA client sets that up; do not weaken it, and never test against GitHub,
  VirusTotal or the catalog).
- Product text is English, official, short; UI through the kit (`gz-ui`) only: theme tokens, points, one Primary
  per toolbar. No `unsafe`, no `static` state, the UI thread never waits and makes no thread
  (`Executors::thread`). Comments only where the why is not obvious; never cite another program.
- The documents are the architect's: do not edit `docs/*.md` or `CLAUDE.md`. List in your report every sentence
  they must gain or change (the new commands' rows for `docs/UI_DESIGN.md` section 7 and `docs/COMMANDS.md`).

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning,
  `cargo fmt` clean (`python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints nothing),
  `python tools/rust/limited.py 60 python tools/rust/check_words.py` ok.
- The unit tests of the crates you changed green (`bash tools/rust/cg.sh test -p <crate>`).
- `test_tour`, `test_previews` and `test_characters` green, each proving what is new (the commands through
  `qa.cmd`, the drop through the kit's zone command the other tabs' suites use, the error state failing the
  tour, the character's model shown).
- `Intermediate/cargo/debug/Grizzly-Cmd.exe help library` lists the new commands with their parameters.
- Write `docs/plan/reports/E1.md`: what you built, the decisions you made, the commands' cards as `help` prints
  them, the document sentences to change, what you did NOT prove, and anything you found that is not yours.
  Answer with a summary of it.

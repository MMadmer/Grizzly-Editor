# Brief E4 - the libraries' two last commands are jobs; the library's writers pass the guard; one more pick list

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's second round is `docs/plan/reports/CRITIC.md`; you close its
findings **1** (the blocker), **3** (the writers' half), **4**, and the minors named below. Read those findings
there first - they give the files and lines - then `CLAUDE.md` ("Commands", "Editor UI", "Dangerous operations",
"Where a mod exports to") and `docs/COMMANDS.md`. Everything is in `crates/gz-ui-previews` and three suites.

## What to build

### A. `library.sync` and `library.rewrite_descriptions` are commands like every other (finding 1, blocker)

Today both go through the window: they queue the window's question ("Synchronize Textures", "Rewrite Texture
Descriptions") and answer `{"started": true}` at once - whether a person clicks or not, and under `-nodlg` when
the logged answer is No and nothing runs. From a commandlet `library.rewrite_descriptions` answers `started: true`
and does nothing; `library.sync` answers `no_window` for a file job that draws nothing. The rules they break:
"A command never asks a question", "Long work is a job the command waits for ... No command answers 'pending, ask
again'", "everything except the picture runs without a window".

- Each command **starts its job itself** on the editor's pools (`gz-task`: a name, stages a person can read, a
  Stop; the libraries' job group E1 made), **waits for it** and answers what it did (what was written, skipped,
  failed, by count and by name up to a few); `wait=false` answers the job at once. It runs with no window and no
  tab: from `Grizzly-Cmd.exe run library.sync kind=texture_library` too.
- **The question is the window action's.** The toolbar / More action (`library.sync_textures`,
  `library.sync_sounds`, the rewrite's) asks a person through the kit's confirm - naming what will be written -
  and then runs the command; under `-nodlg` the action's logged answer is No and nothing runs, but the COMMAND
  never asks. The way the Light Animations Reload does it since E1 (an action with a reply that sends the command
  again) is the model. If one of them loses nothing a person would miss (a synchronisation only makes files that
  are missing or older), it needs no question at all: decide per command and say why in the report.
- **Parameters of their own**, in the words the other library commands use: `kind=texture_library|sound_library`
  (today they say `textures|sounds` and borrow `library.reload`'s struct, so their cards list `discard` and kinds
  they refuse - and `library.reload`'s card lists `sounds` it refuses). Each command's card lists exactly what it
  takes; a typed enum, so a wrong kind is refused with the real ones. Remove the branch no call reaches
  (`libtools.rs:273`).
- Words of the messages the author reads (`libtools.rs` around 151, 158, 185: "Cube Map: scale image...",
  "Can't load texture '...'.", "Smaller cubemap successfully created [0.12 sec].") become sentences in the
  editor's tone ("The smaller cube map was made.", "The texture '...' could not be read: <why>.").

### B. The library's writers pass the guard (finding 3)

The library may be any folder, and until now nothing stopped it from being a game: Save of Light Animations
replaced `gamedata\lanims.xr` there, `library.set` / `library.rebuild` / `library.sync` wrote its
`gamedata\textures\*.dds` and `gamedata\sounds\*.ogg`. The rule: every writer passes the write guard before it
writes; nothing under a game is written.

- The guard exists: **`gz_project::library_guard(&editor) -> LibraryGuard`** (never touches the disk) and
  **`LibraryGuard::check(&path) -> Result<(), String>`** (blocking: call it on the IO pool, where the write
  happens; the error is the sentence to answer). `project.set_library` already refuses a game's folder.
- EVERY write of the library in `gz-ui-previews` checks it first: `library/texsync.rs` (around 78, 108, 112, 144,
  149, 158), `library/sounds.rs` (679-790), `library/lanim.rs` (849, `lanims.xr`), `libtools.rs` (116, 178),
  `library/textures.rs` (1182), and any other `write_atomic` / `fs::write` / `fs::copy` / `fs::rename` /
  `create_dir_all` / `remove_file` under the library's folder or the import folder. One helper of the crate that
  takes the guard and does the write is better than twenty call sites that remember to ask. A refused write is an
  error of the command or the job, in the guard's sentence; nothing is written before the check.
- A unit test with a fake game folder as the library (`tempfile`; no file named like a program): a save of each
  of the three libraries is refused and the folder is byte for byte what it was.

### C. The texture pick is the kit's (finding 4)

`crates/gz-ui-previews/src/local.rs` `local_texture_pick` (used in `library/textures.rs` for a description's
detail, bump and normal map) is the only `egui::ComboBox` outside the kit, with a search field and rows of its
own, and it shows the first 300 textures of a library without saying so. It becomes the kit's `Combo` with a
`PickList` (None as its first item; every texture, the list is virtualized) - look at how
`crates/gz-ui-world/src/picker.rs` `list_picker` does exactly this (the rows are made once per read of the list
and only while the popup is open). The local function goes.

### D. Minors of the same crate

- **The import folder keeps no copy.** `library.import from=<file>` copies the picture into the import folder,
  imports it and leaves `<name>.~png` there (`library/texsync.rs:158`). A picture the command itself staged is
  removed once it is in the library; a picture the author put into the import folder is treated as before.
- **The tour pictures a library with something picked.** `tools/qa/test_tour.py` opens Light Animations and the
  Sound Library with nothing selected, so the timeline, the key actions and the attenuation curve are in no
  picture: after opening each library tab the tour picks its first entry (`library.view`) before it audits and
  pictures. If the QA editor's library has no entry of a kind, say so in the report.
- **Comments that cite another program** in this crate: `libtools.rs:36` (`EDevice->ReloadTextures`),
  `library/texsync.rs:163` (`SynchronizeTextures(true, true, false)`), `local.rs:62` (`CreateChoose(smTexture)`),
  `library/sounds.rs:798` ("the curve the old property canvas plotted"), and `crates/gz-ui-previews/src/lib.rs:29`
  (a doc link to `local::local_checker`, which moved into the kit): each says what THIS code does and why, or
  goes. Never a function, class or member name of another program; "the X-Ray SDK" / "X-Ray's compiler" only
  where a file format is the reason.

## Tests

- `tools/qa/test_previews.py` phase K (it has a library of its own in a temp folder; nothing of the real library
  is touched): `library.sync kind=texture_library` and `kind=sound_library` make what is missing and answer what
  they made; `library.rewrite_descriptions` rewrites and answers the count; each answers a job with `wait=false`
  and `job.wait` ends it; an unknown kind is refused with the real ones; none of them opens a dialog
  (`ui.list`'s dialogs stay empty) - on an editor started WITHOUT `-nodlg` semantics mattering: the command must
  not depend on it.
- `tools/qa/test_cli.py`: one phase line - a commandlet synchronises a temp library
  (`Grizzly-Cmd.exe run library.sync kind=texture_library -root <temp library root> ...`: look at how the critic
  ran it, "an empty library of my own in the scratch folder (`-root`, `-home` there)"), exit 0, the file is there.
- The guard's unit test (B).

## Rules

- Files that are yours: `crates/gz-ui-previews/**`, `tools/qa/test_previews.py`, `tools/qa/test_tour.py`,
  `tools/qa/test_cli.py`. Another engineer (E5) works at the same time on `crates/gz-ui/src/{tests,kitsheet,
  file_dialog}.rs`, `crates/gz-ui-level`, `crates/gz-ui-world`, `crates/gz-ui-mod`, `crates/gz-app/src/contract.rs`
  and other suites; the architect on comments in `gz-scene`, `gz-lc*`, `gz-fmt-ogf`, `gz-entities`, `gz-particles`,
  `gz-levelbuild`, on `tools/rust` and the documents. Do not edit a file that is not yours; if you need a change
  there (a kit widget lacks something), say so in your report and work around it inside your crate.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm, mv): the architect commits.
  Do not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>`. Format only your own files (`rustfmt --edition 2024 <file>`), never
  `cargo fmt --all` without `--check`. A run over two minutes is watched: say every two minutes how far it is.
  No unbounded `find` / `grep -r` over `Intermediate/` or `Bin/`.
- QA only through `python tools/rust/limited.py 300 python tools/qa/run_all.py <suite> [--only <phases>]`, your
  three suites, one at a time, on copies and temp folders. Never open or change anything under `D:\XFinedProjects`,
  never talk to port 28016, never start an editor outside `qa.Editor` (a commandlet in `test_cli` is started the
  way that suite already starts them), never weaken the QA client's network isolation. NEVER point a library at
  the installed game or the repository's own library in a test that writes: a temp folder always.
- Product text is English, official, short; UI through the kit only. No `unsafe`, no `static` state; the UI
  thread never waits and makes no thread (`Executors::thread`).
- The documents are the architect's: list in your report every sentence of `docs/*.md` / `CLAUDE.md` to change
  (the two commands' rows and parameters).

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning in
  your crate, `python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints none of your files,
  `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of yours.
- `bash tools/rust/cg.sh test -p gz-ui-previews` green; `test_previews`, `test_tour`, `test_cli` green.
- `Intermediate/cargo/debug/Grizzly-Cmd.exe help library.sync` and `help library.rewrite_descriptions` show cards
  that list exactly what each takes.
- Write `docs/plan/reports/E4.md`: what you built, the decisions (which command asks in the window and why),
  the cards as `help` prints them, the document sentences to change, what you did NOT prove. Answer with a
  summary of it.

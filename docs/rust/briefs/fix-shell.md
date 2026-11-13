# Brief: the last review's findings - the editor's program, three choosers, what the shell still calls missing

The last review (`docs/rust/briefs/critic-close.md`; its scripts under `Intermediate/critic-close/`)
found no harm done, and seven things that keep "Open: nothing" from being true. You close the
ones in the editor's shell. The reviewer's words are quoted; check each in the code first.

**Another engineer works in the tree at the same time** on the game and Play
(`crates/gz-levelbuild`, `crates/gz-game`, `tools/qa/test_play_game.py`, the Play entries of
`crates/gz-mcp/tools.json`, and the game's repository). The lead edits the documents
(`docs/RUST_MIGRATION.md`, `CLAUDE.md`, `docs/rust/PARITY.md`, `docs/rust/CONVENTIONS.md`,
`tools/rust/notices.py`, `tools/qa/compare_compile.py`). Your files: `crates/grizzly`,
`crates/gz-win`, `crates/gz-app`, `crates/gz-ui-level`, `crates/gz-scene`, `crates/gz-entities`,
the format crates a chooser's list needs, `tools/qa/test_properties.py`, `tools/qa/test_ui_shell.py`,
`tools/qa/test_level_commands.py`, `docs/UI_DESIGN.md`, and a section of your own appended to
`docs/rust/STATE.md` ("The last review, the shell"). If a build fails in a file that is not
yours, wait a minute and build again; never edit it.

## 1. `Grizzly.exe` is a window program

> `crates/grizzly/src/main.rs:1` - `Grizzly.exe` is a console program. Its PE subsystem is 3 in
> every build; `LevelEditor.exe` is 2. Nothing sets `windows_subsystem` or frees the console.
> Started from Explorer it gets a console window, and closing that ends the editor with no
> unsaved prompt.

- The editor's subsystem is Windows: started from Explorer, a shortcut or the taskbar it shows
  its own window and nothing else, and no console can end it behind the author's back.
- Started from a terminal or by a tool it still speaks: `-dump_commands`, `-catalogue`,
  `-headless -compile <dir>`, `-compile_details`, the start's profile and every other line it
  prints reach a parent's console when there is one (attach to the parent's console - the Win32
  for it lives in `gz-win`, with its `// SAFETY:` lines) and reach a pipe or a file when its
  output is redirected (the QA tools and `diff_compile.py` read its stdout through a pipe: they
  must read exactly what they read today). A panic's message and the crash report still land
  where they land today.
- Whatever the editor starts in turn shows no console: every `Command` of a console program has
  `CREATE_NO_WINDOW` (the two in `gz-app` and `gz-github` have it; check every other
  `Command::new` outside `crates/gz-levelbuild`, which is the other engineer's - they add theirs).
- `grizzly-mcp.exe` stays a console program: it is a stdio server.
- Proof: a check of the built programs' subsystem beside the icon check
  (`tools/rust/check_icon.py` reads the PE already: the editor 2, the bridge 3; `check.sh` runs
  it on the release build) - add it there, the script is yours for this; `Grizzly.exe
  -dump_commands > file` and `| more` both give the list; `python tools/qa/run_all.py
  test_server_lifecycle test_ui_shell test_mcp_bridge`; `python tools/qa/diff_compile.py --scene
  mp_test` (it reads the compile's log from stdout); and say what a start from Explorer shows
  now (start it once the way Explorer does - `explorer.exe <path to exe>` on a COPY of a project
  is not needed: the Start page with a home of its own (`GRIZZLY_HOME`) is enough - and list
  the process's windows: one, the editor's; close it over MCP).

## 2. Three choosers list what they are for

> `crates/gz-ui-level/src/choose.rs:113` - three choosers cannot list. The dialog says "The
> editor cannot list these yet: type the value." `ListKey::of_choose` (`lists.rs:39`) knows no
> `skeletonanims`, `skeletonbones`, `gameanim`, which `gz-scene/src/props.rs:611` emits for
> Model\Animation, Fixed bones and Motion (`gz-entities/src/props.rs:474,487,654`). The C++
> filled all three (`cpp-editor-last:Source/Editors/XrECore/Editor/EditorChooseEvents.h:535-539`).

- The three lists, as the C++ filled them (read `EditorChooseEvents.h` and what it calls at the
  tag): the motions of the spawn's model, the bones of the spawn's model, the game's `.anm`
  animations - read off the UI thread, cached like the other chooser lists, with the picker's
  search; "what comes from a list is PICKED, not typed" (`CLAUDE.md`).
- `property_choose` and `property_inspector` over MCP answer the same lists; the typed fallback
  and its sentence go. A list that is truly empty (a model without motions) says so.
- Tests in the crates that read the lists; `tools/qa/test_properties.py` holds one row of each
  kind on the Escape scene or the smallest scene that has such a spawn.

## 3. What the shell still calls "not part of this build"

> Build-out text: `grizzly/src/main.rs:212`, `gz-app/src/cmds.rs:458`,
> `gz-app/src/chrome.rs:1618,2172,2213`, `gz-app/src/shell.rs:299,554,1169`,
> `gz-ui-level/src/status.rs:168`, `gz-app/src/cpp.rs:681,701` (says the Open Scene and Save As
> dialogs are missing; they exist).

For each place: can the editor as built reach it? A branch for a surface that every build
installs is dead - remove it, or, where the shell must hold on a builder that installed
nothing (a test of `gz-app` alone), say what is true without "yet". `cpp.rs`'s `COMMAND_LOAD`
without a scene and `COMMAND_SAVE` with Save As: do what the C++ rows did with the dialogs the
editor has (UI: the dialog; MCP: the refusal that names the argument, never a window) - the
differential run compares these rows (`python tools/qa/diff_editors.py --scenario` of the
scenario that holds them: find it). `rg -n "not part of this build|cannot list these yet"
crates` ends with nothing but the module build's sentence about files of a target.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index. The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`. Scripts longer than a line
  go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- A wait is 300 s at most, with the run's log read between waits; nothing that passed is run
  again to check something new; no series of runs. The author watches the list of your
  background commands.
- `unsafe` only in `gz-win`, every block with its `// SAFETY:` line; no global state.
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from; the game
  is only read. Every editor you start runs on a port and a home of its own and is closed over
  MCP; an editor on port 28016 is the author's: never touch it. No test writes a program file.
  Nothing is pushed or sent anywhere.
- Proof at the end, once: `bash tools/rust/check.sh` (on a release build of yours for the
  subsystem check: `pwsh -NoProfile -File ./Build.ps1`), `bash tools/rust/test.sh`, the suites
  named above and `test_properties`, the differential scenarios you touched.

## Report

Under 350 words: the subsystem before and after, what a start from Explorer and from a
terminal shows, what reads the editor's output and still does; the three lists (where each
comes from, how many entries on the scene you used); each "not part of this build" place and
what became of it; the proofs with their times; anything still open and exactly why.

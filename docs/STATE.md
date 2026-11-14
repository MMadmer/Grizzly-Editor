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
  Stop keeps what ended; the compiler's output matches its records byte for byte
  (`tools/qa/golden/compile`).
- **The game runs what the editor builds** (`test_play_game`, `--with-game`): a new game on the built level; the
  installed game is listed before and after and is equal.
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
- **The game half of the graphs** (the game repository's `tools/nq/run_lua.py test_examples.lua`, its catalog
  parity test): disabled nodes pass through, a file with a `canvas` loads.

## 2. Not proven by a run

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
- The sound preview's playback in the chooser.
- What the manifest's reference says a key of `[world]`, `[start_kit]` or `[screens]` does in the game: it is
  read from the readers' code and the game's document, not from a run of the game. `mod.package` was seen
  refusing a manifest through the check on one wrong tag; the other errors take the same path.
- The window's questions over unsaved work - Exit, Close Tab, a tab that switches its file, Switch Project -
  and the Save dialog of a scene that has no file, picked or cancelled: a person's clicks and the system's
  dialog, which no suite answers. The commands behind them are driven (`test_scene_tools` Z, `test_pda` H);
  the window's own paths are held by the code they share with them (`save_tabs`).

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
- A failed step does not stop the build: the steps that need it are left for later, the rest run, and the answer
  names all three.
- Play's gamedata is a mirror of the game's: hard links where the volume allows, copies otherwise.

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

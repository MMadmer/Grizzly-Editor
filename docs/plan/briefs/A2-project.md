# Brief A2 - projects that stand alone

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5), **`docs/ASSETS.md` sections 5, 8, 9** (the contract
you implement), `docs/COMMANDS.md`, the survey `Intermediate/plan2/assets.md` sections 5 and 6 (every absolute path
that is persisted today, how a project is opened, with `file:line`), and `docs/plan/reports/A1-luadoc.md` (the
lossless document API of `gz-luadata` you write the project file with).

You own: `crates/gz-project` (except its watcher, which stream A1 owns), `crates/gz-game`, `crates/gz-win` (what you
need from Win32: the registry for the file association, the single-instance handover), `crates/grizzly`'s command
line for opening a project (`cli.rs`: the positional path; the architect owns the commandlet verbs in the same
file - re-read before editing), the project side of `crates/gz-app` (`project.rs`) and of `crates/gz-ui-app`
(`link.rs`, the Start page's reading of the project list - not its layout). Stream K restyles `gz-app` and the kit
at the same time: keep your edits to the project code paths.

## 1. The project file

- `<Name>.grizzly` at the project's root (`docs/ASSETS.md` 8): Lua data, header `grizzly = 1`, `id` (a UUID made
  once, never changed), `scene` (project-relative, `/`), `library` (the list of section 9). Read and written
  through `LuaDoc` (`apply_diff`): unknown keys and comments survive. Register the kind in `gz-project`'s
  `asset_kind.rs` (it is the editor's own kind: an icon, never a tile in the Content Browser - it is the
  project) and with `gz-asset` when that crate exists (a check: the header, the scene exists, the id is a UUID).
- Its name is the folder's name when the file is made; renaming the file renames nothing else. Exactly one
  `*.grizzly` per project root: two is a problem `asset.check` names.
- **What is a project**: a folder holding a `*.grizzly` or a `mod.ltx`. A folder with `mod.ltx` and no project file
  is opened by writing the file (a fresh clone). `project.ltx` is read once when it exists - `last_scene` goes to
  the project file's `scene`, `game_root` and `[xms] export_target` to this machine's state - and then deleted.
  No other code reads or writes `project.ltx` afterwards (survey 5 lists every reader: `sdk_root` is dropped).
- The project file is the author's (shipped in git): never in the editor's `.gitignore` template
  (`rules.rs` `ensure_gitignore` - take `project.ltx` out of it), never exported into a build (`is_editor_only`).

## 2. This machine's state lives in the editor's home

`<home>\projects\<project id>.ltx` (LTX, through `LtxDoc`): the game this project is linked to when it is not the
default, the last export target, recent scenes, the tabs to bring back (`temp\session.ltx` moves here: `temp\` is
scratch a `git clean` removes), the recovery manifest's `source=` becomes project-relative. Nothing under the
project names a place outside it: prove it with a test that opens a project, works in it (open a scene, export,
package), then greps every file the editor wrote inside the project for the old absolute root and the machine's
name.

`_projects.ini` stays the Start page's memory (paths and last-opened times), keyed by path as now; a project that
moved shows as Missing until it is opened from its new place, which replaces the entry with the same `id`.

## 3. The game is found, not asked for

- The editor has ONE default game: `[game] root` in `grizzly_editor.ltx` (Preferences > Game: the folder, its
  version, **Change...**, **Find Again**). When unset or invalid it is found by itself, off the UI thread, in this
  order: the game roots this home has linked before (the per-project state files, the old `project.ltx` values
  seen while converting), the installed Dead Air: Refined (the installer's uninstall entry in the registry - read
  the game repo's installer scripts under `D:\Games\Dead Air\DeadAir-x64` for the key it writes; do not guess),
  an `xrEngine.exe` next to or above the editor's own folder. The first one that is a valid game
  (`gz-game`'s existing validation) wins and is saved.
- A project opens against the default game without a question. **Link Game** (the Start page's card, the Mod
  menu) sets this project's own game in the machine state; "Use Default Game" clears it. The Link Game card
  appears only when no valid game is known at all, and then it offers every candidate it found as a pick list
  (audit `ux_rest.md` B.2: "No folder selected" twice and nothing guessed).
- `gz-game`'s commands, typed: `game.link` (`path`, `scope: project | default`), `game.info` (root, version,
  abilities, how it was found), `game.lookup` (`type`, `query`, `limit`, `offset`: one command for every list
  the linked game offers - spawn sections, modes, and the picker types `gz-nq` serves today through
  `quest_lookup`: move the dispatch here and let `gz-nq` register its pickers into it; `quest_lookup`,
  `list_spawn_sections`, `game_modes` go).

## 4. Opening from outside

- `Grizzly.exe <path>` - a positional argument that is a `*.grizzly` file, a `mod.ltx` or a folder opens that
  project (`cli.rs`: today it is logged as unknown; `-project` keeps working and takes a file too).
- **The file association**: at start the author's own editor (not a run with `-home`, `-nodlg`, `-headless` or a
  commandlet verb) registers `.grizzly` for the current user (`HKCU\Software\Classes`: the extension, a ProgID
  `GrizzlyEditor.Project` with the editor's icon and `"<exe>" "%1"`), when it is missing or points at a program
  that no longer exists; Preferences > General > "Open .grizzly Files with This Editor" (on) and **Register Now**.
  A development build registers too (it is what the author runs). `gz-win` does the registry work; no `unsafe`
  beyond what the calls need, each block with `// SAFETY:`.
- **One editor per home**: a second `Grizzly.exe` with a project to open asks the running one (its control
  endpoint) to open it - `project.open path= from_launch=true`, which answers the unsaved-work question in the
  window as any Switch Project does - brings its window to the front and exits 0. Without a project to open, a
  second start just brings the window forward. A run with its own `-home` or a commandlet is its own instance and
  never hands over. The lock is a file in the home (`editor.lock`: pid and port), not a guess from the port.

## 5. Working without the editor: `AGENTS.md`

When a project is created, and once when a project that has none is first opened by this version (remember that
it was written - `agents_guide = written` in the machine state - so a deleted or edited file is never recreated),
write `AGENTS.md` at the root: twenty to thirty lines, generated from the project (its id, module, the kinds of
assets it holds and where), saying what `docs/ASSETS.md` 5 says: the files are the source of truth; the syntax in
three lines; `Grizzly-Cmd.exe check`, `reference <kind>`, `build`, with the editor's real path on this machine NOT
written (the command is found on PATH or beside the editor: say "the editor's folder"); MCP is for the window and
the 3D scene. `AGENTS.md` and `CLAUDE.md` at a project's root are source-only (`rules.rs` `is_source_only`): never
exported, never packaged. A command `project.guide` rewrites it on request.

## 6. The library outside the project (`docs/ASSETS.md` 9)

- Resolution order for a scene's objects, groups and textures: the project, the linked game, the editor's
  library. The library's folder is a preference (Preferences > Library; default: the `rawdata` and `gamedata`
  beside the editor) - every fixed join of the data root that means "the library" (`gz-ui-previews/src/paths.rs`,
  the scene's library service, the viewport's loader: survey 5 lists them) reads that one setting.
- **Copy on place**: an API `gz_project::library::bring_in(refs) -> job` that copies a library object, its
  textures (`.tga` / `.dds`) and thumbnails (`.thm`) into the project under the same relative paths, through
  `WriteGuard`, skipping what the project or the game already holds. The scene stream calls it when the author
  places a library object; you build and test the API and call it from today's place path
  (`gz-scene`'s `place_object`, one call site) so it works now.
- The project file's `library` list: what the open project's scenes reference that resolves only in the editor's
  library - kept current when a scene is saved (an API the scene's save calls; today's save gets the one call).
- `project.collect` (write; a job with a card; `dry_run` answers the file count and bytes first): brings in
  everything the `library` list names; afterwards the list is empty. Project menu: **Collect Library Assets...**,
  which shows the size and asks once (kit confirm, not a danger).
- A reference that resolves nowhere is a problem (`asset.check`: "props\box_wood_01 is in no library: ...") and
  never stops a scene from opening.

## 7. The project's commands, typed

Port `gz-project`'s commands to cards (`docs/COMMANDS.md`), finishing each rename (UI callers, QA suites,
documents):

| Today | Becomes |
|---|---|
| `list_projects`, `open_project`, `project_close`, `project_create`, `project_delete`, `project_forget`, `project_reveal` | `project.list`, `project.open` (`path`: a folder or a project file; `save`, `discard`), `project.close` (`save`, `discard`), `project.create`, `project.delete` (destroy), `project.forget`, `project.reveal` |
| `recovery` | stays for the scene stream |
| `content_copy / move / delete / mkdir / rename / create`, `list_assets` | `file.copy`, `file.move`, `file.delete` (destroy), `file.mkdir`, `file.rename`, `file.create` (`kind`: a template), `file.list` (`path`, `kind`, `query`, `limit`, `offset`) |
| `darf_list`, `darf_copy` | `file.import` (`from: library | game`, `path`, `to`; `overwrite` is `destroys_with`) and `file.list source=library|game` |
| new | `project.collect`, `project.guide`, `project.info` (the file, the id, the game in use and why, what `library` names) |

The `*_discard` twins of the bridge are gone: `discard` is a parameter, declared with `destroys_with`.

## Proof

- Rust tests: the project file round trip (unknown keys, comments), the conversion from `project.ltx`, the
  machine state, the game finder with a fake registry and fake folders, the association's registry writes
  against a test hive path (never the real `HKCU\Software\Classes` in a test), `bring_in` and `collect` on temp
  folders (harmless files only).
- `tools/qa/test_project_browser.py` rewritten for the new names and extended; a new phase or suite
  `test_portable.py`: a copy of a project is opened, closed, the folder MOVED to another path, a fresh home
  made, the editor started on the project FILE (positional), and it opens with the game found, the scene up, no
  absolute path written inside, no `project.ltx`, `AGENTS.md` present; then a second start hands over to the
  first; then `project.collect dry_run`.

## Done means

PLAN section 5 "A2", every sentence, each proven by a named test or phase; `docs/assets/PROJECT.md` written (the
file's keys, the machine state, the association, opening, the library); the Start page and Link Game work on the
new model.

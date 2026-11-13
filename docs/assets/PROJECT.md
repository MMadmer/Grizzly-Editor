# The project

A project is a folder of plain files: a module for Dead Air: Refined, its sources, its scenes and the author's
own notes. Nothing in it names a place outside it - it can be moved, renamed, copied to another disk or another
computer, or cloned from git, and it opens the same. This page is the reference of what makes a folder a project
(`docs/ASSETS.md` sections 5, 8 and 9 are the contract; `Grizzly-Cmd.exe reference project` prints this page).

## What is a project

A folder that holds a **project file** (`*.grizzly`) or a **`mod.ltx`**. A folder with `mod.ltx` and no project
file - a fresh clone of a mod's repository - is opened by writing the project file. A folder with an old
`project.ltx` is a project as well: the first open reads it once and deletes it (below).

## The project file

`<Name>.grizzly` at the project's root, Lua data like every asset (`docs/ASSETS.md` section 1):

```lua
return {
  grizzly = 1,                                    -- the kind and its format version
  id = "7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11",    -- names this project on this and every machine
  scene = "levels/escape.scene",                  -- the scene that opens with the project
  library = { "props\\box_wood_01" },             -- what the scenes take from the editor's library
}
```

| Key | | |
|---|---|---|
| `grizzly` | required | The format version. A newer one than the editor reads opens nothing and loses nothing: "made by a newer Grizzly Editor". A file without it gets it at the next save. |
| `id` | required | A UUID, made once when the file is made and never changed. The editor keys this machine's state of the project by it, and the Start page recognises a moved project by it. A file without one gets one when the project opens. |
| `scene` | optional | The scene the project opens with, project-relative with `/`. The editor writes it when the project closes with a scene open. A scene that is not there is skipped. |
| `library` | optional | What the project's scenes take from the editor's library (section "The library"), as the library spells it. Kept current when a scene is saved; emptied by Collect Library Assets. |

- The file is the author's: it goes into git (the editor's `.gitignore` leaves it in) and never into a build or a
  release. It is not a tile of the Content Browser: it is the project.
- Its name is the folder's name when the file is made; renaming the file renames nothing else, and the folder may
  be renamed freely.
- Exactly one project file per project root. Two are a problem `asset.check` names (`E404`); the open takes the
  first by name.
- It is read and written as a lossless document: comments, unknown keys and layout survive every save, and a file
  the editor did not change is never rewritten.
- `asset.check` names: `E401` an id that is missing or no UUID, `W402` a scene that does not exist, `W403` a
  library reference that resolves nowhere, `E404` a second project file.

## This machine's state

What belongs to this computer lives in the editor's home (`-home`, `GRIZZLY_HOME`, else the editor's data root),
never in the project:

| File | Holds |
|---|---|
| `<home>\projects\<id>.ltx` | where the project was last opened, the game it is linked to when that is not the default one (`[game] root`), the last export target (`[build] export_target`), the recent scenes, the scene a load was under way for (a load that brings the editor down is not tried again at the next start), whether `AGENTS.md` was written |
| `<home>\projects\<id>.tabs.ltx` | the tabs the project brings back |
| `<home>\machine.ltx` | the default game (`[game] root`, `found`), the game roots this home has known (`[seen]`), the library folder (`[library] root`), whether `.grizzly` files open with this editor (`[association] on`) |
| `<home>\_projects.ini` | the Start page's memory: the folders opened last with their ids, the New Project location, the sort |
| `<home>\editor.lock` | the editor that holds this home while it runs: its process id and its control port |

Losing any of them loses conveniences only. A project that moved shows on the Start page as Missing until it is
opened from its new place, which takes the old entry's place by the id. The recovery snapshots of a scene
(`backup\recovery`) name their scene project-relative.

## The game

The editor has one **default game**: Preferences > Game (its folder, its version, how it was found, **Change...**,
**Find Again**). While it is unset or no game any more, the editor finds it by itself, off the UI thread, in this
order, and the first that is a game wins and is saved:

1. a game this home has been linked to before (the projects' state files, an old `project.ltx` converted);
2. the installed Dead Air: Refined, by its installer's uninstall entry -
   `Software\Microsoft\Windows\CurrentVersion\Uninstall\{9732DFF1-E40D-4B23-B215-6D28B1DD0DE0}_is1`,
   `InstallLocation`, the current user's then the machine's;
3. an `xrEngine.exe` next to or above the editor's own folder.

A project opens against the default game without a question. A project may have a game of its own on this
machine (Preferences > Game > This Project, or `game.link scope=project path=...`; **Use Default Game** gives it
back). The Start page's **Link Game** card appears only when no valid game is known at all, and offers every place
the editor looked at as a pick list. `game.info` says which game is read, its version and abilities and how it
was found; `game.info candidates=true` lists every place looked at.

## Opening a project

- The Start page: a card, **New Project**, **Open Folder...**.
- `Grizzly.exe <path>`: a project file, a `mod.ltx` or a folder; `-project <path>` the same (and a listed
  project's name). Over MCP and in a script: `project.open path=...`.
- A double click on the project file: the author's own editor (not a run with `-home`, `GRIZZLY_HOME`, `-nodlg`,
  `-headless` or a commandlet) registers `.grizzly` for the current user at start - `HKCU\Software\Classes`: the
  extension, the type `GrizzlyEditor.Project` with the editor's icon and `"<exe>" "%1"` - when it is missing or
  names a program that no longer exists. Preferences > General > **Open .grizzly Files with This Editor** turns it
  off; **Register Now** (`project.associate now=true`) writes it at once. A development build registers too.
- **One editor per home.** A second start with a project to open hands it to the editor that holds the home
  (`project.open from_launch=true`: unsaved work in the way is asked about in that editor's window, as Switch
  Project asks), lets it bring its window to the front, and ends. Without a project to open, the second start
  only brings that window forward. A commandlet and a compile run are runs of their own and never hand over.

The first open of a folder by this editor:

1. writes the module skeleton (`mod.ltx`) when there is none, and `.gitignore` once in a git working copy;
2. reads the project file - or writes `<Folder>.grizzly` with a new id;
3. converts an old `project.ltx`: its `last_scene` becomes the project file's `scene`, its `game_root` and
   `[xms] export_target` go to this machine's state, and the file is deleted. Nothing reads it again;
4. writes `AGENTS.md` (below) unless the project has one;
5. finds the default game when none is known.

## AGENTS.md

A page at the root for whoever works on the files without the editor: what the project is (its file, its id, its
module), the kinds of assets it holds and where, the syntax in three lines, the commands of `Grizzly-Cmd.exe`
(`check`, `reference <kind>`, `build`) and that MCP is for the window and the 3D scene. It names no place outside
the project. The editor writes it when a project is created and once when it first opens a project that has none;
it never writes it again by itself - a deleted or edited page stays so. `project.guide` writes it anew. Like
`CLAUDE.md`, it is never exported or packaged.

## The library

A scene's objects, groups and textures resolve in this order: the project, the game, the editor's library.
The library is a folder holding `rawdata` and `gamedata` (Preferences > Library; by default the editor's own,
beside it; `project.set_library`).

- **What the author places from the library is copied into the project as it is placed**: the object
  (`rawdata\objects\<ref>.object`), its thumbnail (`.thm`) and the textures its surfaces name
  (`rawdata\textures\*.tga` and `.thm`, `gamedata\textures\*.dds` and `.thm`), under the same relative paths -
  skipping what the project already holds and the textures the game holds. A job of its own does it.
- What a scene still takes from the library - a base scene imported whole keeps taking its objects from there -
  is named in the project file's `library` list, kept current when a scene is saved.
- **Project > Collect Library Assets...** (`project.collect`; `dry_run=true` answers the files and bytes first)
  copies everything the list names into the project; the list is then empty and the project complete by itself.
- A reference that resolves nowhere never stops a scene from opening: `asset.check` names it (`W403`) with what
  would resolve it.

## Commands

`project.list`, `project.open`, `project.close`, `project.create`, `project.delete`, `project.forget`,
`project.reveal`, `project.info` (the file, the id, the game in use and why, the library list),
`project.guide`, `project.collect`, `project.set_library`, `project.associate`; `game.link`, `game.info`,
`game.lookup`; `file.list`, `file.copy`, `file.move`, `file.delete`, `file.mkdir`, `file.rename`, `file.create`,
`file.import`. `help about=project` prints their parameters.

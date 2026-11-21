# Grizzly Editor commands

Everything the editor can do is a **command**: one name, one typed set of parameters, one answer. The window's
menus, buttons, shortcuts and palette run commands; an agent runs them over MCP; a shell runs them as commandlets.
There is one model and three ways in, so nothing can be done one way and not another. This document is the contract
of the model; the commands themselves are documented by the editor (`help`), from the code.

## 1. A command

```rust
/// The parameters: a struct, a doc comment on every field. The request is read into it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Place {
    /// What to place.
    kind: PlaceKind,
    /// The library reference of an object, or the section of a spawn.
    r#ref: String,
    /// Where, in metres.
    position: [f32; 3],
    /// Degrees. Left out: no rotation.
    #[serde(default)]
    rotation: Option<[f32; 3]>,
}

reg.area("scene", "The open level: its objects, the selection, transforms");
reg.cmd(
    Spec::write("scene.place", "Places an object, a light, a spawn point or another thing in the scene.")
        .example("kind=object ref=props/box_wood_01 position=1,0,2"),
    |ed, p: Place, origin| async move { place(&ed, p, origin).await },
);
```

- **Name**: `area.verb` or `area.verb_noun`, lower case, the area a noun (`scene`, `asset`, `mod`), the verb what
  happens (`place`, `check`, `build`). The few that belong to no area have one word (`help`, `state`, `ping`).
- **Card** (`gz_editor::Spec`): the name, a one-line summary, the **effect**, an example. `Registry::cmd` /
  `cmd_sync` register it; `Registry::area` describes an area; `Registry::topic` adds a longer text to `help`.
- **Parameters** are read by the registry, never by the command: an unknown parameter is refused with the nearest
  real name, a missing or mistyped one with the command's usage. A value written as text where a number, a flag or
  a list belongs is taken for what it says (`"5"`, `"yes"`, `"1,2,3"`), so a command line and a JSON caller pass the
  same things. `null` is "not given".
- **Effect** - what the call can do, which decides the door it may come through (section 4):
  `read` (changes nothing), `view` (changes what the editor shows, nothing an author keeps), `write` (changes the
  project or the settings; undoable or repeatable), `destroy` (loses data for good), `publish` (reaches other
  people). A parameter that turns a call into a losing one (`discard`, `overwrite`), or the value of one that does
  (`action=discard`), is named in the card (`destroys_with`): with it the call counts as `destroy`. A `read`
  command writes no file: where the same thing can be read and written, they are two commands (`scene.info`
  answers a scene's summary, `scene.summary` writes it as text).
- A command never opens a question. It did what it was asked, or it says why not. A tab's save that a command
  asks for (`doc.save`, `ui.tab action=close save=true`, `app.quit save=true`) is held to it: what the window's
  Save would ask is refused, naming the command that takes the answer as a parameter - `scene.save path=` for a
  scene without a file; `library.import overwrite=true`, code `exists`, for New Textures that would replace a
  texture - and a save that fails on the pools is the command's answer, with its code. `-nodlg` changes none
  of it: its logged answers are for the window's own actions, and a command's caller is refused with
  `unsaved` on any editor. `app.switch_project` is `project.close` with the caller's `save` or `discard`.
- `file.create` makes the folder it is asked to put the asset into, and refuses one the project never lists
  (a tooling name at any depth, the editor's own folders at the root), a name Windows keeps for a device and
  a file in the way, each in a sentence; the new file's name is free in the whole project, because the
  asset's id comes from it; the file passes the write guard where it really lands.
- `mod.set` turns a value away in the manifest's own sentence (`<key> takes <values>, not '<value>'`), the
  one the file's check gives; `mod.build` and `mod.package` refuse with every error of the manifest, one a
  line, and report its warnings; `mod.info` lists both. A target and a mode that contradict each other
  (`target = default` with a mode, `any` without `mode = *`, `existing` with `plain` alone) are refused by
  `mod.set` in the check's sentence when the request touches them; a choice still without its detail
  (`existing` with no mode yet) is written. `mode` takes one id, a list (`"plain, metro"` or an array) or `*`,
  and a mode set without a target sets it (but `*`, which needs `target = any`).
- `screens.set` takes a layer's `rect` in per cent of the screen and a track as an `.ogg`; a word the game
  would drop (`fit`, `show`, `play`, `hide`, `end`) is refused by name, before anything is written.

## 2. Parameter names

One concept has one name in every command.

| Concept | Name | Form |
|---|---|---|
| a file or folder of the project | `path` | project-relative, `/` or `\`; absolute where the thing may be outside |
| where something is copied or moved | `to` | as `path` |
| scene objects | `objects` | a list of names; left out: the selection |
| one scene object | `object` | its name |
| a library reference (object, group, visual) | `ref` | as the library spells it |
| a spawn section | `section` | |
| which variant of a command | `kind`, or `action` when it is a verb | one word of a fixed list |
| search text | `query` | substring; an area that ranks says so |
| paging | `limit`, `offset` | lists answer `total` when cut |
| a position, a rotation, a scale | `position`, `rotation`, `scale` | `[x,y,z]` metres / degrees / factor |
| a toggle | `on` | bool |
| unsaved work in the way | `save` (save it first), `discard` (lose it) | bool; `discard` is `destroy` |
| where a level's light is baked | `light` | `auto` (left out), `gpu`, `cpu` |
| which part of a thing a read answers | `part` | one word of a fixed list (`level.inspect part=pieces`) |
| a job | `job` | its id |
| waiting for a job the command starts | `wait` | bool, default true |
| an open document | `doc` | the tab's id: `level`, `quest:<path>`, `characters`, ... |
| a UI length | pt both ways | pictures are px and say their `scale` |

## 3. Answers

- Success: `{"ok": true, ...}` with the command's own fields. Failure: `{"ok": false, "error": "<a sentence for
  the caller>", "code": "<what to branch on>", ...}`.
- Codes: `invalid` (the request; carries `usage`), `not_found`, `no_project`, `no_window` (the command needs a
  window and this run has none), `unsaved` (carries `dirty`; send `save` or `discard`), `busy` (carries the job),
  `refused` (a rule says no; the sentence says which), `gate` (wrong door, section 4), `io`, `failed`, `internal`.
- Lists are under a key named after what they hold (`objects`, `files`, `problems`), cut by `limit` (default 100)
  with `total`.
- Paths inside the project are answered project-relative with `/`; anything else absolute.
- A problem is `{file, line, column, code, severity, text, fix?}` everywhere.
- **Long work is a job.** A command that starts one answers when the job has ended, with its result; `wait=false`
  answers `{"job": <id>}` at once. `job.list`, `job.get`, `job.wait`, `job.stop` work on any job. A request that
  outlasts the transport's timeout is answered `{"ok": false, "code": "running", "job": ...}`: not a failure -
  follow the job. No command answers "pending, ask again".
- A command whose answer is a drawn frame (`ui.*`, the pictures) is held until the frames it needs were presented.

## 4. The three ways in

**The window.** A menu item, a toolbar button, a shortcut and a palette row are an *action*: a command name with
its arguments, a title, an icon and default keys (`gz-app`'s `actions.rs`). Direct manipulation - dragging a gizmo,
typing in a field - goes to a store's commands as before; everything with a name on it is a command.

**MCP** - the bridge `grizzly-mcp.exe`, six tools:

| Tool | Runs | |
|---|---|---|
| `grizzly_help` | `help about=` | the areas, an area's commands, a command's parameters, a topic |
| `grizzly_state` | `state` | what is open, what is running, what is wrong - one call to get oriented |
| `grizzly_run` | any `read` / `view` / `write` command, or several in order | the ordinary door |
| `grizzly_destroy` | a `destroy` call | `destructiveHint`: a client asks its user on exactly these |
| `grizzly_release` | a `publish` call | carries the player-safety rules, word for word |
| `grizzly_look` | `shot.*` | a picture of the window, the viewport or an asset, as an image |

A command sent through the wrong door is refused with the right door's name (`code: gate`). Assets are not edited
through MCP: they are text files (`docs/ASSETS.md`), edited with the caller's own file tools and checked with
`asset.check`.

**A shell** - commandlets, `Grizzly-Cmd.exe`:

```text
Grizzly-Cmd.exe run <command> [name=value ...]     one command, its answer on stdout, exit 0 / 1
Grizzly-Cmd.exe script <file>                      a command per line (- reads stdin), stops at the first failure
Grizzly-Cmd.exe help [area | command | topic]
Grizzly-Cmd.exe check [path]                       = run asset.check
Grizzly-Cmd.exe reference <kind>                   = run asset.reference
Grizzly-Cmd.exe build | package | update           = run mod.build / mod.package / update.apply
Grizzly-Cmd.exe serve                              the editor without a window, on its control endpoint
```

A commandlet is the editor without a window: it opens the project (`-project <folder or project file>`, else the
current folder when it is a project), waits until the project is ready, runs, waits for every job the commands
started, prints and ends. It listens on no port, writes its own log, and never asks. Nothing saves by itself:
a run that ends with unsaved work - a scene a step changed and none saved - loses it, so it says which on stderr
and ends with 1; the script saves (`scene.save`, `doc.save`). `-json` prints the raw answer,
`-quiet` drops the job progress it otherwise writes to stderr, `-keep_going` runs a script past a failure. Exit
codes: 0 done, 1 a command failed, 2 the run could not start. A first word that is neither a verb nor a project
(a path, a project file, something that is there) is a usage line and code 2: a mistyped verb opens no window.

**The socket** (127.0.0.1, the QA suites and the bridge): one JSON object a line - `{"cmd": "scene.place", ...the
parameters beside it}` or `{"cmd": ..., "args": {...}}`; `"gate": "run" | "destroy" | "release"` when a door
applies; `{"batch": [ ... ]}` runs several in order and answers `results`. One answer a line.

## 5. The areas

`help` is the authority; this is the map.

| Area | What |
|---|---|
| `app` | the program: version, quit, preferences, keys, layout |
| `job` | every long piece of work: list, get, wait, stop |
| `project` | open, close, create, delete, the project list, recovery, collect, the Start page (`project.page`) |
| `game` | the linked game: link, what it offers (modes, sections, the pickers' lists: `game.lookup`) |
| `file` | the project's files: list, copy, move, rename, delete, create from a template, import |
| `asset` | the text assets: kinds, check, reference, find, migrate |
| `content` | the Content Browser as the window shows it: source, Library kind, folder, search, selection (`content.view`) |
| `preview`, `library` | the model and texture previews; the Texture, Sound and Light Animation libraries |
| `mod` | the module: info, build, package, exclude, thumbnail |
| `release` | GitHub releases and the Mod Browser: publish, list, edit, delete, the catalog |
| `screens`, `world`, `characters`, `pda` | what those tabs do beyond editing their files (imports, presets, previews) |
| `text` | a text file in the Text tab: open, a line, find, the Markdown view, what it shows (`text.view`) |
| `script` | the scripts' declarations, the linked game's and the project's: where a name is declared and what a prefix completes to (`script.lookup`) |
| `scene` | the open level: objects, selection, transforms, properties, tools, undo |
| `level` | building and running: build, status, problems, compile, play, export; what a build made (`level.inspect`) |
| `graph` | an open quest or behaviour graph: edit operations, layout, find |
| `doc` | any open document: list, save, undo, redo, reload, close |
| `viewport` | the camera, the view modes, gestures |
| `ui` | the window: tabs, panels, drawers, dialogs, popups, scroll, splits, scale, audit, the contract |
| `shot` | pictures: the window, the viewport, an asset |
| `update` | the editor's own updates (`docs/UPDATES.md`) |
| `feedback` | bug reports and suggestions to the editor's author, a crash offered at the next start (`docs/REPORTS.md`) |
| `perf` | frame pacing, UI-thread stalls |

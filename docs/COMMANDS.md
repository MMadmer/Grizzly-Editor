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
  people). A parameter that turns a call into a losing one (`discard`) is named in the card
  (`destroys_with`): with it the call counts as `destroy`.
- A command never opens a question. It did what it was asked, or it says why not.

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
started, prints and ends. It listens on no port, writes its own log, and never asks. `-json` prints the raw answer,
`-quiet` drops the job progress it otherwise writes to stderr, `-keep_going` runs a script past a failure. Exit
codes: 0 done, 1 a command failed, 2 the run could not start.

**The socket** (127.0.0.1, the QA suites and the bridge): one JSON object a line - `{"cmd": "scene.place", ...the
parameters beside it}` or `{"cmd": ..., "args": {...}}`; `"gate": "run" | "destroy" | "release"` when a door
applies; `{"batch": [ ... ]}` runs several in order and answers `results`. One answer a line.

## 5. The areas

`help` is the authority; this is the map.

| Area | What |
|---|---|
| `app` | the program: version, quit, preferences, keys, layout |
| `job` | every long piece of work: list, get, wait, stop |
| `project` | open, close, create, delete, the project list, recovery, collect |
| `game` | the linked game: link, what it offers (modes, sections, the pickers' lists: `game.lookup`) |
| `file` | the project's files: list, copy, move, rename, delete, create from a template, import |
| `asset` | the text assets: kinds, check, reference, find, migrate |
| `mod` | the module: info, build, package, exclude, thumbnail |
| `release` | GitHub releases and the Mod Browser: publish, list, edit, delete, the catalog |
| `screens`, `world`, `characters`, `pda` | what those tabs do beyond editing their files (imports, presets, previews) |
| `scene` | the open level: objects, selection, transforms, properties, tools, undo |
| `level` | building and running: build, status, problems, compile, play, export |
| `graph` | an open quest or behaviour graph: edit operations, layout, find |
| `doc` | any open document: list, save, undo, redo, reload, close |
| `viewport` | the camera, the view modes, gestures |
| `ui` | the window: tabs, panels, drawers, dialogs, popups, scroll, splits, scale, audit, the contract |
| `shot` | pictures: the window, the viewport, an asset |
| `update` | the editor's own updates (`docs/UPDATES.md`) |
| `perf` | frame pacing, UI-thread stalls |

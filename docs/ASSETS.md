# Grizzly Editor assets and projects

Everything an author makes with the editor is a text file an agent can create and edit directly with its own file
tools - no editor, no MCP. This document is the contract of that: the one syntax, the rules every asset kind keeps,
how the editor reads, writes and follows the files, and what a project is. It is changed here first.

The per-kind references: `docs/nq/NQ_FORMAT.md` (quests and behaviour graphs), `docs/pda/PDA_FORMAT.md` (PDA
interfaces), `docs/assets/CHARACTER.md`, `docs/assets/SCENE.md`, `docs/assets/PROJECT.md`. `asset.reference`
prints any of them together with what only the linked game knows (the node kinds, the pickers' lists).

## 1. The syntax: Lua data

One syntax for every asset: a Lua file that is `return { ... }` of literals - tables, strings, numbers, booleans.
No code: an identifier used as a value, a call, an operator or a second statement is refused, by the editor and by
the game alike.

```lua
-- comments are the author's and survive every save
return {
  nq = 1,                                   -- the kind's header key and the format version
  id = "night_watch",
  title = { rus = "Ночной дозор", eng = "Night Watch" },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "wait" } },
    { id = "wait", kind = "wait.time", params = { seconds = 6 }, out = { done = "finish" } },
    { id = "finish", kind = "flow.end", params = { status = "completed" } },
  },
}
```

Why this and not JSON, TOML or YAML: it is what the game reads natively (no generated file between an author's text
and the game), it has comments, trailing commas, bare keys and long strings (`[[ ... ]]`, which is where dialogue
lives), it nests as deep as a graph needs, and a model writes it as fluently as any of the others.

`mod.ltx` stays LTX: it is the game's manifest, read by the game's own reader and by the frozen release tooling,
and the editor already edits it without losing a byte it does not change.

## 2. The rules every kind keeps

1. **A header key names the kind and the version**: `nq = 1`, `beh = 1`, `npc = 1`, `pda = 1`, `scene = 1`,
   `group = 1`, `grizzly = 1` (the project). The extension is the kind; the header has to agree with it.
2. **Read every version up to the current one; refuse a newer one, saying so.** A file of an older version is
   migrated in memory when it is read (a chain of small steps, one per version, in the kind's crate) and written
   in the current version the next time the editor saves it. `asset.migrate` does it for a whole project. A file
   of a newer version opens nothing and loses nothing: "made by a newer Grizzly Editor" with the Update action.
3. **Unknown keys are the author's.** A key the editor does not know is kept exactly where it is, at every level,
   through every save. It is listed once as a note (`unknown key 'x' (kept)`), never as an error: tomorrow's
   editor, another tool or the author may know what it is.
4. **Missing is fine.** Everything that has a default may be left out; the reader fills it, the validator does not
   complain, and the writer never materialises a default the author did not write. The editor adds what a format
   REQUIRES and the file lacks (the header key) when it saves.
5. **Names, not positions.** Things are referred to by `id` / `name`, never by an index or a number that shifts
   when something is inserted. Links are written at their source and name their target.
6. **Nothing derived is stored.** No generated outline, no counts, no caches, no absolute path, no machine name.
   Coordinates of graph nodes (`pos`) are optional: a file without them is laid out when it is opened.
7. **One thing per block.** A node, an object, a row is one table on its own line or block, so a text tool's
   search-and-replace has a unique anchor (the `id` / `name`) and a diff shows exactly what changed.
8. **Editor-only data** lives where the game ignores it: `pos` and `comment` on a node, an `editor = { ... }` table
   at the top (quests, behaviour graphs, characters). The game reads none of it.

## 3. How the editor writes: nothing it did not change

`gz-luadata` keeps a file as a lossless document (`LuaDoc`): the syntax tree with every comment, blank line,
quoting style, number spelling and key order. Reading gives the typed model its values; **saving applies the
difference** between the model as it was read and the model as it is now:

- a changed value is rewritten in place; a new key or item is inserted in the kind's canonical place, laid out the
  way the file around it is; a removed one goes with the comment attached to it;
- everything else - other keys, their order, comments, layout, unknown keys - stays byte for byte;
- a file the editor did not change is never rewritten: opening and saving it leaves it identical;
- a new file is written canonically: the header key first, then the kind's own key order, two-space indent, one
  item per line, width 100.

Items of a list that carry an `id` or a `name` are matched by it, so reordering nodes in the file or in the editor
is not a rewrite of every node.

## 4. How the editor follows the files

- **One watcher per project** (`gz-project`): every open document, the content browser, the validators and the
  caches hear about a change from it. Nothing polls and nothing has to be refreshed by hand.
- A file changed outside the editor is re-read at once. An open tab without unsaved edits shows the new content.
  An open tab WITH unsaved edits **rebases** them: its own difference (section 3) is applied to the new text. Only
  when both sides changed the same value does the tab keep its own and say so in a bar - "Changed on disk too:
  <what>" with **Take Theirs** per item.
- A file that does not parse opens nothing over the last good state: the tab shows the reader's sentence with the
  line and column, and comes back by itself when the file is fixed.
- **Problems are positioned.** Every problem a validator finds carries the file, line and column of the thing it
  is about, its code (`E030`), its sentence, and - where the fix is mechanical - what to write instead. The same
  list is the tab's Problems, `asset.check` over MCP and `Grizzly-Cmd.exe check` in a shell.

## 5. Working on a project without the editor

A project is a folder of plain files. An agent (or any tool) works on it directly:

1. Read `AGENTS.md` at the project's root: what the project is, where things are, the three commands below. The
   editor writes it once when it creates or first opens a project (never again after it was deleted or changed).
2. Edit or create the files.
3. `Grizzly-Cmd.exe check [path]` - every problem of the project, or of one file, with positions; exit code 1
   when an error is among them. No window, a second or two.
4. `Grizzly-Cmd.exe reference <kind>` - the format reference of a kind, with what the linked game adds;
   `reference manifest` is `mod.ltx`'s, every section and key with its values.
5. `Grizzly-Cmd.exe build` / `package` - the same builds the editor's buttons run.

An editor that has the project open follows every such change (section 4): the two can work on one project at
once. MCP is for what files cannot do - looking at the editor, driving its UI, the 3D scene's tools.

## 6. The kinds

| Kind | File | Header | Read by | Reference |
|---|---|---|---|---|
| Project | `<Name>.grizzly` | `grizzly` | the editor | `docs/assets/PROJECT.md` |
| Module manifest | `mod.ltx` | `[module] api` | the game, the editor | `asset.reference kind=manifest`; the game's `MODDING.md` is the normative text |
| Quest | `*.nqasset` | `nq` | the game, the editor | `docs/nq/NQ_FORMAT.md` |
| Behaviour graph | `*.behasset` | `beh` | the game, the editor | `docs/nq/NQ_FORMAT.md` section 10 |
| Character | `*.npcasset` | `npc` | the game, the editor | `docs/assets/CHARACTER.md` |
| PDA interface | `*.pdaasset` | `pda` | the game, the editor | `docs/pda/PDA_FORMAT.md` |
| Scene | `*.scene` + its folder | `scene` | the editor | `docs/assets/SCENE.md` |
| Object group | `*.group` | `group` | the editor | `docs/assets/SCENE.md` |

`gz-asset` is the one list of them (`AssetKind`: extension, header key, current version, migrations, validator,
reference, template, icon). The content browser, `asset.*`, the watcher and the build read that list; a new kind
is one entry there.

Binary by nature, and staying so: geometry (`.object`, `.ogf`), textures (`.dds`, `.tga`, `.thm`), sounds, the
painted layers of a scene (AI map, detail objects, wallmarks - sidecar files the scene names), and everything
the level build produces for the game.

## 7. The scene

A scene is `<name>.scene` and the folder `<name>\` beside it:

```text
escape.scene              the scene: header, level options, build settings, bookmarks
escape\
  objects.lua             static objects
  lights.lua  glows.lua  sounds.lua  particles.lua  shapes.lua  fog.lua
  spawns.lua              spawn points: section, transform and NAMED properties
  ways.lua                way paths: named points and links
  zones.lua               sectors and portals
  groups.lua              placed groups
  ai_map.bin  details.bin  wallmarks.bin      the painted layers
```

- One object is one table: `{ name = "box_01", ref = "props\\box_wood_01", pos = { 1, 0, 2 }, rot = { 0, 90, 0 } }`.
  Angles are degrees, `scale` is left out when it is 1, flags are named booleans and only the ones that are on.
- A spawn point's properties are the entity's named properties (`story_id`, `custom_data = [[ ... ]]`, `visual`,
  ...), never a positional packet. A property the editor cannot name is kept under `raw` and written back as it
  was.
- A file of a class that has no objects does not exist.
- The X-Ray SDK scene (`*.level` with `*.part` files) is read and never written: opening one converts it, saving
  writes a `.scene`, and `asset.migrate` converts a project's scenes in one run.

## 8. The project

`<Name>.grizzly` at the root of a project folder is the project: double-clicking it opens the project in the editor
(the editor registers the extension for the current user), and so does `Grizzly.exe <path>` with the file or the
folder. A folder with `mod.ltx` and no project file is a project too (a fresh clone): opening it writes the file.

```lua
return {
  grizzly = 1,
  id = "7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11",   -- names this project on this and every machine
  scene = "levels/escape.scene",                  -- the scene that opens with the project
  library = { "props\\box_wood_01" },             -- what the scenes take from outside the project (section 9)
}
```

- **Nothing in a project names a place outside it.** No absolute path, no machine name: the folder can be moved,
  renamed, copied to another disk or another computer, cloned from git.
- **What is this machine's lives in the editor's home**, by the project's `id`: the linked game when it is not
  the default one, the last export target, the open tabs, recent scenes. Losing it loses conveniences only.
- **The game link is the editor's**: one default game (Preferences > Game), found by itself - the last one
  linked, else the installed Dead Air: Refined. A project opens against it without a question; a project may
  name another one (Link Game), which is this machine's state.
- `project.ltx` is gone: an old one is read once for its game link and last scene, then deleted.
- The project list (`_projects.ini`) is only the Start page's memory: a project that is not on it opens the same.
- A second editor started on a project hands the request to the running one and exits.

## 9. What a scene takes from outside the project

A scene's objects, groups and textures resolve in this order: the project, the linked game, the editor's library
(Preferences > Library, by default the `rawdata` beside the editor).

- **What the author places from the library is copied into the project as it is placed** - the object, its
  textures and thumbnails - so everything a project adds is in the project.
- A base scene imported whole keeps taking its objects from the library; the project file's `library` list names
  them, so another machine knows what is needed. **Project > Collect Library Assets** (`project.collect`) copies
  them all in - the project is then complete by itself - and says the size first.
- A reference that resolves nowhere never stops a scene from opening: the object is a marked placeholder, the
  Problems list names it with what would resolve it, and the build refuses only what it really needs.

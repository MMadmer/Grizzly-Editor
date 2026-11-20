# Grizzly Editor assets and projects

Everything an author makes with the editor is a text file an agent can create and edit directly with its own file
tools - no editor, no MCP. This document is the contract of that: the one syntax, the rules every asset kind keeps,
how the editor reads, writes and follows the files, and what a project is. It is changed here first.

The per-kind references: `docs/nq/NQ_FORMAT.md` (quests and behaviour graphs), `docs/pda/PDA_FORMAT.md` (PDA
interfaces), `docs/assets/CHARACTER.md`, `docs/assets/ITEM.md`, `docs/assets/SCENE.md`, `docs/assets/PROJECT.md`,
`docs/assets/MODEL.md`, `docs/assets/TEXTURE.md`, `docs/assets/SOUND.md`. `asset.reference`
prints any of them together with what only the linked game knows (the node kinds, the pickers' lists, the item
catalog).

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

1. **A header key names the kind and the version**: `nq = 1`, `beh = 1`, `npc = 1`, `pda = 1`, `item = 1`,
   `trade = 1`, `loot = 1`, `recipe = 1`, `scene = 2`, `group = 1`, `mesh = 1`, `tex = 1`, `snd = 1`,
   `grizzly = 1` (the project). The extension is the kind; the
   header has to agree with it.
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
9. **One thing, one place.** A file says what ITS thing is and nothing of another thing; a relation between two
   things is written once, in the file of the side that owns it, and the other side only shows it as a fact with
   the file to open. An item says what the item is - never who sells it (the trader's shelf: a trader file, or
   the shop of a character of the module), who carries it (a loot table) or what makes it (a recipe). A track
   names its sound and carries none of the sound's marks. No kind has a key that writes "a line of some other
   config": that is what a patch (`patch\*.ltxp`) is for, in the open. The formats enforce it - the reader of a
   kind refuses a key that is another kind's to say, by name and with where it belongs (an item's `sold`,
   `loot`, `craft`, `lists`: `I432`; a row of an item's `sections` about another item asset's section: `I433`),
   the build refuses the module, and the game never reads such a key. A new kind, or a new key of one, is held
   to the same question first: whose is it to say?

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
  list is the tab's Problems, `asset.check` over MCP and `Grizzly-Cmd.exe check` in a shell. A text file that is no asset - a
  script (LuaJIT's parser: a syntax error at its line and column), XML or JSON that does not read - is checked the
  same way and its problems are in the same list (`gz-text`).

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
| Item | `*.itemasset` | `item` | the game, the editor | `docs/assets/ITEM.md` |
| Trader | `*.tradeasset` | `trade` | the game, the editor | `docs/assets/TRADER.md` |
| Loot table | `*.lootasset` | `loot` | the game, the editor | `docs/assets/LOOT.md` |
| Recipes | `*.recipeasset` | `recipe` | the game, the editor | `docs/assets/RECIPES.md` |
| Scene | `*.scene` + its folder | `scene` | the editor | `docs/assets/SCENE.md` |
| Object group | `*.group` | `group` | the editor | `docs/assets/SCENE.md` |
| Model | `*.meshasset` | `mesh` | the game (a dynamic model), the editor | `docs/assets/MODEL.md` (a source file, a model of the game by `source = "game:..."`, or `parts`) |
| Texture | `*.texasset` | `tex` | the game, the editor | `docs/assets/TEXTURE.md` |
| Sound | `*.sndasset` | `snd` | the game, the editor | `docs/assets/SOUND.md` |

`gz-asset` is the one list of them (`AssetKind`: extension, header key, current version, migrations, validator,
reference, template, icon). The content browser, `asset.*`, the watcher and the build read that list; a new kind
is one entry there.

**Media: the description is text, the data stays binary.** A model, a texture and a sound are text assets (the
three media kinds above) beside a **source** of a common format - glTF, OBJ or an X-Ray SDK `.object`; PNG, JPG,
TGA, BMP or DDS; WAV or OGG - or, for a model, with its geometry written in it (`parts`). The asset is what
everything refers to (a scene object's `ref` is the asset's project path); the source is its companion. The
description is the module's: the build publishes a copy of the asset's text where the game looks for its `name`
(`gamedata\textures\<name>.texasset`, `sounds\<name>.sndasset`, `meshes\<name>.meshasset`) and the game reads
it over the data - a texture's description in place of a `.thm`, a sound's parameters over its `.ogg`, a model's
surfaces, bones and motions over its `.ogf`. The data is binary by nature and is **baked** into the game's own
containers beside it (`.dds` and the bump pair, `.ogg`, `.ogf` and `.omf`; a static model is compiled into the
level instead), cached in the project's `temp\media\`; the asset where it lies and its source never ship as they
are - but a source that is the game's container already (below).

**A file already in the game's container is published as it is.** A `.dds` (with its `#` companion), an `.ogg`
whose own parameters are the asset's - or that has none the game reads and an asset that states none -, a compiled
`.ogf` of the project that the asset changes nothing of: the build ships the file itself, byte for byte, and bakes
nothing; over such a picture the keys that say how a picture is made (`format`, `mips`, the fades, ...) apply to
nothing and are a warning. Such a file under the module's own `gamedata\textures\`, `sounds\` or `meshes\` has its
asset beside it, named by its path below that folder (`gamedata\textures\wpn\ak.texasset` describes `wpn\ak`): a
module that ships the game's formats - a classic mod made into one - keeps every file where the game looks for it
and says what the game reads of a `.thm` or an `.ogg`'s comment in the asset's text. `asset.import` makes those
assets in place, each read back from the text it writes before it is trusted: a `.thm` beside a picture becomes
its asset's keys (every key the game reads stated, so the game's own `.thm` under that name says nothing more;
none only the SDK reads) and stays beside it - the asset's from then on, read by nobody and never shipped -
unless the caller says `remove_descriptions` - the destroy door, since what only the SDK read of it goes too; a `.thm` alone becomes `source = "game:<name>"`, a description
of the game's texture; an `.ogg`'s comment becomes the sound's keys; a `.ogf` there is a dynamic model. A picture
the game cannot load by its name (`<name>.dds.dds`, a name ending in a space) or that is no DDS gets no asset and
does not ship (the game could not read it). A file an asset cannot say the same way (a `.thm` or a sound's
parameters the asset's keys would change) stays a file of its own with what goes with it, and ships as it is: the
game reads it as it did. A file of the game's formats in `gamedata` outside its kind's folder (a level's lightmap,
which the game reads where it lies) stays a file of its own too.

**A raw file is the project's only through its wrapper.** A raw file is one of a format a media asset wraps - a
picture (`.png`, `.jpg`, `.jpeg`, `.tga`, `.bmp`, `.dds`), a sound (`.wav`, `.ogg`), a model (`.glb`, `.gltf`,
`.obj`, `.ogf`) - or a file that goes with one (`.thm`, a glTF's `.bin`, an OBJ's `.mtl`, a bump pair's
`<stem>#.dds`); an X-Ray SDK `.object` is a library object of its own. One raw file has one asset, and the asset is all there is of it: the content browser shows the
asset and never the raw file, and the build ships what the asset publishes and never the raw file on its own. A
raw file two assets wrap is an error on both (`M117`). A raw file no asset wraps is nowhere: the browser does not
list it and the build leaves it out (`left_out`, "raw data no asset wraps") - but for what another thing names as its
own data and ships or packs itself: an item's icon picture (packed into the module's atlas), a screen's layer and
a track's file (`mod.ltx`), the module's `thumbnail.dds`, what `[vfs]` mounts, what a level build writes
(`levels\<level>\`, `gamedata\levels\<level>\`), and the files of the module's game data an asset cannot say
(above). An asset that `[exclude]` leaves out is no wrapper: its source is a raw file again. The X-Ray SDK library (`rawdata\`) is a library of its own and
keeps its files. A file of a format no asset wraps (a script, a config, Markdown, XML) is the project's as it is.
**A raw file in the module's game folders becomes an asset by itself**: under `gamedata\textures\`, `sounds\` and
`meshes\` the editor imports every raw file no asset wraps, in place, when the project opens and when one appears
there - never replacing a file; a commandlet does not watch, and every module build does it first, so a file put
there a moment ago ships in any build (a job, `asset.import` without `remove_descriptions`: a
`.thm` beside a picture goes with it, a `#` companion with its texture; the `.thm` stays in the project as the
X-Ray SDK's word, and ships no more - its asset says all the game reads of it, and a `.thm` that says otherwise is
`M118`). Elsewhere a raw file is brought in by an import (a drop, Add > Import, `asset.import`). **`asset.export`**
writes an asset's raw file back where the author says (a folder of the computer, never inside the project or a
game), as it was imported: its source with what goes with it (a glTF's buffers and pictures, an OBJ's materials, a
bump pair, the `.thm`), each where it lay beside the source; a description of the game's texture or model writes
the game's file (the `.dds`, the `.ogf`). A model of `parts` has no file: its geometry is the asset's text. A
source is its asset's wherever it lies - in the module's own game data too, where the asset says where it ships;
a file an asset only names (a picture a model wears) ships by its own name unless an asset of its own wraps it.

Binary by nature, and staying so: the X-Ray SDK library as it is (`rawdata\objects\*.object`, `.thm`,
`.wav`, read as before; an asset may wrap an `.object` to restate its surfaces), the painted layers of a scene
(AI map, detail objects, wallmarks - sidecar files the scene names), and everything the level build produces for
the game.

## 7. The scene

A scene is `<name>.scene` and the folder `<name>\` beside it:

```text
escape.scene              the scene: header, level options, build settings, the sun, the camera
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
- **A scene over a level of the game** says `base = "<level>"` (the scene format's version 2): the level's own
  objects are read from the game library as the scene opens (`docs/GAME_LIBRARY.md`, 5) and are never written;
  the files hold what the module adds - all of it the module's, with no `authored` - and, in `spawns.lua`,
  entries of a `name` and `removed = true` for the spawn points of the level the module takes out. A zone's
  volume is the spawn point's `shape` and nothing repeats it; a spawn point's `section` is the game's, or one
  the module makes: of a character or an item (`<module>_<id>`), or in a config of its own. What a level takes
  from a module beside objects and spawn points - the lights that shine, sounds, effects, glows, fog volumes,
  sound environments - is the scene's own lists of them; the build writes them where the game reads them
  (`docs/GAME_LIBRARY.md`, 7).
- A file of the folder that does not parse changes nothing: the open scene keeps the objects it had of it, the
  problem is a `syntax` error at its line and column, and a save leaves the file as it is; once it parses the scene
  follows it again. A scene first opened with such a file opens without that file's objects.
- A group written with a `ref` and no `members` is placed when the scene is read: its members are made from the
  group asset at the group's `pos`, `rot` and `scale`, named `<group>_<member>`, and the scene is unsaved until the
  next save writes them; an asset that cannot be read is an error at the group's line, and the group stays as
  written.
- The X-Ray SDK scene (`*.level` with `*.part` files) is read and never written: opening one converts it, saving
  writes a `.scene`, and `scene.convert` writes a `.scene` beside a `.level` without opening it; `asset.migrate`
  does not take a `.level`.

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

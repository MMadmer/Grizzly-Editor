# Level design by an agent

How a place in a level is built by editing text files: what to ask the editor, what to write, how to know it is
right. Everything here works over MCP, over the socket and from a shell (`Grizzly-Cmd.exe run <command> ...`,
no window) - only the lit pictures (`shot.viewport`, `shot.models`) need a window. `help about=level-design` prints
the short form (section 9, the same text).

A scene cannot be seen by whoever writes it as text. So the work is a loop: **look** (numbers and pictures),
**write** (the scene's files and assets), **settle** (onto the ground), **check** (the audit, a picture), and only
then **build** and **play**. Never write coordinates from memory: ask where the ground is.

## 1. What is where: look before writing

| Question | Ask |
|---|---|
| What does this place look like from above, and where is everything? | `shot.map position=<x,0,z> size=<m>` - every object in a colour of its own with its name, a grid in metres with coordinates in the margins, spawn points, lights, ways and volumes as signs. `legend` lists every object with its colour, middle and area; a pixel reads back to metres through `px_per_m` and `map` (`x = area.x0 + (px - map.left) / px_per_m`, `z = area.z1 - (py - map.top) / px_per_m`). |
| How high is the ground, and how does it slope? | `shot.map mode=height` (colours, contour lines, the heights written where grid lines cross), or `scene.query action=heights area=<x0,z0,x1,z1> step=<m>` for the numbers: rows from `z0` up, each from `x0`, `null` where nothing is; at most 257 x 257 samples (a bigger request is refused naming a step that fits). |
| What is under these points? | `scene.query action=ground positions=[[x,y,z],...] top=true` - the floor, its normal and slope, the object and its surface (texture, game material). Up to 1024 points a call. `top` starts above everything: under a tree that is its crown, so add `only=<the ground's object>` for the ground itself. |
| What is in this box? | `scene.query action=box min=<x,y,z> max=<x,y,z>` (or `position=` + `size=`; `exact=true` tests triangles); `scene.objects near=<x,y,z> radius=<m> sort=distance bounds=true` or `box=` for the objects' own boxes. |
| Is there a clear line between two points? | `scene.query action=ray from=<x,y,z> to=<x,y,z>` - the first thing in the way, with its surface; `all=true` for every one, nearest first. |
| How big is this model, what are its surfaces? | `model.info refs=<ref,...>` - the box in model space, the size, triangles, every surface with its texture, shaders and game material. `model.list query=<text> sizes=true` finds models; `shot.models refs=...` shows up to 16 side by side with their sizes. |
| Would this model fit here? Where does it stand? | `scene.query action=fit ref=<model> position=<x,y,z> rotation=<...>` - `fits`, what blocks it and how deep, the ground under it, `gap`, and `rest`: the position at which it stands on the ground (on the highest point under its box). `except=<names>` leaves objects out - the object itself, when it is already written. |
| Where near here is there room for it? | `scene.query action=free ref=<model> position=<x,y,z> radius=<m> count=<n>` - spots where it fits, standing on the ground, nearest first. |
| Can a creature walk here, and get from A to B? | `scene.query action=ai positions=[...]`, or `from=` + `to=` (`reachable`, `length`); `shot.map mode=ai` shows the nodes. |
| What does it really look like? | `shot.viewport view=<top, front, back, left, right, iso> objects=<names>` (or `area=`), or `position=` + `target=`: a lit picture from a camera of its own, the author's camera untouched; `labels=true` writes the names in and answers their pixels; a flat view answers its `camera.px_per_m`. |

Positions are metres, rotations degrees, `[x, y, z]`; +Y is up. On a map +X is right and +Z is up.

## 2. What to write: the scene's files

A scene is `<name>.scene` and a folder of Lua data files beside it, one per kind (`objects.lua`, `spawns.lua`,
`groups.lua`, `terrain.lua`, ...; `docs/assets/SCENE.md` is the reference of every key, `asset.reference kind=scene`
prints it). An X-Ray SDK `.level` opens as an import: `scene.save` writes its `.scene` and folder, then work on
those. One object is one entry of a list - one line is fine:

```lua
-- <scene>\objects.lua, inside objects = { ... }
{ name = "yard_barrel", ref = "statics\\barrel\\bochka_wood1", pos = { -102.5, -12.7, -162.5 }, authored = true },
```

- **Names, not numbers.** A name is unique in the scene; groups, sectors, ways and quests call an object by it.
- **`ref`** is a library object (`statics\\barrel\\bochka_wood1`) or a model asset of the project by its path
  (`models/yard/hut.meshasset`, section 4).
- **`authored = true`** on a level of the game: the object is the module's and ships. Without it the object is
  taken for the game's own and does not ship - the audit says so (`S210`).
- The editor follows the files: an open scene re-reads a file the moment it is written, and the change is one undo
  step. A file that does not parse **changes nothing** - the scene keeps what it had, the problem (`syntax`) has the
  file, line and column, `scene.save` leaves that file as it is and answers it under `skipped`, and the scene comes
  back when the file is fixed. `asset.check path=<scene>.scene` (or `Grizzly-Cmd.exe check`) lists the problems of
  the scene and its files.
- **A prefab is one line.** A group asset (`*.group`: `group = 1` and the scene's lists, positions relative to its
  pivot) placed as `{ name = "post_a", ref = "prefabs/guard_post.group", pos = { 120, 0, -40 }, rot = { 0, 90, 0 } }`
  in `groups.lua` - written without `members` - is placed when the scene is read: its members are made at the
  group's place, named `post_a_<member>`, with a note (`S023`) at the line; the scene is unsaved until `scene.save`
  writes the members.
- `scene.save` writes only the lines that changed; a comment or a line nobody changed stays as it was.

## 3. Onto the ground

A written `pos` is where the model's pivot goes, not where it stands. Ask first and write the right number -
`scene.query action=fit` answers `rest` - or settle after writing:

- `scene.align action=drop objects=<names>` - each drops onto what is under its whole box (`line=true`: under its
  pivot); `action=surface` also tilts it to the ground; `offset=<m>` leaves it that far above. The answer names
  where each landed and `landed` counts them.
- `scene.save` writes the settled positions back - only those lines.

## 4. Models, textures and sounds

A media asset is a text file beside a source of a common format; everything refers to the asset, the module ships
what the game needs (`docs/assets/MODEL.md`, `TEXTURE.md`, `SOUND.md` list every key; `asset.reference
kind=model|texture|sound`).

| Kind | File | Source | The module ships |
|---|---|---|---|
| Model | `*.meshasset` (`mesh = 1`) | `.glb`, `.gltf`, `.obj`, `.object`, or `parts` written in it | static: nothing of its own - it is compiled into the level (the piece); dynamic: `meshes\<name>.ogf` and the asset |
| Texture | `*.texasset` (`tex = 1`) | `.png`, `.jpg`, `.tga`, `.bmp`, `.dds` | `textures\<name>.dds` and the asset |
| Sound | `*.sndasset` (`snd = 1`) | `.wav`, `.ogg` | `sounds\<name>.ogg` and the asset |

- **Bringing a model in**: `asset.import from=["D:/art/crate.gltf","D:/art/ramp.obj"] to=models/yard` copies each
  source (a `.gltf` with its buffers and pictures, an `.obj` with its `.mtl`) and writes the asset beside it, and a
  texture asset for every picture. glTF and OBJ are right-handed: Z is negated as they are read (an OBJ ramp built
  along +Z lies along -Z in the game).
- **A blockout** is a model asset with `parts` instead of a source - boxes, ramps, cylinders, planes and prisms in
  metres, each on a surface of `surfaces`, its texture repeated every `tile` metres:

  ```lua
  return {
    mesh = 1,
    surfaces = { { name = "plank", texture = "wood\\wood_table_01", material = "materials\\wooden_board" } },
    parts = {
      { shape = "box", size = { 6, 0.2, 4 }, surface = "plank", tile = 2 },                         -- the floor
      { shape = "box", size = { 6, 2.6, 0.2 }, pos = { 0, 0.2, 1.9 }, surface = "plank", tile = 2 },  -- a wall
      { shape = "prism", outline = { { -3.2, -2.2 }, { 3.2, -2.2 }, { 3.2, 2.2 }, { -3.2, 2.2 } }, height = 0.15,
        pos = { 0, 2.8, 0 }, surface = "plank" },                                                     -- the roof
    },
  }
  ```

- **Using it**: `ref = "models/yard/hut.meshasset"` in `objects.lua` (or `scene.place kind=object ref=<path>`);
  `model.info refs=models/yard/hut.meshasset` answers its box and surfaces; `preview.model path=<asset>` shows it.
- Give every surface a texture and a game material the game has: `asset.check` refuses a surface without a texture
  (`M112`) or with a game material the game lacks (`M107`; its own are `default`, `materials\wood`, ...), as the
  level build would, and warns of a texture nobody has (`M104`). A surface that names no material gets `default`.
- **A texture**: `source`, the game `name` (left out: `<module id>\<file name>`), and what a `.thm` says as named
  keys (`use`, `format`, `bump`, `detail`, `material`). A surface names it by that `name`.
- **A sound**: `source`, `name`, `min_distance`, `max_distance`, `volume`, `ai_distance`, `type`. A sound source of
  the scene names it by that `name`.
- `file.create kind=model|texture|sound path=<folder>` makes an asset from its template; a texture or sound asset
  without its source is an error, and the module build refuses it until the source is there.

## 5. Terrain: changed ground

A terrain is an object of `terrain.lua` (the list `terrains`): a height grid over a rectangle that replaces the
ground under it.

```lua
return {
  terrains = {
    { name = "yard", pos = { -106, -12.7, -164.5 }, size = { 16, 10 }, cell = 1, base = "ground", edge = 1.5,
      edits = {
        { op = "rect", min = { -5, -3 }, max = { 5, 3 }, falloff = 1.5, height = -12.7 },
      },
      surface = "ground", cut = true, authored = true },
  },
}
```

- `pos` is the centre, `size` metres along its own x and z, `cell` the grid step (0.5 to 4 m), `rot` its heading.
- `base = "ground"`: every grid point starts at the level's ground under it - the surface of the static objects
  that are not `authored` which the terrain's centre stands on (the one nearest `pos`), followed outward from
  there. What stands over the ground - a roof, a tree's crown - is not the ground; a `pos` written at a bridge's
  height puts the terrain on the bridge. Within `edge` metres of the border the edits blend back to the ground, so
  the border meets it. A number instead is flat at that many metres over `pos`.
- `edits` apply in order, x and z in metres from the centre: `flatten` (a circle), `rect`, `raise`, `ramp`,
  `smooth`, `noise`; `height` is a world height, `amount` relative, `falloff` the soft border. Or `rows`: every
  height written out.
- `surface = "ground"` takes the look and the lightmap of the ground under each face: no seam, no bake. A table is a
  surface of its own laid every `tile` metres.
- `cut = true`: the ground it replaces is gone wherever the editor looks - the picture, every ray, the audit, the
  AI map - and in the game: the triangles of the level's static geometry under it (within 0.5 m of the old
  ground) are clipped away; a wall standing on the ground stays.
- `scene.place kind=terrain position=<x,y,z> size=<x,z> cell=<m>` places one; its keys are `scene.set`'s, the edits
  are written in the file.
- **What the build writes** (on a level of the game): the terrain goes into the module's piece (picture) and
  `overlay.xcform` (collision); `levels\<level>\overlay.cut` holds the region and the old ground's heights,
  `overlay.aiedit` the AI map's nodes under it - new heights, closed where the new ground is too steep or something
  `authored` stands. **The game** then cuts its own ground's picture and collision there (trees, bushes and other
  modules' additions stay), moves those nodes (none is added or removed: saves keep their ids), draws the piece, and
  its grass keeps its kinds but stands on whatever collision is there - on the new ground it follows it. A cutting
  terrain needs a game that states `level-cuts` in its `EngineAbilities` (the build refuses it otherwise); a game
  without it ignores both files and loads the level uncut.
- `level.inspect part=cuts|ai_edits|pieces` reads what the build wrote; the game's log says `* [cuts] <level>: ...`
  and `* [ai edits] <level>: ...`.

## 6. Is it right: check before building

- `asset.check` - the files: syntax, unknown models, bad values, with positions.
- `scene.audit` - where things stand, every finding a warning at the object's line of its file:

  | Code | Word | What |
  |---|---|---|
  | `S201` | `floating` | its bottom is more than 0.25 m above whatever is under it |
  | `S202` | `buried` | more than half of its box is under the ground (or under another object's surface) |
  | `S203` | `overlap` | it stands in another object |
  | `S204` | `duplicate` | the same model at the same place twice |
  | `S205` | `unsupported` | nothing under it within 1000 m |
  | `S206` | `off_ai_map` | a creature's spawn point off the AI map |
  | `S207` | `underground` | a spawn point inside geometry |
  | `S208` | `way_off_ai_map` | a way point off the AI map |
  | `S209` | `light_underground` | a light inside geometry |
  | `S210` | `not_authored` | on a level of the game: an object the game's level does not have and not marked `authored` |
  | `S211` | `terrain_border` | a terrain whose border stands more than 0.5 m off the ground around it |

  Left out, it checks the objects marked `authored` (on a level of the game what the module adds), else every
  object; `objects=` or `area=` choose, `checks=` narrows (codes or words), `limit=` caps the answer.
- A picture: `shot.map highlight=<names>` for the plan, `shot.viewport view=iso objects=<names> labels=true` for the
  look.

## 7. Build and play

`level.status` says what is stale and why; `level.build` builds it - on a level of the game the module's piece
(what it adds to the level's picture: its `authored` static objects and terrains, lit under the whole level) and its
layers; `level.inspect` reads what was built; `level.play` starts the game on it. `mod.build` publishes the media
assets with the module (`media` in its answer). `docs/MOD_QUICKSTART.md` has the rest.

## 8. A worked example: a yard on Escape's slope

Every command below was run against the editor on a copy of a project holding Escape (`l01_escape`, a level of
the game); the answers are quoted short.

1. **Look.** `shot.map position=-106,0,-164.5 size=40` - `area [-126, -184.5, -86, -144.5]`, `px_per_m 25.6`, a
   5 m grid; the legend's largest: `escape_terrain` (the ground), the poplars `trees_topol_1_01_0010` at
   `[-108.97, -150.5]` and `_0008` at `[-122.17, -175.95]`, a fir at `[-107.47, -181.48]`: the rectangle x -114..-98,
   z -169.5..-159.5 is clear.
2. `scene.query action=heights area=-114,-169.5,-98,-159.5 step=4` - 5 x 3 samples, `min -14.33`, `max -11.83`: a
   slope of 2.5 m. `scene.query action=ground position=-106,0,-164.5 top=true` - `floor -12.95`, `slope 15.9`,
   `escape_terrain`, `terrain\terrain_escape`, `materials\earth`.
3. **Pick the props.** `model.list query=bochka_wood1 sizes=true` - `statics\barrel\bochka_wood1` `[0.812, 0.976,
   0.812]`; `model.info refs=statics\shkaf\cabinet_village_140x210_ph` - `size [1.407, 2.1, 0.566]`, two surfaces on
   `materials\wooden_board`.
4. **A blockout hut**: `models/yard/hut.meshasset` as in section 4 (floor, three walls, a roof). `asset.check
   path=models/yard/hut.meshasset` - `total 0`; `model.info refs=models/yard/hut.meshasset` - `size [6.4, 2.95,
   4.4]`, `triangles 60`, the surface `plank` on `wood\wood_table_01`.
5. **The yard.** `scene.place kind=terrain name=yard position=-106,-12.7,-164.5 size=16,10`, `scene.save` - the
   line `{ name = "yard", pos = { -106, -12.7, -164.5 }, authored = true, size = { 16, 10 } }` in `terrain.lua`; then
   the line written as in section 5 (the `rect` at -12.7, `edge = 1.5`, `cut = true`). The same heights again: the
   middle samples are `-12.7`, the border ones the old ground (`-12.5957`, `-13.056`).
6. **The hut and two props** written into `objects.lua`, first with rough heights - `scene.audit
   area=-114,-169.5,-98,-159.5` answers `S201 yard_barrel floats 2.70 m above yard` and `S203 yard_hut stands 0.25 m
   inside yard_cabinet`, each at its line (`objects.lua` 24366, 24367). `scene.query action=fit
   ref=statics\barrel\bochka_wood1 position=-102.5,-10,-162.5 except=yard_barrel` - `fits true`, `rest [-102.5,
   -12.7, -162.5]`; for the cabinet moved to x -101.5, `rest [-101.5, -12.7, -166.6]`. The lines then say:

   ```lua
   { name = "yard_hut", ref = "models/yard/hut.meshasset", pos = { -107, -12.7, -164.5 }, authored = true },
   { name = "yard_barrel", ref = "statics\\barrel\\bochka_wood1", pos = { -102.5, -12.7, -162.5 }, authored = true },
   { name = "yard_cabinet", ref = "statics\\shkaf\\cabinet_village_140x210_ph", pos = { -101.5, -12.7, -166.6 }, rot = { 0, 180, 0 }, authored = true },
   ```

7. **Check.** `scene.audit area=-114,-169.5,-98,-159.5` - `{"problems": [], "total": 0, "checked": 4}`. `shot.map
   position=-106,0,-164.5 size=24 highlight=yard_hut,yard_barrel,yard_cabinet` - the three first in the legend in the
   accent colour `#ff40a0`, `yard_hut` centred at `[-107.0, -164.5]`. `shot.viewport view=iso
   objects=yard_hut,yard_barrel,yard_cabinet labels=true` - 960 x 540, each label with its pixel.
8. **Build.** `level.status` - `spawn_layer` and `piece` never built, the level's own steps `not_needed`.
   `level.build` - Spawn Layer, then Level Piece in 11.7 s writing `pieces/main/*`, `overlay.xcform`, `overlay.cut`,
   `overlay.aiedit`; the report's `cuts = { regions = 1, references = 6, ai_edits = 345, closed = 72 }`.
   `level.inspect part=cuts` - one region `kind ground`, `centre [-106, -12.905, -164.5]`, `half [8, 1.968, 5]`,
   `tolerance 0.5`, `grid [17, 11]`; `part=ai_edits` - `total 345`, `moved 345`, `closed 72`; `part=pieces` - `main`,
   `cut true`, `stream false`, 7 visuals, no problem.

## 9. In short

The text `help about=level-design` prints:

<!-- level-design topic -->
```text
Building a place in a level from text files: look, write, settle, check, build, play.
docs/LEVEL_DESIGN.md is the whole guide. Metres, degrees, [x, y, z]; +Y up; on a map +X right, +Z up.

Look - never write a coordinate from memory:
  shot.map position=<x,0,z> size=<m> [mode=objects|height|ai] [highlight=...]
                                   a map from above; legend, px_per_m and map turn pixels into metres
  scene.query action=heights area=<x0,z0,x1,z1> step=<m>      the ground's heights (257 x 257 at most)
  scene.query action=ground positions=[[x,y,z],...] top=true  floor, slope, object, surface
  scene.query action=ray from=... to=... [all=true]           what is in the way
  scene.query action=box min=... max=...  |  scene.objects near=... radius=... bounds=true  |  scene.get objects=...
  scene.query action=fit ref=<model> position=...            fits, blocked_by, rest (where it stands)
  scene.query action=free ref=<model> position=... radius=...  spots where it fits
  scene.query action=ai positions=... | from=... to=...        the AI map
  model.info refs=...  model.list query=... sizes=true  shot.models refs=...  preview.model path=...
  shot.viewport view=top|iso|... objects=... labels=true | position=... target=...   a lit picture

Write - the scene is <name>.scene and a folder of Lua files; the editor follows every write:
  objects.lua   { name = "crate_01", ref = "<library ref or models/x.meshasset>", pos = {...}, authored = true }
  groups.lua    a group with ref = "<x.group>" and no members is a prefab: placed when read
  terrain.lua   { name, pos, size, cell, base = "ground", edits = {...}, surface = "ground", cut = true }
                (scene.place kind=terrain places one; the edits go in the file)
  models        *.meshasset (a source or parts), *.texasset, *.sndasset; asset.import from=... to=...
                brings .glb/.gltf/.obj/pictures/sounds in; file.create kind=model|texture|sound
  A file that does not parse changes nothing: the problem has its line, scene.save answers it skipped.

Settle:  scene.align action=drop|surface objects=... [offset=<m>], then scene.save (only changed lines).
Check:   asset.check   scene.audit [objects=...|area=...] [checks=...]   (S201 floating ... S211 terrain_border)
Build:   level.status   level.build   level.inspect part=pieces|cuts|ai_edits   mod.build (answers media)   mod.info
Play:    level.play
```
<!-- /level-design topic -->

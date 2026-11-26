# Level design by an agent

How a place in a level is built by editing text files: what to ask the editor, what to write, how to know it is
right. Everything here works over MCP, over the socket and from a shell (`Grizzly-Cmd.exe run <command> ...`,
no window) - only the pictures the graphics card draws (`shot.viewport`, `shot.models`) need a window, and
`shot.scene`, `asset.preview` and `asset.thumbnail` draw the same things on the processor without one.
`help about=level-design` prints the short form (section 9, the same text). `docs/GAME_LIBRARY.md` is the
contract of what the editor knows of the game.

A scene cannot be seen by whoever writes it as text. So the work is a loop: **look** (numbers and pictures),
**write** (the scene's files and assets), **settle** (onto the ground), **check** (the audit, a picture), and only
then **build** and **play**. Never write coordinates from memory: ask where the ground is.

## 0. A level of the game: read it, then lie over it

The editor keeps everything of the linked game described as text (`game.library` answers the folder): every
model with its exact size, collision and bones, every texture, sound, particle effect, spawn section, material,
and every level as a scene with a passport. Read before placing:

| Question | Ask |
|---|---|
| What levels, models, sounds, effects, sections are there? | `asset.find kind=level \| model \| texture \| sound \| particle \| section \| material \| lightanim \| sound_env query=<words>` - `size=[min,max]` (the longest side, metres), `collides=`, `class=` (a section's kind: `npc`, `item`, `anomaly`, ...), paged. |
| What exactly is this one? | `asset.card ref=<name> [kind=]` - for a model its box, pivot, surfaces, collision share, floors, bones with their shapes and mass; for a level its passport. |
| What is this level like? | `asset.card ref=<level> kind=level`: the ground, every structure with its box and its `floors` (height, rectangle, clear height `ceiling` or `sky`, `ai_nodes`, `walkable`), the materials, the entities by section. A building's storeys are read here, not guessed from a picture. |
| What does it look like? | `asset.preview ref=<model>` (a sheet of its sides with a scale bar), `asset.thumbnail refs=[...]` (many models at once, to choose from), `asset.preview ref=<level> kind=level`. |
| What does a config or a script of the game say? | `game.search text=<words>`, `game.read path=<file>`. |

A module that adds to a level of the game writes a scene **over** it - two lines:

```lua
-- rawdata/levels/agroprom_base.scene
return {
  scene = 2,
  base = "l03_agroprom",
}
```

The level's own objects (its geometry with its collision as `level_geometry`, every spawn point of the game by
its name, its paths, lights, sounds, effects, its AI map) then stand in the scene, read from the game; the
scene's files hold only what the module adds. `scene.objects own=true` lists the module's part; an object of the
level answers `level = true` and cannot be changed. To take a spawn point of the level out of the game (an
anomaly in the yard), `scene.delete objects=<name>` or one line of `spawns.lua`:
`{ name = "agr_zone_mine_electric_strong_0002", removed = true }`.

What a module places over a level:

- **static objects** by `ref`: a library object, a model asset of the project, or any compiled model of the
  game (`ref = "dynamics\\box\\box_wood_01.ogf"`); the build compiles them into the module's piece of the level.
  A model of the game in another dress is a model asset over it (`source = "game:dynamics/box/box_wood_01.ogf"`
  with the `surfaces` that change, section 4): static it is compiled like any model, `kind = "dynamic"` it is
  a physics object the game spawns.
- **spawn points** by `section`: any section of the game (`asset.find kind=section`), the module's own
  characters (`section = "<module>_<id>"` of `characters/<id>.npcasset`) with an `order`
  (`order = { kind = "guard", look = "<way>" }`), and the module's own items and whatever else it makes a
  section of (`section = "<module>_<id>"` of `items/<id>.itemasset`; `game.lookup type=spawn_section` lists them
  with the game's). No quest is needed to put a person or a thing somewhere.
- **zones** with one `shape`: `shape = { shapes = { { kind = "box", size = { 20, 6, 12 } } } }`.
- **lights**: `lightmap = true` bakes one into the module's piece (a point light, a spot light inside its
  `cone`, a directional one), `dynamic = true` makes a point or a spot light shine in the game.
- **glows** (`glows.lua`: a `texture` and a `radius` at a place - the halo of a lamp), **fog volumes**
  (`fog.lua`: an emitter's box by `pos` and `scale`, the `occlusion` boxes of its `group`), **sound
  environments** (`environments` of `sounds.lua`: a box with `inner` and `outer`, `asset.find kind=sound_env`),
  **sound sources**, **particle effects** and **environment modifiers** (`point = "env_mod"`).
- **wallmarks** are painted, not written: `scene.wallmarks action=set texture=<wm\\...>` then `action=add
  start=<x,y,z> dir=<x,y,z>` lays one where the ray meets the level; they ship as faces of the module's piece.

Lights that shine, sounds, effects, environment modifiers, glows, fog volumes and sound environments the build
writes into the module's `levels\<level>\overlay.things`, and the game puts them beside the level's own. What a
module cannot add to a level of the game says so in the check: sectors, portals and the level's bounds (`S222`),
grass painted as details and an AI map painted over the level's (`S223`).

## 1. What is where: look before writing

| Question | Ask |
|---|---|
| What does this place look like from above, and where is everything? | `shot.map position=<x,0,z> size=<m>` - every object in a colour of its own with its name, a grid in metres with coordinates in the margins, spawn points, lights, ways and volumes as signs (`signs=false` leaves them out). `below=<height>` starts the view under that height: what is under a roof or under the trees' crowns. `legend` lists the objects with their colour, middle and area, the largest first (100 rows, `legend=<n>` for more; `legend_total` counts them all); a pixel reads back to metres through `px_per_m` and `map` (`x = area.x0 + (px - map.left) / px_per_m`, `z = area.z1 - (py - map.top) / px_per_m`). |
| How high is the ground, and how does it slope? | `shot.map mode=height` (colours, contour lines, the heights written where grid lines cross), or `scene.query action=heights area=<x0,z0,x1,z1> step=<m>` for the numbers: rows from `z0` up, each from `x0`, `null` where nothing is; at most 257 x 257 samples (a bigger request is refused naming a step that fits). |
| What is under these points? | `scene.query action=ground positions=[[x,y,z],...]` - the floor under each point and the ceiling over it, the floor's normal and slope, the object and its surface (a collision face of a level answers its game material). It reads what stops a body, from the point down: a point inside a room answers that room's floor. `top=true` starts above everything instead (a roof, open ground). Up to 1024 points a call. |
| What is in this box? | `scene.query action=box min=<x,y,z> max=<x,y,z>` (or `position=` + `size=`; `exact=true` tests triangles); `scene.objects near=<x,y,z> radius=<m> sort=distance bounds=true` or `box=` for the objects' own boxes. |
| Is there a clear line between two points? | `scene.query action=ray from=<x,y,z> to=<x,y,z>` - the first thing in the way, with its surface; `all=true` for every one, nearest first. What stands in the level is in the way - objects, terrains, the bodies of spawn points; a volume (a zone's shape, a sound environment, a fog volume) is not, and is met only with `volumes=true`, each under its `kind`. |
| How big is this model, what are its surfaces? | `model.info refs=<ref,...>` - the box in model space, the size, triangles, every surface with its texture, shaders and game material. `model.list query=<text> sizes=true` finds models; `shot.models refs=...` shows up to 16 side by side with their sizes. |
| Would this model fit here? Where does it stand? | `scene.query action=fit ref=<model> position=<x,y,z> rotation=<...>` - `fits`, what blocks it and how deep, the ground under it, `gap`, and `rest`: the position at which it stands on the ground (on the highest point under its box). `except=<names>` leaves objects out - the object itself, when it is already written. |
| Where near here is there room for it? | `scene.query action=free ref=<model> position=<x,y,z> radius=<m> count=<n>` - spots where it fits, standing on the floor of the point's height (the storey the point is on; `top=true`: the highest floor), nearest first. The answer's `below` is the height floors were looked for under - 1.5 m over the point: ground that rises past it within the radius (a slope) is not offered, so ask again from a point further up. `size=[x,y,z]` instead of `ref` asks for a box - a person is `[0.7, 1.8, 0.7]`. |
| Can a creature walk here, and get from A to B? | `scene.query action=ai positions=[...]`, or `from=` + `to=` (`reachable`, `length`); `shot.map mode=ai` shows the nodes. |
| What does it really look like? | `shot.scene view=<top, front, back, left, right, iso> objects=<names>` (or `area=`), or `position=` + `target=`: the scene drawn on the processor, no window - what the scene holds of its own named where it is seen, `legend` with each one's pixel and whether it is seen, `camera` for reading a pixel back. `cut=<height>` leaves out what is wholly above it: `view=top area=<x0,z0,x1,z1> cut=<a storey's ceiling>` is that storey's plan. With a window, `shot.viewport` takes the same camera and answers the lit, textured picture (`heading=<degrees>` turns its `iso` view; `labels=true`). |

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
- **`ref`** is a library object (`statics\\barrel\\bochka_wood1`), a compiled model of the game
  (`dynamics\\box\\box_wood_01.ogf`) or a model asset of the project by its path (`models/yard/hut.meshasset`,
  section 4).
- **Whose it is.** In a scene over a level (`base`, section 0) everything the files hold is the module's and
  ships; nothing is marked. In a scene that holds a whole level imported from an X-Ray SDK scene, the module's
  objects say `authored = true` - without it an object is taken for the game's own and does not ship (`S210`).
- The editor follows the files: an open scene re-reads a file the moment it is written, and the change is one undo
  step. A file that does not parse **changes nothing** - the scene keeps what it had, the problem (`syntax`) has the
  file, line and column, `scene.save` leaves that file as it is and answers it under `skipped`, and the scene comes
  back when the file is fixed. What the open scene changed of that file's objects meanwhile is not lost: the save
  names the file under `unwritten`, and when the file reads again the change is put over it, unsaved, for the next
  save. `asset.check path=<scene>.scene` (or `Grizzly-Cmd.exe check`) lists the problems of
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
| Model | `*.meshasset` (`mesh = 1`) | `.glb`, `.gltf`, `.obj`, `.object`, a compiled `.ogf` of the project or of the game (`source = "game:<name>.ogf"`), or `parts` written in it | static: nothing of its own - it is compiled into the level (the piece); dynamic: `meshes\<name>.ogf` and the asset |
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

### What the game makes of it

- **A model of the game placed as a static object** (`ref = "<name>.ogf"`, or a static `.meshasset` over one)
  is compiled into the level's geometry. Its parts name the shader the model was made for (`models\model`);
  the build compiles such a surface with the game's shader for level geometry that treats the texture's alpha
  the same way and says which in the step's `notes` - nothing to write. `asset.find kind=shader` says what
  each shader is for (`draws`: `level`, `model`, `other`; `as_level` for a model's).
- **A model the game draws nothing for** (`invisible` on its card: a stash's case, a collider) is there and
  unseen. A stash is such a case put inside something that is drawn - a crate, a locker; anything else that
  wears one is `S224`.
- **A stash** is a static object that says so: `stash = true` in `objects.lua`, or `stash = { random = false,
  loot = false }` (each key `true` when left out). The object stays what it is - the box one sees, compiled into
  the level or the module's piece -, and the build puts an `inventory_box` over it in the spawn layer, under the
  object's own name, wearing the model's unseen case `<ref>_case` (`dynamics\equipment_cache\equipment_box_02`
  has `..._02_case`; a model without one opens by the box's own case, 0.94 x 0.43 x 0.48 m: `S218`). `random =
  false`: the game never fills it with a random stash's loot; `loot = false`: no NPC who searches stashes comes
  to it - the build writes such a key into the box's custom data as `[xms_stash]`. A quest reaches the stash by
  the object's name: `{ name = "my_stash" }` (`item.spawn where = { into = { name = "my_stash" } }`,
  `objective.fetch spawn = { into = { name = "my_stash" } }`), which the game looks up as the module's object
  first. So the name is the stash's handle: it takes `a-z A-Z 0-9 _ - .` only, and no other object of the
  layer may be written under it (`S226`, an error the build refuses); a stash is not scaled (`S219`).
- **A glow** is a picture that faces the viewer: `texture`, `radius` in metres, or `fixed_size = true` and a
  `radius` that is a share of half the screen's height. It is drawn like a lens flare; `shader =
  "effects\\sun"` hides it by depth as well. No other shader draws one (`S225`).
- **A fog volume** is a simulated mist in a box. The game's own profiles (`environment/fog/*.ltx`) are made for
  its undergrounds - dense, and lit in full: in the open such a volume is a white lump. A mist under the sky
  takes a profile of the module's own (`gamedata\configs\environment\fog\<name>.ltx`, named from `configs`)
  with a low `Density` (0.1) and `Hemi` (0.2); a file that is no profile is `S220` with what it lacks.
- **Taking a spawn point of the level out** (`{ name, removed = true }`) changes the game's own world for the
  module's mode: a new game with the module has none of them, and a save made in it keeps them out for a game
  without the module too. The build says how many it takes out.
- **Seeing it in the game**: the game's own QA starts a built module on a hidden desktop and takes pictures at
  the places asked (its `tools/qa/module`); a model that is not drawn, a light that does not reach, a mist that
  is a lump are seen there and nowhere else.

## 6. Is it right: check before building

- `asset.check` - the files: syntax, unknown models, bad values, with positions. A sound, a particle effect, a
  light animation, a sound environment, a texture, a compiled model or a fog profile that neither the game nor
  the project has is `S220` at its key with the nearest names; a directional light asked to shine in the game
  is `S221` (the game's one directional light is the level's sun); over a level of the game a sector, a portal
  or a bound of the level is `S222` and painted details or a painted AI map are `S223` (the game takes none of
  them from a module); an object whose model the game draws nothing for is `S224` at its line - the model's
  card says `invisible`, and a section whose own model is such (a stash is an unseen case inside something
  drawn) is not reported; a glow that names a shader no glow is drawn with is `S225` (the game draws a glow
  like a lens flare, or like the sun with `effects\sun`); a stash whose name the spawn layer cannot write as it
  is, or writes another object under, is `S226` (an error: a quest reaches the stash by its name); a spawn
  point whose story id is not named after the module (the module's id a word of it) is `S227` (a story id is
  one name in the whole game: the game stops when a second object carrying it spawns, another module's too);
  the name of a level of the module's own not named after the module is `S229` (a level's name is one
  name for the whole game; its smart terrains and way paths are composed under the module's name); a fog
  profile that is a file and no profile - no `[volume]` that counts its emitters, an emitter without a key the
  game reads - is `S220` with what it lacks; a key that is nearly a key of the kind (`type` for `kind`) is a warning that names the key meant.
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
  | `S212` | `terrain_unmade` | a terrain whose model cannot be made from what its file says |
  | `S213` | `terrain_bare` | a terrain that wears the ground's look (`surface = "ground"`) with no ground under it |
  | `S214` | `radio_playlist` | a radio bound (`radio = { plays = ... }`) to a playlist of the module that `mod.ltx` does not declare |
  | `S215` | `in_geometry` | a wall or a steep slope of the level (or of a terrain) passes through its middle |
  | `S216` | `ghost` | a static object none of whose faces stop a body |
  | `S217` | `on_ghost` | it stands on faces that are drawn and stop nothing: a body falls through |
  | `S218` | `stash_no_case` | a stash whose model has no unseen case (`<ref>_case`): it opens by the inventory box's own |
  | `S219` | `stash_scaled` | a stash that is scaled: its case is not |

  The audit reads what stops a body - a level's collision, the faces of a model that collide - and a spawn
  point is the model its entity wears: a crate, a lamp, a stalker stands, floats and overlaps like any object.

  Left out, it checks the objects marked `authored` and every terrain (on a level of the game what the module
  adds), else every object; `objects=` or `area=` choose, `checks=` narrows (codes or words), `limit=` caps the answer.
- A picture: `shot.map highlight=<names>` for the plan, `shot.viewport view=iso objects=<names> labels=true` for the
  look.

## 7. Build and play

`level.status` says what is stale and why; `level.build` builds it - on a level of the game the module's piece
(what it adds to the level's picture: its `authored` static objects and terrains, lit under the whole level) and its
layers; `level.inspect` reads what was built; `level.play` starts the game on it. `mod.build` publishes the media
assets with the module (`media` in its answer), bakes the open scene's spawn layer and things, and over a level of the
game builds the module's piece first when it is due: placing a static object and building the module is enough. On a
level of the module's own `mod.build` compiles nothing and warns when a step is not built: `level.build` first. `docs/MOD_QUICKSTART.md` has the rest.

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

The game - read before placing (docs/GAME_LIBRARY.md):
  asset.find kind=level|model|texture|sound|particle|section|material query=...   what there is
  asset.card ref=<name> [kind=level]     everything known of one; a level's passport: structures, floors
  asset.preview ref=...   asset.thumbnail refs=[...]   pictures of models without a window
  A scene over a level:  return { scene = 2, base = "<level>" }  - the files hold only what the module adds;
  spawns.lua { name = "<a spawn of the level>", removed = true } takes it out; scene.objects own=true

Look - never write a coordinate from memory:
  shot.map position=<x,0,z> size=<m> [mode=objects|height|ai] [highlight=...]
                                   a map from above; legend, px_per_m and map turn pixels into metres
  scene.query action=heights area=<x0,z0,x1,z1> step=<m>      the ground's heights (257 x 257 at most)
  scene.query action=ground positions=[[x,y,z],...] top=true  floor, slope, object, surface
  scene.query action=ray from=... to=... [all=true] [volumes=true]   what is in the way
  scene.query action=box min=... max=...  |  scene.objects near=... radius=... bounds=true  |  scene.get objects=...
  scene.query action=fit ref=<model> position=...            fits, blocked_by, rest (where it stands)
  scene.query action=free ref=<model> position=... radius=...  spots where it fits
  scene.query action=ai positions=... | from=... to=...        the AI map
  model.info refs=...  model.list query=... sizes=true  shot.models refs=...  preview.model path=...
  shot.scene view=top area=<x0,z0,x1,z1> cut=<height> | position=... target=...     the scene, no window
  shot.viewport view=top|iso|... objects=... labels=true | position=... target=...   a lit picture (a window)

Write - the scene is <name>.scene and a folder of Lua files; the editor follows every write:
  objects.lua   { name = "crate_01", ref = "<library ref, <model>.ogf or models/x.meshasset>", pos = {...} }
  spawns.lua    { name = "guard", section = "<a section, or <module>_<character or item id>>", pos = {...},
                  order = { kind = "guard" } }   zones: shape = { shapes = { { kind = "box", size = {...} } } }
  lights.lua    { name, pos, rot, kind = "point"|"spot"|"directional", cone, range, lightmap, dynamic }
  glows.lua     { name, pos, texture, radius }      fog.lua  { name, pos, scale, kind, group, profile }
  sounds.lua    sources { name, pos, sound }   environments { name, pos, scale, inner, outer }
  wallmarks     scene.wallmarks action=set texture=... then action=add start=... dir=...
  groups.lua    a group with ref = "<x.group>" and no members is a prefab: placed when read
  terrain.lua   { name, pos, size, cell, base = "ground", edits = {...}, surface = "ground", cut = true }
                (scene.place kind=terrain places one; the edits go in the file)
  models        *.meshasset (a source, source = "game:<model>.ogf", or parts), *.texasset, *.sndasset;
                asset.import from=... to=... brings .glb/.gltf/.obj/.ogf/pictures/sounds in;
                file.create kind=model|texture|sound
  A file that does not parse changes nothing: the problem has its line, scene.save answers it skipped
  (and unwritten, when the scene holds changes of its objects: they go over the file once it reads).

Settle:  scene.align action=drop|surface objects=... [offset=<m>], then scene.save (only changed lines).
Check:   asset.check (S220 a name nobody has, S221-S223 what the game does not take, S224 a model nobody sees,
         S225 a glow's shader, S226 a stash's name, S227 a story id and S229 a level of the module's
         own not named after the module)
         scene.audit [objects=...|area=...] [checks=...]   (S201 floating ... S219 stash_scaled)
Build:   level.status   level.build   level.inspect part=pieces|cuts|ai_edits   mod.build (answers media)   mod.info
         level.inspect part=visuals piece=main [shader=<text>]   what the module's piece of the level holds
Play:    level.play
```
<!-- /level-design topic -->

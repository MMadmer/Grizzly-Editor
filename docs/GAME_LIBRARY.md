# The game as the editor's library

The editor ships without the game's content: it reads the game it is linked to. This document is the contract of
how that content becomes the editor's own assets - described in text, kept current, drawn without a window - and
of how a level of the game is worked on. `docs/LEVEL_DESIGN.md` is the guide that uses it; `docs/ASSETS.md`
states the rules every asset keeps.

## 1. Principles

- **A wrapper, never a copy.** For everything of the game a level designer names - a model, a texture, a sound, a
  particle effect, a spawn section, a material, a shader, a light animation, a sound environment, a level - the
  editor keeps a text description: what the thing is called, where the game holds it, and every fact about it
  that can be worked out. The model's triangles, the texture's pixels and the sound's samples stay in the game
  and are read from it when they are needed. Nothing of the game is copied, and the game is never written.
- **The editor's, not a project's.** The library lives in the editor's own data
  (`<appdata>\game_library\<game key>\`), one per linked game. Every project that reads that game shares it; a
  project makes none of it.
- **Always current.** The library is brought up to date at every start of the editor, whenever the game it
  reads is another one, whenever the editor reads the game's index again and whenever a command reads the
  library (section 4): a job compares the game's file index with what the library was made from and describes
  again exactly what changed. A file added to the game is an asset of the library after the next start, with
  nothing done by hand.
- **Facts are worked out, not typed, and they are exact.** A description states what the editor measured in the
  game's own file - a model's box, its collision shapes, the faces a thing can stand on -, never a guess from a
  name.
- **Read as text first.** Every description is Lua data an agent reads with its own file tools and searches with
  `grep`; the commands answer the same values. A picture is the second look, for what the numbers leave in doubt.

## 2. Where it is

```text
<appdata>\game_library\<game key>\
  library.lua                 the state: the game, when it was read, the count of every kind
  state.txt                   what every description was made from (the editor's own bookkeeping)
  models\<engine name>.meshasset      one per compiled model of the game (`meshes\**.ogf`)
  models\index\<top folder>.lua       one row a model: the list an agent searches
  textures\<top folder>.lua           one row a texture
  sounds\<top folder>.lua             one row a sound
  particles.lua               every particle effect and group
  sections.lua                every spawn section: what the game makes of it
  materials.lua               game materials, compiler shaders, engine shaders
  lightanims.lua              light animations
  sound_envs.lua              sound environments
  levels\<level>\<level>.scene        a level of the game as a scene (section 5)
  levels\<level>\<level>\*.lua        its parts: spawns, ways, lights, sounds, particles, ...
  levels\<level>\passport.lua         what the level is like (section 6)
  levels\<level>\stamp.txt            what the level was made from, written when it is done
  thumbs\<kind>\<hash>.png    thumbnails, made when first asked for
```

`<game key>` is a hash of the game's folder as it is written. `game.library` answers the folder and the state;
`AGENTS.md` of a project names the command. Every file starts with a comment saying it is generated and from
which game; an edit is lost at the next sync. A catalog and a model's card carry `card = <version>`: a change
of what a kind states is a new version, and the next sync describes that kind again. A list that is one file
and a level are made again when what they are made from changes - the game's files, or the version of the
editor that describes them; a level says what it was made from in `levels\<level>\stamp.txt`, written last.

## 3. What a description states

Positions are metres in the model's own space, +Y up; angles degrees; a name is the name the game uses.

### 3.1 Models - `models\<engine name>.meshasset`

A model asset (`mesh = 1`, `docs/assets/MODEL.md`) whose source is the game's file: `source = "game:<path>"`. It
is the description `asset.card` answers; a scene or a spawn names the
model by its engine name (`ref = "dynamics\\box\\box_wood_01.ogf"`, `visual = "dynamics\\box\\box_wood_01"`).
Beyond the keys of the kind (`kind`, `surfaces`, `bones`) it holds `facts`:

| Key | What |
|---|---|
| `box`, `size`, `sphere` | the bounding box (`min`, `max`), its size, the bounding sphere |
| `pivot` | where the origin lies in the box: `base` (the middle of the bottom), `centre`, or the offset |
| `form` | `static`, `skinned`, `progressive`, `tree`, `lod`, `hierarchy` as the game stores it |
| `triangles`, `vertices` | of the finest level |
| `surfaces` | per surface: `texture`, `shader`, `triangles`, `area`, `collides`, `material` |
| `collision` | what stops a body: `triangles`, `area`, `share` of the drawn area that collides, `box`; `false` when nothing does |
| `shaders` | per surface in `surfaces`: the engine shader the model's part names. A compiled model names shaders for models (`models\model`); placed as a static object it is level geometry, and the level build compiles such a surface with the game's shader for level geometry that treats the texture's alpha the same way (`models\model` as `default`, a cut one as `def_shaders\def_aref`, a blended one as `def_shaders\def_trans` - told from the classes and the properties of the game's own `shaders.xr`, never from a list of names), saying so in its answer |
| `invisible` | `true` for a model the game draws nothing for (its every surface wears `glow\glow_ghost`): it is there, it stops a body, a box of it holds things, and nobody sees it. Said in the row of `asset.find` too; a scene object that wears one is `S224` |
| `bones` | per bone: `name`, `parent`, the collision `shape` (`box` with its `size`, its `center` and its three `axes`, `sphere` with `radius` and `center`, `cylinder` with `radius`, `height` and `center`, `none`), `mass`, `material`, `joint` |
| `mass`, `physics` | the bodies' mass together; `none`, `rigid` (one body), `jointed` |
| `floors` | flat faces a thing can stand on: `y`, `area`, the rectangle `box = { x0, z0, x1, z1 }`, largest first |
| `parts` | the connected pieces of a large model, each with its `box`, `triangles` and its own `floors` |
| `motions` | the motion files it names and how many motions it carries |
| `lod`, `user_data` | the model its distant look is, the text the game reads from it |
| `textures` | every texture it wears |

### 3.2 Textures - `textures\<top folder>.lua`

One row a texture: `name`, `size = { w, h }`, `format`, `mips`, `alpha`, `cube`, `colour` (the average, for a
flat look), and what its description says in the game's words: `type`, `material`, `bump`, `detail`,
`detail_scale`, `fade`.

### 3.3 Sounds - `sounds\<top folder>.lua`

One row a sound: `name`, `length` (seconds), `channels`, `rate`, and the parameters the game plays it with:
`min_distance`, `max_distance`, `volume`, `ai_distance`, `type`.

### 3.4 Particle effects - `particles.lua`

Effects (`name`, `max_particles`, `texture`, `shader`, `time_limit`, `flags` as words, the size and colour a
particle starts with) and groups (`name`, `time_limit`, their effects with the times they play).

### 3.5 Spawn sections - `sections.lua`

One row a section a spawn point may name (it says `$spawn`): `name`, `group` (its place in the Spawn list),
`class`, `kind` - what it spawns, in a word: `weapon`, `item`, `npc`, `monster`, `anomaly`, `restrictor`, `zone`,
else the class's own word (`hanging_lamp`, `object_physic`, `smart_terrain`, ...) -, `visual` (the model it
wears; `asset.find kind=section` answers that model's `size` beside it), `title` (the id of the name the player
reads), `shape = true` when it carries a volume, `walks = true` when it walks the AI map. A character of the
open module is a section too: `<module>_<id>` (section 7); `game.lookup type=spawn_section` lists both.

### 3.6 Materials and shaders - `materials.lua`

`materials`: every game material with its flags as words (`climable` is a ladder, `passable`, `breakable`,
`liquid`, ...) and factors; `compilers`: every compiler shader with `collides`, `draws`, `lightmap`; `shaders`:
every engine shader with what it is for - its `class`, the geometry the class `draws` (`level`: a level's static
geometry; `model`: an object with bones or placed by a matrix; `other`: effects, details, trees, particles),
`alpha` when it cuts (`test`) or blends (`blend`) by the texture's alpha, and for a shader of models `as_level`,
the shader a level's geometry takes in its place when such a model is compiled as a static object.

### 3.7 Light animations, sound environments

`lightanims.lua`: `name`, `fps`, `frames`; `sound_envs.lua`: `name` and what the reverb is.

## 4. Keeping it current

- **The sync is a job of the editor** (`gz-gamelib`). A run that stays up (a window, `-headless`) looks at the
  library by itself: after the first frame of every start, when another game takes the place of the one there
  was (a link, a project that reads a game of its own) and when the game's index is read again (a build that
  installed a module, a refresh of the game's files). A command that reads the library (`asset.find`,
  `asset.card`, a picture) looks first too, in a commandlet as well; a library found current is not looked at
  again for 30 seconds while the index stays the same. A look that finds the library behind starts the sync
  and the log says what differed. Nothing watches the game's folder: a file changed there while the editor
  runs is seen at the next of those looks - a new file when the index is read again. Closing a project does
  not stop the sync; Stop does, and the next sync goes on where it stopped (the state is saved as it goes).
- It reads the game's index (`gz-vfs`), finds per kind what is new, changed (a file's size and place in its
  archive, a loose file's size and time) and gone, and describes only that, on every core. A start with nothing
  changed reads the state and compares: no file of the game is opened.
- **Everything the library holds is looked at, not the files alone**: the models, textures and sounds file by
  file, every list against what it is made from (`particles.xr`, the material and shader libraries, the light
  animations, the sound environments, every config for the spawn sections) and every level against its files
  and the game's spawn. A config that changed makes the sections again; a level that changed is that level
  again.
- **A sync that is cut off loses nothing it finished and leaves nothing half said.** While a kind is described
  its catalogs are written before the state that says they are (every 20 seconds), and a catalog the state
  has rows of and the library no file is written again by the next sync. A level says it is done by itself
  (`stamp.txt`, written after its scene and its passport), so the levels that were not reached are made by
  the next sync; a level that cannot be made says why in the same file and is not tried again until its files
  change. Levels are made on every core at once; a level not begun when Stop comes is left for the next
  sync.
- A description that cannot be made (a file that does not read) is a row with `problem = "<why>"`, never a
  missing row, and the job's answer counts them.
- `game.library` answers the state without waiting (`ready`, `syncing` with the stage, the counts, the
  problems); `action=sync` starts a sync and waits for it; `action=sync rebuild=true` describes everything again.

## 5. A level of the game is a scene, and a module's scene lies over it

- **Every level of the game is a scene of the library** (`levels\<level>\<level>.scene`), made from the files the
  game loads: the geometry it draws and the collision it stops bodies with (the object `level_geometry`, whose
  model `@level\<level>` is read from the game when a scene needs it), every entity of the game's spawn that
  stands on the level with its own name, the patrol paths, the lights, the glows, the static sounds and
  particle effects, the environment modifiers, the fog volumes (`level.fog_vol`: each emitter with the volumes
  it flows around) and the sound environments (`level.snd_env`: the boxes the file was made of, each with the
  environment inside it and the one above it). It is the level as the game has it - not a source it was once
  built from.
- **A scene of a module on such a level names it**: `base = "<level>"` in the `.scene` (the scene format's
  version 2, `docs/assets/SCENE.md`). The scene's files then hold only what the module adds (`objects.lua`,
  `spawns.lua`, ...). What it takes away is an entry of `spawns.lua` that says the name of a spawn point of the
  level and `removed = true`, and nothing else. The level itself is made from the game's files as the scene is
  read (and kept until one of them changes), with the level's AI map (`level.ai`) as the scene's AI map: it is
  always the game's current one, and the module's files stay a few lines an agent reads whole.
- Everything in the files of a scene with a base is the module's: it ships, and the key `authored` is not
  written. What comes from the level cannot be moved, changed or deleted - a module cannot change the game's
  level that way: `scene.delete` of a spawn point of the level takes it out (`removed`), and of anything else is
  refused with the reason. The ground is changed with a terrain (`cut`): the ground it replaces is gone from
  the level's picture and from its collision alike, wherever the editor looks. A copy of one of the level's
  objects - Duplicate, Copy and Paste, a group asset saved from them - is an object of the module's own,
  written whole; the level's own stay out of a group, are not moved by an object tool and take no motion.
  The lines that take out a spawn point the scene holds no object for (the level is not at hand, or it has
  none of that name) are the scene's to keep: Save As and a recovery snapshot carry them.
- `scene.new base=<level>` starts such a scene; `scene.info` answers `base { level, at_hand, level_objects,
  own_objects, removed }`, `scene.objects own=true` lists what the scene holds of its own, and an object of the
  level answers `level = true`.
- **By hand it starts in the Content Browser**: Library > Base Scenes lists the game's levels under `game`
  beside the library's own scenes. Opening one opens the scene of the project that says `base = "<that
  level>"`; when the project has none, a new scene over the level is made and saved as
  `rawdata\levels\<level>.scene` (the first free name). A level is not copied into a project - Bring into
  Project says so on such a tile -, and `content.view` answers `level` for it.
- A scene without `base` is a level of the module's own, as before. An X-Ray SDK `.level` still opens as an
  import.

## 6. The passport of a level - `levels\<level>\passport.lua`

What the level is like, worked out from the level's own collision, so that a place can be understood before it is
looked at (`asset.card ref=<level> kind=level`):

| Key | What |
|---|---|
| `box`, `size` | the level's bounds |
| `ground` | the flat regions that are the ground: `y`, `area`, the rectangle `box = { x0, z0, x1, z1 }`, `slope` (lowest and highest), `ai_nodes`, `walkable`, `sky` |
| `structures` | every connected piece of the collision that stands on the ground and is larger than a prop: `id`, `kind` (`building`, `platform`, `wall`, `other`), `box`, `size`, `footprint` (m2), `triangles`, its `materials`, and its `floors` - each with `y`, `area`, its rectangle, `ai_nodes`, `walkable`, and `ceiling` (the clear height to what is over it) or `sky = true` |
| `also` | what the structures leave out, counted: vegetation, slopes of the ground, small pieces, walls nobody sees |
| `materials`, `collision` | the collision by game material (m2), and its size |
| `top` | a grid of the highest solid thing over every cell: what stands where, from above |
| `ai` | the AI map: node count, the box it covers |
| `counts`, `spawns`, `classes` | how many of each kind stand on it (fog volumes and sound environments among them); the entities by section, the few of a section with their places; the entities by class |
| `details` | what grows on the level by itself (`level.details`): how many plants there are, the pictures they wear, the grid of 2 m slots and how many of them hold a plant (`covered`, `covered_m2`) |

## 7. Placing things: no kind is second class

- **A compiled model is placeable like a library object**: `ref = "<engine name>.ogf"` in `objects.lua` is a
  static object (compiled into the module's piece of the level with its collision), whether the model is the
  game's or the project's own `gamedata\meshes`. The piece is where it ships: the spawn layer leaves a static
  object out where the game loads pieces, so it stands in the game once. A name is a name under `meshes`: one with a drive, a root or a
  step up is no model, so nothing outside the project and the game is read. The piece is lit under the whole
  level, and the level wears textures only the game has: the level compiler reads a texture the editor's own
  data lacks from the linked game - its description and its picture, or the picture alone, which says its size
  and whether it lets light through.
- **A character of the module is a spawn section**: `section = "<module>_<id>"` in `spawns.lua` places it, with
  its Order; the editor knows the section from the project's `.npcasset` (`gz_game::OwnSections`: what the game
  makes of a module's assets is told to the editor by whoever owns the assets), the game makes it.
- **So is everything else the module makes a section of.** The scene reads the game's configs as a game with
  the module in it reads them (`gz_game::ComposedConfigs`: the tree the items area composes - the game's own
  with the module's configs, patches and item assets laid over it): an item of the module (`section =
  "<module>_<id>"`), a section a config of the module brings, a section a patch changes are spawn sections like
  any of the game's - listed by `game.lookup type=spawn_section`, placed, checked, baked. A module that makes
  no section reads the game's own tree, and nothing is composed for it.
- **A model of the game dressed by the module is a model asset** (`docs/assets/MODEL.md`, "Over a compiled
  model"): `source = "game:<name under meshes>.ogf"`, or an `.ogf` of the project, with the rows of `surfaces`
  that change what its parts wear. Static, it is compiled into the level or the piece like any model; dynamic,
  it is published under the module's own name as the same file with that said again - its skeleton, its
  motions and its levels of detail stay its own -, and a scene object of it is a physics object of the spawn
  layer. The game's file is read, never written, and never shipped as it is.
- **What a spawn point's entity wears stands there**: a crate, a lamp, a stalker is its model in every ray, box,
  fit, audit and picture - not a marker.
- **A zone has one shape**: `shape = { shapes = { { kind = "box", size = { 20, 6, 12 } } } }` of the spawn point
  is the volume; `props` does not repeat it, and the entity carries the same.
- **What a level of the game cannot be given through the level's own files reaches it as a layer of the
  module** (the game's `XMS_ARCHITECTURE.md` 8.5; a game says `level-things`): the sound sources, the particle
  effects, the lights marked `dynamic` (a point light or a spot light with its cone), the environment
  modifiers, the glows, the fog volumes (an emitter's box with its profile and the boxes it flows around) and
  the sound environments (a box with the environment inside it and the one above it) the module adds to a
  level of the game - a scene with a base, or a scene of the level's own name whose objects are marked the
  module's - are written by the level build's spawn layer step into the module's
  `levels\<level>\overlay.things` (text) and put by the game beside the level's own. A game that does not
  state the ability is a warning of the build.
- **A level of the module's own writes the same file for what its own files cannot carry**: its glows (the
  game draws none of a level's own file), its sound environments (the level compiler makes no file of them)
  and its spot lights that shine in the game (the game loads every light of a level's own file as a point
  light). Everything else of such a level is in the level's files, as before.
- **Wallmarks painted over a level of the game are part of the module's piece**: the marks of a scene with a
  base are the module's own, compiled into the piece as faces of their shader beside the module's static
  objects; a scene that only paints marks has a piece too. `scene.wallmarks action=set texture=` spells the
  name the game's way and refuses a texture nobody has.
- **A light is baked whatever its kind**: a point light, a spot light inside its cone (all of it on the axis,
  falling to nothing at the cone's edge), a directional one from its direction - on the processor and on the
  card alike (`docs/ARCHITECTURE.md`, the baker). In the game a level's own file carries point lights; a spot
  light shines from `overlay.things`; a directional one never does, because the game has one directional
  light, the level's sun (`S221`).
- **What a module cannot add to a level of the game is said, not dropped in silence**: its sectors and portals
  (`S222`: the level's cells are the game's, and what the module places hangs on them where it stands), the
  grass of the painted details (the game grows the level's own and reads none of a module's) and an AI map
  painted over the level's (the level's is the game's; a module changes it under a terrain that cuts the
  ground, never by painting) - both `S223`.

## 8. Pictures without a window

Every picture below is drawn on the processor (`gz-pix`): no window, no graphics card, the same bytes for the
same input. They work in `Grizzly-Cmd.exe`, `-headless` and beside a window. A face wears the average colour of
its texture, lifted and lit from one side; a face whose texture is mostly holes (leaves, nets) is left out, so
it hides nothing. What a picture is for is shape, size and place: every answer says what a pixel means.

- `asset.preview ref=<asset>`: a model, a level (`kind=level`), a spawn section's model (`kind=section`) as a
  sheet of the sides asked for (`views=[iso, front, right, top]` by default, `back`, `left`, `bottom`, or one
  more by `heading=` and `pitch=`), each tile with its view's name and a scale bar, the size written under them;
  a texture (`kind=texture`) as it is over a chequered ground. The answer says each tile's rectangle and
  `px_per_m`.
- `asset.thumbnail refs=[...]`: a sheet of small pictures (128 px) of up to 64 models, each with its name and
  size - what to choose from. A small picture is kept in the library's `thumbs\` with the model it was made of
  and answered from there while the model is the same; the folder keeps 4096 of them, and past that the ones
  made longest ago go.
- `shot.scene`: the open scene from a camera of the picture's own - `position=` and `target=` (as an eye sees),
  or `view=` framing `objects=`, an `area=`, or what the scene holds of its own (flat, to scale) - with
  `cut=<height>` (nothing wholly above it: a storey of a building seen from above without its ceiling). What the
  scene holds of its own is marked and, where it is seen, named; the answer's `legend` says for each its pixel,
  its depth and whether it is seen, and `camera` says the eye, its axes and the lens.

## 9. Checks that say what would not work

- A value picked from the game - a sound, a particle effect, a light animation, a sound environment, a texture,
  a compiled model, a fog volume's profile (a file of the game's configs) - that neither the game nor the
  project has is a warning at its key (`S220`), with the nearest names there are; a directional light asked to
  shine in the game is `S221`, a sector or a portal of a scene over a level of the game `S222`, details or an AI
  map painted over one `S223`. What the project may bring in a file nobody here reads - a particle
  library, the light animations or the sound environments of its own (`gamedata` `particles.xr`, `lanims.xr`,
  `senvironment.xr`), an effect shipped loose - is not told against the game's lists; and while the library is
  being read its catalogs are half written, so no name is told against them. An unknown key that is nearly a key of the kind is a warning that names
  the key meant.
- What a build left for a level no scene of the project is of any more is a warning of the next build, by the
  level build's own record of that level (`gamedata\levels\<level>\build.state`): layers made by hand have
  none and are told nothing, the level of the scene that is open counts as it is now, and while a scene of
  the project does not read nothing is called a leftover. Opening a level from the browser makes no second
  scene over it while one of the project's scenes does not read - it names that scene.
- `base` naming a level that is not at hand is `S060`; an entry that takes out what the level has not is `S061`;
  an entry that takes out and says more than `name` and `removed` is `S062`.
- `scene.query` and `scene.audit` test against what stops a body, not what is drawn: the collision of a level,
  the faces of a model whose surfaces collide. `ground` answers the material of a collision face; `free` looks
  for room on the floor of the point's height (`top=true`: from above everything); `heights` takes a `position`
  to read the floor at its height. The audit adds `S215` (a wall or a steep slope of a level or a terrain
  through an object's middle), `S216` (a static object nothing collides with), `S217` (an object on faces that
  are drawn and stop nothing); a way that is looked along is not asked to lie on the AI map.
- A command that reads the scene first takes in what another program wrote into its files: whoever writes a file
  and asks at once is answered about what it wrote.

## 10. Commands

| Command | Effect | What |
|---|---|---|
| `game.library` | read / write | the library's folder, state and counts; `action=sync` |
| `asset.find` | read | searches the library: `kind=`, `query=`, a size range, `collides=`, sorted, paged |
| `asset.card` | read | everything known of one asset (section 3), whatever it is |
| `asset.preview`, `asset.thumbnail` | read | pictures (section 8) |
| `game.read` | read | a text file of the game (a config, a script) as text, by lines |
| `game.search` | read | a text searched in the game's configs and scripts |
| `scene.new base=` | write | a scene over a level of the game (section 5) |
| `shot.scene` | read | the scene drawn on the processor (section 8) |

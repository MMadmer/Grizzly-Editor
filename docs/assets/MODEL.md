# Models (`.meshasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-media/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-media` fails when this file is not what the tables say. A model asset is a text file beside its geometry - a glTF 2.0 (`.glb`, `.gltf`), a Wavefront OBJ, an X-Ray SDK `.object` or a compiled model (`.ogf`) -, over a model of the game (`source = "game:..."`), or with its geometry written in it (`parts`). A scene object refers to it by its project path: `ref = "models/bunker/wall_long.meshasset"` (any slashes, any case); any other reference is a library object as before. A static model is compiled into the level by the level build; a dynamic one is published as `meshes\<name>.ogf` (and `.omf` with its motions) beside this text, which the game reads over the model: each surface's texture and shader, each bone's physics, the user data, each motion's speed and flags. The game reads the rows of `surfaces` by position - row n is the nth part the model is drawn in -, so the text that ships has one row for each of them, in their order; here a row is found by its `name`, and rows may be left out or stand in any order.

glTF and OBJ are right-handed and the game is left-handed: Z is negated as the source is read and every triangle turned to face the same way; hard edges of the source's normals are kept; a skin becomes bones and weights (the four heaviest), its clips motions sampled 30 times a second.

Every key but the header may be left out. A key the editor does not know is kept where it is through every save and listed once as a note. The asset may lie anywhere in the project: the module build publishes it - the data baked from its source and a copy of this text - where the game looks for its `name`; the asset where it lies and its source never ship as they are. What is baked is kept in the project's `temp\media\`. `asset.check` names every problem at its key (`M1xx`); `asset.import from=<files> to=<folder>` brings files in and writes their assets. The rules every kind keeps are `docs/ASSETS.md`.

```lua
return {
  mesh = 1,
  source = "wall_long.glb",
  surfaces = {
    { name = "brick", texture = "bunker\\wall_brick", material = "materials\\bricks" },
  },
}
```

A model of the game dressed differently - no geometry of the module's own; `asset.find kind=model` lists the game's models and `asset.card` says what each part wears:

```lua
return {
  mesh = 1,
  kind = "dynamic",
  name = "bunker\\box_red",
  source = "game:dynamics/box/box_wood_01.ogf",
  surfaces = {
    { name = "box_wood_01", texture = "bunker\\box_red" },
  },
}
```

A blockout, no source:

```lua
return {
  mesh = 1,
  parts = {
    { shape = "box", size = { 4, 3, 0.3 }, pos = { 0, 0, 0 }, surface = "brick", tile = 2 },
    { shape = "ramp", size = { 2, 1, 4 }, pos = { 3, 0, 0 }, surface = "concrete" },
    { shape = "cylinder", radius = 0.2, height = 3, sides = 12, pos = { 0, 0, 2 }, surface = "metal" },
    { shape = "plane", size = { 10, 10 }, surface = "concrete", tile = 4 },
    { shape = "prism", outline = { { 0, 0 }, { 6, 0 }, { 6, 4 }, { 0, 4 } }, height = 3, surface = "brick" },
  },
}
```

## The keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `mesh` | integer | - | The header: the format version (1). A file of a newer version is refused by name. |
| `source` | string (a file beside the asset) | the file with the asset's own name and `.glb`, `.gltf`, `.obj`, `.object` or `.ogf` | The model's geometry: glTF 2.0, Wavefront OBJ, an X-Ray SDK `.object`, or a compiled model - an `.ogf` of the project, or a model of the game by its engine name (`"game:dynamics/box/box_wood_01.ogf"`: the project's own `gamedata\meshes` first, then the linked game). Over a compiled model the asset says what changes: `surfaces` by the name of the texture a part wears, `userdata`, a bone's `mass` and `material`; a dynamic one is published as the same file with that said again - its skeleton, its motions and its levels of detail stay its own. Not together with `parts`. |
| `name` | string (a game name) | `<module id>\<file name>` | The name the game knows the model by: a dynamic model is published as `meshes\<name>.ogf`. A static one is compiled into the level and needs none. |
| `kind` | `"static"` `"dynamic"` | "static" | `static`: geometry the level build compiles into the level; `dynamic`: an object the game spawns, published as its own model. |
| `scale` | number, 0.001 to 1000 | 1 | Applied to the source as it is read. |
| `rotate` | { x, y, z } (degrees) | { 0, 0, 0 } | Turns the source as it is read: heading (Y), then pitch (X), then bank (Z). Not applied to a model with bones. |
| `surfaces` | a list of surfaces (below) | the source's | Restates the source's surfaces by their names; a key left out keeps the source's value. |
| `parts` | a list of parts (below) | none | Geometry written in the asset instead of a source: a blockout an agent types. |
| `bones` | a list of bones (below) | the source's | A dynamic model's bones by name: their physics. A dynamic model without bones gets one bone, `root`. |
| `animations` | a list of animations (below) | the source's clips | A dynamic model's motions: the source's clips renamed and set. |
| `userdata` | string | none | A dynamic model's user data: the ini text the game reads from the model. |

## Over a compiled model

A source that is an `.ogf` - a file of the project, or `game:<name under meshes>.ogf`, read from the project's own `gamedata\meshes` first and then from the linked game - is a model that is compiled already. The asset says what changes and nothing else:

- **`surfaces`**: one surface per texture and shader pair the model's parts wear, named after the texture's last name (`prop\prop_crate` is the surface `prop_crate`; `model.info` lists them). A row gives it another `texture`, `shader`, and - for a static model - `compile`, `material`, `two_sided`.
- **static** (the default): the model's geometry at rest is compiled into the level like any other; `scale` and `rotate` apply.
- **dynamic**: published as `meshes\<name>.ogf` - the same file with each changed part's texture and shader, `userdata` and a bone's `mass` said again; a file nothing is asked of is published byte for byte. Its skeleton, its motions (and the motion files it names) and its levels of detail stay its own: `scale` and `rotate` are not applied and say so (`M111`), and a row of `bones` names a bone of the file. The rest is said to the game in the description shipped beside the model, and the game applies it as the model loads: a bone's `material` when it is a dynamic one, a bone's `shape` as the kind alone - the game switches to it when the model holds the size of such a shape for the bone -, and a row of `animations` as the `speed` and the flags of a motion the file itself holds (a motion of a motion file it names is shared with other models and keeps its own; `clip` names nothing here).

The game is never written: what ships is the module's own file under the module's own name.

## A row of `surfaces`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | - | The source's material name the row is about (any case). |
| `texture` | string (a game name) | the texture asset of the source's picture | The texture, by its game name (a texture asset's `name`, a texture of the project or the game). |
| `shader` | string | "default" | The engine shader (`shaders.xr` of the linked game). |
| `compile` | string | "default" | The level compiler's shader (`shaders_xrlc.xr`). |
| `material` | string | "default" | The game material (`gamemtl.xr`): how the surface sounds, breaks and is walked on. |
| `two_sided` | boolean | the source's | Both sides are drawn. |

## A row of `parts`

Every face of a part looks outward and every edge is hard; the texture is laid by box mapping in the part's own frame, one texture every `tile` metres.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `shape` | `"box"` `"ramp"` `"cylinder"` `"plane"` `"prism"` | - | What the part is: a box, a ramp (a wedge rising along +Z), a cylinder (upright), a plane (facing up) or a prism (an outline extruded up). |
| `size` | { x, y, z } metres; a plane's { x, z } | { 1, 1, 1 } | The box, the ramp or the plane. A box and a ramp stand on their base: `pos` is the middle of the bottom. |
| `pos` | { x, y, z } | { 0, 0, 0 } | Where the part stands, metres. |
| `rot` | { x, y, z } (degrees) | { 0, 0, 0 } | How the part is turned: heading (Y), then pitch (X), then bank (Z). |
| `surface` | string | "default" | The surface the part wears: a row of `surfaces`, or one made with the defaults. |
| `tile` | number, 0.001 to 10000 | 1 | The texture repeats every this many metres (laid by box mapping). |
| `radius` | number, 0.001 to 10000 | 0.5 | A cylinder's radius. |
| `height` | number, 0.001 to 10000 | 1 | A cylinder's or a prism's height. |
| `sides` | integer, 3 to 256 | 16 | A cylinder's sides. |
| `outline` | a list of { x, z } points, metres | - | A prism's outline seen from above, any simple polygon, either way round. |

## A row of `bones`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | - | The bone the row is about. |
| `shape` | `"none"` `"box"` `"sphere"` `"cylinder"` | "none" | The bone's collision shape, fitted around the bone's vertices. |
| `mass` | number, 0 to 100000 | 10 | The bone's mass, kilograms. |
| `material` | string | "default_object" | The bone's game material, a dynamic one of `gamemtl.xr` (`objects\...`): said to the game in the model's description, which takes it when it is dynamic and keeps the model's own otherwise. The published model itself keeps the material its source has. |

## A row of `animations`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | - | The motion's name in the game. |
| `clip` | string | the row's `name` | The source's clip the motion is made of. |
| `speed` | number, 0 to 100 | 1 | How fast the motion plays. |
| `loop` | boolean | true | The motion starts again at its end; false: it stops there. |
| `no_mix` | boolean | false | The motion replaces what plays instead of blending in. |
| `sync_part` | boolean | false | The motion keeps time with the other parts of the body. |
| `foot_steps` | boolean | false | The motion's marks sound as steps. |
| `root_mover` | boolean | false | The root bone's motion moves the object. |
| `idle` | boolean | false | A motion the game may cut short: an idle. |
| `weapon_bone` | boolean | false | The motion drives the weapon's bone. |

## Problems

| Code | Severity | What |
|---|---|---|
| `M101` | error | The source is not there: no such file, or a model neither the project nor the linked game has. |
| `M102` | error | The source cannot be read (the reader's sentence). |
| `M103` | error | Two assets of the kind publish one name (on both). |
| `M104` | warning | A texture nobody has: not a texture asset, not a file of the project, not the game's. |
| `M105` | warning | An engine shader the linked game does not have. |
| `M106` | warning | A compiler shader the linked game does not have. |
| `M107` | error | A surface's game material the linked game does not have: the level build refuses it (a bone's: a warning). |
| `M108` | note | A dynamic model whose source has no bones: it gets one bone, `root`. |
| `M109` | error | A value outside what its key takes. |
| `M110` | error | `parts` together with `source`. |
| `M111` | warning, error | What the model cannot take: `rotate` on a model with bones, a part that makes no faces (an error when no part makes any: there is no model), a row of `surfaces` that names no surface of the model, and over a compiled dynamic model `scale`, `rotate`, a bone the file lacks, an animation's `clip` and a row of `animations` that names no motion the file itself holds. |
| `M112` | error | A surface that wears no texture: no level build and no model of the game takes it. |
| `M113` | error | A source that lies outside the project: a project names nothing outside itself. |
| `M114` | warning | A sound's `streamer` on a sound that is not `copyrighted`: the replacement never plays. |
| `M115` | error | A sound's replacement that is copyrighted itself: the streamer mode keeps it from playing too. |
| `M116` | error | A sound's replacement nobody has: no sound asset of the project, no sound of the linked game. |

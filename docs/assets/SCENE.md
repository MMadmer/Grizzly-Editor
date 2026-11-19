# The scene

This file is generated from the editor's own tables of the scene's keys (`crates/gz-scene/src/format/`): the reader, the writer, the Details panel and this reference are one list, so they cannot drift apart. A test of `gz-scene` fails when this file is not what the tables say. The contract the format keeps is `docs/ASSETS.md` (sections 2, 3 and 7).

## Layout

A scene is `<name>.scene` and the folder `<name>\` beside it:

```text
escape.scene        the scene: header, level options, build settings, the sun, the camera, the kinds' eyes
escape\
  objects.lua       objects (Objects)
  lights.lua        lights (Lights)
  glows.lua         glows (Glows)
  sounds.lua        sources (Sound Sources), environments (Sound Environments)
  particles.lua     particles (Particle Effects)
  shapes.lua        shapes (Shapes)
  fog.lua           volumes (Fog Volumes)
  spawns.lua        spawns (Spawn Points)
  ways.lua          ways (Way Paths)
  zones.lua         sectors (Sectors), portals (Portals)
  groups.lua        groups (Groups)
  terrain.lua       terrains (Terrains)
  ai_map.bin        the ai_map layer (binary)
  details.bin       the details layer (binary)
  wallmarks.bin     the wallmarks layer (binary)
```

Every file is Lua data (`return { ... }` of literals). The `.scene` starts with the header `scene = 2`; the files of the folder are parts of the scene and carry no header. A file of a kind the scene has no objects of does not exist. A key at its default is not written, and a key the editor does not know is kept where it is through every save. A save writes only what changed: an object nobody touched keeps its bytes and its comments.

Positions are metres, angles degrees, colours `{ r, g, b }` from 0 to 1. Things are named, never numbered: an object's `name` is unique in the scene (in any case; the case written is kept), and whatever points at an object - a group's members, a sector's meshes, a portal's sectors, a way's links - names it. A number is the shortest text that reads back to the very value; the few values a number of the file cannot be are words: `"-0"`, `"nan"`, `"inf"`, `"-inf"`.

## The .scene

| Key | Type | Default | Meaning |
|---|---|---|---|
| `base` | string (game_level) | "" | The level of the game the scene lies over: the level's own objects are read from the game and never written, and the scene's files hold what the module adds. |
| `level.path` | string | "level" | The folder of the built level under the game's `levels`. |
| `level.prefix` | string | "level_prefix" | What the level's own names start with. |
| `level.map_version` | string | "1.0" | The version the level's `level.ltx` gives in `[map_usage]`. |
| `level.custom_data` | string | "" | Lines the build adds to the level's `level.ltx` as they are. |
| `level.game_types` | a list of `"single"`, `"deathmatch"`, `"team_deathmatch"`, `"artefact_hunt"`, `"capture_the_artefact"`, `"domination_zone"`, `"team_domination_zone"` | every one | The game types the level is made for. |
| `build.quality` | `"draft"` `"high"` `"custom"` | "high" | Draft and high set the values below; custom keeps them as they are set. |
| `build.sky_quality` | integer | 3 | How finely the build samples the light of the sky. |
| `build.sun_quality` | integer | 3 | How finely the build samples the sun. |
| `build.pixels_per_meter` | number (px/m) | 10 | Lightmap pixels per metre of surface. |
| `build.jitter_samples` | `"1"` `"4"` `"9"` | 9 | Light samples per lightmap pixel, 1, 4 or 9: more is smoother and slower to build. |
| `build.smooth_angle` | number (deg) | 75 | Faces that meet at a smaller angle are shaded smooth. |
| `build.weld_distance` | number (m) | 0.005 | Vertices closer than this are merged. |
| `build.collapse_error` | integer | 4 | How much a lightmap may differ from a flat colour to be collapsed into one. |
| `build.zero_error` | integer | 4 | How dark a lightmap may be to be dropped as black. |
| `build.cell_size` | integer (m) | 0 | The game loads the level's picture cell by cell around the player: the size of a cell, 8 m or more. 0 keeps the level one piece. |
| `build.reserved` | a list of 6 whole numbers: the bits of the floats | none | Values of the build parameters no key names, kept bit for bit as an X-Ray SDK scene had them. |
| `build.reserved_word` | integer | 0 | A word of the build parameters no key names. |
| `sun.altitude` | number (deg) | -25 | How high the sun stands for the build's sun shadows. |
| `sun.longitude` | number (deg) | 292 | Where the sun stands round the horizon. |
| `light_controls` | a list of names | none | The scene's own groups of lights besides `$static`, `$hemi` and `$sun`; a light names its group in `control`. |
| `camera.pos` | { x, y, z } (m) | { 0, 0, 0 } | Where the viewport's camera stood when the scene was saved. |
| `camera.heading` | number (deg) | 0 | Where the camera looked, round the vertical. |
| `camera.pitch` | number (deg) | 0 | How far the camera looked up or down. |
| `camera.bank` | number (deg) | 0 | The camera's roll. |
| `classes` | `{ <kind> = { hidden = true, locked = true } }` - kinds: `object`, `light`, `glow`, `sound`, `sound_env`, `particles`, `shape`, `fog`, `spawn`, `way`, `sector`, `portal`, `group`, `terrain`, `ai_map`, `details`, `wallmarks` | none | The kinds of objects that are hidden or locked in the viewport. |
| `view_flags` | `{ <kind> = n }` | none | What the viewport draws of each kind (names, crosses, solid sectors), as the kinds' flags. |
| `random_placement` | `{ on = true, scale = { {min}, {max} }, rotation = { {min}, {max} }, proportional = true, objects = { ref, ... } }` | off | What placing a library object picks at random: the object of a list, its scale and its rotation. |
| `layers` | `{ ai_map = "ai_map.bin", details = "details.bin", wallmarks = "wallmarks.bin" }` | none | The painted layers' files in the scene's folder. |

## A scene over a level of the game

A module that adds to a level of the game says `base = "<level>"` and writes only what it adds:

```lua
-- rawdata/levels/agroprom_base.scene
return {
scene = 2,
base = "l03_agroprom",
}
-- rawdata/levels/agroprom_base/spawns.lua
return {
spawns = {
{ name = "agr_smart_terrain_1_2_anomal_zone", removed = true },
{ name = "base_guard_gate", pos = { 40, 2.5, -12 }, rot = { 0, 90, 0 }, section = "agro_base_guard" },
},
}
```

The level itself is the game's and is never copied into the project: as the scene is read its objects come from the game library (`docs/GAME_LIBRARY.md`) - the level's geometry as one object (`level_geometry`, with the level's own collision), every spawn point of the game's spawn that stands on it, its patrol paths, lights, glows, sounds and effects, fog volumes and sound environments, and its AI map. They stand in the scene for every query, check and picture; `scene.objects` answers them with `level = true` (`own = true` leaves them out), and nothing of them is the scene's to change. What the library holds of the level - the same objects as text, and the level's passport: its structures, floors, materials and heights - is `asset.card ref=<level> kind=level`.

- Everything the scene's files hold is the module's: `authored` is implied and not written.
- An entry of `spawns.lua` with `removed = true` and no `section` is no spawn point of the scene's: it takes the level's spawn point of that `name` out of the game for the module's mode. It says `name` and `removed` alone. An entry that names what the level has not is a warning (`S061`) and stays in the file.
- A key of the `.scene` left out is the level's: its folder, its sun, its light controls.
- The level's painted layers are the level's; a layer file in the scene's folder is the module's own, made when the scene paints it. Wallmarks painted so ship as part of the module's piece of the level; details (grass) and a painted AI map do not ship - the game grows the level's own grass and keeps the level's own AI map, which a module changes under a terrain that cuts the ground alone (`S223`).
- What the module adds beside objects and spawn points - sound sources, particle effects, lights that shine in the game, environment modifiers, glows, fog volumes, sound environments - the build writes into `levels\<level>\overlay.things`, which the game puts beside the level's own. Sectors, portals, the bound of the level and of its AI map are the game's there (`S222`).
- A level the game library cannot give (`S060`: no game linked, no level of that name) leaves the scene with what its own files say; a save then writes them as they are.

A scene without `base` is a level of the module's own, or an X-Ray SDK scene imported whole, where `authored` and `removed` are said object by object.

## Every object

Each list of a folder's file holds one table per object, on its own line or block, `name` first:

```lua
return {
objects = {
{ name = "box_wood_01", ref = "props\\box_wood_01", pos = { 1, 0, 2 }, rot = { 0, 90, 0 } },
},
}
```

These keys belong to objects of every kind (the transform only to the kinds that have it):

| Key | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | - | Unique in the scene: quests, orders, sectors and groups call the object by it. |
| `pos` | { x, y, z } (m) | { 0, 0, 0 } | Where it stands, in metres. |
| `rot` | { x, y, z } (deg) | { 0, 0, 0 } | Pitch, heading and bank, in degrees. |
| `scale` | { x, y, z } | { 1, 1, 1 } | A factor per axis. |
| `authored` | boolean | false | The module adds this object to the base level: it ships in the module's level layers. |
| `removed` | boolean | false | The module takes this spawn of the base level out of the game. |
| `hidden` | boolean | false | Not drawn and not picked in the viewport; the build takes it all the same. |
| `locked` | boolean | false | Drawn, but not picked or changed in the viewport. |
| `group_editable` | boolean | false | A member of a group that is picked and changed by itself; reloading the group keeps it as it is. |
| `draw_selected` | boolean | false | Drawn while it is selected even when its kind is hidden. |
| `motion` | `{ data = "<hex>", time = s, auto_key = true, camera_view = true }`: the X-Ray SDK object motion (.anm body), hex | none | The path a dynamic object moves along, and where its playback stands. |
| `flags` | integer | 0 | Bits of the object's flags no key above names, kept as they were. |

## Objects - `objects` of `objects.lua`

Static and dynamic objects of the library.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `ref` | string (object) | - | The library object (`props\box_wood_01`), or the game model it names in visual mode. |
| `visual` | boolean | false | The reference names a game model (`meshes\...ogf`) instead of a library object. |
| `surfaces` | a list of `{ name, texture, shader, compile, material }` | none | Surfaces of the reference with this object's own texture, shaders and material. |
| `object_flags` | integer | 0 | Bits of the object's own flags no key names, kept as they were. |

## Lights - `lights` of `lights.lua`

The lights the build bakes and the game shines. Every kind is baked into the lightmaps (`lightmap`): a point light all around, a spot light inside its `cone` along where it looks (all of it on the axis, none at the cone's edge), a directional one from its direction. In the game (`dynamic`) a point light and a spot light shine - the spot through the level's `overlay.things` -; a directional one never does, the game's one directional light being the level's sun (`S221`).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `kind` | `"point"` `"spot"` `"directional"` | "point" | How the light shines: all around, inside a cone along where it looks (`rot`), or from one direction over the whole level. |
| `color` | { r, g, b } | { 1, 1, 1 } | The colour of its light. |
| `brightness` | number | 1 | Multiplies the colour; below 0 the light darkens. |
| `range` | number (m) | 8 | How far it reaches. |
| `attenuation` | { x, y, z } | { 1, 0, 0 } | Its constant, linear and quadratic falloff over the range. |
| `cone` | number (deg) | 22.5 | The whole opening of a spot light: its light is full on the axis and gone at the cone's edge. |
| `texture` | string (texture) | - | The picture a spot light throws. |
| `virtual_size` | number (m) | 0 | The size of the light's source the shadows are soft by. |
| `control` | string (light_control) | "$static" | The group of lights it is switched with: `$static`, `$hemi`, `$sun` or one of the scene's `light_controls`. |
| `animation` | string (lightanim) | - | A colour animation of the game's light animations. |
| `lightmap` | boolean | true | The build bakes it into the lightmaps: of the level, or of the module's piece over a level of the game. |
| `dynamic` | boolean | false | It shines in the game and lights what moves: a point light or a spot light; a directional one does not (`S221`). |
| `animated` | boolean | false | Its colour animation plays in the game. |
| `breaking` | boolean | false | It flickers out now and then. |
| `shadows` | boolean | false | Objects cast shadows in it. |
| `fuzzy` | `{ shape = "sphere" \| "box", radius = m, box = { x, y, z }, points = { { x, y, z }, ... } }` | none | A point light spread over random points of a sphere or a box, for softer lightmaps. |
| `light_flags` | integer | 0 | Bits of the light's flags no key names, kept as they were. |

## Glows - `glows` of `glows.lua`

Glows: a picture that always faces the viewer, at a place - the halo of a lamp. The game draws the ones a module brings (the level's `overlay.things`), fading each in while its place is in sight.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `texture` | string (texture) | - | The glow's picture. |
| `shader` | string (e_shader) | "effects\\glow" | How it is drawn: left out (or `effects\glow`, `effects\flare`) like a lens flare - its picture added to the frame, gone when something of the level stands between the eye and it; `effects\sun` the same, hidden by depth as well. The game draws a glow with no other shader (`S225`). |
| `radius` | number (m) | 0.5 | How big it is. |
| `fixed_size` | boolean | false | The same size on the screen at any distance: `radius` is then a share of half the screen's height (0.02 is a small light), not metres. |
| `glow_flags` | integer | 0 | Bits of the glow's flags no key names. |

## Sound Sources - `sources` of `sounds.lua`

Sound sources: a sound of the game played at a place.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `sound` | string (sound_source) | - | The game's sound it plays (`ambient\wind_1`). |
| `volume` | number | 1 | How loud, from 0 to 1. |
| `frequency` | number | 1 | Played faster (above 1) or slower. |
| `min_distance` | number (m) | 1 | Up to this distance it is heard at its full volume. |
| `max_distance` | number (m) | 300 | Beyond this distance it is not heard. |
| `ai_distance` | number (m) | 300 | How far the game's characters hear it. |
| `pause` | { from, to } (s) | { 0, 0 } | A random pause between plays, from and to; 0 plays it looped. |
| `active` | { from, to } (s) | { 0, 0 } | The time of the game's day it plays in, from and to, in seconds after midnight; 0 to 0 is round the clock. |
| `play` | { from, to } (s) | { 0, 0 } | How long each play lasts, from and to; 0 plays the sound once. |
| `sound_flags` | integer | 1 | The source's flags, kept as they were. |

## Sound Environments - `environments` of `sounds.lua`

Sound environments: boxes that change how sound echoes. `pos`, `rot` and `scale` say the box (a unit box scaled: `scale = { 12, 4, 7 }` is 12 x 4 x 7 m). Inside it the game plays `inner`; straight above it, `outer`. `asset.find kind=sound_env` lists the game's environments. They reach the game through the level's `overlay.things`, on a level of the module's own and over a level of the game alike.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `inner` | string (sound_env) | - | The sound environment inside the volume. |
| `outer` | string (sound_env) | - | The sound environment outside the volume. |
| `shapes` | a list of `{ kind = "sphere", center = { x, y, z }, radius = m }` and `{ kind = "box", size = { x, y, z }, center = { x, y, z } }` - the box's sides in metres, `rot = { x, y, z }` (deg) to turn it, or its three axes `x`, `y`, `z` instead of `size`; `center` is where it stands from the object, left out at it | one unit box | The spheres and boxes the volume is made of. |

## Particle Effects - `particles` of `particles.lua`

Particle effects placed in the level.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `ref` | string (particles) | - | The particle effect of the game (`fire\fire_01`). |
| `game_types` | a list of `"single"`, `"deathmatch"`, `"team_deathmatch"`, `"artefact_hunt"`, `"capture_the_artefact"`, `"domination_zone"`, `"team_domination_zone"` | every one | The game types it plays in. |

## Shapes - `shapes` of `shapes.lua`

Shapes: volumes of spheres and boxes (and the level's bound).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `shapes` | a list of `{ kind = "sphere", center = { x, y, z }, radius = m }` and `{ kind = "box", size = { x, y, z }, center = { x, y, z } }` - the box's sides in metres, `rot = { x, y, z }` (deg) to turn it, or its three axes `x`, `y`, `z` instead of `size`; `center` is where it stands from the object, left out at it | none | The spheres and boxes it is made of, in its own coordinates. |
| `usage` | `"common"` `"level_bound"` `"ai_map_bounds"` | "common" | What the shape is for: a volume of its own, the bound of the level, or the AI Map Bounds the AI map is generated inside. |

## Fog Volumes - `volumes` of `fog.lua`

Fog volumes: one box each (`pos`, `rot`, `scale`: a unit box scaled). An emitter's box fills with fog that flows by its `profile`; the occlusion volumes of its `group` are what the fog flows around. A group has one emitter. The fog fills its whole box and is lit by the lights around it: a small box in a still place reads as mist, a large one as smoke. The game shows it with volumetric fog on.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `kind` | `"emitter"` `"occlusion"` | "emitter" | An emitter fills its box with fog; an occlusion volume keeps fog out. |
| `group` | integer | none | Volumes of one group are one fog. |
| `profile` | string (fog_profile) | "environment\\fog\\default.ltx" | How the fog flows, a file of the game's configs said from there (`S220` when neither the game nor the project has it). |

## Spawn Points - `spawns` of `spawns.lua`

Spawn points: what the game spawns, and the editor's own points.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `section` | string (spawn_section) | - | The game's section the spawned thing is made from (`game.lookup type=spawn_section` lists them). |
| `point` | `"player_start"` `"respawn"` `"env_mod"` | - | A point of the editor's own instead of a section: the module's Player Start, a respawn point of the multiplayer games, an environment modifier. |
| `props` | the entity's named properties that differ from a new entity of the section (`story_id`, `custom_data`, `visual`, `trader = { money = 500 }`, ...); `asset.reference kind=scene` lists them | none | What this spawn says beyond its section's defaults. |
| `raw` | string | none | The entity's spawn packet as hex, for an entity the named properties cannot carry (its class is not known, or it holds more than its class writes): written back as it was. |
| `values` | a list of texts | none | The numbered values an X-Ray SDK text scene gave an entity whose class the linked game does not know: written back as they were. |
| `respawn` | boolean | false | The multiplayer games spawn it again after it is gone. |
| `spawn_flags` | integer | 0 | Bits of the spawn's flags no key names. |
| `order` | `{ kind = "guard" \| "patrol" \| "wander" \| "follow", route = way, look = way, radius = m, target = name, distance = m, run = true, hostile = true }` | none | A placed stalker's standing order; the build writes it into the entity's custom data as `[xms_order]`. |
| `radio` | `{ plays = "<playlist id>" \| "<module>.<playlist>" \| "game" \| "off" }`: a playlist of the module by its id (`mod.ltx [playlist.<id>]`), another module's, the game's music alone, silence | none: the player's station | What a radio of the world plays whatever station the player chose; the build writes it into the entity's custom data as `[xms_radio]`. |
| `respawn_point` | `{ team = 1, kind = "actor" \| "artefact" \| "item", game_types = {...}, profile = section }` | `{ team = 1, kind = "actor" }` | Who or what respawns there in the multiplayer games. |
| `environment` | `{ radius = m, power, view_distance = m, fog_color = { r, g, b }, fog_density, ambient_color, sky_color, hemi_color, uses = { "view_distance", "fog_color", ... } }` | radius 10, power 1, view distance 300, every value used | How the weather looks within the modifier's radius. |
| `shape` | `{ shapes = { ... } }` as in `shapes.lua`, `pos`, `rot`, `scale` where they are not the point's | none | The volume of a zone, a restrictor or a level changer. |

## Way Paths - `ways` of `ways.lua`

Way paths: named points and the links between them; orders and quests walk them.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `kind` | `"patrol"` | "patrol" | What the game takes the way for. |
| `points` | a list of `{ name, pos = { x, y, z }, flags = n, links = { "point", { "point", probability }, ... } }` (a point whose name another point shares is linked by its number from 0) | one point | The points of the way and where each leads; orders and quests walk them by name. |

## Sectors - `sectors` of `zones.lua`

Sectors: the cells the visibility of a level of the module's own is divided into. Over a level of the game the cells are the game's and a module adds none (`S222`).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `color` | { r, g, b } | { 1, 1, 1 } | How the viewport shows it. |
| `default` | boolean | false | The sector of everything no other sector holds. |
| `items` | a list of `{ "object", "mesh" }` | none | The meshes of scene objects the sector is made of, by object and mesh name. |
| `map` | integer | the default | The light map its faces go to. |

## Portals - `portals` of `zones.lua`

Portals: the openings between two sectors of a level of the module's own (`S222` over a level of the game).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `front` | string | - | The sector in front of it. |
| `back` | string | - | The sector behind it. |
| `vertices` | a list of `{ x, y, z }` | none | Its polygon, in the level's coordinates. |

## Groups - `groups` of `groups.lua`

Groups: objects held together; the members stay in their own files.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `ref` | string (group) | - | The group asset or library group it was placed from. |
| `members` | a list of object names | none | The objects the group holds, by name; each stays in its own file. |

## Terrains - `terrains` of `terrain.lua`

Terrains: a height grid over a rectangle that replaces the ground under it. `pos` is its centre, `rot` turns it by its heading alone. Its model is made from these keys (and, on the ground, from the static objects under it that the module does not add); with `cut` on, the ground it replaces is cut out of the level wherever the editor and the game look: a triangle of a static object that is not `authored` is clipped by the rectangle, and the part inside goes where it lies within 0.5 m of the old ground.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `size` | `{ x, z }` in metres | { 20, 20 } | How far it reaches along its own x and z, in metres; pos is its centre. |
| `cell` | number (m) | 1 | The step of the height grid, 0.5 to 4 metres. |
| `base` | `"ground"`, or a number: metres over the height of pos | "ground" | Where every point starts: the level's ground under it (of the static objects that are not added by the module, the surface nearest pos under the centre, followed outward: a roof or a tree's crown over it is not the ground), or flat at a height over pos. |
| `edge` | number (m) | 4 | On the ground: the metres along the border over which the edits blend back into the ground around, so the border meets it. |
| `edits` | a list, in order: `{ op = "flatten", at = { x, z }, radius, falloff, height }`, `{ op = "rect", min, max, falloff, height }`, `{ op = "raise", at, radius, falloff, amount }`, `{ op = "ramp", from, to, width, falloff, heights = { from, to } }`, `{ op = "smooth", at, radius, strength }`, `{ op = "noise", amount, scale, seed }` | none | How the heights change, one edit after the other: x and z are metres from the centre along the terrain's own axes, height is a world height, amount is relative, falloff is the width of the soft border. |
| `rows` | a list of rows of heights, from -z to +z, each from -x | none | Instead of the edits: every world height of the grid written out. |
| `surface` | `"ground"`, or `{ texture, shader, compile, material, tile }` | "ground" | What it looks like: the look and the texture coordinates of the ground under each face, or a surface of its own laid flat every tile metres. |
| `cut` | boolean | false | On a level of the game, the ground under it is cut out of the level: its picture, its collision and its AI map. |

## Spawn properties

`props` holds the named properties of the spawn's entity that differ from a new entity of its section - the fields of the game's server entity by name: the object's own (`story_id`, `spawn_story_id`, `custom_data = [[ ... ]]`, `graph_id`, `node_id`, `distance`, `alife_flags`), its model (`visual`, `visual_flags`, `startup_animation`), and a table per part of its class (`trader = { money = 500, specific_character = "..." }`, `creature = { health = 0.5 }`, `zone = { ... }`, `script = { ... }`, ...). `header` holds what the entity's header says beyond the object's name and transform. A part the entity lacks is `false`. The editor proves every save: the properties read back must give the very spawn packet the entity writes, or the spawn is kept as its packet (`raw`, hex) and `asset.check` names it. An entity of an X-Ray SDK text scene whose class the linked game does not know has no packet: it keeps the section's numbered values as they were (`values`).

A zone's volume is said once, by the point's `shape`: `shape = { shapes = { { kind = "box", size = { 20, 6, 12 } } } }`. The entity carries the same shapes and `props` does not repeat them; a file that says `props.shapes` and no `shape` is read into the point's shape.

A section is one of the linked game's (`game.lookup type=spawn_section`) or one the game makes of the module's own assets: a character `characters/guard.npcasset` of the module `agro_base` is the section `agro_base_guard`, placed like any other.

## The painted layers

`ai_map.bin`, `details.bin` and `wallmarks.bin` start with the magic `GZLAYER\0`, the layer format's version and the layer's kind (little-endian 32-bit words), then the layer's data as the editor holds it.

## The ground of the painted layers

The AI map, the detail objects and the wallmarks lie on **the ground**: every static scene object the collision build takes (not in visual mode, its library object not dynamic, no sound occluder, no HOM). A layer narrows it only by:

- an explicit list of object names - the AI map's and the details' own list, kept in their layer files (a list that names every static object is the same as none);
- for the AI map, the **AI Map Bounds** volume: a shape of usage `ai_map_bounds` (2, beside `common` 0 and `level_bound` 1). With one or more such shapes the map is generated inside their box, without one over the whole ground.

The AI map's generation parameters are the layer's own (`scene.ai_map action=set`). The build regenerates an AI map it generated itself when the ground, the bounds or the parameters moved, and keeps a map the author edited (`build.state` in the level folder says which). The detail objects are the game's 2 m slots, painted per mesh (`scene.details`); the layer keeps its meshes with their density, scale range and waving.

## Zones: what the build does with `zones.lua`

A level needs no hand-made zone. The build's geometry step completes what the scene holds and compiles the completed copy, never writing it back:

- a scene without sectors is one cell - nothing is added;
- a scene with sectors gets the **default sector** (`sector_default`, `default = true`) for every static mesh no sector's `items` hold - an existing `sector_default` is filled, else one is made;
- between the sectors no portal joins, **portals are computed** from the sectors' faces; a pair of sectors the author joined by hand keeps the author's portals.

`zones.lua` therefore holds only what the author made by hand (the Zones mode, `scene.zones`): a sector's `items` name a scene object and one mesh of its library object, compared without case; a portal's `front` and `back` name sectors; its `vertices` are its outline in order, at least three. `scene.zones action=compute` keeps in the scene what the build would add, and `action=list` answers `uncovered_meshes`, what the build would put into the default sector.

## Object groups (.group)

A `.group` asset is a set of objects saved for reuse: the header `group = 1`, then the same lists as the scene's folder files (`objects`, `lights`, `spawns`, ...) in one file, positions relative to the group's pivot. Placing it puts copies of its objects into the scene as one group.

## The X-Ray SDK scene

A `.level` with its folder of `.part` files is the X-Ray SDK's scene: the editor reads it and never writes it. Opening one converts it in memory; saving writes the `.scene` beside it. Its scene-wide Snap Objects list is not kept: placing and the wallmarks work on the whole level, and the AI map's and the detail objects' own lists stay with their layers.

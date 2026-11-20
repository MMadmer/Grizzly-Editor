# Sounds (`.sndasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-media/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-media` fails when this file is not what the tables say. A sound asset is a text file beside its sound (`.wav`, `.ogg`) that says what a sound's `.thm` says: how far it carries, how loud it is, what NPCs make of it. It is published as `sounds\<name>.ogg` - a `.wav` encoded at `quality`, an `.ogg` as it is with these parameters written into it - beside a copy of this text, which the game reads over what the `.ogg` carries.

A recording the module may not have streamed says `copyrighted = true`: while the game's streamer mode is on, the game plays it nowhere - its engine refuses to start it, the radio passes over it - unless `streamer` names a sound that plays in its place. The game keeps such a list of its own sounds; a replacement is never a copyrighted sound.

Every key but the header may be left out. A key the editor does not know is kept where it is through every save and listed once as a note. The asset may lie anywhere in the project: the module build publishes it - the data baked from its source and a copy of this text - where the game looks for its `name`; the asset where it lies and its source never ship as they are. What is baked is kept in the project's `temp\media\`. `asset.check` names every problem at its key (`M1xx`); `asset.import from=<files> to=<folder>` brings files in and writes their assets. The rules every kind keeps are `docs/ASSETS.md`.

```lua
return {
  snd = 1,
  source = "generator_hum.wav",
  name = "bunker\\generator_hum",
  max_distance = 40,
  type = "world_ambient",
}
```

## The keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `snd` | integer | - | The header: the format version (1). A file of a newer version is refused by name. |
| `source` | string (a file beside the asset) | the file with the asset's own name and `.wav` or `.ogg` | The sound. An `.ogg` whose own parameters are the asset's - or one with none the game reads (no comment, an encoder's tag) under an asset that states none - is published as it is; otherwise with the asset's parameters said again in it, its sound untouched. |
| `name` | string (a game name) | the path below `gamedata\sounds` for an asset that lies there, else `<module id>\<file name>` | The name the game knows the sound by: it is published as `sounds\<name>.ogg`. |
| `type` | one of these words, or a list of them: `"no_sound"` `"item_picking_up"` `"item_dropping"` `"item_hiding"` `"item_taking"` `"item_using"` `"weapon_shooting"` `"weapon_empty_clicking"` `"weapon_bullet_hit"` `"weapon_recharging"` `"monster_dying"` `"monster_injuring"` `"monster_step"` `"monster_talking"` `"monster_attacking"` `"monster_eating"` `"anomaly_idle"` `"world_object_breaking"` `"world_object_colliding"` `"world_object_exploding"` `"world_ambient"` `"weapon"` `"item"` `"monster"` `"anomaly"` `"world"` `"picking_up"` `"dropping"` `"hiding"` `"taking"` `"using"` `"shooting"` `"empty_clicking"` `"bullet_hit"` `"recharging"` `"dying"` `"injuring"` `"step"` `"talking"` `"attacking"` `"eating"` `"idle"` `"object_breaking"` `"object_colliding"` `"object_exploding"` `"ambient"` | "no_sound" | What NPCs make of it, in the game's own words. |
| `min_distance` | number, 0 to 1000000 | 1 | Up to this distance the sound plays at its full volume, metres. |
| `max_distance` | number, 0.1 to 1000000 | 300 | Beyond this distance it is not heard, metres. |
| `volume` | number, 0 to 10 | 1 | Its base volume. |
| `ai_distance` | number, 0.1 to 1000000 | `max_distance` | How far NPCs hear it, metres. |
| `quality` | number, -0.1 to 1 | 0 | The Vorbis quality a `.wav` source is encoded with. |
| `copyrighted` | boolean | false | A recording that may not be streamed: the game does not play it while its streamer mode is on. |
| `streamer` | string (a game name) | none: nothing plays | What plays in its place while the streamer mode is on, by its game name: a sound that is not copyrighted itself - a sound asset's `name` without `copyrighted`, or a sound of the game its own list (`configs\dead_air_x64_sound_rights.ltx`) does not name. Only for a copyrighted sound. |

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
| `M117` | error | Two assets over one file (on both): a file has one asset, which says where it ships. |
| `M118` | warning | The `.thm` beside a texture's picture says to the game what the asset does not: the game reads the asset, and the `.thm` ships no more - remove it, or say it in the asset. |

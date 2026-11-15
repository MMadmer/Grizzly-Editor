# Sounds (`.sndasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-media/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-media` fails when this file is not what the tables say. A sound asset is a text file beside its sound (`.wav`, `.ogg`) that says what a sound's `.thm` says: how far it carries, how loud it is, what NPCs make of it. It is published as `sounds\<name>.ogg` - a `.wav` encoded at `quality`, an `.ogg` as it is with these parameters written into it - beside a copy of this text, which the game reads over what the `.ogg` carries.

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
| `source` | string (a file beside the asset) | the file with the asset's own name and `.wav` or `.ogg` | The sound. |
| `name` | string (a game name) | `<module id>\<file name>` | The name the game knows the sound by: it is published as `sounds\<name>.ogg`. |
| `min_distance` | number, 0 to 10000 | 1 | Up to this distance the sound plays at its full volume, metres. |
| `max_distance` | number, 0.1 to 10000 | 300 | Beyond this distance it is not heard, metres. |
| `volume` | number, 0 to 10 | 1 | Its base volume. |
| `ai_distance` | number, 0.1 to 10000 | `max_distance` | How far NPCs hear it, metres. |
| `type` | one of these words, or a list of them: `"no_sound"` `"item_picking_up"` `"item_dropping"` `"item_hiding"` `"item_taking"` `"item_using"` `"weapon_shooting"` `"weapon_empty_clicking"` `"weapon_bullet_hit"` `"weapon_recharging"` `"monster_dying"` `"monster_injuring"` `"monster_step"` `"monster_talking"` `"monster_attacking"` `"monster_eating"` `"anomaly_idle"` `"world_object_breaking"` `"world_object_colliding"` `"world_object_exploding"` `"world_ambient"` `"weapon"` `"item"` `"monster"` `"anomaly"` `"world"` `"picking_up"` `"dropping"` `"hiding"` `"taking"` `"using"` `"shooting"` `"empty_clicking"` `"bullet_hit"` `"recharging"` `"dying"` `"injuring"` `"step"` `"talking"` `"attacking"` `"eating"` `"idle"` `"object_breaking"` `"object_colliding"` `"object_exploding"` `"ambient"` | "no_sound" | What NPCs make of it, in the game's own words. |
| `quality` | number, -0.1 to 1 | 0 | The Vorbis quality a `.wav` source is encoded with. |

## Problems

| Code | Severity | What |
|---|---|---|
| `M101` | error | The source is not there. |
| `M102` | error | The source cannot be read (the reader's sentence). |
| `M103` | error | Two assets of the kind publish one name (on both). |
| `M104` | warning | A texture nobody has: not a texture asset, not a file of the project, not the game's. |
| `M105` | warning | An engine shader the linked game does not have. |
| `M106` | warning | A compiler shader the linked game does not have. |
| `M107` | error | A surface's game material the linked game does not have: the level build refuses it (a bone's: a warning). |
| `M108` | note | A dynamic model whose source has no bones: it gets one bone, `root`. |
| `M109` | error | A value outside what its key takes. |
| `M110` | error | `parts` together with `source`. |
| `M111` | warning | What the model cannot take: `rotate` on a model with bones, a part that makes no faces. |
| `M112` | error | A surface that wears no texture: no level build and no model of the game takes it. |

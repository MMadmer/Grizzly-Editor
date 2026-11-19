//! The reference of the scene's files, made from the same tables the reader and the writer use:
//! `docs/assets/SCENE.md` is this text, and `asset.reference kind=scene` answers it.

use super::fields::{self, Field, Ty};
use super::{CLASS_FILES, HEADER, SCENE_FIELDS, VERSION, layers};
use crate::class::ObjClass;

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

fn row<T>(out: &mut String, f: &Field<T>) {
    let ty = match f.ty {
        Ty::Number { .. } | Ty::Angle { .. } | Ty::Integer { .. } if !f.unit.is_empty() => {
            format!("{} ({})", f.ty.describe(), f.unit)
        }
        Ty::Vec3 | Ty::Range if !f.unit.is_empty() => format!("{} ({})", f.ty.describe(), f.unit),
        Ty::Angles => format!("{} (deg)", f.ty.describe()),
        _ => f.ty.describe(),
    };
    let default = if f.default.is_empty() { "-".to_owned() } else { f.default.to_owned() };
    out.push_str(&format!("| `{}` | {} | {} | {} |\n", f.key, esc(&ty), esc(&default), esc(f.help)));
}

fn table_head(out: &mut String) {
    out.push_str("| Key | Type | Default | Meaning |\n|---|---|---|---|\n");
}

/// The whole reference, as Markdown.
#[must_use]
pub fn reference() -> String {
    let mut out = String::new();
    out.push_str(
        "# The scene\n\n\
This file is generated from the editor's own tables of the scene's keys (`crates/gz-scene/src/format/`): \
the reader, the writer, the Details panel and this reference are one list, so they cannot drift apart. \
A test of `gz-scene` fails when this file is not what the tables say. The contract the format keeps is \
`docs/ASSETS.md` (sections 2, 3 and 7).\n\n\
## Layout\n\n\
A scene is `<name>.scene` and the folder `<name>\\` beside it:\n\n\
```text\n\
escape.scene        the scene: header, level options, build settings, the sun, the camera, the kinds' eyes\n\
escape\\\n",
    );
    for cf in &CLASS_FILES {
        let lists: Vec<String> =
            cf.lists.iter().map(|(l, c)| format!("{l} ({})", c.label_plural())).collect();
        out.push_str(&format!("  {:<18}{}\n", cf.file, lists.join(", ")));
    }
    for l in layers::LAYERS {
        out.push_str(&format!("  {:<18}the {} layer (binary)\n", l.file, l.key));
    }
    out.push_str(&format!(
        "```\n\n\
Every file is Lua data (`return {{ ... }}` of literals). The `.scene` starts with the header `{HEADER} = {VERSION}`; \
the files of the folder are parts of the scene and carry no header. A file of a kind the scene has no objects of \
does not exist. A key at its default is not written, and a key the editor does not know is kept where it is \
through every save. A save writes only what changed: an object nobody touched keeps its bytes and its comments.\n\n\
Positions are metres, angles degrees, colours `{{ r, g, b }}` from 0 to 1. Things are named, never numbered: \
an object's `name` is unique in the scene (in any case; the case written is kept), and whatever points at an \
object - a group's members, a sector's meshes, a portal's sectors, a way's links - names it. A number is the \
shortest text that reads back to the very value; the few values a number of the file cannot be are words: \
`\"-0\"`, `\"nan\"`, `\"inf\"`, `\"-inf\"`.\n\n\
## The .scene\n\n"
    ));
    table_head(&mut out);
    for f in SCENE_FIELDS {
        row(&mut out, f);
    }
    out.push_str(
        "\n## A scene over a level of the game\n\n\
A module that adds to a level of the game says `base = \"<level>\"` and writes only what it adds:\n\n\
```lua\n\
-- rawdata/levels/agroprom_base.scene\n\
return {\n\
  scene = 2,\n\
  base = \"l03_agroprom\",\n\
}\n\
-- rawdata/levels/agroprom_base/spawns.lua\n\
return {\n\
  spawns = {\n\
    { name = \"agr_smart_terrain_1_2_anomal_zone\", removed = true },\n\
    { name = \"base_guard_gate\", pos = { 40, 2.5, -12 }, rot = { 0, 90, 0 }, section = \"agro_base_guard\" },\n\
  },\n\
}\n\
```\n\n\
The level itself is the game's and is never copied into the project: as the scene is read its objects come from \
the game library (`docs/GAME_LIBRARY.md`) - the level's geometry as one object (`level_geometry`, with the level's \
own collision), every spawn point of the game's spawn that stands on it, its patrol paths, lights, glows, sounds \
and effects, fog volumes and sound environments, and its AI map. They stand in the scene for every query, check and picture; `scene.objects` answers \
them with `level = true` (`own = true` leaves them out), and nothing of them is the scene's to change. What the \
library holds of the level - the same objects as text, and the level's passport: its structures, floors, materials \
and heights - is `asset.card ref=<level> kind=level`.\n\n\
- Everything the scene's files hold is the module's: `authored` is implied and not written.\n\
- An entry of `spawns.lua` with `removed = true` and no `section` is no spawn point of the scene's: it takes the \
level's spawn point of that `name` out of the game for the module's mode. It says `name` and `removed` alone. An \
entry that names what the level has not is a warning (`S061`) and stays in the file.\n\
- A key of the `.scene` left out is the level's: its folder, its sun, its light controls.\n\
- The level's painted layers are the level's; a layer file in the scene's folder is the module's own, made \
when the scene paints it. Wallmarks painted so ship as part of the module's piece of the level; details \
(grass) and a painted AI map do not ship - the game grows the level's own grass and keeps the level's own \
AI map, which a module changes under a terrain that cuts the ground alone (`S223`).\n\
- What the module adds beside objects and spawn points - sound sources, particle effects, lights that shine \
in the game, environment modifiers, glows, fog volumes, sound environments - the build writes into \
`levels\\<level>\\overlay.things`, which the game puts beside the level's own. Sectors, portals, the bound of \
the level and of its AI map are the game's there (`S222`).\n\
- A level the game library cannot give (`S060`: no game linked, no level of that name) leaves the scene with what \
its own files say; a save then writes them as they are.\n\n\
A scene without `base` is a level of the module's own, or an X-Ray SDK scene imported whole, where `authored` and \
`removed` are said object by object.\n\
\n## Every object\n\n\
Each list of a folder's file holds one table per object, on its own line or block, `name` first:\n\n\
```lua\n\
return {\n\
  objects = {\n\
    { name = \"box_wood_01\", ref = \"props\\\\box_wood_01\", pos = { 1, 0, 2 }, rot = { 0, 90, 0 } },\n\
  },\n\
}\n\
```\n\n\
These keys belong to objects of every kind (the transform only to the kinds that have it):\n\n",
    );
    table_head(&mut out);
    for f in fields::COMMON {
        row(&mut out, f);
    }
    for cf in &CLASS_FILES {
        for (list, class) in cf.lists {
            out.push_str(&format!("\n## {} - `{}` of `{}`\n\n", class.label_plural(), list, cf.file));
            out.push_str(&format!("{}\n\n", kind_text(*class)));
            table_head(&mut out);
            for f in fields::class_fields(*class) {
                row(&mut out, f);
            }
        }
    }
    out.push_str(
        "\n## Spawn properties\n\n\
`props` holds the named properties of the spawn's entity that differ from a new entity of its section - the \
fields of the game's server entity by name: the object's own (`story_id`, `spawn_story_id`, `custom_data = [[ ... ]]`, \
`graph_id`, `node_id`, `distance`, `alife_flags`), its model (`visual`, `visual_flags`, `startup_animation`), and a \
table per part of its class (`trader = { money = 500, specific_character = \"...\" }`, `creature = { health = 0.5 }`, \
`zone = { ... }`, `script = { ... }`, ...). `header` holds what the entity's header says beyond the object's name \
and transform. A part the entity lacks is `false`. The editor proves every save: the properties read back must give \
the very spawn packet the entity writes, or the spawn is kept as its packet (`raw`, hex) and `asset.check` names it. \
An entity of an X-Ray SDK text scene whose class the linked game does not know has no packet: it keeps the \
section's numbered values as they were (`values`).\n\n\
A zone's volume is said once, by the point's `shape`: `shape = { shapes = { { kind = \"box\", size = { 20, 6, 12 } } } }`. \
The entity carries the same shapes and `props` does not repeat them; a file that says `props.shapes` and no `shape` \
is read into the point's shape.\n\n\
A section is one of the linked game's (`game.lookup type=spawn_section`) or one the game makes of the module's own \
assets: a character `characters/guard.npcasset` of the module `agro_base` is the section `agro_base_guard`, placed \
like any other.\n\n\
## The painted layers\n\n\
`ai_map.bin`, `details.bin` and `wallmarks.bin` start with the magic `GZLAYER\\0`, the layer format's version and \
the layer's kind (little-endian 32-bit words), then the layer's data as the editor holds it.\n\n",
    );
    out.push_str(crate::bulk::ground::REFERENCE);
    out.push_str(crate::objtools::zones::REFERENCE);
    out.push_str(
        "## Object groups (.group)\n\n\
A `.group` asset is a set of objects saved for reuse: the header `group = 1`, then the same lists as the scene's \
folder files (`objects`, `lights`, `spawns`, ...) in one file, positions relative to the group's pivot. Placing \
it puts copies of its objects into the scene as one group.\n\n\
## The X-Ray SDK scene\n\n\
A `.level` with its folder of `.part` files is the X-Ray SDK's scene: the editor reads it and never writes it. \
Opening one converts it in memory; saving writes the `.scene` beside it. Its scene-wide Snap Objects list is \
not kept: placing and the wallmarks work on the whole level, and the AI map's and the detail objects' own lists \
stay with their layers.\n",
    );
    out
}

fn kind_text(class: ObjClass) -> &'static str {
    match class {
        ObjClass::SceneObject => "Static and dynamic objects of the library.",
        ObjClass::Light => {
            "The lights the build bakes and the game shines. Every kind is baked into the lightmaps (`lightmap`): a point light all around, a spot light inside its `cone` along where it looks (all of it on the axis, none at the cone's edge), a directional one from its direction. In the game (`dynamic`) a point light and a spot light shine - the spot through the level's `overlay.things` -; a directional one never does, the game's one directional light being the level's sun (`S221`)."
        }
        ObjClass::Glow => {
            "Glows: a picture that always faces the viewer, at a place - the halo of a lamp. The game draws the ones a module brings (the level's `overlay.things`), fading each in while its place is in sight."
        }
        ObjClass::SoundSrc => "Sound sources: a sound of the game played at a place.",
        ObjClass::SoundEnv => {
            "Sound environments: boxes that change how sound echoes. `pos`, `rot` and `scale` say the box (a unit box scaled: `scale = { 12, 4, 7 }` is 12 x 4 x 7 m). Inside it the game plays `inner`; straight above it, `outer`. `asset.find kind=sound_env` lists the game's environments. They reach the game through the level's `overlay.things`, on a level of the module's own and over a level of the game alike."
        }
        ObjClass::Particles => "Particle effects placed in the level.",
        ObjClass::Shape => "Shapes: volumes of spheres and boxes (and the level's bound).",
        ObjClass::FogVolume => {
            "Fog volumes: one box each (`pos`, `rot`, `scale`: a unit box scaled). An emitter's box fills with fog that flows by its `profile`; the occlusion volumes of its `group` are what the fog flows around. A group has one emitter. The fog fills its whole box and is lit by the lights around it: a small box in a still place reads as mist, a large one as smoke. The game shows it with volumetric fog on."
        }
        ObjClass::SpawnPoint => "Spawn points: what the game spawns, and the editor's own points.",
        ObjClass::Way => "Way paths: named points and the links between them; orders and quests walk them.",
        ObjClass::Sector => {
            "Sectors: the cells the visibility of a level of the module's own is divided into. Over a level of the game the cells are the game's and a module adds none (`S222`)."
        }
        ObjClass::Portal => {
            "Portals: the openings between two sectors of a level of the module's own (`S222` over a level of the game)."
        }
        ObjClass::Group => "Groups: objects held together; the members stay in their own files.",
        ObjClass::Terrain => {
            "Terrains: a height grid over a rectangle that replaces the ground under it. `pos` is its centre, `rot` turns it by its heading alone. Its model is made from these keys (and, on the ground, from the static objects under it that the module does not add); with `cut` on, the ground it replaces is cut out of the level wherever the editor and the game look: a triangle of a static object that is not `authored` is clipped by the rectangle, and the part inside goes where it lies within 0.5 m of the old ground."
        }
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => "",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_reference_document_is_what_the_tables_say() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/assets/SCENE.md");
        let written = std::fs::read_to_string(&file).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            written == super::reference(),
            "docs/assets/SCENE.md is not what the scene's tables say: the command `asset.reference kind=scene` answers it"
        );
    }

    #[test]
    fn every_key_has_its_meaning() {
        let text = super::reference();
        for f in super::SCENE_FIELDS {
            assert!(!f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)), "{}", f.key);
        }
        for c in crate::class::ObjClass::ALL {
            for f in super::fields::class_fields(c) {
                assert!(!f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)), "{c:?} {}", f.key);
            }
        }
    }
}

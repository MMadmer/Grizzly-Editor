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
        "\n## Every object\n\n\
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
        ObjClass::Light => "The lights the build bakes and the game shines.",
        ObjClass::Glow => "Glows: light halos the game draws.",
        ObjClass::SoundSrc => "Sound sources: a sound of the game played at a place.",
        ObjClass::SoundEnv => "Sound environments: volumes that change how sound echoes.",
        ObjClass::Particles => "Particle effects placed in the level.",
        ObjClass::Shape => "Shapes: volumes of spheres and boxes (and the level's bound).",
        ObjClass::FogVolume => "Fog volumes: one box each.",
        ObjClass::SpawnPoint => "Spawn points: what the game spawns, and the editor's own points.",
        ObjClass::Way => "Way paths: named points and the links between them; orders and quests walk them.",
        ObjClass::Sector => "Sectors: the cells the level's visibility is divided into.",
        ObjClass::Portal => "Portals: the openings between two sectors.",
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

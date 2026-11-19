//! The lists of the game that are one file each: particle effects, game materials and shaders,
//! light animations, sound environments (`docs/GAME_LIBRARY.md`, 3.4, 3.6, 3.7).

use gz_fmt_xr::{
    GameMaterial, GameMaterials, LightAnims, Particles, Shaders, ShadersXrlc, SoundEnvironments,
};
use serde_json::{Map, Value, json};

/// The versions of what these lists state.
pub const PARTICLES_VERSION: u32 = 1;
pub const MATERIALS_VERSION: u32 = 2;
pub const LIGHTANIMS_VERSION: u32 = 1;
pub const SOUND_ENVS_VERSION: u32 = 1;

fn round(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

fn words(flags: u32, table: &[(u32, &'static str)]) -> Vec<&'static str> {
    table.iter().filter(|(bit, _)| flags & bit != 0).map(|(_, w)| *w).collect()
}

/// A particle effect's flags, in words.
const EFFECT_FLAGS: [(u32, &str); 12] = [
    (1 << 0, "sprite"),
    (1 << 10, "framed"),
    (1 << 11, "animated"),
    (1 << 12, "random_frame"),
    (1 << 13, "random_playback"),
    (1 << 14, "time_limit"),
    (1 << 15, "align_to_path"),
    (1 << 16, "collision"),
    (1 << 17, "dies_on_collision"),
    (1 << 18, "velocity_scale"),
    (1 << 19, "collides_with_dynamic"),
    (1 << 20, "world_align"),
];

/// `particles.lua`: every effect and group, sorted by name. `extra` are loose effect and group
/// files that came beside the library, read already.
#[must_use]
pub fn particles(lib: &Particles) -> Value {
    let mut effects: Vec<Value> = lib
        .effects
        .iter()
        .map(|e| {
            let mut row = Map::new();
            row.insert("name".into(), json!(e.name.as_str()));
            row.insert("max_particles".into(), json!(e.max_particles));
            if !e.texture.is_empty() {
                row.insert("texture".into(), json!(e.texture.as_str()));
            }
            if !e.shader.is_empty() {
                row.insert("shader".into(), json!(e.shader.as_str()));
            }
            if let Some(t) = e.time_limit {
                row.insert("time_limit".into(), json!(round(t)));
            }
            if let Some(f) = &e.frame {
                row.insert("frames".into(), json!(f.frame_count));
            }
            row.insert("flags".into(), json!(words(e.flags, &EFFECT_FLAGS)));
            row.insert("actions".into(), json!(e.actions.len()));
            Value::Object(row)
        })
        .collect();
    effects.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let mut groups: Vec<Value> = lib
        .groups
        .iter()
        .map(|g| {
            json!({
                "name": g.name.as_str(),
                "time_limit": round(g.time_limit),
                "effects": g.effects.iter().map(|e| {
                    let mut row = Map::new();
                    row.insert("effect".into(), json!(e.effect.as_str()));
                    row.insert("from".into(), json!(round(e.time0)));
                    row.insert("to".into(), json!(round(e.time1)));
                    for (key, child) in [("on_play", &e.on_play_child), ("on_birth", &e.on_birth_child), ("on_dead", &e.on_dead_child)] {
                        if !child.is_empty() {
                            row.insert(key.into(), json!(child.as_str()));
                        }
                    }
                    Value::Object(row)
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    groups.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({ "card": PARTICLES_VERSION, "kind": "particles", "groups": groups, "effects": effects })
}

/// A game material's flags, in words: what a level designer asks of a surface.
const MATERIAL_FLAGS: [(u32, &str); 15] = [
    (1 << 0, "breakable"),
    (1 << 2, "bounceable"),
    (1 << 3, "skidmark"),
    (1 << 4, "bloodmark"),
    (1 << 5, "climable"),
    (1 << 7, "passable"),
    (1 << 8, "dynamic"),
    (1 << 9, "liquid"),
    (1 << 10, "no_shadows"),
    (1 << 11, "no_wallmarks"),
    (1 << 12, "actor_obstacle"),
    (1 << 13, "no_ricochet"),
    (1 << 28, "injurious"),
    (1 << 29, "shootable"),
    (1 << 30, "transparent"),
];

fn material_row(m: &GameMaterial) -> Value {
    let mut row = Map::new();
    row.insert("name".into(), json!(m.name.as_str()));
    row.insert("flags".into(), json!(words(m.flags, &MATERIAL_FLAGS)));
    row.insert("friction".into(), json!(round(m.friction)));
    row.insert("shoot_factor".into(), json!(round(m.shoot_factor)));
    row.insert("sound_occlusion".into(), json!(round(m.snd_occlusion_factor)));
    row.insert("vis_transparency".into(), json!(round(m.vis_transparency_factor)));
    if let Some(d) = m.density_factor {
        row.insert("density".into(), json!(round(d)));
    }
    if !m.desc.is_empty() {
        row.insert("note".into(), json!(m.desc.as_str()));
    }
    Value::Object(row)
}

/// `materials.lua`: the game materials, the compiler shaders and the engine shaders' names.
#[must_use]
pub fn materials(
    mtl: Option<&GameMaterials>,
    xrlc: Option<&ShadersXrlc>,
    shaders: Option<&Shaders>,
) -> Value {
    let mut materials: Vec<Value> =
        mtl.map(|l| l.materials.iter().map(material_row).collect()).unwrap_or_default();
    materials.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let mut compilers: Vec<Value> = xrlc
        .map(|l| {
            l.shaders
                .iter()
                .map(|s| {
                    json!({
                        "name": s.name(),
                        "collides": s.flags & gz_fmt_xr::ShaderXrlc::COLLISION != 0,
                        "draws": s.flags & gz_fmt_xr::ShaderXrlc::RENDERING != 0,
                        "vertex_light": s.flags & gz_fmt_xr::ShaderXrlc::LIGHT_VERTEX != 0,
                        "casts_shadow": s.flags & gz_fmt_xr::ShaderXrlc::LIGHT_CAST_SHADOW != 0,
                        "lightmap_density": round(s.lm_density),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    compilers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    // an engine shader with what it is for: the geometry its class draws, how it treats the
    // texture's alpha, and - for a shader of models - the shader a level's geometry takes in its
    // place when a model is compiled as a static object
    let mut engine: Vec<Value> = shaders
        .map(|l| {
            l.blenders
                .iter()
                .map(|b| {
                    let mut row = Map::new();
                    row.insert("name".into(), json!(b.name.as_str()));
                    row.insert("class".into(), json!(b.class_name().trim_end()));
                    row.insert("draws".into(), json!(b.draws().name()));
                    let (test, blend) = b.alpha();
                    if test || blend {
                        row.insert("alpha".into(), json!(if blend { "blend" } else { "test" }));
                    }
                    if let Some(twin) = l.level_twin(&b.name) {
                        row.insert("as_level".into(), json!(twin.name.as_str()));
                    }
                    Value::Object(row)
                })
                .collect()
        })
        .unwrap_or_default();
    engine.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({
        "card": MATERIALS_VERSION,
        "kind": "materials",
        "materials": materials,
        "compilers": compilers,
        "shaders": engine,
    })
}

/// `lightanims.lua`.
#[must_use]
pub fn lightanims(lib: &LightAnims) -> Value {
    let mut rows: Vec<Value> = lib
        .items
        .iter()
        .map(|a| {
            json!({
                "name": a.name.as_str(),
                "fps": round(a.fps),
                "frames": a.frame_count,
                "seconds": if a.fps > 0.0 { round(a.frame_count as f32 / a.fps) } else { 0.0 },
                "keys": a.keys.len(),
            })
        })
        .collect();
    rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({ "card": LIGHTANIMS_VERSION, "kind": "lightanims", "lightanims": rows })
}

/// `sound_envs.lua`.
#[must_use]
pub fn sound_envs(lib: &SoundEnvironments) -> Value {
    let mut rows: Vec<Value> = lib
        .environments
        .iter()
        .map(|e| {
            json!({
                "name": e.name.as_str(),
                "room": round(e.room),
                "decay_time": round(e.decay_time),
                "reverb": round(e.reverb),
                "reverb_delay": round(e.reverb_delay),
                "size": round(e.environment_size),
            })
        })
        .collect();
    rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({ "card": SOUND_ENVS_VERSION, "kind": "sound_envs", "sound_envs": rows })
}

/// The version of what `sections.lua` states.
pub const SECTIONS_VERSION: u32 = 2;

/// What a section spawns, in a word, by the class of its server entity.
fn spawn_kind(class: gz_entities::Class) -> String {
    if class.is_weapon() {
        return "weapon".to_owned();
    }
    if class.is_item() {
        return "item".to_owned();
    }
    // a stalker is a monster to the engine and a person to whoever places one
    if class.is_trader() {
        return "npc".to_owned();
    }
    if class.is_creature() {
        return "monster".to_owned();
    }
    if class.is_anomalous_zone() {
        return "anomaly".to_owned();
    }
    if class.is_restrictor() {
        return "restrictor".to_owned();
    }
    if class.is_zone() {
        return "zone".to_owned();
    }
    // the class's own name, in words
    let name = format!("{class:?}");
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `sections.lua`: every section of the game's configs that a spawn point may name (it says
/// `$spawn`), sorted by name: its place in the Spawn list, its class and what that spawns, the
/// model it wears, whether it carries a volume (`shape`) and walks the AI map.
#[must_use]
pub fn sections(ltx: &gz_ltx::Ltx) -> Value {
    let mut rows: Vec<(String, Value)> = gz_game::spawn::sections(ltx)
        .into_iter()
        .map(|s| {
            let mut row = Map::new();
            row.insert("name".into(), json!(s.section));
            row.insert("group".into(), json!(s.group));
            row.insert("class".into(), json!(s.class.trim()));
            if let Some(info) = gz_entities::class_by_clsid(&s.class) {
                row.insert("kind".into(), json!(spawn_kind(info.class)));
                if info.class.has_shape() {
                    row.insert("shape".into(), json!(true));
                }
                if info.class.is_creature() || info.class.is_trader() {
                    row.insert("walks".into(), json!(true));
                }
            }
            let section = s.section.to_ascii_lowercase();
            let word = |key: &str| {
                ltx.line_exist(&section, key)
                    .then(|| ltx.r_string(&section, key).unwrap_or("").trim().to_owned())
                    .filter(|v| !v.is_empty())
            };
            if let Some(visual) = word("visual") {
                let visual = visual.trim_end_matches(".ogf").replace('/', "\\");
                row.insert("visual".into(), json!(format!("{visual}.ogf")));
            }
            if let Some(title) = word("inv_name") {
                row.insert("title".into(), json!(title));
            }
            (s.section.to_lowercase(), Value::Object(row))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    json!({ "sections": rows.into_iter().map(|r| r.1).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_words() {
        assert_eq!(words(1 << 5 | 1 << 7, &MATERIAL_FLAGS), ["climable", "passable"]);
        assert_eq!(words(1 << 16 | 1, &EFFECT_FLAGS), ["sprite", "collision"]);
        let empty = materials(None, None, None);
        assert_eq!(empty["materials"], json!([]));
        assert_eq!(empty["kind"], "materials");
    }
}

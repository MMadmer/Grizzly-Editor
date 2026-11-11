//! The property rows: what the classes' `FillProp` lists for Details (the selection of the
//! target class) and Level Settings (the scene and the tools), with the C++ keys, labels, value
//! texts, limits and help, merged over a selection, and set from text the way
//! `UIPropertiesForm::SetValueText` sets them.

use gz_base::SmolStr;
use gz_entities::{PropContext, PropKind, PropValue};

use crate::apply::TransformEdit;
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Vec3;
use crate::model::{ObjData, Object, SceneState, co};
use crate::objects::{
    FV_EMITTER, LT_POINT, LT_SPOT, PointKind, SHAPE_COMMON, SO_USE_SURFACE, SpawnEntity, SurfaceOverride,
    env_flags, fcolor_get, light_flags,
};
use crate::tools::{LevelOptions, ToolOpts, ToolPart};

/// How a row is edited, with the C++ widget's limits.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Caption,
    Text,
    /// A name checked for uniqueness.
    Name,
    /// A pick from a list the editor fills (`smTexture`, `smObject`, ...).
    Choose(SmolStr),
    Bool,
    Flag,
    Float {
        min: f32,
        max: f32,
        dec: u8,
    },
    /// Radians, shown in degrees.
    Angle {
        min: f32,
        max: f32,
        dec: u8,
    },
    Int {
        min: i64,
        max: i64,
    },
    Token(Vec<(SmolStr, i64)>),
    Vector {
        min: f32,
        max: f32,
        dec: u8,
    },
    Angle3 {
        dec: u8,
    },
    /// `0xAARRGGBB`.
    Color,
    /// `Fcolor`.
    FColor,
    Button(Vec<SmolStr>),
    Time,
    Canvas,
}

/// A row's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    None,
    Text(String),
    Bool(bool),
    Num(f64),
    Vec3(Vec3),
    Color(u32),
    FColor([f32; 4]),
}

/// One row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: String,
    pub kind: Kind,
    pub value: Val,
    pub enabled: bool,
    pub help: String,
}

impl Row {
    fn new(key: impl Into<String>, kind: Kind, value: Val) -> Self {
        Self { key: key.into(), kind, value, enabled: true, help: String::new() }
    }

    fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// `GetDrawText`.
    #[must_use]
    pub fn text(&self) -> String {
        match (&self.kind, &self.value) {
            (Kind::Float { dec, .. }, Val::Num(v)) => format!("{:.*}", *dec as usize, *v),
            (Kind::Angle { dec, .. }, Val::Num(v)) => format!("{:.*}", *dec as usize, v.to_degrees()),
            (Kind::Int { .. } | Kind::Color, Val::Num(v)) => format!("{}", *v as i64),
            (Kind::Color, Val::Color(c)) => format!("{}", *c as i32),
            (Kind::Time, Val::Num(v)) => {
                let s = *v as i64;
                format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
            }
            (Kind::Token(items), Val::Num(v)) => {
                items.iter().find(|(_, id)| *id == *v as i64).map(|(n, _)| n.to_string()).unwrap_or_default()
            }
            (Kind::Vector { dec, .. }, Val::Vec3(v)) => vec_text(*v, *dec),
            (Kind::Angle3 { dec }, Val::Vec3(v)) => {
                vec_text(Vec3::new(v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees()), *dec)
            }
            (Kind::Button(c), _) => c.iter().map(SmolStr::as_str).collect::<Vec<_>>().join(","),
            (Kind::Bool | Kind::Flag | Kind::Canvas | Kind::FColor, _) => String::new(),
            (_, Val::Text(t)) => t.clone(),
            _ => String::new(),
        }
    }
}

fn vec_text(v: Vec3, dec: u8) -> String {
    let d = dec as usize;
    format!("{{{:.d$}, {:.d$}, {:.d$}}}", v.x, v.y, v.z)
}

fn key(pref: &str, rest: &str) -> String {
    if pref.is_empty() { rest.to_owned() } else { format!("{pref}\\{rest}") }
}

fn float(k: String, v: f32, min: f32, max: f32, dec: u8) -> Row {
    Row::new(k, Kind::Float { min, max, dec }, Val::Num(f64::from(v)))
}

fn flag(k: String, word: u32, bit: u32) -> Row {
    Row::new(k, Kind::Flag, Val::Bool(word & bit != 0))
}

fn text(k: String, t: &str) -> Row {
    Row::new(k, Kind::Text, Val::Text(t.to_owned()))
}

fn choose(k: String, list: &str, t: &str) -> Row {
    Row::new(k, Kind::Choose(SmolStr::new(list)), Val::Text(t.to_owned()))
}

fn caption(k: String, t: &str) -> Row {
    Row::new(k, Kind::Caption, Val::Text(t.to_owned()))
}

fn int(k: String, v: i64, min: i64, max: i64) -> Row {
    Row::new(k, Kind::Int { min, max }, Val::Num(v as f64))
}

fn token(k: String, v: i64, items: &[(&str, i64)]) -> Row {
    Row::new(k, Kind::Token(items.iter().map(|(n, i)| (SmolStr::new(n), *i)).collect()), Val::Num(v as f64))
}

fn button(k: String, captions: &str) -> Row {
    Row::new(k, Kind::Button(captions.split(',').map(SmolStr::new).collect()), Val::None)
}

/// What the rows read beside the scene.
pub struct PropCx<'a> {
    pub lib: Option<&'a Library>,
    pub entity: &'a PropContext,
}

/// `CCustomObject::FillProp`.
fn custom_rows(pref: &str, o: &Object, out: &mut Vec<Row>) {
    out.push(Row::new(key(pref, "Name"), Kind::Name, Val::Text(o.name.to_string())));
    out.push(Row::new(
        key(pref, "Transform\\Position"),
        Kind::Vector { min: -10000.0, max: 10000.0, dec: 2 },
        Val::Vec3(o.position()),
    ));
    out.push(Row::new(key(pref, "Transform\\Rotation"), Kind::Angle3 { dec: 1 }, Val::Vec3(o.rotation())));
    out.push(Row::new(
        key(pref, "Transform\\Scale"),
        Kind::Vector { min: 0.01, max: 10000.0, dec: 2 },
        Val::Vec3(o.scale()),
    ));
    if !o.marks.authored && o.class == ObjClass::SpawnPoint {
        out.push(Row::new(key(pref, "Mod\\Removed from the game"), Kind::Flag, Val::Bool(o.marks.removed)));
    }
    if o.flags.co & co::IN_GROUP != 0 {
        out.push(flag(key(pref, "In group editable"), o.flags.co, co::IN_GROUP_UNIQUE));
    }
}

/// `CEditShape::FillProp`.
fn shape_rows(pref: &str, o: &Object, usage: u8, out: &mut Vec<Row>) {
    custom_rows(pref, o, out);
    out.push(caption(key(pref, "Shape usage"), if usage == SHAPE_COMMON { "common" } else { "level bound" }));
}

const LIGHT_TYPES: &[(&str, i64)] = &[("Point", LT_POINT as i64)];
const FUZZY_SHAPES: &[(&str, i64)] = &[("Sphere", 0), ("Box", 1)];
const FOG_TYPES: &[(&str, i64)] = &[("Emitter", 0), ("Occlusion", 1)];
const LEVEL_SUB_MAP: &[(&str, i64)] = &[("default", 255), ("#0", 0), ("#1", 1), ("#2", 2), ("#3", 3)];
const ORDER_KINDS: &[(&str, i64)] = &[("None", 0), ("Guard", 1), ("Patrol", 2), ("Wander", 3), ("Follow", 4)];
const RPOINT_TYPES: &[(&str, i64)] = &[("Actor Spawn", 0), ("Artefact Spawn", 1), ("Item Spawn", 2)];
const JITTER: &[(&str, i64)] = &[("1 - Low", 1), ("4 - Medium", 4), ("9 - High", 9)];

/// `CCustomObject::AnimationFillProp`.
fn motion_rows(pref: &str, o: &Object, out: &mut Vec<Row>) {
    let flags = o.co_flags();
    out.push(flag(key(pref, "Flags\\Motionable"), flags, co::MOTION));
    if flags & co::MOTION == 0 {
        return;
    }
    let Some(raw) = o.motion.as_ref() else { return };
    let Some(m) = raw.model() else {
        out.push(caption(key(pref, "Motion\\Hint"), "The motion has a version this editor does not read."));
        return;
    };
    let p = crate::objmotion::params(&m, raw);
    out.push(caption(key(pref, "Motion\\Hint"), "Make KEY only on Parent CS"));
    out.push(button(key(pref, "Motion\\Files"), "Import,Export"));
    out.push(button(
        key(pref, "Motion\\Commands"),
        "Add Key,Delete Key,Scale Keys...,Normalize Keys...,Clamp",
    ));
    out.push(button(
        key(pref, "Motion\\Controls"),
        "First Frame,Previous Key,Previous Frame,Play,Pause,Next Frame,Next Key,Last Frame",
    ));
    out.push(flag(key(pref, "Motion\\Flags\\Auto Key"), flags, crate::objmotion::AUTO_KEY));
    out.push(flag(key(pref, "Motion\\Flags\\Camera View"), flags, crate::objmotion::CAMERA_VIEW));
    out.push(float(key(pref, "Motion\\Start Frame (sec)"), p.min_t, -10000.0, p.max_t, 3));
    out.push(float(key(pref, "Motion\\End Frame (sec)"), p.max_t, p.min_t, 10000.0, 3));
    out.push(float(key(pref, "Motion\\Current Frame (sec)"), p.t_current, -10000.0, 10000.0, 3));
    out.push(float(key(pref, "Motion\\Key Time (sec)"), p.t_current, -10000.0, 10000.0, 3));
    out.push(caption(key(pref, "Motion\\Key Count"), &m.envelopes[0].keys.len().to_string()));
    out.push(caption(key(pref, "Motion\\Length (sec)"), &format!("{:.2}", crate::objmotion::length(&m).0)));
}

/// One object's rows, as `FillProp` of its class gives them (`pref` is `""` in Details).
#[must_use]
pub fn object_rows(state: &SceneState, o: &Object, cx: &PropCx<'_>) -> Vec<Row> {
    let pref = "";
    let mut out = Vec::new();
    match &o.data {
        ObjData::SceneObject(d) => {
            custom_rows(pref, o, &mut out);
            out.push(choose(key(pref, "Reference"), "object", &d.reference));
            let surfaces: Vec<SurfaceOverride> = if d.flags & SO_USE_SURFACE != 0 {
                d.surfaces.clone()
            } else {
                library_surfaces(cx.lib, &d.reference)
            };
            for s in &surfaces {
                if s.game_mtl == "materials\\occ" {
                    continue;
                }
                let p = key(&key(pref, "Surfaces"), &s.name);
                out.push(choose(key(&p, "Texture"), "texture", &s.texture));
                out.push(choose(key(&p, "Shader"), "e_shader", &s.shader));
                out.push(choose(key(&p, "Compile"), "c_shader", &s.shader_xrlc));
                out.push(choose(key(&p, "Game Mtl"), "game_material", &s.game_mtl));
            }
            out.push(button(key(&key(pref, "Surfaces"), "Action"), "Clear"));
            // `CSceneObject::FillProp`: a dynamic object moves along a motion
            if cx.lib.and_then(|l| l.cached(&d.reference)).is_some_and(|l| l.object.is_dynamic()) {
                motion_rows(pref, o, &mut out);
            }
        }
        ObjData::Light(d) => {
            custom_rows(pref, o, &mut out);
            out.push(token(key(pref, "Type"), i64::from(d.kind), LIGHT_TYPES));
            out.push(Row::new(key(pref, "Color"), Kind::FColor, Val::FColor(d.color)));
            out.push(float(key(pref, "Brightness"), d.brightness, -3.0, 3.0, 2));
            let controls: Vec<(SmolStr, i64)> = state
                .tools
                .part(ObjClass::Light)
                .light()
                .map(|l| l.lcontrols.iter().map(|(n, i)| (n.clone(), i64::from(*i))).collect())
                .unwrap_or_default();
            out.push(Row::new(
                key(pref, "Light Control"),
                Kind::Token(controls),
                Val::Num(f64::from(d.lcontrol)),
            ));
            out.push(flag(key(pref, "Usage\\LightMap"), d.flags, light_flags::AFFECT_STATIC));
            out.push(flag(key(pref, "Usage\\Dynamic"), d.flags, light_flags::AFFECT_DYNAMIC));
            out.push(flag(key(pref, "Usage\\Animated"), d.flags, light_flags::PROCEDURAL));
            out.push(float(key(pref, "Range"), d.range, 0.1, 1000.0, 2));
            out.push(float(key(pref, "Attenuation\\Constant"), d.attenuation[0], 0.0, 1.0, 6));
            out.push(float(key(pref, "Attenuation\\Linear"), d.attenuation[1], 0.0, 1.0, 6));
            out.push(float(key(pref, "Attenuation\\Quadratic"), d.attenuation[2], 0.0, 1.0, 6));
            out.push(button(key(pref, "Attenuation\\Auto"), "Linear,Quadratic"));
            out.push(Row::new(key(pref, "Attenuation\\Graphic"), Kind::Canvas, Val::None));
            if d.kind == LT_SPOT {
                out.push(Row::new(
                    key(pref, "Spot R1\\Cone Angle"),
                    Kind::Angle { min: 0.1, max: 120f32.to_radians(), dec: 2 },
                    Val::Num(f64::from(d.cone)),
                ));
                out.push(choose(key(pref, "Spot R1\\Texture"), "texture", &d.falloff_tex));
            } else {
                out.push(flag(key(pref, "Fuzzy"), d.flags, light_flags::POINT_FUZZY));
                if d.flags & light_flags::POINT_FUZZY != 0
                    && let Some(f) = &d.fuzzy
                {
                    out.push(int(key(pref, "Fuzzy\\Count"), f.points.len() as i64, 1, 100));
                    out.push(button(key(pref, "Fuzzy\\Generate"), "Random"));
                    out.push(token(key(pref, "Fuzzy\\Shape"), i64::from(f.shape), FUZZY_SHAPES));
                    if f.shape == 0 {
                        out.push(float(key(pref, "Fuzzy\\Radius"), f.sphere_radius, 0.01, 100.0, 2));
                    } else {
                        out.push(Row::new(
                            key(pref, "Fuzzy\\Half Dimension"),
                            Kind::Vector { min: 0.01, max: 100.0, dec: 2 },
                            Val::Vec3(f.box_dim),
                        ));
                    }
                }
            }
            out.push(Row::new(key(pref, "Use In D3D"), Kind::Bool, Val::Bool(d.use_in_d3d)));
        }
        ObjData::Glow(d) => {
            custom_rows(pref, o, &mut out);
            out.push(choose(key(pref, "Texture"), "texture", &d.texture));
            out.push(choose(key(pref, "Shader"), "e_shader", &d.shader));
            out.push(float(key(pref, "Radius"), d.radius, 0.01, 10000.0, 2));
        }
        ObjData::Shape(d) => shape_rows(pref, o, d.usage, &mut out),
        // oracle: `EFogVolume` lists its rows in `FillPropObjects`, which nothing calls, so its
        // Details show the shape's rows
        ObjData::FogVolume(d) => shape_rows(pref, o, d.shape.usage, &mut out),
        ObjData::SoundEnv(d) => {
            shape_rows(pref, o, d.shape.usage, &mut out);
            out.push(choose(key(pref, "Environment Inner"), "sound_env", &d.inner));
            out.push(choose(key(pref, "Environment Outer"), "sound_env", &d.outer));
        }
        ObjData::SoundSrc(d) => {
            custom_rows(pref, o, &mut out);
            out.push(button(key(pref, "Custom\\Controls"), "Play,Stop,Simulate"));
            out.push(choose(key(pref, "Source\\WAVE name"), "sound_source", &d.wav));
            out.push(float(key(pref, "Source\\Frequency"), d.freq, 0.0, 2.0, 2));
            out.push(float(key(pref, "Source\\Volume"), d.volume, 0.0, 1.0, 2));
            out.push(float(key(pref, "Source\\Min dist"), d.min_distance, 0.1, 1000.0, 1).disabled());
            out.push(float(key(pref, "Source\\Max dist"), d.max_distance, 0.1, 1000.0, 1).disabled());
            out.push(float(key(pref, "Source\\Max ai dist"), d.max_ai_distance, 0.1, 1000.0, 1).disabled());
            for (group, hint, v) in [
                ("Active time", "Zero - play sound looped round the clock.", d.active_time),
                ("Play time", "Zero - play sound once.", d.play_time),
                ("Pause delta", "Zero - play sound looped.", d.random_pause),
            ] {
                out.push(caption(key(pref, &format!("Game\\{group}\\Hint")), hint));
                out.push(Row::new(
                    key(pref, &format!("Game\\{group}\\From")),
                    Kind::Time,
                    Val::Num(f64::from(v[0])),
                ));
                out.push(Row::new(
                    key(pref, &format!("Game\\{group}\\To")),
                    Kind::Time,
                    Val::Num(f64::from(v[1])),
                ));
            }
        }
        ObjData::Particles(d) => {
            custom_rows(pref, o, &mut out);
            out.push(choose(key(pref, "Reference"), "particles", &d.reference));
            out.push(button(key(pref, "Controls"), "Play,Stop"));
        }
        ObjData::Way(d) => {
            out.push(Row::new(key(pref, "Way Name"), Kind::Name, Val::Text(o.name.to_string())));
            let _ = d;
        }
        ObjData::Sector(d) => {
            custom_rows(pref, o, &mut out);
            out.push(Row::new(key(pref, "Color"), Kind::FColor, Val::FColor(d.color)));
            let objects = d.items.iter().map(|(o, _)| o).collect::<std::collections::BTreeSet<_>>().len();
            let sub = key(pref, &o.name);
            out.push(caption(key(&sub, "Contents\\Objects"), &objects.to_string()));
            out.push(caption(key(&sub, "Contents\\Meshes"), &d.items.len().to_string()));
            out.push(caption(key(&sub, "Contents\\Faces"), &sector_faces(state, cx.lib, d).to_string()));
            out.push(token(key(&sub, "Change LevelMap to"), i64::from(d.map_idx), LEVEL_SUB_MAP));
        }
        ObjData::Portal(_) | ObjData::Kept(_) => custom_rows(pref, o, &mut out),
        ObjData::Group(g) => {
            custom_rows(pref, o, &mut out);
            out.push(choose(key(pref, "Reference"), "group", &g.reference));
            out.push(button(key(pref, "FreezeObjects"), "Unique,Reference"));
            for m in g.members.iter().filter_map(|m| state.get(*m)) {
                out.push(flag(
                    format!("{} ({})", key(pref, &m.name), m.class.desc()),
                    m.flags.co,
                    co::IN_GROUP_UNIQUE,
                ));
            }
        }
        ObjData::Spawn(d) => {
            custom_rows(pref, o, &mut out);
            if let Some(e) = &d.entity {
                let section = e.section().to_owned();
                out.push(choose(
                    key(&key(pref, &section), "Profile (spawn section)"),
                    "spawn_section",
                    &section,
                ));
                match e {
                    SpawnEntity::Decoded(ent) => {
                        for p in ent.properties(cx.entity) {
                            out.push(entity_row(pref, &p));
                        }
                    }
                    SpawnEntity::RawLtx { .. } | SpawnEntity::RawPacket { .. } => {
                        out.push(
                            caption(
                                key(&key(pref, &section), "Entity"),
                                "kept as read: link the game to edit it",
                            )
                            .disabled(),
                        );
                    }
                }
                if state.meta.level.map_usage & ((1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)) != 0 {
                    out.push(flag(key(pref, "MP respawn"), u32::from(d.sd_flags), 1));
                }
                if d.can_take_order() {
                    order_rows(pref, &d.order, &mut out);
                }
            } else {
                match d.kind {
                    PointKind::PlayerStart => {
                        out.push(caption(
                            key(pref, "Player Start\\Starts"),
                            "a new game of the module's mode",
                        ));
                        out.push(caption(key(pref, "Player Start\\Facing"), "the arrow; rotate to turn it"));
                        out.push(caption(
                            key(pref, "Player Start\\Rules"),
                            "one per module - Mod > World Rules",
                        ));
                    }
                    PointKind::RPoint => {
                        if d.rpoint.kind == 2 {
                            out.push(choose(
                                key(pref, "Respawn Point\\Profile"),
                                "respawn_items",
                                &d.rpoint.profile,
                            ));
                        } else {
                            out.push(int(key(pref, "Respawn Point\\Team"), i64::from(d.rpoint.team), 0, 7));
                        }
                        out.push(token(
                            key(pref, "Respawn Point\\Spawn Type"),
                            i64::from(d.rpoint.kind),
                            RPOINT_TYPES,
                        ));
                    }
                    PointKind::EnvMod => {
                        let e = &d.env;
                        let p = key(pref, "Environment Modificator");
                        out.push(float(key(&p, "Radius"), e.radius, 0.0001, 10000.0, 2));
                        out.push(float(key(&p, "Power"), e.power, 0.000_01, 1000.0, 2));
                        let fl = u32::from(e.flags);
                        out.push(flag(key(&p, "View Distance"), fl, u32::from(env_flags::VIEW_DIST)));
                        if e.flags & env_flags::VIEW_DIST != 0 {
                            out.push(float(key(&p, "View Distance\\ "), e.view_dist, 0.0001, 10000.0, 2));
                        }
                        out.push(flag(key(&p, "Fog Color"), fl, u32::from(env_flags::FOG_COLOR)));
                        if e.flags & env_flags::FOG_COLOR != 0 {
                            out.push(Row::new(key(&p, "Fog Color\\ "), Kind::Color, Val::Color(e.fog_color)));
                        }
                        out.push(flag(key(&p, "Fog Density"), fl, u32::from(env_flags::FOG_DENSITY)));
                        if e.flags & env_flags::FOG_DENSITY != 0 {
                            out.push(float(key(&p, "Fog Density\\ "), e.fog_density, 0.0, 10000.0, 2));
                        }
                        for (name, bit, v) in [
                            ("Ambient Color", env_flags::AMBIENT_COLOR, e.ambient_color),
                            ("Sky Color", env_flags::SKY_COLOR, e.sky_color),
                            ("Hemi Color", env_flags::HEMI_COLOR, e.hemi_color),
                        ] {
                            out.push(flag(key(&p, name), fl, u32::from(bit)));
                            if e.flags & bit != 0 {
                                out.push(Row::new(
                                    key(&p, &format!("{name}\\ ")),
                                    Kind::Color,
                                    Val::Color(v),
                                ));
                            }
                        }
                    }
                    PointKind::Spawn => {}
                }
            }
        }
    }
    // `setEditable`: a read-only tool or a locked group member shows its rows disabled; the
    // fifth row of a member ("In group editable") stays usable
    let editable_tool = state.tools.part(o.class).editable();
    let in_group = o.flags.co & co::IN_GROUP != 0;
    let editable = editable_tool && o.editable();
    for (i, r) in out.iter_mut().enumerate() {
        if !(editable || (i == 4 && in_group)) {
            r.enabled = false;
        }
    }
    out
}

fn order_rows(pref: &str, o: &crate::objects::XmsOrder, out: &mut Vec<Row>) {
    out.push(token(key(pref, "Order\\Kind"), i64::from(o.kind), ORDER_KINDS));
    match o.kind {
        1 => {
            out.push(choose(key(pref, "Order\\Look path"), "way_path", &o.look));
            out.push(float(key(pref, "Order\\Radius"), o.radius, 1.0, 100.0, 0));
        }
        2 => {
            out.push(choose(key(pref, "Order\\Route"), "way_path", &o.route));
            out.push(flag(key(pref, "Order\\Run"), u32::from(o.flags), u32::from(crate::objects::ORDER_RUN)));
        }
        3 => {
            out.push(float(key(pref, "Order\\Radius"), o.radius, 3.0, 150.0, 0));
            out.push(flag(key(pref, "Order\\Run"), u32::from(o.flags), u32::from(crate::objects::ORDER_RUN)));
        }
        4 => {
            out.push(choose(key(pref, "Order\\Target"), "story_id", &o.target));
            out.push(float(key(pref, "Order\\Distance"), o.distance, 1.0, 30.0, 1));
        }
        _ => {}
    }
    out.push(flag(
        key(pref, "Order\\Hostile to player"),
        u32::from(o.flags),
        u32::from(crate::objects::ORDER_HOSTILE),
    ));
}

fn entity_row(pref: &str, p: &gz_entities::Prop) -> Row {
    let k = key(pref, &p.path);
    let mut r = match (&p.kind, &p.value) {
        (PropKind::U8 { min, max, .. }, PropValue::Int(v)) => int(k, *v, i64::from(*min), i64::from(*max)),
        (PropKind::U16 { min, max, .. }, PropValue::Int(v)) => int(k, *v, i64::from(*min), i64::from(*max)),
        (PropKind::U32 { min, max, .. }, PropValue::Int(v)) => int(k, *v, i64::from(*min), i64::from(*max)),
        (PropKind::Float { min, max, decimals, .. }, PropValue::Float(v)) => {
            float(k, *v, *min, *max, *decimals)
        }
        (PropKind::Angle { min, max }, PropValue::Float(v)) => {
            Row::new(k, Kind::Angle { min: *min, max: *max, dec: 2 }, Val::Num(f64::from(*v)))
        }
        (PropKind::Bool, PropValue::Bool(b)) => Row::new(k, Kind::Bool, Val::Bool(*b)),
        (PropKind::Flag { off, on, .. }, PropValue::Bool(b)) => {
            let caption = match (off, on) {
                (Some(off), Some(on)) => Some(if *b { *on } else { *off }),
                _ => None,
            };
            match caption {
                Some(c) => Row::new(k, Kind::Flag, Val::Text(c.to_owned())),
                None => Row::new(k, Kind::Flag, Val::Bool(*b)),
            }
        }
        (PropKind::Token { items, .. }, PropValue::Int(v)) => Row::new(
            k,
            Kind::Token(items.iter().map(|(n, i)| (SmolStr::new(n), i64::from(*i))).collect()),
            Val::Num(*v as f64),
        ),
        (PropKind::Vec3, PropValue::Vec3(v)) => {
            Row::new(k, Kind::Vector { min: 0.0, max: 0.0, dec: 2 }, Val::Vec3(Vec3::from(*v)))
        }
        (PropKind::Color, PropValue::Int(v)) => Row::new(k, Kind::Color, Val::Color(*v as u32)),
        (PropKind::Choose { choose: c, .. }, PropValue::Text(t)) => {
            choose(k, &format!("{c:?}").to_lowercase(), t)
        }
        (_, PropValue::Text(t)) => text(k, t),
        (_, PropValue::Int(v)) => int(k, *v, 0, 0),
        (_, PropValue::Float(v)) => float(k, *v, 0.0, 0.0, 2),
        (_, PropValue::Bool(b)) => Row::new(k, Kind::Bool, Val::Bool(*b)),
        (_, PropValue::Vec3(v)) => {
            Row::new(k, Kind::Vector { min: 0.0, max: 0.0, dec: 2 }, Val::Vec3(Vec3::from(*v)))
        }
    };
    if p.read_only {
        r.enabled = false;
    }
    r
}

fn library_surfaces(lib: Option<&Library>, reference: &str) -> Vec<SurfaceOverride> {
    lib.and_then(|l| l.cached(reference))
        .map(|lo| {
            lo.object
                .surfaces
                .iter()
                .map(|s| SurfaceOverride {
                    name: s.name.clone(),
                    shader: s.shader.clone(),
                    shader_xrlc: s.shader_xrlc.clone(),
                    game_mtl: s.game_mtl.clone(),
                    texture: s.texture.clone(),
                    vmap: s.vmap.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sector_faces(state: &SceneState, lib: Option<&Library>, d: &crate::objects::SectorData) -> usize {
    let Some(lib) = lib else { return 0 };
    d.items
        .iter()
        .filter_map(|(o, m)| {
            let obj = state.find(ObjClass::SceneObject, o)?;
            let ObjData::SceneObject(so) = &obj.data else { return None };
            let lo = lib.cached(&so.reference)?;
            lo.object.meshes.iter().find(|x| x.name.eq_ignore_ascii_case(m)).map(|x| x.faces.len())
        })
        .sum()
}

/// `EScene::FillProp`: Level Settings.
#[must_use]
pub fn world_rows(state: &SceneState) -> Vec<Row> {
    let lo = &state.meta.level;
    let b = &lo.build;
    let custom = b.quality == 2;
    let en = |r: Row| if custom { r } else { r.disabled() };
    let file = state.meta.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
    let mut out = vec![
        caption("Scene\\Name".into(), &file),
        text("Scene\\Name prefix".into(), &lo.level_prefix),
        text("Scene\\Build options\\Level path".into(), &lo.level_path),
        text("Scene\\Build options\\Custom data".into(), &lo.bop),
        text("Scene\\Map version".into(), &lo.map_version),
        button("Scene\\Build options\\Quality".into(), "Draft,High,Custom"),
        en(int(
            "Scene\\Build options\\Lighting\\Hemisphere quality [0-3]".into(),
            i64::from(lo.light_hemi_quality),
            0,
            3,
        )),
        en(int(
            "Scene\\Build options\\Lighting\\Sun shadow quality [0-3]".into(),
            i64::from(lo.light_sun_quality),
            0,
            3,
        )),
        en(float("Scene\\Build options\\Optimizing\\Normal smooth angle".into(), b.sm_angle, 0.0, 180.0, 2)),
        en(float("Scene\\Build options\\Optimizing\\Weld distance (m)".into(), b.weld_distance, 0.0, 1.0, 4)),
        en(float(
            "Scene\\Build options\\Lighting\\Pixel per meter".into(),
            b.lm_pixels_per_meter,
            0.0,
            20.0,
            2,
        )),
        en(int("Scene\\Build options\\Lighting\\Error (LM collapsing)".into(), i64::from(b.lm_rms), 0, 255)),
        en(int("Scene\\Build options\\Lighting\\Error (LM zero)".into(), i64::from(b.lm_rms_zero), 0, 255)),
        en(token(
            "Scene\\Build options\\Lighting\\Jitter samples".into(),
            i64::from(b.lm_jitter_samples),
            JITTER,
        )),
    ];
    // `CLevelPreferences::FillProp` Scene\Targets: each part loaded or not, locked or not
    let mut by_id = ObjClass::ALL.to_vec();
    by_id.sort_by_key(|c| c.id());
    for c in &by_id {
        let t = state.tools.part(*c);
        if *c != ObjClass::SceneObject {
            let mut r = Row::new(key("Scene\\Targets\\Enable", c.desc()), Kind::Flag, Val::Bool(t.enabled));
            r.help = "Loads this part of the scene; off unloads it.".into();
            out.push(r);
        }
        let mut r = Row::new(key("Scene\\Targets\\Read Only", c.desc()), Kind::Flag, Val::Bool(t.readonly));
        r.help = "This part of the scene can be looked at, but not changed or saved.".into();
        out.push(r);
    }
    for c in ObjClass::ALL {
        let t = state.tools.part(c);
        let p = c.desc();
        match c {
            ObjClass::Light => {
                if let Some(l) = t.light() {
                    out.push(flag(key(p, "Common\\Sun Shadow\\Visible"), t.flags, 1 << 31));
                    out.push(Row::new(
                        key(p, "Common\\Sun Shadow\\Altitude"),
                        Kind::Angle { min: -std::f32::consts::FRAC_PI_2, max: 0.0, dec: 2 },
                        Val::Num(f64::from(l.sun_shadow_dir[0])),
                    ));
                    out.push(Row::new(
                        key(p, "Common\\Sun Shadow\\Longitude"),
                        Kind::Angle { min: 0.0, max: std::f32::consts::TAU, dec: 2 },
                        Val::Num(f64::from(l.sun_shadow_dir[1])),
                    ));
                    out.push(flag(key(p, "Common\\Controls\\Draw Name"), t.flags, 1 << 30));
                    out.push(caption(key(p, "Common\\Controls\\Count"), &l.lcontrols.len().to_string()));
                    for (n, _) in &l.lcontrols {
                        if matches!(n.as_str(), "$hemi" | "$static" | "$sun") {
                            out.push(caption(key(p, &format!("Common\\Controls\\System\\{n}")), ""));
                        } else {
                            out.push(button(
                                key(p, &format!("Common\\Controls\\User\\{n}")),
                                "Rename,Remove",
                            ));
                        }
                    }
                }
            }
            ObjClass::Glow => {
                out.push(flag(key(p, "Common\\Test Visibility"), t.flags, 1 << 31));
                out.push(flag(key(p, "Common\\Draw Cross"), t.flags, 1 << 30));
            }
            ObjClass::Portal => out.push(flag(key(p, "Common\\Draw Simple Model"), t.flags, 1 << 31)),
            ObjClass::Sector => out.push(flag(key(p, "Common\\Draw Solid"), t.flags, 1 << 31)),
            ObjClass::AiMap => {
                if let Some(a) = &state.bulk.ai_map {
                    let h = &a.header;
                    out.push(Row::new(key(p, "Common\\Draw Nodes"), Kind::Flag, Val::Bool(h.flags & 2 == 0)));
                    out.push(flag(key(p, "Common\\Slow Calculate Mode"), h.flags, 4));
                    out.push(float(key(p, "Common\\Visible Radius"), h.vis_radius, 10.0, 250.0, 2));
                    out.push(float(key(p, "Common\\Smooth Height"), h.smooth_height, 0.1, 100.0, 2));
                    out.push(int(key(p, "Params\\Brush Size"), i64::from(h.brush_size), 1, 100));
                    out.push(float(key(p, "Params\\Can Up"), h.params.can_up, 0.0, 10.0, 2));
                    out.push(float(key(p, "Params\\Can Down"), h.params.can_down, 0.0, 10.0, 2));
                }
            }
            _ => {}
        }
    }
    out
}

/// A row of the merged list `property_inspector` answers.
#[derive(Clone, Debug, PartialEq)]
pub struct MergedRow {
    pub row: Row,
    pub mixed: bool,
}

/// `AssignItems`: rows of one key over several objects become one row, "mixed" when their
/// values differ; the order is the first object's, rows the first lacks follow.
#[must_use]
pub fn merge(lists: Vec<Vec<Row>>) -> Vec<MergedRow> {
    let mut out: Vec<MergedRow> = Vec::new();
    let mut index: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for list in lists {
        for r in list {
            match index.get(&r.key) {
                Some(&i) => {
                    if let Some(m) = out.get_mut(i) {
                        if m.row.value != r.value {
                            m.mixed = true;
                        }
                        m.row.enabled &= r.enabled;
                    }
                }
                None => {
                    index.insert(r.key.clone(), out.len());
                    out.push(MergedRow { row: r, mixed: false });
                }
            }
        }
    }
    for m in &mut out {
        // a mixed name cannot be given to all of them (`CreateNameCB`)
        if m.mixed && m.row.kind == Kind::Name {
            m.row.enabled = false;
        }
    }
    out
}

/// The units a key may end in, `(m)`, and how the value column writes them.
const KEY_UNITS: &[(&str, &str)] = &[
    ("m", "m"),
    ("cm", "cm"),
    ("km", "km"),
    ("m/s", "m/s"),
    ("s", "s"),
    ("sec", "s"),
    ("seconds", "s"),
    ("ms", "ms"),
    ("min", "min"),
    ("minutes", "min"),
    ("deg", "\u{b0}"),
    ("degrees", "\u{b0}"),
    ("%", "%"),
    ("px", "px"),
    ("fps", "fps"),
    ("kb", "KB"),
    ("mb", "MB"),
];

fn split_leaf(k: &str) -> (String, Option<&'static str>) {
    let mut label = k.rsplit('\\').next().unwrap_or(k).to_owned();
    if let Some(open) = label.rfind(" [")
        && open > 0
        && label.ends_with(']')
    {
        label.truncate(open);
    }
    if let Some(paren) = label.rfind(" (")
        && paren > 0
        && label.ends_with(')')
    {
        let written = &label[paren + 2..label.len() - 1];
        if let Some((_, u)) = KEY_UNITS.iter().find(|(w, _)| w.eq_ignore_ascii_case(written)) {
            label.truncate(paren);
            return (label, Some(u));
        }
    }
    (label, None)
}

/// `LabelOf`: the leaf without its range or unit.
#[must_use]
pub fn label_of(k: &str) -> String {
    split_leaf(k).0
}

/// `GetUnit`: a number's unit, from its key or the degree sign of an angle.
#[must_use]
pub fn unit_of(r: &Row) -> String {
    match r.kind {
        Kind::Float { .. } | Kind::Int { .. } | Kind::Angle { .. } | Kind::Time => {}
        _ => return String::new(),
    }
    if let Some(u) = split_leaf(&r.key).1 {
        return u.to_owned();
    }
    if matches!(r.kind, Kind::Angle { .. }) { "\u{b0}".to_owned() } else { String::new() }
}

/// `GetHelp`: the range a number takes, in its unit.
#[must_use]
pub fn help_of(r: &Row) -> String {
    let unit = unit_of(r);
    let suffix =
        if unit.is_empty() || unit == "\u{b0}" || unit == "%" { unit.clone() } else { format!(" {unit}") };
    let mut help = r.help.clone();
    let range = if let Some(open) = r.key.rfind(" [")
        && r.key.ends_with(']')
        && !r.key[open..].contains('\\')
    {
        format!("Range: {}{suffix}.", &r.key[open + 2..r.key.len() - 1])
    } else {
        match r.kind {
            Kind::Float { min, max, .. } if min < max => {
                format!("Range: {} to {}{suffix}.", crate::ltx::fmt_g(min), crate::ltx::fmt_g(max))
            }
            Kind::Angle { min, max, .. } if min < max => {
                format!(
                    "Range: {} to {}{suffix}.",
                    crate::ltx::fmt_g(min.to_degrees()),
                    crate::ltx::fmt_g(max.to_degrees())
                )
            }
            Kind::Int { min, max } if min < max => format!("Range: {min} to {max}{suffix}."),
            _ => String::new(),
        }
    };
    if !range.is_empty() {
        if !help.is_empty() {
            help.push(' ');
        }
        help.push_str(&range);
    }
    help
}

/// `FindItem`, then `FindItemOfName`: the whole key, its last part, or its label.
#[must_use]
pub fn find<'a>(rows: &'a [MergedRow], k: &str) -> Option<&'a MergedRow> {
    rows.iter()
        .find(|r| r.row.key == k)
        .or_else(|| rows.iter().find(|r| r.row.key.rsplit('\\').next() == Some(k)))
        .or_else(|| rows.iter().find(|r| !k.is_empty() && label_of(&r.row.key) == k))
}

/// A value read from text, checked against the row.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Num(f64),
    Bool(bool),
    Vec3(Vec3),
    Color {
        argb: u32,
        alpha: bool,
    },
    Text(String),
    /// A button: the index of its caption.
    Press(usize),
}

fn parse_bool(t: &str) -> Option<bool> {
    match t.to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" => Some(true),
        "false" | "0" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn parse_vec(t: &str) -> Option<Vec3> {
    let cleaned: String =
        t.chars().map(|c| if matches!(c, '{' | '}' | ',' | ';') { ' ' } else { c }).collect();
    let parts: Vec<f32> =
        cleaned.split_whitespace().map(|p| p.parse::<f32>()).collect::<Result<_, _>>().ok()?;
    (parts.len() == 3).then(|| Vec3::new(parts[0], parts[1], parts[2]))
}

fn parse_color(t: &str) -> Option<(u32, bool)> {
    let s =
        t.strip_prefix('#').or_else(|| t.strip_prefix("0x")).or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    if s.len() != 6 && s.len() != 8 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(if s.len() == 6 { (v | 0xff00_0000, false) } else { (v, true) })
}

fn find_name(t: &str, names: &[SmolStr]) -> Result<usize, String> {
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(t)) {
        return Ok(i);
    }
    let mut s = String::new();
    for (i, n) in names.iter().take(16).enumerate() {
        s.push_str(if i > 0 { ", " } else { "takes one of " });
        s.push_str(n);
    }
    if names.len() > 16 {
        s.push_str(", ...");
    }
    Err(s)
}

/// `SetValueText`'s parsing: the value or the sentence the C++ answers (after the key).
pub fn parse(r: &Row, t: &str) -> Result<Parsed, String> {
    if !r.enabled {
        return Err("is read-only".into());
    }
    match &r.kind {
        Kind::Float { min, max, .. } | Kind::Angle { min, max, .. } => {
            let unit = unit_of(r);
            let mut n = t.trim_end();
            if !unit.is_empty() && n.len() > unit.len() && n.ends_with(unit.as_str()) {
                n = n[..n.len() - unit.len()].trim_end();
            }
            let v: f64 = n.parse().map_err(|_| "takes a number".to_owned())?;
            let angle = matches!(r.kind, Kind::Angle { .. });
            let (lo, hi) = if angle {
                (f64::from(min.to_degrees()), f64::from(max.to_degrees()))
            } else {
                (f64::from(*min), f64::from(*max))
            };
            if min < max && (v < lo || v > hi) {
                return Err(format!(
                    "takes a number from {} to {}",
                    crate::ltx::fmt_g(lo as f32),
                    crate::ltx::fmt_g(hi as f32)
                ));
            }
            Ok(Parsed::Num(if angle { v.to_radians() } else { v }))
        }
        Kind::Int { min, max } => {
            let v: f64 = t.trim().parse().map_err(|_| "takes a number".to_owned())?;
            if min < max && (v < *min as f64 || v > *max as f64) {
                return Err(format!("takes a number from {min} to {max}"));
            }
            Ok(Parsed::Num(v.floor()))
        }
        Kind::Time => t.trim().parse::<f64>().map(Parsed::Num).map_err(|_| "takes a number".to_owned()),
        Kind::Bool | Kind::Flag => {
            parse_bool(t).map(Parsed::Bool).ok_or_else(|| "takes true or false".to_owned())
        }
        Kind::Vector { min, max, .. } => {
            let v = parse_vec(t).ok_or_else(|| "takes three numbers: x, y, z".to_owned())?;
            Ok(Parsed::Vec3(if min < max { v.clamp(Vec3::splat(*min), Vec3::splat(*max)) } else { v }))
        }
        Kind::Angle3 { .. } => {
            let v = parse_vec(t).ok_or_else(|| "takes three numbers: x, y, z".to_owned())?;
            Ok(Parsed::Vec3(Vec3::new(v.x.to_radians(), v.y.to_radians(), v.z.to_radians())))
        }
        Kind::Color | Kind::FColor => {
            let (argb, alpha) =
                parse_color(t).ok_or_else(|| "takes a colour: #RRGGBB or 0xAARRGGBB".to_owned())?;
            Ok(Parsed::Color { argb, alpha })
        }
        Kind::Token(items) => {
            let names: Vec<SmolStr> = items.iter().map(|(n, _)| n.clone()).collect();
            let i = find_name(t, &names)?;
            Ok(Parsed::Num(items.get(i).map_or(0, |(_, id)| *id) as f64))
        }
        Kind::Button(c) => find_name(t, c).map(Parsed::Press),
        Kind::Text | Kind::Name => Ok(Parsed::Text(t.to_owned())),
        Kind::Choose(_) => Ok(Parsed::Text(if t.is_empty() { "<none>".to_owned() } else { t.to_owned() })),
        Kind::Caption | Kind::Canvas => Err("cannot be set by text".into()),
    }
}

/// What setting a row of an object does.
// one per property set, handed straight to the store: boxing the object would only add a copy
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The object changed (and nothing else).
    Object(Object),
    /// A transform row: applied with the group rules (`NumSet*`).
    Transform(TransformEdit),
    /// Several objects changed (a group's objects).
    Objects(Vec<Object>),
    /// The row did nothing (a play button).
    Nothing,
}

fn num(p: &Parsed) -> f64 {
    match p {
        Parsed::Num(v) => *v,
        Parsed::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn on(p: &Parsed) -> bool {
    matches!(p, Parsed::Bool(true))
}

fn set_bit(word: &mut u32, bit: u32, value: bool) {
    if value {
        *word |= bit;
    } else {
        *word &= !bit;
    }
}

fn color_of(p: &Parsed, old: u32) -> u32 {
    match p {
        Parsed::Color { argb, alpha: true } => *argb,
        Parsed::Color { argb, alpha: false } => (*argb & 0x00ff_ffff) | (old & 0xff00_0000),
        _ => old,
    }
}

fn fcolor_of(p: &Parsed, old: [f32; 4]) -> [f32; 4] {
    match p {
        Parsed::Color { argb, alpha } => {
            let f = |s: u32| ((argb >> s) & 0xff) as f32 / 255.0;
            [f(16), f(8), f(0), if *alpha { f(24) } else { old[3] }]
        }
        _ => old,
    }
}

fn text_of(p: &Parsed) -> String {
    match p {
        Parsed::Text(t) => t.clone(),
        _ => String::new(),
    }
}

/// A motion row as an edit: `Some(None)` for what the row leaves to its caller (Play, Pause,
/// Import, Export, the two key dialogs).
fn motion_edit(k: &str, p: &Parsed) -> Result<Option<Option<crate::objmotion::MotionEdit>>, String> {
    use crate::objmotion::MotionEdit as E;
    let press = match p {
        Parsed::Press(i) => Some(*i),
        _ => None,
    };
    Ok(Some(match k {
        "Flags\\Motionable" => Some(E::Motionable(on(p))),
        "Motion\\Files" => None,
        "Motion\\Commands" => match press {
            Some(0) => Some(E::AddKey),
            Some(1) => Some(E::DeleteKey),
            Some(4) => Some(E::Clamp),
            _ => None,
        },
        "Motion\\Controls" => match press {
            Some(0) => Some(E::FirstFrame),
            Some(1) => Some(E::PreviousKey),
            Some(2) => Some(E::PreviousFrame),
            Some(5) => Some(E::NextFrame),
            Some(6) => Some(E::NextKey),
            Some(7) => Some(E::LastFrame),
            _ => None,
        },
        "Motion\\Flags\\Auto Key" => Some(E::AutoKey(on(p))),
        "Motion\\Flags\\Camera View" => Some(E::CameraView(on(p))),
        "Motion\\Start Frame (sec)" => Some(E::Range { min_t: Some(num(p) as f32), max_t: None }),
        "Motion\\End Frame (sec)" => Some(E::Range { min_t: None, max_t: Some(num(p) as f32) }),
        "Motion\\Current Frame (sec)" => Some(E::Current(num(p) as f32)),
        "Motion\\Key Time (sec)" => Some(E::KeyTime(num(p) as f32)),
        _ => return Ok(None),
    }))
}

/// Sets one row of an object. `entity` is the context the spawn rows set through.
pub fn set_object(
    state: &SceneState,
    o: &Object,
    k: &str,
    p: &Parsed,
    cx: &PropCx<'_>,
) -> Result<Effect, String> {
    let mut n = o.clone();
    match k {
        "Name" | "Way Name" => {
            let name = text_of(p).to_lowercase();
            if name.is_empty() || state.find_any(&name).is_some_and(|x| x.id != o.id) {
                return Err("is already used by another object".into());
            }
            n.name = name.into();
            return Ok(Effect::Object(n));
        }
        "Transform\\Position" => return Ok(Effect::Transform(TransformEdit::Position(vec_of(p)))),
        "Transform\\Rotation" => return Ok(Effect::Transform(TransformEdit::Rotation(vec_of(p)))),
        "Transform\\Scale" => return Ok(Effect::Transform(TransformEdit::Scale(vec_of(p)))),
        "Mod\\Removed from the game" => {
            n.marks.removed = on(p);
            return Ok(Effect::Object(n));
        }
        "In group editable" => {
            set_bit(&mut n.flags.co, co::IN_GROUP_UNIQUE, on(p));
            return Ok(Effect::Object(n));
        }
        _ => {}
    }
    if let Some(edit) = motion_edit(k, p)? {
        // Play, Pause and the files are the caller's: they need the frame clock or a file
        let Some(edit) = edit else { return Ok(Effect::Nothing) };
        let e = crate::objmotion::apply(o, &edit)?;
        n.motion = e.motion;
        n.set_co_flags(e.flags);
        if let Some((pos, rot)) = e.pose {
            return Ok(Effect::Objects(vec![{
                let mut moved = n.clone();
                moved.xform.position = pos;
                moved.xform.rotation = rot;
                moved
            }]));
        }
        return Ok(Effect::Object(n));
    }
    match &mut n.data {
        ObjData::SceneObject(d) => {
            if k == "Reference" {
                d.reference = text_of(p).into();
            } else if k == "Surfaces\\Action" {
                d.flags &= !SO_USE_SURFACE;
                d.surfaces.clear();
            } else if let Some(rest) = k.strip_prefix("Surfaces\\") {
                let (sname, field) = rest.rsplit_once('\\').ok_or("is no row of this list")?;
                if d.flags & SO_USE_SURFACE == 0 {
                    d.surfaces = library_surfaces(cx.lib, &d.reference);
                    d.flags |= SO_USE_SURFACE;
                }
                let s = d.surfaces.iter_mut().find(|s| s.name == sname).ok_or("is no row of this list")?;
                let v: SmolStr = text_of(p).into();
                match field {
                    "Texture" => s.texture = v,
                    "Shader" => s.shader = v,
                    "Compile" => s.shader_xrlc = v,
                    "Game Mtl" => s.game_mtl = v,
                    _ => return Err("is no row of this list".into()),
                }
            } else {
                return Err("is no row of this list".into());
            }
        }
        ObjData::Light(d) => match k {
            "Type" => d.kind = num(p) as u32,
            "Color" => d.color = fcolor_of(p, d.color),
            "Brightness" => d.brightness = num(p) as f32,
            "Light Control" => d.lcontrol = num(p) as u32,
            "Usage\\LightMap" => set_bit(&mut d.flags, light_flags::AFFECT_STATIC, on(p)),
            "Usage\\Dynamic" => set_bit(&mut d.flags, light_flags::AFFECT_DYNAMIC, on(p)),
            "Usage\\Animated" => set_bit(&mut d.flags, light_flags::PROCEDURAL, on(p)),
            "Range" => d.range = num(p) as f32,
            "Attenuation\\Constant" => d.attenuation[0] = num(p) as f32,
            "Attenuation\\Linear" => d.attenuation[1] = num(p) as f32,
            "Attenuation\\Quadratic" => d.attenuation[2] = num(p) as f32,
            "Attenuation\\Auto" => {
                // `OnAutoClick`
                let pp = 0.1f32;
                d.attenuation[0] = 1.0;
                if matches!(p, Parsed::Press(0)) {
                    d.attenuation[1] = ((d.brightness - pp - pp * d.range * d.range * d.attenuation[2])
                        / (pp * d.range))
                        .clamp(0.0, 1.0);
                } else {
                    d.attenuation[2] = ((d.brightness - pp - pp * d.range * d.attenuation[1])
                        / (pp * d.range * d.range))
                        .clamp(0.0, 1.0);
                }
            }
            "Spot R1\\Cone Angle" => d.cone = num(p) as f32,
            "Spot R1\\Texture" => d.falloff_tex = text_of(p).into(),
            "Fuzzy" => {
                set_bit(&mut d.flags, light_flags::POINT_FUZZY, on(p));
                d.fuzzy = on(p).then(Default::default);
            }
            "Fuzzy\\Count" => {
                if let Some(f) = &mut d.fuzzy {
                    f.points.resize(num(p).max(1.0) as usize, Vec3::ZERO);
                }
            }
            "Fuzzy\\Shape" => {
                if let Some(f) = &mut d.fuzzy {
                    f.shape = num(p) as u8;
                }
            }
            "Fuzzy\\Radius" => {
                if let Some(f) = &mut d.fuzzy {
                    f.sphere_radius = num(p) as f32;
                }
            }
            "Fuzzy\\Half Dimension" => {
                if let Some(f) = &mut d.fuzzy {
                    f.box_dim = vec_of(p);
                }
            }
            "Fuzzy\\Generate" => return Ok(Effect::Nothing),
            "Use In D3D" => d.use_in_d3d = on(p),
            _ => return Err("is no row of this list".into()),
        },
        ObjData::Glow(d) => match k {
            "Texture" => d.texture = text_of(p).into(),
            "Shader" => d.shader = text_of(p).into(),
            "Radius" => {
                d.radius = num(p) as f32;
                n.xform.scale = Vec3::splat(d.radius);
            }
            _ => return Err("is no row of this list".into()),
        },
        ObjData::SoundEnv(d) => match k {
            "Environment Inner" => d.inner = text_of(p).into(),
            "Environment Outer" => d.outer = text_of(p).into(),
            _ => return Err("is no row of this list".into()),
        },
        ObjData::SoundSrc(d) => match k {
            "Custom\\Controls" => return Ok(Effect::Nothing),
            "Source\\WAVE name" => d.wav = text_of(p).into(),
            "Source\\Frequency" => d.freq = num(p) as f32,
            "Source\\Volume" => d.volume = num(p) as f32,
            "Game\\Active time\\From" => d.active_time[0] = num(p) as f32,
            "Game\\Active time\\To" => d.active_time[1] = num(p) as f32,
            "Game\\Play time\\From" => d.play_time[0] = num(p) as f32,
            "Game\\Play time\\To" => d.play_time[1] = num(p) as f32,
            "Game\\Pause delta\\From" => d.random_pause[0] = num(p) as f32,
            "Game\\Pause delta\\To" => d.random_pause[1] = num(p) as f32,
            _ => return Err("is no row of this list".into()),
        },
        ObjData::Particles(d) => match k {
            "Reference" => d.reference = text_of(p).into(),
            "Controls" => return Ok(Effect::Nothing),
            _ => return Err("is no row of this list".into()),
        },
        ObjData::Sector(d) => {
            if k == "Color" {
                d.color = fcolor_of(p, d.color);
            } else if k.ends_with("Change LevelMap to") {
                d.map_idx = num(p) as u8;
            } else {
                return Err("is no row of this list".into());
            }
        }
        ObjData::Group(g) => {
            if k == "Reference" {
                g.reference = text_of(p).into();
            } else if k == "FreezeObjects" {
                // `OnFreezeAllClick`: Unique makes every object editable, Reference none
                let unique = matches!(p, Parsed::Press(0));
                let members: Vec<Object> = g
                    .members
                    .iter()
                    .filter_map(|m| state.get(*m))
                    .map(|m| {
                        let mut m = (**m).clone();
                        set_bit(&mut m.flags.co, co::IN_GROUP_UNIQUE, unique);
                        m
                    })
                    .collect();
                return Ok(Effect::Objects(members));
            } else {
                let member = g
                    .members
                    .iter()
                    .filter_map(|m| state.get(*m))
                    .find(|m| format!("{} ({})", m.name, m.class.desc()) == k)
                    .ok_or("is no row of this list")?;
                let mut m = (**member).clone();
                set_bit(&mut m.flags.co, co::IN_GROUP_UNIQUE, on(p));
                return Ok(Effect::Objects(vec![m]));
            }
        }
        ObjData::FogVolume(_)
        | ObjData::Shape(_)
        | ObjData::Portal(_)
        | ObjData::Way(_)
        | ObjData::Kept(_) => return Err("is no row of this list".into()),
        ObjData::Spawn(d) => {
            if let Some(field) = k.strip_prefix("Order\\") {
                let o = &mut d.order;
                match field {
                    "Kind" => o.kind = num(p) as u8,
                    "Look path" => o.look = text_of(p).replace("<none>", "").into(),
                    "Route" => o.route = text_of(p).replace("<none>", "").into(),
                    "Target" => o.target = text_of(p).replace("<none>", "").into(),
                    "Radius" => o.radius = num(p) as f32,
                    "Distance" => o.distance = num(p) as f32,
                    "Run" => {
                        o.flags = if on(p) {
                            o.flags | crate::objects::ORDER_RUN
                        } else {
                            o.flags & !crate::objects::ORDER_RUN
                        }
                    }
                    "Hostile to player" => {
                        o.flags = if on(p) {
                            o.flags | crate::objects::ORDER_HOSTILE
                        } else {
                            o.flags & !crate::objects::ORDER_HOSTILE
                        }
                    }
                    _ => return Err("is no row of this list".into()),
                }
            } else if k == "MP respawn" {
                d.sd_flags = if on(p) { d.sd_flags | 1 } else { d.sd_flags & !1 };
            } else if let Some(field) = k.strip_prefix("Respawn Point\\") {
                match field {
                    "Team" => d.rpoint.team = num(p) as u8,
                    "Spawn Type" => d.rpoint.kind = num(p) as u8,
                    "Profile" => d.rpoint.profile = text_of(p).into(),
                    _ => return Err("is no row of this list".into()),
                }
            } else if let Some(field) = k.strip_prefix("Environment Modificator\\") {
                let e = &mut d.env;
                let bit = |name: &str| match name {
                    "View Distance" => Some(env_flags::VIEW_DIST),
                    "Fog Color" => Some(env_flags::FOG_COLOR),
                    "Fog Density" => Some(env_flags::FOG_DENSITY),
                    "Ambient Color" => Some(env_flags::AMBIENT_COLOR),
                    "Sky Color" => Some(env_flags::SKY_COLOR),
                    "Hemi Color" => Some(env_flags::HEMI_COLOR),
                    _ => None,
                };
                match field {
                    "Radius" => e.radius = num(p) as f32,
                    "Power" => e.power = num(p) as f32,
                    "View Distance\\ " => e.view_dist = num(p) as f32,
                    "Fog Color\\ " => e.fog_color = color_of(p, e.fog_color),
                    "Fog Density\\ " => e.fog_density = num(p) as f32,
                    "Ambient Color\\ " => e.ambient_color = color_of(p, e.ambient_color),
                    "Sky Color\\ " => e.sky_color = color_of(p, e.sky_color),
                    "Hemi Color\\ " => e.hemi_color = color_of(p, e.hemi_color),
                    other => {
                        let b = bit(other).ok_or("is no row of this list")?;
                        e.flags = if on(p) { e.flags | b } else { e.flags & !b };
                    }
                }
            } else if k.ends_with("\\Profile (spawn section)") {
                return Err("names another section: place a new spawn of it instead".into());
            } else {
                let Some(SpawnEntity::Decoded(ent)) = &mut d.entity else {
                    return Err("is read-only".into());
                };
                let prop = ent
                    .properties(cx.entity)
                    .into_iter()
                    .find(|x| x.path == k)
                    .ok_or("is no row of this list")?;
                let value = match (&prop.kind, p) {
                    (PropKind::Float { .. } | PropKind::Angle { .. }, _) => PropValue::Float(num(p) as f32),
                    (PropKind::Bool | PropKind::Flag { .. }, _) => PropValue::Bool(on(p)),
                    (PropKind::Vec3, Parsed::Vec3(v)) => PropValue::Vec3((*v).into()),
                    (PropKind::Color, Parsed::Color { argb, .. }) => PropValue::Int(i64::from(*argb)),
                    (_, Parsed::Num(v)) => PropValue::Int(*v as i64),
                    (_, Parsed::Text(t)) => PropValue::Text(t.replace("<none>", "")),
                    _ => return Err("cannot be set by text".into()),
                };
                let e = std::sync::Arc::make_mut(ent);
                e.set(&prop.path, &value, cx.entity).map_err(|err| err.to_string())?;
            }
        }
    }
    Ok(Effect::Object(n))
}

fn vec_of(p: &Parsed) -> Vec3 {
    match p {
        Parsed::Vec3(v) => *v,
        _ => Vec3::ZERO,
    }
}

/// Sets one row of Level Settings: the level options and the tools changed.
pub fn set_world(state: &SceneState, k: &str, p: &Parsed) -> Result<(LevelOptions, Vec<ToolPart>), String> {
    let mut lo = state.meta.level.clone();
    let mut tools: Vec<ToolPart> = Vec::new();
    let b = &mut lo.build;
    match k {
        "Scene\\Name prefix" => lo.level_prefix = text_of(p).into(),
        "Scene\\Build options\\Level path" => lo.level_path = text_of(p).into(),
        "Scene\\Build options\\Custom data" => lo.bop = text_of(p),
        "Scene\\Map version" => lo.map_version = text_of(p).into(),
        "Scene\\Build options\\Quality" => {
            if let Parsed::Press(i) = p {
                lo.set_quality(u16::try_from(*i).unwrap_or(2));
            }
        }
        "Scene\\Build options\\Lighting\\Hemisphere quality [0-3]" => lo.light_hemi_quality = num(p) as u8,
        "Scene\\Build options\\Lighting\\Sun shadow quality [0-3]" => lo.light_sun_quality = num(p) as u8,
        "Scene\\Build options\\Optimizing\\Normal smooth angle" => b.sm_angle = num(p) as f32,
        "Scene\\Build options\\Optimizing\\Weld distance (m)" => b.weld_distance = num(p) as f32,
        "Scene\\Build options\\Lighting\\Pixel per meter" => b.lm_pixels_per_meter = num(p) as f32,
        "Scene\\Build options\\Lighting\\Error (LM collapsing)" => b.lm_rms = num(p) as u32,
        "Scene\\Build options\\Lighting\\Error (LM zero)" => b.lm_rms_zero = num(p) as u32,
        "Scene\\Build options\\Lighting\\Jitter samples" => b.lm_jitter_samples = num(p) as u32,
        _ => {
            let (desc, rest) = k.split_once('\\').ok_or("is no row of this list")?;
            let class =
                ObjClass::ALL.iter().copied().find(|c| c.desc() == desc).ok_or("is no row of this list")?;
            let mut t = state.tools.part(class).clone();
            let bit = |r: &str| -> Option<u32> {
                Some(match (class, r) {
                    (ObjClass::Light, "Common\\Sun Shadow\\Visible")
                    | (ObjClass::Glow, "Common\\Test Visibility") => 1 << 31,
                    (ObjClass::Light, "Common\\Controls\\Draw Name")
                    | (ObjClass::Glow, "Common\\Draw Cross") => 1 << 30,
                    (ObjClass::Portal, "Common\\Draw Simple Model")
                    | (ObjClass::Sector, "Common\\Draw Solid") => 1 << 31,
                    _ => return None,
                })
            };
            if let Some(bit) = bit(rest) {
                set_bit(&mut t.flags, bit, on(p));
            } else if let ToolOpts::Light(l) = &mut t.opts {
                match rest {
                    "Common\\Sun Shadow\\Altitude" => l.sun_shadow_dir[0] = num(p) as f32,
                    "Common\\Sun Shadow\\Longitude" => l.sun_shadow_dir[1] = num(p) as f32,
                    _ => return Err("is no row of this list".into()),
                }
            } else {
                return Err("is no row of this list".into());
            }
            tools.push(t);
        }
    }
    Ok((lo, tools))
}

/// `fcolor_get` re-exported for the renderer's colours.
#[must_use]
pub fn sector_argb(c: [f32; 4]) -> u32 {
    fcolor_get(c)
}

/// The fog volume's emitter type token (kept for the Details panel of a later package).
#[must_use]
pub fn fog_types() -> &'static [(&'static str, i64)] {
    let _ = FV_EMITTER;
    FOG_TYPES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{GlowData, LightData};

    fn cx() -> PropContext {
        PropContext::default()
    }

    #[test]
    fn a_selection_merges_into_mixed_rows() {
        let mut s = SceneState::default();
        let mut a = Object::new(ObjClass::Light, "a", ObjData::Light(LightData::default()));
        a.xform.position = Vec3::X;
        let b = Object::new(ObjClass::Light, "b", ObjData::Light(LightData::default()));
        let (ia, ib) = (s.insert(a), s.insert(b));
        let e = cx();
        let pcx = PropCx { lib: None, entity: &e };
        let rows =
            merge(vec![object_rows(&s, s.get(ia).unwrap(), &pcx), object_rows(&s, s.get(ib).unwrap(), &pcx)]);
        let pos = find(&rows, "Transform\\Position").unwrap();
        assert!(pos.mixed);
        let range = find(&rows, "Range").unwrap();
        assert!(!range.mixed);
        assert_eq!(range.row.text(), "8.00");
        let name = find(&rows, "Name").unwrap();
        assert!(name.mixed && !name.row.enabled);
        assert_eq!(help_of(&range.row), "Range: 0.1 to 1000.");
    }

    #[test]
    fn rows_set_from_text_like_the_grid() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default())));
        let e = cx();
        let pcx = PropCx { lib: None, entity: &e };
        let rows = merge(vec![object_rows(&s, s.get(id).unwrap(), &pcx)]);
        let r = &find(&rows, "Radius").unwrap().row;
        assert_eq!(parse(r, "abc"), Err("takes a number".into()));
        assert_eq!(parse(r, "20000"), Err("takes a number from 0.01 to 10000".into()));
        let p = parse(r, "2.5").unwrap();
        let Effect::Object(o) = set_object(&s, s.get(id).unwrap(), "Radius", &p, &pcx).unwrap() else {
            panic!()
        };
        let ObjData::Glow(g) = &o.data else { panic!() };
        assert_eq!(g.radius, 2.5);
        let pos = &find(&rows, "Position").unwrap().row;
        assert_eq!(parse(pos, "1, 2, 3"), Ok(Parsed::Vec3(Vec3::new(1.0, 2.0, 3.0))));
        let rot = &find(&rows, "Rotation").unwrap().row;
        assert_eq!(rot.text(), "{0.0, 0.0, 0.0}");
        assert_eq!(unit_of(&find(&rows, "Radius").unwrap().row), "");
    }

    #[test]
    fn labels_and_units_come_from_the_key() {
        assert_eq!(label_of("Scene\\Build options\\Optimizing\\Weld distance (m)"), "Weld distance");
        assert_eq!(
            label_of("Scene\\Build options\\Lighting\\Hemisphere quality [0-3]"),
            "Hemisphere quality"
        );
        let w = world_rows(&SceneState::default());
        let weld = w.iter().find(|r| r.key.ends_with("Weld distance (m)")).unwrap();
        assert_eq!(unit_of(weld), "m");
        assert!(!weld.enabled);
        let hemi = w.iter().find(|r| r.key.ends_with("[0-3]")).unwrap();
        assert_eq!(help_of(hemi), "Range: 0-3.");
    }
}

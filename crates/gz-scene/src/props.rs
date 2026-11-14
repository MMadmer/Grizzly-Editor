//! The property rows the Details panel and `scene.get` show: one row per key of the scene's
//! files (the tables of `crate::format::fields`, with their labels, units and help), the spawn
//! entity's named properties, and what only the panel has (buttons, counts). A row's `key` is
//! the stable name files and commands use (`range`, `surfaces.wall.texture`,
//! `props.story_id`); its `path` is where the panel shows it (`Light\Range`).
//!
//! [`object_rows`] lists an object's rows, [`merge`] folds a selection into one list ("mixed"
//! rows), [`parse`] reads a value given as text and [`set_object`] / [`set_world`] apply it.

use gz_base::SmolStr;
use gz_entities::{ListKind, PropContext, PropKind, PropValue};
use gz_luadata::Value;

use crate::apply::TransformEdit;
use crate::class::ObjClass;
use crate::format::fields::{self, Field, Ty};
use crate::format::values;
use crate::library::Library;
use crate::math::{Vec3, fmt_g};
use crate::model::{ObjData, Object, SceneState, co};
use crate::objects::{
    PointKind, SO_USE_SURFACE, SpawnEntity, SurfaceOverride, env_flags, fcolor_get, light_flags,
};
use crate::tools::{LevelOptions, ToolPart};

/// How a row is edited, with its limits.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Caption,
    Text,
    /// A name checked for uniqueness.
    Name,
    /// A pick from a list the editor fills (`object`, `texture`, ...).
    Choose(Choice),
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
    /// Three angles in radians, shown in degrees.
    Angle3 {
        dec: u8,
    },
    /// `0xAARRGGBB`.
    Color,
    /// `{ r, g, b[, a] }` from 0 to 1, shown as its RGB.
    FColor,
    Button(Vec<SmolStr>),
    /// Seconds after midnight, shown as `hh:mm:ss`.
    Time,
    Canvas,
}

/// What a choose row picks from.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// The list: `object`, `texture`, `skeletonanims`, `skeletonbones`, `gameanim`, ...
    pub list: SmolStr,
    /// The list's parameter, where it depends on the object: the model whose motions or bones
    /// it lists, the sections a restriction picks from.
    pub param: Option<SmolStr>,
    /// How many items the row takes: more than one is a comma list.
    pub max: u32,
}

impl Choice {
    #[must_use]
    pub fn of(list: &str) -> Self {
        Self { list: SmolStr::new(list), param: None, max: 1 }
    }
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
    /// The stable key (`range`, `props.story_id`).
    pub key: String,
    /// Where the panel shows it: its sections and its label, `\`-separated.
    pub path: String,
    /// Title Case plain English.
    pub label: String,
    /// `m`, `deg`, `s`, `%`, ... or empty.
    pub unit: &'static str,
    pub kind: Kind,
    pub value: Val,
    pub enabled: bool,
    pub help: String,
}

impl Row {
    fn new(key: impl Into<String>, group: &str, label: &str, kind: Kind, value: Val) -> Self {
        let path = if group.is_empty() { label.to_owned() } else { format!("{group}\\{label}") };
        Self {
            key: key.into(),
            path,
            label: label.to_owned(),
            unit: "",
            kind,
            value,
            enabled: true,
            help: String::new(),
        }
    }

    fn unit(mut self, u: &'static str) -> Self {
        self.unit = u;
        self
    }

    fn help(mut self, h: &str) -> Self {
        self.help = h.to_owned();
        self
    }

    fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The value as the panel writes it.
    #[must_use]
    pub fn text(&self) -> String {
        match (&self.kind, &self.value) {
            (Kind::Float { dec, .. }, Val::Num(v)) => format!("{:.*}", *dec as usize, *v),
            (Kind::Angle { dec, .. }, Val::Num(v)) => format!("{:.*}", *dec as usize, v.to_degrees()),
            (Kind::Int { .. }, Val::Num(v)) => format!("{}", *v as i64),
            (Kind::Color, Val::Color(c)) => format!("#{:06X}", c & 0x00ff_ffff),
            (Kind::FColor, Val::FColor(c)) => format!("#{:06X}", fcolor_get(*c) & 0x00ff_ffff),
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
            (Kind::Flag, Val::Text(t)) => t.clone(),
            (Kind::Bool | Kind::Flag, Val::Bool(b)) => {
                if *b {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            (Kind::Canvas, _) => String::new(),
            (_, Val::Text(t)) => t.clone(),
            _ => String::new(),
        }
    }
}

fn vec_text(v: Vec3, dec: u8) -> String {
    let d = dec as usize;
    format!("{:.d$}, {:.d$}, {:.d$}", v.x, v.y, v.z)
}

fn button(key: &str, group: &str, label: &str, captions: &str) -> Row {
    Row::new(key, group, label, Kind::Button(captions.split(',').map(SmolStr::new).collect()), Val::None)
}

fn caption(key: &str, group: &str, label: &str, text: &str) -> Row {
    Row::new(key, group, label, Kind::Caption, Val::Text(text.to_owned()))
}

/// What the rows read beside the scene.
pub struct PropCx<'a> {
    pub lib: Option<&'a Library>,
    pub entity: &'a PropContext,
}

/// A word of a file as the panel names it: `"point"` reads Point, `"team_deathmatch"` Team
/// Deathmatch.
#[must_use]
pub fn word_label(w: &str) -> String {
    let mut out = String::new();
    for part in w.trim_start_matches('$').split(['_', ' ']) {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let mut c = part.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

/// The row of a key of the files, for `T` an object or the scene.
fn field_row<T>(f: &Field<T>, t: &T) -> Option<Row> {
    let v = (f.get)(t);
    let (kind, val) = match f.ty {
        Ty::Text => (Kind::Text, Val::Text(v.as_str().unwrap_or("").to_owned())),
        Ty::Name => (Kind::Name, Val::Text(v.as_str().unwrap_or("").to_owned())),
        Ty::Pick(list) => (Kind::Choose(Choice::of(list)), Val::Text(v.as_str().unwrap_or("").to_owned())),
        Ty::Bool => (Kind::Bool, Val::Bool(v.as_bool().unwrap_or(false))),
        Ty::Number { min, max, dec } => {
            (Kind::Float { min, max, dec }, Val::Num(v.as_number().unwrap_or(0.0)))
        }
        Ty::Integer { min, max } => (Kind::Int { min, max }, Val::Num(v.as_number().unwrap_or(0.0))),
        Ty::Vec3 => {
            let (min, max) = if f.key == "scale" { (0.01, 10000.0) } else { (-100_000.0, 100_000.0) };
            (Kind::Vector { min, max, dec: 2 }, Val::Vec3(values::as_vec3(&v).unwrap_or(Vec3::ZERO)))
        }
        Ty::Angles => (Kind::Angle3 { dec: 1 }, Val::Vec3(values::as_angles(&v).unwrap_or(Vec3::ZERO))),
        Ty::Angle { min, max } => (
            Kind::Angle { min: min.to_radians(), max: max.to_radians(), dec: 2 },
            Val::Num(f64::from(values::as_angle(&v).unwrap_or(0.0))),
        ),
        Ty::Color => (Kind::FColor, Val::FColor(values::as_color(&v).unwrap_or([0.0; 4]))),
        Ty::Words(words) => {
            let items = words.iter().map(|(w, n)| (SmolStr::new(word_label(w)), i64::from(*n))).collect();
            let n = match &v {
                Value::String(s) => {
                    words.iter().find(|(w, _)| w.eq_ignore_ascii_case(s)).map_or(0, |(_, n)| *n)
                }
                Value::Number(n) => *n as u32,
                _ => 0,
            };
            (Kind::Token(items), Val::Num(f64::from(n)))
        }
        Ty::Range | Ty::Table(_) => return None,
    };
    Some(Row::new(f.key, f.group, f.label, kind, val).unit(unit_word(f.unit)).help(f.help))
}

fn unit_word(u: &'static str) -> &'static str {
    match u {
        "deg" => "\u{b0}",
        other => other,
    }
}

/// A row's value as the file's value of its key.
fn value_of(r: &Row, p: &Parsed) -> Result<Value, String> {
    Ok(match (&r.kind, p) {
        (Kind::Angle { .. }, Parsed::Num(v)) => values::angle(*v as f32),
        (Kind::Angle3 { .. }, Parsed::Vec3(v)) => values::angles(*v),
        (Kind::Vector { .. }, Parsed::Vec3(v)) => values::vec3(*v),
        (Kind::Token(_), Parsed::Num(v)) => Value::Number(*v),
        (Kind::FColor, Parsed::Color { argb, .. }) => {
            let f = |s: u32| ((argb >> s) & 0xff) as f32 / 255.0;
            values::color([f(16), f(8), f(0), 0.0])
        }
        (Kind::Color, Parsed::Color { argb, .. }) => values::argb(*argb),
        (_, Parsed::Num(v)) => Value::Number(*v),
        (_, Parsed::Bool(b)) => Value::Bool(*b),
        (_, Parsed::Text(t)) => Value::String(SmolStr::new(t)),
        (_, Parsed::Vec3(v)) => values::vec3(*v),
        (_, Parsed::Color { argb, .. }) => values::argb(*argb),
        (_, Parsed::Press(_)) => return Err("is a button".into()),
    })
}

/// A Words field's value for a token's number: its word.
fn word_value(f: &Field<Object>, n: f64) -> Value {
    match f.ty {
        Ty::Words(words) => values::word(n as u32, words),
        _ => Value::Number(n),
    }
}

// ---------------------------------------------------------------- objects

const ORDER_KINDS: &[(&str, i64)] = &[("None", 0), ("Guard", 1), ("Patrol", 2), ("Wander", 3), ("Follow", 4)];
const RPOINT_KINDS: &[(&str, i64)] = &[("Actor", 0), ("Artefact", 1), ("Item", 2)];
const FUZZY_SHAPES: &[(&str, i64)] = &[("Sphere", 0), ("Box", 1)];

/// The playback buttons of a spawn's model.
pub const SPAWN_ANIMATION: &str = "spawn.animation";
pub const SPAWN_ANIMATION_BUTTONS: &str = "First Frame,Play,Pause,Stop,Last Frame";

fn token(items: &[(&str, i64)]) -> Kind {
    Kind::Token(items.iter().map(|(n, i)| (SmolStr::new(n), *i)).collect())
}

/// A motion's rows (a dynamic object moves along one).
fn motion_rows(o: &Object, out: &mut Vec<Row>) {
    let g = "Motion";
    let flags = o.co_flags();
    out.push(Row::new(
        "motion.on",
        g,
        "Moves Along a Motion",
        Kind::Bool,
        Val::Bool(flags & co::MOTION != 0),
    ));
    if flags & co::MOTION == 0 {
        return;
    }
    let Some(raw) = o.motion.as_ref() else { return };
    let Some(m) = raw.model() else {
        out.push(caption("motion.version", g, "Motion", "of a version this editor does not read").disabled());
        return;
    };
    let p = crate::objmotion::params(&m, raw);
    out.push(button("motion.files", g, "Files", "Import,Export"));
    out.push(
        button("motion.commands", g, "Keys", "Add Key,Delete Key,Scale Keys...,Normalize Keys...,Clamp")
            .help("Keys are made in the object's own coordinates."),
    );
    out.push(button(
        "motion.controls",
        g,
        "Playback",
        "First Frame,Previous Key,Previous Frame,Play,Pause,Next Frame,Next Key,Last Frame",
    ));
    out.push(Row::new(
        "motion.auto_key",
        g,
        "Auto Key",
        Kind::Bool,
        Val::Bool(flags & crate::objmotion::AUTO_KEY != 0),
    ));
    out.push(Row::new(
        "motion.camera_view",
        g,
        "Camera View",
        Kind::Bool,
        Val::Bool(flags & crate::objmotion::CAMERA_VIEW != 0),
    ));
    let t = |min: f32, max: f32| Kind::Float { min, max, dec: 3 };
    out.push(
        Row::new("motion.start", g, "Start", t(-10000.0, p.max_t), Val::Num(f64::from(p.min_t))).unit("s"),
    );
    out.push(Row::new("motion.end", g, "End", t(p.min_t, 10000.0), Val::Num(f64::from(p.max_t))).unit("s"));
    out.push(
        Row::new("motion.current", g, "Current Time", t(-10000.0, 10000.0), Val::Num(f64::from(p.t_current)))
            .unit("s"),
    );
    out.push(
        Row::new("motion.key_time", g, "Key Time", t(-10000.0, 10000.0), Val::Num(f64::from(p.t_current)))
            .unit("s"),
    );
    out.push(caption("motion.keys", g, "Keys", &m.envelopes[0].keys.len().to_string()));
    out.push(
        caption("motion.length", g, "Length", &format!("{:.2}", crate::objmotion::length(&m).0)).unit("s"),
    );
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

/// One object's rows (see the module).
#[must_use]
pub fn object_rows(state: &SceneState, o: &Object, cx: &PropCx<'_>) -> Vec<Row> {
    let mut out = Vec::new();
    for f in fields::object_fields(o.class) {
        if !(f.applies)(o) {
            continue;
        }
        match (o.class, f.key) {
            (
                _,
                "motion" | "flags" | "draw_selected" | "object_flags" | "light_flags" | "glow_flags"
                | "spawn_flags",
            ) => continue,
            (ObjClass::Light, "control") => {
                let controls: Vec<(SmolStr, i64)> = state
                    .tools
                    .part(ObjClass::Light)
                    .light()
                    .map(|l| {
                        l.lcontrols
                            .iter()
                            .map(|(n, i)| (SmolStr::new(word_label(n)), i64::from(*i)))
                            .collect()
                    })
                    .unwrap_or_default();
                let id = match &o.data {
                    ObjData::Light(d) => d.lcontrol,
                    _ => 0,
                };
                out.push(
                    Row::new("control", f.group, f.label, Kind::Token(controls), Val::Num(f64::from(id)))
                        .help(f.help),
                );
                continue;
            }
            (ObjClass::SpawnPoint, "section") => {
                if let ObjData::Spawn(d) = &o.data
                    && let Some(e) = &d.entity
                {
                    let section = e.section().to_owned();
                    let profile =
                        cx.entity.profiles.get(section.as_str()).map_or(section.as_str(), |p| p.as_str());
                    out.push(
                        Row::new(
                            "section",
                            f.group,
                            f.label,
                            Kind::Choose(Choice::of("spawn_section")),
                            Val::Text(profile.to_owned()),
                        )
                        .help(f.help)
                        .disabled(),
                    );
                }
                continue;
            }
            (ObjClass::SpawnPoint, "point") => {
                if let ObjData::Spawn(d) = &o.data {
                    let (label, help) = match d.kind {
                        PointKind::PlayerStart => (
                            "Player Start",
                            "A new game of the module's mode starts here, looking where the arrow points; one per module (Mod > World Rules).",
                        ),
                        PointKind::RPoint => {
                            ("Respawn Point", "Who or what respawns here in the multiplayer games.")
                        }
                        PointKind::EnvMod => {
                            ("Environment Modifier", "How the weather looks within its radius.")
                        }
                        PointKind::Spawn => ("Spawn", ""),
                    };
                    out.push(caption("point", f.group, f.label, label).help(help));
                }
                continue;
            }
            _ => {}
        }
        if let Some(r) = field_row(f, o) {
            out.push(r);
            continue;
        }
        // the tables of the files, as the rows they are made of
        table_rows(state, o, f, cx, &mut out);
    }
    if let ObjData::SceneObject(d) = &o.data
        && cx.lib.and_then(|l| l.cached(&d.reference)).is_some_and(|l| l.object.is_dynamic())
    {
        motion_rows(o, &mut out);
    }
    // a locked object (or kind) shows its rows disabled but its lock; a group's member that is
    // not editable by itself, but the switch that makes it so
    let changeable = state.changeable(o) && o.editable();
    for r in &mut out {
        if !changeable && r.key != "locked" && r.key != "group_editable" && r.key != "hidden" {
            r.enabled = false;
        }
    }
    out
}

fn table_rows(state: &SceneState, o: &Object, f: &Field<Object>, cx: &PropCx<'_>, out: &mut Vec<Row>) {
    match (&o.data, f.key) {
        (ObjData::SceneObject(d), "surfaces") => {
            let surfaces = if d.flags & SO_USE_SURFACE != 0 {
                d.surfaces.clone()
            } else {
                library_surfaces(cx.lib, &d.reference)
            };
            for s in surfaces.iter().filter(|s| s.game_mtl != "materials\\occ") {
                let g = format!("Surfaces\\{}", s.name);
                let k = |x: &str| format!("surfaces.{}.{x}", s.name);
                out.push(Row::new(
                    k("texture"),
                    &g,
                    "Texture",
                    Kind::Choose(Choice::of("texture")),
                    Val::Text(s.texture.to_string()),
                ));
                out.push(Row::new(
                    k("shader"),
                    &g,
                    "Shader",
                    Kind::Choose(Choice::of("e_shader")),
                    Val::Text(s.shader.to_string()),
                ));
                out.push(
                    Row::new(
                        k("compile"),
                        &g,
                        "Build Shader",
                        Kind::Choose(Choice::of("c_shader")),
                        Val::Text(s.shader_xrlc.to_string()),
                    )
                    .help("How the level build compiles the surface."),
                );
                out.push(Row::new(
                    k("material"),
                    &g,
                    "Material",
                    Kind::Choose(Choice::of("game_material")),
                    Val::Text(s.game_mtl.to_string()),
                ));
            }
            out.push(
                button("surfaces.reset", "Surfaces", "Reset", "Reset")
                    .help("The surfaces take the library object's own values again."),
            );
        }
        (ObjData::Light(d), "fuzzy") => {
            let g = "Fuzzy";
            out.push(
                Row::new(
                    "fuzzy.on",
                    g,
                    "Fuzzy",
                    Kind::Bool,
                    Val::Bool(d.flags & light_flags::POINT_FUZZY != 0),
                )
                .help(f.help),
            );
            if d.flags & light_flags::POINT_FUZZY != 0
                && let Some(fz) = &d.fuzzy
            {
                out.push(Row::new(
                    "fuzzy.count",
                    g,
                    "Points",
                    Kind::Int { min: 1, max: 100 },
                    Val::Num(fz.points.len() as f64),
                ));
                out.push(button("fuzzy.generate", g, "Generate", "Random"));
                out.push(Row::new(
                    "fuzzy.shape",
                    g,
                    "Shape",
                    token(FUZZY_SHAPES),
                    Val::Num(f64::from(fz.shape)),
                ));
                if fz.shape == 0 {
                    out.push(
                        Row::new(
                            "fuzzy.radius",
                            g,
                            "Radius",
                            Kind::Float { min: 0.01, max: 100.0, dec: 2 },
                            Val::Num(f64::from(fz.sphere_radius)),
                        )
                        .unit("m"),
                    );
                } else {
                    out.push(
                        Row::new(
                            "fuzzy.box",
                            g,
                            "Half Size",
                            Kind::Vector { min: 0.01, max: 100.0, dec: 2 },
                            Val::Vec3(fz.box_dim),
                        )
                        .unit("m"),
                    );
                }
            }
        }
        (ObjData::SoundSrc(d), "pause" | "active" | "play") => {
            let (v, kind) = match f.key {
                "pause" => (d.random_pause, Kind::Float { min: 0.0, max: 100_000.0, dec: 1 }),
                "active" => (d.active_time, Kind::Time),
                _ => (d.play_time, Kind::Time),
            };
            let g = format!("{}\\{}", f.group, f.label);
            out.push(
                Row::new(format!("{}.from", f.key), &g, "From", kind.clone(), Val::Num(f64::from(v[0])))
                    .unit("s")
                    .help(f.help),
            );
            out.push(
                Row::new(format!("{}.to", f.key), &g, "To", kind, Val::Num(f64::from(v[1])))
                    .unit("s")
                    .help(f.help),
            );
        }
        (ObjData::Spawn(d), "props") => {
            if let Some(SpawnEntity::Decoded(ent)) = &d.entity {
                for p in ent.properties(cx.entity) {
                    out.push(entity_row(&p, cx.entity));
                }
                if ent.visual_name().is_some() {
                    out.push(button(SPAWN_ANIMATION, "Model", "Playback", SPAWN_ANIMATION_BUTTONS));
                }
            } else if let Some(e) = &d.entity {
                let why = match e {
                    SpawnEntity::Kept { why, .. } if !why.is_empty() => why.clone(),
                    _ => "link the game to edit it".to_owned(),
                };
                out.push(
                    caption("props", "Spawn", "Properties", &format!("kept as the file has it: {why}"))
                        .disabled(),
                );
            }
            if state.meta.level.map_usage & ((1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)) != 0 {
                out.push(Row::new(
                    "respawn",
                    "Spawn",
                    "Respawns in Multiplayer",
                    Kind::Bool,
                    Val::Bool(d.sd_flags & 1 != 0),
                ));
            }
        }
        (ObjData::Spawn(d), "order") => {
            let o = &d.order;
            let g = "Order";
            out.push(
                Row::new("order.kind", g, "Order", token(ORDER_KINDS), Val::Num(f64::from(o.kind)))
                    .help(f.help),
            );
            let way = |k: &str, label: &str, v: &str| {
                Row::new(k, g, label, Kind::Choose(Choice::of("way_path")), Val::Text(v.to_owned()))
            };
            match o.kind {
                1 => {
                    out.push(way("order.look", "Look Along", &o.look));
                    out.push(
                        Row::new(
                            "order.radius",
                            g,
                            "Radius",
                            Kind::Float { min: 1.0, max: 100.0, dec: 0 },
                            Val::Num(f64::from(o.radius)),
                        )
                        .unit("m"),
                    );
                }
                2 => {
                    out.push(way("order.route", "Route", &o.route));
                    out.push(Row::new(
                        "order.run",
                        g,
                        "Run",
                        Kind::Bool,
                        Val::Bool(o.flags & crate::objects::ORDER_RUN != 0),
                    ));
                }
                3 => {
                    out.push(
                        Row::new(
                            "order.radius",
                            g,
                            "Radius",
                            Kind::Float { min: 3.0, max: 150.0, dec: 0 },
                            Val::Num(f64::from(o.radius)),
                        )
                        .unit("m"),
                    );
                    out.push(Row::new(
                        "order.run",
                        g,
                        "Run",
                        Kind::Bool,
                        Val::Bool(o.flags & crate::objects::ORDER_RUN != 0),
                    ));
                }
                4 => {
                    out.push(Row::new(
                        "order.target",
                        g,
                        "Target",
                        Kind::Choose(Choice::of("story_id")),
                        Val::Text(o.target.to_string()),
                    ));
                    out.push(
                        Row::new(
                            "order.distance",
                            g,
                            "Distance",
                            Kind::Float { min: 1.0, max: 30.0, dec: 1 },
                            Val::Num(f64::from(o.distance)),
                        )
                        .unit("m"),
                    );
                }
                _ => {}
            }
            out.push(Row::new(
                "order.hostile",
                g,
                "Hostile to the Player",
                Kind::Bool,
                Val::Bool(o.flags & crate::objects::ORDER_HOSTILE != 0),
            ));
        }
        (ObjData::Spawn(d), "respawn_point") => {
            let g = "Respawn Point";
            if d.rpoint.kind == 2 {
                out.push(Row::new(
                    "respawn_point.profile",
                    g,
                    "Item",
                    Kind::Choose(Choice::of("respawn_items")),
                    Val::Text(d.rpoint.profile.to_string()),
                ));
            } else {
                out.push(Row::new(
                    "respawn_point.team",
                    g,
                    "Team",
                    Kind::Int { min: 0, max: 7 },
                    Val::Num(f64::from(d.rpoint.team)),
                ));
            }
            out.push(Row::new(
                "respawn_point.kind",
                g,
                "Respawns",
                token(RPOINT_KINDS),
                Val::Num(f64::from(d.rpoint.kind)),
            ));
        }
        (ObjData::Spawn(d), "environment") => {
            let e = &d.env;
            let g = "Environment Modifier";
            out.push(
                Row::new(
                    "environment.radius",
                    g,
                    "Radius",
                    Kind::Float { min: 0.0001, max: 10000.0, dec: 2 },
                    Val::Num(f64::from(e.radius)),
                )
                .unit("m"),
            );
            out.push(Row::new(
                "environment.power",
                g,
                "Power",
                Kind::Float { min: 0.000_01, max: 1000.0, dec: 2 },
                Val::Num(f64::from(e.power)),
            ));
            let fl = e.flags;
            let uses = |out: &mut Vec<Row>, key: &str, label: &str, bit: u16| {
                out.push(Row::new(
                    format!("environment.uses.{key}"),
                    g,
                    &format!("Sets {label}"),
                    Kind::Bool,
                    Val::Bool(fl & bit != 0),
                ));
            };
            uses(out, "view_distance", "View Distance", env_flags::VIEW_DIST);
            if fl & env_flags::VIEW_DIST != 0 {
                out.push(
                    Row::new(
                        "environment.view_distance",
                        g,
                        "View Distance",
                        Kind::Float { min: 0.0001, max: 10000.0, dec: 2 },
                        Val::Num(f64::from(e.view_dist)),
                    )
                    .unit("m"),
                );
            }
            uses(out, "fog_color", "Fog Color", env_flags::FOG_COLOR);
            if fl & env_flags::FOG_COLOR != 0 {
                out.push(Row::new(
                    "environment.fog_color",
                    g,
                    "Fog Color",
                    Kind::Color,
                    Val::Color(e.fog_color),
                ));
            }
            uses(out, "fog_density", "Fog Density", env_flags::FOG_DENSITY);
            if fl & env_flags::FOG_DENSITY != 0 {
                out.push(Row::new(
                    "environment.fog_density",
                    g,
                    "Fog Density",
                    Kind::Float { min: 0.0, max: 10000.0, dec: 2 },
                    Val::Num(f64::from(e.fog_density)),
                ));
            }
            for (key, label, bit, v) in [
                ("ambient_color", "Ambient Color", env_flags::AMBIENT_COLOR, e.ambient_color),
                ("sky_color", "Sky Color", env_flags::SKY_COLOR, e.sky_color),
                ("hemi_color", "Sky Light Color", env_flags::HEMI_COLOR, e.hemi_color),
            ] {
                uses(out, key, label, bit);
                if fl & bit != 0 {
                    out.push(Row::new(format!("environment.{key}"), g, label, Kind::Color, Val::Color(v)));
                }
            }
        }
        (ObjData::Way(d), "points") => {
            for p in &d.points {
                let g = format!("Points\\{}", p.name);
                let k = |x: &str| format!("points.{}.{x}", p.name);
                out.push(
                    Row::new(
                        k("pos"),
                        &g,
                        "Position",
                        Kind::Vector { min: -100_000.0, max: 100_000.0, dec: 2 },
                        Val::Vec3(p.position),
                    )
                    .unit("m"),
                );
                out.push(
                    Row::new(
                        k("flags"),
                        &g,
                        "Flags",
                        Kind::Int { min: 0, max: i64::from(u32::MAX) },
                        Val::Num(f64::from(p.flags)),
                    )
                    .help("What the game's logic reads at the point (a number of bits)."),
                );
                let links: Vec<String> = p
                    .links
                    .iter()
                    .map(|(to, prob)| {
                        let n =
                            d.points.get(*to as usize).map_or_else(|| to.to_string(), |q| q.name.to_string());
                        if *prob == 1.0 { n } else { format!("{n} ({prob})") }
                    })
                    .collect();
                out.push(caption(&k("links"), &g, "Leads To", &links.join(", ")));
            }
        }
        (ObjData::Sector(d), "items") => {
            let objects = d.items.iter().map(|(o, _)| o).collect::<std::collections::BTreeSet<_>>().len();
            out.push(caption("items.objects", "Contents", "Objects", &objects.to_string()));
            out.push(caption("items.meshes", "Contents", "Meshes", &d.items.len().to_string()));
            out.push(caption(
                "items.faces",
                "Contents",
                "Faces",
                &sector_faces(state, cx.lib, d).to_string(),
            ));
        }
        (ObjData::Portal(d), "vertices") => {
            out.push(caption("vertices", "Portal", "Corners", &d.vertices.len().to_string()));
        }
        (ObjData::Group(g), "members") => {
            out.push(
                button("group.freeze", "Group", "Members", "All Editable,None Editable")
                    .help("Makes every member editable by itself, or none (reloading the group keeps an editable member as it is)."),
            );
            for m in g.members.iter().filter_map(|m| state.get(*m)) {
                out.push(Row::new(
                    format!("members.{}", m.name),
                    "Group\\Members",
                    &format!("{} ({})", m.name, m.class.desc()),
                    Kind::Bool,
                    Val::Bool(m.flags.co & co::IN_GROUP_UNIQUE != 0),
                ));
            }
        }
        _ => {}
    }
}

/// A row of the spawn entity's property sheet: its key `props.<name>`, its label in plain words.
fn entity_row(p: &gz_entities::Prop, ctx: &PropContext) -> Row {
    let k = format!("props.{}", p.key());
    let (group, label, unit) = p.display();
    let group = if group.is_empty() { "Properties".to_owned() } else { group };
    let story = |names: &std::collections::BTreeMap<u32, SmolStr>, v: i64| {
        let items = names.iter().map(|(id, n)| (n.clone(), i64::from(*id))).collect();
        (Kind::Token(items), Val::Num(v as f64))
    };
    let (kind, val) = match (&p.kind, &p.value) {
        (PropKind::ListToken { list: ListKind::StoryIds, .. }, PropValue::Int(v))
            if !ctx.story_names.is_empty() =>
        {
            story(&ctx.story_names, *v)
        }
        (PropKind::ListToken { list: ListKind::SpawnStoryIds, .. }, PropValue::Int(v))
            if !ctx.spawn_story_names.is_empty() =>
        {
            story(&ctx.spawn_story_names, *v)
        }
        (PropKind::ListToken { list: ListKind::Locations(i), .. }, PropValue::Int(v)) => {
            let items = ctx.locations.get(usize::from(*i)).map_or_else(Vec::new, |l| l.clone());
            (Kind::Token(items), Val::Num(*v as f64))
        }
        (PropKind::U8 { min, max, .. }, PropValue::Int(v)) => {
            (Kind::Int { min: i64::from(*min), max: i64::from(*max) }, Val::Num(*v as f64))
        }
        (PropKind::U16 { min, max, .. }, PropValue::Int(v)) => {
            (Kind::Int { min: i64::from(*min), max: i64::from(*max) }, Val::Num(*v as f64))
        }
        (PropKind::U32 { min, max, .. }, PropValue::Int(v)) => {
            (Kind::Int { min: i64::from(*min), max: i64::from(*max) }, Val::Num(*v as f64))
        }
        (PropKind::Float { min, max, decimals, .. }, PropValue::Float(v)) => {
            (Kind::Float { min: *min, max: *max, dec: *decimals }, Val::Num(f64::from(*v)))
        }
        (PropKind::Angle { min, max }, PropValue::Float(v)) => {
            (Kind::Angle { min: *min, max: *max, dec: 2 }, Val::Num(f64::from(*v)))
        }
        (PropKind::Bool, PropValue::Bool(b)) => (Kind::Bool, Val::Bool(*b)),
        (PropKind::Flag { off, on, .. }, PropValue::Bool(b)) => match (off, on) {
            (Some(off), Some(on)) => {
                (Kind::Flag, Val::Text(if *b { (*on).to_owned() } else { (*off).to_owned() }))
            }
            _ => (Kind::Bool, Val::Bool(*b)),
        },
        (PropKind::Token { items, .. }, PropValue::Int(v)) => (
            Kind::Token(
                items.iter().map(|(n, i)| (SmolStr::new(gz_entities::word_of(n)), i64::from(*i))).collect(),
            ),
            Val::Num(*v as f64),
        ),
        (PropKind::Vec3, PropValue::Vec3(v)) => {
            (Kind::Vector { min: 0.0, max: 0.0, dec: 2 }, Val::Vec3(Vec3::from(*v)))
        }
        (PropKind::Color, PropValue::Int(v)) => (Kind::Color, Val::Color(*v as u32)),
        (PropKind::Choose { choose: c, param, max_items, .. }, PropValue::Text(t)) => {
            let choice = Choice {
                list: SmolStr::new(format!("{c:?}").to_lowercase()),
                param: param.as_deref().map(SmolStr::new),
                max: (*max_items).max(1),
            };
            (Kind::Choose(choice), Val::Text(t.clone()))
        }
        (_, PropValue::Text(t)) => (Kind::Text, Val::Text(t.clone())),
        (_, PropValue::Int(v)) => (Kind::Int { min: 0, max: 0 }, Val::Num(*v as f64)),
        (_, PropValue::Float(v)) => (Kind::Float { min: 0.0, max: 0.0, dec: 2 }, Val::Num(f64::from(*v))),
        (_, PropValue::Bool(b)) => (Kind::Bool, Val::Bool(*b)),
        (_, PropValue::Vec3(v)) => (Kind::Vector { min: 0.0, max: 0.0, dec: 2 }, Val::Vec3(Vec3::from(*v))),
    };
    let mut r = Row::new(k, &group, &label, kind, val).unit(unit);
    if p.read_only {
        r.enabled = false;
    }
    r
}

// ---------------------------------------------------------------- the scene

/// Level Settings: the scene's own keys a person sets.
#[must_use]
pub fn world_rows(state: &SceneState) -> Vec<Row> {
    let custom = state.meta.level.build.quality == 2;
    let mut out = Vec::new();
    let file = state.meta.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
    out.push(caption("file", "Scene", "File", &file).help("The file the scene is saved to."));
    for f in crate::format::SCENE_FIELDS {
        if f.key.starts_with("camera.")
            || matches!(
                f.key,
                "classes"
                    | "view_flags"
                    | "random_placement"
                    | "layers"
                    | "build.reserved"
                    | "build.reserved_word"
                    | "light_controls"
            )
        {
            continue;
        }
        let Some(mut r) = field_row(f, state) else { continue };
        let preset = matches!(
            f.key,
            "build.sky_quality" | "build.sun_quality" | "build.pixels_per_meter" | "build.jitter_samples"
        );
        if preset && !custom {
            r.enabled = false;
            r.help.push_str(" Set by the quality; custom sets it by hand.");
        }
        out.push(r);
    }
    if let Some(l) = state.tools.part(ObjClass::Light).light() {
        let user: Vec<&str> = l
            .lcontrols
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !matches!(*n, "$static" | "$hemi" | "$sun"))
            .collect();
        out.push(
            caption(
                "light_controls",
                "Lights",
                "Light Controls",
                &if user.is_empty() { "none of its own".to_owned() } else { user.join(", ") },
            )
            .help("The scene's own groups of lights; a light picks its group in its Light Control."),
        );
    }
    out
}

// ---------------------------------------------------------------- lists of rows

/// A row of the merged list the panel and `scene.get` answer.
#[derive(Clone, Debug, PartialEq)]
pub struct MergedRow {
    pub row: Row,
    pub mixed: bool,
}

/// The panel's order: inside every section its own rows first, in the order they came, then its
/// sub-sections in the order each first came (by the rows' paths).
#[must_use]
pub fn tree_order(rows: Vec<MergedRow>) -> Vec<MergedRow> {
    #[derive(Default)]
    struct Node {
        rows: Vec<usize>,
        folders: Vec<(String, Node)>,
    }
    fn place(node: &mut Node, parts: &[&str], i: usize) {
        match parts {
            [] | [_] => node.rows.push(i),
            [head, rest @ ..] => {
                let at = match node.folders.iter().position(|(n, _)| n == head) {
                    Some(at) => at,
                    None => {
                        node.folders.push(((*head).to_owned(), Node::default()));
                        node.folders.len() - 1
                    }
                };
                if let Some((_, child)) = node.folders.get_mut(at) {
                    place(child, rest, i);
                }
            }
        }
    }
    fn flatten(node: &Node, out: &mut Vec<usize>) {
        out.extend(&node.rows);
        for (_, child) in &node.folders {
            flatten(child, out);
        }
    }
    let mut root = Node::default();
    for (i, r) in rows.iter().enumerate() {
        let parts: Vec<&str> = r.row.path.split('\\').collect();
        place(&mut root, &parts, i);
    }
    let mut order = Vec::with_capacity(rows.len());
    flatten(&root, &mut order);
    let mut slots: Vec<Option<MergedRow>> = rows.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots.get_mut(i).and_then(Option::take)).collect()
}

/// Rows of one key over several objects become one row, "mixed" when their values differ; the
/// order is the first object's, rows the first lacks follow.
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
        // one name cannot be given to all of them
        if m.mixed && m.row.kind == Kind::Name {
            m.row.enabled = false;
        }
    }
    out
}

/// The label of a key as the panel shows it, when only the key is at hand: its last word in
/// Title Case.
#[must_use]
pub fn label_of(k: &str) -> String {
    word_label(k.rsplit(['.', '\\']).next().unwrap_or(k))
}

/// A number's unit.
#[must_use]
pub fn unit_of(r: &Row) -> String {
    match r.kind {
        Kind::Float { .. }
        | Kind::Int { .. }
        | Kind::Angle { .. }
        | Kind::Time
        | Kind::Vector { .. }
        | Kind::Caption => {}
        _ => return String::new(),
    }
    if !r.unit.is_empty() {
        return r.unit.to_owned();
    }
    if matches!(r.kind, Kind::Angle { .. }) { "\u{b0}".to_owned() } else { String::new() }
}

/// What the row means, and the range a number takes, in its unit.
#[must_use]
pub fn help_of(r: &Row) -> String {
    let unit = unit_of(r);
    let suffix =
        if unit.is_empty() || unit == "\u{b0}" || unit == "%" { unit.clone() } else { format!(" {unit}") };
    let mut help = r.help.clone();
    let range = match r.kind {
        Kind::Float { min, max, .. } if min < max => {
            format!("Range: {} to {}{suffix}.", fmt_g(min), fmt_g(max))
        }
        Kind::Angle { min, max, .. } if min < max => {
            format!("Range: {} to {}{suffix}.", fmt_g(min.to_degrees()), fmt_g(max.to_degrees()))
        }
        Kind::Int { min, max } if min < max => format!("Range: {min} to {max}{suffix}."),
        _ => String::new(),
    };
    if !range.is_empty() {
        if !help.is_empty() {
            help.push(' ');
        }
        help.push_str(&range);
    }
    help
}

/// A row by its key, else by its label (any case).
#[must_use]
pub fn find<'a>(rows: &'a [MergedRow], k: &str) -> Option<&'a MergedRow> {
    rows.iter()
        .find(|r| r.row.key == k)
        .or_else(|| rows.iter().find(|r| r.row.path == k))
        .or_else(|| rows.iter().find(|r| !k.is_empty() && r.row.label.eq_ignore_ascii_case(k)))
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
    match t.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" => Some(true),
        "false" | "0" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn parse_vec(t: &str) -> Option<Vec3> {
    let cleaned: String =
        t.chars().map(|c| if matches!(c, '{' | '}' | '[' | ']' | ',' | ';') { ' ' } else { c }).collect();
    let parts: Vec<f32> =
        cleaned.split_whitespace().map(|p| p.parse::<f32>()).collect::<Result<_, _>>().ok()?;
    (parts.len() == 3).then(|| Vec3::new(parts[0], parts[1], parts[2]))
}

fn parse_color(t: &str) -> Option<(u32, bool)> {
    let s = t.trim();
    let s =
        s.strip_prefix('#').or_else(|| s.strip_prefix("0x")).or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    if s.len() != 6 && s.len() != 8 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(if s.len() == 6 { (v | 0xff00_0000, false) } else { (v, true) })
}

fn find_name(t: &str, names: &[SmolStr]) -> Result<usize, String> {
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(t.trim())) {
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

/// A value as text for a row: the value, or the sentence that says what the row takes (after
/// its label).
pub fn parse(r: &Row, t: &str) -> Result<Parsed, String> {
    if !r.enabled {
        return Err("cannot be changed here".into());
    }
    match &r.kind {
        Kind::Float { min, max, .. } | Kind::Angle { min, max, .. } => {
            let unit = unit_of(r);
            let mut n = t.trim();
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
            if min < max && (v < lo - 1e-6 || v > hi + 1e-6) {
                return Err(format!("takes a number from {} to {}", fmt_g(lo as f32), fmt_g(hi as f32)));
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
        Kind::Time => {
            let parts: Vec<&str> = t.trim().split(':').collect();
            let secs = match parts.as_slice() {
                [h, m, s] => h
                    .parse::<f64>()
                    .ok()
                    .zip(m.parse::<f64>().ok())
                    .zip(s.parse::<f64>().ok())
                    .map(|((h, m), s)| h * 3600.0 + m * 60.0 + s),
                [s] => s.parse::<f64>().ok(),
                _ => None,
            };
            secs.map(Parsed::Num).ok_or_else(|| "takes a time: hh:mm:ss or seconds".to_owned())
        }
        Kind::Bool | Kind::Flag => {
            parse_bool(t).map(Parsed::Bool).ok_or_else(|| "takes true or false".to_owned())
        }
        Kind::Vector { min, max, .. } => {
            let v = parse_vec(t).ok_or_else(|| "takes three numbers: x, y, z".to_owned())?;
            Ok(Parsed::Vec3(if min < max { v.clamp(Vec3::splat(*min), Vec3::splat(*max)) } else { v }))
        }
        Kind::Angle3 { .. } => {
            let v = parse_vec(t).ok_or_else(|| "takes three angles in degrees: x, y, z".to_owned())?;
            Ok(Parsed::Vec3(Vec3::new(
                values::radians(f64::from(v.x)),
                values::radians(f64::from(v.y)),
                values::radians(f64::from(v.z)),
            )))
        }
        Kind::Color | Kind::FColor => {
            let (argb, alpha) = parse_color(t).ok_or_else(|| "takes a colour: #RRGGBB".to_owned())?;
            Ok(Parsed::Color { argb, alpha })
        }
        Kind::Token(items) => {
            let names: Vec<SmolStr> = items.iter().map(|(n, _)| n.clone()).collect();
            match find_name(t, &names) {
                Ok(i) => Ok(Parsed::Num(items.get(i).map_or(0, |(_, id)| *id) as f64)),
                // a file's word (`spot`) or the number is taken as well
                Err(e) => {
                    let w = word_label(t.trim());
                    match names.iter().position(|n| n.eq_ignore_ascii_case(&w)) {
                        Some(i) => Ok(Parsed::Num(items.get(i).map_or(0, |(_, id)| *id) as f64)),
                        None => t
                            .trim()
                            .parse::<f64>()
                            .ok()
                            .filter(|v| items.iter().any(|(_, id)| *id as f64 == *v))
                            .map(Parsed::Num)
                            .ok_or(e),
                    }
                }
            }
        }
        Kind::Button(c) => find_name(t, c).map(Parsed::Press),
        Kind::Text | Kind::Name | Kind::Choose(_) => Ok(Parsed::Text(t.to_owned())),
        Kind::Caption | Kind::Canvas => Err("cannot be set: it only shows a value".into()),
    }
}

/// What setting a row of an object does.
// one per property set, handed straight to the store: boxing the object would only add a copy
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The object changed (and nothing else).
    Object(Object),
    /// A transform row: applied with the group rules.
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

fn text_of(p: &Parsed) -> String {
    match p {
        Parsed::Text(t) => t.clone(),
        _ => String::new(),
    }
}

fn vec_of(p: &Parsed) -> Vec3 {
    match p {
        Parsed::Vec3(v) => *v,
        _ => Vec3::ZERO,
    }
}

fn color_of(p: &Parsed, old: u32) -> u32 {
    match p {
        Parsed::Color { argb, alpha: true } => *argb,
        Parsed::Color { argb, alpha: false } => (*argb & 0x00ff_ffff) | (old & 0xff00_0000),
        _ => old,
    }
}

/// A motion row as an edit: `Some(None)` for what the row leaves to its caller (Play, Pause,
/// Import, Export, the two key dialogs).
fn motion_edit(k: &str, p: &Parsed) -> Option<Option<crate::objmotion::MotionEdit>> {
    use crate::objmotion::MotionEdit as E;
    let press = match p {
        Parsed::Press(i) => Some(*i),
        _ => None,
    };
    Some(match k {
        "motion.on" => Some(E::Motionable(on(p))),
        "motion.files" => None,
        "motion.commands" => match press {
            Some(0) => Some(E::AddKey),
            Some(1) => Some(E::DeleteKey),
            Some(4) => Some(E::Clamp),
            _ => None,
        },
        "motion.controls" => match press {
            Some(0) => Some(E::FirstFrame),
            Some(1) => Some(E::PreviousKey),
            Some(2) => Some(E::PreviousFrame),
            Some(5) => Some(E::NextFrame),
            Some(6) => Some(E::NextKey),
            Some(7) => Some(E::LastFrame),
            _ => None,
        },
        "motion.auto_key" => Some(E::AutoKey(on(p))),
        "motion.camera_view" => Some(E::CameraView(on(p))),
        "motion.start" => Some(E::Range { min_t: Some(num(p) as f32), max_t: None }),
        "motion.end" => Some(E::Range { min_t: None, max_t: Some(num(p) as f32) }),
        "motion.current" => Some(E::Current(num(p) as f32)),
        "motion.key_time" => Some(E::KeyTime(num(p) as f32)),
        _ => return None,
    })
}

/// Sets one row of an object (`k` is the row's key).
pub fn set_object(
    state: &SceneState,
    o: &Object,
    k: &str,
    p: &Parsed,
    cx: &PropCx<'_>,
) -> Result<Effect, String> {
    let mut n = o.clone();
    match k {
        "name" => {
            let name = text_of(p).trim().to_owned();
            if name.is_empty() || state.find_any(&name).is_some_and(|x| x.id != o.id) {
                return Err("is the name of another object".into());
            }
            n.name = name.into();
            return Ok(Effect::Object(n));
        }
        "pos" => return Ok(Effect::Transform(TransformEdit::Position(vec_of(p)))),
        "rot" => return Ok(Effect::Transform(TransformEdit::Rotation(vec_of(p)))),
        "scale" => return Ok(Effect::Transform(TransformEdit::Scale(vec_of(p)))),
        _ => {}
    }
    if let Some(edit) = motion_edit(k, p) {
        // Play, Pause and the files are the caller's: they need the frame clock or a file
        let Some(edit) = edit else { return Ok(Effect::Nothing) };
        let e = crate::objmotion::apply(o, &edit)?;
        n.motion = e.motion;
        n.set_co_flags(e.flags);
        if let Some((pos, rot)) = e.pose {
            n.xform.position = pos;
            n.xform.rotation = rot;
            return Ok(Effect::Objects(vec![n]));
        }
        return Ok(Effect::Object(n));
    }
    // a key of the files
    if let Some(f) = fields::object_field(o.class, k)
        && !matches!(f.ty, Ty::Table(_) | Ty::Range)
    {
        if k == "control" {
            if let ObjData::Light(d) = &mut n.data {
                d.lcontrol = num(p) as u32;
            }
            return Ok(Effect::Object(n));
        }
        let row = field_row(f, o).ok_or("is no row of this object")?;
        let v = match &row.kind {
            Kind::Token(_) => word_value(f, num(p)),
            _ => value_of(&row, p)?,
        };
        (f.set)(&mut n, &v)?;
        return Ok(Effect::Object(n));
    }
    match &mut n.data {
        ObjData::SceneObject(d) => {
            if k == "surfaces.reset" {
                d.flags &= !SO_USE_SURFACE;
                d.surfaces.clear();
            } else if let Some(rest) = k.strip_prefix("surfaces.") {
                let (sname, field) = rest.rsplit_once('.').ok_or("is no row of this object")?;
                if d.flags & SO_USE_SURFACE == 0 {
                    d.surfaces = library_surfaces(cx.lib, &d.reference);
                    d.flags |= SO_USE_SURFACE;
                }
                let s = d.surfaces.iter_mut().find(|s| s.name == sname).ok_or("is no row of this object")?;
                let v: SmolStr = text_of(p).into();
                match field {
                    "texture" => s.texture = v,
                    "shader" => s.shader = v,
                    "compile" => s.shader_xrlc = v,
                    "material" => s.game_mtl = v,
                    _ => return Err("is no row of this object".into()),
                }
            } else {
                return Err("is no row of this object".into());
            }
        }
        ObjData::Light(d) => match k {
            "attenuation.auto" => {
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
            "fuzzy.on" => {
                set_bit(&mut d.flags, light_flags::POINT_FUZZY, on(p));
                d.fuzzy = on(p).then(Default::default);
            }
            "fuzzy.count" => {
                if let Some(f) = &mut d.fuzzy {
                    f.points.resize(num(p).max(1.0) as usize, Vec3::ZERO);
                }
            }
            "fuzzy.shape" => {
                if let Some(f) = &mut d.fuzzy {
                    f.shape = num(p) as u8;
                }
            }
            "fuzzy.radius" => {
                if let Some(f) = &mut d.fuzzy {
                    f.sphere_radius = num(p) as f32;
                }
            }
            "fuzzy.box" => {
                if let Some(f) = &mut d.fuzzy {
                    f.box_dim = vec_of(p);
                }
            }
            "fuzzy.generate" => return Ok(Effect::Nothing),
            _ => return Err("is no row of this object".into()),
        },
        ObjData::SoundSrc(d) => {
            let slot = match k {
                "pause.from" => &mut d.random_pause[0],
                "pause.to" => &mut d.random_pause[1],
                "active.from" => &mut d.active_time[0],
                "active.to" => &mut d.active_time[1],
                "play.from" => &mut d.play_time[0],
                "play.to" => &mut d.play_time[1],
                "sound.controls" => return Ok(Effect::Nothing),
                _ => return Err("is no row of this object".into()),
            };
            *slot = num(p) as f32;
        }
        ObjData::Particles(_) if k == "particles.controls" => return Ok(Effect::Nothing),
        ObjData::Way(w) => {
            let rest = k.strip_prefix("points.").ok_or("is no row of this object")?;
            let (pname, field) = rest.rsplit_once('.').ok_or("is no row of this object")?;
            let pt = w.points.iter_mut().find(|q| q.name == pname).ok_or("is no point of the way")?;
            match field {
                "pos" => pt.position = vec_of(p),
                "flags" => pt.flags = num(p) as u32,
                _ => return Err("cannot be set".into()),
            }
        }
        ObjData::Group(g) => {
            if k == "group.freeze" {
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
            }
            let name = k.strip_prefix("members.").ok_or("is no row of this object")?;
            let member = g
                .members
                .iter()
                .filter_map(|m| state.get(*m))
                .find(|m| m.name == name)
                .ok_or("is no member of the group")?;
            let mut m = (**member).clone();
            set_bit(&mut m.flags.co, co::IN_GROUP_UNIQUE, on(p));
            return Ok(Effect::Objects(vec![m]));
        }
        ObjData::Spawn(d) => {
            if let Some(field) = k.strip_prefix("order.") {
                let o = &mut d.order;
                match field {
                    "kind" => o.kind = num(p) as u8,
                    "look" => o.look = text_of(p).into(),
                    "route" => o.route = text_of(p).into(),
                    "target" => o.target = text_of(p).into(),
                    "radius" => o.radius = num(p) as f32,
                    "distance" => o.distance = num(p) as f32,
                    "run" => {
                        o.flags = if on(p) {
                            o.flags | crate::objects::ORDER_RUN
                        } else {
                            o.flags & !crate::objects::ORDER_RUN
                        }
                    }
                    "hostile" => {
                        o.flags = if on(p) {
                            o.flags | crate::objects::ORDER_HOSTILE
                        } else {
                            o.flags & !crate::objects::ORDER_HOSTILE
                        }
                    }
                    _ => return Err("is no row of this object".into()),
                }
                if o.kind == 2 && o.route.is_empty() && field == "kind" {
                    // a patrol walks a route: it is picked next
                }
            } else if k == "respawn" {
                d.sd_flags = if on(p) { d.sd_flags | 1 } else { d.sd_flags & !1 };
            } else if let Some(field) = k.strip_prefix("respawn_point.") {
                match field {
                    "team" => d.rpoint.team = num(p) as u8,
                    "kind" => d.rpoint.kind = num(p) as u8,
                    "profile" => d.rpoint.profile = text_of(p).into(),
                    _ => return Err("is no row of this object".into()),
                }
            } else if let Some(field) = k.strip_prefix("environment.") {
                let e = &mut d.env;
                let bit = |name: &str| match name {
                    "view_distance" => Some(env_flags::VIEW_DIST),
                    "fog_color" => Some(env_flags::FOG_COLOR),
                    "fog_density" => Some(env_flags::FOG_DENSITY),
                    "ambient_color" => Some(env_flags::AMBIENT_COLOR),
                    "sky_color" => Some(env_flags::SKY_COLOR),
                    "hemi_color" => Some(env_flags::HEMI_COLOR),
                    _ => None,
                };
                match field {
                    "radius" => e.radius = num(p) as f32,
                    "power" => e.power = num(p) as f32,
                    "view_distance" => e.view_dist = num(p) as f32,
                    "fog_color" => e.fog_color = color_of(p, e.fog_color),
                    "fog_density" => e.fog_density = num(p) as f32,
                    "ambient_color" => e.ambient_color = color_of(p, e.ambient_color),
                    "sky_color" => e.sky_color = color_of(p, e.sky_color),
                    "hemi_color" => e.hemi_color = color_of(p, e.hemi_color),
                    other => {
                        let b =
                            other.strip_prefix("uses.").and_then(bit).ok_or("is no row of this object")?;
                        e.flags = if on(p) { e.flags | b } else { e.flags & !b };
                    }
                }
            } else if k == SPAWN_ANIMATION {
                // the playback runs in the viewport; the scene stays as it is
                return Ok(Effect::Nothing);
            } else if let Some(pk) = k.strip_prefix("props.") {
                let Some(SpawnEntity::Decoded(ent)) = &mut d.entity else {
                    return Err("is kept as the file has it: link the game to change it".into());
                };
                let prop = ent
                    .properties(cx.entity)
                    .into_iter()
                    .find(|x| x.key() == pk)
                    .ok_or("is no property of this spawn")?;
                let value = match (&prop.kind, p) {
                    (PropKind::Float { .. } | PropKind::Angle { .. }, _) => PropValue::Float(num(p) as f32),
                    (PropKind::Bool | PropKind::Flag { .. }, _) => PropValue::Bool(on(p)),
                    (PropKind::Vec3, Parsed::Vec3(v)) => PropValue::Vec3((*v).into()),
                    (PropKind::Color, Parsed::Color { argb, .. }) => PropValue::Int(i64::from(*argb)),
                    (_, Parsed::Num(v)) => PropValue::Int(*v as i64),
                    (_, Parsed::Text(t)) => PropValue::Text(t.clone()),
                    _ => return Err("cannot be set by text".into()),
                };
                std::sync::Arc::make_mut(ent)
                    .set(&prop.path, &value, cx.entity)
                    .map_err(|err| err.to_string())?;
            } else {
                return Err("is no row of this object".into());
            }
        }
        _ => return Err("is no row of this object".into()),
    }
    Ok(Effect::Object(n))
}

/// Sets one row of Level Settings: the level options and the kinds' settings after.
pub fn set_world(state: &SceneState, k: &str, p: &Parsed) -> Result<(LevelOptions, Vec<ToolPart>), String> {
    let f = crate::format::SCENE_FIELDS.iter().find(|f| f.key == k).ok_or("is no row of Level Settings")?;
    let mut s = state.clone();
    if k == "level.path" {
        // the build wipes `gamedata\levels\<this>`: one folder name, never a path
        let t = text_of(p);
        if let Some(why) = gz_base::file_name_problem(&t) {
            return Err(format!("{why}: the level folder is the name of one folder under the levels folder"));
        }
    }
    if k == "build.quality" {
        s.meta.level.set_quality(num(p) as u16);
        return Ok((s.meta.level, Vec::new()));
    }
    let row = field_row(f, state).ok_or("is no row of Level Settings")?;
    let v = match (&row.kind, f.ty) {
        (Kind::Token(_), Ty::Words(words)) => values::word(num(p) as u32, words),
        _ => value_of(&row, p)?,
    };
    (f.set)(&mut s, &v)?;
    let tools: Vec<ToolPart> = ObjClass::ALL
        .iter()
        .filter(|c| s.tools.part(**c) != state.tools.part(**c))
        .map(|c| s.tools.part(*c).clone())
        .collect();
    Ok((s.meta.level, tools))
}

/// The items of a pick list the scene itself holds: its way paths (`way_path`), and for an
/// order's target the player and the story ids its spawn points carry (`story_id`).
#[must_use]
pub fn scene_items(state: &SceneState, list: &str) -> Option<Vec<String>> {
    Some(match list {
        "way_path" => state.of_class(ObjClass::Way).map(|o| o.name.to_string()).collect(),
        "story_id" => {
            let mut out = vec!["actor".to_owned()];
            out.extend(scene_story_ids(state));
            out
        }
        _ => return None,
    })
}

/// The story ids the scene's spawn points carry: `story_id = X` under `[story_object]` of their
/// custom data, or the bare id right under the header.
fn scene_story_ids(state: &SceneState) -> Vec<String> {
    let mut out = Vec::new();
    for o in state.of_class(ObjClass::SpawnPoint) {
        let ObjData::Spawn(sd) = &o.data else { continue };
        let Some(ini) = sd.entity.as_ref().and_then(SpawnEntity::entity).and_then(|e| e.custom_data()) else {
            continue;
        };
        let Some(at) = ini.find("[story_object]") else { continue };
        for line in ini[at + "[story_object]".len()..].lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                break;
            }
            match line.split_once('=') {
                Some((k, v)) if k.trim() == "story_id" => out.push(v.trim().to_owned()),
                Some(_) => {}
                None => out.push(line.to_owned()),
            }
            break;
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

/// The ARGB of a sector's colour, for the renderer.
#[must_use]
pub fn sector_argb(c: [f32; 4]) -> u32 {
    fcolor_get(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{GlowData, LightData, default_data};

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
        let pos = find(&rows, "pos").unwrap();
        assert!(pos.mixed);
        assert_eq!(pos.row.label, "Position");
        assert_eq!(unit_of(&pos.row), "m");
        let range = find(&rows, "range").unwrap();
        assert!(!range.mixed);
        assert_eq!(range.row.text(), "8.00");
        assert_eq!(help_of(&range.row), "How far it reaches. Range: 0.1 to 1000 m.");
        let name = find(&rows, "name").unwrap();
        assert!(name.mixed && !name.row.enabled);
        // a light's colour reads as its RGB, never a transparent ARGB
        assert_eq!(find(&rows, "color").unwrap().row.text(), "#FFFFFF");
        // and a light has no scale to show
        assert!(find(&rows, "scale").is_none());
    }

    #[test]
    fn rows_set_from_text() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default())));
        let e = cx();
        let pcx = PropCx { lib: None, entity: &e };
        let rows = merge(vec![object_rows(&s, s.get(id).unwrap(), &pcx)]);
        let r = &find(&rows, "radius").unwrap().row;
        assert_eq!(parse(r, "abc"), Err("takes a number".into()));
        assert_eq!(parse(r, "20000"), Err("takes a number from 0.01 to 10000".into()));
        let p = parse(r, "2.5").unwrap();
        let Effect::Object(o) = set_object(&s, s.get(id).unwrap(), "radius", &p, &pcx).unwrap() else {
            panic!()
        };
        let ObjData::Glow(g) = &o.data else { panic!() };
        assert_eq!(g.radius, 2.5);
        // a glow neither turns nor scales: no transform rows but its position
        assert!(find(&rows, "rot").is_none() && find(&rows, "scale").is_none());
        let pos = &find(&rows, "pos").unwrap().row;
        assert_eq!(parse(pos, "1, 2, 3"), Ok(Parsed::Vec3(Vec3::new(1.0, 2.0, 3.0))));
    }

    #[test]
    fn a_light_takes_its_kind_as_a_word() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(ObjClass::Light, "l", ObjData::Light(LightData::default())));
        let e = cx();
        let pcx = PropCx { lib: None, entity: &e };
        let rows = merge(vec![object_rows(&s, s.get(id).unwrap(), &pcx)]);
        let kind = &find(&rows, "kind").unwrap().row;
        assert_eq!(kind.text(), "Point");
        let p = parse(kind, "spot").unwrap();
        let Effect::Object(o) = set_object(&s, s.get(id).unwrap(), "kind", &p, &pcx).unwrap() else {
            panic!()
        };
        let ObjData::Light(d) = &o.data else { panic!() };
        assert_eq!(d.kind, crate::objects::LT_SPOT);
    }

    #[test]
    fn every_row_has_a_label_in_title_case() {
        let mut s = SceneState::default();
        let e = cx();
        let pcx = PropCx { lib: None, entity: &e };
        for c in ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool()) {
            let id = s.insert(Object::new(c, &format!("{}_01", c.key()), default_data(c)));
            for r in object_rows(&s, s.get(id).unwrap(), &pcx) {
                assert!(!r.label.is_empty(), "{c:?} {}", r.key);
                let first = r.label.chars().next().unwrap();
                assert!(first.is_uppercase() || first.is_ascii_digit(), "{c:?} {}: '{}'", r.key, r.label);
            }
        }
        for r in world_rows(&s) {
            assert!(!r.label.is_empty() && r.label.chars().next().unwrap().is_uppercase(), "{}", r.key);
        }
        for f in fields::COMMON.iter() {
            assert!(!f.label.is_empty() && !f.help.is_empty(), "{}", f.key);
        }
        for c in ObjClass::ALL {
            for f in fields::class_fields(c) {
                assert!(!f.label.is_empty() && !f.help.is_empty(), "{c:?} {}", f.key);
            }
        }
        for f in crate::format::SCENE_FIELDS {
            assert!(!f.label.is_empty() && !f.help.is_empty(), "{}", f.key);
        }
    }

    #[test]
    fn the_level_folder_is_one_folder_name() {
        let s = SceneState::default();
        for bad in ["..\\..\\Windows", "C:\\Users", "a\\b", "..", "nul", ""] {
            let err = set_world(&s, "level.path", &Parsed::Text(bad.to_owned())).unwrap_err();
            assert!(err.contains("one folder"), "{bad:?}: {err}");
        }
        let (lo, _) = set_world(&s, "level.path", &Parsed::Text("l01_escape".to_owned())).unwrap();
        assert_eq!(lo.level_path, "l01_escape");
        let w = world_rows(&s);
        let weld = w.iter().find(|r| r.key == "build.weld_distance").unwrap();
        assert_eq!(unit_of(weld), "m");
        assert_eq!(weld.label, "Weld Distance");
    }
}

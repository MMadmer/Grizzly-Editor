//! What a level takes from a module as text: `levels\<level>\overlay.things`, which the game
//! reads beside the level's own files (its `XMS_ARCHITECTURE.md`, 8.5; a game says
//! `level-things` among its abilities).
//!
//! - Over a level of the game ([`Over::Game`]) it holds everything the module adds beside its
//!   picture, its collision and its spawns: the sounds, the particle effects, the lights that
//!   shine in the game, the environment modifiers, the glows, the fog volumes and the sound
//!   environments.
//! - On a level of the module's own ([`Over::Own`]) the level's own files carry most of that,
//!   and the file holds what they cannot: the glows (the game draws none of a level's own
//!   file), the sound environments (the level compiler makes no file of them) and the spot
//!   lights that shine in the game (the game loads every light of a level's own file as a
//!   point light).
//!
//! Blocks of `[kind:name]` with `key = value` lines, numbers separated by commas, metres and
//! radians as the game holds them. Nothing of the level's own is replaced: the game puts these
//! beside what the level has.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gz_scene::math::{Mat4, Vec3, box_parts};
use gz_scene::objects::{
    CF_BOX, FV_EMITTER, FV_OCCLUSION, GlowShader, LT_DIRECT, LT_SPOT, PointKind, ShapeData, env_flags,
    glow_shader, light_flags,
};
use gz_scene::{ObjData, Object, SceneState};

/// The ability a game states when it reads the file.
pub const ABILITY: &str = "level-things";

/// Whose level the file is written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Over {
    /// A level of the game: what the scene marks as the module's own.
    Game,
    /// A level of the module's own: every object of the scene.
    Own,
}

/// How many things of each kind the file holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub sounds: u32,
    pub particles: u32,
    pub lights: u32,
    pub envs: u32,
    pub glows: u32,
    pub fogs: u32,
    pub sound_envs: u32,
}

impl Counts {
    #[must_use]
    pub fn total(&self) -> u32 {
        self.sounds + self.particles + self.lights + self.envs + self.glows + self.fogs + self.sound_envs
    }

    /// The counts by the names the answers use.
    #[must_use]
    pub fn named(&self) -> [(&'static str, u32); 7] {
        [
            ("sounds", self.sounds),
            ("particles", self.particles),
            ("lights", self.lights),
            ("envs", self.envs),
            ("glows", self.glows),
            ("fogs", self.fogs),
            ("sound_envs", self.sound_envs),
        ]
    }
}

/// The file with what it holds and what it could not say as the scene has it, a sentence each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Things {
    pub text: String,
    pub counts: Counts,
    pub warnings: Vec<String>,
}

/// `<project>\levels\<level>\overlay.things`.
#[must_use]
pub fn path(root: &Path, level: &str) -> PathBuf {
    crate::gather::level_dir(root, level).join("overlay.things")
}

/// The file's first line: what tells a file the level build made from one somebody wrote.
pub const HEADER: &str = "; What the module adds to the level beside its picture and its spawns. Made by the level build: do not edit.";

/// Whether a file of this text is the level build's own: it starts with the build's line. A
/// file that does not was written by somebody, and the build neither replaces nor removes it.
#[must_use]
pub fn is_builds(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').starts_with(HEADER)
}

/// A name as a line of the file says one - a sound, an effect, a texture, a shader, an
/// environment, a profile: back slashes between its folders. `None` for a name the file cannot
/// say: an empty one, or one with a `;` (the rest of the line would be a comment) or a line
/// break (the rest would be another line of the block).
fn said(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && !name.contains([';', '\r', '\n'])).then(|| name.replace('/', "\\"))
}

/// The sentence for a thing left out for a name the file cannot say.
fn unsayable(what: &str, object: &str, key: &str, value: &str) -> String {
    format!(
        "The {what} {object} is left out: its {key} '{}' cannot be written into the file (a name has no `;` and no line break).",
        value.escape_debug()
    )
}

/// A block's name as the file takes it: the object's, with what would end the header or start
/// a comment there (a bracket, `;`, a line break) replaced - the name only tells the blocks apart.
fn block_name(name: &str) -> String {
    name.chars().map(|c| if matches!(c, ']' | '[' | ';' | '\r' | '\n') { '_' } else { c }).collect()
}

/// The glow's flag for a size that is the same on the screen at any distance.
const GLOW_FIXED_SIZE: u16 = 1 << 0;

fn num(v: f32) -> String {
    // the shortest text that reads back to the value; a zero has no sign
    if v == 0.0 { "0".to_owned() } else { format!("{v}") }
}

fn vec3(v: Vec3) -> String {
    format!("{}, {}, {}", num(v.x), num(v.y), num(v.z))
}

/// A colour `0x00RRGGBB` as three numbers from 0 to 1.
fn colour(c: u32) -> String {
    let part = |shift: u32| num(((c >> shift) & 0xff) as f32 / 255.0);
    format!("{}, {}, {}", part(16), part(8), part(0))
}

/// Where a light of the scene looks, as the level build tells it.
fn direction(heading: f32, pitch: f32) -> Vec3 {
    let (sh, ch) = heading.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

/// A box of the level as the file says one: where its middle is, how it is turned (heading,
/// pitch, bank) and how large it is. `skewed` when the matrix is no straight box, which the game
/// cannot be told.
struct BoxSaid {
    position: Vec3,
    rotation: Vec3,
    size: Vec3,
    skewed: bool,
}

fn box_of(m: &Mat4) -> BoxSaid {
    let b = box_parts(m);
    BoxSaid {
        position: b.position,
        rotation: Vec3::new(b.rotation.y, b.rotation.x, b.rotation.z),
        size: b.size,
        skewed: b.skewed,
    }
}

/// The boxes a shape of an object fills in the level. A sphere is said as the box around it.
fn boxes_of(o: &Object, shape: &ShapeData) -> (Vec<BoxSaid>, bool) {
    let world = o.world();
    let mut spheres = false;
    let boxes = shape
        .shapes
        .iter()
        .map(|sh| {
            if sh.kind == CF_BOX {
                box_of(&Mat4::mul_43(&world, &sh.matrix()))
            } else {
                spheres = true;
                let mut around = Mat4::IDENTITY;
                let d = sh.radius() * 2.0;
                (around.0[0][0], around.0[1][1], around.0[2][2]) = (d, d, d);
                let c = sh.center();
                (around.0[3][0], around.0[3][1], around.0[3][2]) = (c.x, c.y, c.z);
                box_of(&Mat4::mul_43(&world, &around))
            }
        })
        .collect();
    (boxes, spheres)
}

fn box_lines(out: &mut String, b: &BoxSaid) {
    let _ = write!(
        out,
        "position = {}\r\nrotation = {}\r\nsize = {}\r\n",
        vec3(b.position),
        vec3(b.rotation),
        vec3(b.size)
    );
}

/// The file for what the scene holds; `None` when it holds none of it.
#[must_use]
pub fn text(scene: &SceneState, over: Over) -> Option<Things> {
    let mut out = format!("{HEADER}\r\n");
    let mut n = Counts::default();
    let mut warnings = Vec::new();
    let game = over == Over::Game;
    let mut names: Vec<&Object> = scene
        .objects
        .values()
        .filter(|o| !o.marks.base && !o.marks.removed && (!game || o.marks.authored))
        .map(|o| &**o)
        .collect();
    names.sort_by_cached_key(|o| o.name.to_lowercase());
    // the fog volumes by group: the emitter, then what it flows around
    let mut fog: BTreeMap<u32, (Option<&Object>, Vec<&Object>)> = BTreeMap::new();
    for o in names {
        let at = o.position();
        match &o.data {
            ObjData::SoundSrc(s) if game && !s.wav.is_empty() => {
                let Some(wav) = said(&s.wav) else {
                    warnings.push(unsayable("sound source", &o.name, "sound", &s.wav));
                    continue;
                };
                let ms = |v: [f32; 2]| {
                    format!("{}, {}", (v[0] * 1000.0).max(0.0) as u32, (v[1] * 1000.0).max(0.0) as u32)
                };
                let _ = write!(
                    out,
                    "\r\n[sound:{}]\r\nname = {}\r\nposition = {}\r\nvolume = {}\r\nfrequency = {}\r\nactive = {}\r\nplay = {}\r\npause = {}\r\n",
                    block_name(&o.name),
                    wav,
                    vec3(at),
                    num(s.volume),
                    num(s.freq),
                    ms(s.active_time),
                    ms(s.play_time),
                    ms(s.random_pause),
                );
                n.sounds += 1;
            }
            ObjData::Particles(p) if game && !p.reference.is_empty() => {
                let Some(effect) = said(&p.reference) else {
                    warnings.push(unsayable("particle effect", &o.name, "effect", &p.reference));
                    continue;
                };
                let r = o.rotation();
                let _ = write!(
                    out,
                    "\r\n[particles:{}]\r\nname = {}\r\nposition = {}\r\nrotation = {}\r\n",
                    block_name(&o.name),
                    effect,
                    vec3(at),
                    // heading, pitch, bank
                    vec3(Vec3::new(r.y, r.x, r.z)),
                );
                n.particles += 1;
            }
            // a light that shines in the game; one that only lights the lightmaps is baked. The
            // game has one directional light, the level's sun: a directional object is none here.
            // A level of the module's own carries its point lights itself.
            ObjData::Light(l)
                if l.flags & light_flags::AFFECT_DYNAMIC != 0
                    && l.kind != LT_DIRECT
                    && (game || l.kind == LT_SPOT) =>
            {
                let r = o.rotation();
                let c = Vec3::new(l.color[0], l.color[1], l.color[2]) * l.brightness;
                let _ = write!(
                    out,
                    "\r\n[light:{}]\r\ntype = {}\r\nposition = {}\r\ndirection = {}\r\ncolor = {}\r\nrange = {}\r\ncone = {}\r\n",
                    block_name(&o.name),
                    if l.kind == LT_SPOT { "spot" } else { "point" },
                    vec3(at),
                    vec3(direction(r.y, r.x)),
                    vec3(c),
                    num(l.range),
                    num(l.cone),
                );
                n.lights += 1;
            }
            ObjData::Spawn(d) if game && d.kind == PointKind::EnvMod => {
                let e = &d.env;
                let _ = write!(
                    out,
                    "\r\n[env:{}]\r\nposition = {}\r\nradius = {}\r\npower = {}\r\n",
                    block_name(&o.name),
                    vec3(at),
                    num(e.radius),
                    num(e.power)
                );
                // a value it does not change is not said
                if e.flags & env_flags::VIEW_DIST != 0 {
                    let _ = write!(out, "far_plane = {}\r\n", num(e.view_dist));
                }
                if e.flags & env_flags::FOG_COLOR != 0 {
                    let _ = write!(out, "fog_color = {}\r\n", colour(e.fog_color));
                }
                if e.flags & env_flags::FOG_DENSITY != 0 {
                    let _ = write!(out, "fog_density = {}\r\n", num(e.fog_density));
                }
                if e.flags & env_flags::AMBIENT_COLOR != 0 {
                    let _ = write!(out, "ambient = {}\r\n", colour(e.ambient_color));
                }
                if e.flags & env_flags::SKY_COLOR != 0 {
                    let _ = write!(out, "sky_color = {}\r\n", colour(e.sky_color));
                }
                if e.flags & env_flags::HEMI_COLOR != 0 {
                    let _ = write!(out, "hemi_color = {}\r\n", colour(e.hemi_color));
                }
                n.envs += 1;
            }
            ObjData::Glow(g) if !g.texture.is_empty() => {
                let Some(texture) = said(&g.texture) else {
                    warnings.push(unsayable("glow", &o.name, "texture", &g.texture));
                    continue;
                };
                let _ = write!(
                    out,
                    "\r\n[glow:{}]\r\nposition = {}\r\nradius = {}\r\ntexture = {texture}\r\n",
                    block_name(&o.name),
                    vec3(at),
                    num(g.radius),
                );
                // the game draws a glow with the lens flares' shader unless it is told the sun's:
                // no other name changes anything there, and none is said
                if glow_shader(&g.shader) == GlowShader::Sun {
                    out.push_str("shader = effects\\sun\r\n");
                }
                if g.flags & GLOW_FIXED_SIZE != 0 {
                    out.push_str("fixed = true\r\n");
                }
                n.glows += 1;
            }
            ObjData::Glow(_) => {
                warnings.push(format!("The glow {} names no texture: it is left out.", o.name))
            }
            ObjData::SoundEnv(e) => {
                if e.inner.is_empty() || e.outer.is_empty() {
                    warnings.push(format!(
                        "The sound environment {} does not name both its inner and its outer environment: it is left out.",
                        o.name
                    ));
                    continue;
                }
                let (Some(inner), Some(outer)) = (said(&e.inner), said(&e.outer)) else {
                    let bad = if said(&e.inner).is_none() { &e.inner } else { &e.outer };
                    warnings.push(unsayable("sound environment", &o.name, "environment", bad));
                    continue;
                };
                let (boxes, spheres) = boxes_of(o, &e.shape);
                if spheres {
                    warnings.push(format!(
                        "The sound environment {} has a sphere: the game takes sound environments as boxes, so the box around it is used.",
                        o.name
                    ));
                }
                for (k, b) in boxes.iter().enumerate() {
                    if b.skewed {
                        warnings.push(format!(
                            "The box of the sound environment {} is skewed (a turned shape in an object scaled unevenly): the game takes it as a straight box.",
                            o.name
                        ));
                    }
                    let name = if k == 0 {
                        block_name(&o.name)
                    } else {
                        format!("{}_{}", block_name(&o.name), k + 1)
                    };
                    let _ = write!(out, "\r\n[sound_env:{name}]\r\n");
                    box_lines(&mut out, b);
                    let _ = write!(out, "inner = {inner}\r\nouter = {outer}\r\n");
                    n.sound_envs += 1;
                }
            }
            // a level of the module's own carries its fog volumes in its own file
            ObjData::FogVolume(f) if game => {
                let group = fog.entry(f.group_id).or_default();
                match f.kind {
                    FV_EMITTER if group.0.is_none() => group.0 = Some(o),
                    FV_OCCLUSION => group.1.push(o),
                    _ => warnings.push(format!(
                        "The fog volume {} is a second emitter of its group: a group has one, it is left out.",
                        o.name
                    )),
                }
            }
            _ => {}
        }
    }
    for (emitter, occluders) in fog.into_values() {
        let Some(o) = emitter else {
            for c in occluders {
                warnings.push(format!(
                    "The fog volume {} occludes a group that has no emitter: it is left out.",
                    c.name
                ));
            }
            continue;
        };
        let ObjData::FogVolume(f) = &o.data else { continue };
        let Some(profile) = said(&f.profile) else {
            warnings.push(unsayable("fog volume", &o.name, "profile", &f.profile));
            continue;
        };
        let _ = write!(out, "\r\n[fog:{}]\r\nprofile = {profile}\r\n", block_name(&o.name));
        box_lines(&mut out, &box_of(&o.world()));
        for c in occluders {
            let b = box_of(&c.world());
            let _ =
                write!(out, "occluder = {}, {}, {}\r\n", vec3(b.position), vec3(b.rotation), vec3(b.size));
        }
        n.fogs += 1;
    }
    (n.total() > 0 || !warnings.is_empty()).then_some(Things { text: out, counts: n, warnings })
}

#[cfg(test)]
mod tests {
    use gz_scene::ObjClass;
    use gz_scene::objects::{LightData, ParticlesData, SoundSrcData};

    use super::*;

    #[test]
    fn the_file_holds_what_the_module_adds_and_nothing_of_the_level() {
        let mut s = SceneState::default();
        let mut own = |class: ObjClass, name: &str, data: ObjData, authored: bool| {
            let mut o = Object::new(class, name, data);
            o.xform.position = Vec3::new(1.0, 2.0, 3.0);
            o.marks.authored = authored;
            s.insert(o);
        };
        let sound = SoundSrcData {
            wav: "ambient/fire".into(),
            volume: 0.5,
            position: Vec3::new(1.0, 2.0, 3.0),
            ..SoundSrcData::default()
        };
        own(ObjClass::SoundSrc, "fire", ObjData::SoundSrc(sound.clone()), true);
        own(ObjClass::SoundSrc, "theirs", ObjData::SoundSrc(sound), false);
        own(
            ObjClass::Particles,
            "smoke",
            ObjData::Particles(ParticlesData {
                reference: "static\\smoke".into(),
                ..ParticlesData::default()
            }),
            true,
        );
        let lamp = LightData { flags: light_flags::AFFECT_DYNAMIC, range: 6.0, ..LightData::default() };
        own(ObjClass::Light, "lamp", ObjData::Light(lamp), true);
        // a light that only lights the lightmaps is the piece's
        let baked = LightData { flags: light_flags::AFFECT_STATIC, ..LightData::default() };
        own(ObjClass::Light, "baked", ObjData::Light(baked), true);
        let Things { text, counts: n, warnings } = text(&s, Over::Game).unwrap();
        assert_eq!(n, Counts { sounds: 1, particles: 1, lights: 1, ..Counts::default() });
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            text.contains("[sound:fire]\r\nname = ambient\\fire\r\nposition = 1, 2, 3\r\nvolume = 0.5\r\n"),
            "{text}"
        );
        assert!(text.contains("[particles:smoke]\r\nname = static\\smoke\r\n"), "{text}");
        assert!(
            text.contains("[light:lamp]\r\ntype = point\r\n") && text.contains("range = 6\r\n"),
            "{text}"
        );
        assert!(!text.contains("theirs") && !text.contains("baked"), "{text}");
        // a name cannot end its block's header or start a comment in it
        assert_eq!(block_name("gate]; x"), "gate__ x");
        // a scene that adds none of it has no file
        assert!(super::text(&SceneState::default(), Over::Game).is_none());
        // a level of the module's own carries its sounds, effects and point lights itself
        assert!(super::text(&s, Over::Own).is_none());
    }

    #[test]
    fn glows_fog_and_sound_environments_are_said_as_places_and_boxes() {
        use gz_scene::objects::{FogData, GlowData, SoundEnvData};
        let mut s = SceneState::default();
        let mut put = |class: ObjClass, name: &str, data: ObjData, pos: Vec3, scale: Vec3, heading: f32| {
            let mut o = Object::new(class, name, data);
            o.xform.position = pos;
            o.xform.scale = scale;
            o.xform.rotation = Vec3::new(0.0, heading, 0.0);
            o.marks.authored = true;
            s.insert(o);
        };
        let glow = GlowData { texture: "glow\\glow_white".into(), radius: 0.4, ..GlowData::default() };
        put(ObjClass::Glow, "lamp_glow", ObjData::Glow(glow), Vec3::new(1.0, 2.0, 3.0), Vec3::ONE, 0.0);
        put(ObjClass::Glow, "bare", ObjData::Glow(GlowData::default()), Vec3::ZERO, Vec3::ONE, 0.0);
        let env =
            SoundEnvData { inner: "room".into(), outer: "open_space".into(), ..SoundEnvData::default() };
        put(
            ObjClass::SoundEnv,
            "hall",
            ObjData::SoundEnv(env),
            Vec3::new(10.0, 1.5, -4.0),
            Vec3::new(8.0, 3.0, 6.0),
            std::f32::consts::FRAC_PI_2,
        );
        let emitter = FogData { group_id: 1, ..FogData::default() };
        let wall = FogData { kind: FV_OCCLUSION, group_id: 1, ..FogData::default() };
        let lost = FogData { kind: FV_OCCLUSION, group_id: 7, ..FogData::default() };
        put(
            ObjClass::FogVolume,
            "mist",
            ObjData::FogVolume(emitter),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(20.0, 2.0, 20.0),
            0.0,
        );
        put(
            ObjClass::FogVolume,
            "mist_wall",
            ObjData::FogVolume(wall),
            Vec3::new(3.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 4.0),
            0.0,
        );
        put(ObjClass::FogVolume, "lost", ObjData::FogVolume(lost), Vec3::ZERO, Vec3::ONE, 0.0);
        let spot = LightData {
            kind: LT_SPOT,
            flags: light_flags::AFFECT_DYNAMIC | light_flags::AFFECT_STATIC,
            ..LightData::default()
        };
        put(ObjClass::Light, "beam", ObjData::Light(spot), Vec3::new(0.0, 3.0, 0.0), Vec3::ONE, 0.0);
        let sun = LightData { kind: LT_DIRECT, flags: light_flags::AFFECT_DYNAMIC, ..LightData::default() };
        put(ObjClass::Light, "second_sun", ObjData::Light(sun), Vec3::ZERO, Vec3::ONE, 0.0);

        let over = text(&s, Over::Game).unwrap();
        assert_eq!(over.counts, Counts { lights: 1, glows: 1, fogs: 1, sound_envs: 1, ..Counts::default() });
        let t = &over.text;
        assert!(
            t.contains(
                "[glow:lamp_glow]\r\nposition = 1, 2, 3\r\nradius = 0.4\r\ntexture = glow\\glow_white\r\n\r\n"
            ),
            "{t}"
        );
        // the file is the build's by its first line
        assert!(is_builds(t) && !is_builds("; my own notes\r\n[glow:a]\r\n") && !is_builds(""));
        assert!(t.contains("[sound_env:hall]\r\nposition = 10, 1.5, -4\r\n"), "{t}");
        assert!(t.contains("size = 8, 3, 6\r\ninner = room\r\nouter = open_space\r\n"), "{t}");
        // the heading the object was turned by reads back from its box
        let turned = t.split("[sound_env:hall]").nth(1).unwrap().lines().nth(2).unwrap();
        let heading: f32 =
            turned.trim_start_matches("rotation = ").split(',').next().unwrap().trim().parse().unwrap();
        assert!((heading - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{turned}");
        assert!(
            t.contains("[fog:mist]\r\nprofile = environment\\fog\\default.ltx\r\nposition = 0, 1, 0\r\n")
                && t.contains("occluder = 3, 1, 0, 0, 0, 0, 1, 2, 4\r\n"),
            "{t}"
        );
        assert!(t.contains("[light:beam]\r\ntype = spot\r\n") && !t.contains("second_sun"), "{t}");
        assert_eq!(over.warnings.len(), 2, "{:?}", over.warnings);
        // the sun's shader and a fixed size are said; any other shader is the flares', unsaid;
        // a name the file cannot say leaves its thing out with a sentence
        let mut lit = SceneState::default();
        let mut glow = |name: &str, shader: &str, texture: &str, flags: u16| {
            let data = GlowData { shader: shader.into(), texture: texture.into(), radius: 0.1, flags };
            let mut o = Object::new(ObjClass::Glow, name, ObjData::Glow(data));
            o.marks.authored = true;
            lit.insert(o);
        };
        glow("a_sun", "effects/SUN", "glow\\a", GLOW_FIXED_SIZE);
        glow("b_other", "default", "glow\\b", 0);
        glow("c_cut", "", "glow\\c ; fixed = true", 0);
        let made = text(&lit, Over::Own).unwrap();
        let t = &made.text;
        assert!(t.contains("[glow:a_sun]\r\nposition = 0, 0, 0\r\nradius = 0.1\r\ntexture = glow\\a\r\nshader = effects\\sun\r\nfixed = true\r\n"), "{t}");
        assert!(
            t.contains("[glow:b_other]\r\nposition = 0, 0, 0\r\nradius = 0.1\r\ntexture = glow\\b\r\n")
                && !t.contains("default"),
            "{t}"
        );
        assert!(!t.contains("c_cut") && made.counts.glows == 2, "{t}");
        assert!(made.warnings.len() == 1 && made.warnings[0].contains("c_cut"), "{:?}", made.warnings);
        assert_eq!(
            (glow_shader("effects\\glow"), glow_shader("models\\model")),
            (GlowShader::Flare, GlowShader::Other)
        );
        assert!(
            over.warnings.iter().any(|w| w.contains("bare"))
                && over.warnings.iter().any(|w| w.contains("lost"))
        );

        // a level of the module's own: what its own files cannot carry
        let own = text(&s, Over::Own).unwrap();
        assert_eq!(own.counts, Counts { lights: 1, glows: 1, sound_envs: 1, ..Counts::default() });
        assert!(!own.text.contains("[fog:") && own.text.contains("[light:beam]"), "{}", own.text);
    }
}

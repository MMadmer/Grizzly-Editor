//! Game Lit, pure: the weather of the linked
//! game at an hour, the game's render settings, the lights the game would draw and the roof map
//! that tells indoors from out.

use std::sync::Arc;

use glam::Vec3;
use gz_ltx::{Ltx, LtxDoc};
use gz_render::{Environment, Frustum, Light, LightKind, RoofMap};
use gz_scene::library::Library;
use gz_scene::objects::{LT_POINT, light_flags};
use gz_scene::{ObjClass, ObjData, SceneState};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::camera::dir_hp;

/// The most lights a frame of Game Lit draws.
pub const MAX_LIGHTS: usize = 128;
/// The roof map is `ROOF_CELLS`^2 cells of `ROOF_CELL` metres round the camera.
pub const ROOF_CELLS: u32 = 256;
pub const ROOF_CELL: f32 = 1.0;

/// The weather at an hour, as loaded (not yet scaled by the lumscales).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Weather {
    pub ambient: Vec3,
    pub hemi: Vec3,
    pub sun_color: Vec3,
    /// From the sun.
    pub sun_dir: Vec3,
    pub fog_color: Vec3,
    pub sky: WeatherSky,
}

/// The sky of the two sections round the hour, as the game mixes them: both cube maps,
/// how far from the first to the second, the colour and the turn blended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherSky {
    /// `sky_texture` of the section before the hour and of the one after.
    pub textures: [SmolStr; 2],
    pub weight: f32,
    pub color: Vec3,
    /// Radians about the world's Y (`sky_rotation`, degrees in the file).
    pub rotation: f32,
}

/// "22:00:00" -> seconds of the day; `None` for a section that is not a time.
#[must_use]
pub fn section_seconds(name: &str) -> Option<i32> {
    let mut it = name.trim().split(':');
    let (h, m, s) = (it.next()?.trim(), it.next()?.trim(), it.next()?.trim());
    let parse = |t: &str| -> Option<i32> {
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        digits.parse().ok()
    };
    Some(parse(h)? * 3600 + parse(m)? * 60 + parse(s)?)
}

/// The first three numbers of a key (`hemisphere_color` carries a fourth).
fn read_color(doc: &LtxDoc, sect: &str, key: &str) -> Vec3 {
    let Some(text) = doc.get(sect, key) else { return Vec3::ZERO };
    let mut v = [0.0f32; 3];
    for (slot, part) in v.iter_mut().zip(text.split(',')) {
        *slot = part.trim().parse().unwrap_or(0.0);
    }
    Vec3::from(v)
}

/// The sun's direction the way the game builds it from a weather: the altitude goes in as heading.
fn read_sun_dir(doc: &LtxDoc, sect: &str) -> Vec3 {
    let num = |k: &str| doc.get(sect, k).and_then(|t| t.trim().parse::<f32>().ok());
    match (num("sun_altitude"), num("sun_longitude")) {
        (Some(alt), Some(long)) => dir_hp(alt.to_radians(), long.to_radians()),
        _ => Vec3::new(0.0, -1.0, 0.0),
    }
}

/// The weather at an hour: the two sections around it blended, around midnight as the game does.
pub fn weather_at(doc: &LtxDoc, name: &str, hour: f32) -> Result<Weather, String> {
    let mut keys: Vec<(i32, &str)> =
        doc.sections().filter_map(|s| section_seconds(s).map(|t| (t, s))).collect();
    if keys.is_empty() {
        return Err(format!("weather '{name}' has no hh:mm:ss sections"));
    }
    keys.sort_by_key(|k| k.0);
    let day = 24 * 3600;
    let want = ((hour * 3600.0).floor() as i32 % day + day) % day;
    let mut a = keys.len() - 1;
    for (i, k) in keys.iter().enumerate() {
        if k.0 <= want {
            a = i;
        }
    }
    let b = (a + 1) % keys.len();
    let (Some(ka), Some(kb)) = (keys.get(a), keys.get(b)) else {
        return Err(format!("weather '{name}' has no hh:mm:ss sections"));
    };
    let span = ((kb.0 - ka.0) % day + day) % day;
    let f = if span != 0 { (((want - ka.0) % day + day) % day) as f32 / span as f32 } else { 0.0 };
    let lerp = |key: &str| read_color(doc, ka.1, key).lerp(read_color(doc, kb.1, key), f);
    let mut sun_dir = read_sun_dir(doc, ka.1).lerp(read_sun_dir(doc, kb.1), f);
    if sun_dir.length_squared() < 1e-6 {
        sun_dir = Vec3::new(0.0, -1.0, 0.0);
    }
    let texture = |sect: &str| SmolStr::new(doc.get(sect, "sky_texture").map_or("", str::trim));
    let rotation = |sect: &str| {
        doc.get(sect, "sky_rotation").and_then(|t| t.trim().parse::<f32>().ok()).map_or(0.0, f32::to_radians)
    };
    let sky = WeatherSky {
        textures: [texture(ka.1), texture(kb.1)],
        weight: f,
        color: lerp("sky_color"),
        rotation: (1.0 - f) * rotation(ka.1) + f * rotation(kb.1),
    };
    Ok(Weather {
        ambient: lerp("ambient_color"),
        hemi: lerp("hemisphere_color"),
        sun_color: lerp("sun_color"),
        sun_dir: sun_dir.normalize(),
        fog_color: lerp("fog_color"),
        sky,
    })
}

/// The weather a new game's first night has: Atmosfear's, `af3_slight_clear_<moon phase>`,
/// the phase counting the days since 2010 from alife's `start_date` (one more from noon) in an
/// 8-day cycle; else the first clear weather, else the first one.
#[must_use]
pub fn default_weather(names: &[String], start_date: Option<&str>, hour: f32) -> String {
    if let Some(date) = start_date {
        let parts: Vec<i32> = date.split('.').filter_map(|p| p.trim().parse().ok()).collect();
        if let [d, m, y] = parts[..] {
            const MONTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let mut day = 365 * (y - 2010) + d;
            for mm in 1..m.min(13) {
                day += MONTHS.get((mm - 1) as usize).copied().unwrap_or(0);
            }
            if hour >= 12.0 {
                day += 1;
            }
            let name = format!("af3_slight_clear_{}", day.rem_euclid(8));
            if names.contains(&name) {
                return name;
            }
        }
    }
    names.iter().find(|n| n.contains("clear")).or_else(|| names.first()).cloned().unwrap_or_default()
}

/// The linked game's render settings (`appdata\user.ltx`), Call of Pripyat's defaults where it
/// does not say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameSettings {
    pub lumscale_sun: f32,
    pub lumscale_hemi: f32,
    pub lumscale_amb: f32,
    pub tonemap: bool,
    pub tonemap_middlegray: f32,
    pub tonemap_lowlum: f32,
    pub tonemap_amount: f32,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            lumscale_sun: 1.0,
            lumscale_hemi: 1.0,
            lumscale_amb: 1.0,
            tonemap: true,
            tonemap_middlegray: 1.0,
            tonemap_lowlum: 0.0001,
            tonemap_amount: 0.7,
        }
    }
}

impl GameSettings {
    /// The game's render settings from its `user.ltx`: console commands, one "name value" a line.
    #[must_use]
    pub fn read(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(key), Some(value)) = (it.next(), it.next()) else { continue };
            let v: f32 = value.parse().unwrap_or(0.0);
            match key {
                "r2_sun_lumscale" => s.lumscale_sun = v,
                "r2_sun_lumscale_hemi" => s.lumscale_hemi = v,
                "r2_sun_lumscale_amb" => s.lumscale_amb = v,
                "r2_tonemap" => s.tonemap = value == "on",
                "r2_tonemap_middlegray" => s.tonemap_middlegray = v,
                "r2_tonemap_lowlum" => s.tonemap_lowlum = v,
                "r2_tonemap_amount" => s.tonemap_amount = v,
                _ => {}
            }
        }
        s
    }

    /// The exposure the game's tonemap gives a frame of mean luminance `lum`.
    #[must_use]
    pub fn tonemap_scale(&self, lum: f32) -> f32 {
        let amount = if self.tonemap { self.tonemap_amount } else { 0.0 };
        let x = 1.0 + (self.tonemap_middlegray - 1.0) * amount;
        let z = 1.0 + (self.tonemap_lowlum - 1.0) * amount;
        (x / (lum * amount + z).max(gz_scene::math::EPS_S)).clamp(0.7, 20.0)
    }

    /// `Exposure`: the factor asked for, else the tonemap's for a night scene (mean luminance
    /// about 0.05) or what the measure settled on.
    #[must_use]
    pub fn exposure(&self, setting: f32, auto: f32) -> f32 {
        if setting > 0.0 {
            setting
        } else if !self.tonemap {
            1.0
        } else if auto > 0.0 {
            auto
        } else {
            self.tonemap_scale(0.05)
        }
    }
}

/// A light the game would draw.
#[derive(Clone, Debug, PartialEq)]
pub struct GameLight {
    /// "level", "lamp", "lamp_ambient", "campfire".
    pub kind: &'static str,
    pub owner: SmolStr,
    pub position: Vec3,
    pub range: f32,
    pub color: Vec3,
    /// 0 for a point light.
    pub spot_cos: f32,
    pub direction: Vec3,
}

impl GameLight {
    fn new(kind: &'static str, owner: &str, position: Vec3, range: f32, color: Vec3) -> Self {
        Self {
            kind,
            owner: SmolStr::new(owner),
            position,
            range,
            color,
            spot_cos: 0.0,
            direction: Vec3::new(0.0, -1.0, 0.0),
        }
    }

    /// The renderer's light.
    #[must_use]
    pub fn render(&self) -> Light {
        let kind = if self.spot_cos > 0.0 {
            LightKind::Spot { direction: self.direction, cos_half: self.spot_cos }
        } else {
            LightKind::Point
        };
        Light { kind, position: self.position, range: self.range.max(0.01), color: self.color }
    }
}

/// A light animation's colour over its length, the way the game reads it (16 samples).
#[must_use]
pub fn anim_color(anims: &gz_fmt_xr::LightAnims, name: &str) -> Option<Vec3> {
    if name.is_empty() {
        return None;
    }
    let item = anims.find(name)?;
    if item.frame_count <= 0 || item.fps <= 0.0 {
        return None;
    }
    let length = item.frame_count as f32 / item.fps;
    let mut sum = Vec3::ZERO;
    for i in 0..16 {
        let (c, _) = item.calculate(length * i as f32 / 16.0)?;
        sum += Vec3::new(((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32);
    }
    Some(sum / (255.0 * 16.0))
}

/// `Collect`: every light the game would draw in the level - the level's dynamic point
/// lights, every hanging lamp the R2+ renderers draw (and its ambient light), every burning
/// campfire.
#[must_use]
pub fn collect(
    state: &SceneState,
    configs: Option<&Ltx>,
    anims: Option<&gz_fmt_xr::LightAnims>,
) -> Vec<GameLight> {
    let mut all = Vec::new();
    for o in state.of_class(ObjClass::Light) {
        let ObjData::Light(l) = &o.data else { continue };
        if !o.flags.visible || l.kind != LT_POINT || l.flags & light_flags::AFFECT_DYNAMIC == 0 {
            continue;
        }
        let color = Vec3::new(l.color[0], l.color[1], l.color[2]) * l.brightness;
        all.push(GameLight::new("level", &o.name, o.position(), l.range, color));
    }
    for o in state.of_class(ObjClass::SpawnPoint) {
        let ObjData::Spawn(sp) = &o.data else { continue };
        if !o.flags.visible {
            continue;
        }
        let Some(e) = sp.entity.as_ref().and_then(|e| e.entity()) else { continue };
        let rot = gz_scene::math::rotation_matrix(o.rotation());
        if let Some(lamp) = e.object().and_then(|ob| ob.lamp()) {
            use gz_entities::model::lamp_flags;
            if lamp.flags & lamp_flags::R2 == 0 {
                continue;
            }
            let color = match anims.and_then(|a| anim_color(a, &lamp.color_animator)) {
                Some(c) => c * lamp.brightness,
                None => {
                    let c = lamp.color;
                    Vec3::new(((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32) / 255.0
                        * lamp.brightness
                }
            };
            // the light bone's place is the model's; the editor lights from the lamp's pivot
            let mut main = GameLight::new("lamp", &o.name, o.position(), lamp.range, color);
            if lamp.flags & lamp_flags::TYPE_SPOT != 0 {
                main.spot_cos = (lamp.spot_cone_angle * 0.5).cos();
                main.direction = rot.row(2).normalize_or_zero();
            }
            all.push(main);
            if lamp.flags & lamp_flags::POINT_AMBIENT != 0 && lamp.ambient_radius > 0.0 {
                all.push(GameLight::new(
                    "lamp_ambient",
                    &o.name,
                    o.position(),
                    lamp.ambient_radius,
                    color * lamp.ambient_power,
                ));
            }
            continue;
        }
        // a campfire burns at night: the zone's idle light, coloured by its light animation
        let section = e.section();
        let Some(cfg) = configs else { continue };
        if !section.contains("campfire") || !cfg.section_exist(section) {
            continue;
        }
        if !cfg.line_exist(section, "idle_light") || !cfg.r_bool(section, "idle_light").unwrap_or(false) {
            continue;
        }
        let range = if cfg.line_exist(section, "idle_light_range") {
            cfg.r_float(section, "idle_light_range").unwrap_or(0.0)
        } else {
            0.0
        };
        if range <= 0.0 {
            continue;
        }
        let anim = if cfg.line_exist(section, "idle_light_anim") {
            cfg.r_string(section, "idle_light_anim").ok()
        } else {
            None
        };
        let color =
            anim.and_then(|n| anims.and_then(|a| anim_color(a, n))).unwrap_or(Vec3::new(1.0, 0.6, 0.3));
        let mut position = o.position();
        if cfg.line_exist(section, "idle_light_height") {
            position.y += cfg.r_float(section, "idle_light_height").unwrap_or(0.0);
        }
        all.push(GameLight::new("campfire", &o.name, position, range, color));
    }
    all
}

/// The lights whose reach the camera sees, nearest first, cut to [`MAX_LIGHTS`]; and
/// how many there were before the cut.
#[must_use]
pub fn gather(all: &[GameLight], frustum: &Frustum, eye: Vec3) -> (Vec<GameLight>, usize) {
    let mut seen: Vec<GameLight> = all
        .iter()
        .filter(|l| l.range > 0.0 && frustum.intersects_sphere(l.position, l.range))
        .cloned()
        .collect();
    let in_view = seen.len();
    seen.sort_by(|a, b| eye.distance_squared(a.position).total_cmp(&eye.distance_squared(b.position)));
    seen.truncate(MAX_LIGHTS);
    (seen, in_view)
}

/// The corner of the roof map round `eye`: 32 m steps, so the camera may wander before it moves.
#[must_use]
pub fn roof_origin(eye: Vec3) -> [f32; 2] {
    let half = ROOF_CELLS as f32 * ROOF_CELL * 0.5;
    [(eye.x / 32.0).floor() * 32.0 - half, (eye.z / 32.0).floor() * 32.0 - half]
}

/// Whether the camera went a quarter of the map away from the map's centre (or there is none).
#[must_use]
pub fn roof_stale(map: Option<&RoofMap>, eye: Vec3) -> bool {
    let half = ROOF_CELLS as f32 * ROOF_CELL * 0.5;
    match map {
        Some(m) if m.cells == ROOF_CELLS => {
            (eye.x - (m.origin[0] + half)).abs() >= half * 0.5
                || (eye.z - (m.origin[1] + half)).abs() >= half * 0.5
        }
        _ => true,
    }
}

/// The roof map: for every cell the height of the topmost static surface over it, straight down
/// from above the region; the static library objects bucketed by 16 x 16 tiles. Rows run in
/// parallel on the caller's pool. Returns the map and how many objects it tested.
#[must_use]
pub fn bake_roof(state: &SceneState, lib: &Library, eye: Vec3, revision: u64) -> (RoofMap, usize) {
    let cells = ROOF_CELLS;
    let cell = ROOF_CELL;
    let origin = roof_origin(eye);
    let (x1, z1) = (origin[0] + cells as f32 * cell, origin[1] + cells as f32 * cell);
    struct Candidate {
        object: Arc<gz_scene::Object>,
        lib: Arc<gz_scene::library::LibraryObject>,
        min: Vec3,
        max: Vec3,
    }
    let mut candidates = Vec::new();
    let mut top = -1e4f32;
    for o in state.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        // a bare visual would be picked by its box, a whole level's terrain included
        if d.visual_mode() || d.reference.is_empty() {
            continue;
        }
        let Some(lo) = lib.cached(&d.reference) else { continue };
        if lo.is_dynamic() {
            continue;
        }
        let b = gz_scene::math::xform_box(&lo.bounds, &o.world());
        if !b.is_valid() || b.max.x < origin[0] || b.min.x > x1 || b.max.z < origin[1] || b.min.z > z1 {
            continue;
        }
        top = top.max(b.max.y);
        candidates.push(Candidate { object: o.clone(), lib: lo, min: b.min, max: b.max });
    }
    let tiles = 16u32;
    let per = cells / tiles;
    let tile = per as f32 * cell;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); (tiles * tiles) as usize];
    let clamp_tile = |v: f32| ((v / tile).floor() as i32).clamp(0, tiles as i32 - 1) as u32;
    for (i, c) in candidates.iter().enumerate() {
        let (tx0, tx1) = (clamp_tile(c.min.x - origin[0]), clamp_tile(c.max.x - origin[0]));
        let (tz0, tz1) = (clamp_tile(c.min.z - origin[1]), clamp_tile(c.max.z - origin[1]));
        for tz in tz0..=tz1 {
            for tx in tx0..=tx1 {
                if let Some(b) = buckets.get_mut((tz * tiles + tx) as usize) {
                    b.push(i);
                }
            }
        }
    }
    let start = top + 1.0;
    let reach = start + 1e4;
    let down = Vec3::new(0.0, -1.0, 0.0);
    let rows: Vec<Vec<f32>> = (0..cells)
        .into_par_iter()
        .map(|z| {
            (0..cells)
                .map(|x| {
                    let s = Vec3::new(
                        origin[0] + (x as f32 + 0.5) * cell,
                        start,
                        origin[1] + (z as f32 + 0.5) * cell,
                    );
                    let mut dist = reach;
                    if let Some(bucket) = buckets.get(((z / per) * tiles + x / per) as usize) {
                        for i in bucket {
                            let Some(c) = candidates.get(*i) else { continue };
                            if s.x < c.min.x
                                || s.x > c.max.x
                                || s.z < c.min.z
                                || s.z > c.max.z
                                || start - c.max.y >= dist
                            {
                                continue;
                            }
                            if let Some(hit) = gz_scene::pick::ray_library(&c.lib, &c.object, s, down, dist)
                                && hit.t < dist
                            {
                                dist = hit.t;
                            }
                        }
                    }
                    if dist < reach { start - dist } else { -1e4 }
                })
                .collect()
        })
        .collect();
    let heights: Vec<f32> = rows.into_iter().flatten().collect();
    (RoofMap { origin, cell, cells, heights: heights.into(), revision }, candidates.len())
}

/// The renderer's environment for a weather, the settings and the exposure.
#[must_use]
pub fn environment(
    w: &Weather,
    s: &GameSettings,
    exposure: f32,
    indoor_hemi: f32,
    roof: Option<Arc<RoofMap>>,
) -> Environment {
    Environment {
        ambient: w.ambient * s.lumscale_amb,
        hemi: w.hemi * s.lumscale_hemi,
        sun_color: w.sun_color * s.lumscale_sun,
        sun_dir: w.sun_dir,
        fog_color: w.fog_color,
        fog_near: 0.0,
        fog_far: 0.0,
        exposure,
        indoor_hemi,
        roof,
        sky: (!w.sky.textures[0].is_empty()).then(|| gz_render::Sky {
            textures: w.sky.textures.clone(),
            color: w.sky.color,
            weight: w.sky.weight,
            rotation: w.sky.rotation,
        }),
    }
}

/// The fog colour at the exposure, as the background.
#[must_use]
pub fn clear_color(w: &Weather, exposure: f32) -> gz_render::Color {
    let c = |v: f32| ((v * exposure).clamp(0.0, 1.0) * 255.0) as u8;
    gz_render::Color::rgb(c(w.fog_color.x), c(w.fog_color.y), c(w.fog_color.z))
}

/// An hour as the clock reads it: "22:00".
#[must_use]
pub fn hour_text(hour: f32) -> String {
    let minutes = (hour * 60.0 + 0.5).floor() as i32;
    format!("{:02}:{:02}", (minutes / 60).rem_euclid(24), minutes.rem_euclid(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEATHER: &str = "[00:00:00]\nambient_color = 0.1, 0.1, 0.1\nhemisphere_color = 0.2, 0.2, 0.2, 1\nsun_color = 0, 0, 0\nfog_color = 0.1, 0.1, 0.2\nsun_altitude = -30\nsun_longitude = 0\n\
[12:00:00]\nambient_color = 0.3, 0.3, 0.3\nhemisphere_color = 0.6, 0.6, 0.6, 1\nsun_color = 1, 1, 1\nfog_color = 0.5, 0.5, 0.6\nsun_altitude = -30\nsun_longitude = 0\n";

    const SKY: &str = "[00:00:00]\nsky_texture = sky\\sky_9_cube\nsky_color = 0, 0, 0.5\nsky_rotation = 90\n\
[12:00:00]\nsky_texture = sky\\sky_1_cube\nsky_color = 1, 1, 1\n";

    #[test]
    fn the_weather_is_blended_between_its_hours() {
        let doc = LtxDoc::parse(WEATHER);
        let w = weather_at(&doc, "w", 6.0).unwrap_or_default();
        assert!((w.ambient.x - 0.2).abs() < 1e-4);
        assert!((w.sun_color.x - 0.5).abs() < 1e-4);
        // past the last key it blends towards the first one, round midnight
        let late = weather_at(&doc, "w", 18.0).unwrap_or_default();
        assert!((late.ambient.x - 0.2).abs() < 1e-4);
        // the sky of both sections, the blend half way, the turn blended in radians
        let w = weather_at(&LtxDoc::parse(SKY), "w", 6.0).unwrap_or_default();
        assert_eq!(w.sky.textures, [SmolStr::new("sky\\sky_9_cube"), SmolStr::new("sky\\sky_1_cube")]);
        assert!((w.sky.weight - 0.5).abs() < 1e-6);
        assert!((w.sky.color - Vec3::new(0.5, 0.5, 0.75)).abs().max_element() < 1e-5);
        assert!((w.sky.rotation - 45f32.to_radians()).abs() < 1e-5);
        let env = environment(&w, &GameSettings::default(), 1.0, 0.15, None);
        assert!(env.sky.is_some_and(|s| s.weight == 0.5));
        assert!(weather_at(&LtxDoc::parse("[x]\na = 1\n"), "w", 1.0).is_err());
        assert_eq!(section_seconds("22:00:00"), Some(79200));
        assert_eq!(section_seconds("sky"), None);
    }

    #[test]
    fn the_default_weather_follows_the_moon() {
        let names: Vec<String> =
            (0..8).map(|i| format!("af3_slight_clear_{i}")).chain(["w_clear".to_owned()]).collect();
        let a = default_weather(&names, Some("12.05.2012"), 22.0);
        assert!(a.starts_with("af3_slight_clear_"));
        assert_eq!(default_weather(&["rain".into(), "w_clear".into()], None, 1.0), "w_clear");
        assert_eq!(default_weather(&[], None, 1.0), "");
    }

    #[test]
    fn the_tonemap_and_the_clock() {
        let s = GameSettings::read("r2_tonemap off\nr2_sun_lumscale 2.5\n");
        assert!(!s.tonemap && (s.lumscale_sun - 2.5).abs() < 1e-6);
        assert_eq!(s.exposure(0.0, 0.0), 1.0);
        assert_eq!(GameSettings::default().exposure(2.0, 0.0), 2.0);
        let night = GameSettings::default().tonemap_scale(0.05);
        assert!(night > 1.0 && night <= 20.0);
        assert_eq!(hour_text(18.5), "18:30");
        assert_eq!(hour_text(23.999), "00:00");
    }

    #[test]
    fn lights_are_cut_to_the_nearest() {
        let all: Vec<GameLight> = (0..200)
            .map(|i| GameLight::new("level", "l", Vec3::new(0.0, 0.0, 5.0 + i as f32), 2.0, Vec3::ONE))
            .collect();
        let cam = gz_render::Camera::looking(Vec3::ZERO, Vec3::Z);
        let (drawn, in_view) = gather(&all, &cam.frustum([100, 100]), Vec3::ZERO);
        assert_eq!(in_view, 200);
        assert_eq!(drawn.len(), MAX_LIGHTS);
        assert!(drawn.first().is_some_and(|l| (l.position.z - 5.0).abs() < 1e-5));
        assert!(roof_stale(None, Vec3::ZERO));
    }
}

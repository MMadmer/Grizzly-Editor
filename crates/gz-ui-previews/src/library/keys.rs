//! The named keys of a library entry's description: what `library.set` takes and answers, and
//! the one place that turns a value written by a caller into a description. The tabs' own edits
//! reach the same stores; these are the same fields by name, with the rules the rows of the
//! Details panel keep (what a type has no row for, what is clamped).

use gz_fmt_sound::SoundThm;
use gz_fmt_tex::{
    BumpMode, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams, TextureType,
};
use serde_json::{Map, Value, json};

use super::sounds::{GAME_TYPES, max_from_min, min_from_max};
use super::textures::{BUMP_TOKENS, TYPE_TOKENS, format_tokens, material_tokens, mip_tokens};

/// The key an unknown one is nearest to, when it is near enough to be a misspelling.
#[must_use]
pub fn nearest<'a>(key: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let key = key.to_ascii_lowercase();
    names
        .into_iter()
        .map(|n| (distance(&key, n), n))
        .filter(|(d, n)| *d <= n.len().max(key.len()) / 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// A key not in `names`, as the sentence a caller reads.
#[must_use]
pub fn unknown_key(what: &str, key: &str, names: &[&str]) -> String {
    match nearest(key, names.iter().copied()) {
        Some(n) => format!("{what} has no key '{key}' - did you mean '{n}'?"),
        None => format!("{what} has no key '{key}' (keys: {})", names.join(", ")),
    }
}

/// The entry named exactly (any case), or why it cannot be found, with the nearest names.
pub fn exact<'a>(what: &str, keys: &'a [&'a str], name: &str) -> Result<&'a str, String> {
    if name.is_empty() {
        return Err(format!("item names the {what}"));
    }
    if let Some(k) = keys.iter().find(|k| k.eq_ignore_ascii_case(name)) {
        return Ok(k);
    }
    let leaf = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    let near: Vec<&str> =
        keys.iter().copied().filter(|k| k.to_ascii_lowercase().contains(&leaf)).take(5).collect();
    Err(if near.is_empty() {
        format!("no {what} is named '{name}'")
    } else {
        format!("no {what} is named '{name}' - did you mean: {}?", near.join(", "))
    })
}

/// The letters and digits of a word, lower case: `Cube Map`, `cube_map` and `cubemap` are one.
fn word(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A value of a token list: by the editor's word for it (any case, spaces and underscores
/// free) or by its number.
pub fn token(key: &str, tokens: &[(u32, &str)], value: &Value) -> Result<u32, String> {
    if let Some(n) = value.as_u64() {
        return u32::try_from(n)
            .ok()
            .filter(|n| tokens.iter().any(|(v, _)| v == n))
            .ok_or_else(|| format!("{key}: {n} is none of {}", words(tokens)));
    }
    let Some(text) = value.as_str() else {
        return Err(format!("{key} takes one of {}", words(tokens)));
    };
    let want = word(text);
    tokens
        .iter()
        .find(|(_, name)| word(name) == want)
        .map(|(v, _)| *v)
        .ok_or_else(|| format!("{key}: '{text}' is none of {}", words(tokens)))
}

fn words(tokens: &[(u32, &str)]) -> String {
    tokens.iter().map(|(_, n)| *n).collect::<Vec<_>>().join(", ")
}

fn flag(key: &str, v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("{key} takes true or false"))
}

fn real(key: &str, v: &Value) -> Result<f32, String> {
    v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| format!("{key} takes a number"))
}

fn whole(key: &str, v: &Value, lo: i64, hi: i64) -> Result<i64, String> {
    let n = v.as_i64().or_else(|| v.as_f64().filter(|x| x.fract() == 0.0).map(|x| x as i64));
    match n {
        Some(n) if (lo..=hi).contains(&n) => Ok(n),
        Some(n) => Err(format!("{key}: {n} is outside {lo} to {hi}")),
        None => Err(format!("{key} takes a whole number")),
    }
}

fn text(key: &str, v: &Value) -> Result<String, String> {
    v.as_str().map(str::to_owned).ok_or_else(|| format!("{key} takes a name"))
}

/// A colour: `#RRGGBB` (opaque), `#AARRGGBB` or `0xAARRGGBB` as text, or the number itself.
pub fn color(key: &str, v: &Value) -> Result<u32, String> {
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).map_err(|_| format!("{key}: {n} is no colour"));
    }
    let Some(s) = v.as_str() else { return Err(format!("{key} takes a colour like #RRGGBB")) };
    let hex = s.trim().trim_start_matches('#').trim_start_matches("0x").trim_start_matches("0X");
    match (u32::from_str_radix(hex, 16), hex.len()) {
        (Ok(n), 6) => Ok(0xFF00_0000 | n),
        (Ok(n), 8) => Ok(n),
        _ => Err(format!("{key}: '{s}' is no colour: write #RRGGBB or #AARRGGBB")),
    }
}

/// A colour as the text `color` reads: `#AARRGGBB`.
#[must_use]
pub fn color_text(argb: u32) -> String {
    format!("#{argb:08X}")
}

// ---- textures ----------------------------------------------------------------------------------

/// The keys of a texture's description, in the order they are applied (the type first: it decides
/// what the others mean), with what each is.
pub const TEXTURE_KEYS: &[(&str, &str)] = &[
    ("type", "2D Texture, Cube Map, Bump Map, Normal Map or Terrain"),
    ("format", "how the game texture is stored (DXT1, DXT5, ...): images and cube maps"),
    ("mips", "make the smaller levels (true or false)"),
    ("mip_filter", "how the smaller levels are made (Box, Kaiser, ...)"),
    ("bump_mode", "None, Use or Use parallax"),
    ("bump_texture", "the bump map the texture is lit with, by its library name"),
    ("bump_normal_map", "a bump map's own normal map, by its library name"),
    ("bump_height", "a bump map's virtual height in metres, 0 to 0.1"),
    ("detail_texture", "the detail texture laid over it, by its library name"),
    ("detail_scale", "its scale, 0.1 to 10000"),
    ("detail_diffuse", "use the detail as diffuse (true or false)"),
    ("detail_bump", "use the detail as bump (true or false)"),
    ("material", "the lighting model the game blends from"),
    ("material_weight", "how much of it"),
    ("dither", "dither the colour (true or false)"),
    ("dither_each_mip", "dither every level (true or false)"),
    ("implicit_lighted", "implicit lighted (true or false)"),
    ("fade_color_on", "fade to a colour (true or false)"),
    ("fade_alpha_on", "fade the alpha (true or false)"),
    ("fade_delay", "the level fading starts at, 0 to 255"),
    ("fade_amount", "percent of the colour to fade in, 0 to 100"),
    ("fade_color", "the colour faded to, #RRGGBB or #AARRGGBB"),
    ("border_color_on", "a colour border (true or false)"),
    ("border_alpha_on", "an alpha border (true or false)"),
    ("border_color", "the border's colour, #RRGGBB or #AARRGGBB"),
];

/// Whether a description of this type has a row for `key`.
fn applies(ty: TextureType, key: &str) -> bool {
    if key == "type" {
        return true;
    }
    match ty {
        TextureType::IMAGE | TextureType::CUBE_MAP => !matches!(key, "bump_normal_map" | "bump_height"),
        TextureType::BUMP_MAP => matches!(key, "bump_normal_map" | "bump_height"),
        TextureType::NORMAL_MAP => matches!(key, "mips" | "mip_filter"),
        TextureType::TERRAIN => matches!(
            key,
            "detail_texture"
                | "detail_scale"
                | "detail_diffuse"
                | "detail_bump"
                | "material"
                | "material_weight"
        ),
        _ => true,
    }
}

fn label_of(tokens: &[(u32, &str)], v: u32) -> Value {
    tokens.iter().find(|(n, _)| *n == v).map_or_else(|| json!(v), |(_, l)| json!(l))
}

/// What a texture key is now, as `library.set` answers it.
#[must_use]
pub fn texture_value(p: &TextureParams, key: &str) -> Value {
    let has = |f: TextureFlags| json!(p.flags.contains(f));
    match key {
        "type" => label_of(&TYPE_TOKENS, p.ty.0),
        "format" => label_of(&format_tokens(), p.fmt.0),
        "mips" => has(TextureFlags::GENERATE_MIP_MAPS),
        "mip_filter" => label_of(&mip_tokens(), p.mip_filter.0),
        "bump_mode" => label_of(&BUMP_TOKENS, p.bump_mode.0),
        "bump_texture" => json!(p.bump_name.as_str()),
        "bump_normal_map" => json!(p.ext_normal_map_name.as_str()),
        "bump_height" => json!(p.bump_virtual_height),
        "detail_texture" => json!(p.detail_name.as_str()),
        "detail_scale" => json!(p.detail_scale),
        "detail_diffuse" => has(TextureFlags::DIFFUSE_DETAIL),
        "detail_bump" => has(TextureFlags::BUMP_DETAIL),
        "material" => label_of(&material_tokens(), p.material.0),
        "material_weight" => json!(p.material_weight),
        "dither" => has(TextureFlags::DITHER_COLOR),
        "dither_each_mip" => has(TextureFlags::DITHER_EACH_MIP_LEVEL),
        "implicit_lighted" => has(TextureFlags::IMPLICIT_LIGHTED),
        "fade_color_on" => has(TextureFlags::FADE_TO_COLOR),
        "fade_alpha_on" => has(TextureFlags::FADE_TO_ALPHA),
        "fade_delay" => json!(p.fade_delay),
        "fade_amount" => json!(p.fade_amount),
        "fade_color" => json!(color_text(p.fade_color)),
        "border_color_on" => has(TextureFlags::COLOR_BORDER),
        "border_alpha_on" => has(TextureFlags::ALPHA_BORDER),
        "border_color" => json!(color_text(p.border_color)),
        _ => Value::Null,
    }
}

/// One key of a texture's description set from a caller's value.
fn set_texture_key(p: &mut TextureParams, key: &str, v: &Value) -> Result<(), String> {
    let set_flag = |p: &mut TextureParams, f: TextureFlags| -> Result<(), String> {
        p.flags.set(f, flag(key, v)?);
        Ok(())
    };
    match key {
        "type" => {
            p.ty = TextureType(token(key, &TYPE_TOKENS, v)?);
            p.apply_type_defaults();
        }
        "format" => p.fmt = TextureFormat(token(key, &format_tokens(), v)?),
        "mips" => set_flag(p, TextureFlags::GENERATE_MIP_MAPS)?,
        "mip_filter" => p.mip_filter = MipFilterId(token(key, &mip_tokens(), v)?),
        "bump_mode" => {
            p.bump_mode = BumpMode(token(key, &BUMP_TOKENS, v)?);
            p.apply_type_defaults();
        }
        "bump_texture" => p.bump_name = text(key, v)?.into(),
        "bump_normal_map" => p.ext_normal_map_name = text(key, v)?.into(),
        "bump_height" => p.bump_virtual_height = real(key, v)?.clamp(0.0, 0.1),
        "detail_texture" => p.detail_name = text(key, v)?.into(),
        "detail_scale" => p.detail_scale = real(key, v)?.clamp(0.1, 10000.0),
        "detail_diffuse" => set_flag(p, TextureFlags::DIFFUSE_DETAIL)?,
        "detail_bump" => set_flag(p, TextureFlags::BUMP_DETAIL)?,
        "material" => p.material = TextureMaterial(token(key, &material_tokens(), v)?),
        "material_weight" => p.material_weight = real(key, v)?,
        "dither" => set_flag(p, TextureFlags::DITHER_COLOR)?,
        "dither_each_mip" => set_flag(p, TextureFlags::DITHER_EACH_MIP_LEVEL)?,
        "implicit_lighted" => set_flag(p, TextureFlags::IMPLICIT_LIGHTED)?,
        "fade_color_on" => set_flag(p, TextureFlags::FADE_TO_COLOR)?,
        "fade_alpha_on" => set_flag(p, TextureFlags::FADE_TO_ALPHA)?,
        "fade_delay" => p.fade_delay = u8::try_from(whole(key, v, 0, 255)?).unwrap_or(0),
        "fade_amount" => p.fade_amount = u32::try_from(whole(key, v, 0, 100)?).unwrap_or(0),
        "fade_color" => p.fade_color = color(key, v)?,
        "border_color_on" => set_flag(p, TextureFlags::COLOR_BORDER)?,
        "border_alpha_on" => set_flag(p, TextureFlags::ALPHA_BORDER)?,
        "border_color" => p.border_color = color(key, v)?,
        _ => {}
    }
    Ok(())
}

/// Applies `set` to a description, all or none: the changed keys with what each was and is.
pub fn apply_texture(p: &mut TextureParams, set: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let names: Vec<&str> = TEXTURE_KEYS.iter().map(|(k, _)| *k).collect();
    if let Some(k) = set.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(unknown_key("a texture description", k, &names));
    }
    let mut work = p.clone();
    let mut changed = Vec::new();
    for key in names {
        let Some(v) = set.get(key) else { continue };
        if !applies(work.ty, key) {
            let ty = work.ty.label().unwrap_or("this type");
            return Err(format!("{key} has no row on a {ty}: set the type first, or leave it out"));
        }
        let from = texture_value(&work, key);
        set_texture_key(&mut work, key, v)?;
        let to = texture_value(&work, key);
        if from != to {
            changed.push(json!({ "key": key, "from": from, "to": to }));
        }
    }
    *p = work;
    Ok(changed)
}

// ---- sounds ------------------------------------------------------------------------------------

/// The keys of a sound's description.
pub const SOUND_KEYS: &[(&str, &str)] = &[
    ("quality", "the Vorbis quality the game sound is encoded with"),
    ("min_dist", "metres, up to which it plays at full volume, 0.01 to 1000 (or \"from_max\")"),
    ("max_dist", "metres, past which it is silent, 0.1 to 1000 (or \"from_min\")"),
    ("max_ai_dist", "metres, how far NPCs hear it; at most max_dist"),
    ("base_volume", "0 to 2"),
    ("game_type", "what NPCs make of the sound (Weapon shooting, NPC step, ...)"),
];

/// What a sound key is now.
#[must_use]
pub fn sound_value(t: &SoundThm, key: &str) -> Value {
    match key {
        "quality" => json!(t.quality),
        "min_dist" => json!(t.min_dist),
        "max_dist" => json!(t.max_dist),
        "max_ai_dist" => json!(t.max_ai_dist),
        "base_volume" => json!(t.base_volume),
        "game_type" => {
            let tokens: Vec<(u32, &str)> = GAME_TYPES.to_vec();
            label_of(&tokens, t.game_type)
        }
        _ => Value::Null,
    }
}

/// Applies `set` to a sound's description with the clamps the rows keep, all or none.
pub fn apply_sound(t: &mut SoundThm, set: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let names: Vec<&str> = SOUND_KEYS.iter().map(|(k, _)| *k).collect();
    if let Some(k) = set.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(unknown_key("a sound description", k, &names));
    }
    let mut work = *t;
    let mut changed = Vec::new();
    for key in names {
        let Some(v) = set.get(key) else { continue };
        let from = sound_value(&work, key);
        match key {
            "quality" => work.quality = real(key, v)?,
            "min_dist" if v.as_str() == Some("from_max") => work.min_dist = min_from_max(work.max_dist),
            "min_dist" => work.min_dist = real(key, v)?.clamp(0.01, 1000.0),
            "max_dist" if v.as_str() == Some("from_min") => work.max_dist = max_from_min(work.min_dist),
            "max_dist" => work.max_dist = real(key, v)?.clamp(0.1, 1000.0),
            "max_ai_dist" => work.max_ai_dist = real(key, v)?.max(0.1),
            "base_volume" => work.base_volume = real(key, v)?.clamp(0.0, 2.0),
            "game_type" => {
                let tokens: Vec<(u32, &str)> = GAME_TYPES.to_vec();
                work.game_type = token(key, &tokens, v)?;
            }
            _ => {}
        }
        // 'Max AI Dist' <= 'Max Dist'
        if work.max_ai_dist > work.max_dist {
            work.max_ai_dist = work.max_dist;
        }
        let to = sound_value(&work, key);
        if from != to {
            changed.push(json!({ "key": key, "from": from, "to": to }));
        }
    }
    *t = work;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_texture_takes_words_and_refuses_what_has_no_row() {
        let mut p = TextureParams::default();
        let changed = apply_texture(&mut p, &set(json!({ "format": "dxt5", "mips": false }))).unwrap();
        assert_eq!(p.fmt, TextureFormat::DXT5);
        assert!(!p.flags.contains(TextureFlags::GENERATE_MIP_MAPS));
        assert_eq!(changed.len(), 2);
        let err = apply_texture(&mut p, &set(json!({ "bump_height": 0.05 }))).unwrap_err();
        assert!(err.contains("no row"), "{err}");
        // a failure changes nothing
        let before = p.clone();
        assert!(apply_texture(&mut p, &set(json!({ "dither": true, "format": "nope" }))).is_err());
        assert_eq!(p, before);
        let typo = apply_texture(&mut p, &set(json!({ "formt": "DXT1" }))).unwrap_err();
        assert!(typo.contains("did you mean 'format'"), "{typo}");
    }

    #[test]
    fn the_type_decides_the_rest() {
        let mut p = TextureParams::default();
        apply_texture(&mut p, &set(json!({ "type": "Normal Map" }))).unwrap();
        assert_eq!(p.fmt, TextureFormat::RGBA);
        apply_texture(&mut p, &set(json!({ "type": "cube_map", "format": "DXT1" }))).unwrap();
        assert_eq!(p.ty, TextureType::CUBE_MAP);
    }

    #[test]
    fn a_sound_is_clamped_like_its_rows() {
        let mut t = SoundThm::default();
        apply_sound(&mut t, &set(json!({ "max_dist": 50.0, "base_volume": 9 }))).unwrap();
        assert_eq!(t.max_dist, 50.0);
        assert_eq!(t.max_ai_dist, 50.0);
        assert_eq!(t.base_volume, 2.0);
        apply_sound(&mut t, &set(json!({ "max_dist": "from_min" }))).unwrap();
        assert!((t.max_dist - max_from_min(t.min_dist)).abs() < 1e-3);
        assert_eq!(color("c", &json!("#102030")).unwrap(), 0xFF10_2030);
    }

    #[test]
    fn items_are_found_exactly() {
        let keys = ["wpn\\wpn_ak74", "wpn\\wpn_ak74u"];
        assert_eq!(exact("texture", &keys, "WPN\\wpn_ak74").unwrap(), "wpn\\wpn_ak74");
        assert!(exact("texture", &keys, "ak74").unwrap_err().contains("did you mean"));
    }
}

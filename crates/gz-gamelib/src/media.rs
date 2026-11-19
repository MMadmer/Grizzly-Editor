//! A texture and a sound of the game described: one row each of their catalogs
//! (`docs/GAME_LIBRARY.md`, 3.2 and 3.3).

use gz_fmt_tex::{DdsKind, read_dds_info, read_dds_level, read_texture_thm};
use gz_media::sound::SoundAsset;
use gz_media::texture::TextureAsset;
use serde_json::{Map, Value, json};

/// The version of what a texture's row states.
pub const TEXTURE_VERSION: u32 = 2;
/// The version of what a sound's row states.
pub const SOUND_VERSION: u32 = 2;

/// A format's word: `dxt1`, `dxt5`, `bc7`, `rgb`, ...
fn format_word(f: gz_fmt_tex::DdsFormat) -> String {
    let text = format!("{f:?}");
    text.split(['(', '{', ' ']).next().unwrap_or("").to_ascii_lowercase()
}

/// A colour as `#rrggbb`.
#[must_use]
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// The mean colour of a picture's pixels by their opacity, and how much of it is opaque (0..1).
#[must_use]
pub fn mean_colour(rgba: &[u8]) -> ([u8; 3], f32) {
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    let mut n = 0u64;
    for p in rgba.as_chunks::<4>().0 {
        let w = u64::from(p[3]);
        r += u64::from(p[0]) * w;
        g += u64::from(p[1]) * w;
        b += u64::from(p[2]) * w;
        a += w;
        n += 1;
    }
    if a == 0 || n == 0 {
        return ([128, 128, 128], 0.0);
    }
    ([(r / a) as u8, (g / a) as u8, (b / a) as u8], a as f32 / (n * 255) as f32)
}

/// Describes a texture: `name` is its game name (`wood\wood_plank_01`), `dds` the file, `thm`
/// its description file when the game holds one beside it.
pub fn describe_texture(name: &str, dds: &[u8], thm: Option<&[u8]>) -> Result<Value, String> {
    let info = read_dds_info(dds).map_err(|e| e.to_string())?;
    let mut row = Map::new();
    row.insert("name".into(), json!(name));
    row.insert("size".into(), json!([info.width, info.height]));
    row.insert("format".into(), json!(format_word(info.format)));
    row.insert("mips".into(), json!(info.mips));
    match info.kind {
        DdsKind::Cube => {
            row.insert("cube".into(), json!(true));
        }
        DdsKind::Volume => {
            row.insert("volume".into(), json!(true));
        }
        DdsKind::Texture => {}
    }
    // the colour of the whole picture, from a small level: enough for a flat look
    let small = info.mips.saturating_sub(1).min(
        // the level about 16 pixels wide
        (32 - info.width.max(info.height).max(1).leading_zeros()).saturating_sub(5),
    );
    let mut alpha = info.format.has_alpha_channel();
    if let Ok(image) = read_dds_level(dds, 0, small) {
        let (rgb, opaque) = mean_colour(&image.rgba);
        row.insert("colour".into(), json!(hex(rgb)));
        // a channel that is opaque everywhere is no transparency
        if opaque < 0.995 {
            alpha = true;
            row.insert("opaque".into(), json!((f64::from(opaque) * 100.0).round() / 100.0));
        } else {
            alpha = false;
        }
    }
    row.insert("alpha".into(), json!(alpha));
    if let Some(bytes) = thm
        && let Ok(t) = read_texture_thm(bytes)
    {
        // what the description says, in the words a texture asset says it in
        let asset = TextureAsset::from_params(&t.params, name, alpha);
        let said = gz_luadata::to_json(&gz_luadata::Value::Table(asset.to_table()));
        if let Some(m) = said.as_object() {
            for (k, v) in m {
                // a table with nothing in it says nothing
                let empty =
                    v.as_object().is_some_and(Map::is_empty) || v.as_array().is_some_and(Vec::is_empty);
                if !empty && !matches!(k.as_str(), "tex" | "source" | "name" | "alpha") {
                    row.insert(k.clone(), v.clone());
                }
            }
        }
    }
    Ok(Value::Object(row))
}

/// Describes a sound: `name` is its game name (`ambient\wind_1`), `ogg` the file.
pub fn describe_sound(name: &str, ogg: &[u8]) -> Result<Value, String> {
    let info = gz_fmt_sound::ogg_info(ogg).map_err(|e| e.to_string())?;
    let mut row = Map::new();
    row.insert("name".into(), json!(name));
    row.insert("length".into(), json!((info.seconds() * 100.0).round() / 100.0));
    row.insert("channels".into(), json!(info.channels));
    if info.rate != 44100 {
        row.insert("rate".into(), json!(info.rate));
    }
    match gz_fmt_sound::read_ogg_params(ogg) {
        Ok(Some(p)) => {
            // every parameter said outright: a row is read without knowing a default
            let asset = SoundAsset {
                min_distance: Some(f64::from(p.min_dist)),
                max_distance: Some(f64::from(p.max_dist)),
                volume: Some(f64::from(p.base_volume)),
                ai_distance: Some(f64::from(p.max_ai_dist)),
                ty: Some(p.game_type),
                ..SoundAsset::default()
            };
            let said = gz_luadata::to_json(&gz_luadata::Value::Table(asset.to_table()));
            if let Some(m) = said.as_object() {
                for (k, v) in m {
                    if k != "snd" {
                        row.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        // a sound without the engine's parameters plays with the engine's defaults
        Ok(None) => {
            row.insert("parameters".into(), json!(false));
        }
        Err(e) => {
            row.insert("problem".into(), json!(e.to_string()));
        }
    }
    Ok(Value::Object(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texture_row_states_size_format_colour_and_whether_it_has_holes() {
        // 4 x 4, the left half red and opaque, the right half clear
        let mut px = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                px.extend(if x < 2 { [200u8, 0, 0, 255] } else { [0, 0, 0, 0] });
            }
        }
        let image = gz_fmt_tex::Image::new(4, 4, px).unwrap();
        let request = gz_fmt_tex::DdsRequest {
            encoding: gz_fmt_tex::DdsEncoding::Rgba8,
            mips: false,
            filter: gz_fmt_tex::MipFilter::Box,
        };
        let dds = gz_fmt_tex::build_dds(&image, &request).unwrap();
        let row = describe_texture("test\\half", &dds, None).unwrap();
        assert_eq!(row["name"], "test\\half");
        assert_eq!(row["size"], json!([4, 4]));
        assert_eq!(row["alpha"], true);
        assert_eq!(row["colour"], "#c80000");
        assert!(row["mips"].as_u64().unwrap() >= 1);
        assert!(describe_texture("x", b"no picture", None).is_err());
        assert_eq!(hex([1, 2, 255]), "#0102ff");
    }
}

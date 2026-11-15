//! The module's icon atlas: every picture an item names as its icon (`icon = "icons/ak74.png"`)
//! packed into `gamedata\textures\ui\<module>_icons.dds`, each scaled to its cells of 50 px.
//!
//! The packing is deterministic: the icons by their items' sections, a shelf packer filling
//! rows left to right, the atlas a power of two on each side, the smallest one that holds them
//! (the squarer of two of one area, then the narrower). The same pictures give the same bytes.

use std::path::{Path, PathBuf};

use gz_fmt_tex::{DdsEncoding, Image, MipFilter, encode_dds, load_image, resize};
use gz_luadata::{Limits, LuaDoc, Seg, Value};

/// The side of a cell, pixels: what `inv_grid_*` counts in.
pub const CELL: u32 = 50;

/// The largest side of the atlas, pixels.
pub const MAX_SIDE: u32 = 8192;

/// The texture's game name of a module's atlas: `ui\<module>_icons`.
#[must_use]
pub fn texture_name(module: &str) -> String {
    format!("ui\\{module}_icons")
}

/// Where the atlas ships in the module: `gamedata\textures\ui\<module>_icons.dds`.
#[must_use]
pub fn texture_path(module: &str) -> String {
    format!("gamedata\\textures\\{}.dds", texture_name(module))
}

/// An icon an item wants in the atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// The item's file, project-relative with `/`.
    pub file: String,
    pub section: String,
    pub picture: PathBuf,
    /// The picture's size, pixels.
    pub px: (u32, u32),
    pub w: Option<u32>,
    pub h: Option<u32>,
}

/// An icon's place in the atlas, in cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub file: String,
    pub section: String,
    pub picture: PathBuf,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The atlas a module's icons make.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// `ui\<module>_icons`.
    pub texture: String,
    /// `gamedata\textures\ui\<module>_icons.dds`.
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub placed: Vec<Placed>,
    /// Icons that did not fit the largest atlas: `(file, why)`.
    pub left_out: Vec<(String, String)>,
}

impl Plan {
    /// The place of an item's icon.
    #[must_use]
    pub fn of_file(&self, file: &str) -> Option<&Placed> {
        self.placed.iter().find(|p| p.file.eq_ignore_ascii_case(file))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }
}

/// The cells a picture of `px` takes: `w` and `h` as given, else worked out from its size,
/// rounded up (a picture of 100 x 50 px is 2 x 1).
#[must_use]
pub fn cells_of(px: (u32, u32), w: Option<u32>, h: Option<u32>) -> (u32, u32) {
    let (pw, ph) = (f64::from(px.0.max(1)), f64::from(px.1.max(1)));
    let cell = f64::from(CELL);
    let up = |x: f64| (x - 1e-9).ceil().max(1.0) as u32;
    match (w, h) {
        (Some(w), Some(h)) => (w.max(1), h.max(1)),
        (Some(w), None) => (w.max(1), up(f64::from(w) * ph / pw)),
        (None, Some(h)) => (up(f64::from(h) * pw / ph), h.max(1)),
        (None, None) => (up(pw / cell), up(ph / cell)),
    }
}

/// Shelf-packs `sizes` (cells) in their order into an atlas `width` pixels wide: the places
/// and the height used, cells.
fn shelves(sizes: &[(u32, u32)], width: u32) -> Option<(Vec<(u32, u32)>, u32)> {
    let cols = width / CELL;
    let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
    let mut out = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        if w > cols {
            return None;
        }
        if x + w > cols {
            y += row;
            x = 0;
            row = 0;
        }
        out.push((x, y));
        x += w;
        row = row.max(h);
    }
    Some((out, y + row))
}

/// Where packed icons go (cells, in their order) and the atlas' width and height, pixels.
pub type Packed = (Vec<(u32, u32)>, u32, u32);

/// The places of `sizes` (cells, in their order) and the atlas' size in pixels; `None` when
/// they do not fit the largest atlas.
#[must_use]
pub fn pack(sizes: &[(u32, u32)]) -> Option<Packed> {
    let mut best: Option<Packed> = None;
    let mut width = 64;
    while width <= MAX_SIDE {
        if let Some((places, used)) = shelves(sizes, width) {
            let height = (used * CELL).max(1).next_power_of_two().max(64);
            if height <= MAX_SIDE {
                let better = match &best {
                    None => true,
                    Some((_, bw, bh)) => {
                        let (a, b) = (u64::from(width) * u64::from(height), u64::from(*bw) * u64::from(*bh));
                        a < b || (a == b && width.max(height) < (*bw).max(*bh))
                    }
                };
                if better {
                    best = Some((places, width, height));
                }
            }
        }
        width *= 2;
    }
    best
}

/// The plan of the icons `wanted`: by section name, packed.
#[must_use]
pub fn plan(module: &str, mut wanted: Vec<Wanted>) -> Plan {
    wanted.sort_by(|a, b| a.section.cmp(&b.section).then_with(|| a.file.cmp(&b.file)));
    let mut out = Plan { texture: texture_name(module), path: texture_path(module), ..Plan::default() };
    let sizes: Vec<(u32, u32)> = wanted.iter().map(|w| cells_of(w.px, w.w, w.h)).collect();
    let limit = MAX_SIDE / CELL;
    let (fit, too_big): (Vec<_>, Vec<_>) =
        wanted.into_iter().zip(sizes).partition(|(_, (w, h))| *w <= limit && *h <= limit);
    for (w, (cw, ch)) in too_big {
        out.left_out.push((w.file, format!("{cw} x {ch} cells is larger than the largest atlas")));
    }
    let sizes: Vec<(u32, u32)> = fit.iter().map(|(_, s)| *s).collect();
    if sizes.is_empty() {
        return out;
    }
    match pack(&sizes) {
        Some((places, width, height)) => {
            out.width = width;
            out.height = height;
            for ((w, (cw, ch)), (x, y)) in fit.into_iter().zip(places) {
                out.placed.push(Placed {
                    file: w.file,
                    section: w.section,
                    picture: w.picture,
                    x,
                    y,
                    w: cw,
                    h: ch,
                });
            }
        }
        None => {
            for (w, _) in fit {
                out.left_out.push((w.file, "the icons do not fit the largest atlas together".into()));
            }
        }
    }
    out
}

/// What tells a picture's size: [`picture_size`], or a test's own.
pub type PictureSize = dyn Fn(&Path) -> Result<(u32, u32), String>;

/// A picture's size, pixels, as the bake reads it. Blocking.
pub fn picture_size(path: &Path) -> Result<(u32, u32), String> {
    let image = read_picture(path)?;
    Ok((image.width, image.height))
}

fn read_picture(path: &Path) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|e| gz_base::io_reason(&e))?;
    let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    load_image(&bytes, Some(&ext)).map(|l| l.image).map_err(|e| e.to_string())
}

/// The atlas as a DDS (DXT5, one level): every icon scaled to its cells. Blocking.
pub fn bake(plan: &Plan) -> Result<Vec<u8>, String> {
    let (w, h) = (plan.width as usize, plan.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for p in &plan.placed {
        let pic = read_picture(&p.picture).map_err(|e| format!("{}: {e}", p.file))?;
        let (tw, th) = (p.w * CELL, p.h * CELL);
        let scaled = if (pic.width, pic.height) == (tw, th) {
            pic
        } else {
            resize(&pic, tw, th, MipFilter::Default).map_err(|e| format!("{}: {e}", p.file))?
        };
        let (ox, oy) = ((p.x * CELL) as usize, (p.y * CELL) as usize);
        for row in 0..th as usize {
            let src = scaled.rgba.get(row * tw as usize * 4..(row + 1) * tw as usize * 4).unwrap_or_default();
            let at = ((oy + row) * w + ox) * 4;
            if let Some(dst) = rgba.get_mut(at..at + src.len()) {
                dst.copy_from_slice(src);
            }
        }
    }
    let image = Image::new(plan.width, plan.height, rgba).map_err(|e| e.to_string())?;
    encode_dds(&[image], DdsEncoding::Dxt5).map_err(|e| e.to_string())
}

/// The asset's text as the module ships it: `icon` rewritten to the rectangle of the atlas,
/// every other byte as the author wrote it.
pub fn shipped_text(text: &str, texture: &str, p: &Placed) -> Result<String, String> {
    let mut doc =
        LuaDoc::parse_with(text, crate::model::stem(&p.file), Limits::asset()).map_err(|e| e.message)?;
    let value: Value = crate::model::cell_value(Some(texture), p.x, p.y, p.w, p.h);
    doc.set(&[Seg::key("icon")], &value).map_err(|e| e.message)?;
    Ok(doc.text().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_follow_the_picture() {
        assert_eq!(cells_of((100, 50), None, None), (2, 1));
        assert_eq!(cells_of((101, 50), None, None), (3, 1));
        assert_eq!(cells_of((10, 10), None, None), (1, 1));
        assert_eq!(cells_of((100, 50), Some(4), None), (4, 2));
        assert_eq!(cells_of((100, 50), None, Some(3)), (6, 3));
        assert_eq!(cells_of((100, 50), Some(1), Some(1)), (1, 1));
    }

    #[test]
    fn packing_is_deterministic_and_as_small_as_it_must_be() {
        let sizes = [(2, 1), (1, 1), (5, 2), (1, 1)];
        let (places, w, h) = pack(&sizes).unwrap();
        assert_eq!(pack(&sizes).unwrap(), (places.clone(), w, h));
        // 512 x 128 holds them in one row, 256 x 256 in four: one area, the squarer wins
        assert_eq!((w, h), (256, 256));
        assert_eq!(places, [(0, 0), (2, 0), (0, 1), (0, 3)]);
        let (_, w, h) = pack(&[(1, 1)]).unwrap();
        assert_eq!((w, h), (64, 64));
        assert!(pack(&[(200, 1)]).is_none());
    }

    #[test]
    fn a_plan_goes_by_section() {
        let want = |file: &str, section: &str, px: (u32, u32)| Wanted {
            file: file.into(),
            section: section.into(),
            picture: PathBuf::from(file),
            px,
            w: None,
            h: None,
        };
        let p =
            plan("we", vec![want("b.itemasset", "wpn_b", (100, 50)), want("a.itemasset", "wpn_a", (50, 50))]);
        assert_eq!(p.texture, "ui\\we_icons");
        assert_eq!(p.path, "gamedata\\textures\\ui\\we_icons.dds");
        assert_eq!(p.placed[0].section, "wpn_a");
        // 128 x 128 (two rows) over 256 x 64 (one): one area, the squarer
        assert_eq!((p.width, p.height), (128, 128));
        assert_eq!((p.placed[1].x, p.placed[1].y, p.placed[1].w), (0, 1, 2));
        assert_eq!(p.of_file("B.ITEMASSET").map(|x| x.section.as_str()), Some("wpn_b"));
    }

    #[test]
    fn the_shipped_text_changes_the_icon_alone() {
        let text = "-- mine\nreturn {\n  item = 1, -- the header\n  base = \"wpn_pm\",\n  icon = \"icons/pm.png\",\n  set = { cost = 5 },\n}\n";
        let placed = Placed {
            file: "a.itemasset".into(),
            section: "x".into(),
            picture: PathBuf::new(),
            x: 3,
            y: 0,
            w: 2,
            h: 1,
        };
        let out = shipped_text(text, "ui\\we_icons", &placed).unwrap();
        assert!(
            out.starts_with("-- mine\nreturn {\n  item = 1, -- the header\n  base = \"wpn_pm\",\n  icon = {"),
            "{out}"
        );
        assert!(out.ends_with("  set = { cost = 5 },\n}\n"), "{out}");
        let v = gz_luadata::parse(&out).unwrap();
        let icon = v.get("icon").unwrap();
        assert_eq!(icon.get("texture").and_then(Value::as_str), Some("ui\\we_icons"));
        assert_eq!(icon.get("x").and_then(Value::as_number), Some(3.0));
        assert_eq!(icon.get("w").and_then(Value::as_number), Some(2.0));
    }

    #[test]
    fn the_atlas_bakes_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let pic = dir.path().join("a.tga");
        let img = image_tga(60, 40);
        std::fs::write(&pic, img).unwrap();
        let p = plan(
            "we",
            vec![Wanted {
                file: "a.itemasset".into(),
                section: "a".into(),
                picture: pic.clone(),
                px: (60, 40),
                w: None,
                h: None,
            }],
        );
        let one = bake(&p).unwrap();
        assert_eq!(one, bake(&p).unwrap());
        let info = gz_fmt_tex::read_dds_info(&one).unwrap();
        assert_eq!((info.width, info.height), (128, 64));
    }

    /// A small picture made in memory.
    fn image_tga(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x * 4) as u8, (y * 6) as u8, 128, 255]);
            }
        }
        let img = Image::new(w, h, px).unwrap();
        gz_fmt_tex::write_tga(&img, gz_fmt_tex::TgaDepth::Bgra32).unwrap()
    }
}

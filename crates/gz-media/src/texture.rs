//! The texture asset (`*.texasset`, `tex = 1`): a picture and what a texture description
//! (`.thm`) says of it, as named keys. [`TextureAsset::to_params`] and
//! [`TextureAsset::from_params`] turn it into a [`TextureParams`] and back without a loss:
//! nothing a `.thm` can say is unsayable here.
//!
//! A key left out is what the texture's `use` needs: the description starts as the editor's new
//! texture of that type ([`base`]), and every key written changes it from there. So
//! [`TextureAsset::from_params`] writes only what differs from that start.

use gz_fmt_tex::{
    BumpMode, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams, TextureType,
};
use gz_luadata::{Seg, Table, Value};

use crate::fields::{self, AUTO, BUMP, DETAIL, TEXTURE, word_of};
use crate::read::{self, Cx};

/// The bump map of a texture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bump {
    /// The bump map's game name, when it is a texture of its own.
    pub texture: Option<String>,
    /// The picture it is made of, beside the asset.
    pub source: Option<String>,
    /// What `source` holds: 0 a height map, 1 a normal map ([`fields::BUMP_FROM`]).
    pub from: Option<u32>,
    pub depth: Option<f64>,
    pub parallax: Option<bool>,
}

/// The detail texture laid over a texture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub texture: Option<String>,
    pub scale: Option<f64>,
    /// [`fields::DETAIL_ON`].
    pub on: Option<u32>,
}

/// A texture asset as its file says; `None` is a key left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextureAsset {
    pub source: Option<String>,
    pub name: Option<String>,
    /// The word of `use` as written (`surface`, `ui`, ...).
    pub use_word: Option<String>,
    /// [`fields::FORMATS`]; [`AUTO`] for `auto`.
    pub format: Option<u32>,
    pub mips: Option<bool>,
    pub mip_filter: Option<u32>,
    pub has_alpha: Option<bool>,
    pub binary_alpha: Option<bool>,
    pub dither: Option<bool>,
    pub dither_each_mip: Option<bool>,
    pub implicit_light: Option<bool>,
    pub alpha_border: Option<bool>,
    pub color_border: Option<bool>,
    pub border_color: Option<u32>,
    pub fade_to_color: Option<bool>,
    pub fade_to_alpha: Option<bool>,
    pub fade_color: Option<u32>,
    pub fade_amount: Option<u32>,
    pub fade_delay: Option<u8>,
    pub material: Option<u32>,
    pub weight: Option<f64>,
    pub normal_map: Option<String>,
    pub bump: Option<Bump>,
    pub detail: Option<Detail>,
    pub extra_flags: Option<u32>,
    /// The relief's depth of a texture without a `bump` table of its own (a bump map itself).
    pub depth: Option<f64>,
}

/// The flags a key names, each with its key.
const FLAG_KEYS: [(&str, TextureFlags); 10] = [
    ("mips", TextureFlags::GENERATE_MIP_MAPS),
    ("binary_alpha", TextureFlags::BINARY_ALPHA),
    ("alpha_border", TextureFlags::ALPHA_BORDER),
    ("color_border", TextureFlags::COLOR_BORDER),
    ("fade_to_color", TextureFlags::FADE_TO_COLOR),
    ("fade_to_alpha", TextureFlags::FADE_TO_ALPHA),
    ("dither", TextureFlags::DITHER_COLOR),
    ("dither_each_mip", TextureFlags::DITHER_EACH_MIP_LEVEL),
    ("implicit_light", TextureFlags::IMPLICIT_LIGHTED),
    ("has_alpha", TextureFlags::HAS_ALPHA),
];

/// The bits no key of the top level names: the detail's two (`detail.on`) and every one the
/// editor has no word for (`extra_flags`).
fn named_bits() -> TextureFlags {
    FLAG_KEYS.iter().fold(TextureFlags::DIFFUSE_DETAIL | TextureFlags::BUMP_DETAIL, |a, (_, f)| a | *f)
}

/// The default bump depth of a description.
const DEPTH: f64 = 0.05;

/// The keys that say how a picture is made into a game texture, and nothing more: over a game
/// texture published as it is (a `.dds`) nothing reads them.
pub const MAKING_KEYS: [&str; 16] = [
    "format",
    "mips",
    "mip_filter",
    "binary_alpha",
    "dither",
    "dither_each_mip",
    "alpha_border",
    "color_border",
    "border_color",
    "fade_to_color",
    "fade_to_alpha",
    "fade_color",
    "fade_amount",
    "fade_delay",
    "normal_map",
    "depth",
];

/// The description a texture of this type starts as: the editor's new texture, alpha as the
/// picture has it, the format by alpha, then what the type needs.
#[must_use]
pub fn base(ty: TextureType, ui: bool, alpha: bool) -> TextureParams {
    let mut p = TextureParams { ty, ..TextureParams::default() };
    p.flags.set(TextureFlags::HAS_ALPHA, alpha);
    p.fmt = if alpha { TextureFormat::DXT5 } else { TextureFormat::DXT1 };
    p.apply_type_defaults();
    if ui || ty == TextureType::CUBE_MAP {
        p.flags.remove(TextureFlags::GENERATE_MIP_MAPS);
    }
    p
}

impl TextureAsset {
    /// The asset of a file's value; what does not read is an issue at its key.
    #[must_use]
    pub fn from_value(v: &Value) -> (Self, Vec<read::Issue>) {
        let mut cx = Cx::default();
        let mut t = TextureAsset {
            source: read::text(&mut cx, v, "source"),
            name: read::text(&mut cx, v, "name"),
            ..Self::default()
        };
        if read::word(&mut cx, v, TEXTURE, "use").is_some() {
            t.use_word = v.get("use").and_then(Value::as_str).map(str::to_ascii_lowercase);
        }
        t.format = read::word(&mut cx, v, TEXTURE, "format");
        t.mip_filter = read::word(&mut cx, v, TEXTURE, "mip_filter");
        t.mips = read::boolean(&mut cx, v, "mips");
        t.has_alpha = read::boolean(&mut cx, v, "has_alpha");
        t.binary_alpha = read::boolean(&mut cx, v, "binary_alpha");
        t.dither = read::boolean(&mut cx, v, "dither");
        t.dither_each_mip = read::boolean(&mut cx, v, "dither_each_mip");
        t.implicit_light = read::boolean(&mut cx, v, "implicit_light");
        t.alpha_border = read::boolean(&mut cx, v, "alpha_border");
        t.color_border = read::boolean(&mut cx, v, "color_border");
        t.border_color = read::color(&mut cx, v, "border_color");
        t.fade_to_color = read::boolean(&mut cx, v, "fade_to_color");
        t.fade_to_alpha = read::boolean(&mut cx, v, "fade_to_alpha");
        t.fade_color = read::color(&mut cx, v, "fade_color");
        t.fade_amount = read::integer(&mut cx, v, TEXTURE, "fade_amount").map(|n| n as u32);
        t.fade_delay = read::integer(&mut cx, v, TEXTURE, "fade_delay").map(|n| n as u8);
        t.material = read::word(&mut cx, v, TEXTURE, "material");
        t.weight = read::number(&mut cx, v, TEXTURE, "weight");
        t.normal_map = read::text(&mut cx, v, "normal_map");
        t.extra_flags = read::integer(&mut cx, v, TEXTURE, "extra_flags").map(|n| n as u32);
        t.depth = read::number(&mut cx, v, TEXTURE, "depth");
        if let Some(b) = v.get("bump") {
            if b.is_table() {
                t.bump = Some(cx.within(&[Seg::key("bump")], |cx| Bump {
                    texture: read::text(cx, b, "texture"),
                    source: read::text(cx, b, "source"),
                    from: read::word(cx, b, BUMP, "from"),
                    depth: read::number(cx, b, BUMP, "depth"),
                    parallax: read::boolean(cx, b, "parallax"),
                }));
            } else if let Some(on) = b.as_bool() {
                // the game's short forms: true is a bump map under the texture's own name with
                // `_bump`, false none
                t.bump = on.then(Bump::default);
            } else {
                cx.bad("bump", "'bump' takes a table ({ source = ..., depth = ... }), true or false");
            }
        }
        if let Some(d) = v.get("detail") {
            if d.is_table() {
                t.detail = Some(cx.within(&[Seg::key("detail")], |cx| Detail {
                    texture: read::text(cx, d, "texture"),
                    scale: read::number(cx, d, DETAIL, "scale"),
                    on: read::word(cx, d, DETAIL, "on"),
                }));
            } else if d.as_bool() != Some(false) {
                // false says "none" outright, as the game reads it
                cx.bad("detail", "'detail' takes a table ({ texture = ..., scale = ... }) or false");
            }
        }
        (t, cx.issues)
    }

    /// The texture type `use` names.
    #[must_use]
    pub fn ty(&self) -> TextureType {
        TextureType(self.use_word.as_deref().and_then(|w| fields::value_of(fields::USES, w)).unwrap_or(0))
    }

    /// The bump map's game name: `bump.texture`, else `<name>_bump`.
    #[must_use]
    pub fn bump_name(&self, name: &str) -> Option<String> {
        let b = self.bump.as_ref()?;
        Some(b.texture.clone().unwrap_or_else(|| format!("{name}_bump")))
    }

    /// The description: `name` is the texture's game name (its bump map's is made of it),
    /// `size` the picture's width and height, `alpha` whether its file has an alpha channel.
    #[must_use]
    pub fn to_params(&self, name: &str, size: (u32, u32), alpha: bool) -> TextureParams {
        let ty = self.ty();
        let ui = self.use_word.as_deref() == Some("ui");
        let mut p = base(ty, ui, self.has_alpha.unwrap_or(alpha));
        p.width = size.0 as i32;
        p.height = size.1 as i32;
        if let Some(f) = self.format.filter(|f| *f != AUTO) {
            p.fmt = TextureFormat(f);
        }
        if let Some(m) = self.mip_filter {
            p.mip_filter = MipFilterId(m);
        }
        for (key, flag) in FLAG_KEYS {
            if let Some(on) = self.flag(key) {
                p.flags.set(flag, on);
            }
        }
        if let Some(c) = self.border_color {
            p.border_color = c;
        }
        if let Some(c) = self.fade_color {
            p.fade_color = c;
        }
        if let Some(n) = self.fade_amount {
            p.fade_amount = n;
        }
        if let Some(n) = self.fade_delay {
            p.fade_delay = n;
        }
        if let Some(m) = self.material {
            p.material = TextureMaterial(m);
        }
        if let Some(w) = self.weight {
            p.material_weight = w as f32;
        }
        if let Some(n) = &self.normal_map {
            p.ext_normal_map_name = n.as_str().into();
        }
        if let Some(d) = self.depth {
            p.bump_virtual_height = d as f32;
        }
        if let Some(b) = &self.bump {
            p.bump_mode = if b.parallax == Some(true) { BumpMode::USE_PARALLAX } else { BumpMode::USE };
            p.bump_name = self.bump_name(name).unwrap_or_default().into();
            p.bump_virtual_height = b.depth.unwrap_or(DEPTH) as f32;
        }
        if let Some(d) = &self.detail {
            p.detail_name = d.texture.clone().unwrap_or_default().into();
            p.detail_scale = d.scale.unwrap_or(1.0) as f32;
            let on = d.on.unwrap_or(1);
            p.flags.set(TextureFlags::DIFFUSE_DETAIL, on & 1 != 0);
            p.flags.set(TextureFlags::BUMP_DETAIL, on & 2 != 0);
        }
        if let Some(x) = self.extra_flags {
            p.flags |= TextureFlags::from_bits_retain(x & !named_bits().bits());
        }
        p
    }

    fn flag(&self, key: &str) -> Option<bool> {
        match key {
            "mips" => self.mips,
            "binary_alpha" => self.binary_alpha,
            "alpha_border" => self.alpha_border,
            "color_border" => self.color_border,
            "fade_to_color" => self.fade_to_color,
            "fade_to_alpha" => self.fade_to_alpha,
            "dither" => self.dither,
            "dither_each_mip" => self.dither_each_mip,
            "implicit_light" => self.implicit_light,
            "has_alpha" => self.has_alpha,
            _ => None,
        }
    }

    fn set_flag(&mut self, key: &str, on: bool) {
        let slot = match key {
            "mips" => &mut self.mips,
            "binary_alpha" => &mut self.binary_alpha,
            "alpha_border" => &mut self.alpha_border,
            "color_border" => &mut self.color_border,
            "fade_to_color" => &mut self.fade_to_color,
            "fade_to_alpha" => &mut self.fade_to_alpha,
            "dither" => &mut self.dither,
            "dither_each_mip" => &mut self.dither_each_mip,
            "implicit_light" => &mut self.implicit_light,
            _ => &mut self.has_alpha,
        };
        *slot = Some(on);
    }

    /// The asset that says what `p` says, writing only what differs from where the texture's
    /// type starts. `name` is the texture's game name, `alpha` whether its picture has alpha.
    #[must_use]
    pub fn from_params(p: &TextureParams, name: &str, alpha: bool) -> Self {
        let mut t = TextureAsset::default();
        if p.ty != TextureType::IMAGE {
            t.use_word = word_of(fields::USES, p.ty.0).map(str::to_owned);
        }
        let start = base(p.ty, false, p.has_alpha());
        if p.has_alpha() != alpha {
            t.has_alpha = Some(p.has_alpha());
        }
        if p.fmt != start.fmt {
            t.format = Some(p.fmt.0);
        }
        if p.mip_filter != start.mip_filter {
            t.mip_filter = Some(p.mip_filter.0);
        }
        for (key, flag) in FLAG_KEYS.into_iter().filter(|(k, _)| *k != "has_alpha") {
            let on = p.flags.contains(flag);
            if on != start.flags.contains(flag) {
                t.set_flag(key, on);
            }
        }
        if p.border_color != start.border_color {
            t.border_color = Some(p.border_color);
        }
        if p.fade_color != start.fade_color {
            t.fade_color = Some(p.fade_color);
        }
        if p.fade_amount != start.fade_amount {
            t.fade_amount = Some(p.fade_amount);
        }
        if p.fade_delay != start.fade_delay {
            t.fade_delay = Some(p.fade_delay);
        }
        if p.material != start.material {
            t.material = Some(p.material.0);
        }
        if p.material_weight != start.material_weight {
            t.weight = Some(f64::from(p.material_weight));
        }
        if !p.ext_normal_map_name.is_empty() {
            t.normal_map = Some(p.ext_normal_map_name.to_string());
        }
        if p.bump_mode == BumpMode::USE || p.bump_mode == BumpMode::USE_PARALLAX {
            let own = format!("{name}_bump");
            t.bump = Some(Bump {
                texture: (p.bump_name.as_str() != own).then(|| p.bump_name.to_string()),
                depth: (p.bump_virtual_height != DEPTH as f32).then_some(f64::from(p.bump_virtual_height)),
                parallax: (p.bump_mode == BumpMode::USE_PARALLAX).then_some(true),
                ..Bump::default()
            });
        }
        let on = u32::from(p.flags.contains(TextureFlags::DIFFUSE_DETAIL))
            | (u32::from(p.flags.contains(TextureFlags::BUMP_DETAIL)) << 1);
        if !p.detail_name.is_empty() || on != 0 || p.detail_scale != 1.0 {
            t.detail = Some(Detail {
                texture: (!p.detail_name.is_empty()).then(|| p.detail_name.to_string()),
                scale: (p.detail_scale != 1.0).then_some(f64::from(p.detail_scale)),
                on: (on != 1).then_some(on),
            });
        }
        let extra = p.flags.bits() & !named_bits().bits();
        if extra != 0 {
            t.extra_flags = Some(extra);
        }
        // the relief's depth of a texture with no bump of its own: a bump map's own
        if t.bump.is_none() && p.bump_virtual_height != start.bump_virtual_height {
            t.depth = Some(f64::from(p.bump_virtual_height));
        }
        t
    }

    /// The file's top-level table: the header first, then the keys in the table's order, each
    /// only when it is set.
    #[must_use]
    pub fn to_table(&self) -> Table {
        let mut top = Table::new();
        top.put("tex", 1);
        let words = |w: &'static [(&'static str, u32)], n: u32| Value::from(word_of(w, n).unwrap_or("auto"));
        for f in TEXTURE {
            let v: Option<Value> = match f.key {
                "source" => self.source.clone().map(Value::from),
                "name" => self.name.clone().map(Value::from),
                "use" => self.use_word.clone().map(Value::from),
                "format" => self.format.map(|n| words(fields::FORMATS, n)),
                "mip_filter" => self.mip_filter.map(|n| words(fields::MIP_FILTERS, n)),
                "border_color" => self.border_color.map(read::color_value),
                "fade_color" => self.fade_color.map(read::color_value),
                "fade_amount" => self.fade_amount.map(Value::from),
                "fade_delay" => self.fade_delay.map(|n| Value::from(u32::from(n))),
                "material" => self.material.map(|n| words(fields::MATERIALS, n)),
                "weight" => self.weight.map(Value::from),
                "normal_map" => self.normal_map.clone().map(Value::from),
                "extra_flags" => self.extra_flags.map(Value::from),
                "depth" => self.depth.map(Value::from),
                "bump" => self.bump.as_ref().map(|b| {
                    let mut t = Table::new();
                    if let Some(x) = &b.texture {
                        t.put("texture", x.as_str());
                    }
                    if let Some(x) = &b.source {
                        t.put("source", x.as_str());
                    }
                    if let Some(x) = b.from {
                        t.put("from", words(fields::BUMP_FROM, x));
                    }
                    if let Some(x) = b.depth {
                        t.put("depth", x);
                    }
                    if let Some(x) = b.parallax {
                        t.put("parallax", x);
                    }
                    // the pair under the texture's own name, said the game's short way
                    if t.is_empty() { Value::from(true) } else { Value::Table(t) }
                }),
                "detail" => self.detail.as_ref().map(|d| {
                    let mut t = Table::new();
                    if let Some(x) = &d.texture {
                        t.put("texture", x.as_str());
                    }
                    if let Some(x) = d.scale {
                        t.put("scale", x);
                    }
                    if let Some(x) = d.on {
                        t.put("on", words(fields::DETAIL_ON, x));
                    }
                    Value::Table(t)
                }),
                key => self.flag(key).map(Value::from),
            };
            if let Some(v) = v {
                top.put(f.key, v);
            }
        }
        top
    }
}

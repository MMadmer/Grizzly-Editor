//! The game's bitmap fonts as the preview draws them (`EditorPdaPreview.cpp`, `FontOf`,
//! `LayoutText`): a font names a section of `configs\fonts.ltx`, whose `texture` names a glyph
//! sheet and its `.ini` of `[symbol_coords]`; text is cp1251 and every byte is one glyph, so a
//! text is exactly as wide here as in the game. Pure functions.

use crate::gamefile::{LtxSection, comma_floats, ltx_value, trim};

/// The `fonts.ltx` section a layout's font name means (`FontSection`); the default font is
/// `letterica18`.
#[must_use]
pub fn font_section(name: &str) -> &'static str {
    match name {
        "letterica16" => "ui_font_letterica16_russian",
        "letterica25" => "ui_font_letter_25",
        "graffiti19" => "ui_font_graffiti19_russian",
        "graffiti22" => "ui_font_graffiti22_russian",
        "graffiti32" => "ui_font_graff_32",
        "graffiti50" => "ui_font_graff_50",
        "arial_14" => "ui_font_arial_14",
        "medium" => "hud_font_medium",
        "small" => "stat_font",
        "di" => "hud_font_di",
        _ => "ui_font_letterica18_russian",
    }
}

/// The glyph sheet a font draws from: `texture` of its section in `fonts.ltx`, lower case.
#[must_use]
pub fn font_texture(fonts_ltx: &str, name: &str) -> Option<String> {
    ltx_value(fonts_ltx, font_section(name), "texture").map(|t| trim(&t).to_ascii_lowercase())
}

/// Where each of the 256 glyphs lies on its sheet, in the sheet's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphTable {
    /// Glyph height in texture pixels: virtual units at 768 lines.
    pub height: f32,
    pub x: [f32; 256],
    pub y: [f32; 256],
    pub w: [f32; 256],
}

impl GlyphTable {
    #[must_use]
    pub fn width_of(&self, c: u8) -> f32 {
        self.w.get(usize::from(c)).copied().unwrap_or(0.0)
    }

    /// The width of a cp1251 text in glyph pixels.
    #[must_use]
    pub fn width(&self, cp: &[u8]) -> f32 {
        cp.iter().map(|&c| self.width_of(c)).sum()
    }
}

/// The glyph table of a font's `.ini` (`[symbol_coords]`: `height`, then `000` .. `255` as
/// `x1, y1, x2`). `None` without a height.
#[must_use]
pub fn parse_glyphs(ini: &str) -> Option<GlyphTable> {
    let sec = LtxSection::read(ini, "symbol_coords")?;
    let height = crate::gamefile::atof(sec.get("height")?);
    let mut t = GlyphTable { height, x: [0.0; 256], y: [0.0; 256], w: [0.0; 256] };
    for i in 0..256usize {
        let key = format!("{i:03}");
        let Some(v) = sec.get(&key) else { continue };
        if let Some(n) = comma_floats(v, 3)
            && let (Some(&x1), Some(&y1), Some(&x2)) = (n.first(), n.get(1), n.get(2))
            && let (Some(gx), Some(gy), Some(gw)) = (t.x.get_mut(i), t.y.get_mut(i), t.w.get_mut(i))
        {
            *gx = x1;
            *gy = y1;
            *gw = x2 - x1;
        }
    }
    Some(t)
}

/// One line of a label: its cp1251 bytes and their width in glyph pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub cp: Vec<u8>,
    pub w: f32,
}

impl Line {
    fn of(t: &GlyphTable, cp: Vec<u8>) -> Self {
        let w = t.width(&cp);
        Self { cp, w }
    }
}

/// The lines a label shows in `max_w` glyph pixels (`LayoutText`): every paragraph (`\n`) one
/// line cut with `...`, or with `wrap` its words wrapped - a word wider than the line breaks
/// where it must.
#[must_use]
pub fn layout_text(t: &GlyphTable, cp: &[u8], max_w: f32, wrap: bool) -> Vec<Line> {
    let mut out = Vec::new();
    for para in cp.split(|&c| c == b'\n') {
        if !wrap {
            let mut l = Line::of(t, para.to_vec());
            if l.w > max_w && max_w > 0.0 {
                let dots = t.width(b"...");
                while !l.cp.is_empty() && t.width(&l.cp) + dots > max_w {
                    l.cp.pop();
                }
                l.cp.extend_from_slice(b"...");
                l.w = t.width(&l.cp);
            }
            out.push(l);
            continue;
        }
        let mut line: Vec<u8> = Vec::new();
        let mut i = 0usize;
        while i <= para.len() {
            let e =
                para.get(i..).and_then(|r| r.iter().position(|&c| c == b' ')).map_or(para.len(), |k| i + k);
            let word = para.get(i..e).unwrap_or(&[]);
            let mut cand = line.clone();
            if !cand.is_empty() {
                cand.push(b' ');
            }
            cand.extend_from_slice(word);
            if !line.is_empty() && t.width(&cand) > max_w {
                out.push(Line::of(t, std::mem::take(&mut line)));
                line = word.to_vec();
            } else {
                line = cand;
            }
            while t.width(&line) > max_w && line.len() > 1 {
                let mut cut = line.len();
                while cut > 1 && t.width(line.get(..cut).unwrap_or(&[])) > max_w {
                    cut -= 1;
                }
                let head: Vec<u8> = line.drain(..cut).collect();
                out.push(Line::of(t, head));
            }
            i = e + 1;
        }
        out.push(Line::of(t, line));
    }
    out
}

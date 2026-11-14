//! Colour arithmetic of the themes: mixing in sRGB, the WCAG 2 contrast ratio and OKLCH.
//!
//! Everything here works on opaque colours; a token with an opacity is made from one at the end.

use egui::Color32;

/// An opaque colour from `0xrrggbb`.
#[must_use]
pub const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `a` to `b` by `t` (0..1), in sRGB.
#[must_use]
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

fn linear(c: u8) -> f32 {
    let s = f32::from(c) / 255.0;
    if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn gamma(x: f32) -> f32 {
    if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

/// The relative luminance of WCAG 2.
#[must_use]
pub fn luminance(c: Color32) -> f32 {
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// The contrast ratio of WCAG 2, 1 to 21.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The least step of `c` toward `toward` that stands at least `min` against each of `on`;
/// `toward` itself when no step does.
#[must_use]
pub fn reach(c: Color32, toward: Color32, on: &[Color32], min: f32) -> Color32 {
    const STEPS: u16 = 64;
    for i in 0..=STEPS {
        let x = mix(c, toward, f32::from(i) / f32::from(STEPS));
        if on.iter().all(|b| contrast(x, *b) >= min) {
            return x;
        }
    }
    toward
}

/// A colour in OKLCH: lightness 0..1, chroma, hue in degrees.
#[must_use]
pub fn to_oklch(c: Color32) -> [f32; 3] {
    let (r, g, b) = (linear(c.r()), linear(c.g()), linear(c.b()));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let ll = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let aa = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [ll, aa.hypot(bb), bb.atan2(aa).to_degrees().rem_euclid(360.0)]
}

/// An OKLCH colour in sRGB, `None` when it lies outside the gamut.
#[must_use]
pub fn from_oklch(l: f32, c: f32, h: f32) -> Option<Color32> {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let rgb = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ];
    if rgb.iter().any(|v| !(-1e-4..=1.0001).contains(v)) {
        return None;
    }
    let ch = |v: f32| (gamma(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    Some(Color32::from_rgb(ch(rgb[0]), ch(rgb[1]), ch(rgb[2])))
}

/// How far apart two colours are in OKLab: about 0.02 is a difference one barely sees.
#[must_use]
pub fn distance(a: Color32, b: Color32) -> f32 {
    let lab = |c: Color32| {
        let [l, ch, h] = to_oklch(c);
        [l, ch * h.to_radians().cos(), ch * h.to_radians().sin()]
    };
    let (x, y) = (lab(a), lab(b));
    ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
}

/// The OKLCH colour of lightness `l` and hue `h` with as much of the chroma `c` as sRGB holds.
#[must_use]
pub fn oklch_fit(l: f32, c: f32, h: f32) -> Color32 {
    let mut chroma = c;
    loop {
        if let Some(x) = from_oklch(l, chroma, h) {
            return x;
        }
        if chroma <= 0.0 {
            return from_oklch(l, 0.0, h).unwrap_or(Color32::GRAY);
        }
        chroma = (chroma - 0.005).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{contrast, from_oklch, mix, reach, rgb, to_oklch};
    use egui::Color32;

    #[test]
    fn the_arithmetic_holds() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(rgb(0x777777), rgb(0x777777)) - 1.0).abs() < 1e-4);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.5), rgb(0x808080));
        // a round trip through OKLCH keeps the colour
        for hex in [0x7971bd, 0xe06c75, 0x98c379, 0x202025, 0xf2effb] {
            let [l, c, h] = to_oklch(rgb(hex));
            assert_eq!(from_oklch(l, c, h), Some(rgb(hex)), "{hex:06x}");
        }
        let grey = reach(rgb(0x555555), Color32::WHITE, &[rgb(0x202020)], 4.5);
        assert!(contrast(grey, rgb(0x202020)) >= 4.5);
    }
}

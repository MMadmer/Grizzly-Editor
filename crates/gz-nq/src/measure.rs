//! How wide a text is drawn on a graph node, without a window: the advance widths of the canvas's
//! faces (`metrics_table`) times the size. The layout sizes a node's box with it and the canvas
//! draws into that box, so a node is as wide as what it says (180-360 pt) everywhere alike.

use crate::metrics_table::{Face as Table, REGULAR, SEMIBOLD};

/// The face a text of a node is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Lines, actions, the id, the comment.
    Body,
    /// The title.
    Strong,
}

fn table(face: Face) -> &'static Table {
    match face {
        Face::Body => &REGULAR,
        Face::Strong => &SEMIBOLD,
    }
}

fn advance(t: &Table, c: char) -> u16 {
    let cp = c as u32;
    for (first, row) in t.ranges {
        if cp >= *first
            && let Some(w) = row.get((cp - first) as usize)
        {
            return if *w == 0 { t.fallback } else { *w };
        }
    }
    t.fallback
}

/// The width of `text` set in `face` at `em` (the font's em size, pt).
#[must_use]
pub fn width(face: Face, em: f32, text: &str) -> f32 {
    let t = table(face);
    let units: u32 = text.chars().map(|c| u32::from(advance(t, c))).sum();
    units as f32 * em / f32::from(t.upem)
}

/// The longest prefix of `text` that fits `max` with an ellipsis after it, with the ellipsis; the
/// whole text when it fits.
#[must_use]
pub fn clip(face: Face, em: f32, text: &str, max: f32) -> String {
    if width(face, em, text) <= max {
        return text.to_owned();
    }
    let t = table(face);
    let dots = f32::from(advance(t, '\u{2026}')) * em / f32::from(t.upem);
    let mut w = 0.0;
    let mut out = String::new();
    for c in text.chars() {
        let a = f32::from(advance(t, c)) * em / f32::from(t.upem);
        if w + a + dots > max {
            break;
        }
        w += a;
        out.push(c);
    }
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_face() {
        // a figure is 1104/2048 em in Segoe UI
        let w = width(Face::Body, 12.0, "1");
        assert!((w - 1104.0 * 12.0 / 2048.0).abs() < 0.01, "{w}");
        // semibold is wider than regular
        assert!(width(Face::Strong, 12.0, "Spawn NPCs") > width(Face::Body, 12.0, "Spawn NPCs"));
        // Cyrillic is in the table
        assert!(width(Face::Body, 12.0, "Ночь") > 20.0);
        let cut = clip(Face::Body, 12.0, "A long title that does not fit", 60.0);
        assert!(cut.ends_with('\u{2026}') && width(Face::Body, 12.0, &cut) <= 60.0, "{cut}");
        assert_eq!(clip(Face::Body, 12.0, "Fits", 60.0), "Fits");
    }
}

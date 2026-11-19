//! Pictures made on the processor.
//!
//! A [`Canvas`] of RGBA pixels with what a schematic needs - lines, rectangles, filled polygons,
//! discs, text in a built-in dot-matrix face - and PNG out. No window, no graphics card, no font
//! file: the same input gives the same bytes on every machine, so a picture can be a test's
//! expectation and a command's answer in a run without a window.
//!
//! Coordinates are pixels, x to the right and y down, the origin the top left corner. Everything
//! is clipped to the canvas: a shape that lies partly or wholly outside is drawn where it is
//! inside, and a coordinate that is no number draws nothing.

mod font;
pub mod raster;

pub use font::{ADVANCE, GLYPH_H, GLYPH_W, LINE, text_size};

/// A colour: red, green, blue and how much of it covers what is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const BLACK: Self = Self(0, 0, 0, 255);
    pub const WHITE: Self = Self(255, 255, 255, 255);
    pub const CLEAR: Self = Self(0, 0, 0, 0);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }

    /// The same colour covering `alpha` of what is under it.
    pub const fn alpha(self, alpha: u8) -> Self {
        Self(self.0, self.1, self.2, alpha)
    }

    /// A colour from a hue in degrees and a saturation and value from 0 to 1.
    pub fn hsv(hue: f32, saturation: f32, value: f32) -> Self {
        let h = if hue.is_finite() { hue.rem_euclid(360.0) / 60.0 } else { 0.0 };
        let s = saturation.clamp(0.0, 1.0);
        let v = value.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let byte = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self(byte(r), byte(g), byte(b), 255)
    }

    /// This colour with `t` (0 to 1) of `other` mixed in; the alphas mix too.
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let f = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Self(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2), f(self.3, other.3))
    }

    /// The colour made darker (`factor` under 1) or lighter (over 1), its alpha kept.
    pub fn shade(self, factor: f32) -> Self {
        let k = if factor.is_finite() { factor.max(0.0) } else { 1.0 };
        let f = |a: u8| (f32::from(a) * k).round().clamp(0.0, 255.0) as u8;
        Self(f(self.0), f(self.1), f(self.2), self.3)
    }

    /// How bright the colour looks, from 0 to 1.
    pub fn luma(self) -> f32 {
        (0.299 * f32::from(self.0) + 0.587 * f32::from(self.1) + 0.114 * f32::from(self.2)) / 255.0
    }
}

/// A picture being drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    /// Four bytes a pixel, rows from the top.
    data: Vec<u8>,
}

/// The largest side of a canvas: a picture past it is somebody's mistake, not a picture.
pub const MAX_SIDE: u32 = 8192;

impl Canvas {
    /// A canvas of one colour. A side is at least 1 and at most [`MAX_SIDE`].
    pub fn new(width: u32, height: u32, fill: Rgba) -> Self {
        let width = width.clamp(1, MAX_SIDE);
        let height = height.clamp(1, MAX_SIDE);
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            data.extend_from_slice(&[fill.0, fill.1, fill.2, fill.3]);
        }
        Self { width, height, data }
    }

    /// A canvas over pixels that exist: four bytes a pixel, rows from the top. `None` when the
    /// bytes are not `width * height * 4` or a side is out of range.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        let sound = (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height);
        (sound && data.len() == width as usize * height as usize * 4).then_some(Self { width, height, data })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels: four bytes each, rows from the top.
    pub fn rgba(&self) -> &[u8] {
        &self.data
    }

    fn index(&self, x: i64, y: i64) -> Option<usize> {
        (x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height))
            .then(|| (y as usize * self.width as usize + x as usize) * 4)
    }

    /// The colour of a pixel; `None` outside the canvas.
    pub fn pixel(&self, x: i64, y: i64) -> Option<Rgba> {
        let i = self.index(x, y)?;
        Some(Rgba(self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]))
    }

    /// Draws one pixel: the colour over what is there, by the colour's alpha.
    pub fn plot(&mut self, x: i64, y: i64, c: Rgba) {
        let Some(i) = self.index(x, y) else { return };
        if c.3 == 255 {
            self.data[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, 255]);
            return;
        }
        if c.3 == 0 {
            return;
        }
        let a = u32::from(c.3);
        let over = |src: u8, dst: u8| ((u32::from(src) * a + u32::from(dst) * (255 - a) + 127) / 255) as u8;
        self.data[i] = over(c.0, self.data[i]);
        self.data[i + 1] = over(c.1, self.data[i + 1]);
        self.data[i + 2] = over(c.2, self.data[i + 2]);
        self.data[i + 3] = (a + u32::from(self.data[i + 3]) * (255 - a) / 255).min(255) as u8;
    }

    /// Fills the rectangle from `x0, y0` up to but not including `x1, y1`.
    pub fn fill_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgba) {
        let (x0, x1) = (x0.min(x1).max(0), x0.max(x1).min(i64::from(self.width)));
        let (y0, y1) = (y0.min(y1).max(0), y0.max(y1).min(i64::from(self.height)));
        for y in y0..y1 {
            for x in x0..x1 {
                self.plot(x, y, c);
            }
        }
    }

    /// The outline of the rectangle from `x0, y0` up to but not including `x1, y1`, one pixel wide.
    pub fn rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgba) {
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        self.fill_rect(x0, y0, x1, y0 + 1, c);
        self.fill_rect(x0, y1 - 1, x1, y1, c);
        self.fill_rect(x0, y0 + 1, x0 + 1, y1 - 1, c);
        self.fill_rect(x1 - 1, y0 + 1, x1, y1 - 1, c);
    }

    /// A line from one point to another, `width` pixels thick (at least 1). A coordinate names a
    /// pixel: a line from 1 to 8 covers the pixels 1 to 8. Every pixel is drawn once, so a
    /// translucent line is even.
    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: u32, c: Rgba) {
        let Some((a, b)) = self.clip(from, to, width as f32) else { return };
        if width > 1 {
            // a thick line is the rectangle around its segment, between the pixels' centres
            let (ax, ay, bx, by) = (a.0 + 0.5, a.1 + 0.5, b.0 + 0.5, b.1 + 0.5);
            let (dx, dy) = (bx - ax, by - ay);
            let len = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = if len > 0.0 { (dx / len, dy / len) } else { (1.0, 0.0) };
            let h = width as f32 * 0.5;
            let (ex, ey) = (ux * 0.5, uy * 0.5);
            let (nx, ny) = (-uy * h, ux * h);
            self.polygon(
                &[
                    (ax - ex + nx, ay - ey + ny),
                    (bx + ex + nx, by + ey + ny),
                    (bx + ex - nx, by + ey - ny),
                    (ax - ex - nx, ay - ey - ny),
                ],
                c,
            );
            return;
        }
        let (mut x, mut y) = (a.0.round() as i64, a.1.round() as i64);
        let (x1, y1) = (b.0.round() as i64, b.1.round() as i64);
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.plot(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// The part of a segment that can reach the canvas (grown by `margin`), or `None`.
    fn clip(&self, a: (f32, f32), b: (f32, f32), margin: f32) -> Option<((f32, f32), (f32, f32))> {
        if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
            return None;
        }
        // in doubles: an end a thousand kilometres away must not move the part that is seen
        let (ax, ay, bx, by) = (f64::from(a.0), f64::from(a.1), f64::from(b.0), f64::from(b.1));
        let m = f64::from(margin.max(1.0)) + 1.0;
        let (xmin, ymin, xmax, ymax) = (-m, -m, f64::from(self.width) + m, f64::from(self.height) + m);
        // Liang-Barsky
        let (dx, dy) = (bx - ax, by - ay);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (p, q) in [(-dx, ax - xmin), (dx, xmax - ax), (-dy, ay - ymin), (dy, ymax - ay)] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    if r > t1 {
                        return None;
                    }
                    t0 = t0.max(r);
                } else {
                    if r < t0 {
                        return None;
                    }
                    t1 = t1.min(r);
                }
            }
        }
        let at = |t: f64| ((ax + t * dx) as f32, (ay + t * dy) as f32);
        Some((at(t0), at(t1)))
    }

    /// Lines through the points in order; `closed` joins the last to the first.
    pub fn polyline(&mut self, points: &[(f32, f32)], closed: bool, width: u32, c: Rgba) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1], width, c);
        }
        if closed && points.len() > 2 {
            self.line(points[points.len() - 1], points[0], width, c);
        }
    }

    /// Fills a polygon (any shape; a point is inside by the even-odd rule). A pixel is filled
    /// when its centre is inside.
    pub fn polygon(&mut self, points: &[(f32, f32)], c: Rgba) {
        if points.len() < 3 || points.iter().any(|p| !(p.0.is_finite() && p.1.is_finite())) {
            return;
        }
        let top = points.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let bottom = points.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        let y0 = (top - 0.5).ceil().max(0.0) as i64;
        let y1 = ((bottom - 0.5).floor() as i64).min(i64::from(self.height) - 1);
        let mut cuts: Vec<f32> = Vec::with_capacity(8);
        for y in y0..=y1 {
            let yc = y as f32 + 0.5;
            cuts.clear();
            for i in 0..points.len() {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                // an edge owns its upper end and not its lower one, so a vertex is counted once
                if (a.1 <= yc) != (b.1 <= yc) {
                    cuts.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            cuts.sort_by(f32::total_cmp);
            for [from, to] in cuts.as_chunks::<2>().0 {
                let x0 = (from - 0.5).ceil().max(0.0) as i64;
                let x1 = ((to - 0.5).floor() as i64).min(i64::from(self.width) - 1);
                for x in x0..=x1 {
                    self.plot(x, y, c);
                }
            }
        }
    }

    /// A filled circle.
    pub fn disc(&mut self, centre: (f32, f32), radius: f32, c: Rgba) {
        if !(centre.0.is_finite() && centre.1.is_finite() && radius.is_finite()) || radius <= 0.0 {
            return;
        }
        let r = radius.min(MAX_SIDE as f32);
        let y0 = ((centre.1 - r).floor() as i64).max(0);
        let y1 = ((centre.1 + r).ceil() as i64).min(i64::from(self.height) - 1);
        let x0 = ((centre.0 - r).floor() as i64).max(0);
        let x1 = ((centre.0 + r).ceil() as i64).min(i64::from(self.width) - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - centre.0, y as f32 + 0.5 - centre.1);
                if dx * dx + dy * dy <= r * r {
                    self.plot(x, y, c);
                }
            }
        }
    }

    /// One line of text, its top left corner at `x, y`, every dot `scale` pixels (at least 1).
    /// A character the face lacks is drawn as `?`. Answers the x past the last glyph.
    pub fn text(&mut self, x: i64, y: i64, text: &str, scale: u32, c: Rgba) -> i64 {
        let s = i64::from(scale.max(1));
        let mut pen = x;
        for ch in text.chars() {
            let g = font::glyph(ch);
            for col in 0..GLYPH_W {
                for row in 0..GLYPH_H {
                    if font::dot(g, col, row) {
                        let (px, py) = (pen + i64::from(col) * s, y + i64::from(row) * s);
                        self.fill_rect(px, py, px + s, py + s, c);
                    }
                }
            }
            pen += i64::from(ADVANCE) * s;
        }
        pen
    }

    /// Text on a plate of its own, so it reads over anything: `x, y` is the plate's top left
    /// corner, the plate is `pad` pixels larger than the text on every side. Answers the plate's
    /// rectangle `(x0, y0, x1, y1)`.
    #[allow(clippy::too_many_arguments)]
    pub fn label(
        &mut self,
        x: i64,
        y: i64,
        text: &str,
        scale: u32,
        pad: u32,
        ink: Rgba,
        plate: Rgba,
    ) -> (i64, i64, i64, i64) {
        let (w, h) = text_size(text, scale);
        let p = i64::from(pad);
        let r = (x, y, x + i64::from(w) + 2 * p, y + i64::from(h) + 2 * p);
        self.fill_rect(r.0, r.1, r.2, r.3, plate);
        self.text(x + p, y + p, text, scale, ink);
        r
    }

    /// Draws another canvas with its top left corner at `x, y`, by its pixels' alpha.
    pub fn blit(&mut self, other: &Canvas, x: i64, y: i64) {
        for oy in 0..i64::from(other.height) {
            for ox in 0..i64::from(other.width) {
                if let Some(c) = other.pixel(ox, oy) {
                    self.plot(x + ox, y + oy, c);
                }
            }
        }
    }

    /// The picture as a PNG file's bytes: 8 bits a channel with alpha, no time stamp, so the
    /// same pixels are the same bytes.
    pub fn png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.data)?;
        }
        Ok(out)
    }

    /// A picture read back from the PNG [`Self::png`] wrote; `None` for any other file.
    #[must_use]
    pub fn from_png(bytes: &[u8]) -> Option<Self> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
        let mut data = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut data).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        data.truncate(info.buffer_size());
        Self::from_rgba(info.width, info.height, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(c: &Canvas, colour: Rgba) -> usize {
        c.rgba().as_chunks::<4>().0.iter().filter(|p| **p == [colour.0, colour.1, colour.2, colour.3]).count()
    }

    #[test]
    fn a_canvas_starts_in_one_colour_and_keeps_its_size_in_range() {
        let c = Canvas::new(4, 3, Rgba::rgb(1, 2, 3));
        assert_eq!((c.width(), c.height()), (4, 3));
        assert_eq!(count(&c, Rgba::rgb(1, 2, 3)), 12);
        let c = Canvas::new(0, MAX_SIDE + 5, Rgba::BLACK);
        assert_eq!((c.width(), c.height()), (1, MAX_SIDE));
        assert!(Canvas::from_rgba(2, 2, vec![0; 16]).is_some());
        assert!(Canvas::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Canvas::from_rgba(0, 2, Vec::new()).is_none());
    }

    #[test]
    fn a_pixel_outside_is_nothing_and_draws_nothing() {
        let mut c = Canvas::new(2, 2, Rgba::BLACK);
        c.plot(-1, 0, Rgba::WHITE);
        c.plot(2, 0, Rgba::WHITE);
        c.plot(0, 2, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::BLACK), 4);
        assert_eq!(c.pixel(5, 5), None);
        c.plot(1, 1, Rgba::WHITE);
        assert_eq!(c.pixel(1, 1), Some(Rgba::WHITE));
    }

    #[test]
    fn a_translucent_colour_mixes_with_what_is_under_it() {
        let mut c = Canvas::new(1, 1, Rgba::rgb(0, 0, 0));
        c.plot(0, 0, Rgba::rgb(255, 255, 255).alpha(128));
        let Some(p) = c.pixel(0, 0) else { panic!("no pixel") };
        assert!((127..=129).contains(&p.0) && p.3 == 255, "{p:?}");
        // nothing of a clear colour
        c.plot(0, 0, Rgba::WHITE.alpha(0));
        assert_eq!(c.pixel(0, 0), Some(p));
    }

    #[test]
    fn a_rectangle_is_filled_up_to_its_far_edge_and_clipped() {
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.fill_rect(2, 3, 5, 7, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 3 * 4);
        assert_eq!(c.pixel(5, 3), Some(Rgba::BLACK));
        let mut c = Canvas::new(4, 4, Rgba::BLACK);
        c.fill_rect(-100, -100, 100, 100, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 16);
        // the corners in any order
        let mut c = Canvas::new(4, 4, Rgba::BLACK);
        c.fill_rect(3, 3, 1, 1, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 4);
        let mut c = Canvas::new(6, 6, Rgba::BLACK);
        c.rect(1, 1, 5, 5, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 12);
        assert_eq!(c.pixel(2, 2), Some(Rgba::BLACK));
    }

    #[test]
    fn a_line_joins_its_ends_and_survives_any_numbers() {
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.line((1.0, 1.0), (8.0, 1.0), 1, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 8);
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.line((0.0, 0.0), (9.0, 9.0), 1, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 10);
        // far outside, crossing the canvas: only what is inside, and at once
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.line((-1.0e9, 5.0), (1.0e9, 5.0), 1, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 10);
        // wholly outside, and no numbers at all
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.line((-50.0, -50.0), (-20.0, -5.0), 3, Rgba::WHITE);
        c.line((f32::NAN, 1.0), (2.0, 2.0), 1, Rgba::WHITE);
        c.line((1.0, 1.0), (f32::INFINITY, 2.0), 1, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 0);
    }

    #[test]
    fn a_thick_translucent_line_covers_each_pixel_once() {
        let ink = Rgba::WHITE.alpha(128);
        let mut c = Canvas::new(12, 12, Rgba::BLACK);
        c.line((2.0, 6.0), (9.0, 6.0), 3, ink);
        let mut once = Canvas::new(1, 1, Rgba::BLACK);
        once.plot(0, 0, ink);
        let Some(shade) = once.pixel(0, 0) else { panic!("no pixel") };
        assert_eq!(count(&c, shade), 8 * 3);
        assert_eq!(count(&c, Rgba::BLACK), 144 - 24);
        // a slanted one too: no pixel is darker or lighter than the rest
        let mut c = Canvas::new(24, 24, Rgba::BLACK);
        c.line((3.0, 4.0), (19.0, 17.0), 4, ink);
        assert_eq!(count(&c, shade) + count(&c, Rgba::BLACK), 24 * 24);
        assert!(count(&c, shade) > 60);
    }

    #[test]
    fn a_polygon_fills_the_pixels_whose_centres_are_inside() {
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        c.polygon(&[(2.0, 2.0), (6.0, 2.0), (6.0, 5.0), (2.0, 5.0)], Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 4 * 3);
        // two squares side by side share an edge and no pixel
        let mut c = Canvas::new(10, 10, Rgba::BLACK);
        let ink = Rgba::WHITE.alpha(128);
        c.polygon(&[(1.0, 1.0), (4.0, 1.0), (4.0, 4.0), (1.0, 4.0)], ink);
        c.polygon(&[(4.0, 1.0), (7.0, 1.0), (7.0, 4.0), (4.0, 4.0)], ink);
        let mut once = Canvas::new(1, 1, Rgba::BLACK);
        once.plot(0, 0, ink);
        let Some(shade) = once.pixel(0, 0) else { panic!("no pixel") };
        assert_eq!(count(&c, shade), 18);
        // a triangle: half of its square, give or take the diagonal
        let mut c = Canvas::new(20, 20, Rgba::BLACK);
        c.polygon(&[(0.0, 0.0), (20.0, 0.0), (0.0, 20.0)], Rgba::WHITE);
        let n = count(&c, Rgba::WHITE);
        assert!((190..=210).contains(&n), "{n}");
        // partly outside, and with a number that is none
        let mut c = Canvas::new(4, 4, Rgba::BLACK);
        c.polygon(&[(-10.0, -10.0), (30.0, -10.0), (30.0, 30.0), (-10.0, 30.0)], Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 16);
        let mut c = Canvas::new(4, 4, Rgba::BLACK);
        c.polygon(&[(0.0, 0.0), (f32::NAN, 0.0), (3.0, 3.0)], Rgba::WHITE);
        c.polygon(&[(0.0, 0.0), (3.0, 3.0)], Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 0);
    }

    #[test]
    fn a_disc_is_round() {
        let mut c = Canvas::new(21, 21, Rgba::BLACK);
        c.disc((10.5, 10.5), 5.0, Rgba::WHITE);
        let n = count(&c, Rgba::WHITE) as f32;
        assert!((n - std::f32::consts::PI * 25.0).abs() < 8.0, "{n}");
        assert_eq!(c.pixel(10, 10), Some(Rgba::WHITE));
        assert_eq!(c.pixel(0, 0), Some(Rgba::BLACK));
        c.disc((f32::NAN, 0.0), 3.0, Rgba::WHITE);
        c.disc((1.0, 1.0), -1.0, Rgba::WHITE);
    }

    #[test]
    fn text_is_drawn_dot_by_dot_at_its_scale() {
        // "1": five columns of 0x00, 0x42, 0x7F, 0x40, 0x00 - 2 + 7 + 1 dots
        let mut c = Canvas::new(8, 8, Rgba::BLACK);
        let end = c.text(0, 0, "1", 1, Rgba::WHITE);
        assert_eq!(end, 6);
        assert_eq!(count(&c, Rgba::WHITE), 10);
        // the stem is the middle column
        assert!((0..7).all(|y| c.pixel(2, y) == Some(Rgba::WHITE)));
        let mut c = Canvas::new(16, 16, Rgba::BLACK);
        c.text(0, 0, "1", 2, Rgba::WHITE);
        assert_eq!(count(&c, Rgba::WHITE), 40);
        // a character the face lacks is a question mark, and text off the canvas is clipped
        let mut a = Canvas::new(8, 8, Rgba::BLACK);
        let mut b = Canvas::new(8, 8, Rgba::BLACK);
        a.text(0, 0, "\u{0416}", 1, Rgba::WHITE);
        b.text(0, 0, "?", 1, Rgba::WHITE);
        assert_eq!(a, b);
        a.text(-100, 500, "far away", 3, Rgba::WHITE);
        assert_eq!(a, b);
    }

    #[test]
    fn a_label_is_text_on_its_plate() {
        let mut c = Canvas::new(40, 20, Rgba::BLACK);
        let plate = Rgba::rgb(10, 20, 30);
        let r = c.label(2, 3, "12", 1, 2, Rgba::WHITE, plate);
        assert_eq!(r, (2, 3, 2 + 11 + 4, 3 + 7 + 4));
        assert_eq!(c.pixel(2, 3), Some(plate));
        assert_eq!(c.pixel(1, 3), Some(Rgba::BLACK));
        assert!(count(&c, Rgba::WHITE) > 10);
    }

    #[test]
    fn one_canvas_is_drawn_into_another() {
        let mut small = Canvas::new(2, 2, Rgba::WHITE);
        small.plot(0, 0, Rgba::CLEAR);
        let mut big = Canvas::new(4, 4, Rgba::BLACK);
        big.blit(&small, 3, 3);
        assert_eq!(count(&big, Rgba::WHITE), 1);
        big.blit(&small, 1, 1);
        assert_eq!(count(&big, Rgba::WHITE), 5);
    }

    #[test]
    fn colours_mix_shade_and_come_from_a_hue() {
        assert_eq!(Rgba::hsv(0.0, 1.0, 1.0), Rgba::rgb(255, 0, 0));
        assert_eq!(Rgba::hsv(120.0, 1.0, 1.0), Rgba::rgb(0, 255, 0));
        assert_eq!(Rgba::hsv(240.0, 1.0, 1.0), Rgba::rgb(0, 0, 255));
        assert_eq!(Rgba::hsv(360.0 + 120.0, 1.0, 1.0), Rgba::rgb(0, 255, 0));
        assert_eq!(Rgba::hsv(f32::NAN, 0.0, 0.5), Rgba::rgb(128, 128, 128));
        assert_eq!(Rgba::BLACK.mix(Rgba::WHITE, 0.5), Rgba::rgb(128, 128, 128));
        assert_eq!(Rgba::BLACK.mix(Rgba::WHITE, 7.0), Rgba::WHITE);
        assert_eq!(Rgba::rgb(100, 200, 50).shade(0.5), Rgba::rgb(50, 100, 25));
        assert_eq!(Rgba::rgb(200, 200, 200).shade(2.0), Rgba::WHITE);
        assert!(Rgba::WHITE.luma() > 0.99 && Rgba::BLACK.luma() < 0.01);
    }

    #[test]
    fn the_png_reads_back_to_the_same_pixels_and_is_the_same_twice() {
        let mut c = Canvas::new(7, 5, Rgba::rgb(9, 8, 7));
        c.line((0.0, 0.0), (6.0, 4.0), 1, Rgba::WHITE);
        c.text(0, 0, "A", 1, Rgba::rgb(255, 0, 0).alpha(200));
        let Ok(bytes) = c.png() else { panic!("no png") };
        assert_eq!(c.png().ok().as_ref(), Some(&bytes));
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let Ok(mut reader) = decoder.read_info() else { panic!("no header") };
        let Some(size) = reader.output_buffer_size() else { panic!("no size") };
        let mut pixels = vec![0u8; size];
        let Ok(info) = reader.next_frame(&mut pixels) else { panic!("no frame") };
        assert_eq!((info.width, info.height), (7, 5));
        assert_eq!(&pixels[..info.buffer_size()], c.rgba());
    }
}

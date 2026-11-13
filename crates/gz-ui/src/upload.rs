//! [`Tiled`] - a big picture that reaches the GPU a few tiles a frame (docs/UI_DESIGN.md,
//! section 4).
//!
//! Handing egui a 2048 x 2048 picture in one go is 16 MB copied and uploaded on the UI thread in
//! the next frame: one long frame. A `Tiled` picture is cut into tiles of 512 x 512 once (on any
//! thread: [`Tiled::new`] takes the decoded pixels), and [`Tiled::pump`] - called every frame it
//! is drawn - hands egui at most a frame's budget of them; the frames in between show what has
//! arrived, the rest in the placeholder colour. [`Tiled::paint`] draws any part of it into a
//! rectangle, the tiles mapped to their place.

use std::collections::VecDeque;
use std::sync::Arc;

use egui::{Color32, ColorImage, Context, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2, pos2};

/// The side of a tile, in pixels.
pub const TILE: usize = 512;
/// How many pixels a frame may hand over: one 2048 x 2048 picture takes four frames.
pub const FRAME_BUDGET: usize = 1024 * 1024;

struct Tile {
    /// Its place in the picture, in pixels.
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// A picture uploaded across frames.
pub struct Tiled {
    name: String,
    size: [usize; 2],
    options: TextureOptions,
    tiles: Vec<Tile>,
    /// The pixels of the tiles still to go, in upload order.
    pending: VecDeque<(usize, Arc<ColorImage>)>,
    handles: Vec<Option<TextureHandle>>,
}

impl Tiled {
    /// Cuts `image` into tiles. No GPU work happens here: it may run on a pool.
    #[must_use]
    pub fn new(name: &str, image: &ColorImage, options: TextureOptions) -> Self {
        let [w, h] = image.size;
        let mut tiles = Vec::new();
        let mut pending = VecDeque::new();
        let mut y = 0;
        while y < h {
            let th = TILE.min(h - y);
            let mut x = 0;
            while x < w {
                let tw = TILE.min(w - x);
                let mut pixels = Vec::with_capacity(tw * th);
                for row in y..y + th {
                    let start = row * w + x;
                    pixels.extend_from_slice(image.pixels.get(start..start + tw).unwrap_or_default());
                }
                let tile_image = ColorImage::new([tw, th], pixels);
                pending.push_back((tiles.len(), Arc::new(tile_image)));
                tiles.push(Tile { x, y, w: tw, h: th });
                x += TILE;
            }
            y += TILE;
        }
        let n = tiles.len();
        Self { name: name.to_owned(), size: [w, h], options, tiles, pending, handles: vec![None; n] }
    }

    /// The picture's size in pixels.
    #[must_use]
    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    /// Every tile is on the GPU.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.pending.is_empty()
    }

    /// How far the upload is, 0..1.
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.tiles.is_empty() { 1.0 } else { 1.0 - self.pending.len() as f32 / self.tiles.len() as f32 }
    }

    /// Hands egui the next tiles, up to `budget` pixels (at least one tile), and asks for another
    /// frame while some are left. Answers how many tiles went.
    pub fn pump(&mut self, ctx: &Context, budget: usize) -> usize {
        let mut spent = 0;
        let mut sent = 0;
        while let Some((i, image)) = self.pending.front().cloned() {
            let px = image.size[0] * image.size[1];
            if sent > 0 && spent + px > budget {
                break;
            }
            self.pending.pop_front();
            let handle = ctx.load_texture(format!("{}#{i}", self.name), image, self.options);
            if let Some(slot) = self.handles.get_mut(i) {
                *slot = Some(handle);
            }
            spent += px;
            sent += 1;
        }
        if !self.pending.is_empty() {
            ctx.request_repaint();
        }
        sent
    }

    /// Draws the part `uv` (0..1 of the picture) into `dest`: each tile at its place, a tile not
    /// there yet in `placeholder`.
    pub fn paint(&self, painter: &Painter, dest: Rect, uv: Rect, tint: Color32, placeholder: Color32) {
        let [w, h] = self.size;
        if w == 0 || h == 0 || uv.width() <= 0.0 || uv.height() <= 0.0 {
            return;
        }
        let scale = Vec2::new(dest.width() / uv.width(), dest.height() / uv.height());
        let to_dest =
            |p: Pos2| pos2(dest.min.x + (p.x - uv.min.x) * scale.x, dest.min.y + (p.y - uv.min.y) * scale.y);
        for (tile, handle) in self.tiles.iter().zip(&self.handles) {
            let t = Rect::from_min_size(
                pos2(tile.x as f32 / w as f32, tile.y as f32 / h as f32),
                Vec2::new(tile.w as f32 / w as f32, tile.h as f32 / h as f32),
            );
            let part = t.intersect(uv);
            if part.width() <= 0.0 || part.height() <= 0.0 {
                continue;
            }
            let on_screen = Rect::from_min_max(to_dest(part.min), to_dest(part.max));
            match handle {
                Some(h) => {
                    let local = Rect::from_min_max(
                        pos2((part.min.x - t.min.x) / t.width(), (part.min.y - t.min.y) / t.height()),
                        pos2((part.max.x - t.min.x) / t.width(), (part.max.y - t.min.y) / t.height()),
                    );
                    painter.image(h.id(), on_screen, local, tint);
                }
                None => {
                    painter.rect_filled(on_screen, 0.0, placeholder);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAME_BUDGET, Tiled};
    use egui::{Color32, ColorImage, TextureOptions};

    #[test]
    fn a_big_picture_goes_up_a_few_tiles_a_frame() {
        let ctx = egui::Context::default();
        let image = ColorImage::new([2048, 1300], vec![Color32::RED; 2048 * 1300]);
        let mut t = Tiled::new("sheet", &image, TextureOptions::LINEAR);
        assert_eq!(t.size(), [2048, 1300]);
        assert!(!t.ready());
        let mut frames = 0;
        while !t.ready() {
            let sent = t.pump(&ctx, FRAME_BUDGET);
            assert!(sent >= 1);
            frames += 1;
            assert!(frames < 20);
        }
        // 4 x 3 tiles, four of 512 x 512 a frame
        assert_eq!(frames, 3);
        assert!((t.progress() - 1.0).abs() < 1e-6);
    }
}

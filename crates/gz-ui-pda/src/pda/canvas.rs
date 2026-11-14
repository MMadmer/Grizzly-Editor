//! The preview's canvas (`DrawCanvas`): the 2D PDA - the device's art from the linked game's
//! `pda.xml` / `pda_16.xml`, its screen with the layout's flat item list drawn in canvas units
//! scaled per axis, the screen noise, the close button - and the mouse on it: the left button
//! plays the layout, the right or middle one drags the device, the wheel scrolls what the game
//! would scroll or zooms at the mouse (Ctrl+wheel always zooms).
//!
//! Everything is drawn with the game's own textures and bitmap fonts ([`crate::assets`]); what
//! cannot run outside the game is shown instead - a stock section as a labelled panel, the map
//! as the level map of `preview.level` with `preview.actor` and `preview.markers`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Color32, CornerRadius, CursorIcon, Event, FontId, MouseWheelUnit, Painter, PointerButton, Pos2, Rect,
    Sense, Stroke, StrokeKind, Ui, Vec2, pos2,
};
use gz_editor::Editor;
use gz_pda::{Clock, Hit, ItemBox, Layout, MapSpot, Preview};
use gz_ui::{Font, Palette, palette};

use crate::assets::{Look, PdaAssets, SheetFrom};
use crate::bitmap::layout_text;
use crate::gamefile::{Device, FRect, LevelMap};
use crate::pda::{Geom, Model};

/// What the game itself draws, as the layout's own colours are given (ARGB): a texture it cannot
/// find, the player on the map.
const GAME_MISSING_TEXTURE: u32 = 0x5AFF_00FF;
const GAME_PLAYER_RING: u32 = 0xFFFF_FFFF;
const GAME_PLAYER_DOT: u32 = 0xFF28_A0FF;

/// The zoom's range: the device's pixels per game pixel.
pub const ZOOM_MIN: f32 = 0.25;
pub const ZOOM_MAX: f32 = 4.0;
/// pt: what Fit keeps round the device, so its frame never meets the toolbar or a pane.
const FIT_MARGIN: f32 = 16.0;
/// How long a toast of a script action stays.
const TOAST: Duration = Duration::from_millis(2500);

/// What the canvas keeps between frames that is not the preview's state.
#[derive(Default)]
pub struct CanvasCache {
    /// The layout the children lists were made for.
    layout: usize,
    kids: Vec<Vec<usize>>,
    roots: Vec<usize>,
    toast_seen: u64,
    toast_until: Option<Instant>,
}

impl CanvasCache {
    fn tree(&mut self, layout: &Arc<Layout>) {
        let key = Arc::as_ptr(layout) as usize;
        if key == self.layout && self.kids.len() == layout.items.len() {
            return;
        }
        self.layout = key;
        let n = layout.items.len();
        self.kids = vec![Vec::new(); n];
        self.roots.clear();
        for (i, it) in layout.items.iter().enumerate() {
            match it.parent.checked_sub(1).and_then(|p| self.kids.get_mut(p)) {
                Some(list) if it.parent - 1 < i => list.push(i),
                _ => self.roots.push(i),
            }
        }
    }
}

/// An ARGB colour (`Col`): the alpha times `mul`.
#[must_use]
pub fn col(argb: u32, mul: f32) -> Color32 {
    let a = ((((argb >> 24) & 0xFF) as f32) * mul) as u32;
    Color32::from_rgba_unmultiplied((argb >> 16) as u8, (argb >> 8) as u8, argb as u8, a.min(255) as u8)
}

fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

fn floor_px(v: f32, ppp: f32) -> f32 {
    (v * ppp).floor() / ppp
}

/// The local time `{time}` and `{date}` show.
#[must_use]
pub fn clock_now() -> Clock {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Clock {
        hour: u32::from(now.hour()),
        minute: u32::from(now.minute()),
        day: u32::from(now.day()),
        month: u32::from(u8::from(now.month())),
    }
}

// ---- where the device goes ------------------------------------------------------------------

/// The game's screen in points (`DeviceSize`): `zoom` x its 768 lines in pixels, or - at 0 -
/// as big as lets the device fit `avail` on both axes with the margin round it.
#[must_use]
pub fn device_size(dev: &Device, wide: bool, avail: Vec2, zoom: f32, ppp: f32) -> Vec2 {
    let aspect = if wide { 16.0 / 9.0 } else { 4.0 / 3.0 };
    let mut h = 768.0 * zoom / ppp;
    if zoom <= 0.0 {
        let r = dev.rect(wide);
        let margin = 2.0 * FIT_MARGIN;
        // the device's points per point of screen height, across and down
        let per_x = r.width() / 1024.0 * aspect;
        let per_y = r.height() / 768.0;
        h = ((avail.x - margin).max(1.0) / per_x).min((avail.y - margin).max(1.0) / per_y);
        h = h.clamp(1.0 / ppp, 768.0 * ZOOM_MAX / ppp);
    }
    Vec2::new(h * aspect, h)
}

/// Where the screen's corner goes for the device to sit in the middle of the canvas.
fn centred(dev: &Device, wide: bool, p0: Pos2, avail: Vec2, size: Vec2) -> Pos2 {
    let r = dev.rect(wide);
    pos2(
        p0.x + avail.x * 0.5 - (r.x1 + r.x2) * 0.5 * size.x / 1024.0,
        p0.y + avail.y * 0.5 - (r.y1 + r.y2) * 0.5 * size.y / 768.0,
    )
}

/// A zoomed device can be dragged around, but some of it always stays in view.
fn clamp_pan(m: &mut Model, dev: &Device, wide: bool, avail: Vec2, size: Vec2) {
    let r = dev.rect(wide);
    let device = Vec2::new(r.width() * size.x / 1024.0, r.height() * size.y / 768.0);
    let keep = 64.0;
    let mx = ((avail.x + device.x) * 0.5 - keep).max(0.0);
    let my = ((avail.y + device.y) * 0.5 - keep).max(0.0);
    m.pan.x = m.pan.x.clamp(-mx, mx);
    m.pan.y = m.pan.y.clamp(-my, my);
}

/// Places the device in `[p0, p0 + avail)` (`PlaceDevice`).
pub fn place_device(m: &mut Model, dev: &Device, p0: Pos2, avail: Vec2, ppp: f32) -> Geom {
    let wide = m.preview.aspect == gz_pda::Aspect::Wide;
    let size = device_size(dev, wide, avail, m.zoom, ppp);
    if m.zoom > 0.0 {
        clamp_pan(m, dev, wide, avail, size);
    } else {
        m.pan = Vec2::ZERO;
    }
    let c = centred(dev, wide, p0, avail, size);
    Geom {
        origin: pos2(floor_px(c.x + m.pan.x, ppp), floor_px(c.y + m.pan.y, ppp)),
        kx: size.x / 1024.0,
        ky: size.y / 768.0,
        screen: dev.side(wide).noise,
    }
}

/// The wheel zooms about the point under the mouse: that point of the device stays where it is.
// every argument is one the caller decides at each call; a struct of them would only rename them
#[allow(clippy::too_many_arguments)]
fn zoom_at(
    m: &mut Model,
    dev: &Device,
    g: &Geom,
    p0: Pos2,
    avail: Vec2,
    at: Pos2,
    factor: f32,
    ppp: f32,
) -> Geom {
    let wide = m.preview.aspect == gz_pda::Aspect::Wide;
    let next = (g.ky * ppp * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    let ux = (at.x - g.origin.x) / g.kx;
    let uy = (at.y - g.origin.y) / g.ky;
    let size = device_size(dev, wide, avail, next, ppp);
    let c = centred(dev, wide, p0, avail, size);
    m.zoom = next;
    m.pan = Vec2::new(at.x - ux * size.x / 1024.0 - c.x, at.y - uy * size.y / 768.0 - c.y);
    place_device(m, dev, p0, avail, ppp)
}

/// Every item's rectangle in points and whether it shows (`ComputeLayout`).
#[must_use]
pub fn item_rects(preview: &Preview, g: &Geom) -> (Vec<ItemBox>, Vec<(Rect, bool)>) {
    let boxes = preview.boxes();
    let l = &preview.layout;
    let (root, cx, cy) = (g.root(), g.cx(l.canvas_w), g.cy(l.canvas_h));
    let rects = boxes
        .iter()
        .map(|b| {
            (
                Rect::from_min_max(
                    pos2(root.x + b.x0 * cx, root.y + b.y0 * cy),
                    pos2(root.x + b.x1 * cx, root.y + b.y1 * cy),
                ),
                b.shown,
            )
        })
        .collect();
    (boxes, rects)
}

/// A point in points as canvas units.
#[must_use]
pub fn to_canvas(g: &Geom, layout: &Layout, p: Pos2) -> (f32, f32) {
    let root = g.root();
    ((p.x - root.x) / g.cx(layout.canvas_w), (p.y - root.y) / g.cy(layout.canvas_h))
}

/// Where the preview map puts a world point inside a map slot: the level fits the slot at zoom
/// 1 like the game's OptimalFit, world x right, world z up.
#[derive(Clone, Copy, Debug)]
pub struct MapView {
    pub mid: Pos2,
    pub fit: f32,
    pub center: (f32, f32),
}

impl MapView {
    #[must_use]
    pub fn w2p(&self, x: f32, z: f32) -> Pos2 {
        pos2(self.mid.x + (x - self.center.0) * self.fit, self.mid.y - (z - self.center.1) * self.fit)
    }
}

/// The view of a map slot (`MapView`); centres the map on the player (the whole level when
/// zoomed all the way out) the first time it is asked.
pub fn map_view(m: &mut Model, map: &LevelMap, slot: Rect) -> Option<MapView> {
    let [x1, z1, x2, z2] = map.bound;
    let (bw, bh) = (x2 - x1, z2 - z1);
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let fit = (slot.width() / bw).min(slot.height() / bh) * m.preview.map_zoom;
    if !m.preview.map_center_set {
        let middle = ((x1 + x2) * 0.5, (z1 + z2) * 0.5);
        m.map_center = m.preview.layout.actor.unwrap_or(middle);
        if m.preview.map_zoom <= 1.0 {
            m.map_center = middle;
        }
        m.preview.map_center_set = true;
    }
    Some(MapView { mid: slot.center(), fit, center: m.map_center })
}

// ---- drawing --------------------------------------------------------------------------------

struct Draw<'a> {
    painter: Painter,
    assets: &'a Arc<PdaAssets>,
    editor: &'a Arc<Editor>,
    p: Arc<Palette>,
    g: Geom,
    cx: f32,
    cy: f32,
    ppp: f32,
    now: Clock,
    pointer: Option<Pos2>,
    hover_hint: String,
}

impl Draw<'_> {
    fn clip(&self, r: Rect) -> Painter {
        self.painter.with_clip_rect(r)
    }

    fn tex(&self, painter: &Painter, id: &str, r: Rect, tint: u32, rounding: f32) {
        match self.assets.tex(self.editor, id) {
            Look::Ready((sheet, uv)) => {
                // the sheet comes in tiles: a piece is drawn tile by tile, its corners square
                let _ = rounding;
                sheet.paint(painter, r, uv, col(tint, 1.0));
            }
            // a texture the game would not find: the magenta X-Ray shows for one
            Look::Missing => {
                painter.rect_filled(r, CornerRadius::ZERO, col(GAME_MISSING_TEXTURE, 1.0));
            }
            Look::Loading => {}
        }
    }

    /// An editor's note inside the device - not game text: the editor's font, centred, wrapped.
    fn note(&self, painter: &Painter, r: Rect, text: &str) {
        let font = Font::Body.id();
        let wrap = (r.width() - 24.0).max(Font::Body.line());
        let galley = painter.layout(text.to_owned(), font, self.p.text_secondary, wrap);
        let size = galley.size();
        let at = pos2(
            floor_px((r.min.x + r.max.x - size.x) * 0.5, self.ppp),
            floor_px((r.min.y + r.max.y - size.y) * 0.5, self.ppp),
        );
        painter.galley(at, galley, self.p.text_secondary);
    }

    /// A label in the game's bitmap font (`DrawLabel`): a text exactly as wide as in the game.
    #[allow(clippy::too_many_arguments)]
    fn label(
        &self,
        painter: &Painter,
        utf8: &str,
        font: &str,
        color: u32,
        align: char,
        valign: char,
        wrap: bool,
        r: Rect,
    ) {
        if utf8.is_empty() || (color >> 24) == 0 {
            return;
        }
        // glyph pixel -> point (768 lines = the 1024 font set)
        let px = self.g.ky;
        let tint = col(color, 1.0);
        let f = match self.assets.font(self.editor, font) {
            Look::Loading => return,
            Look::Ready(f) => f,
            Look::Missing => {
                // no game fonts (no linked game): the editor's own font, sized alike
                let galley = painter.layout_no_wrap(
                    utf8.to_owned(),
                    FontId::new(12.0 * px, Font::Body.family()),
                    tint,
                );
                let ts = galley.size();
                let x = match align {
                    'c' => (r.min.x + r.max.x - ts.x) * 0.5,
                    'r' => r.max.x - ts.x,
                    _ => r.min.x,
                };
                let y = match valign {
                    't' => r.min.y,
                    'b' => r.max.y - ts.y,
                    _ => (r.min.y + r.max.y - ts.y) * 0.5,
                };
                painter.galley(pos2(x, y), galley, tint);
                return;
            }
        };
        let (font, sheet) = f;
        let (cp, _) = gz_base::text::utf8_to_cp1251(utf8);
        let max_w = r.width() / px;
        let lines = layout_text(&font.glyphs, &cp, max_w, wrap);
        let gh = font.glyphs.height;
        let lh = gh * px;
        let total = lh * lines.len() as f32;
        let mut y = match valign {
            't' => r.min.y,
            'b' => r.max.y - total,
            _ => (r.min.y + r.max.y - total) * 0.5,
        };
        for ln in &lines {
            let mut x = match align {
                'c' => (r.min.x + r.max.x - ln.w * px) * 0.5,
                'r' => r.max.x - ln.w * px,
                _ => r.min.x,
            };
            for &c in &ln.cp {
                let i = usize::from(c);
                let gw = font.glyphs.w.get(i).copied().unwrap_or(0.0);
                if gw > 0.0 && c != b' ' {
                    let gx = font.glyphs.x.get(i).copied().unwrap_or(0.0);
                    let gy = font.glyphs.y.get(i).copied().unwrap_or(0.0);
                    sheet.paint(
                        painter,
                        Rect::from_min_max(pos2(x, y), pos2(x + gw * px, y + gh * px)),
                        sheet.uv(gx, gy, gw, gh),
                        tint,
                    );
                }
                x += gw * px;
            }
            y += lh;
        }
    }

    /// A spot of the layout's `map_spots` centred on `c`, as the game draws it.
    fn spot(&self, painter: &Painter, sp: &MapSpot, c: Pos2) {
        let k = self.g.ky;
        let h = sp.size * 0.5 * k;
        let r = Rect::from_min_max(pos2(c.x - h, c.y - h), pos2(c.x + h, c.y + h));
        match sp.rect {
            None => self.tex(painter, &sp.icon, r, sp.color, 0.0),
            Some([x, y, w, hh]) => match self.assets.sheet(self.editor, &sp.icon, SheetFrom::Project) {
                Look::Ready(s) => s.paint(painter, r, s.uv(x, y, w, hh), col(sp.color, 1.0)),
                Look::Missing => {
                    painter.rect_filled(r, CornerRadius::ZERO, col(GAME_MISSING_TEXTURE, 1.0));
                }
                Look::Loading => {}
            },
        }
        if let Some(inner) = &sp.inner {
            let a = pos2(r.min.x + inner.x * k, r.min.y + inner.y * k);
            self.tex(
                painter,
                &inner.icon,
                Rect::from_min_size(a, Vec2::splat(inner.size * k)),
                inner.color,
                0.0,
            );
        }
    }

    fn map(&mut self, m: &mut Model, slot: Rect) {
        let painter = self.clip(slot);
        let level = m.preview.layout.preview_level.clone();
        let map = self.assets.level_map(self.editor, &level);
        let sheet = match &map {
            Look::Ready(lm) => self.assets.sheet(self.editor, &lm.tex, SheetFrom::Game),
            Look::Loading => Look::Loading,
            Look::Missing => Look::Missing,
        };
        if let (Look::Ready(lm), Look::Ready(sheet)) = (&map, &sheet)
            && let Some(v) = map_view(m, lm, slot)
        {
            let [x1, z1, x2, z2] = lm.bound;
            let r = Rect::from_two_pos(v.w2p(x1, z2), v.w2p(x2, z1));
            sheet.paint(&painter, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let l = m.preview.layout.clone();
            let s = 11.0 * self.g.ky;
            for mk in &l.markers {
                let at = v.w2p(mk.x, mk.z);
                let r = Rect::from_min_max(pos2(at.x - s, at.y - 2.0 * s), pos2(at.x + s, at.y));
                // a marker naming a map_spots node looks like that spot in the game: centred on its place
                match (!mk.spot.is_empty()).then(|| l.find_spot(&mk.spot)).flatten() {
                    Some(sp) => self.spot(&painter, sp, at),
                    None => self.tex(&painter, &mk.icon, r, mk.color, 0.0),
                }
                if !mk.hint.is_empty() && self.pointer.is_some_and(|p| r.contains(p) && slot.contains(p)) {
                    self.hover_hint.clone_from(&mk.hint);
                }
            }
            let at = l.actor.map_or(v.mid, |(x, z)| v.w2p(x, z));
            // the player as the layout's map_spots draws it in the game, the stock dot otherwise
            match l.find_spot("actor_level_spot") {
                Some(sp) => self.spot(&painter, sp, at),
                None => {
                    painter.circle_filled(at, 6.0 * self.g.ky, col(GAME_PLAYER_RING, 1.0));
                    painter.circle_filled(at, 4.0 * self.g.ky, col(GAME_PLAYER_DOT, 1.0));
                }
            }
            return;
        }
        // the editor's placeholder, not the game's look: a grid in the theme and a note on what is missing
        painter.rect_filled(slot, CornerRadius::ZERO, self.p.canvas);
        let cell = (40.0 * self.g.ky).max(4.0);
        let stroke = Stroke::new(1.0 / self.ppp, self.p.border_subtle);
        let mut x = slot.min.x;
        while x < slot.max.x {
            painter.line_segment([pos2(x, slot.min.y), pos2(x, slot.max.y)], stroke);
            x += cell;
        }
        let mut y = slot.min.y;
        while y < slot.max.y {
            painter.line_segment([pos2(slot.min.x, y), pos2(slot.max.x, y)], stroke);
            y += cell;
        }
        if matches!(map, Look::Loading) || matches!(sheet, Look::Loading) {
            return;
        }
        let text = if level.is_empty() {
            "The live map. Set preview.level to see a level here."
        } else {
            "The live map. The linked game has no map of this level."
        };
        self.note(&painter, slot, text);
    }

    fn back_color(m: &Model, i: usize) -> u32 {
        let Some(it) = m.preview.layout.items.get(i) else { return 0 };
        let clickable = Preview::clickable(it);
        if clickable && m.pressed == Some(i) && m.hovered == Some(i) && it.has_press {
            return it.press;
        }
        if m.preview.selected(i) && it.has_selected {
            return it.selected;
        }
        if clickable && m.hovered == Some(i) && it.has_hover {
            return it.hover;
        }
        it.color
    }

    /// One item and what it holds (`DrawItem`).
    fn item(
        &mut self,
        painter: &Painter,
        m: &mut Model,
        rects: &[(Rect, bool)],
        kids: &[Vec<usize>],
        i: usize,
    ) {
        let Some(&(a, shown)) = rects.get(i) else { return };
        if !shown {
            return;
        }
        let layout = m.preview.layout.clone();
        let Some(it) = layout.items.get(i) else { return };
        let (cx, cy) = (self.cx, self.cy);
        let rounding = it.radius * cy;
        let shaped = it.kind != "text" && it.kind != "image";
        let mut b = a;
        if it.kind == "row" {
            b.max.y = a.min.y + it.h * cy;
        }
        if shaped {
            let c = Self::back_color(m, i);
            if (c >> 24) > 0 {
                painter.rect_filled(b, radius(rounding), col(c, 1.0));
            }
        }
        match it.kind.as_str() {
            "map" => self.map(m, a),
            "image" => self.tex(painter, &it.texture, a, it.tint, rounding),
            "text" => {
                let text = m.preview.format(&it.text, &self.now);
                self.label(painter, &text, &it.font, it.text_color, it.align, it.valign, it.wrap, a);
            }
            "button" => {
                // the same placement CUIPdaItem::InitParts makes, in canvas units (docs/pda/PDA_FORMAT.md)
                let (w, h) = (it.w, it.h);
                let stacked = !it.icon.is_empty() && it.has_text && it.align == 'c' && it.valign != 'c';
                let sel = m.preview.selected(i);
                let hov = Preview::clickable(it) && m.hovered == Some(i);
                let mut text_left = 0.0;
                if !it.icon.is_empty() {
                    let icon = it.icon_size.min(w.min(h));
                    let alone = !it.has_text;
                    let x = if alone || stacked { (w - icon) * 0.5 } else { 12.0 };
                    let y = if stacked { h * 0.5 - icon + 2.0 } else { (h - icon) * 0.5 };
                    let ic = if sel && it.has_icon_selected { it.icon_selected } else { it.icon_color };
                    let r = Rect::from_min_max(
                        pos2(a.min.x + x * cx, a.min.y + y * cy),
                        pos2(a.min.x + (x + icon) * cx, a.min.y + (y + icon) * cy),
                    );
                    self.tex(painter, &it.icon, r, ic, 0.0);
                    if !alone && !stacked {
                        text_left = x + icon + 8.0;
                    }
                }
                if it.has_text {
                    let pad = if it.align != 'c' { 12.0 } else { 0.0 };
                    let l = f32::max(pad, text_left);
                    let t = if stacked { h * 0.5 + 2.0 } else { 0.0 };
                    let mut tc = it.text_color;
                    if sel && it.has_text_selected {
                        tc = it.text_selected;
                    } else if hov && it.has_text_hover {
                        tc = it.text_hover;
                    }
                    let text = m.preview.format(&it.text, &self.now);
                    let r = Rect::from_min_max(
                        pos2(a.min.x + l * cx, a.min.y + t * cy),
                        pos2(a.min.x + (w - pad) * cx, a.min.y + h * cy),
                    );
                    self.label(
                        painter,
                        &text,
                        &it.font,
                        tc,
                        it.align,
                        if stacked { 't' } else { it.valign },
                        it.wrap,
                        r,
                    );
                }
            }
            _ => {}
        }
        let children = kids.get(i).cloned().unwrap_or_default();
        // children: a list clips its rows, and shows its empty line while it has none
        if it.kind == "list" {
            let inner = painter.with_clip_rect(a);
            for &k in &children {
                self.item(&inner, m, rects, kids, k);
            }
            if children.is_empty() && !it.empty.is_empty() {
                let r = Rect::from_min_max(a.min, pos2(a.max.x, a.min.y + it.h.min(60.0) * cy));
                self.label(painter, &it.empty, &it.font, it.text_color, 'c', 'c', false, r);
            }
        } else {
            for &k in &children {
                self.item(painter, m, rects, kids, k);
            }
        }
        if shaped && it.has_border && (it.border >> 24) > 0 {
            let w = (it.border_w * cy).max(1.0 / self.ppp);
            painter.rect_stroke(b, radius(rounding), Stroke::new(w, col(it.border, 1.0)), StrokeKind::Inside);
        }
    }

    fn stock_panel(&self, painter: &Painter, r: Rect, stock: &str) {
        painter.rect_filled(r, CornerRadius::ZERO, self.p.raised);
        const NAMES: [(&str, &str); 9] = [
            ("eptTasks", "Tasks and map"),
            ("eptRanking", "Ranking"),
            ("eptRelations", "Relations"),
            ("eptContacts", "Contacts"),
            ("eptLogs", "Logs"),
            ("eptEncyclopedia", "Encyclopedia"),
            ("eptMap", "Map"),
            ("eptStatistics", "Statistics"),
            ("eptFractionWar", "Faction war"),
        ];
        let title = NAMES.iter().find(|(k, _)| *k == stock).map_or(stock, |(_, t)| t);
        let note = format!(
            "Stock PDA section: {title} ({stock})\nThe game shows its own screen here; the overlay stays on top."
        );
        self.note(painter, r, &note);
    }

    /// The device's art runs on past the game's screen; each side of the device that lies on the
    /// screen's edge gets that edge drawn along the device and no further - a line beyond it
    /// would read as a glitch. True when the pointer is on one of the lines.
    fn screen_edge(&self, dr: FRect, edges: u8) -> bool {
        let (ext, reach) = (0.0, 4.0);
        let stroke = Stroke::new(1.0, self.p.border_strong);
        let (a, b) = (self.g.at(dr.x1, dr.y1), self.g.at(dr.x2, dr.y2));
        let m = self.pointer;
        let half = 0.5 / self.ppp;
        let mut hover = false;
        let mut across = |y: f32| {
            self.painter.line_segment([pos2(a.x - ext, y), pos2(b.x + ext, y)], stroke);
            hover |= m.is_some_and(|m| (m.y - y).abs() <= reach && m.x >= a.x - ext && m.x <= b.x + ext);
        };
        if edges & 1 != 0 {
            across(floor_px(a.y, self.ppp) + half);
        }
        if edges & 2 != 0 {
            across(floor_px(b.y, self.ppp) - half);
        }
        let mut down = |x: f32| {
            self.painter.line_segment([pos2(x, a.y - ext), pos2(x, b.y + ext)], stroke);
            hover |= m.is_some_and(|m| (m.x - x).abs() <= reach && m.y >= a.y - ext && m.y <= b.y + ext);
        };
        if edges & 4 != 0 {
            down(floor_px(a.x, self.ppp) + half);
        }
        if edges & 8 != 0 {
            down(floor_px(b.x, self.ppp) - half);
        }
        hover
    }
}

/// The wheel of this frame in notches and whether Ctrl was held.
fn wheel(ui: &Ui) -> (f32, bool) {
    ui.input(|i| {
        let mut notches = 0.0;
        let mut ctrl = i.modifiers.ctrl;
        for e in &i.raw.events {
            if let Event::MouseWheel { unit, delta, modifiers, .. } = e {
                notches += match unit {
                    MouseWheelUnit::Line => delta.y,
                    MouseWheelUnit::Point => delta.y / 50.0,
                    MouseWheelUnit::Page => delta.y * 3.0,
                };
                ctrl |= modifiers.ctrl;
            }
        }
        (notches, ctrl)
    })
}

/// Draws the device and the layout into `rect` and handles the mouse there.
pub fn draw_canvas(
    ui: &mut Ui,
    rect: Rect,
    m: &mut Model,
    dev: &Device,
    assets: &Arc<PdaAssets>,
    editor: &Arc<Editor>,
    cache: &mut CanvasCache,
) {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let (p0, avail) = (rect.min, rect.size());
    let wide = m.preview.aspect == gz_pda::Aspect::Wide;
    let side = dev.side(wide).clone();
    let resp = ui.interact(rect, ui.make_persistent_id("gz-ui-pda.canvas"), Sense::click_and_drag());
    let over = resp.contains_pointer();
    let pointer = ctx.pointer_latest_pos();
    let dragging = resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle);
    let mut g = place_device(m, dev, p0, avail, ppp);
    if dragging {
        // a fitted device starts panning from where it is
        if m.zoom <= 0.0 {
            m.zoom = g.ky * ppp;
        }
        m.pan += resp.drag_delta();
        g = place_device(m, dev, p0, avail, ppp);
    }
    let (mut boxes, mut rects) = item_rects(&m.preview, &g);
    let (notches, ctrl) = wheel(ui);
    if over && notches != 0.0 {
        let mut taken = false;
        if !ctrl
            && !m.preview.closed
            && let Some(at) = pointer
        {
            let (u, v) = to_canvas(&g, &m.preview.layout, at);
            taken = m.preview.wheel(&boxes, u, v, notches);
        }
        if !taken && let Some(at) = pointer {
            g = zoom_at(m, dev, &g, p0, avail, at, if notches > 0.0 { 1.15 } else { 1.0 / 1.15 }, ppp);
        }
        (boxes, rects) = item_rects(&m.preview, &g);
    }

    let layout = m.preview.layout.clone();
    cache.tree(&layout);
    let dr = dev.rect(wide);
    let (da, db) = (g.at(dr.x1, dr.y1), g.at(dr.x2, dr.y2));
    m.device_gap = (da.x - p0.x).min(da.y - p0.y).min(p0.x + avail.x - db.x).min(p0.y + avail.y - db.y);
    m.screen_edges = u8::from(dr.y1 <= 0.0)
        | (u8::from(dr.y2 >= 768.0) << 1)
        | (u8::from(dr.x1 <= 0.0) << 2)
        | (u8::from(dr.x2 >= 1024.0) << 3);
    m.geom = Some(g);
    m.ppp = ppp;

    let mut d = Draw {
        painter: ui.painter_at(rect),
        assets,
        editor,
        p: palette(&ctx),
        g,
        cx: g.cx(layout.canvas_w),
        cy: g.cy(layout.canvas_h),
        ppp,
        now: clock_now(),
        pointer: pointer.filter(|_| over),
        hover_hint: String::new(),
    };
    // the backdrop covers the device only: on 16:9 the rest of the game's screen would be cut
    d.painter.rect_filled(Rect::from_min_max(da, db), CornerRadius::ZERO, d.p.dim);
    let bg = Rect::from_min_max(g.at(side.bg.x1, side.bg.y1), g.at(side.bg.x2, side.bg.y2));
    d.tex(&d.painter.clone(), &side.bg_tex, bg, 0xFFFF_FFFF, 0.0);
    let on_edge = d.screen_edge(dr, m.screen_edges);

    let screen = Rect::from_min_max(g.at(g.screen.x1, g.screen.y1), g.at(g.screen.x2, g.screen.y2));
    let sp = d.clip(screen);
    if m.preview.closed {
        sp.rect_filled(screen, CornerRadius::ZERO, d.p.dim);
    } else {
        let roots = cache.roots.clone();
        for &r in &roots {
            if layout.items.get(r).is_some_and(|it| it.layer != "overlay") {
                d.item(&sp, m, &rects, &cache.kids, r);
            }
        }
        if !m.preview.stock.is_empty() {
            let stock = m.preview.stock.clone();
            d.stock_panel(&sp, screen, &stock);
        }
        for &r in &roots {
            if layout.items.get(r).is_some_and(|it| it.layer == "overlay") {
                d.item(&sp, m, &rects, &cache.kids, r);
            }
        }
    }
    let base = d.painter.clone();
    if layout.noise {
        d.tex(&base, &side.noise_tex, screen, (side.noise_alpha << 24) | 0x00FF_FFFF, 0.0);
    }
    if layout.close {
        let close =
            Rect::from_min_max(g.at(side.close.x1, side.close.y1), g.at(side.close.x2, side.close.y2));
        d.tex(&base, &side.close_tex, close, 0xFFFF_FFFF, 0.0);
    }
    if m.preview.closed {
        d.note(&base, screen, "The PDA is closed. Click to open it again.");
    }

    // the left button plays the layout
    let now = d.now;
    m.hovered = None;
    let (pressed_now, released_now, primary_down) =
        ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down()));
    if over && !m.preview.closed && !dragging {
        let hit = match pointer {
            Some(at) => {
                let (u, v) = to_canvas(&g, &layout, at);
                m.preview.hit_test(&boxes, u, v)
            }
            None => Hit::Nothing,
        };
        let item = if let Hit::Item(i) = hit { Some(i) } else { None };
        m.hovered = item;
        if d.hover_hint.is_empty()
            && let Some(i) = item
            && let Some(it) = layout.items.get(i)
            && !it.hint.is_empty()
        {
            d.hover_hint = m.preview.format(&it.hint, &now);
        }
        if d.hover_hint.is_empty() && item.is_none() && on_edge {
            "The game's screen ends here. The PDA runs on past it, so the game shows it cut like this."
                .clone_into(&mut d.hover_hint);
        }
        if pressed_now {
            m.pressed = item;
        }
        if released_now {
            if let (Some(p), Some(i)) = (m.pressed, item)
                && p == i
            {
                m.preview.click(i, &now);
            }
            m.pressed = None;
        }
        // dragging the map with the left button pans it, the way the game's map does
        if resp.dragged_by(PointerButton::Primary)
            && m.pressed.is_none()
            && let Some(at) = pointer
        {
            let delta = resp.drag_delta();
            for (i, &(r, shown)) in rects.iter().enumerate().rev() {
                if !shown || !r.contains(at) || layout.items.get(i).is_none_or(|it| it.kind != "map") {
                    continue;
                }
                if let Look::Ready(lm) = assets.level_map(editor, &layout.preview_level)
                    && m.preview.map_center_set
                {
                    let [x1, z1, x2, z2] = lm.bound;
                    let (bw, bh) = (x2 - x1, z2 - z1);
                    if bw > 0.0 && bh > 0.0 {
                        let fit = (r.width() / bw).min(r.height() / bh) * m.preview.map_zoom;
                        m.map_center.0 -= delta.x / fit;
                        m.map_center.1 += delta.y / fit;
                    }
                }
                break;
            }
        }
    } else if over && m.preview.closed && pressed_now {
        m.preview.closed = false;
    }
    if !over && !primary_down {
        m.pressed = None;
    }
    if dragging {
        ctx.set_cursor_icon(CursorIcon::Move);
    }
    if !d.hover_hint.is_empty() {
        resp.on_hover_text(d.hover_hint.clone());
    }

    // a script action or a signal leaves a note on the screen for a moment
    if m.preview.toast_serial != cache.toast_seen {
        cache.toast_seen = m.preview.toast_serial;
        cache.toast_until = (!m.preview.toast.is_empty()).then(|| Instant::now() + TOAST);
    }
    if let Some(until) = cache.toast_until {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            cache.toast_until = None;
        } else {
            let font = Font::Body.id();
            let wrap = (screen.width() - 48.0).max(Font::Body.line());
            let galley = base.layout(m.preview.toast.clone(), font, d.p.text, wrap);
            let ts = galley.size();
            let c = pos2(screen.center().x, screen.max.y - 40.0 * g.ky);
            let pad = Vec2::new(12.0, 7.0);
            let back = Rect::from_min_max(
                pos2(c.x - ts.x * 0.5 - pad.x, c.y - ts.y * 0.5 - pad.y),
                pos2(c.x + ts.x * 0.5 + pad.x, c.y + ts.y * 0.5 + pad.y),
            );
            base.rect_filled(back, CornerRadius::same(6), Palette::alpha(d.p.overlay, 0.92));
            base.galley(
                pos2(floor_px(c.x - ts.x * 0.5, ppp), floor_px(c.y - ts.y * 0.5, ppp)),
                galley,
                d.p.text,
            );
            ctx.request_repaint_after(left);
        }
    }
}

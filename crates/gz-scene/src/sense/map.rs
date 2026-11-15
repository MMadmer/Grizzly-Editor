//! `shot.map`: the scene from above as a schematic, drawn on the processor.
//!
//! One ray a pixel falls from above everything; what it hits first gives the pixel its object,
//! its height and its slope. From that a picture is drawn that an agent can read back into
//! numbers: every object in a colour of its own with its name on it, the ground shaded by its
//! relief, a grid in metres with its coordinates at the edges. +X is right, +Z is up. The same
//! scene gives the same picture, on any machine, with no window and no graphics card.
//!
//! This file is the drawing alone: it takes the samples and what the scene holds as plain data
//! ([`MapData`]) and knows nothing of rays, so it is tested without a scene.

use gz_pix::{Canvas, Rgba};

/// The longer side of the map itself, px, at most.
pub const MAX_PX: u32 = 2048;
/// The names drawn into one map at most; the legend names every object anyway.
const MAX_LABELS: usize = 96;
/// Room for the coordinates left of the map and above it, px.
pub const MARGIN_LEFT: u32 = 46;
pub const MARGIN_TOP: u32 = 14;

/// What a map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMode {
    /// Every object in a colour of its own over the shaded ground.
    Objects,
    /// The heights as colours with contour lines and spot heights.
    Height,
    /// The AI map's nodes over the ground.
    Ai,
}

/// One pixel's ray: what it hit first from above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The index of the object in [`MapData::objects`]; `u32::MAX` when the ray hit nothing.
    pub object: u32,
    /// The height of the hit, metres.
    pub height: f32,
    /// How much of the hit surface faces up: 1 flat, 0 a wall.
    pub up: f32,
    /// How much the hit surface leans towards -X and +Z (the light's side): -1 to 1.
    pub lit: f32,
}

impl Sample {
    pub const NONE: Self = Self { object: u32::MAX, height: 0.0, up: 1.0, lit: 0.0 };

    /// A sample of a hit with the surface's normal.
    #[must_use]
    pub fn hit(object: u32, height: f32, normal: [f32; 3]) -> Self {
        // the light comes from the upper left of the picture, as on a printed map
        const L: [f32; 3] = [-0.5, 0.7, 0.5];
        let len = (L[0] * L[0] + L[1] * L[1] + L[2] * L[2]).sqrt();
        let lit = (normal[0] * L[0] + normal[1] * L[1] + normal[2] * L[2]) / len;
        Self { object, height, up: normal[1].clamp(-1.0, 1.0), lit: lit.clamp(-1.0, 1.0) }
    }
}

/// An object rays can hit.
#[derive(Clone, Debug, PartialEq)]
pub struct MapObject {
    pub name: String,
    /// What it is, as the commands name kinds (`object`, `terrain`, ...).
    pub kind: &'static str,
    /// Drawn in the accent colour, named before anything else.
    pub highlight: bool,
}

/// What a point of the scene is, for its sign on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Spawn,
    Light,
    Sound,
    Particles,
    WayPoint,
}

/// A thing without a body the rays could hit: a spawn point, a light, a way's point.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPoint {
    pub name: String,
    pub kind: PointKind,
    /// World x and z.
    pub at: [f32; 2],
    pub highlight: bool,
    /// Whether its name is written beside it: a way's points are many, the way is named once.
    pub named: bool,
}

/// A line over the map: a way's links, a shape's outline.
#[derive(Clone, Debug, PartialEq)]
pub struct MapLine {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
    /// A way (drawn in the ways' colour) or a volume (in the volumes').
    pub way: bool,
}

/// A place the caller wants to see marked.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMark {
    pub at: [f32; 2],
    pub text: String,
}

/// Everything a map is drawn from.
#[derive(Clone, Debug)]
pub struct MapData {
    /// The ground rectangle: x0, z0, x1, z1 with x0 < x1 and z0 < z1.
    pub area: [f32; 4],
    /// The map's size in px, without the margins.
    pub size: [u32; 2],
    /// `size[0] * size[1]` samples, rows from the top (z1) down, each from x0 to x1.
    pub samples: Vec<Sample>,
    pub objects: Vec<MapObject>,
    pub points: Vec<MapPoint>,
    pub lines: Vec<MapLine>,
    pub marks: Vec<MapMark>,
    /// The AI map's nodes (world x, z), for [`MapMode::Ai`].
    pub nodes: Vec<[f32; 2]>,
    /// The AI map's cell, metres.
    pub node_size: f32,
    pub mode: MapMode,
    /// Metres between grid lines; `None` picks by the scale.
    pub grid: Option<f32>,
    /// Whether names are drawn.
    pub labels: bool,
}

/// One row of the legend: an object the map shows.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendRow {
    pub object: String,
    pub kind: &'static str,
    /// The colour it is drawn in, `#rrggbb`.
    pub colour: String,
    /// The middle of what the map shows of it: world x and z.
    pub centre: [f32; 2],
    /// How many square metres of the map it covers, seen from above.
    pub area: f32,
    /// Whether its name is on the map.
    pub named: bool,
}

/// The picture and what it says in numbers.
pub struct Map {
    pub canvas: Canvas,
    pub px_per_m: f32,
    pub grid: f32,
    pub legend: Vec<LegendRow>,
    /// The lowest and the highest height the rays found, when any hit.
    pub heights: Option<(f32, f32)>,
    /// Metres between the contour lines of [`MapMode::Height`].
    pub contour: Option<f32>,
}

const VOID: Rgba = Rgba::rgb(30, 33, 40);
const MARGIN: Rgba = Rgba::rgb(22, 24, 29);
const GROUND: Rgba = Rgba::rgb(132, 138, 120);
const GRID: Rgba = Rgba::rgb(255, 255, 255).alpha(46);
const GRID_STRONG: Rgba = Rgba::rgb(255, 255, 255).alpha(92);
const INK: Rgba = Rgba::rgb(238, 238, 238);
const PLATE: Rgba = Rgba::rgb(0, 0, 0).alpha(176);
const ACCENT: Rgba = Rgba::rgb(255, 64, 160);
const MARK: Rgba = Rgba::rgb(255, 48, 48);
const WAY: Rgba = Rgba::rgb(64, 224, 255);
const VOLUME: Rgba = Rgba::rgb(210, 120, 255);
const NODE: Rgba = Rgba::rgb(70, 220, 110);

/// The grid steps a map may take, metres.
const STEPS: [f32; 10] = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0];

/// The grid step for a scale: the smallest whose lines are 56 px apart or more.
#[must_use]
pub fn grid_step(px_per_m: f32) -> f32 {
    STEPS.iter().copied().find(|s| s * px_per_m >= 56.0).unwrap_or(1000.0)
}

/// Metres between contour lines for a range of heights: about ten lines.
#[must_use]
pub fn contour_step(range: f32) -> f32 {
    const C: [f32; 9] = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0];
    C.iter().copied().find(|c| range / c <= 12.0).unwrap_or(100.0)
}

/// A colour of an object's own, from its name: the same name is the same colour in every map.
#[must_use]
pub fn colour_of(name: &str) -> Rgba {
    // FNV-1a
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let hue = (h % 360) as f32;
    let saturation = 0.45 + ((h >> 9) % 4) as f32 * 0.1;
    let value = 0.78 + ((h >> 13) % 3) as f32 * 0.08;
    Rgba::hsv(hue, saturation, value)
}

fn hex(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// A height as a colour: blue low, green, yellow, red, white high.
#[must_use]
pub fn ramp(t: f32) -> Rgba {
    const STOPS: [Rgba; 6] = [
        Rgba::rgb(38, 70, 150),
        Rgba::rgb(52, 150, 170),
        Rgba::rgb(86, 170, 86),
        Rgba::rgb(214, 200, 86),
        Rgba::rgb(200, 104, 60),
        Rgba::rgb(240, 240, 240),
    ];
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let f = t * (STOPS.len() - 1) as f32;
    let i = (f as usize).min(STOPS.len() - 2);
    STOPS[i].mix(STOPS[i + 1], f - i as f32)
}

fn number(v: f32, step: f32) -> String {
    if step >= 1.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

impl MapData {
    fn px_per_m(&self) -> f32 {
        self.size[0] as f32 / (self.area[2] - self.area[0]).max(1e-3)
    }

    /// Where a world point is in the picture (with the margins), px.
    fn to_px(&self, x: f32, z: f32) -> (f32, f32) {
        let k = self.px_per_m();
        (MARGIN_LEFT as f32 + (x - self.area[0]) * k, MARGIN_TOP as f32 + (self.area[3] - z) * k)
    }

    fn sample(&self, i: i64, j: i64) -> Option<&Sample> {
        (i >= 0 && j >= 0 && i < i64::from(self.size[0]) && j < i64::from(self.size[1]))
            .then(|| self.samples.get(j as usize * self.size[0] as usize + i as usize))
            .flatten()
    }
}

/// Draws the map.
#[must_use]
pub fn draw(d: &MapData) -> Map {
    let (w, h) = (d.size[0].clamp(1, MAX_PX), d.size[1].clamp(1, MAX_PX));
    let k = d.px_per_m();
    let mut canvas = Canvas::new(w + MARGIN_LEFT, h + MARGIN_TOP, MARGIN);

    // the heights the rays found
    let hits = d.samples.iter().filter(|s| s.object != u32::MAX);
    let heights = hits.fold(None, |acc: Option<(f32, f32)>, s| {
        Some(acc.map_or((s.height, s.height), |(lo, hi)| (lo.min(s.height), hi.max(s.height))))
    });
    let (lo, hi) = heights.unwrap_or((0.0, 1.0));
    let range = (hi - lo).max(1e-3);
    let contour = (d.mode == MapMode::Height && heights.is_some()).then(|| contour_step(range));

    // what each object covers: its pixels, their middle, and the rectangle around them
    let mut cover = vec![(0u64, 0f64, 0f64); d.objects.len()];
    let mut around = vec![(u32::MAX, u32::MAX, 0u32, 0u32); d.objects.len()];
    for j in 0..h {
        for i in 0..w {
            let Some(s) = d.sample(i64::from(i), i64::from(j)) else { continue };
            if let Some(c) = cover.get_mut(s.object as usize) {
                c.0 += 1;
                c.1 += f64::from(i) + 0.5;
                c.2 += f64::from(j) + 0.5;
                let r = &mut around[s.object as usize];
                *r = (r.0.min(i), r.1.min(j), r.2.max(i + 1), r.3.max(j + 1));
            }
        }
    }
    // the ground is what covers the map: an object over a third of it is drawn as relief, not
    // as a thing with a colour of its own
    let total = u64::from(w) * u64::from(h);
    let is_ground: Vec<bool> =
        d.objects.iter().zip(&cover).map(|(o, c)| o.kind == "terrain" || c.0 * 3 > total).collect();
    let colours: Vec<Rgba> = d
        .objects
        .iter()
        .zip(&is_ground)
        .map(|(o, g)| {
            if o.highlight {
                ACCENT
            } else if *g {
                GROUND
            } else {
                colour_of(&o.name)
            }
        })
        .collect();

    // the pixels
    for j in 0..h {
        for i in 0..w {
            let (ii, jj) = (i64::from(i), i64::from(j));
            let Some(s) = d.sample(ii, jj) else { continue };
            let (x, y) = (i64::from(i + MARGIN_LEFT), i64::from(j + MARGIN_TOP));
            if s.object == u32::MAX {
                canvas.plot(x, y, VOID);
                continue;
            }
            let t = (s.height - lo) / range;
            // light from the upper left; a wall seen from above is dark
            let shade = 0.62 + 0.38 * s.lit.max(0.0) - 0.25 * (1.0 - s.up.abs());
            let base = match d.mode {
                MapMode::Height => ramp(t),
                MapMode::Objects | MapMode::Ai => {
                    let c = colours.get(s.object as usize).copied().unwrap_or(GROUND);
                    // the ground lightens with its height, so a hill reads as one
                    if is_ground.get(s.object as usize).copied().unwrap_or(false) && c == GROUND {
                        c.shade(0.82 + 0.36 * t)
                    } else {
                        c
                    }
                }
            };
            let mut c = base.shade(shade.clamp(0.25, 1.15));
            // an outline where the object changes or the height jumps: every body has an edge
            let mut edge = false;
            for (di, dj) in [(1i64, 0i64), (0, 1)] {
                if let Some(n) = d.sample(ii + di, jj + dj) {
                    let other = n.object != s.object;
                    let step = n.object != u32::MAX && (n.height - s.height).abs() > 0.6_f32.max(1.5 / k);
                    edge |= other || step;
                }
            }
            if edge {
                c = c.shade(0.45);
            }
            if let Some(step) = contour {
                // a contour line where the height crosses a multiple of the step
                let band = (s.height / step).floor();
                let crosses = [(1i64, 0i64), (0, 1)].iter().any(|(di, dj)| {
                    d.sample(ii + di, jj + dj)
                        .is_some_and(|n| n.object != u32::MAX && (n.height / step).floor() != band)
                });
                if crosses {
                    c = c.shade(0.6);
                }
            }
            canvas.plot(x, y, c);
        }
    }

    // the AI map's nodes
    if d.mode == MapMode::Ai {
        let half = (d.node_size.max(0.1) * k * 0.5 - 0.5).max(0.5);
        for n in &d.nodes {
            let (x, y) = d.to_px(n[0], n[1]);
            if x < MARGIN_LEFT as f32 || y < MARGIN_TOP as f32 {
                continue;
            }
            canvas.fill_rect(
                (x - half).round() as i64,
                (y - half).round() as i64,
                (x + half).round().max((x - half).round() + 1.0) as i64,
                (y + half).round().max((y - half).round() + 1.0) as i64,
                NODE.alpha(170),
            );
        }
    }

    // the highlighted objects' outlines, over everything of the map itself
    for j in 0..h {
        for i in 0..w {
            let (ii, jj) = (i64::from(i), i64::from(j));
            let Some(s) = d.sample(ii, jj) else { continue };
            if !d.objects.get(s.object as usize).is_some_and(|o| o.highlight) {
                continue;
            }
            let rim = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(di, dj)| d.sample(ii + di, jj + dj).is_none_or(|n| n.object != s.object));
            if rim {
                let (x, y) = (i64::from(i + MARGIN_LEFT), i64::from(j + MARGIN_TOP));
                canvas.fill_rect(x - 1, y - 1, x + 1, y + 1, Rgba::WHITE);
            }
        }
    }

    // the grid, on world multiples of its step
    let step = d.grid.filter(|g| g.is_finite() && *g > 0.0 && g * k >= 4.0).unwrap_or_else(|| grid_step(k));
    let first = |from: f32| (from / step).ceil() as i64;
    let last = |to: f32| (to / step).floor() as i64;
    let (left, top) = (MARGIN_LEFT as f32, MARGIN_TOP as f32);
    let (right, bottom) = (left + w as f32, top + h as f32);
    for n in first(d.area[0])..=last(d.area[2]) {
        let x = n as f32 * step;
        let (px, _) = d.to_px(x, d.area[1]);
        let strong = n % 5 == 0;
        canvas.line((px, top), (px, bottom - 1.0), 1, if strong { GRID_STRONG } else { GRID });
    }
    for n in first(d.area[1])..=last(d.area[3]) {
        let z = n as f32 * step;
        let (_, py) = d.to_px(d.area[0], z);
        let strong = n % 5 == 0;
        canvas.line((left, py), (right - 1.0, py), 1, if strong { GRID_STRONG } else { GRID });
    }

    // lines: ways and volumes
    for l in &d.lines {
        let pts: Vec<(f32, f32)> = l.points.iter().map(|p| d.to_px(p[0], p[1])).collect();
        canvas.polyline(&pts, l.closed, 2, if l.way { WAY } else { VOLUME });
    }

    // names: marks first, then what is highlighted, then bodies by their size, then points
    struct Name {
        text: String,
        /// The middle of what it names, px of the picture.
        at: (f32, f32),
        /// The rectangle of what it names, px of the picture: the name stands beside it.
        body: (i64, i64, i64, i64),
        ink: Rgba,
        legend: Option<usize>,
    }
    let mut names: Vec<Name> = Vec::new();
    let mut legend: Vec<LegendRow> = Vec::new();
    let mut order: Vec<usize> = (0..d.objects.len()).filter(|i| cover[*i].0 > 0).collect();
    order.sort_by(|a, b| {
        let (oa, ob) = (&d.objects[*a], &d.objects[*b]);
        ob.highlight
            .cmp(&oa.highlight)
            .then(cover[*b].0.cmp(&cover[*a].0))
            .then_with(|| oa.name.cmp(&ob.name))
    });
    for i in order {
        let (n, sx, sy) = cover[i];
        let (cx, cy) = ((sx / n as f64) as f32, (sy / n as f64) as f32);
        let centre = [d.area[0] + cx / k, d.area[3] - cy / k];
        let o = &d.objects[i];
        legend.push(LegendRow {
            object: o.name.clone(),
            kind: o.kind,
            colour: hex(colours[i]),
            centre: [round2(centre[0]), round2(centre[1])],
            area: round2(n as f32 / (k * k)),
            named: false,
        });
        // the ground is not a thing to name; anything else is, where the map shows it
        if !is_ground[i] || o.highlight {
            let r = around[i];
            let (ml, mt) = (i64::from(MARGIN_LEFT), i64::from(MARGIN_TOP));
            names.push(Name {
                text: o.name.clone(),
                at: (cx + left, cy + top),
                body: (i64::from(r.0) + ml, i64::from(r.1) + mt, i64::from(r.2) + ml, i64::from(r.3) + mt),
                ink: INK,
                legend: Some(legend.len() - 1),
            });
        }
    }
    let inside = |p: (f32, f32)| p.0 >= left && p.0 < right && p.1 >= top && p.1 < bottom;
    let mut point_names: Vec<Name> = Vec::new();
    for p in &d.points {
        let at = d.to_px(p.at[0], p.at[1]);
        if !inside(at) {
            continue;
        }
        let c = if p.highlight {
            ACCENT
        } else {
            match p.kind {
                PointKind::Spawn => Rgba::rgb(255, 208, 64),
                PointKind::Light => Rgba::rgb(255, 250, 200),
                PointKind::Sound => Rgba::rgb(120, 200, 255),
                PointKind::Particles => Rgba::rgb(255, 150, 90),
                PointKind::WayPoint => WAY,
            }
        };
        match p.kind {
            // a diamond: something lives or appears here
            PointKind::Spawn => {
                let r = 5.0;
                canvas.polygon(&[(at.0, at.1 - r), (at.0 + r, at.1), (at.0, at.1 + r), (at.0 - r, at.1)], c);
                canvas.polyline(
                    &[(at.0, at.1 - r), (at.0 + r, at.1), (at.0, at.1 + r), (at.0 - r, at.1)],
                    true,
                    1,
                    Rgba::BLACK,
                );
            }
            PointKind::WayPoint => canvas.disc(at, 3.0, c),
            _ => {
                canvas.disc(at, 4.0, Rgba::BLACK);
                canvas.disc(at, 3.0, c);
            }
        }
        if p.named || p.highlight {
            let (x, y) = (at.0 as i64, at.1 as i64);
            let name =
                Name { text: p.name.clone(), at, body: (x - 6, y - 6, x + 7, y + 7), ink: c, legend: None };
            if p.highlight {
                names.insert(0, name);
            } else {
                point_names.push(name);
            }
        }
    }
    names.extend(point_names);

    let mut taken: Vec<(i64, i64, i64, i64)> = Vec::new();
    let clear = |r: &(i64, i64, i64, i64), taken: &[(i64, i64, i64, i64)]| {
        !taken.iter().any(|t| r.0 < t.2 && t.0 < r.2 && r.1 < t.3 && t.1 < r.3)
    };
    let plate_at = |at: (f32, f32), text: &str| {
        let (tw, th) = gz_pix::text_size(text, 1);
        let (pw, ph) = (i64::from(tw) + 4, i64::from(th) + 4);
        let x =
            (at.0 as i64 - pw / 2).clamp(i64::from(MARGIN_LEFT), (i64::from(MARGIN_LEFT + w) - pw).max(0));
        let y = (at.1 as i64 - ph / 2).clamp(i64::from(MARGIN_TOP), (i64::from(MARGIN_TOP + h) - ph).max(0));
        (x, y, x + pw, y + ph)
    };
    // the marks are the caller's question: always drawn, over everything
    let mut mark_plates = Vec::new();
    for m in &d.marks {
        let at = d.to_px(m.at[0], m.at[1]);
        if !inside(at) {
            continue;
        }
        let r = plate_at((at.0, at.1 - 12.0), &m.text);
        taken.push(r);
        mark_plates.push((at, r, m.text.clone()));
    }
    if d.labels {
        let mut drawn = 0;
        for n in &names {
            if drawn >= MAX_LABELS {
                break;
            }
            // on a body much larger than its name the name stands in its middle; a smaller one
            // would vanish under it, so its name stands above it, below it or beside it
            let (tw, th) = gz_pix::text_size(&n.text, 1);
            let (pw, ph) = (i64::from(tw) + 4, i64::from(th) + 4);
            let b = n.body;
            let roomy = (b.2 - b.0) * 2 >= pw * 3 && (b.3 - b.1) >= ph * 3;
            let (mx, my) = ((b.0 + b.2) as f32 * 0.5, (b.1 + b.3) as f32 * 0.5);
            let half = (pw as f32 * 0.5, ph as f32 * 0.5);
            let places = [
                (mx, b.1 as f32 - half.1 - 2.0),
                (mx, b.3 as f32 + half.1 + 2.0),
                (b.2 as f32 + half.0 + 2.0, my),
                (b.0 as f32 - half.0 - 2.0, my),
            ];
            let free = |r: &(i64, i64, i64, i64)| {
                // the plate was not pushed back into the map over the body it names
                let off_body = roomy || r.2 <= b.0 || r.0 >= b.2 || r.3 <= b.1 || r.1 >= b.3;
                off_body && clear(r, &taken)
            };
            let first = if roomy { Some(plate_at(n.at, &n.text)).filter(|r| free(r)) } else { None };
            let Some(r) = first.or_else(|| places.iter().map(|at| plate_at(*at, &n.text)).find(|r| free(r)))
            else {
                continue;
            };
            canvas.label(r.0, r.1, &n.text, 1, 2, n.ink, PLATE);
            taken.push(r);
            drawn += 1;
            if let Some(i) = n.legend {
                legend[i].named = true;
            }
        }
    }
    if d.mode == MapMode::Height && heights.is_some() {
        // spot heights where the grid's lines cross: the picture read back in numbers
        for nx in first(d.area[0])..=last(d.area[2]) {
            for nz in first(d.area[1])..=last(d.area[3]) {
                let (x, z) = (nx as f32 * step, nz as f32 * step);
                let (px, py) = d.to_px(x, z);
                let (i, j) = ((px - left).floor() as i64, (py - top).floor() as i64);
                let Some(s) = d.sample(i, j).filter(|s| s.object != u32::MAX) else { continue };
                let text = format!("{:.1}", s.height);
                let r = plate_at((px, py), &text);
                if clear(&r, &taken) {
                    canvas.label(r.0, r.1, &text, 1, 2, INK, PLATE);
                    taken.push(r);
                }
            }
        }
    }
    for (at, r, text) in &mark_plates {
        canvas.line((at.0 - 6.0, at.1), (at.0 + 6.0, at.1), 2, MARK);
        canvas.line((at.0, at.1 - 6.0), (at.0, at.1 + 6.0), 2, MARK);
        canvas.label(r.0, r.1, text, 1, 2, Rgba::WHITE, MARK.shade(0.7));
    }

    // the coordinates at the edges: x along the top, z down the left side
    canvas.fill_rect(0, 0, i64::from(MARGIN_LEFT + w), i64::from(MARGIN_TOP), MARGIN);
    canvas.fill_rect(0, 0, i64::from(MARGIN_LEFT), i64::from(MARGIN_TOP + h), MARGIN);
    let mut last_end = i64::MIN;
    for n in first(d.area[0])..=last(d.area[2]) {
        let x = n as f32 * step;
        let (px, _) = d.to_px(x, d.area[1]);
        let text = number(x, step);
        let (tw, _) = gz_pix::text_size(&text, 1);
        let tx = px as i64 - i64::from(tw) / 2;
        if tx > last_end + 4 && tx + i64::from(tw) <= i64::from(MARGIN_LEFT + w) {
            canvas.text(tx, 3, &text, 1, INK);
            last_end = tx + i64::from(tw);
        }
        canvas.line((px, top - 3.0), (px, top - 1.0), 1, INK);
    }
    for n in first(d.area[1])..=last(d.area[3]) {
        let z = n as f32 * step;
        let (_, py) = d.to_px(d.area[0], z);
        let text = number(z, step);
        let (tw, th) = gz_pix::text_size(&text, 1);
        let ty = py as i64 - i64::from(th) / 2;
        if ty >= i64::from(MARGIN_TOP) - 3 && ty + i64::from(th) <= i64::from(MARGIN_TOP + h) {
            canvas.text(i64::from(MARGIN_LEFT) - 5 - i64::from(tw), ty, &text, 1, INK);
        }
        canvas.line((left - 3.0, py), (left - 1.0, py), 1, INK);
    }
    // which way is which, in the corner the coordinates leave free
    canvas.text(2, 3, "z\\x", 1, INK.mix(MARGIN, 0.4));

    Map { canvas, px_per_m: k, grid: step, legend, heights, contour }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of 10 x 10 m at 20 px a metre: flat ground (object 0) with a 2 x 2 m box (object 1)
    /// in the middle, 3 m high.
    fn data() -> MapData {
        let (w, h) = (200u32, 200u32);
        let mut samples = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            for i in 0..w {
                let (x, z) = ((i as f32 + 0.5) / 20.0, 10.0 - (j as f32 + 0.5) / 20.0);
                let on_box = (4.0..6.0).contains(&x) && (4.0..6.0).contains(&z);
                samples.push(if on_box {
                    Sample::hit(1, 3.0, [0.0, 1.0, 0.0])
                } else {
                    Sample::hit(0, 0.0, [0.0, 1.0, 0.0])
                });
            }
        }
        MapData {
            area: [0.0, 0.0, 10.0, 10.0],
            size: [w, h],
            samples,
            objects: vec![
                MapObject { name: "ground".into(), kind: "object", highlight: false },
                MapObject { name: "box_01".into(), kind: "object", highlight: false },
            ],
            points: Vec::new(),
            lines: Vec::new(),
            marks: Vec::new(),
            nodes: Vec::new(),
            node_size: 0.7,
            mode: MapMode::Objects,
            grid: None,
            labels: true,
        }
    }

    fn px(m: &Map, x: f32, z: f32) -> Rgba {
        let (i, j) = (MARGIN_LEFT as f32 + x * m.px_per_m, MARGIN_TOP as f32 + (10.0 - z) * m.px_per_m);
        m.canvas.pixel(i as i64, j as i64).unwrap_or(Rgba::CLEAR)
    }

    #[test]
    fn the_scale_and_the_steps_are_what_the_numbers_say() {
        let m = draw(&data());
        assert!((m.px_per_m - 20.0).abs() < 1e-4);
        assert_eq!((m.canvas.width(), m.canvas.height()), (200 + MARGIN_LEFT, 200 + MARGIN_TOP));
        // 20 px a metre: lines 5 m apart are the first that stand 56 px apart
        assert_eq!(m.grid, 5.0);
        assert_eq!(grid_step(100.0), 1.0);
        assert_eq!(grid_step(56.0), 1.0);
        assert_eq!(grid_step(1.0), 100.0);
        assert_eq!(grid_step(0.01), 1000.0);
        assert_eq!(contour_step(3.0), 0.25);
        assert_eq!(contour_step(60.0), 5.0);
        assert_eq!(m.heights, Some((0.0, 3.0)));
        assert_eq!(m.contour, None);
    }

    #[test]
    fn an_object_has_its_own_colour_and_the_ground_has_none() {
        let m = draw(&data());
        let ground = px(&m, 1.3, 1.3);
        let on_box = px(&m, 4.4, 5.6);
        assert_ne!(ground, on_box);
        // the legend says the colour the box was drawn in, and where it is
        let row = m.legend.iter().find(|r| r.object == "box_01");
        let Some(row) = row else { panic!("no legend row") };
        assert_eq!(row.colour, hex(colour_of("box_01")));
        assert_eq!(row.centre, [5.0, 5.0]);
        assert!((row.area - 4.0).abs() < 0.05, "{}", row.area);
        assert!(row.named);
        let Some(g) = m.legend.iter().find(|r| r.object == "ground") else { panic!("no ground") };
        assert!(!g.named && g.colour == hex(GROUND) && g.area > 90.0);
        // the same name, the same colour; another name, another one
        assert_eq!(colour_of("box_01"), colour_of("box_01"));
        assert_ne!(colour_of("box_01"), colour_of("box_02"));
    }

    #[test]
    fn a_body_has_an_edge_and_a_name() {
        let m = draw(&data());
        // the last pixel of the ground before the box is darker than the ground beside it
        let k = m.px_per_m;
        let at = |x: f32, z: f32| px(&m, x, z);
        assert!(at(4.0 - 0.5 / k, 5.6).luma() < at(3.0, 5.6).luma() - 0.1);
        // the name stands above the box, which it would otherwise hide: ink over it, none on it
        let (cx, cy) = (MARGIN_LEFT as i64 + 100, MARGIN_TOP as i64 + 100);
        let ink_in = |m: &Map, y0: i64, y1: i64| {
            (cx - 24..cx + 24).any(|x| (y0..y1).any(|y| m.canvas.pixel(x, y) == Some(INK)))
        };
        assert!(ink_in(&m, cy - 20 - 14, cy - 20));
        assert!(!ink_in(&m, cy - 18, cy + 18));
        // without labels nothing is written on the map itself
        let mut d = data();
        d.labels = false;
        let m = draw(&d);
        assert!(!ink_in(&m, cy - 20 - 14, cy + 20));
        assert!(m.legend.iter().all(|r| !r.named));
    }

    #[test]
    fn the_grid_carries_its_coordinates_at_the_edges() {
        let m = draw(&data());
        // a line at x = 5: lighter than the ground beside it, away from the box and its name
        let line = px(&m, 5.0 + 0.01, 1.03);
        let beside = px(&m, 5.3, 1.03);
        assert!(line.luma() > beside.luma(), "{line:?} {beside:?}");
        // digits in the top margin over x = 5 and x = 10 ... and in the left one
        let ink_in = |x0: i64, y0: i64, x1: i64, y1: i64| {
            (x0..x1).any(|x| (y0..y1).any(|y| m.canvas.pixel(x, y) == Some(INK)))
        };
        let x5 = MARGIN_LEFT as i64 + 100;
        assert!(ink_in(x5 - 4, 2, x5 + 4, 11));
        let z5 = MARGIN_TOP as i64 + 100;
        assert!(ink_in(MARGIN_LEFT as i64 - 14, z5 - 5, MARGIN_LEFT as i64 - 4, z5 + 5));
        // nothing of the map is drawn into the margins
        assert_eq!(m.canvas.pixel(20, 60), Some(MARGIN));
    }

    #[test]
    fn a_mark_a_point_and_a_way_are_drawn_where_they_are() {
        let mut d = data();
        d.labels = false;
        d.marks.push(MapMark { at: [2.0, 8.0], text: "A".into() });
        d.points.push(MapPoint {
            name: "spawn_1".into(),
            kind: PointKind::Spawn,
            at: [8.0, 2.0],
            highlight: false,
            named: true,
        });
        d.lines.push(MapLine { points: vec![[1.0, 1.0], [3.0, 1.0]], closed: false, way: true });
        let m = draw(&d);
        assert_eq!(px(&m, 2.0 - 0.2, 8.0), MARK);
        assert_eq!(px(&m, 8.0, 2.0), Rgba::rgb(255, 208, 64));
        assert_eq!(px(&m, 2.0, 1.0), WAY);
        // a mark outside the area is not drawn and breaks nothing
        d.marks.push(MapMark { at: [50.0, 50.0], text: "far".into() });
        let _ = draw(&d);
    }

    #[test]
    fn a_highlighted_object_is_in_the_accent_colour() {
        let mut d = data();
        d.objects[1].highlight = true;
        d.labels = false;
        let m = draw(&d);
        let c = px(&m, 4.6, 5.4);
        // away from the grid's lines: the accent, shaded by the light like everything else
        assert!(c.0 > 200 && c.1 < 90 && c.2 > 110, "{c:?}");
        let Some(row) = m.legend.iter().find(|r| r.object == "box_01") else { panic!("no row") };
        assert_eq!(row.colour, hex(ACCENT));
    }

    #[test]
    fn heights_are_colours_with_contours_and_spot_heights() {
        let mut d = data();
        d.mode = MapMode::Height;
        // a slope: 0 m in the west, 10 m in the east
        for (n, s) in d.samples.iter_mut().enumerate() {
            let i = n % 200;
            *s = Sample::hit(0, (i as f32 + 0.5) / 20.0, [0.0, 1.0, 0.0]);
        }
        d.objects.truncate(1);
        let m = draw(&d);
        assert_eq!(m.contour, Some(1.0));
        let Some((lo, hi)) = m.heights else { panic!("no heights") };
        assert!(lo < 0.1 && hi > 9.9);
        // low is blue, high is light
        let west = px(&m, 0.4, 7.7);
        let east = px(&m, 9.6, 7.7);
        assert!(west.2 > west.0 && east.luma() > west.luma() + 0.2, "{west:?} {east:?}");
        // a contour where the height crosses 3 m: darker than half a metre on
        assert!(px(&m, 3.0 - 0.6 / 20.0, 7.7).luma() < px(&m, 3.5, 7.7).luma());
        // a spot height at the crossing of the grid's lines
        let (cx, cy) = (MARGIN_LEFT as i64 + 100, MARGIN_TOP as i64 + 100);
        let inked = (cx - 14..cx + 14).any(|x| (cy - 6..cy + 6).any(|y| m.canvas.pixel(x, y) == Some(INK)));
        assert!(inked);
        assert_eq!(ramp(0.0), Rgba::rgb(38, 70, 150));
        assert_eq!(ramp(1.0), Rgba::rgb(240, 240, 240));
        assert_eq!(ramp(f32::NAN), ramp(0.0));
    }

    #[test]
    fn the_ai_map_is_dots_over_the_ground() {
        let mut d = data();
        d.mode = MapMode::Ai;
        d.labels = false;
        d.nodes = vec![[2.0, 2.0], [2.7, 2.0]];
        let m = draw(&d);
        let on = px(&m, 2.0, 2.0);
        let off = px(&m, 7.5, 7.5);
        assert!(on.1 > on.0 && on.1 > on.2, "{on:?}");
        assert!(!(off.1 > off.0 + 40), "{off:?}");
    }

    #[test]
    fn nothing_under_a_ray_is_the_void_and_an_empty_map_is_a_picture_too() {
        let mut d = data();
        d.samples.fill(Sample::NONE);
        let m = draw(&d);
        assert_eq!(m.heights, None);
        assert!(m.legend.is_empty());
        assert_eq!(px(&m, 3.3, 3.3), VOID);
        // samples that are too few: the rest is left as the margin's colour, nothing panics
        d.samples.truncate(10);
        let _ = draw(&d);
        // the same data, the same bytes
        let (a, b) = (draw(&data()), draw(&data()));
        assert_eq!(a.canvas, b.canvas);
    }
}

//! The lightmap and the packing of the lit charts into lightmap pages, by visual: the charts of
//! one visual into one tight rectangle (a raster of 64-bit words, the charts turned when that
//! packs better), the rectangles into 1024 x 1024 pages by a skyline; every chart captured into
//! its page - the faces get their lightmap coordinates - and every page saved: its border filled
//! and its texels packed into the two pictures the game reads (`lmap#N_1`, `lmap#N_2`). Where a
//! chart lands is light (its size after the light), but which faces share a page is the visuals',
//! decided before the light. The DDS writer is `gz-fmt-tex`'s.

use gz_fmt_tex::{DdsEncoding, Image, TgaDepth, encode_dds, write_tga};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::deflector::{Deflector, Deflectors, LMAP_SIZE};
use crate::layer::{BORDER, LmLayer, apply_borders, blit, blit_r, image_of};
use crate::mesh::{DeflId, Mesh};
use crate::{Ctl, LcError, Result};

/// `alpha_ref` of the page's placement mask.
const ALPHA_REF: u8 = (254 - BORDER) as u8;
const SIDE: usize = LMAP_SIZE as usize;

/// A page while deflectors are captured into it, then the name the faces' geometry
/// refers to it by.
#[derive(Clone, Debug, Default)]
pub struct Lightmap {
    pub lm: LmLayer,
    /// `lm_texture.name`: `lmap#N`.
    pub name: SmolStr,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

/// The two pictures of a saved page.
#[derive(Clone, Debug, PartialEq)]
pub struct LightmapPage {
    /// `lmap#N`.
    pub name: SmolStr,
    /// `<name>_1`: the static light in rgb, the sun in alpha.
    pub base: Image,
    /// `<name>_2`: the sun in rgb, the hemisphere in alpha.
    pub hemi: Image,
}

impl LightmapPage {
    /// Both pictures as DXT5 without mip levels.
    pub fn dds(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            encode_dds(std::slice::from_ref(&self.base), DdsEncoding::Dxt5)?,
            encode_dds(std::slice::from_ref(&self.hemi), DdsEncoding::Dxt5)?,
        ))
    }

    /// A plain picture of the page for a look: the base above the hemisphere, alpha kept.
    pub fn debug_tga(&self) -> Result<Vec<u8>> {
        let mut rgba = self.base.rgba.clone();
        rgba.extend_from_slice(&self.hemi.rgba);
        let both = Image::new(self.base.width, self.base.height * 2, rgba)?;
        Ok(write_tga(&both, TgaDepth::Bgra32)?)
    }
}

impl Lightmap {
    /// `Capture`: the deflector's faces get their coordinates in this page (a channel each) and
    /// its texels are copied in at `(b_u, b_v)`, turned a quarter when `rotated`.
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        page: u32,
        d: &Deflector,
        mesh: &mut Mesh,
        b_u: u32,
        b_v: u32,
        s_u: u32,
        s_v: u32,
        rotated: bool,
    ) {
        if self.lm.surface.is_empty() {
            self.lm.create(LMAP_SIZE, LMAP_SIZE);
        }
        let tris = d.remap_uv_into(
            b_u + BORDER,
            b_v + BORDER,
            s_u.saturating_sub(2 * BORDER),
            s_v.saturating_sub(2 * BORDER),
            LMAP_SIZE,
            LMAP_SIZE,
            rotated,
        );
        for t in &tris {
            if let Some(face) = mesh.face_mut(t.owner) {
                face.lmap = Some(page);
                face.add_channel(t.uv[0], t.uv[1], t.uv[2]);
            }
        }
        let l = &d.layer;
        let (real_w, real_h) = (l.width + 2 * BORDER, l.height + 2 * BORDER);
        if rotated {
            blit_r(&mut self.lm, LMAP_SIZE, LMAP_SIZE, l, real_w, real_h, b_u, b_v, 254 - BORDER);
        } else {
            blit(&mut self.lm, LMAP_SIZE, LMAP_SIZE, l, real_w, real_h, b_u, b_v, 254 - BORDER);
        }
    }

    /// `Save`: the page's texels kept or dropped by their marker, sixteen rings of border grown,
    /// packed into the two pictures; the texels are freed.
    pub fn save(&mut self, id: u32) -> LightmapPage {
        let lm = &mut self.lm;
        if lm.surface.is_empty() {
            lm.create(LMAP_SIZE, LMAP_SIZE);
        }
        lm.marker.par_iter_mut().for_each(|m| *m = if u32::from(*m) >= 254 - BORDER { 255 } else { 0 });
        for r in ((254 - 16 + 1)..=254).rev() {
            apply_borders(lm, r);
        }
        let base = image_of(lm.width, lm.height, &lm.pack());
        let hemi = image_of(lm.width, lm.height, &lm.pack_hemi());
        self.has_alpha = true;
        self.width = lm.width;
        self.height = lm.height;
        self.name = SmolStr::new(format!("lmap#{id}"));
        lm.destroy();
        LightmapPage { name: self.name.clone(), base, hemi }
    }
}

/// A placed rectangle, corners inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub ax: u32,
    pub ay: u32,
    pub bx: u32,
    pub by: u32,
}

impl Rect {
    #[must_use]
    pub fn size_x(&self) -> u32 {
        self.bx - self.ax + 1
    }

    #[must_use]
    pub fn size_y(&self) -> u32 {
        self.by - self.ay + 1
    }
}

/// The texels of a chart that take room in a page (marked at least [`ALPHA_REF`], what the
/// capture's blit copies), as rows of 64-bit words: bit `b` of word `j` of a row is column
/// `64 j + b`.
#[derive(Clone, Debug)]
struct Shape {
    w: usize,
    h: usize,
    words: usize,
    bits: Vec<u64>,
}

impl Shape {
    /// The layer's texels with its border, upright or turned a quarter as `blit_r` turns them
    /// (row `y` of the layer becomes column `y`).
    fn of(lm: &LmLayer, rotated: bool) -> Self {
        let s_x = lm.width as usize + 2 * BORDER as usize;
        let s_y = lm.height as usize + 2 * BORDER as usize;
        let (w, h) = if rotated { (s_y, s_x) } else { (s_x, s_y) };
        let words = w.div_ceil(64);
        let mut bits = vec![0u64; words * h];
        for row in 0..h {
            for col in 0..w {
                let (x, y) = if rotated { (row, col) } else { (col, row) };
                if lm.marker.get(y * s_x + x).is_some_and(|&m| m >= ALPHA_REF)
                    && let Some(word) = bits.get_mut(row * words + col / 64)
                {
                    *word |= 1 << (col % 64);
                }
            }
        }
        Self { w, h, words, bits }
    }

    fn row(&self, y: usize) -> &[u64] {
        self.bits.get(y * self.words..(y + 1) * self.words).unwrap_or_default()
    }

    fn texels(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }
}

/// A raster `w` texels wide whose rows grow as charts are put into it.
#[derive(Clone, Debug)]
struct Raster {
    w: usize,
    words: usize,
    rows: Vec<u64>,
}

impl Raster {
    fn new(w: usize) -> Self {
        Self { w, words: w.div_ceil(64) + 1, rows: Vec::new() }
    }

    fn height(&self) -> usize {
        self.rows.len() / self.words
    }

    /// Whether the shape's texels meet none of the raster's at `(x, y)`: a chart row against the
    /// raster's row by shifts and `&`, a word at a time. Rows past the raster's end are free.
    fn fits(&self, s: &Shape, x: usize, y: usize) -> bool {
        if x + s.w > self.w {
            return false;
        }
        let (q, sh) = (x / 64, x % 64);
        for r in 0..s.h {
            let Some(row) = self.rows.get((y + r) * self.words..(y + r + 1) * self.words) else {
                return true;
            };
            for (j, &cw) in s.row(r).iter().enumerate() {
                if cw == 0 {
                    continue;
                }
                let lo = cw << sh;
                let hi = if sh == 0 { 0 } else { cw >> (64 - sh) };
                if row.get(q + j).is_some_and(|&m| m & lo != 0)
                    || row.get(q + j + 1).is_some_and(|&m| m & hi != 0)
                {
                    return false;
                }
            }
        }
        true
    }

    fn put(&mut self, s: &Shape, x: usize, y: usize) {
        let need = (y + s.h) * self.words;
        if self.rows.len() < need {
            self.rows.resize(need, 0);
        }
        let (q, sh) = (x / 64, x % 64);
        for r in 0..s.h {
            let base = (y + r) * self.words;
            for (j, &cw) in s.row(r).iter().enumerate() {
                if let Some(m) = self.rows.get_mut(base + q + j) {
                    *m |= cw << sh;
                }
                if sh != 0
                    && let Some(m) = self.rows.get_mut(base + q + j + 1)
                {
                    *m |= cw >> (64 - sh);
                }
            }
        }
    }

    /// The first place, row by row, where the shape fits: always found, at the latest below
    /// everything placed.
    fn first_fit(&self, s: &Shape) -> Option<(usize, usize)> {
        let x_max = self.w.checked_sub(s.w)?;
        (0..=self.height()).find_map(|y| (0..=x_max).find(|&x| self.fits(s, x, y)).map(|x| (x, y)))
    }
}

/// Where a chart lies in its visual's rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spot {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    rotated: bool,
}

/// A visual's rectangle: its width and height, where each chart lies in it, the texels they take.
type Packed = (u32, u32, Vec<Spot>, usize);

/// A visual's charts packed into one tight rectangle: the charts by area (the larger first, then
/// in their order), each where it first fits upright or turned - the turn whose bottom edge is
/// higher - in a raster about as wide as the root of their area. `None` when the rectangle
/// would not fit a page.
fn pack_visual(layers: &[&LmLayer]) -> Option<Packed> {
    let shapes: Vec<[Shape; 2]> = layers.iter().map(|l| [Shape::of(l, false), Shape::of(l, true)]).collect();
    let area: usize = shapes.iter().map(|s| s[0].w * s[0].h).sum();
    let narrow = shapes.iter().map(|s| s[0].w.min(s[0].h)).max().unwrap_or(1);
    let side = (area as f64).sqrt().ceil() as usize;
    let w = side.max(narrow).min(SIDE);
    let mut order: Vec<usize> = (0..shapes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(shapes[i][0].w * shapes[i][0].h), i));
    let mut raster = Raster::new(w);
    let mut spots = vec![Spot { x: 0, y: 0, w: 0, h: 0, rotated: false }; shapes.len()];
    let (mut used_w, mut used_h, mut texels) = (0usize, 0usize, 0usize);
    for i in order {
        let [up, turned] = &shapes[i];
        let fit = |s: &Shape| raster.first_fit(s).map(|(x, y)| (y + s.h, y, x));
        let (shape, rotated, (_, y, x)) = match (fit(up), fit(turned)) {
            (Some(a), Some(b)) if b < a => (turned, true, b),
            (Some(a), _) => (up, false, a),
            (None, Some(b)) => (turned, true, b),
            (None, None) => return None,
        };
        raster.put(shape, x, y);
        used_w = used_w.max(x + shape.w);
        used_h = used_h.max(y + shape.h);
        texels += shape.texels();
        spots[i] = Spot { x: x as u32, y: y as u32, w: shape.w as u32, h: shape.h as u32, rotated };
    }
    (used_w <= SIDE && used_h <= SIDE).then_some((used_w as u32, used_h as u32, spots, texels))
}

/// A page's skyline: its columns' heights as runs `(x, y, w)` from left to right.
#[derive(Clone, Debug)]
struct Skyline(Vec<(u32, u32, u32)>);

impl Default for Skyline {
    fn default() -> Self {
        Self(vec![(0, 0, LMAP_SIZE)])
    }
}

impl Skyline {
    /// The lowest place a `w` x `h` rectangle fits, the leftmost of equal ones.
    fn find(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        let mut best: Option<(u32, u32)> = None;
        for (i, &(x, _, _)) in self.0.iter().enumerate() {
            if x + w > LMAP_SIZE {
                break;
            }
            let mut y = 0;
            for &(sx, sy, _) in &self.0[i..] {
                if sx >= x + w {
                    break;
                }
                y = y.max(sy);
            }
            if y + h <= LMAP_SIZE && best.is_none_or(|(by, bx)| (y, x) < (by, bx)) {
                best = Some((y, x));
            }
        }
        best.map(|(y, x)| (x, y))
    }

    fn add(&mut self, x: u32, y: u32, w: u32, h: u32) {
        let mut out = Vec::with_capacity(self.0.len() + 2);
        let mut inserted = false;
        for &(sx, sy, sw) in &self.0 {
            let end = sx + sw;
            if end <= x || sx >= x + w {
                if sx >= x + w && !inserted {
                    out.push((x, y + h, w));
                    inserted = true;
                }
                out.push((sx, sy, sw));
                continue;
            }
            if sx < x {
                out.push((sx, sy, x - sx));
            }
            if !inserted {
                out.push((x, y + h, w));
                inserted = true;
            }
            if end > x + w {
                out.push((x + w, sy, end - x - w));
            }
        }
        if !inserted {
            out.push((x, y + h, w));
        }
        // runs of one height joined
        let mut joined: Vec<(u32, u32, u32)> = Vec::with_capacity(out.len());
        for r in out {
            match joined.last_mut() {
                Some(l) if l.1 == r.1 && l.0 + l.2 == r.0 => l.2 += r.2,
                _ => joined.push(r),
            }
        }
        self.0 = joined;
    }
}

/// What the packing of the lightmaps did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackReport {
    pub pages: usize,
    /// The texels the charts take with their borders, of all the pages' texels.
    pub fill: f64,
    /// The visuals whose charts did not fit one page: placed chart by chart, they end up on
    /// several pages (the caller makes a visual of each page's faces). Expected never.
    pub split: Vec<usize>,
}

/// One rectangle to place: a visual, or one chart of a visual that did not fit a page whole.
struct Block {
    visual: usize,
    /// The group whose pages it goes on.
    group: u32,
    w: u32,
    h: u32,
    charts: Vec<(DeflId, Spot)>,
}

/// The lightmaps packed by visual, in two levels: the charts of each visual (`visuals`, the
/// deflectors of each visual's faces) into one tight rectangle, the rectangles into pages by a
/// skyline in a stable order - the larger first, then the visual's index -, each into the first
/// page it fits. Every chart of a visual is on one page by construction. Then every chart is
/// captured - its faces get their page and their lightmap coordinates - and every page saved, the
/// pages in parallel. `lightmaps` gets the pages; the deflectors are destroyed afterwards.
pub fn pack_lightmaps(
    defls: &mut Deflectors,
    visuals: &[Vec<DeflId>],
    mesh: &mut Mesh,
    lightmaps: &mut Vec<Lightmap>,
    ctl: &Ctl<'_>,
) -> Result<(Vec<LightmapPage>, PackReport)> {
    pack_grouped(defls, visuals, None, mesh, lightmaps, ctl).map(|(pages, report, _)| (pages, report))
}

/// [`pack_lightmaps`] with the visuals in groups (`groups[i]` the group of visual `i`; `None`: one
/// group): a page holds the charts of one group, and every group's pages are numbered from 1 in
/// the order they are opened (one group: the pages' order). Answers the group of every page too.
pub fn pack_grouped(
    defls: &mut Deflectors,
    visuals: &[Vec<DeflId>],
    groups: Option<&[u32]>,
    mesh: &mut Mesh,
    lightmaps: &mut Vec<Lightmap>,
    ctl: &Ctl<'_>,
) -> Result<(Vec<LightmapPage>, PackReport, Vec<u32>)> {
    let group_of = |i: usize| groups.and_then(|g| g.get(i)).copied().unwrap_or(0);
    ctl.stage("the charts of each visual");
    let packed: Vec<Option<Packed>> = visuals
        .par_iter()
        .map(|v| {
            if ctl.cancelled() {
                return None;
            }
            let layers: Vec<&LmLayer> = v.iter().filter_map(|&d| defls.get(d).map(|x| &x.layer)).collect();
            if layers.is_empty() { None } else { pack_visual(&layers) }
        })
        .collect();
    ctl.check()?;
    let mut report = PackReport::default();
    let mut blocks = Vec::new();
    let mut texels = 0usize;
    for (i, (v, p)) in visuals.iter().zip(packed).enumerate() {
        let ids: Vec<DeflId> = v.iter().copied().filter(|&d| defls.get(d).is_some()).collect();
        if ids.is_empty() {
            continue;
        }
        match p {
            Some((w, h, spots, t)) => {
                texels += t;
                blocks.push(Block {
                    visual: i,
                    group: group_of(i),
                    w,
                    h,
                    charts: ids.into_iter().zip(spots).collect(),
                });
            }
            None => {
                // a pathology the merge's limit rules out: the visual's charts one by one
                report.split.push(i);
                for d in ids {
                    let Some(l) = defls.get(d).map(|x| &x.layer) else { continue };
                    let Some((w, h, spots, t)) = pack_visual(&[l]) else {
                        return Err(LcError::Invalid(format!(
                            "A deflector of {} x {} texels does not fit a lightmap page.",
                            l.width, l.height
                        )));
                    };
                    texels += t;
                    blocks.push(Block {
                        visual: i,
                        group: group_of(i),
                        w,
                        h,
                        charts: spots.into_iter().map(|s| (d, s)).collect(),
                    });
                }
            }
        }
    }
    blocks.sort_by_key(|b| (std::cmp::Reverse(u64::from(b.w) * u64::from(b.h)), b.visual));

    ctl.stage("the visuals into pages");
    // per page: its group, its skyline, its number in the group
    let mut skylines: Vec<(u32, Skyline, u32)> = Vec::new();
    // per page: the charts with where they lie in it
    let mut placed: Vec<Vec<(DeflId, Spot)>> = Vec::new();
    for (n, b) in blocks.iter().enumerate() {
        if n % 256 == 0 {
            ctl.check()?;
            ctl.progress(n, blocks.len());
        }
        let found = skylines
            .iter()
            .enumerate()
            .filter(|(_, s)| s.0 == b.group)
            .find_map(|(p, s)| s.1.find(b.w, b.h).map(|at| (p, at)));
        let (page, (x, y)) = match found {
            Some(f) => f,
            None => {
                let s = Skyline::default();
                let at = s.find(b.w, b.h).ok_or_else(|| {
                    LcError::Invalid(format!("A lightmap of {} x {} texels does not fit a page.", b.w, b.h))
                })?;
                let number = skylines.iter().filter(|s| s.0 == b.group).count() as u32;
                skylines.push((b.group, s, number));
                placed.push(Vec::new());
                (skylines.len() - 1, at)
            }
        };
        if let Some(s) = skylines.get_mut(page) {
            s.1.add(x, y, b.w, b.h);
        }
        if let Some(list) = placed.get_mut(page) {
            list.extend(b.charts.iter().map(|&(d, s)| (d, Spot { x: s.x + x, y: s.y + y, ..s })));
        }
    }
    ctl.check()?;

    // the faces' coordinates in their pages, one chart after another
    ctl.stage("capturing the charts");
    let first = lightmaps.len();
    for (p, list) in placed.iter().enumerate() {
        for &(d, s) in list {
            if let Some(defl) = defls.get(d) {
                capture_faces(defl, mesh, (first + p) as u32, s);
            }
        }
    }
    ctl.check()?;
    ctl.stage("saving the pages");
    let defls_ref = &*defls;
    let numbers: Vec<u32> = skylines.iter().map(|s| s.2).collect();
    let pages: Vec<(Lightmap, LightmapPage)> = placed
        .par_iter()
        .enumerate()
        .map(|(p, list)| {
            let mut lmap = Lightmap::default();
            lmap.lm.create(LMAP_SIZE, LMAP_SIZE);
            for &(d, s) in list {
                if let Some(defl) = defls_ref.get(d) {
                    lmap.blit_chart(defl, s);
                }
            }
            let number = numbers.get(p).copied().unwrap_or(p as u32) as usize;
            let page = lmap.save((first + number + 1) as u32);
            (lmap, page)
        })
        .collect();
    ctl.check()?;
    report.pages = pages.len();
    report.fill =
        if pages.is_empty() { 0.0 } else { texels as f64 / (pages.len() as f64 * (SIDE * SIDE) as f64) };
    let mut out = Vec::with_capacity(pages.len());
    for (l, p) in pages {
        lightmaps.push(l);
        out.push(p);
    }
    defls.clear();
    Ok((out, report, skylines.iter().map(|s| s.0).collect()))
}

/// The chart's faces get their page and their coordinates in it (a channel each).
fn capture_faces(d: &Deflector, mesh: &mut Mesh, page: u32, s: Spot) {
    let tris = d.remap_uv_into(
        s.x + BORDER,
        s.y + BORDER,
        s.w.saturating_sub(2 * BORDER),
        s.h.saturating_sub(2 * BORDER),
        LMAP_SIZE,
        LMAP_SIZE,
        s.rotated,
    );
    for t in &tris {
        if let Some(face) = mesh.face_mut(t.owner) {
            face.lmap = Some(page);
            face.add_channel(t.uv[0], t.uv[1], t.uv[2]);
        }
    }
}

impl Lightmap {
    /// The chart's texels copied in where it lies, turned a quarter when it is.
    fn blit_chart(&mut self, d: &Deflector, s: Spot) {
        let l = &d.layer;
        let (real_w, real_h) = (l.width + 2 * BORDER, l.height + 2 * BORDER);
        if s.rotated {
            blit_r(&mut self.lm, LMAP_SIZE, LMAP_SIZE, l, real_w, real_h, s.x, s.y, 254 - BORDER);
        } else {
            blit(&mut self.lm, LMAP_SIZE, LMAP_SIZE, l, real_w, real_h, s.x, s.y, 254 - BORDER);
        }
    }
}

/// Every deflector on the list packed as a visual of its own, in the list's order: what a build
/// without visuals (a test of the light) makes of its deflectors.
pub fn merge_lightmaps(
    defls: &mut Deflectors,
    mesh: &mut Mesh,
    lightmaps: &mut Vec<Lightmap>,
    ctl: &Ctl<'_>,
) -> Result<Vec<LightmapPage>> {
    let visuals: Vec<Vec<DeflId>> = defls.list().iter().map(|&d| vec![d]).collect();
    Ok(pack_lightmaps(defls, &visuals, mesh, lightmaps, ctl)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(w: u32, h: u32) -> LmLayer {
        let mut l = LmLayer::new(w + 2, h + 2);
        l.marker.iter_mut().for_each(|m| *m = 255);
        l.width = w;
        l.height = h;
        l
    }

    /// A chart shaped like a triangle: the texels on and below the diagonal.
    fn triangle(n: u32) -> LmLayer {
        let mut l = LmLayer::new(n + 2, n + 2);
        let s = (n + 2) as usize;
        for y in 0..s {
            for x in 0..=y {
                l.marker[y * s + x] = 255;
            }
        }
        l.width = n;
        l.height = n;
        l
    }

    fn overlap(a: &Spot, b: &Spot) -> bool {
        !(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y)
    }

    #[test]
    fn a_visuals_charts_interlock_and_never_meet() {
        // two triangles fill one square: the second goes into the first one's empty half
        let (a, b) = (triangle(30), triangle(30));
        let (w, h, spots, texels) = pack_visual(&[&a, &b]).unwrap();
        assert!(w <= 64 && h <= 64, "{w} x {h}");
        let mut raster = Raster::new(w as usize);
        for (l, s) in [&a, &b].iter().zip(&spots) {
            let shape = Shape::of(l, s.rotated);
            assert!(raster.fits(&shape, s.x as usize, s.y as usize), "{spots:?}");
            raster.put(&shape, s.x as usize, s.y as usize);
        }
        assert_eq!(texels, 2 * (32 * 33 / 2));
        // rectangles of every size, several words wide: no two meet, all inside the rectangle
        let layers: Vec<LmLayer> = [(100, 50), (30, 300), (130, 70), (1, 1), (0, 0), (70, 9), (200, 3)]
            .iter()
            .map(|&(w, h)| lit(w, h))
            .collect();
        let refs: Vec<&LmLayer> = layers.iter().collect();
        let (w, h, spots, _) = pack_visual(&refs).unwrap();
        for (i, a) in spots.iter().enumerate() {
            assert!(a.x + a.w <= w && a.y + a.h <= h);
            for b in &spots[i + 1..] {
                assert!(!overlap(a, b), "{a:?} {b:?}");
            }
        }
        // the shifts across words: a chart placed at x = 70 reads two words of every row
        let mut r = Raster::new(256);
        let s = Shape::of(&lit(80, 2), false);
        r.put(&s, 70, 0);
        assert!(!r.fits(&s, 0, 0) && !r.fits(&s, 149, 1) && r.fits(&s, 152, 0) && r.fits(&s, 0, 4));
    }

    #[test]
    fn the_skyline_places_low_and_left_and_never_overlaps() {
        let mut s = Skyline::default();
        let mut placed: Vec<Spot> = Vec::new();
        for &(w, h) in &[(500, 300), (500, 200), (24, 1000), (300, 300), (700, 100), (100, 50)] {
            let (x, y) = s.find(w, h).unwrap();
            s.add(x, y, w, h);
            let spot = Spot { x, y, w, h, rotated: false };
            assert!(x + w <= LMAP_SIZE && y + h <= LMAP_SIZE);
            assert!(placed.iter().all(|p| !overlap(p, &spot)), "{spot:?} {placed:?}");
            placed.push(spot);
        }
        assert_eq!((placed[0].x, placed[0].y), (0, 0));
        assert_eq!((placed[1].x, placed[1].y), (500, 0));
        assert_eq!((placed[2].x, placed[2].y), (1000, 0));
        assert!(s.find(1024, 1024).is_none());
    }
}

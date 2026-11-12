//! `CLightmap` and the merge of the deflectors into lightmap pages (`CBuild::xrPhase_MergeLM`,
//! `xrPhase_MergeLM_Surface.cpp`): the deflectors sorted, as many as eight pages could hold
//! placed one by one at the first free spot of a 1024 x 1024 page (or turned a quarter),
//! captured into it - the faces get their lightmap coordinates - and the page saved: its border
//! filled and its texels packed into the two pictures the game reads (`lmap#N_1`, `lmap#N_2`).
//! The DDS writer is `gz-fmt-tex`'s.

use gz_fmt_tex::{DdsEncoding, Image, TgaDepth, encode_dds, write_tga};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::data::BuildMaterial;
use crate::deflector::{Deflector, Deflectors, LMAP_SIZE};
use crate::layer::{BORDER, LmLayer, apply_borders, blit, blit_r, image_of};
use crate::mesh::{DeflId, Mesh};
use crate::{Ctl, LcError, Result};

/// `alpha_ref` of the page's placement mask.
const ALPHA_REF: u8 = (254 - BORDER) as u8;
const SIDE: usize = LMAP_SIZE as usize;

/// `CLightmap`: a page while deflectors are captured into it, then the name the faces' geometry
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
    /// Both pictures as DXT5 without mip levels, as the C++ asks of its encoder.
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

/// `L_rect`: a placed rectangle, corners inclusive.
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

/// The placement mask of a page (`surface` of `xrPhase_MergeLM_Surface.cpp`): 255 where a
/// placed deflector's texel lies.
#[derive(Clone, Debug)]
pub struct PageMask {
    bits: Vec<u8>,
}

impl Default for PageMask {
    /// `_InitSurface`.
    fn default() -> Self {
        Self { bits: vec![0; SIDE * SIDE] }
    }
}

/// The marker of a deflector's texel `(x, y)`, `s_x` texels to a row.
fn mark(lm: &LmLayer, s_x: usize, x: usize, y: usize) -> u8 {
    lm.marker.get(y * s_x + x).copied().unwrap_or(0)
}

impl PageMask {
    fn at(&self, x: usize, y: usize) -> u8 {
        self.bits.get(y * SIDE + x).copied().unwrap_or(255)
    }

    /// `Place_Perpixel`: whether the deflector's texels fit at `(ax, ay)` without meeting a
    /// placed one. oracle: the C++ tests an upright layer's rows eight texels at a time while
    /// more than eight are left (`_mm_max_epu8` against `alpha_ref`: a texel marked exactly
    /// `alpha_ref` counts as free there) and the rest one by one (`>= alpha_ref`: taken);
    /// a turned layer one by one. The port tests the same way: where a deflector lands decides
    /// which page the ones after it reach, and with it the level's visuals.
    fn fits(&self, ax: usize, ay: usize, lm: &LmLayer, rotated: bool) -> bool {
        let s_x = lm.width as usize + 2 * BORDER as usize;
        let s_y = lm.height as usize + 2 * BORDER as usize;
        let (rows, cols) = if rotated { (s_x, s_y) } else { (s_y, s_x) };
        // the columns of the eight-at-a-time part: `for (x = 0; x < s_x - 8; x += 8)`
        let wide = if rotated || s_x <= 8 { 0 } else { (s_x - 8).div_ceil(8) * 8 };
        for y in 0..rows {
            for x in 0..cols {
                let m = if rotated { mark(lm, s_x, y, x) } else { mark(lm, s_x, x, y) };
                let taken = if x < wide { m > ALPHA_REF } else { m >= ALPHA_REF };
                if taken && self.at(ax + x, ay + y) != 0 {
                    return false;
                }
            }
        }
        true
    }

    /// `_rect_register`.
    fn register(&mut self, ax: usize, ay: usize, lm: &LmLayer, rotated: bool) {
        let s_x = lm.width as usize + 2 * BORDER as usize;
        let s_y = lm.height as usize + 2 * BORDER as usize;
        let (rows, cols) = if rotated { (s_x, s_y) } else { (s_y, s_x) };
        for y in 0..rows {
            for x in 0..cols {
                let m = if rotated { mark(lm, s_x, y, x) } else { mark(lm, s_x, x, y) };
                if m >= ALPHA_REF
                    && let Some(b) = self.bits.get_mut((ay + y) * SIDE + ax + x)
                {
                    *b = 255;
                }
            }
        }
    }

    /// The first free spot in row-major order where a `w` x `h` rectangle of the layer fits.
    /// The rows are searched in parallel and the first row that has one wins, which is the
    /// C++'s answer.
    fn first_fit(&self, w: usize, h: usize, lm: &LmLayer, rotated: bool) -> Option<(usize, usize)> {
        let (x_max, y_max) = ((SIDE + 1).checked_sub(w)?, (SIDE + 1).checked_sub(h)?);
        (0..y_max).into_par_iter().find_map_first(|y| {
            (0..x_max).find(|&x| self.at(x, y) == 0 && self.fits(x, y, lm, rotated)).map(|x| (x, y))
        })
    }

    /// `_rect_place`: the deflector's layer placed upright where it first fits, else turned a
    /// quarter; `None` when the page has no room for it.
    pub fn place(&mut self, lm: &LmLayer) -> Option<(Rect, bool)> {
        let w = lm.width as usize + 2 * BORDER as usize;
        let h = lm.height as usize + 2 * BORDER as usize;
        for (rotated, (rw, rh)) in [(false, (w, h)), (true, (h, w))] {
            if let Some((x, y)) = self.first_fit(rw, rh, lm, rotated) {
                self.register(x, y, lm, rotated);
                let r = Rect { ax: x as u32, ay: y as u32, bx: (x + rw - 1) as u32, by: (y + rh - 1) as u32 };
                return Some((r, rotated));
            }
        }
        None
    }
}

/// `sort_defl_analyze` as an order: the larger largest-deflector of the material first, then
/// the higher sector, the higher material index, the larger deflector.
fn better(a: (u32, u16, u16, u32), b: (u32, u16, u16, u32)) -> std::cmp::Ordering {
    b.cmp(&a)
}

/// `CBuild::xrPhase_MergeLM`: every deflector not merged yet placed into pages, each page
/// captured and saved in turn; `lightmaps` gets the pages, the pictures are answered in order.
/// The deflectors are destroyed afterwards, as the C++ does. The C++ takes ties of the sort in
/// the order its shuffled list had them; here in the list's own order.
pub fn merge_lightmaps(
    defls: &mut Deflectors,
    mesh: &mut Mesh,
    materials: &[BuildMaterial],
    lightmaps: &mut Vec<Lightmap>,
    ctl: &Ctl<'_>,
) -> Result<Vec<LightmapPage>> {
    let mut layer: Vec<DeflId> =
        defls.list().iter().copied().filter(|&d| defls.get(d).is_some_and(|x| !x.merged)).collect();
    let mut pages = Vec::new();
    while !layer.is_empty() {
        ctl.check()?;
        ctl.stage(&format!("Building lightmap {}...", lightmaps.len()));

        // the largest deflector of every material
        let mut max_area = vec![0u32; materials.len()];
        let key_of = |d: DeflId, defls: &Deflectors| {
            defls.get(d).map(|x| (x.base_material(mesh), x.layer.area())).unwrap_or((0, 0))
        };
        for &d in &layer {
            let (m, a) = key_of(d, defls);
            if let Some(slot) = max_area.get_mut(usize::from(m)) {
                *slot = (*slot).max(a);
            }
        }
        let sort_key = |d: DeflId| {
            let (m, a) = key_of(d, defls);
            let mat_area = max_area.get(usize::from(m)).copied().unwrap_or(0);
            let sector = materials.get(usize::from(m)).map_or(0, |x| x.sector);
            (mat_area, sector, m, a)
        };
        layer.sort_by(|&a, &b| better(sort_key(a), sort_key(b)));

        // as many as eight pages could hold
        let max = LMAP_SIZE * LMAP_SIZE * 8;
        let mut cur = 0u32;
        let mut count = 0usize;
        for &d in &layer {
            let a = key_of(d, defls).1;
            if cur + a > max {
                break;
            }
            cur += a;
            count += 1;
        }

        let mut mask = PageMask::default();
        let page = lightmaps.len() as u32;
        let mut lmap = Lightmap::default();
        let mut placed = 0usize;
        for (i, &d) in layer.iter().take(count).enumerate() {
            let Some(defl) = defls.get(d) else { continue };
            if let Some((r, rotated)) = mask.place(&defl.layer) {
                lmap.capture(page, defl, mesh, r.ax, r.ay, r.size_x(), r.size_y(), rotated);
                if let Some(defl) = defls.get_mut(d) {
                    defl.merged = true;
                }
                placed += 1;
            }
            ctl.progress(i + 1, count);
        }
        if placed == 0 {
            // an empty page holds any layer `OA_Export` makes; one made otherwise may not fit
            let big =
                layer.first().and_then(|&d| defls.get(d)).map_or((0, 0), |x| (x.layer.width, x.layer.height));
            return Err(LcError::Invalid(format!(
                "A deflector of {} x {} texels does not fit a lightmap page.",
                big.0, big.1
            )));
        }

        // the merged leave the layer, and their mark goes with them (`pred_remove`)
        layer.retain(|&d| match defls.get_mut(d) {
            Some(x) if x.merged => {
                x.merged = false;
                false
            }
            Some(_) => true,
            None => false,
        });
        pages.push(lmap.save(page + 1));
        lightmaps.push(lmap);
    }
    defls.clear();
    Ok(pages)
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

    #[test]
    fn deflectors_are_placed_without_overlap() {
        let mut mask = PageMask::default();
        let layers = [lit(100, 50), lit(30, 300), lit(510, 510), lit(1, 1), lit(510, 200), lit(0, 0)];
        let mut placed: Vec<Rect> = Vec::new();
        for l in &layers {
            let (r, _) = mask.place(l).expect("an empty page holds these");
            assert!(r.bx < LMAP_SIZE && r.by < LMAP_SIZE);
            for o in &placed {
                let apart = r.bx < o.ax || o.bx < r.ax || r.by < o.ay || o.by < r.ay;
                assert!(apart, "{r:?} overlaps {o:?}");
            }
            placed.push(r);
        }
        // the first goes to the corner, the second right of it
        assert_eq!((placed[0].ax, placed[0].ay), (0, 0));
        assert_eq!((placed[1].ax, placed[1].ay), (102, 0));
        // the mask counts every texel once
        let taken = mask.bits.iter().filter(|&&b| b != 0).count();
        let want: u32 = layers.iter().map(LmLayer::area).sum();
        assert_eq!(taken as u32, want);
    }

    /// oracle: a texel marked exactly `alpha_ref` is free in the C++'s eight-at-a-time columns
    /// of an upright layer and taken in the rest (`Place_Perpixel`).
    #[test]
    fn a_texel_marked_alpha_ref_is_free_only_in_the_wide_columns() {
        // a layer of 20 texels to a row with the border: columns 0..16 are tested eight at a time
        let mut probe = lit(18, 2);
        let row = probe.width as usize + 2;
        probe.marker.iter_mut().for_each(|m| *m = 0);
        let mut mask = PageMask::default();
        // what is taken: the page's texel (3, 0)
        mask.bits[3] = 255;
        if let Some(m) = probe.marker.get_mut(3) {
            *m = ALPHA_REF;
        }
        assert!(mask.fits(0, 0, &probe, false), "column 3 is a wide one: alpha_ref is free there");
        // turned, the layer's texel (3, 0) lands on the page's (0, 3)
        let mut turned = PageMask::default();
        turned.bits[3 * SIDE] = 255;
        assert!(!turned.fits(0, 0, &probe, true), "a turned layer is tested one by one");
        let mut mask = PageMask::default();
        mask.bits[17] = 255;
        probe.marker.iter_mut().for_each(|m| *m = 0);
        if let Some(m) = probe.marker.get_mut(17) {
            *m = ALPHA_REF;
        }
        assert!(row == 20 && !mask.fits(0, 0, &probe, false), "column 17 is past the wide ones");
    }

    #[test]
    fn a_layer_too_wide_for_what_is_left_is_turned() {
        let mut mask = PageMask::default();
        // a band 1000 wide and 600 high leaves a strip 1024 x 424 at the bottom
        mask.place(&lit(998, 598)).unwrap();
        let (r, rotated) = mask.place(&lit(10, 500)).unwrap();
        // upright it would need 502 rows: only the right strip of 24 columns has them
        assert!(!rotated);
        assert_eq!((r.ax, r.ay), (1000, 0));
        let (_, rotated) = mask.place(&lit(500, 30)).unwrap();
        assert!(!rotated);
        // 30 x 700 fits nowhere upright, turned it fits the bottom strip
        let (r, rotated) = mask.place(&lit(28, 698)).unwrap();
        assert!(rotated);
        assert_eq!((r.size_x(), r.size_y()), (700, 30));
    }
}

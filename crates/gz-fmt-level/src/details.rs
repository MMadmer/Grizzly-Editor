//! Detail objects (grass): the game's `level.details` / the editor's `build.details`, the Detail
//! Objects tool's part of a scene, and the `.dti` list of objects by colour.

use std::sync::Arc;

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::bytes::{aabb_at, f32_at, fits, u16_at, u32_at, vec3_at};
use crate::math::{i_ceil, i_floor};
use crate::{Aabb, LevelError, Result};

/// `DETAIL_VERSION`: the version of a built details file.
pub const DETAIL_VERSION: u32 = 3;
/// The side of a slot in metres.
pub const DETAIL_SLOT_SIZE: f32 = 2.0;
/// The version of the tool's scene data.
pub const DETMGR_VERSION: u32 = 3;
/// The version of an object reference in scene data and `.dti` files.
pub const DETOBJ_VERSION: u32 = 1;
/// `DO_NO_WAVING`: a model flag.
pub const DO_NO_WAVING: u32 = 0x0001;

const CHUNK_HEADER: u32 = 0x0000;
const CHUNK_OBJECTS: u32 = 0x0001;
const CHUNK_SLOTS: u32 = 0x0002;
const CHUNK_VERSION: u32 = 0x1000;
const CHUNK_BBOX: u32 = 0x1001;
const CHUNK_BASE_TEXTURE: u32 = 0x1002;
const CHUNK_COLOR_INDEX: u32 = 0x1003;
const CHUNK_SNAP_OBJECTS: u32 = 0x1004;
const CHUNK_DENSITY: u32 = 0x1005;
const CHUNK_FLAGS: u32 = 0x1006;

const DETOBJ_CHUNK_VERSION: u32 = 0x1000;
const DETOBJ_CHUNK_REFERENCE: u32 = 0x0101;
const DETOBJ_CHUNK_SCALE_LIMITS: u32 = 0x0102;
const DETOBJ_CHUNK_DENSITY_FACTOR: u32 = 0x0103;
const DETOBJ_CHUNK_FLAGS: u32 = 0x0104;

const SLOT: usize = 16;
const HEADER: usize = 24;
const VERTEX: usize = 20;

/// `DetailHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailHeader {
    pub version: u32,
    pub object_count: u32,
    pub offs_x: i32,
    pub offs_z: i32,
    pub size_x: u32,
    pub size_z: u32,
}

impl DetailHeader {
    fn read(b: &[u8]) -> Self {
        Self {
            version: u32_at(b, 0),
            object_count: u32_at(b, 4),
            offs_x: u32_at(b, 8) as i32,
            offs_z: u32_at(b, 12) as i32,
            size_x: u32_at(b, 16),
            size_z: u32_at(b, 20),
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_u32(self.version);
        w.w_u32(self.object_count);
        w.w_i32(self.offs_x);
        w.w_i32(self.offs_z);
        w.w_u32(self.size_x);
        w.w_u32(self.size_z);
    }

    /// Slots of the grid.
    #[must_use]
    pub fn slot_count(&self) -> u64 {
        u64::from(self.size_x) * u64::from(self.size_z)
    }

    /// The world x of the centre of a slot column.
    #[must_use]
    pub fn from_slot_x(&self, x: i32) -> f32 {
        (x.wrapping_sub(self.offs_x)) as f32 * DETAIL_SLOT_SIZE + DETAIL_SLOT_SIZE * 0.5
    }

    /// The world z of the centre of a slot row.
    #[must_use]
    pub fn from_slot_z(&self, z: i32) -> f32 {
        (z.wrapping_sub(self.offs_z)) as f32 * DETAIL_SLOT_SIZE + DETAIL_SLOT_SIZE * 0.5
    }

    /// The index of slot `(x, z)`: `z * size_x + x`.
    #[must_use]
    pub fn slot_index(&self, x: u32, z: u32) -> Option<usize> {
        (x < self.size_x && z < self.size_z).then(|| z as usize * self.size_x as usize + x as usize)
    }
}

/// `DetailSlot`: 16 bytes of bit fields.
///
/// ```text
/// word 0: y_base:12  y_height:8  id0:6  id1:6
/// word 1: id2:6  id3:6  c_dir:4  c_hemi:4  c_r:4  c_g:4  c_b:4
/// palette[4]: four 4-bit densities each, one palette per object id
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailSlot {
    pub word0: u32,
    pub word1: u32,
    pub palette: [u16; 4],
}

impl DetailSlot {
    /// An object id that means "no object".
    pub const ID_EMPTY: u8 = 0x3f;

    fn read(b: &[u8]) -> Self {
        Self {
            word0: u32_at(b, 0),
            word1: u32_at(b, 4),
            palette: [u16_at(b, 8), u16_at(b, 10), u16_at(b, 12), u16_at(b, 14)],
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_u32(self.word0);
        w.w_u32(self.word1);
        for p in self.palette {
            w.w_u16(p);
        }
    }

    fn field(word: u32, shift: u32, bits: u32) -> u32 {
        (word >> shift) & ((1 << bits) - 1)
    }

    fn set_field(word: &mut u32, shift: u32, bits: u32, value: u32) {
        let mask = ((1u32 << bits) - 1) << shift;
        *word = (*word & !mask) | ((value << shift) & mask);
    }

    /// The base height in steps of 20 cm above -200 m.
    #[must_use]
    pub fn y_base(&self) -> u32 {
        Self::field(self.word0, 0, 12)
    }

    /// The height of the slot in steps of 10 cm.
    #[must_use]
    pub fn y_height(&self) -> u32 {
        Self::field(self.word0, 12, 8)
    }

    /// The object of part `part` (0..4); [`Self::ID_EMPTY`] for none.
    #[must_use]
    pub fn id(&self, part: usize) -> u8 {
        (match part {
            0 => Self::field(self.word0, 20, 6),
            1 => Self::field(self.word0, 26, 6),
            2 => Self::field(self.word1, 0, 6),
            3 => Self::field(self.word1, 6, 6),
            _ => u32::from(Self::ID_EMPTY),
        }) as u8
    }

    pub fn set_id(&mut self, part: usize, id: u8) {
        let v = u32::from(id);
        match part {
            0 => Self::set_field(&mut self.word0, 20, 6, v),
            1 => Self::set_field(&mut self.word0, 26, 6, v),
            2 => Self::set_field(&mut self.word1, 0, 6, v),
            3 => Self::set_field(&mut self.word1, 6, 6, v),
            _ => {}
        }
    }

    /// The lighting of the slot: sun, hemisphere, r, g, b - 4 bits each.
    #[must_use]
    pub fn colors(&self) -> [u8; 5] {
        [12, 16, 20, 24, 28].map(|shift| Self::field(self.word1, shift, 4) as u8)
    }

    pub fn set_colors(&mut self, colors: [u8; 5]) {
        for (shift, c) in [12, 16, 20, 24, 28].into_iter().zip(colors) {
            Self::set_field(&mut self.word1, shift, 4, u32::from(c));
        }
    }

    /// The density (0..=15) of object part `part` in corner `corner` of the slot.
    #[must_use]
    pub fn density(&self, part: usize, corner: usize) -> u8 {
        let word = self.palette.get(part).copied().unwrap_or(0);
        ((word >> ((corner & 3) * 4)) & 0xf) as u8
    }

    pub fn set_density(&mut self, part: usize, corner: usize, value: u8) {
        if let Some(word) = self.palette.get_mut(part) {
            let shift = (corner & 3) * 4;
            *word = (*word & !(0xf << shift)) | (u16::from(value & 0xf) << shift);
        }
    }

    /// `r_ybase`: the base height in metres.
    #[must_use]
    pub fn base_height(&self) -> f32 {
        self.y_base() as f32 * 0.2 - 200.0
    }

    /// `r_yheight`: the height of the slot in metres.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.y_height() as f32 * 0.1
    }

    /// `w_y`: stores a base height and a height, the height stretched by what the base lost.
    pub fn set_heights(&mut self, base: f32, height: f32) {
        let b = i_floor((base + 200.0) / 0.2).clamp(0, 4095);
        Self::set_field(&mut self.word0, 0, 12, b as u32);
        let error = base - self.base_height();
        let h = i_ceil((height + error) / 0.1).clamp(0, 255);
        Self::set_field(&mut self.word0, 12, 8, h as u32);
    }

    /// `w_qclr`: a `0..=1` value in `range` steps, the way slot colours are quantised.
    #[must_use]
    pub fn quantize(value: f32, range: u32) -> u32 {
        i_floor(value * range as f32).clamp(0, range as i32) as u32
    }

    /// `color_editor`: the lighting a slot has while it is edited (half sun, half sky, no rgb).
    pub fn set_editor_colors(&mut self) {
        let half = Self::quantize(0.5, 15) as u8;
        self.set_colors([half, half, 0, 0, 0]);
    }

    /// `is_empty`: no part names an object.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        (0..4).all(|p| self.id(p) == Self::ID_EMPTY)
    }

    /// The slot as the level build writes it: lighting cleared for the compiler to fill, object
    /// ids replaced through `remap` (index = scene object, value = exported object).
    #[must_use]
    pub fn for_export(&self, remap: &[u8]) -> Self {
        let mut out = *self;
        out.set_colors([0; 5]);
        for part in 0..4 {
            let id = out.id(part);
            if id != Self::ID_EMPTY {
                out.set_id(part, remap.get(id as usize).copied().unwrap_or(u8::MAX));
            }
        }
        out
    }
}

fn read_slots(bytes: &[u8]) -> Vec<DetailSlot> {
    bytes.as_chunks::<SLOT>().0.iter().map(|c| DetailSlot::read(c)).collect()
}

/// A vertex of a detail model: position and texture coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DetailVertex {
    pub pos: Vec3,
    pub u: f32,
    pub v: f32,
}

/// A detail model of a built file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailModel {
    pub shader: SmolStr,
    pub texture: SmolStr,
    /// [`DO_NO_WAVING`].
    pub flags: u32,
    pub min_scale: f32,
    pub max_scale: f32,
    pub verts: Vec<DetailVertex>,
    pub indices: Vec<u16>,
}

impl DetailModel {
    fn read(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let shader = SmolStr::new(r.r_stringz()?);
        let texture = SmolStr::new(r.r_stringz()?);
        let flags = r.r_u32()?;
        let min_scale = r.r_f32()?;
        let max_scale = r.r_f32()?;
        let vert_count = r.r_u32()?;
        let index_count = r.r_u32()?;
        let nv = fits(r.elapsed(), u64::from(vert_count), VERTEX as u64, "detail model vertices")?;
        let verts = r
            .take(nv * VERTEX)?
            .as_chunks::<VERTEX>()
            .0
            .iter()
            .map(|c| DetailVertex { pos: vec3_at(c, 0), u: f32_at(c, 12), v: f32_at(c, 16) })
            .collect();
        let ni = fits(r.elapsed(), u64::from(index_count), 2, "detail model indices")?;
        let indices = r.take(ni * 2)?.as_chunks::<2>().0.iter().map(|c| u16_at(c, 0)).collect();
        Ok(Self { shader, texture, flags, min_scale, max_scale, verts, indices })
    }

    fn write(&self, w: &mut Writer) {
        w.w_stringz(&self.shader);
        w.w_stringz(&self.texture);
        w.w_u32(self.flags);
        w.w_f32(self.min_scale);
        w.w_f32(self.max_scale);
        w.w_u32(self.verts.len() as u32);
        w.w_u32(self.indices.len() as u32);
        for v in &self.verts {
            w.w_vec3(v.pos);
            w.w_f32(v.u);
            w.w_f32(v.v);
        }
        for i in &self.indices {
            w.w_u16(*i);
        }
    }
}

/// A built details file: `level.details`, `build.details`.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelDetails {
    pub header: DetailHeader,
    pub models: Vec<DetailModel>,
    pub slots: Arc<[DetailSlot]>,
    /// The order of the three chunks in the file. The editor's build writes objects, slots,
    /// header ([`Self::BUILD_ORDER`]); the compilers write header, objects, slots.
    pub chunk_order: [u32; 3],
}

impl LevelDetails {
    /// The chunk order of `build.details`: objects, slots, header.
    pub const BUILD_ORDER: [u32; 3] = [CHUNK_OBJECTS, CHUNK_SLOTS, CHUNK_HEADER];

    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let head = r.need_chunk(CHUNK_HEADER)?;
        if head.bytes().len() < HEADER {
            return Err(LevelError::Invalid("details: the header chunk is too short".into()));
        }
        let header = DetailHeader::read(head.bytes());

        let mut models = Vec::new();
        if let Some(objects) = r.open_chunk(CHUNK_OBJECTS)? {
            let objects = objects.reader();
            // objects are numbered from 0 without gaps; the engine stops at the first gap
            let mut id = 0u32;
            while let Some(chunk) = objects.open_chunk(id)? {
                models.push(DetailModel::read(chunk.bytes())?);
                id += 1;
            }
        }
        let slots = r.need_chunk(CHUNK_SLOTS)?;
        if slots.bytes().len() % SLOT != 0 {
            return Err(LevelError::Invalid("details: the slot chunk is not a whole number of slots".into()));
        }
        let slots = read_slots(slots.bytes());

        let mut chunk_order = [u32::MAX; 3];
        let mut seen = 0;
        for (id, _, _) in r.chunks() {
            if matches!(id, CHUNK_HEADER | CHUNK_OBJECTS | CHUNK_SLOTS)
                && !chunk_order[..seen].contains(&id)
                && let Some(slot) = chunk_order.get_mut(seen)
            {
                *slot = id;
                seen += 1;
            }
        }
        // a file without an objects chunk still gets a full order
        for id in Self::BUILD_ORDER {
            if !chunk_order.contains(&id)
                && let Some(slot) = chunk_order.iter_mut().find(|s| **s == u32::MAX)
            {
                *slot = id;
            }
        }
        Ok(Self { header, models, slots: slots.into(), chunk_order })
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(64 + self.slots.len() * SLOT);
        for id in self.chunk_order {
            match id {
                CHUNK_HEADER => w.chunk(CHUNK_HEADER, |w| self.header.write(w)),
                CHUNK_OBJECTS => w.chunk(CHUNK_OBJECTS, |w| {
                    for (i, model) in self.models.iter().enumerate() {
                        w.chunk(i as u32, |w| model.write(w));
                    }
                }),
                CHUNK_SLOTS => w.chunk(CHUNK_SLOTS, |w| {
                    for slot in self.slots.iter() {
                        slot.write(w);
                    }
                }),
                _ => {}
            }
        }
        w.into_bytes()
    }
}

/// An object of the tool: a reference to a library object and how it is scattered.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailObjectRef {
    /// The library object, `trees\grass\grass_01`.
    pub reference: SmolStr,
    pub min_scale: f32,
    pub max_scale: f32,
    pub density_factor: f32,
    /// [`DO_NO_WAVING`].
    pub flags: u32,
}

impl Default for DetailObjectRef {
    fn default() -> Self {
        Self { reference: SmolStr::default(), min_scale: 0.5, max_scale: 2.0, density_factor: 1.0, flags: 0 }
    }
}

impl DetailObjectRef {
    fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = r.need_chunk(DETOBJ_CHUNK_VERSION)?.reader().r_u32()?;
        if version != DETOBJ_VERSION {
            return Err(LevelError::Version { what: "detail object", found: version });
        }
        let reference = SmolStr::new(r.need_chunk(DETOBJ_CHUNK_REFERENCE)?.reader().r_stringz()?);
        let scale = r.need_chunk(DETOBJ_CHUNK_SCALE_LIMITS)?;
        let mut scale = scale.reader();
        let mut out =
            Self { reference, min_scale: scale.r_f32()?, max_scale: scale.r_f32()?, ..Self::default() };
        if let Some(c) = r.open_chunk(DETOBJ_CHUNK_DENSITY_FACTOR)? {
            out.density_factor = c.reader().r_f32()?;
        }
        if let Some(c) = r.open_chunk(DETOBJ_CHUNK_FLAGS)? {
            out.flags = c.reader().r_u32()?;
        }
        Ok(out)
    }

    fn write(&self, w: &mut Writer) {
        w.chunk(DETOBJ_CHUNK_VERSION, |w| w.w_u32(DETOBJ_VERSION));
        w.chunk(DETOBJ_CHUNK_REFERENCE, |w| w.w_stringz(&self.reference));
        w.chunk(DETOBJ_CHUNK_SCALE_LIMITS, |w| {
            w.w_f32(self.min_scale);
            w.w_f32(self.max_scale);
        });
        w.chunk(DETOBJ_CHUNK_DENSITY_FACTOR, |w| w.w_f32(self.density_factor));
        w.chunk(DETOBJ_CHUNK_FLAGS, |w| w.w_u32(self.flags));
    }

    /// The scale limits the tool settles on after a load: a zero minimum becomes 0.1 and the
    /// maximum is never below the minimum.
    #[must_use]
    pub fn loaded_scales(&self) -> (f32, f32) {
        let min = if self.min_scale.abs() < crate::math::EPS_S { 0.1 } else { self.min_scale };
        (min, if self.max_scale < min { min } else { self.max_scale })
    }
}

/// The objects a colour of the base texture scatters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorIndex {
    /// The colour, `0xAARRGGBB`.
    pub color: u32,
    /// Names of [`DetailObjectRef`]s.
    pub objects: Vec<SmolStr>,
}

/// The object list of the tool: what a `.dti` file holds, and a part of the scene data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailColorIndices {
    pub objects: Vec<DetailObjectRef>,
    /// At most 255 colours of at most 255 objects: the counts are bytes.
    pub colors: Vec<ColorIndex>,
}

impl DetailColorIndices {
    /// Reads a `.dti` file (or the same two chunks out of scene data).
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_from(&Reader::new(bytes))
    }

    fn read_from(r: &Reader<'_>) -> Result<Self> {
        let mut out = Self::default();
        if let Some(objects) = r.open_chunk(CHUNK_OBJECTS)? {
            let objects = objects.reader();
            let mut id = 0u32;
            while let Some(chunk) = objects.open_chunk(id)? {
                out.objects.push(DetailObjectRef::read(chunk.bytes())?);
                id += 1;
            }
        }
        let colors = r.need_chunk(CHUNK_COLOR_INDEX)?;
        let mut c = colors.reader();
        let count = c.r_u8()?;
        for _ in 0..count {
            let color = c.r_u32()?;
            let refs = c.r_u8()?;
            let mut objects = Vec::with_capacity(usize::from(refs));
            for _ in 0..refs {
                objects.push(SmolStr::new(c.r_stringz()?));
            }
            out.colors.push(ColorIndex { color, objects });
        }
        Ok(out)
    }

    /// A `.dti` file.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write_to(&mut w);
        w.into_bytes()
    }

    fn write_to(&self, w: &mut Writer) {
        w.chunk(CHUNK_OBJECTS, |w| {
            for (i, object) in self.objects.iter().enumerate() {
                w.chunk(i as u32, |w| object.write(w));
            }
        });
        w.chunk(CHUNK_COLOR_INDEX, |w| {
            w.w_u8(self.colors.len() as u8);
            for entry in &self.colors {
                w.w_u32(entry.color);
                w.w_u8(entry.objects.len() as u8);
                for name in &entry.objects {
                    w.w_stringz(name);
                }
            }
        });
    }
}

/// Flags of the tool.
pub mod tool_flags {
    pub const BASE_TEXTURE_DRAW: u32 = 1 << 31;
    pub const BASE_TEXTURE_BLENDED: u32 = 1 << 30;
    pub const SLOT_BOXES_DRAW: u32 = 1 << 29;
    pub const OBJECTS_DRAW: u32 = 1 << 28;
}

/// The Detail Objects tool's part of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDetails {
    /// [`tool_flags`]. A scene without the chunk keeps the tool's default, objects drawn.
    pub flags: u32,
    pub header: DetailHeader,
    pub indices: DetailColorIndices,
    pub slots: Arc<[DetailSlot]>,
    pub bbox: Aabb,
    /// The base texture the colours are read from, when one is set.
    pub base_texture: Option<SmolStr>,
    /// Objects per square metre; `None` when the scene does not say (the editor's setting stays).
    pub density: Option<f32>,
    pub snap_objects: Vec<SmolStr>,
}

impl Default for SceneDetails {
    fn default() -> Self {
        Self {
            flags: tool_flags::OBJECTS_DRAW,
            header: DetailHeader::default(),
            indices: DetailColorIndices::default(),
            slots: Arc::from([]),
            bbox: Aabb::INVALID,
            base_texture: None,
            density: None,
            snap_objects: Vec::new(),
        }
    }
}

impl SceneDetails {
    /// Reads the tool's chunk of a scene.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = r.need_chunk(CHUNK_VERSION)?.reader().r_u32()?;
        if version != DETMGR_VERSION {
            return Err(LevelError::Version { what: "scene details", found: version });
        }
        let mut out = Self::default();
        if let Some(c) = r.open_chunk(CHUNK_FLAGS)? {
            out.flags = c.reader().r_u32()?;
        }
        let head = r.need_chunk(CHUNK_HEADER)?;
        if head.bytes().len() < HEADER {
            return Err(LevelError::Invalid("details: the header chunk is too short".into()));
        }
        out.header = DetailHeader::read(head.bytes());

        let slots = r.need_chunk(CHUNK_SLOTS)?;
        let mut s = slots.reader();
        let count = s.r_u32()?;
        let n = fits(s.elapsed(), u64::from(count), SLOT as u64, "detail slots")?;
        out.slots = read_slots(s.take(n * SLOT)?).into();

        out.indices = DetailColorIndices::read_from(&r)?;

        let bbox = r.need_chunk(CHUNK_BBOX)?;
        if bbox.bytes().len() < 24 {
            return Err(LevelError::Invalid("details: the box chunk is too short".into()));
        }
        out.bbox = aabb_at(bbox.bytes(), 0);

        if let Some(c) = r.open_chunk(CHUNK_SNAP_OBJECTS)? {
            let mut c = c.reader();
            let count = c.r_u32()?;
            for _ in 0..count {
                out.snap_objects.push(SmolStr::new(c.r_stringz()?));
            }
        }
        if let Some(c) = r.open_chunk(CHUNK_DENSITY)? {
            out.density = Some(c.reader().r_f32()?);
        }
        if let Some(c) = r.open_chunk(CHUNK_BASE_TEXTURE)? {
            out.base_texture = Some(SmolStr::new(c.reader().r_stringz()?));
        }
        Ok(out)
    }

    /// The tool's chunk of a scene. The slot count written is the header's
    /// `size_x * size_z`, as the X-Ray SDK writes it; the slots themselves are written as they are.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(256 + self.slots.len() * SLOT);
        w.chunk(CHUNK_VERSION, |w| w.w_u32(DETMGR_VERSION));
        w.chunk(CHUNK_FLAGS, |w| w.w_u32(self.flags));
        w.chunk(CHUNK_HEADER, |w| self.header.write(w));
        self.indices.write_to(&mut w);
        w.chunk(CHUNK_SLOTS, |w| {
            w.w_u32(self.header.size_x.wrapping_mul(self.header.size_z));
            for slot in self.slots.iter() {
                slot.write(w);
            }
        });
        w.chunk(CHUNK_BBOX, |w| {
            w.w_vec3(self.bbox.min);
            w.w_vec3(self.bbox.max);
        });
        if let Some(texture) = &self.base_texture {
            w.chunk(CHUNK_BASE_TEXTURE, |w| w.w_stringz(texture));
        }
        if let Some(density) = self.density {
            w.chunk(CHUNK_DENSITY, |w| w.w_f32(density));
        }
        w.chunk(CHUNK_SNAP_OBJECTS, |w| {
            w.w_u32(self.snap_objects.len() as u32);
            for name in &self.snap_objects {
                w.w_stringz(name);
            }
        });
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(seed: u32) -> DetailSlot {
        let mut s = DetailSlot::default();
        s.set_heights(seed as f32 * 0.37 - 3.0, 1.25);
        s.set_id(0, (seed % 5) as u8);
        s.set_id(1, DetailSlot::ID_EMPTY);
        s.set_id(2, 7);
        s.set_id(3, DetailSlot::ID_EMPTY);
        s.set_colors([8, 7, 1, 2, 3]);
        s.set_density(0, 0, 15);
        s.set_density(0, 3, 4);
        s.set_density(2, 1, 9);
        s
    }

    #[test]
    fn slot_fields_do_not_overlap() {
        let mut s = DetailSlot::default();
        s.set_id(0, 0x3f);
        assert_eq!(s.word0, 0x3f << 20);
        s.set_id(1, 0x2a);
        assert_eq!(s.word0, (0x2a << 26) | (0x3f << 20));
        s.set_id(2, 1);
        s.set_id(3, 0x3f);
        assert_eq!(s.word1, (0x3f << 6) | 1);
        s.set_colors([0xf, 0, 0xf, 0, 0xf]);
        assert_eq!(s.word1, 0xf0f0_f000 | (0x3f << 6) | 1);
        assert_eq!(s.colors(), [0xf, 0, 0xf, 0, 0xf]);
        assert_eq!([s.id(0), s.id(1), s.id(2), s.id(3)], [0x3f, 0x2a, 1, 0x3f]);
        s.set_density(1, 2, 0xc);
        assert_eq!(s.palette, [0, 0x0c00, 0, 0]);
        assert_eq!(s.density(1, 2), 0xc);

        // 12.3 m: base 1061 steps of 20 cm above -200 m, height rounded up in steps of 10 cm
        s.set_heights(12.3, 0.95);
        assert_eq!(s.y_base(), 1061);
        assert!((s.base_height() - 12.2).abs() < 1e-3);
        assert!((10..=11).contains(&s.y_height()), "{}", s.y_height());
        assert!(s.base_height() + s.height() >= 12.3 + 0.95 - 1e-3);
        assert_eq!(DetailSlot::quantize(0.5, 15), 7);
        assert_eq!(DetailSlot::quantize(2.0, 15), 15);
    }

    #[test]
    fn a_slot_is_prepared_for_the_build() {
        let s = slot(3);
        let out = s.for_export(&[9, 9, 9, 4, 9, 9, 9, 2]);
        assert_eq!([out.id(0), out.id(1), out.id(2), out.id(3)], [4, 0x3f, 2, 0x3f]);
        assert_eq!(out.colors(), [0; 5]);
        assert_eq!(out.palette, s.palette);
        assert_eq!(out.y_base(), s.y_base());
        assert!(!out.is_empty());
    }

    fn built() -> LevelDetails {
        let model = DetailModel {
            shader: "details\\set".into(),
            texture: "build_details".into(),
            flags: DO_NO_WAVING,
            min_scale: 0.5,
            max_scale: 1.5,
            verts: vec![
                DetailVertex { pos: Vec3::ZERO, u: 0.0, v: 1.0 },
                DetailVertex { pos: Vec3::X, u: 1.0, v: 1.0 },
                DetailVertex { pos: Vec3::Y, u: 0.5, v: 0.0 },
            ],
            indices: vec![0, 1, 2],
        };
        LevelDetails {
            header: DetailHeader { version: 3, object_count: 2, offs_x: 4, offs_z: -2, size_x: 2, size_z: 3 },
            models: vec![model.clone(), DetailModel { texture: "other".into(), ..model }],
            slots: (0..6).map(slot).collect(),
            chunk_order: LevelDetails::BUILD_ORDER,
        }
    }

    #[test]
    fn a_built_file_round_trips_in_either_chunk_order() {
        let file = built();
        let bytes = file.write();
        let ids: Vec<u32> = Reader::new(&bytes).chunks().map(|c| c.0).collect();
        assert_eq!(ids, [1, 2, 0]);
        let back = LevelDetails::read(&bytes).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.write(), bytes);

        let compiled = LevelDetails { chunk_order: [0, 1, 2], ..file };
        let bytes = compiled.write();
        let back = LevelDetails::read(&bytes).unwrap();
        assert_eq!(back.chunk_order, [0, 1, 2]);
        assert_eq!(back.write(), bytes);
        assert_eq!(back.header.slot_index(1, 2), Some(5));
        assert_eq!(back.header.slot_index(2, 0), None);
        assert_eq!(back.header.from_slot_x(4), 1.0);
        assert_eq!(back.header.from_slot_z(0), 5.0);
    }

    fn scene() -> SceneDetails {
        SceneDetails {
            flags: tool_flags::OBJECTS_DRAW | tool_flags::BASE_TEXTURE_DRAW,
            header: DetailHeader { version: 0, object_count: 0, offs_x: 1, offs_z: 1, size_x: 3, size_z: 2 },
            indices: DetailColorIndices {
                objects: vec![
                    DetailObjectRef {
                        reference: "trees\\grass\\grass_01".into(),
                        ..DetailObjectRef::default()
                    },
                    DetailObjectRef {
                        reference: "trees\\grass\\bush".into(),
                        min_scale: 0.0,
                        max_scale: 0.05,
                        density_factor: 0.3,
                        flags: DO_NO_WAVING,
                    },
                ],
                colors: vec![
                    ColorIndex {
                        color: 0xff00_ff00,
                        objects: vec!["trees\\grass\\grass_01".into(), "trees\\grass\\bush".into()],
                    },
                    ColorIndex { color: 0xff80_8080, objects: vec![] },
                ],
            },
            slots: (0..6).map(slot).collect(),
            bbox: Aabb::new(Vec3::new(-2.0, 0.0, -2.0), Vec3::new(4.0, 3.0, 2.0)),
            base_texture: Some("terrain\\terrain_base".into()),
            density: Some(0.6),
            snap_objects: vec!["terrain".into()],
        }
    }

    #[test]
    fn the_scene_chunk_round_trips() {
        let data = scene();
        let bytes = data.write();
        let ids: Vec<u32> = Reader::new(&bytes).chunks().map(|c| c.0).collect();
        assert_eq!(ids, [0x1000, 0x1006, 0, 1, 0x1003, 2, 0x1001, 0x1002, 0x1005, 0x1004]);
        let back = SceneDetails::read(&bytes).unwrap();
        assert_eq!(back, data);
        assert_eq!(back.write(), bytes);
        assert_eq!(back.indices.objects[1].loaded_scales(), (0.1, 0.1));
        assert_eq!(back.indices.objects[0].loaded_scales(), (0.5, 2.0));

        // without a base texture the chunk is not written
        let plain = SceneDetails { base_texture: None, ..data };
        let bytes = plain.write();
        assert!(Reader::new(&bytes).find_chunk(0x1002).is_none());
        assert_eq!(SceneDetails::read(&bytes).unwrap(), plain);
    }

    #[test]
    fn a_dti_file_round_trips() {
        let list = scene().indices;
        let bytes = list.write();
        let ids: Vec<u32> = Reader::new(&bytes).chunks().map(|c| c.0).collect();
        assert_eq!(ids, [1, 0x1003]);
        let back = DetailColorIndices::read(&bytes).unwrap();
        assert_eq!(back, list);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = built().write();
        assert!(LevelDetails::read(&bytes[..12]).is_err());
        assert!(LevelDetails::read(&[]).is_err());
        // a model that claims more vertices than its chunk holds
        let mut model = Writer::new();
        model.w_stringz("s");
        model.w_stringz("t");
        model.w_u32(0);
        model.w_f32(1.0);
        model.w_f32(1.0);
        model.w_u32(u32::MAX);
        model.w_u32(3);
        assert!(DetailModel::read(model.bytes()).is_err());

        let scene_bytes = scene().write();
        assert!(SceneDetails::read(&scene_bytes[..40]).is_err());
        let mut version = scene_bytes;
        version[8] = 2;
        assert!(matches!(SceneDetails::read(&version), Err(LevelError::Version { found: 2, .. })));
    }
}

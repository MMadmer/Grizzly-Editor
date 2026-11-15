//! `level`: the compiled level's visuals, as the compiler writes them (`gz-lc`'s `tree.rs`) and
//! the engine reads them. A reader for looking at a build's result, not a loader: the vertex and
//! index buffers stay in `level.geom`; what a visual says about them (container, start, count) is
//! kept.
//!
//! Top-level chunks: 1 header (`u16` version, `u16` quality), 2 shader table, 3 visuals (one chunk
//! per visual, its id the visual's index), 4 portals, 6 lights, 7 glows, 8 sectors (one chunk per
//! sector with 2 = root visual, 1 = portal ids).
//!
//! A level piece (`piece`, the game's `XMS_ARCHITECTURE.md` 8.3) is the same file without portals,
//! lights, glows and sectors, with chunk 32: its root visuals, each with the sector it hangs on
//! (`0xFFFF`: the sector at the visual's centre). [`GeomTables`] reads what a geometry file
//! (`level.geom`, `piece.geom`, their fast twins) holds, and [`CompiledLevel::piece_problems`]
//! checks every id a piece's visuals name against it, as the engine does before it loads one.

use glam::Vec3;
use gz_chunk::Reader;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{Aabb, LevelError, Result};

const LEVEL_HEADER: u32 = 1;
const LEVEL_SHADERS: u32 = 2;
const LEVEL_VISUALS: u32 = 3;
const LEVEL_PORTALS: u32 = 4;
const LEVEL_LIGHTS: u32 = 6;
const LEVEL_GLOWS: u32 = 7;
const LEVEL_SECTORS: u32 = 8;
const PIECE_ROOTS: u32 = 32;
const GEOM_VB: u32 = 9;
const GEOM_IB: u32 = 10;
const GEOM_SWIS: u32 = 11;
/// The compiler's version in the header of every level and piece.
pub const LEVEL_COMPILER_VERSION: u16 = 14;
/// A piece's root without a sector of its own: the engine finds it at the visual's centre.
pub const PIECE_SECTOR_AUTO: u16 = 0xFFFF;

const OGF_HEADER: u32 = 1;
const OGF_SWIDATA: u32 = 6;
const OGF_CHILDREN_L: u32 = 10;
const OGF_LODDEF2: u32 = 11;
const OGF_TREEDEF2: u32 = 12;
const OGF_SWICONTAINER: u32 = 20;
const OGF_GCONTAINER: u32 = 21;
const OGF_FASTPATH: u32 = 22;

/// One portal record: two sector ids, six vertices, the count in use.
const PORTAL_SIZE: usize = 2 + 2 + 6 * 12 + 4;
/// One light record: the controller and the engine's light.
const LIGHT_SIZE: usize = 4 + 4 + 3 * 16 + 2 * 12 + 7 * 4;
/// One glow record: position, size, shader id.
const GLOW_SIZE: usize = 12 + 4 + 2;

/// What kind of visual a record is (`MT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualKind {
    Normal,
    Hierarchy,
    Progressive,
    Lod,
    TreeStatic,
    TreeProgressive,
    /// A type this reader does not know by name.
    Other(u8),
}

impl VisualKind {
    #[must_use]
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Normal,
            1 => Self::Hierarchy,
            2 => Self::Progressive,
            6 => Self::Lod,
            7 => Self::TreeStatic,
            11 => Self::TreeProgressive,
            n => Self::Other(n),
        }
    }

    /// The word an answer uses.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Normal => "normal".into(),
            Self::Hierarchy => "hierarchy".into(),
            Self::Progressive => "progressive".into(),
            Self::Lod => "lod".into(),
            Self::TreeStatic => "model".into(),
            Self::TreeProgressive => "model_progressive".into(),
            Self::Other(n) => format!("type_{n}"),
        }
    }

    /// Whether the visual draws faces itself.
    #[must_use]
    pub fn has_geometry(self) -> bool {
        matches!(self, Self::Normal | Self::Progressive | Self::TreeStatic | Self::TreeProgressive)
    }
}

/// Where a visual's vertices and indices lie in the geometry file's containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub vertex_buffer: u32,
    pub vertex_start: u32,
    pub vertex_count: u32,
    pub index_buffer: u32,
    pub index_start: u32,
    pub index_count: u32,
}

impl Geometry {
    #[must_use]
    pub fn faces(&self) -> u32 {
        self.index_count / 3
    }
}

/// A visual of the compiled level.
#[derive(Clone, Debug, PartialEq)]
pub struct Visual {
    /// The chunk's id: what a parent's child list and a sector's root name.
    pub index: u32,
    pub kind: VisualKind,
    /// An index into [`CompiledLevel::shaders`].
    pub shader: u16,
    pub aabb: Aabb,
    pub center: Vec3,
    pub radius: f32,
    pub geometry: Option<Geometry>,
    /// The fast path's geometry (`level.geomx`), where the visual has one.
    pub fast: Option<Geometry>,
    pub children: Vec<u32>,
    /// A static model's reference: the transform, row after row.
    pub transform: Option<[[f32; 4]; 4]>,
    /// A LOD visual's quads are there.
    pub lod: bool,
    /// The visual carries sliding windows (a progressive mesh).
    pub sliding: bool,
    /// The sliding windows a static model's progressive reference takes from the geometry file.
    pub swi: Option<u32>,
    /// The sector the visual lies under, through the sectors' root visuals.
    pub sector: Option<u16>,
}

/// A portal between two sectors.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledPortal {
    pub front: u16,
    pub back: u16,
    /// The vertices in use.
    pub vertices: Vec<Vec3>,
}

/// A sector: its root visual and the portals it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSector {
    pub index: u32,
    pub root: u32,
    pub portals: Vec<u16>,
}

/// A root visual of a piece and the sector it hangs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieceRoot {
    pub visual: u32,
    /// [`PIECE_SECTOR_AUTO`]: the sector at the visual's centre.
    pub sector: u16,
}

/// A dynamic light of the level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompiledLight {
    pub controller: u32,
    /// 1 point, 2 spot, 3 directional.
    pub kind: u32,
    pub diffuse: [f32; 4],
    pub position: Vec3,
    pub direction: Vec3,
    pub range: f32,
}

/// A glow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompiledGlow {
    pub position: Vec3,
    pub size: f32,
    pub shader: u16,
}

/// What a shader table entry says: `shader/texture[,lightmap pages]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderEntry<'a> {
    pub shader: &'a str,
    /// The first texture, the base one.
    pub texture: &'a str,
    /// The lightmap page (`lmap#N` -> N), when the entry names one.
    pub lightmap: Option<u32>,
}

/// Splits a table entry the way the compiler built it.
#[must_use]
pub fn parse_shader(entry: &str) -> ShaderEntry<'_> {
    let (shader, textures) = entry.split_once('/').unwrap_or((entry, ""));
    let mut parts = textures.split(',');
    let texture = parts.next().unwrap_or("");
    let lightmap = parts
        .filter_map(|t| {
            let tail = t.rsplit(['\\', '/']).next().unwrap_or(t);
            tail.strip_prefix("lmap#")
        })
        .find_map(|n| n.split('_').next().and_then(|n| n.parse().ok()));
    ShaderEntry { shader, texture, lightmap }
}

/// The compiled level.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledLevel {
    pub version: u16,
    pub quality: u16,
    pub shaders: Vec<String>,
    pub visuals: Vec<Visual>,
    pub portals: Vec<CompiledPortal>,
    pub sectors: Vec<CompiledSector>,
    pub lights: Vec<CompiledLight>,
    pub glows: Vec<CompiledGlow>,
    /// A piece's root visuals (chunk 32); a level has none.
    pub roots: Vec<PieceRoot>,
}

fn invalid(text: impl Into<String>) -> LevelError {
    LevelError::Invalid(text.into())
}

/// The chunks of one level, in file order. A header that does not fit, a body that runs past the
/// end or a compressed one (the compiler writes none) is an error: the walk of `gz-chunk` would
/// stop silently there.
fn walk<'a>(data: &'a [u8], what: &str) -> Result<Vec<(u32, &'a [u8])>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for (id, packed, body) in Reader::new(data).chunks() {
        if packed {
            return Err(invalid(format!(
                "The {what} holds a compressed chunk {id}, which a compiled level never does."
            )));
        }
        used += 8 + body.len();
        out.push((id, body));
    }
    if used != data.len() {
        return Err(invalid(format!(
            "The {what} is cut short or damaged: a chunk after byte {used} does not fit in the file's {} bytes.",
            data.len()
        )));
    }
    Ok(out)
}

fn chunk<'a>(list: &[(u32, &'a [u8])], id: u32) -> Option<&'a [u8]> {
    list.iter().find(|c| c.0 == id).map(|c| c.1)
}

fn short<T>(r: gz_chunk::Result<T>, what: &str) -> Result<T> {
    r.map_err(|e| invalid(format!("The {what} is cut short: {e}.")))
}

fn read_geometry(body: &[u8]) -> Result<Geometry> {
    let mut r = Reader::new(body);
    let mut next = || short(r.r_u32(), "visual's geometry container");
    Ok(Geometry {
        vertex_buffer: next()?,
        vertex_start: next()?,
        vertex_count: next()?,
        index_buffer: next()?,
        index_start: next()?,
        index_count: next()?,
    })
}

fn read_visual(index: u32, body: &[u8]) -> Result<Visual> {
    let parts = walk(body, &format!("visual {index}"))?;
    let head = chunk(&parts, OGF_HEADER).ok_or_else(|| invalid(format!("Visual {index} has no header.")))?;
    let mut r = Reader::new(head);
    let what = format!("header of visual {index}");
    let _version = short(r.r_u8(), &what)?;
    let kind = VisualKind::from_code(short(r.r_u8(), &what)?);
    let shader = short(r.r_u16(), &what)?;
    let min = short(r.r_vec3(), &what)?;
    let max = short(r.r_vec3(), &what)?;
    let center = short(r.r_vec3(), &what)?;
    let radius = short(r.r_f32(), &what)?;

    let geometry = chunk(&parts, OGF_GCONTAINER).map(read_geometry).transpose()?;
    let fast = match chunk(&parts, OGF_FASTPATH) {
        Some(body) => {
            let inner = walk(body, &format!("fast path of visual {index}"))?;
            chunk(&inner, OGF_GCONTAINER).map(read_geometry).transpose()?
        }
        None => None,
    };
    let children = match chunk(&parts, OGF_CHILDREN_L) {
        Some(body) => {
            let mut r = Reader::new(body);
            let what = format!("child list of visual {index}");
            let count = short(r.r_u32(), &what)? as usize;
            if count > r.elapsed() / 4 {
                return Err(invalid(format!(
                    "The {what} names {count} children and has room for {}.",
                    r.elapsed() / 4
                )));
            }
            let mut ids = Vec::with_capacity(count);
            for _ in 0..count {
                ids.push(short(r.r_u32(), &what)?);
            }
            ids
        }
        None => Vec::new(),
    };
    let transform = match chunk(&parts, OGF_TREEDEF2) {
        Some(body) => short(Reader::new(body).r_matrix(), &format!("transform of visual {index}"))?.into(),
        None => None,
    };
    Ok(Visual {
        index,
        kind,
        shader,
        aabb: Aabb::new(min, max),
        center,
        radius,
        geometry,
        fast,
        children,
        transform,
        lod: chunk(&parts, OGF_LODDEF2).is_some(),
        sliding: chunk(&parts, OGF_SWIDATA).is_some(),
        swi: match chunk(&parts, OGF_SWICONTAINER) {
            Some(body) => Some(short(Reader::new(body).r_u32(), &format!("windows of visual {index}"))?),
            None => None,
        },
        sector: None,
    })
}

fn read_sectors(body: &[u8]) -> Result<Vec<CompiledSector>> {
    let mut out = Vec::new();
    for (index, sector) in walk(body, "sector table")? {
        let parts = walk(sector, &format!("sector {index}"))?;
        let root = chunk(&parts, 2).ok_or_else(|| invalid(format!("Sector {index} has no root visual.")))?;
        let root = short(Reader::new(root).r_u32(), &format!("root of sector {index}"))?;
        let list = chunk(&parts, 1).unwrap_or_default();
        if !list.len().is_multiple_of(2) {
            return Err(invalid(format!("The portal list of sector {index} ends in half an id.")));
        }
        let portals = list.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
        out.push(CompiledSector { index, root, portals });
    }
    Ok(out)
}

fn read_roots(body: &[u8]) -> Result<Vec<PieceRoot>> {
    let mut r = Reader::new(body);
    let count = short(r.r_u32(), "piece's roots")? as usize;
    if count.saturating_mul(8) != r.elapsed() {
        return Err(invalid(format!(
            "The piece's root chunk names {count} roots and holds {} bytes for them.",
            r.elapsed()
        )));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let visual = short(r.r_u32(), "piece's roots")?;
        let sector = short(r.r_u16(), "piece's roots")?;
        short(r.r_u16(), "piece's roots")?;
        out.push(PieceRoot { visual, sector });
    }
    Ok(out)
}

fn records(body: &[u8], size: usize, what: &str) -> Result<usize> {
    if !body.len().is_multiple_of(size) {
        return Err(invalid(format!(
            "The {what} chunk holds {} bytes, which is no whole number of its {size}-byte records.",
            body.len()
        )));
    }
    Ok(body.len() / size)
}

fn read_portals(body: &[u8]) -> Result<Vec<CompiledPortal>> {
    let n = records(body, PORTAL_SIZE, "portal")?;
    let mut r = Reader::new(body);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let front = short(r.r_u16(), "portal chunk")?;
        let back = short(r.r_u16(), "portal chunk")?;
        let mut all = [Vec3::ZERO; 6];
        for v in &mut all {
            *v = short(r.r_vec3(), "portal chunk")?;
        }
        let count = short(r.r_u32(), "portal chunk")? as usize;
        out.push(CompiledPortal { front, back, vertices: all[..count.min(6)].to_vec() });
    }
    Ok(out)
}

fn read_lights(body: &[u8]) -> Result<Vec<CompiledLight>> {
    let n = records(body, LIGHT_SIZE, "light")?;
    let mut r = Reader::new(body);
    let mut out = Vec::with_capacity(n);
    let what = "light chunk";
    for _ in 0..n {
        let controller = short(r.r_u32(), what)?;
        let kind = short(r.r_u32(), what)?;
        let diffuse = short(r.r_vec4(), what)?.to_array();
        short(r.advance(32), what)?;
        let position = short(r.r_vec3(), what)?;
        let direction = short(r.r_vec3(), what)?;
        let range = short(r.r_f32(), what)?;
        short(r.advance(24), what)?;
        out.push(CompiledLight { controller, kind, diffuse, position, direction, range });
    }
    Ok(out)
}

fn read_glows(body: &[u8]) -> Result<Vec<CompiledGlow>> {
    let n = records(body, GLOW_SIZE, "glow")?;
    let mut r = Reader::new(body);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(CompiledGlow {
            position: short(r.r_vec3(), "glow chunk")?,
            size: short(r.r_f32(), "glow chunk")?,
            shader: short(r.r_u16(), "glow chunk")?,
        });
    }
    Ok(out)
}

impl CompiledLevel {
    /// Reads a `level` file. A file cut short, a count that does not fit its chunk or a chunk the
    /// format needs and the file lacks is an error with a sentence.
    pub fn read(level: &[u8]) -> Result<Self> {
        let top = walk(level, "level file")?;
        let head = chunk(&top, LEVEL_HEADER).ok_or_else(|| invalid("The level file has no header chunk."))?;
        let mut r = Reader::new(head);
        let version = short(r.r_u16(), "level header")?;
        let quality = short(r.r_u16(), "level header")?;

        let table =
            chunk(&top, LEVEL_SHADERS).ok_or_else(|| invalid("The level file has no shader table."))?;
        let mut r = Reader::new(table);
        let count = short(r.r_u32(), "shader table")? as usize;
        // a string is at least its terminator
        if count > r.elapsed() {
            return Err(invalid(format!(
                "The shader table names {count} shaders and has room for {}.",
                r.elapsed()
            )));
        }
        let mut shaders = Vec::with_capacity(count);
        for _ in 0..count {
            shaders.push(short(r.r_stringz(), "shader table")?);
        }

        let body = chunk(&top, LEVEL_VISUALS).ok_or_else(|| invalid("The level file has no visuals."))?;
        let mut visuals = Vec::new();
        for (index, vbody) in walk(body, "visual table")? {
            visuals.push(read_visual(index, vbody)?);
        }
        let sectors = chunk(&top, LEVEL_SECTORS).map(read_sectors).transpose()?.unwrap_or_default();
        let portals = chunk(&top, LEVEL_PORTALS).map(read_portals).transpose()?.unwrap_or_default();
        let lights = chunk(&top, LEVEL_LIGHTS).map(read_lights).transpose()?.unwrap_or_default();
        let glows = chunk(&top, LEVEL_GLOWS).map(read_glows).transpose()?.unwrap_or_default();
        let roots = chunk(&top, PIECE_ROOTS).map(read_roots).transpose()?.unwrap_or_default();

        let mut level = Self { version, quality, shaders, visuals, portals, sectors, lights, glows, roots };
        level.assign_sectors();
        Ok(level)
    }

    /// Reads a `piece` file: a level file with its roots and nothing of a level's own. One without
    /// roots is refused.
    pub fn read_piece(piece: &[u8]) -> Result<Self> {
        let p = Self::read(piece)?;
        if p.roots.is_empty() {
            return Err(invalid("The piece has no root chunk (32): no visual of it would be drawn."));
        }
        Ok(p)
    }

    /// Every visual under a sector's root belongs to that sector; the first sector to reach one
    /// keeps it. A piece's root that names its sector gives it to what is under it.
    fn assign_sectors(&mut self) {
        let at: FxHashMap<u32, usize> = self.visuals.iter().enumerate().map(|(k, v)| (v.index, k)).collect();
        let mut seen = FxHashSet::default();
        let named = self.roots.iter().filter(|r| r.sector != PIECE_SECTOR_AUTO).map(|r| (r.visual, r.sector));
        let starts: Vec<(u32, u16)> =
            self.sectors.iter().map(|s| (s.root, s.index as u16)).chain(named).collect();
        for (root, sector) in starts {
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                let Some(&k) = at.get(&id) else { continue };
                if !seen.insert(id) {
                    continue;
                }
                self.visuals[k].sector = Some(sector);
                stack.extend(self.visuals[k].children.iter().copied());
            }
        }
    }

    /// What the engine refuses a piece for, in sentences (empty: it loads): the compiler's
    /// version, anything of a level's own (sectors, portals, lights, glows), every id its visuals
    /// name outside its own tables - the shader, the vertex and index buffers and the ranges in
    /// them, the fast geometry, the sliding windows -, a child that is not an earlier visual of the
    /// piece, and a root that is no visual of it.
    #[must_use]
    pub fn piece_problems(&self, geom: &GeomTables, fast: Option<&GeomTables>) -> Vec<String> {
        let mut out = Vec::new();
        if self.version != LEVEL_COMPILER_VERSION {
            out.push(format!(
                "The header says the compiler's version {}, not {LEVEL_COMPILER_VERSION}.",
                self.version
            ));
        }
        if !self.sectors.is_empty()
            || !self.portals.is_empty()
            || !self.lights.is_empty()
            || !self.glows.is_empty()
        {
            out.push("A piece carries no sectors, portals, lights or glows.".to_owned());
        }
        if self.roots.is_empty() {
            out.push("The piece has no root.".to_owned());
        }
        let range = |g: &Geometry, t: &GeomTables, what: &str, v: u32| -> Option<String> {
            let verts = t.vertex_buffers.get(g.vertex_buffer as usize);
            let idx = t.index_buffers.get(g.index_buffer as usize);
            match (verts, idx) {
                (Some(&vc), Some(&ic)) => {
                    let vend = u64::from(g.vertex_start) + u64::from(g.vertex_count);
                    let iend = u64::from(g.index_start) + u64::from(g.index_count);
                    (vend > u64::from(vc) || iend > u64::from(ic))
                        .then(|| format!("Visual {v}: its {what} lies outside its buffers."))
                }
                _ => Some(format!("Visual {v}: its {what} names a buffer the piece does not have.")),
            }
        };
        for (k, v) in self.visuals.iter().enumerate() {
            if v.index as usize != k {
                out.push(format!("Visual {k} is stored as visual {}.", v.index));
            }
            if usize::from(v.shader) >= self.shaders.len() {
                out.push(format!("Visual {}: shader {} is not in the piece's table.", v.index, v.shader));
            }
            if let Some(g) = &v.geometry {
                out.extend(range(g, geom, "geometry", v.index));
            }
            if let Some(g) = &v.fast {
                match fast {
                    Some(f) => out.extend(range(g, f, "fast geometry", v.index)),
                    None => out.push(format!("Visual {}: fast geometry without piece.geomx.", v.index)),
                }
            }
            if let Some(w) = v.swi
                && w as usize >= geom.windows
            {
                out.push(format!("Visual {}: sliding windows {w} are not in piece.geom.", v.index));
            }
            if let Some(c) = v.children.iter().find(|&&c| c >= v.index) {
                out.push(format!("Visual {}: its child {c} is not an earlier visual.", v.index));
            }
        }
        for r in &self.roots {
            if r.visual as usize >= self.visuals.len() {
                out.push(format!("Root {} is no visual of the piece.", r.visual));
            }
        }
        out
    }

    /// The shader table entry of a visual, split.
    #[must_use]
    pub fn shader_of(&self, shader: u16) -> Option<ShaderEntry<'_>> {
        self.shaders.get(usize::from(shader)).map(|s| parse_shader(s))
    }

    /// The box of the level: the sectors' root visuals, or every visual without sectors.
    #[must_use]
    pub fn aabb(&self) -> Aabb {
        let roots: FxHashSet<u32> =
            self.sectors.iter().map(|s| s.root).chain(self.roots.iter().map(|r| r.visual)).collect();
        let mut out = Aabb::INVALID;
        for v in &self.visuals {
            if roots.is_empty() || roots.contains(&v.index) {
                out.min = out.min.min(v.aabb.min);
                out.max = out.max.max(v.aabb.max);
            }
        }
        out
    }
}

/// What a geometry file holds: the vertices of each vertex buffer, the indices of each index
/// buffer, how many sets of sliding windows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeomTables {
    pub vertex_buffers: Vec<u32>,
    pub index_buffers: Vec<u32>,
    pub windows: usize,
}

/// The bytes of a vertex element's type (`D3DDECLTYPE`).
fn element_size(ty: u8) -> Option<usize> {
    Some(match ty {
        0 | 4 | 5 | 6 | 8 | 9 | 11 | 13 | 14 | 15 => 4,
        1 | 7 | 10 | 12 | 16 => 8,
        2 => 12,
        3 => 16,
        _ => return None,
    })
}

impl GeomTables {
    /// Reads `level.geom`, `piece.geom` or a fast twin; the buffers' bytes are walked over.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let top = walk(bytes, "geometry file")?;
        let mut out = Self::default();
        if let Some(body) = chunk(&top, GEOM_VB) {
            let mut r = Reader::new(body);
            let what = "vertex buffers";
            let count = short(r.r_u32(), what)?;
            for _ in 0..count {
                // the declaration: 8-byte elements up to the end mark (stream 0xFF)
                let mut stride = 0usize;
                loop {
                    let e = short(r.take(8), what)?;
                    if e[0] == 0xFF {
                        break;
                    }
                    let size = element_size(e[4]).ok_or_else(|| {
                        invalid(format!("A vertex declaration has an unknown type {}.", e[4]))
                    })?;
                    if u16::from_le_bytes([e[0], e[1]]) == 0 {
                        stride = stride.max(usize::from(u16::from_le_bytes([e[2], e[3]])) + size);
                    }
                }
                let n = short(r.r_u32(), what)?;
                short(r.advance((n as usize).saturating_mul(stride)), what)?;
                out.vertex_buffers.push(n);
            }
        }
        if let Some(body) = chunk(&top, GEOM_IB) {
            let mut r = Reader::new(body);
            let count = short(r.r_u32(), "index buffers")?;
            for _ in 0..count {
                let n = short(r.r_u32(), "index buffers")?;
                short(r.advance((n as usize).saturating_mul(2)), "index buffers")?;
                out.index_buffers.push(n);
            }
        }
        if let Some(body) = chunk(&top, GEOM_SWIS) {
            let mut r = Reader::new(body);
            let count = short(r.r_u32(), "sliding windows")?;
            for _ in 0..count {
                short(r.advance(16), "sliding windows")?;
                let n = short(r.r_u32(), "sliding windows")?;
                short(r.advance((n as usize).saturating_mul(8)), "sliding windows")?;
                out.windows += 1;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use gz_chunk::Writer;

    fn header(w: &mut Writer, ty: u8, shader: u16, min: Vec3, max: Vec3) {
        w.chunk(OGF_HEADER, |w| {
            w.w_u8(4);
            w.w_u8(ty);
            w.w_u16(shader);
            w.w_vec3(min);
            w.w_vec3(max);
            w.w_vec3((min + max) * 0.5);
            w.w_f32(1.0);
        });
    }

    fn leaf(w: &mut Writer, shader: u16, min: Vec3, max: Vec3, verts: u32, indices: u32) {
        w.chunk(OGF_GCONTAINER, |w| {
            for v in [0, 0, verts, 0, 0, indices] {
                w.w_u32(v);
            }
        });
        header(w, 0, shader, min, max);
    }

    /// Two leaves under a hierarchy, one sector, one portal, one light, one glow.
    pub(crate) fn sample() -> Vec<u8> {
        let mut level = Writer::new();
        level.chunk(LEVEL_HEADER, |w| {
            w.w_u16(14);
            w.w_u16(2);
        });
        level.chunk(LEVEL_SHADERS, |w| {
            w.w_u32(3);
            w.w_stringz("");
            w.w_stringz("def_shaders\\lmap/wall\\wall_a,lmap#1");
            w.w_stringz("def_shaders\\vert/floor");
        });
        let mut visuals = Writer::new();
        visuals.chunk(0, |w| leaf(w, 1, Vec3::ZERO, Vec3::ONE, 4, 6));
        visuals.chunk(1, |w| leaf(w, 2, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0), 8, 12));
        visuals.chunk(2, |w| {
            header(w, 1, 0, Vec3::ZERO, Vec3::new(2.0, 1.0, 1.0));
            w.chunk(OGF_CHILDREN_L, |w| {
                w.w_u32(2);
                w.w_u32(0);
                w.w_u32(1);
            });
        });
        level.chunk(LEVEL_VISUALS, |w| w.w(visuals.bytes()));
        level.chunk(LEVEL_PORTALS, |w| {
            w.w_u16(0);
            w.w_u16(1);
            for k in 0..6 {
                w.w_vec3(Vec3::splat(k as f32));
            }
            w.w_u32(4);
        });
        level.chunk(LEVEL_LIGHTS, |w| {
            w.w_u32(7);
            w.w_u32(1);
            for _ in 0..12 {
                w.w_f32(0.5);
            }
            w.w_vec3(Vec3::new(1.0, 2.0, 3.0));
            w.w_vec3(Vec3::Y);
            for v in [9.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0] {
                w.w_f32(v);
            }
        });
        level.chunk(LEVEL_GLOWS, |w| {
            w.w_vec3(Vec3::new(5.0, 5.0, 5.0));
            w.w_f32(0.25);
            w.w_u16(2);
        });
        let mut sectors = Writer::new();
        sectors.chunk(0, |w| {
            w.chunk(2, |w| w.w_u32(2));
            w.chunk(1, |w| w.w_u16(0));
        });
        level.chunk(LEVEL_SECTORS, |w| w.w(sectors.bytes()));
        level.into_bytes()
    }

    #[test]
    fn a_hand_built_level_reads_back() {
        let l = CompiledLevel::read(&sample()).unwrap();
        assert_eq!((l.version, l.quality), (14, 2));
        assert_eq!(l.shaders.len(), 3);
        assert_eq!(l.visuals.len(), 3);
        assert_eq!(l.visuals[2].kind, VisualKind::Hierarchy);
        assert_eq!(l.visuals[2].children, [0, 1]);
        let g = l.visuals[1].geometry.unwrap();
        assert_eq!((g.vertex_count, g.faces()), (8, 4));
        assert_eq!(l.visuals[0].sector, Some(0));
        assert_eq!(l.visuals[2].sector, Some(0));
        assert_eq!(l.sectors, [CompiledSector { index: 0, root: 2, portals: vec![0] }]);
        assert_eq!(l.portals.len(), 1);
        assert_eq!((l.portals[0].front, l.portals[0].back, l.portals[0].vertices.len()), (0, 1, 4));
        assert_eq!(l.lights.len(), 1);
        assert_eq!((l.lights[0].controller, l.lights[0].range), (7, 9.0));
        assert_eq!(l.lights[0].position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!((l.glows[0].size, l.glows[0].shader), (0.25, 2));
        assert_eq!(l.aabb(), Aabb::new(Vec3::ZERO, Vec3::new(2.0, 1.0, 1.0)));
        let s = l.shader_of(1).unwrap();
        assert_eq!((s.shader, s.texture, s.lightmap), ("def_shaders\\lmap", "wall\\wall_a", Some(1)));
        assert_eq!(l.shader_of(2).unwrap().lightmap, None);
    }

    #[test]
    fn a_cut_file_is_refused_with_a_sentence() {
        let bytes = sample();
        for cut in [0, 5, 20, bytes.len() / 2, bytes.len() - 1] {
            let err = CompiledLevel::read(&bytes[..cut]).unwrap_err().to_string();
            assert!(!err.is_empty(), "cut at {cut}");
        }
        // a visual chunk that is itself cut short
        let mut visuals = Writer::new();
        visuals.chunk(0, |w| w.w_u8(4));
        let mut level = Writer::new();
        level.chunk(LEVEL_HEADER, |w| {
            w.w_u16(14);
            w.w_u16(0);
        });
        level.chunk(LEVEL_SHADERS, |w| w.w_u32(0));
        level.chunk(LEVEL_VISUALS, |w| w.w(visuals.bytes()));
        let err = CompiledLevel::read(&level.into_bytes()).unwrap_err().to_string();
        assert!(err.contains("visual 0"), "{err}");
    }

    #[test]
    fn a_wild_count_is_an_error_not_an_allocation() {
        let mut level = Writer::new();
        level.chunk(LEVEL_HEADER, |w| {
            w.w_u16(14);
            w.w_u16(0);
        });
        level.chunk(LEVEL_SHADERS, |w| w.w_u32(u32::MAX));
        level.chunk(LEVEL_VISUALS, |_| {});
        assert!(CompiledLevel::read(&level.into_bytes()).is_err());
    }

    #[test]
    fn a_missing_required_chunk_is_named() {
        let mut level = Writer::new();
        level.chunk(LEVEL_HEADER, |w| {
            w.w_u16(14);
            w.w_u16(0);
        });
        let err = CompiledLevel::read(&level.into_bytes()).unwrap_err().to_string();
        assert!(err.contains("shader table"), "{err}");
    }
}

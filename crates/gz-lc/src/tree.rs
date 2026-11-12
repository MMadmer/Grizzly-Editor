//! The visual tree and its files: `Flex2OGF` (`xrFlex2OGF.cpp`), the MU models' visuals
//! (`xrMU_Model_Calc_ogf.cpp`, `xrMU_Model_export_geometry.cpp`, `xrMU_Model_export_OGF.cpp`),
//! the sectors and their hierarchy (`xrSectors.cpp`, `Sector.cpp`), and the savers
//! (`xrSaveOGF.cpp`, `OGF_Face_Save.cpp`, `OGF_Face.cpp`'s node and LOD writers, `vbm.h`).

use glam::{Vec2, Vec3};
use gz_chunk::Writer;
use gz_fmt_level::{Aabb, BuildLod};
use gz_lc_light::math::{EPS, EPS_L, Matrix, i_floor, u8_clr};
use gz_lc_light::{ColorC, LcGlobalData, MuModel, ShaderXrlc};
use rayon::prelude::*;
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::load::Extra;
use crate::ogf::{
    Ogf, OgfTexture, OgfVertex, PM_METRIC_MU, PM_METRIC_STATIC, SlideWindow, basis_vec4, s16_tc_lmap,
    s24_tc_base, u8_vec4,
};
use crate::sphere::{Bounds, MsvcRand, calc_bounds, min_sphere};
use crate::split::Split;
use crate::{Error, Log, Result};

/// `MT_*`.
const MT_NORMAL: u8 = 0;
const MT_HIERRARHY: u8 = 1;
const MT_PROGRESSIVE: u8 = 2;
const MT_LOD: u8 = 6;
const MT_TREE_ST: u8 = 7;
const MT_TREE_PM: u8 = 11;
const OGF_HEADER: u32 = 1;
const OGF_SWIDATA: u32 = 6;
const OGF_CHILDREN_L: u32 = 10;
const OGF_LODDEF2: u32 = 11;
const OGF_TREEDEF2: u32 = 12;
const OGF_SWICONTAINER: u32 = 20;
const OGF_GCONTAINER: u32 = 21;
const OGF_FASTPATH: u32 = 22;
const FORMAT_VERSION: u8 = 4;

/// `OGF_Base`.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeBase {
    pub level: i32,
    pub sector: u16,
    pub connected: bool,
    pub bbox: Aabb,
    pub c: Vec3,
    pub r: f32,
}

impl NodeBase {
    fn new(level: i32, sector: u16) -> Self {
        Self { level, sector, connected: false, bbox: Aabb::INVALID, c: Vec3::ZERO, r: 0.0 }
    }

    fn set(&mut self, b: Bounds) {
        self.bbox = b.bbox;
        self.c = b.center;
        self.r = b.radius;
    }
}

/// `OGF_Reference`.
#[derive(Clone, Debug, PartialEq)]
pub struct OgfRef {
    pub material: u16,
    pub textures: Vec<OgfTexture>,
    /// The model and the subdivision whose visual this places.
    pub model: (usize, usize),
    pub vb_id: u32,
    pub vb_start: u32,
    pub ib_id: u32,
    pub ib_start: u32,
    pub sw_id: u32,
    pub xform: Matrix,
    pub c_scale: ColorC,
    pub c_bias: ColorC,
}

/// `OGF_LOD::_vertex`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LodVertex {
    pub v: Vec3,
    pub t: Vec2,
    pub c_rgb_hemi: u32,
    pub c_sun: u8,
}

/// An item of `g_tree`.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Ogf(Box<Ogf>),
    Ref(Box<OgfRef>),
    Node(Vec<u32>),
    Lod { children: Vec<u32>, material: u32, faces: Box<[[LodVertex; 4]; 8]> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub base: NodeBase,
    pub item: Item,
}

/// The tree, the MU models' visuals, the containers and the shader names of a build.
#[derive(Debug, Default)]
pub struct Tree {
    pub nodes: Vec<TreeNode>,
    /// The visual of every MU model's subdivision.
    pub mu: Vec<Vec<Ogf>>,
    pub vb: VbContainer,
    pub ib: IbContainer,
    pub swi: Vec<Vec<SlideWindow>>,
    pub xvb: VbContainer,
    pub xib: IbContainer,
    pub shaders: Vec<String>,
    /// The bounds the C++ takes with `CalcBounds`, in its order: what each is of.
    pub bounds_jobs: Vec<BoundsOf>,
}

/// What a `CalcBounds` call measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsOf {
    Tree(usize),
    Mu(usize, usize),
}

impl Tree {
    /// `RegisterShader`: a name's index, case ignored, added when new.
    pub fn register_shader(&mut self, name: &str) -> u16 {
        if let Some(i) = self.shaders.iter().position(|s| s.eq_ignore_ascii_case(name)) {
            return i as u16;
        }
        self.shaders.push(name.to_owned());
        (self.shaders.len() - 1) as u16
    }

    /// `GetGeometry` of a tree item.
    fn geometry(&self, id: usize, out: &mut Vec<Vec3>) {
        let Some(n) = self.nodes.get(id) else { return };
        match &n.item {
            Item::Ogf(o) => out.extend(o.data.vertices.iter().map(|v| v.p)),
            Item::Ref(r) => {
                if let Some(o) = self.mu.get(r.model.0).and_then(|m| m.get(r.model.1)) {
                    out.extend(o.data.vertices.iter().map(|v| r.xform.transform_tiny(v.p)));
                }
            }
            Item::Node(ch) | Item::Lod { children: ch, .. } => {
                for &c in ch {
                    self.geometry(c as usize, out);
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------- containers

/// A vertex declaration as `D3DVERTEXELEMENT9` records, the end mark included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub bytes: Vec<u8>,
    pub stride: u32,
}

fn decl(elems: &[(u16, u8, u8, u8)], stride: u32) -> Decl {
    let mut bytes = Vec::with_capacity((elems.len() + 1) * 8);
    for &(offset, ty, usage, index) in elems {
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&[ty, 0, usage, index]);
    }
    bytes.extend_from_slice(&0xffu16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&[17, 0, 0, 0]);
    Decl { bytes, stride }
}

const FLOAT3: u8 = 2;
const D3DCOLOR: u8 = 4;
const SHORT2: u8 = 6;
const SHORT4: u8 = 7;
const U_POSITION: u8 = 0;
const U_NORMAL: u8 = 3;
const U_TEXCOORD: u8 = 5;
const U_TANGENT: u8 = 6;
const U_BINORMAL: u8 = 7;
const U_COLOR: u8 = 10;

fn decl_lmap() -> Decl {
    decl(
        &[
            (0, FLOAT3, U_POSITION, 0),
            (12, D3DCOLOR, U_NORMAL, 0),
            (16, D3DCOLOR, U_TANGENT, 0),
            (20, D3DCOLOR, U_BINORMAL, 0),
            (24, SHORT2, U_TEXCOORD, 0),
            (28, SHORT2, U_TEXCOORD, 1),
        ],
        32,
    )
}
fn decl_vert() -> Decl {
    decl(
        &[
            (0, FLOAT3, U_POSITION, 0),
            (12, D3DCOLOR, U_NORMAL, 0),
            (16, D3DCOLOR, U_TANGENT, 0),
            (20, D3DCOLOR, U_BINORMAL, 0),
            (24, D3DCOLOR, U_COLOR, 0),
            (28, SHORT2, U_TEXCOORD, 0),
        ],
        32,
    )
}
fn decl_x() -> Decl {
    decl(&[(0, FLOAT3, U_POSITION, 0)], 12)
}
fn decl_mu() -> Decl {
    decl(
        &[
            (0, FLOAT3, U_POSITION, 0),
            (12, D3DCOLOR, U_NORMAL, 0),
            (16, D3DCOLOR, U_TANGENT, 0),
            (20, D3DCOLOR, U_BINORMAL, 0),
            (24, SHORT4, U_TEXCOORD, 0),
        ],
        32,
    )
}

const VB_MAX_SIZE: usize = 4096 * 1024;
const VB_MAX_VERTICES: usize = 65535;

/// `VBContainer`.
#[derive(Clone, Debug, Default)]
pub struct VbContainer {
    decls: Vec<Decl>,
    data: Vec<Vec<u8>>,
}

impl VbContainer {
    /// `Begin`, `Add`, `End`: the container's id and the first vertex.
    pub fn add(&mut self, d: &Decl, bytes: Vec<u8>) -> (u32, u32) {
        let collected = bytes.len();
        let verts = collected / d.stride as usize;
        for (cid, (dd, data)) in self.decls.iter().zip(self.data.iter_mut()).enumerate() {
            if dd != d {
                continue;
            }
            let already = data.len();
            if already + collected > VB_MAX_SIZE {
                continue;
            }
            let va = already / d.stride as usize;
            if va + verts > VB_MAX_VERTICES {
                continue;
            }
            data.extend_from_slice(&bytes);
            return (cid as u32, va as u32);
        }
        self.decls.push(d.clone());
        self.data.push(bytes);
        ((self.decls.len() - 1) as u32, 0)
    }

    fn save(&self, w: &mut Writer) {
        w.w_u32(self.decls.len() as u32);
        for (d, data) in self.decls.iter().zip(&self.data) {
            w.w(&d.bytes);
            w.w_u32((data.len() / d.stride as usize) as u32);
            w.w(data);
        }
    }
}

/// `IBContainer`.
#[derive(Clone, Debug, Default)]
pub struct IbContainer {
    data: Vec<Vec<u16>>,
}

impl IbContainer {
    pub fn register(&mut self, idx: &[u16]) -> (u32, u32) {
        const LIMIT: usize = 1024 * 1024;
        for (id, d) in self.data.iter_mut().enumerate() {
            if d.len() + idx.len() < LIMIT {
                let start = d.len();
                d.extend_from_slice(idx);
                return (id as u32, start as u32);
            }
        }
        self.data.push(idx.to_vec());
        ((self.data.len() - 1) as u32, 0)
    }

    fn save(&self, w: &mut Writer) {
        w.w_u32(self.data.len() as u32);
        for d in &self.data {
            w.w_u32(d.len() as u32);
            let mut raw = Vec::with_capacity(d.len() * 2);
            for v in d {
                raw.extend_from_slice(&v.to_le_bytes());
            }
            w.w(&raw);
        }
    }
}

fn save_swi(list: &[Vec<SlideWindow>], w: &mut Writer) {
    w.w_u32(list.len() as u32);
    for s in list {
        for _ in 0..4 {
            w.w_u32(0);
        }
        w.w_u32(s.len() as u32);
        write_windows(w, s);
    }
}

fn write_windows(w: &mut Writer, s: &[SlideWindow]) {
    for x in s {
        w.w_u32(x.offset);
        w.w_u16(x.num_tris);
        w.w_u16(x.num_verts);
    }
}

// --------------------------------------------------------------------------------- Flex2OGF

fn face_vertices(g: &LcGlobalData, f: gz_lc_light::FaceId) -> Option<[OgfVertex; 3]> {
    let face = g.mesh.face(f)?;
    let mut out: [Option<OgfVertex>; 3] = [None, None, None];
    for (k, slot) in out.iter_mut().enumerate() {
        let v = g.mesh.vertex(face.v[k])?;
        let uv: SmallVec<[Vec2; 3]> = face.tc.iter().map(|t| t.uv[k]).collect();
        *slot = Some(OgfVertex {
            p: v.p,
            n: v.n,
            t: face.basis_tangent[k],
            b: face.basis_binormal[k],
            color: v.c,
            uv,
        });
    }
    let [a, b, c] = out;
    Some([a?, b?, c?])
}

fn texture_of(g: &LcGlobalData, material: u16) -> OgfTexture {
    let t = g.materials.get(usize::from(material)).and_then(|m| g.textures.get(usize::from(m.surface)));
    t.map_or(OgfTexture { name: SmolStr::default(), width: 1, height: 1 }, |t| OgfTexture {
        name: t.name.clone(),
        width: t.width,
        height: t.height,
    })
}

/// `CBuild::Flex2OGF`'s first half for one subdivision: the textures and the faces, then
/// `Optimize`.
fn build_ogf(g: &LcGlobalData, faces: &[gz_lc_light::FaceId]) -> Option<Ogf> {
    let &first = faces.first()?;
    let f0 = g.mesh.face(first)?;
    let material = f0.material;
    let sector = g.materials.get(usize::from(material)).map_or(0, |m| m.sector);
    let mut o = Ogf { material, sector, ..Ogf::default() };
    let base = texture_of(g, material);
    o.textures.push(base.clone());
    if g.has_implicit_lighting(first) {
        o.textures.push(OgfTexture { name: SmolStr::new(format!("{}_lm.dds", base.name)), ..base });
    } else if let Some(page) = f0.lmap.and_then(|p| g.lightmaps.get(p as usize)) {
        for n in ["_1", "_2"] {
            o.textures.push(OgfTexture {
                name: SmolStr::new(format!("{}{n}", page.name)),
                width: page.width,
                height: page.height,
            });
        }
    }
    let opaque = gz_lc_light::cache_opacity(f0, &g.materials, &g.textures);
    for &f in faces {
        if let Some(v) = face_vertices(g, f) {
            o.build_face(&v, !opaque);
        }
    }
    let optimize_uv = g.shader(material).flags & ShaderXrlc::OPTIMIZE_UV != 0;
    o.optimize(optimize_uv);
    Some(o)
}

/// `CBuild::Flex2OGF`: one visual per subdivision, built in parallel; the bounds are the
/// caller's (they draw from the one `rand` stream), then progressive and stripified.
pub fn flex2ogf(g: &LcGlobalData, splits: Vec<Split>, tree: &mut Tree, noise: bool, log: &mut Log) {
    for (i, s) in splits.iter().enumerate() {
        if s.len() > SS_HIGH_X2 {
            log.warn(format!("! ERROR: subdiv #{i} has more than {SS_HIGH_X2} faces ({})", s.len()));
        }
    }
    let ogfs: Vec<Option<Ogf>> = splits.par_iter().map(|s| build_ogf(g, s)).collect();
    for o in ogfs.into_iter().flatten() {
        let id = tree.nodes.len();
        tree.nodes.push(TreeNode { base: NodeBase::new(0, o.sector), item: Item::Ogf(Box::new(o)) });
        tree.bounds_jobs.push(BoundsOf::Tree(id));
    }
    // the bounds of these come before the progressive pass: it reorders the vertices
    let draft = g.params.quality == gz_lc_light::QUALITY_DRAFT;
    let logs: Vec<Vec<String>> = tree
        .nodes
        .par_iter_mut()
        .map(|n| {
            let mut l = Vec::new();
            if let Item::Ogf(o) = &mut n.item {
                n.base.set(pending_bounds(o));
                if !noise {
                    o.make_progressive(PM_METRIC_STATIC, draft, &mut l);
                }
                o.stripify(&mut l);
            }
            l
        })
        .collect();
    for l in logs.into_iter().flatten() {
        log.msg(l);
    }
}

const SS_HIGH_X2: usize = 2 * 2 * 1024;

/// The points `CalcBounds` will read, taken now (a placeholder holds them until the caller
/// takes the spheres in the C++ order).
fn pending_bounds(o: &mut Ogf) -> Bounds {
    o.bounds = None;
    let pts = o.geometry();
    let mut bb = Aabb::INVALID;
    for p in &pts {
        bb.modify(*p);
    }
    bb.min -= Vec3::splat(EPS_L);
    bb.max += Vec3::splat(EPS_L);
    o.snapshot = pts;
    Bounds { bbox: bb, center: Vec3::ZERO, radius: 0.0 }
}

// --------------------------------------------------------------------------------- MU models

fn mu_face_vertices(
    m: &MuModel,
    f: gz_lc_light::FaceId,
    index: &rustc_hash::FxHashMap<gz_lc_light::VertId, usize>,
) -> Option<[OgfVertex; 3]> {
    let face = m.mesh.face(f)?;
    let mut out: [Option<OgfVertex>; 3] = [None, None, None];
    for (k, slot) in out.iter_mut().enumerate() {
        let v = m.mesh.vertex(face.v[k])?;
        let id = index.get(&face.v[k]).copied().unwrap_or(0);
        let tc = face.tc0();
        *slot = Some(OgfVertex {
            p: v.p,
            n: v.n,
            t: gz_lc_light::Basis::default(),
            b: gz_lc_light::Basis::default(),
            color: m.color.get(id).copied().unwrap_or_default(),
            uv: SmallVec::from_slice(&[tc[k]]),
        });
    }
    let [a, b, c] = out;
    Some([a?, b?, c?])
}

/// `calc_ogf` of every model: a visual per subdivision, with its tangent basis, made
/// progressive and stripified; the bounds are taken by the caller in model order.
pub fn mu_calc_ogf(g: &LcGlobalData, tree: &mut Tree, log: &mut Log) {
    let draft = g.params.quality == gz_lc_light::QUALITY_DRAFT;
    let results: Vec<(Vec<Ogf>, Vec<String>)> = g
        .mu_models
        .par_iter()
        .map(|m| {
            let index: rustc_hash::FxHashMap<_, _> =
                m.mesh.vertices().iter().enumerate().map(|(i, &v)| (v, i)).collect();
            let mut l = Vec::new();
            let mut out = Vec::new();
            for s in &m.subdivs {
                let material = s.material as u16;
                let mut o = Ogf { material, sector: 0, ..Ogf::default() };
                o.textures.push(texture_of(g, material));
                let faces = m.mesh.faces();
                for &f in faces.iter().skip(s.start as usize).take(s.count as usize) {
                    if let Some(v) = mu_face_vertices(m, f, &index) {
                        o.build_face(&v, true);
                    }
                }
                let optimize_uv = g.shader(material).flags & ShaderXrlc::OPTIMIZE_UV != 0;
                o.optimize(optimize_uv);
                let _ = pending_bounds(&mut o);
                o.calculate_tb();
                o.make_progressive(PM_METRIC_MU, draft, &mut l);
                o.stripify(&mut l);
                out.push(o);
            }
            (out, l)
        })
        .collect();
    for (mi, (ogfs, l)) in results.into_iter().enumerate() {
        for si in 0..ogfs.len() {
            tree.bounds_jobs.push(BoundsOf::Mu(mi, si));
        }
        tree.mu.push(ogfs);
        for line in l {
            log.msg(line);
        }
    }
}

/// `QC`: a texture coordinate of `[-16, 16]` in 16 bits.
fn qc(v: f32) -> i16 {
    i_floor(v * (32768 / 16) as f32).clamp(-32768, 32767) as i16
}

/// `export_geometry` of every model: its subdivisions' vertices into the shared buffers.
pub fn mu_export_geometry(g: &LcGlobalData, tree: &mut Tree) {
    let d = decl_mu();
    for (mi, m) in g.mu_models.iter().enumerate() {
        let mut bb = Aabb::INVALID;
        for &v in m.mesh.vertices() {
            if let Some(x) = m.mesh.vertex(v) {
                bb.modify(x.p);
            }
        }
        let c = bb.center();
        let low = Vec3::new(c.x, bb.min.y, c.z);
        let ysize = bb.max.y - bb.min.y;
        let Some(ogfs) = tree.mu.get(mi) else { continue };
        let mut placed = Vec::with_capacity(ogfs.len());
        for o in ogfs {
            let mut bytes = Vec::with_capacity(o.data.vertices.len() * 32);
            for v in &o.data.vertices {
                for c in v.p.to_array() {
                    bytes.extend_from_slice(&c.to_le_bytes());
                }
                let col = v.color.get();
                let n = Vec3::new(
                    (v.n.x + 1.0) * (0.5 * 255.0),
                    (v.n.y + 1.0) * (0.5 * 255.0),
                    (v.n.z + 1.0) * (0.5 * 255.0),
                );
                let b = |x: f32| i_floor(x).clamp(0, 255) as u8;
                let cc = i_floor(col.hemi * 255.0).clamp(0, 255) as u8;
                bytes.extend_from_slice(&[b(n.z), b(n.y), b(n.x), cc]);
                bytes.extend_from_slice(&basis_vec4(&v.t, 0));
                bytes.extend_from_slice(&basis_vec4(&v.b, 0));
                let uv = v.uv.first().copied().unwrap_or(Vec2::ZERO);
                bytes.extend_from_slice(&qc(uv.x).to_le_bytes());
                bytes.extend_from_slice(&qc(uv.y).to_le_bytes());
                let f1 = (v.p.y - low.y) / ysize;
                let f2 = gz_lc_light::math::distance(v.p, low) / ysize;
                bytes.extend_from_slice(&qc((f1 + f2) / 2.0).to_le_bytes());
                bytes.extend_from_slice(&0i16.to_le_bytes());
            }
            let (vb, vs) = tree.vb.add(&d, bytes);
            let flat: Vec<u16> = o.data.faces.iter().flatten().copied().collect();
            let (ib, is) = tree.ib.register(&flat);
            let sw = if o.progressive() {
                tree.swi.push(o.data.swi.clone());
                (tree.swi.len() - 1) as u32
            } else {
                u32::MAX
            };
            placed.push((vb, vs, ib, is, sw));
        }
        if let Some(m) = tree.mu.get_mut(mi) {
            for (o, (vb, vs, ib, is, sw)) in m.iter_mut().zip(placed) {
                o.data.vb_id = vb;
                o.data.vb_start = vs;
                o.data.ib_id = ib;
                o.data.ib_start = is;
                o.mu_sw_id = sw;
            }
        }
    }
}

/// `export_ogf` of every reference: a visual per subdivision placing the model's, and a LOD
/// node over them when the model has a LOD, its corners coloured by the reference's light.
pub fn mu_export_refs(g: &LcGlobalData, extra: &Extra, tree: &mut Tree) -> Result<()> {
    for r in &g.mu_refs {
        let mi = r.model as usize;
        let Some(m) = g.mu_models.get(mi) else { continue };
        let mut generated = Vec::new();
        let count = tree.mu.get(mi).map_or(0, Vec::len);
        for si in 0..count {
            let Some(o) = tree.mu.get(mi).and_then(|x| x.get(si)) else { continue };
            let item = OgfRef {
                material: o.material,
                textures: vec![o.textures.first().cloned().unwrap_or(OgfTexture {
                    name: SmolStr::default(),
                    width: 1,
                    height: 1,
                })],
                model: (mi, si),
                vb_id: o.data.vb_id,
                vb_start: o.data.vb_start,
                ib_id: o.data.ib_id,
                ib_start: o.data.ib_start,
                sw_id: o.mu_sw_id,
                xform: r.xform,
                c_scale: r.c_scale,
                c_bias: r.c_bias,
            };
            let id = tree.nodes.len();
            generated.push(id as u32);
            let mut node = TreeNode { base: NodeBase::new(0, r.sector), item: Item::Ref(Box::new(item)) };
            node.base.sector = r.sector;
            tree.nodes.push(node);
            tree.bounds_jobs.push(BoundsOf::Tree(id));
            set_box(tree, id);
        }
        if m.lod_id == u16::MAX {
            continue;
        }
        let lod: &BuildLod = extra.lods.get(usize::from(m.lod_id)).ok_or_else(|| {
            Error::Invalid(format!("The model '{}' names a LOD the build does not have.", m.name))
        })?;
        let mut faces = Box::new([[LodVertex::default(); 4]; 8]);
        for (lf, face) in faces.iter_mut().enumerate() {
            for (lv, v) in face.iter_mut().enumerate() {
                v.v = r.xform.transform_tiny(lod.faces[lf].v[lv]);
                v.t = lod.faces[lf].t[lv];
                v.c_rgb_hemi = 0xffff_ffff;
                v.c_sun = 0xff;
            }
        }
        if generated.is_empty() {
            return Err(Error::Invalid(format!("The LOD of the model '{}' has no visual under it.", m.name)));
        }
        let id = tree.nodes.len();
        for &c in &generated {
            if let Some(n) = tree.nodes.get_mut(c as usize) {
                n.base.connected = true;
            }
        }
        tree.nodes.push(TreeNode {
            base: NodeBase::new(1, r.sector),
            item: Item::Lod { children: generated, material: lod.material, faces: faces.clone() },
        });
        tree.bounds_jobs.push(BoundsOf::Tree(id));
        set_box(tree, id);
        // the corners' colours: every vertex of the model weighted by its nearness
        let verts: Vec<Vec3> = m
            .mesh
            .vertices()
            .iter()
            .filter_map(|&v| m.mesh.vertex(v))
            .map(|x| r.xform.transform_tiny(x.p))
            .collect();
        for face in faces.iter_mut() {
            for v in face.iter_mut() {
                let mut c = ColorC::default();
                let mut n = 0.0f32;
                for (i, bp) in verts.iter().enumerate() {
                    let base = r.color.get(i).copied().unwrap_or_default().get();
                    let d = gz_lc_light::math::distance(v.v, *bp);
                    let a = 1.0 / (1.0 + 100.0 * d * d);
                    let mut vc = base;
                    vc.mul(a);
                    c.add(&vc);
                    n += a;
                }
                let s = 1.0 / (n + EPS);
                c.mul(s);
                v.c_rgb_hemi = gz_lc_light::math::color_rgba(
                    u8_clr(c.rgb.x),
                    u8_clr(c.rgb.y),
                    u8_clr(c.rgb.z),
                    u8_clr(c.hemi),
                );
                v.c_sun = u8_clr(c.sun);
            }
        }
        if let Some(TreeNode { item: Item::Lod { faces: f, .. }, .. }) = tree.nodes.get_mut(id) {
            *f = faces;
        }
    }
    Ok(())
}

/// The box `CalcBounds` will give an item, known before its sphere: the box of its points
/// grown by `EPS_L` (the union of its children's boxes for a node).
fn set_box(tree: &mut Tree, id: usize) {
    let mut pts = Vec::new();
    tree.geometry(id, &mut pts);
    let mut bb = Aabb::INVALID;
    for p in &pts {
        bb.modify(*p);
    }
    bb.min -= Vec3::splat(EPS_L);
    bb.max += Vec3::splat(EPS_L);
    if let Some(n) = tree.nodes.get_mut(id) {
        n.base.bbox = bb;
    }
}

// --------------------------------------------------------------------------------- sectors

/// `ValidateMerge` of the hierarchy.
fn validate_node(base: &Aabb, bb: &Aabb, limit: f32) -> Option<f32> {
    let mut m = *base;
    m.merge(bb);
    let mut sz = m.max - m.min;
    sz += Vec3::splat(EPS_L);
    if sz.x > limit || sz.y > limit || sz.z > limit {
        return None;
    }
    let s = m.max - m.min;
    Some(s.x * s.y * s.z)
}

/// `CSector`.
#[derive(Clone, Debug, Default)]
pub struct Sector {
    pub root: usize,
    pub portals: Vec<u16>,
}

/// `CBuild::BuildSectors`: per sector the hierarchy of nodes over its visuals, then the
/// portals.
pub fn build_sectors(extra: &mut Extra, tree: &mut Tree, log: &mut Log) -> Result<Vec<Sector>> {
    let max = tree.nodes.iter().map(|n| n.base.sector).max().unwrap_or(0);
    if max == u16::MAX {
        return Err(Error::Invalid("A visual has no sector.".into()));
    }
    let count = usize::from(max) + 1;
    log.msg(format!("{count} sectors accepted."));
    let present: Vec<bool> =
        (0..count).map(|s| tree.nodes.iter().any(|n| usize::from(n.base.sector) == s)).collect();
    let mut sectors = Vec::with_capacity(count);
    for sid in 0..count {
        if !present.get(sid).copied().unwrap_or(false) {
            return Err(Error::Invalid(format!("The sector #{sid} has no visual.")));
        }
        let root = build_hierarchy(extra, tree, sid as u16, log)?;
        sectors.push(Sector { root, portals: Vec::new() });
    }
    for (i, p) in extra.portals.iter().enumerate() {
        for s in [p.sector_front, p.sector_back] {
            let sec = sectors.get_mut(usize::from(s)).ok_or_else(|| {
                Error::Invalid(format!("The portal #{i} names a sector the level does not have."))
            })?;
            sec.portals.push(i as u16);
        }
    }
    for s in &mut sectors {
        s.portals.sort_unstable();
    }
    Ok(sectors)
}

fn build_hierarchy(extra: &mut Extra, tree: &mut Tree, sid: u16, log: &mut Log) -> Result<usize> {
    let mut scene = Aabb::INVALID;
    for n in &tree.nodes {
        scene.merge(&n.base.bbox);
    }
    scene.min -= Vec3::splat(EPS_L);
    scene.max += Vec3::splat(EPS_L);
    extra.scene_bb = scene;
    let size = scene.max - scene.min;
    let mut delimiter = size.x.max(size.y.max(size.z)) * 2.0;
    let mut level = 2;
    let mut limit = (32.0f32 / 4.0).max(4.0);
    if delimiter <= limit {
        delimiter *= 2.0;
    }
    let mut any = false;
    while limit <= delimiter {
        let isize = tree.nodes.len();
        for i in 0..isize {
            let Some(n) = tree.nodes.get(i) else { continue };
            if n.base.connected || n.base.sector != sid {
                continue;
            }
            let mut children = vec![i as u32];
            let mut bbox = n.base.bbox;
            if let Some(n) = tree.nodes.get_mut(i) {
                n.base.connected = true;
            }
            loop {
                let mut best: Option<usize> = None;
                let mut best_v = f32::MAX;
                for j in 0..isize {
                    let Some(c) = tree.nodes.get(j) else { continue };
                    if c.base.connected || c.base.sector != sid {
                        continue;
                    }
                    if let Some(v) = validate_node(&bbox, &c.base.bbox, limit)
                        && v < best_v
                    {
                        best_v = v;
                        best = Some(j);
                    }
                }
                let Some(b) = best else { break };
                children.push(b as u32);
                if let Some(c) = tree.nodes.get_mut(b) {
                    c.base.connected = true;
                    let cb = c.base.bbox;
                    bbox.merge(&cb);
                }
            }
            if children.len() > 1 {
                let id = tree.nodes.len();
                tree.nodes.push(TreeNode { base: NodeBase::new(level, sid), item: Item::Node(children) });
                tree.bounds_jobs.push(BoundsOf::Tree(id));
                set_box(tree, id);
                any = true;
            } else if let Some(n) = tree.nodes.get_mut(i) {
                n.base.connected = false;
            }
        }
        if isize != tree.nodes.len() {
            level += 1;
        }
        limit *= 2.0;
    }
    if any {
        return Ok(tree.nodes.len() - 1);
    }
    let mut root = None;
    for (i, n) in tree.nodes.iter().enumerate() {
        if !n.base.connected && n.base.sector == sid {
            if root.is_some() {
                return Err(Error::Invalid(format!("The sector #{sid} has more than one root.")));
            }
            root = Some(i);
        }
    }
    root.ok_or_else(|| {
        log.msg(format!("Can't build hierrarhy for sector #{sid}"));
        Error::Invalid(format!("Can't build the hierarchy of the sector #{sid}."))
    })
}

// --------------------------------------------------------------------------------- bounds

/// The spheres of every `CalcBounds` call in the C++ order: each drawn from the `rand` stream at
/// the offset the calls before it leave, computed in parallel; a call that restarts (and draws
/// more) has the ones after it computed again from where it ended.
pub fn take_bounds(tree: &mut Tree, rng: &mut MsvcRand) {
    let jobs = tree.bounds_jobs.clone();
    let points: Vec<Vec<Vec3>> = jobs
        .par_iter()
        .map(|j| match *j {
            BoundsOf::Tree(id) => {
                if let Some(TreeNode { item: Item::Ogf(o), .. }) = tree.nodes.get(id) {
                    o.snapshot.clone()
                } else {
                    let mut p = Vec::new();
                    tree.geometry(id, &mut p);
                    p
                }
            }
            BoundsOf::Mu(m, s) => {
                tree.mu.get(m).and_then(|x| x.get(s)).map(|o| o.snapshot.clone()).unwrap_or_default()
            }
        })
        .collect();
    let mut results: Vec<Option<Bounds>> = vec![None; jobs.len()];
    let mut start = 0usize;
    let mut state = *rng;
    while start < jobs.len() {
        // the draws each call takes when it does not restart
        let mut offsets = Vec::with_capacity(jobs.len() - start);
        let mut at = 0u64;
        for p in &points[start..] {
            offsets.push(at);
            at += p.len().saturating_sub(1) as u64;
        }
        let computed: Vec<(Bounds, MsvcRand, MsvcRand)> = points[start..]
            .par_iter()
            .zip(offsets.par_iter())
            .map(|(p, &off)| {
                let mut r = state.advanced(off);
                let begin = r;
                let b = calc_bounds(p, &mut r);
                (b, begin, r)
            })
            .collect();
        let mut next = jobs.len();
        for (k, (b, begin, end)) in computed.into_iter().enumerate() {
            let expect = begin.advanced(points[start + k].len().saturating_sub(1) as u64);
            results[start + k] = Some(b);
            if end != expect {
                state = end;
                next = start + k + 1;
                break;
            }
            if start + k + 1 == jobs.len() {
                state = end;
            }
        }
        start = next;
    }
    *rng = state;
    for (j, b) in jobs.iter().zip(results) {
        let Some(b) = b else { continue };
        match *j {
            BoundsOf::Tree(id) => {
                if let Some(n) = tree.nodes.get_mut(id) {
                    n.base.set(b);
                    if let Item::Ogf(o) = &mut n.item {
                        o.bounds = Some(b);
                        o.snapshot = Vec::new();
                    }
                }
            }
            BoundsOf::Mu(m, s) => {
                if let Some(o) = tree.mu.get_mut(m).and_then(|x| x.get_mut(s)) {
                    o.bounds = Some(b);
                    o.snapshot = Vec::new();
                }
            }
        }
    }
    let _ = min_sphere;
}

// --------------------------------------------------------------------------------- savers

fn header(w: &mut Writer, ty: u8, shader: u16, base: &NodeBase) {
    w.chunk(OGF_HEADER, |w| {
        w.w_u8(FORMAT_VERSION);
        w.w_u8(ty);
        w.w_u16(shader);
        w.w_vec3(base.bbox.min);
        w.w_vec3(base.bbox.max);
        w.w_vec3(base.c);
        w.w_f32(base.r);
    });
}

fn shader_name(g: &LcGlobalData, extra: &Extra, material: u32, textures: &str) -> String {
    let sh = g
        .materials
        .get(material as usize)
        .and_then(|m| extra.shader_render.get(usize::from(m.shader)))
        .map_or("", SmolStr::as_str);
    format!("{sh}/{textures}")
}

fn tex_list(textures: &[OgfTexture]) -> String {
    let mut s = String::new();
    for t in textures {
        if !s.is_empty() {
            s.push(',');
        }
        s.push_str(t.name.split('.').next().unwrap_or(""));
    }
    s
}

/// `OGF::PreSave`: the visual's vertices and faces into the shared containers.
fn pre_save(g: &LcGlobalData, tree: &mut Tree, id: usize) {
    let Some(TreeNode { item: Item::Ogf(o), .. }) = tree.nodes.get(id) else { return };
    let vertex_colored = g.shader(o.material).flags & ShaderXrlc::LIGHT_VERTEX != 0;
    let mut fast = None;
    if !o.fast.vertices.is_empty() && !o.fast.faces.is_empty() {
        let mut bytes = Vec::with_capacity(o.fast.vertices.len() * 12);
        for p in &o.fast.vertices {
            for c in p.to_array() {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
        }
        let flat: Vec<u16> = o.fast.faces.iter().flatten().copied().collect();
        fast = Some((bytes, flat));
    }
    let mut bytes = Vec::with_capacity(o.data.vertices.len() * 32);
    for v in &o.data.vertices {
        let c = v.color.get();
        let n = gz_lc_light::math::normalize(v.n);
        let (tu, tv) =
            (s24_tc_base(v.uv.first().map_or(0.0, |x| x.x)), s24_tc_base(v.uv.first().map_or(0.0, |x| x.y)));
        for x in v.p.to_array() {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        bytes.extend_from_slice(&u8_vec4(n, u8_clr(c.hemi)));
        bytes.extend_from_slice(&basis_vec4(&v.t, tu.1));
        bytes.extend_from_slice(&basis_vec4(&v.b, tv.1));
        if vertex_colored {
            let col = [u8_clr(c.rgb.z), u8_clr(c.rgb.y), u8_clr(c.rgb.x), u8_clr(c.sun)];
            bytes.extend_from_slice(&col);
            bytes.extend_from_slice(&tu.0.to_le_bytes());
            bytes.extend_from_slice(&tv.0.to_le_bytes());
        } else {
            let lm = v.uv.get(1).copied().unwrap_or(Vec2::ZERO);
            bytes.extend_from_slice(&tu.0.to_le_bytes());
            bytes.extend_from_slice(&tv.0.to_le_bytes());
            bytes.extend_from_slice(&s16_tc_lmap(lm.x).to_le_bytes());
            bytes.extend_from_slice(&s16_tc_lmap(lm.y).to_le_bytes());
        }
    }
    let flat: Vec<u16> = o.data.faces.iter().flatten().copied().collect();
    let x_place = fast.map(|(b, f)| (tree.xvb.add(&decl_x(), b), tree.xib.register(&f)));
    let d = if vertex_colored { decl_vert() } else { decl_lmap() };
    let (vb, vs) = tree.vb.add(&d, bytes);
    let (ib, is) = tree.ib.register(&flat);
    if let Some(TreeNode { item: Item::Ogf(o), .. }) = tree.nodes.get_mut(id) {
        o.data.vb_id = vb;
        o.data.vb_start = vs;
        o.data.ib_id = ib;
        o.data.ib_start = is;
        if let Some(((xv, xs), (xi, xis))) = x_place {
            o.fast.vb_id = xv;
            o.fast.vb_start = xs;
            o.fast.ib_id = xi;
            o.fast.ib_start = xis;
        }
    }
}

fn container(w: &mut Writer, vb: u32, vs: u32, vc: usize, ib: u32, is: u32, ic: usize) {
    w.chunk(OGF_GCONTAINER, |w| {
        w.w_u32(vb);
        w.w_u32(vs);
        w.w_u32(vc as u32);
        w.w_u32(ib);
        w.w_u32(is);
        w.w_u32(ic as u32);
    });
}

fn swidata(w: &mut Writer, s: &[SlideWindow]) {
    w.chunk(OGF_SWIDATA, |w| {
        for _ in 0..4 {
            w.w_u32(0);
        }
        w.w_u32(s.len() as u32);
        write_windows(w, s);
    });
}

fn save_item(g: &LcGlobalData, extra: &Extra, tree: &mut Tree, id: usize, w: &mut Writer) {
    let Some(node) = tree.nodes.get(id).cloned() else { return };
    match &node.item {
        Item::Ogf(o) => {
            let sid =
                tree.register_shader(&shader_name(g, extra, u32::from(o.material), &tex_list(&o.textures)));
            let ty = if o.progressive() { MT_PROGRESSIVE } else { MT_NORMAL };
            container(
                w,
                o.data.vb_id,
                o.data.vb_start,
                o.data.vertices.len(),
                o.data.ib_id,
                o.data.ib_start,
                o.data.faces.len() * 3,
            );
            if ty == MT_PROGRESSIVE {
                swidata(w, &o.data.swi);
            }
            if !o.fast.vertices.is_empty() && !o.fast.faces.is_empty() {
                w.chunk(OGF_FASTPATH, |w| {
                    container(
                        w,
                        o.fast.vb_id,
                        o.fast.vb_start,
                        o.fast.vertices.len(),
                        o.fast.ib_id,
                        o.fast.ib_start,
                        o.fast.faces.len() * 3,
                    );
                    if ty == MT_PROGRESSIVE {
                        swidata(w, &o.fast.swi);
                    }
                });
            }
            header(w, ty, sid, &node.base);
        }
        Item::Ref(r) => {
            let sid =
                tree.register_shader(&shader_name(g, extra, u32::from(r.material), &tex_list(&r.textures)));
            let model = tree.mu.get(r.model.0).and_then(|m| m.get(r.model.1));
            let pm = model.is_some_and(Ogf::progressive);
            let (vc, fc) = model.map_or((0, 0), |m| (m.data.vertices.len(), m.data.faces.len()));
            container(w, r.vb_id, r.vb_start, vc, r.ib_id, r.ib_start, fc * 3);
            w.chunk(OGF_TREEDEF2, |w| {
                for row in r.xform.m {
                    for v in row {
                        w.w_f32(v);
                    }
                }
                for c in [&r.c_scale, &r.c_bias] {
                    w.w_f32(c.rgb.x);
                    w.w_f32(c.rgb.y);
                    w.w_f32(c.rgb.z);
                    w.w_f32(c.hemi);
                    w.w_f32(c.sun);
                }
            });
            header(w, if pm { MT_TREE_PM } else { MT_TREE_ST }, sid, &node.base);
            if pm {
                w.chunk(OGF_SWICONTAINER, |w| w.w_u32(r.sw_id));
            }
        }
        Item::Node(ch) => {
            header(w, MT_HIERRARHY, 0, &node.base);
            w.chunk(OGF_CHILDREN_L, |w| {
                w.w_u32(ch.len() as u32);
                for c in ch {
                    w.w_u32(*c);
                }
            });
        }
        Item::Lod { children, material, faces } => {
            let tex = g
                .materials
                .get(*material as usize)
                .and_then(|m| g.textures.get(usize::from(m.surface)))
                .map_or(SmolStr::default(), |t| t.name.clone());
            let sid = tree.register_shader(&shader_name(g, extra, *material, &tex));
            header(w, MT_LOD, sid, &node.base);
            w.chunk(OGF_CHILDREN_L, |w| {
                w.w_u32(children.len() as u32);
                for c in children {
                    w.w_u32(*c);
                }
            });
            w.chunk(OGF_LODDEF2, |w| {
                for f in faces.iter() {
                    for v in f {
                        w.w_vec3(v.v);
                        w.w_vec2(v.t);
                        w.w_u32(v.c_rgb_hemi);
                        // oracle: the struct's three bytes of padding are whatever the C++ had
                        w.w(&[v.c_sun, 0, 0, 0]);
                    }
                }
            });
        }
    }
}

/// `CBuild::SaveTREE`: the visuals' buffers in the order of their first texture, the visuals in
/// tree order, the two geometry files, the shader table.
pub fn save_tree(
    g: &LcGlobalData,
    extra: &Extra,
    tree: &mut Tree,
    level: &mut Writer,
    log: &mut Log,
) -> (Vec<u8>, Vec<u8>) {
    let mut remap: Vec<usize> =
        (0..tree.nodes.len()).filter(|&i| matches!(tree.nodes[i].item, Item::Ogf(_))).collect();
    let name = |i: usize| match &tree.nodes[i].item {
        Item::Ogf(o) => o.textures.first().map(|t| t.name.clone()).unwrap_or_default(),
        _ => SmolStr::default(),
    };
    let names: Vec<SmolStr> = remap.iter().map(|&i| name(i)).collect();
    let mut order: Vec<usize> = (0..remap.len()).collect();
    order.sort_by(|&a, &b| names[a].as_bytes().cmp(names[b].as_bytes()));
    remap = order.into_iter().map(|k| remap[k]).collect();
    log.msg(format!("remap-size: {} / {}", remap.len(), tree.nodes.len()));
    for &id in &remap {
        pre_save(g, tree, id);
    }
    let mut visuals = Writer::new();
    for id in 0..tree.nodes.len() {
        visuals.chunk(id as u32, |w| save_item(g, extra, tree, id, w));
    }
    level.chunk(3, |w| w.w(visuals.bytes()));
    let geom = |vb: &VbContainer, ib: &IbContainer, swi: &[Vec<SlideWindow>]| {
        let mut w = Writer::new();
        w.chunk(1, |w| {
            w.w_u16(14);
            // oracle: the C++ leaves the quality of the geometry files' header uninitialised
            w.w_u16(0);
        });
        w.chunk(9, |w| vb.save(w));
        w.chunk(10, |w| ib.save(w));
        w.chunk(11, |w| save_swi(swi, w));
        w.into_bytes()
    };
    let main = geom(&tree.vb, &tree.ib, &tree.swi);
    let fast = geom(&tree.xvb, &tree.xib, &[]);
    level.chunk(2, |w| {
        w.w_u32(tree.shaders.len() as u32);
        for s in &tree.shaders {
            w.w_stringz(s);
        }
    });
    (main, fast)
}

/// `CBuild::SaveSectors`.
pub fn save_sectors(sectors: &[Sector], level: &mut Writer) {
    let mut m = Writer::new();
    for (i, s) in sectors.iter().enumerate() {
        m.chunk(i as u32, |w| {
            w.chunk(2, |w| w.w_u32(s.root as u32));
            w.chunk(1, |w| {
                for p in &s.portals {
                    w.w_u16(*p);
                }
            });
        });
    }
    level.chunk(8, |w| w.w(m.bytes()));
}

/// The glows chunk: position, size and the shader of each.
pub fn save_glows(g: &LcGlobalData, extra: &Extra, tree: &mut Tree, level: &mut Writer) {
    let mut ids = Vec::with_capacity(extra.glows.len());
    for gl in &extra.glows {
        let m = g.materials.get(gl.material as usize);
        let sh = m.and_then(|m| extra.shader_render.get(usize::from(m.shader))).map_or("", SmolStr::as_str);
        let tx = m.and_then(|m| g.textures.get(usize::from(m.surface))).map_or("", |t| t.name.as_str());
        ids.push(tree.register_shader(&format!("{sh}/{tx}")));
    }
    level.chunk(7, |w| {
        for (gl, id) in extra.glows.iter().zip(ids) {
            w.w_vec3(gl.pos);
            w.w_f32(gl.size);
            w.w_u16(id);
        }
    });
}

/// `CBuild::SaveLights`.
pub fn save_lights(extra: &Extra, level: &mut Writer) {
    level.chunk(6, |w| {
        for l in &extra.lights_dynamic {
            w.w_u32(l.light.controller);
            write_flight(w, &l.light.light);
        }
    });
}

fn write_flight(w: &mut Writer, l: &gz_fmt_level::Light) {
    w.w_u32(l.kind);
    for c in [l.diffuse, l.specular, l.ambient] {
        for v in c {
            w.w_f32(v);
        }
    }
    w.w_vec3(l.position);
    w.w_vec3(l.direction);
    for v in [l.range, l.falloff, l.attenuation0, l.attenuation1, l.attenuation2, l.theta, l.phi] {
        w.w_f32(v);
    }
}

/// `CBuild::BuildPortals`.
pub fn save_portals(extra: &Extra, level: &mut Writer) {
    level.chunk(4, |w| {
        for p in &extra.portals {
            w.w_u16(p.sector_front);
            w.w_u16(p.sector_back);
            for v in p.vertices {
                w.w_vec3(v);
            }
            w.w_u32(p.count);
        }
    });
}

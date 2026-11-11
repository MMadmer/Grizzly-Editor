//! A visual of any type: `dxRender_Visual::Load`, `Fvisual`, `FProgressive`,
//! `FHierrarhyVisual`, `FLOD`, `FTreeVisual`, `CSkeletonX`, `CKinematics`,
//! `CKinematicsAnimated`.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_chunk::Reader;
use smol_str::SmolStr;

use crate::bones::{BoneIk, BoneName, read_ik, read_names};
use crate::format::{Desc, FORMAT_VERSION, Header, MT, chunk};
use crate::motion::MotionSet;
use crate::vertex::VertexBuffer;
use crate::{OgfError, Result, invalid};

/// Children nest; a file deeper than this is not a model.
const MAX_DEPTH: u32 = 16;

/// A range of a buffer the level holds (`OGF_VCONTAINER`, `OGF_ICONTAINER`, `OGF_GCONTAINER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferRef {
    pub id: u32,
    pub base: u32,
    pub count: u32,
}

/// `FSlideWindow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideWindow {
    pub offset: u32,
    pub num_tris: u16,
    pub num_verts: u16,
}

/// `OGF_SWIDATA`: the levels of detail of a progressive mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct SlideWindows {
    pub reserved: [u32; 4],
    pub windows: Vec<SlideWindow>,
}

/// `FLOD::_vertex` (28 bytes with its padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodVertex {
    pub position: Vec3,
    pub uv: Vec2,
    /// RGB and hemisphere light.
    pub rgb_hemi: u32,
    pub sun: u8,
}

/// One of the eight billboards of a LOD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodFacet {
    pub vertices: [LodVertex; 4],
}

/// `OGF_TREEDEF2`: the placement and the light of a tree, as stored (the engine halves the
/// colours when it loads them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeDef {
    pub xform: [[f32; 4]; 4],
    /// RGB, hemisphere, sun.
    pub scale: [f32; 5],
    pub bias: [f32; 5],
}

/// A visual and everything it carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Visual {
    pub header: Header,
    /// Texture and shader.
    pub texture: Option<(SmolStr, SmolStr)>,
    pub vertices: Option<VertexBuffer>,
    pub indices: Option<Arc<[u16]>>,
    pub swi: Option<SlideWindows>,
    pub vcontainer: Option<BufferRef>,
    pub icontainer: Option<BufferRef>,
    /// Vertices and indices in the level's buffers.
    pub gcontainer: Option<(BufferRef, BufferRef)>,
    /// `OGF_FASTPATH`: the position-only copy for shadows.
    pub fast_path: Option<(BufferRef, BufferRef)>,
    pub fast_path_swi: Option<SlideWindows>,
    /// `OGF_SWICONTAINER`: a slide window list of the level.
    pub swi_container: Option<u32>,
    pub children: Vec<Visual>,
    /// `OGF_CHILDREN_L`: indices of visuals of the level.
    pub child_refs: Vec<u32>,
    pub lod: Option<Box<[LodFacet; 8]>>,
    pub tree: Option<TreeDef>,
    pub desc: Option<Desc>,
    pub bones: Vec<BoneName>,
    pub ik: Vec<BoneIk>,
    pub user_data: Option<String>,
    /// `OGF_S_LODS`: the model a skeleton's LOD is.
    pub lod_ref: Option<SmolStr>,
    /// `OGF_S_MOTION_REFS` (a comma list) or `OGF_S_MOTION_REFS2`.
    pub motion_refs: Vec<SmolStr>,
    pub motions: Option<MotionSet>,
    pub collision_vertices: Option<Arc<[Vec3]>>,
    pub collision_indices: Option<Arc<[u32]>>,
}

fn body<'a>(r: &Reader<'a>, id: u32) -> Result<Option<gz_chunk::Chunk<'a>>> {
    Ok(r.open_chunk(id)?)
}

/// A chunk the engine reads with `find_chunk` / `r_chunk_safe`: its stored bytes, whatever its
/// compression mark says (only `open_chunk` unpacks). Dead Air's `dev_pda_hud.ogf` marks a
/// plain header and description as compressed.
fn raw<'a>(r: &Reader<'a>, id: u32) -> Result<Option<gz_chunk::Chunk<'a>>> {
    Ok(r.find_chunk(id).map(|(b, _)| gz_chunk::Chunk::borrowed(b)))
}

fn buffer_ref(r: &mut Reader<'_>) -> Result<BufferRef> {
    Ok(BufferRef { id: r.r_u32()?, base: r.r_u32()?, count: r.r_u32()? })
}

fn swi(body: &[u8]) -> Result<SlideWindows> {
    let mut r = Reader::new(body);
    let reserved = [r.r_u32()?, r.r_u32()?, r.r_u32()?, r.r_u32()?];
    let n = r.r_u32()? as usize;
    if n.saturating_mul(8) > r.elapsed() {
        return Err(invalid(format!("the model states {n} slide windows and cannot hold them")));
    }
    let windows = (0..n)
        .map(|_| Ok(SlideWindow { offset: r.r_u32()?, num_tris: r.r_u16()?, num_verts: r.r_u16()? }))
        .collect::<Result<_>>()?;
    Ok(SlideWindows { reserved, windows })
}

fn u32_list(r: &mut Reader<'_>, n: usize, what: &str) -> Result<Vec<u32>> {
    if n.saturating_mul(4) > r.elapsed() {
        return Err(invalid(format!("the model states {n} {what} and cannot hold them")));
    }
    (0..n).map(|_| Ok(r.r_u32()?)).collect()
}

impl Visual {
    /// Reads a model: an `.ogf` file or a child of one.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_at(bytes, 0)
    }

    fn read_at(bytes: &[u8], depth: u32) -> Result<Self> {
        if depth > MAX_DEPTH {
            return Err(invalid("the model's children nest too deep"));
        }
        let r = Reader::new(bytes);
        let header = {
            let c = raw(&r, chunk::HEADER)?.ok_or_else(|| invalid("the model has no header"))?;
            Header::read(&mut c.reader())?
        };
        if header.format_version != FORMAT_VERSION {
            return Err(OgfError::Version(header.format_version));
        }
        let mut v = Visual {
            header,
            texture: None,
            vertices: None,
            indices: None,
            swi: None,
            vcontainer: None,
            icontainer: None,
            gcontainer: None,
            fast_path: None,
            fast_path_swi: None,
            swi_container: None,
            children: Vec::new(),
            child_refs: Vec::new(),
            lod: None,
            tree: None,
            desc: None,
            bones: Vec::new(),
            ik: Vec::new(),
            user_data: None,
            lod_ref: None,
            motion_refs: Vec::new(),
            motions: None,
            collision_vertices: None,
            collision_indices: None,
        };
        if let Some(c) = raw(&r, chunk::TEXTURE)? {
            let mut t = c.reader();
            v.texture = Some((SmolStr::new(t.r_stringz()?), SmolStr::new(t.r_stringz()?)));
        }
        let skinned = matches!(header.kind, MT::SKELETON_GEOMDEF_PM | MT::SKELETON_GEOMDEF_ST);
        if let Some(c) = raw(&r, chunk::VERTICES)? {
            v.vertices = Some(VertexBuffer::read(c.bytes(), skinned)?);
        }
        if let Some(c) = raw(&r, chunk::INDICES)? {
            let mut ir = c.reader();
            let n = ir.r_u32()? as usize;
            if n.saturating_mul(2) > ir.elapsed() {
                return Err(invalid(format!("the model states {n} indices and cannot hold them")));
            }
            v.indices = Some((0..n).map(|_| Ok(ir.r_u16()?)).collect::<Result<Vec<_>>>()?.into());
        }
        if let Some(c) = body(&r, chunk::SWIDATA)? {
            v.swi = Some(swi(c.bytes())?);
        }
        if let Some(c) = raw(&r, chunk::VCONTAINER)? {
            v.vcontainer = Some(buffer_ref(&mut c.reader())?);
        }
        if let Some(c) = raw(&r, chunk::ICONTAINER)? {
            v.icontainer = Some(buffer_ref(&mut c.reader())?);
        }
        if let Some(c) = body(&r, chunk::GCONTAINER)? {
            let mut g = c.reader();
            v.gcontainer = Some((buffer_ref(&mut g)?, buffer_ref(&mut g)?));
        }
        if let Some(c) = body(&r, chunk::FASTPATH)? {
            let f = c.reader();
            if let Some(g) = f.open_chunk(chunk::GCONTAINER)? {
                let mut g = g.reader();
                v.fast_path = Some((buffer_ref(&mut g)?, buffer_ref(&mut g)?));
            }
            if let Some(s) = f.open_chunk(chunk::SWIDATA)? {
                v.fast_path_swi = Some(swi(s.bytes())?);
            }
        }
        if let Some(c) = raw(&r, chunk::SWICONTAINER)? {
            v.swi_container = Some(c.reader().r_u32()?);
        }
        if let Some(c) = body(&r, chunk::CHILDREN)? {
            let cr = c.reader();
            for (i, (id, _, _)) in cr.chunks().enumerate() {
                if id as usize != i {
                    break;
                }
                let child = cr.open_chunk(id)?.ok_or_else(|| invalid("a child of the model is missing"))?;
                v.children.push(Self::read_at(child.bytes(), depth + 1)?);
            }
        }
        if let Some(c) = body(&r, chunk::CHILDREN_L)? {
            let mut cr = c.reader();
            let n = cr.r_u32()? as usize;
            v.child_refs = u32_list(&mut cr, n, "child references")?;
        }
        if let Some(c) = raw(&r, chunk::LODDEF2)? {
            let mut lr = c.reader();
            if lr.elapsed() < 8 * 4 * 28 {
                return Err(invalid("the LOD's billboards are cut short"));
            }
            let mut facets = [LodFacet {
                vertices: [LodVertex { position: Vec3::ZERO, uv: Vec2::ZERO, rgb_hemi: 0, sun: 0 }; 4],
            }; 8];
            for f in &mut facets {
                for lv in &mut f.vertices {
                    *lv = LodVertex {
                        position: lr.r_vec3()?,
                        uv: lr.r_vec2()?,
                        rgb_hemi: lr.r_u32()?,
                        sun: lr.r_u8()?,
                    };
                    lr.advance(3)?;
                }
            }
            v.lod = Some(Box::new(facets));
        }
        if let Some(c) = raw(&r, chunk::TREEDEF2)? {
            let mut tr = c.reader();
            let mut xform = [[0.0f32; 4]; 4];
            for row in &mut xform {
                for x in row.iter_mut() {
                    *x = tr.r_f32()?;
                }
            }
            let five = |tr: &mut Reader<'_>| -> Result<[f32; 5]> {
                Ok([tr.r_f32()?, tr.r_f32()?, tr.r_f32()?, tr.r_f32()?, tr.r_f32()?])
            };
            let scale = five(&mut tr)?;
            let bias = five(&mut tr)?;
            v.tree = Some(TreeDef { xform, scale, bias });
        }
        if let Some(c) = raw(&r, chunk::S_DESC)? {
            // the game never reads it; a description that is not one (Dead Air's
            // `dev_pda_hud.ogf` holds a line of text there) leaves the model readable
            v.desc = Desc::read(c.bytes()).ok();
        }
        if let Some(c) = raw(&r, chunk::S_BONE_NAMES)? {
            v.bones = read_names(c.bytes())?;
        }
        if let Some(c) = body(&r, chunk::S_IKDATA)? {
            v.ik = read_ik(c.bytes(), v.bones.len())?;
        }
        if let Some(c) = body(&r, chunk::S_USERDATA)? {
            // the engine reads the chunk as an ini (`CInifile` over the chunk): a text that ends
            // with the chunk, without a terminator, is whole too
            let bytes = c.bytes();
            let text = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
            v.user_data = Some(gz_base::text::decode_guess(text).into_owned());
        }
        if let Some(c) = body(&r, chunk::S_LODS)? {
            v.lod_ref = Some(SmolStr::new(c.reader().r_line()));
        }
        if let Some(c) = raw(&r, chunk::S_MOTION_REFS)? {
            let list = c.reader().r_stringz()?;
            v.motion_refs =
                list.split(',').map(|s| SmolStr::new(s.trim())).filter(|s| !s.is_empty()).collect();
        }
        if let Some(c) = raw(&r, chunk::S_MOTION_REFS2)? {
            let mut mr = c.reader();
            let n = mr.r_u32()? as usize;
            if n > mr.elapsed() {
                return Err(invalid(format!("the model states {n} motion references and cannot hold them")));
            }
            v.motion_refs = (0..n).map(|_| Ok(SmolStr::new(mr.r_stringz()?))).collect::<Result<_>>()?;
        }
        let params = body(&r, chunk::S_SMPARAMS)?;
        let keys = body(&r, chunk::S_MOTIONS)?;
        if params.is_some() || keys.is_some() {
            let count = (!v.bones.is_empty()).then_some(v.bones.len());
            v.motions = Some(MotionSet::read_chunks(
                params.as_ref().map(|c| c.bytes()),
                keys.as_ref().map(|c| c.bytes()),
                count,
            )?);
        }
        if let Some(c) = body(&r, chunk::COLLISION_VERTICES)? {
            let mut cr = c.reader();
            let n = cr.elapsed() / 12;
            v.collision_vertices = Some((0..n).map(|_| Ok(cr.r_vec3()?)).collect::<Result<Vec<_>>>()?.into());
        }
        if let Some(c) = body(&r, chunk::COLLISION_INDICES)? {
            let mut cr = c.reader();
            let n = cr.elapsed() / 4;
            v.collision_indices = Some(u32_list(&mut cr, n, "collision indices")?.into());
        }
        Ok(v)
    }

    /// The type of the visual (`MT`).
    #[must_use]
    pub fn kind(&self) -> u8 {
        self.header.kind
    }

    /// Every visual with geometry, this one and its children, depth first.
    #[must_use]
    pub fn meshes(&self) -> Vec<&Visual> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(v) = stack.pop() {
            if v.vertices.is_some() {
                out.push(v);
            }
            stack.extend(v.children.iter().rev());
        }
        out
    }
}

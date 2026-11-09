//! A whole `.object` (`EditObject.h`, `EditObjectIO.cpp`, `EditObjectMotions.cpp`).

use glam::Vec3;
use gz_chunk::{Chunk, ChunkError, Reader, Writer};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::bone::Bone;
use crate::io::{Text, TextEncoding, checked_count, chunk, chunk_u32, need_chunk, w_str};
use crate::mesh::Mesh;
use crate::motion::{SMotion, with_text};
use crate::xmath::Aabb;
use crate::{ObjectError, Result};

const OBJECT_VERSION: u32 = 0x0010;
const CHUNK_OBJECT_BODY: u32 = 0x7777;
const CHUNK_VERSION: u32 = 0x0900;
const CHUNK_FLAGS: u32 = 0x0903;
const CHUNK_SURFACES: u32 = 0x0905;
const CHUNK_SURFACES2: u32 = 0x0906;
const CHUNK_SURFACES3: u32 = 0x0907;
const CHUNK_EDITMESHES: u32 = 0x0910;
const CHUNK_CLASSSCRIPT: u32 = 0x0912;
const CHUNK_BONES: u32 = 0x0913;
const CHUNK_SMOTIONS: u32 = 0x0916;
const CHUNK_SURFACES_XRLC: u32 = 0x0918;
const CHUNK_BONEPARTS: u32 = 0x0919;
const CHUNK_ACTORTRANSFORM: u32 = 0x0920;
const CHUNK_BONES2: u32 = 0x0921;
const CHUNK_DESC: u32 = 0x0922;
const CHUNK_BONEPARTS2: u32 = 0x0923;
const CHUNK_SMOTIONS2: u32 = 0x0924;
const CHUNK_LODS: u32 = 0x0925;
const CHUNK_SMOTIONS3: u32 = 0x0926;

bitflags::bitflags! {
    /// What kind of object this is.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct ObjectFlags: u32 {
        const DYNAMIC = 1 << 0;
        const PROGRESSIVE = 1 << 1;
        const USING_LOD = 1 << 2;
        const HOM = 1 << 3;
        const MULTIPLE_USAGE = 1 << 4;
        const SOUND_OCCLUDER = 1 << 5;
        const HQ_EXPORT = 1 << 6;
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct SurfaceFlags: u32 {
        const TWO_SIDED = 1 << 0;
    }
}

/// `CSurface`: a material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub name: SmolStr,
    /// Without an extension.
    pub texture: SmolStr,
    pub vmap: SmolStr,
    /// The engine shader.
    pub shader: SmolStr,
    /// The compiler shader.
    pub shader_xrlc: SmolStr,
    pub game_mtl: SmolStr,
    pub flags: SurfaceFlags,
    /// The D3D FVF of the surface; its texture count is what exporters look at.
    pub fvf: u32,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            name: SmolStr::default(),
            texture: SmolStr::default(),
            vmap: SmolStr::default(),
            shader: SmolStr::default(),
            shader_xrlc: SmolStr::default(),
            game_mtl: SmolStr::new_static("default"),
            flags: SurfaceFlags::empty(),
            fvf: 0,
        }
    }
}

impl Surface {
    /// `SetTexture`: a texture is named without its extension.
    pub fn set_texture(&mut self, name: &str) {
        self.texture = SmolStr::new(match name.rfind('.') {
            Some(dot) => &name[..dot],
            None => name,
        });
    }

    /// Texture coordinate sets in the FVF.
    #[must_use]
    pub fn tex_count(&self) -> u32 {
        (self.fvf & 0x0f00) >> 8
    }

    #[must_use]
    pub fn two_sided(&self) -> bool {
        self.flags.contains(SurfaceFlags::TWO_SIDED)
    }
}

/// `SBonePart`: a named group of bones that motions can play on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BonePart {
    pub alias: SmolStr,
    pub bones: Vec<SmolStr>,
}

/// `CEditableObject`.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub flags: ObjectFlags,
    /// The user data ("class script"): an ini text the game reads from the model.
    pub user_data: String,
    /// The LOD reference.
    pub lod_ref: SmolStr,
    pub surfaces: Vec<Surface>,
    pub meshes: Vec<Mesh>,
    /// In hierarchy order: the root first, every parent before its children.
    pub bones: Vec<Bone>,
    pub motions: Vec<SMotion>,
    pub motion_refs: Vec<SmolStr>,
    pub bone_parts: Vec<BonePart>,
    /// Applied to the root bone when motions are exported.
    pub actor_position: Vec3,
    pub actor_rotation: Vec3,
    pub created_by: SmolStr,
    pub created_at: u32,
    pub modified_by: SmolStr,
    pub modified_at: u32,
    /// How the strings of the file this came from are stored; [`Object::write`] uses it.
    pub encoding: TextEncoding,
}

impl Default for Object {
    fn default() -> Self {
        Self {
            flags: ObjectFlags::empty(),
            user_data: String::new(),
            lod_ref: SmolStr::default(),
            surfaces: Vec::new(),
            meshes: Vec::new(),
            bones: Vec::new(),
            motions: Vec::new(),
            motion_refs: Vec::new(),
            bone_parts: Vec::new(),
            actor_position: Vec3::ZERO,
            actor_rotation: Vec3::ZERO,
            created_by: SmolStr::new_static("unknown"),
            created_at: 0,
            modified_by: SmolStr::new_static("unknown"),
            modified_at: 0,
            encoding: TextEncoding::Utf8,
        }
    }
}

fn invalid(what: impl std::fmt::Display) -> ObjectError {
    ObjectError::Invalid(what.to_string())
}

/// The sub-chunks `0, 1, 2, ...` of a container, up to the first id that is missing.
fn numbered_chunks<'a>(r: &Reader<'a>) -> Result<Vec<Chunk<'a>>> {
    let mut out = Vec::new();
    for id in 0u32.. {
        match r.open_chunk(id)? {
            Some(c) if !c.bytes().is_empty() => out.push(c),
            _ => break,
        }
    }
    Ok(out)
}

impl Object {
    /// Reads a `.object` file.
    pub fn read(bytes: &[u8]) -> Result<Object> {
        let file = Reader::new(bytes);
        let body = file.open_chunk(CHUNK_OBJECT_BODY)?.filter(|c| !c.bytes().is_empty()).ok_or(ObjectError::NotAnObject)?;
        let (mut object, encoding) = with_text(|text| Self::read_body(&body.reader(), text))?;
        object.encoding = encoding;
        Ok(object)
    }

    /// `CEditableObject::Load(IReader&)`.
    fn read_body(f: &Reader<'_>, text: &mut Text) -> Result<Object> {
        let version = chunk_u32(f, CHUNK_VERSION).ok_or(ChunkError::Missing(CHUNK_VERSION))?;
        if version != OBJECT_VERSION {
            return Err(ObjectError::Version { what: "The object", version });
        }
        let flags = chunk_u32(f, CHUNK_FLAGS).ok_or(ChunkError::Missing(CHUNK_FLAGS))?;
        let mut o = Object { flags: ObjectFlags::from_bits_retain(flags), ..Object::default() };

        if let Some(mut c) = chunk(f, CHUNK_CLASSSCRIPT) {
            o.user_data = text.r_str(&mut c)?.to_string();
        }
        if let Some(mut c) = chunk(f, CHUNK_LODS) {
            o.lod_ref = text.r_str(&mut c)?;
        }
        o.surfaces = Self::read_surfaces(f, text)?;

        // meshes: independent of one another, so read side by side
        if let Some(meshes) = f.open_chunk(CHUNK_EDITMESHES)?.filter(|c| !c.bytes().is_empty()) {
            let chunks = numbered_chunks(&meshes.reader())?;
            let surfaces = &o.surfaces;
            let base = *text;
            let read: Vec<Result<(Mesh, bool)>> = chunks
                .par_iter()
                .map(|c| {
                    let mut t = base;
                    Mesh::read(&c.reader(), surfaces, &mut t).map(|m| (m, t.non_utf8))
                })
                .collect();
            for r in read {
                let (mesh, non_utf8) = r?;
                text.non_utf8 |= non_utf8;
                o.meshes.push(mesh);
            }
        }

        // bones
        if let Some(bones) = f.open_chunk(CHUNK_BONES2)?.filter(|c| !c.bytes().is_empty()) {
            for c in numbered_chunks(&bones.reader())? {
                o.bones.push(Bone::read(&c.reader(), text)?);
            }
            o.prepare_bones()?;
        } else if let Some(mut c) = chunk(f, CHUNK_BONES) {
            let count = checked_count(&c, c.clone().r_u32()? as usize, 31)?;
            c.advance(4)?;
            for _ in 0..count {
                o.bones.push(Bone::read_v0(&mut c, text)?);
            }
            o.prepare_bones()?;
        }

        // skeletal motions
        if let Some(mut c) = chunk(f, CHUNK_SMOTIONS) {
            let count = checked_count(&c, c.clone().r_u32()? as usize, 32)?;
            c.advance(4)?;
            for _ in 0..count {
                let mut m = SMotion::read(&mut c, text)?;
                m.sort_bones_by_skeleton(&o.bones);
                o.motions.push(m);
            }
        }
        if let Some(mut c) = chunk(f, CHUNK_SMOTIONS2) {
            let list = text.r_str(&mut c)?;
            o.motion_refs = gz_base::text::split_list(&list).map(SmolStr::new).collect();
        } else if let Some(mut c) = chunk(f, CHUNK_SMOTIONS3) {
            let count = checked_count(&c, c.clone().r_u32()? as usize, 1)?;
            c.advance(4)?;
            for _ in 0..count {
                o.motion_refs.push(text.r_str(&mut c)?);
            }
        }

        // bone parts
        if let Some(mut c) = chunk(f, CHUNK_BONEPARTS) {
            let count = checked_count(&c, c.clone().r_u32()? as usize, 5)?;
            c.advance(4)?;
            let mut ok = true;
            'parts: for _ in 0..count {
                let mut part = BonePart { alias: text.r_str(&mut c)?, bones: Vec::new() };
                let n = checked_count(&c, c.clone().r_u32()? as usize, 4)?;
                c.advance(4)?;
                for _ in 0..n {
                    match o.bones.get(c.r_u32()? as usize) {
                        Some(b) => part.bones.push(b.name.clone()),
                        None => {
                            ok = false;
                            break 'parts;
                        }
                    }
                }
                o.bone_parts.push(part);
            }
            if !ok {
                o.bone_parts.clear();
            }
            o.verify_bone_parts();
        } else if let Some(mut c) = chunk(f, CHUNK_BONEPARTS2) {
            let count = checked_count(&c, c.clone().r_u32()? as usize, 5)?;
            c.advance(4)?;
            for _ in 0..count {
                let mut part = BonePart { alias: text.r_str(&mut c)?, bones: Vec::new() };
                let n = checked_count(&c, c.clone().r_u32()? as usize, 1)?;
                c.advance(4)?;
                for _ in 0..n {
                    part.bones.push(text.r_str(&mut c)?);
                }
                o.bone_parts.push(part);
            }
            o.verify_bone_parts();
        }

        if let Some(mut c) = chunk(f, CHUNK_ACTORTRANSFORM) {
            o.actor_position = c.r_vec3()?;
            o.actor_rotation = c.r_vec3()?;
        }
        if let Some(mut c) = chunk(f, CHUNK_DESC) {
            o.created_by = text.r_str(&mut c)?;
            o.created_at = c.r_u32()?;
            o.modified_by = text.r_str(&mut c)?;
            o.modified_at = c.r_u32()?;
        }
        o.verify_mesh_names();
        Ok(o)
    }

    fn read_surfaces(f: &Reader<'_>, text: &mut Text) -> Result<Vec<Surface>> {
        let (mut c, form) = if let Some(c) = chunk(f, CHUNK_SURFACES3) {
            (c, 3)
        } else if let Some(c) = chunk(f, CHUNK_SURFACES2) {
            (c, 2)
        } else {
            (need_chunk(f, CHUNK_SURFACES)?, 1)
        };
        let count = checked_count(&c, c.clone().r_u32()? as usize, 17)?;
        c.advance(4)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let mut s = Surface { name: text.r_str(&mut c)?, ..Surface::default() };
            let tc_count;
            if form == 1 {
                s.shader = text.r_str(&mut c)?;
                s.flags.set(SurfaceFlags::TWO_SIDED, c.r_u8()? != 0);
                s.fvf = c.r_u32()?;
                tc_count = c.r_u32()?;
                let texture = text.r_str(&mut c)?;
                s.set_texture(&texture);
                s.vmap = text.r_str(&mut c)?;
                s.shader_xrlc = SmolStr::new_static("default");
            } else {
                s.shader = text.r_str(&mut c)?;
                s.shader_xrlc = text.r_str(&mut c)?;
                if form == 3 {
                    s.game_mtl = text.r_str(&mut c)?;
                }
                let texture = text.r_str(&mut c)?;
                s.set_texture(&texture);
                s.vmap = text.r_str(&mut c)?;
                s.flags = SurfaceFlags::from_bits_retain(c.r_u32()?);
                s.fvf = c.r_u32()?;
                tc_count = c.r_u32()?;
            }
            // the C++ stops on both
            if s.shader.is_empty() {
                return Err(invalid(format_args!("Surface '{}' has no shader.", s.name)));
            }
            if tc_count < 1 {
                return Err(invalid(format_args!("Surface '{}' has no texture coordinates.", s.name)));
            }
            out.push(s);
        }
        if form == 1 {
            if let Some(mut c) = chunk(f, CHUNK_SURFACES_XRLC) {
                for s in &mut out {
                    s.shader_xrlc = text.r_str(&mut c)?;
                }
            }
        }
        Ok(out)
    }

    /// Writes a `.object` file: `CEditableObject::Save`.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let enc = self.encoding;
        let mut w = Writer::new();
        w.open_chunk(CHUNK_OBJECT_BODY);

        w.chunk(CHUNK_VERSION, |w| w.w_u16(OBJECT_VERSION as u16));
        w.chunk(CHUNK_CLASSSCRIPT, |w| w_str(w, &self.user_data, enc));
        w.chunk(CHUNK_LODS, |w| w_str(w, &self.lod_ref, enc));
        w.chunk(CHUNK_FLAGS, |w| w.w_u32(self.flags.bits()));

        w.chunk(CHUNK_EDITMESHES, |w| {
            for (i, m) in self.meshes.iter().enumerate() {
                w.chunk(i as u32, |w| m.write(w, &self.surfaces, enc));
            }
        });

        w.chunk(CHUNK_SURFACES3, |w| {
            w.w_u32(self.surfaces.len() as u32);
            for s in &self.surfaces {
                w_str(w, &s.name, enc);
                w_str(w, &s.shader, enc);
                w_str(w, &s.shader_xrlc, enc);
                w_str(w, &s.game_mtl, enc);
                w_str(w, &s.texture, enc);
                w_str(w, &s.vmap, enc);
                w.w_u32(s.flags.bits());
                w.w_u32(s.fvf);
                w.w_u32(1);
            }
        });

        if !self.bones.is_empty() {
            w.chunk(CHUNK_BONES2, |w| {
                for (i, b) in self.bones.iter().enumerate() {
                    w.chunk(i as u32, |w| b.write(w, enc));
                }
            });
        }
        if !self.motions.is_empty() {
            w.chunk(CHUNK_SMOTIONS, |w| {
                w.w_u32(self.motions.len() as u32);
                for m in &self.motions {
                    m.write(w, enc);
                }
            });
        }
        if !self.motion_refs.is_empty() {
            w.chunk(CHUNK_SMOTIONS3, |w| {
                w.w_u32(self.motion_refs.len() as u32);
                for r in &self.motion_refs {
                    w_str(w, r, enc);
                }
            });
        }
        if !self.bone_parts.is_empty() {
            w.chunk(CHUNK_BONEPARTS2, |w| {
                w.w_u32(self.bone_parts.len() as u32);
                for p in &self.bone_parts {
                    w_str(w, &p.alias, enc);
                    w.w_u32(p.bones.len() as u32);
                    for b in &p.bones {
                        w_str(w, b, enc);
                    }
                }
            });
        }
        if self.is_dynamic() {
            w.chunk(CHUNK_ACTORTRANSFORM, |w| {
                w.w_vec3(self.actor_position);
                w.w_vec3(self.actor_rotation);
            });
        }
        w.chunk(CHUNK_DESC, |w| {
            w_str(w, &self.created_by, enc);
            w.w_u32(self.created_at);
            w_str(w, &self.modified_by, enc);
            w.w_u32(self.modified_at);
        });

        w.close_chunk();
        w.into_bytes()
    }

    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        self.flags.contains(ObjectFlags::DYNAMIC)
    }

    #[must_use]
    pub fn is_skeleton(&self) -> bool {
        !self.bones.is_empty()
    }

    /// `IsAnimated`: it has motions of its own or refers to motion files.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        !self.motions.is_empty() || !self.motion_refs.is_empty()
    }

    /// `FindSurfaceByName`: names compare without case.
    #[must_use]
    pub fn surface_index(&self, name: &str) -> Option<usize> {
        self.surfaces.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// `FindBoneByNameIdx`.
    #[must_use]
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// `GetBoneIndexByWMap`.
    #[must_use]
    pub fn bone_index_by_wmap(&self, wmap: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.wmap == wmap)
    }

    /// `GetRootBoneID`.
    #[must_use]
    pub fn root_bone(&self) -> Option<usize> {
        self.bones.iter().position(Bone::is_root)
    }

    /// The children of a bone, in bone order.
    #[must_use]
    pub fn bone_children(&self, bone: usize) -> Vec<usize> {
        (0..self.bones.len()).filter(|i| self.bones[*i].parent_id.map(usize::from) == Some(bone) && *i != bone).collect()
    }

    /// `UpdateBox`: the box around the boxes of all meshes.
    #[must_use]
    pub fn bounding_box(&self) -> Aabb {
        let mut b = Aabb::INVALID;
        for m in &self.meshes {
            for i in 0..8 {
                b.modify(m.bbox.corner(i));
            }
        }
        b
    }

    /// `PrepareBones`: puts the bones into hierarchy order - children sorted by name under
    /// their parent, the root first - and resolves every parent.
    pub fn prepare_bones(&mut self) -> Result<()> {
        if self.bones.is_empty() {
            return Ok(());
        }
        let mut bones = std::mem::take(&mut self.bones);
        // a bone whose parent is not here is a root; the last one found is the root
        let names: Vec<SmolStr> = bones.iter().map(|b| b.name.clone()).collect();
        let mut root_name = None;
        for b in &mut bones {
            if !names.contains(&b.parent) {
                b.parent = SmolStr::default();
                root_name = Some(b.name.clone());
            }
        }
        let Some(root_name) = root_name else {
            self.bones = bones;
            return Err(invalid("The skeleton has no root bone."));
        };

        bones.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let find = |bones: &[Bone], name: &str| bones.iter().position(|b| b.name == name);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); bones.len()];
        for i in 0..bones.len() {
            if let Some(p) = find(&bones, &bones[i].parent) {
                children[p].push(i);
            }
        }
        // oracle: with several parentless bones the C++ keeps the tree of the last one only
        let root = bones.iter().rposition(|b| b.name == root_name && b.parent.is_empty()).or_else(|| find(&bones, &root_name));
        let Some(root) = root else {
            self.bones = bones;
            return Err(invalid("The skeleton has no root bone."));
        };

        // depth first, without recursion
        let mut order = Vec::with_capacity(bones.len());
        let mut parent_of = vec![None; bones.len()];
        let mut seen = vec![false; bones.len()];
        let mut stack = vec![(root, None::<usize>)];
        while let Some((i, parent)) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            parent_of[i] = parent;
            order.push(i);
            for c in children[i].iter().rev() {
                stack.push((*c, Some(i)));
            }
        }
        let mut new_index = vec![usize::MAX; bones.len()];
        for (new, old) in order.iter().enumerate() {
            new_index[*old] = new;
        }
        let mut slots: Vec<Option<Bone>> = bones.into_iter().map(Some).collect();
        let mut out = Vec::with_capacity(order.len());
        for old in &order {
            if let Some(mut b) = slots[*old].take() {
                b.parent_id = parent_of[*old].and_then(|p| u16::try_from(new_index[p]).ok());
                out.push(b);
            }
        }
        self.bones = out;
        Ok(())
    }

    /// `VerifyBoneParts`: drops names that are no bone from every part; true when every bone
    /// then sits in exactly one part.
    pub fn verify_bone_parts(&mut self) -> bool {
        let mut used = vec![0u32; self.bones.len()];
        let bones = &self.bones;
        for part in &mut self.bone_parts {
            part.bones.retain(|name| match bones.iter().position(|b| b.name == *name) {
                Some(i) => {
                    used[i] += 1;
                    true
                }
                None => false,
            });
        }
        used.iter().all(|n| *n == 1)
    }

    /// `VerifyMeshNames`: gives every mesh a name of its own.
    fn verify_mesh_names(&mut self) {
        let mut idx = 0u32;
        for i in 0..self.meshes.len() {
            let raw = if self.meshes[i].name.is_empty() { "mesh" } else { self.meshes[i].name.as_str() };
            let pref = raw.trim_matches(|c: char| c <= ' ').to_owned();
            let mut name = pref.clone();
            while self.meshes.iter().enumerate().any(|(j, m)| j != i && m.name.eq_ignore_ascii_case(&name)) {
                name = format!("{pref}{idx:2}");
                idx += 1;
            }
            if self.meshes[i].name != name {
                self.meshes[i].name = SmolStr::new(name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::tests::bent_quad;
    use crate::motion::BoneMotion;

    fn sample() -> Object {
        let mut o = Object {
            flags: ObjectFlags::DYNAMIC,
            user_data: "[collide]\nignore_static\n".into(),
            lod_ref: "lods\\crate".into(),
            created_by: "\\\\PC\\author".into(),
            created_at: 1_000_000,
            modified_at: 1_000_100,
            actor_position: Vec3::new(0.0, 0.5, 0.0),
            ..Object::default()
        };
        o.surfaces.push(Surface {
            name: "Mat".into(),
            texture: "crete\\wall".into(),
            vmap: "Texture".into(),
            shader: "default".into(),
            shader_xrlc: "default".into(),
            game_mtl: "materials\\wood".into(),
            flags: SurfaceFlags::TWO_SIDED,
            fvf: 0x112,
        });
        let mut mesh = bent_quad([0, 0]);
        mesh.rebuild_vmaps().unwrap();
        o.meshes.push(mesh);
        for (name, parent) in [("root", ""), ("b_leg", "root"), ("a_arm", "root"), ("hand", "a_arm")] {
            o.bones.push(Bone { name: name.into(), parent: parent.into(), wmap: name.into(), ..Bone::default() });
        }
        o.prepare_bones().unwrap();
        o.bone_parts.push(BonePart { alias: "all".into(), bones: o.bones.iter().map(|b| b.name.clone()).collect() });
        let mut m = SMotion { frame_end: 10, ..SMotion::default() };
        m.set_name("idle");
        m.bone_motions.push(BoneMotion { name: "hand".into(), ..BoneMotion::default() });
        m.sort_bones_by_skeleton(&o.bones);
        o.motions.push(m);
        o
    }

    #[test]
    fn bones_are_ordered_parent_first_children_by_name() {
        let o = sample();
        let names: Vec<&str> = o.bones.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["root", "a_arm", "hand", "b_leg"]);
        let parents: Vec<Option<u16>> = o.bones.iter().map(|b| b.parent_id).collect();
        assert_eq!(parents, [None, Some(0), Some(1), Some(0)]);
        assert_eq!(o.root_bone(), Some(0));
        assert_eq!(o.bone_children(0), vec![1, 3]);
        assert_eq!(o.motions[0].bone_motions.len(), 4);
        assert_eq!(o.motions[0].bone_motions[2].name, "hand");
    }

    #[test]
    fn an_object_round_trips_byte_for_byte() {
        let o = sample();
        let bytes = o.write();
        let back = Object::read(&bytes).unwrap();
        // envelope parameters settle on their 16-bit steps at the first read
        assert_eq!(back.write(), bytes);
        let again = Object::read(&back.write()).unwrap();
        assert_eq!(again, back);
        assert_eq!(back.surfaces, o.surfaces);
        assert_eq!(back.meshes, o.meshes);
        assert_eq!(back.bones, o.bones);
        assert_eq!(back.bone_parts, o.bone_parts);
        assert_eq!(back.user_data, o.user_data);
        assert_eq!((back.created_by.as_str(), back.created_at, back.modified_at), ("\\\\PC\\author", 1_000_000, 1_000_100));
        assert!(back.is_skeleton() && back.is_dynamic() && back.is_animated());
        assert_eq!(back.bounding_box(), Aabb { min: Vec3::ZERO, max: Vec3::ONE });
    }

    #[test]
    fn cp1251_files_keep_their_bytes() {
        let mut o = sample();
        o.encoding = TextEncoding::Cp1251;
        o.surfaces[0].texture = "стены\\бетон".into();
        let bytes = o.write();
        assert!(std::str::from_utf8(&bytes).is_err());
        let back = Object::read(&bytes).unwrap();
        assert_eq!(back.encoding, TextEncoding::Cp1251);
        assert_eq!(back.surfaces[0].texture, "стены\\бетон");
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(matches!(Object::read(b""), Err(ObjectError::NotAnObject)));
        assert!(matches!(Object::read(b"not an object at all"), Err(ObjectError::NotAnObject)));

        let mut w = Writer::new();
        w.open_chunk(CHUNK_OBJECT_BODY);
        w.chunk(CHUNK_VERSION, |w| w.w_u16(0x0011));
        assert!(matches!(Object::read(&w.into_bytes()), Err(ObjectError::Version { version: 0x11, .. })));

        // a surface count that the chunk cannot hold
        let mut w = Writer::new();
        w.open_chunk(CHUNK_OBJECT_BODY);
        w.chunk(CHUNK_VERSION, |w| w.w_u16(0x0010));
        w.chunk(CHUNK_FLAGS, |w| w.w_u32(0));
        w.chunk(CHUNK_SURFACES3, |w| w.w_u32(u32::MAX));
        assert!(Object::read(&w.into_bytes()).is_err());

        // every truncation of a real file is an error or an object, never a panic
        let bytes = sample().write();
        for cut in (0..bytes.len()).step_by(7) {
            let _ = Object::read(&bytes[..cut]);
        }
        // and so is every single flipped byte in the headers
        for at in (0..bytes.len().min(600)).step_by(3) {
            let mut b = bytes.clone();
            b[at] ^= 0xff;
            let _ = Object::read(&b);
        }
    }

    #[test]
    fn duplicate_mesh_names_get_a_number() {
        let mut o = sample();
        let mut second = o.meshes[0].clone();
        second.name = "QUAD".into();
        o.meshes.push(second);
        let back = Object::read(&o.write()).unwrap();
        assert_eq!(back.meshes[0].name, "quad 0");
        assert_eq!(back.meshes[1].name, "QUAD");
    }
}

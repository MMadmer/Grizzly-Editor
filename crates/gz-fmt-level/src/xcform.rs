//! `overlay.xcform`: a module's world-space collision for a level - triangles that are added to
//! the base collision, and cut boxes that take base triangles away.
//!
//! ```text
//! u32 "XCF1"   u32 version (1 | 2)   u16 sector   u16 reserved
//! u32 material count, then that many zero-terminated game material names
//! u32 vertex count, then 12 bytes each
//! u32 triangle count, then 16 bytes each: u32 v0, v1, v2, u16 material index, u16 pad
//! version 2: u32 cut count, then 24 bytes each: centre, half extents
//! ```

use std::sync::Arc;

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::bytes::{fits, read_vec3s, u16_at, u32_at, vec3_at};
use crate::math::similar;
use crate::{Aabb, LevelError, Result};

/// `"XCF1"`.
pub const XCFORM_MAGIC: u32 = 0x3146_4358;
/// The version the editor writes: the cut list is present.
pub const XCFORM_VERSION: u32 = 2;
/// How close two cut boxes are to count as the same one.
pub const CUT_TOLERANCE: f32 = 0.005;

const TRI: usize = 16;

/// A triangle of the overlay: vertex indices and an index into the material names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XcTri {
    pub v: [u32; 3],
    pub material: u16,
}

/// The collision overlay of one level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XcForm {
    /// The sector every triangle is given.
    pub sector: u16,
    /// Game material names (`materials\default`); a triangle names one by index.
    pub materials: Vec<SmolStr>,
    pub verts: Arc<[Vec3]>,
    pub tris: Arc<[XcTri]>,
    /// Boxes that delete the base triangles lying wholly inside them. Kept as boxes, the way
    /// the editor holds them; the file stores centre and half extents.
    pub cuts: Vec<Aabb>,
}

impl XcForm {
    /// Reads the file the way the game does. A file cut short inside the cut list keeps the cuts
    /// that are whole, as the game does; anything else that does not fit is an error.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        match Self::parse(bytes, false) {
            (form, None) => Ok(form),
            (_, Some(err)) => Err(err),
        }
    }

    /// Reads the file the way the editor reads back what it wrote before, so that an export on
    /// top of it gives the bytes the C++ gives. `true` when the whole file was taken.
    // oracle: ReadXCForm wants 18 bytes for a 16-byte triangle, so a file with more than
    // 2 + 12 * cuts triangles stops before the triangles and its cuts are not read. Whatever was
    // read up to that point stays, as it does in the C++ struct.
    #[must_use]
    pub fn read_back(bytes: &[u8]) -> (Self, bool) {
        let (form, err) = Self::parse(bytes, true);
        (form, err.is_none())
    }

    fn parse(bytes: &[u8], editor: bool) -> (Self, Option<LevelError>) {
        let mut out = XcForm::default();
        let bad = |text: &str| Some(LevelError::Invalid(format!("collision overlay: {text}")));
        if bytes.len() < 16 {
            return (out, bad("the file is shorter than its header"));
        }
        if u32_at(bytes, 0) != XCFORM_MAGIC {
            return (out, bad("the file does not start with XCF1"));
        }
        let version = u32_at(bytes, 4);
        if version != 1 && version != 2 {
            return (out, Some(LevelError::Version { what: "collision overlay", found: version }));
        }
        out.sector = u16_at(bytes, 8);
        let material_count = u32_at(bytes, 12);

        let mut r = Reader::new(bytes);
        if r.seek(16).is_err() {
            return (out, bad("the file is shorter than its header"));
        }
        let mut i = 0u32;
        while i < material_count && !r.eof() {
            match r.r_stringz() {
                Ok(name) => out.materials.push(SmolStr::new(name)),
                Err(_) => return (out, bad("a material name has no terminator")),
            }
            i += 1;
        }

        let Ok(vert_count) = r.r_u32() else {
            return (out, bad("the vertex count is missing"));
        };
        // the vertices and the triangle count after them
        let need = u64::from(vert_count) * 12 + 4;
        if need > r.elapsed() as u64 {
            return (out, bad("the vertices do not fit into the file"));
        }
        let Ok(raw) = r.take(vert_count as usize * 12) else {
            return (out, bad("the vertices do not fit into the file"));
        };
        out.verts = read_vec3s(raw).into();

        let Ok(tri_count) = r.r_u32() else {
            return (out, bad("the triangle count is missing"));
        };
        let per_tri = if editor { TRI as u64 + 2 } else { TRI as u64 };
        if fits(r.elapsed(), u64::from(tri_count), per_tri, "overlay triangles").is_err() {
            return (out, bad("the triangles do not fit into the file"));
        }
        let Ok(raw) = r.take(tri_count as usize * TRI) else {
            return (out, bad("the triangles do not fit into the file"));
        };
        out.tris = raw
            .as_chunks::<TRI>()
            .0
            .iter()
            .map(|c| XcTri { v: [u32_at(c, 0), u32_at(c, 4), u32_at(c, 8)], material: u16_at(c, 12) })
            .collect();

        if version >= 2 && r.elapsed() >= 4 {
            let cut_count = r.r_u32().unwrap_or(0);
            let mut i = 0u32;
            while i < cut_count {
                let Ok(rec) = r.take(24) else { break };
                out.cuts.push(Aabb::from_center(vec3_at(rec, 0), vec3_at(rec, 12)));
                i += 1;
            }
        }
        (out, None)
    }

    /// The file, always as version 2. A cut is written as its centre and `max - centre`.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let names: usize = self.materials.iter().map(|m| m.len() + 1).sum();
        let mut w = Writer::with_capacity(
            24 + names + self.verts.len() * 12 + self.tris.len() * TRI + self.cuts.len() * 24,
        );
        w.w_u32(XCFORM_MAGIC);
        w.w_u32(XCFORM_VERSION);
        w.w_u16(self.sector);
        w.w_u16(0);
        w.w_u32(self.materials.len() as u32);
        for name in &self.materials {
            w.w_stringz(name);
        }
        w.w_u32(self.verts.len() as u32);
        for v in self.verts.iter() {
            w.w_vec3(*v);
        }
        w.w_u32(self.tris.len() as u32);
        for t in self.tris.iter() {
            w.w_u32(t.v[0]);
            w.w_u32(t.v[1]);
            w.w_u32(t.v[2]);
            w.w_u16(t.material);
            w.w_u16(0);
        }
        w.w_u32(self.cuts.len() as u32);
        for cut in &self.cuts {
            w.w_vec3(cut.center());
            w.w_vec3(cut.half_extents());
        }
        w.into_bytes()
    }

    /// The index of a material name, added at the end when it is new: the editor's `MtlIndex`.
    pub fn material_index(materials: &mut Vec<SmolStr>, name: &str) -> usize {
        if let Some(i) = materials.iter().position(|m| m == name) {
            return i;
        }
        materials.push(SmolStr::new(name));
        materials.len() - 1
    }

    /// Adds the boxes that are not there yet (within [`CUT_TOLERANCE`]) and says how many were
    /// new: cuts accumulate over exports, and the same selection exported twice changes nothing.
    pub fn add_cuts(&mut self, boxes: &[Aabb]) -> usize {
        let mut added = 0;
        for b in boxes {
            if !self.cuts.iter().any(|c| same_cut(b, c)) {
                self.cuts.push(*b);
                added += 1;
            }
        }
        added
    }

    /// The triangles the game takes: those whose three vertices exist.
    pub fn valid_tris(&self) -> impl Iterator<Item = &XcTri> {
        let n = self.verts.len() as u32;
        self.tris.iter().filter(move |t| t.v.iter().all(|&v| v < n))
    }
}

/// Whether two cut boxes are the same one for the editor.
#[must_use]
pub fn same_cut(a: &Aabb, b: &Aabb) -> bool {
    similar(a.min, b.min, CUT_TOLERANCE) && similar(a.max, b.max, CUT_TOLERANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tris: usize) -> XcForm {
        let mut materials = Vec::new();
        let a = XcForm::material_index(&mut materials, "materials\\default");
        let b = XcForm::material_index(&mut materials, "materials\\wood");
        assert_eq!(XcForm::material_index(&mut materials, "materials\\default"), a);
        let list: Vec<XcTri> = (0..tris)
            .map(|i| XcTri { v: [0, 1, 2], material: if i % 2 == 0 { a as u16 } else { b as u16 } })
            .collect();
        XcForm {
            sector: 7,
            materials,
            verts: vec![Vec3::ZERO, Vec3::X, Vec3::new(0.0, 0.5, 2.0)].into(),
            tris: list.into(),
            cuts: vec![Aabb::new(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 3.0))],
        }
    }

    #[test]
    fn writes_the_layout_the_game_reads() {
        let form = sample(1);
        let bytes = form.write();
        let mut expect = Vec::new();
        expect.extend_from_slice(b"XCF1");
        expect.extend_from_slice(&2u32.to_le_bytes());
        expect.extend_from_slice(&7u16.to_le_bytes());
        expect.extend_from_slice(&0u16.to_le_bytes());
        expect.extend_from_slice(&2u32.to_le_bytes());
        expect.extend_from_slice(b"materials\\default\0materials\\wood\0");
        expect.extend_from_slice(&3u32.to_le_bytes());
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 2.0] {
            expect.extend_from_slice(&f.to_le_bytes());
        }
        expect.extend_from_slice(&1u32.to_le_bytes());
        for i in [0u32, 1, 2] {
            expect.extend_from_slice(&i.to_le_bytes());
        }
        expect.extend_from_slice(&[0, 0, 0, 0]);
        expect.extend_from_slice(&1u32.to_le_bytes());
        for f in [0.0f32, 1.0, 1.0, 1.0, 1.0, 2.0] {
            expect.extend_from_slice(&f.to_le_bytes());
        }
        assert_eq!(bytes, expect);

        let back = XcForm::read(&bytes).unwrap();
        assert_eq!(back, form);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn the_editor_read_back_loses_cuts_behind_many_triangles() {
        // 14 triangles and one cut: 28 <= 4 + 24, the editor still reads it
        let few = sample(14).write();
        let (form, whole) = XcForm::read_back(&few);
        assert!(whole);
        assert_eq!(form.cuts.len(), 1);
        assert_eq!(form.tris.len(), 14);

        // 15 triangles: the check fails, the triangles and the cut are not taken
        let many = sample(15).write();
        let (form, whole) = XcForm::read_back(&many);
        assert!(!whole);
        assert!(form.cuts.is_empty());
        assert!(form.tris.is_empty());
        assert_eq!(form.verts.len(), 3);
        assert_eq!(form.materials.len(), 2);
        // the game reads the same file whole
        assert_eq!(XcForm::read(&many).unwrap().cuts.len(), 1);
    }

    #[test]
    fn version_one_has_no_cuts_and_a_cut_list_may_end_early() {
        let mut v1 = sample(1).write();
        v1[4] = 1;
        assert!(XcForm::read(&v1).unwrap().cuts.is_empty());

        let mut two = sample(1);
        two.cuts.push(Aabb::new(Vec3::splat(5.0), Vec3::splat(6.0)));
        let bytes = two.write();
        let cut = XcForm::read(&bytes[..bytes.len() - 3]).unwrap();
        assert_eq!(cut.cuts.len(), 1);
    }

    #[test]
    fn cuts_accumulate_once() {
        let mut form = sample(1);
        let near = Aabb::new(Vec3::new(-1.001, 0.0, -1.0), Vec3::new(1.0, 2.004, 3.0));
        let far = Aabb::new(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.006, 3.0));
        assert_eq!(form.add_cuts(&[near, far, far]), 1);
        assert_eq!(form.cuts.len(), 2);
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = sample(2).write();
        assert!(XcForm::read(&bytes[..10]).is_err());
        assert!(XcForm::read(b"XCF2aaaaaaaaaaaaaaaa").is_err());
        let mut version = bytes.clone();
        version[4] = 9;
        assert!(matches!(XcForm::read(&version), Err(LevelError::Version { found: 9, .. })));
        // a vertex count that claims the whole address space
        let at = 16 + "materials\\default\0materials\\wood\0".len();
        let mut huge = bytes.clone();
        huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(XcForm::read(&huge).is_err());
        // cut inside the triangles
        assert!(XcForm::read(&bytes[..at + 4 + 36 + 4 + 20]).is_err());
        // an index past the vertices is kept, and left out of what the game takes
        let mut wild = sample(2);
        wild.tris = vec![XcTri { v: [0, 1, 9], material: 0 }, XcTri { v: [0, 1, 2], material: 0 }].into();
        let back = XcForm::read(&wild.write()).unwrap();
        assert_eq!(back.tris.len(), 2);
        assert_eq!(back.valid_tris().count(), 1);
    }
}

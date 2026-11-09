//! `level.cform`: the static collision of a level - a header, vertices and `CDB::TRI` records.

use std::sync::Arc;

use glam::Vec3;

use crate::bytes::{aabb_at, fits, push_aabb, push_vec3s, put_u32, read_vec3s, u32_at};
use crate::{Aabb, LevelError, Result};

/// `CFORM_CURRENT_VERSION`.
pub const CFORM_VERSION: u32 = 4;

const HEADER: usize = 36;
const TRI: usize = 16;

/// `CDB::TRI`: three vertex indices and a word of `material:14, suppress_shadows:1,
/// suppress_wm:1, sector:16`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tri {
    pub verts: [u32; 3],
    /// The packed word, as the file holds it.
    pub dummy: u32,
}

impl Tri {
    const MATERIAL: u32 = 0x3fff;
    const SHADOWS: u32 = 1 << 14;
    const WALLMARKS: u32 = 1 << 15;

    /// A triangle with a material and a sector; both flags off.
    #[must_use]
    pub fn new(verts: [u32; 3], material: u16, sector: u16) -> Self {
        Self { verts, dummy: (u32::from(material) & Self::MATERIAL) | (u32::from(sector) << 16) }
    }

    #[must_use]
    pub fn material(&self) -> u16 {
        (self.dummy & Self::MATERIAL) as u16
    }

    #[must_use]
    pub fn suppress_shadows(&self) -> bool {
        self.dummy & Self::SHADOWS != 0
    }

    #[must_use]
    pub fn suppress_wallmarks(&self) -> bool {
        self.dummy & Self::WALLMARKS != 0
    }

    #[must_use]
    pub fn sector(&self) -> u16 {
        (self.dummy >> 16) as u16
    }

    pub fn set_material(&mut self, material: u16) {
        self.dummy = (self.dummy & !Self::MATERIAL) | (u32::from(material) & Self::MATERIAL);
    }

    pub fn set_sector(&mut self, sector: u16) {
        self.dummy = (self.dummy & 0xffff) | (u32::from(sector) << 16);
    }

    pub fn set_suppress_shadows(&mut self, on: bool) {
        self.dummy = if on { self.dummy | Self::SHADOWS } else { self.dummy & !Self::SHADOWS };
    }

    pub fn set_suppress_wallmarks(&mut self, on: bool) {
        self.dummy = if on { self.dummy | Self::WALLMARKS } else { self.dummy & !Self::WALLMARKS };
    }
}

/// A collision form: `hdrCFORM`, the vertices, the triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct CForm {
    pub version: u32,
    pub aabb: Aabb,
    pub verts: Arc<[Vec3]>,
    pub tris: Arc<[Tri]>,
}

impl CForm {
    /// A form of the current version whose box is the box of its vertices.
    #[must_use]
    pub fn new(verts: Vec<Vec3>, tris: Vec<Tri>) -> Self {
        let aabb = Aabb::from_points(&verts);
        Self { version: CFORM_VERSION, aabb, verts: verts.into(), tris: tris.into() }
    }

    /// Reads a `level.cform` (or the body of a chunk that holds one). A triangle that names a
    /// vertex the file does not have is an error: the engine would read past the array.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let Some(head) = bytes.get(..HEADER) else {
            return Err(LevelError::Invalid("the collision form is shorter than its header".into()));
        };
        let version = u32_at(head, 0);
        if version != CFORM_VERSION {
            return Err(LevelError::Version { what: "collision form", found: version });
        }
        let vert_count = u32_at(head, 4);
        let face_count = u32_at(head, 8);
        let aabb = aabb_at(head, 12);

        let body = bytes.get(HEADER..).unwrap_or_default();
        let nv = fits(body.len(), u64::from(vert_count), 12, "collision vertices")?;
        let rest = body.get(nv * 12..).unwrap_or_default();
        let nf = fits(rest.len(), u64::from(face_count), TRI as u64, "collision triangles")?;

        let verts = read_vec3s(body.get(..nv * 12).unwrap_or_default());
        let mut tris = Vec::with_capacity(nf);
        for rec in rest.get(..nf * TRI).unwrap_or_default().chunks_exact(TRI) {
            let tri = Tri { verts: [u32_at(rec, 0), u32_at(rec, 4), u32_at(rec, 8)], dummy: u32_at(rec, 12) };
            if tri.verts.iter().any(|&v| v >= vert_count) {
                return Err(LevelError::Invalid(format!(
                    "collision triangle {} names a vertex outside the {vert_count} the form has",
                    tris.len()
                )));
            }
            tris.push(tri);
        }
        Ok(Self { version, aabb, verts: verts.into(), tris: tris.into() })
    }

    /// The file: header, vertices, triangles.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.verts.len() * 12 + self.tris.len() * TRI);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(self.verts.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.tris.len() as u32).to_le_bytes());
        push_aabb(&mut out, &self.aabb);
        push_vec3s(&mut out, &self.verts);
        let start = out.len();
        out.resize(start + self.tris.len() * TRI, 0);
        if let Some(dst) = out.get_mut(start..) {
            for (rec, t) in dst.chunks_exact_mut(TRI).zip(self.tris.iter()) {
                put_u32(rec, 0, t.verts[0]);
                put_u32(rec, 4, t.verts[1]);
                put_u32(rec, 8, t.verts[2]);
                put_u32(rec, 12, t.dummy);
            }
        }
        out
    }

    /// The triangles as bare index triples, the shape [`crate::Bvh::build`] takes.
    #[must_use]
    pub fn indices(&self) -> Vec<[u32; 3]> {
        self.tris.iter().map(|t| t.verts).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CForm {
        let verts = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 2.0, 1.0)];
        let mut b = Tri::new([1, 3, 2], 0x3abc, 0xfffe);
        b.set_suppress_wallmarks(true);
        CForm::new(verts, vec![Tri::new([0, 1, 2], 7, 3), b])
    }

    #[test]
    fn round_trips_byte_for_byte() {
        let form = sample();
        let bytes = form.write();
        assert_eq!(bytes.len(), 36 + 4 * 12 + 2 * 16);
        let back = CForm::read(&bytes).unwrap();
        assert_eq!(back, form);
        assert_eq!(back.write(), bytes);
        assert_eq!(back.aabb.max, Vec3::new(1.0, 2.0, 1.0));
    }

    #[test]
    fn the_packed_word_keeps_its_fields_apart() {
        let form = sample();
        let t = form.tris[1];
        assert_eq!(t.material(), 0x3abc);
        assert_eq!(t.sector(), 0xfffe);
        assert!(t.suppress_wallmarks());
        assert!(!t.suppress_shadows());
        assert_eq!(t.dummy, 0xfffe_0000 | 0x8000 | 0x3abc);
        let mut u = t;
        u.set_material(0xffff);
        u.set_sector(1);
        u.set_suppress_shadows(true);
        assert_eq!(u.dummy, 0x0001_ffff);
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = sample().write();
        assert!(CForm::read(&bytes[..20]).is_err());
        assert!(CForm::read(&bytes[..bytes.len() - 1]).is_err());

        let mut huge = bytes.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(CForm::read(&huge).is_err());

        let mut version = bytes.clone();
        version[0] = 3;
        assert!(matches!(CForm::read(&version), Err(LevelError::Version { found: 3, .. })));

        // a triangle that points past the vertex array
        let mut wild = bytes;
        let at = 36 + 4 * 12;
        wild[at..at + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(CForm::read(&wild).is_err());
    }
}

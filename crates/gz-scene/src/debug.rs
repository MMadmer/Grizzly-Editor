//! The compiler's error list drawn over the scene (`CToolCustom::SDebugDraw`): Build > Import /
//! Export / Clear Error List and Scene > Clear Debug Draw (`EScene::LoadCompilerError` /
//! `SaveCompilerError`, `ClearDebugDraw`).
//!
//! The file is the C++ structures written as they lie in memory: chunk 10 the points (56 bytes:
//! the position, the colour, two flags, the padding and the `xr_string` of a caption), 11 the
//! lines (32 bytes), 12 the wire faces (44 bytes); the compiler's own older chunks 0 (points) and
//! 2 (faces) are read too.

use gz_chunk::{Reader, Writer};

use crate::SceneError;
use crate::math::Vec3;

const POINT_SIZE: usize = 56;
const LINE_SIZE: usize = 32;
const FACE_SIZE: usize = 44;
/// `xr_string`'s small-buffer capacity: a longer caption lived on the heap of the process that
/// wrote it and is not in the file.
const SSO_CAP: u64 = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct DbgPoint {
    pub p: Vec3,
    pub color: u32,
    /// Numbered in the view ("P: 3").
    pub numbered: bool,
    pub m: bool,
    pub descr: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbgLine {
    pub p: [Vec3; 2],
    pub color: u32,
    pub numbered: bool,
    pub m: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbgFace {
    pub p: [Vec3; 3],
    pub color: u32,
    pub numbered: bool,
    pub m: bool,
}

/// What the viewport draws of the last compiler run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugDraw {
    pub points: Vec<DbgPoint>,
    pub lines: Vec<DbgLine>,
    pub wire_faces: Vec<DbgFace>,
}

impl DebugDraw {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.lines.is_empty() && self.wire_faces.is_empty()
    }
}

fn vec3(b: &[u8], at: usize) -> Vec3 {
    let f =
        |o: usize| b.get(at + o..at + o + 4).map_or(0.0, |x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    Vec3::new(f(0), f(4), f(8))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).map_or(0, |x| {
        let mut a = [0u8; 8];
        a.copy_from_slice(x);
        u64::from_le_bytes(a)
    })
}

/// A record count and its records, never more than the bytes hold.
fn records(body: &[u8], size: usize) -> Result<Vec<&[u8]>, SceneError> {
    let n = u32_at(body, 0) as usize;
    let rest = body.get(4..).unwrap_or(&[]);
    if rest.len() / size < n {
        return Err(SceneError::load("The error list ends early."));
    }
    Ok(rest.chunks_exact(size).take(n).collect())
}

/// `EScene::LoadCompilerError`.
pub fn read_compiler_error(bytes: &[u8]) -> Result<DebugDraw, SceneError> {
    let r = Reader::new(bytes);
    let mut d = DebugDraw::default();
    if let Some((body, _)) = r.find_chunk(10) {
        for rec in records(body, POINT_SIZE)? {
            // MSVC x64 `std::string` at 24: the buffer (16), the size, the capacity
            let (size, cap) = (u64_at(rec, 40), u64_at(rec, 48));
            let descr = if cap <= SSO_CAP && size <= SSO_CAP {
                String::from_utf8_lossy(rec.get(24..24 + size as usize).unwrap_or(&[])).into_owned()
            } else {
                String::new()
            };
            d.points.push(DbgPoint {
                p: vec3(rec, 0),
                color: u32_at(rec, 12),
                numbered: rec.get(16).is_some_and(|b| *b != 0),
                m: rec.get(17).is_some_and(|b| *b != 0),
                descr,
            });
        }
    } else if let Some((body, _)) = r.find_chunk(0) {
        for rec in records(body, 12)? {
            d.points.push(DbgPoint {
                p: vec3(rec, 0),
                color: 0xff00_ff00,
                numbered: true,
                m: true,
                descr: "TJ".into(),
            });
        }
    }
    if let Some((body, _)) = r.find_chunk(12) {
        for rec in records(body, FACE_SIZE)? {
            d.wire_faces.push(DbgFace {
                p: [vec3(rec, 0), vec3(rec, 12), vec3(rec, 24)],
                color: u32_at(rec, 36),
                numbered: rec.get(40).is_some_and(|b| *b != 0),
                m: rec.get(41).is_some_and(|b| *b != 0),
            });
        }
    } else if let Some((body, _)) = r.find_chunk(2) {
        for rec in records(body, 36)? {
            d.wire_faces.push(DbgFace {
                p: [vec3(rec, 0), vec3(rec, 12), vec3(rec, 24)],
                color: 0xffff_0000,
                numbered: true,
                m: true,
            });
        }
    }
    Ok(d)
}

fn w_vec3(out: &mut Vec<u8>, v: Vec3) {
    for c in [v.x, v.y, v.z] {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

/// `EScene::SaveCompilerError`: the three lists as the C++ structures lie in memory (a caption
/// that fits the small buffer inline, a longer one as an empty string: its heap is not ours to
/// write).
#[must_use]
pub fn write_compiler_error(d: &DebugDraw) -> Vec<u8> {
    let mut w = Writer::new();
    let mut points = Vec::with_capacity(4 + d.points.len() * POINT_SIZE);
    points.extend_from_slice(&u32::try_from(d.points.len()).unwrap_or(u32::MAX).to_le_bytes());
    for p in &d.points {
        let start = points.len();
        w_vec3(&mut points, p.p);
        points.extend_from_slice(&p.color.to_le_bytes());
        points.push(u8::from(p.numbered));
        points.push(u8::from(p.m));
        points.resize(start + 24, 0);
        let text = if p.descr.len() as u64 <= SSO_CAP { p.descr.as_bytes() } else { &[][..] };
        let mut buf = [0u8; 16];
        buf[..text.len()].copy_from_slice(text);
        points.extend_from_slice(&buf);
        points.extend_from_slice(&(text.len() as u64).to_le_bytes());
        points.extend_from_slice(&SSO_CAP.to_le_bytes());
    }
    w.w_chunk(10, &points);
    let mut lines = Vec::with_capacity(4 + d.lines.len() * LINE_SIZE);
    lines.extend_from_slice(&u32::try_from(d.lines.len()).unwrap_or(u32::MAX).to_le_bytes());
    for l in &d.lines {
        w_vec3(&mut lines, l.p[0]);
        w_vec3(&mut lines, l.p[1]);
        lines.extend_from_slice(&l.color.to_le_bytes());
        lines.extend_from_slice(&[u8::from(l.numbered), u8::from(l.m), 0, 0]);
    }
    w.w_chunk(11, &lines);
    let mut faces = Vec::with_capacity(4 + d.wire_faces.len() * FACE_SIZE);
    faces.extend_from_slice(&u32::try_from(d.wire_faces.len()).unwrap_or(u32::MAX).to_le_bytes());
    for f in &d.wire_faces {
        for p in f.p {
            w_vec3(&mut faces, p);
        }
        faces.extend_from_slice(&f.color.to_le_bytes());
        faces.extend_from_slice(&[u8::from(f.numbered), u8::from(f.m), 0, 0]);
    }
    w.w_chunk(12, &faces);
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_list_reads_back() {
        let d = DebugDraw {
            points: vec![DbgPoint {
                p: Vec3::X,
                color: 0xff00ff00,
                numbered: true,
                m: true,
                descr: "TJ".into(),
            }],
            lines: vec![DbgLine { p: [Vec3::ZERO, Vec3::Y], color: 1, numbered: false, m: false }],
            wire_faces: vec![DbgFace {
                p: [Vec3::ZERO, Vec3::X, Vec3::Z],
                color: 2,
                numbered: true,
                m: false,
            }],
        };
        let bytes = write_compiler_error(&d);
        let back = read_compiler_error(&bytes).unwrap();
        assert_eq!(back.points, d.points);
        assert_eq!(back.wire_faces, d.wire_faces);
        // the lines are written, not read (the C++ load skips them)
        assert!(back.lines.is_empty());
    }

    #[test]
    fn a_count_past_the_bytes_is_refused() {
        let mut w = Writer::new();
        w.w_chunk(12, &1000u32.to_le_bytes());
        assert!(read_compiler_error(&w.into_bytes()).is_err());
    }
}

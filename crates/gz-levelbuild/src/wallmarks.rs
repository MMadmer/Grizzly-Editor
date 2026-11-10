//! The Wallmark tool's marks (`ESceneWallmarkTool`): read off the scene's part, written as
//! `level.wallmarks` (`Export`) and as faces of the build project (`ExportStatic`).

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};

/// `COMPILER_SHADER`: the compiler shader of a mark's faces.
pub const COMPILER_SHADER: &str = "def_shaders\\def_vertex_ghost_no_shadow";
const WM_CHUNK_ITEMS: u32 = 4;
const WM_CHUNK_ITEMS2: u32 = 5;
/// `sizeof(FVF::LIT)`: position, colour, texture coordinates.
const LIT: usize = 24;

/// One mark: its bounding sphere and its triangles (`FVF::LIT`, three per face).
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub center: Vec3,
    pub radius: f32,
    pub verts: Vec<(Vec3, u32, Vec2)>,
}

/// A shader and texture with their marks.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub shader: String,
    pub texture: String,
    pub marks: Vec<Mark>,
}

/// `LoadStream`'s items: the old chunk (no size) or the new one (width, height, rotation per
/// mark). Slots without marks are dropped (`RefiningSlots`). A part that does not read gives
/// what was read up to there.
#[must_use]
pub fn read(part: &[u8]) -> Vec<Slot> {
    let r = Reader::new(part);
    let (items, sized) = match (r.open_chunk(WM_CHUNK_ITEMS).ok().flatten(), r.open_chunk(WM_CHUNK_ITEMS2).ok().flatten()) {
        (Some(c), _) => (c, false),
        (None, Some(c)) => (c, true),
        (None, None) => return Vec::new(),
    };
    let items = items.reader();
    let mut out = Vec::new();
    for id in 0u32.. {
        let Ok(Some(c)) = items.open_chunk(id) else { break };
        let mut q = c.reader();
        let Ok(count) = q.r_u32() else { break };
        if count == 0 {
            continue;
        }
        let (Ok(shader), Ok(texture)) = (q.r_stringz(), q.r_stringz()) else { break };
        let mut marks = Vec::new();
        for _ in 0..count.min(1 << 20) {
            // flags, the box, the sphere
            if q.advance(1 + 24).is_err() {
                break;
            }
            let (Ok(center), Ok(radius)) = (q.r_vec3(), q.r_f32()) else { break };
            if sized && q.advance(12).is_err() {
                break;
            }
            let Ok(n) = q.r_u32() else { break };
            let Ok(raw) = q.take((n as usize).saturating_mul(LIT)) else { break };
            let verts = raw
                .chunks_exact(LIT)
                .map(|v| {
                    let f = |i: usize| f32::from_le_bytes([v[i], v[i + 1], v[i + 2], v[i + 3]]);
                    (Vec3::new(f(0), f(4), f(8)), u32::from_le_bytes([v[12], v[13], v[14], v[15]]), Vec2::new(f(16), f(20)))
                })
                .collect();
            marks.push(Mark { center, radius, verts });
        }
        if !marks.is_empty() {
            out.push(Slot { shader, texture, marks });
        }
    }
    out
}

/// `Export`: `level.wallmarks`.
#[must_use]
pub fn write(slots: &[Slot]) -> Vec<u8> {
    let mut w = Writer::new();
    w.chunk(1, |w| {
        w.w_u32(slots.len() as u32);
        for s in slots {
            w.w_u32(s.marks.len() as u32);
            if s.marks.is_empty() {
                continue;
            }
            w.w_stringz(&s.shader);
            w.w_stringz(&s.texture);
            for m in &s.marks {
                w.w_vec3(m.center);
                w.w_f32(m.radius);
                w.w_u32(m.verts.len() as u32);
                for (p, c, t) in &m.verts {
                    w.w_vec3(*p);
                    w.w_u32(*c);
                    w.w_vec2(*t);
                }
            }
        }
    });
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_read_back() {
        let mut w = Writer::new();
        w.chunk(WM_CHUNK_ITEMS2, |w| {
            w.chunk(0, |w| {
                w.w_u32(1);
                w.w_stringz("effects\\wallmarkset");
                w.w_stringz("wm\\wm_blood");
                w.w_u8(0);
                w.w(&[0u8; 24]);
                w.w_vec3(Vec3::ONE);
                w.w_f32(0.5);
                w.w(&[0u8; 12]);
                w.w_u32(3);
                for i in 0..3 {
                    w.w_vec3(Vec3::splat(i as f32));
                    w.w_u32(0xffff_ffff);
                    w.w_vec2(Vec2::ZERO);
                }
            });
            w.chunk(1, |w| w.w_u32(0));
        });
        let slots = read(&w.into_bytes());
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].marks[0].verts.len(), 3);
        assert_eq!(slots[0].marks[0].radius, 0.5);
        let file = write(&slots);
        assert_eq!(&file[0..4], &1u32.to_le_bytes());
    }
}

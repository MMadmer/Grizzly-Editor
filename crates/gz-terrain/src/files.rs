//! `levels\<level>\overlay.cut` (`XCU1`) and `levels\<level>\overlay.aiedit` (`XAE1`), written
//! and read exactly as the game's reader takes them (`src/xrCore/XMS/xms_level_edits.cpp`, the
//! game's `docs/dead-air/XMS_ARCHITECTURE.md` 8.4): little-endian, the same caps, and a file the
//! game would refuse is refused here with the game's reason.

use glam::Vec3;

use crate::clip::{CutKind, Region};

const CUT_MAGIC: &[u8; 4] = b"XCU1";
const AIEDIT_MAGIC: &[u8; 4] = b"XAE1";
/// The version written, and the newest read.
pub const VERSION: u32 = 1;
/// The game's caps: a module is content from anywhere.
pub const MAX_BYTES: usize = 64 << 20;
pub const MAX_REGIONS: u32 = 4096;
pub const MAX_GRID_SIDE: u32 = 4096;
pub const MAX_AI_EDITS: u32 = 1 << 20;
/// x, z, old y, new y, the normal and the flags. The game's reader asks for 36 bytes an edit
/// before it reads 32 of each (`ai_edit_bytes`): the contract's record is the 32 it reads.
const AI_EDIT_BYTES: usize = 32;

/// A change of one AI node in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiEdit {
    pub x: f32,
    pub z: f32,
    pub old_y: f32,
    pub new_y: f32,
    /// Of unit length, pointing up.
    pub normal: Vec3,
    /// Unlinked from its neighbours, both ways.
    pub closed: bool,
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The bytes of `overlay.cut` holding these regions, in order.
#[must_use]
pub fn write_cuts(regions: &[Region]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(CUT_MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, u32::try_from(regions.len()).unwrap_or(u32::MAX));
    for r in regions {
        out.extend_from_slice(&[r.kind.id(), 0, 0, 0]);
        for v in [r.centre.x, r.centre.y, r.centre.z, r.half.x, r.half.y, r.half.z, r.yaw] {
            put_f32(&mut out, v);
        }
        if r.kind == CutKind::Ground {
            put_f32(&mut out, r.tolerance);
            put_u32(&mut out, r.columns);
            put_u32(&mut out, r.rows);
            for h in &r.heights {
                put_f32(&mut out, *h);
            }
        }
    }
    out
}

/// The bytes of `overlay.aiedit` holding these edits, in order.
#[must_use]
pub fn write_ai_edits(edits: &[AiEdit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + edits.len() * AI_EDIT_BYTES);
    out.extend_from_slice(AIEDIT_MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, u32::try_from(edits.len()).unwrap_or(u32::MAX));
    for e in edits {
        for v in [e.x, e.z, e.old_y, e.new_y, e.normal.x, e.normal.y, e.normal.z] {
            put_f32(&mut out, v);
        }
        put_u32(&mut out, u32::from(e.closed));
    }
    out
}

struct Cursor<'a> {
    at: &'a [u8],
}

impl Cursor<'_> {
    fn has(&self, n: usize) -> bool {
        self.at.len() >= n
    }

    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut b = [0u8; N];
        if let Some((head, rest)) = self.at.split_first_chunk::<N>() {
            b = *head;
            self.at = rest;
        }
        b
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }
}

fn header(c: &mut Cursor<'_>, magic: &[u8; 4], what: &'static str) -> Result<u32, String> {
    if !c.has(12) || &c.take::<4>() != magic {
        return Err(format!("not an {what} file"));
    }
    match c.u32() {
        v if v > VERSION => Err("made for a newer game".into()),
        0 => Err("version 0".into()),
        _ => Ok(c.u32()),
    }
}

/// The regions of an `overlay.cut`; the game's reason when it would refuse the file.
pub fn read_cuts(bytes: &[u8]) -> Result<Vec<Region>, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("larger than {} MB", MAX_BYTES >> 20));
    }
    let mut c = Cursor { at: bytes };
    let count = header(&mut c, CUT_MAGIC, "XCU1")?;
    if count > MAX_REGIONS {
        return Err("too many regions".into());
    }
    if !c.has(count as usize * 32) {
        return Err("cut short".into());
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if !c.has(32) {
            return Err("cut short".into());
        }
        let [kind, ..] = c.take::<4>();
        let centre = Vec3::new(c.f32(), c.f32(), c.f32());
        let half = Vec3::new(c.f32(), c.f32(), c.f32());
        let yaw = c.f32();
        let kind = match kind {
            0 => CutKind::Everything,
            1 => CutKind::Ground,
            _ => return Err("a region of an unknown kind".into()),
        };
        let sized = |v: f32| v > 0.0 && v < 1e6;
        if !centre.is_finite()
            || !yaw.is_finite()
            || !sized(half.x)
            || !sized(half.z)
            || !(half.y >= 0.0 && half.y < 1e6)
        {
            return Err("a region with no size or a value that is not a number".into());
        }
        let mut r =
            Region { kind, centre, half, yaw, tolerance: 0.0, columns: 0, rows: 0, heights: Vec::new() };
        if kind == CutKind::Ground {
            if !c.has(12) {
                return Err("cut short".into());
            }
            r.tolerance = c.f32();
            r.columns = c.u32();
            r.rows = c.u32();
            if !(r.tolerance >= 0.0 && r.tolerance < 1e6)
                || r.columns < 2
                || r.rows < 2
                || r.columns > MAX_GRID_SIDE
                || r.rows > MAX_GRID_SIDE
            {
                return Err("a ground region with a bad tolerance or grid".into());
            }
            let samples = r.columns as usize * r.rows as usize;
            if !c.has(samples * 4) {
                return Err("cut short".into());
            }
            r.heights = (0..samples).map(|_| c.f32()).collect();
            if r.heights.iter().any(|h| !h.is_finite()) {
                return Err("a height that is not a number".into());
            }
        }
        out.push(r);
    }
    Ok(out)
}

/// The edits of an `overlay.aiedit`, their normals of unit length; the game's reason when it
/// would refuse the file.
pub fn read_ai_edits(bytes: &[u8]) -> Result<Vec<AiEdit>, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("larger than {} MB", MAX_BYTES >> 20));
    }
    let mut c = Cursor { at: bytes };
    let count = header(&mut c, AIEDIT_MAGIC, "XAE1")?;
    if count > MAX_AI_EDITS {
        return Err("too many edits".into());
    }
    if !c.has(count as usize * AI_EDIT_BYTES) {
        return Err("cut short".into());
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (x, z, old_y, new_y) = (c.f32(), c.f32(), c.f32(), c.f32());
        let normal = Vec3::new(c.f32(), c.f32(), c.f32());
        let flags = c.u32();
        if ![x, z, old_y, new_y].iter().all(|v| v.is_finite()) || !normal.is_finite() {
            return Err("a value that is not a number".into());
        }
        let length = normal.length();
        // the values are finite here: no comparison meets a NaN
        if length <= 1e-6 || normal.y / length <= 0.0 {
            return Err("a normal that does not point up".into());
        }
        out.push(AiEdit { x, z, old_y, new_y, normal: normal / length, closed: flags & 1 != 0 });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Vec<Region> {
        vec![
            Region {
                kind: CutKind::Ground,
                centre: Vec3::new(120.0, 12.0, -40.0),
                half: Vec3::new(20.0, 3.0, 10.0),
                yaw: -0.5,
                tolerance: 0.5,
                columns: 3,
                rows: 2,
                heights: vec![11.0, 11.5, 12.0, 12.5, 13.0, 13.5],
            },
            Region {
                kind: CutKind::Everything,
                centre: Vec3::new(0.0, 1.0, 0.0),
                half: Vec3::new(1.0, 2.0, 3.0),
                yaw: 0.0,
                tolerance: 0.0,
                columns: 0,
                rows: 0,
                heights: Vec::new(),
            },
        ]
    }

    #[test]
    fn the_cuts_are_written_and_read_back() {
        let r = regions();
        let bytes = write_cuts(&r);
        assert_eq!(&bytes[..4], b"XCU1");
        assert_eq!(bytes.len(), 12 + 32 + 12 + 6 * 4 + 32);
        // the heights row by row from local -z, each row from local -x
        assert_eq!(&bytes[12 + 32 + 12..12 + 32 + 16], &11.0f32.to_le_bytes());
        assert_eq!(read_cuts(&bytes).expect("read"), r);
        assert_eq!(write_cuts(&r), bytes, "the same regions give the same bytes");
    }

    #[test]
    fn the_ai_edits_are_written_and_read_back() {
        let e = vec![
            AiEdit { x: 1.0, z: 2.0, old_y: 3.0, new_y: 3.5, normal: Vec3::Y, closed: false },
            AiEdit {
                x: -4.0,
                z: 0.5,
                old_y: 1.0,
                new_y: 0.0,
                normal: Vec3::new(0.6, 0.8, 0.0),
                closed: true,
            },
        ];
        let bytes = write_ai_edits(&e);
        assert_eq!(bytes.len(), 12 + 2 * 32);
        assert_eq!(&bytes[12 + 28..12 + 32], &0u32.to_le_bytes());
        assert_eq!(&bytes[12 + 32 + 28..], &1u32.to_le_bytes());
        let back = read_ai_edits(&bytes).expect("read");
        assert_eq!(back.len(), 2);
        assert!(back[1].closed && (back[1].normal.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_hostile_file_is_refused_with_the_games_reason() {
        let good = write_cuts(&regions());
        assert_eq!(read_cuts(&good[..good.len() - 1]).unwrap_err(), "cut short");
        assert_eq!(read_cuts(b"XCU2\x01\0\0\0\0\0\0\0").unwrap_err(), "not an XCU1 file");
        let mut newer = good.clone();
        newer[4] = 2;
        assert_eq!(read_cuts(&newer).unwrap_err(), "made for a newer game");
        let mut many = good.clone();
        many[8..12].copy_from_slice(&(MAX_REGIONS + 1).to_le_bytes());
        assert_eq!(read_cuts(&many).unwrap_err(), "too many regions");
        // a count that promises more than the file holds is refused before anything is read
        many[8..12].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(read_cuts(&many).unwrap_err(), "cut short");
        let mut nan = good.clone();
        nan[16..20].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(read_cuts(&nan).unwrap_err(), "a region with no size or a value that is not a number");
        let mut grid = good.clone();
        grid[12 + 32 + 4..12 + 32 + 8].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(read_cuts(&grid).unwrap_err(), "a ground region with a bad tolerance or grid");
        let mut kind = good;
        kind[12] = 7;
        assert_eq!(read_cuts(&kind).unwrap_err(), "a region of an unknown kind");

        let mut down = write_ai_edits(&[AiEdit {
            x: 0.0,
            z: 0.0,
            old_y: 0.0,
            new_y: 0.0,
            normal: Vec3::NEG_Y,
            closed: false,
        }]);
        assert_eq!(read_ai_edits(&down).unwrap_err(), "a normal that does not point up");
        down[20..24].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(read_ai_edits(&down).unwrap_err(), "a value that is not a number");
        assert_eq!(read_ai_edits(b"XAE1").unwrap_err(), "not an XAE1 file");
    }
}

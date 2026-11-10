//! What the world reader needs of a spawn packet without decoding it: the `CSE_Abstract`
//! header of `Spawn_Read` and the vertices `CSE_ALifeObject::STATE_Read` starts with.

use glam::Vec3;
use gz_chunk::Reader;
use smol_str::SmolStr;

use crate::graph::r_name;
use crate::{Result, damaged};

const M_SPAWN: u16 = 1;
const M_SPAWN_VERSION: u16 = 1 << 5;

/// The head of a spawn packet.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketHead {
    pub section: SmolStr,
    pub name: SmolStr,
    pub position: Vec3,
    pub angle: Vec3,
    pub id: u16,
    pub id_parent: u16,
    pub version: u16,
    pub spawn_id: u16,
    /// `m_tGraphID`, when the state is a `CSE_ALifeObject`'s (every entity of `all.spawn`).
    pub game_vertex: Option<u16>,
    /// `m_tNodeID`.
    pub level_vertex: Option<u32>,
}

/// Reads the head of a spawn packet (`M_SPAWN` word first).
pub fn peek_packet(packet: &[u8]) -> Result<PacketHead> {
    let mut r = Reader::new(packet);
    if r.r_u16()? != M_SPAWN {
        return Err(damaged("spawn packet", "it does not start with M_SPAWN"));
    }
    let section = r_name(&mut r)?;
    let name = r_name(&mut r)?;
    r.advance(2)?;
    let position = r.r_vec3()?;
    let angle = r.r_vec3()?;
    r.advance(2)?;
    let id = r.r_u16()?;
    let id_parent = r.r_u16()?;
    r.advance(2)?;
    let flags = r.r_u16()?;
    let v = if flags & M_SPAWN_VERSION != 0 { r.r_u16()? } else { 0 };
    let mut head = PacketHead { section, name, position, angle, id, id_parent, version: v, spawn_id: u16::MAX, game_vertex: None, level_vertex: None };
    if v == 0 || v > 128 {
        return Ok(head);
    }
    if v > 120 {
        r.advance(2)?;
    }
    if v > 69 {
        r.advance(2)?;
    }
    if v > 70 {
        let n = if v > 93 { usize::from(r.r_u16()?) } else { usize::from(r.r_u8()?) };
        r.advance(n)?;
    }
    if v > 79 {
        head.spawn_id = r.r_u16()?;
    }
    if v < 112 {
        if v > 82 {
            r.advance(4)?;
        }
        if v > 83 {
            r.advance(4)?;
            r.r_stringz_raw()?;
            r.advance(16)?;
        }
        if v > 84 {
            r.advance(16)?;
        }
    }
    r.advance(2)?;
    // CSE_ALifeObject::STATE_Read
    if v >= 1 {
        if v > 24 {
            if v < 83 {
                r.advance(4)?;
            }
        } else {
            r.advance(1)?;
        }
        if v < 83 {
            r.advance(4)?;
        }
        if v < 4 {
            r.advance(2)?;
        }
        head.game_vertex = Some(r.r_u16()?);
        r.advance(4)?;
    }
    if v >= 4 {
        r.advance(4)?;
    }
    if v >= 8 {
        head.level_vertex = Some(r.r_u32()?);
    }
    Ok(head)
}

#[cfg(test)]
pub(crate) mod tests {
    use gz_chunk::Writer;

    use super::*;

    /// A packet as `Spawn_Write` lays it out, with the first fields of an ALife state.
    pub(crate) fn packet(section: &str, name: &str, game_vertex: u16, level_vertex: u32) -> Vec<u8> {
        let mut w = Writer::new();
        w.w_u16(M_SPAWN);
        w.w_stringz(section);
        w.w_stringz(name);
        w.w_u8(0);
        w.w_u8(0xFE);
        w.w_vec3(Vec3::new(1.0, 2.0, 3.0));
        w.w_vec3(Vec3::ZERO);
        w.w_u16(0);
        w.w_u16(7);
        w.w_u16(u16::MAX);
        w.w_u16(u16::MAX);
        w.w_u16(M_SPAWN_VERSION | 1);
        w.w_u16(128);
        w.w_u16(u16::MAX);
        w.w_u16(12);
        w.w_u16(0);
        w.w_u16(3);
        w.w_u16(2 + 2 + 4 + 4 + 4);
        w.w_u16(game_vertex);
        w.w_f32(0.0);
        w.w_u32(1);
        w.w_u32(level_vertex);
        w.into_bytes()
    }

    #[test]
    fn the_head_and_the_vertices_are_found() {
        let p = packet("stalker", "esc_wolf", 42, 1234);
        let h = peek_packet(&p).unwrap();
        assert_eq!((h.section.as_str(), h.name.as_str(), h.id, h.version, h.spawn_id), ("stalker", "esc_wolf", 7, 128, 3));
        assert_eq!((h.game_vertex, h.level_vertex), (Some(42), Some(1234)));
        assert_eq!(h.position, Vec3::new(1.0, 2.0, 3.0));
        assert!(peek_packet(&p[..p.len() - 3]).is_err());
        assert!(peek_packet(&[0, 0]).is_err());
    }
}

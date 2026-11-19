//! Models as the game will resolve them, and what can be read off an `.ogf` without loading it.
//!
//! A spawn op whose visual the game cannot load is a DELAYED FATAL: the crash fires at online
//! spawn, when the player walks close enough. So the bake proves every model reachable first,
//! and a model that lives in the project is staged into `gamedata\meshes\`, the only module
//! subtree the engine mirrors into its file system.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Where a model was found, in the order it is looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// `<project>\gamedata\meshes\<visual>.ogf` is there already.
    Staged,
    /// A file of the project (a prop's raw `.ogf` path) to be staged from.
    Project(PathBuf),
    /// The linked game ships it.
    Game,
    /// The editor's own meshes (`$game_meshes$`): staged into the project when picked there.
    Sdk(PathBuf),
    Missing,
}

/// A spawn element's model: where it is and its bytes (the export resolution order: the
/// project's staging first, then the game).
#[derive(Clone, Debug)]
pub struct SpawnVisual {
    pub found: Found,
    pub bytes: Option<Arc<[u8]>>,
}

/// What [`crate::gather`] found for the models a bake names.
#[derive(Clone, Debug, Default)]
pub struct Visuals {
    /// A prop's reference (as the scene holds it) -> where its model is.
    pub props: HashMap<String, Found>,
    /// A spawn element's visual (configs form: `props\box`) -> where it is.
    pub spawns: HashMap<String, SpawnVisual>,
    /// A reference that names a model asset -> the name the module build publishes it under
    /// when it is a dynamic model; `None` for a static one, which is level geometry and no
    /// object of the spawn layer.
    pub assets: HashMap<String, Option<String>>,
}

/// OGF container chunks: `u32 id` (bit 31 = compressed), `u32 size`, the body.
fn find_chunk(data: &[u8], want: u32) -> Option<&[u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let id = u32_at(data, off) & !0x8000_0000;
        let size = u32_at(data, off + 4) as usize;
        let body = off + 8;
        if body + size > data.len() {
            return None;
        }
        if id == want {
            return data.get(body..body + size);
        }
        off = body + size;
    }
    None
}

fn u32_at(d: &[u8], off: usize) -> u32 {
    d.get(off..off + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The game's way of saying "there, but not drawn" - a model whose every child that names a
/// texture names `gz_fmt_ogf::UNDRAWN_TEXTURE` (the stash's default).
#[must_use]
pub fn ogf_is_ghost(data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }
    let Some(kids) = find_chunk(data, 9) else { return false };
    let (mut seen, mut off) = (0u32, 0usize);
    while off + 8 <= kids.len() {
        let size = u32_at(kids, off + 4) as usize;
        if off + 8 + size > kids.len() {
            break;
        }
        let child = &kids[off + 8..off + 8 + size];
        if let Some(tex) = find_chunk(child, 2)
            && !tex.is_empty()
        {
            let Some(end) = tex.iter().position(|b| *b == 0) else { return false };
            if !tex[..end].eq_ignore_ascii_case(gz_fmt_ogf::UNDRAWN_TEXTURE.as_bytes()) {
                return false;
            }
            seen += 1;
        }
        off += 8 + size;
    }
    seen > 0
}

/// The bone names of an OGF (`OGF_S_BONE_NAMES`, chunk 13): what a physics object's fixed bones
/// may name.
#[must_use]
pub fn ogf_bone_names(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let id = u32_at(data, off) & 0x7fff_ffff;
        let len = u32_at(data, off + 4) as usize;
        let body = off + 8;
        if body + len > data.len() {
            return out;
        }
        if id == 13 && len >= 4 {
            let count = u32_at(data, body);
            let end = body + len;
            let mut p = body + 4;
            let strlen = |p: usize| data[p..end].iter().position(|b| *b == 0).unwrap_or(end - p);
            let mut i = 0u32;
            while i < count && p < end {
                let n = strlen(p);
                out.push(String::from_utf8_lossy(&data[p..p + n]).into_owned());
                p += n + 1;
                // the parent's name, then the bone's box (an oriented box, 15 floats)
                if p < end {
                    p += strlen(p) + 1 + 60;
                } else {
                    p += 1 + 60;
                }
                i += 1;
            }
            return out;
        }
        off = body + len;
    }
    out
}

/// A physics object's fixed bones as the game will take them. The engine
/// asserts "wrong fixed bone" on the first name the visual lacks, the moment the object comes
/// online - so `all` becomes every bone of the model and an unknown name is dropped (the root
/// bone stays, the body never falls free). Returns the bones to write and the ones dropped.
/// A model whose bones cannot be read keeps the list as it is.
#[must_use]
pub fn checked_fixed_bones(bones: &[String], fixed: &str) -> (String, String) {
    let Some(first) = bones.first() else { return (fixed.to_owned(), String::new()) };
    if fixed.eq_ignore_ascii_case("all") {
        return (bones.join(","), String::new());
    }
    let (mut out, mut bad) = (String::new(), String::new());
    for item in fixed.split(',') {
        let name = item.trim();
        let found = bones.iter().any(|b| b.eq_ignore_ascii_case(name));
        let to = if found { &mut out } else { &mut bad };
        if !to.is_empty() {
            to.push(',');
        }
        to.push_str(name);
    }
    if out.is_empty() {
        out = first.clone();
    }
    (out, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn ghosts_are_told_apart() {
        let ghost = chunk(9, &chunk(0, &chunk(2, b"glow\\glow_ghost\0default\0")));
        let mut file = chunk(1, &[0u8; 8]);
        file.extend(ghost);
        assert!(ogf_is_ghost(&file));
        let mut real = chunk(1, &[0u8; 8]);
        real.extend(chunk(9, &chunk(0, &chunk(2, b"props\\box\0models\\model\0"))));
        assert!(!ogf_is_ghost(&real));
    }

    #[test]
    fn fixed_bones_are_checked() {
        let bones = vec!["root".to_owned(), "lid".to_owned()];
        assert_eq!(checked_fixed_bones(&bones, "all"), ("root,lid".to_owned(), String::new()));
        assert_eq!(checked_fixed_bones(&bones, "lid, hinge"), ("lid".to_owned(), "hinge".to_owned()));
        assert_eq!(checked_fixed_bones(&bones, "hinge"), ("root".to_owned(), "hinge".to_owned()));
        assert_eq!(checked_fixed_bones(&[], "hinge"), ("hinge".to_owned(), String::new()));
    }

    #[test]
    fn bone_names_are_read() {
        let mut body = 2u32.to_le_bytes().to_vec();
        body.extend_from_slice(b"root\0\0");
        body.extend_from_slice(&[0u8; 60]);
        body.extend_from_slice(b"lid\0root\0");
        body.extend_from_slice(&[0u8; 60]);
        let file = chunk(13, &body);
        assert_eq!(ogf_bone_names(&file), ["root", "lid"]);
    }
}

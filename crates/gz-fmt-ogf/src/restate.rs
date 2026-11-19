//! A compiled model said again: the same file with what its parts wear, its user data and its
//! bones' physics changed.
//!
//! A module that only dresses a model of the game differently has no source to compile from -
//! the model is the game's `.ogf`. The file is walked chunk by chunk and every chunk nobody
//! asked to change is copied byte for byte (its compression mark with it): the vertices, the
//! skeleton, the motion references and the levels of detail stay exactly the game's. A model
//! nothing was asked of is the same bytes.

use std::borrow::Cow;

use gz_chunk::{Reader, Writer};

use crate::bones::{read_ik, read_names};
use crate::format::chunk;
use crate::{Result, invalid};

/// The mark of a compressed chunk in its id.
const COMPRESSED: u32 = 1 << 31;
/// How deep a model's children may nest.
const MAX_DEPTH: u32 = 8;

/// What a bone gets in place of its own. Its game material is not among it: the game stops on a
/// model whose bone has a material that is not a dynamic one, so the file keeps the one it
/// loads with, and another is said in the model's description, where the game tells it first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneChange {
    pub mass: Option<f32>,
}

/// What a part wearing `(texture, shader)` wears instead; `None` leaves the part alone.
pub type Wear<'a> = &'a dyn Fn(&str, &str) -> Option<(String, String)>;

/// What a compiled model is said again with.
pub struct Restate<'a> {
    pub wear: Wear<'a>,
    /// The model's user data, when it is set anew.
    pub user_data: Option<&'a str>,
    /// What a bone, by its name, gets in place of its own.
    pub bone: &'a dyn Fn(&str) -> BoneChange,
}

/// How many things of the model were changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Restated {
    pub parts: usize,
    pub bones: usize,
}

/// The model's bytes with the changes applied.
pub fn restate(bytes: &[u8], with: &Restate<'_>) -> Result<(Vec<u8>, Restated)> {
    let mut done = Restated::default();
    let out = walk(bytes, with, 0, &mut done)?;
    Ok((out, done))
}

fn stored(w: &mut Writer, id: u32, compressed: bool, body: &[u8]) {
    w.w_u32(if compressed { id | COMPRESSED } else { id });
    w.w_u32(body.len() as u32);
    w.w(body);
}

/// A chunk's body as it reads: unpacked when its mark says it is packed.
fn plain(body: &[u8], compressed: bool) -> Result<Cow<'_, [u8]>> {
    Ok(if compressed { Cow::Owned(gz_chunk::lzhuf::decompress(body)?) } else { Cow::Borrowed(body) })
}

/// The children of a model said again, or `None` when nothing of them was asked to change: the
/// chunk then stays as it is stored.
fn children(body: &[u8], with: &Restate<'_>, depth: u32, done: &mut Restated) -> Result<Option<Vec<u8>>> {
    let before = *done;
    let cr = Reader::new(body);
    let mut inner = Writer::with_capacity(body.len() + 64);
    let mut end = 0usize;
    for (child, compressed, part) in cr.chunks() {
        end += 8 + part.len();
        let said = walk(&plain(part, compressed)?, with, depth + 1, done)?;
        stored(&mut inner, child, false, &said);
    }
    if end != body.len() {
        return Err(invalid("the model's children end in the middle of a chunk"));
    }
    Ok((*done != before).then(|| inner.into_bytes()))
}

fn walk(bytes: &[u8], with: &Restate<'_>, depth: u32, done: &mut Restated) -> Result<Vec<u8>> {
    if depth > MAX_DEPTH {
        return Err(invalid("the model's children nest too deep"));
    }
    let r = Reader::new(bytes);
    // the engine reads the bones' names as they are stored, whatever the chunk's mark says
    let names = match r.find_chunk(chunk::S_BONE_NAMES) {
        Some((body, _)) => read_names(body)?,
        None => Vec::new(),
    };
    let mut w = Writer::with_capacity(bytes.len() + 64);
    let mut user_data_written = false;
    let mut end = 0usize;
    for (id, compressed, body) in r.chunks() {
        end += 8 + body.len();
        match id {
            // the engine reads the texture chunk as stored, whatever its mark says
            chunk::TEXTURE => {
                let mut t = Reader::new(body);
                let (texture, shader) = (t.r_stringz()?, t.r_stringz()?);
                match (with.wear)(&texture, &shader) {
                    Some((texture, shader)) => {
                        done.parts += 1;
                        w.chunk(chunk::TEXTURE, |w| {
                            w.w_stringz(&texture);
                            w.w_stringz(&shader);
                        });
                    }
                    None => stored(&mut w, id, compressed, body),
                }
            }
            // this very chunk, not the first of its id: a model said again holds each as it was
            chunk::CHILDREN => match children(&plain(body, compressed)?, with, depth, done)? {
                Some(said) => stored(&mut w, id, false, &said),
                None => stored(&mut w, id, compressed, body),
            },
            chunk::S_USERDATA if with.user_data.is_some() => {
                user_data_written = true;
                // user data said empty is a model without any: no chunk, not an empty one
                if let Some(text) = with.user_data.filter(|t| !t.is_empty()) {
                    w.chunk(chunk::S_USERDATA, |w| w.w_stringz(text));
                }
            }
            chunk::S_IKDATA if !names.is_empty() => {
                let mut ik = read_ik(&plain(body, compressed)?, names.len())?;
                let mut changed = 0;
                for (bone, name) in ik.iter_mut().zip(&names) {
                    let change = (with.bone)(&name.name);
                    let Some(mass) = change.mass.filter(|m| *m != bone.mass) else { continue };
                    changed += 1;
                    bone.mass = mass;
                }
                if changed == 0 {
                    stored(&mut w, id, compressed, body);
                } else {
                    done.bones += changed;
                    w.chunk(chunk::S_IKDATA, |w| ik.iter().for_each(|b| b.write(w)));
                }
            }
            _ => stored(&mut w, id, compressed, body),
        }
    }
    // what follows the last whole chunk is no chunk: a model that ends in the middle of one is
    // not said again as if it were whole
    if end != bytes.len() {
        return Err(invalid("the model ends in the middle of a chunk"));
    }
    if depth == 0
        && !user_data_written
        && let Some(text) = with.user_data
        && !text.is_empty()
    {
        w.chunk(chunk::S_USERDATA, |w| w.w_stringz(text));
    }
    Ok(w.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Visual;
    use crate::export::{ExportOptions, export_skeleton};

    fn skinned() -> Vec<u8> {
        let mut o = crate::export::tests::skeleton();
        o.user_data = "[collide]\nignore_static\n".into();
        export_skeleton(&o, &ExportOptions::default(), 2).unwrap().bytes
    }

    #[test]
    fn nothing_asked_is_the_same_bytes() {
        let bytes = skinned();
        let with = Restate { wear: &|_, _| None, user_data: None, bone: &|_| BoneChange::default() };
        let (out, done) = restate(&bytes, &with).unwrap();
        assert_eq!(out, bytes);
        assert_eq!(done, Restated::default());
    }

    #[test]
    fn a_part_wears_another_texture_and_the_rest_stays() {
        let bytes = skinned();
        let before = Visual::read(&bytes).unwrap();
        let with = Restate {
            wear: &|t, s| (t == "tex\\a").then(|| ("mod\\crate_red".to_owned(), s.to_owned())),
            user_data: Some("[collide]\n"),
            bone: &|n| match n {
                "root" => BoneChange { mass: Some(42.0) },
                _ => BoneChange::default(),
            },
        };
        let (out, done) = restate(&bytes, &with).unwrap();
        assert_eq!(done, Restated { parts: 1, bones: 1 });
        let after = Visual::read(&out).unwrap();
        let worn: Vec<_> = after.meshes().iter().filter_map(|m| m.texture.clone()).collect();
        assert_eq!(worn, vec![("mod\\crate_red".into(), "models\\model".into())]);
        assert_eq!(after.user_data.as_deref(), Some("[collide]\n"));
        assert_eq!(after.ik[0].mass, 42.0);
        // the material a bone loads with is the file's own
        assert_eq!(after.ik[0].game_mtl, before.ik[0].game_mtl);
        // the geometry and the skeleton are the model's own
        assert_eq!(after.bones, before.bones);
        let (a, b) = (after.meshes(), before.meshes());
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].vertices, b[0].vertices);
        assert_eq!(a[0].indices, b[0].indices);
    }

    /// A file's top-level chunks packed the way the game's tools may store them.
    fn packed(bytes: &[u8], which: u32) -> Vec<u8> {
        let mut w = Writer::new();
        for (id, compressed, body) in Reader::new(bytes).chunks() {
            if id == which && !compressed {
                stored(&mut w, id, true, &gz_chunk::lzhuf::compress(body));
            } else {
                stored(&mut w, id, compressed, body);
            }
        }
        w.into_bytes()
    }

    #[test]
    fn packed_chunks_stay_as_they_are_stored_and_read_when_asked() {
        let nothing = Restate { wear: &|_, _| None, user_data: None, bone: &|_| BoneChange::default() };
        for which in [chunk::CHILDREN, chunk::S_IKDATA] {
            let bytes = packed(&skinned(), which);
            let (out, done) = restate(&bytes, &nothing).unwrap();
            assert_eq!(out, bytes, "chunk {which:#x}");
            assert_eq!(done, Restated::default());
        }
        // a change inside packed children and packed bone data is made all the same
        let bytes = packed(&packed(&skinned(), chunk::CHILDREN), chunk::S_IKDATA);
        let with = Restate {
            wear: &|t, s| (t == "tex\\a").then(|| ("mod\\red".to_owned(), s.to_owned())),
            user_data: None,
            bone: &|_| BoneChange { mass: Some(3.5) },
        };
        let (out, done) = restate(&bytes, &with).unwrap();
        assert_eq!(done.parts, 1);
        let after = Visual::read(&out).unwrap();
        assert_eq!(after.meshes()[0].texture.as_ref().map(|t| t.0.as_str()), Some("mod\\red"));
        assert!(after.ik.iter().all(|b| b.mass == 3.5));
    }

    #[test]
    fn user_data_said_empty_is_no_user_data_and_a_mass_already_there_is_no_change() {
        let bytes = skinned();
        let before = Visual::read(&bytes).unwrap();
        let mass = before.ik[0].mass;
        let with =
            Restate { wear: &|_, _| None, user_data: Some(""), bone: &|_| BoneChange { mass: Some(mass) } };
        let (out, _) = restate(&bytes, &with).unwrap();
        let after = Visual::read(&out).unwrap();
        assert_eq!(after.user_data, None);
        assert!(Reader::new(&out).find_chunk(chunk::S_USERDATA).is_none());
    }

    #[test]
    fn a_cut_file_is_refused() {
        let bytes = skinned();
        let with = Restate { wear: &|_, _| None, user_data: None, bone: &|_| BoneChange::default() };
        assert!(restate(&bytes[..bytes.len() - 3], &with).is_err());
    }
}

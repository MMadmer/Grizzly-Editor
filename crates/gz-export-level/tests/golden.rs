//! The level exports against what the C++ editor wrote for the same object: `tests/data` holds the
//! oracle run's files (`tools/qa/diff_editors.py`, scenario `s6_scene`: `statics\army_base\prop_map_02`
//! placed in Escape at (-226, -20, -146), turned 0.5 rad about Y, scaled 1.25, selected alone, then
//! Collision Overlay, Cut and Visual Overlay). The overlay and the cut byte for byte (one game
//! material). The model's children - one per surface - and the collision mesh after them follow the
//! C++'s surface map, keyed by `CSurface` pointers (D14): they are compared as the differential
//! compares them (STATE.md, "For the plan"), every other chunk byte for byte. The model's build name
//! is the oracle machine's `\\computer\user`, written over with a QA one of the same length.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;

use gz_export_level::overlay;
use gz_scene::library::LibraryObject;
use gz_scene::math::Vec3;
use gz_scene::objects::SceneObjectData;
use gz_scene::{ObjClass, ObjData, Object, SceneState};

const REFERENCE: &str = "statics\\army_base\\prop_map_02";
const BUILD_NAME: &str = "\\\\QAMACHINE000000\\quser";

fn data(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join(name)).unwrap()
}

fn scene() -> (SceneState, Arc<LibraryObject>) {
    let object = gz_fmt_object::Object::read(&data("prop_map_02.object")).unwrap();
    let lib = Arc::new(LibraryObject::from_object(REFERENCE, "prop_map_02.object".into(), object));
    let mut o = Object::new(
        ObjClass::SceneObject,
        "qa_golden",
        ObjData::SceneObject(SceneObjectData::library(REFERENCE)),
    );
    o.xform.position = Vec3::new(-226.0, -20.0, -146.0);
    o.xform.rotation = Vec3::new(0.0, 0.5, 0.0);
    o.xform.scale = Vec3::splat(1.25);
    let mut s = SceneState::default();
    s.insert(o);
    (s, lib)
}

/// The chunks of a chunk stream: id and body, in file order.
fn chunks(data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let id = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let size = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push((id, &data[at + 8..at + 8 + size]));
        at += 8 + size;
    }
    out
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

/// Two models are the same: every chunk but the children (9) and the collision mesh (25, 26) byte
/// for byte, the children as a multiset of their bytes, the collision triangles as a multiset of
/// their corners.
fn same_model(rust: &[u8], cpp: &[u8]) {
    let (a, b) = (chunks(rust), chunks(cpp));
    assert_eq!(a.iter().map(|c| c.0).collect::<Vec<_>>(), b.iter().map(|c| c.0).collect::<Vec<_>>());
    let get = |list: &[(u32, &[u8])], id: u32| list.iter().find(|c| c.0 == id).map(|c| c.1.to_vec()).unwrap();
    for (id, body) in &a {
        if ![9, 25, 26].contains(id) {
            assert_eq!(*body, get(&b, *id).as_slice(), "chunk {id}");
        }
    }
    let children =
        |list: &[(u32, &[u8])]| sorted(chunks(&get(list, 9)).into_iter().map(|c| c.1.to_vec()).collect());
    assert_eq!(children(&a), children(&b), "the children as a set");
    let tris = |list: &[(u32, &[u8])]| {
        let (verts, idx) = (get(list, 25), get(list, 26));
        let corner = |i: u32| verts[i as usize * 12..i as usize * 12 + 12].to_vec();
        let ids: Vec<u32> = idx.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        sorted(
            ids.as_chunks::<3>()
                .0
                .iter()
                .map(|t| [corner(t[0]), corner(t[1]), corner(t[2])])
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(tris(&a), tris(&b), "the collision triangles as a set");
}

#[test]
fn the_collision_overlay_is_the_cpp_one() {
    let (s, lib) = scene();
    let look = move |_: &str| Some(lib.clone());
    let made = overlay::collision(&s, false, 0, &look, &[]).unwrap();
    assert_eq!(made.form.write(), data("golden.xcform"));
}

#[test]
fn the_cut_is_the_cpp_one() {
    let (s, lib) = scene();
    let bounds = |o: &Object| gz_scene::objects::world_box(&s, o, &|_: &str| Some(lib.bounds));
    let made = overlay::cut(&s, false, false, 0.0, &bounds, &data("golden.xcform"), "").unwrap();
    assert_eq!(made.added, 1);
    assert_eq!(made.form.write(), data("golden_cut.xcform"));
    assert_eq!(made.ltx.as_bytes(), data("golden_cut.ltx").as_slice());
}

#[test]
fn the_visual_overlay_is_the_cpp_one() {
    let (s, lib) = scene();
    let look = move |_: &str| Some(lib.clone());
    let made = overlay::visuals(&s, false, &look, BUILD_NAME, None).unwrap();
    assert_eq!(made.files.len(), 1);
    assert_eq!(made.files[0].name, "qa_golden");
    same_model(&made.files[0].bytes, &data("golden.ogf"));
    let cut = String::from_utf8(data("golden_cut.ltx")).unwrap();
    let ltx = overlay::registry(&cut, &["qa_golden".to_owned()]);
    assert_eq!(ltx, String::from_utf8(data("golden_ogf.ltx")).unwrap());
}

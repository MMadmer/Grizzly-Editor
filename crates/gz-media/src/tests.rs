//! The media kinds over fixtures the tests write as text (Lua, OBJ, glTF JSON) and pictures and
//! sounds they make in memory, into temp folders of their own.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{RelPath, Severity};
use gz_fmt_object::glam::Vec3;
use gz_fmt_object::{Object, ObjectFlags};
use gz_fmt_sound::SoundThm;
use gz_fmt_tex::{
    BumpMode, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams, TextureType,
};
use gz_luadata::{LuaDataError, LuaDoc, Seg, write_asset};
use gz_project::asset_kind::MediaKind;

use crate::check::{self, texture_known};
use crate::index::{self, Entry, Index};
use crate::model::{BOX, CYLINDER, PLANE, PRISM, Part, RAMP};
use crate::publish::{Payload, publish};
use crate::read::Issue;
use crate::service::GameLists;
use crate::sound::SoundAsset;
use crate::texture::TextureAsset;
use crate::{fields, files, import, kind, parts, resolve};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("gz-media-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    fn put(&self, rel: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(rel.replace('/', "\\"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A tenth of a second of a tone: RIFF, PCM, 16 bits, mono, 22050 Hz.
fn wav() -> Vec<u8> {
    let samples: Vec<i16> = (0..2205).map(|i| ((i as f32 * 0.1).sin() * 8000.0) as i16).collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&22050u32.to_le_bytes());
    out.extend_from_slice(&(22050u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// A unit cube from 0 to 1 with one material that wears a picture.
const CUBE_OBJ: &str = "mtllib cube.mtl\n\
v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
usemtl brick\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

const CUBE_MTL: &str = "newmtl brick\nKd 1 1 1\nmap_Kd tex/brick.png\n";

fn issues_of(dir: &Dir, rel: &str, kind: MediaKind, index: &Index, game: &GameLists) -> Vec<Issue> {
    let text = std::fs::read_to_string(dir.0.join(rel.replace('/', "\\"))).unwrap();
    let v = kind::load(kind, rel, &text).unwrap();
    check::issues(&dir.0, rel, kind, &v, index, game, &resolve::no_game())
}

fn codes(i: &[Issue]) -> Vec<&'static str> {
    i.iter().map(|x| x.code).collect()
}

// ---- versions --------------------------------------------------------------------------------

/// A version 1 called the source `src`.
fn v1_to_v2(doc: &mut LuaDoc) -> Result<(), LuaDataError> {
    if let Some(s) = doc.get(&[Seg::key("src")]) {
        doc.set(&[Seg::key("source")], &s)?;
        doc.remove(&[Seg::key("src")])?;
    }
    Ok(())
}

const STEPS: &[gz_asset::Migration] = &[v1_to_v2];

#[test]
fn every_kind_reads_its_older_versions_through_its_steps_and_refuses_a_newer_one() {
    for k in MediaKind::ALL {
        let h = k.header();
        // the editor's own entry: version 1, no steps, a newer file refused by name
        let e = kind::load(k, "a/x.asset", &format!("return {{ {h} = 2 }}")).unwrap_err();
        assert!(e.contains("made by a newer Grizzly Editor") && e.contains(&format!("{h} = 2")), "{e}");
        assert!(kind::load(k, "a/x.asset", &format!("return {{ {h} = 1, source = \"s\" }}")).is_ok());
        // a chain of one step lifts a version 1 file to 2 in memory
        let lifted = Arc::new(kind::ops_with(k, 2, STEPS));
        let v =
            kind::load_with(&lifted, "a/x.asset", &format!("return {{ {h} = 1, src = \"s.png\" }}")).unwrap();
        assert_eq!(v.get_number(h, 0.0), 2.0);
        assert_eq!(v.get_string("source", ""), "s.png");
        assert!(v.get("src").is_none());
        let e = kind::load_with(&lifted, "a/x.asset", &format!("return {{ {h} = 3 }}")).unwrap_err();
        assert!(e.contains("made by a newer Grizzly Editor"), "{e}");
    }
}

#[test]
fn the_kinds_know_the_keys_of_their_tables() {
    for (k, lists) in [
        (
            MediaKind::Model,
            vec![
                ("", fields::MODEL),
                ("surfaces.*", fields::SURFACE),
                ("parts.*", fields::PART),
                ("bones.*", fields::BONE),
                ("animations.*", fields::ANIMATION),
            ],
        ),
        (MediaKind::Texture, vec![("", fields::TEXTURE), ("bump", fields::BUMP), ("detail", fields::DETAIL)]),
        (MediaKind::Sound, vec![("", fields::SOUND)]),
    ] {
        let known = kind::known(k);
        for (at, table) in lists {
            let (_, keys) = known.iter().find(|(p, _)| *p == at).unwrap();
            assert_eq!(*keys, fields::keys(table).as_slice(), "{k:?} {at}");
        }
    }
}

// ---- the texture -----------------------------------------------------------------------------

fn sample_params() -> Vec<TextureParams> {
    let mut all = Vec::new();
    let full = TextureParams {
        fmt: TextureFormat::DXT5,
        flags: TextureFlags::from_bits_retain(
            (TextureFlags::BINARY_ALPHA
                | TextureFlags::ALPHA_BORDER
                | TextureFlags::COLOR_BORDER
                | TextureFlags::FADE_TO_COLOR
                | TextureFlags::FADE_TO_ALPHA
                | TextureFlags::DITHER_EACH_MIP_LEVEL
                | TextureFlags::IMPLICIT_LIGHTED
                | TextureFlags::HAS_ALPHA
                | TextureFlags::BUMP_DETAIL)
                .bits()
                | 0x8000_0400,
        ),
        border_color: 0x1122_3344,
        fade_color: 0xaabb_ccdd,
        fade_amount: 50,
        fade_delay: 3,
        mip_filter: MipFilterId::GAUSSIAN,
        width: 256,
        height: 128,
        detail_name: "detail\\detail_grnd".into(),
        detail_scale: 6.5,
        ty: TextureType::IMAGE,
        material: TextureMaterial::PHONG_METAL,
        material_weight: 0.25,
        bump_virtual_height: 0.02,
        bump_mode: BumpMode::USE_PARALLAX,
        bump_name: "wall\\wall_bump".into(),
        ext_normal_map_name: "wall\\wall_nmap".into(),
    };
    all.push(full.clone());
    // every type, every format, every filter, every lighting model, both bump modes
    for ty in 0..5 {
        all.push(TextureParams { ty: TextureType(ty), width: 64, height: 64, ..TextureParams::default() });
    }
    for (_, f) in fields::FORMATS.iter().filter(|(_, f)| *f != fields::AUTO) {
        all.push(TextureParams { fmt: TextureFormat(*f), width: 8, height: 8, ..TextureParams::default() });
    }
    for (_, m) in fields::MIP_FILTERS {
        all.push(TextureParams {
            mip_filter: MipFilterId(*m),
            width: 8,
            height: 8,
            ..TextureParams::default()
        });
    }
    for (_, m) in fields::MATERIALS {
        all.push(TextureParams {
            material: TextureMaterial(*m),
            width: 8,
            height: 8,
            ..TextureParams::default()
        });
    }
    all.push(TextureParams {
        bump_mode: BumpMode::USE,
        bump_name: "tex\\own_bump".into(),
        width: 8,
        height: 8,
        ..TextureParams::default()
    });
    // a detail on neither, a detail on both
    let mut neither =
        TextureParams { detail_name: "d\\x".into(), width: 8, height: 8, ..TextureParams::default() };
    all.push(neither.clone());
    neither.flags |= TextureFlags::DIFFUSE_DETAIL | TextureFlags::BUMP_DETAIL;
    all.push(neither);
    // no mips, no dither
    all.push(TextureParams { flags: TextureFlags::empty(), width: 8, height: 8, ..TextureParams::default() });
    all
}

#[test]
fn a_texture_description_round_trips_through_the_asset_and_its_text() {
    for p in sample_params() {
        for alpha in [false, true] {
            let size = (p.width as u32, p.height as u32);
            let asset = TextureAsset::from_params(&p, "tex\\own", alpha);
            assert_eq!(asset.to_params("tex\\own", size, alpha), p, "{asset:?}");
            // and through the file's text
            let text = write_asset("", &asset.to_table());
            let v = kind::load(MediaKind::Texture, "t/own.texasset", &text).unwrap();
            let (back, issues) = TextureAsset::from_value(&v);
            assert!(issues.is_empty(), "{issues:?}\n{text}");
            assert_eq!(back.to_params("tex\\own", size, alpha), p, "{text}");
        }
    }
}

#[test]
fn a_texture_written_with_words_starts_where_its_use_does() {
    let v = gz_luadata::parse("return { tex = 1, use = \"normal\" }").unwrap();
    let p = TextureAsset::from_value(&v).0.to_params("n", (4, 4), false);
    assert_eq!(
        (p.ty, p.fmt, p.mip_filter),
        (TextureType::NORMAL_MAP, TextureFormat::RGBA, MipFilterId::KAISER)
    );
    let v = gz_luadata::parse("return { tex = 1, use = \"ui\" }").unwrap();
    let p = TextureAsset::from_value(&v).0.to_params("n", (4, 4), true);
    assert_eq!((p.ty, p.fmt), (TextureType::IMAGE, TextureFormat::DXT5));
    assert!(!p.flags.contains(TextureFlags::GENERATE_MIP_MAPS));
    // a bump made of a source is the texture's own `<name>_bump`
    let v = gz_luadata::parse("return { tex = 1, bump = { source = \"h.png\" } }").unwrap();
    let p = TextureAsset::from_value(&v).0.to_params("walls\\brick", (4, 4), false);
    assert_eq!((p.bump_mode, p.bump_name.as_str()), (BumpMode::USE, "walls\\brick_bump"));
    // the short forms the game reads are read here the same: true a bump under the texture's
    // name, false none; a detail said false is none
    let said = |text: &str| TextureAsset::from_value(&gz_luadata::parse(text).unwrap());
    let (yes, issues) = said("return { tex = 1, bump = true }");
    assert!(issues.is_empty() && yes.bump == Some(crate::texture::Bump::default()), "{issues:?}");
    let (no, issues) = said("return { tex = 1, bump = false, detail = false }");
    assert!(issues.is_empty() && no.bump.is_none() && no.detail.is_none(), "{issues:?}");
    assert_eq!(codes(&said("return { tex = 1, detail = true }").1), ["M109"]);
    assert_eq!(codes(&said("return { tex = 1, bump = 3 }").1), ["M109"]);
}

#[test]
fn a_texture_value_outside_its_key_is_an_issue_at_the_key() {
    let v = gz_luadata::parse(
        "return { tex = 1, format = \"dxt9\", fade_amount = 200, border_color = { 1, 2, 3 }, mips = \"yes\", bump = { depth = 2 } }",
    )
    .unwrap();
    let (t, issues) = TextureAsset::from_value(&v);
    let at: Vec<String> = issues.iter().map(|i| gz_luadata::path::show(&i.path)).collect();
    assert_eq!(at, ["format", "mips", "border_color", "fade_amount", "bump.depth"], "{issues:?}");
    assert!(issues.iter().all(|i| i.code == "M109" && i.is_error()));
    assert!(t.format.is_none() && t.fade_amount.is_none());
    assert!(issues[0].message.contains("dxt1"), "{}", issues[0].message);
}

// ---- the sound -------------------------------------------------------------------------------

#[test]
fn a_sound_description_round_trips_and_its_type_is_a_word() {
    let thm = SoundThm {
        quality: 0.4,
        min_dist: 2.0,
        max_dist: 40.0,
        max_ai_dist: 25.0,
        base_volume: 0.5,
        game_type: 0x0800_0080,
    };
    let a = SoundAsset::from_thm(&thm);
    assert_eq!(a.to_thm(), thm);
    let text = write_asset("", &a.to_table());
    assert!(text.contains("type = \"world_ambient\""), "{text}");
    let v = kind::load(MediaKind::Sound, "s/x.sndasset", &text).unwrap();
    assert_eq!(SoundAsset::from_value(&v).0.to_thm(), thm);
    // the defaults, and the AI distance that follows the distance
    let v = gz_luadata::parse("return { snd = 1, max_distance = 12 }").unwrap();
    let t = SoundAsset::from_value(&v).0.to_thm();
    assert_eq!((t.min_dist, t.max_dist, t.max_ai_dist, t.base_volume), (1.0, 12.0, 12.0, 1.0));
    let v = gz_luadata::parse("return { snd = 1, type = \"loud\", max_distance = 0 }").unwrap();
    assert_eq!(codes(&SoundAsset::from_value(&v).1), ["M109", "M109"]);
    // the type is said in the game's own words, one or a list of them, and written back the same
    let ty = |text: &str| SoundAsset::from_value(&gz_luadata::parse(text).unwrap());
    assert_eq!(ty("return { snd = 1, type = \"monster_step\" }").0.ty, Some(0x2000_8000));
    assert_eq!(ty("return { snd = 1, type = \"world_object_breaking\" }").0.ty, Some(0x0800_0400));
    assert_eq!(ty("return { snd = 1, type = \"object_breaking\" }").0.ty, Some(0x0000_0400));
    assert_eq!(ty("return { snd = 1, type = \"no_sound\" }").0.ty, Some(0));
    let (both, issues) = ty("return { snd = 1, type = { \"weapon\", \"item\", \"shooting\" } }");
    assert!(issues.is_empty() && both.ty == Some(0xc020_0000), "{issues:?} {:?}", both.ty);
    let text = write_asset("", &both.to_table());
    assert!(
        text.contains("\"weapon\"") && text.contains("\"item\"") && text.contains("\"shooting\""),
        "{text}"
    );
    assert_eq!(ty(&text).0.ty, both.ty);
    assert_eq!(codes(&ty("return { snd = 1, type = { \"weapon\", \"npc_step\" } }").1), ["M109"]);
    assert_eq!(codes(&ty("return { snd = 1, type = 7 }").1), ["M109"]);
    // every word is one the game reads: none twice, and the bits the game has
    let words: Vec<&str> = crate::fields::SOUND_TYPES.iter().map(|(w, _)| *w).collect();
    let mut once = words.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(once.len(), words.len());
    assert_eq!(crate::fields::SOUND_TYPES.iter().fold(0, |all, (_, v)| all | v), 0xffff_ff80);
}

// ---- the parts -------------------------------------------------------------------------------

/// The signed volume of a model's faces: positive when every face looks outward.
fn volume(o: &Object) -> f32 {
    o.meshes
        .iter()
        .map(|m| {
            m.faces
                .iter()
                .map(|f| {
                    let [a, b, c] = f.v.map(|v| m.points[v.point as usize]);
                    a.dot(b.cross(c)) / 6.0
                })
                .sum::<f32>()
        })
        .sum()
}

fn faces(o: &Object) -> usize {
    o.meshes.iter().map(|m| m.faces.len()).sum()
}

fn one(part: Part) -> Object {
    parts::build(&[part], &[]).unwrap()
}

#[test]
fn every_shape_has_its_bounds_and_its_faces_and_looks_outward() {
    let boxed = one(Part { shape: BOX, size: Some(vec![4.0, 3.0, 2.0]), ..Part::default() });
    let b = boxed.bounding_box();
    assert_eq!((b.min, b.max), (Vec3::new(-2.0, 0.0, -1.0), Vec3::new(2.0, 3.0, 1.0)));
    assert_eq!(faces(&boxed), 12);
    assert!((volume(&boxed) - 24.0).abs() < 1e-3, "{}", volume(&boxed));

    let ramp = one(Part { shape: RAMP, size: Some(vec![2.0, 1.0, 4.0]), ..Part::default() });
    let b = ramp.bounding_box();
    assert_eq!((b.min, b.max), (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.0, 2.0)));
    assert_eq!(faces(&ramp), 8);
    assert!((volume(&ramp) - 4.0).abs() < 1e-3, "{}", volume(&ramp));

    let cyl = one(Part { shape: CYLINDER, radius: 1.0, height: 2.0, sides: 12, ..Part::default() });
    let b = cyl.bounding_box();
    assert!((b.max.x - 1.0).abs() < 1e-4 && (b.min.y, b.max.y) == (0.0, 2.0));
    assert_eq!(faces(&cyl), 4 * 12 - 4);
    // a 12-sided prism of radius 1: 3 r^2 a side of area, 2 high
    assert!((volume(&cyl) - 6.0).abs() < 1e-3, "{}", volume(&cyl));

    let plane = one(Part { shape: PLANE, size: Some(vec![10.0, 6.0]), ..Part::default() });
    let b = plane.bounding_box();
    assert_eq!((b.min, b.max), (Vec3::new(-5.0, 0.0, -3.0), Vec3::new(5.0, 0.0, 3.0)));
    assert_eq!(faces(&plane), 2);
    assert!(plane.meshes[0].face_normals().iter().all(|n| (n.y - 1.0).abs() < 1e-5));

    // a concave L, written either way round
    let l = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]];
    for outline in [l.clone(), l.iter().rev().copied().collect()] {
        let prism = one(Part { shape: PRISM, outline, height: 2.0, ..Part::default() });
        assert_eq!(faces(&prism), 2 * 4 + 2 * 6);
        assert!((volume(&prism) - 12.0).abs() < 1e-3, "{}", volume(&prism));
        let b = prism.bounding_box();
        assert_eq!((b.min, b.max), (Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 2.0, 3.0)));
    }
    // every convex shape's faces look away from its middle
    for o in [&boxed, &ramp, &cyl] {
        let b = o.bounding_box();
        let mid = (b.min + b.max) * 0.5;
        let m = &o.meshes[0];
        for (f, n) in m.faces.iter().zip(m.face_normals()) {
            let c = f.v.iter().map(|v| m.points[v.point as usize]).sum::<Vec3>() / 3.0;
            assert!(n.dot(c - mid) > 0.0, "a face looks inward");
        }
    }
}

#[test]
fn parts_are_placed_turned_and_tiled() {
    let p = Part {
        shape: BOX,
        size: Some(vec![4.0, 2.0, 2.0]),
        pos: [10.0, 1.0, 0.0],
        rot: [0.0, 90.0, 0.0],
        tile: 2.0,
        surface: "brick".into(),
        ..Part::default()
    };
    let o = one(p);
    let b = o.bounding_box();
    assert!((b.min - Vec3::new(9.0, 1.0, -2.0)).abs().max_element() < 1e-4, "{:?}", b.min);
    assert!((b.max - Vec3::new(11.0, 3.0, 2.0)).abs().max_element() < 1e-4, "{:?}", b.max);
    assert_eq!(o.surfaces[0].name, "brick");
    // the top is 4 m by 2 m in the part's frame: two textures by one at tile 2
    let m = &o.meshes[0];
    let normals = m.face_normals();
    let top: Vec<usize> = (0..m.faces.len()).filter(|i| normals[*i].y > 0.9).collect();
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for f in &top {
        for uv in m.face_uv(*f).unwrap() {
            lo = [lo[0].min(uv.x), lo[1].min(uv.y)];
            hi = [hi[0].max(uv.x), hi[1].max(uv.y)];
        }
    }
    assert!(((hi[0] - lo[0]) - 2.0).abs() < 1e-4 && ((hi[1] - lo[1]) - 1.0).abs() < 1e-4, "{lo:?} {hi:?}");
    // every edge is hard
    assert!(m.smooth_groups.iter().all(|g| *g == 0b111));
}

#[test]
fn an_outline_that_crosses_itself_is_refused() {
    assert!(parts::triangulate(&[[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]]).is_err());
    assert!(parts::triangulate(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]).is_err());
    assert_eq!(parts::triangulate(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).unwrap().len(), 2);
}

// ---- models ----------------------------------------------------------------------------------

#[test]
fn a_model_asset_lays_its_transform_and_surfaces_over_its_source() {
    let d = Dir::new("model");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("models/cube.obj", CUBE_OBJ);
    d.put("models/cube.mtl", CUBE_MTL);
    d.put("models/tex/brick.png", png(4, 4, [200, 50, 50, 255]));
    d.put("models/tex/brick.texasset", "return { tex = 1 }\n");
    d.put(
        "models/cube.meshasset",
        "return {\n  mesh = 1,\n  scale = 2,\n  surfaces = { { name = \"BRICK\", material = \"materials\\\\bricks\", two_sided = true } },\n}\n",
    );
    let m = resolve::model(&d.0, "models/cube.meshasset", &resolve::no_game()).unwrap();
    let b = m.object.bounding_box();
    // OBJ is right-handed: its +Z is the game's -Z
    assert!((b.max - Vec3::new(2.0, 2.0, 0.0)).abs().max_element() < 1e-4, "{:?}", b.max);
    assert!((b.min - Vec3::new(0.0, 0.0, -2.0)).abs().max_element() < 1e-4, "{:?}", b.min);
    let s = &m.object.surfaces[0];
    assert_eq!(s.game_mtl, "materials\\bricks");
    assert!(s.two_sided());
    // the texture asset of its picture: `<module>\<file name>`
    assert_eq!(s.texture, "bunker\\brick");
    assert!(!m.object.is_dynamic());
    // the stamp follows the source's bytes
    let first = m.stamp;
    d.put("models/cube.obj", CUBE_OBJ.replace("v 1 1 1", "v 1 1 1.5"));
    assert_ne!(resolve::model(&d.0, "models/cube.meshasset", &resolve::no_game()).unwrap().stamp, first);

    // the scene's library takes it from the provider by its project path, any case
    let lib = gz_scene::library::Library::new(vec![d.0.join("rawdata").join("objects")]);
    lib.add_provider(crate::provider(d.0.clone(), resolve::no_game()));
    let o = lib.load("Models/Cube.meshasset").unwrap();
    assert!(o.file.as_os_str().is_empty() && !o.tris.is_empty() && o.stamp != 0);
    assert!(lib.load("props\\box").is_err());
}

#[test]
fn a_dynamic_model_without_bones_gets_one_and_exports() {
    let d = Dir::new("dynamic");
    d.put(
        "m/crate.meshasset",
        "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  userdata = \"[x]\",\n  parts = { { shape = \"box\" } },\n  bones = { { name = \"root\", shape = \"box\", mass = 25, material = \"objects\\\\box\" } },\n}\n",
    );
    let m = resolve::model(&d.0, "m/crate.meshasset", &resolve::no_game()).unwrap();
    assert!(m.object.flags.contains(ObjectFlags::DYNAMIC));
    assert_eq!(m.object.bones.len(), 1);
    let bone = &m.object.bones[0];
    // the material a bone is published with is its source's: the asset's is the description's
    assert_eq!((bone.name.as_str(), bone.mass, bone.game_mtl.as_str()), ("root", 25.0, "default_object"));
    assert_eq!(bone.shape.kind, 1);
    assert!((bone.shape.box_halfsize - Vec3::new(0.5, 0.5, 0.5)).abs().max_element() < 1e-5);
    assert_eq!(m.object.user_data, "[x]");
    let note = m.issues.iter().find(|i| i.code == "M108").unwrap();
    assert_eq!(note.severity, Severity::Note);
    let e = gz_fmt_ogf::export_skeleton(&m.object, &gz_fmt_ogf::ExportOptions::default(), 4).unwrap();
    assert!(gz_fmt_ogf::Visual::read(&e.bytes).is_ok());
}

// ---- the index and the checks ----------------------------------------------------------------

fn game(textures: bool) -> GameLists {
    GameLists {
        linked: textures,
        shaders: ["default".to_owned()].into_iter().collect(),
        compiler_shaders: ["default".to_owned()].into_iter().collect(),
        materials: ["default".to_owned()].into_iter().collect(),
        dynamic_materials: Default::default(),
        rights: Vec::new(),
        files: None,
    }
}

#[test]
fn the_index_names_every_asset_and_two_with_one_name_are_an_error_on_both() {
    let d = Dir::new("index");
    d.put("mod.ltx", "[module]\nid = bunker\n\n[exclude]\nwip\n");
    d.put("a/brick.png", png(4, 4, [1, 2, 3, 255]));
    d.put("a/brick.texasset", "return { tex = 1 }\n");
    d.put("b/other.png", png(4, 4, [1, 2, 3, 255]));
    d.put("b/other.texasset", "return {\n  tex = 1,\n  name = \"Bunker/Brick\",\n}\n");
    d.put("wip/brick.texasset", "return { tex = 1, source = \"x.png\" }\n");
    d.put("s/hum.wav", wav());
    d.put("s/hum.sndasset", "return { snd = 1 }\n");
    d.put("temp/media/x.texasset", "return { tex = 1 }\n");
    let index = index::scan(&d.0);
    assert_eq!(index.module, "bunker");
    let paths: Vec<&str> = index.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["a/brick.texasset", "b/other.texasset", "s/hum.sndasset", "wip/brick.texasset"]);
    assert_eq!(index.entries[0].name, "bunker\\brick");
    assert!(index.entries[3].excluded && !index.entries[3].ships());
    assert_eq!(index.named(MediaKind::Texture, "BUNKER\\brick").len(), 2);
    assert_eq!(index.names(MediaKind::Sound), ["bunker\\hum"]);
    assert!(
        index.is_source("a/brick.png")
            && index.is_source("A/Brick.TexAsset")
            && !index.is_source("a/readme.txt")
    );
    let g = game(false);
    let a = issues_of(&d, "a/brick.texasset", MediaKind::Texture, &index, &g);
    let b = issues_of(&d, "b/other.texasset", MediaKind::Texture, &index, &g);
    assert_eq!(codes(&a), ["M103"]);
    assert!(a[0].path.is_empty() && a[0].message.contains("b/other.texasset"), "{a:?}");
    assert_eq!(codes(&b), ["M103"]);
    assert_eq!(gz_luadata::path::show(&b[0].path), "name");
    // an excluded one with the same name is nobody's rival, and its own source is missing
    let w = issues_of(&d, "wip/brick.texasset", MediaKind::Texture, &index, &g);
    assert_eq!(codes(&w), ["M101"]);
    assert_eq!(gz_luadata::path::show(&w[0].path), "source");
    assert!(issues_of(&d, "s/hum.sndasset", MediaKind::Sound, &index, &g).is_empty());
    assert_eq!(
        index.published_paths(),
        [
            "gamedata\\textures\\bunker\\brick.dds",
            "gamedata\\textures\\bunker\\brick.texasset",
            "gamedata\\textures\\Bunker\\Brick.dds",
            "gamedata\\textures\\Bunker\\Brick.texasset",
            "gamedata\\sounds\\bunker\\hum.ogg",
            "gamedata\\sounds\\bunker\\hum.sndasset",
        ]
    );
}

#[test]
fn a_model_names_its_problems_at_their_keys() {
    let d = Dir::new("checks");
    d.put("m/gone.meshasset", "return { mesh = 1, source = \"gone.glb\" }\n");
    d.put("m/broken.obj", "v 1 2\nf 1 2 3\n");
    d.put("m/broken.meshasset", "return { mesh = 1, source = \"broken.obj\" }\n");
    d.put("m/both.meshasset", "return { mesh = 1, source = \"x.obj\", parts = { { shape = \"box\" } } }\n");
    d.put(
        "m/ok.meshasset",
        "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  scale = 0,\n  parts = { { shape = \"box\", surface = \"s\" }, { shape = \"star\" } },\n  surfaces = {\n    { name = \"s\", texture = \"nobody\\\\has\", shader = \"bogus\", compile = \"bogus\", material = \"bogus\" },\n  },\n}\n",
    );
    d.put("m/bare.meshasset", "return { mesh = 1, parts = { { shape = \"box\" } } }\n");
    // a source that is there, outside the project; and one named by a drive, which is no path
    let outside = d.0.with_file_name(format!("gz-media-outside-{}.obj", std::process::id()));
    std::fs::write(&outside, "v 0 0 0\nv 1 0 0\nv 0 0 1\nf 1 2 3\n").unwrap();
    let up = format!("../../{}", outside.file_name().unwrap().to_string_lossy());
    d.put("m/out.meshasset", format!("return {{ mesh = 1, source = \"{up}\" }}\n"));
    d.put("m/drive.meshasset", "return { mesh = 1, source = \"C:/art/hut.glb\" }\n");
    d.put("rawdata/textures/mine/own.tga", "x");
    let index = index::scan(&d.0);
    let g = game(true);
    // a surface without a texture: what the level build would refuse is said here first
    let bare = issues_of(&d, "m/bare.meshasset", MediaKind::Model, &index, &g);
    assert_eq!(codes(&bare), ["M112"]);
    assert!(bare[0].is_error() && gz_luadata::path::show(&bare[0].path) == "surfaces", "{bare:?}");
    let out = issues_of(&d, "m/out.meshasset", MediaKind::Model, &index, &g);
    let _ = std::fs::remove_file(&outside);
    assert_eq!(codes(&out), ["M113"], "{out:?}");
    assert!(out[0].is_error() && gz_luadata::path::show(&out[0].path) == "source");
    assert_eq!(codes(&issues_of(&d, "m/drive.meshasset", MediaKind::Model, &index, &g)), ["M101"]);
    assert!(crate::files::inside(&d.0, &d.0.join("m").join("a.glb")));
    assert!(!crate::files::inside(&d.0, &d.0.with_file_name("elsewhere").join("a.glb")));
    assert!(!crate::files::inside(&d.0, &d.0));
    let gone = issues_of(&d, "m/gone.meshasset", MediaKind::Model, &index, &g);
    assert_eq!(codes(&gone), ["M101"]);
    assert_eq!(gz_luadata::path::show(&gone[0].path), "source");
    assert!(gone[0].message.contains("gone.glb"));
    let broken = issues_of(&d, "m/broken.meshasset", MediaKind::Model, &index, &g);
    assert_eq!(codes(&broken), ["M102"]);
    assert!(broken[0].message.contains("broken.obj"), "{}", broken[0].message);
    let both = issues_of(&d, "m/both.meshasset", MediaKind::Model, &index, &g);
    assert_eq!(codes(&both), ["M110"]);
    assert_eq!(gz_luadata::path::show(&both[0].path), "parts");
    let ok = issues_of(&d, "m/ok.meshasset", MediaKind::Model, &index, &g);
    let at: Vec<(&str, String)> = ok.iter().map(|i| (i.code, gz_luadata::path::show(&i.path))).collect();
    for want in [
        ("M109", "scale".to_owned()),
        ("M109", "parts[1].shape".to_owned()),
        ("M108", "kind".to_owned()),
        ("M104", "surfaces[name=s].texture".to_owned()),
        ("M105", "surfaces[name=s].shader".to_owned()),
        ("M106", "surfaces[name=s].compile".to_owned()),
        ("M107", "surfaces[name=s].material".to_owned()),
    ] {
        assert!(at.contains(&(want.0, want.1.clone())), "{want:?} not in {at:?}");
    }
    // what the level build refuses is an error; the rest of what the game lacks is a warning
    assert!(ok.iter().filter(|i| ["M104", "M105", "M106", "M108"].contains(&i.code)).all(|i| !i.is_error()));
    assert!(ok.iter().filter(|i| i.code == "M107").all(|i| i.is_error()));
    // a texture of the project's files, of an asset, or with no game to ask is somebody's
    assert!(texture_known(&d.0, &index, &g, "mine\\own"));
    assert!(!texture_known(&d.0, &index, &g, "nobody\\has"));
    assert!(texture_known(&d.0, &index, &game(false), "nobody\\has"));
}

// ---- the bake --------------------------------------------------------------------------------

#[test]
fn the_bake_publishes_what_ships_the_same_bytes_every_time() {
    let d = Dir::new("publish");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("t/brick.png", png(8, 8, [120, 60, 30, 255]));
    d.put("t/brick_h.png", png(8, 8, [128, 128, 128, 200]));
    d.put("t/brick.texasset", "return {\n  tex = 1,\n  bump = { source = \"brick_h.png\" },\n}\n");
    d.put("s/hum.wav", wav());
    d.put("s/hum.sndasset", "return { snd = 1, max_distance = 30, type = \"world_ambient\" }\n");
    d.put("m/crate.meshasset", "return { mesh = 1, kind = \"dynamic\", name = \"props\\\\crate\", parts = { { shape = \"box\" } } }\n");
    d.put("m/wall.meshasset", "return { mesh = 1, parts = { { shape = \"box\" } } }\n");
    let index = index::scan(&d.0);
    let run = || {
        let mut got: Vec<(String, Vec<u8>)> = Vec::new();
        let mut sink = |rel: &str, p: Payload| -> Result<(), String> {
            let bytes = match p {
                Payload::Bytes(b) => b,
                Payload::File(f) => std::fs::read(f).map_err(|e| e.to_string())?,
            };
            got.push((rel.to_owned(), bytes));
            Ok(())
        };
        let done = publish(&d.0, &index, &resolve::no_game(), &mut sink);
        (done, got)
    };
    let (first, files) = run();
    assert!(first.problems.is_empty(), "{:?}", first.problems);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "gamedata\\meshes\\props\\crate.ogf",
            "gamedata\\meshes\\props\\crate.meshasset",
            "gamedata\\sounds\\bunker\\hum.ogg",
            "gamedata\\sounds\\bunker\\hum.sndasset",
            "gamedata\\textures\\bunker\\brick.dds",
            "gamedata\\textures\\bunker\\brick_bump.dds",
            "gamedata\\textures\\bunker\\brick_bump#.dds",
            "gamedata\\textures\\bunker\\brick.texasset",
        ]
    );
    assert_eq!(index.published_paths(), names);
    assert!(files[0].1.len() > 64 && files[4].1.starts_with(b"DDS "));
    assert_eq!(gz_fmt_sound::read_ogg_params(&files[2].1).unwrap().unwrap().max_dist, 30.0);
    assert_eq!(files[3].1, std::fs::read(d.0.join("s\\hum.sndasset")).unwrap());
    assert_eq!(first.cached, 0);
    // from the cache: the same bytes
    let (second, again) = run();
    assert_eq!(second.cached, 5);
    assert_eq!(files, again);
    // and baked anew: the same bytes again
    std::fs::remove_dir_all(crate::publish::cache_dir(&d.0)).unwrap();
    let (third, anew) = run();
    assert_eq!(third.cached, 0);
    assert_eq!(files, anew);
}

/// The game reads row `n` of a shipped model's `surfaces` as its `n`th child; the author writes
/// rows by name in any order. What ships has one row per child, in the children's order.
#[test]
fn a_model_ships_its_surfaces_in_the_order_of_the_baked_model_s_children() {
    let d = Dir::new("surfaces");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    // the rows the other way round than the parts use them, one with a key the bake does not read
    let door = "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  name = \"props\\\\door\",\n  surfaces = {\n    { name = \"frame\", texture = \"wood\\\\frame\", material = \"materials\\\\wood\" },\n    { name = \"leaf\", texture = \"wood\\\\leaf\" },\n  },\n  parts = {\n    { shape = \"box\", surface = \"leaf\" },\n    { shape = \"box\", pos = { 2, 0, 0 }, surface = \"frame\" },\n  },\n}\n";
    d.put("m/door.meshasset", door);
    // one surface, said as it is baked: the author's file ships as it is
    let plank = "-- a plank\nreturn { mesh = 1, kind = \"dynamic\", name = \"props\\\\plank\", surfaces = { { name = \"wood\", texture = \"wood\\\\plank\", shader = \"default\" } }, parts = { { shape = \"box\", surface = \"wood\" } } }\n";
    d.put("m/plank.meshasset", plank);
    let index = index::scan(&d.0);
    let mut got: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sink = |rel: &str, p: Payload| -> Result<(), String> {
        let bytes = match p {
            Payload::Bytes(b) => b,
            Payload::File(f) => std::fs::read(f).map_err(|e| e.to_string())?,
        };
        got.push((rel.to_owned(), bytes));
        Ok(())
    };
    let done = publish(&d.0, &index, &resolve::no_game(), &mut sink);
    assert!(done.problems.is_empty(), "{:?}", done.problems);
    let file =
        |name: &str| &got.iter().find(|(n, _)| n.ends_with(name)).unwrap_or_else(|| panic!("{name}")).1;
    let children: Vec<String> = gz_fmt_ogf::Visual::read(file("door.ogf"))
        .unwrap()
        .children
        .iter()
        .map(|c| c.texture.as_ref().map(|(t, _)| t.to_string()).unwrap_or_default())
        .collect();
    assert_eq!(children.len(), 2, "{children:?}");
    let text = String::from_utf8(file("door.meshasset").clone()).unwrap();
    let shipped = gz_luadata::parse(&text).unwrap();
    let rows = shipped.get("surfaces").unwrap().arr();
    let said: Vec<&str> =
        rows.iter().map(|r| r.get("texture").and_then(gz_luadata::Value::as_str).unwrap()).collect();
    assert_eq!(said, children.iter().map(String::as_str).collect::<Vec<_>>(), "{text}");
    // each row is the author's of that surface, with what the bake does not read kept
    let frame =
        rows.iter().find(|r| r.get("name").and_then(gz_luadata::Value::as_str) == Some("frame")).unwrap();
    assert_eq!(frame.get("material").and_then(gz_luadata::Value::as_str), Some("materials\\wood"));
    assert!(rows.iter().all(|r| r.get("shader").and_then(gz_luadata::Value::as_str) == Some("default")));
    // the shipped text is an asset the editor reads back
    assert!(kind::load(MediaKind::Model, "m/door.meshasset", &text).is_ok());
    assert_eq!(file("plank.meshasset").as_slice(), plank.as_bytes());
}

// ---- the import ------------------------------------------------------------------------------

#[test]
fn an_import_copies_a_source_with_what_it_names_and_writes_the_assets() {
    let src = Dir::new("import-src");
    src.put("cube.obj", CUBE_OBJ);
    src.put("cube.mtl", CUBE_MTL);
    src.put("tex/brick.png", png(4, 4, [9, 9, 9, 255]));
    src.put("hum.wav", wav());
    src.put("notes.txt", "x");
    // a glTF triangle with its buffer beside it and a picture it names
    let pos: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 0., 1.].iter().flat_map(|v| v.to_le_bytes()).collect();
    src.put("tri.bin", &pos);
    src.put("pics/stone.png", png(2, 2, [1, 1, 1, 255]));
    let gltf = serde_json::json!({
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 36, "uri": "tri.bin" }],
        "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 0, 1] }],
        "images": [{ "uri": "pics/stone.png" }],
        "textures": [{ "source": 0 }],
        "materials": [{ "name": "stone", "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "material": 0 }] }],
        "nodes": [{ "mesh": 0 }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0,
    });
    src.put("tri.gltf", gltf.to_string());
    let from: Vec<PathBuf> =
        ["cube.obj", "hum.wav", "tri.gltf", "notes.txt"].iter().map(|f| src.0.join(f)).collect();
    let plan = import::plan(&from, None, "bunker", Path::new(r"Z:\no-project"));
    let rel: Vec<&str> = plan.writes.iter().map(|(r, _)| r.as_str()).collect();
    for want in [
        "models/cube.obj",
        "models/cube.mtl",
        "models/tex/brick.png",
        "models/tex/brick.texasset",
        "models/cube.meshasset",
        "sounds/hum.wav",
        "sounds/hum.sndasset",
        "models/tri.gltf",
        "models/tri.bin",
        "models/pics/stone.png",
        "models/pics/stone.texasset",
        "models/tri.meshasset",
    ] {
        assert!(rel.contains(&want), "{want} not in {rel:?}");
    }
    assert_eq!(plan.problems.len(), 1, "{:?}", plan.problems);
    assert!(plan.problems[0].starts_with("notes.txt"));
    let text = |name: &str| match &plan.writes.iter().find(|(r, _)| r.as_str() == name).unwrap().1 {
        import::Content::Bytes(b) => String::from_utf8(b.clone()).unwrap(),
        import::Content::Copy(p) => p.display().to_string(),
    };
    let cube = text("models/cube.meshasset");
    assert!(
        cube.contains("source = \"cube.obj\"") && cube.contains("texture = \"bunker\\\\brick\""),
        "{cube}"
    );
    assert!(text("models/tri.meshasset").contains("{ name = \"stone\", texture = \"bunker\\\\stone\" }"));
    assert!(text("models/pics/stone.texasset").contains("source = \"stone.png\""));
    // written into a project, every asset reads and the model resolves with its textures
    let project = Dir::new("import-dst");
    for (r, c) in &plan.writes {
        let bytes = match c {
            import::Content::Bytes(b) => b.clone(),
            import::Content::Copy(p) => std::fs::read(p).unwrap(),
        };
        project.put(r.as_str(), bytes);
    }
    project.put("mod.ltx", "[module]\nid = bunker\n");
    let m = resolve::model(&project.0, "models/cube.meshasset", &resolve::no_game()).unwrap();
    assert_eq!(m.object.surfaces[0].texture, "bunker\\brick");
    let index = index::scan(&project.0);
    assert!(index.by_name(MediaKind::Texture, "bunker\\stone").is_some());
    assert!(index.is_source("models/tri.bin") && index.is_source("models/cube.mtl"));
    // into a folder of its own
    let to = RelPath::new("art/in").unwrap();
    let plan = import::plan(&from[..1], Some(&to), "", &project.0);
    assert!(plan.writes.iter().all(|(r, _)| r.as_str().starts_with("art/in/")));
    assert!(texture_known(Path::new("Z:\\nowhere"), &Index::default(), &game(false), "x"));
}

#[test]
fn a_copyrighted_sound_names_a_free_replacement_and_the_game_list_reads_its_masks() {
    // the game's own list: names and masks, comments and other sections left out
    let rights = crate::sound::parse_rights(
        "; the game's\r\n[other]\r\nnot\\this\r\n[copyrighted]\r\nmusic\\radio\\*\r\nRadio/Edward_Meditation = radio\\free ; a replacement\r\n*_theme_*\r\n",
    );
    let masks: Vec<&str> = rights.iter().map(|r| r.mask.as_str()).collect();
    assert_eq!(masks, ["music\\radio\\*", "radio\\edward_meditation", "*_theme_*"]);
    assert_eq!(rights[1].instead, "radio\\free");
    assert_eq!(
        crate::sound::right_of(&rights, "radio\\edward_meditation").map(|r| r.instead.as_str()),
        Some("radio\\free")
    );
    let listed = |n: &str| crate::sound::listed(&rights, n);
    assert!(
        listed("music\\radio\\music_3") && listed("MUSIC/radio/x.ogg") && listed("radio\\edward_meditation")
    );
    assert!(
        listed("a\\guitar_theme_1") && !listed("music\\radiox") && !listed("radio\\edward") && !listed("")
    );
    assert!(crate::sound::mask_matches("a*b*c", "axbyc") && !crate::sound::mask_matches("a*b*c", "axcyb"));
    assert!(crate::sound::parse_rights("").is_empty());

    // the keys read and write back; left out, a sound is free
    let v = gz_luadata::parse("return { snd = 1, copyrighted = true, streamer = \"od\\\\calm\" }").unwrap();
    let (a, issues) = SoundAsset::from_value(&v);
    assert!(
        issues.is_empty() && a.is_copyrighted() && a.streamer.as_deref() == Some("od\\calm"),
        "{issues:?}"
    );
    let text = write_asset("", &a.to_table());
    assert!(text.contains("copyrighted = true") && text.contains("streamer = \"od\\\\calm\""), "{text}");
    assert!(!SoundAsset::from_value(&gz_luadata::parse("return { snd = 1 }").unwrap()).0.is_copyrighted());
    let bad = gz_luadata::parse("return { snd = 1, copyrighted = 1 }").unwrap();
    assert_eq!(codes(&SoundAsset::from_value(&bad).1), ["M109"]);

    // the check: a free replacement passes, a copyrighted one and one nobody has are errors, a
    // replacement on a free sound is a warning
    let d = Dir::new("streamer");
    d.put("mod.ltx", "[module]\nid = od\n");
    for n in ["song", "calm", "loud", "plain"] {
        d.put(&format!("s/{n}.wav"), wav());
    }
    d.put("s/song.sndasset", "return { snd = 1, copyrighted = true, streamer = \"od\\\\calm\" }\n");
    d.put("s/calm.sndasset", "return { snd = 1 }\n");
    d.put("s/loud.sndasset", "return { snd = 1, copyrighted = true, streamer = \"od\\\\song\" }\n");
    d.put("s/plain.sndasset", "return { snd = 1, streamer = \"od\\\\calm\" }\n");
    let index = index::scan(&d.0);
    let lone = GameLists { linked: true, ..GameLists::default() };
    let of = |rel: &str, index: &Index, game: &GameLists| {
        codes(&issues_of(&d, rel, MediaKind::Sound, index, game)).into_iter().collect::<Vec<_>>()
    };
    assert!(of("s/song.sndasset", &index, &game(false)).is_empty());
    assert_eq!(of("s/loud.sndasset", &index, &game(false)), ["M115"]);
    assert_eq!(of("s/plain.sndasset", &index, &game(false)), ["M114"]);
    d.put("s/song.sndasset", "return { snd = 1, copyrighted = true, streamer = \"ambient\\\\wind\" }\n");
    let index = index::scan(&d.0);
    // without a game nothing says the game lacks it; with one that has no such sound it is nobody's
    assert!(of("s/song.sndasset", &index, &game(false)).is_empty());
    assert_eq!(of("s/song.sndasset", &index, &lone), ["M116"]);
    // the picker's list: the module's free sounds, never a copyrighted one
    let free: Vec<String> =
        crate::sound::free_sounds(&d.0, &index, &lone).into_iter().map(|f| f.name).collect();
    assert_eq!(free, ["od\\calm", "od\\plain"]);
    // what a name is to the streamer mode, where the game reads it: the module's asset, the game's list
    let f = crate::sound::facts(&d.0, &index, &lone, "od\\song");
    assert_eq!(
        (f.mark, f.instead.as_str(), f.asset.as_deref()),
        (crate::sound::Mark::Copyrighted, "ambient\\wind", Some("s/song.sndasset"))
    );
    let f = crate::sound::facts(&d.0, &index, &lone, "od\\plain");
    assert_eq!((f.mark, f.instead.as_str()), (crate::sound::Mark::Free, ""));
    assert_eq!(crate::sound::facts(&d.0, &index, &lone, "nobody\\x").mark, crate::sound::Mark::Missing);
    assert_eq!(crate::sound::facts(&d.0, &index, &game(false), "nobody\\x").mark, crate::sound::Mark::Free);
}

#[test]
fn music_from_anywhere_becomes_a_sound_asset_under_the_modules_name() {
    let outside = Dir::new("music-out");
    outside.put("Rain Song.ogg", wav());
    let project = Dir::new("music-in");
    project.put("mod.ltx", "[module]\nid = od\n");
    project.put("tracks/inside.ogg", wav());
    let none = |_: &str| false;
    // from outside: copied into sounds\, its asset beside it, published under the module's id
    let a = import::sound_asset(&project.0, &outside.0.join("Rain Song.ogg"), "od", &none, None).unwrap();
    assert_eq!(
        (a.asset.as_str(), a.name.as_str(), a.written),
        ("sounds/Rain Song.sndasset", "od\\Rain Song", true)
    );
    assert!(project.0.join("sounds").join("Rain Song.ogg").is_file());
    let text = std::fs::read_to_string(project.0.join("sounds").join("Rain Song.sndasset")).unwrap();
    assert!(text.contains("snd = 1") && text.contains("source = \"Rain Song.ogg\""), "{text}");
    // the same file again is the same sound
    let again = import::sound_asset(&project.0, &outside.0.join("Rain Song.ogg"), "od", &none, None).unwrap();
    assert_eq!((again.asset.as_str(), again.written), ("sounds/Rain Song.sndasset", false));
    // a file of the project stays where it is, its asset beside it
    let inside =
        import::sound_asset(&project.0, &project.0.join("tracks").join("inside.ogg"), "od", &none, None)
            .unwrap();
    assert_eq!((inside.asset.as_str(), inside.name.as_str()), ("tracks/inside.sndasset", "od\\inside"));
    // a name another asset publishes is not taken: the asset gets a stem of its own
    let taken = |n: &str| n.eq_ignore_ascii_case("od\\inside");
    project.put("other/inside.ogg", wav());
    let other =
        import::sound_asset(&project.0, &project.0.join("other").join("inside.ogg"), "od", &taken, None)
            .unwrap();
    assert_eq!((other.asset.as_str(), other.name.as_str()), ("other/inside_2.sndasset", "od\\inside_2"));
    let text = std::fs::read_to_string(project.0.join("other").join("inside_2.sndasset")).unwrap();
    assert!(text.contains("source = \"inside.ogg\""), "{text}");
    // a sound asset given is that sound; another kind of file is refused by name
    let given = import::sound_asset(
        &project.0,
        &project.0.join("sounds").join("Rain Song.sndasset"),
        "od",
        &none,
        None,
    )
    .unwrap();
    assert_eq!((given.name.as_str(), given.written), ("od\\Rain Song", false));
    project.put("notes.txt", "x");
    assert!(
        import::sound_asset(&project.0, &project.0.join("notes.txt"), "od", &none, None)
            .unwrap_err()
            .contains("notes.txt")
    );
}

// ---- a model over a compiled model -------------------------------------------------------------

/// The bake of a door of two surfaces: a compiled model to stand for one of the game's.
fn compiled_door(d: &Dir) -> Vec<u8> {
    let door = "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  name = \"props\\\\door\",\n  surfaces = {\n    { name = \"frame\", texture = \"wood\\\\frame\" },\n    { name = \"leaf\", texture = \"wood\\\\leaf\" },\n  },\n  parts = {\n    { shape = \"box\", surface = \"leaf\" },\n    { shape = \"box\", pos = { 2, 0, 0 }, surface = \"frame\" },\n  },\n}\n";
    d.put("src/door.meshasset", door);
    let m = resolve::model(&d.0, "src/door.meshasset", &resolve::no_game()).unwrap();
    let bytes =
        gz_fmt_ogf::export_skeleton(&m.object, &gz_fmt_ogf::ExportOptions::default(), 4).unwrap().bytes;
    std::fs::remove_file(d.0.join("src\\door.meshasset")).unwrap();
    bytes
}

fn published(d: &Dir, game: &resolve::GameModel) -> Vec<(String, Vec<u8>)> {
    let index = index::scan(&d.0);
    let mut got: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sink = |rel: &str, p: Payload| -> Result<(), String> {
        let bytes = match p {
            Payload::Bytes(b) => b,
            Payload::File(f) => std::fs::read(f).map_err(|e| e.to_string())?,
        };
        got.push((rel.to_owned(), bytes));
        Ok(())
    };
    let done = publish(&d.0, &index, game, &mut sink);
    assert!(done.problems.is_empty(), "{:?}", done.problems);
    got
}

#[test]
fn a_model_over_a_model_of_the_game_says_only_what_changes() {
    let d = Dir::new("over-game");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let ogf = compiled_door(&d);
    let theirs = ogf.clone();
    let game: resolve::GameModel =
        Arc::new(move |name: &str| (name == "props\\door").then(|| theirs.clone()));
    // a static one: the game's geometry, a surface by the name of the texture its part wears
    d.put(
        "o/red.meshasset",
        "return { mesh = 1, source = \"game:props/door.ogf\", surfaces = { { name = \"leaf\", texture = \"mod\\\\red\" } } }\n",
    );
    let m = resolve::model(&d.0, "o/red.meshasset", &game).unwrap();
    let c = m.compiled.as_ref().unwrap();
    assert_eq!((c.game.as_deref(), c.bones.as_slice()), (Some("props\\door"), &["root".to_owned()][..]));
    assert!(m.source.is_none() && !m.object.is_dynamic() && m.object.bones.is_empty());
    let mut worn: Vec<(&str, &str)> =
        m.object.surfaces.iter().map(|s| (s.name.as_str(), s.texture.as_str())).collect();
    worn.sort_unstable();
    assert_eq!(worn, [("frame", "wood\\frame"), ("leaf", "mod\\red")]);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    // the game's file is part of what the model is
    let other: resolve::GameModel = {
        let mut changed = ogf.clone();
        changed.push(0);
        Arc::new(move |_: &str| Some(changed.clone()))
    };
    assert_ne!(resolve::model(&d.0, "o/red.meshasset", &other).map(|x| x.stamp).ok(), Some(m.stamp));
    // the index knows it has no file of the project for a source
    let e = index::entry(&d.0, &RelPath::new("o/red.meshasset").unwrap(), "bunker", false);
    assert_eq!((e.game.as_deref(), e.source.is_none(), e.ships()), (Some("props\\door"), true, false));
    // a model nobody has is said at the source
    let f = resolve::model(&d.0, "o/red.meshasset", &resolve::no_game()).unwrap_err();
    assert_eq!((f.issue.code, f.issue.path.clone()), (resolve::MISSING, vec![Seg::key("source")]));
    // and a name that climbs out of the models is no name
    d.put("o/out.meshasset", "return { mesh = 1, source = \"game:../../fsgame.ogf\" }\n");
    assert_eq!(resolve::model(&d.0, "o/out.meshasset", &game).unwrap_err().issue.code, resolve::MISSING);
}

#[test]
fn a_dynamic_model_over_a_compiled_one_is_published_as_that_file_said_again() {
    let d = Dir::new("over-dynamic");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let ogf = compiled_door(&d);
    let theirs = ogf.clone();
    let game: resolve::GameModel =
        Arc::new(move |name: &str| (name == "props\\door").then(|| theirs.clone()));
    d.put(
        "o/blue.meshasset",
        "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  name = \"props\\\\blue\",\n  source = \"game:props/door.ogf\",\n  userdata = \"[collide]\\nignore_static\\n\",\n  bones = { { name = \"root\", mass = 7, material = \"objects\\\\barrel\" } },\n  surfaces = { { name = \"frame\", texture = \"mod\\\\blue\" } },\n}\n",
    );
    // the same over a file of the project, found by the asset's own name
    d.put("o/own.ogf", &ogf);
    d.put("o/own.meshasset", "return { mesh = 1, kind = \"dynamic\", name = \"props\\\\own\" }\n");
    let got = published(&d, &game);
    let file =
        |name: &str| &got.iter().find(|(n, _)| n.ends_with(name)).unwrap_or_else(|| panic!("{name}")).1;
    let before = gz_fmt_ogf::Visual::read(&ogf).unwrap();
    let after = gz_fmt_ogf::Visual::read(file("blue.ogf")).unwrap();
    let textures = |v: &gz_fmt_ogf::Visual| -> Vec<String> {
        v.children
            .iter()
            .map(|c| c.texture.as_ref().map(|(t, _)| t.to_string()).unwrap_or_default())
            .collect()
    };
    let (was, now) = (textures(&before), textures(&after));
    assert_eq!(was.len(), 2);
    for (w, n) in was.iter().zip(&now) {
        assert_eq!(n, if w == "wood\\frame" { "mod\\blue" } else { w });
    }
    assert_eq!(after.user_data.as_deref(), Some("[collide]\nignore_static\n"));
    // the mass is said again in the file; the material stays the file's own and travels in the
    // description, where the game takes it when it is a dynamic one
    assert_eq!((after.ik[0].mass, &after.ik[0].game_mtl), (7.0, &before.ik[0].game_mtl));
    let described = std::str::from_utf8(file("blue.meshasset")).unwrap();
    assert!(described.contains("objects\\\\barrel"), "{described}");
    // the geometry and the skeleton are the file's own
    assert_eq!(after.bones, before.bones);
    for (a, b) in after.children.iter().zip(&before.children) {
        assert_eq!((&a.vertices, &a.indices), (&b.vertices, &b.indices));
    }
    // no motions of its own are published: the file names its own
    assert!(!got.iter().any(|(n, _)| n.ends_with("blue.omf")));
    // the text ships one row per part, in the parts' order
    let shipped = gz_luadata::parse(std::str::from_utf8(file("blue.meshasset")).unwrap()).unwrap();
    let rows: Vec<String> = shipped
        .get("surfaces")
        .unwrap()
        .arr()
        .iter()
        .map(|r| r.get("texture").and_then(gz_luadata::Value::as_str).unwrap_or_default().to_owned())
        .collect();
    assert_eq!(rows, now);
    // nothing asked of the project's own file: published byte for byte
    assert_eq!(file("own.ogf"), &ogf);
    // what a compiled model cannot take is said, and a bone it lacks is named
    d.put(
        "o/bad.meshasset",
        "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  source = \"own.ogf\",\n  scale = 2,\n  bones = { { name = \"arm\", mass = 1 }, { name = \"root\", shape = \"box\" } },\n  animations = { { name = \"open\" } },\n}\n",
    );
    let m = resolve::model(&d.0, "o/bad.meshasset", &game).unwrap();
    let said: Vec<&str> = m.issues.iter().map(|i| i.code).collect();
    // the scale, the bone the file lacks, the motion the file does not hold; a bone's shape is
    // the game's to apply and is no problem
    assert_eq!(said, [resolve::CANNOT; 3], "{:?}", m.issues);
    assert!(m.issues.iter().any(|i| i.message.contains("no bone 'arm'") && i.message.contains("root")));
    assert!(m.issues.iter().any(|i| i.message.contains("no motion 'open'")), "{:?}", m.issues);
    assert!(!m.issues.iter().any(|i| i.path.last() == Some(&Seg::key("shape"))), "{:?}", m.issues);
    // a bone's material the game has and no moving body is made of is said, with what happens
    let mut lists = GameLists { linked: true, ..GameLists::default() };
    lists.materials.extend(["default", "materials\\wood", "objects\\barrel"].map(str::to_owned));
    lists.dynamic_materials.insert("objects\\barrel".to_owned());
    d.put(
        "o/mtl.meshasset",
        "return {\n  mesh = 1,\n  kind = \"dynamic\",\n  source = \"own.ogf\",\n  bones = { { name = \"root\", material = \"materials\\\\wood\" } },\n}\n",
    );
    let index = index::scan(&d.0);
    let found = issues_of(&d, "o/mtl.meshasset", MediaKind::Model, &index, &lists);
    let mtl: Vec<&Issue> = found
        .iter()
        .filter(|i| i.code == crate::check::NO_MATERIAL && i.path.first() == Some(&Seg::key("bones")))
        .collect();
    assert_eq!(mtl.len(), 1, "{found:?}");
    assert!(mtl[0].message.contains("no dynamic game material"), "{}", mtl[0].message);
    // a source is its asset's wherever it lies - in the module's own game data too: the asset
    // says where it ships
    d.put("gamedata/meshes/bunker/crate.ogf", &ogf);
    d.put(
        "o/crate.meshasset",
        "return { mesh = 1, kind = \"dynamic\", name = \"bunker\\\\crate_red\", source = \"../gamedata/meshes/bunker/crate.ogf\" }\n",
    );
    let index = index::scan(&d.0);
    assert!(index.is_source("gamedata/meshes/bunker/crate.ogf"));
    assert!(index.is_source("o/own.ogf") && index.is_source("o/crate.meshasset"));
    // and the model of the game a `game:` asset is made over is the project's own file there
    d.put("o/over.meshasset", "return { mesh = 1, source = \"game:bunker/crate.ogf\" }\n");
    let index = index::scan(&d.0);
    let users: Vec<&str> =
        index.users_of("gamedata\\meshes\\bunker\\crate.ogf").iter().map(|e| e.path.as_str()).collect();
    assert!(users.contains(&"o/over.meshasset") && users.contains(&"o/crate.meshasset"), "{users:?}");
}

#[test]
fn a_surface_row_that_names_no_surface_is_said_with_the_names_there_are() {
    let d = Dir::new("no-surface");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("m/cube.obj", CUBE_OBJ);
    d.put("m/cube.mtl", CUBE_MTL);
    d.put(
        "m/cube.meshasset",
        "return { mesh = 1, surfaces = { { name = \"brick\", texture = \"a\\\\b\" }, { name = \"door\", texture = \"a\\\\c\" } } }\n",
    );
    let index = index::scan(&d.0);
    let found = issues_of(&d, "m/cube.meshasset", MediaKind::Model, &index, &GameLists::default());
    let row: Vec<&Issue> = found.iter().filter(|i| i.code == resolve::CANNOT).collect();
    assert_eq!(row.len(), 1, "{found:?}");
    assert_eq!(row[0].path, vec![Seg::key("surfaces"), Seg::item("name", "door")]);
    assert!(row[0].message.contains("it has: brick"), "{}", row[0].message);
}

/// A game texture of one 4 x 4 level, DXT1.
#[test]
fn the_raw_files_nothing_wraps_are_the_automatic_imports_and_go_with_what_they_belong_to() {
    let d = Dir::new("auto");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    for f in [
        "gamedata/textures/wpn/ak.dds",
        "gamedata/textures/wpn/ak.thm",
        "gamedata/textures/wpn/ak_bump.dds",
        "gamedata/textures/wpn/ak_bump#.dds",
        "gamedata/textures/wpn/lens.thm",
        "gamedata/textures/wpn/done.dds",
        "gamedata/textures/wpn/done.thm",
        "gamedata/sounds/bunker/hum.ogg",
        "gamedata/meshes/bunker/crate.ogf",
        "gamedata/scripts/bunker.script",
        "gamedata/levels/bunker/lmap#1_1.dds",
        "art/loose.png",
    ] {
        d.put(f, "x");
    }
    d.put("gamedata/textures/wpn/done.texasset", "return { tex = 1 }\n");
    let tree = gz_project::FileTree::scan(&d.0);
    let index = index::scan(&d.0);
    let mut got: Vec<String> =
        crate::auto::unwrapped(&tree, &index).iter().map(ToString::to_string).collect();
    got.sort_unstable();
    assert_eq!(
        got,
        [
            "gamedata/meshes/bunker/crate.ogf",
            "gamedata/sounds/bunker/hum.ogg",
            "gamedata/textures/wpn/ak.dds",
            "gamedata/textures/wpn/ak_bump.dds",
            "gamedata/textures/wpn/lens.thm",
        ]
    );
}

#[test]
fn an_export_writes_the_file_an_asset_was_made_over_with_what_goes_with_it() {
    use crate::export::{From, files_of, target};
    let d = Dir::new("export");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("m/cube.obj", CUBE_OBJ);
    d.put("m/cube.mtl", CUBE_MTL);
    d.put("m/cube.meshasset", "return { mesh = 1 }\n");
    d.put("gamedata/textures/wpn/ak.dds", dds());
    d.put("gamedata/textures/wpn/ak.thm", "x");
    d.put("gamedata/textures/wpn/ak.texasset", "return { tex = 1 }\n");
    d.put("t/lens.texasset", "return { tex = 1, source = \"game:wpn/lens\" }\n");
    d.put("o/box.meshasset", "return { mesh = 1, parts = { { shape = \"box\", size = { 1, 1, 1 } } } }\n");
    let index = index::scan(&d.0);
    let there = |r: &RelPath| r.to_fs(&d.0).is_file();
    let names = |rel: &str| -> Vec<(String, From)> {
        files_of(index.at(rel).unwrap(), there).unwrap().into_iter().map(|o| (o.name, o.from)).collect()
    };
    let at = |r: &str| From::Project(RelPath::new(r).unwrap());
    assert_eq!(
        names("m/cube.meshasset"),
        [("cube.obj".to_owned(), at("m/cube.obj")), ("cube.mtl".to_owned(), at("m/cube.mtl"))]
    );
    assert_eq!(
        names("gamedata/textures/wpn/ak.texasset"),
        [
            ("ak.dds".to_owned(), at("gamedata/textures/wpn/ak.dds")),
            ("ak.thm".to_owned(), at("gamedata/textures/wpn/ak.thm"))
        ]
    );
    assert_eq!(
        names("t/lens.texasset"),
        [("lens.dds".to_owned(), From::GameTexture("wpn\\lens".to_owned()))]
    );
    assert!(files_of(index.at("o/box.meshasset").unwrap(), there).unwrap_err().contains("parts"));
    // a name never climbs out of the folder it is written into
    let to = Path::new("D:\\out");
    assert_eq!(target(to, "tex\\a.png"), Some(PathBuf::from("D:\\out\\tex\\a.png")));
    assert_eq!(target(to, "..\\a.png"), None);
    assert_eq!(target(to, "C:\\a.png"), None);
}

#[test]
fn what_an_asset_takes_is_one_set_and_what_no_asset_can_say_ships_as_it_is() {
    let d = Dir::new("taken");
    d.put("mod.ltx", "[module]\nid = bunker\n[exclude]\nwip\n");
    // a texture asset with its .thm kept beside its picture, a model of the game over the
    // project's own file, an excluded asset over a picture
    d.put("gamedata/textures/wpn/ak.dds", dds());
    d.put("gamedata/textures/wpn/ak.thm", "x");
    d.put("gamedata/textures/wpn/ak.texasset", "return { tex = 1 }\n");
    d.put("gamedata/meshes/dyn/box.ogf", "x");
    d.put("o/box.meshasset", "return { mesh = 1, kind = \"dynamic\", source = \"game:dyn/box.ogf\" }\n");
    d.put("wip/old.png", png(2, 2, [1, 2, 3, 255]));
    d.put("wip/old.texasset", "return { tex = 1 }\n");
    let index = index::scan(&d.0);
    let taken = index.taken();
    for k in ["gamedata/textures/wpn/ak.dds", "gamedata/textures/wpn/ak.thm", "gamedata/meshes/dyn/box.ogf"] {
        assert!(taken.contains(k), "{k}: {taken:?}");
    }
    assert!(!taken.contains("wip/old.png"), "an excluded asset wraps nothing");
    // nothing the project holds is left to the import
    let tree = gz_project::FileTree::scan(&d.0);
    assert!(crate::auto::unwrapped(&tree, &index).is_empty());

    // what the import cannot say ships as it is; a picture the game cannot load does not
    d.put("gamedata/textures/wpn/odd.dds", dds());
    d.put("gamedata/textures/wpn/odd.thm", "x");
    d.put("gamedata/textures/wpn/bad.dds", "not a picture");
    d.put("gamedata/textures/wpn/note.png", png(2, 2, [1, 2, 3, 255]));
    d.put("gamedata/sounds/amb/wind.ogg", "x");
    let tree = gz_project::FileTree::scan(&d.0);
    let index = index::scan(&d.0);
    let mut kept = crate::auto::kept_as_files(&d.0, &tree, &index);
    kept.sort_unstable();
    assert_eq!(
        kept,
        ["gamedata/sounds/amb/wind.ogg", "gamedata/textures/wpn/odd.dds", "gamedata/textures/wpn/odd.thm"]
    );
}

#[test]
fn an_asset_made_in_place_from_its_thm_says_what_the_thm_says() {
    let d = Dir::new("thm-same");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let mut p = TextureParams {
        material: gz_fmt_tex::TextureMaterial(2),
        material_weight: 0.25,
        width: 4,
        height: 4,
        ..TextureParams::default()
    };
    p.detail_name = "detail\\detail_grnd_grass".into();
    p.detail_scale = 4.0;
    p.flags.insert(gz_fmt_tex::TextureFlags::DIFFUSE_DETAIL);
    d.put("gamedata/textures/wpn/ak.dds", dds());
    d.put(
        "gamedata/textures/wpn/ak.thm",
        gz_fmt_tex::write_texture_thm(&gz_fmt_tex::TextureThm { pixels: None, params: p }).unwrap(),
    );
    let plan = import::plan(&[d.0.join("gamedata\\textures\\wpn\\ak.dds")], None, "bunker", &d.0);
    assert_eq!(plan.assets, ["gamedata/textures/wpn/ak.texasset"], "{:?}", plan.problems);
    for (rel, content) in plan.writes {
        if let import::Content::Bytes(b) = content {
            d.put(rel.as_str(), b);
        }
    }
    let found = issues_of(
        &d,
        "gamedata/textures/wpn/ak.texasset",
        MediaKind::Texture,
        &index::scan(&d.0),
        &game(true),
    );
    assert!(!found.iter().any(|i| i.code == check::STALE_THM), "{found:?}");
}

/// The number the game reads for a lighting model's word.
fn gz_media_material(word: &str) -> u32 {
    crate::fields::MATERIALS.iter().find(|(w, _)| *w == word).map(|(_, n)| *n).unwrap()
}

fn dds() -> Vec<u8> {
    let img = gz_fmt_tex::Image::filled(4, 4, [40, 80, 120, 255]).unwrap();
    gz_fmt_tex::encode_dds(&[img], gz_fmt_tex::DdsEncoding::Dxt1).unwrap()
}

/// The game's data of a classic mod lying in the project: a texture with its description and a
/// bump pair, a description of a texture of the game alone, a sound with its parameters, a
/// compiled model. Imported in place, each gets its asset beside it saying what the file says,
/// and the build publishes every file as it is - nothing is made again.
#[test]
fn game_files_of_the_project_become_assets_in_place_and_ship_as_they_are() {
    let d = Dir::new("in-place");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    // a texture with a bump map of its own, and the bump pair
    let mut p = TextureParams {
        material: gz_fmt_tex::TextureMaterial(2),
        material_weight: 0.25,
        ..TextureParams::default()
    };
    p.bump_mode = gz_fmt_tex::BumpMode::USE;
    p.bump_name = "wpn\\ak_bump".into();
    p.width = 4;
    p.height = 4;
    let thm = |params: &TextureParams| {
        gz_fmt_tex::write_texture_thm(&gz_fmt_tex::TextureThm { pixels: None, params: params.clone() })
            .unwrap()
    };
    let main = dds();
    d.put("gamedata/textures/wpn/ak.dds", &main);
    d.put("gamedata/textures/wpn/ak.thm", thm(&p));
    let mut bump =
        TextureParams { ty: TextureType::BUMP_MAP, width: 4, height: 4, ..TextureParams::default() };
    bump.apply_type_defaults();
    d.put("gamedata/textures/wpn/ak_bump.dds", dds());
    d.put("gamedata/textures/wpn/ak_bump#.dds", b"DDS the error texture");
    d.put("gamedata/textures/wpn/ak_bump.thm", thm(&bump));
    // the description of a texture of the game, its picture the game's
    let mut lens = TextureParams { width: 64, height: 64, ..TextureParams::default() };
    lens.detail_name = "detail\\detail_glass".into();
    lens.flags.insert(gz_fmt_tex::TextureFlags::DIFFUSE_DETAIL);
    d.put("gamedata/textures/wpn/lens.thm", thm(&lens));
    // a sound with its parameters, a compiled model
    let params = gz_fmt_sound::OggParams {
        min_dist: 2.0,
        max_dist: 45.5,
        base_volume: 0.7,
        game_type: 0,
        max_ai_dist: 45.5,
    };
    let wave = gz_fmt_sound::read_wav(&wav()).unwrap();
    let ogg = gz_fmt_sound::encode_game_sound(&wave, 0.3, &params).unwrap();
    d.put("gamedata/sounds/weapons/ak_shot.ogg", &ogg);
    let ogf = compiled_door(&d);
    d.put("gamedata/meshes/dynamics/weapons/ak.ogf", &ogf);

    let files: Vec<PathBuf> = [
        "gamedata/textures/wpn/ak.dds",
        "gamedata/textures/wpn/ak_bump.dds",
        "gamedata/textures/wpn/lens.thm",
        "gamedata/sounds/weapons/ak_shot.ogg",
        "gamedata/meshes/dynamics/weapons/ak.ogf",
    ]
    .iter()
    .map(|f| d.0.join(f.replace('/', "\\")))
    .collect();
    let plan = import::plan(&files, None, "bunker", &d.0);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    // nothing is copied: every asset lies beside its file, the descriptions it took in go
    assert!(plan.copied.is_empty(), "{:?}", plan.copied);
    let removes: Vec<&str> = plan.removes.iter().map(RelPath::as_str).collect();
    assert_eq!(
        removes,
        [
            "gamedata/textures/wpn/ak.thm",
            "gamedata/textures/wpn/ak_bump.thm",
            "gamedata/textures/wpn/lens.thm"
        ]
    );
    for (rel, content) in &plan.writes {
        if let import::Content::Bytes(b) = content {
            d.put(rel.as_str(), b);
        }
    }
    for r in &plan.removes {
        std::fs::remove_file(r.to_fs(&d.0)).unwrap();
    }
    let text = |rel: &str| std::fs::read_to_string(d.0.join(rel.replace('/', "\\"))).unwrap();
    // the texture says its description, and what the game reads of it outright
    let ak = text("gamedata/textures/wpn/ak.texasset");
    assert!(!ak.contains("source =") && !ak.contains("name ="), "{ak}");
    assert!(
        ak.contains("material = \"phong\"") && ak.contains("weight = 0.25") && ak.contains("detail = false"),
        "{ak}"
    );
    let value = gz_luadata::parse(&ak).unwrap();
    let back = TextureAsset::from_value(&value).0.to_params("wpn\\ak", (4, 4), p.has_alpha());
    assert_eq!(back, p, "the asset says what the .thm said");
    let lens_text = text("gamedata/textures/wpn/lens.texasset");
    assert!(
        lens_text.contains("source = \"game:wpn/lens\"") && lens_text.contains("detail_glass"),
        "{lens_text}"
    );
    let shot = text("gamedata/sounds/weapons/ak_shot.sndasset");
    assert!(
        shot.contains("min_distance = 2")
            && shot.contains("max_distance = 45.5")
            && shot.contains("volume = 0.7"),
        "{shot}"
    );
    assert!(text("gamedata/meshes/dynamics/weapons/ak.meshasset").contains("kind = \"dynamic\""));
    // a second import changes nothing
    let again = import::plan(&files, None, "bunker", &d.0);
    assert!(again.writes.is_empty() && again.kept.len() == 4, "{:?} {:?}", again.writes, again.kept);

    // the names are the game's, and the build ships every file as it is
    let index = index::scan(&d.0);
    let name = |rel: &str| index.at(rel).map(|e| e.name.clone()).unwrap_or_default();
    assert_eq!(name("gamedata/textures/wpn/ak.texasset"), "wpn\\ak");
    assert_eq!(name("gamedata/textures/wpn/lens.texasset"), "wpn\\lens");
    assert_eq!(name("gamedata/sounds/weapons/ak_shot.sndasset"), "weapons\\ak_shot");
    assert_eq!(name("gamedata/meshes/dynamics/weapons/ak.meshasset"), "dynamics\\weapons\\ak");
    assert!(
        index.is_source("gamedata/textures/wpn/ak.dds")
            && index.is_source("gamedata/textures/wpn/ak_bump#.dds")
    );
    let mut paths: Vec<String> = Vec::new();
    let mut files_of: Vec<(String, PathBuf)> = Vec::new();
    let mut sink = |rel: &str, payload: Payload| -> Result<(), String> {
        paths.push(rel.to_owned());
        if let Payload::File(f) = payload {
            files_of.push((rel.to_owned(), f));
        }
        Ok(())
    };
    let done = publish(&d.0, &index, &resolve::no_game(), &mut sink);
    assert!(done.problems.is_empty(), "{:?}", done.problems);
    let at = |rel: &str| files_of.iter().find(|(r, _)| r == rel).map(|(_, f)| f.clone());
    let lies = |rel: &str| d.0.join(rel.replace('/', "\\"));
    // the very files of the project, nothing of the cache
    assert_eq!(at("gamedata\\textures\\wpn\\ak.dds"), Some(lies("gamedata/textures/wpn/ak.dds")));
    assert_eq!(at("gamedata\\textures\\wpn\\ak_bump#.dds"), Some(lies("gamedata/textures/wpn/ak_bump#.dds")));
    assert_eq!(
        at("gamedata\\sounds\\weapons\\ak_shot.ogg"),
        Some(lies("gamedata/sounds/weapons/ak_shot.ogg"))
    );
    assert_eq!(
        at("gamedata\\meshes\\dynamics\\weapons\\ak.ogf"),
        Some(lies("gamedata/meshes/dynamics/weapons/ak.ogf"))
    );
    // the game's texture: its description alone
    assert!(paths.contains(&"gamedata\\textures\\wpn\\lens.texasset".to_owned()));
    assert!(!paths.iter().any(|p| p.ends_with("lens.dds")));
    assert!(!d.0.join("temp").join("media").exists(), "nothing was baked");
    // a sound asset that says another volume is the same sound with its parameters said again
    d.put("gamedata/sounds/weapons/ak_shot.sndasset", shot.replace("volume = 0.7", "volume = 0.5"));
    let index = index::scan(&d.0);
    let got = published(&d, &resolve::no_game());
    let now = &got.iter().find(|(r, _)| r == "gamedata\\sounds\\weapons\\ak_shot.ogg").unwrap().1;
    assert_eq!(gz_fmt_sound::read_ogg_params(now).unwrap().map(|o| o.base_volume), Some(0.5));
    assert!(index.is_source("gamedata/sounds/weapons/ak_shot.ogg"));
}

/// A file of the module's game data an asset publishes under another name keeps shipping
/// where it lies; one the asset publishes at its own place is the asset's.
#[test]
fn a_file_has_one_asset_and_a_game_file_ships_where_its_asset_publishes_it() {
    let d = Dir::new("takes");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("gamedata/textures/wpn/ak.dds", dds());
    d.put("gamedata/textures/wpn/ak.texasset", "return { tex = 1 }\n");
    d.put(
        "art/red.texasset",
        "return { tex = 1, source = \"../gamedata/textures/wpn/ak.dds\", name = \"wpn\\\\ak_red\" }\n",
    );
    let index = index::scan(&d.0);
    let ak = index.at("gamedata/textures/wpn/ak.texasset").unwrap();
    let red = index.at("art/red.texasset").unwrap();
    // a source is its asset's: two assets over one file are an error on both
    assert!(ak.takes("gamedata/textures/wpn/ak.dds") && red.takes("gamedata/textures/wpn/ak.dds"));
    assert!(index.is_source("gamedata/textures/wpn/ak.dds"));
    for rel in ["gamedata/textures/wpn/ak.texasset", "art/red.texasset"] {
        let found = issues_of(&d, rel, MediaKind::Texture, &index, &game(false));
        assert!(
            found.iter().any(|i| i.code == check::WRAPPED_TWICE
                && i.severity == Severity::Error
                && i.path == vec![Seg::key("source")]),
            "{rel}: {found:?}"
        );
    }
    std::fs::remove_file(d.0.join("art\\red.texasset")).unwrap();
    let index = index::scan(&d.0);
    assert!(
        !issues_of(&d, "gamedata/textures/wpn/ak.texasset", MediaKind::Texture, &index, &game(false))
            .iter()
            .any(|i| i.code == check::WRAPPED_TWICE)
    );
    // a picture a model wears is the game's data where it lies: the model does not take it
    d.put("gamedata/meshes/wpn/crate.meshasset", "return { mesh = 1, source = \"crate.obj\" }\n");
    let crate_entry = Entry {
        uses: vec![RelPath::new("gamedata/textures/wpn/skin.dds").unwrap()],
        ..index::scan(&d.0).at("gamedata/meshes/wpn/crate.meshasset").unwrap().clone()
    };
    assert!(!crate_entry.takes("gamedata/textures/wpn/skin.dds"));
    std::fs::remove_file(d.0.join("gamedata\\meshes\\wpn\\crate.meshasset")).unwrap();
    // a .thm kept beside the picture that says to the game what the asset does not: a warning;
    // one that says the same, or nothing the game reads, is no news
    let thm = |params: &TextureParams| {
        gz_fmt_tex::write_texture_thm(&gz_fmt_tex::TextureThm { pixels: None, params: params.clone() })
            .unwrap()
    };
    let stale = |d: &Dir| {
        issues_of(d, "gamedata/textures/wpn/ak.texasset", MediaKind::Texture, &index::scan(&d.0), &game(true))
            .iter()
            .filter(|i| i.code == check::STALE_THM)
            .map(|i| i.severity)
            .collect::<Vec<_>>()
    };
    let mut lit = TextureParams {
        material: gz_fmt_tex::TextureMaterial(2),
        material_weight: 0.25,
        width: 4,
        height: 4,
        ..TextureParams::default()
    };
    d.put("gamedata/textures/wpn/ak.thm", thm(&lit));
    assert_eq!(stale(&d), [Severity::Warning]);
    d.put("gamedata/textures/wpn/ak.texasset", "return { tex = 1, material = \"metal\", weight = 0.25 }\n");
    lit.material = gz_fmt_tex::TextureMaterial(gz_media_material("metal"));
    d.put("gamedata/textures/wpn/ak.thm", thm(&lit));
    assert!(stale(&d).is_empty(), "the asset says what the .thm says");
    d.put("gamedata/textures/wpn/ak.texasset", "return { tex = 1 }\n");
    d.put(
        "gamedata/textures/wpn/ak.thm",
        thm(&TextureParams { width: 4, height: 4, ..TextureParams::default() }),
    );
    assert!(stale(&d).is_empty(), "a .thm that says nothing to the game");
    std::fs::remove_file(d.0.join("gamedata\\textures\\wpn\\ak.thm")).unwrap();
    // a texture asset there alone, named after the file, and a description of the game's texture
    // that names another texture is refused
    d.put(
        "gamedata/textures/wpn/odd.texasset",
        "return { tex = 1, source = \"game:wpn/lens\", name = \"wpn\\\\other\" }\n",
    );
    let found = issues_of(
        &d,
        "gamedata/textures/wpn/odd.texasset",
        MediaKind::Texture,
        &index::scan(&d.0),
        &game(false),
    );
    assert!(found.iter().any(|i| i.code == resolve::CANNOT && i.severity == Severity::Error), "{found:?}");
    // over a .dds the keys that say how a picture is made apply to nothing: a warning at each,
    // and the file still ships as it is
    d.put(
        "gamedata/textures/wpn/ak.texasset",
        "return { tex = 1, format = \"dxt5\", mips = false, normal_map = \"wpn/nobody_nmap\" }\n",
    );
    let found = issues_of(
        &d,
        "gamedata/textures/wpn/ak.texasset",
        MediaKind::Texture,
        &index::scan(&d.0),
        &game(true),
    );
    let said: Vec<_> = found.iter().map(|i| (i.code, i.severity, i.path.clone())).collect();
    assert_eq!(
        said,
        ["format", "mips", "normal_map"].map(|k| (resolve::CANNOT, Severity::Warning, vec![Seg::key(k)])),
        "{found:?}"
    );
    let got = published(&d, &resolve::no_game());
    let ak = &got.iter().find(|(r, _)| r == "gamedata\\textures\\wpn\\ak.dds").unwrap().1;
    assert_eq!(ak, &dds());
    // a description of a texture the game lacks describes a texture nobody has: a warning, and
    // it ships all the same
    d.put("gamedata/textures/wpn/odd.texasset", "return { tex = 1, source = \"game:wpn/lens\" }\n");
    let found = issues_of(
        &d,
        "gamedata/textures/wpn/odd.texasset",
        MediaKind::Texture,
        &index::scan(&d.0),
        &game(true),
    );
    assert_eq!(
        found.iter().map(|i| (i.code, i.severity)).collect::<Vec<_>>(),
        [(check::NO_TEXTURE, Severity::Warning)],
        "{found:?}"
    );
}

/// A file of the game data the game cannot load as a texture gets no asset: the import leaves
/// it a file of its own, which the build ships as it is.
#[test]
fn a_texture_file_the_game_cannot_load_stays_a_file_of_its_own() {
    let d = Dir::new("unloadable");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    d.put("gamedata/textures/item/can.dds", dds());
    // the game drops one `.dds` from a name it is given, and no name ends in a space
    d.put("gamedata/textures/item/can.dds.dds", dds());
    d.put("gamedata/textures/item/glass .dds", dds());
    // and a file that is no DDS at all
    d.put("gamedata/textures/item/broken.dds", b"DDS \x18\0\0\0 too short");
    let files: Vec<PathBuf> = ["can.dds", "can.dds.dds", "glass .dds", "broken.dds"]
        .iter()
        .map(|f| d.0.join("gamedata\\textures\\item").join(f))
        .collect();
    let plan = import::plan(&files, None, "bunker", &d.0);
    let written: Vec<&str> = plan.writes.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(written, ["gamedata/textures/item/can.texasset"]);
    assert_eq!(plan.problems.len(), 3, "{:?}", plan.problems);
    assert!(plan.problems.iter().all(|p| p.contains("stays a file of its own")), "{:?}", plan.problems);
    assert!(files::texture_name_reached("item\\can") && files::texture_name_reached("wpn\\scope.v2"));
    assert!(!files::texture_name_reached("item\\can.dds") && !files::texture_name_reached("item\\glass "));
}

/// A sound may be heard over a whole level: the game takes any distance.
#[test]
fn a_sound_may_be_heard_far() {
    let (asset, issues) = SoundAsset::from_value(
        &gz_luadata::parse("return { snd = 1, min_distance = 90000, max_distance = 90000 }").unwrap(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(asset.max_distance, Some(90_000.0));
}

/// The checksum of an Ogg page: CRC-32 over the page with its checksum field zero.
fn ogg_crc(page: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in page {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

/// An `.ogg` whose first comment is no parameters the game reads: the game's block made into an
/// encoder's tag, the page's checksum said again.
fn tagged_ogg() -> Vec<u8> {
    let params = gz_fmt_sound::OggParams {
        min_dist: 2.0,
        max_dist: 45.5,
        base_volume: 0.7,
        game_type: 0,
        max_ai_dist: 45.5,
    };
    let wave = gz_fmt_sound::read_wav(&wav()).unwrap();
    let mut ogg = gz_fmt_sound::encode_game_sound(&wave, 0.3, &params).unwrap();
    let block = params.to_comment();
    let at = ogg.windows(block.len()).position(|w| w == block.as_slice()).unwrap();
    ogg[at..at + 4].copy_from_slice(b"ENC=");
    let start = ogg[..at].windows(4).rposition(|w| w == b"OggS").unwrap();
    let segs = usize::from(ogg[start + 26]);
    let len = 27 + segs + ogg[start + 27..start + 27 + segs].iter().map(|&b| usize::from(b)).sum::<usize>();
    ogg[start + 22..start + 26].fill(0);
    let crc = ogg_crc(&ogg[start..start + len]);
    ogg[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
    ogg
}

/// A sound whose comment holds no parameters the game reads - an encoder's tag - is played with
/// the game's defaults: an asset that states nothing ships it as it is. A stream that does not
/// read at all is baked, and the build says why.
#[test]
fn a_sound_without_the_games_parameters_ships_as_it_is() {
    let d = Dir::new("tagged");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let tagged = tagged_ogg();
    assert!(gz_fmt_sound::read_ogg_params(&tagged).is_err() && gz_fmt_sound::ogg_info(&tagged).is_ok());
    d.put("gamedata/sounds/weapons/tagged.ogg", &tagged);
    d.put("gamedata/sounds/weapons/tagged.sndasset", "return { snd = 1 }\n");
    let got = published(&d, &resolve::no_game());
    let shipped = &got.iter().find(|(r, _)| r == "gamedata\\sounds\\weapons\\tagged.ogg").unwrap().1;
    assert_eq!(shipped, &tagged);
    // the importer reads no parameters from it: its asset states none
    std::fs::remove_file(d.0.join("gamedata\\sounds\\weapons\\tagged.sndasset")).unwrap();
    let plan = import::plan(&[d.0.join("gamedata\\sounds\\weapons\\tagged.ogg")], None, "bunker", &d.0);
    assert_eq!(plan.writes.len(), 1, "{:?}", plan.problems);
    let text = match &plan.writes[0].1 {
        import::Content::Bytes(b) => String::from_utf8(b.clone()).unwrap(),
        import::Content::Copy(_) => String::new(),
    };
    assert_eq!(
        text.trim(),
        "return {\n  snd = 1,\n}".trim(),
        "a source found by the asset's own name is not named"
    );
    // a broken stream
    d.put("gamedata/sounds/weapons/tagged.ogg", b"OggS but no stream");
    d.put("gamedata/sounds/weapons/tagged.sndasset", "return { snd = 1 }\n");
    let index = index::scan(&d.0);
    let mut sink = |_: &str, _: Payload| -> Result<(), String> { Ok(()) };
    let done = publish(&d.0, &index, &resolve::no_game(), &mut sink);
    assert!(!done.problems.is_empty(), "a stream that does not read is not shipped silently");
}

/// Files of the game's formats brought in from outside any tree of the game's data land where
/// they are put and take the name of that place: a picture and its description, a description
/// alone of the game's texture, a picture the game could not load copied in as a file.
#[test]
fn game_files_brought_into_the_game_data_take_the_name_of_where_they_land() {
    let d = Dir::new("brought");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let away = Dir::new("brought-from");
    let p = TextureParams {
        material: gz_fmt_tex::TextureMaterial(2),
        width: 4,
        height: 4,
        ..TextureParams::default()
    };
    let thm = gz_fmt_tex::write_texture_thm(&gz_fmt_tex::TextureThm { pixels: None, params: p }).unwrap();
    away.put("ak.dds", dds());
    away.put("ak.thm", &thm);
    away.put("lens.thm", &thm);
    away.put("twice.dds.dds", dds());
    let files: Vec<PathBuf> =
        ["ak.dds", "lens.thm", "twice.dds.dds"].iter().map(|f| away.0.join(f)).collect();
    let to = RelPath::new("gamedata/textures/wpn").unwrap();
    let plan = import::plan(&files, Some(&to), "bunker", &d.0);
    let written: Vec<&str> = plan.writes.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(
        written,
        [
            "gamedata/textures/wpn/ak.dds",
            "gamedata/textures/wpn/ak.texasset",
            "gamedata/textures/wpn/lens.texasset",
            "gamedata/textures/wpn/twice.dds.dds"
        ],
        "{:?}",
        plan.problems
    );
    let text = |rel: &str| match &plan.writes.iter().find(|(r, _)| r.as_str() == rel).unwrap().1 {
        import::Content::Bytes(b) => String::from_utf8(b.clone()).unwrap(),
        import::Content::Copy(f) => f.display().to_string(),
    };
    let ak = text("gamedata/textures/wpn/ak.texasset");
    assert!(ak.contains("material = \"phong\"") && !ak.contains("name ="), "{ak}");
    assert!(text("gamedata/textures/wpn/lens.texasset").contains("source = \"game:wpn/lens\""));
    assert!(plan.removes.is_empty(), "nothing outside the project is removed");
    assert_eq!(plan.problems.len(), 1, "{:?}", plan.problems);
    assert!(plan.problems[0].contains("copied in as a file of its own"), "{:?}", plan.problems);
}

/// Over a game texture published as it is, an asset made from its `.thm` says only what is read
/// of it: what the game reads, outright, and none of the keys that say how a picture is made. A
/// file of the project brought into another folder is copied there; one of the game's formats
/// in the module's game data outside its kind's folder - a level's lightmap - gets no asset.
#[test]
fn an_import_says_of_a_game_texture_only_what_is_read_of_it() {
    let d = Dir::new("read-of-it");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let mut p = TextureParams {
        fmt: gz_fmt_tex::TextureFormat::DXT5,
        width: 4,
        height: 4,
        ..TextureParams::default()
    };
    p.mip_filter = gz_fmt_tex::MipFilterId(1);
    p.flags.insert(gz_fmt_tex::TextureFlags::DITHER_COLOR);
    p.bump_mode = gz_fmt_tex::BumpMode::USE;
    p.bump_name = "wpn\\scope_bump".into();
    p.bump_virtual_height = 0.2;
    let thm = gz_fmt_tex::write_texture_thm(&gz_fmt_tex::TextureThm { pixels: None, params: p }).unwrap();
    d.put("gamedata/textures/wpn/scope.dds", dds());
    d.put("gamedata/textures/wpn/scope.thm", &thm);
    d.put("gamedata/levels/yard/lmap#1_1.dds", dds());
    d.put("art/crate.png", png(4, 4, [9, 9, 9, 255]));
    let files =
        [d.0.join("gamedata\\textures\\wpn\\scope.dds"), d.0.join("gamedata\\levels\\yard\\lmap#1_1.dds")];
    let plan = import::plan(&files, None, "bunker", &d.0);
    let written: Vec<&str> = plan.writes.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(written, ["gamedata/textures/wpn/scope.texasset"], "{:?}", plan.problems);
    assert!(plan.problems.len() == 1 && plan.problems[0].contains("where it lies"), "{:?}", plan.problems);
    let removes: Vec<&str> = plan.removes.iter().map(RelPath::as_str).collect();
    assert_eq!(removes, ["gamedata/textures/wpn/scope.thm"]);
    let text = match &plan.writes[0].1 {
        import::Content::Bytes(b) => String::from_utf8(b.clone()).unwrap(),
        import::Content::Copy(_) => String::new(),
    };
    for k in crate::texture::MAKING_KEYS {
        assert!(!text.contains(&format!("{k} =")), "{k} in {text}");
    }
    assert!(
        text.contains("bump = {") && text.contains("material = \"blinn\"") && text.contains("detail = false"),
        "{text}"
    );
    d.put("gamedata/textures/wpn/scope.texasset", &text);
    let found = issues_of(
        &d,
        "gamedata/textures/wpn/scope.texasset",
        MediaKind::Texture,
        &index::scan(&d.0),
        &game(false),
    );
    assert!(found.is_empty(), "{found:?}");
    // a file of the project brought into another folder is copied there, its own place untouched
    let to = RelPath::new("textures/props").unwrap();
    let plan = import::plan(&[d.0.join("art\\crate.png")], Some(&to), "bunker", &d.0);
    let written: Vec<&str> = plan.writes.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(written, ["textures/props/crate.png", "textures/props/crate.texasset"]);
    assert!(plan.removes.is_empty());
}

/// A sound whose parameters an asset cannot say the same way - a type bit the game's words do
/// not name - gets no asset: it stays a file of its own, shipped as it is.
#[test]
fn a_sound_an_asset_cannot_say_stays_a_file_of_its_own() {
    let d = Dir::new("unsaid");
    d.put("mod.ltx", "[module]\nid = bunker\n");
    let params = gz_fmt_sound::OggParams {
        min_dist: 2.0,
        max_dist: 45.5,
        base_volume: 0.7,
        game_type: 0x0000_0001,
        max_ai_dist: 45.5,
    };
    let wave = gz_fmt_sound::read_wav(&wav()).unwrap();
    d.put("gamedata/sounds/odd.ogg", gz_fmt_sound::encode_game_sound(&wave, 0.3, &params).unwrap());
    let plan = import::plan(&[d.0.join("gamedata\\sounds\\odd.ogg")], None, "bunker", &d.0);
    assert!(plan.writes.is_empty(), "{:?}", plan.writes);
    assert!(
        plan.problems.len() == 1 && plan.problems[0].contains("stays a file of its own"),
        "{:?}",
        plan.problems
    );
}

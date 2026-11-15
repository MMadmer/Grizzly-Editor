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
use crate::index::{self, Index};
use crate::model::{BOX, CYLINDER, PLANE, PRISM, Part, RAMP};
use crate::publish::{Payload, publish};
use crate::read::Issue;
use crate::service::GameLists;
use crate::sound::SoundAsset;
use crate::texture::TextureAsset;
use crate::{fields, import, kind, parts, resolve};

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
    check::issues(&dir.0, rel, kind, &v, index, game)
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
    let m = resolve::model(&d.0, "models/cube.meshasset").unwrap();
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
    assert_ne!(resolve::model(&d.0, "models/cube.meshasset").unwrap().stamp, first);

    // the scene's library takes it from the provider by its project path, any case
    let lib = gz_scene::library::Library::new(vec![d.0.join("rawdata").join("objects")]);
    lib.add_provider(crate::provider(d.0.clone()));
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
    let m = resolve::model(&d.0, "m/crate.meshasset").unwrap();
    assert!(m.object.flags.contains(ObjectFlags::DYNAMIC));
    assert_eq!(m.object.bones.len(), 1);
    let bone = &m.object.bones[0];
    assert_eq!((bone.name.as_str(), bone.mass, bone.game_mtl.as_str()), ("root", 25.0, "objects\\box"));
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
        let done = publish(&d.0, &index, &mut sink);
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
    let done = publish(&d.0, &index, &mut sink);
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
    let plan = import::plan(&from, None, "bunker");
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
    let m = resolve::model(&project.0, "models/cube.meshasset").unwrap();
    assert_eq!(m.object.surfaces[0].texture, "bunker\\brick");
    let index = index::scan(&project.0);
    assert!(index.by_name(MediaKind::Texture, "bunker\\stone").is_some());
    assert!(index.is_source("models/tri.bin") && index.is_source("models/cube.mtl"));
    // into a folder of its own
    let to = RelPath::new("art/in").unwrap();
    let plan = import::plan(&from[..1], Some(&to), "");
    assert!(plan.writes.iter().all(|(r, _)| r.as_str().starts_with("art/in/")));
    assert!(texture_known(Path::new("Z:\\nowhere"), &Index::default(), &game(false), "x"));
}

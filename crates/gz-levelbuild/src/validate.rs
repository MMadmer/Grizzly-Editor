//! What a scene must hold before it is built, and the messages that say what is missing.
//!
//! In this order: each kind's own check in class order (every scene object resolves and its surfaces have a texture and a shader, every sector has
//! its faces, compiler shaders and something to render, each stopping at the first object that
//! fails; the scene objects' duplicate positions per reference and, in a full test, the LOD
//! textures of the multiple-usage objects; the lights' controls and that there is a light; the
//! wallmarks' engine shaders), the portal test (no static mesh without
//! a sector, every sector sound) when the scene has sectors or portals, the HOM note, that there
//! are spawn points, lights and scene objects (and glows when asked), duplicate names, the
//! engine and compiler shaders of the static objects and whether they agree, the sound library
//! (sound environments, sound sources) and the particle
//! systems' references.
//!
//! A scene object in visual mode passes; a particle system is checked for a reference, not for
//! one the particle library knows; a LOD texture counts as there when the linked game or the
//! editor's own `gamedata` has it, and its message names the editor's `gamedata\textures` path.

use std::collections::BTreeSet;
use std::sync::Arc;

use gz_base::{Finding, Findings};
use gz_fmt_object::ObjectFlags;
use gz_fmt_object::xmath::{EPS_L, similar};
use gz_fmt_xr::ShaderXrlc;
use gz_scene::library::{Library, LibraryObject, normalize};
use gz_scene::objtools::sector;
use gz_scene::{ObjClass, ObjData, SceneState};

use crate::libs::Engine;
use crate::prj::lod_texture_name;

/// What a validation is asked to test: portals, the hierarchical occlusion test, glows, shader
/// compatibility and the full test.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tests {
    pub portal: bool,
    pub hom: bool,
    pub glow: bool,
    pub shaders: bool,
    pub full: bool,
}

impl Tests {
    /// What a level build tests (its portal test runs when the scene
    /// has sectors or portals, which `validate` looks at itself).
    pub const BUILD: Tests = Tests { portal: true, hom: true, glow: true, shaders: true, full: true };
    /// What making the game files tests.
    pub const MAKE_GAME: Tests =
        Tests { portal: false, hom: false, glow: false, shaders: false, full: false };
}

/// A texture of the game by its path under `textures\` (`lod\lod_props_box.dds`): the path a
/// message names and whether the file is there.
pub type TextureCheck<'a> = &'a (dyn Fn(&str) -> (String, bool) + Sync);

/// The blenders that bake into a light map: the default, its alpha
/// test, BmmD, LmEbB and LaEmB.
const LMAPPED: [&str; 5] = ["LM      ", "LM_AREF ", "BmmDold ", "LmEbB   ", "LaEmB   "];

pub(crate) fn lmapped(engine: &Engine, shader: &str) -> Option<bool> {
    let b = engine.shaders.as_ref()?.blender(shader)?;
    Some(LMAPPED.contains(&b.class_name().as_str()))
}

/// The extension after the last dot of the file name replaced.
fn change_ext(name: &str, ext: &str) -> String {
    let file_at = name.rfind(['\\', '/']).map_or(0, |i| i + 1);
    match name[file_at..].rfind('.') {
        Some(dot) => format!("{}{ext}", &name[..file_at + dot]),
        None => format!("{name}{ext}"),
    }
}

/// The LOD texture files a multiple-usage object needs, under `textures\`: the picture and its
/// normal map.
#[must_use]
pub fn lod_texture_files(lib_name: &str) -> [String; 2] {
    let name = lod_texture_name(lib_name);
    [change_ext(&name, ".dds"), change_ext(&format!("{name}_nm"), ".dds")]
}

/// Whether a resolved reference is multiple-usage static.
#[must_use]
pub fn is_mu_static(lo: &LibraryObject) -> bool {
    let f = lo.object.flags;
    !f.intersects(ObjectFlags::SOUND_OCCLUDER | ObjectFlags::DYNAMIC | ObjectFlags::HOM)
        && f.contains(ObjectFlags::MULTIPLE_USAGE)
}

/// The scene objects' check; `full` adds the LOD textures.
fn scene_objects(
    snap: &SceneState,
    look: &dyn Fn(&str) -> Option<Arc<LibraryObject>>,
    full: bool,
    texture: TextureCheck<'_>,
    log: &mut Vec<Finding>,
) -> bool {
    let objects: Vec<_> = snap.of_class(ObjClass::SceneObject).collect();
    // each object resolves and its surfaces are complete; the first failing one ends it
    let mut good = true;
    for o in &objects {
        let ObjData::SceneObject(d) = &o.data else { continue };
        if d.visual_mode() {
            continue;
        }
        let Some(lo) = look(&d.reference) else {
            good = false;
            break;
        };
        // every surface has a texture and a shader
        let mut surfaces_good = true;
        for s in &lo.object.surfaces {
            if s.texture.is_empty() || s.shader.is_empty() {
                log.warn(format!(
                    "Object '{}': the surface '{}' has no texture or no shader.",
                    lo.name, s.name
                ));
                surfaces_good = false;
            }
        }
        if !surfaces_good {
            good = false;
            break;
        }
    }
    // the duplicates of each object, then its LOD textures (both resolve to one
    // library object)
    let resolved: Vec<_> = objects
        .iter()
        .map(|o| match &o.data {
            ObjData::SceneObject(d) if !d.visual_mode() => look(&d.reference),
            _ => None,
        })
        .collect();
    for (ia, a) in objects.iter().enumerate() {
        let Some(la) = &resolved[ia] else { continue };
        for (ib, b) in objects.iter().enumerate() {
            if ia == ib {
                continue;
            }
            let same = resolved[ib].as_ref().is_some_and(|lb| normalize(&lb.name) == normalize(&la.name));
            if same && similar(a.position(), b.position(), EPS_L) {
                good = false;
                log.info(format!(
                    "Objects '{}' and '{}' of '{}' sit at the same position.",
                    a.name, b.name, la.name
                ));
            }
        }
        if full && is_mu_static(la) {
            let mut missing = false;
            for file in lod_texture_files(&la.name) {
                let (path, there) = texture(&file);
                if !there {
                    log.warn(format!("The texture '{path}' is missing."));
                    missing = true;
                }
            }
            if missing {
                good = false;
            }
        }
    }
    good
}

/// The wallmarks' check: the engine shader of every slot with marks.
fn wallmarks(snap: &SceneState, engine: &Engine, log: &mut Vec<Finding>) -> bool {
    let slots = snap.bulk.wallmarks.as_ref().map(|w| crate::wallmarks::read(&w.raw)).unwrap_or_default();
    let mut good = true;
    for s in &slots {
        if s.marks.is_empty() {
            continue;
        }
        if !engine.has_shader(&s.shader) || lmapped(engine, &s.shader) == Some(true) {
            log.info(format!("The wallmark shader '{}' is missing or cannot be used.", s.shader));
            good = false;
        }
    }
    good
}

/// The sounds' check: the first sound environment with the same inner and outer
/// environment, or the first sound source without a wave, fails it.
fn sounds(snap: &SceneState, log: &mut Vec<Finding>) -> bool {
    for o in snap.of_class(ObjClass::SoundEnv) {
        if let ObjData::SoundEnv(e) = &o.data
            && e.inner == e.outer
        {
            log.info(format!("Sound environment '{}' has the same inner and outer environment.", o.name));
            return false;
        }
    }
    for o in snap.of_class(ObjClass::SoundSrc) {
        if let ObjData::SoundSrc(s) = &o.data
            && s.wav.is_empty()
        {
            log.info(format!("Sound source '{}' has no wave.", o.name));
            return false;
        }
    }
    true
}

/// The scene's validation: `Ok` with its findings, `Err` with them when it failed.
pub fn validate(
    snap: &SceneState,
    lib: &Library,
    engine: &Engine,
    tests: Tests,
    texture: TextureCheck<'_>,
) -> Result<Vec<Finding>, Vec<Finding>> {
    let look = |r: &str| lib.cached(r);
    let mut log = Vec::new();
    let mut ok = true;
    let shaders = engine.xrlc.as_ref();

    // the tools in id order, each its own check
    for class in ObjClass::ALL {
        let good = match class {
            ObjClass::SceneObject => scene_objects(snap, &look, tests.full, texture, &mut log),
            ObjClass::Light => {
                let controls = snap.tools.part(class).light();
                let mut good = snap.count(class) > 0;
                for o in snap.of_class(class) {
                    let ObjData::Light(l) = &o.data else { continue };
                    if controls.and_then(|c| c.name_of(l.lcontrol)).is_none() {
                        good = false;
                        log.info(format!("{} '{}' has an invalid light control.", class.desc(), o.name));
                    }
                }
                good
            }
            // each sector's own check, the first failing one ends it
            ObjClass::Sector => {
                let mut good = true;
                for s in snap.of_class(class) {
                    let ObjData::Sector(d) = &s.data else { continue };
                    let problems = sector::validate_sector(&s.name, &sector::items(snap, lib, d), shaders);
                    if !problems.is_empty() {
                        log.extend(problems);
                        good = false;
                        break;
                    }
                }
                good
            }
            ObjClass::Wallmark => wallmarks(snap, engine, &mut log),
            _ => true,
        };
        if !good {
            log.error(format!("The {} check failed.", class.desc()));
            ok = false;
        }
    }

    if tests.portal && (snap.count(ObjClass::Sector) > 0 || snap.count(ObjClass::Portal) > 0) {
        let (portals_good, msgs) = sector::validate(snap, lib, shaders);
        log.extend(msgs);
        if !portals_good {
            ok = false;
        }
    }
    if tests.hom {
        let has_hom = snap.of_class(ObjClass::SceneObject).any(|o| {
            let ObjData::SceneObject(d) = &o.data else { return false };
            look(&d.reference).is_some_and(|lo| lo.object.flags.contains(ObjectFlags::HOM))
        });
        if !has_hom {
            log.warn("The level has no HOM objects.");
        }
    }
    if snap.count(ObjClass::SpawnPoint) == 0 {
        log.error("The level has no spawn point.");
        ok = false;
    }
    if snap.count(ObjClass::Light) == 0 {
        log.error("The level has no light.");
        ok = false;
    }
    if snap.count(ObjClass::SceneObject) == 0 {
        log.error("The level has no scene object.");
        ok = false;
    }
    if tests.glow && snap.count(ObjClass::Glow) == 0 {
        log.error("The level has no glow.");
        ok = false;
    }
    // the first name another object also has
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let duplicate = snap.objects.values().find(|o| !seen.insert(o.name.to_ascii_lowercase()));
    if let Some(o) = duplicate {
        log.info(format!("The object name '{}' is used twice.", o.name));
        log.error("Two objects have the same name.");
        ok = false;
    }

    if tests.shaders {
        let mut good = true;
        let mut statics = 0usize;
        let mut checked: BTreeSet<String> = BTreeSet::new();
        for o in snap.of_class(ObjClass::SceneObject) {
            let ObjData::SceneObject(d) = &o.data else { continue };
            let Some(lo) = look(&d.reference) else { continue };
            let f = lo.object.flags;
            let movable = f.intersects(ObjectFlags::SOUND_OCCLUDER | ObjectFlags::DYNAMIC | ObjectFlags::HOM);
            if movable {
                continue;
            }
            statics += 1;
            if !checked.insert(d.reference.to_ascii_lowercase()) {
                continue;
            }
            // the engine and compiler shaders of each surface must agree
            for s in &lo.object.surfaces {
                let xrlc = engine.xrlc_flags(&s.shader_xrlc);
                match (engine.has_shader(&s.shader), xrlc) {
                    (true, Some(flags)) => {
                        let vertex_lit = flags & ShaderXrlc::LIGHT_VERTEX != 0;
                        if lmapped(engine, &s.shader).is_some_and(|lm| lm == vertex_lit) {
                            log.info(format!(
                                "Object '{}': the engine shader '{}' does not fit the compiler shader '{}'.",
                                lo.name, s.shader, s.shader_xrlc
                            ));
                            good = false;
                        }
                    }
                    _ => {
                        log.info(format!(
                            "Object '{}': the engine shader '{}' or the compiler shader '{}' is invalid or missing.",
                            lo.name, s.shader, s.shader_xrlc
                        ));
                        good = false;
                    }
                }
            }
        }
        if !good {
            log.error("The scene has shaders that do not fit each other (see the lines above).");
            ok = false;
        }
        if statics == 0 {
            log.error("The level has no static geometry.");
            ok = false;
        }
    }

    if !sounds(snap, &mut log) {
        ok = false;
    }

    for o in snap.of_class(ObjClass::Particles) {
        let ObjData::Particles(p) = &o.data else { continue };
        if p.reference.is_empty() {
            log.error("A particle system has no reference.");
            ok = false;
        }
    }

    if ok {
        Ok(log)
    } else {
        log.info("The scene failed its checks.");
        Err(log)
    }
}

#[cfg(test)]
mod tests {
    use gz_base::Severity;
    use gz_chunk::Writer;
    use gz_scene::Object;
    use gz_scene::objects::{SectorData, SoundEnvData, SoundSrcData};
    use gz_scene::tools::WallmarkPart;

    use super::*;

    fn run(snap: &SceneState, tests: Tests) -> Result<Vec<Finding>, Vec<Finding>> {
        let lib = Library::new(Vec::new());
        let check = |n: &str| (format!("gamedata\\textures\\{n}"), false);
        validate(snap, &lib, &Engine::default(), tests, &check)
    }

    fn found(r: &Result<Vec<Finding>, Vec<Finding>>) -> &[Finding] {
        match r {
            Ok(l) | Err(l) => l,
        }
    }

    fn says(r: &Result<Vec<Finding>, Vec<Finding>>, severity: Severity, text: &str) -> bool {
        found(r).iter().any(|f| f.severity == severity && f.text == text)
    }

    #[test]
    fn a_sound_environment_with_one_environment_twice_fails() {
        let mut s = SceneState::default();
        let env = SoundEnvData { inner: "cave".into(), outer: "cave".into(), ..SoundEnvData::default() };
        s.insert(Object::new(ObjClass::SoundEnv, "env_1", ObjData::SoundEnv(env)));
        let r = run(&s, Tests::MAKE_GAME);
        assert!(r.is_err());
        assert!(says(
            &r,
            Severity::Note,
            "Sound environment 'env_1' has the same inner and outer environment."
        ));
    }

    #[test]
    fn a_sound_source_without_a_wave_fails() {
        let mut s = SceneState::default();
        s.insert(Object::new(ObjClass::SoundSrc, "snd_1", ObjData::SoundSrc(SoundSrcData::default())));
        let r = run(&s, Tests::MAKE_GAME);
        assert!(r.is_err());
        assert!(says(&r, Severity::Note, "Sound source 'snd_1' has no wave."));
    }

    #[test]
    fn an_empty_sector_fails_its_tool_and_the_portal_test() {
        let mut s = SceneState::default();
        s.insert(Object::new(ObjClass::Sector, "sector_1", ObjData::Sector(SectorData::default())));
        let r = run(&s, Tests::BUILD);
        assert!(r.is_err());
        assert!(says(&r, Severity::Error, "Sector 'sector_1' has too few faces."));
        assert!(says(&r, Severity::Error, "The Sector check failed."));
        // the portal test cannot box a scene without scene objects
        assert!(found(&r).iter().any(|x| x.text.contains("bounding box")));
        // making the game files runs no portal test
        let r = run(&s, Tests::MAKE_GAME);
        assert!(!found(&r).iter().any(|x| x.text.contains("bounding box")));
    }

    #[test]
    fn lod_texture_names() {
        assert_eq!(
            lod_texture_files("props\\box"),
            ["lod\\lod_props_box.dds".to_owned(), "lod\\lod_props_box_nm.dds".to_owned()]
        );
        assert_eq!(change_ext("a.b\\c", ".dds"), "a.b\\c.dds");
        assert_eq!(change_ext("a\\c.tga", ".dds"), "a\\c.dds");
    }

    #[test]
    fn a_wallmark_slot_without_its_shader_fails() {
        // one slot with one mark of no triangles, and no engine shader library at all
        let mut items = Writer::new();
        items.chunk(0, |w| {
            w.w_u32(1);
            w.w_stringz("effects\\wallmark");
            w.w_stringz("wm\\a");
            w.w_u8(0);
            for _ in 0..6 {
                w.w_f32(0.0);
            }
            w.w_vec3(glam::Vec3::ZERO);
            w.w_f32(1.0);
            w.w_u32(0);
        });
        let mut part = Writer::new();
        part.chunk(1, |w| w.w_u16(3));
        // the flags and the tool's parameters are required chunks of the wallmarks' stream
        part.chunk(2, |w| w.w_u32(0));
        part.chunk(3, |w| {
            for _ in 0..3 {
                w.w_f32(1.0);
            }
            w.w_stringz("effects\\wallmarkmult");
            w.w_stringz("wm\\a");
        });
        part.w_chunk(4, &items.into_bytes());
        let mut s = SceneState::default();
        s.bulk.wallmarks = Some(Arc::new(WallmarkPart::read(&part.into_bytes()).unwrap()));
        let r = run(&s, Tests::MAKE_GAME);
        assert!(says(
            &r,
            Severity::Note,
            "The wallmark shader 'effects\\wallmark' is missing or cannot be used."
        ));
        let failed = format!("The {} check failed.", ObjClass::Wallmark.desc());
        assert!(says(&r, Severity::Error, &failed), "{:?}", found(&r));
    }

    #[test]
    fn no_text_starts_with_a_mark_because_the_level_is_a_value() {
        // a scene with one empty sector fails nearly every check there is
        let mut s = SceneState::default();
        s.insert(Object::new(ObjClass::Sector, "sector_1", ObjData::Sector(SectorData::default())));
        let r = run(&s, Tests::BUILD);
        assert!(found(&r).len() > 4);
        for f in found(&r) {
            assert!(!f.text.starts_with(['!', '~', '*']) && !f.text.starts_with("ERROR"), "{f:?}");
        }
    }
}

//! `EScene::Validate`: what a scene must hold before it is built, with the C++ messages.
//!
//! What is checked here: each tool's own check (`ESceneCustomOTool::Validate` - every scene
//! object's library reference resolves; the scene objects' duplicate positions with one
//! reference; the lights' controls, and that there is a light at all), the HOM note, that there
//! are spawn points, lights and scene objects (and glows when asked), duplicate names, the
//! engine and compiler shaders of the static objects and whether they agree, and the particle
//! systems' references.
//!
//! Not ported: the portal test (`CPortalUtils::Validate` needs the sectors' captured meshes),
//! the sectors' and wallmarks' shader checks, the MU objects' LOD texture notes of the full test
//! and the sound library's check.

use std::collections::BTreeSet;
use std::sync::Arc;

use gz_fmt_object::ObjectFlags;
use gz_fmt_xr::ShaderXrlc;
use gz_scene::library::LibraryObject;
use gz_scene::{ObjClass, ObjData, SceneState};

use crate::libs::Engine;

/// What a validation is asked to test (`Validate(bNeedOkMsg, bTestPortal, bTestHOM, bTestGlow,
/// bTestShaderCompatible, bFullTest)`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Tests {
    pub hom: bool,
    pub glow: bool,
    pub shaders: bool,
    pub full: bool,
}

impl Tests {
    /// What a level build tests (`SceneBuilder::Compile`).
    pub const BUILD: Tests = Tests { hom: true, glow: true, shaders: true, full: true };
    /// What making the game files tests (`SceneBuilder::MakeGame`).
    pub const MAKE_GAME: Tests = Tests { hom: false, glow: false, shaders: false, full: false };
}

/// The blenders that bake into a light map (`IBlender::canBeLMAPped`): the default, its alpha
/// test, BmmD, LmEbB and LaEmB.
const LMAPPED: [&str; 5] = ["LM      ", "LM_AREF ", "BmmDold ", "LmEbB   ", "LaEmB   "];

fn lmapped(engine: &Engine, shader: &str) -> Option<bool> {
    let b = engine.shaders.as_ref()?.blender(shader)?;
    Some(LMAPPED.contains(&b.class_name().as_str()))
}

/// The scene's validation: `Ok` with the log lines, `Err` with them when it failed.
pub fn validate(
    snap: &SceneState,
    look: &(dyn Fn(&str) -> Option<Arc<LibraryObject>> + Sync),
    engine: &Engine,
    tests: Tests,
) -> Result<Vec<String>, Vec<String>> {
    let mut log = Vec::new();
    let mut ok = true;
    let tool_failed = |log: &mut Vec<String>, class: ObjClass| {
        log.push(format!("ERROR: Validate '{}' failed!", class.desc()))
    };

    // the tools in id order, each its own check
    for class in ObjClass::ALL {
        match class {
            ObjClass::SceneObject => {
                let objects: Vec<_> = snap.of_class(class).collect();
                let mut good = true;
                // every object's reference loads (`CSceneObject::Validate`); the first that does
                // not ends the tool's check
                for o in &objects {
                    let ObjData::SceneObject(d) = &o.data else { continue };
                    if !d.visual_mode() && look(&d.reference).is_none() {
                        good = false;
                        break;
                    }
                }
                for a in &objects {
                    let ObjData::SceneObject(da) = &a.data else { continue };
                    for b in &objects {
                        let ObjData::SceneObject(db) = &b.data else { continue };
                        if a.id == b.id || !da.reference.eq_ignore_ascii_case(&db.reference) {
                            continue;
                        }
                        if a.position().abs_diff_eq(b.position(), 0.0001) {
                            good = false;
                            log.push(format!(
                                "Duplicate object position '{}'-'{}' with reference '{}'.",
                                a.name, b.name, da.reference
                            ));
                        }
                    }
                }
                if !good {
                    tool_failed(&mut log, class);
                    ok = false;
                }
            }
            ObjClass::Light => {
                let controls = snap.tools.part(class).light();
                let mut good = snap.count(class) > 0;
                for o in snap.of_class(class) {
                    let ObjData::Light(l) = &o.data else { continue };
                    if controls.and_then(|c| c.name_of(l.lcontrol)).is_none() {
                        good = false;
                        log.push(format!("{}: '{}' - Invalid light control.", class.desc(), o.name));
                    }
                }
                if !good {
                    tool_failed(&mut log, class);
                    ok = false;
                }
            }
            _ => {}
        }
    }

    if tests.hom {
        let has_hom = snap.of_class(ObjClass::SceneObject).any(|o| {
            let ObjData::SceneObject(d) = &o.data else { return false };
            look(&d.reference).is_some_and(|lo| lo.object.flags.contains(ObjectFlags::HOM))
        });
        if !has_hom {
            log.push("!Level doesn't contain HOM objects!".to_owned());
        }
    }
    if snap.count(ObjClass::SpawnPoint) == 0 {
        log.push("*ERROR: Can't find any Spawn Object.".to_owned());
        ok = false;
    }
    if snap.count(ObjClass::Light) == 0 {
        log.push("*ERROR: Can't find any Light Object.".to_owned());
        ok = false;
    }
    if snap.count(ObjClass::SceneObject) == 0 {
        log.push("*ERROR: Can't find any Scene Object.".to_owned());
        ok = false;
    }
    if tests.glow && snap.count(ObjClass::Glow) == 0 {
        log.push("*ERROR: Can't find any Glow Object.".to_owned());
        ok = false;
    }
    // `FindDuplicateName`: the first name another object also has
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let duplicate = snap.objects.values().find(|o| !seen.insert(o.name.to_ascii_lowercase()));
    if let Some(o) = duplicate {
        log.push(format!("Duplicate object name already exists: '{}'", o.name));
        log.push("*ERROR: Found duplicate object name.".to_owned());
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
            // `CEditableObject::CheckShaderCompatible`
            for s in &lo.object.surfaces {
                let xrlc = engine.xrlc_flags(&s.shader_xrlc);
                match (engine.has_shader(&s.shader), xrlc) {
                    (true, Some(flags)) => {
                        let vertex_lit = flags & ShaderXrlc::LIGHT_VERTEX != 0;
                        if lmapped(engine, &s.shader).is_some_and(|lm| lm == vertex_lit) {
                            log.push(format!(
                                "Object '{}': engine shader '{}' non compatible with compiler shader '{}'",
                                lo.name, s.shader, s.shader_xrlc
                            ));
                            good = false;
                        }
                    }
                    _ => {
                        log.push(format!(
                            "Object '{}': invalid or missing shader [E:'{}', C:'{}']",
                            lo.name, s.shader, s.shader_xrlc
                        ));
                        good = false;
                    }
                }
            }
        }
        if !good {
            log.push("*ERROR: Scene has non compatible shaders. See log.".to_owned());
            ok = false;
        }
        if statics == 0 {
            log.push("*ERROR: Can't find static geometry.".to_owned());
            ok = false;
        }
    }

    for o in snap.of_class(ObjClass::Particles) {
        let ObjData::Particles(p) = &o.data else { continue };
        if p.reference.is_empty() {
            log.push("*ERROR: Particle System hasn't reference.".to_owned());
            ok = false;
        }
    }

    if ok {
        Ok(log)
    } else {
        log.push("Validation FAILED!".to_owned());
        Err(log)
    }
}

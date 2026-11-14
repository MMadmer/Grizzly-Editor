//! The base scenes of the repository's `rawdata\levels`, X-Ray SDK `.level` files with their
//! parts: every one imports, is written as a `.scene` with its folder, and reads back equal -
//! the same objects with the same data, every spawn packet byte for byte, the painted layers
//! bit for bit, the level's options and the kinds' settings - and written again from what it
//! read, nothing changes. The linked game's configs decode the spawn points into their named
//! properties, so the test needs the game (`GRIZZLY_TEST_GAME`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gz_base::RelPath;
use gz_base::text::cp1251_to_utf8;
use gz_corpus::{Game, check_all, finish, key};
use gz_ltx::{Dialect, Loader, Ltx, ParsedFile, parse_file};
use gz_scene::format::{self, Plan, Texts, layers};
use gz_scene::objects::SpawnEntity;
use gz_scene::{ObjData, Object, SceneState, sdk};
use rayon::prelude::*;

struct Configs(BTreeMap<String, ParsedFile>);

impl Loader for Configs {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        self.0.get(&key(path))
    }

    fn list(&self, dir: &str, mask: &str) -> Vec<String> {
        let mut prefix = key(dir);
        prefix.push('/');
        let tail = mask.trim_start_matches('*').to_lowercase();
        self.0
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rel| rel.ends_with(&tail))
            .map(|rel| rel.replace('/', "\\"))
            .collect()
    }
}

/// The game's `system.ltx` with everything it includes.
fn system_ltx(game: &Game) -> Ltx {
    let files = game
        .files()
        .par_iter()
        .filter(|e| e.key.starts_with("configs/") && e.key.ends_with(".ltx"))
        .filter_map(|e| {
            Some((e.key.clone(), parse_file(&cp1251_to_utf8(&game.read(e).ok()?), Dialect::Game)))
        })
        .collect();
    Ltx::assemble("configs\\system.ltx", &Configs(files), Dialect::Game).map(|l| l.ltx).unwrap_or_default()
}

fn scenes(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "level")).collect();
    out.sort();
    out
}

/// The files a plan writes, as the disk then holds them.
fn texts_of(plan: &Plan) -> Texts {
    let mut t = Texts::default();
    for o in &plan.files {
        let Some(b) = &o.bytes else { continue };
        if o.file.is_empty() {
            t.scene = String::from_utf8(b.clone()).unwrap_or_default();
        } else if o.file.ends_with(".lua") {
            t.files.push((o.file.clone(), String::from_utf8(b.clone()).unwrap_or_default()));
        } else {
            t.layers.push((o.file.clone(), b.clone()));
        }
    }
    t
}

/// A spawn point's packet, as the game would be given it, and for an entity of an X-Ray SDK
/// text scene that has none (the game does not know its class) the section's numbered values.
fn packet(e: &SpawnEntity) -> Option<(Vec<u8>, Vec<String>)> {
    match e {
        SpawnEntity::Decoded(x) => {
            let p = x.to_spawn_packet();
            let values = if p.is_empty() {
                x.to_scene().map(|v| v.iter().map(|i| i.to_ltx()).collect()).unwrap_or_default()
            } else {
                Vec::new()
            };
            Some((p, values))
        }
        SpawnEntity::RawPacket { packet, .. } => Some((packet.to_vec(), Vec::new())),
        SpawnEntity::Kept { raw, .. } => raw.as_ref().map(|r| (r.to_vec(), Vec::new())),
        SpawnEntity::RawLtx { values, .. } => Some((Vec::new(), values.clone())),
    }
}

/// What differs between an object and the one read back.
fn differs(a: &Object, b: &Object) -> Option<String> {
    if a.class != b.class {
        return Some("its kind".into());
    }
    if a.xform != b.xform {
        return Some(format!("its transform ({:?} read back as {:?})", a.xform, b.xform));
    }
    if a.flags != b.flags {
        return Some(format!("its flags ({:?} read back as {:?})", a.flags, b.flags));
    }
    if a.marks != b.marks {
        return Some("its mod marks".into());
    }
    if a.motion.as_deref().map(|m| &m.motion) != b.motion.as_deref().map(|m| &m.motion) {
        return Some("its motion".into());
    }
    match (&a.data, &b.data) {
        (ObjData::Spawn(x), ObjData::Spawn(y)) => {
            let (pa, pb) = (x.entity.as_ref().and_then(packet), y.entity.as_ref().and_then(packet));
            if pa != pb {
                let at = pa
                    .as_ref()
                    .zip(pb.as_ref())
                    .and_then(|((p, _), (q, _))| p.iter().zip(q).position(|(u, v)| u != v));
                let (va, vb) = (pa.map(|p| p.1).unwrap_or_default(), pb.map(|p| p.1).unwrap_or_default());
                return Some(if va == vb {
                    format!("its spawn packet (from byte {at:?})")
                } else {
                    format!("its kept values ({} read back as {})", va.len(), vb.len())
                });
            }
            let (mut x2, mut y2) = ((**x).clone(), (**y).clone());
            x2.entity = None;
            y2.entity = None;
            (x2 != y2).then(|| "its spawn point's data".to_owned())
        }
        (ObjData::Group(x), ObjData::Group(y)) => {
            (x.reference != y.reference || x.members.len() != y.members.len()).then(|| "its group".to_owned())
        }
        (x, y) => {
            (x != y).then(|| format!("its data: {}", first_difference(&format!("{x:?}"), &format!("{y:?}"))))
        }
    }
}

/// Where two descriptions part, with a little of each.
fn first_difference(a: &str, b: &str) -> String {
    let at = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let from = a[..at].rfind([',', '{']).unwrap_or(0);
    let cut = |s: &str| s.get(from..(at + 40).min(s.len())).unwrap_or("").to_owned();
    format!("{} read back as {}", cut(a), cut(b))
}

/// The scene read back holds what was written.
fn same(a: &SceneState, b: &SceneState) -> Result<(), String> {
    if a.objects.len() != b.objects.len() {
        return Err(format!("{} objects read back as {}", a.objects.len(), b.objects.len()));
    }
    for o in a.objects.values() {
        let Some(r) = b.find(o.class, &o.name) else { return Err(format!("{} is not read back", o.name)) };
        if let Some(what) = differs(o, r) {
            return Err(format!("{}: {what}", o.name));
        }
        if o.owner.and_then(|g| a.get(g)).map(|g| &g.name) != r.owner.and_then(|g| b.get(g)).map(|g| &g.name)
        {
            return Err(format!("{}: its group", o.name));
        }
    }
    // the build's kept values may be no numbers: told by their bits
    if format!("{:?}", a.meta.level) != format!("{:?}", b.meta.level) {
        return Err("the level's options".into());
    }
    // a layer that holds nothing is not kept
    for l in layers::LAYERS.iter().filter(|l| layers::has(**l, &a.bulk)) {
        if layers::write(*l, &a.bulk) != layers::write(*l, &b.bulk) {
            return Err(format!("the layer {}", l.file));
        }
    }
    for c in gz_scene::ObjClass::ALL {
        let (x, y) = (a.tools.part(c), b.tools.part(c));
        if x.visible != y.visible || x.locked != y.locked || x.flags != y.flags {
            return Err(format!("the eye, lock or flags of the kind {} ({x:?} read back as {y:?})", c.key()));
        }
        if x.opts != y.opts {
            return Err(format!(
                "the settings of the kind {}: {}",
                c.key(),
                first_difference(&format!("{:?}", x.opts), &format!("{:?}", y.opts))
            ));
        }
    }
    Ok(())
}

fn check(scene: &Path, settings: &(dyn gz_entities::Settings + Sync)) -> Result<(), String> {
    let main = std::fs::read(scene).map_err(|e| e.to_string())?;
    let kind = sdk::scene_kind(&main)?;
    let parts = sdk::read_parts(scene, &sdk::parts_needed(&main, kind));
    let state = sdk::parse_scene(&main, scene, &parts, Some(settings)).map_err(|e| e.to_string())?.state;
    let plan = format::write(&state, None, Some(settings)).map_err(|e| {
        let odd: Vec<String> = state
            .objects
            .values()
            .filter(|o| format!("{:?}", o.data).contains("NaN") || format!("{:?}", o.xform).contains("NaN"))
            .take(3)
            .map(|o| format!("{} {}", o.name, first_difference(&format!("{:?}", o.data), "")))
            .collect();
        let meta = format!("{:?}", state.meta);
        let tools = format!("{:?}", state.tools);
        let at =
            |t: &str| t.find("NaN").map(|i| t[i.saturating_sub(80)..i + 3].to_owned()).unwrap_or_default();
        format!("not written: {e} ({}) [{}] [{}]", odd.join("; "), at(&meta), at(&tools))
    })?;
    let disk = texts_of(&plan);
    let stem = scene.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let main_rel = RelPath::new(&format!("levels/{stem}.scene")).map_err(|e| e.to_string())?;
    let dir_rel = RelPath::new(&format!("levels/{stem}")).map_err(|e| e.to_string())?;
    let read = format::read(&disk, &main_rel, &dir_rel, Some(settings)).map_err(|r| r.problem.line_text())?;
    if let Some(p) = read.problems.iter().find(|p| p.severity == gz_base::Severity::Error) {
        return Err(format!("read back with an error: {}", p.line_text()));
    }
    same(&state, &read.state)?;
    let again = format::write(&read.state, Some(&read.files), Some(settings)).map_err(|e| e.to_string())?;
    if let Some(f) = again.files.first() {
        return Err(format!(
            "written again from what it read, {} changes",
            if f.file.is_empty() { "the .scene" } else { &f.file }
        ));
    }
    Ok(())
}

#[test]
fn every_base_scene_imports_and_reads_back_from_its_own_files() {
    let all = scenes(&gz_corpus::repo().join("rawdata").join("levels"));
    if all.is_empty() {
        gz_corpus::skipped("the repository has no rawdata\\levels");
        return;
    }
    let Some(game) = gz_corpus::game() else { return };
    let game = game.expect("the game mounts");
    let settings = gz_scene::game::GameSettings::new(Arc::new(system_ltx(&game)), None);
    let started = Instant::now();
    let failed = check_all(&all, |p| p.display().to_string(), |p| check(p, &settings));
    finish("base scenes", all.len(), started, &failed);
}

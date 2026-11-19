//! The X-Ray SDK scene importer.
//!
//! The SDK's scene is `<name>.level` and, beside it, a folder `<name>\` with one part per kind:
//! `<class>.part` (and `group.part1` for light presets). The main file and the object parts are
//! LTX, the painted layers (AI map, details, wallmarks) chunk streams; an older scene is one
//! binary `.level` with the kinds' data inside it or in the parts. The editor reads every form
//! and never writes one: an imported scene is saved as a `.scene` (`crate::format`).
//!
//! Reading is split so that the files can be read on the IO pool and parsed on the CPU pool:
//! [`scene_kind`] and [`parts_needed`] say which files to read, [`parse_scene`] parses them
//! (the parts in parallel).

pub mod ltx;
pub mod objects;
pub mod tools;

use std::path::{Path, PathBuf};

use gz_chunk::Reader;
use gz_entities::Settings;
use rayon::prelude::*;

use self::ltx::IniReader;
use self::objects::{Loaded, ReadCx};
use crate::SceneError;
use crate::class::ObjClass;
use crate::math::{Transform, Vec3};
use crate::model::{ObjData, ObjId, SceneMeta, SceneState};
use crate::tools::ToolPart;

pub const CURRENT_FILE_VERSION: u32 = 5;
const CHUNK_VERSION: u32 = 0x9df3;
const CHUNK_OBJECT_LIST: u32 = 0x7708;
const CHUNK_CAMERA: u32 = 0x7709;
const CHUNK_SNAPOBJECTS: u32 = 0x7710;
const CHUNK_LEVELOP: u32 = 0x7711;
const CHUNK_OBJECT_COUNT: u32 = 0x7712;
const CHUNK_LEVEL_TAG: u32 = 0x7777;
pub const CHUNK_TOOLS_GUID: u32 = 0x7000;
pub const CHUNK_TOOLS_DATA: u32 = 0x8000;
const TOOL_CHUNK_OBJECT_COUNT: u32 = 2;
const TOOL_CHUNK_OBJECTS: u32 = 3;

/// The extension of the SDK's scene.
pub const EXT: &str = "level";

/// The configs spawn entities decode with, shareable between the parse threads.
pub type SyncSettings<'a> = Option<&'a (dyn Settings + Sync)>;

/// How a `.level` is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    Ltx,
    Binary,
}

/// The kind of a `.level`, or why the file is not a scene.
pub fn scene_kind(bytes: &[u8]) -> Result<SceneKind, String> {
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    if bytes[0] == b'[' {
        let low = String::from_utf8_lossy(bytes).to_lowercase();
        let Some(sec) = low.find("[version]") else {
            return Err("not a level file (no [version] section)".into());
        };
        let val = low[sec..].find("value").map(|v| v + sec);
        let next = low[sec + 1..].find('[').map(|n| n + sec + 1);
        let Some(val) = val.filter(|v| next.is_none_or(|n| *v < n)) else {
            return Err("not a level file (no version value)".into());
        };
        let version = low[val..].find('=').map_or(0, |eq| atoi(&low[val + eq + 1..]));
        return if version == i64::from(CURRENT_FILE_VERSION) {
            Ok(SceneKind::Ltx)
        } else {
            Err("unsupported level version".into())
        };
    }
    match Reader::new(bytes).find_chunk(CHUNK_VERSION) {
        None => Err("not a level file (no version chunk)".into()),
        Some((b, _)) if b.len() != 4 => Err("not a level file (invalid version chunk)".into()),
        Some((b, _)) if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != CURRENT_FILE_VERSION => {
            Err("unsupported level version".into())
        }
        Some(_) => Ok(SceneKind::Binary),
    }
}

fn atoi(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n: i64 = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |a, d| a.saturating_mul(10).saturating_add(i64::from(d - b'0')));
    if neg { -n } else { n }
}

/// `<dir>\<stem>\`.
#[must_use]
pub fn part_dir(scene: &Path) -> PathBuf {
    crate::format::folder_of(scene)
}

/// `<class>.part`, `<class>.part1` ...
#[must_use]
pub fn part_path(scene: &Path, class: ObjClass, index: usize) -> PathBuf {
    let base = format!("{}.part", class.name());
    part_dir(scene).join(if index == 0 { base } else { format!("{base}{index}") })
}

/// Whether a class's part is LTX (the object kinds) or a chunk stream (the painted layers).
#[must_use]
pub fn uses_ltx(class: ObjClass) -> bool {
    class.is_object_tool()
}

/// The classes whose part files a load reads: every one for an LTX scene, the ones the binary
/// file does not carry inside itself otherwise.
#[must_use]
pub fn parts_needed(main: &[u8], kind: SceneKind) -> Vec<ObjClass> {
    match kind {
        SceneKind::Ltx => ObjClass::ALL.to_vec(),
        SceneKind::Binary => {
            let r = Reader::new(main);
            ObjClass::ALL
                .iter()
                .copied()
                .filter(|c| r.find_chunk(CHUNK_TOOLS_DATA + c.id()).is_none())
                .collect()
        }
    }
}

/// The part files of one class: each file's path and bytes.
pub type ClassFiles = Vec<(PathBuf, Vec<u8>)>;

/// The part files that were found, per class, in their numbered order.
#[derive(Default)]
pub struct PartFiles {
    pub files: Vec<(ObjClass, ClassFiles)>,
}

/// Reads the numbered part files of the classes a load needs (blocking).
#[must_use]
pub fn read_parts(file: &Path, classes: &[ObjClass]) -> PartFiles {
    let files = classes
        .par_iter()
        .map(|c| {
            let mut out = Vec::new();
            for i in 0.. {
                if i > 0 && !uses_ltx(*c) {
                    break;
                }
                let p = part_path(file, *c, i);
                match std::fs::read(&p) {
                    Ok(b) => out.push((p, b)),
                    Err(_) => break,
                }
            }
            (*c, out)
        })
        .collect();
    PartFiles { files }
}

/// An imported scene and what the import noticed.
pub struct LoadedScene {
    pub state: SceneState,
    pub warnings: Vec<String>,
}

/// One kind's share of a load.
struct ToolLoad {
    class: ObjClass,
    tool: ToolPart,
    objects: Vec<Loaded>,
    bulk: Option<Vec<u8>>,
    warnings: Vec<String>,
}

impl ToolLoad {
    fn new(class: ObjClass) -> Self {
        Self { class, tool: ToolPart::new(class), objects: Vec::new(), bulk: None, warnings: Vec::new() }
    }
}

fn guid_from(b: &[u8]) -> [u64; 2] {
    let mut a = [0u8; 8];
    let mut c = [0u8; 8];
    a.copy_from_slice(b.get(..8).unwrap_or(&[0; 8]));
    c.copy_from_slice(b.get(8..16).unwrap_or(&[0; 8]));
    [u64::from_le_bytes(a), u64::from_le_bytes(c)]
}

/// The objects and the kind's own data of a kind's chunk.
fn load_tool_stream(
    class: ObjClass,
    body: &[u8],
    settings: SyncSettings<'_>,
) -> Result<ToolLoad, SceneError> {
    let mut load = ToolLoad::new(class);
    if !class.is_object_tool() {
        load.bulk = Some(body.to_vec());
        return Ok(load);
    }
    let r = Reader::new(body);
    let rejected = || SceneError::load(format!("the scene's {} could not be read", class.label_plural()));
    tools::tool_stream(&mut load.tool, &r)?;
    let count = match r.find_chunk(TOOL_CHUNK_OBJECT_COUNT) {
        Some((b, false)) if b.len() == 4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => return Err(rejected()),
    };
    if count < 0 {
        return Err(rejected());
    }
    let mut cx = ReadCx::new(settings.map(|s| s as &dyn Settings));
    if let Some(list) = r.open_chunk(TOOL_CHUNK_OBJECTS)? {
        load.objects = objects::read_stream_list(&list.reader(), &mut cx)?;
    } else if count > 0 {
        return Err(rejected());
    }
    if count > 0 && load.objects.len() != count as usize {
        return Err(rejected());
    }
    load.warnings = cx.warnings;
    Ok(load)
}

/// One numbered LTX part file, added to what earlier files gave.
fn load_part_ltx(
    load: &mut ToolLoad,
    text: &str,
    file: &Path,
    guid: [u64; 2],
    settings: SyncSettings<'_>,
) -> Result<(), SceneError> {
    let fname = file.display().to_string();
    let r = IniReader::parse(text, &fname)?;
    if !r.line_exist("guid", "guid_g0") || !r.line_exist("guid", "guid_g1") {
        return Err(SceneError::load(format!("Level part has no GUID section: {fname}")));
    }
    if [r.r_u64("guid", "guid_g0")?, r.r_u64("guid", "guid_g1")?] != guid {
        return Err(SceneError::load(format!("Level part GUID does not match the scene: {fname}")));
    }
    tools::tool_ltx(&mut load.tool, &r)?;
    if !r.line_exist("main", "objects_count") {
        return Err(SceneError::load(format!("{}: objects_count is missing", load.class.label_plural())));
    }
    let count = r.r_u32("main", "objects_count")?;
    let mut cx = ReadCx::new(settings.map(|s| s as &dyn Settings));
    for i in 0..count {
        let s = format!("object_{i}");
        if !r.section_exist(&s) {
            return Err(SceneError::load(format!("{}: section '{s}' is missing", load.class.label_plural())));
        }
        load.objects.push(objects::read_ltx(&r, &s, &mut cx)?);
    }
    load.warnings.append(&mut cx.warnings);
    Ok(())
}

/// A kind's part files: LTX files numbered until the first gap, or one chunk stream.
fn load_part(
    class: ObjClass,
    files: &[(PathBuf, Vec<u8>)],
    guid: [u64; 2],
    settings: SyncSettings<'_>,
) -> Result<ToolLoad, SceneError> {
    let mut load = ToolLoad::new(class);
    for (path, bytes) in files {
        if bytes.is_empty() {
            return Err(SceneError::load(format!("Level part is empty: {}", path.display())));
        }
        if bytes[0] == b'[' && uses_ltx(class) {
            let text = gz_base::text::decode_guess(bytes);
            load_part_ltx(&mut load, &text, path, guid, settings)?;
            continue;
        }
        let r = Reader::new(bytes);
        match r.find_chunk(CHUNK_TOOLS_GUID) {
            Some((g, false)) if g.len() == 16 => {
                if guid_from(g) != guid {
                    return Err(SceneError::load(format!(
                        "Level part GUID does not match the scene: {}",
                        path.display()
                    )));
                }
            }
            _ => {
                return Err(SceneError::load(format!(
                    "Level part has an invalid GUID chunk: {}",
                    path.display()
                )));
            }
        }
        let Some(data) = r.open_chunk(CHUNK_TOOLS_DATA + class.id())? else {
            return Err(SceneError::load(format!("Level part has no tool data chunk: {}", path.display())));
        };
        let mut l = load_tool_stream(class, data.bytes(), settings)?;
        l.objects.splice(0..0, std::mem::take(&mut load.objects));
        return Ok(l);
    }
    Ok(load)
}

/// Reads an SDK scene: `main` is the `.level`, `parts` the part files that exist. The scene
/// comes as the editor's model, its file the `.scene` beside the `.level` (not written yet).
pub fn parse_scene(
    main: &[u8],
    file: &Path,
    parts: &PartFiles,
    settings: SyncSettings<'_>,
) -> Result<LoadedScene, SceneError> {
    let fname = file.display().to_string();
    let kind =
        scene_kind(main).map_err(|why| SceneError::load(format!("Can't load map '{fname}': {why}")))?;
    let mut meta = SceneMeta {
        file: Some(file.with_extension(crate::format::EXT)),
        imported: Some(file.to_path_buf()),
        ..SceneMeta::default()
    };
    let mut loads: Vec<ToolLoad> = Vec::new();
    let mut warnings = Vec::new();
    let mut snap_names: Vec<String> = Vec::new();
    let mut legacy: Vec<Loaded> = Vec::new();
    let guid = match kind {
        SceneKind::Ltx => {
            let text = gz_base::text::decode_guess(main);
            let r = IniReader::parse(&text, &fname)?;
            meta.level = tools::level_ltx(&r)?;
            let complete = ["camera:pos", "camera:hpb", "guid:guid_g0", "guid:guid_g1"]
                .iter()
                .all(|k| k.split_once(':').is_some_and(|(s, key)| r.line_exist(s, key)));
            if !complete {
                return Err(SceneError::load(format!("LTX scene metadata is incomplete: {fname}")));
            }
            meta.camera_pos = r.r_fvector3("camera", "pos")?;
            meta.camera_hpb = r.r_fvector3("camera", "hpb")?;
            snap_names = r.lines("snap_objects").into_iter().map(|(k, _)| k).collect();
            const OWN: [&str; 7] =
                ["version", "level_options", "build_params", "guid", "level_tag", "camera", "snap_objects"];
            for s in r.sections().filter(|s| !OWN.contains(s)) {
                warnings.push(format!(
                    "the section [{s}] of {fname} means nothing to the editor: it is not imported"
                ));
            }
            [r.r_u64("guid", "guid_g0")?, r.r_u64("guid", "guid_g1")?]
        }
        SceneKind::Binary => {
            let r = Reader::new(main);
            let lop = r.open_chunk(CHUNK_LEVELOP)?.ok_or_else(|| {
                SceneError::load(format!("Binary scene has no level-options chunk: {fname}"))
            })?;
            meta.level = tools::level_bin(&lop.reader())
                .map_err(|_| SceneError::load(format!("Invalid binary level options: {fname}")))?;
            if let Some(c) = r.open_chunk(CHUNK_CAMERA)? {
                if c.bytes().len() != 24 {
                    return Err(SceneError::load(format!(
                        "Binary scene has an invalid camera chunk: {fname}"
                    )));
                }
                let mut q = c.reader();
                meta.camera_hpb = q.r_vec3()?;
                meta.camera_pos = q.r_vec3()?;
            }
            let guid = match r.find_chunk(CHUNK_TOOLS_GUID) {
                Some((g, false)) if g.len() == 16 => guid_from(g),
                _ => {
                    return Err(SceneError::load(format!(
                        "Binary scene has an invalid tool GUID chunk: {fname}"
                    )));
                }
            };
            // the level tag names the machine and the user who saved it: not imported
            let _ = r.open_chunk(CHUNK_LEVEL_TAG)?;
            let count = r.find_chunk(CHUNK_OBJECT_COUNT);
            let list = r.open_chunk(CHUNK_OBJECT_LIST)?;
            if count.is_some() != list.is_some() {
                return Err(SceneError::load(format!(
                    "Binary scene has incomplete legacy object metadata: {fname}"
                )));
            }
            if let (Some((cb, _)), Some(list)) = (count, list) {
                if cb.len() != 4 {
                    return Err(SceneError::load(format!(
                        "Binary scene has an invalid legacy object-count chunk: {fname}"
                    )));
                }
                let n = u32::from_le_bytes([cb[0], cb[1], cb[2], cb[3]]);
                let mut cx = ReadCx::new(settings.map(|s| s as &dyn Settings));
                legacy = objects::read_stream_list(&list.reader(), &mut cx)?;
                warnings.append(&mut cx.warnings);
                if legacy.len() != n as usize {
                    return Err(SceneError::load(format!(
                        "Binary scene '{fname}' declares {n} legacy objects but contains {}",
                        legacy.len()
                    )));
                }
            }
            let embedded: Vec<(ObjClass, Vec<u8>)> = ObjClass::ALL
                .iter()
                .filter_map(|c| {
                    r.open_chunk(CHUNK_TOOLS_DATA + c.id()).ok().flatten().map(|ch| (*c, ch.into_owned()))
                })
                .collect();
            let results: Vec<Result<ToolLoad, SceneError>> =
                embedded.par_iter().map(|(c, body)| load_tool_stream(*c, body, settings)).collect();
            for res in results {
                loads.push(res.map_err(|e| SceneError::load(format!("{e} in '{fname}'")))?);
            }
            if let Some(c) = r.open_chunk(CHUNK_SNAPOBJECTS)? {
                let mut q = c.reader();
                let n = q.r_u32()?;
                for _ in 0..n {
                    snap_names.push(q.r_stringz().map_err(|_| {
                        SceneError::load(format!("Binary scene has a truncated snap-object list: {fname}"))
                    })?);
                }
                if !q.eof() {
                    return Err(SceneError::load(format!(
                        "Binary scene has trailing snap-object data: {fname}"
                    )));
                }
            }
            guid
        }
    };
    let results: Vec<Result<ToolLoad, SceneError>> = parts
        .files
        .par_iter()
        .filter(|(c, files)| !files.is_empty() && !loads.iter().any(|l| l.class == *c))
        .map(|(c, files)| load_part(*c, files, guid, settings))
        .collect();
    for res in results {
        loads.push(res?);
    }
    loads.sort_by_key(|l| l.class);
    let mut state = SceneState { meta, ..SceneState::default() };
    for l in loads {
        warnings.extend(l.warnings);
        if let Some(bulk) = &l.bulk
            && let Some(w) = state.bulk.set_stream(l.class, bulk)
        {
            warnings.push(w);
        }
        *state.tools.part_mut(l.class) = l.tool;
        for o in l.objects {
            insert_loaded(&mut state, o);
        }
    }
    for o in legacy {
        insert_loaded(&mut state, o);
    }
    // the list the moving tools snapped to and the wallmarks lay on: both take the whole level
    // now (the AI map's and the detail objects' own lists are kept with their layers)
    if !snap_names.is_empty() {
        warnings.push(format!(
            "the scene's Snap Objects list ({} objects) is not imported: placing and the wallmarks work on the whole level",
            snap_names.len()
        ));
    }
    // an object of a version the editor does not read keeps its name and its place
    let kept: Vec<ObjId> =
        state.objects.values().filter(|o| matches!(o.data, ObjData::Kept(_))).map(|o| o.id).collect();
    for id in kept {
        let Some(o) = state.get(id) else { continue };
        let mut n = (**o).clone();
        if let ObjData::Kept(k) = &n.data {
            let why = match k {
                crate::objects::KeptObject::Ltx { why, .. }
                | crate::objects::KeptObject::Stream { why, .. } => why,
            };
            warnings.push(format!(
                "'{}' is of an X-Ray SDK version the editor does not read ({why}): only its place is kept",
                n.name
            ));
        }
        n.data = crate::objects::default_data(n.class);
        state.replace(n);
    }
    // a zone's shape is said once: a point whose entity says no shapes takes its shape object's
    let zones: Vec<_> = state.of_class(crate::class::ObjClass::SpawnPoint).cloned().collect();
    for zone in zones {
        let mut synced = (*zone).clone();
        crate::objects::sync_shape(&mut synced);
        if synced != *zone {
            state.replace(synced);
        }
    }
    Ok(LoadedScene { state, warnings })
}

/// What the scene's model keeps of an imported object's transform and flags: what its kind has
/// (a sound source stands where its sound plays, a way path where its points are, a glow's
/// size is its radius), and none of the flag bits the SDK no longer means anything by.
fn normalised(mut obj: crate::model::Object) -> crate::model::Object {
    let caps = obj.class.caps();
    match &obj.data {
        ObjData::SoundSrc(s) => obj.xform = Transform::at(s.position),
        ObjData::Way(_) | ObjData::Sector(_) | ObjData::Portal(_) => obj.xform = Transform::default(),
        _ => {}
    }
    if !caps.rotates {
        obj.xform.rotation = Vec3::ZERO;
    }
    if !caps.scales || obj.class == ObjClass::Glow {
        obj.xform.scale = Vec3::ONE;
    }
    obj.flags.co &= !crate::format::fields::CO_LEGACY;
    // a -0 means 0, but where a spawn packet holds it: that packet is kept to the byte
    if obj.class != ObjClass::SpawnPoint {
        let x = &mut obj.xform;
        (x.position, x.rotation, x.scale) =
            (x.position + Vec3::ZERO, x.rotation + Vec3::ZERO, x.scale + Vec3::ZERO);
    }
    // which points were selected is the SDK's view, not the scene's
    if let ObjData::Way(w) = &mut obj.data {
        for p in &mut w.points {
            p.selected = false;
        }
    }
    obj
}

/// Adds an imported object: a name already used gets a free one, a group's members go in first.
pub fn insert_loaded(state: &mut SceneState, l: Loaded) -> ObjId {
    let mut members = Vec::with_capacity(l.members.len());
    for m in l.members {
        members.push(insert_loaded(state, m));
    }
    let mut obj = normalised(l.obj);
    if !state.name_free(&obj.name) {
        obj.name = state.unique_name(&obj.name).into();
    }
    let is_group = matches!(obj.data, ObjData::Group(_));
    if let ObjData::Group(g) = &mut obj.data {
        g.members = members.clone();
    }
    let id = state.insert(obj);
    if is_group {
        for m in members {
            if let Some(o) = state.get(m) {
                let mut o = (**o).clone();
                o.owner = Some(id);
                state.replace(o);
            }
        }
    }
    id
}

/// A group body's chunks with the reference chunk (4) naming `reference`; a body that does not
/// parse as chunks is given back as it is.
fn with_reference(body: &[u8], reference: &str) -> Vec<u8> {
    const CHUNK_REFERENCE: u32 = 4;
    let chunk = |out: &mut Vec<u8>| {
        out.extend_from_slice(&CHUNK_REFERENCE.to_le_bytes());
        out.extend_from_slice(&u32::try_from(reference.len() + 1).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(reference.as_bytes());
        out.push(0);
    };
    let mut out = Vec::with_capacity(body.len() + reference.len() + 9);
    let mut at = 0;
    let mut written = false;
    while at + 8 <= body.len() {
        let id = u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
        let size = u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]) as usize;
        let Some(end) = (at + 8).checked_add(size).filter(|e| *e <= body.len()) else {
            return body.to_vec();
        };
        if id != CHUNK_REFERENCE {
            out.extend_from_slice(&body[at..end]);
        } else if !written {
            chunk(&mut out);
            written = true;
        }
        at = end;
    }
    if at != body.len() {
        return body.to_vec();
    }
    if !written {
        chunk(&mut out);
    }
    out
}

/// A group of the library as the X-Ray SDK writes it (the object's body without the scene's
/// class wrapper, its reference empty): read as a group named `reference`. Answers it and the
/// reader's warnings.
pub fn read_library_group(
    body: &[u8],
    reference: &str,
    settings: Option<&(dyn gz_entities::Settings + Sync)>,
) -> Result<(Loaded, Vec<String>), String> {
    let mut w = gz_chunk::Writer::new();
    w.open_chunk(objects::CHUNK_OBJECT_CLASS);
    w.w_u32(ObjClass::Group.id());
    w.close_chunk();
    w.open_chunk(objects::CHUNK_OBJECT_BODY);
    w.w(&with_reference(body, reference));
    w.close_chunk();
    let bytes = w.into_bytes();
    let mut cx = objects::ReadCx::new(settings.map(|s| s as &dyn gz_entities::Settings));
    let loaded = objects::read_stream(&gz_chunk::Reader::new(&bytes), &mut cx)
        .map_err(|e| format!("the group {reference} could not be read: {e}"))?;
    if !matches!(loaded.obj.data, ObjData::Group(_)) {
        return Err(format!("the group {reference} holds no objects"));
    }
    Ok((loaded, cx.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_a_scene_says_why() {
        assert_eq!(scene_kind(b""), Err("the file is empty".into()));
        assert_eq!(scene_kind(b"[other]\r\nx=1"), Err("not a level file (no [version] section)".into()));
        assert_eq!(scene_kind(b"[version]\r\nvalue = 4\r\n"), Err("unsupported level version".into()));
        assert_eq!(scene_kind(b"\x01\x02\x03"), Err("not a level file (no version chunk)".into()));
    }

    fn chunk(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        v.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn a_library_group_gets_the_reference_it_was_opened_by() {
        let body = [chunk(0, &[0x11, 0]), chunk(4, &[0]), chunk(1, &[1, 2, 3])].concat();
        let want = [chunk(0, &[0x11, 0]), chunk(4, b"doors\0"), chunk(1, &[1, 2, 3])].concat();
        assert_eq!(with_reference(&body, "doors"), want);
        // no reference chunk: one is added; a torn body is left as it is
        let bare = chunk(0, &[0x11, 0]);
        assert_eq!(with_reference(&bare, "a"), [bare.clone(), chunk(4, b"a\0")].concat());
        assert_eq!(with_reference(&bare[..5], "a"), bare[..5].to_vec());
    }
}

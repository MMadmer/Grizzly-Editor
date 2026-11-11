//! The scene files (`SceneIO.cpp`).
//!
//! A scene is `<name>.level` and, beside it, a folder `<name>\` with one part per tool:
//! `<class>.part` (and `group.part1` for light presets). The C++ editor writes the main file
//! and the object parts as LTX and the bulk parts (AI map, details, wallmarks) as chunk
//! streams; it reads that, the older binary `.level` (tool data inside it or in the parts) and
//! its own undo / recovery stream. Reading is split so that the files can be read on the IO pool
//! and parsed on the CPU pool: [`scene_kind`] and [`parts_needed`] say which files to read,
//! [`parse_scene`] parses them (the parts in parallel).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::SmolStr;
use gz_chunk::{Reader, Writer};
use gz_entities::Settings;
use rayon::prelude::*;

use crate::SceneError;
use crate::class::ObjClass;
use crate::ltx::{IniReader, IniWriter};
use crate::model::{ObjData, ObjId, Object, SceneMeta, SceneState, co};
use crate::objects::{self, Loaded, ReadCx};
use crate::tools::{LevelOptions, ToolPart};

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

/// The configs spawn entities decode with, shareable between the parse threads.
pub type SyncSettings<'a> = Option<&'a (dyn Settings + Sync)>;

/// How a `.level` is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    Ltx,
    Binary,
}

/// `IsSceneFile`: the kind, or why the file is not a scene (the C++ texts).
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
        return if version == CURRENT_FILE_VERSION as i64 {
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

/// `LevelPartPath`: `<dir>\<stem>\`.
#[must_use]
pub fn part_dir(scene: &Path) -> PathBuf {
    let stem = scene.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    scene.parent().map_or_else(|| PathBuf::from(&stem), |p| p.join(&stem))
}

/// `<class>.part`, `<class>.part1` ...
#[must_use]
pub fn part_path(scene: &Path, class: ObjClass, index: usize) -> PathBuf {
    let base = format!("{}.part", class.name());
    part_dir(scene).join(if index == 0 { base } else { format!("{base}{index}") })
}

/// `LegacySaveBackupName`: `x.level` keeps the last version as `x.~level`.
#[must_use]
pub fn backup_path(target: &Path) -> PathBuf {
    match target.extension() {
        Some(ext) => target.with_extension(format!("~{}", ext.to_string_lossy())),
        None => {
            let mut s = target.as_os_str().to_owned();
            s.push("~");
            PathBuf::from(s)
        }
    }
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

/// The part files that were found, per class, in their numbered order.
/// The part files of one class: each file's path and bytes.
pub type ClassFiles = Vec<(PathBuf, Vec<u8>)>;

#[derive(Default)]
pub struct PartFiles {
    pub files: Vec<(ObjClass, ClassFiles)>,
}

/// A loaded scene and what the load noticed.
pub struct LoadedScene {
    pub state: SceneState,
    pub warnings: Vec<String>,
}

/// One tool's share of a load.
struct ToolLoad {
    class: ObjClass,
    tool: ToolPart,
    objects: Vec<Loaded>,
    bulk: Option<Vec<u8>>,
    warnings: Vec<String>,
}

fn guid_bytes(g: [u64; 2]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&g[0].to_le_bytes());
    out[8..].copy_from_slice(&g[1].to_le_bytes());
    out
}

fn guid_from(b: &[u8]) -> [u64; 2] {
    let mut a = [0u8; 8];
    let mut c = [0u8; 8];
    a.copy_from_slice(b.get(..8).unwrap_or(&[0; 8]));
    c.copy_from_slice(b.get(8..16).unwrap_or(&[0; 8]));
    [u64::from_le_bytes(a), u64::from_le_bytes(c)]
}

/// The objects and the tool's own data of a tool chunk (`ESceneCustomOTool::LoadStream`).
fn load_tool_stream(
    class: ObjClass,
    body: &[u8],
    settings: SyncSettings<'_>,
) -> Result<ToolLoad, SceneError> {
    let mut tool = ToolPart::new(class);
    let mut load =
        ToolLoad { class, tool: ToolPart::new(class), objects: Vec::new(), bulk: None, warnings: Vec::new() };
    if !class.is_object_tool() {
        load.bulk = Some(body.to_vec());
        return Ok(load);
    }
    let r = Reader::new(body);
    let rejected =
        || SceneError::load(format!("Editor tool '{}' rejected embedded scene data", class.name()));
    tool.read_stream(&r)?;
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
    load.tool = tool;
    load.warnings = cx.warnings;
    Ok(load)
}

/// `LoadLevelPartLTX` for one numbered file, added to what earlier files gave.
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
    let used = load.tool.read_ltx(&r)?;
    if !r.line_exist("main", "objects_count") {
        return Err(SceneError::load(format!(
            "{} tools: required objects_count is missing",
            load.class.desc()
        )));
    }
    let count = r.r_u32("main", "objects_count")?;
    let mut cx = ReadCx::new(settings.map(|s| s as &dyn Settings));
    for i in 0..count {
        let s = format!("object_{i}");
        if !r.section_exist(&s) {
            return Err(SceneError::load(format!(
                "{} tools: required section '{s}' is missing",
                load.class.desc()
            )));
        }
        load.objects.push(objects::read_ltx(&r, &s, &mut cx)?);
    }
    load.warnings.append(&mut cx.warnings);
    // what neither the tool nor its objects read stays as it is
    let own = load.tool.own_sections();
    for sect in r.sections() {
        let object = sect
            .strip_prefix("object_")
            .and_then(|rest| rest.split('_').next())
            .and_then(|n| n.parse::<u32>().ok());
        if object.is_some_and(|n| n < count) {
            continue;
        }
        if sect == "main" {
            for (k, v) in r.lines(sect) {
                if !used.contains(&k.as_str()) {
                    load.tool.kept_ltx.push((sect.to_owned(), k, v));
                }
            }
            continue;
        }
        if own.contains(&sect) {
            continue;
        }
        for (k, v) in r.lines(sect) {
            load.tool.kept_ltx.push((sect.to_owned(), k, v));
        }
    }
    Ok(())
}

/// A tool's part files: LTX files numbered until the first gap, or one chunk stream.
fn load_part(
    class: ObjClass,
    files: &[(PathBuf, Vec<u8>)],
    guid: [u64; 2],
    settings: SyncSettings<'_>,
) -> Result<ToolLoad, SceneError> {
    let mut load =
        ToolLoad { class, tool: ToolPart::new(class), objects: Vec::new(), bulk: None, warnings: Vec::new() };
    for (path, bytes) in files {
        if bytes.is_empty() {
            return Err(SceneError::load(format!("Level part is empty: {}", path.display())));
        }
        if bytes[0] == b'[' && class.uses_ltx() {
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

/// Reads the scene: `main` is the `.level`, `parts` the part files that exist.
pub fn parse_scene(
    main: &[u8],
    file: &Path,
    parts: &PartFiles,
    settings: SyncSettings<'_>,
) -> Result<LoadedScene, SceneError> {
    let fname = file.display().to_string();
    let kind =
        scene_kind(main).map_err(|why| SceneError::load(format!("Can't load map '{fname}': {why}")))?;
    let mut meta = SceneMeta { file: Some(file.to_path_buf()), ..SceneMeta::default() };
    let mut loads: Vec<ToolLoad> = Vec::new();
    let mut warnings = Vec::new();
    let mut snap_names: Vec<String> = Vec::new();
    let mut legacy: Vec<Loaded> = Vec::new();
    match kind {
        SceneKind::Ltx => {
            let text = gz_base::text::decode_guess(main);
            let r = IniReader::parse(&text, &fname)?;
            meta.level = LevelOptions::read_ltx(&r)?;
            let complete = [
                "camera:pos",
                "camera:hpb",
                "guid:guid_g0",
                "guid:guid_g1",
                "level_tag:owner",
                "level_tag:create_time",
            ]
            .iter()
            .all(|k| k.split_once(':').is_some_and(|(s, key)| r.line_exist(s, key)));
            if !complete {
                return Err(SceneError::load(format!("LTX scene metadata is incomplete: {fname}")));
            }
            meta.camera_pos = r.r_fvector3("camera", "pos")?;
            meta.camera_hpb = r.r_fvector3("camera", "hpb")?;
            meta.guid = [r.r_u64("guid", "guid_g0")?, r.r_u64("guid", "guid_g1")?];
            meta.owner = r.r_string("level_tag", "owner")?.into();
            meta.create_time = r.r_u32("level_tag", "create_time")?;
            snap_names = r.lines("snap_objects").into_iter().map(|(k, _)| k).collect();
            const OWN: [&str; 7] =
                ["version", "level_options", "build_params", "guid", "level_tag", "camera", "snap_objects"];
            meta.kept =
                r.sections().filter(|s| !OWN.contains(s)).map(|s| (s.to_owned(), r.lines(s))).collect();
        }
        SceneKind::Binary => {
            let r = Reader::new(main);
            let lop = r.open_chunk(CHUNK_LEVELOP)?.ok_or_else(|| {
                SceneError::load(format!("Binary scene has no level-options chunk: {fname}"))
            })?;
            meta.level = LevelOptions::read_bin(&lop.reader())
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
            match r.find_chunk(CHUNK_TOOLS_GUID) {
                Some((g, false)) if g.len() == 16 => meta.guid = guid_from(g),
                _ => {
                    return Err(SceneError::load(format!(
                        "Binary scene has an invalid tool GUID chunk: {fname}"
                    )));
                }
            }
            if let Some(c) = r.open_chunk(CHUNK_LEVEL_TAG)? {
                let b = c.bytes();
                if b.len() <= 4
                    || b.get(..b.len() - 4).and_then(|s| s.iter().position(|x| *x == 0)) != Some(b.len() - 5)
                {
                    return Err(SceneError::load(format!(
                        "Binary scene has an invalid level-tag chunk: {fname}"
                    )));
                }
                let mut q = c.reader();
                meta.owner = q.r_stringz()?.into();
                meta.create_time = q.r_u32()?;
            }
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
        }
    }
    let guid = meta.guid;
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
    for name in snap_names {
        match state.find(ObjClass::SceneObject, &name) {
            Some(o) => {
                let id = o.id;
                state.tools.snap.push(id);
            }
            None => warnings.push(format!("EScene: Can't find snap object '{name}'.")),
        }
    }
    Ok(LoadedScene { state, warnings })
}

/// Adds a read object: a name already used gets the next free one (`GenObjectName`), a group's
/// members go in first.
pub fn insert_loaded(state: &mut SceneState, l: Loaded) -> ObjId {
    let mut members = Vec::with_capacity(l.members.len());
    for m in l.members {
        members.push(insert_loaded(state, m));
    }
    let mut obj = l.obj;
    if state.name_used(&obj.name) {
        obj.name = state.gen_name(Some(&obj.name)).into();
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

/// A file the save writes, or one it removes (a stale numbered part).
#[derive(Clone, Debug)]
pub struct SaveFile {
    pub path: PathBuf,
    pub bytes: Option<Vec<u8>>,
}

/// The objects a tool saves: not inside a group, with this `save_id`.
fn saved_objects(state: &SceneState, class: ObjClass, save_id: usize) -> Vec<&Arc<Object>> {
    state
        .of_class(class)
        .filter(|o| o.flags.co & co::IN_GROUP == 0 || o.owner.is_none())
        .filter(|o| o.owner.is_none_or(|g| state.get(g).is_none()))
        .filter(|o| match &o.data {
            ObjData::Group(g) => g.save_id() == save_id,
            _ => save_id == 0,
        })
        .collect()
}

/// One LTX part file of a tool.
fn tool_ltx(state: &SceneState, class: ObjClass, index: usize) -> Result<Vec<u8>, SceneError> {
    let mut w = IniWriter::new();
    state.tools.part(class).write_ltx(&mut w);
    let objs = saved_objects(state, class, index);
    for (i, o) in objs.iter().enumerate() {
        objects::write_ltx(&mut w, &format!("object_{i}"), o, state, 0)?;
    }
    w.w_u32("main", "objects_count", u32::try_from(objs.len()).unwrap_or(0));
    w.w_u64("guid", "guid_g0", state.meta.guid[0]);
    w.w_u64("guid", "guid_g1", state.meta.guid[1]);
    Ok(w.save().into_bytes())
}

/// A tool's chunk (`SaveStream`).
pub fn tool_stream(state: &SceneState, class: ObjClass) -> Result<Vec<u8>, SceneError> {
    if !class.is_object_tool() {
        return Ok(state.bulk.stream_of(class));
    }
    let mut w = Writer::new();
    let objs: Vec<&Object> = state
        .of_class(class)
        .filter(|o| o.owner.is_none_or(|g| state.get(g).is_none()))
        .map(AsRef::as_ref)
        .collect();
    w.open_chunk(TOOL_CHUNK_OBJECTS);
    objects::write_stream_list(&mut w, objs.iter().copied(), state, 0)?;
    w.close_chunk();
    w.w_chunk(TOOL_CHUNK_OBJECT_COUNT, &i32::try_from(objs.len()).unwrap_or(i32::MAX).to_le_bytes());
    state.tools.part(class).write_stream_tail(&mut w);
    Ok(w.into_bytes())
}

/// The main LTX file (`EScene::SaveLTX`'s `ini`).
#[must_use]
pub fn main_ltx(state: &SceneState) -> Vec<u8> {
    let m = &state.meta;
    let mut w = IniWriter::new();
    w.w_u32("version", "value", CURRENT_FILE_VERSION);
    m.level.write_ltx(&mut w);
    w.w_u64("guid", "guid_g0", m.guid[0]);
    w.w_u64("guid", "guid_g1", m.guid[1]);
    w.w_string("level_tag", "owner", &m.owner);
    w.w_u32("level_tag", "create_time", m.create_time);
    w.w_fvector3("camera", "hpb", m.camera_hpb);
    w.w_fvector3("camera", "pos", m.camera_pos);
    for id in &state.tools.snap {
        if let Some(o) = state.get(*id) {
            w.w_key("snap_objects", &o.name);
        }
    }
    for (s, lines) in &m.kept {
        for (k, v) in lines {
            w.w_raw(s, k, v);
        }
    }
    w.save().into_bytes()
}

/// Everything `EScene::SaveLTX` writes for `file`, part files numbered past what the tool needs
/// listed as files to remove. Parts of tools that are disabled or read-only are left alone.
pub fn write_scene(state: &SceneState, file: &Path) -> Result<Vec<SaveFile>, SceneError> {
    let classes: Vec<ObjClass> = ObjClass::ALL
        .iter()
        .copied()
        .filter(|c| state.tools.part(*c).enabled && state.tools.part(*c).editable())
        .collect();
    let parts: Vec<Result<Vec<SaveFile>, SceneError>> = classes
        .par_iter()
        .map(|c| -> Result<Vec<SaveFile>, SceneError> {
            let mut out = Vec::new();
            if c.uses_ltx() {
                for i in 0..c.save_file_count() {
                    out.push(SaveFile { path: part_path(file, *c, i), bytes: Some(tool_ltx(state, *c, i)?) });
                }
                out.push(SaveFile { path: part_path(file, *c, c.save_file_count()), bytes: None });
            } else {
                let mut w = Writer::new();
                w.w_chunk(CHUNK_TOOLS_GUID, &guid_bytes(state.meta.guid));
                w.w_chunk(CHUNK_TOOLS_DATA + c.id(), &tool_stream(state, *c)?);
                out.push(SaveFile { path: part_path(file, *c, 0), bytes: Some(w.into_bytes()) });
            }
            Ok(out)
        })
        .collect();
    let mut files = Vec::new();
    for p in parts {
        files.extend(p?);
    }
    files.push(SaveFile { path: file.to_path_buf(), bytes: Some(main_ltx(state)) });
    Ok(files)
}

/// `SaveUndoStream`: the binary scene the recovery snapshots are. `force` writes every tool that
/// has something (`bForceSaveAll`).
pub fn write_undo_stream(state: &SceneState, force: bool) -> Result<Vec<u8>, SceneError> {
    let _ = force;
    let m = &state.meta;
    let mut w = Writer::new();
    w.chunk(CHUNK_VERSION, |w| w.w_u32(CURRENT_FILE_VERSION));
    w.chunk(CHUNK_LEVELOP, |w| m.level.write_bin(w));
    w.w_chunk(CHUNK_TOOLS_GUID, &guid_bytes(m.guid));
    w.chunk(CHUNK_LEVEL_TAG, |w| {
        w.w_stringz(&m.owner);
        w.w_u32(m.create_time);
    });
    w.chunk(CHUNK_CAMERA, |w| {
        w.w_vec3(m.camera_hpb);
        w.w_vec3(m.camera_pos);
    });
    let snap: Vec<&SmolStr> =
        state.tools.snap.iter().filter_map(|id| state.get(*id)).map(|o| &o.name).collect();
    w.chunk(CHUNK_SNAPOBJECTS, |w| {
        w.w_u32(u32::try_from(snap.len()).unwrap_or(0));
        for n in &snap {
            w.w_stringz(n);
        }
    });
    for c in ObjClass::ALL {
        let part = state.tools.part(c);
        if !part.enabled || !part.editable() || !needs_save(state, c) {
            continue;
        }
        w.w_chunk(CHUNK_TOOLS_DATA + c.id(), &tool_stream(state, c)?);
    }
    Ok(w.into_bytes())
}

/// `IsNeedSave`.
#[must_use]
pub fn needs_save(state: &SceneState, class: ObjClass) -> bool {
    match class {
        ObjClass::Glow
        | ObjClass::Light
        | ObjClass::Portal
        | ObjClass::Sector
        | ObjClass::SpawnPoint
        | ObjClass::SceneObject => true,
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => state.bulk.needs_save(class),
        _ => state.count(class) > 0,
    }
}

/// A fresh GUID for a new scene (`xrGUID::create`).
#[must_use]
pub fn new_guid(seed: u64) -> [u64; 2] {
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let t =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    [mix(t ^ seed), mix(t.rotate_left(17) ^ seed.wrapping_add(0x9E37_79B9_7F4A_7C15))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::objects::{GroupData, ShapeData, ShapeDef, default_data};

    fn scene() -> SceneState {
        let mut s = SceneState::default();
        s.meta.guid = [1, 2];
        s.meta.owner = "author".into();
        for class in ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool() && *c != ObjClass::Group) {
            let mut o = Object::new(class, &format!("{}_a", class.name()), default_data(class));
            o.xform.position = Vec3::new(class.id() as f32, 0.0, 0.0);
            if let ObjData::Shape(sh) = &mut o.data {
                *sh = ShapeData::unit_box();
                sh.shapes.push(ShapeDef::sphere(Vec3::ZERO, 1.0));
            }
            s.insert(o);
        }
        let m = s.insert(Object::new(ObjClass::Glow, "member", default_data(ObjClass::Glow)));
        let mut g = Object::new(
            ObjClass::Group,
            "grp",
            ObjData::Group(GroupData { reference: "g\\one".into(), members: vec![m] }),
        );
        g.xform.position = Vec3::ONE;
        let gid = s.insert(g);
        let mut mo = (**s.get(m).unwrap()).clone();
        mo.owner = Some(gid);
        mo.flags.co |= co::IN_GROUP;
        s.replace(mo);
        s.tools.snap.push(s.find(ObjClass::SceneObject, "scene_object_a").unwrap().id);
        s
    }

    fn parts_of(files: &[SaveFile]) -> PartFiles {
        let mut p = PartFiles::default();
        for c in ObjClass::ALL {
            let mine: Vec<(PathBuf, Vec<u8>)> = files
                .iter()
                .filter(|f| {
                    f.path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{}.part", c.name())))
                })
                .filter_map(|f| f.bytes.clone().map(|b| (f.path.clone(), b)))
                .collect();
            p.files.push((c, mine));
        }
        p
    }

    #[test]
    fn a_scene_round_trips_through_its_files() {
        let s = scene();
        let file = Path::new("C:\\p\\rawdata\\levels\\test.level");
        let files = write_scene(&s, file).unwrap();
        let main = files.iter().find(|f| f.path == file).and_then(|f| f.bytes.clone()).unwrap();
        assert_eq!(scene_kind(&main), Ok(SceneKind::Ltx));
        let back = parse_scene(&main, file, &parts_of(&files), None).unwrap().state;
        assert_eq!(back.objects.len(), s.objects.len());
        for o in s.objects.values() {
            let b = back.find(o.class, &o.name).unwrap();
            assert_eq!(b.class, o.class);
            if o.class != ObjClass::SoundEnv {
                assert_eq!(
                    b.data.clone().without_members(),
                    o.data.clone().without_members(),
                    "{:?}",
                    o.class
                );
            }
        }
        let g = back.find(ObjClass::Group, "grp").unwrap();
        let ObjData::Group(gd) = &g.data else { panic!() };
        assert_eq!(gd.members.len(), 1);
        assert_eq!(back.get(gd.members[0]).unwrap().owner, Some(g.id));
        assert_eq!(back.tools.snap.len(), 1);
        // and writes the same files again
        let again = write_scene(&back, file).unwrap();
        for (a, b) in files.iter().zip(again.iter()) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.bytes, b.bytes, "{}", a.path.display());
        }
    }

    #[test]
    fn the_undo_stream_is_a_scene_too() {
        let s = scene();
        let bytes = write_undo_stream(&s, true).unwrap();
        assert_eq!(scene_kind(&bytes), Ok(SceneKind::Binary));
        let back = parse_scene(&bytes, Path::new("x.level"), &PartFiles::default(), None).unwrap().state;
        assert_eq!(back.objects.len(), s.objects.len());
        assert_eq!(write_undo_stream(&back, true).unwrap(), bytes);
    }

    #[test]
    fn unknown_sections_and_chunks_are_kept() {
        let s = scene();
        let file = Path::new("t.level");
        let mut files = write_scene(&s, file).unwrap();
        // a section of the future in a part, and one in the main file
        for f in &mut files {
            if f.path.file_name().is_some_and(|n| n == "light.part")
                && let Some(b) = &mut f.bytes
            {
                b.extend_from_slice(b"[future]\r\n        key = 1\r\n");
            }
            if f.path == file
                && let Some(b) = &mut f.bytes
            {
                b.extend_from_slice(b"[later]\r\n        x = y\r\n");
            }
        }
        let main = files.iter().find(|f| f.path == file).and_then(|f| f.bytes.clone()).unwrap();
        let back = parse_scene(&main, file, &parts_of(&files), None).unwrap().state;
        let again = write_scene(&back, file).unwrap();
        let light = again.iter().find(|f| f.path.file_name().is_some_and(|n| n == "light.part")).unwrap();
        assert!(String::from_utf8_lossy(light.bytes.as_ref().unwrap()).contains("[future]"));
        let main2 = again.iter().find(|f| f.path == file).unwrap();
        assert!(String::from_utf8_lossy(main2.bytes.as_ref().unwrap()).contains("[later]"));

        // a binary scene with a chunk of a tool's stream nobody knows
        let mut stream = tool_stream(&s, ObjClass::Glow).unwrap();
        stream.extend_from_slice(&[0x99, 0x55, 0, 0, 2, 0, 0, 0, 7, 7]);
        let mut w = Writer::new();
        w.chunk(CHUNK_VERSION, |w| w.w_u32(CURRENT_FILE_VERSION));
        w.chunk(CHUNK_LEVELOP, |w| s.meta.level.write_bin(w));
        w.w_chunk(CHUNK_TOOLS_GUID, &guid_bytes(s.meta.guid));
        w.w_chunk(CHUNK_TOOLS_DATA + ObjClass::Glow.id(), &stream);
        let bytes = w.into_bytes();
        let back = parse_scene(&bytes, Path::new("b.level"), &PartFiles::default(), None).unwrap().state;
        assert_eq!(back.tools.part(ObjClass::Glow).kept_chunks.len(), 1);
        assert!(tool_stream(&back, ObjClass::Glow).unwrap().ends_with(&[0x99, 0x55, 0, 0, 2, 0, 0, 0, 7, 7]));
    }

    #[test]
    fn not_a_scene_says_why() {
        assert_eq!(scene_kind(b""), Err("the file is empty".into()));
        assert_eq!(scene_kind(b"[other]\r\nx=1"), Err("not a level file (no [version] section)".into()));
        assert_eq!(scene_kind(b"[version]\r\nvalue = 4\r\n"), Err("unsupported level version".into()));
        assert_eq!(scene_kind(b"\x01\x02\x03"), Err("not a level file (no version chunk)".into()));
    }
}

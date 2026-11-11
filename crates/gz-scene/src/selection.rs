//! The selection as a file and on the clipboard (`EScene::SaveSelection` / `LoadSelection`,
//! `CopySelection` / `PasteSelection` / `CutSelection`).
//!
//! A selection file is a chunk stream: the version chunk, then one chunk per tool
//! (`CHUNK_TOOLS_DATA + class`) holding what the tool's `SaveSelection` writes - for an object
//! tool its version, its selected objects and their count, for the AI map, the detail objects
//! and the wallmarks their whole stream. Older files may also hold a list of objects of any class
//! (`CHUNK_OBJECT_LIST`). The clipboard carries the C++ editor's own record
//! (`CF_XRAY_CLASS_LIST`: the class filter and the path of a selection file), so a copy made in
//! either editor pastes in the other.

use std::path::{Path, PathBuf};

use gz_chunk::{Reader, Writer};
use gz_entities::Settings;

use crate::SceneError;
use crate::class::ObjClass;
use crate::io::{CHUNK_TOOLS_DATA, CURRENT_FILE_VERSION, needs_save, tool_stream};
use crate::model::SceneState;
use crate::objects::{self, Loaded, ReadCx};

const CHUNK_VERSION: u32 = 0x9df3;
const CHUNK_OBJECT_LIST: u32 = 0x7708;
const TOOL_CHUNK_VERSION: u32 = 0x1001;
const TOOL_CHUNK_OBJECT_COUNT: u32 = 2;
const TOOL_CHUNK_OBJECTS: u32 = 3;

/// The classes in the order the C++ keeps its tools (a map by class id).
fn classes_by_id() -> Vec<ObjClass> {
    let mut all = ObjClass::ALL.to_vec();
    all.sort_by_key(|c| c.id());
    all
}

/// One tool's share of a selection file.
#[derive(Clone, Debug)]
pub enum ToolSelection {
    /// The objects of an object tool, as read.
    Objects(Vec<Loaded>),
    /// The whole stream of the AI map, the detail objects or the wallmarks.
    Bulk(Vec<u8>),
}

/// A selection file as read.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// `CHUNK_OBJECT_LIST` of an older file: added as the author's and selected.
    pub legacy: Vec<Loaded>,
    pub tools: Vec<(ObjClass, ToolSelection)>,
}

impl Selection {
    /// How many objects it adds.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.legacy.len()
            + self
                .tools
                .iter()
                .map(|(_, t)| match t {
                    ToolSelection::Objects(o) => o.len(),
                    ToolSelection::Bulk(_) => 0,
                })
                .sum::<usize>()
    }
}

/// `ESceneXxxTool::SaveSelection`: an object tool's selected objects; a bulk tool's stream.
fn tool_selection(state: &SceneState, class: ObjClass) -> Result<Vec<u8>, SceneError> {
    if !class.is_object_tool() {
        return tool_stream(state, class);
    }
    let mut w = Writer::new();
    w.w_chunk(TOOL_CHUNK_VERSION, &0u16.to_le_bytes());
    let objs: Vec<_> = state.of_class(class).filter(|o| state.is_selected(o.id)).collect();
    w.open_chunk(TOOL_CHUNK_OBJECTS);
    objects::write_stream_list(&mut w, objs.iter().map(|o| o.as_ref()), state, 0)?;
    w.close_chunk();
    w.w_chunk(TOOL_CHUNK_OBJECT_COUNT, &i32::try_from(objs.len()).unwrap_or(i32::MAX).to_le_bytes());
    Ok(w.into_bytes())
}

/// `EScene::SaveSelection(classfilter)`: `None` (`OBJCLASS_DUMMY`) writes every tool that has
/// something to save, a class only that tool.
pub fn write_selection(state: &SceneState, class: Option<ObjClass>) -> Result<Vec<u8>, SceneError> {
    let mut w = Writer::new();
    w.chunk(CHUNK_VERSION, |w| w.w_u32(CURRENT_FILE_VERSION));
    let classes = match class {
        Some(c) => vec![c],
        None => classes_by_id().into_iter().filter(|c| needs_save(state, *c)).collect(),
    };
    for c in classes {
        w.w_chunk(CHUNK_TOOLS_DATA + c.id(), &tool_selection(state, c)?);
    }
    Ok(w.into_bytes())
}

/// `ESceneCustomOTool::LoadSelection`: the version, the count and the objects, all of them or
/// none.
fn read_tool_objects(
    class: ObjClass,
    r: &Reader<'_>,
    settings: Option<&dyn Settings>,
    warnings: &mut Vec<String>,
) -> Option<Vec<Loaded>> {
    let version = match r.find_chunk(TOOL_CHUNK_VERSION) {
        Some((b, false)) if b.len() >= 2 => u16::from_le_bytes([b[0], b[1]]),
        _ => {
            warnings.push(format!("{} tools: the selection holds no version.", class.desc()));
            return None;
        }
    };
    if version != 0 {
        warnings.push(format!("{} tools: Unsupported version.", class.desc()));
        return None;
    }
    let count = match r.find_chunk(TOOL_CHUNK_OBJECT_COUNT) {
        Some((b, false)) if b.len() == 4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => return None,
    };
    if count < 0 {
        return None;
    }
    let list = match r.open_chunk(TOOL_CHUNK_OBJECTS) {
        Ok(Some(l)) => l,
        _ if count == 0 => return Some(Vec::new()),
        _ => return None,
    };
    let mut cx = ReadCx::new(settings);
    let read = objects::read_stream_list(&list.reader(), &mut cx);
    warnings.append(&mut cx.warnings);
    match read {
        Ok(objs) if objs.len() == count as usize => Some(objs),
        _ => None,
    }
}

/// `EScene::LoadSelection`: what a selection file adds. The C++ texts for a file it refuses.
pub fn read_selection(
    bytes: &[u8],
    settings: Option<&dyn Settings>,
) -> Result<(Selection, Vec<String>), SceneError> {
    let r = Reader::new(bytes);
    let version = match r.find_chunk(CHUNK_VERSION) {
        Some((b, false)) if b.len() == 4 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => return Err(SceneError::load("EScene: unsupported file version. Can't load Level.")),
    };
    if version != CURRENT_FILE_VERSION {
        return Err(SceneError::load("EScene: unsupported file version. Can't load Level."));
    }
    let mut warnings = Vec::new();
    let mut out = Selection::default();
    if let Some(list) = r.open_chunk(CHUNK_OBJECT_LIST)? {
        let mut cx = ReadCx::new(settings);
        match objects::read_stream_list(&list.reader(), &mut cx) {
            Ok(objs) => out.legacy = objs,
            Err(_) => warnings.push("EScene. Failed to load selection.".into()),
        }
        warnings.append(&mut cx.warnings);
    }
    for c in classes_by_id() {
        let Some(chunk) = r.open_chunk(CHUNK_TOOLS_DATA + c.id())? else { continue };
        if !c.is_object_tool() {
            out.tools.push((c, ToolSelection::Bulk(chunk.into_owned())));
            continue;
        }
        match read_tool_objects(c, &chunk.reader(), settings, &mut warnings) {
            Some(objs) => out.tools.push((c, ToolSelection::Objects(objs))),
            None => warnings.push(format!("EScene. Failed to load selection ({}).", c.desc())),
        }
    }
    Ok((out, warnings))
}

/// The clipboard format the C++ editor registers.
pub const CLIPBOARD_FORMAT: &str = "CF_XRAY_CLASS_LIST";
/// `MAX_PATH`: the file name's field in the record.
const CLIP_PATH: usize = 260;

/// `SceneClipData` (packed): the class filter, then the selection file's path, zero padded.
#[must_use]
pub fn clip_record(class: Option<ObjClass>, file: &Path) -> Vec<u8> {
    // OBJCLASS_DUMMY is -1
    let filter: i32 = class.map_or(-1, |c| i32::try_from(c.id()).unwrap_or(-1));
    let mut out = Vec::with_capacity(4 + CLIP_PATH);
    out.extend_from_slice(&filter.to_le_bytes());
    let path = file.to_string_lossy();
    let bytes = path.as_bytes();
    let n = bytes.len().min(CLIP_PATH - 1);
    out.extend_from_slice(&bytes[..n]);
    out.resize(4 + CLIP_PATH, 0);
    out
}

/// The selection file a clipboard record names.
#[must_use]
pub fn clip_file(record: &[u8]) -> Option<PathBuf> {
    let name = record.get(4..)?;
    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
    let text = String::from_utf8_lossy(&name[..end]).into_owned();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// The clipboard's record of a copy, when it holds one.
pub fn clipboard_get() -> Result<Vec<u8>, String> {
    let format = clipboard_win::register_format(CLIPBOARD_FORMAT)
        .ok_or_else(|| "Failed to open clipboard".to_owned())?
        .get();
    let _clip = clipboard_win::Clipboard::new_attempts(10).map_err(|_| "Failed to open clipboard".to_owned())?;
    let mut out = Vec::new();
    match clipboard_win::raw::get_vec(format, &mut out) {
        Ok(_) if !out.is_empty() => Ok(out),
        _ => Err("No data in clipboard".to_owned()),
    }
}

/// Puts a record on the clipboard.
pub fn clipboard_set(record: &[u8]) -> Result<(), String> {
    let format = clipboard_win::register_format(CLIPBOARD_FORMAT)
        .ok_or_else(|| "Failed to open clipboard".to_owned())?
        .get();
    let _clip = clipboard_win::Clipboard::new_attempts(10).map_err(|_| "Failed to open clipboard".to_owned())?;
    clipboard_win::raw::set(format, record).map_err(|_| "Failed to open clipboard".to_owned())
}

/// `GetTempFileName(<temp>, "clip", 0, ..)`: a new file `clipXXXX.tmp` in `dir`, created empty.
pub fn new_clip_file(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_millis() as u32);
    for i in 0..0xFFFFu32 {
        let n = (seed.wrapping_add(i) & 0xFFFF).max(1);
        let p = dir.join(format!("clip{n:X}.tmp"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temporary file name"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObjData, Object};
    use crate::objects::GlowData;

    #[test]
    fn a_selection_reads_back_what_was_selected() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::Glow, "a", ObjData::Glow(GlowData::default())));
        s.insert(Object::new(ObjClass::Glow, "b", ObjData::Glow(GlowData::default())));
        s.selection.insert(a);
        let bytes = write_selection(&s, Some(ObjClass::Glow)).unwrap();
        let (sel, warnings) = read_selection(&bytes, None).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(sel.object_count(), 1);
        let ToolSelection::Objects(o) = &sel.tools[0].1 else { panic!() };
        assert_eq!(o[0].obj.name, "a");
        // and the same bytes again from what was read
        let mut back = SceneState::default();
        let id = crate::io::insert_loaded(&mut back, o[0].clone());
        back.selection.insert(id);
        assert_eq!(write_selection(&back, Some(ObjClass::Glow)).unwrap(), bytes);
    }

    #[test]
    fn the_clip_record_is_the_cpp_struct() {
        let r = clip_record(Some(ObjClass::Glow), Path::new("C:\\t\\clip1.tmp"));
        assert_eq!(r.len(), 264);
        assert_eq!(clip_file(&r), Some(PathBuf::from("C:\\t\\clip1.tmp")));
    }
}

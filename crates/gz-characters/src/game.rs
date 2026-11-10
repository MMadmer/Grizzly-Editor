//! What validation reads of the linked game: the trade categories, a model's bones and the
//! bones a stalker's damage section names. Pure functions over bytes and configs.

use gz_chunk::Reader;
use gz_ltx::Ltx;

/// The game file the trade categories come from.
pub const TRADE_PRESETS: &str = "configs\\misc\\trade\\trade_presets.ltx";

/// The trade categories of `trade_presets.ltx`: every section that lists items, `unused` left
/// out (nobody trades in those). A section that only inherits (`[medic]:unused,trash,...`) is a
/// recipe made of categories, not one itself (`ReadPresets`).
#[must_use]
pub fn trade_categories(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut listed = false;
    let flush = |current: &str, listed: &mut bool, out: &mut Vec<String>| {
        if *listed && current != "unused" {
            out.push(current.to_owned());
        }
        *listed = false;
    };
    for raw in text.split('\n') {
        let mut line = raw.trim_matches([' ', '\t', '\r', '\n']);
        if let Some(sc) = line.find(';') {
            line = line.get(..sc).unwrap_or("").trim_matches([' ', '\t', '\r', '\n']);
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            flush(&current, &mut listed, &mut out);
            let close = rest.find(']');
            let derived = close.is_some_and(|c| rest.get(c..).is_some_and(|tail| tail.contains(':')));
            current = if derived { String::new() } else { close.map_or(rest, |c| rest.get(..c).unwrap_or("")).to_owned() };
            continue;
        }
        if !current.is_empty() {
            listed = true;
        }
    }
    flush(&current, &mut listed, &mut out);
    out
}

/// The `OGF_S_BONE_NAMES` chunk id.
const BONE_NAMES: u32 = gz_fmt_ogf::chunk::S_BONE_NAMES;

/// The bone names of a skeletal model, in the file's order; `None` when it carries none or the
/// chunk does not read (`OgfBoneNames`). Only the top-level chunk list and that one chunk are
/// looked at: nothing else of the model is decoded.
#[must_use]
pub fn ogf_bone_names(data: &[u8]) -> Option<Vec<String>> {
    let (body, _) = Reader::new(data).find_chunk(BONE_NAMES)?;
    // a count, then per bone its name, its parent's name and an obb of 15 floats
    let count = u32::from_le_bytes(body.get(..4)?.try_into().ok()?);
    let mut p = 4usize;
    let mut out = Vec::new();
    for _ in 0..count {
        let name_end = p + body.get(p..)?.iter().position(|&b| b == 0)?;
        out.push(gz_base::text::cp1251_to_utf8(body.get(p..name_end)?).into_owned());
        p = name_end + 1;
        let parent_end = p + body.get(p..)?.iter().position(|&b| b == 0)?;
        p = parent_end + 1 + 60;
        if p > body.len() {
            return None;
        }
    }
    Some(out)
}

/// The bones the damage section of a stalker names: the engine stops on the first one a model
/// lacks (`CDamageManager::load_section`). Every character's spawn section is `:stalker`.
#[must_use]
pub fn stalker_damage_bones(ltx: &Ltx) -> Vec<String> {
    let Ok(sect) = ltx.r_string("stalker", "damage") else { return Vec::new() };
    let Some(section) = ltx.section(&sect.trim().to_ascii_lowercase()) else { return Vec::new() };
    section.items.iter().filter(|i| &*i.key != "default").map(|i| i.key.to_string()).collect()
}

/// `meshes\<visual>`, `.ogf` added when the visual has no extension - where the linked game
/// keeps an actor's model.
#[must_use]
pub fn model_path(visual: &str) -> String {
    let mut rel = format!("meshes\\{visual}");
    let leaf = rel.rsplit(['\\', '/']).next().unwrap_or("");
    if !leaf.contains('.') {
        rel.push_str(".ogf");
    }
    rel
}

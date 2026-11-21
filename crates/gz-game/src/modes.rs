//! The game modes a module can target, read out of the linked game.
//!
//! Dead Air has no config listing its modes: a mode is a checkbox the new-game script creates
//! (`scripts\ui_mm_faction_select.script`), laid out in `configs\ui\ui_mm_faction_select_16.xml`
//! and stored as a `[character_creation]` key ending with `_mode`. What is no mode the game says
//! itself: the checkboxes `[modifiers]` of `configs\dead_air_x64_mode_exclusive.ltx` names (hardcore,
//! easy, ...) are modifiers, the rule the engine applies when a new game starts. A global mod replaces
//! the screen and that file, so the list is scanned, never hardcoded. Modules add theirs with
//! `[provides_mode]`.
//!
//! The reading is a text scan, byte for byte, quirks included.

use gz_base::text::cp1251_to_utf8;
use gz_vfs::{GameFiles, ModuleRoot};

/// A mode a module can be built for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameMode {
    /// What modules name it by: `new_game_metro_mode` -> `metro`.
    pub id: String,
    /// What the player reads (UTF-8); the id when no string table names it.
    pub caption: String,
    /// The `[character_creation]` key it comes from (empty for a module's mode).
    pub key: String,
    /// `game`, or the folder of the module declaring it.
    pub source: String,
}

/// What the scan found, and why it could not read everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeScan {
    pub modes: Vec<GameMode>,
    pub error: Option<String>,
}

pub const LAYOUT_16: &str = "configs/ui/ui_mm_faction_select_16.xml";
pub const LAYOUT: &str = "configs/ui/ui_mm_faction_select.xml";
pub const SCRIPT: &str = "scripts/ui_mm_faction_select.script";
/// The game's word about its new-game screen: `[modifiers]` are no modes.
pub const MODIFIERS: &str = "configs/dead_air_x64_mode_exclusive.ltx";
pub const NO_LAYOUT: &str =
    "the linked game has no ui_mm_faction_select xml - cannot tell modes from options";

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= hay.len()).then_some(from);
    }
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| from + i)
}

fn find_byte(hay: &[u8], b: u8, from: usize) -> Option<usize> {
    hay.get(from..)?.iter().position(|&c| c == b).map(|i| from + i)
}

/// `rfind(b, at)`: the last `b` at or before `at`.
fn rfind_byte(hay: &[u8], b: u8, at: usize) -> Option<usize> {
    let end = at.saturating_add(1).min(hay.len());
    hay.get(..end)?.iter().rposition(|&c| c == b)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle, 0).is_some()
}

/// The checkboxes `[modifiers]` of the game's screen config names, lower case: `check_hardcore`.
#[must_use]
pub fn modifiers(config: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in config.split(|c| *c == b'\n') {
        let line = line.split(|c| *c == b';').next().unwrap_or(&[]);
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            inside = line.eq_ignore_ascii_case("[modifiers]");
            continue;
        }
        if inside {
            let name = line.split(|c: char| c == '=' || c.is_whitespace()).next().unwrap_or("");
            if !name.is_empty() {
                out.push(name.to_ascii_lowercase());
            }
        }
    }
    out
}

/// `new_game_metro_mode` -> `metro`, `new_game_good_loot` -> `good_loot`.
#[must_use]
pub fn normalize_id(config_key: &str) -> String {
    let mut id = config_key;
    if id.get(..9).is_some_and(|h| h.eq_ignore_ascii_case("new_game_")) {
        id = id.get(9..).unwrap_or("");
    }
    let n = id.len();
    if n > 5 && id.get(n - 5..).is_some_and(|t| t.eq_ignore_ascii_case("_mode")) {
        id = id.get(..n - 5).unwrap_or(id);
    }
    id.to_owned()
}

/// Whether `at` lies inside a Lua block comment: Dead Air's story and azazel checkboxes sit in
/// `--[[ ... --]]` with the call itself unprefixed.
fn inside_block_comment(script: &[u8], at: usize) -> bool {
    let byte = |i: usize| script.get(i).copied().unwrap_or(0);
    let mut block = false;
    let mut i = 0;
    while i + 1 < at {
        if block {
            if byte(i) == b']' && byte(i + 1) == b']' {
                block = false;
                i += 1;
            }
            i += 1;
            continue;
        }
        if byte(i) != b'-' || byte(i + 1) != b'-' {
            i += 1;
            continue;
        }
        if i + 3 < at && byte(i + 2) == b'[' && byte(i + 3) == b'[' {
            block = true;
            i += 3;
        } else {
            match find_byte(script, b'\n', i) {
                Some(eol) if eol < at => i = eol,
                _ => return block,
            }
        }
        i += 1;
    }
    block
}

/// A checkbox is real only when the script creates it outside a comment.
fn checkbox_live(script: &[u8], node: &[u8]) -> bool {
    let mut needle = b"main_dialog:".to_vec();
    needle.extend_from_slice(node);
    let mut at = 0;
    while let Some(found) = find(script, &needle, at) {
        let bol = rfind_byte(script, b'\n', found).map_or(0, |p| p + 1);
        let line = script.get(bol..found).unwrap_or(&[]);
        let commented = line
            .iter()
            .position(|c| *c != b' ' && *c != b'\t')
            .is_some_and(|first| line.get(first..).is_some_and(|l| l.starts_with(b"--")));
        if !commented && contains(line, b"InitCheck") && !inside_block_comment(script, found) {
            return true;
        }
        at = found + needle.len();
    }
    false
}

/// A checkbox of the layout, before its caption is looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkbox {
    pub node: String,
    pub key: String,
    pub id: String,
    /// The string id of its caption, as the layout holds it (bytes, usually ASCII).
    pub caption_id: Vec<u8>,
    /// A mode, not an option: its key ends with `_mode` and the game names it no modifier.
    pub campaign: bool,
    /// The script really creates it.
    pub available: bool,
}

/// Every `<check_...>` of the layout, campaign or option; `modifiers` are what [`modifiers`] read.
#[must_use]
pub fn checkboxes(ui: &[u8], script: &[u8], modifiers: &[String]) -> Vec<Checkbox> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(at) = find(ui, b"<check_", pos) {
        let name_at = at + 1;
        let mut name_end = name_at;
        while ui.get(name_end).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            name_end += 1;
        }
        let node = ui.get(name_at..name_end).unwrap_or(&[]);
        pos = name_end;

        // check_metro_mode pairs with new_game_metro_mode, check_survival with
        // new_game_survival_mode: whichever spelling the script uses
        let node_text = String::from_utf8_lossy(node).into_owned();
        let base = format!("new_game_{}", node_text.get(6..).unwrap_or(""));
        let with = format!("{base}_mode");
        let key = if !script.is_empty() && contains(script, with.as_bytes()) { with } else { base };
        let id = normalize_id(&key);
        let available = script.is_empty() || checkbox_live(script, node);
        let mode_key = key.len() > 5 && key.get(key.len() - 5..).is_some_and(|t| t.eq_ignore_ascii_case("_mode"));
        let campaign = mode_key && !modifiers.iter().any(|m| m.eq_ignore_ascii_case(&node_text));

        // the caption lives on the sibling label node, as a string-table id
        let mut cap_tag = b"<cap_".to_vec();
        cap_tag.extend_from_slice(node);
        let caption_id = find(ui, &cap_tag, 0)
            .and_then(|cap_at| find_byte(ui, b'>', cap_at))
            .and_then(|open| find(ui, b"<text", open))
            .and_then(|t| find_byte(ui, b'>', t))
            .and_then(|t_open| find(ui, b"</text>", t_open).and_then(|t_end| ui.get(t_open + 1..t_end)))
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        out.push(Checkbox { node: node_text, key, id, caption_id, campaign, available });
    }
    out
}

/// One string table file's say on the captions still open: `id="<sid>"`, then the text
/// between the next `<text>` and `</text>`, as written.
fn resolve_in(text: &[u8], ids: &[Vec<u8>], captions: &mut [String]) {
    for (sid, caption) in ids.iter().zip(captions.iter_mut()) {
        // nothing to look up, or done (an empty text found is looked for again)
        if sid.is_empty() || !caption.is_empty() {
            continue;
        }
        let mut pat = b"id=\"".to_vec();
        pat.extend_from_slice(sid);
        pat.push(b'"');
        let Some(at) = find(text, &pat, 0) else { continue };
        let Some(open) = find(text, b"<text>", at) else { continue };
        let Some(end) = find(text, b"</text>", open) else { continue };
        *caption = cp1251_to_utf8(text.get(open + 6..end).unwrap_or(&[])).into_owned();
    }
}

/// Valid UTF-8 as it is, anything else read as cp1251.
fn manifest_text(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_owned(),
        Err(_) => cp1251_to_utf8(raw).into_owned(),
    }
}

fn trim_end<'a>(mut s: &'a [u8], set: &[u8]) -> &'a [u8] {
    while let Some((last, rest)) = s.split_last() {
        if !set.contains(last) {
            break;
        }
        s = rest;
    }
    s
}

fn trim_start<'a>(mut s: &'a [u8], set: &[u8]) -> &'a [u8] {
    while let Some((first, rest)) = s.split_first() {
        if !set.contains(first) {
            break;
        }
        s = rest;
    }
    s
}

/// The `[provides_mode]` entries of one module's `mod.ltx`: `id` opens an entry, `title` names
/// it. The scan stops at the next line holding a `[`.
#[must_use]
pub fn provides_mode(manifest: &[u8], module: &str) -> Vec<GameMode> {
    // the file is read as a C string
    let text = manifest.split(|&b| b == 0).next().unwrap_or(&[]);
    let mut out = Vec::new();
    let Some(sect) = find(text, b"[provides_mode]", 0) else { return out };
    let mut pending: Option<GameMode> = None;
    let mut pos = sect;
    while pos < text.len() {
        let eol = find_byte(text, b'\n', pos);
        let line = text.get(pos..eol.unwrap_or(text.len())).unwrap_or(&[]);
        pos = eol.map_or(text.len(), |e| e + 1);
        if contains(line, b"[") && !contains(line, b"[provides_mode]") {
            break;
        }
        let Some(eq) = find_byte(line, b'=', 0) else { continue };
        let key = trim_end(line.get(..eq).unwrap_or(&[]), b" \t");
        let val = trim_end(trim_start(line.get(eq + 1..).unwrap_or(&[]), b" \t"), b" \t\r");
        if key.eq_ignore_ascii_case(b"id") {
            out.extend(pending.take().filter(|p| !p.id.is_empty()));
            let id = manifest_text(val);
            pending =
                Some(GameMode { caption: id.clone(), id, key: String::new(), source: module.to_owned() });
        } else if key.eq_ignore_ascii_case(b"title")
            && let Some(p) = pending.as_mut().filter(|p| !p.id.is_empty())
        {
            p.caption = manifest_text(val);
        }
    }
    out.extend(pending.filter(|p| !p.id.is_empty()));
    out
}

/// Appends the modules' modes, `modules\` first: one module in both roots, or two modules
/// naming one id, declare it once.
pub fn add_module_modes(out: &mut Vec<GameMode>, roots: [Vec<GameMode>; 2]) {
    let was = out.len();
    for found in roots {
        for m in found {
            let dup = out.get(was..).unwrap_or(&[]).iter().any(|o| o.id.eq_ignore_ascii_case(&m.id));
            if !dup {
                out.push(m);
            }
        }
    }
}

/// The `[provides_mode]` modes of every module installed in the game. Blocking.
#[must_use]
pub fn module_modes(files: &GameFiles) -> [Vec<GameMode>; 2] {
    ModuleRoot::ALL.map(|root| {
        let mut found = Vec::new();
        for m in files.modules().iter().filter(|m| m.root == root) {
            // the manifest is read from the disk, files up to 1 MB
            let Some(size) = m.get("mod.ltx").map(|f| f.size) else { continue };
            if size == 0 || size >= 1 << 20 {
                continue;
            }
            let Ok(bytes) = m.read("mod.ltx") else { continue };
            found.extend(provides_mode(&bytes, &m.id));
        }
        found
    })
}

/// The text of a game file; empty when it is missing or cannot be read.
fn read_text(files: &GameFiles, path: &str) -> Vec<u8> {
    files.read(path).unwrap_or_default()
}

/// How many string table files are read at once while captions are still open.
const CAPTION_BATCH: usize = 32;

/// Looks the captions up in every `configs\text\**.xml` of the game, in path order, the first
/// file that names an id winning; a caption nobody defines stays the mode's id.
fn resolve_captions(files: &GameFiles, boxes: &[Checkbox]) -> Vec<String> {
    let ids: Vec<Vec<u8>> = boxes.iter().map(|b| b.caption_id.clone()).collect();
    let mut captions = vec![String::new(); boxes.len()];
    if ids.iter().any(|i| !i.is_empty()) {
        let tables: Vec<&str> = files.find("configs/text", "xml").into_iter().map(|f| f.path).collect();
        for batch in tables.chunks(CAPTION_BATCH) {
            let open = ids.iter().zip(&captions).any(|(sid, c)| !sid.is_empty() && c.is_empty());
            if !open {
                break;
            }
            for text in files.read_many(batch).into_iter().flatten() {
                resolve_in(&text, &ids, &mut captions);
            }
        }
    }
    for (caption, b) in captions.iter_mut().zip(boxes) {
        if caption.is_empty() {
            caption.clone_from(&b.id);
        }
    }
    captions
}

/// Scans the modes over a mounted game. Blocking; parallel reads on the caller's pool.
#[must_use]
pub fn scan(files: &GameFiles) -> ModeScan {
    // the 16:9 layout is what the game uses; the 4:3 one is a fallback
    let mut ui = read_text(files, LAYOUT_16);
    if ui.is_empty() {
        ui = read_text(files, LAYOUT);
    }
    let script = read_text(files, SCRIPT);
    let mut modes = Vec::new();
    if ui.is_empty() {
        add_module_modes(&mut modes, module_modes(files));
        return ModeScan { modes, error: Some(NO_LAYOUT.to_owned()) };
    }
    let boxes = checkboxes(&ui, &script, &modifiers(&read_text(files, MODIFIERS)));
    let captions = resolve_captions(files, &boxes);
    // options are parsed only to be told apart; they never reach a caller
    for (b, caption) in boxes.into_iter().zip(captions) {
        if b.campaign && b.available {
            modes.push(GameMode { id: b.id, caption, key: b.key, source: "game".to_owned() });
        }
    }
    add_module_modes(&mut modes, module_modes(files));
    ModeScan { modes, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: &str = r#"<w>
  <check_hard_mode x="410" y="70"/>
  <check_metro_mode x="50" y="40" max="9" width="20"/>
  <cap_check_metro_mode x="80" y="40"><text>st_cap_check_metro_mode</text></cap_check_metro_mode>
  <check_survival x="60" y="70"/>
  <cap_check_survival><text font="x">st_cap_survival</text></cap_check_survival>
  <check_one_life x="410" y="40"/>
  <cap_check_one_life><text>st_one_life</text></cap_check_one_life>
  <check_azazel x="45" y="100"/>
  <check_story x="45" y="130"/>
  <check_nocap item_x="10" x="44"/>
</w>"#;

    const SCRIPT: &str = "function x()\n\
  self.metro = main_dialog:InitCheck(\"check_metro_mode\") -- main_dialog:check_metro_mode\n\
  local s = xml:InitCheck(\"main_dialog:check_survival\", self) if new_game_survival_mode then end\n\
  self.one = xml:InitCheck(\"main_dialog:check_one_life\", self)\n\
  -- self.az = xml:InitCheck(\"main_dialog:check_azazel\", self)\n\
  --[[\n\
  self.story = xml:InitCheck(\"main_dialog:check_story\", self)\n\
  --]]\n\
  self.nc = xml:InitCheck(\"main_dialog:check_nocap\", self)\n\
  self.hard = xml:InitCheck(\"main_dialog:check_hard_mode\", self)\n\
end\n";

    #[test]
    fn keys_modifiers_scripts_and_captions() {
        let listed = modifiers(b"[exclusive_campaigns]\r\ncheck_survival\r\n\r\n; one life\r\n[modifiers]\r\nCheck_Hard_Mode ; hardcore\r\n");
        assert_eq!(listed, ["check_hard_mode"]);
        let boxes = checkboxes(UI.as_bytes(), SCRIPT.as_bytes(), &listed);
        let summary: Vec<(&str, &str, &str, bool, bool)> = boxes
            .iter()
            .map(|b| (b.node.as_str(), b.key.as_str(), b.id.as_str(), b.campaign, b.available))
            .collect();
        assert_eq!(
            summary,
            [
                ("check_hard_mode", "new_game_hard_mode", "hard", false, true),
                ("check_metro_mode", "new_game_metro_mode", "metro", true, true),
                ("check_survival", "new_game_survival_mode", "survival", true, true),
                ("check_one_life", "new_game_one_life", "one_life", false, true),
                ("check_azazel", "new_game_azazel", "azazel", false, false),
                ("check_story", "new_game_story", "story", false, false),
                ("check_nocap", "new_game_nocap", "nocap", false, true),
            ]
        );
        assert_eq!(boxes[1].caption_id, b"st_cap_check_metro_mode");
        assert_eq!(boxes[2].caption_id, b"st_cap_survival");
        assert!(boxes[6].caption_id.is_empty());
    }

    #[test]
    fn a_mode_key_is_a_mode_unless_the_game_names_it_a_modifier() {
        let boxes = checkboxes(b"<check_metro_mode x=\"5\"/>", b"", &[]);
        assert_eq!(boxes.len(), 1);
        assert!(boxes[0].campaign);
        assert!(boxes[0].available);
        assert_eq!(boxes[0].key, "new_game_metro_mode");
        assert!(!checkboxes(b"<check_metro_mode x=\"5\"/>", b"", &["check_metro_mode".to_owned()])[0].campaign);
    }

    #[test]
    fn ids() {
        assert_eq!(normalize_id("new_game_metro_mode"), "metro");
        assert_eq!(normalize_id("new_game_good_loot"), "good_loot");
        assert_eq!(normalize_id("NEW_GAME_x_MODE"), "x");
        assert_eq!(normalize_id("_mode"), "_mode");
    }

    #[test]
    fn captions_first_file_wins() {
        let ids = vec![b"st_a".to_vec(), b"st_b".to_vec(), Vec::new()];
        let mut caps = vec![String::new(); 3];
        resolve_in(
            b"<string id=\"st_a\"><text>\xc0\xe1</text></string><string id=\"st_b\"><text></text></string>",
            &ids,
            &mut caps,
        );
        resolve_in(
            b"<string id=\"st_a\"><text>later</text></string><string id=\"st_b\"><text> B </text></string>",
            &ids,
            &mut caps,
        );
        assert_eq!(caps, ["Аб", " B ", ""]);
    }

    #[test]
    fn module_manifests() {
        let text = b"[module]\r\nname = X\r\n[provides_mode]\r\nid = night\r\ntitle = \xcd\xee\xf7\xfc\r\n  id = skipped\r\nid=day\r\ntitle=Day \r\n[other]\r\nid = never\r\n";
        let modes = provides_mode(text, "mymod");
        let got: Vec<(&str, &str, &str)> =
            modes.iter().map(|m| (m.id.as_str(), m.caption.as_str(), m.source.as_str())).collect();
        assert_eq!(got, [("night", "Ночь", "mymod"), ("day", "Day", "mymod")]);
        assert!(provides_mode(b"[module]\nid = x\n", "m").is_empty());
        // an empty id opens nothing: its title goes nowhere
        let empty = provides_mode(b"[provides_mode]\nid =\ntitle = T\nid = a\n", "m");
        assert_eq!(
            empty.iter().map(|m| (m.id.as_str(), m.caption.as_str())).collect::<Vec<_>>(),
            [("a", "a")]
        );
        assert_eq!(
            provides_mode("[provides_mode]\nid = n\ntitle = Ночь UTF\n".as_bytes(), "m")[0].caption,
            "Ночь UTF"
        );
    }

    #[test]
    fn module_modes_once() {
        let m = |id: &str, src: &str| GameMode {
            id: id.into(),
            caption: id.into(),
            key: String::new(),
            source: src.into(),
        };
        let mut out = vec![m("metro", "game")];
        add_module_modes(
            &mut out,
            [vec![m("night", "a"), m("metro", "a"), m("Night", "b")], vec![m("night", "a"), m("day", "c")]],
        );
        let ids: Vec<(&str, &str)> = out.iter().map(|g| (g.id.as_str(), g.source.as_str())).collect();
        // a module's mode is compared with the other modules' only: the game's metro stays beside it
        assert_eq!(ids, [("metro", "game"), ("night", "a"), ("metro", "a"), ("day", "c")]);
    }

    #[test]
    fn block_comments() {
        let s = b"--[[ a ]] x --[[ b\n y";
        assert!(!inside_block_comment(s, 11));
        assert!(inside_block_comment(s, s.len()));
        assert!(!inside_block_comment(b"-- line\nz", 8));
    }
}

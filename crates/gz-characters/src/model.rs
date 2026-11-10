//! A character as plain data, read and written the C++ way (`EditorCharacters::Parse`,
//! `Load`, `Write`, `Template`).

use gz_luadata::{Table, Value, format_g, write_value};

/// One row of a starting kit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supply {
    pub section: String,
    /// How many; 0 means "drop what the base gave me", the only way to take one item out of an
    /// inherited kit.
    pub count: i32,
}

impl Supply {
    #[must_use]
    pub fn new(section: &str, count: i32) -> Self {
        Self { section: section.to_owned(), count }
    }
}

/// One line of a trader's stock: what is on the shelf after every restock, with its chance.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeRow {
    pub section: String,
    pub count: i32,
    /// (0, 1]
    pub chance: f32,
}

impl Default for TradeRow {
    fn default() -> Self {
        Self { section: String::new(), count: 1, chance: 1.0 }
    }
}

/// What a trader sells, buys and keeps in stock. The categories are the linked game's trade
/// presets, the prices fractions of an item's cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    /// The character says something about trade.
    pub declared: bool,
    /// `trade = false`: not a trader, whatever the base is.
    pub off: bool,
    pub sells: Vec<String>,
    pub buys: Vec<String>,
    pub stock: Vec<TradeRow>,
    /// Asks this much of an item's cost.
    pub markup: f32,
    /// Pays this much of it.
    pub pays: f32,
    /// Game hours between restocks.
    pub restock: i32,
}

impl Default for Trade {
    fn default() -> Self {
        Self { declared: false, off: false, sells: Vec::new(), buys: Vec::new(), stock: Vec::new(), markup: 1.0, pays: 0.3, restock: 24 }
    }
}

impl Trade {
    #[must_use]
    pub fn is_trader(&self) -> bool {
        self.declared && !self.off
    }
}

/// One character of the module. Empty means UNSET, not "the default": the defaults are applied
/// once, after the base chain is flattened.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    /// Where it was read from: project-relative with `/`, or the absolute path (`/`) of a file
    /// outside the project; empty for a new one.
    pub file: String,
    /// `[a-z0-9_]`, unique in the module.
    pub id: String,
    /// Another character of the project to start from.
    pub base: String,
    /// A template: never a character in the game.
    pub is_abstract: bool,
    /// A string, or a table keyed by language.
    pub name: Value,
    pub bio: Value,
    pub community: String,
    /// Meshes-relative, extension optional.
    pub visual: String,
    pub icon: String,
    pub start_dialog: String,
    pub spec_rank: String,
    pub rank: i32,
    pub reputation: i32,
    pub has_rank: bool,
    pub has_reputation: bool,
    pub has_money: bool,
    pub money_min: i32,
    pub money_max: i32,
    pub money_infinite: bool,
    pub supplies: Vec<Supply>,
    /// Set here as a whole, or the base's.
    pub trade: Trade,
    /// The one-man squad section is made (default true).
    pub squad: bool,
    /// `squad` is set here rather than inherited.
    pub has_squad: bool,
}

impl Default for Character {
    fn default() -> Self {
        Self {
            file: String::new(),
            id: String::new(),
            base: String::new(),
            is_abstract: false,
            name: Value::Nil,
            bio: Value::Nil,
            community: String::new(),
            visual: String::new(),
            icon: String::new(),
            start_dialog: String::new(),
            spec_rank: String::new(),
            rank: 0,
            reputation: 0,
            has_rank: false,
            has_reputation: false,
            has_money: false,
            money_min: 0,
            money_max: 0,
            money_infinite: false,
            supplies: Vec::new(),
            trade: Trade::default(),
            squad: true,
            has_squad: false,
        }
    }
}

// ---- texts ---------------------------------------------------------------------------------

fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r', '\n'])
}

/// The languages a text carries: `rus` for a plain string, every key with a string value of a
/// table (`TextLanguages`).
pub fn text_languages(v: &Value, langs: &mut Vec<String>) {
    match v {
        Value::String(_) => langs.push("rus".to_owned()),
        Value::Table(t) => langs.extend(t.fields.iter().filter(|(_, x)| x.is_string()).map(|(k, _)| k.to_string())),
        _ => {}
    }
}

/// A text in one language, falling back to `rus` and then to any language - what the game
/// shows an English install of a Russian-only character (`TextIn`).
#[must_use]
pub fn text_in(v: &Value, lang: &str) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Table(t) => {
            let pick = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
            pick(lang).or_else(|| pick("rus")).or_else(|| t.fields.iter().find_map(|(_, x)| x.as_str().map(str::to_owned))).unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// What is actually STORED for a language, without falling back: the form binds to this, or
/// "not set" and "set to the same text" would look alike (`TextRaw`).
#[must_use]
pub fn text_raw(v: &Value, lang: &str) -> String {
    match v {
        Value::String(s) if lang == "rus" => s.to_string(),
        Value::Table(t) => t.get(lang).and_then(Value::as_str).map(str::to_owned).unwrap_or_default(),
        _ => String::new(),
    }
}

/// No language holds more than blanks (`TextEmpty`).
#[must_use]
pub fn text_empty(v: &Value) -> bool {
    let mut langs = Vec::new();
    text_languages(v, &mut langs);
    langs.iter().all(|l| trim(&text_in(v, l)).is_empty())
}

/// Writes one language back without losing the others: a plain string stays one while only
/// `rus` is in play and becomes a table the moment a second language exists (`TextSet`).
pub fn text_set(v: &mut Value, lang: &str, text: &str) {
    if let Value::Table(t) = v {
        if text.is_empty() {
            t.erase(lang);
        } else {
            t.set(lang, text);
        }
        if t.fields.is_empty() {
            *v = Value::Nil;
        }
        return;
    }
    if lang == "rus" {
        *v = if text.is_empty() { Value::Nil } else { Value::from(text) };
        return;
    }
    // a second language arrives: what was there keeps its place as rus
    let had = v.as_str().map(str::to_owned).unwrap_or_default();
    let mut t = Table::new();
    if !had.is_empty() {
        t.set("rus", had.as_str());
    }
    if !text.is_empty() {
        t.set(lang, text);
    }
    *v = if t.fields.is_empty() { Value::Nil } else { Value::Table(t) };
}

/// `{ rus = "..." }` and `"..."` mean the same thing downstream, so the simple case stays simple
/// in the file (`TextCollapse`).
pub fn text_collapse(v: &mut Value) {
    let Value::Table(t) = v else { return };
    let [(key, Value::String(s))] = t.fields.as_slice() else { return };
    if key != "rus" {
        return;
    }
    *v = Value::String(s.clone());
}

// ---- ids -----------------------------------------------------------------------------------

/// `[a-z0-9_]`, not starting with a digit, up to 48 (`ValidId`).
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && !id.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

/// `<module>_<id>`: the name the game knows the character by - the profile, the spawn section,
/// the squad section's stem (`GameId`).
#[must_use]
pub fn game_id(module_id: &str, char_id: &str) -> String {
    if module_id.is_empty() { char_id.to_owned() } else { format!("{module_id}_{char_id}") }
}

/// A file name as an id: lower case, anything outside `[a-z0-9_]` an underscore
/// (`NqText::IdFromName`).
#[must_use]
pub fn id_from_name(name: &str) -> String {
    gz_nq::text::id_from_name(name)
}

// ---- reading -------------------------------------------------------------------------------

/// `int(double)`: toward zero (the C++ cast; out of range saturates here).
fn to_int(n: f64) -> i32 {
    n as i32
}

/// The names of `sells` / `buys`: one string or a list of strings, empty ones dropped.
fn names(list: Option<&Value>, out: &mut Vec<String>) {
    match list {
        None => {}
        Some(Value::String(s)) => {
            if !s.is_empty() {
                out.push(s.to_string());
            }
        }
        Some(v) => out.extend(v.arr().iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)),
    }
}

fn read_trade(t: &Value, trade: &mut Trade) {
    match t {
        Value::Bool(false) => {
            trade.declared = true;
            trade.off = true;
        }
        Value::Table(_) => {
            trade.declared = true;
            names(t.get("sells"), &mut trade.sells);
            names(t.get("buys"), &mut trade.buys);
            trade.markup = t.get_number("markup", 1.0) as f32;
            trade.pays = t.get_number("pays", 0.3) as f32;
            trade.restock = to_int(t.get_number("restock", 24.0));
            if let Some(s) = t.get("stock") {
                for row in s.arr() {
                    let mut r = TradeRow::default();
                    match row {
                        Value::String(sec) => r.section = sec.to_string(),
                        Value::Table(_) => {
                            r.section = row.get_string("section", "");
                            r.count = to_int(row.get_number("count", 1.0));
                            r.chance = row.get_number("chance", 1.0) as f32;
                        }
                        // oracle: any other row is kept as an empty one, which E118 then names
                        _ => {}
                    }
                    trade.stock.push(r);
                }
            }
        }
        _ => {}
    }
}

/// Reads the text of a character file (`EditorCharacters::Parse`). `chunk_name` is what LuaJIT
/// names the chunk in its messages: the file's base name. The error is the sentence the C++
/// gave: `line N: <message>` when the reader names a line.
pub fn parse(text: &str, chunk_name: &str) -> Result<Character, String> {
    let v = gz_luadata::parse_named(text, chunk_name).map_err(|e| if e.line != 0 { format!("line {}: {}", e.line, e.message) } else { e.message })?;
    if !v.is_table() {
        return Err("a character file must return one table".into());
    }
    let mut c = Character {
        id: v.get_string("id", ""),
        base: v.get_string("base", ""),
        is_abstract: v.get_bool("abstract", false),
        community: v.get_string("community", ""),
        visual: v.get_string("visual", ""),
        icon: v.get_string("icon", ""),
        start_dialog: v.get_string("start_dialog", ""),
        spec_rank: v.get_string("spec_rank", ""),
        ..Character::default()
    };
    if let Some(n) = v.get("name") {
        c.name = n.clone();
    }
    if let Some(b) = v.get("bio") {
        c.bio = b.clone();
    }
    if let Some(Value::Bool(b)) = v.get("squad") {
        c.squad = *b;
        c.has_squad = true;
    }
    if let Some(Value::Number(n)) = v.get("rank") {
        c.rank = to_int(*n);
        c.has_rank = true;
    }
    if let Some(Value::Number(n)) = v.get("reputation") {
        c.reputation = to_int(*n);
        c.has_reputation = true;
    }
    if let Some(m @ Value::Table(_)) = v.get("money") {
        c.has_money = true;
        c.money_min = to_int(m.get_number("min", 0.0));
        c.money_max = to_int(m.get_number("max", f64::from(c.money_min)));
        c.money_infinite = m.get_bool("infinite", false);
        if c.money_max < c.money_min {
            c.money_max = c.money_min;
        }
    }
    if let Some(s) = v.get("supplies") {
        for row in s.arr() {
            let mut item = Supply::new("", 1);
            match row {
                // a bare string is the one-item shorthand
                Value::String(sec) => item.section = sec.to_string(),
                Value::Table(_) => {
                    item.section = row.get_string("section", "");
                    item.count = to_int(row.get_number("count", 1.0));
                }
                _ => {}
            }
            // 0 is meaningful: it takes an inherited row away again
            item.count = item.count.max(0);
            if !item.section.is_empty() {
                c.supplies.push(item);
            }
        }
    }
    // trade = false says "not a trader" over whatever the base sells; a table is a shop
    if let Some(t) = v.get("trade") {
        read_trade(t, &mut c.trade);
    }
    Ok(c)
}

/// The base name of a path without its extension (`NqUtil::BaseName`).
#[must_use]
pub fn base_name(path: &str) -> &str {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match leaf.rfind('.') {
        Some(dot) if dot > 0 => leaf.get(..dot).unwrap_or(leaf),
        _ => leaf,
    }
}

/// A file's text as a character (`EditorCharacters::Load` after the read): `file` is what
/// [`Character::file`] says; the file name is the id when the file names none - a renamed file
/// renames the character, the rule quests follow.
pub fn load_text(text: &str, file: &str) -> Result<Character, String> {
    let stem = base_name(file);
    let mut c = parse(text, stem)?;
    file.clone_into(&mut c.file);
    if c.id.is_empty() {
        c.id = id_from_name(stem);
    }
    Ok(c)
}

/// Reads a character file as `NqLua::ReadFile` does: at most 16 MB, a UTF-8 BOM dropped; the
/// error texts are the C++ ones.
pub fn read_file(path: &std::path::Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "cannot open file".to_owned())?;
    if meta.len() > 16 << 20 {
        return Err("file too large".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "read error".to_owned())?;
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(body).into_owned())
}

// ---- writing -------------------------------------------------------------------------------

/// A Lua string literal, escaped the way the canonical writer escapes one.
fn lua_string(s: &str) -> String {
    write_value(&Value::from(s), 0)
}

/// `%g` of a float as the C++ printed it (the float promoted to a double).
fn g(v: f32) -> String {
    format_g(f64::from(v), 6)
}

fn name_list(list: &[String]) -> String {
    let mut out = String::from("{");
    for (i, n) in list.iter().enumerate() {
        out.push_str(if i == 0 { " " } else { ", " });
        out.push_str(&lua_string(n));
    }
    out.push_str(if list.is_empty() { "}" } else { " }" });
    out
}

/// The canonical text of a character: fixed key order, fixed layout, `write(read(x))`
/// idempotent (`EditorCharacters::Write`). A name or bio that holds `rus` alone is written as
/// the plain string it means.
#[must_use]
pub fn write(c: &Character) -> String {
    let mut name = c.name.clone();
    text_collapse(&mut name);
    let mut bio = c.bio.clone();
    text_collapse(&mut bio);

    let mut s = String::with_capacity(512);
    s.push_str("-- A character of this module. The game reads this file as it is - see docs/MOD_QUICKSTART.md.\n");
    s.push_str("return {\n");
    s.push_str("\tnpc       = 1,\n");
    s.push_str(&format!("\tid        = {},\n", lua_string(&c.id)));
    if !c.base.is_empty() {
        s.push_str(&format!("\tbase      = {},\n", lua_string(&c.base)));
    }
    if c.is_abstract {
        s.push_str("\tabstract  = true,\n");
    }
    if !text_empty(&name) {
        s.push_str(&format!("\tname      = {},\n", write_value(&name, 1)));
    }
    if !text_empty(&bio) {
        s.push_str(&format!("\tbio       = {},\n", write_value(&bio, 1)));
    }
    for (key, value) in [("community", &c.community), ("visual   ", &c.visual), ("icon     ", &c.icon), ("spec_rank", &c.spec_rank)] {
        if !value.is_empty() {
            s.push_str(&format!("\t{key} = {},\n", lua_string(value)));
        }
    }
    if c.has_rank {
        s.push_str(&format!("\trank      = {},\n", c.rank));
    }
    if c.has_reputation {
        s.push_str(&format!("\treputation = {},\n", c.reputation));
    }
    if c.has_money {
        s.push_str(&format!("\tmoney     = {{ min = {}, max = {}", c.money_min, c.money_max));
        if c.money_infinite {
            s.push_str(", infinite = true");
        }
        s.push_str(" },\n");
    }
    if !c.supplies.is_empty() {
        s.push_str("\tsupplies  = {\n");
        for item in &c.supplies {
            s.push_str(&format!("\t\t{{ section = {}", lua_string(&item.section)));
            if item.count != 1 {
                s.push_str(&format!(", count = {}", item.count));
            }
            s.push_str(" },\n");
        }
        s.push_str("\t},\n");
    }
    if c.trade.declared && c.trade.off {
        s.push_str("\ttrade     = false,\n");
    } else if c.trade.declared {
        let t = &c.trade;
        s.push_str("\ttrade     = {\n");
        s.push_str(&format!("\t\tsells   = {},\n", name_list(&t.sells)));
        s.push_str(&format!("\t\tbuys    = {},\n", name_list(&t.buys)));
        s.push_str(&format!("\t\tmarkup  = {},\n", g(t.markup)));
        s.push_str(&format!("\t\tpays    = {},\n", g(t.pays)));
        s.push_str(&format!("\t\trestock = {},\n", t.restock));
        if !t.stock.is_empty() {
            s.push_str("\t\tstock   = {\n");
            for r in &t.stock {
                s.push_str(&format!("\t\t\t{{ section = {}", lua_string(&r.section)));
                if r.count != 1 {
                    s.push_str(&format!(", count = {}", r.count));
                }
                if r.chance != 1.0 {
                    s.push_str(&format!(", chance = {}", g(r.chance)));
                }
                s.push_str(" },\n");
            }
            s.push_str("\t\t},\n");
        }
        s.push_str("\t},\n");
    }
    if !c.start_dialog.is_empty() {
        s.push_str(&format!("\tstart_dialog = {},\n", lua_string(&c.start_dialog)));
    }
    if c.has_squad && !c.squad {
        s.push_str("\tsquad     = false,\n");
    }
    s.push_str("}\n");
    s
}

/// A starting point for a new character, named after its file (`EditorCharacters::Template`).
/// The name is a placeholder in the mod's language; the portrait is a loner's face of Dead Air
/// Refined (the old SoC id is no picture there).
#[must_use]
pub fn template(id: &str) -> String {
    let c = Character {
        id: if id.is_empty() { "new_character".to_owned() } else { id_from_name(id) },
        community: "stalker".into(),
        name: Value::from("New character"),
        visual: "actors\\stalker_neutral\\stalker_neutral_1".into(),
        icon: "ui_inGame2_neutral_2".into(),
        has_rank: true,
        rank: 300,
        ..Character::default()
    };
    write(&c)
}

//! Reading a layout: the asset through the game's own reader (`EditorPda::Read`), and the
//! template a new layout starts as.

use std::sync::Arc;

use gz_lua::{Arg, Chunk, LuaError, LuaHost, Script};
use gz_luadata::{Table, Value};

use crate::layout::{Layout, from_reader, read_preview};

/// Where the game keeps the reader.
pub const GAME_READER: &str = "scripts\\xms_pda_layout.script";
/// The editor's copy, under its `gamedata`, kept byte-identical to the game's.
pub const BUNDLED_READER: &str = "configs\\pda\\xms_pda_layout.script";
/// The chunk name of the reader's messages.
const READER_CHUNK: &str = "xms_pda_layout.script";

/// The game's reader, and where it came from.
#[derive(Clone, Debug)]
pub struct Reader {
    pub script: Script,
    /// `game:scripts\xms_pda_layout.script` or `bundled:configs\pda\xms_pda_layout.script`.
    pub source: String,
}

impl Reader {
    /// The linked game's copy (`game = true`) or the editor's bundled one.
    #[must_use]
    pub fn new(text: impl Into<Arc<str>>, game: bool) -> Self {
        let source = if game { format!("game:{GAME_READER}") } else { format!("bundled:{BUNDLED_READER}") };
        Self { script: Script::new(READER_CHUNK, text), source }
    }
}

/// Why a text gives no layout: the file does not parse (with its 1-based line, 0 when
/// unknown), or the reader cannot read it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ReadError {
    pub message: String,
    pub line: u32,
}

impl ReadError {
    fn plain(message: impl Into<String>) -> Self {
        Self { message: message.into(), line: 0 }
    }

    /// `line N: message`, or the message alone without a line: how the answers name it.
    #[must_use]
    pub fn with_line(&self) -> String {
        if self.line > 0 { format!("line {}: {}", self.line, self.message) } else { self.message.clone() }
    }
}

/// The base name of a path without its extension (`NqUtil::BaseName`).
#[must_use]
pub fn base_name(path: &str) -> String {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match leaf.rfind('.') {
        Some(dot) if dot > 0 => leaf.get(..dot).unwrap_or(leaf).to_owned(),
        _ => leaf.to_owned(),
    }
}

/// The extension of a path's last component, lower case, with its dot (`NqUtil::Extension`).
#[must_use]
pub fn extension(path: &str) -> String {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    leaf.rfind('.').and_then(|d| leaf.get(d..)).unwrap_or("").to_ascii_lowercase()
}

#[must_use]
pub fn is_pda_path(path: &str) -> bool {
    extension(path) == ".pdaasset"
}

/// Runs `text` through the game's reader (`EditorPda::Read`). `chunk` is the file's base name
/// (the name its messages carry), `lang` the language of the texts (`rus` when empty). A file
/// that parses always comes back as a layout with its coded problems; one that does not is an
/// error with its line. Blocking: run it on the CPU pool.
///
/// The preview's aspect and state do not enter here: the reader turns the asset into canvas
/// units for every aspect, and the preview plays its state on top of the list
/// ([`crate::Preview`]).
pub fn evaluate(
    host: &LuaHost,
    reader: &Reader,
    text: &str,
    chunk: &str,
    lang: &str,
) -> Result<Layout, ReadError> {
    // the loader the game uses for every declarative asset: a syntax error gets its line, code
    // outside strings is refused before anything runs
    let value =
        gz_luadata::parse_named(text, chunk).map_err(|e| ReadError { message: e.message, line: e.line })?;
    let name = if chunk.is_empty() { "pdaasset" } else { chunk };
    let asset =
        Arg::Chunk(Chunk { name: name.to_owned(), text: Arc::from(gz_lua::vanilla_source(text).as_ref()) });
    let mut opts = Table::new();
    opts.fields.push(("lang".into(), Value::String(if lang.is_empty() { "rus" } else { lang }.into())));
    // `state` and `lists` are the reader's bookkeeping for run-time rows: no data of the layout
    let answer = host
        .call_omitting(
            &reader.script,
            "normalize",
            &[asset, Arg::Value(Value::Table(opts))],
            &["state", "lists"],
        )
        .map_err(|e| {
            ReadError::plain(match e {
                LuaError::State(_) => "cannot create a Lua state".to_owned(),
                LuaError::ScriptCompile(m) => format!("the PDA layout reader does not compile: {m}"),
                LuaError::ScriptRun(m) | LuaError::Call(m) => format!("the PDA layout reader failed: {m}"),
                LuaError::NoFunction(_) => "the PDA layout reader has no normalize()".to_owned(),
                LuaError::ArgCompile { message, .. } => message,
                LuaError::ArgRun { message, .. } => format!("code outside strings: {message}"),
                LuaError::Result(m) => format!("the PDA layout reader failed: {m}"),
            })
        })?;
    let Some(r) = answer.as_table() else {
        return Err(ReadError::plain("the PDA layout reader returned nothing"));
    };
    let mut layout = from_reader(r);
    read_preview(&value, &mut layout);
    Ok(layout)
}

/// A new layout: a starting point with the three pages every app has (`EditorPda::Template`).
#[must_use]
pub fn template(id: &str, title: &str) -> String {
    let safe: String = if id.is_empty() { "app" } else { id }
        .bytes()
        .map(|c| match c {
            b'A'..=b'Z' => char::from(c - b'A' + b'a'),
            b'a'..=b'z' | b'0'..=b'9' | b'_' => char::from(c),
            _ => '_',
        })
        .collect();
    // the title goes into a Lua string literal
    let mut name = String::new();
    for ch in if title.is_empty() { safe.as_str() } else { title }.chars() {
        if ch == '"' || ch == '\\' {
            name.push('\\');
        }
        if ch != '\n' && ch != '\r' {
            name.push(ch);
        }
    }
    TEMPLATE.replacen("@ID@", &safe, 1).replacen("@TITLE@", &name, 1)
}

const TEMPLATE: &str = r##"-- The PDA interface of this module: what the PDA shows instead of its stock tabs.
-- Pick it in Mod > PDA Interface (or mod.set set={"pda":{"layout":...}}). Format: docs/pda/PDA_FORMAT.md.
return {
  pda = 1,
  id = "@ID@",
  title = "@TITLE@",
  start = "home",
  keys = { map = "map" },

  colors = { bg = "#101114", bar = "#18191E", card = "#1F2128", line = "#2E323C",
             accent = "#FFD21E", text = "#F4F4F6", muted = "#9AA0AB" },

  styles = {
    tab = { kind = "button", color = "none", hover = "#FFFFFF0D", text_color = "muted",
            text_selected = "accent", icon_color = "muted", icon_selected = "accent",
            font = "letterica16", align = "center", valign = "bottom", icon_size = 26 },
  },

  chrome = {
    { kind = "rect", x = 0, y = 0, w = 960, h = 680, color = "bg" },
    { kind = "rect", x = 0, y = 0, w = 960, h = 64, color = "bar" },
    { kind = "text", x = 24, y = 0, w = 600, h = 64, text = "{title}", font = "letterica25", color = "text" },
    { kind = "text", x = 800, y = 0, w = 136, h = 64, text = "{time}", color = "muted", align = "right" },
    { kind = "rect", x = 0, y = 604, w = 960, h = 76, color = "bar" },
    { style = "tab", x = 0, y = 604, w = 320, h = 76, text = "Home", icon = "ui_xms_icon_home", page = "home" },
    { style = "tab", x = 320, y = 604, w = 320, h = 76, text = "Map", icon = "ui_xms_icon_map", page = "map" },
    { style = "tab", x = 640, y = 604, w = 320, h = 76, text = "Journal", icon = "ui_xms_icon_list", page = "journal" },
  },

  pages = {
    { id = "home", title = "Home", widgets = {
      { kind = "group", x = 24, y = 88, w = 912, h = 160, radius = 18, color = "card", widgets = {
        { kind = "text", x = 24, y = 20, w = 860, h = 34, text = "Hello, {actor}", font = "letterica25", color = "text" },
        { kind = "text", x = 24, y = 60, w = 860, h = 80, text = "Your screens go here.", color = "muted", valign = "top", wrap = true },
      } },
    } },
    { id = "map", title = "Map", widgets = {
      { kind = "map", x = 16, y = 80, w = 928, h = 508, color = "#0B0C0F" },
    } },
    { id = "journal", title = "Journal", stock = "eptTasks" },
  },
}
"##;

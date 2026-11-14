//! The editor's own settings file: the preferences, the theme and the interface scale, the
//! window's place, the layout store (sizes and the kit's short lists) and the keys the author
//! bound.
//!
//! One LTX file under the editor's data root, read once at start and written when something
//! changed - on the IO pool, never from a frame. Nothing is migrated from `level.ini`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::{Editor, SmolStr};
use gz_ltx::LtxDoc;
use gz_ui::{Lang, Theme};
use parking_lot::Mutex;
use serde_json::{Value, json};

/// The file's name under the editor's data root.
pub const FILE_NAME: &str = "grizzly_editor.ltx";

/// How a preference's value is read and shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefType {
    Float,
    U32,
    Bool,
    /// Degrees on the wire and in the file.
    Angle,
    /// `0xAARRGGBB`.
    Color,
}

/// A preference: its `.ini` name, its type, its default and its range.
#[derive(Clone, Copy, Debug)]
pub struct PrefDef {
    pub key: &'static str,
    pub ty: PrefType,
    pub default: f64,
    pub lo: f64,
    pub hi: f64,
}

const fn def(key: &'static str, ty: PrefType, default: f64, lo: f64, hi: f64) -> PrefDef {
    PrefDef { key, ty, default, lo, hi }
}

/// Every preference of the window itself and of the level editor, by its settings key. An area
/// adds its own with [`EditorSettings::add_defs`] (a group of Preferences, `ShellBuilder::pref_group`).
pub const PREFS: [PrefDef; 34] = [
    def("active_idle_fps", PrefType::U32, 120.0, 30.0, 240.0),
    def("vsync", PrefType::Bool, 1.0, 0.0, 1.0),
    def("floating_panels", PrefType::Bool, 0.0, 0.0, 1.0),
    def("view_np", PrefType::Float, 0.1, 0.01, 10.0),
    def("view_fp", PrefType::Float, 1500.0, 10.0, 10000.0),
    def("view_fov", PrefType::Angle, 60.0, 0.1, 170.0),
    def("fog_color", PrefType::Color, 0x0055_5555 as f64, 0.0, 0.0),
    def("fog_fogness", PrefType::Float, 0.9, 0.0, 100.0),
    def("cam_fly_speed", PrefType::Float, 5.0, 0.01, 100.0),
    def("cam_fly_alt", PrefType::Float, 1.8, 0.0, 1000.0),
    def("cam_sens_rot", PrefType::Float, 0.6, 0.01, 10.0),
    def("cam_sens_move", PrefType::Float, 0.6, 0.01, 10.0),
    def("tools_sens_move", PrefType::Float, 0.3, 0.01, 10.0),
    def("tools_sens_rot", PrefType::Float, 0.3, 0.01, 10.0),
    def("tools_sens_scale", PrefType::Float, 0.3, 0.01, 10.0),
    def("tools_show_move_axis", PrefType::Bool, 0.0, 0.0, 1.0),
    def("bp_lim_depth", PrefType::Bool, 1.0, 0.0, 1.0),
    def("bp_cull", PrefType::Bool, 1.0, 0.0, 1.0),
    def("bp_depth_tolerance", PrefType::Float, 0.1, 0.0, 10000.0),
    def("snap_angle_step", PrefType::Angle, 15.0, 0.1, 180.0),
    def("snap_grid_step", PrefType::Float, 0.1, 0.01, 1000.0),
    def("snap_scale_step", PrefType::Float, 0.25, 0.01, 1000.0),
    def("snap_moveto", PrefType::Float, 0.5, 0.01, 1000.0),
    def("grid_cell_size", PrefType::Float, 1.0, 0.1, 10.0),
    def("grid_cell_count", PrefType::U32, 100.0, 10.0, 1000.0),
    def("scene_undo_level", PrefType::U32, 125.0, 0.0, 125.0),
    def("scene_recent_count", PrefType::U32, 10.0, 0.0, 25.0),
    def("scene_clear_color", PrefType::Color, 0x0055_5555 as f64, 0.0, 0.0),
    def("render_radius", PrefType::Float, 400.0, 10.0, 100000.0),
    def("object_flags", PrefType::U32, 0.0, 0.0, 4_294_967_295.0),
    def("recovery_enabled", PrefType::Bool, 1.0, 0.0, 1.0),
    def("recovery_interval_minutes", PrefType::U32, 10.0, 1.0, 120.0),
    def("recovery_interaction_delay_seconds", PrefType::U32, 15.0, 0.0, 120.0),
    def("recovery_retention", PrefType::U32, 10.0, 2.0, 64.0),
];

#[must_use]
pub fn pref_def(key: &str) -> Option<&'static PrefDef> {
    PREFS.iter().find(|p| p.key == key)
}

/// The interface scales Preferences offers besides Auto (the monitor's), in percent.
pub const INTERFACE_SCALES: [u32; 5] = [100, 125, 150, 175, 200];

/// `auto` (the monitor's scale: `None`) or a percentage of [`INTERFACE_SCALES`]; `None` for
/// anything else.
#[must_use]
pub fn parse_interface_scale(text: &str) -> Option<Option<f32>> {
    let t = text.trim().trim_end_matches('%').trim();
    if t.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    let v: f32 = t.parse().ok()?;
    let percent = if v <= 4.0 { (v * 100.0).round() } else { v.round() };
    INTERFACE_SCALES.iter().find(|s| **s as f32 == percent).map(|s| Some(*s as f32 / 100.0))
}

/// A number as C's `%g` would print it, as a JSON number.
fn number(v: f64) -> Value {
    let short: f64 = format!("{}", v as f32).parse().unwrap_or(v);
    if short.fract() == 0.0 && short.abs() < 1e15 { json!(short as i64) } else { json!(short) }
}

/// The value of a preference as `preferences` answers it.
#[must_use]
pub fn pref_json(def: &PrefDef, value: f64) -> Value {
    match def.ty {
        PrefType::Float | PrefType::Angle => number(value),
        PrefType::U32 => json!(value as u64),
        PrefType::Bool => json!(value != 0.0),
        PrefType::Color => json!(format!("0x{:08X}", value as u32)),
    }
}

/// Reads a value given as text for a preference; `Err` names what it takes.
pub fn parse_pref(def: &PrefDef, text: &str) -> Result<f64, String> {
    match def.ty {
        PrefType::Bool => match text {
            "true" | "1" => Ok(1.0),
            "false" | "0" => Ok(0.0),
            _ => Err("a true or false value".to_owned()),
        },
        PrefType::Color => {
            let (digits, rgb_only) = match text.strip_prefix('#') {
                Some(rest) => (rest, rest.len() == 6),
                None => (text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text), false),
            };
            let c = u32::from_str_radix(digits, 16)
                .map_err(|_| "a colour as 0xAARRGGBB or #RRGGBB".to_owned())?;
            Ok(f64::from(if rgb_only { c | 0xff00_0000 } else { c }))
        }
        _ => {
            let range = || format!("a number {}..{}", fmt_g(def.lo), fmt_g(def.hi));
            let v: f64 = text.trim().parse().map_err(|_| range())?;
            if text.trim().is_empty() || v < def.lo || v > def.hi || !v.is_finite() {
                return Err(range());
            }
            Ok(if def.ty == PrefType::U32 { v.trunc() } else { v })
        }
    }
}

fn fmt_g(v: f64) -> String {
    format!("{}", v as f32)
}

/// Where the window stood, in pt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
    pub maximized: bool,
}

struct Inner {
    doc: LtxDoc,
    prefs: Vec<f64>,
    /// What an area added (`add_defs`), with their values.
    extra: Vec<(PrefDef, f64)>,
    dirty: bool,
    /// How many times the language was set in this run: a choice made now wins over `-lang`.
    lang_rev: u64,
}

/// The editor's settings, an `Editor` service. The file is read by [`EditorSettings::load`] at
/// start; afterwards everything is in memory.
pub struct EditorSettings {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// The file's writes in the order they were asked for.
    order: Arc<gz_editor::fsx::InOrder>,
    /// The Preferences page last shown.
    pub page: Mutex<SmolStr>,
    /// `-theme <id>`: the theme of this run, never saved; a theme picked meanwhile replaces it.
    run_theme: Mutex<Option<&'static Theme>>,
}

impl EditorSettings {
    /// Reads the file (one read; a missing file is the defaults).
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let doc = std::fs::read(path)
            .map(|b| LtxDoc::parse(&gz_base::text::decode_guess(&b)))
            .unwrap_or_else(|_| LtxDoc::new());
        let prefs = PREFS
            .iter()
            .map(|d| doc.get("preferences", d.key).and_then(|t| parse_pref(d, t).ok()).unwrap_or(d.default))
            .collect();
        Self {
            path: path.to_path_buf(),
            inner: Mutex::new(Inner { doc, prefs, extra: Vec::new(), dirty: false, lang_rev: 0 }),
            order: Arc::default(),
            page: Mutex::new(SmolStr::new("General")),
            run_theme: Mutex::new(None),
        }
    }

    /// The settings of this editor, read on first use from the editor's home.
    pub fn of(editor: &Editor) -> Arc<EditorSettings> {
        editor.services.get_or_init(|| EditorSettings::load(&editor.paths.home.join(FILE_NAME)))
    }

    #[must_use]
    pub fn pref(&self, key: &str) -> Option<f64> {
        let s = self.inner.lock();
        match PREFS.iter().position(|d| d.key == key) {
            Some(i) => s.prefs.get(i).copied(),
            None => s.extra.iter().find(|(d, _)| d.key == key).map(|(_, v)| *v),
        }
    }

    pub fn set_pref(&self, key: &str, value: f64) {
        let Some(def) = self.def(key) else { return };
        let text = match def.ty {
            PrefType::Bool => (if value != 0.0 { "true" } else { "false" }).to_owned(),
            PrefType::Color => format!("0x{:08X}", value as u32),
            PrefType::U32 => format!("{}", value as u64),
            _ => fmt_g(value),
        };
        let mut s = self.inner.lock();
        match PREFS.iter().position(|d| d.key == key) {
            Some(i) => {
                if let Some(slot) = s.prefs.get_mut(i) {
                    *slot = value;
                }
            }
            None => {
                if let Some(slot) = s.extra.iter_mut().find(|(d, _)| d.key == key) {
                    slot.1 = value;
                }
            }
        }
        s.doc.set("preferences", key, &text);
        s.dirty = true;
    }

    /// The definition of a preference: the window's own or one an area added.
    #[must_use]
    pub fn def(&self, key: &str) -> Option<PrefDef> {
        pref_def(key)
            .copied()
            .or_else(|| self.inner.lock().extra.iter().find(|(d, _)| d.key == key).map(|(d, _)| *d))
    }

    /// The preferences areas added, in the order they came.
    #[must_use]
    pub fn extra_defs(&self) -> Vec<PrefDef> {
        self.inner.lock().extra.iter().map(|(d, _)| *d).collect()
    }

    /// Adds an area's preferences: `app.prefs` lists and sets them, and their values come from the
    /// settings file. A key added twice keeps the first definition.
    pub fn add_defs(&self, defs: &[PrefDef]) {
        let mut s = self.inner.lock();
        for d in defs {
            if pref_def(d.key).is_some() || s.extra.iter().any(|(e, _)| e.key == d.key) {
                continue;
            }
            let v = s.doc.get("preferences", d.key).and_then(|t| parse_pref(d, t).ok()).unwrap_or(d.default);
            s.extra.push((*d, v));
        }
    }

    /// Preferences > Interface > Interface Scale: `None` is Auto, the monitor's scale.
    #[must_use]
    pub fn interface_scale(&self) -> Option<f32> {
        let s = self.inner.lock();
        s.doc.get("ui", "scale").and_then(parse_interface_scale).flatten()
    }

    pub fn set_interface_scale(&self, scale: Option<f32>) {
        let mut s = self.inner.lock();
        let text = scale.map_or_else(|| "auto".to_owned(), |v| format!("{}", (v * 100.0).round() as u32));
        s.doc.set("ui", "scale", &text);
        s.dirty = true;
    }

    /// Preferences > Interface > Theme (`[ui] theme`), or this run's `-theme`; a name no theme has is
    /// the default.
    #[must_use]
    pub fn theme(&self) -> &'static Theme {
        if let Some(t) = *self.run_theme.lock() {
            return t;
        }
        self.inner.lock().doc.get("ui", "theme").map_or_else(Theme::default_theme, Theme::from_settings)
    }

    /// Picks the theme and keeps it; a `-theme` of this run gives way to it.
    pub fn set_theme(&self, theme: &'static Theme) {
        *self.run_theme.lock() = None;
        let mut s = self.inner.lock();
        s.doc.set("ui", "theme", theme.id);
        s.dirty = true;
    }

    /// `-theme <id>`: the theme of this run alone; the file keeps its own.
    pub fn set_run_theme(&self, theme: &'static Theme) {
        *self.run_theme.lock() = Some(theme);
    }

    #[must_use]
    pub fn placement(&self) -> Option<Placement> {
        let s = self.inner.lock();
        let num = |k: &str| s.doc.get("window", k).and_then(|v| v.trim().parse::<f32>().ok());
        let size = [num("w")?, num("h")?];
        let pos = num("x").zip(num("y")).map(|(x, y)| [x, y]);
        Some(Placement {
            pos,
            size,
            maximized: s.doc.get("window", "maximized").is_some_and(gz_ltx::doc::parse_bool),
        })
    }

    pub fn set_placement(&self, p: Placement) {
        let mut s = self.inner.lock();
        if let Some([x, y]) = p.pos {
            s.doc.set("window", "x", &format!("{}", x.round()));
            s.doc.set("window", "y", &format!("{}", y.round()));
        }
        s.doc.set("window", "w", &format!("{}", p.size[0].round()));
        s.doc.set("window", "h", &format!("{}", p.size[1].round()));
        s.doc.set("window", "maximized", if p.maximized { "true" } else { "false" });
        s.dirty = true;
    }

    /// The sizes the layout store keeps.
    #[must_use]
    pub fn layout(&self) -> Vec<(SmolStr, f32)> {
        let s = self.inner.lock();
        s.doc
            .entries("layout")
            .filter_map(|(k, v)| v.trim().parse::<f32>().ok().map(|v| (SmolStr::new(k), v)))
            .collect()
    }

    pub fn set_layout(&self, values: &[(SmolStr, f32)]) {
        let mut s = self.inner.lock();
        let entries: Vec<(String, String)> =
            values.iter().map(|(k, v)| (k.to_string(), format!("{v}"))).collect();
        s.doc.replace_entries("layout", entries);
        s.dirty = true;
    }

    /// The shortcuts the author bound: id and keys (`""` when taken away).
    #[must_use]
    pub fn shortcuts(&self) -> Vec<(SmolStr, String)> {
        let s = self.inner.lock();
        s.doc
            .entries("shortcuts")
            .map(|(k, v)| (SmolStr::new(k), gz_ltx::doc::unquote(v).to_owned()))
            .collect()
    }

    pub fn set_shortcut(&self, id: &str, keys: &str) {
        let mut s = self.inner.lock();
        s.doc.set("shortcuts", id, &gz_ltx::doc::quote(keys));
        s.dirty = true;
    }

    /// Forgets what the author bound an action to: its defaults apply again.
    pub fn remove_shortcut(&self, id: &str) {
        let mut s = self.inner.lock();
        if s.doc.remove("shortcuts", id) {
            s.dirty = true;
        }
    }

    /// The kit's short lists (a pick list's recent and favourite entries, a section's state):
    /// `[lists]`, each a key and its items as a JSON array.
    #[must_use]
    pub fn lists(&self) -> Vec<(SmolStr, Vec<String>)> {
        let s = self.inner.lock();
        s.doc
            .entries("lists")
            .filter_map(|(k, v)| {
                serde_json::from_str::<Vec<String>>(gz_ltx::doc::unquote(v))
                    .ok()
                    .map(|l| (SmolStr::new(k), l))
            })
            .collect()
    }

    pub fn set_lists(&self, lists: &[(SmolStr, Vec<String>)]) {
        let mut s = self.inner.lock();
        let entries: Vec<(String, String)> = lists
            .iter()
            .map(|(k, v)| (k.to_string(), gz_ltx::doc::quote(&serde_json::to_string(v).unwrap_or_default())))
            .collect();
        s.doc.replace_entries("lists", entries);
        s.dirty = true;
    }

    /// The adapter the device is made on, by name (`[render] gpu_adapter`); empty is the system's
    /// default.
    #[must_use]
    pub fn gpu_adapter(&self) -> String {
        let s = self.inner.lock();
        s.doc.get("render", "gpu_adapter").map(|v| gz_ltx::doc::unquote(v).to_owned()).unwrap_or_default()
    }

    pub fn set_gpu_adapter(&self, name: &str) {
        let mut s = self.inner.lock();
        s.doc.set("render", "gpu_adapter", &gz_ltx::doc::quote(name));
        s.dirty = true;
    }

    /// `[editor_prefs] tools_settings`: the tool switches,
    /// `None` while the file holds none.
    #[must_use]
    pub fn tools_settings(&self) -> Option<u32> {
        let s = self.inner.lock();
        s.doc.get("editor_prefs", "tools_settings").and_then(|v| v.trim().parse().ok())
    }

    pub fn set_tools_settings(&self, flags: u32) {
        let mut s = self.inner.lock();
        if s.doc.get("editor_prefs", "tools_settings").and_then(|v| v.trim().parse::<u32>().ok())
            != Some(flags)
        {
            s.doc.set("editor_prefs", "tools_settings", &flags.to_string());
            s.dirty = true;
        }
    }

    /// A switch of the View menu kept with the render settings: `mute_sounds`
    /// , `real_time`.
    #[must_use]
    pub fn render_flag(&self, key: &str) -> bool {
        self.inner.lock().doc.get("render", key).is_some_and(gz_ltx::doc::parse_bool)
    }

    pub fn set_render_flag(&self, key: &str, on: bool) {
        let mut s = self.inner.lock();
        s.doc.set("render", key, if on { "true" } else { "false" });
        s.dirty = true;
    }

    /// Preferences > General > Language (`[ui] language`): the system's unless the author picked
    /// one. A value the editor does not know reads as the system's.
    #[must_use]
    pub fn language(&self) -> LangChoice {
        self.inner.lock().doc.get("ui", "language").and_then(LangChoice::parse).unwrap_or_default()
    }

    pub fn set_language(&self, choice: LangChoice) {
        let mut s = self.inner.lock();
        s.doc.set("ui", "language", choice.key());
        s.dirty = true;
        s.lang_rev += 1;
    }

    /// How many times the language was set in this run.
    #[must_use]
    pub fn language_rev(&self) -> u64 {
        self.inner.lock().lang_rev
    }

    /// Something changed since the file was last written.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.inner.lock().dirty
    }

    /// Writes the file if something changed, on the IO pool.
    pub fn save_soon(&self, editor: &Editor) {
        if let Some((ticket, path, text)) = self.take_dirty() {
            let (log, order) = (editor.log.clone(), self.order.clone());
            editor.exec.spawn_io(move || {
                if let Err(e) = order.write(ticket, &path, text.as_bytes()) {
                    log.warn(format!(
                        "the editor's settings could not be written to {}: {e}",
                        path.display()
                    ));
                }
            });
        }
    }

    /// Writes the file now if something changed: the editor is closing.
    pub fn save_now(&self, editor: &Editor) {
        if let Some((ticket, path, text)) = self.take_dirty()
            && let Err(e) = self.order.write(ticket, &path, text.as_bytes())
        {
            editor.log.warn(format!("the editor's settings could not be written to {}: {e}", path.display()));
        }
    }

    /// The text to write and its place among the file's writes, taken under one lock.
    fn take_dirty(&self) -> Option<(u64, PathBuf, String)> {
        let mut s = self.inner.lock();
        if !std::mem::take(&mut s.dirty) {
            return None;
        }
        Some((self.order.ticket(), self.path.clone(), s.doc.to_string()))
    }
}

/// The window's language as the settings keep it: the system's, or one the author picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LangChoice {
    /// Russian when Windows speaks Russian to the user, English otherwise.
    #[default]
    System,
    Lang(Lang),
}

impl LangChoice {
    /// The key of the settings file and of `app.prefs`: `system`, `en`, `ru`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            LangChoice::System => "system",
            LangChoice::Lang(l) => l.key(),
        }
    }

    /// `system`, a language's key or its name.
    #[must_use]
    pub fn parse(text: &str) -> Option<LangChoice> {
        if text.trim().eq_ignore_ascii_case("system") {
            return Some(LangChoice::System);
        }
        Lang::parse(text).map(LangChoice::Lang)
    }

    /// The language shown for this choice on a system whose interface speaks `system` (an ISO
    /// 639-1 code, `gz_win::user_ui_language`).
    #[must_use]
    pub fn resolve(self, system: &str) -> Lang {
        match self {
            LangChoice::System => system_lang(system),
            LangChoice::Lang(l) => l,
        }
    }
}

/// The language the window speaks when it follows the system: Russian for a Russian Windows,
/// English for any other.
#[must_use]
pub fn system_lang(code: &str) -> Lang {
    if code.eq_ignore_ascii_case("ru") { Lang::Ru } else { Lang::En }
}

/// Every choice of the language, in the order Preferences lists them.
pub const LANG_CHOICES: [LangChoice; 3] =
    [LangChoice::System, LangChoice::Lang(Lang::En), LangChoice::Lang(Lang::Ru)];

#[cfg(test)]
mod tests {
    use super::{
        EditorSettings, LangChoice, PREFS, parse_interface_scale, parse_pref, pref_def, pref_json,
        system_lang,
    };
    use gz_ui::Lang;

    #[test]
    fn the_language_follows_the_system_unless_one_is_picked() {
        assert_eq!(system_lang("ru"), Lang::Ru);
        assert_eq!(system_lang("en"), Lang::En);
        assert_eq!(system_lang("de"), Lang::En, "a language the editor lacks reads as English");
        assert_eq!(system_lang(""), Lang::En);
        assert_eq!(LangChoice::System.resolve("ru"), Lang::Ru);
        assert_eq!(LangChoice::Lang(Lang::En).resolve("ru"), Lang::En);
        assert_eq!(LangChoice::Lang(Lang::Ru).resolve("en"), Lang::Ru);
        assert_eq!(LangChoice::parse("System"), Some(LangChoice::System));
        assert_eq!(LangChoice::parse("ru"), Some(LangChoice::Lang(Lang::Ru)));
        assert_eq!(LangChoice::parse("English"), Some(LangChoice::Lang(Lang::En)));
        assert_eq!(LangChoice::parse("klingon"), None);
        assert_eq!(LangChoice::default().key(), "system");
    }

    #[test]
    fn values_parse_as_their_types_say() {
        let fov = pref_def("view_fov").map(|d| parse_pref(d, "75"));
        assert_eq!(fov, Some(Ok(75.0)));
        let fps = pref_def("active_idle_fps").map(|d| parse_pref(d, "10"));
        assert_eq!(fps, Some(Err("a number 30..240".to_owned())));
        let color = pref_def("fog_color").map(|d| parse_pref(d, "#102030"));
        assert_eq!(color, Some(Ok(f64::from(0xff10_2030u32))));
        let flag = pref_def("bp_cull").map(|d| parse_pref(d, "yes"));
        assert_eq!(flag, Some(Err("a true or false value".to_owned())));
    }

    #[test]
    fn answers() {
        let Some(np) = pref_def("view_np") else { return };
        assert_eq!(pref_json(np, 0.1), serde_json::json!(0.1));
        let Some(fov) = pref_def("view_fov") else { return };
        assert_eq!(pref_json(fov, 60.0), serde_json::json!(60));
        let Some(c) = pref_def("fog_color") else { return };
        assert_eq!(pref_json(c, f64::from(0x0055_5555u32)), serde_json::json!("0x00555555"));
        assert_eq!(PREFS.len(), 34);
        assert_eq!(parse_interface_scale("auto"), Some(None));
        assert_eq!(parse_interface_scale("125%"), Some(Some(1.25)));
        assert_eq!(parse_interface_scale("1.5"), Some(Some(1.5)));
        assert_eq!(parse_interface_scale("130"), None);
    }

    #[test]
    fn themes() {
        let dir = std::env::temp_dir().join(format!("gz_settings_theme_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("settings.ltx");
        // a theme by id or title; a name no theme has is the default
        for (was, is) in [
            ("grizzly-graphite", "grizzly-graphite"),
            ("Solarized Light", "solarized-light"),
            ("violet", "grizzly-dark"),
        ] {
            let _ = std::fs::write(&file, format!("[ui]\ntheme = {was}\n"));
            assert_eq!(EditorSettings::load(&file).theme().id, is, "{was}");
        }
        let s = EditorSettings::load(&dir.join("missing.ltx"));
        assert_eq!(s.theme().id, "grizzly-dark");
        let light = gz_ui::Theme::find("solarized-light").inspect(|t| s.set_run_theme(t));
        assert_eq!(Some(s.theme()), light);
        // a run's theme is never written; a pick is, and replaces it
        assert!(!s.inner.lock().doc.to_string().contains("solarized"));
        if let Some(t) = gz_ui::Theme::find("nord") {
            s.set_theme(t);
        }
        assert_eq!(s.theme().id, "nord");
        assert!(s.inner.lock().doc.to_string().contains("theme = nord"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! The editor's own settings file: the preferences, the theme, the window's place, the layout
//! store and the shortcuts the author bound.
//!
//! One LTX file under the editor's data root, read once at start and written when something
//! changed - on the IO pool, never from a frame. Nothing is migrated from `level.ini`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::{Editor, SmolStr};
use gz_ltx::LtxDoc;
use gz_ui::Accent;
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

/// How many of [`PREFS`] the `preferences` command lists and takes: the C++ editor's table
/// (`kPrefs`). The rest are kept and edited by the Preferences tab only.
pub const WIRE_PREFS: usize = 28;

/// Every preference: first the ones `preferences action=get` lists, in its order, then the ones
/// only the Preferences tab edits (the objects' switches, recovery).
pub const PREFS: [PrefDef; 33] = [
    def("active_idle_fps", PrefType::U32, 120.0, 30.0, 240.0),
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
    def("snap_angle_step", PrefType::Angle, 5.625, 0.1, 180.0),
    def("snap_grid_step", PrefType::Float, 0.1, 0.01, 1000.0),
    def("snap_scale_step", PrefType::Float, 0.1, 0.01, 1000.0),
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

/// A preference the `preferences` command knows.
#[must_use]
pub fn wire_pref_def(key: &str) -> Option<&'static PrefDef> {
    PREFS.iter().take(WIRE_PREFS).find(|p| p.key == key)
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
    dirty: bool,
}

/// The editor's settings, an `Editor` service. The file is read by [`EditorSettings::load`] at
/// start; afterwards everything is in memory.
pub struct EditorSettings {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// The Preferences page last shown.
    pub page: Mutex<SmolStr>,
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
            inner: Mutex::new(Inner { doc, prefs, dirty: false }),
            page: Mutex::new(SmolStr::new("General")),
        }
    }

    /// The settings of this editor, read on first use from the editor's data root.
    pub fn of(editor: &Editor) -> Arc<EditorSettings> {
        editor.services.get_or_init(|| EditorSettings::load(&editor.paths.root.join(FILE_NAME)))
    }

    #[must_use]
    pub fn pref(&self, key: &str) -> Option<f64> {
        let i = PREFS.iter().position(|d| d.key == key)?;
        self.inner.lock().prefs.get(i).copied()
    }

    pub fn set_pref(&self, key: &str, value: f64) {
        let Some((i, def)) = PREFS.iter().enumerate().find(|(_, d)| d.key == key) else { return };
        let text = match def.ty {
            PrefType::Bool => (if value != 0.0 { "true" } else { "false" }).to_owned(),
            PrefType::Color => format!("0x{:08X}", value as u32),
            PrefType::U32 => format!("{}", value as u64),
            _ => fmt_g(value),
        };
        let mut s = self.inner.lock();
        if let Some(slot) = s.prefs.get_mut(i) {
            *slot = value;
        }
        s.doc.set("preferences", key, &text);
        s.dirty = true;
    }

    #[must_use]
    pub fn accent(&self) -> Accent {
        match self.inner.lock().doc.get("ui", "theme") {
            Some("graphite") => Accent::Graphite,
            _ => Accent::Violet,
        }
    }

    pub fn set_accent(&self, accent: Accent) {
        let mut s = self.inner.lock();
        s.doc.set("ui", "theme", theme_key(accent));
        s.dirty = true;
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

    /// Something changed since the file was last written.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.inner.lock().dirty
    }

    /// Writes the file if something changed, on the IO pool.
    pub fn save_soon(&self, editor: &Editor) {
        if let Some((path, text)) = self.take_dirty() {
            let log = editor.log.clone();
            editor.exec.spawn_io(move || {
                if let Err(e) = gz_editor::fsx::write_atomic(&path, text.as_bytes()) {
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
        if let Some((path, text)) = self.take_dirty()
            && let Err(e) = gz_editor::fsx::write_atomic(&path, text.as_bytes())
        {
            editor.log.warn(format!("the editor's settings could not be written to {}: {e}", path.display()));
        }
    }

    fn take_dirty(&self) -> Option<(PathBuf, String)> {
        let mut s = self.inner.lock();
        if !std::mem::take(&mut s.dirty) {
            return None;
        }
        Some((self.path.clone(), s.doc.to_string()))
    }
}

/// The wire key of an accent family.
#[must_use]
pub fn theme_key(accent: Accent) -> &'static str {
    match accent {
        Accent::Violet => "grizzly-purple",
        Accent::Graphite => "graphite",
    }
}

/// The name the author reads.
#[must_use]
pub fn theme_name(accent: Accent) -> &'static str {
    match accent {
        Accent::Violet => "Grizzly Purple",
        Accent::Graphite => "Graphite",
    }
}

/// A preset by its key or its name, any case.
#[must_use]
pub fn parse_theme(text: &str) -> Option<Accent> {
    [Accent::Violet, Accent::Graphite]
        .into_iter()
        .find(|a| text.eq_ignore_ascii_case(theme_key(*a)) || text.eq_ignore_ascii_case(theme_name(*a)))
}

#[cfg(test)]
mod tests {
    use super::{PREFS, WIRE_PREFS, parse_pref, parse_theme, pref_def, pref_json};
    use gz_ui::Accent;

    #[test]
    fn values_parse_like_the_cpp_editor() {
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
        assert_eq!(PREFS.len(), 33);
        assert_eq!(PREFS.get(WIRE_PREFS - 1).map(|d| d.key), Some("render_radius"));
    }

    #[test]
    fn themes() {
        assert_eq!(parse_theme("Graphite"), Some(Accent::Graphite));
        assert_eq!(parse_theme("grizzly-purple"), Some(Accent::Violet));
        assert_eq!(parse_theme("pink"), None);
    }
}

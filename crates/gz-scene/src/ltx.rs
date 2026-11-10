//! The C++ editor's `CInifile` as the scene files use it: [`IniWriter`] builds a file the way
//! `w_*` and `save_as` do (sections and keys sorted, names lower-cased and stripped of blanks,
//! `%-32s` padding, a space after every comma outside quotes); [`IniReader`] reads one with the
//! editor's rules (`gz_ltx`, `Dialect::Editor`).

use std::collections::BTreeMap;

use gz_ltx::{Dialect, Ltx};
use rustc_hash::FxHashMap;

use crate::SceneError;
use crate::math::Vec3;

/// `_parse`: every blank outside quotes is dropped.
#[must_use]
pub fn parse_ws(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut inside = false;
    for ch in src.chars() {
        if ch.is_ascii_whitespace() && !inside {
            continue;
        }
        if ch == '"' {
            inside = !inside;
        }
        out.push(ch);
    }
    out
}

/// `_decorate`: a space after every comma outside quotes.
fn decorate(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    let mut inside = false;
    for ch in src.chars() {
        out.push(ch);
        if ch == ',' && !inside {
            out.push(' ');
        } else if ch == '"' {
            inside = !inside;
        }
    }
    out
}

/// C's `%f`.
#[must_use]
pub fn fmt_f(v: f32) -> String {
    if v.is_nan() {
        return if v.is_sign_negative() { "-nan(ind)".to_owned() } else { "nan".to_owned() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".to_owned() } else { "inf".to_owned() };
    }
    format!("{:.6}", f64::from(v))
}

/// C's `%g` for the numbers the scene prints (`SXmsOrder::Section`, the MCP answers).
#[must_use]
pub fn fmt_g(v: f32) -> String {
    let v = f64::from(v);
    if v == 0.0 {
        return "0".to_owned();
    }
    if !v.is_finite() {
        return if v.is_nan() { "nan".to_owned() } else if v < 0.0 { "-inf".to_owned() } else { "inf".to_owned() };
    }
    let exp = v.abs().log10().floor() as i32;
    if !(-5..6).contains(&exp) {
        let s = format!("{v:.5e}");
        // Rust writes `1.50000e7`; C writes `1.5e+07`
        let (mant, e) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = trim_zeros(mant);
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = usize::try_from(5 - exp).unwrap_or(0);
    trim_zeros(&format!("{v:.decimals$}")).to_owned()
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

/// A file being written. `w_string` overrides a key that is already there, as the editor's
/// `CInifile` (with `eOverrideNames`) does.
#[derive(Clone, Debug, Default)]
pub struct IniWriter {
    sections: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

impl IniWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `w_string`: the section lower-cased, every part stripped of blanks outside quotes.
    pub fn w_string(&mut self, section: &str, key: &str, value: &str) {
        let s = parse_ws(section).to_lowercase();
        let k = parse_ws(key);
        let v = parse_ws(value);
        let sect = self.sections.entry(s).or_default();
        if k.is_empty() {
            return;
        }
        sect.insert(k, (!v.is_empty()).then_some(v));
    }

    /// A key without a value (`w_string(sect, name, nullptr)`).
    pub fn w_key(&mut self, section: &str, key: &str) {
        self.w_string(section, key, "");
    }

    /// `w_u32` prints with `%d`.
    pub fn w_u32(&mut self, section: &str, key: &str, v: u32) {
        self.w_string(section, key, &(v as i32).to_string());
    }
    pub fn w_s32(&mut self, section: &str, key: &str, v: i32) {
        self.w_string(section, key, &v.to_string());
    }
    pub fn w_u16(&mut self, section: &str, key: &str, v: u16) {
        self.w_string(section, key, &v.to_string());
    }
    pub fn w_u8(&mut self, section: &str, key: &str, v: u8) {
        self.w_string(section, key, &v.to_string());
    }
    pub fn w_u64(&mut self, section: &str, key: &str, v: u64) {
        self.w_string(section, key, &v.to_string());
    }
    pub fn w_float(&mut self, section: &str, key: &str, v: f32) {
        self.w_string(section, key, &fmt_f(v));
    }
    pub fn w_fvector2(&mut self, section: &str, key: &str, v: [f32; 2]) {
        self.w_string(section, key, &format!("{},{}", fmt_f(v[0]), fmt_f(v[1])));
    }
    pub fn w_fvector3(&mut self, section: &str, key: &str, v: Vec3) {
        self.w_string(section, key, &format!("{},{},{}", fmt_f(v.x), fmt_f(v.y), fmt_f(v.z)));
    }
    pub fn w_fcolor(&mut self, section: &str, key: &str, v: [f32; 4]) {
        self.w_string(section, key, &format!("{},{},{},{}", fmt_f(v[0]), fmt_f(v[1]), fmt_f(v[2]), fmt_f(v[3])));
    }
    /// `w_color`: `r,g,b,a` of `0xAARRGGBB`.
    pub fn w_color(&mut self, section: &str, key: &str, argb: u32) {
        let (a, r, g, b) = (argb >> 24, (argb >> 16) & 0xff, (argb >> 8) & 0xff, argb & 0xff);
        self.w_string(section, key, &format!("{r},{g},{b},{a}"));
    }
    pub fn w_bool(&mut self, section: &str, key: &str, v: bool) {
        self.w_string(section, key, if v { "on" } else { "off" });
    }
    /// A value already in the form the file holds (a kept line, an entity's value).
    pub fn w_raw(&mut self, section: &str, key: &str, value: &str) {
        self.w_string(section, key, value);
    }

    /// `save_as`.
    #[must_use]
    pub fn save(&self) -> String {
        let mut out = String::new();
        for (name, items) in &self.sections {
            out.push('[');
            out.push_str(name);
            out.push_str("]\r\n");
            for (key, value) in items {
                let mut line = String::with_capacity(48 + key.len());
                line.push_str("        ");
                line.push_str(key);
                pad(&mut line, 8 + 32);
                line.push_str(" = ");
                if let Some(v) = value {
                    let d = decorate(v);
                    let start = line.len();
                    line.push_str(&d);
                    pad(&mut line, start + 32);
                }
                out.push_str(line.trim_end());
                out.push_str("\r\n");
            }
            out.push_str(" \r\n");
        }
        out
    }
}

/// `%-Ns` pads bytes, not characters.
fn pad(line: &mut String, to: usize) {
    while line.len() < to {
        line.push(' ');
    }
}

/// A file being read, with the C++ reader's rules.
#[derive(Clone, Debug)]
pub struct IniReader {
    ltx: Ltx,
    file: String,
}

impl IniReader {
    /// Parses one file (no includes are followed: a scene file has none).
    pub fn parse(text: &str, file: &str) -> Result<Self, SceneError> {
        let parsed = gz_ltx::parse_file(text, Dialect::Editor);
        let mut map = FxHashMap::default();
        map.insert("scene".to_owned(), parsed);
        let loaded = Ltx::assemble_map("scene", &map, Dialect::Editor).map_err(|e| SceneError::load(format!("{e} ({file})")))?;
        Ok(Self { ltx: loaded.ltx, file: file.to_owned() })
    }

    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    #[must_use]
    pub fn section_exist(&self, section: &str) -> bool {
        self.ltx.section_exist(&section.to_lowercase())
    }

    #[must_use]
    pub fn line_exist(&self, section: &str, key: &str) -> bool {
        self.ltx.line_exist(&section.to_lowercase(), key)
    }

    /// Every section, sorted by name.
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.ltx.sections().iter().map(|s| &*s.name)
    }

    /// The keys and raw values of a section in the engine's order.
    #[must_use]
    pub fn lines(&self, section: &str) -> Vec<(String, String)> {
        self.ltx
            .section(&section.to_lowercase())
            .map(|s| s.items.iter().map(|it| (it.key.to_string(), it.value.as_deref().unwrap_or("").to_owned())).collect())
            .unwrap_or_default()
    }

    fn missing(&self, section: &str, key: &str) -> SceneError {
        SceneError::load(format!("{}: [{}] has no '{}'", self.file, section, key))
    }

    /// The value as stored (`r_string`), quotes included; empty for `key =`.
    pub fn r_string_raw(&self, section: &str, key: &str) -> Result<String, SceneError> {
        let s = section.to_lowercase();
        if !self.ltx.line_exist(&s, key) {
            return Err(self.missing(section, key));
        }
        Ok(self.ltx.r_string_raw(&s, key).ok().flatten().unwrap_or("").to_owned())
    }

    /// `r_string` of a name or a path: the C++ stores what it read.
    pub fn r_string(&self, section: &str, key: &str) -> Result<String, SceneError> {
        self.r_string_raw(section, key)
    }

    /// `r_string_wb`: one pair of quotes stripped.
    pub fn r_string_wb(&self, section: &str, key: &str) -> Result<String, SceneError> {
        let v = self.r_string_raw(section, key)?;
        let v = v.strip_suffix('"').unwrap_or(&v);
        Ok(v.strip_prefix('"').unwrap_or(v).to_owned())
    }

    pub fn r_u32(&self, section: &str, key: &str) -> Result<u32, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_u32(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_s32(&self, section: &str, key: &str) -> Result<i32, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_s32(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_u16(&self, section: &str, key: &str) -> Result<u16, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_u16(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_u8(&self, section: &str, key: &str) -> Result<u8, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_u8(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_u64(&self, section: &str, key: &str) -> Result<u64, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_u64(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_float(&self, section: &str, key: &str) -> Result<f32, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_float(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_fvector2(&self, section: &str, key: &str) -> Result<[f32; 2], SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_fvector2(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_fvector3(&self, section: &str, key: &str) -> Result<Vec3, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_fvector3(&s, key).map(Vec3::from).map_err(|_| self.missing(section, key))
    }
    pub fn r_fcolor(&self, section: &str, key: &str) -> Result<[f32; 4], SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_fcolor(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_color(&self, section: &str, key: &str) -> Result<u32, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_color(&s, key).map_err(|_| self.missing(section, key))
    }
    pub fn r_bool(&self, section: &str, key: &str) -> Result<bool, SceneError> {
        let s = section.to_lowercase();
        self.ltx.r_bool(&s, key).map_err(|_| self.missing(section, key))
    }

    /// Optional reads: `None` when the key is not there.
    pub fn opt_u32(&self, section: &str, key: &str) -> Option<u32> {
        self.line_exist(section, key).then(|| self.r_u32(section, key).ok()).flatten()
    }
    pub fn opt_string(&self, section: &str, key: &str) -> Option<String> {
        self.line_exist(section, key).then(|| self.r_string(section, key).ok()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_like_save_as() {
        let mut w = IniWriter::new();
        w.w_u32("Main", "objects_count", 2);
        w.w_fvector3("object_0", "position", Vec3::new(1.0, -2.5, 0.0));
        w.w_string("object_0", "name", "my box");
        w.w_key("snap_objects", "wall");
        let text = w.save();
        assert!(text.starts_with("[main]\r\n        objects_count                    = 2\r\n \r\n"));
        assert!(text.contains("        name                             = mybox\r\n"));
        assert!(text.contains("position                         = 1.000000, -2.500000, 0.000000\r\n"));
        assert!(text.contains("        wall                             =\r\n"));
        // the reader takes back what the writer gave
        let r = IniReader::parse(&text, "t").unwrap();
        assert_eq!(r.r_u32("main", "objects_count").unwrap(), 2);
        assert_eq!(r.r_fvector3("object_0", "position").unwrap(), Vec3::new(1.0, -2.5, 0.0));
        assert!(r.line_exist("snap_objects", "wall"));
    }

    #[test]
    fn negative_u32_and_quotes() {
        let mut w = IniWriter::new();
        w.w_u32("s", "f", 0x8000_0001);
        w.w_string("s", "q", "\"a b, c\"");
        let r = IniReader::parse(&w.save(), "t").unwrap();
        assert_eq!(r.r_u32("s", "f").unwrap(), 0x8000_0001);
        assert_eq!(r.r_string_wb("s", "q").unwrap(), "a b, c");
    }

    #[test]
    fn g_formats_like_c() {
        assert_eq!(fmt_g(20.0), "20");
        assert_eq!(fmt_g(3.5), "3.5");
        assert_eq!(fmt_g(0.1), "0.1");
        assert_eq!(fmt_g(1.0e7), "1e+07");
    }
}

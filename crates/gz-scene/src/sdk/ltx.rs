//! The X-Ray SDK's LTX files as its scene files use them: [`IniReader`] reads one with the SDK
//! reader's rules (`gz_ltx`, `Dialect::Editor`).

use gz_ltx::{Dialect, Ltx};
use rustc_hash::FxHashMap;

use crate::SceneError;
use crate::math::Vec3;

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
        let loaded = Ltx::assemble_map("scene", &map, Dialect::Editor)
            .map_err(|e| SceneError::load(format!("{e} ({file})")))?;
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
            .map(|s| {
                s.items
                    .iter()
                    .map(|it| (it.key.to_string(), it.value.as_deref().unwrap_or("").to_owned()))
                    .collect()
            })
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

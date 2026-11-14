//! The game's configuration as the entity constructors see it.
//!
//! This crate depends on no config crate: whoever holds the resolved `system.ltx` implements
//! [`Settings`] for it. [`MapSettings`] is a plain in-memory implementation for tests and
//! tools.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::packet::{atof, atoi};

/// Read access to the resolved `system.ltx`.
pub trait Settings {
    /// `section_exist`.
    fn section_exist(&self, section: &str) -> bool;

    /// The value of a key with comments and surrounding blanks removed, as `r_string` gives it.
    fn r_string(&self, section: &str, key: &str) -> Option<&str>;

    /// `line_exist`.
    fn line_exist(&self, section: &str, key: &str) -> bool {
        self.r_string(section, key).is_some()
    }

    /// The text of a file under `$game_config$`: the `custom_data` a section names.
    fn config_text(&self, _path: &str) -> Option<Cow<'_, str>> {
        None
    }

    /// `r_float`: C's `atof`.
    fn r_float(&self, section: &str, key: &str) -> Option<f32> {
        self.r_string(section, key).map(|v| atof(v) as f32)
    }

    /// `r_s32`: C's `atoi`.
    fn r_s32(&self, section: &str, key: &str) -> Option<i32> {
        self.r_string(section, key).map(atoi)
    }

    /// `r_u32`: `u32(atoi(..))`.
    fn r_u32(&self, section: &str, key: &str) -> Option<u32> {
        self.r_string(section, key).map(|v| atoi(v) as u32)
    }

    /// `r_u16`: `u16(atoi(..))`.
    fn r_u16(&self, section: &str, key: &str) -> Option<u16> {
        self.r_string(section, key).map(|v| atoi(v) as u16)
    }

    /// `r_bool`: `on`, `yes`, `true` and `1` are true.
    fn r_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.r_string(section, key).map(is_bool_true)
    }
}

/// Whether a value is a boolean.
#[must_use]
pub fn is_bool_true(text: &str) -> bool {
    ["on", "yes", "true", "1"].iter().any(|t| text.eq_ignore_ascii_case(t))
}

/// Sections in memory. Section and key names are matched as written.
#[derive(Clone, Debug, Default)]
pub struct MapSettings {
    sections: BTreeMap<String, BTreeMap<String, String>>,
    files: BTreeMap<String, String>,
}

impl MapSettings {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds keys to a section, creating it.
    #[must_use]
    pub fn section(mut self, name: &str, lines: &[(&str, &str)]) -> Self {
        let section = self.sections.entry(name.to_owned()).or_default();
        for (key, value) in lines {
            section.insert((*key).to_owned(), (*value).to_owned());
        }
        self
    }

    /// Adds a file under `$game_config$`.
    #[must_use]
    pub fn file(mut self, path: &str, text: &str) -> Self {
        self.files.insert(path.to_owned(), text.to_owned());
        self
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.sections.entry(section.to_owned()).or_default().insert(key.to_owned(), value.to_owned());
    }
}

impl Settings for MapSettings {
    fn section_exist(&self, section: &str) -> bool {
        self.sections.contains_key(section)
    }

    fn r_string(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }

    fn config_text(&self, path: &str) -> Option<Cow<'_, str>> {
        self.files.get(path).map(|t| Cow::Borrowed(t.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_reads_follow_c() {
        let s = MapSettings::new().section("a", &[("f", "1.5"), ("i", "-3"), ("b", "On"), ("n", "off")]);
        assert_eq!(s.r_float("a", "f"), Some(1.5));
        assert_eq!(s.r_s32("a", "i"), Some(-3));
        assert_eq!(s.r_u32("a", "i"), Some(u32::MAX - 2));
        assert_eq!(s.r_bool("a", "b"), Some(true));
        assert_eq!(s.r_bool("a", "n"), Some(false));
        assert!(s.line_exist("a", "f") && !s.line_exist("a", "x") && !s.line_exist("z", "f"));
        assert!(s.section_exist("a") && !s.section_exist("z"));
    }
}

//! Relative paths as the editor names files: forward slashes, no `..`, compared the way
//! Windows compares names.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("the path leaves its root: {0}")]
    Escapes(String),
    #[error("the path is absolute: {0}")]
    Absolute(String),
    #[error("the path holds a character no file name may have: {0}")]
    BadCharacter(String),
}

/// Why `name` is not ONE file-name component Windows takes as it is, or `None` when it is: no
/// separator, no `.` or `..`, no drive, no reserved device name (`CON`, `NUL`, `COM1`, ... with
/// or without an extension), no character a file name may not hold, no trailing dot or space.
#[must_use]
pub fn file_name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("is empty");
    }
    if name.contains(['/', '\\']) {
        return Some("holds a path separator: it names one folder, not a path");
    }
    if name == "." || name == ".." {
        return Some("is no folder name");
    }
    if name.contains(':') {
        return Some("holds a drive or a stream (':')");
    }
    if name.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        return Some("holds a character no file name may have");
    }
    if name.ends_with(['.', ' ']) {
        return Some("ends with a dot or a space, which Windows drops");
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let reserved =
        ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].iter().any(|r| stem.eq_ignore_ascii_case(r))
            || (stem.len() == 4
                && ["COM", "LPT"].iter().any(|p| stem.get(..3).is_some_and(|h| h.eq_ignore_ascii_case(p)))
                && stem.as_bytes().get(3).is_some_and(u8::is_ascii_digit));
    if reserved {
        return Some("is a name Windows reserves for a device");
    }
    None
}

/// A path below some root. The text keeps the case the author wrote; equality, order and hash
/// ignore it.
#[derive(Clone)]
pub struct RelPath {
    text: SmolStr,
    key: SmolStr,
}

impl RelPath {
    /// The root itself.
    #[must_use]
    pub fn root() -> Self {
        Self { text: SmolStr::default(), key: SmolStr::default() }
    }

    /// Normalises separators and `.` segments. Refuses `..`, drive letters and UNC prefixes.
    pub fn new(raw: &str) -> Result<Self, PathError> {
        if raw.starts_with('/') || raw.starts_with('\\') {
            return Err(PathError::Absolute(raw.to_owned()));
        }
        let mut out = String::with_capacity(raw.len());
        for seg in raw.split(['/', '\\']) {
            match seg {
                "" | "." => continue,
                ".." => return Err(PathError::Escapes(raw.to_owned())),
                _ => {}
            }
            if seg.contains(':') {
                return Err(PathError::Absolute(raw.to_owned()));
            }
            if seg.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
                return Err(PathError::BadCharacter(raw.to_owned()));
            }
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(seg);
        }
        Ok(Self::from_normal(out))
    }

    fn from_normal(text: String) -> Self {
        let key = if text.chars().any(char::is_uppercase) {
            SmolStr::new(text.to_lowercase())
        } else {
            SmolStr::new(&text)
        };
        Self { text: SmolStr::new(text), key }
    }

    /// The path of `abs` below `root`, when it is below it.
    #[must_use]
    pub fn from_fs(root: &Path, abs: &Path) -> Option<Self> {
        let rel = abs.strip_prefix(root).ok()?;
        Self::new(rel.to_str()?).ok()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The lower-case form the path is compared by.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.text.is_empty()
    }

    pub fn segments(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.text.split('/').filter(|s| !s.is_empty())
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.segments().count()
    }

    /// Appends a relative path.
    pub fn join(&self, tail: &str) -> Result<Self, PathError> {
        let tail = Self::new(tail)?;
        Ok(self.join_path(&tail))
    }

    #[must_use]
    pub fn join_path(&self, tail: &RelPath) -> Self {
        if self.is_root() {
            return tail.clone();
        }
        if tail.is_root() {
            return self.clone();
        }
        Self::from_normal(format!("{}/{}", self.text, tail.text))
    }

    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.text.rfind('/') {
            Some(i) => Self::from_normal(self.text[..i].to_owned()),
            None => Self::root(),
        })
    }

    /// The last segment; empty for the root.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.text.rsplit('/').next().unwrap_or_default()
    }

    /// The file name without its last extension. A leading dot is part of the name.
    #[must_use]
    pub fn stem(&self) -> &str {
        let name = self.file_name();
        match name.rfind('.') {
            Some(0) | None => name,
            Some(i) => &name[..i],
        }
    }

    /// The last extension without its dot, as written.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        match name.rfind('.') {
            Some(0) | None => None,
            Some(i) => Some(&name[i + 1..]),
        }
    }

    /// Whether the extension is `ext` (given without a dot), ignoring case.
    #[must_use]
    pub fn has_extension(&self, ext: &str) -> bool {
        self.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
    }

    #[must_use]
    pub fn with_extension(&self, ext: &str) -> Self {
        let name_start = self.text.rfind('/').map_or(0, |i| i + 1);
        let stem_end = name_start + self.stem().len();
        let mut text = self.text[..stem_end].to_owned();
        if !ext.is_empty() {
            text.push('.');
            text.push_str(ext);
        }
        Self::from_normal(text)
    }

    /// Whether `self` is `base` or lies below it. Whole segments only.
    #[must_use]
    pub fn starts_with(&self, base: &RelPath) -> bool {
        if base.is_root() {
            return true;
        }
        self.key.strip_prefix(base.key.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// The part of `self` below `base`.
    #[must_use]
    pub fn strip_prefix(&self, base: &RelPath) -> Option<Self> {
        if !self.starts_with(base) {
            return None;
        }
        if base.is_root() {
            return Some(self.clone());
        }
        let rest = &self.text[base.text.len()..];
        Some(Self::from_normal(rest.trim_start_matches('/').to_owned()))
    }

    /// The path on disk below `root`.
    #[must_use]
    pub fn to_fs(&self, root: &Path) -> PathBuf {
        let mut out = root.to_path_buf();
        for seg in self.segments() {
            out.push(seg);
        }
        out
    }

    /// The form the game's files use.
    #[must_use]
    pub fn to_backslash(&self) -> String {
        self.text.replace('/', "\\")
    }
}

impl PartialEq for RelPath {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for RelPath {}
impl Hash for RelPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}
impl PartialOrd for RelPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RelPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}
impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}
impl fmt::Debug for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.text.as_str())
    }
}
impl Default for RelPath {
    fn default() -> Self {
        Self::root()
    }
}
impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text)
    }
}
impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        RelPath::new(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_and_compares_without_case() {
        let a = RelPath::new("Quests\\.\\Main//Intro.nqasset").unwrap();
        assert_eq!(a.as_str(), "Quests/Main/Intro.nqasset");
        assert_eq!(a, RelPath::new("quests/main/intro.NQASSET").unwrap());
        assert_eq!(a.file_name(), "Intro.nqasset");
        assert_eq!(a.stem(), "Intro");
        assert!(a.has_extension("NqAsset"));
        assert_eq!(a.parent().unwrap().as_str(), "Quests/Main");
        assert!(a.starts_with(&RelPath::new("QUESTS").unwrap()));
        assert!(!a.starts_with(&RelPath::new("Quest").unwrap()));
        assert_eq!(a.strip_prefix(&RelPath::new("quests").unwrap()).unwrap().as_str(), "Main/Intro.nqasset");
    }

    #[test]
    fn refuses_what_leaves_the_root() {
        assert!(RelPath::new("a/../b").is_err());
        assert!(RelPath::new("C:/a").is_err());
        assert!(RelPath::new("\\\\server\\share").is_err());
        assert!(RelPath::new("/abs").is_err());
        assert!(RelPath::new("").unwrap().is_root());
    }

    #[test]
    fn dot_names_keep_their_name() {
        let p = RelPath::new(".git/config").unwrap();
        assert_eq!(p.parent().unwrap().stem(), ".git");
        assert_eq!(p.parent().unwrap().extension(), None);
        assert_eq!(RelPath::new("route.v2.behasset").unwrap().stem(), "route.v2");
        assert_eq!(RelPath::new("a/b.tga").unwrap().with_extension("dds").as_str(), "a/b.dds");
    }

    #[test]
    fn one_file_name_component() {
        for ok in ["l01_escape", "level", "Агропром", "a.b", "com10", "lpt", "x y"] {
            assert_eq!(file_name_problem(ok), None, "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "a\\b",
            "a/b",
            "..\\..\\x",
            "C:",
            "C:\\Windows",
            "\\\\server\\share",
            "nul",
            "CON.txt",
            "com1",
            "LPT9.level",
            "a?b",
            "a*",
            "a.",
            "a ",
            "tab\t",
        ] {
            assert!(file_name_problem(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn cyrillic_names_compare_without_case() {
        assert_eq!(RelPath::new("Квесты/Начало").unwrap(), RelPath::new("квесты/начало").unwrap());
    }
}

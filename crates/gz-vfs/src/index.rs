//! A sorted, immutable `path -> where` table with its paths in one string.
//!
//! The order is the C++ editor's: its keys had backslashes, so `/` sorts as `\` would
//! (`a1.ogg` before `a/x.ogg`), and listings come out in the order they always did.

use std::cmp::Ordering;

use crate::path::normalize;

/// Compares two keys as their backslash spellings compare.
fn cmp_key(a: &str, b: &str) -> Ordering {
    let map = |c: u8| if c == b'/' { b'\\' } else { c };
    a.bytes().map(map).cmp(b.bytes().map(map))
}

/// Where a file's bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A file of the loose tree.
    Loose,
    /// Inside an archive: its place in [`crate::GameFiles::archives`], where the stored bytes
    /// start and how many there are.
    Archive { archive: u32, offset: u32, packed: u32 },
}

/// A file of the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRef<'a> {
    /// The key: lower case, forward slashes, relative to the tree's root.
    pub path: &'a str,
    /// The unpacked size.
    pub size: u64,
    pub source: Source,
}

impl FileRef<'_> {
    /// `"archive"` or `"loose"`, as the DARF listing names it.
    #[must_use]
    pub fn source_name(&self) -> &'static str {
        match self.source {
            Source::Archive { .. } => "archive",
            Source::Loose => "loose",
        }
    }

    /// The file's name, its folders left out.
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }
}

/// An entry of a folder listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed<'a> {
    /// A folder below, by its name (lower case).
    Dir(&'a str),
    File(FileRef<'a>),
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    at: usize,
    len: usize,
    size: u64,
    source: Source,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    text: String,
    slots: Vec<Slot>,
}

/// The extension a `find` asks for: no dot, lower case; `""` is any.
fn ext_key(ext: &str) -> String {
    ext.strip_prefix('.').unwrap_or(ext).to_lowercase()
}

fn has_ext(path: &str, ext: &str) -> bool {
    if ext.is_empty() {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(_, e)| e == ext)
}

impl Index {
    /// From `(key, size, source)` with distinct keys, in any order.
    pub fn build(mut items: Vec<(String, u64, Source)>) -> Self {
        items.sort_unstable_by(|a, b| cmp_key(&a.0, &b.0));
        items.dedup_by(|b, a| a.0 == b.0);
        let mut text = String::with_capacity(items.iter().map(|i| i.0.len()).sum());
        let mut slots = Vec::with_capacity(items.len());
        for (path, size, source) in items {
            slots.push(Slot { at: text.len(), len: path.len(), size, source });
            text.push_str(&path);
        }
        Self { text, slots }
    }

    fn path(&self, s: &Slot) -> &str {
        self.text.get(s.at..s.at + s.len).unwrap_or("")
    }

    fn file(&self, s: &Slot) -> FileRef<'_> {
        FileRef { path: self.path(s), size: s.size, source: s.source }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = FileRef<'_>> + '_ {
        self.slots.iter().map(|s| self.file(s))
    }

    /// A file by its key.
    pub fn get_key(&self, key: &str) -> Option<FileRef<'_>> {
        let i = self.slots.binary_search_by(|s| cmp_key(self.path(s), key)).ok()?;
        self.slots.get(i).map(|s| self.file(s))
    }

    pub fn get(&self, path: &str) -> Option<FileRef<'_>> {
        self.get_key(&normalize(path))
    }

    /// The slots below the folder `dir_key` (`""` for the root), and the prefix they share.
    fn below(&self, dir_key: &str) -> (&[Slot], String) {
        let prefix = if dir_key.is_empty() { String::new() } else { format!("{dir_key}/") };
        let start = self.slots.partition_point(|s| cmp_key(self.path(s), &prefix) == Ordering::Less);
        let rest = self.slots.get(start..).unwrap_or(&[]);
        let n = rest.partition_point(|s| self.path(s).starts_with(prefix.as_str()));
        (rest.get(..n).unwrap_or(&[]), prefix)
    }

    pub fn is_dir(&self, dir: &str) -> bool {
        let key = normalize(dir);
        key.is_empty() || !self.below(&key).0.is_empty()
    }

    /// What a folder holds: its sub-folders (by name, once each) and its files, in key order.
    pub fn list(&self, dir: &str) -> Vec<Listed<'_>> {
        let (slots, prefix) = self.below(&normalize(dir));
        let mut out = Vec::new();
        let mut last_dir: Option<&str> = None;
        for s in slots {
            let path = self.path(s);
            let rest = path.get(prefix.len()..).unwrap_or("");
            match rest.split_once('/') {
                Some((sub, _)) => {
                    if last_dir != Some(sub) {
                        out.push(Listed::Dir(sub));
                        last_dir = Some(sub);
                    }
                }
                None => out.push(Listed::File(self.file(s))),
            }
        }
        out
    }

    /// Every file below a folder, at any depth, with the extension `ext` (`"xml"` or `".xml"`,
    /// any case; `""` for every file), in key order.
    pub fn find(&self, dir: &str, ext: &str) -> Vec<FileRef<'_>> {
        let ext = ext_key(ext);
        let (slots, _) = self.below(&normalize(dir));
        slots.iter().map(|s| self.file(s)).filter(|f| has_ext(f.path, &ext)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        Index::build(vec![
            ("configs/system.ltx".into(), 10, Source::Loose),
            ("configs/text/eng/a.xml".into(), 1, Source::Loose),
            ("configs/text/rus/b.xml".into(), 2, Source::Archive { archive: 0, offset: 8, packed: 2 }),
            ("configs/text.ltx".into(), 3, Source::Loose),
            ("configs/text/rus/c.XML.bak".into(), 4, Source::Loose),
            ("meshes/a.ogf".into(), 5, Source::Loose),
        ])
    }

    #[test]
    fn lookups() {
        let ix = sample();
        assert_eq!(ix.len(), 6);
        assert_eq!(ix.get("CONFIGS\\System.ltx").map(|f| f.size), Some(10));
        assert!(ix.get("configs").is_none());
        assert!(ix.is_dir("configs/text"));
        assert!(ix.is_dir(""));
        assert!(!ix.is_dir("configs/tex"));
    }

    #[test]
    fn listing_and_finding() {
        let ix = sample();
        let names: Vec<String> = ix
            .list("configs")
            .iter()
            .map(|l| match l {
                Listed::Dir(d) => format!("{d}/"),
                Listed::File(f) => f.name().to_owned(),
            })
            .collect();
        // '.' sorts before '/': the file comes first
        assert_eq!(names, ["system.ltx", "text.ltx", "text/"]);
        let xml: Vec<&str> = ix.find("configs/text", ".XML").iter().map(|f| f.path).collect();
        assert_eq!(xml, ["configs/text/eng/a.xml", "configs/text/rus/b.xml"]);
        assert_eq!(ix.find("", "").len(), 6);
        assert!(ix.find("nothing", "").is_empty());
    }

    #[test]
    fn backslash_order() {
        let ix = Index::build(vec![("s/a/x.ogg".into(), 1, Source::Loose), ("s/a1.ogg".into(), 2, Source::Loose), ("s/a.ogg".into(), 3, Source::Loose)]);
        let keys: Vec<&str> = ix.iter().map(|f| f.path).collect();
        assert_eq!(keys, ["s/a.ogg", "s/a1.ogg", "s/a/x.ogg"]);
        assert_eq!(ix.get("s/a1.ogg").map(|f| f.size), Some(2));
        assert_eq!(ix.find("s/a", "").len(), 1);
        assert_eq!(ix.list("s").len(), 3);
    }
}

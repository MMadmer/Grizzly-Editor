//! The game's `system.ltx` tree as the editor reads it (`EditorGameConfigs::Activate`, which
//! unpacked the configs to disk and handed them to the editor's `CInifile`). Here nothing is
//! unpacked: the includes are discovered breadth-first, every level read and parsed in
//! parallel straight from the index, then assembled in the engine's order.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::text::cp1251_to_utf8;
use gz_ltx::{Dialect, Loader, Ltx, LtxWarning, ParsedFile, include_path, parse_file};
use gz_vfs::{GameFiles, normalize};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// The root of the tree, as the engine names it.
pub const SYSTEM_LTX: &str = "configs\\system.ltx";

/// Where the bytes of game files come from: the linked game's index, or a folder on disk (the
/// editor's bundled copy).
pub trait FileSource: Sync {
    /// The bytes of each key ([`normalize`]d path), in order; `None` for a file that is not there
    /// or does not read. Parallel on the caller's pool.
    fn read_keys(&self, keys: &[String]) -> Vec<Option<Vec<u8>>>;
}

impl FileSource for GameFiles {
    fn read_keys(&self, keys: &[String]) -> Vec<Option<Vec<u8>>> {
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.read_many(&refs).into_iter().map(Result::ok).collect()
    }
}

/// A folder on disk read as a game's `gamedata`.
#[derive(Clone, Debug)]
pub struct DiskFolder(pub PathBuf);

impl FileSource for DiskFolder {
    fn read_keys(&self, keys: &[String]) -> Vec<Option<Vec<u8>>> {
        keys.par_iter().map(|k| std::fs::read(self.0.join(k)).ok()).collect()
    }
}

/// A loaded tree and what the reader read past.
#[derive(Clone, Debug)]
pub struct ConfigTree {
    pub ltx: Arc<Ltx>,
    pub warnings: Vec<LtxWarning>,
    /// How many files it was read from.
    pub files: usize,
}

struct Parsed(FxHashMap<String, ParsedFile>);

impl Loader for Parsed {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        self.0.get(&normalize(path))
    }
}

/// Reads `root` and every file it includes from `source`, and assembles them with the editor's
/// `CInifile` rules (a missing include is skipped with a warning). `io` runs the reads (the IO
/// pool); parsing runs on the caller's pool. `missing` is the error when `root` is not there.
pub fn load_tree(
    source: &dyn FileSource,
    root: &str,
    io: Option<&rayon::ThreadPool>,
    missing: &str,
) -> Result<ConfigTree, String> {
    let read = |keys: &[String]| match io {
        Some(pool) => pool.install(|| source.read_keys(keys)),
        None => source.read_keys(keys),
    };
    let root_key = normalize(root);
    let mut parsed: FxHashMap<String, ParsedFile> = FxHashMap::default();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    seen.insert(root_key.clone());
    let mut frontier = vec![root_key.clone()];
    while !frontier.is_empty() {
        let bytes = read(&frontier);
        let level: Vec<(String, ParsedFile)> = frontier
            .into_par_iter()
            .zip(bytes)
            .filter_map(|(key, bytes)| bytes.map(|b| (key, parse_file(&cp1251_to_utf8(&b), Dialect::Editor))))
            .collect();
        let mut next = Vec::new();
        for (key, file) in &level {
            for inc in file.includes() {
                let target = normalize(&include_path(key, &inc.name));
                if seen.insert(target.clone()) {
                    next.push(target);
                }
            }
        }
        parsed.extend(level);
        frontier = next;
    }
    if !parsed.contains_key(&root_key) {
        return Err(missing.to_owned());
    }
    let files = parsed.len();
    let loaded = Ltx::assemble(root, &Parsed(parsed), Dialect::Editor).map_err(|e| e.to_string())?;
    Ok(ConfigTree { ltx: Arc::new(loaded.ltx), warnings: loaded.warnings, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Map(FxHashMap<String, Vec<u8>>);

    impl FileSource for Map {
        fn read_keys(&self, keys: &[String]) -> Vec<Option<Vec<u8>>> {
            keys.iter().map(|k| self.0.get(k).cloned()).collect()
        }
    }

    fn source(files: &[(&str, &[u8])]) -> Map {
        Map(files.iter().map(|(k, v)| (normalize(k), v.to_vec())).collect())
    }

    #[test]
    fn includes_breadth_first() {
        let src = source(&[
            (
                "configs/system.ltx",
                b"#include \"Weapons\\weapons.ltx\"\n#include \"missing.ltx\"\n[root]\na = 1\n",
            ),
            ("configs/weapons/weapons.ltx", b"#include \"..\\items\\knife.ltx\"\n[wpn]:root\nb = \xc0\n"),
            ("configs/items/knife.ltx", b"[knife]\nc = 3\n"),
        ]);
        let tree = load_tree(&src, SYSTEM_LTX, None, "no root").unwrap();
        assert_eq!(tree.files, 3);
        assert_eq!(tree.ltx.r_string("wpn", "a").unwrap(), "1");
        assert_eq!(tree.ltx.r_string("wpn", "b").unwrap(), "А");
        assert_eq!(tree.ltx.r_string("knife", "c").unwrap(), "3");
        assert!(matches!(tree.warnings.as_slice(), [LtxWarning::IncludeSkipped { .. }]));
    }

    #[test]
    fn a_missing_root_is_the_callers_error() {
        let src = source(&[]);
        assert_eq!(
            load_tree(&src, SYSTEM_LTX, None, "the linked game has no configs\\system.ltx").unwrap_err(),
            "the linked game has no configs\\system.ltx"
        );
    }

    #[test]
    fn include_cycles_are_read_once() {
        let src = source(&[
            ("configs/system.ltx", b"#include \"a.ltx\"\n[s]\n"),
            ("configs/a.ltx", b"#include \"system.ltx\"\n[t]\n"),
        ]);
        // the engine refuses the loop; discovery itself must end
        assert!(load_tree(&src, SYSTEM_LTX, None, "x").is_err());
    }
}

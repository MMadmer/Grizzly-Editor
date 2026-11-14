//! The object library: `.object` files by reference name, loaded on demand,
//! parsed once and shared. The project's own `rawdata\objects` comes before the editor's.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Rev, RevCounter, SmolStr};
use gz_fmt_level::Bvh;
use gz_fmt_object::MeshFlags;
use gz_fmt_object::Object as EditObject;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::math::{Aabb, Vec3};

/// A library object as the scene uses it: the model, its box and its picking tree.
pub struct LibraryObject {
    pub name: SmolStr,
    pub file: PathBuf,
    pub object: EditObject,
    /// The object's bounding box, in object space.
    pub bounds: Aabb,
    /// The visible meshes' triangles, object space, for picking.
    pub verts: Arc<[Vec3]>,
    pub tris: Arc<[[u32; 3]]>,
    pub bvh: Option<Bvh>,
    /// The file's bytes as one number, the same on every machine; 0 for an object made in
    /// memory. What a build compares to know the model changed.
    pub stamp: u64,
}

impl std::fmt::Debug for LibraryObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryObject")
            .field("name", &self.name)
            .field("tris", &self.tris.len())
            .finish_non_exhaustive()
    }
}

impl LibraryObject {
    /// Builds the picking data of a read `.object`.
    #[must_use]
    pub fn from_object(name: &str, file: PathBuf, object: EditObject) -> Self {
        let mut verts: Vec<Vec3> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for m in object.meshes.iter().filter(|m| m.flags.contains(MeshFlags::VISIBLE)) {
            let base = u32::try_from(verts.len()).unwrap_or(u32::MAX);
            verts.extend(m.points.iter().copied());
            for f in m.faces.iter() {
                tris.push([base + f.v[0].point, base + f.v[1].point, base + f.v[2].point]);
            }
        }
        let bvh = Bvh::build(&verts, &tris).ok();
        let b = object.bounding_box();
        let bounds = Aabb::new(b.min, b.max);
        Self {
            name: SmolStr::new(name),
            file,
            object,
            bounds,
            verts: verts.into(),
            tris: tris.into(),
            bvh,
            stamp: 0,
        }
    }

    /// A dynamic object cannot be scaled.
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        self.object.is_dynamic()
    }

    /// The triangle `i` in object space.
    #[must_use]
    pub fn tri(&self, i: u32) -> Option<[Vec3; 3]> {
        let t = self.tris.get(i as usize)?;
        Some([
            *self.verts.get(t[0] as usize)?,
            *self.verts.get(t[1] as usize)?,
            *self.verts.get(t[2] as usize)?,
        ])
    }
}

/// The reference name as the library keys it: lower case, backslashes, no `.object`.
#[must_use]
pub fn normalize(name: &str) -> String {
    let n = name.replace('/', "\\").to_lowercase();
    n.strip_suffix(".object").map_or(n.clone(), str::to_owned)
}

enum Slot {
    Loaded(Arc<LibraryObject>),
    Failed(String),
}

/// The library: the folders it reads, and what it has read.
pub struct Library {
    roots: Vec<PathBuf>,
    cache: Mutex<FxHashMap<String, Slot>>,
    listing: Mutex<Option<Arc<Vec<String>>>>,
    rev: RevCounter,
}

impl Library {
    /// `roots` in the order they win: the project's `rawdata\objects`, then the editor's.
    #[must_use]
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            cache: Mutex::new(FxHashMap::default()),
            listing: Mutex::new(None),
            rev: RevCounter::new(),
        }
    }

    /// Moves whenever an object is loaded: what a pick tree over the scene's boxes depends on.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.rev.get()
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// What is loaded, without touching a file.
    #[must_use]
    pub fn cached(&self, name: &str) -> Option<Arc<LibraryObject>> {
        match self.cache.lock().get(&normalize(name)) {
            Some(Slot::Loaded(o)) => Some(o.clone()),
            _ => None,
        }
    }

    /// The file a reference names, in the first root that has it. Blocking.
    #[must_use]
    pub fn file_of(&self, name: &str) -> Option<PathBuf> {
        let rel = format!("{}.object", normalize(name));
        self.roots.iter().map(|r| r.join(&rel)).find(|p| p.is_file())
    }

    /// A reference loaded once and shared. Blocking: call it on the IO pool.
    pub fn load(&self, name: &str) -> Result<Arc<LibraryObject>, String> {
        let key = normalize(name);
        match self.cache.lock().get(&key) {
            Some(Slot::Loaded(o)) => return Ok(o.clone()),
            Some(Slot::Failed(e)) => return Err(e.clone()),
            None => {}
        }
        let result = self.read(&key);
        let mut cache = self.cache.lock();
        match &result {
            Ok(o) => {
                cache.insert(key, Slot::Loaded(o.clone()));
                self.rev.bump();
            }
            Err(e) => {
                cache.insert(key, Slot::Failed(e.clone()));
            }
        }
        result
    }

    fn read(&self, key: &str) -> Result<Arc<LibraryObject>, String> {
        let file = self.file_of(key).ok_or_else(|| format!("Can't find file '{key}.object'"))?;
        let bytes = std::fs::read(&file).map_err(|e| format!("Can't read '{}': {e}", file.display()))?;
        let object = EditObject::read(&bytes).map_err(|e| format!("Can't load '{}': {e}", file.display()))?;
        let mut made = LibraryObject::from_object(key, file, object);
        // FNV-1a over the file: stable, and cheap beside reading the model
        made.stamp = bytes
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
        Ok(Arc::new(made))
    }

    /// Loads many references at once on the rayon pool this is called in. Blocking.
    pub fn prefetch(&self, names: &[SmolStr]) {
        use rayon::prelude::*;
        let missing: Vec<&SmolStr> = {
            let cache = self.cache.lock();
            names.iter().filter(|n| !n.is_empty() && !cache.contains_key(&normalize(n))).collect()
        };
        missing.par_iter().for_each(|n| {
            let _ = self.load(n);
        });
    }

    /// Forgets what was read: the files changed.
    pub fn clear(&self) {
        self.cache.lock().clear();
        *self.listing.lock() = None;
        self.rev.bump();
    }

    /// Every reference name the roots hold, sorted, each once. Read once and kept
    /// until [`Self::clear`]. Blocking.
    pub fn list(&self) -> Arc<Vec<String>> {
        if let Some(l) = self.listing.lock().as_ref() {
            return l.clone();
        }
        let mut names: Vec<String> = Vec::new();
        for root in &self.roots {
            collect(root, root, &mut names);
        }
        names.sort();
        names.dedup();
        let names = Arc::new(names);
        *self.listing.lock() = Some(names.clone());
        names
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if gz_project::is_tooling_entry(&name) {
            continue;
        }
        if p.is_dir() {
            collect(root, &p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("object"))
            && let Ok(rel) = p.with_extension("").strip_prefix(root)
        {
            out.push(normalize(&rel.to_string_lossy()));
        }
    }
}

/// `$groups$` (`rawdata\groups`): the `.group` files Groups lists.
#[must_use]
pub fn list_groups(root: &Path) -> Vec<String> {
    // as `FS.file_list($groups$)`: lower case, in byte order
    gz_project::editor_file_list(root, "group")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized() {
        assert_eq!(normalize("Props/Box_1.object"), "props\\box_1");
        assert_eq!(normalize("props\\box_1"), "props\\box_1");
    }

    #[test]
    fn a_missing_reference_fails_once_and_is_remembered() {
        let lib = Library::new(vec![PathBuf::from("Z:\\nowhere")]);
        assert!(lib.load("a\\b").is_err());
        assert!(lib.load("a\\b").is_err());
        assert!(lib.cached("a\\b").is_none());
    }
}

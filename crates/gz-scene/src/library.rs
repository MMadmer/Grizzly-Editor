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
    /// Per triangle of `tris`: its surface, an index into `object.surfaces` ([`NO_SURFACE`]
    /// for a face no surface lists).
    pub tri_surface: Arc<[u32]>,
    pub bvh: Option<Bvh>,
    /// The file's bytes as one number, the same on every machine; 0 for an object made in
    /// memory. What a build compares to know the model changed.
    pub stamp: u64,
    /// For a model the module build publishes as a model of its own (a dynamic model asset):
    /// the name the game knows it by. `None` for everything else.
    pub published: Option<SmolStr>,
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
        let mut tri_surface: Vec<u32> = Vec::new();
        for m in object.meshes.iter().filter(|m| m.flags.contains(MeshFlags::VISIBLE)) {
            let base = u32::try_from(verts.len()).unwrap_or(u32::MAX);
            let first = tris.len();
            verts.extend(m.points.iter().copied());
            for f in m.faces.iter() {
                tris.push([base + f.v[0].point, base + f.v[1].point, base + f.v[2].point]);
            }
            tri_surface.resize(tris.len(), NO_SURFACE);
            for sf in &m.surf_faces {
                for face in sf.faces.iter() {
                    if let Some(slot) = tri_surface.get_mut(first + *face as usize) {
                        *slot = sf.surface;
                    }
                }
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
            tri_surface: tri_surface.into(),
            bvh,
            stamp: 0,
            published: None,
        }
    }

    /// A library object made in memory from a model (a model asset, a terrain): every mesh's
    /// box is worked out from its points first, so a model whose boxes were never filled in has
    /// its true bounds. What a provider ([`Library::add_provider`]) answers.
    #[must_use]
    pub fn provided(name: &str, mut object: EditObject) -> Self {
        for m in &mut object.meshes {
            if !m.points.is_empty() {
                let mut b = gz_fmt_object::Aabb::INVALID;
                for p in m.points.iter() {
                    b.modify(*p);
                }
                m.bbox = b;
            }
        }
        Self::from_object(name, PathBuf::new(), object)
    }

    /// The surface of the triangle `i` (name, texture, shaders, game material).
    #[must_use]
    pub fn surface_of(&self, i: u32) -> Option<&gz_fmt_object::Surface> {
        let s = *self.tri_surface.get(i as usize)?;
        self.object.surfaces.get(s as usize)
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

/// What stands between a reference and its stamp in a [`Library::mesh_key`]: a character no
/// file name and no reference holds.
pub const MESH_STAMP: char = '|';

/// Makes a project's provider: called once for every scene service, as it makes its library,
/// with the editor and the project's root. `None` when it has nothing for that project.
pub type ProviderMaker = Arc<dyn Fn(&Arc<gz_editor::Editor>, &Path) -> Option<Provider> + Send + Sync>;

/// The provider makers of this editor: a crate whose models are no `.object` files (model
/// assets) adds its maker here when it installs, and every project's scene gets its provider.
#[derive(Default)]
pub struct ProviderMakers {
    list: parking_lot::RwLock<Vec<ProviderMaker>>,
}

impl ProviderMakers {
    /// The editor's list.
    #[must_use]
    pub fn of(editor: &gz_editor::Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Adds a maker; scenes opened from now on get its provider.
    pub fn add(&self, maker: ProviderMaker) {
        self.list.write().push(maker);
    }

    /// Gives a new library every maker's provider for the project at `root`.
    pub fn provide(&self, editor: &Arc<gz_editor::Editor>, root: &Path, library: &Library) {
        let makers: Vec<ProviderMaker> = self.list.read().clone();
        for provider in makers.iter().filter_map(|m| m(editor, root)) {
            library.add_provider(provider);
        }
    }
}

/// The triangle of no surface in [`LibraryObject::tri_surface`].
pub const NO_SURFACE: u32 = u32::MAX;

/// A source of library objects that are no `.object` file: it answers a reference it knows with
/// its object, and `None` for every other.
pub type Provider = Arc<dyn Fn(&str) -> Option<Arc<LibraryObject>> + Send + Sync>;

/// The library: the folders it reads, and what it has read.
///
/// **Providers.** Before a reference is looked for in the roots, every provider added with
/// [`Library::add_provider`] is asked in the order they were added, with the reference
/// normalized ([`normalize`]: lower case, backslashes, no `.object`); the first answer wins.
/// That is how models that are no `.object` take part: a model asset (a reference ending in
/// `.meshasset`) or a terrain (`@terrain\<name>`) is made by its provider, usually with
/// [`LibraryObject::provided`]. A provided object is cached, counted in [`Library::rev`],
/// answered by [`Library::cached`] and [`Library::load`] and stands in the pick tree exactly
/// like one read from a file. A provider may block (it runs where `load` runs, on the IO pool);
/// when what it answers changes, its owner calls [`Library::forget`] with the reference.
pub struct Library {
    roots: Vec<PathBuf>,
    providers: parking_lot::RwLock<Vec<Provider>>,
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
            providers: parking_lot::RwLock::new(Vec::new()),
            cache: Mutex::new(FxHashMap::default()),
            listing: Mutex::new(None),
            rev: RevCounter::new(),
        }
    }

    /// Adds a provider, asked after the ones added before it and before the roots.
    pub fn add_provider(&self, provider: Provider) {
        self.providers.write().push(provider);
    }

    /// Drops one reference from what was read (loaded or failed), so the next load reads it
    /// again: its file or its provider's answer changed. Moves [`Self::rev`] when something
    /// was dropped; answers whether it was.
    pub fn forget(&self, reference: &str) -> bool {
        let gone = self.cache.lock().remove(&normalize(reference)).is_some();
        if gone {
            self.rev.bump();
        }
        gone
    }

    /// What a changed file of the roots means: an `.object` under a root is forgotten (and the
    /// listing read again). Answers whether the file was one of the library's.
    pub fn forget_file(&self, file: &Path) -> bool {
        if !file.extension().is_some_and(|x| x.eq_ignore_ascii_case("object")) {
            return false;
        }
        let Some(rel) = self.roots.iter().find_map(|r| file.strip_prefix(r).ok()) else { return false };
        let reference = normalize(&rel.to_string_lossy());
        *self.listing.lock() = None;
        self.forget(&reference);
        true
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

    /// The object a provider answers for a reference: a model that is no file. `None` for a
    /// reference no provider knows - the roots are not asked. Loaded once and shared like any
    /// other. Blocking: call it on the IO pool.
    pub fn provided(&self, name: &str) -> Option<Arc<LibraryObject>> {
        let key = normalize(name);
        if let Some(slot) = self.cache.lock().get(&key) {
            return match slot {
                Slot::Loaded(o) if o.file.as_os_str().is_empty() => Some(o.clone()),
                _ => None,
            };
        }
        let providers: Vec<Provider> = self.providers.read().clone();
        let made = providers.iter().find_map(|p| p(&key))?;
        let mut cache = self.cache.lock();
        if let Some(Slot::Loaded(o)) = cache.get(&key) {
            return Some(o.clone());
        }
        cache.insert(key, Slot::Loaded(made.clone()));
        self.rev.bump();
        Some(made)
    }

    /// The name the renderer keeps a reference's mesh under: the reference itself for a file,
    /// and for an object made in memory the reference with its stamp, so a model whose source
    /// changed is a new mesh and the old one is simply no longer asked for.
    #[must_use]
    pub fn mesh_key(&self, name: &str) -> String {
        match self.cached(name) {
            Some(o) if o.file.as_os_str().is_empty() => format!("{name}{MESH_STAMP}{:016x}", o.stamp),
            _ => name.to_owned(),
        }
    }

    /// The reference of a [`Self::mesh_key`].
    #[must_use]
    pub fn reference_of(mesh_key: &str) -> &str {
        mesh_key.rsplit_once(MESH_STAMP).map_or(mesh_key, |(r, _)| r)
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
        // another thread loaded it meanwhile: keep the one there, the revision moved for it
        if let Some(Slot::Loaded(o)) = cache.get(&key) {
            return Ok(o.clone());
        }
        match &result {
            Ok(o) => {
                cache.insert(key, Slot::Loaded(o.clone()));
                self.rev.bump();
            }
            Err(e) => {
                // what a provider makes from the scene itself (`@terrain\...`, `@cut\...`) may
                // only not be there yet - asked for before the scene that holds it was published:
                // such a miss is not remembered, the next asker gets it made
                if !key.starts_with('@') {
                    cache.insert(key, Slot::Failed(e.clone()));
                }
            }
        }
        result
    }

    fn read(&self, key: &str) -> Result<Arc<LibraryObject>, String> {
        let providers: Vec<Provider> = self.providers.read().clone();
        if let Some(o) = providers.iter().find_map(|p| p(key)) {
            return Ok(o);
        }
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
    // lower case, in byte order, like every list of the editor's files
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

    #[test]
    fn a_miss_of_a_model_the_scene_makes_is_not_remembered() {
        let lib = Library::new(vec![PathBuf::from("Z:\\nowhere")]);
        let there = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let t = there.clone();
        lib.add_provider(Arc::new(move |name: &str| {
            (name == "@terrain\\yard" && t.load(std::sync::atomic::Ordering::Relaxed))
                .then(|| Arc::new(LibraryObject::provided(name, EditObject::default())))
        }));
        // asked for before its scene holds it: not there, and not held against it
        assert!(lib.load("@terrain\\yard").is_err());
        there.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(lib.load("@terrain\\yard").is_ok());
        assert!(lib.cached("@terrain\\yard").is_some());
        // a file that is not there stays not there until it is forgotten
        assert!(lib.load("props\\box").is_err());
        assert!(lib.load("props\\box").is_err_and(|e| e.contains("Can't find")));
    }

    #[test]
    fn an_object_made_in_memory_has_a_mesh_key_that_follows_its_stamp() {
        let lib = Library::new(vec![PathBuf::from("Z:\\nowhere")]);
        let stamp = Arc::new(std::sync::atomic::AtomicU64::new(7));
        let s = stamp.clone();
        lib.add_provider(Arc::new(move |name: &str| {
            (name == "models\\wall.meshasset").then(|| {
                let mut o = LibraryObject::provided(name, EditObject::default());
                o.stamp = s.load(std::sync::atomic::Ordering::Relaxed);
                Arc::new(o)
            })
        }));
        // a reference no provider knows is not looked for in the roots
        assert!(lib.provided("props\\box").is_none());
        assert!(lib.cached("props\\box").is_none());
        // before it is loaded the key is the reference; then the reference with its stamp
        assert_eq!(lib.mesh_key("Models/Wall.meshasset"), "Models/Wall.meshasset");
        let Some(o) = lib.provided("Models/Wall.meshasset") else { panic!("not provided") };
        assert!(o.file.as_os_str().is_empty() && o.stamp == 7);
        let key = lib.mesh_key("models\\wall.meshasset");
        assert_eq!(key, "models\\wall.meshasset|0000000000000007");
        assert_eq!(Library::reference_of(&key), "models\\wall.meshasset");
        assert_eq!(Library::reference_of("props\\box"), "props\\box");
        // its source changed: forgotten, made again, a new key
        stamp.store(9, std::sync::atomic::Ordering::Relaxed);
        assert!(lib.forget("models\\wall.meshasset"));
        assert!(lib.provided("models\\wall.meshasset").is_some_and(|o| o.stamp == 9));
        assert_eq!(lib.mesh_key("models\\wall.meshasset"), "models\\wall.meshasset|0000000000000009");
    }
}
